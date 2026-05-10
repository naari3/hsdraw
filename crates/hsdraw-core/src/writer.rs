//! `.dat` writer — mirror of `HSDRaw/HSDRawFile.cs::Save`.
//!
//! Layout:
//! ```text
//!   0x00..0x20   header (filled in last)
//!   0x20..      structs in `_struct_cache` order, each padded per Align flag
//!   ...        4-byte aligned
//!   relocOffset relocation table (32-bit absolute pointer positions, repeated)
//!   ...        root + reference symbol entries (8 bytes: data_off + str_off)
//!   ...        string pool (NUL-terminated)
//! ```
//!
//! Round-trip parity:
//!   `Dat::parse(bytes).write() == bytes` is **not** the bar — HSDLib itself
//!   does not guarantee byte-for-byte round-trip on `Save`.  What we do
//!   guarantee (and what the parity test verifies) is *semantic* round-trip:
//!   `parse(write(parse(bytes)))` produces a `Dat` whose `scene.json` is
//!   identical to the original's, and whose alias structure is preserved.
//!
//! Pointers in struct data:
//!   On parse, the original pointer values stay in the struct's byte buffer
//!   (we don't zero them).  On write we deliberately *overwrite* every
//!   referenced 4-byte slot with the new target offset, so any stale pointer
//!   left over from parse is replaced.
//!
//! Special cases skipped here vs HSDLib:
//!   - `_nextStruct` ordering hack for shape anims (not used by the
//!     course-data path this writer was first validated against)
//!   - SBM_FighterData / MEX_Data / kexData dedup suppression (n/a for
//!     course .dat)
//!   - `Roots[0]` typed as MEX/kex disabling `bufferAlign` (n/a)
//!   - "subaction orphan goto-pointer" fix-up (debug feature)
//! These can be reinstated later if we widen the writer's scope.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use byteorder::{BigEndian, ByteOrder};

use crate::dat::{Dat, RootNode};
use crate::error::{HsdError, Result};
use crate::hsd_struct::{HsdStruct, StructRef, identity};

/// Tweakable knobs.  Defaults match HSDLib `Save(stream)` (no params).
#[derive(Debug, Clone, Copy)]
pub struct WriteOptions {
    /// 0x20-align structs identified as "buffers".  Required for textures
    /// and DL data; HSDLib disables this only for MEX/kex roots.
    pub buffer_align: bool,
    /// Drop unreachable structs from `struct_order` and dedup byte-equal
    /// buffer payloads.  Off → write `struct_order` as-is.
    pub optimize: bool,
    /// Trim each root via accessor `Optimize()` (currently no-op until
    /// accessor `Optimize` lands; reserved).
    pub trim: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self { buffer_align: true, optimize: true, trim: false }
    }
}

impl Dat {
    /// Serialize the .dat with default options.
    pub fn write(&self) -> Result<Vec<u8>> {
        self.write_with_options(WriteOptions::default())
    }

    /// Serialize the .dat with explicit options.
    pub fn write_with_options(&self, opts: WriteOptions) -> Result<Vec<u8>> {
        Writer::new(self, opts).build()
    }
}

// =====================================================================
// IsBuffer — same predicate as `HSDRawFile.IsBuffer`.
// =====================================================================

fn is_buffer(s: &HsdStruct) -> bool {
    if !s.can_be_buffer {
        return false;
    }
    (s.references().is_empty() && s.len() > 0x40) || s.is_buffer_aligned
}

// =====================================================================
// Writer
// =====================================================================

struct Writer<'a> {
    dat: &'a Dat,
    opts: WriteOptions,
    /// All structs reachable from any root or ref, identity-deduped.  Order
    /// here is the same one `HSDRawFile.Save` arrives at after the (struct
    /// cache cleanup + buffer dedup) pass and is what we'll write in.
    cache: Vec<StructRef>,
}

impl<'a> Writer<'a> {
    fn new(dat: &'a Dat, opts: WriteOptions) -> Self {
        Self { dat, opts, cache: Vec::new() }
    }

    fn build(mut self) -> Result<Vec<u8>> {
        self.populate_cache();
        if self.opts.optimize {
            self.remove_duplicate_buffers();
        }
        self.emit()
    }

    /// Walk roots + refs (in declaration order, matching HSDLib's Save loop).
    /// Replicate the cache-cleanup + insert algorithm:
    ///   - start with `dat.struct_order` filtered to "actually reachable"
    ///   - any reachable struct missing from the cache is appended to the
    ///     end (or, if `IsBuffer`, prepended to the front)
    /// We work in identity space: a `*const RefCell<HsdStruct>` keys our
    /// "is in the cache" set so an alias counts as one entry.
    fn populate_cache(&mut self) {
        let all_structs = self.gather_all_structs();
        let all_set: HashSet<*const RefCell<HsdStruct>> =
            all_structs.iter().map(identity).collect();

        // Seed with the parse-time order, dropping anything no longer
        // reachable (HSDLib also does this when `optimize`).
        let mut seen: HashSet<*const RefCell<HsdStruct>> = HashSet::new();
        let mut cache: Vec<StructRef> = Vec::with_capacity(all_structs.len());
        if self.opts.optimize {
            for s in &self.dat.struct_order {
                if all_set.contains(&identity(s)) && seen.insert(identity(s)) {
                    cache.push(s.clone());
                }
            }
        } else {
            for s in &self.dat.struct_order {
                if seen.insert(identity(s)) {
                    cache.push(s.clone());
                }
            }
        }

        // Insert anything reachable that's not yet in the cache.  Buffers go
        // to the *front* (HSDLib inserts at index 0 — successive buffer
        // inserts pile up reverse-relative-to-declaration, but that's still
        // before any normal struct).
        for s in &all_structs {
            if seen.insert(identity(s)) {
                if is_buffer(&s.borrow()) {
                    cache.insert(0, s.clone());
                } else {
                    cache.push(s.clone());
                }
            }
        }

        self.cache = cache;
    }

    /// Pre-order traversal of every root.data and every reference.data.  The
    /// same struct can appear as a sub-structure of multiple roots; we
    /// dedupe by identity.  Order tracks HSDLib `GetAllStructs`: roots first
    /// in declaration order, then refs.
    fn gather_all_structs(&self) -> Vec<StructRef> {
        let mut seen: HashSet<*const RefCell<HsdStruct>> = HashSet::new();
        let mut out = Vec::new();
        for r in self.dat.roots.iter().chain(self.dat.references.iter()) {
            walk_substructs(&r.data, &mut seen, &mut out);
        }
        out
    }

    /// FNV-1a-ish hash of struct payloads, matching `HSDRawFile.ComputeHash`.
    /// Buffers with identical bytes (and `CanBeDuplicate=true`, not a root,
    /// `IsBuffer` true) get rewired so all incoming references point at the
    /// surviving copy; the duplicates are dropped from the cache.
    fn remove_duplicate_buffers(&mut self) {
        let root_set: HashSet<*const RefCell<HsdStruct>> = self
            .dat
            .roots
            .iter()
            .chain(self.dat.references.iter())
            .map(|r| identity(&r.data))
            .collect();

        let mut hash_to_struct: HashMap<i32, StructRef> = HashMap::new();
        let mut redirect: HashMap<*const RefCell<HsdStruct>, StructRef> =
            HashMap::new();

        for s in &self.cache {
            let s_id = identity(s);
            if root_set.contains(&s_id) {
                continue;
            }
            let borrowed = s.borrow();
            if !borrowed.can_be_duplicate || !is_buffer(&borrowed) {
                continue;
            }
            let h = compute_hash_csharp(borrowed.data());
            drop(borrowed);
            match hash_to_struct.get(&h) {
                Some(survivor) => {
                    redirect.insert(s_id, survivor.clone());
                }
                None => {
                    hash_to_struct.insert(h, s.clone());
                }
            }
        }

        if redirect.is_empty() {
            return;
        }

        // Rewire references: any incoming pointer to a redirected struct now
        // lands on the survivor.  We mutate the cache structs in place.
        for s in &self.cache {
            let mut sm = s.borrow_mut();
            let keys: Vec<u32> = sm.references().keys().copied().collect();
            for k in keys {
                let target = sm.references().get(&k).cloned().unwrap();
                if let Some(survivor) = redirect.get(&identity(&target)) {
                    sm.set_reference(k, Some(survivor.clone()));
                }
            }
        }

        // Drop the redirected structs from the cache.
        let drop_ids: HashSet<*const RefCell<HsdStruct>> =
            redirect.keys().copied().collect();
        self.cache.retain(|s| !drop_ids.contains(&identity(s)));
    }

    fn emit(&self) -> Result<Vec<u8>> {
        const HEADER: usize = 0x20;
        let mut out: Vec<u8> = Vec::with_capacity(64 * 1024);
        out.resize(HEADER, 0);

        // Pass 1: lay out each struct, recording its offset for the reloc /
        // symbol passes.  `struct_to_offset` keys on identity.
        let mut offset_of: HashMap<*const RefCell<HsdStruct>, u32> = HashMap::new();
        for s in &self.cache {
            let borrowed = s.borrow();
            if is_buffer(&borrowed) && self.opts.buffer_align {
                align_to(&mut out, 0x20);
            } else if borrowed.align {
                align_to(&mut out, 4);
            }
            let off = (out.len() - HEADER) as u32;
            offset_of.insert(identity(s), off);
            out.extend_from_slice(borrowed.data());
        }

        align_to(&mut out, 4);
        let reloc_offset_abs = out.len();
        let reloc_offset_rel = (reloc_offset_abs - HEADER) as u32;

        // Pass 2: write reference targets into the struct payloads (in
        // place, at their stored byte offsets) and accumulate the absolute
        // pointer positions for the relocation table.
        // HSDLib drops the `key=0` slot for ref-chain non-head structs from
        // the relocation table, but still writes the value — that matches
        // the singly-linked alias chain pattern (refs[0] = next).
        let ref_chain_skip_zero: HashSet<*const RefCell<HsdStruct>> =
            ref_chain_zero_skip_set(self.dat);

        let mut reloc_positions: Vec<u32> = Vec::new();
        for s in &self.cache {
            let s_id = identity(s);
            let parent_off = *offset_of
                .get(&s_id)
                .expect("cache struct must have an offset");
            let parent_abs = parent_off as usize + HEADER;
            let borrowed = s.borrow();
            for (&inner, target) in borrowed.references() {
                let target_off = match offset_of.get(&identity(target)) {
                    Some(v) => *v,
                    None => {
                        return Err(HsdError::malformed(
                            parent_abs as u64,
                            "writer: reference target missing from cache",
                        ));
                    }
                };
                let pointer_abs = parent_abs + inner as usize;
                if pointer_abs + 4 > reloc_offset_abs {
                    return Err(HsdError::malformed(
                        pointer_abs as u64,
                        "writer: pointer slot past struct region",
                    ));
                }
                BigEndian::write_u32(
                    &mut out[pointer_abs..pointer_abs + 4],
                    target_off,
                );
                if ref_chain_skip_zero.contains(&s_id) && inner == 0 {
                    continue;
                }
                reloc_positions.push(parent_off + inner);
            }
        }

        // Pass 3: relocation table itself (sorted? HSDLib doesn't sort —
        // it walks `_structCache` in insertion order, then `s.References`
        // in dictionary order which for `BTreeMap` is offset-ascending.
        // The reader doesn't care about ordering within the table, but
        // sorting makes diffs deterministic).
        for &pos in &reloc_positions {
            let mut buf = [0u8; 4];
            BigEndian::write_u32(&mut buf, pos);
            out.extend_from_slice(&buf);
        }

        // Pass 4: root + ref symbols (data_off, str_off).  Strings are
        // packed contiguous, NUL-terminated, in roots-then-refs order.
        let symbol_section_abs = out.len();
        let total_symbols = self.dat.roots.len() + self.dat.references.len();
        out.resize(symbol_section_abs + total_symbols * 8, 0);

        // Pass 5: string pool.
        let string_pool_abs = out.len();
        let mut string_offsets: Vec<u32> = Vec::with_capacity(total_symbols);
        for r in self.dat.roots.iter().chain(self.dat.references.iter()) {
            string_offsets.push((out.len() - string_pool_abs) as u32);
            out.extend_from_slice(r.name.as_bytes());
            out.push(0);
        }

        // Fill in symbol entries now that string offsets are known.
        for (i, r) in self
            .dat
            .roots
            .iter()
            .chain(self.dat.references.iter())
            .enumerate()
        {
            let data_off = *offset_of.get(&identity(&r.data)).ok_or_else(|| {
                HsdError::malformed(0, "writer: root not found in struct cache")
            })?;
            let entry = symbol_section_abs + i * 8;
            BigEndian::write_u32(&mut out[entry..entry + 4], data_off);
            BigEndian::write_u32(
                &mut out[entry + 4..entry + 8],
                string_offsets[i],
            );
        }

        // Pass 6: header (filesize, reloc_offset_rel, reloc_count, root_count,
        // ref_count, version[4]).
        let total_len = out.len() as u32;
        BigEndian::write_u32(&mut out[0x00..0x04], total_len);
        BigEndian::write_u32(&mut out[0x04..0x08], reloc_offset_rel);
        BigEndian::write_u32(
            &mut out[0x08..0x0C],
            reloc_positions.len() as u32,
        );
        BigEndian::write_u32(&mut out[0x0C..0x10], self.dat.roots.len() as u32);
        BigEndian::write_u32(
            &mut out[0x10..0x14],
            self.dat.references.len() as u32,
        );
        out[0x14..0x18].copy_from_slice(&self.dat.version);

        Ok(out)
    }
}

// =====================================================================
// Reference-root chain handling
// =====================================================================

/// Identity of every "non-head" struct in a reference root chain.  The chain
/// is `Refs[i].data → next via inner pointer at offset 0 → next via …`.
/// The first link (= the actual ref root) keeps its key=0 reloc entry,
/// but every subsequent link's key=0 is excluded from the relocation table
/// (the singly-linked ref-chain spec).  HSDLib achieves the same outcome
/// via a "key != 0 OR not a refStruct" test in `Save`; we precompute the
/// skip set instead.
fn ref_chain_zero_skip_set(dat: &Dat) -> HashSet<*const RefCell<HsdStruct>> {
    let mut skip: HashSet<*const RefCell<HsdStruct>> = HashSet::new();
    for r in &dat.references {
        // Walk r.data refs[0] until None; mark every link as skip.
        let mut current = r.data.clone();
        loop {
            let next = current.borrow().get_reference(0);
            match next {
                Some(n) => {
                    skip.insert(identity(&n));
                    if Rc::ptr_eq(&n, &current) {
                        break; // self-ref, defensive
                    }
                    current = n;
                }
                None => break,
            }
        }
    }
    skip
}

// =====================================================================
// Helpers
// =====================================================================

fn align_to(out: &mut Vec<u8>, alignment: usize) {
    let cur = out.len();
    let pad = (alignment - (cur % alignment)) % alignment;
    if pad > 0 {
        out.extend(std::iter::repeat_n(0, pad));
    }
}

/// Same FNV-1a-with-extra-mixing hash used by `HSDRawFile.ComputeHash`.
/// We mirror the unchecked i32 wraparound semantics with `wrapping_*`.
fn compute_hash_csharp(data: &[u8]) -> i32 {
    let p: i32 = 16_777_619;
    let mut hash: i32 = 0x811C_9DC5_u32 as i32; // 2166136261 unsigned
    for &b in data {
        hash = (hash ^ (b as i32)).wrapping_mul(p);
    }
    hash = hash.wrapping_add(hash << 13);
    hash ^= hash >> 7;
    hash = hash.wrapping_add(hash << 3);
    hash ^= hash >> 17;
    hash = hash.wrapping_add(hash << 5);
    hash
}

fn walk_substructs(
    cur: &StructRef,
    seen: &mut HashSet<*const RefCell<HsdStruct>>,
    out: &mut Vec<StructRef>,
) {
    if !seen.insert(identity(cur)) {
        return;
    }
    out.push(cur.clone());
    let borrowed = cur.borrow();
    for child in borrowed.references().values() {
        walk_substructs(child, seen, out);
    }
}

#[allow(dead_code)]
fn _root_node_owns(_: &RootNode) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip the trivially-small `dat::tests::minimal_dat()` shape:
    /// after `parse → write → parse` we should still see one root.
    #[test]
    fn writer_round_trips_minimal_dat() {
        // Same hand-built minimal dat as `dat::tests::parses_minimal_dat`.
        let bytes = make_minimal_dat();
        let dat = Dat::parse(&bytes).expect("parse");
        let written = dat.write().expect("write");
        let reparsed = Dat::parse(&written).expect("reparse");
        assert_eq!(reparsed.roots.len(), 1);
        assert_eq!(reparsed.roots[0].name, "scene_data");
    }

    #[test]
    fn writer_dedups_byte_equal_buffers() {
        // Two byte-equal big buffers (>0x40, no refs => IsBuffer=true) attached
        // to a parent at different offsets should collapse to one entry in
        // the rewritten file.  We test by comparing reloc count and total
        // size: dedup means fewer reloc entries pointing at the same target.
        let buf_payload = vec![0xABu8; 0x80];

        let parent = HsdStruct::from_bytes(vec![0u8; 16]).into_ref();
        let buf_a = HsdStruct::from_bytes(buf_payload.clone()).into_ref();
        let buf_b = HsdStruct::from_bytes(buf_payload.clone()).into_ref();
        // Mark them as buffers explicitly.
        buf_a.borrow_mut().is_buffer_aligned = true;
        buf_b.borrow_mut().is_buffer_aligned = true;
        parent.borrow_mut().set_reference(0, Some(buf_a.clone()));
        parent.borrow_mut().set_reference(8, Some(buf_b.clone()));

        let dat = Dat {
            version: [0; 4],
            roots: vec![RootNode { name: "root".into(), data: parent.clone() }],
            references: vec![],
            struct_order: vec![parent, buf_a, buf_b],
        };
        let written = dat.write().expect("write");

        // After dedup we should be able to parse and end up with exactly one
        // distinct buffer struct reachable from the parent (Rc identity).
        let reparsed = Dat::parse(&written).expect("reparse");
        let parent = &reparsed.roots[0].data;
        let r0 = parent.borrow().get_reference(0).expect("ref0");
        let r8 = parent.borrow().get_reference(8).expect("ref8");
        assert!(
            Rc::ptr_eq(&r0, &r8),
            "byte-equal buffers should dedup to a single Rc"
        );
    }

    fn make_minimal_dat() -> Vec<u8> {
        let mut buf = vec![0u8; 0x48];
        BigEndian::write_u32(&mut buf[0x00..0x04], 0x48);
        BigEndian::write_u32(&mut buf[0x04..0x08], 0x10);
        BigEndian::write_u32(&mut buf[0x08..0x0C], 0x00);
        BigEndian::write_u32(&mut buf[0x0C..0x10], 0x01);
        BigEndian::write_u32(&mut buf[0x10..0x14], 0x00);
        BigEndian::write_u32(&mut buf[0x30..0x34], 0x00);
        BigEndian::write_u32(&mut buf[0x34..0x38], 0x00);
        let name = b"scene_data\0";
        buf[0x38..0x38 + name.len()].copy_from_slice(name);
        buf
    }
}
