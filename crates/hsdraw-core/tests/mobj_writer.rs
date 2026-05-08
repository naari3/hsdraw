//! Round-trip tests for MObj / Material / PeDesc allocation primitives.
//!
//! These mirror the same parse → mutate → write → re-parse loop that
//! the POBJ writer tests use, but exercise the material side of the
//! `D{Obj} → M{Obj} → Material/PeDesc/Texture` chain.  Phase 1 doesn't
//! ship a TObj writer (texture re-pack is roadmapped); the tests here
//! cover the unlit / vertex-colored / placeholder material paths
//! mkgp2-patch's addon needs to attach POBJs to.

use hsdraw_core::accessor::Accessor;
use hsdraw_core::common::{
    DObj, JObj, MObj, Material, PeDesc, SObj,
};
use hsdraw_core::dat::{Dat, RootNode};
use hsdraw_core::gx::MaterialRenderMode;
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
