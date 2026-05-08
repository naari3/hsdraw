//! High-level accessors mapping to `HSDRaw/Common/HSD_*.cs`.  Phase 1 carries
//! only the read-side fields needed for the JObj tree dump and the future
//! Blender JSON export; field names match HSDLib exactly so cross-referencing
//! the C# source is mechanical.

use crate::accessor::{Accessor, accessor};
use crate::error::Result;
use crate::gx::{GxTexFmt, GxTexMapId, GxTexFilter, GxWrapMode, JObjFlag, MaterialRenderMode, PObjFlag, TObjFlags, AlphaMap, ColorMap, CoordType, GxTlutFmt};
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
    pub fn allocate_unlit_color(r: u8, g: u8, b: u8, a: u8) -> Self {
        let mobj = Self::allocate_default();
        let _ = mobj.set_render_flags(
            MaterialRenderMode::CONSTANT | MaterialRenderMode::DIFFUSE,
        );
        let mat = Material::allocate_default();
        let _ = mat.set_dif_rgba([r, g, b, a]);
        let _ = mat.set_alpha(1.0);
        let _ = mat.set_shininess(50.0);
        mobj.set_material(Some(mat));
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
    pub fn amb_rgba(&self) -> Result<[u8; 4]> {
        let s = self.s();
        Ok([s.get_byte(0x00)?, s.get_byte(0x01)?, s.get_byte(0x02)?, s.get_byte(0x03)?])
    }
    pub fn dif_rgba(&self) -> Result<[u8; 4]> {
        let s = self.s();
        Ok([s.get_byte(0x04)?, s.get_byte(0x05)?, s.get_byte(0x06)?, s.get_byte(0x07)?])
    }
    pub fn spc_rgba(&self) -> Result<[u8; 4]> {
        let s = self.s();
        Ok([s.get_byte(0x08)?, s.get_byte(0x09)?, s.get_byte(0x0A)?, s.get_byte(0x0B)?])
    }
    pub fn alpha(&self) -> Result<f32> { self.s().get_f32(0x0C) }
    pub fn shininess(&self) -> Result<f32> { self.s().get_f32(0x10) }

    // ----- mutators ------------------------------------------------
    /// Allocate a fresh HSD_Material: 0x14 bytes, all-zero fields
    /// (ambient/diffuse/specular = (0,0,0,0), alpha = 0.0, shininess = 0.0).
    /// Mirrors HSDLib `new HSD_Material()` post-ctor state.  Pair with
    /// `set_*_rgba` / `set_alpha` / `set_shininess` for sensible values.
    pub fn allocate_default() -> Self {
        Material::from_struct(HsdStruct::with_capacity(0x14).into_ref())
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
}

// =====================================================================
// Image  (HSDRaw/Common/HSD_TOBJ.cs:341, TrimmedSize 0x18)
// =====================================================================

accessor!(Image);

impl Image {
    pub fn image_data(&self) -> Option<Vec<u8>> {
        self.s().get_reference(0x00).map(|s| s.borrow().data().to_vec())
    }
    pub fn width(&self) -> Result<i16> { self.s().get_i16(0x04) }
    pub fn height(&self) -> Result<i16> { self.s().get_i16(0x06) }
    pub fn format(&self) -> Result<GxTexFmt> {
        Ok(GxTexFmt::from(self.s().get_u32(0x08)?))
    }
    pub fn mipmap(&self) -> Result<i32> { self.s().get_i32(0x0C) }
    pub fn min_lod(&self) -> Result<f32> { self.s().get_f32(0x10) }
    pub fn max_lod(&self) -> Result<f32> { self.s().get_f32(0x14) }
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
}
