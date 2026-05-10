# `hsdraw` Python API (minimal reference)

ABI3-py37 wheel — load via the Blender add-on vendor path described in
`docs/notes/phase6.md`.  Surface mirrors HSDLib `HSDRawFile` /
`HSDRootNode` / `HSD_JOBJ` 1:1 so existing csx scripts can be ported
straight into Python without bringing project-specific schemas (e.g.
`scene.json` from mkgp2-patch) into this library.

## API table — csx (HSDLib) ↔ Python

| Python                                  | csx (HSDLib)                                              | 用途                |
|-----------------------------------------|-----------------------------------------------------------|---------------------|
| `hsdraw.parse_dat(bytes) -> Dat`        | `new HSDRawFile(bytes)`                                   | parse               |
| `Dat.roots() -> list[Root]`             | `file.Roots`                                              | iterate aliases     |
| `Dat.add_root(name, target)`            | `file.Roots.Add(new HSDRootNode { Name=…, Data=… })`      | alias add           |
| `Dat.remove_root(name) -> bool`         | `file.Roots.RemoveAt(file.Roots.FindIndex(…))`            | alias remove        |
| `Dat.rename_root(old, new) -> bool`     | `root.Name = new`                                         | alias rename        |
| `Dat.repoint_root(name, target) -> bool`| `root.Data = newAccessor`                                 | alias repoint       |
| `Dat.find_root_for(target) -> Root?`    | `file.Roots.FirstOrDefault(r => r.Data._s == s)`          | reverse lookup      |
| `Dat.scene_data() -> Root?`             | `file.Roots["scene_data"]`                                | course scene root   |
| `Dat.write(optimize=True, buffer_align=True) -> bytes` | `file.Save(stream)`                        | serialize           |
| `Root.name` (getter)                    | `HSDRootNode.Name`                                        | name                |
| `Root.data` -> `HsdStruct`              | `HSDRootNode.Data`                                        | the struct it points to |
| `JObj.alloc() -> JObj`                  | `new HSD_JOBJ()` + `SX=SY=SZ=1f`                          | alloc joint         |
| `JObj.from_struct(s) -> JObj`           | `(HSD_JOBJ) s`                                            | typed view          |
| `JObj.child` / `.next`                  | `j.Child` / `j.Next`                                      | hierarchy walk      |
| `JObj.set_child(j or None)`             | `j.Child = …`                                             | hierarchy edit      |
| `JObj.set_next(j or None)`              | `j.Next = …`                                              | hierarchy edit      |
| `JObj.set_dobj(d or None)`              | `j.Dobj = …`                                              | attach geometry/material |
| `JObj.flags` getter/setter (`u32`)      | `j.Flags` (JOBJ_FLAG)                                     | flag bits           |
| `JObj.tx` … `.sz` getter/setter         | `j.TX` … `j.SZ`                                           | local TRS           |
| `JObj.local_trs() -> tuple[9]`          | (none — convenience)                                      | snapshot            |
| `JObj.set_local_trs(tx, ty, …, sz)`     | (none — convenience)                                      | bulk write          |
| `JObj.as_struct() -> HsdStruct`         | `j._s`                                                    | drop the typed view |
| `DObj.alloc()`                          | `new HSD_DOBJ()`                                          | alloc material/mesh shell |
| `DObj.set_mobj(s or None)`              | `d.Mobj = …`                                              | material attach     |
| `DObj.set_pobj(p or None)`              | `d.Pobj = …`                                              | POBJ attach         |
| `DObj.set_next(d or None)`              | `d.Next = …`                                              | DObj chain          |
| `MeshBuilder()` + `.add_*` + `.build()` | `POBJ_Generator.CreatePOBJsFromTriangleList(…)`           | POBJ write          |
| `MeshBuilder.set_use_triangle_strips(b)` | `POBJ_Generator.UseTriangleStrips = b`                   | toggle Phase 2 opt  |
| `MeshBuilder.add_envelope([(JObj, w)])` | `HSD_Envelope.Add(jobj, weight)`                          | Phase 3 skinning    |
| `MeshBuilder.add_envelope_index(idx)`   | `GX_Vertex.PNMTXIDX`                                      | Phase 3 per-vertex bone slot |
| `Pobj.flags` / `.display_list_size`     | `p.Flags` / `p.DisplayListSize`                           | POBJ inspection     |
| `Pobj.set_next(p or None)`              | `p.Next = …`                                              | POBJ chain          |
| `MObj.alloc()` / `MObj.alloc_unlit_color(r,g,b,a)` | `new HSD_MOBJ()` / unlit preset                | material shell      |
| `MObj.set_material(m or None)`          | `m.Material = …`                                          | attach Material     |
| `MObj.set_pe_desc(p or None)`           | `m.PEDesc = …`                                            | pixel-process desc  |
| `MObj.set_textures(s or None)`          | `m.Textures = …`                                          | TObj chain attach   |
| `MObj.render_flags` getter/setter       | `m.RenderFlags` (RENDER_MODE)                             | flag bits           |
| `Material.alloc()` + `.{amb,dif,spc}_rgba` + `.alpha` + `.shininess` | `new HSD_Material { … }`              | material colors     |
| `PeDesc.alloc()` + per-byte setters     | `new HSD_PEDesc { BlendMode=…, … }`                      | PE descriptor       |
| `Dat.alloc_scene_data() -> Dat`         | `new HSDRawFile()` + manual SOBJ tree                     | from-scratch synthesis |
| `SObj.alloc()` / `.from_struct(s)`      | `new HSD_SOBJ()` / `(HSD_SOBJ) s`                         | scene-object alloc / view |
| `SObj.jobj_descs() -> [JObjDesc]`       | `sobj.JOBJDescs.Array`                                    | enumerate descriptors |
| `SObj.set_jobj_descs([JObjDesc, …])`    | `sobj.JOBJDescs = HSDNullPointerArrayAccessor.From(…)`    | replace descriptor list |
| `SObj.jobj_descs_array() -> HsdStruct?` | `sobj.JOBJDescs._s`                                       | raw array struct    |
| `JObjDesc.alloc()` / `.from_struct(s)`  | `new HSD_JOBJDesc()` / `(HSD_JOBJDesc) s`                 | descriptor alloc / view |
| `JObjDesc.root_joint` / `.set_root_joint(j)` | `desc.RootJoint` / `= j`                             | per-descriptor root |
| `TObj.alloc()` + per-field setters      | `new HSD_TOBJ { … }`                                      | texture-object alloc |
| `TObj.set_image_data(img)` / `.set_tlut_data(t)` | `tobj.ImageData = …` / `.TLUTData = …`           | image / palette refs |
| `TObj.set_coord_type(c)` / `.set_color_operation(o)` / `.set_alpha_operation(o)` | `tobj.CoordType=` / `.ColorOperation=` / `.AlphaOperation=` (nibble-preserving) | flag nibbles |
| `TObj.tex_gen_src` (property; assign `int` to set) | `tobj.GXTexGenSrc` (HSD_TOBJ offset 0x0C `u32`) | tex-gen source — raw `GXTexGenSrc` enum int (`GX_TG_POS`=0, `GX_TG_TEX0`=4, …) |
| `TObj.set_flag_bit(mask, on)` | (none — bit-RMW helper) | preserves coord/color/alpha nibbles |
| `TObj.set_lightmap_{diffuse,specular,ambient,ext,shadow}(b)` / `.set_bump(b)` | `tobj.Flags |= LIGHTMAP_*` / `BUMP` | named flag setters (RMW) |
| `TObj.is_lightmap_{diffuse,specular,ambient,ext,shadow}()` / `.is_bump()` | `(tobj.Flags & LIGHTMAP_*) != 0` | named flag getters |
| `TObj.lod_data` (property) / `.set_lod_data(lod or None)` / `.set_lod(min_filter=…, bias=…, bias_clamp=…, enable_edge_lod=…, anisotropy=…)` | `tobj.LOD = HSD_TOBJ_LOD { … }` | LOD struct attach (set_lod は alloc + attach 一括) |
| `Lod.alloc()` + `.{min_filter,bias,bias_clamp,enable_edge_lod,anisotropy}` | `new HSD_TOBJ_LOD { MinFilter=…, Bias=…, BiasClamp=…, EnableEdgeLOD=…, Anisotropy=… }` | HSD_TOBJ_LOD wrapper (size 0x10) |
| `TObj.to_dict()` / `MObj.to_dict()` / `Pobj.to_dict()` / `JObj.to_dict()` / `SObj.to_dict()` / `Lod.to_dict()` | (none — diagnostic helper) | フィールド全スナップショットを Python dict で返す (debug 時の手 parse 不要) |
| `Image.alloc()` + `.set_image_data_bytes(b)` | `new HSD_Image { ImageData = HSDStruct(b) }`         | image alloc + payload |
| `Image.{width,height,format,mipmap,min_lod,max_lod}` | `img.Width` / `.Height` / `.Format` / `.MipMap` / `.LODBias` / `.MaxLOD` | per-field setters |
| `hsdraw.gx_encode(format, w, h, rgba, swap_rb_for_rgb5a3=False) -> bytes` | `GXImageConverter.EncodeImage(GX_TF_*, w, h, rgba)` | RGBA8 → GX bytes encoder (RGBA8 / RGB565 / RGB5A3 / CMP only); `swap_rb_for_rgb5a3=True` で RGB5A3 のみ R/B を事前スワップ (BGR-order sampler 向け) |
| `hsdraw.gx_decode(format, w, h, gx_bytes, palette=None, palette_format=2) -> bytes` | `GXImageConverter.DecodeImage(GX_TF_*, w, h, raw, tlutFmt, tlutData)` | GX bytes → RGBA8 decoder (all formats, RGBA-ordered output, BGRA swap done in core) |
| `HsdStruct.byte_size()` / `.raw()`      | `_s.Length` / `_s.GetData()`                              | introspection       |
| `HsdStruct.references() -> [(off, target)]` | `_s.References`                                       | walk raw refs       |
| `HsdStruct.get_reference(offset)`       | `_s.GetReference<HSDAccessor>(offset)` (sans typed cast)  | offset lookup       |
| `HsdStruct.set_reference(offset, target_or_None)` | `_s.SetReference(offset, target)`                | deep-field repoint  |
| `HsdStruct.set_u8(offset, v)` / `.set_u16(offset, v)` / `.set_u32(offset, v)` / `.set_bytes(offset, b)` | `_s.SetByte(offset, v)` / `.SetInt16(…)` / `.SetInt32(…)` / `.SetBytes(…)` | byte-level patch primitives |
| `__eq__` / `__hash__` on JObj/HsdStruct | `obj._s == other._s` / `RuntimeHelpers.GetHashCode(_s)`   | identity comparison |

## Quick reference

### Functions

- `hsdraw.version() -> str`
- `hsdraw.parse_dat(bytes) -> Dat`
- `hsdraw.export_scene_json(bytes, source_dat="", tex_dir=None) -> str`
  — kept for callers that just want the read-side JSON output (parity-
  verified against csx `hsd_export_for_blender.csx`).
- `hsdraw.write_dat(bytes, optimize=True, buffer_align=True) -> bytes`
  — same as `Dat.write()` for callers that don't want to hold a `Dat`.
- `hsdraw.gx_encode(format, width, height, rgba, swap_rb_for_rgb5a3=False) -> bytes` —
  RGBA8 → GX-format byte payload.  `format` is the `GxTexFmt` integer
  (4=RGB565, 5=RGB5A3, 6=RGBA8, 14=CMP); other values raise
  `ValueError`.  Output is padded to the format's natural tile
  boundary; feed it into `Image.set_image_data_bytes(...)`.
  `swap_rb_for_rgb5a3=True` で RGB5A3 経路の R / B チャンネルを事前
  スワップ — BGR-order でサンプルするレンダラー向けのオプション。
  RGB5A3 以外には作用しない。
- `hsdraw.gx_decode(format, width, height, gx_bytes, palette=None,
  palette_format=2) -> bytes` — inverse of `gx_encode`, plus support
  for I4 / I8 / IA4 / IA8 / CIxx (palette mandatory for CIxx;
  `palette_format` is the `GxTlutFmt` integer, defaults to 2 = RGB5A3).
  Output is `4 * width * height` bytes RGBA8 — the Rust core mirrors
  HSDLib's BGRA→RGBA swap internally for RGBA8 / CMP, so callers
  don't need a manual swap (csx scripts on top of HSDLib do, since
  `t.GetDecodedImageData()` returns BGRA).

### Mutation primitives

The mutation surface lives on three classes — `Dat`, `Root`, `JObj` —
plus a thin `HsdStruct` for identity comparison.  Every Python handle
shares the same `Rc` as the parent `Dat`, so editing a `JObj` you
pulled out of `Dat.scene_data().data` mutates the live tree, and the
next `Dat.write()` picks it up.

`Root` is read-only (think tuple-shaped record).  Mutate aliases via
the parent `Dat` (`add_root`, `remove_root`, `rename_root`,
`repoint_root`).

## End-to-end example: alias add + TRS edit + new joint reparent

This is the full rewrite of csx `hsd_import_from_blender.csx` Pass 0–4
expressed against the Python primitive surface.  An add-on can drive
its own JSON schema interpretation in pure Python and call this for
the actual mutation:

```python
import hsdraw

# ---- parse ----------------------------------------------------------
raw  = open("base.dat", "rb").read()
dat  = hsdraw.parse_dat(raw)

# ---- DFS walk to assign jobj_N IDs (csx Pass 0 in Python) -----------
def walk_joints(scene_root):
    """Returns a {jobj_id: JObj} dict in the same DFS order csx uses."""
    out, seen, counter = {}, set(), [0]
    def visit(j):
        key = hash(j)            # JObj.__hash__ is Rc-identity
        if key in seen: return
        seen.add(key)
        jid = f"jobj_{counter[0]}"; counter[0] += 1
        out[jid] = j
        c = j.child
        while c is not None:
            visit(c); c = c.next
    # Walk via the SOBJ accessor — for now we only have the JObj typed
    # view, so SOBJ traversal is an add-on-side helper that knows the
    # JOBJDescs[] layout.  Sketch:
    #   sobj_struct = scene_root.data         # HsdStruct
    #   ... add-on parses the JOBJDescs[] array out of sobj_struct ...
    return out

# ---- alias add (csx Pass 2) -----------------------------------------
joints = walk_joints(dat.scene_data())
dat.add_root("MR_highway_inu_joint", joints["jobj_3"])

# ---- TRS edit (csx Pass 3) ------------------------------------------
joints["jobj_2"].tx = 123.5
joints["jobj_2"].set_local_trs(123.5, 0.0, 0.0,
                               0.0,   0.0, 0.25,
                               2.5,   1.0, 1.0)

# ---- alloc + reparent (csx Phase 1.x — orphan / aliased / spliced) --
new_joint = hsdraw.JObj.alloc()
new_joint.tx = 42.0
# (a) orphan: just leave it in a local — writer GCs it on save.
# (b) aliased: register as a top-level alias root.
dat.add_root("MR_highway_new_joint", new_joint)
# (c) hierarchy-spliced: prepend as parent.Child[0].
parent = joints["jobj_1"]
old_first = parent.child
new_joint.set_next(old_first)
parent.set_child(new_joint)

# ---- save (csx final step) ------------------------------------------
open("out.dat", "wb").write(dat.write())
```

The DFS walker is intentionally add-on-side: it depends on the
project's id-naming convention (`jobj_N`) which is `mkgp2-patch`'s
schema, not HSDLib's.

### Errors

`PyValueError` is raised on:

- malformed .dat (header / relocation table inconsistencies)
- TRS / flag setter on a struct shorter than 0x40 bytes (the JObj
  setters auto-grow, but a primitive HsdStruct read OOB still errors)
- `MeshBuilder.build()` called with mismatched attribute counts,
  out-of-range triangle indices, > 65,535 vertices, or no triangles

`PyTypeError`:

- `Dat.add_root` / `repoint_root` given anything that isn't a `JObj`
  or `HsdStruct`.

## End-to-end example: brand-new mesh → JObj attach → save (Phase 1)

For a Blender add-on building a fresh mesh from `bpy.data` (e.g. a UV-
mapped racetrack stub) and writing it back into an MKGP2 course .dat:

```python
import hsdraw

dat = hsdraw.parse_dat(open("base.dat", "rb").read())

# ---- 1. build the POBJ from CPU-side mesh data --------------------
mesh = hsdraw.MeshBuilder()

# 3 verts, 1 triangle.  Push positions / normals / colors / UVs
# (each optional except positions); their counts must match.
mesh.add_position(0.0, 0.0, 0.0)
mesh.add_position(1.0, 0.0, 0.0)
mesh.add_position(0.0, 1.0, 0.0)
mesh.add_normal(0.0, 0.0, 1.0)
mesh.add_normal(0.0, 0.0, 1.0)
mesh.add_normal(0.0, 0.0, 1.0)
mesh.add_color(0xFF, 0x00, 0x00, 0xFF)   # red, full alpha
mesh.add_color(0xFF, 0x00, 0x00, 0xFF)
mesh.add_color(0xFF, 0x00, 0x00, 0xFF)
mesh.add_triangle(0, 1, 2)
# NOTE: mesh.set_cull_back / set_cull_front are deprecated — POBJ.flags
# 0x4000 / 0x8000 collide with POBJ_TYPE_MASK (SHAPEANIM / ENVELOPE).
# Face culling is on the MObj/RenderFlags side, not POBJ-side.
pobj = mesh.build()                       # -> hsdraw.Pobj

# ---- 2. shell DObj (material attach is up to the caller) ----------
dobj = hsdraw.DObj.alloc()
dobj.set_pobj(pobj)
# dobj.set_mobj(my_mobj_struct)  # Phase 1 doesn't ship a MObj builder;
                                  # reuse one pulled out of an existing
                                  # course .dat or supply a raw HsdStruct.

# ---- 3. attach to a JObj and add as a top-level alias root --------
new_joint = hsdraw.JObj.alloc()
new_joint.set_dobj(dobj)
dat.add_root("MR_highway_my_mesh_joint", new_joint)

# ---- 4. save ------------------------------------------------------
open("out.dat", "wb").write(dat.write())
```

### POBJ writer capabilities (Phase 1–3)

- **Phase 1 — TRIANGLES**: minimum-friction emit path.  `0x90`
  primitive + GX_INDEX16 indices + fixed F32×3 / RGBA8 / F32×2 attrs.
- **Phase 2 — TRIANGLE_STRIP optimization (default on)**: greedy
  stripper produces `0x98 (TriangleStrip)` groups for chains of ≥ 4
  verts plus a trailing `Triangles` group for the leftover.  Toggle
  with `mb.set_use_triangle_strips(False)` if you want the predictable
  Phase 1 byte layout (e.g. for diff-against-HSDLib).
- **Phase 3 — envelope rigging**: per-vertex `add_envelope_index(i)`
  references envelopes added via `add_envelope([(jobj, weight), …])`.
  Sets `POBJ_FLAG.ENVELOPE`, emits `GX_VA_PNMTXIDX` direct attribute.
  Up to 85 envelopes per POBJ (matrix-slot range); split above that.

> **Deprecated**: `MeshBuilder.set_cull_back` / `set_cull_front` —
> the values 0x4000 / 0x8000 collide with `POBJ_TYPE_MASK`
> (`SHAPEANIM` / `ENVELOPE`).  Calling either is a no-op; the PyO3
> binding emits `DeprecationWarning`.  Express culling through the
> MObj/RenderFlags side instead.

### Limits

- ≤ 65,535 verts per `MeshBuilder` (one POBJ).  Larger meshes split
  add-on-side into multiple POBJs.
- Fixed attribute formats: POS F32×3, NRM F32×3, CLR0 RGBA8, TEX0 F32×2.
  Multi-format / quantized buffers (I8 / S8 positions, etc.) are
  deferred — see `docs/roadmap.md`.
- The stripper is greedy, not vertex-cache-aware; HSDLib's
  `TriangleConverter` produces a tighter encoding on large meshes.
- One material / POBJ per attach point — multi-DObj DObj chains and
  multi-MObj LOD selection are caller-side.

## Skinned mesh example (Phase 3)

```python
import hsdraw

dat = hsdraw.parse_dat(open("base.dat", "rb").read())

# Locate two bones in the existing tree (caller-side schema parsing).
joints = ...                       # {"bone_arm": JObj, "bone_hand": JObj}
bone_arm  = joints["bone_arm"]
bone_hand = joints["bone_hand"]

mesh = hsdraw.MeshBuilder()
# 4 verts: 2 bound to arm, 2 split 50/50 arm + hand
mesh.add_position(0.0, 0.0, 0.0); mesh.add_position(1.0, 0.0, 0.0)
mesh.add_position(0.0, 1.0, 0.0); mesh.add_position(1.0, 1.0, 0.0)
env_arm  = mesh.add_envelope([(bone_arm, 1.0)])
env_blend = mesh.add_envelope([(bone_arm, 0.5), (bone_hand, 0.5)])
mesh.add_envelope_index(env_arm)
mesh.add_envelope_index(env_arm)
mesh.add_envelope_index(env_blend)
mesh.add_envelope_index(env_blend)
mesh.add_triangle(0, 1, 2)
mesh.add_triangle(1, 3, 2)
pobj = mesh.build()

# Wire as before via DObj / JObj attach.
```

## End-to-end example: from-scratch synthesis (no base .dat)

`Dat.alloc_scene_data()` produces an empty SObj → JOBJDescs[1] →
JObjDesc → root JObj scaffold; from there you wire DObjs, MObjs,
TObjs, and Images yourself.  Useful for the vanilla-independent
export pipeline (no base file to start from):

```python
import hsdraw

dat  = hsdraw.Dat.alloc_scene_data()
sobj = hsdraw.SObj.from_struct(dat.scene_data().data)
root = sobj.jobj_descs()[0].root_joint           # placeholder JObj

# ---- encode a 4×4 RGBA8 source into RGB565 GX bytes ----------------
src = bytes(b"\x40\x80\xC0\xFF" * (4 * 4))        # solid teal
gx  = hsdraw.gx_encode(4, 4, 4, src)              # format 4 = RGB565
assert len(gx) == 32

# ---- build the material chain --------------------------------------
img = hsdraw.Image.alloc()
img.width  = 4
img.height = 4
img.format = 4                                    # RGB565
img.set_image_data_bytes(gx)

tobj = hsdraw.TObj.alloc()
tobj.tex_map_id = 0                               # GX_TEXMAP0
tobj.set_scale(1.0, 1.0, 1.0)
tobj.set_image_data(img)

mobj = hsdraw.MObj.alloc_unlit_color(0xFF, 0xFF, 0xFF, 0xFF)
mobj.set_textures(tobj)

dobj = hsdraw.DObj.alloc()
dobj.set_mobj(mobj)
root.set_dobj(dobj)

# Add some POBJ via MeshBuilder if you want geometry too — see the
# Phase 1 example above.

open("from_scratch.dat", "wb").write(dat.write())
```

The chain `Dat.alloc_scene_data → ... → Image.set_image_data_bytes`
covers everything an addon needs to produce a self-contained .dat
without holding a vanilla base file.  See
`crates/hsdraw-core/tests/from_scratch.rs` for the round-trip
verification path the CI gates on.

## Recipes — byte-level patches / TObj fine-tuning

```python
import hsdraw

# (1) GXTexGenSrc (offset 0x0C u32) — exposed as a Python property,
#     so assign the raw GX enum integer.  0=GX_TG_POS (default),
#     4=GX_TG_TEX0, etc.  See HSDLib's GXTexGenSrc enum or
#     hsdraw_core::gx::GxTexGenSrc for the full set.
tobj.tex_gen_src = 4               # GX_TG_TEX0
assert tobj.tex_gen_src == 4

# (2) Lightmap / bump named setters do bit-RMW on Flags, preserving
#     the coord_type / color_op / alpha_op nibbles.
tobj.set_lightmap_diffuse(True)
tobj.set_bump(False)
assert tobj.is_lightmap_diffuse() and not tobj.is_bump()

# (3) BGR-order RGB5A3 sampler — pre-swap R/B at encode time.
gx = hsdraw.gx_encode(5, w, h, rgba, swap_rb_for_rgb5a3=True)

# (4) Byte-level patch on any HsdStruct (no post-write find/replace).
s = tobj.as_struct()
s.set_u32(0x0C, 4)                          # GX_TG_TEX0 in raw bytes
s.set_u8(0x40, s.raw()[0x40] | 0x10)        # LIGHTMAP_DIFFUSE bit on
s.set_bytes(0x20, b"\x00" * 4)              # blank 4 bytes at 0x20

# (5) HSD_TOBJ_LOD attach — overrides default GX hardware sampler.
#     Useful when the default min_filter / aniso behaviour produces
#     unexpected texture-footprint averaging.  Use raw GX enum ints:
#       min_filter: 0=GX_NEAR  1=GX_LINEAR  2..5=mipmap variants
#       anisotropy: 0=GX_ANISO_1  1=GX_ANISO_2  2=GX_ANISO_4  3=GX_MAX
tobj.set_lod(min_filter=0, bias=0.0, bias_clamp=False,
             enable_edge_lod=False, anisotropy=0)
assert tobj.lod_data.min_filter == 0

# (6) to_dict() — debug-style snapshot of every typed-view field.
#     Avoids hand-parsing raw bytes against HSDLib offsets.
print(tobj.to_dict())
print(pobj.to_dict())   # raw flags, display_list_size, child presence
print(mobj.to_dict())   # render_flags, material/textures/pe_desc presence
print(jobj.to_dict())   # flags, child/next/dobj presence, TRS triples
```

## Limitations (deliberate non-goals)

This binding is the HSDLib surface, not the Blender add-on surface.
The following stay out of `hsdraw` core:

- **`scene.json` schema** — that's `mkgp2-patch`'s convention and
  belongs in the add-on.  The add-on builds its `jobj_id → JObj` map,
  iterates the JSON, and calls the primitives above.
- **Paletted-format encoders** — CI4 / CI8 / CI14X2 / I4 / I8 / IA4 /
  IA8 are read-only.  The vanilla MKGP2 corpus has zero hits across
  7,812 textures, so the addon can route paletted sources through
  RGB5A3 / RGB565 instead.  Adding palette quantization is mechanical
  when a use case lands.

## Identity contract

`JObj` and `HsdStruct` define `__eq__` / `__hash__` as Rc-identity:

```python
a = jobj_via_path_1
b = jobj_via_path_2
if a == b:
    # they wrap the same underlying HsdStruct (alias!)
```

Use this instead of relying on `is` (which compares Python wrapper
objects, not underlying struct identity — two PyJObj instances created
from the same StructRef compare `is`-False but `==`-True).
