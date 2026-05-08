//! `hsdraw-cli` — Phase 1 dumper.  `hsdraw-cli decode foo.dat` walks every
//! public root and prints a JObj/DObj/MObj/POBJ tree to stdout, modeled after
//! `mkgp2-patch/tools/hsd/hsd_dump.csx`.  Future phases extend this with
//! `--json out/`, texture export, etc.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hsdraw_core::accessor::{Accessor, id_of};
use hsdraw_core::common::{DObj, JObj, MObj, PObj, SObj, TObj};
use hsdraw_core::error::HsdError;
use hsdraw_core::gx::{jobj_flag_names, render_flag_names};
use hsdraw_core::Dat;

#[derive(Parser, Debug)]
#[command(version, about = "HSD .dat reader/writer (work in progress)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Parse a .dat and print a JObj/DObj tree dump to stdout.
    Decode {
        dat: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Decode { dat } => decode(&dat),
    }
}

fn decode(path: &PathBuf) -> Result<()> {
    let bytes = fs::read(path)
        .with_context(|| format!("read {}", path.display()))?;
    let dat = Dat::parse(&bytes)
        .with_context(|| format!("parse {}", path.display()))?;

    println!("File: {}", path.display());
    println!(
        "Version: {:?} (raw 0x{:08X})",
        std::str::from_utf8(&dat.version).unwrap_or("?"),
        u32::from_be_bytes(dat.version)
    );
    println!("Roots: {}", dat.roots.len());
    println!("References: {}", dat.references.len());
    println!("Structs: {}", dat.struct_order.len());
    println!();

    let mut visited: HashSet<*const std::cell::RefCell<hsdraw_core::HsdStruct>> = HashSet::new();
    let mut jobj_counter = 0u32;

    for root in &dat.roots {
        println!(
            "=== Root '{}' (struct len 0x{:X}) ===",
            root.name,
            root.data.borrow().len()
        );

        // Heuristic kind detection by length / the symbol-id rules from
        // HSDLib's `GuessAccessor`.  We stay conservative: if the struct
        // length matches a JObj (0x40+) and the name doesn't suggest SObj,
        // treat it as a JObj root; otherwise check for SObj shape.
        let is_scene_data = root.name == "scene_data";
        let is_jobj_root = root.name.ends_with("_joint");

        if is_scene_data {
            let sobj = SObj::from_struct(root.data.clone());
            for (i, desc) in sobj.jobj_descs().into_iter().enumerate() {
                println!("  JOBJDesc[{}]:", i);
                if let Some(rj) = desc.root_joint() {
                    walk_jobj(&rj, 2, &mut jobj_counter, &mut visited)?;
                }
            }
        } else if is_jobj_root {
            let jobj = JObj::from_struct(root.data.clone());
            walk_jobj(&jobj, 1, &mut jobj_counter, &mut visited)?;
        } else {
            println!("  (unrecognized root kind, skipping body dump)");
        }

        println!();
    }

    Ok(())
}

fn walk_jobj(
    j: &JObj,
    depth: usize,
    counter: &mut u32,
    visited: &mut HashSet<*const std::cell::RefCell<hsdraw_core::HsdStruct>>,
) -> std::result::Result<(), HsdError> {
    // Iterate the linked Next chain at this level; recurse into Child.
    let mut cur = Some(j.clone());
    while let Some(jobj) = cur {
        let prefix = "  ".repeat(depth);
        let id_key = id_of(&jobj);
        let already = !visited.insert(id_key);

        let n = *counter;
        *counter += 1;

        let flags = jobj.flags()?;
        let names = jobj_flag_names(flags);

        let alias_marker = if already { " (ALIAS)" } else { "" };
        println!(
            "{}JObj#{} flags=[{}]{}  T=({:.3},{:.3},{:.3}) R=({:.3},{:.3},{:.3}) S=({:.3},{:.3},{:.3})",
            prefix,
            n,
            names.join(", "),
            alias_marker,
            jobj.tx()?, jobj.ty()?, jobj.tz()?,
            jobj.rx()?, jobj.ry()?, jobj.rz()?,
            jobj.sx()?, jobj.sy()?, jobj.sz()?,
        );

        if !already {
            // DObjs
            if let Some(d) = jobj.dobj()? {
                walk_dobj(&d, depth + 1)?;
            }
            // recurse into Child subtree
            if let Some(child) = jobj.child() {
                walk_jobj(&child, depth + 1, counter, visited)?;
            }
        }

        cur = jobj.next();
    }
    Ok(())
}

fn walk_dobj(d: &DObj, depth: usize) -> std::result::Result<(), HsdError> {
    let mut cur = Some(d.clone());
    let mut idx = 0;
    while let Some(dobj) = cur {
        let prefix = "  ".repeat(depth);
        println!(
            "{}DObj#{} class={:?}",
            prefix, idx,
            dobj.class_name()?.unwrap_or_default()
        );
        if let Some(m) = dobj.mobj() {
            walk_mobj(&m, depth + 1)?;
        }
        if let Some(p) = dobj.pobj() {
            walk_pobj(&p, depth + 1)?;
        }
        cur = dobj.next();
        idx += 1;
    }
    Ok(())
}

fn walk_mobj(m: &MObj, depth: usize) -> std::result::Result<(), HsdError> {
    let prefix = "  ".repeat(depth);
    let flags = m.render_flags()?;
    println!(
        "{}MObj RenderFlags=[{}] (0x{:08X})",
        prefix,
        render_flag_names(flags).join(", "),
        flags.bits(),
    );
    if let Some(mat) = m.material() {
        let dif = mat.dif_rgba()?;
        println!(
            "{}  Mat DIF=({},{},{},{}) Alpha={:.3} Shininess={:.3}",
            prefix, dif[0], dif[1], dif[2], dif[3],
            mat.alpha()?, mat.shininess()?,
        );
    }
    if let Some(t) = m.textures() {
        walk_tobj(&t, depth + 1)?;
    }
    Ok(())
}

fn walk_tobj(t: &TObj, depth: usize) -> std::result::Result<(), HsdError> {
    let mut cur = Some(t.clone());
    let mut idx = 0;
    while let Some(tobj) = cur {
        let prefix = "  ".repeat(depth);
        let img = tobj.image_data();
        let (w, h, fmt) = if let Some(i) = &img {
            (i.width().unwrap_or(0), i.height().unwrap_or(0), format!("{:?}", i.format().unwrap_or(hsdraw_core::gx::GxTexFmt::Unknown(0))))
        } else {
            (0, 0, "<no image>".to_owned())
        };
        println!(
            "{}TObj#{} TexMap={:?} Wrap=(S:{:?}, T:{:?}) ColorOp={:?} AlphaOp={:?} Blending={:.3} {}x{} {}",
            prefix, idx,
            tobj.tex_map_id()?,
            tobj.wrap_s()?, tobj.wrap_t()?,
            tobj.color_operation()?, tobj.alpha_operation()?,
            tobj.blending()?,
            w, h, fmt,
        );
        cur = tobj.next();
        idx += 1;
    }
    Ok(())
}

fn walk_pobj(p: &PObj, depth: usize) -> std::result::Result<(), HsdError> {
    let mut cur = Some(p.clone());
    let mut idx = 0;
    while let Some(pobj) = cur {
        let prefix = "  ".repeat(depth);
        let dl_size = pobj.display_list_size()?;
        let dl_first = pobj
            .display_list_buffer()
            .map(|b| {
                b.iter()
                    .take(8)
                    .map(|x| format!("{:02X}", x))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let sb = pobj.single_bound_jobj()?;
        println!(
            "{}PObj#{} flags=0x{:04X} DLsize={} DL[0..8]={}{}",
            prefix, idx,
            pobj.flags()?.bits(),
            dl_size,
            dl_first,
            if sb.is_some() { " sb=Yes" } else { "" },
        );
        cur = pobj.next();
        idx += 1;
    }
    Ok(())
}

