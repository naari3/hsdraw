//! GX enums and bitflags mirroring `HSDRaw/GX/Enums.cs`.
//!
//! `bitflags!` is intentionally dependency-free: hand-rolled flag types with
//! `from_bits_retain` semantics so unknown bits round-trip unchanged.

#![allow(non_camel_case_types)]

// =====================================================================
// JOBJ_FLAG (HSDRaw/Common/HSD_JOBJ.cs:9)
// =====================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct JObjFlag(u32);

impl JObjFlag {
    pub const SKELETON: Self = Self(1 << 0);
    pub const SKELETON_ROOT: Self = Self(1 << 1);
    pub const ENVELOPE_MODEL: Self = Self(1 << 2);
    pub const CLASSICAL_SCALING: Self = Self(1 << 3);
    pub const HIDDEN: Self = Self(1 << 4);
    pub const PTCL: Self = Self(1 << 5);
    pub const MTX_DIRTY: Self = Self(1 << 6);
    pub const LIGHTING: Self = Self(1 << 7);
    pub const TEXGEN: Self = Self(1 << 8);
    pub const BILLBOARD: Self = Self(1 << 9);
    pub const VBILLBOARD: Self = Self(2 << 9);
    pub const HBILLBOARD: Self = Self(3 << 9);
    pub const RBILLBOARD: Self = Self(4 << 9);
    pub const INSTANCE: Self = Self(1 << 12);
    pub const PBILLBOARD: Self = Self(1 << 13);
    pub const SPLINE: Self = Self(1 << 14);
    pub const FLIP_IK: Self = Self(1 << 15);
    pub const SPECULAR: Self = Self(1 << 16);
    pub const USE_QUATERNION: Self = Self(1 << 17);
    pub const OPA: Self = Self(1 << 18);
    pub const XLU: Self = Self(1 << 19);
    pub const TEXEDGE: Self = Self(1 << 20);
    pub const JOINT1: Self = Self(1 << 21);
    pub const JOINT2: Self = Self(2 << 21);
    pub const EFFECTOR: Self = Self(3 << 21);
    pub const USER_DEFINED_MTX: Self = Self(1 << 23);
    pub const MTX_INDEPEND_PARENT: Self = Self(1 << 24);
    pub const MTX_INDEPEND_SRT: Self = Self(1 << 25);
    pub const ROOT_OPA: Self = Self(1 << 28);
    pub const ROOT_XLU: Self = Self(1 << 29);
    pub const ROOT_TEXEDGE: Self = Self(1 << 30);

    pub fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    pub fn bits(self) -> u32 {
        self.0
    }

    pub fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl std::ops::BitOr for JObjFlag {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

// =====================================================================
// PObjFlag  (HSDRaw/Common/HSD_POBJ.cs:9)
// =====================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct PObjFlag(u16);

impl PObjFlag {
    pub const SKIN: Self = Self(0 << 12);
    pub const SHAPEANIM: Self = Self(1 << 12);
    pub const ENVELOPE: Self = Self(2 << 12);
    pub const SHAPESET: Self = Self(1 << 9);
    /// HSDLib calls this `CULLBACK` (= `1 << 14`), but the bit lands
    /// inside `POBJ_TYPE_MASK` (0xE000) without matching any valid
    /// POBJ type, so renderers dispatching on the type nibble treat
    /// it as an unknown POBJ.  Cull mode belongs on `PeDesc`, not
    /// POBJ.flags — kept here for read-side compatibility with legacy
    /// .dat files only.
    #[deprecated(
        note = "POBJ.flags 0x4000 trap — cull mode belongs on PeDesc, not POBJ.flags."
    )]
    pub const CULLBACK: Self = Self(1 << 14);
    /// HSDLib calls this `CULLFRONT` (= `1 << 15`).  Same trap as
    /// `CULLBACK` — collides with `POBJ_FLAG.ENVELOPE` in HSDLib's
    /// enum encoding.  Use `PeDesc` for cull mode.
    #[deprecated(
        note = "POBJ.flags 0x8000 trap — collides with POBJ_FLAG.ENVELOPE."
    )]
    pub const CULLFRONT: Self = Self(1 << 15);

    pub fn from_bits_retain(bits: u16) -> Self {
        Self(bits)
    }

    pub fn bits(self) -> u16 {
        self.0
    }

    pub fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl std::ops::BitOr for PObjFlag {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

// =====================================================================
// MaterialRenderMode (HSDRaw/Common/HSD_MOBJ.cs:8 RENDER_MODE).
// Bits match HSDLib exactly so `render_flags_raw` round-trips and the
// `render_flag_names` table emits the same `, `-joined string the csx
// uses for parity testing.
// =====================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct MaterialRenderMode(u32);

impl MaterialRenderMode {
    pub const CONSTANT: Self = Self(1 << 0);
    pub const VERTEX: Self = Self(1 << 1);
    pub const BOTH: Self = Self((1 << 0) | (1 << 1));
    pub const DIFFUSE: Self = Self(1 << 2);
    pub const SPECULAR: Self = Self(1 << 3);
    pub const TEX0: Self = Self(1 << 4);
    pub const TEX1: Self = Self(1 << 5);
    pub const TEX2: Self = Self(1 << 6);
    pub const TEX3: Self = Self(1 << 7);
    pub const TEX4: Self = Self(1 << 8);
    pub const TEX5: Self = Self(1 << 9);
    pub const TEX6: Self = Self(1 << 10);
    pub const TEX7: Self = Self(1 << 11);
    pub const TOON: Self = Self(1 << 12);
    pub const ALPHA_MAT: Self = Self(1 << 13);
    pub const ALPHA_VTX: Self = Self(2 << 13);
    pub const ALPHA_BOTH: Self = Self(3 << 13);
    pub const ZOFST: Self = Self(1 << 24);
    pub const EFFECT: Self = Self(1 << 25);
    pub const SHADOW: Self = Self(1 << 26);
    pub const ZMODE_ALWAYS: Self = Self(1 << 27);
    pub const DF_ALL: Self = Self(1 << 28);
    pub const NO_ZUPDATE: Self = Self(1 << 29);
    pub const XLU: Self = Self(1 << 30);
    pub const USER: Self = Self(1 << 31);

    pub fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub fn bits(self) -> u32 {
        self.0
    }
    pub fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }
}

impl std::ops::BitOr for MaterialRenderMode {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

// =====================================================================
// TOBJ_FLAGS (HSDRaw/Common/HSD_TOBJ.cs:10)
// =====================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct TObjFlags(u32);

impl TObjFlags {
    pub const LIGHTMAP_DIFFUSE: Self = Self(1 << 4);
    pub const LIGHTMAP_SPECULAR: Self = Self(1 << 5);
    pub const LIGHTMAP_AMBIENT: Self = Self(1 << 6);
    pub const LIGHTMAP_EXT: Self = Self(1 << 7);
    pub const LIGHTMAP_SHADOW: Self = Self(1 << 8);
    pub const BUMP: Self = Self(1 << 24);
    pub const MTX_DIRTY: Self = Self(1 << 31);

    pub fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub fn bits(self) -> u32 {
        self.0
    }
    pub fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }
}

// =====================================================================
// Plain enums.  All have `From<u32>` returning `Unknown(_)` for unrecognized
// values to keep round-trip lossless on weird inputs.
// =====================================================================

macro_rules! plain_enum {
    ($name:ident, $repr:ty, { $($variant:ident = $value:expr),* $(,)? }) => {
        #[derive(Copy, Clone, Debug, PartialEq, Eq)]
        pub enum $name {
            $($variant,)*
            Unknown($repr),
        }

        impl From<$repr> for $name {
            fn from(value: $repr) -> Self {
                match value {
                    $(v if v == $value => $name::$variant,)*
                    other => $name::Unknown(other),
                }
            }
        }

        impl From<$name> for $repr {
            fn from(value: $name) -> Self {
                match value {
                    $($name::$variant => $value,)*
                    $name::Unknown(v) => v,
                }
            }
        }

        impl $name {
            pub fn name(self) -> std::borrow::Cow<'static, str> {
                match self {
                    $($name::$variant => std::borrow::Cow::Borrowed(stringify!($variant)),)*
                    $name::Unknown(v) => std::borrow::Cow::Owned(format!("Unknown({})", v)),
                }
            }
        }
    };
}

plain_enum!(GxTexFmt, u32, {
    I4 = 0,
    I8 = 1,
    IA4 = 2,
    IA8 = 3,
    RGB565 = 4,
    RGB5A3 = 5,
    RGBA8 = 6,
    CI4 = 8,
    CI8 = 9,
    CI14X2 = 10,
    CMP = 14,
});

plain_enum!(GxTlutFmt, u32, {
    IA8 = 0,
    RGB565 = 1,
    RGB5A3 = 2,
});

plain_enum!(GxTexMapId, u32, {
    GX_TEXMAP0 = 0,
    GX_TEXMAP1 = 1,
    GX_TEXMAP2 = 2,
    GX_TEXMAP3 = 3,
    GX_TEXMAP4 = 4,
    GX_TEXMAP5 = 5,
    GX_TEXMAP6 = 6,
    GX_TEXMAP7 = 7,
    GX_MAX_TEXMAP = 8,
    GX_TEXMAP_NULL = 9,
    GX_TEXMAP_DISABLE = 10,
});

plain_enum!(GxWrapMode, u32, {
    CLAMP = 0,
    REPEAT = 1,
    MIRROR = 2,
});

plain_enum!(GxTexFilter, u32, {
    GX_NEAR = 0,
    GX_LINEAR = 1,
    GX_NEAR_MIP_NEAR = 2,
    GX_LIN_MIP_NEAR = 3,
    GX_NEAR_MIP_LIN = 4,
    GX_LIN_MIP_LIN = 5,
});

plain_enum!(GxAnisotropy, u32, {
    GX_ANISO_1 = 0,
    GX_ANISO_2 = 1,
    GX_ANISO_4 = 2,
    GX_MAX_ANISOTROPY = 3,
});

plain_enum!(CoordType, u32, {
    UV = 0,
    REFLECTION = 1,
    HILIGHT = 2,
    SHADOW = 3,
    TOON = 4,
    GRADATION = 5,
});

plain_enum!(ColorMap, u32, {
    NONE = 0,
    ALPHA_MASK = 1,
    RGB_MASK = 2,
    BLEND = 3,
    MODULATE = 4,
    REPLACE = 5,
    PASS = 6,
    ADD = 7,
    SUB = 8,
});

plain_enum!(AlphaMap, u32, {
    NONE = 0,
    ALPHA_MASK = 1,
    BLEND = 2,
    MODULATE = 3,
    REPLACE = 4,
    PASS = 5,
    ADD = 6,
    SUB = 7,
});

plain_enum!(GxPrimitiveType, u8, {
    Quads = 0x80,
    Triangles = 0x90,
    TriangleStrip = 0x98,
    TriangleFan = 0xA0,
    Lines = 0xA8,
    LineStrip = 0xB0,
    Points = 0xB8,
});

plain_enum!(GxTexGenSrc, u32, {
    GX_TG_POS = 0,
    GX_TG_NRM = 1,
    GX_TG_BINRM = 2,
    GX_TG_TANGENT = 3,
    GX_TG_TEX0 = 4,
    GX_TG_TEX1 = 5,
    GX_TG_TEX2 = 6,
    GX_TG_TEX3 = 7,
    GX_TG_TEX4 = 8,
    GX_TG_TEX5 = 9,
    GX_TG_TEX6 = 10,
    GX_TG_TEX7 = 11,
    GX_TG_TEXCOORD0 = 12,
    GX_TG_TEXCOORD1 = 13,
    GX_TG_TEXCOORD2 = 14,
    GX_TG_TEXCOORD3 = 15,
    GX_TG_TEXCOORD4 = 16,
    GX_TG_TEXCOORD5 = 17,
    GX_TG_TEXCOORD6 = 18,
    GX_TG_COLOR0 = 19,
    GX_TG_COLOR1 = 20,
});

plain_enum!(GxAttribName, u32, {
    GX_VA_PNMTXIDX = 0,
    GX_VA_TEX0MTXIDX = 1,
    GX_VA_TEX1MTXIDX = 2,
    GX_VA_TEX2MTXIDX = 3,
    GX_VA_TEX3MTXIDX = 4,
    GX_VA_TEX4MTXIDX = 5,
    GX_VA_TEX5MTXIDX = 6,
    GX_VA_TEX6MTXIDX = 7,
    GX_VA_TEX7MTXIDX = 8,
    GX_VA_POS = 9,
    GX_VA_NRM = 10,
    GX_VA_CLR0 = 11,
    GX_VA_CLR1 = 12,
    GX_VA_TEX0 = 13,
    GX_VA_TEX1 = 14,
    GX_VA_TEX2 = 15,
    GX_VA_TEX3 = 16,
    GX_VA_TEX4 = 17,
    GX_VA_TEX5 = 18,
    GX_VA_TEX6 = 19,
    GX_VA_TEX7 = 20,
    GX_POS_MTX_ARRAY = 21,
    GX_NRM_MTX_ARRAY = 22,
    GX_TEX_MTX_ARRAY = 23,
    GX_LIGHT_ARRAY = 24,
    GX_VA_NBT = 25,
    GX_VA_MAX_ATTR = 26,
    GX_VA_NULL = 0xFF,
});

plain_enum!(GxAttribType, u32, {
    GX_NONE = 0,
    GX_DIRECT = 1,
    GX_INDEX8 = 2,
    GX_INDEX16 = 3,
});

plain_enum!(GxCompType, u32, {
    UInt8 = 0,
    Int8 = 1,
    UInt16 = 2,
    Int16 = 3,
    Float = 4,
});

/// `MaterialRenderMode` flag-name list for stringification (matches the
/// HSDLib enum variant names that the csx export emits via `.ToString()`).
/// The canonical bit values come straight from `RENDER_MODE` in
/// `HSD_MOBJ.cs`.  The only colliding bit is 13 (TEX1 vs ALPHA_MAT) and 14
/// (TEX2 vs ALPHA_VTX) — those positions actually serve double duty in the
/// HSDLib enum, and `.ToString()` joins multiple matching names.  We mirror
/// that behavior so parity tests stay happy.
pub fn render_flag_names(flags: MaterialRenderMode) -> Vec<&'static str> {
    let bits = flags.bits();
    // Sorted descending for the greedy decomposition that C#
    // `[Flags]` ToString runs.  Composites (BOTH = CONSTANT | VERTEX,
    // ALPHA_BOTH = ALPHA_MAT | ALPHA_VTX) come before their constituent
    // bits so a fully-set sub-field collapses to the composite name.
    let table: &[(u32, &str)] = &[
        (1 << 31, "USER"),
        (1 << 30, "XLU"),
        (1 << 29, "NO_ZUPDATE"),
        (1 << 28, "DF_ALL"),
        (1 << 27, "ZMODE_ALWAYS"),
        (1 << 26, "SHADOW"),
        (1 << 25, "EFFECT"),
        (1 << 24, "ZOFST"),
        (3 << 13, "ALPHA_BOTH"),
        (2 << 13, "ALPHA_VTX"),
        (1 << 13, "ALPHA_MAT"),
        (1 << 12, "TOON"),
        (1 << 11, "TEX7"),
        (1 << 10, "TEX6"),
        (1 << 9, "TEX5"),
        (1 << 8, "TEX4"),
        (1 << 7, "TEX3"),
        (1 << 6, "TEX2"),
        (1 << 5, "TEX1"),
        (1 << 4, "TEX0"),
        (1 << 3, "SPECULAR"),
        (1 << 2, "DIFFUSE"),
        (3, "BOTH"), // CONSTANT | VERTEX
        (1 << 1, "VERTEX"),
        (1 << 0, "CONSTANT"),
    ];
    // RENDER_MODE has no named-zero member, so a 0-valued flag set yields
    // the empty list — csx's `Split(", ").Where(!IsNullOrWhiteSpace)`
    // collapses the bare `"0"` from C#'s ToString to the same empty list.
    flag_names_for(bits, table, None)
}

/// `JObjFlag` flag-name list, formatted to match `Enum.ToString()` on the
/// C# side.  See `flag_names_for` for the algorithm; .NET emits the named
/// 0 value when the integer is exactly 0 (here: `"NULL"`), uses greedy
/// descending decomposition for composite multi-bit fields like JOINT1/2/
/// EFFECTOR and BILLBOARD/V/H/R, and finally outputs the matched names in
/// ascending numeric order.
pub fn jobj_flag_names(flags: JObjFlag) -> Vec<&'static str> {
    let bits = flags.bits();
    // (mask, name), sorted DESCENDING for greedy-match.  Composites
    // (EFFECTOR=3<<21, HBILLBOARD=3<<9, etc.) come before the single-bit
    // entries that they would otherwise mask.
    let table: &[(u32, &str)] = &[
        (1 << 30, "ROOT_TEXEDGE"),
        (1 << 29, "ROOT_XLU"),
        (1 << 28, "ROOT_OPA"),
        (1 << 25, "MTX_INDEPEND_SRT"),
        (1 << 24, "MTX_INDEPEND_PARENT"),
        (1 << 23, "USER_DEFINED_MTX"),
        (3 << 21, "EFFECTOR"),
        (2 << 21, "JOINT2"),
        (1 << 21, "JOINT1"),
        (1 << 20, "TEXEDGE"),
        (1 << 19, "XLU"),
        (1 << 18, "OPA"),
        (1 << 17, "USE_QUATERNION"),
        (1 << 16, "SPECULAR"),
        (1 << 15, "FLIP_IK"),
        (1 << 14, "SPLINE"),
        (1 << 13, "PBILLBOARD"),
        (1 << 12, "INSTANCE"),
        (4 << 9, "RBILLBOARD"),
        (3 << 9, "HBILLBOARD"),
        (2 << 9, "VBILLBOARD"),
        (1 << 9, "BILLBOARD"),
        (1 << 8, "TEXGEN"),
        (1 << 7, "LIGHTING"),
        (1 << 6, "MTX_DIRTY"),
        (1 << 5, "PTCL"),
        (1 << 4, "HIDDEN"),
        (1 << 3, "CLASSICAL_SCALING"),
        (1 << 2, "ENVELOPE_MODEL"),
        (1 << 1, "SKELETON_ROOT"),
        (1 << 0, "SKELETON"),
    ];
    flag_names_for(bits, table, Some("NULL"))
}

/// Greedy-match against `table` (entries sorted DESCENDING), then reverse
/// the result so the output is in ascending numeric value — exactly the
/// order C# `[Flags]` `Enum.ToString()` produces.  When `bits == 0` and
/// `zero_name` is `Some(n)`, returns `vec![n]`.
pub fn flag_names_for(
    bits: u32,
    table: &[(u32, &'static str)],
    zero_name: Option<&'static str>,
) -> Vec<&'static str> {
    if bits == 0 {
        return match zero_name {
            Some(n) => vec![n],
            None => Vec::new(),
        };
    }
    let mut remaining = bits;
    let mut names: Vec<&'static str> = Vec::new();
    for (mask, name) in table {
        if (remaining & mask) == *mask {
            names.push(*name);
            remaining &= !mask;
            if remaining == 0 {
                break;
            }
        }
    }
    names.reverse();
    names
}
