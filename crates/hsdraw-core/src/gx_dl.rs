//! GX vertex attribute table + display list bytecode.
//!
//! Mirrors `HSDRaw/GX/GX_Attribute.cs` + `GX_DisplayList.cs` +
//! `GX_PrimitiveGroup.cs` + `Tools/GX_VertexAccessor.cs`.  The output of
//! [`unpack`] is a vertex list per primitive group, fully resolved (POS / NRM
//! / TEX0 / CLR0 / matrix indices) — the same shape `GX_VertexAccessor.
//! GetDecodedVertices` returns on the C# side.

use crate::common::PObj;
use crate::error::{HsdError, Result};
use crate::gx::{GxAttribName, GxAttribType, GxCompType, GxPrimitiveType};
use crate::hsd_struct::StructRef;

/// One row of the GX vertex attribute table (HSDLib `GX_Attribute`,
/// TrimmedSize 0x18).  The buffer reference holds the actual per-vertex data
/// indexed by display-list entries.
#[derive(Debug, Clone)]
pub struct GxAttribute {
    pub name: GxAttribName,
    pub kind: GxAttribType,
    pub comp_count: u32,
    pub comp_type: u32, // raw value; clr semantics differ from POS/NRM/TEX
    pub scale: u8,
    pub stride: u16,
    pub buffer: Option<StructRef>,
}

// `GxAttribute` is read in-place from an embedded array via
// `read_attribute_at`, not from a freestanding `StructRef`, so a
// from_struct constructor would just be dead code.

/// Decoded vertex shape — one entry per DL index.  Every channel is filled
/// with sensible zeros when the attribute isn't present, so consumers can
/// always read `.pos` / `.uv0` etc. without checking.
#[derive(Debug, Clone, Default)]
pub struct GxVertex {
    pub pos: [f32; 3],
    pub nrm: [f32; 3],
    pub clr0: [f32; 4],
    pub clr1: [f32; 4],
    pub tex0: [f32; 2],
    pub tex1: [f32; 2],
    pub tex2: [f32; 2],
    pub tex3: [f32; 2],
    pub tex4: [f32; 2],
    pub tex5: [f32; 2],
    pub tex6: [f32; 2],
    pub tex7: [f32; 2],
    pub pn_mtx_idx: u16,
    pub tex0_mtx_idx: u16,
    pub tex1_mtx_idx: u16,
    pub tex2_mtx_idx: u16,
}

#[derive(Debug, Clone)]
pub struct GxPrimitiveGroup {
    pub primitive_type: GxPrimitiveType,
    /// Vertices in DL order — one per index group emitted by the DL.
    pub vertices: Vec<GxVertex>,
}

#[derive(Debug, Clone, Default)]
pub struct GxDisplayList {
    pub attributes: Vec<GxAttribute>,
    pub primitives: Vec<GxPrimitiveGroup>,
}

impl GxDisplayList {
    /// Total count of decoded vertices across all primitive groups (matches
    /// `csx dl.Vertices.Count`).
    pub fn total_vertices(&self) -> usize {
        self.primitives.iter().map(|p| p.vertices.len()).sum()
    }

    /// Convenience for callers that want HSDLib-style "is attribute X present
    /// at all" checks.
    pub fn has_attribute(&self, name: GxAttribName) -> bool {
        self.attributes
            .iter()
            .any(|a| a.name == name && a.kind != GxAttribType::GX_NONE)
    }
}

// =====================================================================
// Public entry point
// =====================================================================

/// Unpack a PObj's vertex attribute table + display list into per-primitive
/// vertex lists.  Mirrors `new GX_DisplayList(pobj)` on the C# side.
pub fn unpack(pobj: &PObj) -> Result<GxDisplayList> {
    let mut dl = GxDisplayList::default();
    dl.attributes = read_attributes(pobj)?;

    if dl.attributes.is_empty()
        || dl.attributes.last().map(|a| a.name) != Some(GxAttribName::GX_VA_NULL)
    {
        // Mirror HSDLib's behavior: warn (we just bail with empty) so the
        // caller still sees the malformed flag in struct dump output.
        return Ok(dl);
    }

    let buffer = match pobj.display_list_buffer() {
        Some(b) => b,
        None => return Ok(dl),
    };

    dl.primitives = read_primitives(&buffer, &dl.attributes)?;
    Ok(dl)
}

// =====================================================================
// Attribute table reader
// =====================================================================

fn read_attributes(pobj: &PObj) -> Result<Vec<GxAttribute>> {
    let Some(attr_struct) = pobj.attributes_struct() else {
        return Ok(Vec::new());
    };
    let len = attr_struct.borrow().len() as u32;
    let mut out = Vec::new();
    let mut off = 0u32;
    while off + 0x18 <= len {
        // Slice the embedded fragment by manually replicating the offsets;
        // GxAttribute::from_struct expects an Rc rooted at offset 0, so we
        // wrap the parent struct re-base via temporary clone.
        let attr = read_attribute_at(&attr_struct, off)?;
        let is_terminator = attr.name == GxAttribName::GX_VA_NULL;
        out.push(attr);
        if is_terminator {
            break;
        }
        off += 0x18;
    }
    Ok(out)
}

fn read_attribute_at(s: &StructRef, off: u32) -> Result<GxAttribute> {
    let b = s.borrow();
    Ok(GxAttribute {
        name: GxAttribName::from(b.get_u32(off + 0x00)?),
        kind: GxAttribType::from(b.get_u32(off + 0x04)?),
        comp_count: b.get_u32(off + 0x08)?,
        comp_type: b.get_u32(off + 0x0C)?,
        scale: b.get_byte(off + 0x10)?,
        stride: b.get_u16(off + 0x12)?,
        // The buffer ref *is* tracked at the `off + 0x14` offset, since the
        // attribute table is a contiguous embedded array within a single
        // parent struct (cf. POBJ.cs:185 FromAttributes which assigns one
        // bookkeeping reference at offset 0x14 of the containing struct, then
        // walks the array embedded after it).  In the parsed Rc tree those
        // refs all live in the parent struct's `references` map keyed by the
        // absolute offset within the parent.
        buffer: b.references().get(&(off + 0x14)).cloned(),
    })
}

// =====================================================================
// Display-list bytecode reader
// =====================================================================

fn read_primitives(
    buffer: &[u8],
    attrs: &[GxAttribute],
) -> Result<Vec<GxPrimitiveGroup>> {
    let mut out = Vec::new();
    let mut cur = Cursor::new(buffer);
    while cur.has_more() {
        let prim_byte = match cur.try_read_u8() {
            Some(b) => b,
            None => break,
        };
        if prim_byte == 0 {
            break;
        }
        let prim_type = GxPrimitiveType::from(prim_byte);
        let count = cur
            .read_u16()
            .ok_or_else(|| HsdError::malformed(0, "DL: truncated vert count"))?;
        let mut vertices = Vec::with_capacity(count as usize);
        for _ in 0..count {
            vertices.push(read_one_vertex(&mut cur, attrs)?);
        }
        out.push(GxPrimitiveGroup {
            primitive_type: prim_type,
            vertices,
        });
    }
    Ok(out)
}

fn read_one_vertex(cur: &mut Cursor<'_>, attrs: &[GxAttribute]) -> Result<GxVertex> {
    let mut v = GxVertex::default();
    let mut direct_clr0 = None::<[u8; 4]>;
    let mut direct_clr1 = None::<[u8; 4]>;
    let mut indices: [u16; 32] = [0; 32];

    for (i, a) in attrs.iter().enumerate() {
        if a.name == GxAttribName::GX_VA_NULL {
            continue;
        }
        match a.kind {
            GxAttribType::GX_DIRECT => match a.name {
                GxAttribName::GX_VA_CLR0 => {
                    direct_clr0 = Some(read_direct_color(cur, a.comp_type)?);
                }
                GxAttribName::GX_VA_CLR1 => {
                    direct_clr1 = Some(read_direct_color(cur, a.comp_type)?);
                }
                _ => {
                    // For position/normal/uv/matrix-idx in DIRECT mode the
                    // DL byte is the value itself (= 1-byte fixed for
                    // matrix-index or similar small attributes).
                    let val = cur
                        .read_u8()
                        .ok_or_else(|| HsdError::malformed(0, "DL: truncated direct"))?;
                    indices[i] = val as u16;
                }
            },
            GxAttribType::GX_INDEX8 => {
                let val = cur
                    .read_u8()
                    .ok_or_else(|| HsdError::malformed(0, "DL: truncated index8"))?;
                indices[i] = val as u16;
            }
            GxAttribType::GX_INDEX16 => {
                let val = cur
                    .read_u16()
                    .ok_or_else(|| HsdError::malformed(0, "DL: truncated index16"))?;
                indices[i] = val;
            }
            GxAttribType::GX_NONE | GxAttribType::Unknown(_) => {}
        }
    }

    // Resolve fields — direct ones are values, indexed ones are buffer reads.
    for (i, a) in attrs.iter().enumerate() {
        if a.name == GxAttribName::GX_VA_NULL {
            continue;
        }
        match a.name {
            GxAttribName::GX_VA_PNMTXIDX => {
                if a.kind == GxAttribType::GX_DIRECT {
                    v.pn_mtx_idx = indices[i];
                }
            }
            GxAttribName::GX_VA_TEX0MTXIDX => {
                if a.kind == GxAttribType::GX_DIRECT {
                    v.tex0_mtx_idx = indices[i];
                }
            }
            GxAttribName::GX_VA_TEX1MTXIDX => {
                if a.kind == GxAttribType::GX_DIRECT {
                    v.tex1_mtx_idx = indices[i];
                }
            }
            GxAttribName::GX_VA_TEX2MTXIDX => {
                if a.kind == GxAttribType::GX_DIRECT {
                    v.tex2_mtx_idx = indices[i];
                }
            }
            GxAttribName::GX_VA_POS => {
                if a.kind != GxAttribType::GX_DIRECT {
                    let f = decode_data_at(a, indices[i] as usize)?;
                    if f.len() >= 1 {
                        v.pos[0] = f[0];
                    }
                    if f.len() >= 2 {
                        v.pos[1] = f[1];
                    }
                    if f.len() >= 3 {
                        v.pos[2] = f[2];
                    }
                }
            }
            GxAttribName::GX_VA_NRM => {
                if a.kind != GxAttribType::GX_DIRECT {
                    let f = decode_data_at(a, indices[i] as usize)?;
                    if f.len() >= 3 {
                        v.nrm = [f[0], f[1], f[2]];
                    }
                }
            }
            GxAttribName::GX_VA_NBT => {
                if a.kind != GxAttribType::GX_DIRECT {
                    let f = decode_data_at(a, indices[i] as usize)?;
                    if f.len() >= 3 {
                        v.nrm = [f[0], f[1], f[2]];
                    }
                    // BITAN/TAN are dropped here (not in scope for course mesh
                    // export); HSDLib stores them in GX_Vertex but csx never
                    // serializes them.
                }
            }
            GxAttribName::GX_VA_CLR0 => {
                if a.kind == GxAttribType::GX_DIRECT {
                    let c = direct_clr0.unwrap_or([255, 255, 255, 255]);
                    v.clr0 = [
                        c[0] as f32 / 255.0,
                        c[1] as f32 / 255.0,
                        c[2] as f32 / 255.0,
                        c[3] as f32 / 255.0,
                    ];
                } else {
                    let f = decode_color_at(a, indices[i] as usize)?;
                    v.clr0 = f;
                }
            }
            GxAttribName::GX_VA_CLR1 => {
                if a.kind == GxAttribType::GX_DIRECT {
                    let c = direct_clr1.unwrap_or([255, 255, 255, 255]);
                    v.clr1 = [
                        c[0] as f32 / 255.0,
                        c[1] as f32 / 255.0,
                        c[2] as f32 / 255.0,
                        c[3] as f32 / 255.0,
                    ];
                } else {
                    let f = decode_color_at(a, indices[i] as usize)?;
                    v.clr1 = f;
                }
            }
            GxAttribName::GX_VA_TEX0
            | GxAttribName::GX_VA_TEX1
            | GxAttribName::GX_VA_TEX2
            | GxAttribName::GX_VA_TEX3
            | GxAttribName::GX_VA_TEX4
            | GxAttribName::GX_VA_TEX5
            | GxAttribName::GX_VA_TEX6
            | GxAttribName::GX_VA_TEX7 => {
                if a.kind != GxAttribType::GX_DIRECT {
                    let f = decode_data_at(a, indices[i] as usize)?;
                    let dst = match a.name {
                        GxAttribName::GX_VA_TEX0 => &mut v.tex0,
                        GxAttribName::GX_VA_TEX1 => &mut v.tex1,
                        GxAttribName::GX_VA_TEX2 => &mut v.tex2,
                        GxAttribName::GX_VA_TEX3 => &mut v.tex3,
                        GxAttribName::GX_VA_TEX4 => &mut v.tex4,
                        GxAttribName::GX_VA_TEX5 => &mut v.tex5,
                        GxAttribName::GX_VA_TEX6 => &mut v.tex6,
                        GxAttribName::GX_VA_TEX7 => &mut v.tex7,
                        _ => unreachable!(),
                    };
                    if f.len() >= 1 {
                        dst[0] = f[0];
                    }
                    if f.len() >= 2 {
                        dst[1] = f[1];
                    }
                }
            }
            _ => {}
        }
    }

    Ok(v)
}

fn decode_data_at(a: &GxAttribute, index: usize) -> Result<Vec<f32>> {
    let Some(buffer) = &a.buffer else {
        return Ok(Vec::new());
    };
    let stride = a.stride as usize;
    if stride == 0 {
        return Ok(Vec::new());
    }
    let comp_type = GxCompType::from(a.comp_type);
    let comp_size = match comp_type {
        GxCompType::UInt8 | GxCompType::Int8 => 1,
        GxCompType::UInt16 | GxCompType::Int16 => 2,
        GxCompType::Float => 4,
        GxCompType::Unknown(_) => 1,
    };
    let count = stride / comp_size;
    let mut out = Vec::with_capacity(count);
    let offset = stride * index;
    let buf = buffer.borrow();

    for i in 0..count {
        let val_off = (offset + i * comp_size) as u32;
        let raw = match comp_type {
            GxCompType::UInt8 => buf.get_byte(val_off)? as f32,
            GxCompType::Int8 => {
                let v = buf.get_byte(val_off)?;
                (v as i8) as f32
            }
            GxCompType::UInt16 => buf.get_u16(val_off)? as f32,
            GxCompType::Int16 => buf.get_i16(val_off)? as f32,
            GxCompType::Float => buf.get_f32(val_off)?,
            GxCompType::Unknown(_) => buf.get_byte(val_off)? as f32,
        };
        out.push(raw / ((1u32 << a.scale) as f32));
    }
    Ok(out)
}

fn decode_color_at(a: &GxAttribute, index: usize) -> Result<[f32; 4]> {
    let mut c = [1f32; 4];
    let Some(buffer) = &a.buffer else {
        return Ok(c);
    };
    let stride = a.stride as usize;
    let offset = (stride * index) as u32;
    let buf = buffer.borrow();

    // GXCompTypeClr (HSDLib enum) shares numeric values with GXCompType but
    // means: 0=RGB565 1=RGB8 2=RGBX8 3=RGBA4 4=RGBA6 5=RGBA8.  Mirror the
    // HSDLib `GetColorAt` switch byte-for-byte.
    match a.comp_type {
        // RGB565
        0 => {
            let pixel = buf.get_i16(offset)?;
            c[0] = ((((pixel >> 0) & 0x1F) << 3) & 0xff) as f32 / 255.0;
            c[1] = ((((pixel >> 5) & 0x3F) << 2) & 0xff) as f32 / 255.0;
            c[2] = ((((pixel >> 11) & 0x1F) << 3) & 0xff) as f32 / 255.0;
            c[3] = 1.0;
        }
        // RGB8
        1 => {
            c[0] = buf.get_byte(offset)? as f32 / 255.0;
            c[1] = buf.get_byte(offset + 1)? as f32 / 255.0;
            c[2] = buf.get_byte(offset + 2)? as f32 / 255.0;
            c[3] = 1.0;
        }
        // RGBA4
        3 => {
            let b0 = buf.get_byte(offset)?;
            let b1 = buf.get_byte(offset + 1)?;
            c[0] = ((b0 >> 4) as f32) * 16.0 / 255.0;
            c[1] = ((b0 & 0xF) as f32) * 16.0 / 255.0;
            c[2] = ((b1 >> 4) as f32) * 16.0 / 255.0;
            c[3] = ((b1 & 0xF) as f32) * 16.0 / 255.0;
        }
        // RGBA6 — HSDLib calls this approximate; we mirror it.
        4 => {
            let p = (buf.get_u32(offset)? & 0x00FF_FFFF) as f32;
            let p = p as u32;
            c[0] = ((p >> 18) & 0x3F) as f32 / 0x3F as f32;
            c[1] = ((p >> 12) & 0x3F) as f32 / 0x3F as f32;
            c[2] = ((p >> 6) & 0x3F) as f32 / 0x3F as f32;
            c[3] = (p & 0x3F) as f32 / 0x3F as f32;
        }
        // RGBA8 / RGBX8 / others — straight 4-byte read
        2 | 5 => {
            c[0] = buf.get_byte(offset)? as f32 / 255.0;
            c[1] = buf.get_byte(offset + 1)? as f32 / 255.0;
            c[2] = buf.get_byte(offset + 2)? as f32 / 255.0;
            c[3] = buf.get_byte(offset + 3)? as f32 / 255.0;
        }
        _ => {}
    }
    Ok(c)
}

fn read_direct_color(cur: &mut Cursor<'_>, comp_type: u32) -> Result<[u8; 4]> {
    // Mirrors `GX_PrimitiveGroup.ReadDirectGXColor`.
    let mut clr = [255u8, 255, 255, 255];
    match comp_type {
        // 0 = RGB565
        0 => {
            let b = cur
                .read_u16()
                .ok_or_else(|| HsdError::malformed(0, "DL: truncated dir RGB565"))?;
            clr[0] = ((((b >> 11) & 0x1F) << 3) | (((b >> 11) & 0x1F) >> 2)) as u8;
            clr[1] = ((((b >> 5) & 0x3F) << 2) | (((b >> 5) & 0x3F) >> 4)) as u8;
            clr[2] = ((((b) & 0x1F) << 3) | (((b) & 0x1F) >> 2)) as u8;
            clr[3] = 255;
        }
        // 1 = RGB888
        1 => {
            clr[0] = cur.read_u8().ok_or_else(|| HsdError::malformed(0, "DL: truncated"))?;
            clr[1] = cur.read_u8().ok_or_else(|| HsdError::malformed(0, "DL: truncated"))?;
            clr[2] = cur.read_u8().ok_or_else(|| HsdError::malformed(0, "DL: truncated"))?;
            clr[3] = 255;
        }
        // 2 = RGBX888
        2 | 5 => {
            clr[0] = cur.read_u8().ok_or_else(|| HsdError::malformed(0, "DL: truncated"))?;
            clr[1] = cur.read_u8().ok_or_else(|| HsdError::malformed(0, "DL: truncated"))?;
            clr[2] = cur.read_u8().ok_or_else(|| HsdError::malformed(0, "DL: truncated"))?;
            clr[3] = cur.read_u8().ok_or_else(|| HsdError::malformed(0, "DL: truncated"))?;
        }
        // 3 = RGBA4
        3 => {
            let b = cur
                .read_u16()
                .ok_or_else(|| HsdError::malformed(0, "DL: truncated dir RGBA4"))?;
            clr[0] = ((((b >> 12) & 0xF) << 4) | ((b >> 12) & 0xF)) as u8;
            clr[1] = ((((b >> 8) & 0xF) << 4) | ((b >> 8) & 0xF)) as u8;
            clr[2] = ((((b >> 4) & 0xF) << 4) | ((b >> 4) & 0xF)) as u8;
            clr[3] = ((((b) & 0xF) << 4) | ((b) & 0xF)) as u8;
        }
        // 4 = RGBA6
        4 => {
            let b1 = cur.read_u8().ok_or_else(|| HsdError::malformed(0, "DL: truncated dir RGBA6"))?;
            let b2 = cur.read_u8().ok_or_else(|| HsdError::malformed(0, "DL: truncated dir RGBA6"))?;
            let b3 = cur.read_u8().ok_or_else(|| HsdError::malformed(0, "DL: truncated dir RGBA6"))?;
            let b = ((b1 as u32) << 16) | ((b2 as u32) << 8) | (b3 as u32);
            clr[0] = ((((b >> 18) & 0x3F) << 2) | (((b >> 18) & 0x3F) >> 4)) as u8;
            clr[1] = ((((b >> 12) & 0x3F) << 2) | (((b >> 12) & 0x3F) >> 4)) as u8;
            clr[2] = ((((b >> 6) & 0x3F) << 2) | (((b >> 6) & 0x3F) >> 4)) as u8;
            clr[3] = ((((b) & 0x3F) << 2) | (((b) & 0x3F) >> 4)) as u8;
        }
        _ => {
            return Err(HsdError::malformed(0, "DL: unknown direct color comp_type"));
        }
    }
    Ok(clr)
}

// =====================================================================
// Big-endian byte cursor
// =====================================================================

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn has_more(&self) -> bool {
        self.pos < self.data.len()
    }
    fn try_read_u8(&mut self) -> Option<u8> {
        let v = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }
    fn read_u8(&mut self) -> Option<u8> {
        self.try_read_u8()
    }
    fn read_u16(&mut self) -> Option<u16> {
        if self.pos + 2 > self.data.len() {
            return None;
        }
        let v = u16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Some(v)
    }
}

// =====================================================================
// Forward kinematics: JObj local + world matrix builder.
// `docs/notes/phase0.md` §6 specifies row-vector convention with
// `local = S * R(xyz) * T` and `world = local * parentWorld`.
// =====================================================================

use crate::common::JObj;

/// 4x4 row-major matrix.  The entries follow `m[row][col]` indexing; written
/// out in csx as `M11..M44` by row.
pub type Mat4 = [[f32; 4]; 4];

pub fn mat4_identity() -> Mat4 {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn mat4_mul(a: Mat4, b: Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[i][k] * b[k][j];
            }
            out[i][j] = s;
        }
    }
    out
}

pub fn mat4_translate(tx: f32, ty: f32, tz: f32) -> Mat4 {
    let mut m = mat4_identity();
    m[3][0] = tx;
    m[3][1] = ty;
    m[3][2] = tz;
    m
}

pub fn mat4_scale(sx: f32, sy: f32, sz: f32) -> Mat4 {
    let mut m = mat4_identity();
    m[0][0] = sx;
    m[1][1] = sy;
    m[2][2] = sz;
    m
}

/// Euler XYZ rotation matrix matching csx `MatrixFromEuler` exactly:
///     | cy*cz                cy*sz             -sy   0 |
///     | cz*sx*sy - cx*sz     sz*sx*sy + cx*cz   sx*cy 0 |
///     | cz*cx*sy + sx*sz     sz*cx*sy - sx*cz   cx*cy 0 |
///     | 0                    0                  0     1 |
pub fn mat4_euler_xyz(rx: f32, ry: f32, rz: f32) -> Mat4 {
    let (sx, cx) = (rx.sin(), rx.cos());
    let (sy, cy) = (ry.sin(), ry.cos());
    let (sz, cz) = (rz.sin(), rz.cos());
    [
        [cy * cz, cy * sz, -sy, 0.0],
        [cz * sx * sy - cx * sz, sz * sx * sy + cx * cz, sx * cy, 0.0],
        [cz * cx * sy + sx * sz, sz * cx * sy - sx * cz, cx * cy, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

pub fn jobj_local(j: &JObj) -> Result<Mat4> {
    let s = mat4_scale(j.sx()?, j.sy()?, j.sz()?);
    let r = mat4_euler_xyz(j.rx()?, j.ry()?, j.rz()?);
    let t = mat4_translate(j.tx()?, j.ty()?, j.tz()?);
    // local = S * R * T (row-vector)
    Ok(mat4_mul(mat4_mul(s, r), t))
}

/// Transform a row-vector position `[x y z 1]` by a 4x4 matrix.
pub fn transform_point(p: [f32; 3], m: &Mat4) -> [f32; 3] {
    [
        p[0] * m[0][0] + p[1] * m[1][0] + p[2] * m[2][0] + m[3][0],
        p[0] * m[0][1] + p[1] * m[1][1] + p[2] * m[2][1] + m[3][1],
        p[0] * m[0][2] + p[1] * m[1][2] + p[2] * m[2][2] + m[3][2],
    ]
}

/// Transform a row-vector normal — translation column zeroed, no
/// renormalization here (callers normalize once at the end).
pub fn transform_normal(n: [f32; 3], m: &Mat4) -> [f32; 3] {
    [
        n[0] * m[0][0] + n[1] * m[1][0] + n[2] * m[2][0],
        n[0] * m[0][1] + n[1] * m[1][1] + n[2] * m[2][1],
        n[0] * m[0][2] + n[1] * m[1][2] + n[2] * m[2][2],
    ]
}

pub fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l == 0.0 {
        [0.0, 0.0, 0.0]
    } else {
        [v[0] / l, v[1] / l, v[2] / l]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_round_trip() {
        let i = mat4_identity();
        let p = transform_point([1.0, 2.0, 3.0], &i);
        assert_eq!(p, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn translate_then_scale() {
        let m = mat4_mul(mat4_scale(2.0, 2.0, 2.0), mat4_translate(1.0, 0.0, 0.0));
        // row-vector: v' = v * (S*T) — first scale, then translate
        let p = transform_point([1.0, 0.0, 0.0], &m);
        assert!((p[0] - 3.0).abs() < 1e-6, "got {:?}", p);
    }
}
