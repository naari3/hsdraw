# hsdraw

Pure-Rust reader / writer for HSD (HAL System Development) `.dat` files,
with a PyO3 binding that mirrors HSDLib's accessor surface
(`HSDRawFile` / `HSDRootNode` / `HSD_JOBJ` / `HSD_DOBJ` / `HSD_MOBJ` /
`HSD_POBJ` / …).  Read existing files, mutate the structural fields
(alias roots, joint hierarchy, TRS, flags), build new POBJs from CPU-
side mesh data, and serialize back — all without depending on
`dotnet-script` or HSDLib at runtime.

## Status

| Layer | What works |
|---|---|
| **Reader** | header + relocation table + struct identity, JObj / DObj / MObj / POBJ / SObj walk, GX texture decode (11 formats), DL unpack with full vertex / matrix algebra |
| **Writer** | HSDLib `Save`-compatible: struct identity dedup, FNV-1a buffer hash dedup, ref-chain key=0 skip, alias-root round-trip across the 9-file corpus |
| **csx parity** | semantic JSON diff (ε = 1e-5) + PNG pixel-equal across 6 vanilla MKGP2 courses (`test_course_start_gate`, `MR_highway_short_A` & `_long_A`, `DK_jungle_short_a` & `_long_a`, `AT_demo`) |
| **Mutation API** | HSDLib-equivalent allocation + setter primitives — see [details below](#mutation-primitives) |
| **From-scratch synthesis** | `Dat::alloc_scene_data` factory + TObj / Image allocators + `gx_image::encode_image` (RGBA8 / RGB565 / RGB5A3 / CMP) — produce a self-contained .dat with no base file |
| **POBJ writer** | `MeshBuilder` (positions / normals / colors / UVs / triangles / envelopes) → `Pobj`; greedy `TRIANGLE_STRIP` decomposition (toggleable, on by default), envelope rigging |
| **PyO3** | abi3-py37 wheel covers Python 3.7+; identity-aware `__eq__` / `__hash__` on every typed view |
| **CI** | `cargo test` on Linux / macOS / Windows + 5-platform abi3 wheel matrix via maturin |

Test totals: **78 passing** env-free (20 unit + 12 mutation primitive +
4 parity + 15 POBJ writer round-trip + 12 MObj / Material / PeDesc /
TObj / Image round-trip + 13 GX texture encoder + 2 from-scratch
chain).  With `MKGP2_FILES_DIR` + `MKGP2_PATCH_DIR` set, an additional
6 csx-parity courses + 9-file writer round-trip corpus run.

### Mutation primitives

| Class | Methods |
|---|---|
| `Dat` | `add_root`, `remove_root`, `rename_root`, `repoint_root`, `find_root_for`, `scene_data`, `alloc_scene_data`, `write` |
| `JObj` | `alloc`, `set_child`, `set_next`, `set_dobj`, `flags`, `tx … sz`, `local_trs` / `set_local_trs` |
| `DObj` | `alloc`, `set_mobj`, `set_pobj`, `set_next` |
| `MObj` | `alloc`, `alloc_unlit_color`, `set_material`, `set_textures`, `set_pe_desc`, `render_flags` |
| `Material` | `alloc`, `{amb,dif,spc}_rgba`, `alpha`, `shininess` |
| `PeDesc` | `alloc`, `blend_mode`, `src_factor`, `dst_factor`, `depth_function`, `alpha_*`, … |
| `SObj` | `alloc`, `jobj_descs`, `set_jobj_descs`, `jobj_descs_array` |
| `JObjDesc` | `alloc`, `root_joint`, `set_root_joint` |
| `TObj` | `alloc`, `set_tex_map_id`, `set_rotation` / `set_scale` / `set_translation`, `set_wrap_s` / `_t`, `set_repeat_*`, `set_flags` + nibble setters (`coord_type` / `color_op` / `alpha_op`), `set_blending`, `set_mag_filter`, `set_image_data`, `set_tlut_data`, `set_next`, `tex_gen_src` (property — `GXTexGenSrc` u32 at 0x0C), `set_flag_bit(mask, on)`, `set_lightmap_{diffuse,specular,ambient,ext,shadow}` / `set_bump` (bit-RMW; preserves coord/color/alpha nibbles) |
| `Image` | `alloc`, `set_image_data_bytes`, `set_width` / `_height`, `set_format`, `set_mipmap`, `set_min_lod` / `_max_lod` |
| `HsdStruct` | `byte_size`, `raw`, `references`, `get_reference`, `set_reference`, `set_u8` / `set_u16` / `set_u32` / `set_bytes` (byte-level patch primitives) |

These mirror HSDLib's accessor / setter pairs at the byte-offset level;
nothing here carries project-specific schemas.  See
[`docs/python_api.md`](docs/python_api.md) for the csx ↔ Python
correspondence table and end-to-end examples.

### POBJ writer

`MeshBuilder` produces a fresh `HSD_POBJ` (with attribute table,
per-attribute vertex buffers, and GX display-list bytecode) from CPU-
side mesh data:

- **TRIANGLES path** (Phase 1): `0x90` primitive group, GX_INDEX16
  indices, single attribute group per POBJ.
- **TRIANGLE_STRIP optimization** (Phase 2, default on): greedy
  stripper picks the longest of three seed orientations, emits
  `0x98 (TriangleStrip)` groups for chains of ≥ 4 verts plus a
  `Triangles` group for the leftover.  Toggle via
  `MeshBuilder.set_use_triangle_strips(False)` for predictable byte
  layouts (debugging / HSDLib parity).
- **Envelope rigging** (Phase 3): `add_envelope([(JObj, weight), …])` +
  `add_envelope_index(i)` per vertex.  Sets `POBJ_FLAG.ENVELOPE`,
  emits `GX_VA_PNMTXIDX` as a DIRECT 1-byte per-vertex attribute, wires
  the null-terminated envelope-pointer array at POBJ + 0x14.  Up to 85
  envelopes per POBJ (matrix-slot range).

`MeshBuilder.set_cull_back` / `set_cull_front` are **deprecated** —
the values they used to fold into POBJ.flags (0x4000 / 0x8000) collide
with `POBJ_TYPE_MASK` (`SHAPEANIM` / `ENVELOPE`) in HSDLib's `POBJ_FLAG`
enum.  Calling them is a no-op (a `DeprecationWarning` is raised on the
PyO3 side); face culling lives on the MObj/RenderFlags side instead.

Fixed attribute encoding: POS F32×3, NRM F32×3, CLR0 RGBA8, TEX0 F32×2.
Multi-format / quantized buffers and a vertex-cache-aware stripper
remain on the roadmap.

## Layout

```
crates/
  hsdraw-core/      pure Rust parser + writer, no Python deps
  hsdraw-py/        PyO3 binding (abi3-py37, cdylib → hsdraw.pyd / hsdraw.so)
  hsdraw-cli/       hsdraw-cli {decode,encode} foo.dat — parity / round-trip CLI
docs/
  handoff.md        original design spec
  python_api.md     Python surface + csx ↔ Python table
  roadmap.md        deferred work
  notes/            phase{0,4,5,6}.md walkthroughs (early-phase design notes)
.github/workflows/
  test.yml          cargo test on 3 OS + maturin develop smoke
  wheels.yml        5-platform abi3 wheel matrix (manual dispatch / tag push)
```

## Build

```bash
cargo check --workspace
cargo build --release -p hsdraw-cli   # binary at target/release/hsdraw-cli
```

Python wheel (local develop):

```bash
python -m venv .venv && source .venv/bin/activate  # or .venv\Scripts\Activate.ps1
pip install maturin
maturin develop --release -m crates/hsdraw-py/Cargo.toml
python -c "import hsdraw; print(hsdraw.version())"
```

Cross-platform wheels (linux x86_64 / aarch64, macos x86_64 / arm64,
windows x86_64) are built by `.github/workflows/wheels.yml`.  Trigger
manually via **Actions → wheels → Run workflow** or push a `v*` tag.

## Test

```bash
cargo test --workspace
# → 78 PASS, env-free
```

For the corpus comparisons (requires `dotnet-script` on PATH plus a
local `mkgp2-patch` checkout and an extracted `.dat` directory):

```bash
MKGP2_PATCH_DIR=/path/to/mkgp2-patch \
MKGP2_FILES_DIR="/path/to/mkgp2/files" \
  cargo test --test parity -- --nocapture
```

`tests/parity.rs` runs the env-aware csx parity (6 vanilla courses) and
the writer round-trip (9 files) on top of the 4 env-free parity tests.

## CLI

```bash
hsdraw-cli decode foo.dat --out out/      # → out/scene.json + out/tex/*.png
hsdraw-cli encode foo.dat --out foo2.dat  # parse → write smoke
```

## Python API at a glance

Full reference + csx-equivalent table lives in
[`docs/python_api.md`](docs/python_api.md).  Two short examples:

### Alias add + TRS edit + new joint splice

```python
import hsdraw

dat = hsdraw.parse_dat(open("base.dat", "rb").read())

new_jobj = hsdraw.JObj.alloc()
new_jobj.set_local_trs(5.0, 0.0, 0.0,
                       0.0, 0.0, 0.0,
                       1.0, 1.0, 1.0)
new_jobj.flags = 0x40000             # HSDLib JOBJ_FLAG.OPA

dat.add_root("MR_highway_test_joint", new_jobj)
assert dat.find_root_for(new_jobj).name == "MR_highway_test_joint"

open("out.dat", "wb").write(dat.write())
```

### Build a brand-new mesh, attach it to a JObj, and save

```python
import hsdraw

dat = hsdraw.parse_dat(open("base.dat", "rb").read())

mesh = hsdraw.MeshBuilder()
mesh.add_position(0.0, 0.0, 0.0)
mesh.add_position(1.0, 0.0, 0.0)
mesh.add_position(0.0, 1.0, 0.0)
mesh.add_color(0xFF, 0x00, 0x00, 0xFF)   # 3 verts × red
mesh.add_color(0xFF, 0x00, 0x00, 0xFF)
mesh.add_color(0xFF, 0x00, 0x00, 0xFF)
mesh.add_triangle(0, 1, 2)
pobj = mesh.build()                       # → hsdraw.Pobj

mobj = hsdraw.MObj.alloc_unlit_color(0xFF, 0x00, 0x00, 0xFF)
dobj = hsdraw.DObj.alloc()
dobj.set_mobj(mobj)
dobj.set_pobj(pobj)

new_joint = hsdraw.JObj.alloc()
new_joint.set_dobj(dobj)
dat.add_root("MR_highway_my_mesh_joint", new_joint)

open("out.dat", "wb").write(dat.write())
```

`JObj` / `DObj` / `MObj` / `Pobj` / `HsdStruct` define `__eq__` /
`__hash__` as Rc-identity, so alias detection (`a == b` ⇔ same
underlying struct) works the same way HSDLib's reference equality does.

## What this is *not*

- **Not a fighter / MEX / kex specialization**.
  `crates/hsdraw-core/src/writer.rs` skips HSDLib's
  `RemoveDuplicateBuffers` suppression for `SBM_FighterData` /
  `MEX_Data` / `kexData`.  Adding those is mechanical when a fighter
  use-case shows up — see [`docs/roadmap.md`](docs/roadmap.md).
- **Not a vertex-cache-aware stripper**.  The greedy stripper produces
  smaller DL bytecode than the bare `Triangles` path, but HSDLib's full
  `TriangleConverter` (cache simulation + priority heap) is tighter on
  large meshes.
- **Not a paletted-format encoder**.  RGBA8 / RGB565 / RGB5A3 / CMP
  encoding is in (CMP via `texpresso` BC1 with the GX-specific BE word
  swap + 8x8 super-block swizzle).  CI4 / CI8 / CI14X2 / I4 / I8 /
  IA4 / IA8 stay read-only — vanilla MKGP2 corpus has zero hits across
  7,812 textures so the addon can route paletted sources through
  RGB5A3 / RGB565 instead.  Adding palette quantization is mechanical
  when a use case lands.
- **Not an in-place vanilla-POBJ editor**.  Mesh edits go through the
  "decompose via reader → `MeshBuilder` rebuild → splice" flow, not by
  mutating the original DL bytecode.

## License

MIT OR Apache-2.0 (consumer's choice).
