//! Scene-level JSON export.
//!
//! Read-side dump of an `HSDRawFile`-equivalent tree into a flat JSON
//! schema (textures / materials / joints / joint_aliases / meshes).
//! Intended as the structural representation a downstream tool (e.g. a
//! Blender add-on) can drive its own scene-import pipeline against.
//!
//! The schema and field ordering are pinned by `tests/parity*.rs`
//! (semantic JSON diff with ε = 1e-5); see `docs/notes/phase0.md` §4
//! for the schema reference and `docs/notes/csx_export_parity.md` for
//! the optional cross-check against the upstream `dotnet-script`
//! golden output (which originally seeded this dump's field choices).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use indexmap::IndexMap;
use serde::Serialize;

use crate::accessor::Accessor;
use crate::common::{DObj, JObj, JObjDesc, MObj, PObj, SObj, TObj};
use crate::dat::{Dat, RootNode};
use crate::error::{HsdError, Result};
use crate::gx::{
    jobj_flag_names, render_flag_names, AlphaMap, ColorMap, GxTexFmt, PObjFlag,
};
use crate::gx_dl;
use crate::gx_dl::{Mat4, mat4_identity, mat4_mul, transform_normal, transform_point, normalize};
use crate::gx_image;
use crate::hsd_struct::{HsdStruct, identity};

// =====================================================================
// JSON DTOs (csx record types).  Field ordering follows csx for readability;
// JSON key ordering doesn't matter for parity (compare semantically).
// =====================================================================

#[derive(Serialize, Debug)]
pub struct Scene {
    pub source_dat: String,
    pub tex_dir: String,
    pub textures: Vec<TextureDto>,
    pub materials: Vec<MaterialDto>,
    pub joints: Vec<JointDto>,
    pub joint_aliases: IndexMap<String, String>,
    pub meshes: Vec<MeshDto>,
}

#[derive(Serialize, Debug)]
pub struct TextureDto {
    pub id: String,
    pub file: String,
    pub width: i32,
    pub height: i32,
    pub format: String,
}

#[derive(Serialize, Debug)]
pub struct TextureRefDto {
    pub tex_id: String,
    pub tex_map_id: String,
    pub wrap_s: String,
    pub wrap_t: String,
    pub repeat_s: i32,
    pub repeat_t: i32,
    pub mag_filter: String,
    pub color_op: String,
    pub alpha_op: String,
    pub blending: f32,
}

#[derive(Serialize, Debug)]
pub struct MaterialDto {
    pub id: String,
    pub render_flags: String,
    pub render_flags_raw: u32,
    pub diffuse_rgba: [i32; 4],
    pub alpha: f32,
    pub textures: Vec<TextureRefDto>,
}

#[derive(Serialize, Debug)]
pub struct PrimitiveDto {
    #[serde(rename = "type")]
    pub primitive_type: String,
    pub indices: Vec<u32>,
}

#[derive(Serialize, Debug)]
pub struct MeshDto {
    pub id: String,
    pub joint: String,
    pub single_bind_joint: Option<String>,
    pub material: Option<String>,
    pub cull: String,
    pub source_path: String,
    pub vertices: Vec<[f32; 3]>,
    pub uvs: Option<Vec<[f32; 2]>>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub colors: Option<Vec<[f32; 4]>>,
    pub primitives: Vec<PrimitiveDto>,
}

#[derive(Serialize, Debug)]
pub struct JointDto {
    pub id: String,
    pub name: Option<String>,
    pub flags: Vec<String>,
    pub translation: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
    pub world_matrix: [f32; 16],
    pub parent: Option<String>,
    pub children: Vec<String>,
}

// =====================================================================
// Exporter — drives csx-equivalent walk order and texture interning.
// =====================================================================

/// `tex_dir` is where `<sha>.png` files get written (relative path is
/// recorded in the JSON `tex_dir` field).  Pass `None` to skip texture file
/// writes — useful for tests that only diff the JSON structure.
pub fn export_scene(
    dat: &Dat,
    source_dat: impl Into<String>,
    tex_dir: Option<&Path>,
) -> Result<Scene> {
    let mut exporter = Exporter::new(tex_dir);
    let source_dat = source_dat.into();

    // Phase A: scene_data first (csx hsd_export_for_blender.csx:282-290).
    if let Some(root) = dat.scene_data() {
        let sobj = SObj::from_struct(root.data.clone());
        for desc in sobj.jobj_descs() {
            if let Some(rj) = desc.root_joint() {
                exporter.build_world(&rj, mat4_identity());
                exporter.emit_joint(&rj, None)?;
                exporter.emit_meshes(&rj)?;
            }
            // Anim slots dropped — out of Phase 1〜4 scope.
            let _: JObjDesc = desc;
        }
    }

    // Phase B: remaining roots (csx 291-304).
    for root in &dat.roots {
        if root.name == "scene_data" {
            continue;
        }
        export_one_root(&mut exporter, root)?;
    }

    Ok(exporter.into_scene(source_dat))
}

fn export_one_root(ex: &mut Exporter, root: &RootNode) -> Result<()> {
    let s_borrow = root.data.borrow();
    let len = s_borrow.len();
    drop(s_borrow);

    // Heuristic: HSDRawFile.GuessAccessor uses suffix matching.  For Phase 4
    // we treat any root whose struct length is JObj-sized (>= 0x40) as an
    // HSD_JOBJ — matches the csx "is HSD_JOBJ rj" branch on well-formed
    // JObj-root inputs.
    if len < 0x40 {
        return Ok(());
    }
    let jobj = JObj::from_struct(root.data.clone());
    ex.build_world(&jobj, mat4_identity());
    let id = identity(jobj.as_struct());
    if let Some(existing) = ex.jobj_id_by_struct.get(&id).cloned() {
        ex.joint_aliases.insert(root.name.clone(), existing);
    } else {
        let new_id = ex.emit_joint(&jobj, None)?;
        ex.joint_aliases.insert(root.name.clone(), new_id);
        ex.emit_meshes(&jobj)?;
    }
    Ok(())
}

// =====================================================================
// Internal exporter state
// =====================================================================

struct Exporter<'tex> {
    tex_dir: Option<&'tex Path>,
    textures: Vec<TextureDto>,
    materials: Vec<MaterialDto>,
    joints: Vec<JointDto>,
    meshes: Vec<MeshDto>,
    joint_aliases: IndexMap<String, String>,

    image_id_by_sha: HashMap<String, String>, // sha → id (= same string)
    jobj_id_by_struct: HashMap<*const RefCell<HsdStruct>, String>,
    world_by_jobj: HashMap<*const RefCell<HsdStruct>, Mat4>,

    joint_counter: u32,
    material_counter: u32,
    mesh_counter: u32,
}

impl<'tex> Exporter<'tex> {
    fn new(tex_dir: Option<&'tex Path>) -> Self {
        Self {
            tex_dir,
            textures: Vec::new(),
            materials: Vec::new(),
            joints: Vec::new(),
            meshes: Vec::new(),
            joint_aliases: IndexMap::new(),
            image_id_by_sha: HashMap::new(),
            jobj_id_by_struct: HashMap::new(),
            world_by_jobj: HashMap::new(),
            joint_counter: 0,
            material_counter: 0,
            mesh_counter: 0,
        }
    }

    fn into_scene(self, source_dat: String) -> Scene {
        Scene {
            source_dat,
            tex_dir: "tex".into(),
            textures: self.textures,
            materials: self.materials,
            joints: self.joints,
            joint_aliases: self.joint_aliases,
            meshes: self.meshes,
        }
    }

    /// Walk the `Next`/`Child` chain rooted at `j`, accumulating world
    /// matrices.  Visited tracking is by struct identity so alias roots
    /// don't double-compute.
    fn build_world(&mut self, j: &JObj, parent_world: Mat4) {
        let mut cur = Some(j.clone());
        while let Some(jobj) = cur {
            let id = identity(jobj.as_struct());
            if self.world_by_jobj.contains_key(&id) {
                cur = jobj.next();
                continue;
            }
            let local = match gx_dl::jobj_local(&jobj) {
                Ok(l) => l,
                Err(_) => mat4_identity(),
            };
            let world = mat4_mul(local, parent_world);
            self.world_by_jobj.insert(id, world);
            if let Some(child) = jobj.child() {
                self.build_world(&child, world);
            }
            cur = jobj.next();
        }
    }

    /// Emit a joint and all of its children, returning the assigned id.
    /// Idempotent: if the joint's struct was already emitted, returns the
    /// existing id without descending again (alias dedup, csx 175-180).
    fn emit_joint(&mut self, j: &JObj, parent_id: Option<&str>) -> Result<String> {
        let id_key = identity(j.as_struct());
        if let Some(existing) = self.jobj_id_by_struct.get(&id_key) {
            return Ok(existing.clone());
        }
        let id = format!("jobj_{}", self.joint_counter);
        self.joint_counter += 1;
        self.jobj_id_by_struct.insert(id_key, id.clone());

        let world = self.world_by_jobj.get(&id_key).copied().unwrap_or(mat4_identity());
        let world_matrix = flatten_mat4(world);

        let flags = j.flags()?;
        let flag_names = jobj_flag_names(flags)
            .into_iter()
            .map(|s| s.to_owned())
            .collect();

        let dto = JointDto {
            id: id.clone(),
            name: None,
            flags: flag_names,
            translation: [j.tx()?, j.ty()?, j.tz()?],
            rotation: [j.rx()?, j.ry()?, j.rz()?],
            scale: [j.sx()?, j.sy()?, j.sz()?],
            world_matrix,
            parent: parent_id.map(|s| s.to_owned()),
            children: Vec::new(), // filled in below
        };
        let dto_idx = self.joints.len();
        self.joints.push(dto);

        // Recurse into children, collect their IDs into this joint's list.
        if let Some(child) = j.child() {
            let mut child_ids = Vec::new();
            let mut cur = Some(child);
            while let Some(c) = cur {
                let cid = self.emit_joint(&c, Some(&id))?;
                child_ids.push(cid);
                cur = c.next();
            }
            self.joints[dto_idx].children = child_ids;
        }

        Ok(id)
    }

    /// Emit meshes for joint `j` and its descendants, mirroring csx
    /// `EmitMeshes`.
    fn emit_meshes(&mut self, j: &JObj) -> Result<()> {
        let mut cur = Some(j.clone());
        while let Some(jobj) = cur {
            let id_key = identity(jobj.as_struct());
            let Some(joint_id) = self.jobj_id_by_struct.get(&id_key).cloned() else {
                cur = jobj.next();
                continue;
            };
            if let Some(d) = jobj.dobj()? {
                self.emit_dobj_chain(&jobj, &d, &joint_id)?;
            }
            if let Some(child) = jobj.child() {
                self.emit_meshes(&child)?;
            }
            cur = jobj.next();
        }
        Ok(())
    }

    fn emit_dobj_chain(&mut self, parent_jobj: &JObj, d: &DObj, joint_id: &str) -> Result<()> {
        let mut cur = Some(d.clone());
        let mut d_idx = 0u32;
        while let Some(dobj) = cur {
            let mat_id = if let Some(m) = dobj.mobj() {
                Some(self.emit_material(&m)?)
            } else {
                None
            };
            if let Some(p) = dobj.pobj() {
                self.emit_pobj_chain(parent_jobj, &p, joint_id, mat_id.as_deref(), d_idx)?;
            }
            cur = dobj.next();
            d_idx += 1;
        }
        Ok(())
    }

    fn emit_pobj_chain(
        &mut self,
        parent_jobj: &JObj,
        p: &PObj,
        joint_id: &str,
        material_id: Option<&str>,
        d_idx: u32,
    ) -> Result<()> {
        let parent_id = identity(parent_jobj.as_struct());
        let parent_t = self.world_by_jobj.get(&parent_id).copied().unwrap_or(mat4_identity());

        let mut cur = Some(p.clone());
        let mut p_idx = 0u32;
        while let Some(pobj) = cur {
            let mesh_id = format!("mesh_{}", self.mesh_counter);
            self.mesh_counter += 1;

            let dl = gx_dl::unpack(&pobj)?;

            // single_bind_joint resolution
            let (sb_t, sb_id) = match pobj.single_bound_jobj()? {
                Some(sb) => {
                    let key = identity(sb.as_struct());
                    let t = self.world_by_jobj.get(&key).copied().unwrap_or(mat4_identity());
                    let id = self.jobj_id_by_struct.get(&key).cloned();
                    (t, id)
                }
                None => (mat4_identity(), None),
            };
            let final_t = mat4_mul(parent_t, sb_t);
            let mut rot_only = final_t;
            rot_only[3][0] = 0.0;
            rot_only[3][1] = 0.0;
            rot_only[3][2] = 0.0;

            // Detect optional channels exactly the csx way.
            let has_uv = dl
                .primitives
                .iter()
                .flat_map(|pg| pg.vertices.iter())
                .any(|v| v.tex0[0] != 0.0 || v.tex0[1] != 0.0);
            let has_nrm = dl
                .primitives
                .iter()
                .flat_map(|pg| pg.vertices.iter())
                .any(|v| v.nrm[0] != 0.0 || v.nrm[1] != 0.0 || v.nrm[2] != 0.0);
            let has_clr = dl.has_attribute(crate::gx::GxAttribName::GX_VA_CLR0);

            let mut verts = Vec::new();
            let mut uvs = if has_uv { Some(Vec::new()) } else { None };
            let mut nrms = if has_nrm { Some(Vec::new()) } else { None };
            let mut cols = if has_clr { Some(Vec::new()) } else { None };

            for pg in &dl.primitives {
                for v in &pg.vertices {
                    let p = transform_point([v.pos[0], v.pos[1], v.pos[2]], &final_t);
                    verts.push(p);
                    if let Some(u) = uvs.as_mut() {
                        u.push([v.tex0[0], v.tex0[1]]);
                    }
                    if let Some(n) = nrms.as_mut() {
                        let nn = normalize(transform_normal([v.nrm[0], v.nrm[1], v.nrm[2]], &rot_only));
                        n.push(nn);
                    }
                    if let Some(c) = cols.as_mut() {
                        c.push([v.clr0[0], v.clr0[1], v.clr0[2], v.clr0[3]]);
                    }
                }
            }

            // Per-primitive index runs are 0..N-1 cursor (csx 252-258).
            let mut prims = Vec::new();
            let mut cursor = 0u32;
            for pg in &dl.primitives {
                let n = pg.vertices.len() as u32;
                let indices: Vec<u32> = (0..n).map(|i| cursor + i).collect();
                let prim_name = primitive_type_name(pg.primitive_type);
                prims.push(PrimitiveDto {
                    primitive_type: prim_name,
                    indices,
                });
                cursor += n;
            }

            // POBJ_FLAG cull mode resolution.  HSDLib's CULLBACK /
            // CULLFRONT bits are deprecated for write (the writer no
            // longer emits them — see `pobj_writer::set_cull_back`),
            // but legacy course .dat files use the same bit positions
            // for their own cull encoding, so we keep the read-side
            // interpretation for csx-parity JSON output.
            #[allow(deprecated)]
            let flags_cullback = PObjFlag::CULLBACK;
            #[allow(deprecated)]
            let flags_cullfront = PObjFlag::CULLFRONT;
            let flags = pobj.flags()?;
            let cull_back = flags.contains(flags_cullback);
            let cull_front = flags.contains(flags_cullfront);
            let cull = match (cull_back, cull_front) {
                (true, true) => "BOTH",
                (true, false) => "BACK",
                (false, true) => "FRONT",
                (false, false) => "NONE",
            };

            self.meshes.push(MeshDto {
                id: mesh_id,
                joint: joint_id.to_owned(),
                single_bind_joint: sb_id,
                material: material_id.map(|s| s.to_owned()),
                cull: cull.to_owned(),
                source_path: format!("{}/DObj{}/PObj{}", joint_id, d_idx, p_idx),
                vertices: verts,
                uvs,
                normals: nrms,
                colors: cols,
                primitives: prims,
            });

            cur = pobj.next();
            p_idx += 1;
        }
        Ok(())
    }

    fn emit_material(&mut self, m: &MObj) -> Result<String> {
        let id = format!("mat_{}", self.material_counter);
        self.material_counter += 1;

        let mut tex_list = Vec::new();
        let mut cur = m.textures();
        while let Some(t) = cur {
            if let Some(tex_id) = self.intern_texture(&t)? {
                tex_list.push(TextureRefDto {
                    tex_id,
                    tex_map_id: format!("{:?}", t.tex_map_id()?),
                    wrap_s: format!("{:?}", t.wrap_s()?),
                    wrap_t: format!("{:?}", t.wrap_t()?),
                    repeat_s: t.repeat_s()? as i32,
                    repeat_t: t.repeat_t()? as i32,
                    mag_filter: format!("{:?}", t.mag_filter()?),
                    color_op: color_op_name(t.color_operation()?),
                    alpha_op: alpha_op_name(t.alpha_operation()?),
                    blending: t.blending()?,
                });
            }
            cur = t.next();
        }

        let (dif, alpha) = if let Some(mat) = m.material() {
            (mat.dif_rgba()?, mat.alpha()?)
        } else {
            ([255, 255, 255, 255], 1.0)
        };

        let render_flags = m.render_flags()?;
        let render_flags_str = render_flag_names(render_flags).join(", ");

        self.materials.push(MaterialDto {
            id: id.clone(),
            render_flags: render_flags_str,
            render_flags_raw: render_flags.bits(),
            diffuse_rgba: [
                dif[0] as i32, dif[1] as i32, dif[2] as i32, dif[3] as i32,
            ],
            alpha,
            textures: tex_list,
        });

        Ok(id)
    }

    fn intern_texture(&mut self, t: &TObj) -> Result<Option<String>> {
        let Some(img) = t.image_data() else {
            return Ok(None);
        };
        let Some(raw) = img.image_data() else {
            return Ok(None);
        };
        let sha = sha1_short_id(&raw);
        if self.image_id_by_sha.contains_key(&sha) {
            return Ok(Some(sha));
        }
        let w = img.width()? as i32;
        let h = img.height()? as i32;
        if w <= 0 || h <= 0 {
            return Ok(None);
        }
        let fmt = img.format()?;
        let fmt_name = match fmt {
            GxTexFmt::Unknown(v) => format!("Unknown_{}", v),
            other => format!("{:?}", other),
        };

        // Side-effect: write tex/<sha>.png if the exporter has an out dir.
        if let Some(dir) = self.tex_dir {
            let palette = t.tlut_data().and_then(|tl| {
                let bytes = tl.tlut_data()?;
                let f = tl.format().ok()?;
                Some((f, bytes))
            });
            let pal_ref = palette.as_ref().map(|(f, b)| (*f, b.as_slice()));

            match gx_image::decode_image(fmt, w as u32, h as u32, &raw, pal_ref) {
                Ok(rgba) => match gx_image::encode_png(&rgba, w as u32, h as u32) {
                    Ok(png_bytes) => {
                        std::fs::create_dir_all(dir).map_err(HsdError::from)?;
                        let path = dir.join(format!("{}.png", sha));
                        std::fs::write(&path, &png_bytes).map_err(HsdError::from)?;
                    }
                    Err(_) => return Ok(None),
                },
                Err(_) => return Ok(None),
            }
        }

        self.textures.push(TextureDto {
            id: sha.clone(),
            file: format!("tex/{}.png", sha),
            width: w,
            height: h,
            format: fmt_name,
        });
        self.image_id_by_sha.insert(sha.clone(), sha.clone());
        Ok(Some(sha))
    }
}

// =====================================================================
// Helpers
// =====================================================================

fn flatten_mat4(m: Mat4) -> [f32; 16] {
    [
        m[0][0], m[0][1], m[0][2], m[0][3],
        m[1][0], m[1][1], m[1][2], m[1][3],
        m[2][0], m[2][1], m[2][2], m[2][3],
        m[3][0], m[3][1], m[3][2], m[3][3],
    ]
}

fn primitive_type_name(p: crate::gx::GxPrimitiveType) -> String {
    use crate::gx::GxPrimitiveType as P;
    match p {
        P::Quads => "Quads",
        P::Triangles => "Triangles",
        P::TriangleStrip => "TriangleStrip",
        P::TriangleFan => "TriangleFan",
        P::Lines => "Lines",
        P::LineStrip => "LineStrip",
        P::Points => "Points",
        P::Unknown(v) => return format!("Unknown_0x{:02X}", v),
    }
    .to_owned()
}

fn color_op_name(c: ColorMap) -> String {
    match c {
        ColorMap::NONE => "NONE",
        ColorMap::ALPHA_MASK => "ALPHA_MASK",
        ColorMap::RGB_MASK => "RGB_MASK",
        ColorMap::BLEND => "BLEND",
        ColorMap::MODULATE => "MODULATE",
        ColorMap::REPLACE => "REPLACE",
        ColorMap::PASS => "PASS",
        ColorMap::ADD => "ADD",
        ColorMap::SUB => "SUB",
        ColorMap::Unknown(v) => return format!("Unknown_{}", v),
    }
    .to_owned()
}

fn alpha_op_name(a: AlphaMap) -> String {
    match a {
        AlphaMap::NONE => "NONE",
        AlphaMap::ALPHA_MASK => "ALPHA_MASK",
        AlphaMap::BLEND => "BLEND",
        AlphaMap::MODULATE => "MODULATE",
        AlphaMap::REPLACE => "REPLACE",
        AlphaMap::PASS => "PASS",
        AlphaMap::ADD => "ADD",
        AlphaMap::SUB => "SUB",
        AlphaMap::Unknown(v) => return format!("Unknown_{}", v),
    }
    .to_owned()
}

fn sha1_short_id(data: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let hash = Sha1::digest(data);
    let mut s = String::with_capacity(12);
    for b in &hash[..6] {
        use std::fmt::Write;
        let _ = write!(&mut s, "{:02X}", b);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitive_type_strings() {
        use crate::gx::GxPrimitiveType as P;
        assert_eq!(primitive_type_name(P::Triangles), "Triangles");
        assert_eq!(primitive_type_name(P::TriangleStrip), "TriangleStrip");
    }

    #[test]
    fn color_op_strings_match_csx() {
        assert_eq!(color_op_name(ColorMap::MODULATE), "MODULATE");
        assert_eq!(color_op_name(ColorMap::BLEND), "BLEND");
    }
}
