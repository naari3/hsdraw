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
| `HsdStruct.byte_size()` / `.raw()`      | `_s.Length` / `_s.GetData()`                              | introspection       |
| `HsdStruct.references() -> [(off, target)]` | `_s.References`                                       | walk raw refs       |
| `HsdStruct.get_reference(offset)`       | `_s.GetReference<HSDAccessor>(offset)` (sans typed cast)  | offset lookup       |
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
mesh.set_cull_back(True)                  # POBJ_FLAG.CULLBACK
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

## Limitations (deliberate non-goals)

This binding is the HSDLib surface, not the Blender add-on surface.
The following stay out of `hsdraw` core:

- **`scene.json` schema** — that's `mkgp2-patch`'s convention and
  belongs in the add-on.  The add-on builds its `jobj_id → JObj` map,
  iterates the JSON, and calls the primitives above.
- **Material / DObj / TObj typed views** — JObj is enough for csx Pass
  0–4.  Other accessors land in `hsdraw_core` first (already there for
  read-only walk) and get expose to Python when an actual writer use
  case shows up.
- **POBJ writer** (Blender mesh → fresh display list) — see
  `docs/roadmap.md`.

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
