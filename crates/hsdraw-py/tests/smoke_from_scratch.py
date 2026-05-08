"""End-to-end Python smoke for the from-scratch synthesis pipeline.

Mirrors ``crates/hsdraw-core/tests/from_scratch.rs`` at the PyO3 layer:
``Dat.alloc_scene_data`` → root JObj → DObj → MObj → Material → TObj →
Image (with ``gx_encode``-produced RGBA8 payload) → write → re-parse →
walk and assert each link is intact.

Run via the CI ``maturin develop smoke`` job after the wheel installs;
no pytest dependency, just plain ``python tests/smoke_from_scratch.py``.
"""

import hsdraw


def main() -> None:
    # Module-level sanity (matches the existing inline smoke).
    print("version:", hsdraw.version())
    assert callable(hsdraw.parse_dat)
    assert callable(hsdraw.write_dat)
    assert callable(hsdraw.export_scene_json)
    assert callable(hsdraw.gx_encode)

    # ---- Phase 1: scaffold ---------------------------------------
    dat = hsdraw.Dat.alloc_scene_data()
    assert dat.root_names() == ["scene_data"]

    sobj = hsdraw.SObj.from_struct(dat.scene_data().data)
    descs = sobj.jobj_descs()
    assert len(descs) == 1
    root = descs[0].root_joint
    assert root is not None
    assert abs(root.sx - 1.0) < 1e-6, "identity scale"

    # ---- Phase 2: encode 4×4 RGBA8 → RGB565 ----------------------
    src_rgba = bytearray()
    for y in range(4):
        for x in range(4):
            src_rgba.extend([0x10 + x * 0x20, 0x10 + y * 0x20, 0x80, 0xFF])
    gx_bytes = hsdraw.gx_encode(4, 4, 4, bytes(src_rgba))  # format 4 = RGB565
    assert len(gx_bytes) == 32, f"got {len(gx_bytes)} bytes for 4×4 RGB565"

    # ---- Phase 3: build the chain --------------------------------
    img = hsdraw.Image.alloc()
    img.width = 4
    img.height = 4
    img.format = 4  # RGB565
    img.set_image_data_bytes(gx_bytes)

    tobj = hsdraw.TObj.alloc()
    tobj.tex_map_id = 0  # GX_TEXMAP0
    tobj.set_scale(1.0, 1.0, 1.0)
    tobj.wrap_s = 1  # REPEAT
    tobj.wrap_t = 1
    tobj.set_image_data(img)

    mat = hsdraw.Material.alloc()
    mat.dif_rgba = (0xFF, 0xFF, 0xFF, 0xFF)
    mat.alpha = 1.0
    mat.shininess = 50.0

    mobj = hsdraw.MObj.alloc()
    mobj.render_flags = (1 << 4) | (1 << 2)  # TEX0 | DIFFUSE
    mobj.set_material(mat)
    mobj.set_textures(tobj)

    dobj = hsdraw.DObj.alloc()
    dobj.set_mobj(mobj)

    root.set_dobj(dobj)
    root.tx = 7.5
    root.ry = 0.25

    # ---- Phase 4: write + re-parse + walk ------------------------
    written = dat.write()
    dat2 = hsdraw.parse_dat(written)
    assert dat2.root_names() == ["scene_data"]

    sobj2 = hsdraw.SObj.from_struct(dat2.scene_data().data)
    root2 = sobj2.jobj_descs()[0].root_joint
    assert abs(root2.tx - 7.5) < 1e-5
    assert abs(root2.ry - 0.25) < 1e-5

    # Reach the Image via DObj→MObj→TObj.  PyO3 `child` getter is exposed
    # but that's None here — we set DObj on the root directly.  Use the
    # raw HsdStruct.get_reference path to extract the DObj from offset
    # 0x10, mirroring the addon's typical walk.
    dobj_struct = root2.as_struct().get_reference(0x10)
    assert dobj_struct is not None, "DObj at JObj+0x10"
    mobj_struct = dobj_struct.get_reference(0x08)
    assert mobj_struct is not None, "MObj at DObj+0x08"
    mobj2 = hsdraw.MObj.from_struct(mobj_struct)
    assert mobj2.material is not None
    assert mobj2.material.dif_rgba == (0xFF, 0xFF, 0xFF, 0xFF)
    tobj2 = mobj2.textures
    assert tobj2 is not None
    assert tobj2.tex_map_id == 0
    assert tobj2.wrap_s == 1
    assert tobj2.wrap_t == 1
    img2 = tobj2.image_data
    assert img2 is not None
    assert img2.width == 4
    assert img2.height == 4
    assert img2.format == 4  # RGB565
    payload = img2.image_data()
    assert payload == gx_bytes, "GX-encoded bytes round-trip byte-equal"

    print("from-scratch chain round-trip: OK")


if __name__ == "__main__":
    main()
