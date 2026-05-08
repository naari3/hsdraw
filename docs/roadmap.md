# Roadmap

What's *not* in scope for Phases 0–6 but worth picking up later.  Items
are roughly ordered by leverage on the MKGP2 add-on workflow.

## POBJ writer (Blender mesh → fresh display list)

**csx parity status:** csx itself doesn't have one.  The
`hsd_import_from_blender.csx` script intentionally only mutates
structural fields (joint hierarchy / alias roots / TRS / flags); mesh
content is taken verbatim from the base .dat.  hsdraw's POBJ writer is
the upstream of csx in this regard.

### Phase 1–3 — done

- `pobj_writer::MeshBuilder` Rust core: positions / normals / colors /
  UVs / triangle indices → `PObj`, with optional envelope rigging.
- **Phase 1 (TRIANGLES emit)**: `GX_DRAW_TRIANGLES = 0x90`, GX_INDEX16
  attributes, single attribute group per POBJ.  Fixed encoding: POS
  F32×3, NRM F32×3, CLR0 RGBA8, TEX0 F32×2.
- **Phase 2 (TRIANGLE_STRIP optimization)**: greedy stripper (3-way
  orientation pick, edge-adjacency walk) producing `0x98 (TriangleStrip)`
  primitive groups for chains of ≥ 4 verts plus a trailing `Triangles`
  group for the leftover.  Toggle via `MeshBuilder.set_use_triangle_strips`
  (default on); the strip output has been measured smaller than the
  Phase 1 path on the 96-tri ribbon test.  Not the full HSDLib
  `TriangleConverter` (no vertex-cache simulator / priority heap), but
  fast and good-enough for course-mesh input sizes.
- **Phase 3 (envelope rigging)**: `MeshBuilder.add_envelope` +
  `add_envelope_index` for skinned meshes.  Sets `POBJ_FLAG.ENVELOPE`,
  emits `GX_VA_PNMTXIDX` as the first attribute (DIRECT, 1 byte), wires
  a null-terminated array of `HSD_Envelope` structs at POBJ + 0x14.
  Per-vertex `PNMTXIDX` value is `(envelope_index × 3)` — each envelope
  reserves a pos/normal/binormal triple of GX matrix slots.  Capped at
  85 envelopes per POBJ (`u8` matrix slot range); split into multiple
  POBJs above that.
- `DObj::allocate_default` + `set_mobj` / `set_pobj`, `JObj::set_dobj`
  primitives so the new POBJ can be wired into an existing tree.
- `MObj::allocate_default` / `allocate_unlit_color`, `Material` /
  `PeDesc` allocation + per-field setters for new-material attach.
- 14 round-trip tests in `tests/pobj_writer.rs` (Phase 1 single-tri /
  quad / +nrm+clr / +UV / 96-vert ribbon, Phase 2 strip variants +
  byte-size shrinkage gate, Phase 3 envelope round-trip + validation
  rejections + envelope-with-strips combo) plus 5 in `tests/mobj_writer.rs`
  (default sizes, unlit preset round-trip, explicit-setter round-trip).
- PyO3 expose: `MeshBuilder` (with `set_use_triangle_strips` /
  `add_envelope` / `add_envelope_index`), `Pobj` / `DObj` / `MObj` /
  `Material` / `PeDesc` Python classes, `JObj.set_dobj`.

### Future widening (not in scope yet)

- **Vertex-cache-aware stripper**: HSDLib's full `TriangleConverter`
  port for tighter strip output on large meshes.  Mechanical but
  multi-day work.
- **Multiple primitive groups with mixed envelope sets per POBJ**:
  HSDLib's `POBJ_Generator` re-groups primitives by influence-set; the
  current writer puts everything in one POBJ regardless.  Useful for
  fighter data where one logical mesh spans many bone groups.

## Texture re-pack — paletted formats

RGBA8 / RGB565 / RGB5A3 / CMP encoders ship in `gx_image::encode_image`
(CMP via `texpresso` BC1 + GX-specific BE word swap + 8x8 super-block
swizzle).  Plus the TObj / Image allocators in `common.rs` and the
`Dat::alloc_scene_data` factory let an addon produce a self-contained
.dat with no base file — see `tests/from_scratch.rs` for the
end-to-end verification path.

What's still deferred:

- **Paletted formats (CI4 / CI8 / CI14X2)** — vanilla MKGP2 corpus has
  zero hits across 7,812 textures so the addon routes paletted sources
  through RGB5A3 / RGB565 instead.  Adding palette quantization (median-
  cut + nearest-color index assignment) plus a Tlut allocator is
  mechanical when a use case lands.
- **Intensity formats (I4 / I8 / IA4 / IA8)** — same story.  These are
  pure 8-bit-channel quantize-then-pack inverse operations once a
  consumer wants them.
- **PNG ↔ raw byte path** — Image.image_data() returns raw bytes, but
  there's no `decode_to_png` / `encode_from_png` convenience yet; the
  addon does the PNG codec step itself via `Pillow` since it already
  has UV-mapping and Blender-side processing in Python.

## Higher-platform wheel matrix

`.github/workflows/wheels.yml` ships 5 platforms today.  Pending need:

- **windows-arm64**: native Windows on Snapdragon-class hardware.
  Maturin + Rust `aarch64-pc-windows-msvc` target + native runner.
- **linux-musl** (manylinux musl variants): for Alpine-based Blender
  containers.  Same crate, just a different cibuildwheel selector.

Both are mechanical CI work; gating on actual demand.

## Larger PyO3 surface (`Dat` accessor graph)

Today `hsdraw.parse_dat` returns a thin probe.  The full surface
hinted at in `docs/handoff.md`:

```python
for name, root in dat.public_roots():
    for jobj in root.iter_descendants():
        for dobj in jobj.dobjs():
            mesh = dobj.unpack_mesh()
            tex  = dobj.material().texture(0).decode_rgba8()
```

would let an add-on read the tree directly without round-tripping
through JSON.  Lock in once the writer's mutation API stabilizes
(otherwise we paint into a corner where Python users hold pointers
into structs the writer wants to dedup).

## ftData / MEX / kex specialization

`writer.rs` currently skips HSDLib's `RemoveDuplicateBuffers`
suppression for `SBM_FighterData` / `MEX_Data` / `kexData` (course
.dat doesn't trigger those branches).  If hsdraw is ever pointed at
Smash Melee fighter data the writer needs to mirror those branches —
currently it would produce a structurally-correct but content-corrupt
file because shared buffers in fighter data aren't actually
content-equivalent.  Tracked as a writer-time `WriteOptions` flag.

## Subaction goto-pointer fix-up

Same family as above — HSDLib's debug-build orphan handling for
subaction structs (`HSDRawFile.cs` L344–L368).  Skipped because course
.dat has no orphans and we strip them on parse.  Reinstating it is a
post-walk pass over `_structCache`, doable when it matters.
