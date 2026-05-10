//! High-level accessors mapping to `HSDRaw/Common/HSD_*.cs`.  Phase 1 carries
//! only the read-side fields needed for the JObj tree dump and the future
//! Blender JSON export; field names match HSDLib exactly so cross-referencing
//! the C# source is mechanical.

use crate::accessor::{Accessor, accessor};
use crate::error::Result;
use crate::gx::{
    AlphaMap, ColorMap, CoordType, GxAnisotropy, GxTexFilter, GxTexFmt, GxTexGenSrc, GxTexMapId,
    GxTlutFmt, GxWrapMode, JObjFlag, MaterialRenderMode, PObjFlag, TObjFlags,
};
use crate::hsd_struct::{HsdStruct, StructRef};

// =====================================================================
// JObj  (HSDRaw/Common/HSD_JOBJ.cs, TrimmedSize 0x40)
// =====================================================================

accessor!(JObj);

impl JObj {
    pub fn class_name(&self) -> Result<Option<String>> {
        self.s().get_string(0x00)
    }
    pub fn flags(&self) -> Result<JObjFlag> {
        Ok(JObjFlag::from_bits_retain(self.s().get_u32(0x04)?))
    }
    pub fn child(&self) -> Option<JObj> {
        self.ref_at::<JObj>(0x08)
    }
    pub fn next(&self) -> Option<JObj> {
        self.ref_at::<JObj>(0x0C)
    }
    /// `Dobj` only when neither SPLINE nor PTCL flags are set; otherwise the
    /// reference at 0x10 is a spline / particle joint payload (out of scope
    /// for Phase 1 — we treat them as "no Dobj" here).
    pub fn dobj(&self) -> Result<Option<DObj>> {
        let f = self.flags()?;
        if f.intersects(JObjFlag::SPLINE | JObjFlag::PTCL) {
            return Ok(None);
        }
        Ok(self.ref_at::<DObj>(0x10))
    }
    pub fn rx(&self) -> Result<f32> { self.s().get_f32(0x14) }
    pub fn ry(&self) -> Result<f32> { self.s().get_f32(0x18) }
    pub fn rz(&self) -> Result<f32> { self.s().get_f32(0x1C) }
    pub fn sx(&self) -> Result<f32> { self.s().get_f32(0x20) }
    pub fn sy(&self) -> Result<f32> { self.s().get_f32(0x24) }
    pub fn sz(&self) -> Result<f32> { self.s().get_f32(0x28) }
    pub fn tx(&self) -> Result<f32> { self.s().get_f32(0x2C) }
    pub fn ty(&self) -> Result<f32> { self.s().get_f32(0x30) }
    pub fn tz(&self) -> Result<f32> { self.s().get_f32(0x34) }

    // ----- mutators (used by `import_from_scene_json`) -----------------
    // Mirror HSDLib `HSD_JOBJ` setters at the same offsets.  Tied to the
    // 0x40 layout — a struct shorter than that (e.g. a freshly allocated
    // 0-byte HsdStruct) must be `resize`d up first; the helper below does
    // exactly that, idempotently.
    pub fn set_flags(&self, flags: JObjFlag) -> Result<()> {
        self.ensure_jobj_size();
        self.0.borrow_mut().set_u32(0x04, flags.bits())
    }
    pub fn set_child(&self, child: Option<JObj>) {
        self.ensure_jobj_size();
        self.0
            .borrow_mut()
            .set_reference(0x08, child.map(|c| c.0));
    }
    pub fn set_next(&self, next: Option<JObj>) {
        self.ensure_jobj_size();
        self.0
            .borrow_mut()
            .set_reference(0x0C, next.map(|c| c.0));
    }
    pub fn set_rx(&self, v: f32) -> Result<()> { self.ensure_jobj_size(); self.0.borrow_mut().set_f32(0x14, v) }
    pub fn set_ry(&self, v: f32) -> Result<()> { self.ensure_jobj_size(); self.0.borrow_mut().set_f32(0x18, v) }
    pub fn set_rz(&self, v: f32) -> Result<()> { self.ensure_jobj_size(); self.0.borrow_mut().set_f32(0x1C, v) }
    pub fn set_sx(&self, v: f32) -> Result<()> { self.ensure_jobj_size(); self.0.borrow_mut().set_f32(0x20, v) }
    pub fn set_sy(&self, v: f32) -> Result<()> { self.ensure_jobj_size(); self.0.borrow_mut().set_f32(0x24, v) }
    pub fn set_sz(&self, v: f32) -> Result<()> { self.ensure_jobj_size(); self.0.borrow_mut().set_f32(0x28, v) }
    pub fn set_tx(&self, v: f32) -> Result<()> { self.ensure_jobj_size(); self.0.borrow_mut().set_f32(0x2C, v) }
    pub fn set_ty(&self, v: f32) -> Result<()> { self.ensure_jobj_size(); self.0.borrow_mut().set_f32(0x30, v) }
    pub fn set_tz(&self, v: f32) -> Result<()> { self.ensure_jobj_size(); self.0.borrow_mut().set_f32(0x34, v) }

    /// Set / clear the DObj reference at offset 0x10.  Mirrors HSDLib's
    /// `HSD_JOBJ.Dobj = …` setter, which clears `SPLINE` and `PTCL` flags
    /// (those bits use the same 0x10 slot as a tagged-union payload).
    /// Pass `None` to detach.
    pub fn set_dobj(&self, dobj: Option<DObj>) -> Result<()> {
        self.ensure_jobj_size();
        self.0
            .borrow_mut()
            .set_reference(0x10, dobj.map(|d| d.0));
        let f = self.flags()?;
        let cleared = f.bits()
            & !(JObjFlag::SPLINE.bits() | JObjFlag::PTCL.bits());
        self.set_flags(JObjFlag::from_bits_retain(cleared))
    }

    /// Allocate a brand-new HSD_JOBJ struct: 0x40 bytes, scale=(1,1,1),
    /// everything else zero (matches HSDLib `new HSD_JOBJ()` post-ctor
    /// state plus the SX/SY/SZ identity init the csx import script does).
    /// Caller is responsible for keeping the resulting `Rc` alive.
    pub fn allocate_default() -> Self {
        let s = HsdStruct::with_capacity(0x40).into_ref();
        let j = JObj::from_struct(s);
        // identity scale; csx hsd_import_from_blender.csx sets these
        // explicitly for newly-allocated joints
        let _ = j.set_sx(1.0);
        let _ = j.set_sy(1.0);
        let _ = j.set_sz(1.0);
        j
    }

    /// Grow the underlying struct to 0x40 bytes if it isn't already.
    /// Idempotent.  Internal helper for the setters above so that a JObj
    /// wrapped around a too-short HsdStruct (HSDLib also allows this in
    /// its constructor path) is auto-promoted on first write.
    fn ensure_jobj_size(&self) {
        let mut s = self.0.borrow_mut();
        if s.len() < 0x40 {
            s.resize(0x40);
        }
    }
}

// =====================================================================
// DObj  (HSDRaw/Common/HSD_DOBJ.cs, TrimmedSize 0x10)
// =====================================================================

accessor!(DObj);

impl DObj {
    pub fn class_name(&self) -> Result<Option<String>> { self.s().get_string(0x00) }
    pub fn next(&self) -> Option<DObj> { self.ref_at::<DObj>(0x04) }
    pub fn mobj(&self) -> Option<MObj> { self.ref_at::<MObj>(0x08) }
    pub fn pobj(&self) -> Option<PObj> { self.ref_at::<PObj>(0x0C) }

    // ----- mutators ------------------------------------------------
    /// Allocate a brand-new HSD_DOBJ: 0x10 bytes, all zero.  Matches
    /// HSDLib `new HSD_DOBJ()` post-ctor state.  Caller is responsible
    /// for keeping the `Rc` alive (typically by passing it through
    /// `JObj::set_dobj`).
    pub fn allocate_default() -> Self {
        DObj::from_struct(HsdStruct::with_capacity(0x10).into_ref())
    }

    pub fn set_next(&self, next: Option<DObj>) {
        self.ensure_dobj_size();
        self.0.borrow_mut().set_reference(0x04, next.map(|d| d.0));
    }

    pub fn set_mobj(&self, mobj: Option<MObj>) {
        self.ensure_dobj_size();
        self.0.borrow_mut().set_reference(0x08, mobj.map(|m| m.0));
    }

    pub fn set_pobj(&self, pobj: Option<PObj>) {
        self.ensure_dobj_size();
        self.0.borrow_mut().set_reference(0x0C, pobj.map(|p| p.0));
    }

    fn ensure_dobj_size(&self) {
        let mut s = self.0.borrow_mut();
        if s.len() < 0x10 {
            s.resize(0x10);
        }
    }
}

// =====================================================================
// MObj  (HSDRaw/Common/HSD_MOBJ.cs, TrimmedSize 0x18)
// =====================================================================

/// Tweakable inputs for [`MObj::allocate_textured_with`].
///
/// Field defaults model a "lit single-texture surface" TEV stage
/// layout — one of the simplest HSD textured-render configurations.
/// Override individual fields when the target runtime expects a
/// different stage layout (multi-texture, projected, decal, etc.)
/// without having to drop down to manual `TObj` construction.
#[derive(Debug, Clone, Copy)]
pub struct TexturedPreset {
    pub render_flags: MaterialRenderMode,
    pub tex_map_id: GxTexMapId,
    pub tex_gen_src: GxTexGenSrc,
    pub scale: [f32; 3],
    pub wrap_s: GxWrapMode,
    pub wrap_t: GxWrapMode,
    pub repeat_s: u8,
    pub repeat_t: u8,
    pub mag_filter: GxTexFilter,
    pub color_op: ColorMap,
    pub alpha_op: AlphaMap,
    pub blending: f32,
    pub lightmap_diffuse: bool,
}

impl Default for TexturedPreset {
    fn default() -> Self {
        Self {
            render_flags: MaterialRenderMode::CONSTANT
                | MaterialRenderMode::DIFFUSE
                | MaterialRenderMode::TEX0
                | MaterialRenderMode::ALPHA_MAT,
            tex_map_id: GxTexMapId::GX_TEXMAP0,
            tex_gen_src: GxTexGenSrc::GX_TG_TEX0,
            scale: [1.0, 1.0, 1.0],
            wrap_s: GxWrapMode::REPEAT,
            wrap_t: GxWrapMode::REPEAT,
            repeat_s: 1,
            repeat_t: 1,
            mag_filter: GxTexFilter::GX_LINEAR,
            color_op: ColorMap::MODULATE,
            alpha_op: AlphaMap::MODULATE,
            blending: 1.0,
            lightmap_diffuse: true,
        }
    }
}

/// Tweakable inputs for [`MObj::allocate_unlit_color_with`].
///
/// Defaults: `render_flags = CONSTANT | DIFFUSE`, `alpha = 1.0`,
/// `shininess = 50.0` (HSDLib's post-`Trim` default).
#[derive(Debug, Clone, Copy)]
pub struct UnlitColorPreset {
    pub render_flags: MaterialRenderMode,
    pub alpha: f32,
    pub shininess: f32,
}

impl Default for UnlitColorPreset {
    fn default() -> Self {
        Self {
            render_flags: MaterialRenderMode::CONSTANT
                | MaterialRenderMode::DIFFUSE,
            alpha: 1.0,
            shininess: 50.0,
        }
    }
}

accessor!(MObj);

impl MObj {
    pub fn class_name(&self) -> Result<Option<String>> { self.s().get_string(0x00) }
    pub fn render_flags(&self) -> Result<MaterialRenderMode> {
        Ok(MaterialRenderMode::from_bits_retain(self.s().get_u32(0x04)?))
    }
    pub fn textures(&self) -> Option<TObj> { self.ref_at::<TObj>(0x08) }
    pub fn material(&self) -> Option<Material> { self.ref_at::<Material>(0x0C) }
    pub fn pe_desc(&self) -> Option<PeDesc> { self.ref_at::<PeDesc>(0x14) }

    // ----- mutators ------------------------------------------------
    /// Allocate a fresh HSD_MOBJ: 0x18 bytes, all-zero fields (no
    /// textures / material / PE attached).  Mirrors HSDLib `new
    /// HSD_MOBJ()` post-ctor state.
    pub fn allocate_default() -> Self {
        MObj::from_struct(HsdStruct::with_capacity(0x18).into_ref())
    }

    /// "Unlit single-color" preset: render flags `CONSTANT |
    /// DIFFUSE`, a fresh `Material` with diffuse RGBA8 = (r, g, b, a),
    /// alpha = 1.0, shininess = 50.0 (HSDLib's default in `Trim`),
    /// no textures, no PE descriptor.  Useful as a placeholder when
    /// the addon doesn't yet have a real material to point at.
    ///
    /// Caveat: this preset only wires render flags + diffuse Material
    /// — it does *not* set the TObj-side `LIGHTMAP_DIFFUSE` flag, so a
    /// TObj attached afterwards won't necessarily participate in the
    /// downstream consumer's lighting pipeline.  Whether that matters
    /// depends on the consumer's TEV stage configuration; if you need
    /// a textured lit preset out of the box, use
    /// [`Self::allocate_textured`] instead.
    pub fn allocate_unlit_color(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::allocate_unlit_color_with(r, g, b, a, UnlitColorPreset::default())
    }

    /// Identical to [`Self::allocate_unlit_color`] but lets the caller
    /// override the render-flag word, the Material alpha, and the
    /// Material shininess via a [`UnlitColorPreset`].
    pub fn allocate_unlit_color_with(
        r: u8,
        g: u8,
        b: u8,
        a: u8,
        preset: UnlitColorPreset,
    ) -> Self {
        let mobj = Self::allocate_default();
        let _ = mobj.set_render_flags(preset.render_flags);
        let mat = Material::allocate_default();
        let _ = mat.set_dif_rgba([r, g, b, a]);
        let _ = mat.set_alpha(preset.alpha);
        let _ = mat.set_shininess(preset.shininess);
        mobj.set_material(Some(mat));
        mobj
    }

    /// "Textured lit" preset: a one-call MObj + TObj wiring path for
    /// the common case of "give me a lit mesh that samples one
    /// texture".  The caller supplies a pre-built [`Material`]
    /// (e.g. via `Material::allocate(amb, dif, spc, alpha,
    /// shininess)`) and a pre-built [`Image`].
    ///
    /// What this builds:
    ///   - `MObj.render_flags` = `CONSTANT | DIFFUSE | TEX0 | ALPHA_MAT`
    ///     — a bit pattern that enables both the diffuse-lighting stage
    ///     and the TEX0 sampler on a typical HSD TEV pipeline.
    ///   - `Material` is attached as-is (caller controls the colors).
    ///   - A fresh `TObj` with: `tex_map_id = GX_TEXMAP0`,
    ///     `tex_gen_src = GX_TG_TEX0`, `coord_type = UV`,
    ///     `color_op = MODULATE`, `alpha_op = MODULATE`, `scale =
    ///     (1, 1, 1)`, `wrap_s = wrap_t = REPEAT`, `repeat_s =
    ///     repeat_t = 1`, `mag_filter = GX_LINEAR`, `blending =
    ///     1.0`, and `LIGHTMAP_DIFFUSE` set.  This is one observed
    ///     runtime convention for "lit single-texture surface" in HSD
    ///     content; callers whose target runtime expects a different
    ///     stage layout (multi-texture, projected, etc.) should build
    ///     the TObj manually instead.
    ///   - `Image` is attached as `TObj.image_data`.
    ///
    /// What this does **not** wire up:
    ///   - PE descriptor (alpha test / blend mode) — caller attaches
    ///     a `PeDesc` separately if needed.
    ///   - TLUT — for paletted image formats the caller has to
    ///     attach `Tlut` separately via `tobj.set_tlut_data(...)`.
    pub fn allocate_textured(material: Material, image: Image) -> Self {
        Self::allocate_textured_with(material, image, TexturedPreset::default())
    }

    /// Identical to [`Self::allocate_textured`] but lets the caller
    /// override individual TObj / render-flag fields via a
    /// [`TexturedPreset`].  Use this when the target runtime expects
    /// a different TEV stage layout (multi-texture, projected,
    /// decal, etc.) than the default course-mesh field pattern.
    pub fn allocate_textured_with(
        material: Material,
        image: Image,
        preset: TexturedPreset,
    ) -> Self {
        let mobj = Self::allocate_default();
        let _ = mobj.set_render_flags(preset.render_flags);
        mobj.set_material(Some(material));

        let tobj = TObj::allocate_default();
        let _ = tobj.set_tex_map_id(preset.tex_map_id);
        let _ = tobj.set_tex_gen_src(preset.tex_gen_src);
        let _ = tobj.set_scale(preset.scale[0], preset.scale[1], preset.scale[2]);
        let _ = tobj.set_wrap_s(preset.wrap_s);
        let _ = tobj.set_wrap_t(preset.wrap_t);
        let _ = tobj.set_repeat_s(preset.repeat_s);
        let _ = tobj.set_repeat_t(preset.repeat_t);
        let _ = tobj.set_blending(preset.blending);
        let _ = tobj.set_mag_filter(preset.mag_filter);
        let _ = tobj.set_color_operation(preset.color_op);
        let _ = tobj.set_alpha_operation(preset.alpha_op);
        let _ = tobj.set_lightmap_diffuse(preset.lightmap_diffuse);
        tobj.set_image_data(Some(image));

        mobj.set_textures(Some(tobj));
        mobj
    }


    pub fn set_render_flags(&self, flags: MaterialRenderMode) -> Result<()> {
        self.ensure_mobj_size();
        self.0.borrow_mut().set_u32(0x04, flags.bits())
    }

    pub fn set_textures(&self, textures: Option<TObj>) {
        self.ensure_mobj_size();
        self.0
            .borrow_mut()
            .set_reference(0x08, textures.map(|t| t.0));
    }

    pub fn set_material(&self, material: Option<Material>) {
        self.ensure_mobj_size();
        self.0
            .borrow_mut()
            .set_reference(0x0C, material.map(|m| m.0));
    }

    pub fn set_pe_desc(&self, pe: Option<PeDesc>) {
        self.ensure_mobj_size();
        self.0.borrow_mut().set_reference(0x14, pe.map(|p| p.0));
    }

    fn ensure_mobj_size(&self) {
        let mut s = self.0.borrow_mut();
        if s.len() < 0x18 {
            s.resize(0x18);
        }
    }
}

// =====================================================================
// Material  (HSDRaw/Common/HSD_MOBJ.cs:101, TrimmedSize 0x14)
// =====================================================================

accessor!(Material);

impl Material {
    /// Ambient color RGBA8 (offset 0x00..0x04).  HSDLib `HSD_Material.AMB_*`
    /// — the `GX_AMB` register feed for the GX hardware lighting stage.
    pub fn amb_rgba(&self) -> Result<[u8; 4]> {
        let s = self.s();
        Ok([s.get_byte(0x00)?, s.get_byte(0x01)?, s.get_byte(0x02)?, s.get_byte(0x03)?])
    }
    /// Diffuse color RGBA8 (offset 0x04..0x08).  HSDLib
    /// `HSD_Material.DIF_*` — the `GX_DIF` register feed.
    pub fn dif_rgba(&self) -> Result<[u8; 4]> {
        let s = self.s();
        Ok([s.get_byte(0x04)?, s.get_byte(0x05)?, s.get_byte(0x06)?, s.get_byte(0x07)?])
    }
    /// Specular color RGBA8 (offset 0x08..0x0C).  HSDLib
    /// `HSD_Material.SPC_*` — the `GX_SPC` register feed.
    pub fn spc_rgba(&self) -> Result<[u8; 4]> {
        let s = self.s();
        Ok([s.get_byte(0x08)?, s.get_byte(0x09)?, s.get_byte(0x0A)?, s.get_byte(0x0B)?])
    }
    /// Material alpha multiplier f32 (offset 0x0C).  Multiplies into
    /// every TEV stage's `RAS` source α.  Sensible value range [0.0, 1.0].
    pub fn alpha(&self) -> Result<f32> { self.s().get_f32(0x0C) }
    /// Specular shininess f32 (offset 0x10).  HSDLib's "Phong cosine"
    /// exponent — higher values produce a tighter highlight.  Sensible
    /// value range roughly [1.0, 200.0].
    pub fn shininess(&self) -> Result<f32> { self.s().get_f32(0x10) }

    // ----- mutators ------------------------------------------------
    /// Allocate a fresh HSD_Material: 0x14 bytes, all-zero fields
    /// (ambient/diffuse/specular = (0,0,0,0), alpha = 0.0, shininess = 0.0).
    /// Mirrors HSDLib `new HSD_Material()` post-ctor state.  Pair with
    /// `set_*_rgba` / `set_alpha` / `set_shininess` for sensible values,
    /// or use [`Self::allocate`] for a one-call named-argument
    /// constructor.
    pub fn allocate_default() -> Self {
        Material::from_struct(HsdStruct::with_capacity(0x14).into_ref())
    }

    /// One-call constructor: allocate + set every field in a single
    /// shot.  Equivalent to `allocate_default()` + 5 setters.  Use for
    /// the common case of building a fresh Material from known values
    /// (e.g. importing from a Blender BSDF).  Field byte-order:
    /// ambient / diffuse / specular each take RGBA8 (4 bytes each
    /// = 12 bytes total at offsets 0x00 / 0x04 / 0x08), then α (f32 at
    /// 0x0C), then shininess (f32 at 0x10).
    pub fn allocate(
        amb: [u8; 4],
        dif: [u8; 4],
        spc: [u8; 4],
        alpha: f32,
        shininess: f32,
    ) -> Result<Self> {
        let mat = Self::allocate_default();
        mat.set_amb_rgba(amb)?;
        mat.set_dif_rgba(dif)?;
        mat.set_spc_rgba(spc)?;
        mat.set_alpha(alpha)?;
        mat.set_shininess(shininess)?;
        Ok(mat)
    }

    pub fn set_amb_rgba(&self, rgba: [u8; 4]) -> Result<()> {
        self.ensure_material_size();
        let mut s = self.0.borrow_mut();
        for i in 0..4 {
            s.data_mut()[i] = rgba[i];
        }
        Ok(())
    }

    pub fn set_dif_rgba(&self, rgba: [u8; 4]) -> Result<()> {
        self.ensure_material_size();
        let mut s = self.0.borrow_mut();
        for i in 0..4 {
            s.data_mut()[0x04 + i] = rgba[i];
        }
        Ok(())
    }

    pub fn set_spc_rgba(&self, rgba: [u8; 4]) -> Result<()> {
        self.ensure_material_size();
        let mut s = self.0.borrow_mut();
        for i in 0..4 {
            s.data_mut()[0x08 + i] = rgba[i];
        }
        Ok(())
    }

    pub fn set_alpha(&self, v: f32) -> Result<()> {
        self.ensure_material_size();
        self.0.borrow_mut().set_f32(0x0C, v)
    }

    pub fn set_shininess(&self, v: f32) -> Result<()> {
        self.ensure_material_size();
        self.0.borrow_mut().set_f32(0x10, v)
    }

    fn ensure_material_size(&self) {
        let mut s = self.0.borrow_mut();
        if s.len() < 0x14 {
            s.resize(0x14);
        }
    }
}

// =====================================================================
// PeDesc  (HSDRaw/Common/HSD_MOBJ.cs:156, TrimmedSize 0xC)
// =====================================================================

accessor!(PeDesc);

impl PeDesc {
    pub fn flags(&self) -> Result<u8> { self.s().get_byte(0x00) }
    pub fn alpha_ref0(&self) -> Result<u8> { self.s().get_byte(0x01) }
    pub fn alpha_ref1(&self) -> Result<u8> { self.s().get_byte(0x02) }
    pub fn destination_alpha(&self) -> Result<u8> { self.s().get_byte(0x03) }
    pub fn blend_mode(&self) -> Result<u8> { self.s().get_byte(0x04) }
    pub fn src_factor(&self) -> Result<u8> { self.s().get_byte(0x05) }
    pub fn dst_factor(&self) -> Result<u8> { self.s().get_byte(0x06) }
    pub fn blend_op(&self) -> Result<u8> { self.s().get_byte(0x07) }
    pub fn depth_function(&self) -> Result<u8> { self.s().get_byte(0x08) }
    pub fn alpha_comp0(&self) -> Result<u8> { self.s().get_byte(0x09) }
    pub fn alpha_op(&self) -> Result<u8> { self.s().get_byte(0x0A) }
    pub fn alpha_comp1(&self) -> Result<u8> { self.s().get_byte(0x0B) }

    /// Allocate a fresh HSD_PEDesc: 0xC bytes, all-zero fields.
    /// Mirrors HSDLib `new HSD_PEDesc()` post-ctor state.  Use the
    /// per-byte setters below to fill in blend mode / depth test / etc.
    pub fn allocate_default() -> Self {
        PeDesc::from_struct(HsdStruct::with_capacity(0xC).into_ref())
    }

    pub fn set_flags(&self, v: u8) -> Result<()> { self.put_byte(0x00, v) }
    pub fn set_alpha_ref0(&self, v: u8) -> Result<()> { self.put_byte(0x01, v) }
    pub fn set_alpha_ref1(&self, v: u8) -> Result<()> { self.put_byte(0x02, v) }
    pub fn set_destination_alpha(&self, v: u8) -> Result<()> { self.put_byte(0x03, v) }
    pub fn set_blend_mode(&self, v: u8) -> Result<()> { self.put_byte(0x04, v) }
    pub fn set_src_factor(&self, v: u8) -> Result<()> { self.put_byte(0x05, v) }
    pub fn set_dst_factor(&self, v: u8) -> Result<()> { self.put_byte(0x06, v) }
    pub fn set_blend_op(&self, v: u8) -> Result<()> { self.put_byte(0x07, v) }
    pub fn set_depth_function(&self, v: u8) -> Result<()> { self.put_byte(0x08, v) }
    pub fn set_alpha_comp0(&self, v: u8) -> Result<()> { self.put_byte(0x09, v) }
    pub fn set_alpha_op(&self, v: u8) -> Result<()> { self.put_byte(0x0A, v) }
    pub fn set_alpha_comp1(&self, v: u8) -> Result<()> { self.put_byte(0x0B, v) }

    fn put_byte(&self, off: usize, v: u8) -> Result<()> {
        let mut s = self.0.borrow_mut();
        if s.len() < 0xC {
            s.resize(0xC);
        }
        s.data_mut()[off] = v;
        Ok(())
    }
}

// =====================================================================
// PObj  (HSDRaw/Common/HSD_POBJ.cs, TrimmedSize 0x18)
// =====================================================================

accessor!(PObj);

impl PObj {
    pub fn class_name(&self) -> Result<Option<String>> { self.s().get_string(0x00) }
    pub fn next(&self) -> Option<PObj> { self.ref_at::<PObj>(0x04) }
    pub fn attributes_struct(&self) -> Option<StructRef> { self.s().get_reference(0x08) }
    pub fn flags(&self) -> Result<PObjFlag> {
        Ok(PObjFlag::from_bits_retain(self.s().get_u16(0x0C)?))
    }
    /// Display list size in bytes (HSDLib stores it in 32-byte units at 0x0E).
    pub fn display_list_size(&self) -> Result<u32> {
        Ok(self.s().get_i16(0x0E)? as u32 * 32)
    }
    pub fn display_list_buffer(&self) -> Option<Vec<u8>> {
        self.s().get_reference(0x10).map(|s| s.borrow().data().to_vec())
    }

    /// The 0x14 slot is a tagged union driven by `Flags`.  Phase 1 only
    /// needs the SingleBoundJOBJ branch; ShapeSet / EnvelopeWeights are
    /// stubbed for now.
    pub fn single_bound_jobj(&self) -> Result<Option<JObj>> {
        let f = self.flags()?;
        if f.intersects(PObjFlag::SHAPESET | PObjFlag::ENVELOPE) {
            return Ok(None);
        }
        Ok(self.ref_at::<JObj>(0x14))
    }

    /// Overwrite the `Flags` u16 at offset 0x0C in place.  Use sparingly
    /// — `MeshBuilder::build` already produces the correct flag value
    /// for the mesh shape it's emitting.  This setter exists for the
    /// case where the caller needs to write a bit pattern that HSDLib's
    /// canonical `POBJ_FLAG` enum doesn't cover cleanly (e.g. game-
    /// specific repurposing of bits otherwise reserved by `POBJ_TYPE_MASK`
    /// or `POBJ_FLAG.ENVELOPE`).  Caller is responsible for keeping the
    /// 0x14 reference consistent with whatever bits they're setting —
    /// e.g. don't toggle `ENVELOPE` on without also wiring a valid
    /// envelope-pointer array at 0x14.
    pub fn set_flags(&self, flags: PObjFlag) -> Result<()> {
        self.0.borrow_mut().set_u16(0x0C, flags.bits())
    }
}

// =====================================================================
// TObj  (HSDRaw/Common/HSD_TOBJ.cs, TrimmedSize 0x5C)
// =====================================================================

accessor!(TObj);

impl TObj {
    pub fn class_name(&self) -> Result<Option<String>> { self.s().get_string(0x00) }
    pub fn next(&self) -> Option<TObj> { self.ref_at::<TObj>(0x04) }
    pub fn tex_map_id(&self) -> Result<GxTexMapId> {
        Ok(GxTexMapId::from(self.s().get_u32(0x08)?))
    }
    /// `GXTexGenSrc` at offset 0x0C (HSDLib `HSD_TOBJ.GXTexGenSrc`).
    /// Selects which vertex source feeds the texture-coord generator:
    /// `GX_TG_TEX0` (4) takes UVs from the POBJ's TEX0 attribute,
    /// `GX_TG_POS` (0) generates them from world position, etc.  The
    /// freshly-allocated default is 0 (`GX_TG_POS`); regular UV-mapped
    /// textures want `GX_TG_TEX0` set explicitly via `set_tex_gen_src`.
    pub fn tex_gen_src(&self) -> Result<GxTexGenSrc> {
        Ok(GxTexGenSrc::from(self.s().get_u32(0x0C)?))
    }
    pub fn rx(&self) -> Result<f32> { self.s().get_f32(0x10) }
    pub fn ry(&self) -> Result<f32> { self.s().get_f32(0x14) }
    pub fn rz(&self) -> Result<f32> { self.s().get_f32(0x18) }
    pub fn sx(&self) -> Result<f32> { self.s().get_f32(0x1C) }
    pub fn sy(&self) -> Result<f32> { self.s().get_f32(0x20) }
    pub fn sz(&self) -> Result<f32> { self.s().get_f32(0x24) }
    pub fn tx(&self) -> Result<f32> { self.s().get_f32(0x28) }
    pub fn ty(&self) -> Result<f32> { self.s().get_f32(0x2C) }
    pub fn tz(&self) -> Result<f32> { self.s().get_f32(0x30) }
    pub fn wrap_s(&self) -> Result<GxWrapMode> {
        Ok(GxWrapMode::from(self.s().get_u32(0x34)?))
    }
    pub fn wrap_t(&self) -> Result<GxWrapMode> {
        Ok(GxWrapMode::from(self.s().get_u32(0x38)?))
    }
    pub fn repeat_s(&self) -> Result<u8> { self.s().get_byte(0x3C) }
    pub fn repeat_t(&self) -> Result<u8> { self.s().get_byte(0x3D) }
    pub fn flags(&self) -> Result<TObjFlags> {
        Ok(TObjFlags::from_bits_retain(self.s().get_u32(0x40)?))
    }
    pub fn coord_type(&self) -> Result<CoordType> {
        Ok(CoordType::from(self.s().get_u32(0x40)? & 0xF))
    }
    pub fn color_operation(&self) -> Result<ColorMap> {
        Ok(ColorMap::from((self.s().get_u32(0x40)? >> 16) & 0xF))
    }
    pub fn alpha_operation(&self) -> Result<AlphaMap> {
        Ok(AlphaMap::from((self.s().get_u32(0x40)? >> 20) & 0xF))
    }
    pub fn blending(&self) -> Result<f32> { self.s().get_f32(0x44) }
    pub fn mag_filter(&self) -> Result<GxTexFilter> {
        Ok(GxTexFilter::from(self.s().get_u32(0x48)?))
    }
    pub fn image_data(&self) -> Option<Image> { self.ref_at::<Image>(0x4C) }
    pub fn tlut_data(&self) -> Option<Tlut> { self.ref_at::<Tlut>(0x50) }
    /// `LOD` reference at offset 0x54 (HSDLib `HSD_TOBJ.LOD` →
    /// `HSD_TOBJ_LOD`).  Carries the per-TObj min-filter / LOD-bias /
    /// anisotropy settings.  When NULL, GX hardware applies the global
    /// defaults (min_filter = `GX_NEAR`, bias = 0, aniso = 1× — which
    /// can be a footprint-averaging surprise on textures with no LOD
    /// specified).  See [`Lod`] for the field accessors.
    pub fn lod_data(&self) -> Option<Lod> { self.ref_at::<Lod>(0x54) }
    /// `TEV` reference at offset 0x58 (HSDLib `HSD_TOBJ.TEV` →
    /// `HSD_TOBJ_TEV`).  Returned as a raw `StructRef` for now — the
    /// TEV struct has 28 bytes of register data that no public consumer
    /// is decoding yet.  Use [`crate::hsd_struct::HsdStruct::set_u8`]
    /// etc. on the returned struct to poke individual fields, or
    /// `get_reference(0x58)` directly.
    pub fn tev_data(&self) -> Option<StructRef> {
        self.s().get_reference(0x58)
    }

    // ----- mutators ------------------------------------------------
    /// Allocate a fresh HSD_TOBJ: 0x5C bytes, all-zero fields (no
    /// image data, no TLUT, identity-zero TRS, wrap=CLAMP, repeat=0,
    /// no flags).  Mirrors HSDLib `new HSD_TOBJ()` post-ctor state.
    /// Pair with the per-field setters below to fill in tex_map_id /
    /// scale / wrap / image_data / etc. as needed.
    pub fn allocate_default() -> Self {
        TObj::from_struct(HsdStruct::with_capacity(0x5C).into_ref())
    }

    pub fn set_next(&self, next: Option<TObj>) {
        self.ensure_tobj_size();
        self.0.borrow_mut().set_reference(0x04, next.map(|t| t.0));
    }

    pub fn set_tex_map_id(&self, id: GxTexMapId) -> Result<()> {
        self.ensure_tobj_size();
        self.0.borrow_mut().set_u32(0x08, u32::from(id))
    }

    /// Set the `GXTexGenSrc` (offset 0x0C).  HSDLib equivalent:
    /// `tobj.GXTexGenSrc = GX_TG_TEX0`.  Default after
    /// `allocate_default` is `GX_TG_POS` (=0) — the all-zero-bytes
    /// initial state — so callers building a fresh TObj for a
    /// UV-mapped texture should set this to `GX_TG_TEX0` explicitly.
    pub fn set_tex_gen_src(&self, src: GxTexGenSrc) -> Result<()> {
        self.ensure_tobj_size();
        self.0.borrow_mut().set_u32(0x0C, u32::from(src))
    }

    /// Set rotation triple at (0x10, 0x14, 0x18).
    pub fn set_rotation(&self, rx: f32, ry: f32, rz: f32) -> Result<()> {
        self.ensure_tobj_size();
        let mut s = self.0.borrow_mut();
        s.set_f32(0x10, rx)?;
        s.set_f32(0x14, ry)?;
        s.set_f32(0x18, rz)
    }

    /// Set scale triple at (0x1C, 0x20, 0x24).
    pub fn set_scale(&self, sx: f32, sy: f32, sz: f32) -> Result<()> {
        self.ensure_tobj_size();
        let mut s = self.0.borrow_mut();
        s.set_f32(0x1C, sx)?;
        s.set_f32(0x20, sy)?;
        s.set_f32(0x24, sz)
    }

    /// Set translation triple at (0x28, 0x2C, 0x30).
    pub fn set_translation(&self, tx: f32, ty: f32, tz: f32) -> Result<()> {
        self.ensure_tobj_size();
        let mut s = self.0.borrow_mut();
        s.set_f32(0x28, tx)?;
        s.set_f32(0x2C, ty)?;
        s.set_f32(0x30, tz)
    }

    pub fn set_wrap_s(&self, w: GxWrapMode) -> Result<()> {
        self.ensure_tobj_size();
        self.0.borrow_mut().set_u32(0x34, u32::from(w))
    }

    pub fn set_wrap_t(&self, w: GxWrapMode) -> Result<()> {
        self.ensure_tobj_size();
        self.0.borrow_mut().set_u32(0x38, u32::from(w))
    }

    pub fn set_repeat_s(&self, v: u8) -> Result<()> {
        self.ensure_tobj_size();
        self.0.borrow_mut().data_mut()[0x3C] = v;
        Ok(())
    }

    pub fn set_repeat_t(&self, v: u8) -> Result<()> {
        self.ensure_tobj_size();
        self.0.borrow_mut().data_mut()[0x3D] = v;
        Ok(())
    }

    /// Set the flags u32 at offset 0x40.  HSDLib packs `coord_type`
    /// (low 4 bits), `color_operation` (bits 16-19) and
    /// `alpha_operation` (bits 20-23) into the same word; if you want
    /// to set those without clobbering the base flags use
    /// `set_coord_type` / `set_color_operation` / `set_alpha_operation`,
    /// which read-modify-write the relevant nibble in place.  For
    /// individual `TObjFlags` bits use the named `set_lightmap_*` /
    /// `set_bump` setters below — they RMW just one bit and preserve
    /// the nibbles + every other flag.
    pub fn set_flags(&self, flags: TObjFlags) -> Result<()> {
        self.ensure_tobj_size();
        self.0.borrow_mut().set_u32(0x40, flags.bits())
    }

    /// RMW a single `TObjFlags` bit at offset 0x40, preserving every
    /// other bit (including the coord_type / color_op / alpha_op
    /// nibbles).  Foundation for the named `set_lightmap_*` /
    /// `set_bump` setters; exposed in case callers want to toggle a
    /// custom mask.
    pub fn set_flag_bit(&self, mask: TObjFlags, on: bool) -> Result<()> {
        self.ensure_tobj_size();
        let mut s = self.0.borrow_mut();
        let old = s.get_u32(0x40)?;
        let new = if on {
            old | mask.bits()
        } else {
            old & !mask.bits()
        };
        s.set_u32(0x40, new)
    }

    /// `LIGHTMAP_DIFFUSE` (bit 4).  HSDLib's `LIGHTMAP_DIFFUSE` flag —
    /// signals that this texture participates in the diffuse lighting
    /// stage.  Some shipped renderers skip texture sampling entirely
    /// for TObjs without this bit, even when render-flags / TEX0 /
    /// TObj-ref are wired up correctly, so it's worth setting on
    /// freshly-allocated TObjs unless you specifically don't want the
    /// texture to influence the diffuse channel.
    pub fn set_lightmap_diffuse(&self, on: bool) -> Result<()> {
        self.set_flag_bit(TObjFlags::LIGHTMAP_DIFFUSE, on)
    }
    pub fn is_lightmap_diffuse(&self) -> Result<bool> {
        Ok(self.flags()?.intersects(TObjFlags::LIGHTMAP_DIFFUSE))
    }

    /// `LIGHTMAP_SPECULAR` (bit 5).
    pub fn set_lightmap_specular(&self, on: bool) -> Result<()> {
        self.set_flag_bit(TObjFlags::LIGHTMAP_SPECULAR, on)
    }
    pub fn is_lightmap_specular(&self) -> Result<bool> {
        Ok(self.flags()?.intersects(TObjFlags::LIGHTMAP_SPECULAR))
    }

    /// `LIGHTMAP_AMBIENT` (bit 6).
    pub fn set_lightmap_ambient(&self, on: bool) -> Result<()> {
        self.set_flag_bit(TObjFlags::LIGHTMAP_AMBIENT, on)
    }
    pub fn is_lightmap_ambient(&self) -> Result<bool> {
        Ok(self.flags()?.intersects(TObjFlags::LIGHTMAP_AMBIENT))
    }

    /// `LIGHTMAP_EXT` (bit 7).
    pub fn set_lightmap_ext(&self, on: bool) -> Result<()> {
        self.set_flag_bit(TObjFlags::LIGHTMAP_EXT, on)
    }
    pub fn is_lightmap_ext(&self) -> Result<bool> {
        Ok(self.flags()?.intersects(TObjFlags::LIGHTMAP_EXT))
    }

    /// `LIGHTMAP_SHADOW` (bit 8).
    pub fn set_lightmap_shadow(&self, on: bool) -> Result<()> {
        self.set_flag_bit(TObjFlags::LIGHTMAP_SHADOW, on)
    }
    pub fn is_lightmap_shadow(&self) -> Result<bool> {
        Ok(self.flags()?.intersects(TObjFlags::LIGHTMAP_SHADOW))
    }

    /// `BUMP` (bit 24).
    pub fn set_bump(&self, on: bool) -> Result<()> {
        self.set_flag_bit(TObjFlags::BUMP, on)
    }
    pub fn is_bump(&self) -> Result<bool> {
        Ok(self.flags()?.intersects(TObjFlags::BUMP))
    }

    /// Replace the low 4 bits of the 0x40 word with `coord` (the GX
    /// `_GX_TG_TYPE` value).  Other bits in the word are preserved.
    pub fn set_coord_type(&self, coord: CoordType) -> Result<()> {
        self.modify_flag_nibble(0, u32::from(coord))
    }

    /// Replace bits 16-19 of the 0x40 word with `op`.  Other bits are
    /// preserved.
    pub fn set_color_operation(&self, op: ColorMap) -> Result<()> {
        self.modify_flag_nibble(16, u32::from(op))
    }

    /// Replace bits 20-23 of the 0x40 word with `op`.  Other bits are
    /// preserved.
    pub fn set_alpha_operation(&self, op: AlphaMap) -> Result<()> {
        self.modify_flag_nibble(20, u32::from(op))
    }

    fn modify_flag_nibble(&self, shift: u32, val: u32) -> Result<()> {
        self.ensure_tobj_size();
        let mut s = self.0.borrow_mut();
        let old = s.get_u32(0x40)?;
        let mask = !(0xF_u32 << shift);
        let new = (old & mask) | ((val & 0xF) << shift);
        s.set_u32(0x40, new)
    }

    pub fn set_blending(&self, v: f32) -> Result<()> {
        self.ensure_tobj_size();
        self.0.borrow_mut().set_f32(0x44, v)
    }

    pub fn set_mag_filter(&self, f: GxTexFilter) -> Result<()> {
        self.ensure_tobj_size();
        self.0.borrow_mut().set_u32(0x48, u32::from(f))
    }

    /// Attach (or detach) the `Image` reference at offset 0x4C.
    /// HSDLib: `tobj.ImageData = HSD_Image`.
    pub fn set_image_data(&self, img: Option<Image>) {
        self.ensure_tobj_size();
        self.0
            .borrow_mut()
            .set_reference(0x4C, img.map(|i| i.0));
    }

    /// Attach (or detach) the `Tlut` reference at offset 0x50.  HSDLib:
    /// `tobj.TLUTData = HSD_Tlut`.
    pub fn set_tlut_data(&self, tlut: Option<Tlut>) {
        self.ensure_tobj_size();
        self.0
            .borrow_mut()
            .set_reference(0x50, tlut.map(|t| t.0));
    }

    /// Attach (or detach) the `Lod` reference at offset 0x54.  HSDLib:
    /// `tobj.LOD = HSD_TOBJ_LOD`.  When NULL, GX hardware picks the
    /// global default min_filter / lod_bias / aniso — for textures
    /// that don't ship with a LOD struct, the runtime defaults can
    /// produce a footprint-averaging look on minified texels.  Set
    /// an explicit Lod with `Lod::allocate_default` + per-field
    /// setters when that matters.
    pub fn set_lod_data(&self, lod: Option<Lod>) {
        self.ensure_tobj_size();
        self.0
            .borrow_mut()
            .set_reference(0x54, lod.map(|l| l.0));
    }

    fn ensure_tobj_size(&self) {
        let mut s = self.0.borrow_mut();
        if s.len() < 0x5C {
            s.resize(0x5C);
        }
    }
}

// =====================================================================
// Image  (HSDRaw/Common/HSD_TOBJ.cs:341, TrimmedSize 0x18)
// =====================================================================

accessor!(Image);

impl Image {
    pub fn image_data(&self) -> Option<Vec<u8>> {
        self.s().get_reference(0x00).map(|s| s.borrow().data().to_vec())
    }
    /// Underlying buffer struct (= the raw GX-encoded bytes payload).
    /// Returned as `StructRef` since this is a leaf buffer with no
    /// outgoing references; useful when callers need to share the same
    /// payload across multiple Images (writer buffer dedup picks it up
    /// either way, but identity-share keeps the parse-side `Rc::ptr_eq`
    /// invariant intact).
    pub fn image_data_struct(&self) -> Option<StructRef> {
        self.s().get_reference(0x00)
    }
    pub fn width(&self) -> Result<i16> { self.s().get_i16(0x04) }
    pub fn height(&self) -> Result<i16> { self.s().get_i16(0x06) }
    pub fn format(&self) -> Result<GxTexFmt> {
        Ok(GxTexFmt::from(self.s().get_u32(0x08)?))
    }
    pub fn mipmap(&self) -> Result<i32> { self.s().get_i32(0x0C) }
    pub fn min_lod(&self) -> Result<f32> { self.s().get_f32(0x10) }
    pub fn max_lod(&self) -> Result<f32> { self.s().get_f32(0x14) }

    // ----- mutators ------------------------------------------------
    /// Allocate a fresh HSD_Image: 0x18 bytes, all-zero fields (no raw
    /// bytes attached, width/height = 0, format = I4 (=0)).  Mirrors
    /// HSDLib `new HSD_Image()` post-ctor state.  Pair with
    /// `set_image_data` + `set_width` + `set_height` + `set_format` to
    /// fill in the actual texture payload.
    pub fn allocate_default() -> Self {
        Image::from_struct(HsdStruct::with_capacity(0x18).into_ref())
    }

    /// Wrap raw GX-encoded bytes (typically from `gx_image::encode_image`)
    /// in a fresh leaf buffer struct and attach it at offset 0.  Marks
    /// the buffer as 0x20-aligned (writer convention for texture data).
    /// HSDLib equivalent: `image.ImageData = HSDStruct(rawBytes)`
    /// followed by the `IsBufferAligned` flag pull.
    pub fn set_image_data_bytes(&self, bytes: Vec<u8>) {
        self.ensure_image_size();
        let buf = HsdStruct::from_bytes(bytes).into_ref();
        // Texture payloads must land on a 0x20 boundary at write time,
        // matching HSDLib's `_isBufferAligned` semantics.
        buf.borrow_mut().is_buffer_aligned = true;
        self.0.borrow_mut().set_reference(0x00, Some(buf));
    }

    /// Replace (or detach) the raw-bytes ref at offset 0 with an
    /// existing `StructRef`.  Useful for sharing one image_data payload
    /// across multiple Images (e.g. mipmap chain re-use, multi-TObj
    /// aliasing).  Pass `None` to detach.
    pub fn set_image_data_struct(&self, buf: Option<StructRef>) {
        self.ensure_image_size();
        self.0.borrow_mut().set_reference(0x00, buf);
    }

    pub fn set_width(&self, w: i16) -> Result<()> {
        self.ensure_image_size();
        self.0.borrow_mut().set_i16(0x04, w)
    }

    pub fn set_height(&self, h: i16) -> Result<()> {
        self.ensure_image_size();
        self.0.borrow_mut().set_i16(0x06, h)
    }

    pub fn set_format(&self, f: GxTexFmt) -> Result<()> {
        self.ensure_image_size();
        self.0.borrow_mut().set_u32(0x08, u32::from(f))
    }

    pub fn set_mipmap(&self, v: i32) -> Result<()> {
        self.ensure_image_size();
        self.0.borrow_mut().set_i32(0x0C, v)
    }

    pub fn set_min_lod(&self, v: f32) -> Result<()> {
        self.ensure_image_size();
        self.0.borrow_mut().set_f32(0x10, v)
    }

    pub fn set_max_lod(&self, v: f32) -> Result<()> {
        self.ensure_image_size();
        self.0.borrow_mut().set_f32(0x14, v)
    }

    fn ensure_image_size(&self) {
        let mut s = self.0.borrow_mut();
        if s.len() < 0x18 {
            s.resize(0x18);
        }
    }
}

// =====================================================================
// Tlut  (HSDRaw/Common/HSD_TOBJ.cs:368, TrimmedSize 0x20)
// =====================================================================

accessor!(Tlut);

impl Tlut {
    pub fn tlut_data(&self) -> Option<Vec<u8>> {
        self.s().get_reference(0x00).map(|s| s.borrow().data().to_vec())
    }
    pub fn format(&self) -> Result<GxTlutFmt> {
        Ok(GxTlutFmt::from(self.s().get_u32(0x04)?))
    }
    pub fn gx_tlut(&self) -> Result<i32> { self.s().get_i32(0x08) }
    pub fn color_count(&self) -> Result<i16> { self.s().get_i16(0x0C) }
}

// =====================================================================
// Lod  (HSDRaw/Common/HSD_TOBJ.cs:389 `HSD_TOBJ_LOD`, TrimmedSize 0x10)
// =====================================================================
//
// Per-TObj min-filter / LOD-bias / anisotropy settings.  Layout (BE):
//   0x00 i32 MinFilter         (`GXTexFilter` enum)
//   0x04 f32 Bias              (mip LOD bias)
//   0x08 u8  BiasClamp         (bool)
//   0x09 u8  EnableEdgeLOD     (bool)
//   0x0A i32 Anisotropy        (`GXAnisotropy`; **byte-unaligned!** —
//                               HSDLib uses `_s.GetInt32(0x0A)` so the
//                               i32 occupies bytes 0x0A..=0x0D)
//   0x0E .. 0x0F  padding to TrimmedSize 0x10

accessor!(Lod);

impl Lod {
    pub fn min_filter(&self) -> Result<GxTexFilter> {
        Ok(GxTexFilter::from(self.s().get_u32(0x00)?))
    }
    pub fn bias(&self) -> Result<f32> { self.s().get_f32(0x04) }
    pub fn bias_clamp(&self) -> Result<bool> {
        Ok(self.s().get_byte(0x08)? == 1)
    }
    pub fn enable_edge_lod(&self) -> Result<bool> {
        Ok(self.s().get_byte(0x09)? == 1)
    }
    /// `Anisotropy` at the (byte-unaligned) i32 offset 0x0A.  Mirrors
    /// HSDLib's `_s.GetInt32(0x0A)` — bytes 0x0A..=0x0D form the BE i32.
    pub fn anisotropy(&self) -> Result<GxAnisotropy> {
        Ok(GxAnisotropy::from(self.s().get_u32(0x0A)?))
    }

    // ----- mutators ------------------------------------------------
    /// Allocate a fresh HSD_TOBJ_LOD (0x10 bytes, all-zero — i.e.
    /// `MinFilter = GX_NEAR`, `Bias = 0`, `BiasClamp = false`,
    /// `EnableEdgeLOD = false`, `Anisotropy = GX_ANISO_1`).  Pair with
    /// the per-field setters below; attach to a TObj via
    /// [`TObj::set_lod_data`].
    pub fn allocate_default() -> Self {
        Lod::from_struct(HsdStruct::with_capacity(0x10).into_ref())
    }

    pub fn set_min_filter(&self, f: GxTexFilter) -> Result<()> {
        self.ensure_lod_size();
        self.0.borrow_mut().set_u32(0x00, u32::from(f))
    }

    pub fn set_bias(&self, v: f32) -> Result<()> {
        self.ensure_lod_size();
        self.0.borrow_mut().set_f32(0x04, v)
    }

    pub fn set_bias_clamp(&self, on: bool) -> Result<()> {
        self.ensure_lod_size();
        self.0.borrow_mut().set_u8(0x08, if on { 1 } else { 0 })
    }

    pub fn set_enable_edge_lod(&self, on: bool) -> Result<()> {
        self.ensure_lod_size();
        self.0.borrow_mut().set_u8(0x09, if on { 1 } else { 0 })
    }

    /// Set `Anisotropy` at the byte-unaligned i32 offset 0x0A.  Mirrors
    /// HSDLib `_s.SetInt32(0x0A, …)` — writes bytes 0x0A..=0x0D BE.
    pub fn set_anisotropy(&self, a: GxAnisotropy) -> Result<()> {
        self.ensure_lod_size();
        self.0.borrow_mut().set_u32(0x0A, u32::from(a))
    }

    fn ensure_lod_size(&self) {
        let mut s = self.0.borrow_mut();
        if s.len() < 0x10 {
            s.resize(0x10);
        }
    }
}

// =====================================================================
// SObj  (HSDRaw/Common/HSD_SOBJ.cs:19, TrimmedSize 0x10)
// =====================================================================

accessor!(SObj);

impl SObj {
    /// `JOBJDescs` is a `HSDNullPointerArrayAccessor<HSD_JOBJDesc>`: the slot
    /// at 0x00 references an inline array of refs terminated by a NULL ptr.
    /// Length is determined by counting non-NULL entries.
    pub fn jobj_descs(&self) -> Vec<JObjDesc> {
        let Some(arr) = self.s().get_reference(0x00) else {
            return Vec::new();
        };
        let arr_borrow = arr.borrow();
        let mut out = Vec::new();
        let mut i = 0u32;
        loop {
            // Each slot is a 4-byte pointer to a HSD_JOBJDesc struct (or 0 to
            // terminate).  In our model the references map holds the resolved
            // child; absence at offset i*4 ends the array.
            match arr_borrow.get_reference(i * 4) {
                Some(child) => out.push(JObjDesc(child)),
                None => break,
            }
            i += 1;
        }
        out
    }

    /// The raw `JOBJDescs[]` array struct (the HSDLib
    /// `HSDNullPointerArrayAccessor<HSD_JOBJDesc>` payload).  Returned as a
    /// `StructRef` because it has no typed accessor — callers operate on it
    /// via `HsdStruct::set_reference(i*4, …)` to slot in additional
    /// JObjDescs.  `None` if no array is attached yet.
    pub fn jobj_descs_array(&self) -> Option<StructRef> {
        self.s().get_reference(0x00)
    }

    // ----- mutators ------------------------------------------------
    /// Allocate a fresh HSD_SOBJ: 0x10 bytes, all-zero fields (no
    /// JOBJDescs[] attached, no cameras / lights / fog).  Mirrors HSDLib
    /// `new HSD_SOBJ()` post-ctor state.  Pair with `set_jobj_descs_array`
    /// to attach a populated descriptor array.
    pub fn allocate_default() -> Self {
        SObj::from_struct(HsdStruct::with_capacity(0x10).into_ref())
    }

    /// Attach (or detach) the `JOBJDescs[]` array struct at offset 0.
    /// HSDLib equivalent: `sobj.JOBJDescs = HSDNullPointerArrayAccessor<…>`
    /// — we just take the underlying array struct directly since the
    /// "array accessor" is a tagged view, not a separate allocation.
    pub fn set_jobj_descs_array(&self, arr: Option<StructRef>) {
        self.ensure_sobj_size();
        self.0.borrow_mut().set_reference(0x00, arr);
    }

    fn ensure_sobj_size(&self) {
        let mut s = self.0.borrow_mut();
        if s.len() < 0x10 {
            s.resize(0x10);
        }
    }
}

/// Build a `JOBJDescs[]` array struct holding `descs` in order, NULL-
/// terminated (HSDLib `HSDNullPointerArrayAccessor.Save` convention).
///
/// Layout: each entry is a 4-byte pointer; the `n`-entry array needs at
/// least `4 * (n + 1)` bytes (the trailing 4-byte slot stays at zero so
/// the reader stops there).  The *zero-entry* case still produces a 4-
/// byte struct rather than 0 bytes — keeps the round-trip happy.
pub fn build_jobj_descs_array(descs: &[JObjDesc]) -> StructRef {
    let n = descs.len();
    let cap = 4 * (n + 1);
    let arr = HsdStruct::with_capacity(cap).into_ref();
    {
        let mut a = arr.borrow_mut();
        for (i, d) in descs.iter().enumerate() {
            a.set_reference((i * 4) as u32, Some(d.0.clone()));
        }
    }
    arr
}

// =====================================================================
// JObjDesc  (HSDRaw/Common/HSD_SOBJ.cs:35, TrimmedSize 0x10)
// =====================================================================

accessor!(JObjDesc);

impl JObjDesc {
    pub fn root_joint(&self) -> Option<JObj> {
        self.ref_at::<JObj>(0x00)
    }
    // Anim slots (0x04..0x0C) intentionally not exposed yet.

    // ----- mutators ------------------------------------------------
    /// Allocate a fresh HSD_JOBJDesc: 0x10 bytes, all-zero fields (no
    /// root joint, no anim slots).  Mirrors HSDLib `new HSD_JOBJDesc()`
    /// post-ctor state.  Pair with `set_root_joint` to attach a joint.
    pub fn allocate_default() -> Self {
        JObjDesc::from_struct(HsdStruct::with_capacity(0x10).into_ref())
    }

    /// Attach (or detach) the root joint at offset 0.  HSDLib:
    /// `desc.RootJoint = j`.  The anim slots (0x04..0x0C) are left
    /// untouched.
    pub fn set_root_joint(&self, j: Option<JObj>) {
        self.ensure_jobj_desc_size();
        self.0.borrow_mut().set_reference(0x00, j.map(|jj| jj.0));
    }

    fn ensure_jobj_desc_size(&self) {
        let mut s = self.0.borrow_mut();
        if s.len() < 0x10 {
            s.resize(0x10);
        }
    }
}
