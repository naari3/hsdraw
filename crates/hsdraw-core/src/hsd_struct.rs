//! `HsdStruct` — the moral equivalent of `HSDLib` `HSDStruct`.
//!
//! A struct is a flat byte buffer plus a map of internal byte offsets that
//! point at other structs.  References are tracked by Rc identity (the same
//! `Rc<RefCell<HsdStruct>>` placed at two locations means a real alias, not a
//! byte-equal duplicate).  This identity is what makes alias-root round-trip
//! work in the writer (see `docs/notes/phase0.md` §3).
//!
//! `BTreeMap` (not `HashMap`) so iteration order is the offset order — both
//! the writer's relocation table emission and the reader's substructure walk
//! observe a stable order, which is also what HSDLib's `Dictionary<int,...>`
//! happens to give in practice (insertion-ordered, but offsets are inserted
//! ascending during parse).

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

use byteorder::{BigEndian, ByteOrder};

use crate::error::{HsdError, Result};

pub type StructRef = Rc<RefCell<HsdStruct>>;

#[derive(Debug)]
pub struct HsdStruct {
    /// Raw bytes of this struct (NOT including any pointed-at substructures).
    data: Vec<u8>,

    /// Byte-offset → referenced struct.  An offset of `K` means the 4 bytes at
    /// `data[K..K+4]` originally held a pointer; in our model the actual ptr
    /// value is gone (overwritten with zeros on parse, never re-stored) and
    /// the resolved target is held here.
    references: BTreeMap<u32, StructRef>,

    /// `IsBufferAligned` in HSDLib — texture / DL buffers want 0x20 alignment
    /// at write time.  Default false; set when this struct was loaded as a
    /// `byte[]` payload via `SetBuffer` or auto-detected as such by the
    /// reader.
    pub is_buffer_aligned: bool,

    /// `Align` in HSDLib — when not a buffer, write-time alignment is 4 if
    /// true, none if false.  Default true.
    pub align: bool,

    /// `CanBeDuplicate` in HSDLib — buffer dedup (`RemoveDuplicateBuffers`)
    /// will consider this struct as a candidate for byte-hash-based merging.
    /// Default true.
    pub can_be_duplicate: bool,

    /// `CanBeBuffer` in HSDLib — controls whether `IsBuffer()` may return
    /// true.  Default true.
    pub can_be_buffer: bool,
}

impl HsdStruct {
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    pub fn with_capacity(size: usize) -> Self {
        Self {
            data: vec![0; size],
            references: BTreeMap::new(),
            is_buffer_aligned: false,
            align: true,
            can_be_duplicate: true,
            can_be_buffer: true,
        }
    }

    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self {
            data,
            references: BTreeMap::new(),
            is_buffer_aligned: false,
            align: true,
            can_be_duplicate: true,
            can_be_buffer: true,
        }
    }

    /// Wrap a fresh struct in `Rc<RefCell<…>>` for shared, mutable access.
    pub fn into_ref(self) -> StructRef {
        Rc::new(RefCell::new(self))
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn set_data(&mut self, data: Vec<u8>) {
        // Drop any references that fall past the new tail.  HSDLib does the
        // same in `Resize`.
        let new_len = data.len() as u32;
        self.data = data;
        self.references.retain(|&k, _| k < new_len);
    }

    pub fn resize(&mut self, new_len: usize) {
        self.data.resize(new_len, 0);
        let cap = new_len as u32;
        self.references.retain(|&k, _| k < cap);
    }

    pub fn references(&self) -> &BTreeMap<u32, StructRef> {
        &self.references
    }

    pub fn references_mut(&mut self) -> &mut BTreeMap<u32, StructRef> {
        &mut self.references
    }

    /// Set or replace a reference at `offset`; passing `None` removes it.
    pub fn set_reference(&mut self, offset: u32, target: Option<StructRef>) {
        match target {
            Some(t) => {
                self.references.insert(offset, t);
            }
            None => {
                self.references.remove(&offset);
            }
        }
    }

    pub fn get_reference(&self, offset: u32) -> Option<StructRef> {
        self.references.get(&offset).cloned()
    }

    // ------------------------------------------------------------------
    // Big-endian primitive accessors.  Out-of-bounds reads return Ok(0)
    // for `i32`/`u32`/etc. or an Err for byte ranges we can't satisfy at
    // all — chosen to match HSDLib `GetInt32`'s behavior of zero-extending
    // truncated reads while still flagging clearly-bogus offsets.
    // ------------------------------------------------------------------

    pub fn get_byte(&self, offset: u32) -> Result<u8> {
        self.data
            .get(offset as usize)
            .copied()
            .ok_or(HsdError::StructOob {
                at: offset,
                requested: 1,
                len: self.data.len() as u32,
            })
    }

    pub fn get_bytes(&self, offset: u32, len: u32) -> Result<&[u8]> {
        let start = offset as usize;
        let end = start
            .checked_add(len as usize)
            .ok_or(HsdError::StructOob {
                at: offset,
                requested: len,
                len: self.data.len() as u32,
            })?;
        self.data.get(start..end).ok_or(HsdError::StructOob {
            at: offset,
            requested: len,
            len: self.data.len() as u32,
        })
    }

    pub fn get_u16(&self, offset: u32) -> Result<u16> {
        Ok(BigEndian::read_u16(self.get_bytes(offset, 2)?))
    }

    pub fn get_i16(&self, offset: u32) -> Result<i16> {
        Ok(BigEndian::read_i16(self.get_bytes(offset, 2)?))
    }

    pub fn get_u32(&self, offset: u32) -> Result<u32> {
        Ok(BigEndian::read_u32(self.get_bytes(offset, 4)?))
    }

    pub fn get_i32(&self, offset: u32) -> Result<i32> {
        Ok(BigEndian::read_i32(self.get_bytes(offset, 4)?))
    }

    pub fn get_f32(&self, offset: u32) -> Result<f32> {
        Ok(BigEndian::read_f32(self.get_bytes(offset, 4)?))
    }

    /// Read a NUL-terminated UTF-8 string from a referenced sub-struct's data
    /// buffer.  Returns `None` if no reference is set at `offset` (matches
    /// HSDLib `GetString` returning null).
    pub fn get_string(&self, offset: u32) -> Result<Option<String>> {
        let Some(refed) = self.references.get(&offset) else {
            return Ok(None);
        };
        let borrowed = refed.borrow();
        let bytes = borrowed.data();
        let nul = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        std::str::from_utf8(&bytes[..nul])
            .map(|s| Some(s.to_owned()))
            .map_err(|source| HsdError::Utf8 {
                offset: offset as u64,
                source,
            })
    }
}

// -----------------------------------------------------------------------
// Identity helpers.  Use these wherever HSDLib uses object reference
// equality (= the alias-root mechanism).  Two structs are "the same" iff
// they share the same `Rc` allocation.
// -----------------------------------------------------------------------

/// Identity key used for visited-set tracking and writer dedup.  Stable for
/// the lifetime of the `Rc`; not `Send` (Rc is single-threaded).
pub fn identity(s: &StructRef) -> *const RefCell<HsdStruct> {
    Rc::as_ptr(s)
}

pub fn ptr_eq(a: &StructRef, b: &StructRef) -> bool {
    Rc::ptr_eq(a, b)
}

/// Walk the substructure DAG starting at `root`, returning each unique
/// reachable struct exactly once in pre-order.  Cycles are broken by
/// identity, not by content.
pub fn collect_substructs(root: &StructRef) -> Vec<StructRef> {
    let mut seen: HashSet<*const RefCell<HsdStruct>> = HashSet::new();
    let mut out = Vec::new();
    walk(root, &mut seen, &mut out);
    out
}

fn walk(
    cur: &StructRef,
    seen: &mut HashSet<*const RefCell<HsdStruct>>,
    out: &mut Vec<StructRef>,
) {
    if !seen.insert(identity(cur)) {
        return;
    }
    out.push(cur.clone());
    let borrowed = cur.borrow();
    for child in borrowed.references.values() {
        walk(child, seen, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_is_identity_not_content() {
        let a = HsdStruct::from_bytes(vec![1, 2, 3, 4]).into_ref();
        let b = HsdStruct::from_bytes(vec![1, 2, 3, 4]).into_ref();
        assert!(!ptr_eq(&a, &b), "byte-equal but distinct allocs => not alias");
        assert!(ptr_eq(&a, &a.clone()), "same Rc => alias");
    }

    #[test]
    fn cycle_walk_terminates() {
        let s = HsdStruct::from_bytes(vec![0; 4]).into_ref();
        s.borrow_mut().set_reference(0, Some(s.clone())); // self-ref
        let all = collect_substructs(&s);
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn be_reads() {
        let s = HsdStruct::from_bytes(vec![0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC]);
        assert_eq!(s.get_u32(0).unwrap(), 0x1234_5678);
        assert_eq!(s.get_u16(4).unwrap(), 0x9ABC);
    }
}
