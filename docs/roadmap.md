# Roadmap

What's *not* in scope for Phases 0–6 but worth picking up later.  Items
are roughly ordered by leverage on the MKGP2 add-on workflow.

## POBJ writer (Blender mesh → fresh display list)

**csx parity status:** csx itself doesn't have one.  The
`hsd_import_from_blender.csx` script intentionally only mutates
structural fields (joint hierarchy / alias roots / TRS / flags); mesh
content is taken verbatim from the base .dat.

**Goal:** consume the JSON `meshes[]` (vertex / primitive arrays
already present in `scene.json`) and re-encode them as GX display-list
bytes plus a fresh `HSD_POBJ` accessor + attribute table.  This is the
direction the add-on actually wants — without it, "edit a mesh in
Blender → save .dat" still requires HSDRawViewer's IONET importer.

**Sketch:**

1. Add a writer-side mirror of `gx_dl::unpack`: take a
   `Vec<Vertex>` + `Vec<Primitive>`, decide ATTR layout (DIRECT /
   INDEX8 / INDEX16 per attr) the way HSDLib's
   `GX_VertexAccessor.SetVertices` does, then emit the DL bytecode.
2. Allocate a new `HSD_POBJ` struct, fill the attribute table at
   offset 0x08, the DL size at 0x0C, and the DL buffer ref at 0x10.
3. Re-link DObj → POBJ chain when the JSON mesh `material_index` /
   `pobj_index` changes.

**Why not yet:** the GX DL encoder is an honest
`gx_dl::unpack` inverse — needs a quantization decision per attribute,
buffer dedup, and a bounded-precision check (positions go through
GX_F32 by default but I8/S8/U8 are tempting for size).  A few-day
project worth taking up once the import edit loop actually wants it.

## Texture re-pack from PNG

Same shape — invert `gx_image::decode_image` for each format.  CMP and
RGB5A3 are the awkward ones (they involve perceptual tuning); the
others are mechanical.  Useful when the add-on edits a UV-mapped image
in Blender and wants to feed it back.

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
