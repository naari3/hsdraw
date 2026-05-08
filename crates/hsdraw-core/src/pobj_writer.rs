//! POBJ writer — Phase 1 MVP.
//!
//! Generates an `HSD_POBJ` (+ attribute table + per-attribute vertex
//! buffers + GX display-list bytecode) from CPU-side mesh data.  Mirrors
//! `HSDRaw/Tools/POBJ_Generator.cs` at the byte layout level, but
//! intentionally limited:
//!
//! - **single attribute group per POBJ** (no Next chain)
//! - **TRIANGLES primitive only** — no triangle-strip optimization, the
//!   bytecode is `0x90 (count*3) <per-vertex u16 indices>`.  Phase 2 lifts
//!   this with a greedy stripification pass; for now the DL is bigger than
//!   HSDLib's optimized output but renders correctly.
//! - **fixed attribute encoding** — POS / NRM as F32×3, CLR0 as RGBA8,
//!   TEX0 as F32×2 — all `GX_INDEX16`.  Mesh data with more than 65,535
//!   verts has to be split into multiple POBJs by the caller.
//! - **no envelope rigging / shapeset** — MKGP2 course mesh is static
//!   single-bind, which the caller provides via `JObj.set_dobj` after
//!   `build()`.
//!
//! Per-attribute byte buffers are stored as separate "buffer" structs
//! (`is_buffer_aligned = true`); the writer 0x20-aligns them and dedups
//! byte-equal payloads via FNV-1a, exactly as HSDLib does.  The DL buffer
//! is treated the same way.

use crate::accessor::Accessor;
use crate::common::{JObj, PObj};
use crate::error::{HsdError, Result};
use crate::gx::{GxAttribName, GxAttribType, GxCompType, PObjFlag};
use crate::hsd_struct::{HsdStruct, StructRef};

/// CPU-side mesh inputs.  Builder pattern: push positions / normals /
/// colors / uvs / triangles, then call [`build`](MeshBuilder::build) for
/// a finished `PObj`.  The returned `PObj` is owned by you until you
/// attach it to a `DObj` via `DObj::set_pobj`.
#[derive(Debug, Clone)]
pub struct MeshBuilder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[u8; 4]>,
    uvs: Vec<[f32; 2]>,
    triangles: Vec<[u32; 3]>,
    cull_back: bool,
    cull_front: bool,
    /// Phase 2: greedy `TRIANGLE_STRIP` decomposition.  Default on —
    /// emits `0x98 (TriangleStrip)` primitive groups for chains of ≥ 2
    /// adjacent triangles, falling back to a single `0x90 (Triangles)`
    /// group for the leftover.  Disable to keep the Phase 1 single-
    /// group `Triangles`-only output (handy for debugging / parity vs.
    /// HSDLib-emitted DL bytecode).
    use_triangle_strips: bool,
    /// Phase 3: envelope rigging.  `envelopes[i]` is one matrix slot's
    /// worth of (jobj, weight) influences.  Empty → mesh is
    /// statically-bound (no PNMTXIDX, no `POBJ_FLAG.ENVELOPE`).
    envelopes: Vec<Vec<(JObj, f32)>>,
    /// Per-vertex envelope index (parallel to `positions`).  Required
    /// (with same length as `positions`) when `envelopes` is non-empty.
    envelope_indices: Vec<u32>,
}

impl Default for MeshBuilder {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            colors: Vec::new(),
            uvs: Vec::new(),
            triangles: Vec::new(),
            cull_back: false,
            cull_front: false,
            use_triangle_strips: true,
            envelopes: Vec::new(),
            envelope_indices: Vec::new(),
        }
    }
}

impl MeshBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_position(&mut self, x: f32, y: f32, z: f32) {
        self.positions.push([x, y, z]);
    }

    pub fn add_normal(&mut self, x: f32, y: f32, z: f32) {
        self.normals.push([x, y, z]);
    }

    /// RGBA8 color.  Stored verbatim in the per-vertex buffer; if you
    /// need RGB565 / RGBA4 / etc.  use multiple POBJs (one per format)
    /// — this MVP fixes RGBA8 to keep the encoding path branchless.
    pub fn add_color(&mut self, r: u8, g: u8, b: u8, a: u8) {
        self.colors.push([r, g, b, a]);
    }

    pub fn add_uv(&mut self, u: f32, v: f32) {
        self.uvs.push([u, v]);
    }

    /// Indices into the position list (0-based).  Triangle ordering is
    /// (i0, i1, i2) → CCW when viewed from outside; cull flags are
    /// honored verbatim by the GPU.
    pub fn add_triangle(&mut self, i0: u32, i1: u32, i2: u32) {
        self.triangles.push([i0, i1, i2]);
    }

    /// Sets `POBJ_FLAG.CULLBACK` on the produced POBJ.
    pub fn set_cull_back(&mut self, on: bool) {
        self.cull_back = on;
    }

    /// Sets `POBJ_FLAG.CULLFRONT` on the produced POBJ.
    pub fn set_cull_front(&mut self, on: bool) {
        self.cull_front = on;
    }

    /// Toggle Phase 2 greedy `TRIANGLE_STRIP` decomposition (on by
    /// default).  Pass `false` to keep the Phase 1 single-`Triangles`-
    /// group emit path — useful for debugging or for pinning down
    /// exact DL bytecode when comparing against HSDLib's optimized
    /// output.
    pub fn set_use_triangle_strips(&mut self, on: bool) {
        self.use_triangle_strips = on;
    }

    /// Phase 3: add a new envelope (one matrix slot's worth of bone
    /// influences).  Each envelope is a list of `(JObj, weight)` —
    /// weights should sum to ~1.0 per envelope (HSDLib's renderer
    /// blends them as `sum(weight_i × matrix_i)`).  Returns the
    /// envelope's index, which a vertex's `add_envelope_index` then
    /// references.  `(envelope_index × 3)` is the GX matrix slot the
    /// renderer ends up using; the multiplier handles pos / normal /
    /// binormal triple — see HSDLib `POBJ_Generator`.
    ///
    /// MKGP2 course meshes are static (single-bind) and don't need
    /// this; it's here for Smash-style fighter use cases.
    pub fn add_envelope(&mut self, weights: Vec<(JObj, f32)>) -> u32 {
        self.envelopes.push(weights);
        (self.envelopes.len() - 1) as u32
    }

    /// Per-vertex envelope index (parallel to position pushes).  When
    /// envelopes are in use, every vertex must have an associated
    /// envelope; the count must match `positions`.  Caller's
    /// responsibility to track ordering — same as for normals / UVs.
    pub fn add_envelope_index(&mut self, env_idx: u32) {
        self.envelope_indices.push(env_idx);
    }

    pub fn envelope_count(&self) -> usize {
        self.envelopes.len()
    }

    /// Convenience: how many positions have been pushed.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Validate inputs and emit the POBJ.  Returns the `PObj` accessor
    /// view; the underlying `Rc<RefCell<HsdStruct>>` is the value-shaped
    /// `PObj.0` field — you can clone it and stash it elsewhere if you
    /// need additional handles.
    pub fn build(self) -> Result<PObj> {
        let n_verts = self.positions.len();
        if n_verts == 0 {
            return Err(HsdError::malformed(
                0,
                "MeshBuilder::build: no positions added",
            ));
        }
        if n_verts > 0xFFFF {
            return Err(HsdError::malformed(
                0,
                "MeshBuilder::build: vertex count exceeds u16 (split into multiple POBJs)",
            ));
        }
        if self.triangles.is_empty() {
            return Err(HsdError::malformed(
                0,
                "MeshBuilder::build: no triangles added",
            ));
        }
        if !self.normals.is_empty() && self.normals.len() != n_verts {
            return Err(HsdError::malformed(
                0,
                "MeshBuilder::build: normal count != position count",
            ));
        }
        if !self.colors.is_empty() && self.colors.len() != n_verts {
            return Err(HsdError::malformed(
                0,
                "MeshBuilder::build: color count != position count",
            ));
        }
        if !self.uvs.is_empty() && self.uvs.len() != n_verts {
            return Err(HsdError::malformed(
                0,
                "MeshBuilder::build: uv count != position count",
            ));
        }
        for tri in &self.triangles {
            for &idx in tri {
                if idx as usize >= n_verts {
                    return Err(HsdError::malformed(
                        0,
                        "MeshBuilder::build: triangle index out of range",
                    ));
                }
            }
        }

        // ---------- Phase 3 envelope validation ----------
        let use_envelopes = !self.envelopes.is_empty();
        if use_envelopes {
            if self.envelope_indices.len() != n_verts {
                return Err(HsdError::malformed(
                    0,
                    "MeshBuilder::build: envelope_indices count != position count",
                ));
            }
            for &eidx in &self.envelope_indices {
                if (eidx as usize) >= self.envelopes.len() {
                    return Err(HsdError::malformed(
                        0,
                        "MeshBuilder::build: envelope index out of range",
                    ));
                }
            }
            // PNMTXIDX is 1 byte; per HSDLib `POBJ_Generator` the matrix
            // slot used is `envelope_index * 3`, so the highest envelope
            // index must satisfy `(idx * 3) < 256` → idx < 86.
            if self.envelopes.len() >= 86 {
                return Err(HsdError::malformed(
                    0,
                    "MeshBuilder::build: envelope count exceeds 85 (PNMTXIDX overflow — split into multiple POBJs)",
                ));
            }
        } else if !self.envelope_indices.is_empty() {
            return Err(HsdError::malformed(
                0,
                "MeshBuilder::build: envelope_indices added without any envelopes",
            ));
        }

        // ---------- per-attribute byte buffers --------------------------
        let pos_buf = encode_f32_buffer(self.positions.iter().map(|p| p.as_slice()));
        let pos_ref = make_buffer_struct(pos_buf);

        let nrm_ref = if !self.normals.is_empty() {
            Some(make_buffer_struct(encode_f32_buffer(
                self.normals.iter().map(|p| p.as_slice()),
            )))
        } else {
            None
        };

        let clr_ref = if !self.colors.is_empty() {
            let mut buf = Vec::with_capacity(n_verts * 4);
            for c in &self.colors {
                buf.extend_from_slice(c);
            }
            Some(make_buffer_struct(buf))
        } else {
            None
        };

        let uv_ref = if !self.uvs.is_empty() {
            Some(make_buffer_struct(encode_f32_buffer(
                self.uvs.iter().map(|p| p.as_slice()),
            )))
        } else {
            None
        };

        // ---------- attribute table ------------------------------------
        // Layout: N × 0x18 + NULL terminator (also 0x18).  The Buffer
        // reference for attribute i lives at offset i*0x18 + 0x14 *of the
        // parent struct* — that's how HSDLib parses it back via
        // `frankenStruct.GetEmbeddedStruct(i*0x18, 0x18)` and we mirror
        // that on the read side in `gx_dl::read_attribute_at`.
        let mut attrs: Vec<AttrSpec> = Vec::new();

        // PNMTXIDX (Phase 3): when envelopes are in use, emit this as
        // the FIRST attribute so the per-vertex DL bytecode reads
        // `<1 byte PNMTXIDX><N×2 byte indices>` (the order matches
        // HSDLib `POBJ_Generator`).  AttributeType = GX_DIRECT, no
        // buffer — the value lives inline in the DL stream.
        if use_envelopes {
            attrs.push(AttrSpec {
                name: GxAttribName::GX_VA_PNMTXIDX,
                kind: GxAttribType::GX_DIRECT,
                comp_count: 0,
                comp_type: 0,
                stride: 0,
                buffer: None,
            });
        }

        // POS: PosXYZ (1), Float (4), stride 12.  HSDLib's GX_VA_POS.
        attrs.push(AttrSpec {
            name: GxAttribName::GX_VA_POS,
            kind: GxAttribType::GX_INDEX16,
            // GXCompCnt::PosXYZ = 1
            comp_count: 1,
            comp_type: GxCompType::Float.into(),
            stride: 12,
            buffer: Some(pos_ref),
        });
        if let Some(b) = nrm_ref {
            // NRM: NrmXYZ (0), Float (4), stride 12.
            attrs.push(AttrSpec {
                name: GxAttribName::GX_VA_NRM,
                kind: GxAttribType::GX_INDEX16,
                comp_count: 0,
                comp_type: GxCompType::Float.into(),
                stride: 12,
                buffer: Some(b),
            });
        }
        if let Some(b) = clr_ref {
            // CLR0: ClrRGBA (1), GXCompTypeClr::RGBA8 (= 5), stride 4.
            attrs.push(AttrSpec {
                name: GxAttribName::GX_VA_CLR0,
                kind: GxAttribType::GX_INDEX16,
                comp_count: 1,
                comp_type: 5,
                stride: 4,
                buffer: Some(b),
            });
        }
        if let Some(b) = uv_ref {
            // TEX0: TexST (1), Float (4), stride 8.
            attrs.push(AttrSpec {
                name: GxAttribName::GX_VA_TEX0,
                kind: GxAttribType::GX_INDEX16,
                comp_count: 1,
                comp_type: GxCompType::Float.into(),
                stride: 8,
                buffer: Some(b),
            });
        }

        let n_attr = attrs.len() as u32;
        let attr_size = ((n_attr + 1) * 0x18) as usize;
        let attr_struct = HsdStruct::with_capacity(attr_size).into_ref();
        {
            let mut s = attr_struct.borrow_mut();
            for (i, a) in attrs.iter().enumerate() {
                let off = (i as u32) * 0x18;
                s.set_u32(off + 0x00, a.name.into())?;
                s.set_u32(off + 0x04, a.kind.into())?;
                s.set_u32(off + 0x08, a.comp_count)?;
                s.set_u32(off + 0x0C, a.comp_type)?;
                // 0x10: scale (u8) = 0; 0x11: padding = 0
                s.set_u16(off + 0x12, a.stride)?;
                // 0x14: per-attribute buffer ref; absent for GX_DIRECT
                // (PNMTXIDX) where the value lives inline in the DL.
                s.set_reference(off + 0x14, a.buffer.clone());
            }
            // GX_VA_NULL terminator at the tail; remaining fields stay
            // zero-initialized (matches HSDLib's `new GX_Attribute()`).
            let term = n_attr * 0x18;
            s.set_u32(term + 0x00, GxAttribName::GX_VA_NULL.into())?;
        }

        // ---------- DL bytecode ---------------------------------------
        // The bytecode is one or more primitive groups (each
        // `<u8 primitive type><u16 vertex count><per-vertex u16 indices>`)
        // followed by a `0x00` terminator and 0x20 alignment padding.  We
        // emit either:
        //
        //   - Phase 1 path (`use_triangle_strips = false`): one
        //     `0x90 (Triangles)` group with `count * 3` vertices.
        //   - Phase 2 path (default): greedy `0x98 (TriangleStrip)`
        //     decomposition for chains of ≥ MIN_STRIP_VERTS vertices,
        //     plus a single trailing `Triangles` group for the leftover.
        //
        // The MVP emits the same vertex index for every attribute (i.e.,
        // POS[i] / NRM[i] / CLR[i] / UV[i] all addressed by i) — that's
        // what the reader expects in the simplest decode path.
        let env_per_vertex: Option<&[u32]> = if use_envelopes {
            Some(&self.envelope_indices)
        } else {
            None
        };
        let dl_buf = if self.use_triangle_strips {
            let (strips, leftover) = build_strips(&self.triangles, MIN_STRIP_VERTS);
            encode_dl_mixed(&strips, &leftover, &attrs, env_per_vertex)?
        } else {
            // Single-group path needs a u16-fitting vertex count.
            let n_dl_verts = self.triangles.len() * 3;
            if n_dl_verts > 0xFFFF {
                return Err(HsdError::malformed(
                    0,
                    "MeshBuilder::build: triangle count exceeds GX primitive group limit (split into multiple POBJs)",
                ));
            }
            encode_dl_triangles(&self.triangles, &attrs, env_per_vertex)
        };
        let dl_size_in_32_units = (dl_buf.len() / 32) as i16;
        let dl_ref = make_buffer_struct(dl_buf);

        // ---------- envelope array (Phase 3) --------------------------
        // When in use, 0x14 of POBJ points at a struct holding a
        // null-terminated array of HSD_Envelope refs (one per envelope
        // index used by the DL).  Each HSD_Envelope is itself an
        // 8-byte (jobj_ref, weight) entry array with a trailing
        // (null, 0) terminator — see HSDLib `HSD_Envelope.Add`.
        let envelope_array_ref = if use_envelopes {
            Some(build_envelope_array(&self.envelopes)?)
        } else {
            None
        };

        // ---------- POBJ struct ---------------------------------------
        let pobj_struct = HsdStruct::with_capacity(0x18).into_ref();
        {
            let mut s = pobj_struct.borrow_mut();
            // 0x00 ClassName: null
            // 0x04 Next: null
            s.set_reference(0x08, Some(attr_struct));
            let mut flags: u16 = 0;
            if self.cull_back {
                flags |= PObjFlag::CULLBACK.bits();
            }
            if self.cull_front {
                flags |= PObjFlag::CULLFRONT.bits();
            }
            if use_envelopes {
                flags |= PObjFlag::ENVELOPE.bits();
            }
            s.set_u16(0x0C, flags)?;
            s.set_i16(0x0E, dl_size_in_32_units)?;
            s.set_reference(0x10, Some(dl_ref));
            // 0x14: tagged union — when ENVELOPE flag is set, points
            // at the null-terminated envelope-pointer array.  Otherwise
            // null (= no SingleBoundJOBJ / ShapeSet either).
            s.set_reference(0x14, envelope_array_ref);
        }
        Ok(PObj::from_struct(pobj_struct))
    }
}

struct AttrSpec {
    name: GxAttribName,
    /// `GX_DIRECT` for inline 1-byte values (PNMTXIDX), `GX_INDEX16`
    /// for the buffer-backed F32 / RGBA8 attrs.  GX_INDEX8 / GX_NONE
    /// aren't emitted by Phase 1–3.
    kind: GxAttribType,
    comp_count: u32,
    comp_type: u32,
    /// Stride bytes per indexed entry (matches HSDLib `GX_Attribute.Stride`).
    /// 0 for DIRECT attributes (no buffer).
    stride: u16,
    /// Per-attribute payload buffer (`is_buffer_aligned = true` —
    /// 0x20-aligned + FNV-1a-deduped at write time).  `None` for
    /// DIRECT attributes.
    buffer: Option<StructRef>,
}

/// Flatten an iterator of f32 slices into a big-endian byte buffer.
fn encode_f32_buffer<'a, I>(items: I) -> Vec<u8>
where
    I: IntoIterator<Item = &'a [f32]>,
{
    let mut out = Vec::new();
    for item in items {
        for v in item {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
    out
}

/// Wrap a raw byte payload as a `StructRef` flagged for 0x20 alignment
/// (`is_buffer_aligned`).  The writer's `is_buffer` predicate then puts
/// it through the FNV-1a dedup + 0x20-align path, matching HSDLib's
/// `SetBuffer` path.
fn make_buffer_struct(bytes: Vec<u8>) -> StructRef {
    let s = HsdStruct::from_bytes(bytes).into_ref();
    s.borrow_mut().is_buffer_aligned = true;
    s
}

/// Build the DL bytecode for one TRIANGLES primitive group + EOF
/// terminator + 0x20 padding.  Per-vertex emit is delegated to
/// `write_vertex_attrs` so DIRECT (PNMTXIDX) and INDEX16 (POS / NRM /
/// CLR / UV) attribute kinds are handled uniformly.
fn encode_dl_triangles(
    triangles: &[[u32; 3]],
    attrs: &[AttrSpec],
    env_per_vertex: Option<&[u32]>,
) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let n_dl_verts = (triangles.len() * 3) as u16;
    out.push(0x90); // GX_DRAW_TRIANGLES
    out.extend_from_slice(&n_dl_verts.to_be_bytes());
    for tri in triangles {
        for &vert_idx in tri {
            write_vertex_attrs(&mut out, vert_idx, attrs, env_per_vertex);
        }
    }
    finalize_dl_buffer(&mut out);
    out
}

/// Phase 2 DL emitter: one `TriangleStrip` (0x98) primitive group per
/// strip, plus one trailing `Triangles` (0x90) group for the leftover
/// (if any).  Single byte per primitive group's `primitive_type`, u16
/// BE vertex count, then per-vertex attribute payload — exactly the
/// shape the reader's `gx_dl::read_primitives` parses.
fn encode_dl_mixed(
    strips: &[Vec<u32>],
    leftover: &[[u32; 3]],
    attrs: &[AttrSpec],
    env_per_vertex: Option<&[u32]>,
) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();

    for strip in strips {
        if strip.len() > 0xFFFF {
            // No real-world MKGP2 mesh approaches this, but a 16-bit
            // strip-vertex-count overflow would silently truncate.  Bail
            // out so the caller knows to split the source mesh up.
            return Err(HsdError::malformed(
                0,
                "MeshBuilder::build: triangle-strip vertex count exceeds u16 (split into multiple POBJs)",
            ));
        }
        out.push(0x98); // GX_DRAW_TRIANGLE_STRIP
        out.extend_from_slice(&(strip.len() as u16).to_be_bytes());
        for &vert_idx in strip {
            write_vertex_attrs(&mut out, vert_idx, attrs, env_per_vertex);
        }
    }

    if !leftover.is_empty() {
        let n_dl_verts = leftover.len() * 3;
        if n_dl_verts > 0xFFFF {
            return Err(HsdError::malformed(
                0,
                "MeshBuilder::build: leftover triangle count exceeds u16 (split into multiple POBJs)",
            ));
        }
        out.push(0x90); // GX_DRAW_TRIANGLES
        out.extend_from_slice(&(n_dl_verts as u16).to_be_bytes());
        for tri in leftover {
            for &vert_idx in tri {
                write_vertex_attrs(&mut out, vert_idx, attrs, env_per_vertex);
            }
        }
    }

    finalize_dl_buffer(&mut out);
    Ok(out)
}

/// Per-vertex attribute payload emit.  Handled here so the strip /
/// triangle / leftover paths share one definition.  For each attr in
/// declaration order:
///
///   - `GX_DIRECT` + `GX_VA_PNMTXIDX` → 1 byte = `(env_idx * 3) as u8`.
///     `env_per_vertex` must be `Some(_)` and `vert_idx` must be in
///     range; the validation pass in `build()` already enforces this.
///   - `GX_INDEX16` → 2 bytes BE = `vert_idx as u16`.
///   - Other kinds aren't emitted by the Phase 1–3 writer (callers
///     should never see them on this path).
fn write_vertex_attrs(
    out: &mut Vec<u8>,
    vert_idx: u32,
    attrs: &[AttrSpec],
    env_per_vertex: Option<&[u32]>,
) {
    let idx16 = vert_idx as u16;
    for a in attrs {
        match a.kind {
            GxAttribType::GX_DIRECT => {
                if a.name == GxAttribName::GX_VA_PNMTXIDX {
                    let env = env_per_vertex
                        .expect("PNMTXIDX without env_per_vertex is a writer bug");
                    let pid = (env[vert_idx as usize] * 3) as u8;
                    out.push(pid);
                }
                // DIRECT non-PNMTXIDX (e.g. inline color) isn't emitted
                // by Phase 1–3; future widening hooks here.
            }
            GxAttribType::GX_INDEX16 => {
                out.extend_from_slice(&idx16.to_be_bytes());
            }
            _ => {}
        }
    }
}

/// Build the null-terminated envelope-pointer array struct + per-
/// envelope HSD_Envelope structs.  The array struct holds one
/// 4-byte slot per envelope plus a final null slot (no entry — its
/// absence in the references map is the terminator).  Each envelope
/// struct holds N × 8-byte `(jobj_ref, weight_f32)` entries plus one
/// trailing 8-byte `(null, 0)` slot — matches HSDLib `HSD_Envelope.Add`'s
/// "Length += 8" twice on first add convention.
fn build_envelope_array(envelopes: &[Vec<(JObj, f32)>]) -> Result<StructRef> {
    let n_env = envelopes.len();
    if n_env == 0 {
        return Err(HsdError::malformed(
            0,
            "build_envelope_array: empty envelope list",
        ));
    }
    let arr_struct = HsdStruct::with_capacity((n_env + 1) * 4).into_ref();
    {
        let mut arr = arr_struct.borrow_mut();
        for (i, weights) in envelopes.iter().enumerate() {
            let env_struct = build_envelope(weights)?;
            arr.set_reference((i as u32) * 4, Some(env_struct));
        }
        // Final 4 bytes: null pointer terminator (already zero-init).
    }
    Ok(arr_struct)
}

/// Build one HSD_Envelope struct for the given (jobj, weight) list.
fn build_envelope(weights: &[(JObj, f32)]) -> Result<StructRef> {
    if weights.is_empty() {
        return Err(HsdError::malformed(
            0,
            "build_envelope: envelope must have at least one entry",
        ));
    }
    // (N + 1) entries × 8 bytes — last entry is (null, 0) terminator.
    let env = HsdStruct::with_capacity((weights.len() + 1) * 8).into_ref();
    {
        let mut s = env.borrow_mut();
        for (i, (jobj, weight)) in weights.iter().enumerate() {
            let off = (i as u32) * 8;
            s.set_reference(off, Some(jobj.0.clone()));
            s.set_f32(off + 4, *weight)?;
        }
        // Trailing 8-byte zero-initialized entry plays the role of
        // HSDLib's "Length is one extra 8-byte block" terminator.
    }
    Ok(env)
}

/// Stream terminator + 0x20 padding (POBJ's `DisplayListSize` field
/// stores `bytes/32`; the buffer must be a multiple of 32 — HSDLib
/// does the same with `writer.Align(0x20)`).
fn finalize_dl_buffer(out: &mut Vec<u8>) {
    out.push(0x00);
    while out.len() % 0x20 != 0 {
        out.push(0);
    }
}

/// Minimum strip vertex count to be worth emitting.  3 verts = 1 tri
/// (same byte cost as a TRIANGLES leftover entry, no benefit); 4 verts
/// = 2 tris saves bytes vs. TRIANGLES.  Single seed tris that don't
/// extend at all therefore go to the leftover bucket.
const MIN_STRIP_VERTS: usize = 4;

/// Greedy stripper: walk the triangle adjacency graph, growing a
/// triangle strip out of each unvisited seed in whichever of the
/// seed's three orientations yields the longest strip.  Triangles
/// that can't be grown to `min_verts` go to the leftover list.
///
/// Output:
///   - `Vec<Vec<u32>>` — list of strips, each a sequence of vertex
///     indices (length ≥ `min_verts`).  Decoded triangles per strip:
///     `(v[i], v[i+1], v[i+2])` for `i ∈ [0, len-2]`.
///   - `Vec<[u32; 3]>` — leftover triangles flattened from non-stripped
///     seeds.
///
/// This is *not* the full HSDLib `TriangleConverter` (which uses a
/// vertex-cache simulator + a priority heap).  The output is bigger
/// than HSDLib's optimized DL but still significantly smaller than the
/// Phase 1 single-`Triangles`-group path on real meshes, and it's
/// O(n_tri × avg_strip_len) — comfortably fast for the addon's input
/// sizes (course meshes ≈ a few thousand tris).
fn build_strips(
    triangles: &[[u32; 3]],
    min_verts: usize,
) -> (Vec<Vec<u32>>, Vec<[u32; 3]>) {
    use std::collections::HashMap;

    // Edge → tri-indices that contain that edge.  Edge is undirected:
    // `(min(a,b), max(a,b))` keys.
    let mut adj: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, tri) in triangles.iter().enumerate() {
        let edges = [
            (tri[0], tri[1]),
            (tri[1], tri[2]),
            (tri[2], tri[0]),
        ];
        for &(x, y) in &edges {
            let key = if x < y { (x, y) } else { (y, x) };
            adj.entry(key).or_default().push(i);
        }
    }

    let mut visited = vec![false; triangles.len()];
    let mut strips: Vec<Vec<u32>> = Vec::new();
    let mut leftover: Vec<[u32; 3]> = Vec::new();

    for seed in 0..triangles.len() {
        if visited[seed] {
            continue;
        }
        let tri = triangles[seed];

        // Try all 3 cyclic orientations and pick the longest strip.
        // Each orientation places a different edge as the strip's
        // initial "advance" edge:
        //   0 → (a, b, c) → advance edge (b, c)
        //   1 → (b, c, a) → advance edge (c, a)
        //   2 → (c, a, b) → advance edge (a, b)
        let mut best: (Vec<u32>, Vec<usize>) = (Vec::new(), Vec::new());
        for orient in 0..3 {
            let (v0, v1, v2) = match orient {
                0 => (tri[0], tri[1], tri[2]),
                1 => (tri[1], tri[2], tri[0]),
                _ => (tri[2], tri[0], tri[1]),
            };
            let mut local_visited = visited.clone();
            local_visited[seed] = true;
            let mut strip_verts = vec![v0, v1, v2];
            let mut consumed = vec![seed];

            loop {
                let last1 = strip_verts[strip_verts.len() - 2];
                let last2 = strip_verts[strip_verts.len() - 1];
                let key = if last1 < last2 { (last1, last2) } else { (last2, last1) };
                let next = adj.get(&key).and_then(|cands| {
                    cands.iter().copied().find(|&idx| !local_visited[idx])
                });
                let nidx = match next {
                    Some(i) => i,
                    None => break,
                };
                let n = triangles[nidx];
                // The neighbor's third vertex — the one not on the
                // shared edge — is the strip's next vertex.
                let v_new = if n[0] != last1 && n[0] != last2 {
                    n[0]
                } else if n[1] != last1 && n[1] != last2 {
                    n[1]
                } else {
                    n[2]
                };
                strip_verts.push(v_new);
                consumed.push(nidx);
                local_visited[nidx] = true;
            }

            if strip_verts.len() > best.0.len() {
                best = (strip_verts, consumed);
            }
        }

        if best.0.len() >= min_verts {
            for &idx in &best.1 {
                visited[idx] = true;
            }
            strips.push(best.0);
        } else {
            // Even the best orientation didn't extend past the seed;
            // emit it as a single TRIANGLES entry.
            visited[seed] = true;
            leftover.push(tri);
        }
    }

    (strips, leftover)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_positions() {
        let mb = MeshBuilder::new();
        assert!(mb.build().is_err());
    }

    #[test]
    fn rejects_no_triangles() {
        let mut mb = MeshBuilder::new();
        mb.add_position(0.0, 0.0, 0.0);
        mb.add_position(1.0, 0.0, 0.0);
        mb.add_position(0.0, 1.0, 0.0);
        assert!(mb.build().is_err());
    }

    #[test]
    fn rejects_triangle_index_oob() {
        let mut mb = MeshBuilder::new();
        mb.add_position(0.0, 0.0, 0.0);
        mb.add_position(1.0, 0.0, 0.0);
        mb.add_position(0.0, 1.0, 0.0);
        mb.add_triangle(0, 1, 5); // 5 is OOB
        assert!(mb.build().is_err());
    }

    #[test]
    fn rejects_attribute_count_mismatch() {
        let mut mb = MeshBuilder::new();
        mb.add_position(0.0, 0.0, 0.0);
        mb.add_position(1.0, 0.0, 0.0);
        mb.add_position(0.0, 1.0, 0.0);
        mb.add_normal(0.0, 0.0, 1.0); // only 1 normal, but 3 positions
        mb.add_triangle(0, 1, 2);
        assert!(mb.build().is_err());
    }

    #[test]
    fn pobj_layout_size() {
        let mut mb = MeshBuilder::new();
        mb.add_position(0.0, 0.0, 0.0);
        mb.add_position(1.0, 0.0, 0.0);
        mb.add_position(0.0, 1.0, 0.0);
        mb.add_triangle(0, 1, 2);
        let pobj = mb.build().expect("build");
        assert_eq!(pobj.0.borrow().len(), 0x18);
    }
}
