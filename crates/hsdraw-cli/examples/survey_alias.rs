//! Find structs that are reachable as a sub-struct of one root *and* are
//! the direct `data` of a different root — this is the alias-root pattern
//! `mkgp2docs/hsd_alias_and_blender_pipeline.md` describes for
//! `MR_highway_*_joint` (the same JObj `_s` is exposed via both
//! `scene_data.RootJoint.Child_N` and a top-level `*_joint` symbol).
//!
//! Usage:
//!     HSDRAW_PARITY_CORPUS_DIR="..." cargo run -p hsdraw-cli --example survey_alias
//!
//! (The older `MKGP2_FILES_DIR` env name is still honored as a
//! back-compat alias and will be dropped in a future release.)
//!
//! Output: `name, n_alias_roots`  for every file with at least one alias.

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use hsdraw_core::{Dat, hsd_struct::collect_substructs};

fn main() {
    let dir = match std::env::var("HSDRAW_PARITY_CORPUS_DIR")
        .or_else(|_| std::env::var("MKGP2_FILES_DIR"))
    {
        Ok(v) => PathBuf::from(v),
        Err(_) => {
            eprintln!("HSDRAW_PARITY_CORPUS_DIR not set");
            std::process::exit(2);
        }
    };
    // Optional second arg: a single .dat path under any directory (used to
    // include the dynamically-generated `MR_highway_short_A_inu_aliased.dat`).
    let extras: Vec<PathBuf> = std::env::args()
        .skip(1)
        .map(PathBuf::from)
        .collect();

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "dat"))
        .collect();
    entries.sort();
    entries.extend(extras);

    println!("name,roots,refs,alias_root_count");
    for path in entries {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let dat = match Dat::parse(&bytes) {
            Ok(d) => d,
            Err(_) => continue,
        };

        // Collect all sub-structs reachable from each root individually.
        // We then ask: for each root `j`, is `dat.roots[j].data` reachable
        // as a sub-struct of some *other* root `i ≠ j`?  That's the alias.
        let per_root_subs: Vec<HashSet<*const _>> = dat
            .roots
            .iter()
            .map(|r| {
                collect_substructs(&r.data)
                    .iter()
                    .map(Rc::as_ptr)
                    .collect()
            })
            .collect();

        let mut alias_count = 0;
        for (j, rj) in dat.roots.iter().enumerate() {
            let pj = Rc::as_ptr(&rj.data);
            for (i, subs_i) in per_root_subs.iter().enumerate() {
                if i == j {
                    continue;
                }
                if subs_i.contains(&pj) {
                    alias_count += 1;
                    break;
                }
            }
        }

        if alias_count > 0 || !dat.references.is_empty() {
            println!(
                "{},{},{},{}",
                path.file_name().unwrap().to_string_lossy(),
                dat.roots.len(),
                dat.references.len(),
                alias_count
            );
        }
    }
}
