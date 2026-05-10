//! Round-trip tests for the Phase 1 POBJ writer.
//!
//! Each test:
//!   1. Builds a `PObj` via `MeshBuilder` + attaches it to a synthetic
//!      `Dat` (one JObj → DObj → POBJ chain rooted at `scene_data`).
//!   2. `dat.write()` → bytes
//!   3. `Dat::parse(bytes)` → reload
//!   4. Walks back to the POBJ, runs `gx_dl::unpack`, and asserts the
//!      resulting vertex stream / attribute set / DL primitive type
//!      match the inputs.
//!
//! Test (6) from the brief — decompose & reassemble a vanilla course's
//! POBJ and assert byte-identical — is intentionally omitted: the
//! Phase 1 writer emits TRIANGLES while HSDLib emits TRIANGLE_STRIP, so
//! byte equality is impossible by design.  The "DL parses back" gate is
//! covered by every test below (the synthetic POBJ has to round-trip
//! through `Dat::parse` to be inspectable).

use hsdraw_core::accessor::Accessor;
use hsdraw_core::common::{DObj, JObj, PObj, SObj};
use hsdraw_core::dat::{Dat, RootNode};
use hsdraw_core::gx::{GxAttribName, GxAttribType, GxPrimitiveType, PObjFlag};
use hsdraw_core::gx_dl;
use hsdraw_core::hsd_struct::HsdStruct;
use hsdraw_core::pobj_writer::{ColorFormat, MeshBuilder, NormalFormat, PosFormat, UvFormat};

/// Build a minimal `Dat` with a `scene_data` root pointing at a single
/// JObj that owns one DObj that owns the given POBJ.  The SObj /
/// JOBJDescs[] / JObjDesc / JObj plumbing matches the same layout the
/// existing `mutation::build_synthetic_tree` uses.
fn dat_with_pobj(pobj: PObj) -> Dat {
    let dobj = DObj::allocate_default();
    dobj.set_pobj(Some(pobj));

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

/// `parse → walk` helper: pulls the first POBJ off the reparsed
/// `Dat`'s scene_data tree.  Used both for DL unpacking and for
/// inspecting POBJ flags / 0x14 references in Phase 3 envelope tests.
fn first_pobj(dat: &Dat) -> PObj {
    let scene = dat.scene_data().expect("scene_data root");
    let sobj = SObj::from_struct(scene.data.clone());
    let descs = sobj.jobj_descs();
    assert_eq!(descs.len(), 1, "synthetic tree has exactly one JObjDesc");
    let root_joint = descs[0].root_joint().expect("root joint present");
    let dobj = root_joint
        .dobj()
        .expect("flags allow Dobj")
        .expect("DObj attached");
    dobj.pobj().expect("POBJ attached")
}

/// `parse → walk → unpack` chain: pulls the first POBJ off the
/// reparsed `Dat`'s scene_data tree and decodes its DL.
fn unpack_first_pobj(dat: &Dat) -> gx_dl::GxDisplayList {
    gx_dl::unpack(&first_pobj(dat)).expect("DL decodes")
}

fn round_trip(dat: Dat) -> Dat {
    let bytes = dat.write().expect("dat.write");
    Dat::parse(&bytes).expect("Dat::parse")
}

// =====================================================================
// (1) single triangle
// =====================================================================

#[test]
fn single_triangle_round_trips() {
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_triangle(0, 1, 2);
    let pobj = mb.build().expect("build");

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    // Attributes: POS only, plus the GX_VA_NULL terminator.
    assert!(dl.has_attribute(GxAttribName::GX_VA_POS));
    assert!(!dl.has_attribute(GxAttribName::GX_VA_NRM));
    assert!(!dl.has_attribute(GxAttribName::GX_VA_CLR0));
    assert!(!dl.has_attribute(GxAttribName::GX_VA_TEX0));
    assert_eq!(dl.attributes.last().unwrap().name, GxAttribName::GX_VA_NULL);
    let pos = dl
        .attributes
        .iter()
        .find(|a| a.name == GxAttribName::GX_VA_POS)
        .unwrap();
    assert_eq!(pos.kind, GxAttribType::GX_INDEX16);
    assert_eq!(pos.stride, 12);

    // 1 primitive group, TRIANGLES, 3 verts.
    assert_eq!(dl.primitives.len(), 1);
    let pg = &dl.primitives[0];
    assert_eq!(pg.primitive_type, GxPrimitiveType::Triangles);
    assert_eq!(pg.vertices.len(), 3);
    let p = &pg.vertices;
    assert_eq!(p[0].pos, [0.0, 0.0, 0.0]);
    assert_eq!(p[1].pos, [1.0, 0.0, 0.0]);
    assert_eq!(p[2].pos, [0.0, 1.0, 0.0]);
}

// =====================================================================
// (2) single quad (= 2 triangles)
// =====================================================================

#[test]
fn single_quad_round_trips_triangles_only() {
    let mut mb = MeshBuilder::new();
    // (0,0)─(1,0)
    //   │     │
    // (0,1)─(1,1)
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_position(1.0, 1.0, 0.0);
    mb.add_triangle(0, 1, 2);
    mb.add_triangle(1, 3, 2);
    // Phase 1 path: force the single-`Triangles`-group emit so this
    // test keeps pinning that bytecode shape.  The Phase 2 strip
    // version of the same input is covered by `single_quad_with_strips_round_trips`.
    mb.set_use_triangle_strips(false);
    let pobj = mb.build().expect("build");

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    assert_eq!(dl.primitives.len(), 1);
    let pg = &dl.primitives[0];
    assert_eq!(pg.primitive_type, GxPrimitiveType::Triangles);
    assert_eq!(pg.vertices.len(), 6);

    // Tri 0: 0, 1, 2
    assert_eq!(pg.vertices[0].pos, [0.0, 0.0, 0.0]);
    assert_eq!(pg.vertices[1].pos, [1.0, 0.0, 0.0]);
    assert_eq!(pg.vertices[2].pos, [0.0, 1.0, 0.0]);
    // Tri 1: 1, 3, 2
    assert_eq!(pg.vertices[3].pos, [1.0, 0.0, 0.0]);
    assert_eq!(pg.vertices[4].pos, [1.0, 1.0, 0.0]);
    assert_eq!(pg.vertices[5].pos, [0.0, 1.0, 0.0]);
}

// =====================================================================
// (3) +normal +color
// =====================================================================

#[test]
fn with_normal_and_color_round_trips() {
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_normal(0.0, 0.0, 1.0);
    mb.add_normal(0.0, 0.0, 1.0);
    mb.add_normal(0.0, 0.0, 1.0);
    // 0xFF0000FF = red, full alpha
    mb.add_color(0xFF, 0x00, 0x00, 0xFF);
    mb.add_color(0xFF, 0x00, 0x00, 0xFF);
    mb.add_color(0xFF, 0x00, 0x00, 0xFF);
    mb.add_triangle(0, 1, 2);
    let pobj = mb.build().expect("build");

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    assert!(dl.has_attribute(GxAttribName::GX_VA_POS));
    assert!(dl.has_attribute(GxAttribName::GX_VA_NRM));
    assert!(dl.has_attribute(GxAttribName::GX_VA_CLR0));

    let v = &dl.primitives[0].vertices;
    assert_eq!(v.len(), 3);
    for vert in v {
        assert_eq!(vert.nrm, [0.0, 0.0, 1.0]);
        // Red channel is 1.0, others are 0; alpha is 1.0.
        assert!((vert.clr0[0] - 1.0).abs() < 1e-6);
        assert!(vert.clr0[1].abs() < 1e-6);
        assert!(vert.clr0[2].abs() < 1e-6);
        assert!((vert.clr0[3] - 1.0).abs() < 1e-6);
    }
}

// =====================================================================
// (4) +UV
// =====================================================================

#[test]
fn with_uv_round_trips() {
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_uv(0.0, 0.0);
    mb.add_uv(1.0, 0.0);
    mb.add_uv(0.0, 1.0);
    mb.add_triangle(0, 1, 2);
    let pobj = mb.build().expect("build");

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    assert!(dl.has_attribute(GxAttribName::GX_VA_TEX0));
    let v = &dl.primitives[0].vertices;
    assert_eq!(v.len(), 3);
    assert_eq!(v[0].tex0, [0.0, 0.0]);
    assert_eq!(v[1].tex0, [1.0, 0.0]);
    assert_eq!(v[2].tex0, [0.0, 1.0]);
}

// =====================================================================
// (5) Blender ribbon — 96 verts / 96 tris
//
// Models a half of a thin oval racetrack ring as a triangle strip
// expanded into individual triangles (= what the Phase 1 writer would
// receive from a Blender mesh that has had its tristrips already
// triangulated by `bpy_extras` / mesh.calc_loop_triangles()).
//
// Geometry: 48 segments around half an ellipse, 2 verts per segment
// (inner + outer rim), so 96 verts total.  Triangles: 47 quads × 2 +
// 2 caps = 96.  The exact numbers are tuned so the test exercises:
//   - DL `count` field crosses the 1-byte boundary (255)
//   - per-attribute buffer hits multiple 0x20-aligned regions
// =====================================================================

#[test]
fn ribbon_round_trips() {
    let mut mb = MeshBuilder::new();
    let segs: usize = 48;
    let inner_r = 5.0f32;
    let outer_r = 6.0f32;
    let two_pi = std::f32::consts::TAU;
    // 48 + 0 = 48 outer + 48 inner = 96 verts (no closing seam — the
    // far side of the ring is implied by the strip wrap)
    for s in 0..segs {
        let t = (s as f32) / (segs as f32);
        let theta = t * (two_pi / 2.0); // half-ring
        let (st, ct) = theta.sin_cos();
        // Outer rim
        mb.add_position(outer_r * ct, outer_r * st, 0.0);
        // Inner rim
        mb.add_position(inner_r * ct, inner_r * st, 0.0);
    }
    let n = (segs * 2) as u32;
    assert_eq!(n, 96);

    // Tri-strip expansion: for each pair of adjacent segments (i, i+1)
    // emit two triangles forming a quad.  47 quads × 2 = 94 tris.  Add
    // 2 cap tris at each end so we hit exactly 96.
    for i in 0..(segs - 1) as u32 {
        let a = 2 * i;
        let b = 2 * i + 1;
        let c = 2 * i + 2;
        let d = 2 * i + 3;
        // (a, c, b) and (b, c, d)
        mb.add_triangle(a, c, b);
        mb.add_triangle(b, c, d);
    }
    // Two cap tris (degenerate-but-legal — using the existing 4 corners)
    mb.add_triangle(0, 1, 2);
    mb.add_triangle((n - 4) as u32, (n - 2) as u32, (n - 3) as u32);

    assert_eq!(mb.vertex_count(), 96);
    assert_eq!(mb.triangle_count(), 96);
    // Force Phase 1 emit so this test pins the single-`Triangles`-
    // group shape.  The strip version is covered separately.
    mb.set_use_triangle_strips(false);
    let pobj = mb.build().expect("build");
    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    assert_eq!(dl.primitives.len(), 1);
    let pg = &dl.primitives[0];
    assert_eq!(pg.primitive_type, GxPrimitiveType::Triangles);
    assert_eq!(pg.vertices.len(), 96 * 3, "96 tris × 3 verts in DL stream");

    // Spot-check: vertex 0 is outer-rim at theta=0 → (outer_r, 0, 0).
    assert!((pg.vertices[0].pos[0] - outer_r).abs() < 1e-4);
    assert!(pg.vertices[0].pos[1].abs() < 1e-4);
}

// =====================================================================
// Phase 2 strip tests
// =====================================================================

/// Helper: count total decoded triangles (across all primitive groups
/// in a DL).  Strips of N verts decode to N-2 tris; TRIANGLES groups
/// of T verts decode to T/3 tris.
fn decoded_tri_count(dl: &gx_dl::GxDisplayList) -> usize {
    dl.primitives
        .iter()
        .map(|pg| match pg.primitive_type {
            GxPrimitiveType::TriangleStrip => pg.vertices.len().saturating_sub(2),
            GxPrimitiveType::Triangles => pg.vertices.len() / 3,
            _ => 0,
        })
        .sum()
}

/// Helper: build the same ribbon mesh used in `ribbon_round_trips`.
fn build_ribbon_mb() -> MeshBuilder {
    let mut mb = MeshBuilder::new();
    let segs: usize = 48;
    let inner_r = 5.0f32;
    let outer_r = 6.0f32;
    let two_pi = std::f32::consts::TAU;
    for s in 0..segs {
        let t = (s as f32) / (segs as f32);
        let theta = t * (two_pi / 2.0);
        let (st, ct) = theta.sin_cos();
        mb.add_position(outer_r * ct, outer_r * st, 0.0);
        mb.add_position(inner_r * ct, inner_r * st, 0.0);
    }
    let n = (segs * 2) as u32;
    for i in 0..(segs - 1) as u32 {
        let a = 2 * i;
        let b = 2 * i + 1;
        let c = 2 * i + 2;
        let d = 2 * i + 3;
        mb.add_triangle(a, c, b);
        mb.add_triangle(b, c, d);
    }
    mb.add_triangle(0, 1, 2);
    mb.add_triangle(n - 4, n - 2, n - 3);
    mb
}

#[test]
fn single_tri_stays_in_triangles_with_strips_on() {
    // 1 triangle = 3 verts.  Min strip length is 4, so a lone tri
    // can't be promoted to a strip — it stays in the TRIANGLES
    // leftover bucket.
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_triangle(0, 1, 2);
    // Default: strips ON.
    let pobj = mb.build().expect("build");
    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);
    assert_eq!(dl.primitives.len(), 1);
    assert_eq!(dl.primitives[0].primitive_type, GxPrimitiveType::Triangles);
    assert_eq!(dl.primitives[0].vertices.len(), 3);
}

#[test]
fn single_quad_with_strips_round_trips() {
    // Same 2-tri quad as the Phase 1 test, but with strips ON: the
    // two triangles share an edge, so the stripper packs them into
    // one TriangleStrip of 4 verts, halving the index payload.
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_position(1.0, 1.0, 0.0);
    mb.add_triangle(0, 1, 2);
    mb.add_triangle(1, 3, 2);
    let pobj = mb.build().expect("build");
    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    assert_eq!(dl.primitives.len(), 1);
    let pg = &dl.primitives[0];
    assert_eq!(
        pg.primitive_type,
        GxPrimitiveType::TriangleStrip,
        "two-tri shared-edge mesh should become a single TriangleStrip"
    );
    assert_eq!(pg.vertices.len(), 4);
    assert_eq!(decoded_tri_count(&dl), 2);

    // Verify all 4 input positions appear in the strip output.
    let positions: std::collections::HashSet<[u32; 3]> = pg
        .vertices
        .iter()
        .map(|v| {
            [
                v.pos[0].to_bits(),
                v.pos[1].to_bits(),
                v.pos[2].to_bits(),
            ]
        })
        .collect();
    assert_eq!(positions.len(), 4, "strip vertex set should be 4 unique positions");
}

#[test]
fn ribbon_with_strips_round_trips() {
    let mb = build_ribbon_mb();
    let pobj = mb.build().expect("build");
    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    // The greedy stripper produces at least one strip; the total
    // decoded triangle count must match the input regardless of how
    // the primitives split.
    assert_eq!(decoded_tri_count(&dl), 96, "input had 96 tris");
    assert!(
        dl.primitives
            .iter()
            .any(|pg| pg.primitive_type == GxPrimitiveType::TriangleStrip),
        "ribbon should produce at least one TriangleStrip"
    );
}

#[test]
fn strips_shrink_dl_byte_size_for_ribbon() {
    // Same ribbon, two builds (strips on / off), compare the resulting
    // DL byte size.  The strip path *must* produce a smaller payload —
    // that's the whole point of Phase 2.
    let mut mb_off = build_ribbon_mb();
    mb_off.set_use_triangle_strips(false);
    let pobj_off = mb_off.build().expect("build off");

    let mb_on = build_ribbon_mb();
    let pobj_on = mb_on.build().expect("build on");

    let size_off = pobj_off
        .display_list_size()
        .expect("dl size off") as usize;
    let size_on = pobj_on
        .display_list_size()
        .expect("dl size on") as usize;

    assert!(
        size_on < size_off,
        "strip path ({} bytes) should be smaller than triangles path ({} bytes)",
        size_on,
        size_off
    );
}

// =====================================================================
// Phase 3 envelope rigging tests
// =====================================================================

#[test]
fn envelope_round_trips() {
    // Quad whose first 2 verts use envelope 0, last 2 use envelope 1.
    // Envelope 0: 100% bone A; envelope 1: 50/50 bone A and bone B.
    let bone_a = JObj::allocate_default();
    bone_a.set_tx(10.0).unwrap();
    let bone_b = JObj::allocate_default();
    bone_b.set_tx(20.0).unwrap();

    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_position(1.0, 1.0, 0.0);
    let env0 = mb.add_envelope(vec![(bone_a.clone(), 1.0)]);
    let env1 = mb.add_envelope(vec![(bone_a.clone(), 0.5), (bone_b.clone(), 0.5)]);
    mb.add_envelope_index(env0);
    mb.add_envelope_index(env0);
    mb.add_envelope_index(env1);
    mb.add_envelope_index(env1);
    mb.add_triangle(0, 1, 2);
    mb.add_triangle(1, 3, 2);
    // Force Phase 1-style emit so we can pin the per-vertex DL bytes.
    mb.set_use_triangle_strips(false);
    let pobj = mb.build().expect("build");

    // Pre-write inspection: ENVELOPE flag set, 0x14 ref present.
    assert!(
        pobj.flags()
            .unwrap()
            .intersects(PObjFlag::ENVELOPE),
        "ENVELOPE flag should be set on a rigged POBJ"
    );

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    // Attribute table must declare PNMTXIDX as GX_DIRECT *first*.
    assert_eq!(dl.attributes[0].name, GxAttribName::GX_VA_PNMTXIDX);
    assert_eq!(dl.attributes[0].kind, GxAttribType::GX_DIRECT);
    assert!(dl.has_attribute(GxAttribName::GX_VA_POS));

    // Decoded vertex stream: 6 entries (2 tris × 3 verts).  Vert→env
    // mapping is 0/0/1/1 → matrix slots 0/0/3/3.  Triangles are
    // (0,1,2)+(1,3,2), so the DL stream's PNMTXIDX values are:
    //   tri0: env(0)=0, env(1)=0, env(2)=3
    //   tri1: env(1)=0, env(3)=3, env(2)=3
    let v = &dl.primitives[0].vertices;
    assert_eq!(v.len(), 6);
    let expected_pn = [0u16, 0, 3, 0, 3, 3];
    for (i, vert) in v.iter().enumerate() {
        assert_eq!(
            vert.pn_mtx_idx, expected_pn[i],
            "vert {} pn_mtx_idx mismatch",
            i
        );
    }

    // Envelope array round-trips: walk POBJ.0x14 → arr[0] → entry[0] →
    // bone_a (verify its tx == 10.0).
    let pobj2 = first_pobj(&dat);
    let arr_struct = pobj2
        .0
        .borrow()
        .get_reference(0x14)
        .expect("envelope array attached");
    let env0_struct = arr_struct
        .borrow()
        .get_reference(0)
        .expect("envelope[0] present");
    let env0_jobj_struct = env0_struct
        .borrow()
        .get_reference(0)
        .expect("envelope[0] entry[0] jobj ref");
    let recovered = JObj::from_struct(env0_jobj_struct);
    assert!((recovered.tx().unwrap() - 10.0).abs() < 1e-6);
    let env0_weight = env0_struct.borrow().get_f32(4).unwrap();
    assert!((env0_weight - 1.0).abs() < 1e-6);

    // envelope[1] entry[1] should be bone_b with weight 0.5.
    let env1_struct = arr_struct
        .borrow()
        .get_reference(4)
        .expect("envelope[1] present");
    let env1_e1_jobj_struct = env1_struct
        .borrow()
        .get_reference(8)
        .expect("envelope[1] entry[1] jobj");
    let recovered2 = JObj::from_struct(env1_e1_jobj_struct);
    assert!((recovered2.tx().unwrap() - 20.0).abs() < 1e-6);
    let env1_e1_weight = env1_struct.borrow().get_f32(8 + 4).unwrap();
    assert!((env1_e1_weight - 0.5).abs() < 1e-6);
}

#[test]
fn envelope_index_count_mismatch_rejected() {
    let bone = JObj::allocate_default();
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    let env = mb.add_envelope(vec![(bone, 1.0)]);
    mb.add_envelope_index(env); // 1 idx, 3 verts
    mb.add_triangle(0, 1, 2);
    assert!(mb.build().is_err());
}

#[test]
fn envelope_indices_without_envelopes_rejected() {
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_envelope_index(0);
    mb.add_envelope_index(0);
    mb.add_envelope_index(0);
    mb.add_triangle(0, 1, 2);
    assert!(mb.build().is_err());
}

#[test]
fn envelope_index_out_of_range_rejected() {
    let bone = JObj::allocate_default();
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_envelope(vec![(bone, 1.0)]); // only env 0 exists
    mb.add_envelope_index(0);
    mb.add_envelope_index(5); // OOB
    mb.add_envelope_index(0);
    mb.add_triangle(0, 1, 2);
    assert!(mb.build().is_err());
}

#[test]
fn envelope_with_triangle_strips() {
    // Same envelope-rigged quad as `envelope_round_trips`, but with
    // strips enabled.  The two tris form a strip of 4 verts; PNMTXIDX
    // is per-vertex regardless of primitive type, so the per-vertex
    // 1-byte PNMTXIDX still rides alongside POS/NRM index16.
    let bone_a = JObj::allocate_default();
    bone_a.set_tx(7.0).unwrap();

    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_position(1.0, 1.0, 0.0);
    let env = mb.add_envelope(vec![(bone_a, 1.0)]);
    for _ in 0..4 {
        mb.add_envelope_index(env);
    }
    mb.add_triangle(0, 1, 2);
    mb.add_triangle(1, 3, 2);
    let pobj = mb.build().expect("build");

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    // PNMTXIDX is the first attribute, in DIRECT mode.
    assert_eq!(dl.attributes[0].name, GxAttribName::GX_VA_PNMTXIDX);
    assert_eq!(dl.attributes[0].kind, GxAttribType::GX_DIRECT);

    // 1 strip of 4 verts; all PNMTXIDX = 0 (env 0 → slot 0).
    let pg = &dl.primitives[0];
    assert_eq!(pg.primitive_type, GxPrimitiveType::TriangleStrip);
    assert_eq!(pg.vertices.len(), 4);
    for vert in &pg.vertices {
        assert_eq!(vert.pn_mtx_idx, 0);
    }
}

#[test]
fn deprecated_set_cull_back_does_not_write_pobj_flags() {
    // Pin the #9 deprecation: set_cull_back / set_cull_front are now
    // no-ops at the POBJ.flags level — the historical 0x4000 / 0x8000
    // bits collide with POBJ_TYPE_MASK and POBJ_FLAG.ENVELOPE.  After
    // calling both setters and building, the POBJ.flags word must NOT
    // carry either bit.
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_triangle(0, 1, 2);
    #[allow(deprecated)]
    {
        mb.set_cull_back(true);
        mb.set_cull_front(true);
    }
    let pobj = mb.build().expect("build");
    let bits = pobj.flags().expect("flags").bits();
    assert_eq!(
        bits & 0x4000,
        0,
        "set_cull_back must not set POBJ.flags bit 0x4000 (got 0x{:04X})",
        bits
    );
    assert_eq!(
        bits & 0x8000,
        0,
        "set_cull_front must not set POBJ.flags bit 0x8000 (got 0x{:04X})",
        bits
    );
}

#[test]
fn pos_mat_idx_emits_pnmtxidx_without_envelope_flag() {
    // Pin #3: set_use_pos_mat_idx(true) + add_pos_mat_idx(byte) emits
    // a DIRECT 1-byte GX_VA_PNMTXIDX attribute for every vertex
    // *without* setting POBJ_FLAG.ENVELOPE.  Verifies:
    //   - POBJ.flags has bit 0x8000 (ENVELOPE) clear
    //   - DL bytecode contains the per-vertex matrix index byte we set
    //   - 0x14 reference (where envelope-pointer-array would live) is null
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_triangle(0, 1, 2);
    mb.set_use_pos_mat_idx(true);
    mb.add_pos_mat_idx(0);
    mb.add_pos_mat_idx(0);
    mb.add_pos_mat_idx(0);
    let pobj = mb.build().expect("build");
    assert_eq!(
        pobj.flags().unwrap().bits() & 0x8000,
        0,
        "ENVELOPE bit must be clear"
    );
    let s = pobj.0.borrow();
    assert!(
        s.get_reference(0x14).is_none(),
        "0x14 (envelope array) must be null without envelopes"
    );
    drop(s);

    // The DL bytecode for one Triangles group of 3 verts should be:
    //   0x90 (TRIANGLES) + 0x0003 (BE u16 vert count)
    //   per-vertex: 1 byte PNMTXIDX + 2 bytes POS index BE = 3 bytes
    //   tail: 0x00 EOF + 0x20 alignment padding
    let dl = pobj.display_list_buffer().expect("dl");
    assert_eq!(dl[0], 0x90, "primitive header is TRIANGLES");
    assert_eq!(&dl[1..3], &0x0003u16.to_be_bytes(), "vertex count u16");
    // Per-vertex stream: 3 verts × (1 PNMTX + 2 POS) = 9 bytes
    // Bytes 3, 6, 9 should be the PNMTX bytes (0 each); bytes
    // 4..6 / 7..9 / 10..12 are the POS indices 0/1/2 BE.
    assert_eq!(dl[3], 0, "vert 0 PNMTXIDX byte");
    assert_eq!(&dl[4..6], &0x0000u16.to_be_bytes(), "vert 0 POS idx");
    assert_eq!(dl[6], 0, "vert 1 PNMTXIDX byte");
    assert_eq!(&dl[7..9], &0x0001u16.to_be_bytes(), "vert 1 POS idx");
    assert_eq!(dl[9], 0, "vert 2 PNMTXIDX byte");
    assert_eq!(&dl[10..12], &0x0002u16.to_be_bytes(), "vert 2 POS idx");
}

#[test]
fn pos_mat_idx_byte_value_round_trips_in_dl() {
    // Same shape as above but with non-zero PNMTXIDX values to
    // confirm the raw byte is written verbatim (no `* 3` multiplier
    // — that's the envelope path's HSDLib convention).
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_triangle(0, 1, 2);
    mb.add_pos_mat_idx(15);
    mb.add_pos_mat_idx(33);
    mb.add_pos_mat_idx(7);
    let pobj = mb.build().expect("build");
    let dl = pobj.display_list_buffer().expect("dl");
    assert_eq!(dl[3], 15, "vert 0 raw PNMTXIDX byte");
    assert_eq!(dl[6], 33, "vert 1 raw PNMTXIDX byte");
    assert_eq!(dl[9], 7, "vert 2 raw PNMTXIDX byte");
}

#[test]
fn pos_mat_idx_count_mismatch_errors() {
    // Validation: pos_mat_idx count must match positions count.
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_triangle(0, 1, 2);
    mb.add_pos_mat_idx(0);
    mb.add_pos_mat_idx(0);
    // missing 3rd
    let err = mb.build().unwrap_err();
    assert!(
        err.to_string().contains("pos_mat_idx count != position count"),
        "expected count mismatch error, got: {}",
        err
    );
}

#[test]
fn pos_mat_idx_and_envelopes_are_mutually_exclusive() {
    // Validation: combining envelopes with pos_mat_idx is rejected.
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_triangle(0, 1, 2);
    let jobj = hsdraw_core::common::JObj::allocate_default();
    mb.add_envelope(vec![(jobj, 1.0)]);
    mb.add_envelope_index(0);
    mb.add_envelope_index(0);
    mb.add_envelope_index(0);
    mb.set_use_pos_mat_idx(true);
    mb.add_pos_mat_idx(0);
    mb.add_pos_mat_idx(0);
    mb.add_pos_mat_idx(0);
    let err = mb.build().unwrap_err();
    assert!(
        err.to_string().contains("mutually exclusive"),
        "expected mutual-exclusion error, got: {}",
        err
    );
}

#[test]
fn pobj_flags_setter_writes_arbitrary_u16() {
    // Pin #1: PObj::set_flags writes the raw u16 verbatim, including
    // bit patterns HSDLib's POBJ_FLAG enum doesn't model cleanly (e.g.
    // 0x8000 = ENVELOPE on a POBJ that has no envelope-pointer array).
    // Tests both the in-memory write and survival through writer
    // round-trip.
    use hsdraw_core::gx::PObjFlag;
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_triangle(0, 1, 2);
    let pobj = mb.build().expect("build");
    pobj.set_flags(PObjFlag::from_bits_retain(0x8000)).expect("set_flags");
    assert_eq!(pobj.flags().unwrap().bits(), 0x8000);

    // Round-trip through writer + re-parse; flag must survive verbatim.
    let dat = round_trip(dat_with_pobj(pobj));
    let pobj2 = first_pobj(&dat);
    assert_eq!(
        pobj2.flags().unwrap().bits(),
        0x8000,
        "POBJ.flags=0x8000 must round-trip through writer + reader"
    );
}

// =====================================================================
// Attribute format builder (Phase 7) — API back-compat gate.
//
// The format setters (`set_pos_format` / `set_normal_format` /
// `set_color_format` / `set_uv_format`) accept enums whose default
// variants reproduce the pre-Phase-7 hard-coded behavior (`F32×3` /
// `RGBA8` / `F32×2`).  Calling each setter with its default value must
// produce a byte-identical DAT to the no-setter path.  Non-default
// variants (S16x3 / S8x3 / Rgb565 / Rgba4 / S16x2 / S8x2) are reserved
// placeholders that `build()` rejects until the per-format encoders
// land (todo.md §2.7).
// =====================================================================

#[test]
fn format_setters_with_defaults_match_no_setter_path() {
    // No-setter baseline: a tiny POS-only triangle.
    let mut mb0 = MeshBuilder::new();
    mb0.add_position(0.0, 0.0, 0.0);
    mb0.add_position(1.0, 0.0, 0.0);
    mb0.add_position(0.0, 1.0, 0.0);
    mb0.add_triangle(0, 1, 2);
    let pobj0 = mb0.build().expect("build baseline");
    let bytes0 = dat_with_pobj(pobj0).write().expect("write baseline");

    // With every setter called explicitly with the default enum
    // variant.  Output must match baseline exactly.
    let mut mb1 = MeshBuilder::new();
    mb1.add_position(0.0, 0.0, 0.0);
    mb1.add_position(1.0, 0.0, 0.0);
    mb1.add_position(0.0, 1.0, 0.0);
    mb1.add_triangle(0, 1, 2);
    mb1.set_pos_format(PosFormat::F32x3);
    mb1.set_normal_format(NormalFormat::F32x3);
    mb1.set_color_format(ColorFormat::Rgba8);
    mb1.set_uv_format(UvFormat::F32x2);
    let pobj1 = mb1.build().expect("build with default setters");
    let bytes1 = dat_with_pobj(pobj1).write().expect("write with setters");

    assert_eq!(
        bytes0, bytes1,
        "default-format setters must produce byte-identical output"
    );
}

/// `S16×3` quantized POS: `round(v * (1 << exponent))` clamped to i16,
/// the GX `Scale` field set to `exponent` so the decoder's
/// `raw / (1 << scale)` step recovers the original.  We use exponent=8
/// (= 1/256 unit) which fits the [-1, +1] inputs in this test with
/// ~0.004 quantization error per component.
#[test]
fn pos_format_s16x3_round_trips_via_attribute_scale() {
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.5, -0.25);
    mb.add_position(-0.75, 0.125, 0.0);
    mb.add_triangle(0, 1, 2);
    mb.set_pos_format(PosFormat::S16x3 { exponent: 8 });
    let pobj = mb.build().expect("build with S16x3");

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    let pos = dl
        .attributes
        .iter()
        .find(|a| a.name == GxAttribName::GX_VA_POS)
        .expect("POS attribute");
    assert_eq!(pos.stride, 6, "S16x3 stride is 6 bytes");
    // GxCompType::Int16 = 3.
    assert_eq!(pos.comp_type, 3);
    assert_eq!(pos.scale, 8);

    let v = &dl.primitives[0].vertices;
    // 1/256 quantization: error per component is at most 0.5 LSB ≈ 0.002.
    let eps = 1.0 / 256.0;
    for (got, want) in v
        .iter()
        .map(|vert| vert.pos)
        .zip([[0.0, 0.0, 0.0], [1.0, 0.5, -0.25], [-0.75, 0.125, 0.0]])
    {
        for axis in 0..3 {
            assert!(
                (got[axis] - want[axis]).abs() <= eps,
                "axis {} differs by more than 1 LSB: got {} want {}",
                axis,
                got[axis],
                want[axis]
            );
        }
    }
}

/// `S8×3` quantized POS: same scheme as S16x3 but `i8` per component.
/// Range is much tighter — `exponent=4` (= 1/16 unit) covers [-8, +8]
/// with ~0.06 max quantization error.
#[test]
fn pos_format_s8x3_round_trips_via_attribute_scale() {
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.5, -0.25);
    mb.add_position(-0.75, 0.125, 0.0);
    mb.add_triangle(0, 1, 2);
    mb.set_pos_format(PosFormat::S8x3 { exponent: 4 });
    let pobj = mb.build().expect("build with S8x3");

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    let pos = dl
        .attributes
        .iter()
        .find(|a| a.name == GxAttribName::GX_VA_POS)
        .expect("POS attribute");
    assert_eq!(pos.stride, 3);
    // GxCompType::Int8 = 1.
    assert_eq!(pos.comp_type, 1);
    assert_eq!(pos.scale, 4);

    let v = &dl.primitives[0].vertices;
    let eps = 1.0 / 16.0;
    for (got, want) in v
        .iter()
        .map(|vert| vert.pos)
        .zip([[0.0, 0.0, 0.0], [1.0, 0.5, -0.25], [-0.75, 0.125, 0.0]])
    {
        for axis in 0..3 {
            assert!(
                (got[axis] - want[axis]).abs() <= eps,
                "S8x3 axis {} differs by more than 1 LSB: got {} want {}",
                axis,
                got[axis],
                want[axis]
            );
        }
    }
}

/// `S16×2` quantized TEX0 — covers the UV path of the quantization
/// machinery (independent of POS / NRM since the buffer layout is
/// 2-component there).  exponent=12 covers [0, 8] UV space with 1/4096
/// LSB precision.
#[test]
fn uv_format_s16x2_round_trips_via_attribute_scale() {
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_uv(0.0, 0.0);
    mb.add_uv(1.0, 0.5);
    mb.add_uv(0.5, 1.0);
    mb.add_triangle(0, 1, 2);
    mb.set_uv_format(UvFormat::S16x2 { exponent: 12 });
    let pobj = mb.build().expect("build with S16x2");

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    let tex0 = dl
        .attributes
        .iter()
        .find(|a| a.name == GxAttribName::GX_VA_TEX0)
        .expect("TEX0 attribute");
    assert_eq!(tex0.stride, 4);
    assert_eq!(tex0.comp_type, 3);
    assert_eq!(tex0.scale, 12);

    let v = &dl.primitives[0].vertices;
    let eps = 1.0 / 4096.0;
    for (got, want) in v
        .iter()
        .map(|vert| vert.tex0)
        .zip([[0.0, 0.0], [1.0, 0.5], [0.5, 1.0]])
    {
        for axis in 0..2 {
            assert!(
                (got[axis] - want[axis]).abs() <= eps,
                "TEX0 axis {} differs by more than 1 LSB: got {} want {}",
                axis,
                got[axis],
                want[axis]
            );
        }
    }
}

/// `RGB565` CLR0 — `r5g6b5` bit-packed RGB; alpha is dropped (== 1.0 on
/// decode).  Channel precision: 1/31 for R/B, 1/63 for G.
#[test]
fn color_format_rgb565_round_trips() {
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    // Pure red / green / blue + full alpha (alpha will be dropped).
    mb.add_color(0xFF, 0x00, 0x00, 0xFF);
    mb.add_color(0x00, 0xFF, 0x00, 0xFF);
    mb.add_color(0x00, 0x00, 0xFF, 0xFF);
    mb.add_triangle(0, 1, 2);
    mb.set_color_format(ColorFormat::Rgb565);
    let pobj = mb.build().expect("build with Rgb565");

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    let clr = dl
        .attributes
        .iter()
        .find(|a| a.name == GxAttribName::GX_VA_CLR0)
        .expect("CLR0 attribute");
    assert_eq!(clr.stride, 2);
    assert_eq!(clr.comp_type, 0, "RGB565 = comp_type 0");
    assert_eq!(clr.comp_count, 0, "RGB = comp_count 0");

    let v = &dl.primitives[0].vertices;
    // 5-bit channel: highest representable is 31<<3 = 248 / 255 ≈ 0.972.
    // Sufficient tolerance for "primary color stays the dominant channel".
    let r_eps = 1.0 / 31.0;
    let g_eps = 1.0 / 63.0;
    // Pixel 0: red dominant.
    assert!(v[0].clr0[0] > 1.0 - r_eps, "red[0]={}", v[0].clr0[0]);
    assert!(v[0].clr0[1] < r_eps);
    assert!(v[0].clr0[2] < r_eps);
    assert!((v[0].clr0[3] - 1.0).abs() < 1e-6, "alpha dropped → 1.0");
    // Pixel 1: green dominant.
    assert!(v[1].clr0[0] < g_eps);
    assert!(v[1].clr0[1] > 1.0 - g_eps);
    assert!(v[1].clr0[2] < g_eps);
    // Pixel 2: blue dominant.
    assert!(v[2].clr0[0] < r_eps);
    assert!(v[2].clr0[1] < g_eps);
    assert!(v[2].clr0[2] > 1.0 - r_eps);
}

/// `RGBA4` CLR0 — top 4 bits per channel.  Channel precision: 1/15.
#[test]
fn color_format_rgba4_round_trips() {
    let mut mb = MeshBuilder::new();
    mb.add_position(0.0, 0.0, 0.0);
    mb.add_position(1.0, 0.0, 0.0);
    mb.add_position(0.0, 1.0, 0.0);
    mb.add_color(0xFF, 0x00, 0x80, 0xFF);
    mb.add_color(0x00, 0xFF, 0x00, 0x80);
    mb.add_color(0x40, 0x40, 0x40, 0x40);
    mb.add_triangle(0, 1, 2);
    mb.set_color_format(ColorFormat::Rgba4);
    let pobj = mb.build().expect("build with Rgba4");

    let dat = round_trip(dat_with_pobj(pobj));
    let dl = unpack_first_pobj(&dat);

    let clr = dl
        .attributes
        .iter()
        .find(|a| a.name == GxAttribName::GX_VA_CLR0)
        .expect("CLR0 attribute");
    assert_eq!(clr.stride, 2);
    assert_eq!(clr.comp_type, 3);
    assert_eq!(clr.comp_count, 1);

    // 4-bit channel: decoder formula is `(nibble<<4) / 255` → 1 LSB ≈
    // 16/255 ≈ 0.063.
    let eps = 16.0 / 255.0;
    let want = [
        [0xFF, 0x00, 0x80, 0xFF],
        [0x00, 0xFF, 0x00, 0x80],
        [0x40, 0x40, 0x40, 0x40],
    ];
    for (vert, raw) in dl.primitives[0].vertices.iter().zip(want) {
        for i in 0..4 {
            let want_f = (raw[i] & 0xF0) as f32 / 255.0;
            assert!(
                (vert.clr0[i] - want_f).abs() <= eps,
                "rgba4 channel {} got {} want {}",
                i,
                vert.clr0[i],
                want_f
            );
        }
    }
}
