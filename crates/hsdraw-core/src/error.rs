use thiserror::Error;

pub type Result<T, E = HsdError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum HsdError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Header / relocation table read past EOF or pointed at an impossible
    /// offset.  Carries enough context to find the bad byte in a hex viewer.
    /// `context` is owned so callers can format runtime values into it
    /// (e.g. `format!("alias '{}' refs unknown joint", name)` from the
    /// importer).  Use `HsdError::malformed` for static messages —
    /// constructing the variant directly works for dynamic ones.
    #[error("malformed dat at offset 0x{offset:X}: {context}")]
    Malformed {
        offset: u64,
        context: std::borrow::Cow<'static, str>,
    },

    #[error("read past struct end: requested 0x{requested:X} bytes at offset 0x{at:X}, struct length 0x{len:X}")]
    StructOob {
        at: u32,
        requested: u32,
        len: u32,
    },

    #[error("UTF-8 decode failed for symbol at offset 0x{offset:X}: {source}")]
    Utf8 {
        offset: u64,
        #[source]
        source: std::str::Utf8Error,
    },
}

impl HsdError {
    pub fn malformed(offset: u64, context: impl Into<std::borrow::Cow<'static, str>>) -> Self {
        Self::Malformed { offset, context: context.into() }
    }
}
