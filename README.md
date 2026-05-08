# hsdraw

HSD (HAL System Development) `.dat` reader/writer in Rust, with PyO3 bindings
for use from Blender add-ons.

Built to back the `mkgp2-patch` toolchain (currently using `dotnet-script` +
HSDLib via `tools/hsd/hsd_export_for_blender.csx`). Goal: drop the dotnet
dependency and make `import hsdraw` from Blender-bundled CPython work.

## Status

Phase 0–6 implemented. See `docs/handoff.md` for the original design spec
and `docs/notes/phaseN.md` for per-phase notes.

- Reader (parse + walk JObj/DObj/MObj/POBJ tree, GX texture decode, DL unpack)
- Writer (HSDLib `Save`-compatible, alias-root round-trip on 9-file corpus)
- csx parity gate (`MKGP2_FILES_DIR`+`MKGP2_PATCH_DIR` env, 6 vanilla courses)
- PyO3 binding (`hsdraw.parse_dat` / `write_dat` / `export_scene_json`)
- GitHub Actions: cargo test on 3 OS + cibuildwheel-style 5-platform wheels

## Layout

```
crates/
  hsdraw-core/      pure Rust parser + writer, no Python deps
  hsdraw-py/        PyO3 binding (abi3-py37, cdylib -> hsdraw.pyd / hsdraw.so)
  hsdraw-cli/       single-binary "hsdraw-cli decode foo.dat out/" for parity test
tests/
  data/             small synthetic .dat files for CI parity tests
  parity/           harness that runs csx + Rust on the same .dat and diffs
```

## Build

```bash
cargo check --workspace
cargo build --release -p hsdraw-cli                # binary at target/release/hsdraw-cli
```

For the Python wheel:

```bash
python -m venv .venv && source .venv/bin/activate  # (or .venv\Scripts\Activate.ps1)
pip install maturin
maturin develop --release -m crates/hsdraw-py/Cargo.toml
python -c "import hsdraw; print(hsdraw.version())"
```

Cross-platform wheels are built by `.github/workflows/wheels.yml` for
linux x86_64 / aarch64, macos x86_64 / arm64, and windows x86_64.  Trigger
manually via *Actions → wheels → Run workflow*, or push a `v*` tag.

## Test

```bash
cargo test --workspace                              # unit tests + synthetic gate
MKGP2_PATCH_DIR=/path/to/mkgp2-patch \
MKGP2_FILES_DIR="/path/to/mkgp2/files" \
  cargo test --test parity -- --nocapture          # full csx + writer round-trip
```

The first form is what CI runs; the second adds csx-vs-Rust parity diff and
writer round-trip across the vanilla MKGP2 corpus (requires `dotnet-script`
on PATH and the env-vars to point at a local mkgp2-patch checkout + a
directory holding extracted .dat files).

## CLI

```bash
hsdraw-cli decode foo.dat --out out/                # → out/scene.json + out/tex/*.png
hsdraw-cli encode foo.dat --out foo_rewritten.dat   # parse → write smoke
```

## License

MIT OR Apache-2.0 (consumer's choice).
