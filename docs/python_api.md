# `hsdraw` Python API (minimal reference)

ABI3-py37 wheel — load via the Blender add-on vendor path described in
`docs/notes/phase6.md`.  Surface mirrors the csx pipeline pieces an add-on
needs to call:

| csx                                  | Python                                |
|--------------------------------------|---------------------------------------|
| `hsd_export_for_blender.csx`         | `hsdraw.export_scene_json`            |
| `hsd_import_from_blender.csx`        | `hsdraw.import_from_scene_json`       |
| (any HSDLib `Save`)                  | `hsdraw.write_dat`                    |
| `new HSDRawFile(bytes).Roots`        | `hsdraw.parse_dat(bytes).root_names()`|

## Functions

### `hsdraw.version() -> str`

Crate version (matches `Cargo.toml`).  Useful for telemetry / pinning
checks in the add-on.

### `hsdraw.parse_dat(data: bytes) -> Dat`

Validates the .dat header + relocation table.  Returns a `Dat` handle
exposing `.root_names() -> list[str]` and `.byte_size() -> int`.  The
fuller object-graph API (walk JObj/DObj/MObj) is deferred — for now use
`export_scene_json` for read access.

### `hsdraw.export_scene_json(data, /, source_dat="", tex_dir=None) -> str`

Returns a JSON string identical to what `hsd_export_for_blender.csx`
writes (parity-verified across 6 vanilla MKGP2 courses).  When
`tex_dir` is provided, decoded PNGs are dumped alongside; otherwise
only the JSON skeleton is produced.

### `hsdraw.write_dat(data, /, optimize=True, buffer_align=True) -> bytes`

`parse → write` with HSDLib-compatible struct identity dedup + buffer
hash dedup.  Used as a smoke test for the writer; the alias-root
round-trip parity holds across the 9-file fixture in
`docs/notes/phase5.md`.

### `hsdraw.import_from_scene_json(base_dat, scene_json, /) -> (bytes, dict)`

Drop-in replacement for invoking `hsd_import_from_blender.csx` from a
Blender add-on.  Applies the same Pass 0–4 mutations:

1. Walk base scene tree DFS (matches `hsd_export_for_blender.csx`'s
   `EmitJoint` ordering, so `jobj_N` ids in the JSON line up).
2. Allocate fresh 0x40-byte `HSD_JOBJ` for any JSON joint id absent
   from the base.
3. `joint_aliases` add / repoint / stale-prune against
   `dat.roots`.  Errors out on blank names / unknown target ids.
4. Per-joint TRS + JOBJ_FLAG sync.
5. Joint hierarchy rewire (rebuild `Child` / `Next` chain to match
   `joint.children[]`).
6. `write_dat`-equivalent serialization.

`scene_json` accepts either a `str` (typical
`open(...).read()` pattern) or `bytes`.  The returned `dict` reports
per-pass counts for asserting / logging:

```
{
  "joints_walked": 16,
  "new_joints":    0,
  "aliases_added": 1,
  "aliases_repointed": 0,
  "aliases_removed":   0,
  "trs_changed":   0,
  "flags_changed": 0,
  "hierarchy_rewired": 0,
}
```

## Add-on integration snippet

This is the minimum a Blender add-on operator needs to call when
exporting an edited bundle back to a .dat — drop-in replacement for the
`subprocess.run(['dotnet-script', 'hsd_import_from_blender.csx', ...])`
path that `mkgp2-patch` currently uses.

```python
import hsdraw, json, pathlib

def import_bundle_to_dat(base_dat_path: str,
                        bundle_dir: str,
                        out_dat_path: str) -> dict:
    base = pathlib.Path(base_dat_path).read_bytes()
    scene = (pathlib.Path(bundle_dir) / "scene.json").read_text()
    out, stats = hsdraw.import_from_scene_json(base, scene)
    pathlib.Path(out_dat_path).write_bytes(out)
    return stats

# Export-time read-only path (replaces hsd_export_for_blender.csx):
def export_bundle(dat_path: str, bundle_dir: str) -> dict:
    raw = pathlib.Path(dat_path).read_bytes()
    bundle = pathlib.Path(bundle_dir)
    bundle.mkdir(parents=True, exist_ok=True)
    js = hsdraw.export_scene_json(
        raw,
        source_dat=pathlib.Path(dat_path).name,
        tex_dir=str(bundle / "tex"),
    )
    (bundle / "scene.json").write_text(js)
    return json.loads(js)
```

## Errors

`PyValueError` is raised on:

- malformed .dat (header / relocation table inconsistencies)
- unknown `JOBJ_FLAG` name in `scene.json` (typo, schema drift)
- alias with empty name or unknown target id
- hierarchy `joint.children[]` referencing an unknown joint id

In every case the Rust-side error message is included; the add-on can
surface it as `self.report({'ERROR'}, str(exc))` without further
processing.

## Limitations (Phase 1 MVP — match csx)

- Mesh / DObj / texture content is **not** edited.  The base .dat
  provides them; the importer only edits joint hierarchy, alias roots,
  TRS, and flags.  See `docs/roadmap.md` for the POBJ writer plan that
  would lift this.
- The `hsdraw.Dat` handle is a probe (root names + byte size); the
  full accessor surface (walk JObj/DObj/MObj, mutate accessors, …) is
  reserved for Phase 7+.  Use `export_scene_json` if you just need read
  access to the structural metadata.
