//! Reverse direction of `export.rs` — apply a `scene.json` mutation set
//! back onto a base .dat.  Mirrors `mkgp2-patch/tools/hsd/
//! hsd_import_from_blender.csx` Pass 0–4 step-for-step:
//!
//! - **Pass 0**: walk base scene_data tree DFS, naming joints `jobj_N`
//!   in the same order the export script assigned (so the IDs in
//!   `scene.json` line up with the corresponding HSDStructs).  Aliased
//!   joints are recognized by Rc identity and *not* re-numbered.
//! - **Pass 1**: allocate a fresh 0x40-byte HSD_JOBJ for every JSON
//!   joint id that didn't show up in the base walk.  Identity scale,
//!   rest is zero-init.  HSDLib's writer GCs orphans (= unreachable from
//!   any root) so leaving an unreferenced new joint in the map is fine.
//! - **Pass 2**: alias additions, repointing, and stale removals against
//!   `dat.roots`.  Stale = name ends with `_joint`, isn't `scene_data`,
//!   isn't in `joint_aliases`.  (csx uses `r.Data is HSD_JOBJ`; we use
//!   the name-suffix that HSDLib's `GuessAccessor` keys off.)
//! - **Pass 3**: write JOBJ_FLAG + TRS values from JSON onto each
//!   matched joint.  Both sides are compared first to avoid dirtying a
//!   clean struct.
//! - **Pass 4**: rebuild `JObj.Child` and `JObj.Next` chains so the
//!   parent's first child + sibling order match `joint.children[]`.
//!
//! Geometry / DObj content / textures are not re-encoded here — same
//! Phase-1 MVP scope as the csx script.  See `docs/roadmap.md` for
//! POBJ writer plans.
//!
//! `import_from_scene_json` is the only public entry point; everything
//! else lives in helpers parallel to the C# script's lambda-style
//! breakdown.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::accessor::Accessor;
use crate::common::{JObj, JObjDesc, SObj};
use crate::dat::{Dat, RootNode};
use crate::error::{HsdError, Result};
use crate::gx::{jobj_flag_from_name, JObjFlag};
use crate::hsd_struct::{HsdStruct, StructRef, identity, ptr_eq};

/// Per-joint deltas read out of `scene.json`.  Field names match the csx
/// exporter; missing fields fall back to writer-friendly defaults.
#[derive(Debug, Deserialize)]
struct JointDelta {
    id: String,
    #[serde(default)]
    flags: Vec<String>,
    #[serde(default)]
    translation: Vec<f32>, // expect 3
    #[serde(default)]
    rotation: Vec<f32>,
    #[serde(default)]
    scale: Vec<f32>,
    #[serde(default)]
    children: Vec<String>,
    // `parent` is informational; the children[] arrays are canonical.
    // Kept for completeness so the field round-trips through serde even
    // though we don't read it directly (the rewire pass works off
    // children[]).
    #[serde(default)]
    #[allow(dead_code)]
    parent: Option<String>,
}

/// Top-level subset of `scene.json` we actually consume.  Other keys
/// (textures, materials, meshes, source_dat, …) are ignored — the
/// importer only edits joint hierarchy + alias roots in this phase.
#[derive(Debug, Deserialize, Default)]
struct SceneEdits {
    #[serde(default)]
    joints: Vec<JointDelta>,
    #[serde(default)]
    joint_aliases: indexmap::IndexMap<String, String>,
}

/// Counters returned for logging / parity-test verification.
#[derive(Debug, Default, Clone, Copy)]
pub struct ImportStats {
    pub joints_walked: u32,
    pub new_joints: u32,
    pub aliases_added: u32,
    pub aliases_repointed: u32,
    pub aliases_removed: u32,
    pub trs_changed: u32,
    pub flags_changed: u32,
    pub hierarchy_rewired: u32,
}

/// Parse `base_dat`, apply the edits in `scene_json_bytes`, and return
/// freshly-serialized .dat bytes.  Equivalent to running:
///
/// ```text
/// dotnet-script hsd_import_from_blender.csx -- base.dat <bundle dir> out.dat
/// ```
///
/// (the `<bundle dir>` argument was only used by csx to locate
/// `scene.json`; we take the JSON contents directly.)
///
/// Returns the produced bytes plus an `ImportStats` describing what
/// changed.  Caller writes the bytes wherever they want.
pub fn import_from_scene_json(
    base_dat: &[u8],
    scene_json_bytes: &[u8],
) -> Result<(Vec<u8>, ImportStats)> {
    let mut dat = Dat::parse(base_dat)?;
    let scene: SceneEdits = serde_json::from_slice(scene_json_bytes)
        .map_err(|e| HsdError::malformed(0, format!("scene.json parse: {}", e)))?;

    let mut stats = ImportStats::default();

    // ----- Pass 0: build jobj_id → JObj map by walking base tree ------
    let mut counter: u32 = 0;
    let mut jobj_by_id: indexmap::IndexMap<String, JObj> = indexmap::IndexMap::new();
    let mut id_by_struct: HashMap<*const std::cell::RefCell<HsdStruct>, String> =
        HashMap::new();

    if let Some(scene_root) = dat.roots.iter().find(|r| r.name == "scene_data") {
        let sobj = SObj::from_struct(scene_root.data.clone());
        for desc in sobj.jobj_descs() {
            let desc: JObjDesc = desc;
            if let Some(rj) = desc.root_joint() {
                emit_joint(rj, &mut counter, &mut jobj_by_id, &mut id_by_struct);
            }
        }
    }
    // Then any non-scene_data root that is itself a HSD_JOBJ.  Aliases
    // already walked via the SOBJ tree are skipped by the identity check
    // in `emit_joint`.
    for r in &dat.roots {
        if r.name == "scene_data" {
            continue;
        }
        if !looks_like_jobj_root(&r.name) {
            continue;
        }
        let j = JObj::from_struct(r.data.clone());
        emit_joint(j, &mut counter, &mut jobj_by_id, &mut id_by_struct);
    }
    stats.joints_walked = counter;

    // ----- Pass 1: allocate new HSD_JOBJ for unknown JSON ids ---------
    for jdto in &scene.joints {
        if jdto.id.is_empty() {
            continue;
        }
        if jobj_by_id.contains_key(&jdto.id) {
            continue;
        }
        let nj = JObj::allocate_default();
        id_by_struct.insert(identity(&nj.0), jdto.id.clone());
        jobj_by_id.insert(jdto.id.clone(), nj);
        stats.new_joints += 1;
    }

    // ----- Pass 2: alias add / repoint / remove -----------------------
    // Existing roots indexed by name so alias adds can repoint in-place.
    // We track new additions in a separate buffer so we don't perturb
    // the existing index during iteration.
    let mut existing_by_name: HashMap<String, usize> = HashMap::new();
    for (i, r) in dat.roots.iter().enumerate() {
        existing_by_name.insert(r.name.clone(), i);
    }
    let mut additions: Vec<RootNode> = Vec::new();

    for (alias_name, target_id) in &scene.joint_aliases {
        if alias_name.trim().is_empty() {
            return Err(HsdError::malformed(0, "alias name empty"));
        }
        let Some(target) = jobj_by_id.get(target_id) else {
            return Err(HsdError::malformed(
                0,
                format!(
                    "alias '{}' references unknown joint id '{}'",
                    alias_name, target_id
                ),
            ));
        };
        match existing_by_name.get(alias_name) {
            Some(&i) => {
                if !ptr_eq(&dat.roots[i].data, &target.0) {
                    dat.roots[i].data = target.0.clone();
                    stats.aliases_repointed += 1;
                }
            }
            None => {
                additions.push(RootNode {
                    name: alias_name.clone(),
                    data: target.0.clone(),
                });
                stats.aliases_added += 1;
            }
        }
    }
    dat.roots.extend(additions);

    // Stale removals — JObj-shaped roots not present in joint_aliases.
    let json_alias_names: HashSet<&str> =
        scene.joint_aliases.keys().map(|s| s.as_str()).collect();
    let mut keep: Vec<bool> = Vec::with_capacity(dat.roots.len());
    for r in &dat.roots {
        let drop = r.name != "scene_data"
            && looks_like_jobj_root(&r.name)
            && !json_alias_names.contains(r.name.as_str());
        keep.push(!drop);
    }
    let mut new_roots: Vec<RootNode> = Vec::with_capacity(keep.iter().filter(|k| **k).count());
    for (r, k) in dat.roots.drain(..).zip(keep.iter()) {
        if *k {
            new_roots.push(r);
        } else {
            stats.aliases_removed += 1;
        }
    }
    dat.roots = new_roots;

    // ----- Pass 3: per-joint TRS + flags sync -------------------------
    for jdto in &scene.joints {
        let Some(j) = jobj_by_id.get(&jdto.id) else {
            continue;
        };
        // flags — bitflags 2.x in this build doesn't auto-impl
        // BitOrAssign on a `Flags` newtype with an integer payload, so
        // we OR via `bits()` and reconstruct.  Equivalent semantics.
        let mut new_bits: u32 = 0;
        for name in &jdto.flags {
            match jobj_flag_from_name(name.as_str()) {
                Some(f) => new_bits |= f.bits(),
                None => {
                    return Err(HsdError::malformed(
                        0,
                        format!(
                            "joint {}: unknown JOBJ_FLAG '{}'",
                            jdto.id, name
                        ),
                    ));
                }
            }
        }
        let new_flags = JObjFlag::from_bits_retain(new_bits);
        if j.flags()? != new_flags {
            j.set_flags(new_flags)?;
            stats.flags_changed += 1;
        }
        // TRS
        let t = pad3(&jdto.translation);
        let r = pad3(&jdto.rotation);
        let s = pad3(&jdto.scale);
        let moved = j.tx()? != t[0] || j.ty()? != t[1] || j.tz()? != t[2]
            || j.rx()? != r[0] || j.ry()? != r[1] || j.rz()? != r[2]
            || j.sx()? != s[0] || j.sy()? != s[1] || j.sz()? != s[2];
        if moved {
            j.set_tx(t[0])?; j.set_ty(t[1])?; j.set_tz(t[2])?;
            j.set_rx(r[0])?; j.set_ry(r[1])?; j.set_rz(r[2])?;
            j.set_sx(s[0])?; j.set_sy(s[1])?; j.set_sz(s[2])?;
            stats.trs_changed += 1;
        }
    }

    // ----- Pass 4: hierarchy rewire (Child / Next chain) --------------
    for jdto in &scene.joints {
        let Some(parent) = jobj_by_id.get(&jdto.id) else {
            continue;
        };
        // resolve every child id
        let mut resolved: Vec<JObj> = Vec::with_capacity(jdto.children.len());
        let mut all_resolved = true;
        for cid in &jdto.children {
            match jobj_by_id.get(cid) {
                Some(c) => resolved.push(c.clone()),
                None => {
                    all_resolved = false;
                    break;
                }
            }
        }
        if !all_resolved {
            // csx logs a warning and skips; we surface the same warning
            // path via the error type for parity tests.
            return Err(HsdError::malformed(
                0,
                format!(
                    "joint {}: hierarchy child references unknown id",
                    jdto.id
                ),
            ));
        }

        let desired_child = resolved.first().cloned();
        let mut changed = false;
        if !same_jobj(parent.child(), &desired_child) {
            parent.set_child(desired_child);
            changed = true;
        }
        for i in 0..resolved.len() {
            let desired_next = resolved.get(i + 1).cloned();
            if !same_jobj(resolved[i].next(), &desired_next) {
                resolved[i].set_next(desired_next);
                changed = true;
            }
        }
        if changed {
            stats.hierarchy_rewired += 1;
        }
    }

    // ----- Save -------------------------------------------------------
    let out = dat.write()?;
    Ok((out, stats))
}

// =====================================================================
// Helpers
// =====================================================================

/// Csx's name-keyed JObj-root recognizer, distilled from
/// `HSDRawFile.GuessAccessor` (the only `_joint`-suffixed names map to
/// `HSD_JOBJ`).  We also accept the `*_animjoint`/`*_matanim_joint`/
/// `*_shapeanim_joint` suffixes' base form so the heuristic doesn't
/// mis-prune them, but those aren't aliases the importer cares about.
fn looks_like_jobj_root(name: &str) -> bool {
    name.ends_with("_joint")
        && !name.ends_with("_animjoint")
        && !name.ends_with("matanim_joint")
        && !name.ends_with("shapeanim_joint")
}

fn pad3(v: &[f32]) -> [f32; 3] {
    [
        *v.first().unwrap_or(&0.0),
        *v.get(1).unwrap_or(&0.0),
        *v.get(2).unwrap_or(&0.0),
    ]
}

/// Both empty or both Some(x, y) where ptr_eq(x, y).  Lets the rewire
/// pass skip a write when the chain is already correct.
fn same_jobj(a: Option<JObj>, b: &Option<JObj>) -> bool {
    match (&a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => ptr_eq(&x.0, &y.0),
        _ => false,
    }
}

fn emit_joint(
    j: JObj,
    counter: &mut u32,
    jobj_by_id: &mut indexmap::IndexMap<String, JObj>,
    id_by_struct: &mut HashMap<*const std::cell::RefCell<HsdStruct>, String>,
) {
    let key = identity(&j.0);
    if id_by_struct.contains_key(&key) {
        return;
    }
    let id = format!("jobj_{}", *counter);
    *counter += 1;
    id_by_struct.insert(key, id.clone());
    jobj_by_id.insert(id, j.clone());

    if let Some(child) = j.child() {
        let mut cur = Some(child);
        while let Some(c) = cur {
            emit_joint(c.clone(), counter, jobj_by_id, id_by_struct);
            cur = c.next();
        }
    }
}

#[allow(dead_code)]
fn _struct_ref_anchor(_: StructRef) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_round_trip_via_name() {
        let f = JObjFlag::from_bits_retain((1 << 18) | (1 << 19)); // OPA | XLU
        let names = crate::gx::jobj_flag_names(f);
        let mut bits = 0u32;
        for n in &names {
            bits |= jobj_flag_from_name(n).expect("known").bits();
        }
        assert_eq!(JObjFlag::from_bits_retain(bits), f);
    }

    #[test]
    fn pad3_handles_short_arrays() {
        assert_eq!(pad3(&[1.0, 2.0, 3.0]), [1.0, 2.0, 3.0]);
        assert_eq!(pad3(&[]), [0.0, 0.0, 0.0]);
        assert_eq!(pad3(&[5.0]), [5.0, 0.0, 0.0]);
    }

    #[test]
    fn looks_like_jobj_root_matches_kar_names() {
        assert!(looks_like_jobj_root("MR_highway_inu_joint"));
        assert!(looks_like_jobj_root("DK_jungle_road_joint"));
        assert!(!looks_like_jobj_root("scene_data"));
        assert!(!looks_like_jobj_root("foo_animjoint"));
        assert!(!looks_like_jobj_root("foo_matanim_joint"));
    }
}
