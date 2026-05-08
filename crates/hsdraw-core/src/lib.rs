//! hsdraw-core — pure-Rust HSD `.dat` reader/writer.
//!
//! Phase scope: Reader (header, relocation table, struct identity, JObj tree
//! walk).  GX texture decode, DL unpack, writer, alias-root round-trip are
//! built on top in later phases.  See `docs/notes/phase0.md` for the spec the
//! whole crate is built against.

pub mod accessor;
pub mod common;
pub mod dat;
pub mod error;
pub mod export;
pub mod gx;
pub mod gx_dl;
pub mod gx_image;
pub mod hsd_struct;

pub use accessor::Accessor;
pub use dat::Dat;
pub use error::{HsdError, Result};
pub use hsd_struct::{HsdStruct, StructRef};
