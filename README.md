# hsdraw

HSD (HAL System Development) `.dat` reader/writer in Rust, with PyO3 bindings
for use from Blender add-ons.

Built to back the `mkgp2-patch` toolchain (currently using `dotnet-script` +
HSDLib via `tools/hsd/hsd_export_for_blender.csx`). Goal: drop the dotnet
dependency and make `import hsdraw` from Blender-bundled CPython work.

## Status

Pre-Phase 0. See `docs/handoff.md` for the full design spec / scope / parity
test requirements.

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
cargo build --release -p hsdraw-cli
```

For the Python wheel:

```bash
cargo install maturin
maturin develop -m crates/hsdraw-py/Cargo.toml   # local install into venv
maturin build --release -m crates/hsdraw-py/Cargo.toml --features pyo3/abi3-py37
```

## Test

```bash
cargo test --workspace
cargo test --test parity                           # synthetic corpus, CI gate
MKGP2_FILES_DIR=/path/to/mkgp2/files \
  cargo test --test parity -- --ignored            # vanilla MKGP2 corpus
```

## License

MIT OR Apache-2.0 (consumer's choice).
