//! `.dat` archive: header, relocation table, public symbol resolution.
//!
//! Mirrors `HSDLib/HSDRaw/HSDRawFile.cs::Open`.  See `docs/notes/phase0.md` §1
//! for the precise byte layout and the six pitfalls we have to mirror.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use byteorder::{BigEndian, ByteOrder};

use crate::error::{HsdError, Result};
use crate::hsd_struct::{HsdStruct, StructRef, identity};

/// File header lives in the first 0x20 bytes; struct data follows.
const HEADER_SIZE: u32 = 0x20;

#[derive(Debug)]
pub struct Dat {
    pub version: [u8; 4],
    pub roots: Vec<RootNode>,
    pub references: Vec<RootNode>,

    /// Insertion order of every struct seen during parse.  HSDLib keeps this
    /// to preserve a deterministic write order; we'll need it the same way in
    /// Phase 5.  Order is "ascending file offset" because parse appends as it
    /// walks the sorted offset list.
    pub struct_order: Vec<StructRef>,
}

#[derive(Debug, Clone)]
pub struct RootNode {
    pub name: String,
    pub data: StructRef,
}

impl Dat {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Parser::new(bytes).parse()
    }

    pub fn root(&self, name: &str) -> Option<&RootNode> {
        self.roots.iter().find(|r| r.name == name)
    }

    /// Convenience for the Blender pipeline: the `scene_data` root if any
    /// (every MKGP2 course .dat has it).  The accessor layer turns this into
    /// an `Sobj`.
    pub fn scene_data(&self) -> Option<&RootNode> {
        self.root("scene_data")
    }
}

// =====================================================================
// Parser
// =====================================================================

struct Parser<'a> {
    bytes: &'a [u8],
}

impl<'a> Parser<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn read_u32(&self, abs: u64) -> Result<u32> {
        let start = abs as usize;
        let end = start.checked_add(4).ok_or_else(|| {
            HsdError::malformed(abs, "u32 read overflow")
        })?;
        if end > self.bytes.len() {
            return Err(HsdError::malformed(abs, "u32 read past EOF"));
        }
        Ok(BigEndian::read_u32(&self.bytes[start..end]))
    }

    fn read_i32(&self, abs: u64) -> Result<i32> {
        Ok(self.read_u32(abs)? as i32)
    }

    fn read_string_nul(&self, abs: u64) -> Result<String> {
        let start = abs as usize;
        if start >= self.bytes.len() {
            return Err(HsdError::malformed(abs, "string read past EOF"));
        }
        let tail = &self.bytes[start..];
        let nul = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
        std::str::from_utf8(&tail[..nul])
            .map(|s| s.to_owned())
            .map_err(|source| HsdError::Utf8 {
                offset: abs,
                source,
            })
    }

    fn parse(&self) -> Result<Dat> {
        if self.bytes.len() < HEADER_SIZE as usize {
            return Err(HsdError::malformed(0, "file shorter than header"));
        }

        // ---------- header ----------
        let fsize = self.read_u32(0x00)?;
        let reloc_offset_rel = self.read_u32(0x04)?;
        let reloc_count = self.read_u32(0x08)?;
        let root_count = self.read_u32(0x0C)?;
        let ref_count = self.read_u32(0x10)?;

        if fsize as usize != self.bytes.len() {
            // Not necessarily fatal — HSDLib trusts the file size.  We log
            // via a malformed *only* when it's flat-out impossible (would
            // require us to read past EOF later); a discrepancy by itself
            // we tolerate, matching HSDLib's lax behavior.
        }

        let reloc_offset = reloc_offset_rel
            .checked_add(HEADER_SIZE)
            .ok_or_else(|| HsdError::malformed(0x04, "reloc_offset overflow"))?;

        let mut version = [0u8; 4];
        version.copy_from_slice(&self.bytes[0x14..0x18]);

        // ---------- relocation table ----------
        // For each entry, the slot value is the *position* of a pointer in
        // struct data; reading at that position gives us the *target* offset.
        // Both are stored relative; we add 0x20 to land in absolute file
        // space.
        let mut offsets: BTreeSet<u32> = BTreeSet::new();
        let mut relocs: HashMap<u32, u32> = HashMap::new(); // pos → target
        offsets.insert(reloc_offset);

        for i in 0..reloc_count {
            let entry_pos = reloc_offset + 4 * i;
            let pointer_pos = self.read_u32(entry_pos as u64)? + HEADER_SIZE;

            // HSDLib reads `objectOff` straight from `pointer_pos` even when
            // it lands past EOF (the C# FileStream zero-fills past-EOF reads
            // — well, it raises, but in HSDLib's call path that bubbles up).
            // We're more permissive: we silently drop reloc entries whose
            // pointer doesn't have 4 bytes of room.  The cost is ignoring a
            // truly bogus reloc; the gain is we keep parsing a partially-
            // garbage file far enough to be useful.  None of the vanilla
            // MKGP2 corpus hits this branch.
            let target_rel = match self.read_i32(pointer_pos as u64) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // HSDLib special-case: pointer points beyond reloc table → assume
            // file was manually relocated; treat fsize as a struct-end
            // sentinel so the trailing block gets carved out.
            if target_rel as u32 > reloc_offset {
                offsets.insert(fsize);
            }

            // Negative target = "alternate null"; HSDLib drops these silently.
            if target_rel < 0 {
                continue;
            }

            let target = (target_rel as u32).wrapping_add(HEADER_SIZE);
            relocs.insert(pointer_pos, target);
            offsets.insert(target);
        }

        // ---------- root + reference symbol tables ----------
        let symbol_table_pos = reloc_offset + reloc_count * 4;
        let string_pool_pos =
            symbol_table_pos + (root_count + ref_count) * 8;

        let mut root_offsets: Vec<u32> = Vec::with_capacity(root_count as usize);
        let mut root_names: Vec<String> = Vec::with_capacity(root_count as usize);
        let mut ref_offsets: Vec<u32> = Vec::with_capacity(ref_count as usize);
        let mut ref_names: Vec<String> = Vec::with_capacity(ref_count as usize);

        for i in 0..root_count {
            let entry = symbol_table_pos + i * 8;
            let data_off = self.read_u32(entry as u64)? + HEADER_SIZE;
            let str_off = self.read_u32((entry + 4) as u64)?;
            root_offsets.push(data_off);
            root_names.push(
                self.read_string_nul((string_pool_pos + str_off) as u64)?,
            );
        }

        for i in 0..ref_count {
            let entry = symbol_table_pos + (root_count + i) * 8;
            let data_off = self.read_u32(entry as u64)? + HEADER_SIZE;
            let str_off = self.read_u32((entry + 4) as u64)?;
            ref_offsets.push(data_off);
            ref_names.push(
                self.read_string_nul((string_pool_pos + str_off) as u64)?,
            );

            // Reference roots own a singly-linked chain of struct fragments
            // (HSDRawFile.cs:192-216).  Walk it: at each step the *first
            // word* of the current struct holds the next struct's relative
            // offset; 0 / -1 terminate.  Inject every chain step into both
            // the offsets set and the relocation map so the struct-cutting
            // pass below treats the chain like any other reference.
            let mut current = data_off;
            loop {
                let next_rel = self.read_i32(current as u64)?;
                if next_rel == 0 || next_rel == -1 {
                    break;
                }
                let next = (next_rel as u32).wrapping_add(HEADER_SIZE);
                relocs.insert(current, next);
                offsets.insert(next);
                current = next;
            }
        }

        for &v in &root_offsets {
            offsets.insert(v);
        }
        for &v in &ref_offsets {
            offsets.insert(v);
        }

        // ---------- carve struct buffers between consecutive offsets ----------
        let sorted_offsets: Vec<u32> = offsets.into_iter().collect();
        let mut offset_to_struct: HashMap<u32, StructRef> = HashMap::new();
        // (parent_offset → list of (inner_pointer_pos, target_offset)) so the
        // subsequent reference-wiring pass can iterate by parent.
        let mut offset_to_outgoing: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        let mut struct_order: Vec<StructRef> = Vec::with_capacity(sorted_offsets.len());

        if sorted_offsets.is_empty() {
            return Err(HsdError::malformed(0, "no offsets resolved"));
        }

        // Pre-sort the relocation source positions for the per-struct slice.
        let mut reloc_positions: Vec<u32> = relocs.keys().copied().collect();
        reloc_positions.sort_unstable();

        for window in sorted_offsets.windows(2) {
            let start = window[0];
            let end = window[1];
            // The terminator (typically `relocOffset` or `fsize`) shows up as
            // the last sorted offset; we don't slice into it.
            if start as usize >= self.bytes.len() {
                continue;
            }
            let cap = (end as usize).min(self.bytes.len());
            if cap <= start as usize {
                continue;
            }
            let slice = &self.bytes[start as usize..cap];

            let s = HsdStruct::from_bytes(slice.to_vec()).into_ref();
            offset_to_struct.insert(start, s.clone());
            struct_order.push(s);

            // Collect the relocation entries that originate inside this
            // struct.  Linear span lookup in the sorted list — the windows
            // are small in practice and we don't need a binary search yet.
            let mut outgoing = Vec::new();
            for &pos in &reloc_positions {
                if pos < start {
                    continue;
                }
                if pos >= end {
                    break;
                }
                if let Some(&target) = relocs.get(&pos) {
                    outgoing.push((pos, target));
                }
            }
            offset_to_outgoing.insert(start, outgoing);
        }

        // ---------- wire references ----------
        for (parent_off, parent) in &offset_to_struct {
            let outgoing = offset_to_outgoing.remove(parent_off).unwrap_or_default();
            let mut parent_mut = parent.borrow_mut();
            let parent_len = parent_mut.len() as u32;
            for (inner_pos, target_off) in outgoing {
                let inner_off = inner_pos.wrapping_sub(*parent_off);
                if inner_off + 4 > parent_len {
                    continue;
                }
                if let Some(target) = offset_to_struct.get(&target_off) {
                    parent_mut.set_reference(inner_off, Some(target.clone()));
                }
            }
        }

        // ---------- assemble Roots / References ----------
        let mut roots = Vec::with_capacity(root_count as usize);
        for (off, name) in root_offsets.iter().zip(root_names.into_iter()) {
            let s = offset_to_struct.get(off).cloned().ok_or_else(|| {
                HsdError::malformed(*off as u64, "root struct offset missing")
            })?;
            roots.push(RootNode { name, data: s });
        }

        let mut references = Vec::with_capacity(ref_count as usize);
        for (off, name) in ref_offsets.iter().zip(ref_names.into_iter()) {
            let s = offset_to_struct.get(off).cloned().ok_or_else(|| {
                HsdError::malformed(*off as u64, "ref struct offset missing")
            })?;
            references.push(RootNode { name, data: s });
        }

        // Mark structs that look like buffers (no outgoing references and a
        // bigger payload) — this is what HSDLib's `IsBuffer()` check looks
        // for at write time, and we need the flag set when the reader
        // already knows.  Also catches HSD_Image.ImageData payloads — they
        // were inserted as referenced sub-structs from a parent at offset 0,
        // so they show up here too.
        for s in &struct_order {
            let mut sm = s.borrow_mut();
            if sm.references().is_empty() && sm.len() > 0x40 {
                sm.is_buffer_aligned = true;
            }
        }

        // Detect orphans (= structs not reachable from any root/reference).
        // HSDLib in Release silently drops them.  We do the same in Phase 1;
        // if we ever want to surface them for debugging, a `keep_orphans`
        // flag can revive HSDLib's `Orphan0xXXXX` synthetic root behavior.
        let mut reachable: HashSet<*const RefCell<HsdStruct>> = HashSet::new();
        for r in roots.iter().chain(references.iter()) {
            mark_reachable(&r.data, &mut reachable);
        }
        let struct_order: Vec<StructRef> = struct_order
            .into_iter()
            .filter(|s| reachable.contains(&identity(s)))
            .collect();

        Ok(Dat {
            version,
            roots,
            references,
            struct_order,
        })
    }
}

fn mark_reachable(s: &StructRef, set: &mut HashSet<*const RefCell<HsdStruct>>) {
    if !set.insert(identity(s)) {
        return;
    }
    let borrowed = s.borrow();
    for child in borrowed.references().values() {
        mark_reachable(child, set);
    }
}

#[allow(dead_code)]
fn _identity(s: &StructRef) -> *const RefCell<HsdStruct> {
    Rc::as_ptr(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-built minimal .dat: header + one struct + one root.  Crafted so
    /// the parser exercises every header field but stays under 0x100 bytes.
    fn minimal_dat() -> Vec<u8> {
        // Layout:
        //   0x00..0x20  header
        //   0x20..0x30  one struct (16 bytes, all zero)
        //   0x30..0x34  reloc table (0 entries, just the terminator anchor)
        //   0x34..0x3C  root entry (data_rel=0x00, str_rel=0x00)
        //   0x3C..0x47  string pool: "scene_data\0"
        //   pad to 0x48
        let mut buf = vec![0u8; 0x48];

        // header
        BigEndian::write_u32(&mut buf[0x00..0x04], 0x48); // fsize
        BigEndian::write_u32(&mut buf[0x04..0x08], 0x10); // reloc_offset_rel (= 0x30 abs)
        BigEndian::write_u32(&mut buf[0x08..0x0C], 0x00); // reloc_count
        BigEndian::write_u32(&mut buf[0x0C..0x10], 0x01); // root_count
        BigEndian::write_u32(&mut buf[0x10..0x14], 0x00); // ref_count
        // version chars at 0x14..0x18 left as zeros

        // one struct at 0x20..0x30 left as zeros

        // root entry at 0x30..0x38  (= reloc_offset (no entries) + 0)
        BigEndian::write_u32(&mut buf[0x30..0x34], 0x00); // data_rel = 0  → abs 0x20
        BigEndian::write_u32(&mut buf[0x34..0x38], 0x00); // str_rel = 0

        // string pool at 0x38
        let name = b"scene_data\0";
        buf[0x38..0x38 + name.len()].copy_from_slice(name);

        buf
    }

    #[test]
    fn parses_minimal_dat() {
        let bytes = minimal_dat();
        let dat = Dat::parse(&bytes).expect("parse");
        assert_eq!(dat.roots.len(), 1);
        assert_eq!(dat.roots[0].name, "scene_data");
        assert_eq!(dat.roots[0].data.borrow().len(), 0x10);
    }
}
