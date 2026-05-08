//! Thin wrappers around `StructRef` that expose typed field accessors at the
//! HSDLib offsets documented in `docs/notes/phase0.md` §8.
//!
//! Every accessor is a *value type* `pub struct Foo(StructRef)` — copying it
//! is a free `Rc` clone, identity-preserving.  This is what gives us alias
//! detection: walking from two roots that share the same JObj struct will
//! return `Foo`s whose underlying `Rc` pointers compare equal via
//! [`crate::hsd_struct::ptr_eq`].

use crate::hsd_struct::{HsdStruct, StructRef};

/// Marker trait so we can write `T: Accessor` bounds.  No required methods
/// for now; promoted to a real interface (e.g. `from_struct`) when the
/// writer phase needs to instantiate accessors generically.
pub trait Accessor {
    fn from_struct(s: StructRef) -> Self;
    fn as_struct(&self) -> &StructRef;
}

macro_rules! accessor {
    ($name:ident) => {
        #[derive(Clone, Debug)]
        pub struct $name(pub StructRef);

        impl Accessor for $name {
            fn from_struct(s: StructRef) -> Self {
                Self(s)
            }
            fn as_struct(&self) -> &StructRef {
                &self.0
            }
        }

        impl $name {
            #[inline]
            fn s(&self) -> std::cell::Ref<'_, HsdStruct> {
                self.0.borrow()
            }

            /// Strong-typed reference at `offset`, wrapped in `T`.
            pub fn ref_at<T: Accessor>(&self, offset: u32) -> Option<T> {
                self.s().get_reference(offset).map(T::from_struct)
            }
        }
    };
}

pub(crate) use accessor;

/// Helper to expose an accessor's underlying struct identity for visited-set
/// tracking without forcing callers to know about `Rc::as_ptr`.
pub fn id_of<T: Accessor>(a: &T) -> *const std::cell::RefCell<HsdStruct> {
    crate::hsd_struct::identity(a.as_struct())
}

pub fn same<T: Accessor>(a: &T, b: &T) -> bool {
    crate::hsd_struct::ptr_eq(a.as_struct(), b.as_struct())
}

