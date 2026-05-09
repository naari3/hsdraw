//! Round-trip tests for the GX texture encoders.
//!
//! Mechanical formats (RGBA8 / RGB565 / RGB5A3) round-trip byte-equal
//! through encode→decode for any encoder-representable RGBA8 input
//! (i.e. RGB565 inputs whose channels were already snapped to the
//! 5/6/5-bit grid, etc.).  We verify by:
//!   1. starting from a small handcrafted RGBA8 pattern,
//!   2. encoding,
//!   3. decoding,
//!   4. asserting decoded pixels match the encoder-quantized expected
//!      pixels (i.e. the RGB565-snapped or RGB5A3-snapped form of the
//!      input — exact byte equality).
//!
//! For CMP we don't claim byte-equality (BC1 is lossy and the encoder
//! choice differs from HSDLib's), so we assert per-channel RMS error
//! against the original RGBA8 input is below a quality threshold that
//! all-color blocks satisfy in practice.

use hsdraw_core::gx::GxTexFmt;
use hsdraw_core::gx_image::{
    decode_image, encode_image, encode_image_with_options, image_size, EncodeOptions,
};

/// Build a 4x4 RGBA8 pattern with deterministic per-pixel values.
fn pattern_4x4() -> Vec<u8> {
    let mut out = Vec::with_capacity(4 * 4 * 4);
    for y in 0..4u8 {
        for x in 0..4u8 {
            out.extend_from_slice(&[
                0x10 + x * 0x20,            // R
                0x10 + y * 0x20,            // G
                0x80,                       // B (constant)
                0xFF,                       // A
            ]);
        }
    }
    out
}

/// Quantize-then-dequantize an 8-bit channel through the encoder's
/// round-half-up pick (`q = (c8 * levels + 127) / 255`) followed by the
/// decoder's de-quantizer.  Produces the byte-exact value the decoder
/// will report after a round-trip — useful for asserting encode→decode
/// equality of pre-snapped inputs.
///
/// `mode` selects which decoder formula the format uses:
///   * `DequantMode::Shift{n}` — RGB565 path.  Decoder is simple
///     left-shift `c_n << (8 - n)` (no smoothing low bits), giving
///     channels on the `2^(8-n)`-step grid.
///   * `DequantMode::Smooth{levels}` — RGB5A3 / palette / general
///     path.  Decoder is `c_n * 255 / levels` integer-floor.
fn quant_via(c8: u8, levels: u32, mode: DequantMode) -> u8 {
    let q = ((c8 as u32) * levels + 127) / 255;
    match mode {
        DequantMode::Shift { n } => ((q << (8 - n)) & 0xff) as u8,
        DequantMode::Smooth => ((q * 255) / levels) as u8,
    }
}

#[derive(Copy, Clone)]
enum DequantMode {
    /// Used for RGB565 channels: decoder does `c_n << (8 - n)`.
    Shift { n: u32 },
    /// Used for RGB5A3 / RGBA8: decoder does `c_n * 255 / levels`.
    Smooth,
}

#[test]
fn rgba8_round_trips_byte_equal_for_4x4() {
    let rgba = pattern_4x4();
    let encoded = encode_image(GxTexFmt::RGBA8, 4, 4, &rgba).expect("encode RGBA8");
    // 4x4 RGBA8 = 4*4*4 = 64 bytes.
    assert_eq!(encoded.len(), image_size(GxTexFmt::RGBA8, 4, 4));
    assert_eq!(encoded.len(), 64);

    let decoded = decode_image(GxTexFmt::RGBA8, 4, 4, &encoded, None).expect("decode");
    assert_eq!(decoded, rgba, "RGBA8 must round-trip byte-equal");
}

#[test]
fn rgba8_pads_width_to_4() {
    // 5x4 (width = 5 → padded to 8).  Encoded size matches image_size for
    // the *padded* width (image_size pads w up to 4, so 8*4*4 = 128).
    let mut rgba = vec![0u8; 5 * 4 * 4];
    for i in (0..rgba.len()).step_by(4) {
        rgba[i] = (i % 256) as u8;
        rgba[i + 3] = 0xFF;
    }
    let encoded = encode_image(GxTexFmt::RGBA8, 5, 4, &rgba).expect("encode");
    // Padded width = 8 → 8 * 4 * 4 = 128 bytes.
    assert_eq!(encoded.len(), 128);
}

#[test]
fn rgb565_round_trips_for_quantized_input() {
    // Use only encoder-representable values (5/6/5-bit channel grid)
    // so encode→decode is byte-equal.  We snap the source ahead of
    // time so the assertion checks the entire pipeline, not just the
    // round-trip of pre-snapped values.
    let src = pattern_4x4();
    // Snap each channel to the 5/6/5-bit grid the way the encoder will.
    // RGB565 decoder uses simple left-shift (`c_n << (8 - n)`), not the
    // smoother `c_n * 255 / levels` form RGB5A3 / RGBA8 use.
    let mut expected = src.clone();
    for px in expected.chunks_mut(4) {
        px[0] = quant_via(px[0], 31, DequantMode::Shift { n: 5 }); // R
        px[1] = quant_via(px[1], 63, DequantMode::Shift { n: 6 }); // G
        px[2] = quant_via(px[2], 31, DequantMode::Shift { n: 5 }); // B
        px[3] = 255;
    }

    let encoded = encode_image(GxTexFmt::RGB565, 4, 4, &src).expect("encode");
    assert_eq!(encoded.len(), image_size(GxTexFmt::RGB565, 4, 4));
    assert_eq!(encoded.len(), 32);

    let decoded = decode_image(GxTexFmt::RGB565, 4, 4, &encoded, None).expect("decode");
    assert_eq!(
        decoded, expected,
        "RGB565 decode of encoder output must equal the 5/6/5-snapped source"
    );
}

#[test]
fn rgb5a3_uses_rgb555_for_opaque_pixels() {
    // All-opaque source → encoder picks RGB555 form, decoder recovers
    // 5-bit channels.  Round-trip of pre-snapped values is byte-equal.
    let src = pattern_4x4();
    let mut expected = src.clone();
    for px in expected.chunks_mut(4) {
        px[0] = quant_via(px[0], 31, DequantMode::Smooth);
        px[1] = quant_via(px[1], 31, DequantMode::Smooth);
        px[2] = quant_via(px[2], 31, DequantMode::Smooth);
        px[3] = 255;
    }
    let encoded = encode_image(GxTexFmt::RGB5A3, 4, 4, &src).expect("encode");
    let decoded = decode_image(GxTexFmt::RGB5A3, 4, 4, &encoded, None).expect("decode");
    assert_eq!(decoded, expected);
}

#[test]
fn rgb5a3_uses_rgb4a3_for_translucent_pixels() {
    // Translucent source → encoder picks RGB4A3 form: 3-bit alpha,
    // 4-bit RGB channels.  Quantized expected mirrors that.
    let mut src = pattern_4x4();
    for px in src.chunks_mut(4) {
        px[3] = 0x80; // 50% alpha
    }
    let mut expected = src.clone();
    for px in expected.chunks_mut(4) {
        px[0] = quant_via(px[0], 15, DequantMode::Smooth);
        px[1] = quant_via(px[1], 15, DequantMode::Smooth);
        px[2] = quant_via(px[2], 15, DequantMode::Smooth);
        px[3] = quant_via(px[3], 7, DequantMode::Smooth);
    }
    let encoded = encode_image(GxTexFmt::RGB5A3, 4, 4, &src).expect("encode");
    let decoded = decode_image(GxTexFmt::RGB5A3, 4, 4, &encoded, None).expect("decode");
    assert_eq!(decoded, expected);
}

#[test]
fn rgb5a3_per_pixel_alpha_branch_is_chosen_independently() {
    // First two rows opaque (RGB555), last two rows translucent (RGB4A3)
    // — the per-pixel branch selector must produce the correct form for
    // each pixel independently.  We verify by decoding and comparing to
    // a hand-snapped expected.
    let mut src = pattern_4x4();
    for (i, px) in src.chunks_mut(4).enumerate() {
        let row = i / 4;
        px[3] = if row < 2 { 0xFF } else { 0x40 };
    }
    let mut expected = src.clone();
    for (i, px) in expected.chunks_mut(4).enumerate() {
        let row = i / 4;
        if row < 2 {
            px[0] = quant_via(px[0], 31, DequantMode::Smooth);
            px[1] = quant_via(px[1], 31, DequantMode::Smooth);
            px[2] = quant_via(px[2], 31, DequantMode::Smooth);
            px[3] = 255;
        } else {
            px[0] = quant_via(px[0], 15, DequantMode::Smooth);
            px[1] = quant_via(px[1], 15, DequantMode::Smooth);
            px[2] = quant_via(px[2], 15, DequantMode::Smooth);
            px[3] = quant_via(px[3], 7, DequantMode::Smooth);
        }
    }
    let encoded = encode_image(GxTexFmt::RGB5A3, 4, 4, &src).expect("encode");
    let decoded = decode_image(GxTexFmt::RGB5A3, 4, 4, &encoded, None).expect("decode");
    assert_eq!(decoded, expected);
}

#[test]
fn cmp_round_trips_within_rms_threshold_8x8() {
    // 8x8 = 1 super-block (4 BC1 blocks).  Use a single-channel smooth
    // ramp so each 4x4 block has 4 collinear colors in RGB space — BC1
    // (4 endpoints lerped along the c0-c1 line) handles this near-
    // optimally.  RMS bound below is generous to absorb 5/6/5
    // quantization at the endpoints; for natural images the encoder
    // does much better.
    let mut rgba = Vec::with_capacity(8 * 8 * 4);
    for y in 0..8u8 {
        for x in 0..8u8 {
            let t = ((y as u32 * 8 + x as u32) * 255 / 63) as u8;
            rgba.extend_from_slice(&[t, t, t, 0xFF]);
        }
    }
    let encoded = encode_image(GxTexFmt::CMP, 8, 8, &rgba).expect("encode CMP");
    // 8x8 CMP = 8*8/2 = 32 bytes (4 4x4 blocks × 8 bytes).
    assert_eq!(encoded.len(), image_size(GxTexFmt::CMP, 8, 8));
    assert_eq!(encoded.len(), 32);

    let decoded = decode_image(GxTexFmt::CMP, 8, 8, &encoded, None).expect("decode");
    let rms = channel_rms(&rgba, &decoded);
    assert!(
        rms < 8.0,
        "CMP grayscale ramp round-trip RMS = {} (threshold 8.0)",
        rms
    );
}

#[test]
fn cmp_super_block_swizzle_addresses_correctly_for_16x16() {
    // 16x16 = 4 super-blocks (2x2 layout).  We build a pattern where
    // each super-block has a distinct constant color: that way any bug
    // in the swizzle would show up as a "color from wrong super-block"
    // mismatch, not a fuzzy rounding error.
    let colors: [[u8; 4]; 4] = [
        [0xFF, 0x00, 0x00, 0xFF], // top-left = red
        [0x00, 0xFF, 0x00, 0xFF], // top-right = green
        [0x00, 0x00, 0xFF, 0xFF], // bottom-left = blue
        [0xFF, 0xFF, 0x00, 0xFF], // bottom-right = yellow
    ];
    let mut rgba = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16u32 {
        for x in 0..16u32 {
            let sb = ((y / 8) * 2 + (x / 8)) as usize;
            rgba.extend_from_slice(&colors[sb]);
        }
    }
    let encoded = encode_image(GxTexFmt::CMP, 16, 16, &rgba).expect("encode");
    assert_eq!(encoded.len(), image_size(GxTexFmt::CMP, 16, 16));
    let decoded = decode_image(GxTexFmt::CMP, 16, 16, &encoded, None).expect("decode");

    // Each super-block region in the decoded output must carry that
    // super-block's color (modulo BC1 quantization to RGB565 — pick
    // colors that survive cleanly: pure primary channels at 0xFF round
    // to 0xFF after 5/6-bit snap).
    for sb_y in 0..2 {
        for sb_x in 0..2 {
            let sb = (sb_y * 2 + sb_x) as usize;
            let expected = colors[sb];
            // Spot-check the center pixel of each super-block.
            let cx = (sb_x * 8 + 4) as usize;
            let cy = (sb_y * 8 + 4) as usize;
            let p = (cy * 16 + cx) * 4;
            // Channels round to 5/6/5 then back to 8 — primary 0xFF / 0
            // map exactly, so we can assert byte-equal here.
            assert_eq!(
                &decoded[p..p + 4],
                &expected,
                "super-block ({},{}) center pixel must keep its solid color",
                sb_x,
                sb_y
            );
        }
    }
}

#[test]
fn rgb5a3_swap_rb_option_is_inverse_of_post_decode_swap() {
    // With swap_rb_for_rgb5a3 enabled, the encoder pre-swaps R↔B in
    // the source.  Decoding the result with the un-swapped decoder
    // gives BGRA-ordered output relative to the input — so post-
    // swapping R↔B in the decoded buffer must recover the same RGBA
    // values the un-swapped encoder→decoder pair produces (i.e. the
    // 5/5/5-snapped originals from `rgb5a3_uses_rgb555_for_opaque_pixels`).
    let src = pattern_4x4();
    let opts = EncodeOptions { swap_rb_for_rgb5a3: true };
    let enc_swap = encode_image_with_options(GxTexFmt::RGB5A3, 4, 4, &src, opts).unwrap();
    let dec_swap = decode_image(GxTexFmt::RGB5A3, 4, 4, &enc_swap, None).unwrap();

    // Post-swap R↔B in the decoded buffer.
    let mut unswapped = dec_swap.clone();
    for px in unswapped.chunks_mut(4) {
        px.swap(0, 2);
    }

    // Un-swapped encode → decode of the same source.
    let enc_plain = encode_image(GxTexFmt::RGB5A3, 4, 4, &src).unwrap();
    let dec_plain = decode_image(GxTexFmt::RGB5A3, 4, 4, &enc_plain, None).unwrap();

    assert_eq!(
        unswapped, dec_plain,
        "swap_rb_for_rgb5a3 + post-decode R↔B swap must recover the un-swapped pipeline output"
    );
}

#[test]
fn encode_with_options_default_matches_plain_encode() {
    // The thin wrapper [`encode_image`] = [`encode_image_with_options`]
    // with `EncodeOptions::default()`.  Pin that contract.
    let src = pattern_4x4();
    for fmt in [
        GxTexFmt::RGBA8,
        GxTexFmt::RGB565,
        GxTexFmt::RGB5A3,
    ] {
        let plain = encode_image(fmt, 4, 4, &src).unwrap();
        let with_opts = encode_image_with_options(
            fmt,
            4,
            4,
            &src,
            EncodeOptions::default(),
        )
        .unwrap();
        assert_eq!(plain, with_opts, "{:?} default options divergence", fmt);
    }
}

#[test]
fn swap_rb_option_is_a_noop_for_non_rgb5a3() {
    // Only RGB5A3 reads the swap_rb flag.  Opt-in must not perturb
    // RGBA8 / RGB565 / CMP outputs.
    let src = pattern_4x4();
    let opts = EncodeOptions { swap_rb_for_rgb5a3: true };
    for fmt in [GxTexFmt::RGBA8, GxTexFmt::RGB565] {
        let plain = encode_image(fmt, 4, 4, &src).unwrap();
        let with_swap = encode_image_with_options(fmt, 4, 4, &src, opts).unwrap();
        assert_eq!(
            plain, with_swap,
            "{:?} encoder must ignore swap_rb_for_rgb5a3",
            fmt
        );
    }

    // CMP: source needs to be 8x8 to match the format's tile bound.
    let mut cmp_src = Vec::with_capacity(8 * 8 * 4);
    for i in 0..64u8 {
        cmp_src.extend_from_slice(&[i.wrapping_mul(3), i, i.wrapping_mul(7), 0xFF]);
    }
    let plain = encode_image(GxTexFmt::CMP, 8, 8, &cmp_src).unwrap();
    let with_swap = encode_image_with_options(GxTexFmt::CMP, 8, 8, &cmp_src, opts).unwrap();
    assert_eq!(plain, with_swap, "CMP encoder must ignore swap_rb_for_rgb5a3");
}

#[test]
fn encode_rejects_unsupported_formats() {
    let rgba = vec![0u8; 4 * 4 * 4];
    for fmt in [
        GxTexFmt::I4,
        GxTexFmt::I8,
        GxTexFmt::IA4,
        GxTexFmt::IA8,
        GxTexFmt::CI4,
        GxTexFmt::CI8,
        GxTexFmt::CI14X2,
    ] {
        assert!(
            encode_image(fmt, 4, 4, &rgba).is_err(),
            "format {:?} must reject encode (unsupported)",
            fmt
        );
    }
}

#[test]
fn encode_rejects_size_mismatch() {
    let rgba = vec![0u8; 16]; // 4 pixels = 4 bytes per pixel × 4 = 16, but for 4x4 we need 64
    assert!(encode_image(GxTexFmt::RGBA8, 4, 4, &rgba).is_err());
}

/// Per-channel RMS error between two RGBA8 buffers (assumes equal length).
fn channel_rms(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let mut sum_sq: f64 = 0.0;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let d = x as f64 - y as f64;
        sum_sq += d * d;
    }
    (sum_sq / a.len() as f64).sqrt()
}
