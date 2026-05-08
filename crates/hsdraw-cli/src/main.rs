//! `hsdraw-cli` — Phase 1 dumper.  `hsdraw-cli decode foo.dat` walks every
//! public root and prints a JObj/DObj/MObj/POBJ tree to stdout, modeled after
//! `mkgp2-patch/tools/hsd/hsd_dump.csx`.  Future phases extend this with
//! `--json out/`, texture export, etc.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hsdraw_core::accessor::{Accessor, id_of};
use hsdraw_core::common::{DObj, JObj, MObj, PObj, SObj, TObj};
use hsdraw_core::error::HsdError;
use hsdraw_core::export;
use hsdraw_core::gx::{jobj_flag_names, render_flag_names};
use hsdraw_core::gx_dl;
use hsdraw_core::gx_image;
use hsdraw_core::Dat;

#[derive(Parser, Debug)]
#[command(version, about = "HSD .dat reader/writer (work in progress)")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Parse a .dat and print a JObj/DObj tree dump to stdout.  When
    /// `--out` is given, texture data is decoded and written as
    /// `<out>/tex/<sha1>.png` next to a future `scene.json` (Phase 4).
    Decode {
        dat: PathBuf,
        /// Output directory for the JSON+PNG bundle.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// `parse → write`: read a .dat and emit a freshly-serialized .dat at
    /// `--out`.  Useful as a smoke test for the writer (Phase 5) and as a
    /// debugging tool: comparing input vs output sizes / reloc counts often
    /// surfaces issues quickly.
    Encode {
        dat: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Disable buffer dedup + unreachable struct removal.
        #[arg(long, default_value_t = false)]
        no_optimize: bool,
        /// Disable 0x20 alignment for buffers (textures will likely break).
        #[arg(long, default_value_t = false)]
        no_buffer_align: bool,
    },
    /// Apply a `scene.json` mutation set onto a base .dat and write the
    /// result.  Equivalent to running
    /// `mkgp2-patch/tools/hsd/hsd_import_from_blender.csx` on the same
    /// inputs (Pass 0–4: alias add/remove/repoint, TRS+flag sync,
    /// hierarchy rewire, new joint allocation).  Geometry / materials /
    /// textures are not re-encoded.
    Import {
        /// Base .dat to mutate.  Provides every struct that isn't being
        /// edited (mesh DLs, materials, textures, …).
        base: PathBuf,
        /// Bundle directory containing `scene.json` (and a `tex/` dir,
        /// not consumed in this MVP).  Mirrors the csx CLI shape.
        bundle_dir: PathBuf,
        /// Where to write the freshly-serialized .dat.
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Decode { dat, out } => decode(&dat, out.as_deref()),
        Command::Encode { dat, out, no_optimize, no_buffer_align } => {
            encode(&dat, &out, no_optimize, no_buffer_align)
        }
        Command::Import { base, bundle_dir, out } => {
            import(&base, &bundle_dir, &out)
        }
    }
}

fn import(base: &PathBuf, bundle_dir: &PathBuf, out: &PathBuf) -> Result<()> {
    use hsdraw_core::import::import_from_scene_json;
    let scene_path = bundle_dir.join("scene.json");
    let base_bytes = fs::read(base)
        .with_context(|| format!("read {}", base.display()))?;
    let scene_bytes = fs::read(&scene_path)
        .with_context(|| format!("read {}", scene_path.display()))?;
    let (out_bytes, stats) = import_from_scene_json(&base_bytes, &scene_bytes)
        .with_context(|| "import_from_scene_json")?;
    fs::write(out, &out_bytes)
        .with_context(|| format!("write {}", out.display()))?;
    println!(
        "Wrote: {} ({} bytes; base {} bytes)\n  joints walked={} new={} \n  aliases added={} repointed={} removed={}\n  trs-changed={} flags-changed={} hierarchy-rewired={}",
        out.display(),
        out_bytes.len(),
        base_bytes.len(),
        stats.joints_walked, stats.new_joints,
        stats.aliases_added, stats.aliases_repointed, stats.aliases_removed,
        stats.trs_changed, stats.flags_changed, stats.hierarchy_rewired,
    );
    Ok(())
}

fn encode(path: &PathBuf, out: &PathBuf, no_optimize: bool, no_buffer_align: bool) -> Result<()> {
    use hsdraw_core::writer::WriteOptions;
    let bytes = fs::read(path)
        .with_context(|| format!("read {}", path.display()))?;
    let dat = Dat::parse(&bytes)
        .with_context(|| format!("parse {}", path.display()))?;

    let opts = WriteOptions {
        optimize: !no_optimize,
        buffer_align: !no_buffer_align,
        trim: false,
    };
    let written = dat
        .write_with_options(opts)
        .with_context(|| "write failed")?;
    fs::write(out, &written)
        .with_context(|| format!("write {}", out.display()))?;
    println!(
        "Wrote: {} ({} bytes; original {} bytes)",
        out.display(),
        written.len(),
        bytes.len()
    );
    Ok(())
}

fn decode(path: &PathBuf, out: Option<&Path>) -> Result<()> {
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

    // When --out is given, drive the canonical csx-equivalent exporter.
    // The tree dump path below stays available for sanity checking; the
    // exporter and the dump are independent walks of the same Rc tree.
    if let Some(out_dir) = out {
        fs::create_dir_all(out_dir)
            .with_context(|| format!("mkdir {}", out_dir.display()))?;
        let tex_dir = out_dir.join("tex");
        let source_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>")
            .to_owned();
        let scene = export::export_scene(&dat, source_name, Some(&tex_dir))?;
        let json = serde_json::to_string(&scene)
            .with_context(|| "serde_json::to_string failed")?;
        let json_path = out_dir.join("scene.json");
        fs::write(&json_path, json.as_bytes())
            .with_context(|| format!("write {}", json_path.display()))?;
        println!("\nWrote: {}", json_path.display());
        println!("  textures: {}", scene.textures.len());
        println!("  materials: {}", scene.materials.len());
        println!(
            "  joints: {} (aliases: {})",
            scene.joints.len(),
            scene.joint_aliases.len()
        );
        println!("  meshes: {}", scene.meshes.len());
        return Ok(());
    }

    let tex_state_dir: Option<PathBuf> = None;
    let mut tex_state = TextureExport::new(tex_state_dir);
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
                    walk_jobj(
                        &rj,
                        2,
                        &mut jobj_counter,
                        &mut visited,
                        &mut tex_state,
                    )?;
                }
            }
        } else if is_jobj_root {
            let jobj = JObj::from_struct(root.data.clone());
            walk_jobj(
                &jobj,
                1,
                &mut jobj_counter,
                &mut visited,
                &mut tex_state,
            )?;
        } else {
            println!("  (unrecognized root kind, skipping body dump)");
        }

        println!();
    }

    if let Some(stats) = tex_state.finish()? {
        println!(
            "Textures: {} unique (skipped {} dup, errors {})",
            stats.unique, stats.duplicates, stats.errors
        );
    }

    Ok(())
}

/// Tracks dedup of textures by SHA-1 of the *encoded* GX byte buffer (not
/// the decoded RGBA), matching csx so the resulting PNG file names are the
/// same on both sides.
struct TextureExport {
    tex_dir: Option<PathBuf>,
    sha_seen: HashMap<String, ()>,
    unique: usize,
    duplicates: usize,
    errors: usize,
}

#[derive(Debug)]
struct TextureExportStats {
    unique: usize,
    duplicates: usize,
    errors: usize,
}

impl TextureExport {
    fn new(tex_dir: Option<PathBuf>) -> Self {
        Self {
            tex_dir,
            sha_seen: HashMap::new(),
            unique: 0,
            duplicates: 0,
            errors: 0,
        }
    }

    /// Returns the 12-hex-char id (matches csx `Sha1(...).Substring(0, 12)`).
    fn intern(&mut self, tobj: &TObj) -> Option<String> {
        let img = tobj.image_data()?;
        let raw = img.image_data()?;
        if raw.is_empty() {
            return None;
        }
        let sha = sha1_short_id(&raw);

        // Skip if we've seen this exact encoded buffer before.
        if self.sha_seen.contains_key(&sha) {
            self.duplicates += 1;
            return Some(sha);
        }

        if let Some(dir) = &self.tex_dir {
            let w = match img.width() {
                Ok(v) if v > 0 => v as u32,
                _ => {
                    self.errors += 1;
                    return None;
                }
            };
            let h = match img.height() {
                Ok(v) if v > 0 => v as u32,
                _ => {
                    self.errors += 1;
                    return None;
                }
            };
            let fmt = match img.format() {
                Ok(f) => f,
                Err(_) => {
                    self.errors += 1;
                    return None;
                }
            };

            let palette = tobj.tlut_data().and_then(|t| {
                let bytes = t.tlut_data()?;
                let f = t.format().ok()?;
                Some((f, bytes))
            });
            let palette_ref = palette.as_ref().map(|(f, b)| (*f, b.as_slice()));

            match gx_image::decode_image(fmt, w, h, &raw, palette_ref) {
                Ok(rgba) => match gx_image::encode_png(&rgba, w, h) {
                    Ok(png) => {
                        let path = dir.join(format!("{}.png", sha));
                        if let Err(e) = fs::write(&path, &png) {
                            eprintln!("  WARN: write {} failed: {}", path.display(), e);
                            self.errors += 1;
                            return None;
                        }
                        self.unique += 1;
                    }
                    Err(e) => {
                        eprintln!("  WARN: png encode failed for {}: {:?}", sha, e);
                        self.errors += 1;
                        return None;
                    }
                },
                Err(e) => {
                    eprintln!("  WARN: decode failed for {} ({:?}): {:?}", sha, fmt, e);
                    self.errors += 1;
                    return None;
                }
            }
        } else {
            self.unique += 1;
        }

        self.sha_seen.insert(sha.clone(), ());
        Some(sha)
    }

    fn finish(self) -> Result<Option<TextureExportStats>> {
        if self.tex_dir.is_none() && self.unique == 0 && self.duplicates == 0 && self.errors == 0 {
            return Ok(None);
        }
        Ok(Some(TextureExportStats {
            unique: self.unique,
            duplicates: self.duplicates,
            errors: self.errors,
        }))
    }
}

fn sha1_short_id(data: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let hash = Sha1::digest(data);
    // csx: `BitConverter.ToString(hash).Replace("-","").Substring(0,12)`
    // → first 12 hex chars, uppercase.
    let mut s = String::with_capacity(12);
    for b in &hash[..6] {
        use std::fmt::Write;
        let _ = write!(&mut s, "{:02X}", b);
    }
    s
}

fn walk_jobj(
    j: &JObj,
    depth: usize,
    counter: &mut u32,
    visited: &mut HashSet<*const std::cell::RefCell<hsdraw_core::HsdStruct>>,
    tex: &mut TextureExport,
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
                walk_dobj(&d, depth + 1, tex)?;
            }
            // recurse into Child subtree
            if let Some(child) = jobj.child() {
                walk_jobj(&child, depth + 1, counter, visited, tex)?;
            }
        }

        cur = jobj.next();
    }
    Ok(())
}

fn walk_dobj(d: &DObj, depth: usize, tex: &mut TextureExport) -> std::result::Result<(), HsdError> {
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
            walk_mobj(&m, depth + 1, tex)?;
        }
        if let Some(p) = dobj.pobj() {
            walk_pobj(&p, depth + 1)?;
        }
        cur = dobj.next();
        idx += 1;
    }
    Ok(())
}

fn walk_mobj(m: &MObj, depth: usize, tex: &mut TextureExport) -> std::result::Result<(), HsdError> {
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
        walk_tobj(&t, depth + 1, tex)?;
    }
    Ok(())
}

fn walk_tobj(t: &TObj, depth: usize, tex: &mut TextureExport) -> std::result::Result<(), HsdError> {
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
        let sha = tex.intern(&tobj).unwrap_or_default();
        println!(
            "{}TObj#{} TexMap={:?} Wrap=(S:{:?}, T:{:?}) ColorOp={:?} AlphaOp={:?} Blending={:.3} {}x{} {} sha={}",
            prefix, idx,
            tobj.tex_map_id()?,
            tobj.wrap_s()?, tobj.wrap_t()?,
            tobj.color_operation()?, tobj.alpha_operation()?,
            tobj.blending()?,
            w, h, fmt,
            sha,
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
        let sb = pobj.single_bound_jobj()?;

        // Unpack the DL fully so we surface vertex/primitive counts in the
        // dump.  Errors are non-fatal — we just note them and keep walking.
        let dl_summary = match gx_dl::unpack(&pobj) {
            Ok(dl) => format!(
                "verts={} prims={} attrs={}",
                dl.total_vertices(),
                dl.primitives.len(),
                dl.attributes.len().saturating_sub(1) // exclude NULL terminator
            ),
            Err(e) => format!("DL_ERR: {:?}", e),
        };

        println!(
            "{}PObj#{} flags=0x{:04X} DLsize={} {}{}",
            prefix, idx,
            pobj.flags()?.bits(),
            dl_size,
            dl_summary,
            if sb.is_some() { " sb=Yes" } else { "" },
        );
        cur = pobj.next();
        idx += 1;
    }
    Ok(())
}

