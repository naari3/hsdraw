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
| `JObj.flags` getter/setter (`u32`)      | `j.Flags` (JOBJ_FLAG)                                     | flag bits           |
| `JObj.tx` … `.sz` getter/setter         | `j.TX` … `j.SZ`                                           | local TRS           |
| `JObj.local_trs() -> tuple[9]`          | (none — convenience)                                      | snapshot            |
| `JObj.set_local_trs(tx, ty, …, sz)`     | (none — convenience)                                      | bulk write          |
| `JObj.as_struct() -> HsdStruct`         | `j._s`                                                    | drop the typed view |
| `HsdStruct.byte_size()` / `.raw()`      | `_s.Length` / `_s.GetData()`                              | introspection       |
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

`PyTypeError`:

- `Dat.add_root` / `repoint_root` given anything that isn't a `JObj`
  or `HsdStruct`.

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
