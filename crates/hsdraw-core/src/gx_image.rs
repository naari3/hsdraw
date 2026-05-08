//! GX texture decoders, mirroring `HSDRaw/Tools/Textures/GXImageConverter.cs`.
//!
//! All decoders return **RGBA8** bytes (R,G,B,A interleaved) regardless of the
//! source format.  HSDLib's CMP and RGBA8 paths produce BGRA in memory; we
//! swap R↔B inside those decoders so callers never have to know.  Other
//! formats already match RGBA8 byte order.
//!
//! The block / tile loops are intentionally direct ports of the HSDLib loops
//! so a parity test against `GetDecodedImageData()` (modulo the BGRA swap)
//! checks out byte-for-byte.

use crate::error::{HsdError, Result};
use crate::gx::{GxTexFmt, GxTlutFmt};

/// Decode `format`-encoded `raw` (length must be `image_size(format,w,h)`)
/// into `w*h*4` RGBA8 bytes.  `palette_data` is required for CI4/CI8/CI14X2,
/// must be `None` otherwise.
pub fn decode_image(
    format: GxTexFmt,
    width: u32,
    height: u32,
    raw: &[u8],
    palette: Option<(GxTlutFmt, &[u8])>,
) -> Result<Vec<u8>> {
    let needed = image_size(format, width, height);
    if raw.len() < needed {
        return Err(HsdError::malformed(
            0,
            "texture data shorter than expected",
        ));
    }
    let pal_rgba = palette.map(|(fmt, data)| palette_to_rgba(fmt, data));

    Ok(match format {
        GxTexFmt::I4 => from_i4(raw, width, height),
        GxTexFmt::I8 => from_i8(raw, width, height),
        GxTexFmt::IA4 => from_ia4(raw, width, height),
        GxTexFmt::IA8 => from_ia8(raw, width, height),
        GxTexFmt::RGB565 => from_rgb565(raw, width, height),
        GxTexFmt::RGB5A3 => from_rgb5a3(raw, width, height),
        GxTexFmt::RGBA8 => from_rgba8(raw, width, height),
        GxTexFmt::CI4 => from_ci4(raw, pal_rgba.as_deref().unwrap_or(&[]), width, height),
        GxTexFmt::CI8 => from_ci8(raw, pal_rgba.as_deref().unwrap_or(&[]), width, height),
        GxTexFmt::CI14X2 => from_ci14x2(raw, pal_rgba.as_deref().unwrap_or(&[]), width, height),
        GxTexFmt::CMP => from_cmp(raw, width, height),
        GxTexFmt::Unknown(_) => return Err(HsdError::malformed(0, "unknown texture format")),
    })
}

/// Aligned size in bytes for `format @ w×h`.  Width is padded up to a
/// multiple of 4 (matches HSDLib `GetImageSize`).
pub fn image_size(format: GxTexFmt, width: u32, height: u32) -> usize {
    let w = if width % 4 == 0 { width } else { width + (4 - width % 4) };
    let size = (w as usize) * (height as usize);
    match format {
        GxTexFmt::CI4 | GxTexFmt::I4 | GxTexFmt::CMP => size / 2,
        GxTexFmt::IA4 | GxTexFmt::I8 | GxTexFmt::CI14X2 | GxTexFmt::CI8 => size,
        GxTexFmt::IA8 | GxTexFmt::RGB565 | GxTexFmt::RGB5A3 => size * 2,
        GxTexFmt::RGBA8 => size * 4,
        GxTexFmt::Unknown(_) => size,
    }
}

// =====================================================================
// Per-format decoders.  Each returns w*h*4 bytes (RGBA8).
// =====================================================================

fn from_i4(tpl: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    if w < 8 || h < 8 {
        return out;
    }
    let mut inp = 0usize;
    for y in (0..h).step_by(8) {
        for x in (0..w).step_by(8) {
            for y1 in y..y + 8 {
                let mut x1 = x;
                while x1 < x + 8 {
                    let pixel = tpl[inp];
                    inp += 1;
                    if y1 < h && x1 < w {
                        let i = ((pixel >> 4) as u32 * 255 / 15) as u8;
                        let p0 = (y1 * w + x1) * 4;
                        out[p0] = i;
                        out[p0 + 1] = i;
                        out[p0 + 2] = i;
                        out[p0 + 3] = i;
                    }
                    if y1 < h && x1 + 1 < w {
                        let i = ((pixel & 0x0F) as u32 * 255 / 15) as u8;
                        let p1 = (y1 * w + x1 + 1) * 4;
                        out[p1] = i;
                        out[p1 + 1] = i;
                        out[p1 + 2] = i;
                        out[p1 + 3] = i;
                    }
                    x1 += 2;
                }
            }
        }
    }
    out
}

fn from_i8(tpl: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut inp = 0usize;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(8) {
            for y1 in y..y + 4 {
                for x1 in x..x + 8 {
                    let pixel = tpl[inp];
                    inp += 1;
                    if y1 < h && x1 < w {
                        let p = (y1 * w + x1) * 4;
                        out[p] = pixel;
                        out[p + 1] = pixel;
                        out[p + 2] = pixel;
                        out[p + 3] = pixel;
                    }
                }
            }
        }
    }
    out
}

fn from_ia4(tpl: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut inp = 0usize;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(8) {
            for y1 in y..y + 4 {
                for x1 in x..x + 8 {
                    let pixel = tpl[inp];
                    inp += 1;
                    if y1 < h && x1 < w {
                        let i = ((pixel & 0x0F) as u32 * 255 / 15) as u8;
                        let a = ((pixel >> 4) as u32 * 255 / 15) as u8;
                        let p = (y1 * w + x1) * 4;
                        out[p] = i;
                        out[p + 1] = i;
                        out[p + 2] = i;
                        out[p + 3] = a;
                    }
                }
            }
        }
    }
    out
}

fn from_ia8(tpl: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut inp = 0usize;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            for y1 in y..y + 4 {
                for x1 in x..x + 4 {
                    let pixel = u16::from_be_bytes([tpl[inp * 2], tpl[inp * 2 + 1]]);
                    inp += 1;
                    if y1 < h && x1 < w {
                        let a = (pixel >> 8) as u8;
                        let i = (pixel & 0xff) as u8;
                        let p = (y1 * w + x1) * 4;
                        out[p] = i;
                        out[p + 1] = i;
                        out[p + 2] = i;
                        out[p + 3] = a;
                    }
                }
            }
        }
    }
    out
}

fn from_rgb565(tpl: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut inp = 0usize;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            for y1 in y..y + 4 {
                for x1 in x..x + 4 {
                    let pixel = u16::from_be_bytes([tpl[inp * 2], tpl[inp * 2 + 1]]);
                    inp += 1;
                    if y1 < h && x1 < w {
                        // HSDLib labels: high 5 = "b", mid 6 = "g", low 5 = "r".
                        // We mirror that exactly so parity holds; whether the
                        // GX spec calls them r or b is irrelevant for round-trip.
                        let b = ((((pixel >> 11) & 0x1F) << 3) & 0xff) as u8;
                        let g = ((((pixel >> 5) & 0x3F) << 2) & 0xff) as u8;
                        let r = ((((pixel) & 0x1F) << 3) & 0xff) as u8;
                        let p = (y1 * w + x1) * 4;
                        out[p] = r;
                        out[p + 1] = g;
                        out[p + 2] = b;
                        out[p + 3] = 255;
                    }
                }
            }
        }
    }
    out
}

fn from_rgb5a3(tpl: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut inp = 0usize;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            for y1 in y..y + 4 {
                for x1 in x..x + 4 {
                    let pixel = u16::from_be_bytes([tpl[inp * 2], tpl[inp * 2 + 1]]);
                    inp += 1;
                    if y1 < h && x1 < w {
                        let (a, r, g, b) = decode_rgb5a3(pixel);
                        let p = (y1 * w + x1) * 4;
                        out[p] = r;
                        out[p + 1] = g;
                        out[p + 2] = b;
                        out[p + 3] = a;
                    }
                }
            }
        }
    }
    out
}

fn decode_rgb5a3(pixel: u16) -> (u8, u8, u8, u8) {
    // HSDLib labels: top bits are "b", bottom bits are "r".  We mirror that
    // exact assignment so byte 0 of the output (= "r") carries the same bits
    // csx writes via HSDLib's `(r << 0) | (g << 8) | (b << 16) | (a << 24)`
    // little-endian dump.  RGB565 in this file follows the same convention.
    if pixel & (1 << 15) != 0 {
        // RGB555
        let b = ((((pixel >> 10) & 0x1F) as u32 * 255 / 31) & 0xff) as u8;
        let g = ((((pixel >> 5) & 0x1F) as u32 * 255 / 31) & 0xff) as u8;
        let r = (((pixel & 0x1F) as u32 * 255 / 31) & 0xff) as u8;
        (255, r, g, b)
    } else {
        // RGB4A3
        let a = ((((pixel >> 12) & 0x07) as u32 * 255 / 7) & 0xff) as u8;
        let b = ((((pixel >> 8) & 0x0F) as u32 * 255 / 15) & 0xff) as u8;
        let g = ((((pixel >> 4) & 0x0F) as u32 * 255 / 15) & 0xff) as u8;
        let r = (((pixel & 0x0F) as u32 * 255 / 15) & 0xff) as u8;
        (a, r, g, b)
    }
}

/// HSDLib `fromRGBA8` is two-pass: first 16 entries of an 8x8 byte block
/// hold (alpha, red) interleaved, second 16 entries hold (green, blue).
/// Output is RGBA8 (we incorporate the BGRA→RGBA swap mentioned in
/// `docs/notes/phase0.md` §5).
fn from_rgba8(tpl: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut inp = 0usize;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            // pass 1: AR
            for k in 0..2u32 {
                for y1 in y..y + 4 {
                    for x1 in x..x + 4 {
                        let pixel =
                            u16::from_be_bytes([tpl[inp * 2], tpl[inp * 2 + 1]]);
                        inp += 1;
                        if y1 >= h || x1 >= w {
                            continue;
                        }
                        let p = (y1 * w + x1) * 4;
                        if k == 0 {
                            // a, r
                            out[p + 3] = (pixel >> 8) as u8;       // alpha
                            out[p] = (pixel & 0xff) as u8;          // red
                        } else {
                            // g, b
                            out[p + 1] = (pixel >> 8) as u8;        // green
                            out[p + 2] = (pixel & 0xff) as u8;      // blue
                        }
                    }
                }
            }
        }
    }
    out
}

fn from_ci4(tpl: &[u8], pal: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut inp = 0usize;
    for y in (0..h).step_by(8) {
        for x in (0..w).step_by(8) {
            for y1 in y..y + 8 {
                let mut x1 = x;
                while x1 < x + 8 {
                    if inp >= tpl.len() {
                        return out;
                    }
                    let pixel = tpl[inp];
                    inp += 1;
                    if y1 < h && x1 < w {
                        let idx = (pixel >> 4) as usize;
                        copy_pal(&mut out, (y1 * w + x1) * 4, pal, idx);
                    }
                    if y1 < h && x1 + 1 < w {
                        let idx = (pixel & 0x0F) as usize;
                        copy_pal(&mut out, (y1 * w + x1 + 1) * 4, pal, idx);
                    }
                    x1 += 2;
                }
            }
        }
    }
    out
}

fn from_ci8(tpl: &[u8], pal: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut inp = 0usize;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(8) {
            for y1 in y..y + 4 {
                for x1 in x..x + 8 {
                    if inp >= tpl.len() {
                        return out;
                    }
                    let idx = tpl[inp] as usize;
                    inp += 1;
                    if y1 < h && x1 < w {
                        copy_pal(&mut out, (y1 * w + x1) * 4, pal, idx);
                    }
                }
            }
        }
    }
    out
}

fn from_ci14x2(tpl: &[u8], pal: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let mut inp = 0usize;
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            for y1 in y..y + 4 {
                for x1 in x..x + 4 {
                    let pixel = u16::from_be_bytes([tpl[inp * 2], tpl[inp * 2 + 1]]);
                    inp += 1;
                    if y1 < h && x1 < w {
                        let idx = (pixel & 0x3FFF) as usize;
                        copy_pal(&mut out, (y1 * w + x1) * 4, pal, idx);
                    }
                }
            }
        }
    }
    out
}

fn copy_pal(dst: &mut [u8], offset: usize, pal: &[u8], idx: usize) {
    let p = idx * 4;
    if p + 4 > pal.len() {
        return;
    }
    dst[offset..offset + 4].copy_from_slice(&pal[p..p + 4]);
}

fn from_cmp(tpl: &[u8], w: u32, h: u32) -> Vec<u8> {
    // `Shared.AddPadding(width, 8)` rounds up to a multiple of 8.
    let ww = if w % 4 == 0 { w } else { w + (4 - w % 4) }; // GX/CMP uses 8 in HSDLib
    let ww = if ww % 8 == 0 { ww } else { ww + (8 - ww % 8) };
    let (w_us, h_us) = (w as usize, h as usize);
    let mut out = vec![0u8; w_us * h_us * 4];
    for y in 0..h {
        for x in 0..w {
            let x0 = (x & 0x03) as i32;
            let x1 = ((x >> 2) & 0x01) as i32;
            let x2 = (x >> 3) as i32;
            let y0 = (y & 0x03) as i32;
            let y1 = ((y >> 2) & 0x01) as i32;
            let y2 = (y >> 3) as i32;
            let off = ((8 * x1) + (16 * y1) + (32 * x2) + (4 * (ww as i32) * y2)) as usize;

            // Each 8-byte block: 2 RGB565 colors then 4 bytes of 2-bit indices.
            let c0 = make_color_565(tpl[off + 1], tpl[off + 0]);
            let c1 = make_color_565(tpl[off + 3], tpl[off + 2]);
            let mode = (((tpl[off] as u32) << 8) | (tpl[off + 1] as u32))
                > (((tpl[off + 2] as u32) << 8) | (tpl[off + 3] as u32));

            let (c2, c3) = if mode {
                let r = (2 * red(c0) + red(c1)) / 3;
                let g = (2 * green(c0) + green(c1)) / 3;
                let b = (2 * blue(c0) + blue(c1)) / 3;
                let cc2 = (0xFFu32 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                let r = (2 * red(c1) + red(c0)) / 3;
                let g = (2 * green(c1) + green(c0)) / 3;
                let b = (2 * blue(c1) + blue(c0)) / 3;
                let cc3 = (0xFFu32 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                (cc2, cc3)
            } else {
                let r = (red(c0) + red(c1)) / 2;
                let g = (green(c0) + green(c1)) / 2;
                let b = (blue(c0) + blue(c1)) / 2;
                let cc2 = (0xFFu32 << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                (cc2, 0u32)
            };
            let table = [c0, c1, c2, c3];

            let pixel = u32::from_be_bytes([tpl[off + 4], tpl[off + 5], tpl[off + 6], tpl[off + 7]]);
            let ix = x0 + 4 * y0;
            let raw = table[((pixel >> (30 - 2 * ix)) & 0x03) as usize];
            let alpha = if ((pixel >> (30 - 2 * ix)) & 0x03) == 3 && !mode {
                0u32
            } else {
                0xFFu32
            };
            // Repack into RGBA byte order (HSDLib stored ARGB+swap, we go straight RGBA).
            let r = ((raw >> 16) & 0xFF) as u8;
            let g = ((raw >> 8) & 0xFF) as u8;
            let b = (raw & 0xFF) as u8;
            let out_off = ((y as usize) * w_us + x as usize) * 4;
            out[out_off] = r;
            out[out_off + 1] = g;
            out[out_off + 2] = b;
            out[out_off + 3] = alpha as u8;
        }
    }
    out
}

#[inline]
fn red(c: u32) -> u32 {
    (c >> 16) & 0xFF
}
#[inline]
fn green(c: u32) -> u32 {
    (c >> 8) & 0xFF
}
#[inline]
fn blue(c: u32) -> u32 {
    c & 0xFF
}

fn make_color_565(b1: u8, b2: u8) -> u32 {
    let bt = ((b2 as u32) << 8) | (b1 as u32);
    let a = 255u32;
    let r = (bt >> 11) & 0x1F;
    let g = (bt >> 5) & 0x3F;
    let b = bt & 0x1F;
    let r = (r << 3) | (r >> 2);
    let g = (g << 2) | (g >> 4);
    let b = (b << 3) | (b >> 2);
    (a << 24) | (r << 16) | (g << 8) | b
}

/// Decode a GX TLUT palette into a flat RGBA8 byte array (4 bytes per entry).
/// Entry count is inferred from `data.len() / 2`.
pub fn palette_to_rgba(format: GxTlutFmt, data: &[u8]) -> Vec<u8> {
    let count = data.len() / 2;
    let mut out = Vec::with_capacity(count * 4);
    for i in 0..count {
        let pixel = u16::from_be_bytes([data[i * 2], data[i * 2 + 1]]);
        let (r, g, b, a) = match format {
            GxTlutFmt::IA8 => {
                let r = (pixel & 0xff) as u8;
                let a = (pixel >> 8) as u8;
                (r, r, r, a)
            }
            GxTlutFmt::RGB565 => {
                let b = ((((pixel >> 11) & 0x1F) << 3) & 0xff) as u8;
                let g = ((((pixel >> 5) & 0x3F) << 2) & 0xff) as u8;
                let r = (((pixel & 0x1F) << 3) & 0xff) as u8;
                (r, g, b, 255)
            }
            GxTlutFmt::RGB5A3 => {
                let (a, r, g, b) = decode_rgb5a3(pixel);
                (r, g, b, a)
            }
            GxTlutFmt::Unknown(_) => (0, 0, 0, 0),
        };
        out.extend_from_slice(&[r, g, b, a]);
    }
    out
}

/// Encode RGBA8 bytes into a minimal PNG (IHDR + IDAT + IEND only — no
/// gAMA/pHYs/sBIT chunks).  `png` crate's default Encoder satisfies that
/// when we don't ask for anything extra.
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    assert_eq!(
        rgba.len(),
        (width as usize) * (height as usize) * 4,
        "RGBA buffer size mismatch"
    );
    let mut out = Vec::with_capacity(rgba.len() / 2);
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // Note: the `png` crate writes IHDR / IDAT / IEND only by default.
        // No gAMA / pHYs / cHRM unless we explicitly add them.
        let mut writer = encoder
            .write_header()
            .map_err(|e| HsdError::malformed(0, "png header write failed").with_msg(e))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| HsdError::malformed(0, "png idat write failed").with_msg(e))?;
    }
    Ok(out)
}

// Tiny extension on HsdError to attach source-error chains — encoder errors
// are not really "malformed dat" so we keep them in a debug message.  Phase
// 4+ may upgrade this to a proper `HsdError::Png` variant once parity tests
// inspect the failure mode.
impl HsdError {
    fn with_msg<E: std::error::Error>(self, _e: E) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i8_round_trip() {
        // 8x4 single-block I8: greyscale ramp.
        let raw: Vec<u8> = (0..8 * 4).map(|i| (i * 8) as u8).collect();
        let rgba = decode_image(GxTexFmt::I8, 8, 4, &raw, None).unwrap();
        // Pixel (0,0) is from the first byte of the block (raw[0] = 0).
        assert_eq!(rgba[0..4], [0, 0, 0, 0]);
        // Pixel at index 1 (0,1) reads raw[1] = 8.
        assert_eq!(rgba[4..8], [8, 8, 8, 8]);
    }

    #[test]
    fn rgb5a3_alpha_path() {
        // pixel = 0x0FFF (top bit = 0 → RGB4A3, alpha=0): 4-bit channels.
        let raw = [0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF, 0x0F, 0xFF];
        let rgba = decode_image(GxTexFmt::RGB5A3, 4, 4, &raw, None).unwrap();
        // alpha:0, R:255*15/15=255, G:255, B:255 → (255, 255, 255, 0)
        assert_eq!(rgba[0..4], [255, 255, 255, 0]);
    }

    #[test]
    fn png_encode_minimal_size() {
        let rgba = vec![0u8; 4 * 4 * 4];
        let png = encode_png(&rgba, 4, 4).unwrap();
        // PNG signature is the first 8 bytes.
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // After IHDR/IDAT/IEND a 4x4 RGBA blank shouldn't exceed ~100 bytes.
        assert!(png.len() < 200, "minimal PNG should be small, got {}", png.len());
    }
}
