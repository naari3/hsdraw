//! Rust-API-level tests for the HSDLib-equivalent mutation primitives
//! (`Dat::add_root` / `remove_root` / `rename_root` / `repoint_root` /
//! `find_root_for`, `JObj::allocate_default`, `JObj::set_*`).
//!
//! These cover the same five behaviors a Blender add-on cares about
//! (the project-side csx Pass 0–4 distillation), but at the
//! primitive level — no `scene.json` schema is involved.  Each test
//! drives parse → mutate → write → parse and asserts the expected
//! delta on the rebuilt tree.
//!
//! The full corpus tests live in `tests/parity.rs`; this file is the
//! self-contained CI gate (no `MKGP2_FILES_DIR` env required).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use hsdraw_core::accessor::Accessor;
use hsdraw_core::common::{JObj, JObjDesc, SObj};
use hsdraw_core::dat::{Dat, RootNode};
use hsdraw_core::hsd_struct::{HsdStruct, ptr_eq};

/// Path to the committed synthetic fixture (`tests/data/synthetic_minimal.dat`).
fn synthetic_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("synthetic_minimal.dat")
}

/// Build a minimum-viable in-memory `Dat` for tests that need a bit more
/// shape than the synthetic fixture (a SOBJ with a JOBJDescs[] and a
/// 3-joint root tree).  Doesn't go through parse — we just construct
/// the Rc graph directly, which the writer is happy to consume.
fn build_synthetic_tree() -> Dat {
    // Three-joint chain: root → child0 → child0.next = sibling
    let root = JObj::allocate_default();
    let child0 = JObj::allocate_default();
    let sibling = JObj::allocate_default();
    child0.set_next(Some(sibling.clone()));
    root.set_child(Some(child0.clone()));

    // Tag the joints with distinct TRS so we can identify them post-write.
    root.set_tx(1.0).unwrap();
    child0.set_tx(2.0).unwrap();
    sibling.set_tx(3.0).unwrap();

    // SOBJ at offset 0 references a JOBJDescs[] array; entry 0 points
    // at a JObjDesc whose RootJoint = our root.
    let jobj_desc = HsdStruct::with_capacity(0x10).into_ref();
    jobj_desc.borrow_mut().set_reference(0x00, Some(root.0.clone()));

    let jobj_descs_arr = HsdStruct::with_capacity(0x08).into_ref();
    jobj_descs_arr
        .borrow_mut()
        .set_reference(0x00, Some(jobj_desc.clone()));
    // entry 1 is left null which terminates the array (HSDLib convention).

    let sobj = HsdStruct::with_capacity(0x10).into_ref();
    sobj.borrow_mut().set_reference(0x00, Some(jobj_descs_arr));

    Dat {
        version: [0; 4],
        roots: vec![RootNode {
            name: "scene_data".to_owned(),
            data: sobj,
        }],
        references: vec![],
        struct_order: vec![],
    }
}

/// Walk a freshly-parsed `Dat`'s scene_data tree and return its joints
/// in DFS order.  Identifies a joint by its TRS values (tx, ty, tz).
fn collect_joints_dfs(dat: &Dat) -> Vec<JObj> {
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<*const RefCell<HsdStruct>> =
        std::collections::HashSet::new();
    fn walk(
        j: JObj,
        out: &mut Vec<JObj>,
        seen: &mut std::collections::HashSet<*const RefCell<HsdStruct>>,
    ) {
        let key = Rc::as_ptr(j.as_struct());
        if !seen.insert(key) {
            return;
        }
        out.push(j.clone());
        if let Some(c) = j.child() {
            let mut cur = Some(c);
            while let Some(cc) = cur {
                walk(cc.clone(), out, seen);
                cur = cc.next();
            }
        }
    }
    if let Some(scene_root) = dat.scene_data() {
        let sobj = SObj::from_struct(scene_root.data.clone());
        for desc in sobj.jobj_descs() {
            let desc: JObjDesc = desc;
            if let Some(rj) = desc.root_joint() {
                walk(rj, &mut out, &mut seen);
            }
        }
    }
    for r in &dat.roots {
        if r.name == "scene_data" || !r.name.ends_with("_joint") {
            continue;
        }
        walk(JObj::from_struct(r.data.clone()), &mut out, &mut seen);
    }
    out
}

// =====================================================================
// 1. alias add
// =====================================================================

#[test]
fn primitive_alias_add_round_trips() {
    let dat = build_synthetic_tree();
    // Pick the second joint (DFS index 1 = first child) as the alias target.
    let target = collect_joints_dfs(&dat).into_iter().nth(1).unwrap();

    // Capture the target's TRS so we can recover its identity post-write.
    let snap_tx = target.tx().unwrap();

    // Add an alias.
    let mut dat = dat;
    dat.add_root("test_joint", target.0.clone());
    let written = dat.write().expect("write");

    // After re-parse the alias must be present and point at a struct
    // with the same TRS (Rc identity is renewed across (de)serialization).
    let dat2 = Dat::parse(&written).expect("reparse");
    let alias = dat2
        .roots
        .iter()
        .find(|r| r.name == "test_joint")
        .expect("alias root present");
    let aliased = JObj::from_struct(alias.data.clone());
    assert!((aliased.tx().unwrap() - snap_tx).abs() < 1e-6);
}

// =====================================================================
// 2. alias remove
// =====================================================================

#[test]
fn primitive_alias_remove_round_trips() {
    // Real corpus would be ideal here, but env-free CI: we add an alias
    // and immediately remove it to assert the round-trip drops it.
    let mut dat = build_synthetic_tree();
    let target = collect_joints_dfs(&dat).into_iter().nth(1).unwrap();
    dat.add_root("ephemeral_joint", target.0.clone());
    assert!(dat.remove_root("ephemeral_joint"));
    assert!(!dat.remove_root("ephemeral_joint")); // second call is a no-op

    let written = dat.write().expect("write");
    let dat2 = Dat::parse(&written).expect("reparse");
    assert!(
        !dat2.roots.iter().any(|r| r.name == "ephemeral_joint"),
        "removed alias must not appear in the rebuilt file"
    );
}

// =====================================================================
// 3. TRS edit
// =====================================================================

#[test]
fn primitive_trs_edit_round_trips() {
    let dat = build_synthetic_tree();
    // Edit child0 (index 1) to a distinctive TRS signature.
    let target = collect_joints_dfs(&dat).into_iter().nth(1).unwrap();
    target.set_tx(123.5).unwrap();
    target.set_rz(0.25).unwrap();
    target.set_sx(2.5).unwrap();

    let written = dat.write().expect("write");
    let dat2 = Dat::parse(&written).expect("reparse");
    // Joints renumber on re-parse but the SOBJ tree is intact, and
    // child0 was the root's first child (DFS index 1).
    let target2 = collect_joints_dfs(&dat2).into_iter().nth(1).unwrap();
    assert!((target2.tx().unwrap() - 123.5).abs() < 1e-5);
    assert!((target2.rz().unwrap() - 0.25).abs() < 1e-5);
    assert!((target2.sx().unwrap() - 2.5).abs() < 1e-5);
}

// =====================================================================
// 4. new JObj alloc + reparent
// =====================================================================

#[test]
fn primitive_new_jobj_alloc_and_reparent_round_trips() {
    let dat = build_synthetic_tree();
    let parent = collect_joints_dfs(&dat).into_iter().next().unwrap();

    // Allocate a fresh joint.
    let new_joint = JObj::allocate_default();
    // Mark it with a unique TRS so we can find it post-write.
    new_joint.set_tx(42.0).unwrap();

    // Splice it as the *new* first child of `parent`, pushing the old
    // first child to be `new_joint.Next`.
    let old_first = parent.child().expect("parent had a child");
    new_joint.set_next(Some(old_first.clone()));
    parent.set_child(Some(new_joint.clone()));

    let written = dat.write().expect("write");
    let dat2 = Dat::parse(&written).expect("reparse");

    // The new joint must be reachable, with its TRS preserved.
    let joints = collect_joints_dfs(&dat2);
    let found = joints
        .iter()
        .find(|j| (j.tx().unwrap() - 42.0).abs() < 1e-5)
        .expect("new joint reachable in rebuilt tree");
    // It should be the new root.Child[0] — sx is identity (1.0) since
    // we left the rest of TRS at JObj::allocate_default's defaults.
    assert!((found.sx().unwrap() - 1.0).abs() < 1e-6);

    // And the old first child's TRS (tx=2.0 from build_synthetic_tree)
    // must now appear as the new joint's Next.
    let new_next = found.next().expect("new joint must have a Next sibling");
    assert!((new_next.tx().unwrap() - 2.0).abs() < 1e-5);
}

// =====================================================================
// 5. hierarchy rewire (move existing joint to a new parent)
// =====================================================================

#[test]
fn primitive_hierarchy_rewire_round_trips() {
    let dat = build_synthetic_tree();
    let joints = collect_joints_dfs(&dat);
    let root = joints[0].clone();
    let child0 = joints[1].clone(); // tx=2.0
    let sibling = joints[2].clone(); // tx=3.0

    // Reparent: take `sibling` out of root.Child chain and make it a
    // child of `child0` instead.
    // 1) detach sibling from root.Child chain (child0.next was sibling)
    child0.set_next(None);
    // 2) attach sibling as child0's first child
    child0.set_child(Some(sibling.clone()));
    sibling.set_next(None);

    // sanity (pre-write): the live tree already reflects the change
    let mut chain_pre: Vec<f32> = Vec::new();
    if let Some(c) = root.child() {
        let mut cur = Some(c);
        while let Some(cc) = cur {
            chain_pre.push(cc.tx().unwrap());
            cur = cc.next();
        }
    }
    assert_eq!(chain_pre, vec![2.0]);

    let written = dat.write().expect("write");
    let dat2 = Dat::parse(&written).expect("reparse");

    let new_joints = collect_joints_dfs(&dat2);
    let new_root = new_joints
        .iter()
        .find(|j| (j.tx().unwrap() - 1.0).abs() < 1e-5)
        .unwrap();
    let new_child0 = new_root.child().expect("root must still have a child");
    assert!((new_child0.tx().unwrap() - 2.0).abs() < 1e-5);
    assert!(
        new_child0.next().is_none(),
        "child0's Next must now be None"
    );
    let grandchild = new_child0
        .child()
        .expect("child0 should now have its own child");
    assert!((grandchild.tx().unwrap() - 3.0).abs() < 1e-5);
}

// =====================================================================
// Identity: find_root_for round-trips
// =====================================================================

#[test]
fn primitive_find_root_for_returns_existing_alias() {
    let mut dat = build_synthetic_tree();
    let target = collect_joints_dfs(&dat).into_iter().nth(1).unwrap();
    dat.add_root("alpha_joint", target.0.clone());
    dat.add_root("beta_joint", target.0.clone()); // double-alias OK

    let found = dat.find_root_for(&target.0).expect("must find alpha");
    assert_eq!(found.name, "alpha_joint", "should return the first match");

    // Repointing alpha to a different struct moves find() onto beta.
    let other = JObj::allocate_default();
    dat.repoint_root("alpha_joint", other.0.clone());
    let found2 = dat.find_root_for(&target.0).expect("must find beta now");
    assert_eq!(found2.name, "beta_joint");
}

// =====================================================================
// Synthetic-fixture sanity: the committed minimal .dat parses fine
// (covered in tests/parity.rs already, but we re-assert here so this
// test file works as a stand-alone smoke check too).
// =====================================================================

#[test]
fn synthetic_minimal_fixture_parses() {
    let bytes = std::fs::read(synthetic_fixture()).expect("fixture present");
    let dat = Dat::parse(&bytes).expect("parse");
    assert_eq!(dat.roots.len(), 1);
    assert_eq!(dat.roots[0].name, "scene_data");
}

// =====================================================================
// JObj allocate_default initial state
// =====================================================================

#[test]
fn jobj_allocate_default_has_identity_scale() {
    let j = JObj::allocate_default();
    assert!((j.sx().unwrap() - 1.0).abs() < 1e-6);
    assert!((j.sy().unwrap() - 1.0).abs() < 1e-6);
    assert!((j.sz().unwrap() - 1.0).abs() < 1e-6);
    assert_eq!(j.tx().unwrap(), 0.0);
    assert_eq!(j.flags().unwrap().bits(), 0);
    assert!(j.child().is_none());
    assert!(j.next().is_none());
}

// =====================================================================
// Identity: ptr_eq survives Rc clone (sanity check for the JObj /
// HsdStruct equality contract the Python binding relies on)
// =====================================================================

#[test]
fn jobj_identity_via_rc_clone() {
    let a = JObj::allocate_default();
    let b = JObj::from_struct(a.0.clone());
    assert!(ptr_eq(&a.0, &b.0));
    let c = JObj::allocate_default();
    assert!(!ptr_eq(&a.0, &c.0));
}

// =====================================================================
// HsdStruct::references() / get_reference() — exercises the raw-ref
// walk path the mkgp2-patch addon's Pass 0 uses (it has no SObj typed
// accessor in Python, so it must enumerate references itself to reach
// the JOBJDescs[] array out of scene_data).
// =====================================================================

#[test]
fn synthetic_struct_references_enumerate() {
    let dat = build_synthetic_tree();
    let scene = dat.scene_data().expect("scene_data root present");

    // SOBJ has exactly one reference: offset 0x00 → JOBJDescs[] array.
    let refs = {
        let s = scene.data.borrow();
        s.references()
            .iter()
            .map(|(off, t)| (*off, t.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(refs.len(), 1, "SOBJ should have exactly one reference");
    assert_eq!(refs[0].0, 0x00);

    // get_reference(0x00) hits; out-of-range offset returns None.
    let descs_arr = scene
        .data
        .borrow()
        .get_reference(0x00)
        .expect("offset 0 ref present");
    assert!(scene.data.borrow().get_reference(0x10).is_none());
    // The references()-iterated entry and the get_reference()-found
    // entry must be the same Rc allocation, not a byte-equal copy.
    assert!(ptr_eq(&descs_arr, &refs[0].1));

    // Walk: descs_arr[0] → JObjDesc → RootJoint, and the recovered JObj
    // must have the tx=1.0 we tagged the synthetic root with.
    let desc = descs_arr
        .borrow()
        .get_reference(0x00)
        .expect("descs_arr[0] present");
    let root_joint = desc
        .borrow()
        .get_reference(0x00)
        .expect("desc.RootJoint present");
    let root = JObj::from_struct(root_joint);
    assert!((root.tx().unwrap() - 1.0).abs() < 1e-6);
}
