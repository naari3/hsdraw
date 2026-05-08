//! Parity test harness.  Drives both csx (`mkgp2-patch/tools/hsd/
//! hsd_export_for_blender.csx`) and the Rust exporter on the same .dat,
//! then diffs `scene.json` semantically and `tex/*.png` for pixel-level
//! equality.  Set `MKGP2_PATCH_DIR` (= the mkgp2-patch repo root) and
//! `MKGP2_FILES_DIR` (= the directory holding vanilla .dat files) to enable
//! the `#[ignore]`d corpus tests.
//!
//! Without those env vars, the harness still runs the Rust-only smoke test
//! against the synthetic fixture in `tests/data/`.  csx-driven comparisons
//! are skipped via `eprintln!("skipped"); return;` per
//! `docs/handoff.md` § "csx parity tests".
//!
//! See `docs/notes/phase0.md` §4 for the diff rules; this file is the
//! executable embodiment of those rules.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use hsdraw_core::Dat;
use hsdraw_core::export;
use serde_json::Value;

const FLOAT_EPS: f64 = 1e-5;

// =====================================================================
// Helpers
// =====================================================================

fn workspace_root() -> PathBuf {
    // crates/hsdraw-core/tests/  →  ../../..
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .expect("workspace root")
}

fn mkgp2_patch_dir() -> Option<PathBuf> {
    std::env::var_os("MKGP2_PATCH_DIR").map(PathBuf::from)
}

fn mkgp2_files_dir() -> Option<PathBuf> {
    std::env::var_os("MKGP2_FILES_DIR").map(PathBuf::from)
}

fn dotnet_script_available() -> bool {
    Command::new("dotnet-script")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_csx(dat: &Path, out_dir: &Path) -> Result<(), String> {
    let patch = mkgp2_patch_dir()
        .ok_or_else(|| "MKGP2_PATCH_DIR not set".to_owned())?;
    let csx = patch
        .join("tools")
        .join("hsd")
        .join("hsd_export_for_blender.csx");
    if !csx.exists() {
        return Err(format!("csx not found at {}", csx.display()));
    }
    let out = Command::new("dotnet-script")
        .arg(&csx)
        .arg("--")
        .arg(dat)
        .arg(out_dir)
        .output()
        .map_err(|e| format!("dotnet-script run failed: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "csx exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

fn run_rust(dat: &Path, out_dir: &Path) -> Result<(), String> {
    let bytes = std::fs::read(dat).map_err(|e| e.to_string())?;
    let parsed = Dat::parse(&bytes).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let tex_dir = out_dir.join("tex");
    let scene = export::export_scene(
        &parsed,
        dat.file_name().unwrap().to_string_lossy().into_owned(),
        Some(&tex_dir),
    )
    .map_err(|e| format!("export failed: {:?}", e))?;
    let json = serde_json::to_string(&scene).map_err(|e| e.to_string())?;
    std::fs::write(out_dir.join("scene.json"), json).map_err(|e| e.to_string())?;
    Ok(())
}

// =====================================================================
// Semantic JSON diff (eps for floats; key order ignored; arrays positional)
// =====================================================================

#[derive(Debug)]
struct JsonDiff(String);

fn diff_json(a: &Value, b: &Value, path: &str) -> Result<(), JsonDiff> {
    use Value::*;
    match (a, b) {
        (Null, Null) => Ok(()),
        (Bool(x), Bool(y)) if x == y => Ok(()),
        (Number(x), Number(y)) => {
            let xf = x.as_f64().unwrap_or(f64::NAN);
            let yf = y.as_f64().unwrap_or(f64::NAN);
            if xf.is_nan() && yf.is_nan() {
                return Ok(());
            }
            // Integer fast path: same i64 representation = exact match.
            if let (Some(xi), Some(yi)) = (x.as_i64(), y.as_i64()) {
                if xi == yi {
                    return Ok(());
                }
            }
            if (xf - yf).abs() <= FLOAT_EPS
                || (xf.abs().max(yf.abs()) > 0.0
                    && ((xf - yf) / xf.abs().max(yf.abs())).abs() <= FLOAT_EPS)
            {
                Ok(())
            } else {
                Err(JsonDiff(format!(
                    "number mismatch at {}: {} vs {}",
                    path, xf, yf
                )))
            }
        }
        (String(x), String(y)) if x == y => Ok(()),
        (Array(x), Array(y)) => {
            if x.len() != y.len() {
                return Err(JsonDiff(format!(
                    "array length mismatch at {}: {} vs {}",
                    path,
                    x.len(),
                    y.len()
                )));
            }
            for (i, (xi, yi)) in x.iter().zip(y.iter()).enumerate() {
                diff_json(xi, yi, &format!("{}/{}", path, i))?;
            }
            Ok(())
        }
        (Object(x), Object(y)) => {
            // Use BTreeMap to make key iteration deterministic and sort-friendly.
            let xb: BTreeMap<&std::string::String, &Value> = x.iter().collect();
            let yb: BTreeMap<&std::string::String, &Value> = y.iter().collect();
            // Missing-key check both ways so we get a useful diff message.
            for k in xb.keys() {
                if !yb.contains_key(*k) {
                    return Err(JsonDiff(format!(
                        "missing key {:?} on right at {}",
                        k, path
                    )));
                }
            }
            for k in yb.keys() {
                if !xb.contains_key(*k) {
                    return Err(JsonDiff(format!(
                        "missing key {:?} on left at {}",
                        k, path
                    )));
                }
            }
            for (k, vx) in &xb {
                let vy = yb[k];
                diff_json(vx, vy, &format!("{}/{}", path, k))?;
            }
            Ok(())
        }
        _ => Err(JsonDiff(format!(
            "type mismatch at {}: {:?} vs {:?}",
            path, a, b
        ))),
    }
}

fn diff_json_files(a: &Path, b: &Path) -> Result<(), JsonDiff> {
    let a_bytes = std::fs::read(a).map_err(|e| JsonDiff(e.to_string()))?;
    let b_bytes = std::fs::read(b).map_err(|e| JsonDiff(e.to_string()))?;
    let a_val: Value =
        serde_json::from_slice(&a_bytes).map_err(|e| JsonDiff(e.to_string()))?;
    let b_val: Value =
        serde_json::from_slice(&b_bytes).map_err(|e| JsonDiff(e.to_string()))?;
    diff_json(&a_val, &b_val, "")
}

// =====================================================================
// PNG comparison: bytes first, falls back to RGBA pixel decode if differing.
// We treat pixel-equal PNGs as PASS even if the encoded bytes differ — strict
// byte-equality across encoders is unrealistic (deflate impl variance), see
// `docs/notes/phase0.md` §7 #8.
// =====================================================================

// Fields are read via `Debug` in panic messages; rustc's dead-code pass
// doesn't see that, so silence the warnings on the variant payloads.
#[allow(dead_code)]
#[derive(Debug)]
enum PngDiff {
    SizeMismatch { left: u32, right: u32, dim: &'static str },
    PixelMismatch { x: u32, y: u32, left: [u8; 4], right: [u8; 4] },
    DecodeError(String),
}

fn diff_png_files(a: &Path, b: &Path) -> Result<bool, PngDiff> {
    let a_bytes = std::fs::read(a).map_err(|e| PngDiff::DecodeError(e.to_string()))?;
    let b_bytes = std::fs::read(b).map_err(|e| PngDiff::DecodeError(e.to_string()))?;
    if a_bytes == b_bytes {
        return Ok(true); // byte-identical
    }
    let (a_w, a_h, a_pix) = decode_png(&a_bytes)?;
    let (b_w, b_h, b_pix) = decode_png(&b_bytes)?;
    if a_w != b_w {
        return Err(PngDiff::SizeMismatch { left: a_w, right: b_w, dim: "width" });
    }
    if a_h != b_h {
        return Err(PngDiff::SizeMismatch { left: a_h, right: b_h, dim: "height" });
    }
    for y in 0..a_h {
        for x in 0..a_w {
            let off = ((y * a_w + x) * 4) as usize;
            let l = [a_pix[off], a_pix[off + 1], a_pix[off + 2], a_pix[off + 3]];
            let r = [b_pix[off], b_pix[off + 1], b_pix[off + 2], b_pix[off + 3]];
            if l != r {
                return Err(PngDiff::PixelMismatch { x, y, left: l, right: r });
            }
        }
    }
    Ok(false) // pixel-equal but byte-different
}

fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), PngDiff> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|e| PngDiff::DecodeError(e.to_string()))?;
    let info = reader.info().clone();
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    reader
        .next_frame(&mut buf)
        .map_err(|e| PngDiff::DecodeError(e.to_string()))?;
    // Make sure both have RGBA8 layout — png crate may decode as RGB (3
    // channels) or 8-bit grayscale; we require RGBA.
    if info.color_type != png::ColorType::Rgba {
        return Err(PngDiff::DecodeError(format!(
            "expected RGBA, got {:?}",
            info.color_type
        )));
    }
    Ok((info.width, info.height, buf))
}

fn diff_tex_dirs(a: &Path, b: &Path, artifact_dir: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let entries: Vec<PathBuf> = match std::fs::read_dir(a) {
        Ok(it) => it
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "png"))
            .collect(),
        Err(e) => {
            errors.push(format!("read_dir {}: {}", a.display(), e));
            return errors;
        }
    };
    for left in &entries {
        let name = left.file_name().expect("name");
        let right = b.join(name);
        if !right.exists() {
            errors.push(format!("missing on right: {}", name.to_string_lossy()));
            continue;
        }
        match diff_png_files(left, &right) {
            Ok(true) => {} // byte-identical: best
            Ok(false) => {
                eprintln!(
                    "  [info] {} pixel-equal but byte-different",
                    name.to_string_lossy()
                );
            }
            Err(e) => {
                // CI artifact: copy both PNGs into the artifact dir so
                // the test log links a downloadable bundle for triage.
                if let Err(io_err) = dump_png_artifact(artifact_dir, left, &right) {
                    eprintln!("  [warn] artifact dump failed: {}", io_err);
                }
                errors.push(format!("{}: {:?}", name.to_string_lossy(), e));
            }
        }
    }
    // Check the other direction for entries-only-on-right.
    if let Ok(it) = std::fs::read_dir(b) {
        for e in it.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.extension().map_or(false, |ext| ext == "png") && !a.join(p.file_name().unwrap()).exists() {
                errors.push(format!("missing on left: {}", p.file_name().unwrap().to_string_lossy()));
            }
        }
    }
    errors
}

/// Copy mismatching PNGs into `<artifact_dir>/<left|right>/<basename>` so a
/// failed CI run produces a self-contained directory for `actions/upload-
/// artifact`.  We don't synthesize a diff image — eyeballing the two
/// alongside each other is enough to characterize most regressions.
fn dump_png_artifact(artifact_dir: &Path, left: &Path, right: &Path) -> std::io::Result<()> {
    let l_dst = artifact_dir.join("csx");
    let r_dst = artifact_dir.join("rust");
    std::fs::create_dir_all(&l_dst)?;
    std::fs::create_dir_all(&r_dst)?;
    if let Some(name) = left.file_name() {
        std::fs::copy(left, l_dst.join(name))?;
    }
    if let Some(name) = right.file_name() {
        std::fs::copy(right, r_dst.join(name))?;
    }
    Ok(())
}

// =====================================================================
// Tests
// =====================================================================

/// Self-check: `diff_json` itself works as advertised.
#[test]
fn json_diff_recognizes_eps() {
    let a: Value = serde_json::from_str(r#"{"x": 1.0, "y": [2.0, 3.0]}"#).unwrap();
    let b: Value = serde_json::from_str(r#"{"x": 1.0000001, "y": [2.0, 3.0]}"#).unwrap();
    diff_json(&a, &b, "").expect("close enough");

    let c: Value = serde_json::from_str(r#"{"x": 1.0001, "y": [2.0, 3.0]}"#).unwrap();
    diff_json(&a, &c, "").expect_err("should diverge");
}

#[test]
fn rust_export_runs_on_synthetic() {
    // `tests/data/synthetic_minimal.dat` is committed to the repo so the CI
    // gate doesn't depend on the vanilla MKGP2 corpus.  Until Phase 5 ships
    // the writer, we generate the synthetic fixture in-place from the same
    // hand-crafted byte literal used by `dat::tests::parses_minimal_dat`.
    let synthetic = make_synthetic_minimal();
    let dat = Dat::parse(&synthetic).expect("parse synthetic");
    assert_eq!(dat.roots.len(), 1);
    assert_eq!(dat.roots[0].name, "scene_data");

    // export shouldn't crash even on a barebones SOBJ with no JOBJDescs.
    let scene = export::export_scene(&dat, "synthetic_minimal.dat", None)
        .expect("export ok");
    assert_eq!(scene.source_dat, "synthetic_minimal.dat");
}

#[test]
fn vanilla_corpus_round_trips() {
    if mkgp2_files_dir().is_none() {
        eprintln!("skipped: MKGP2_FILES_DIR not set");
        return;
    }
    if mkgp2_patch_dir().is_none() {
        eprintln!("skipped: MKGP2_PATCH_DIR not set");
        return;
    }
    if !dotnet_script_available() {
        eprintln!("skipped: dotnet-script unavailable");
        return;
    }

    let files_dir = mkgp2_files_dir().unwrap();
    // The handoff prescribes "MR_highway 短/長, mc_jungle, mc_kingdom,
    // mc_palace, st_pyramid (6 コース)". The MKGP2 vanilla files actually
    // ship under different prefixes (AT_/DK_/DNA_/MR_/…), so we pick six
    // real course .dat files that exercise the full breadth of texture
    // formats (CMP/RGBA8/RGB5A3/CI8/IA8) and PObj layouts.
    let target_files = [
        "test_course_start_gate.dat", // synthetic-ish smoke test
        "MR_highway_short_A.dat",
        "MR_highway_long_A.dat",
        "DK_jungle_short_a.dat",
        "DK_jungle_long_a.dat",
        "AT_demo.dat",
    ];

    for name in target_files {
        let dat = files_dir.join(name);
        if !dat.exists() {
            eprintln!("  [skip] {} not present", name);
            continue;
        }
        let stage = workspace_root().join("target").join("parity").join(name);
        let _ = std::fs::remove_dir_all(&stage);
        let csx_dir = stage.join("csx");
        let rust_dir = stage.join("rust");
        std::fs::create_dir_all(&csx_dir).unwrap();
        std::fs::create_dir_all(&rust_dir).unwrap();

        run_csx(&dat, &csx_dir).expect("csx run");
        run_rust(&dat, &rust_dir).expect("rust run");

        // scene.json semantic diff
        diff_json_files(&csx_dir.join("scene.json"), &rust_dir.join("scene.json"))
            .unwrap_or_else(|JsonDiff(msg)| {
                panic!("{}: scene.json mismatch: {}", name, msg)
            });

        // PNG diff (pixel-equal acceptable).  Mismatches dump both csx and
        // rust copies into `target/parity/<file>/artifacts/` so CI can
        // upload it as a single artifact bundle for offline triage.
        let artifact_dir = stage.join("artifacts");
        let png_errs = diff_tex_dirs(&csx_dir.join("tex"), &rust_dir.join("tex"), &artifact_dir);
        if !png_errs.is_empty() {
            panic!(
                "{}: png mismatch (artifacts at {}):\n  - {}",
                name,
                artifact_dir.display(),
                png_errs.join("\n  - ")
            );
        }

        eprintln!("  ✓ {} parity OK", name);
    }
}

// =====================================================================
// Synthetic minimal .dat (≈0x48 bytes) — same hand-crafted file used in the
// dat parser unit test.  Lives in test code rather than as a checked-in
// binary while Phase 5 / the writer is pending.
// =====================================================================

fn make_synthetic_minimal() -> Vec<u8> {
    use byteorder::{BigEndian, ByteOrder};
    let mut buf = vec![0u8; 0x48];
    BigEndian::write_u32(&mut buf[0x00..0x04], 0x48); // fsize
    BigEndian::write_u32(&mut buf[0x04..0x08], 0x10); // reloc_offset_rel
    BigEndian::write_u32(&mut buf[0x08..0x0C], 0x00); // reloc_count
    BigEndian::write_u32(&mut buf[0x0C..0x10], 0x01); // root_count
    BigEndian::write_u32(&mut buf[0x10..0x14], 0x00); // ref_count
    BigEndian::write_u32(&mut buf[0x30..0x34], 0x00); // root data_rel
    BigEndian::write_u32(&mut buf[0x34..0x38], 0x00); // root str_rel
    let name = b"scene_data\0";
    buf[0x38..0x38 + name.len()].copy_from_slice(name);
    buf
}
