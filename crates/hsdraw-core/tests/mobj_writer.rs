//! Round-trip tests for MObj / Material / PeDesc / TObj / Image
//! allocation primitives.  Each test drives parse → mutate → write →
//! re-parse and asserts the expected delta on the rebuilt tree.

use hsdraw_core::accessor::Accessor;
use hsdraw_core::common::{
    DObj, Image, JObj, Lod, MObj, Material, PeDesc, SObj, TObj,
};
use hsdraw_core::dat::{Dat, RootNode};
use hsdraw_core::gx::{
    AlphaMap, ColorMap, CoordType, GxAnisotropy, GxTexFilter, GxTexFmt, GxTexGenSrc, GxTexMapId,
    GxWrapMode, MaterialRenderMode, TObjFlags,
};
use hsdraw_core::hsd_struct::HsdStruct;

/// Wire a MObj into a synthetic Dat the same way the POBJ-writer
/// fixture does: scene_data → JOBJDescs[] → JObjDesc → JObj → DObj
/// → MObj.  The DObj has no POBJ in this fixture (this file only
/// exercises the material side).
fn dat_with_mobj(mobj: MObj) -> Dat {
    let dobj = DObj::allocate_default();
    dobj.set_mobj(Some(mobj));

    let jobj = JObj::allocate_default();
    jobj.set_dobj(Some(dobj.clone())).expect("set_dobj");

    let jobj_desc = HsdStruct::with_capacity(0x10).into_ref();
    jobj_desc
        .borrow_mut()
        .set_reference(0x00, Some(jobj.0.clone()));

    let descs_arr = HsdStruct::with_capacity(0x08).into_ref();
    descs_arr
        .borrow_mut()
        .set_reference(0x00, Some(jobj_desc));

    let sobj = HsdStruct::with_capacity(0x10).into_ref();
    sobj.borrow_mut().set_reference(0x00, Some(descs_arr));

    Dat {
        version: [0; 4],
        roots: vec![RootNode { name: "scene_data".into(), data: sobj }],
        references: vec![],
        struct_order: vec![],
    }
}

/// Walk the synthetic tree and pull out the first MObj.
fn extract_first_mobj(dat: &Dat) -> MObj {
    let scene = dat.scene_data().expect("scene_data root");
    let sobj = SObj::from_struct(scene.data.clone());
    let descs = sobj.jobj_descs();
    let root_joint = descs[0].root_joint().expect("root joint");
    let dobj = root_joint
        .dobj()
        .expect("flags allow Dobj")
        .expect("DObj attached");
    dobj.mobj().expect("MObj attached")
}

fn round_trip(dat: Dat) -> Dat {
    Dat::parse(&dat.write().expect("write")).expect("parse")
}

#[test]
fn mobj_default_is_0x18_bytes() {
    let mobj = MObj::allocate_default();
    assert_eq!(mobj.0.borrow().len(), 0x18);
    // All fields zero — render flags = 0, no refs.
    assert_eq!(mobj.render_flags().unwrap().bits(), 0);
    assert!(mobj.material().is_none());
    assert!(mobj.textures().is_none());
    assert!(mobj.pe_desc().is_none());
}

#[test]
fn material_default_is_0x14_bytes_zero() {
    let m = Material::allocate_default();
    assert_eq!(m.0.borrow().len(), 0x14);
    assert_eq!(m.amb_rgba().unwrap(), [0; 4]);
    assert_eq!(m.dif_rgba().unwrap(), [0; 4]);
    assert_eq!(m.spc_rgba().unwrap(), [0; 4]);
    assert_eq!(m.alpha().unwrap(), 0.0);
    assert_eq!(m.shininess().unwrap(), 0.0);
}

#[test]
fn pedesc_default_is_0xc_bytes_zero() {
    let p = PeDesc::allocate_default();
    assert_eq!(p.0.borrow().len(), 0xC);
    assert_eq!(p.flags().unwrap(), 0);
    assert_eq!(p.blend_mode().unwrap(), 0);
}

#[test]
fn unlit_color_preset_round_trips() {
    // Build an unlit single-color MObj (red) and round-trip it.
    let mobj = MObj::allocate_unlit_color(0xFF, 0x00, 0x00, 0xFF);

    // Pre-write: render flags = CONSTANT|DIFFUSE, material attached.
    let flags = mobj.render_flags().unwrap();
    assert!(flags.intersects(MaterialRenderMode::CONSTANT));
    assert!(flags.intersects(MaterialRenderMode::DIFFUSE));
    assert!(mobj.material().is_some());
    {
        let mat = mobj.material().unwrap();
        assert_eq!(mat.dif_rgba().unwrap(), [0xFF, 0x00, 0x00, 0xFF]);
        assert!((mat.alpha().unwrap() - 1.0).abs() < 1e-6);
        assert!((mat.shininess().unwrap() - 50.0).abs() < 1e-6);
    }

    let dat = round_trip(dat_with_mobj(mobj));
    let mobj2 = extract_first_mobj(&dat);
    let flags2 = mobj2.render_flags().unwrap();
    assert!(flags2.intersects(MaterialRenderMode::CONSTANT));
    assert!(flags2.intersects(MaterialRenderMode::DIFFUSE));

    let mat2 = mobj2.material().expect("material survives round-trip");
    assert_eq!(mat2.dif_rgba().unwrap(), [0xFF, 0x00, 0x00, 0xFF]);
    assert!((mat2.alpha().unwrap() - 1.0).abs() < 1e-6);
    assert!((mat2.shininess().unwrap() - 50.0).abs() < 1e-6);
}

// =====================================================================
// TObj / Image allocator + setters
// =====================================================================

#[test]
fn tobj_default_is_0x5c_bytes_zero() {
    let t = TObj::allocate_default();
    assert_eq!(t.0.borrow().len(), 0x5C);
    // All numeric fields zero, no image_data / tlut.
    assert_eq!(t.tex_map_id().unwrap(), GxTexMapId::GX_TEXMAP0);
    assert_eq!(t.rx().unwrap(), 0.0);
    assert_eq!(t.flags().unwrap().bits(), 0);
    assert_eq!(t.coord_type().unwrap(), CoordType::UV);
    assert!(t.image_data().is_none());
    assert!(t.tlut_data().is_none());
}

#[test]
fn image_default_is_0x18_bytes_zero() {
    let i = Image::allocate_default();
    assert_eq!(i.0.borrow().len(), 0x18);
    assert_eq!(i.width().unwrap(), 0);
    assert_eq!(i.height().unwrap(), 0);
    // GxTexFmt::I4 = 0 — that's what `from(0)` returns.
    assert_eq!(i.format().unwrap(), GxTexFmt::I4);
    assert_eq!(i.mipmap().unwrap(), 0);
    assert!(i.image_data().is_none());
}

#[test]
fn tobj_flag_nibble_setters_preserve_other_bits() {
    let t = TObj::allocate_default();
    t.set_flags(TObjFlags::BUMP).unwrap();          // bit 24
    t.set_coord_type(CoordType::REFLECTION).unwrap(); // low 4 bits → 1
    t.set_color_operation(ColorMap::BLEND).unwrap();   // bits 16-19 → 3
    t.set_alpha_operation(AlphaMap::MODULATE).unwrap(); // bits 20-23 → 3

    let raw = t.0.borrow().get_u32(0x40).unwrap();
    // BUMP (1<<24) | (CoordType::REFLECTION = 1) | (BLEND << 16 = 3<<16)
    // | (MODULATE << 20 = 3<<20)
    let expected = (1u32 << 24) | 1u32 | (3u32 << 16) | (3u32 << 20);
    assert_eq!(raw, expected);

    // Re-readable through the typed accessors.
    assert_eq!(t.coord_type().unwrap(), CoordType::REFLECTION);
    assert_eq!(t.color_operation().unwrap(), ColorMap::BLEND);
    assert_eq!(t.alpha_operation().unwrap(), AlphaMap::MODULATE);
    assert!(t.flags().unwrap().intersects(TObjFlags::BUMP));
}

#[test]
fn tobj_image_chain_round_trips() {
    // Build a MObj with one TObj attached to texture0.  The TObj points
    // at a fresh Image whose payload is 8 bytes of arbitrary GX-encoded
    // data (we don't care about decode parity here — that's the H3
    // encoder's job; this test just pins the *structural* round-trip).
    let mobj = MObj::allocate_default();
    mobj.set_render_flags(MaterialRenderMode::TEX0 | MaterialRenderMode::DIFFUSE)
        .unwrap();

    let tobj = TObj::allocate_default();
    tobj.set_tex_map_id(GxTexMapId::GX_TEXMAP0).unwrap();
    tobj.set_scale(1.0, 1.0, 1.0).unwrap();
    tobj.set_wrap_s(GxWrapMode::REPEAT).unwrap();
    tobj.set_wrap_t(GxWrapMode::CLAMP).unwrap();
    tobj.set_repeat_s(1).unwrap();
    tobj.set_repeat_t(2).unwrap();
    tobj.set_blending(1.0).unwrap();
    tobj.set_mag_filter(GxTexFilter::GX_LINEAR).unwrap();
    tobj.set_coord_type(CoordType::UV).unwrap();
    tobj.set_color_operation(ColorMap::MODULATE).unwrap();
    tobj.set_alpha_operation(AlphaMap::MODULATE).unwrap();

    let img = Image::allocate_default();
    img.set_width(4).unwrap();
    img.set_height(4).unwrap();
    img.set_format(GxTexFmt::RGB565).unwrap();
    img.set_min_lod(0.0).unwrap();
    img.set_max_lod(0.0).unwrap();
    // 4×4 RGB565 = 4*4*2 = 32 bytes.  Distinct values so dedup doesn't
    // collapse this against another texture in the writer pass.
    let payload: Vec<u8> = (0..32).map(|i| 0x40 + i as u8).collect();
    img.set_image_data_bytes(payload.clone());

    tobj.set_image_data(Some(img));
    mobj.set_textures(Some(tobj));

    let dat = round_trip(dat_with_mobj(mobj));
    let mobj2 = extract_first_mobj(&dat);
    let tobj2 = mobj2.textures().expect("TObj survives round-trip");
    assert_eq!(tobj2.tex_map_id().unwrap(), GxTexMapId::GX_TEXMAP0);
    assert!((tobj2.sx().unwrap() - 1.0).abs() < 1e-6);
    assert_eq!(tobj2.wrap_s().unwrap(), GxWrapMode::REPEAT);
    assert_eq!(tobj2.wrap_t().unwrap(), GxWrapMode::CLAMP);
    assert_eq!(tobj2.repeat_s().unwrap(), 1);
    assert_eq!(tobj2.repeat_t().unwrap(), 2);
    assert_eq!(tobj2.coord_type().unwrap(), CoordType::UV);
    assert_eq!(tobj2.color_operation().unwrap(), ColorMap::MODULATE);
    assert_eq!(tobj2.alpha_operation().unwrap(), AlphaMap::MODULATE);
    assert_eq!(tobj2.mag_filter().unwrap(), GxTexFilter::GX_LINEAR);

    let img2 = tobj2.image_data().expect("Image survives round-trip");
    assert_eq!(img2.width().unwrap(), 4);
    assert_eq!(img2.height().unwrap(), 4);
    assert_eq!(img2.format().unwrap(), GxTexFmt::RGB565);
    let bytes = img2.image_data().expect("raw bytes survived");
    assert_eq!(bytes, payload);
}

#[test]
fn tobj_tex_gen_src_round_trips() {
    // GXTexGenSrc lives at offset 0x0C — verify it round-trips through
    // the writer + re-parse and isn't clobbered by any other setter.
    let mobj = MObj::allocate_default();
    let tobj = TObj::allocate_default();
    tobj.set_tex_map_id(GxTexMapId::GX_TEXMAP0).unwrap();
    tobj.set_scale(1.0, 1.0, 1.0).unwrap();
    tobj.set_tex_gen_src(GxTexGenSrc::GX_TG_TEX0).unwrap();
    let img = Image::allocate_default();
    img.set_width(4).unwrap();
    img.set_height(4).unwrap();
    img.set_format(GxTexFmt::RGB565).unwrap();
    img.set_image_data_bytes(vec![0u8; 32]);
    tobj.set_image_data(Some(img));
    mobj.set_textures(Some(tobj));

    let dat = round_trip(dat_with_mobj(mobj));
    let tobj2 = extract_first_mobj(&dat).textures().unwrap();
    assert_eq!(tobj2.tex_gen_src().unwrap(), GxTexGenSrc::GX_TG_TEX0);
}

#[test]
fn tobj_lod_round_trips() {
    // HSD_TOBJ_LOD layout (BE, TrimmedSize 0x10):
    //   0x00 i32 MinFilter
    //   0x04 f32 Bias
    //   0x08 u8  BiasClamp
    //   0x09 u8  EnableEdgeLOD
    //   0x0A i32 Anisotropy   (byte-unaligned!)
    // Drive every field through writer round-trip and verify each
    // returns the exact same value, including the unaligned i32.
    let mobj = MObj::allocate_default();
    let tobj = TObj::allocate_default();
    tobj.set_tex_map_id(GxTexMapId::GX_TEXMAP0).unwrap();
    tobj.set_scale(1.0, 1.0, 1.0).unwrap();
    let img = Image::allocate_default();
    img.set_width(4).unwrap();
    img.set_height(4).unwrap();
    img.set_format(GxTexFmt::RGB565).unwrap();
    img.set_image_data_bytes(vec![0u8; 32]);
    tobj.set_image_data(Some(img));

    let lod = Lod::allocate_default();
    lod.set_min_filter(GxTexFilter::GX_LIN_MIP_LIN).unwrap();
    lod.set_bias(-1.5).unwrap();
    lod.set_bias_clamp(true).unwrap();
    lod.set_enable_edge_lod(true).unwrap();
    lod.set_anisotropy(GxAnisotropy::GX_MAX_ANISOTROPY).unwrap();
    tobj.set_lod_data(Some(lod));

    mobj.set_textures(Some(tobj));

    let dat = round_trip(dat_with_mobj(mobj));
    let tobj2 = extract_first_mobj(&dat).textures().unwrap();
    let lod2 = tobj2.lod_data().expect("LOD attached after round-trip");
    assert_eq!(lod2.min_filter().unwrap(), GxTexFilter::GX_LIN_MIP_LIN);
    assert_eq!(lod2.bias().unwrap(), -1.5);
    assert!(lod2.bias_clamp().unwrap());
    assert!(lod2.enable_edge_lod().unwrap());
    assert_eq!(lod2.anisotropy().unwrap(), GxAnisotropy::GX_MAX_ANISOTROPY);
}

#[test]
fn tobj_lod_default_is_0x10_zero() {
    // `Lod::allocate_default` must produce a 0x10-byte all-zero struct
    // — i.e. MinFilter=GX_NEAR, Bias=0, BiasClamp=false,
    // EnableEdgeLOD=false, Anisotropy=GX_ANISO_1.  Pin that contract so
    // a reader looking at a default-allocated LOD doesn't see leftover
    // bytes.
    let lod = Lod::allocate_default();
    assert_eq!(lod.0.borrow().len(), 0x10);
    assert_eq!(lod.min_filter().unwrap(), GxTexFilter::GX_NEAR);
    assert_eq!(lod.bias().unwrap(), 0.0);
    assert!(!lod.bias_clamp().unwrap());
    assert!(!lod.enable_edge_lod().unwrap());
    assert_eq!(lod.anisotropy().unwrap(), GxAnisotropy::GX_ANISO_1);
}

#[test]
fn tobj_named_lightmap_setters_preserve_other_bits() {
    // RMW guarantee: set_lightmap_diffuse(true) must leave the
    // coord_type / color_op / alpha_op nibbles + every other flag bit
    // alone.  We seed those nibbles + an extra flag (BUMP), then flip
    // LIGHTMAP_DIFFUSE on and verify nothing else changed.
    let tobj = TObj::allocate_default();
    tobj.set_coord_type(CoordType::REFLECTION).unwrap();
    tobj.set_color_operation(ColorMap::BLEND).unwrap();
    tobj.set_alpha_operation(AlphaMap::MODULATE).unwrap();
    tobj.set_bump(true).unwrap();
    let snap_before = tobj.0.borrow().get_u32(0x40).unwrap();

    tobj.set_lightmap_diffuse(true).unwrap();
    let after = tobj.0.borrow().get_u32(0x40).unwrap();
    assert_eq!(
        after,
        snap_before | (1u32 << 4),
        "set_lightmap_diffuse must add exactly bit 4"
    );
    assert!(tobj.is_lightmap_diffuse().unwrap());
    assert_eq!(tobj.coord_type().unwrap(), CoordType::REFLECTION);
    assert_eq!(tobj.color_operation().unwrap(), ColorMap::BLEND);
    assert_eq!(tobj.alpha_operation().unwrap(), AlphaMap::MODULATE);
    assert!(tobj.is_bump().unwrap());

    // Toggle off → returns to the seeded state.
    tobj.set_lightmap_diffuse(false).unwrap();
    assert_eq!(tobj.0.borrow().get_u32(0x40).unwrap(), snap_before);
    assert!(!tobj.is_lightmap_diffuse().unwrap());

    // All five lightmap setters + BUMP work the same way.  Each
    // iteration: toggle to the opposite of current, then toggle back,
    // assert we're at the original state.  Robust to whether the bit
    // is on or off in the seeded `tobj` already.
    for f in [
        TObjFlags::LIGHTMAP_DIFFUSE,
        TObjFlags::LIGHTMAP_SPECULAR,
        TObjFlags::LIGHTMAP_AMBIENT,
        TObjFlags::LIGHTMAP_EXT,
        TObjFlags::LIGHTMAP_SHADOW,
        TObjFlags::BUMP,
    ] {
        let before = tobj.0.borrow().get_u32(0x40).unwrap();
        let was_set = (before & f.bits()) != 0;
        tobj.set_flag_bit(f, !was_set).unwrap();
        let after = tobj.0.borrow().get_u32(0x40).unwrap();
        assert_eq!((after & f.bits()) != 0, !was_set, "toggle to opposite");
        assert_eq!(after & !f.bits(), before & !f.bits(), "other bits intact");
        tobj.set_flag_bit(f, was_set).unwrap();
        assert_eq!(tobj.0.borrow().get_u32(0x40).unwrap(), before, "restore");
    }
}

#[test]
fn tobj_chain_two_textures_round_trips() {
    // Wire two TObjs in a Next chain: texture 0 + texture 1 sharing the
    // same MObj.  Common pattern for diffuse + lightmap rigging.
    let mobj = MObj::allocate_default();
    mobj.set_render_flags(
        MaterialRenderMode::TEX0
            | MaterialRenderMode::TEX1
            | MaterialRenderMode::DIFFUSE,
    )
    .unwrap();

    let make_tobj = |id: GxTexMapId, w: i16| {
        let t = TObj::allocate_default();
        t.set_tex_map_id(id).unwrap();
        t.set_scale(1.0, 1.0, 1.0).unwrap();
        let img = Image::allocate_default();
        img.set_width(w).unwrap();
        img.set_height(4).unwrap();
        img.set_format(GxTexFmt::RGB565).unwrap();
        let n = (w as usize) * 4 * 2;
        let payload: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_add(w as u8)).collect();
        img.set_image_data_bytes(payload);
        t.set_image_data(Some(img));
        t
    };
    let tobj0 = make_tobj(GxTexMapId::GX_TEXMAP0, 4);
    let tobj1 = make_tobj(GxTexMapId::GX_TEXMAP1, 8);
    tobj0.set_next(Some(tobj1));
    mobj.set_textures(Some(tobj0));

    let dat = round_trip(dat_with_mobj(mobj));
    let mobj2 = extract_first_mobj(&dat);
    let head = mobj2.textures().expect("TObj0");
    assert_eq!(head.tex_map_id().unwrap(), GxTexMapId::GX_TEXMAP0);
    assert_eq!(head.image_data().unwrap().width().unwrap(), 4);
    let next = head.next().expect("TObj1 chain link");
    assert_eq!(next.tex_map_id().unwrap(), GxTexMapId::GX_TEXMAP1);
    assert_eq!(next.image_data().unwrap().width().unwrap(), 8);
    assert!(next.next().is_none(), "chain ends after TObj1");
}

#[test]
fn explicit_setter_path_round_trips() {
    // Build a MObj with explicit ambient + diffuse + specular + alpha
    // + shininess, plus a PE descriptor with a non-zero blend mode.
    let mobj = MObj::allocate_default();
    mobj.set_render_flags(MaterialRenderMode::DIFFUSE).unwrap();

    let mat = Material::allocate_default();
    mat.set_amb_rgba([0x10, 0x10, 0x10, 0xFF]).unwrap();
    mat.set_dif_rgba([0x80, 0xC0, 0x00, 0xFF]).unwrap();
    mat.set_spc_rgba([0xFF, 0xFF, 0xFF, 0xFF]).unwrap();
    mat.set_alpha(0.75).unwrap();
    mat.set_shininess(8.5).unwrap();
    mobj.set_material(Some(mat));

    let pe = PeDesc::allocate_default();
    pe.set_blend_mode(1).unwrap(); // GX_BLEND
    pe.set_src_factor(4).unwrap(); // GX_BL_SRCALPHA
    pe.set_dst_factor(5).unwrap(); // GX_BL_INVSRCALPHA
    pe.set_alpha_ref0(0x40).unwrap();
    mobj.set_pe_desc(Some(pe));

    let dat = round_trip(dat_with_mobj(mobj));
    let mobj2 = extract_first_mobj(&dat);

    let mat2 = mobj2.material().unwrap();
    assert_eq!(mat2.amb_rgba().unwrap(), [0x10, 0x10, 0x10, 0xFF]);
    assert_eq!(mat2.dif_rgba().unwrap(), [0x80, 0xC0, 0x00, 0xFF]);
    assert_eq!(mat2.spc_rgba().unwrap(), [0xFF, 0xFF, 0xFF, 0xFF]);
    assert!((mat2.alpha().unwrap() - 0.75).abs() < 1e-6);
    assert!((mat2.shininess().unwrap() - 8.5).abs() < 1e-6);

    let pe2 = mobj2.pe_desc().expect("pe survives round-trip");
    assert_eq!(pe2.blend_mode().unwrap(), 1);
    assert_eq!(pe2.src_factor().unwrap(), 4);
    assert_eq!(pe2.dst_factor().unwrap(), 5);
    assert_eq!(pe2.alpha_ref0().unwrap(), 0x40);
}
