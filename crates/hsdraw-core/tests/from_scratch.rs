//! End-to-end integration test for the from-scratch synthesis pipeline:
//! `Dat::alloc_scene_data` → root JObj → DObj → MObj → Material → TObj
//! → Image (with `gx_encode`-produced RGBA8 payload) → write → re-parse
//! → walk and verify every link.
//!
//! This is the test the mkgp2-patch addon's vanilla-independent export
//! path effectively runs at the integration layer: it builds a full
//! self-contained .dat from CPU-side data with no base file to start
//! from.  Per-link assertions catch regressions in any one of the
//! H1/H2/H3 layers in isolation.

use hsdraw_core::accessor::Accessor;
use hsdraw_core::common::{
    DObj, Image, JObj, MObj, Material, SObj, TObj,
};
use hsdraw_core::dat::Dat;
use hsdraw_core::gx::{GxTexFmt, GxTexMapId, GxWrapMode, MaterialRenderMode};
use hsdraw_core::gx_image::{decode_image, encode_image};

/// Walk scene_data → SObj → JOBJDescs[0] → root JObj.
fn root_joint(dat: &Dat) -> JObj {
    let scene = dat.scene_data().expect("scene_data root");
    let sobj = SObj::from_struct(scene.data.clone());
    let descs = sobj.jobj_descs();
    descs[0]
        .root_joint()
        .expect("JObjDesc must have a root joint")
}

#[test]
fn from_scratch_scene_data_with_textured_mesh_round_trips() {
    // ---- Phase 1: scaffold ---------------------------------------
    let dat = Dat::alloc_scene_data();
    let root = root_joint(&dat);

    // ---- Phase 2: a 4x4 RGBA8 source we encode + attach ----------
    // 4 rows × 4 cols, each pixel a distinct deterministic color so a
    // post-decode comparison spots any swizzle / endian bug.
    let mut src_rgba = Vec::with_capacity(4 * 4 * 4);
    for y in 0..4u8 {
        for x in 0..4u8 {
            src_rgba.extend_from_slice(&[
                0x10 + x * 0x20,
                0x10 + y * 0x20,
                0x80,
                0xFF,
            ]);
        }
    }
    // RGB565 encode: lossy but the encoder→decoder round-trip after
    // 5/6/5-snap is byte-equal for already-snapped sources.  We just
    // assert the encoded buffer has the right length here (decode-side
    // verification is in tests/gx_encoder.rs).
    let gx_bytes = encode_image(GxTexFmt::RGB565, 4, 4, &src_rgba)
        .expect("encode RGB565");
    assert_eq!(gx_bytes.len(), 32);

    // ---- Phase 3: build the material chain -----------------------
    let img = Image::allocate_default();
    img.set_width(4).unwrap();
    img.set_height(4).unwrap();
    img.set_format(GxTexFmt::RGB565).unwrap();
    img.set_image_data_bytes(gx_bytes.clone());

    let tobj = TObj::allocate_default();
    tobj.set_tex_map_id(GxTexMapId::GX_TEXMAP0).unwrap();
    tobj.set_scale(1.0, 1.0, 1.0).unwrap();
    tobj.set_wrap_s(GxWrapMode::REPEAT).unwrap();
    tobj.set_wrap_t(GxWrapMode::REPEAT).unwrap();
    tobj.set_image_data(Some(img));

    let mat = Material::allocate_default();
    mat.set_dif_rgba([0xFF, 0xFF, 0xFF, 0xFF]).unwrap();
    mat.set_alpha(1.0).unwrap();
    mat.set_shininess(50.0).unwrap();

    let mobj = MObj::allocate_default();
    mobj.set_render_flags(
        MaterialRenderMode::TEX0 | MaterialRenderMode::DIFFUSE,
    )
    .unwrap();
    mobj.set_material(Some(mat));
    mobj.set_textures(Some(tobj));

    let dobj = DObj::allocate_default();
    dobj.set_mobj(Some(mobj));

    root.set_dobj(Some(dobj)).expect("set_dobj");

    // Tag the root joint so we can identify it after re-parse.
    root.set_tx(7.5).unwrap();
    root.set_ry(0.25).unwrap();

    // ---- Phase 4: write + re-parse + verify ----------------------
    let written = dat.write().expect("write");
    let dat2 = Dat::parse(&written).expect("re-parse");

    // Roots: just scene_data.
    assert_eq!(dat2.roots.len(), 1);
    assert_eq!(dat2.roots[0].name, "scene_data");
    assert!(dat2.references.is_empty());

    // Walk the chain back out.
    let root2 = root_joint(&dat2);
    assert!((root2.tx().unwrap() - 7.5).abs() < 1e-5);
    assert!((root2.ry().unwrap() - 0.25).abs() < 1e-5);

    let dobj2 = root2
        .dobj()
        .expect("flags allow Dobj")
        .expect("DObj attached");
    let mobj2 = dobj2.mobj().expect("MObj");

    let flags2 = mobj2.render_flags().unwrap();
    assert!(flags2.intersects(MaterialRenderMode::TEX0));
    assert!(flags2.intersects(MaterialRenderMode::DIFFUSE));

    let mat2 = mobj2.material().expect("Material");
    assert_eq!(mat2.dif_rgba().unwrap(), [0xFF, 0xFF, 0xFF, 0xFF]);
    assert!((mat2.alpha().unwrap() - 1.0).abs() < 1e-6);
    assert!((mat2.shininess().unwrap() - 50.0).abs() < 1e-6);

    let tobj2 = mobj2.textures().expect("TObj");
    assert_eq!(tobj2.tex_map_id().unwrap(), GxTexMapId::GX_TEXMAP0);
    assert_eq!(tobj2.wrap_s().unwrap(), GxWrapMode::REPEAT);
    assert_eq!(tobj2.wrap_t().unwrap(), GxWrapMode::REPEAT);
    assert!((tobj2.sx().unwrap() - 1.0).abs() < 1e-6);

    let img2 = tobj2.image_data().expect("Image");
    assert_eq!(img2.width().unwrap(), 4);
    assert_eq!(img2.height().unwrap(), 4);
    assert_eq!(img2.format().unwrap(), GxTexFmt::RGB565);

    // Raw bytes survive byte-for-byte (they're a leaf buffer).
    let recovered = img2.image_data().expect("raw payload");
    assert_eq!(
        recovered, gx_bytes,
        "GX-encoded bytes must round-trip byte-equal"
    );

    // Decode them back: RGB565 round-trip lossy at the encoder edge,
    // but the decoder output is deterministic — assert byte-level
    // identity against an encode→decode of the original source.
    let baseline = decode_image(GxTexFmt::RGB565, 4, 4, &gx_bytes, None)
        .expect("baseline decode");
    let post_round_trip = decode_image(GxTexFmt::RGB565, 4, 4, &recovered, None)
        .expect("post-round-trip decode");
    assert_eq!(baseline, post_round_trip);
}

#[test]
fn from_scratch_scene_data_with_two_joints_and_one_textured_mesh() {
    // Hierarchy stress: root has a child joint, child carries the
    // mesh.  Validates that adding a child JObj to the from-scratch
    // factory chain doesn't disturb the SObj→JOBJDescs→RootJoint
    // round-trip.
    let dat = Dat::alloc_scene_data();
    let root = root_joint(&dat);
    root.set_tx(0.0).unwrap();

    let child = JObj::allocate_default();
    child.set_tx(10.0).unwrap();
    root.set_child(Some(child.clone()));

    // Attach the mesh's MObj to the child.
    let img = Image::allocate_default();
    img.set_width(4).unwrap();
    img.set_height(4).unwrap();
    img.set_format(GxTexFmt::RGBA8).unwrap();
    let solid_red: Vec<u8> = (0..4 * 4)
        .flat_map(|_| [0xFF, 0x00, 0x00, 0xFF])
        .collect();
    let gx_bytes = encode_image(GxTexFmt::RGBA8, 4, 4, &solid_red).unwrap();
    img.set_image_data_bytes(gx_bytes);

    let tobj = TObj::allocate_default();
    tobj.set_tex_map_id(GxTexMapId::GX_TEXMAP0).unwrap();
    tobj.set_image_data(Some(img));

    let mobj = MObj::allocate_unlit_color(0xFF, 0xFF, 0xFF, 0xFF);
    mobj.set_textures(Some(tobj));

    let dobj = DObj::allocate_default();
    dobj.set_mobj(Some(mobj));
    child.set_dobj(Some(dobj)).unwrap();

    let written = dat.write().unwrap();
    let dat2 = Dat::parse(&written).unwrap();

    // Re-walk: root has tx=0, child has tx=10 + a textured DObj.
    let root2 = root_joint(&dat2);
    assert!((root2.tx().unwrap() - 0.0).abs() < 1e-6);
    let child2 = root2.child().expect("child joint reachable");
    assert!((child2.tx().unwrap() - 10.0).abs() < 1e-5);
    assert!(child2.next().is_none(), "single child only");
    let img2 = child2
        .dobj()
        .unwrap()
        .unwrap()
        .mobj()
        .unwrap()
        .textures()
        .unwrap()
        .image_data()
        .unwrap();
    assert_eq!(img2.width().unwrap(), 4);
    assert_eq!(img2.format().unwrap(), GxTexFmt::RGBA8);
}
