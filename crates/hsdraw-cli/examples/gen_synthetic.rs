//! Generates `crates/hsdraw-core/tests/data/synthetic_minimal.dat` — the
//! committed minimal fixture for the CI parity gate.  Run once, commit
//! result; idempotent.
//!
//!     cargo run -p hsdraw-cli --example gen_synthetic
//!
//! Building the file from a hand-written byte buffer rather than from
//! Rust API calls means the layout is exactly what the unit tests in
//! `dat.rs::tests` expect.

use std::path::PathBuf;

use hsdraw_core::Dat;

fn write_be_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn main() {
    // Same hand-built layout used by `dat::tests::parses_minimal_dat`:
    //   header → one zero-filled struct → empty reloc table → one root
    //   pointing at offset 0 with name "scene_data".
    let mut buf = vec![0u8; 0x48];
    write_be_u32(&mut buf, 0x00, 0x48); // fsize
    write_be_u32(&mut buf, 0x04, 0x10); // reloc_offset_rel
    write_be_u32(&mut buf, 0x08, 0x00); // reloc_count
    write_be_u32(&mut buf, 0x0C, 0x01); // root_count
    write_be_u32(&mut buf, 0x10, 0x00); // ref_count
    write_be_u32(&mut buf, 0x30, 0x00); // root data_rel
    write_be_u32(&mut buf, 0x34, 0x00); // root str_rel
    let name = b"scene_data\0";
    buf[0x38..0x38 + name.len()].copy_from_slice(name);

    // Round-trip through the writer so the committed fixture is exactly
    // what `Dat::write` produces — that's what the parity test diffs
    // against the parser.  If they ever drift we want to know.
    let dat = Dat::parse(&buf).expect("parse hand-built minimal");
    let canonical = dat.write().expect("write canonical");

    let workspace = std::env::current_dir().expect("cwd");
    let mut out = PathBuf::from(&workspace);
    out.push("crates");
    out.push("hsdraw-core");
    out.push("tests");
    out.push("data");
    std::fs::create_dir_all(&out).expect("mkdir tests/data");
    out.push("synthetic_minimal.dat");
    std::fs::write(&out, &canonical).expect("write fixture");
    println!("wrote {} ({} bytes)", out.display(), canonical.len());
}
