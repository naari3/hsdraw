"""End-to-end Python smoke for ``hsdraw.gx_decode``.

Mirrors ``crates/hsdraw-core/tests/gx_encoder.rs``'s round-trip cases
at the PyO3 layer: encode a known RGBA8 source via ``gx_encode``,
decode it back via ``gx_decode``, and assert the RGBA8 output matches
either byte-for-byte (lossless RGBA8) or up to the format's
quantization grid (RGB565 / RGB5A3).  CMP is BC1-lossy so we fall back
to a per-channel RMS bound.

Run via the CI ``maturin develop smoke`` job after the wheel installs.
"""

import math
import hsdraw


def _solid_pattern_4x4() -> bytes:
    """4x4 RGBA8 with deterministic per-pixel values (alpha 0xFF)."""
    out = bytearray()
    for y in range(4):
        for x in range(4):
            out.extend([0x10 + x * 0x20, 0x10 + y * 0x20, 0x80, 0xFF])
    return bytes(out)


def _quant_shift(c8: int, n: int) -> int:
    """Round-half-up to N-bit then shift back to 8-bit (RGB565 path)."""
    levels = (1 << n) - 1
    q = (c8 * levels + 127) // 255
    return (q << (8 - n)) & 0xFF


def _quant_smooth(c8: int, levels: int) -> int:
    """Round-half-up to ``levels``-step then ``c_n * 255 / levels`` (RGB5A3 path)."""
    q = (c8 * levels + 127) // 255
    return (q * 255) // levels


def _channel_rms(a: bytes, b: bytes) -> float:
    assert len(a) == len(b)
    s = 0
    for x, y in zip(a, b):
        s += (x - y) * (x - y)
    return math.sqrt(s / len(a))


def main() -> None:
    print("version:", hsdraw.version())
    assert callable(hsdraw.gx_decode)

    src = _solid_pattern_4x4()

    # ---- RGBA8 (lossless) -------------------------------------------
    enc = hsdraw.gx_encode(6, 4, 4, src)              # 6 = RGBA8
    assert len(enc) == 64, f"RGBA8 encoded len = {len(enc)}"
    dec = hsdraw.gx_decode(6, 4, 4, enc)
    assert len(dec) == 64
    assert dec == src, "RGBA8 must round-trip byte-equal"

    # ---- RGB565 (lossy via 5/6/5 truncation) ------------------------
    enc = hsdraw.gx_encode(4, 4, 4, src)              # 4 = RGB565
    assert len(enc) == 32
    dec = hsdraw.gx_decode(4, 4, 4, enc)
    expected = bytearray()
    for i in range(0, len(src), 4):
        r, g, b, _a = src[i : i + 4]
        expected.extend(
            [_quant_shift(r, 5), _quant_shift(g, 6), _quant_shift(b, 5), 255]
        )
    assert dec == bytes(expected), "RGB565 round-trip must match 5/6/5-snapped expected"

    # ---- RGB5A3 RGB555 branch (alpha = 0xFF → 5/5/5 + alpha=255) ----
    enc = hsdraw.gx_encode(5, 4, 4, src)              # 5 = RGB5A3
    assert len(enc) == 32
    dec = hsdraw.gx_decode(5, 4, 4, enc)
    expected = bytearray()
    for i in range(0, len(src), 4):
        r, g, b, _a = src[i : i + 4]
        expected.extend(
            [
                _quant_smooth(r, 31),
                _quant_smooth(g, 31),
                _quant_smooth(b, 31),
                255,
            ]
        )
    assert dec == bytes(expected), "RGB5A3 (RGB555 branch) round-trip mismatch"

    # ---- RGB5A3 RGB4A3 branch (alpha = 0x80 → 4/4/4 + 3-bit alpha) --
    src_translucent = bytearray(src)
    for i in range(3, len(src_translucent), 4):
        src_translucent[i] = 0x80
    enc = hsdraw.gx_encode(5, 4, 4, bytes(src_translucent))
    dec = hsdraw.gx_decode(5, 4, 4, enc)
    expected = bytearray()
    for i in range(0, len(src_translucent), 4):
        r, g, b, a = src_translucent[i : i + 4]
        expected.extend(
            [
                _quant_smooth(r, 15),
                _quant_smooth(g, 15),
                _quant_smooth(b, 15),
                _quant_smooth(a, 7),
            ]
        )
    assert dec == bytes(expected), "RGB5A3 (RGB4A3 branch) round-trip mismatch"

    # ---- CMP (BC1-lossy, 8x8 super-block) ---------------------------
    # Use a single-channel grayscale ramp; one super-block (4 4x4
    # blocks) decodes within RMS < 8 — well under the BC1 noise floor.
    ramp = bytearray()
    for y in range(8):
        for x in range(8):
            t = (y * 8 + x) * 255 // 63
            ramp.extend([t, t, t, 0xFF])
    enc = hsdraw.gx_encode(14, 8, 8, bytes(ramp))     # 14 = CMP
    assert len(enc) == 32
    dec = hsdraw.gx_decode(14, 8, 8, enc)
    rms = _channel_rms(bytes(ramp), dec)
    assert rms < 8.0, f"CMP grayscale ramp RMS = {rms} (threshold 8.0)"

    # ---- empty palette path: kwargs default to (None, RGB5A3) -------
    # Non-paletted formats must ignore the palette arg.  Pass an
    # arbitrary bytes value to confirm it doesn't perturb decode.
    dec_with_palette = hsdraw.gx_decode(6, 4, 4, hsdraw.gx_encode(6, 4, 4, src), b"\x00" * 8, 2)
    assert dec_with_palette == src, "Palette arg must be ignored for non-paletted formats"

    # ---- size-too-short error path ----------------------------------
    try:
        hsdraw.gx_decode(6, 4, 4, b"\x00" * 4)        # 4 << 64
    except ValueError:
        pass
    else:
        raise AssertionError("decode of truncated payload must raise ValueError")

    # ---- shape sanity (the user's spec example) ---------------------
    out = hsdraw.gx_decode(6, 4, 4, bytes(64))
    assert len(out) == 64

    print("gx_decode round-trip: OK")


if __name__ == "__main__":
    main()
