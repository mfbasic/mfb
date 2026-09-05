//! `__canvas_pngDecode` — a PNG reader, in MFBASIC, on top of `__canvas_inflate`.
//!
//! Decodes to RGBA8, which is the one format `canvas::createImage` takes, so the
//! decoder's whole job is "whatever the file says, produce those bytes".
//!
//! **All five colour types, all six bit depths, interlaced or not.** Not because a
//! program is likely to hand us a 4-bit interlaced palette image, but because refusing
//! one would be refusing a *legal PNG* — and the difference between "this build does not
//! read that" and "that file is broken" is exactly what `ErrBadImageFile` is supposed to
//! tell a caller. A decoder that silently covers only the common half makes that error
//! message a lie.
//!
//! What it does not do: APNG (an animation is not an `Image`), and `gAMA`/`iCCP` colour
//! management (the pixels are delivered as stored — a renderer that later grows a colour
//! pipeline should apply it there, not have it baked in here by a decoder).
//!
//! Registered via `add_helper`; body byte-significant (2-space indent → `.ncode`
//! columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Signature and chunk walk.
///
/// `IDAT` is gathered across chunks before inflating: the spec allows a stream to be
/// split at any byte boundary, so concatenating first is not an optimisation but the
/// only correct reading. `PLTE` and `tRNS` are kept whole and interpreted by the pixel
/// conversion, which is where the colour type decides what they mean.
///
/// `__canvas_pngSlice` is for a *fresh* slice — a pass block, a row. It is not how the
/// chunk walk accumulates: passing a growing list through it copies the list per call
/// (bug-509, DEC-52), so `__canvas_pngDecode` appends to its accumulators itself.
#[rustfmt::skip]
const PNG_CHUNKS: &str =
r#"FUNC __canvas_isPng(bytes AS List OF Byte) AS Boolean
  IF len(bytes) < 8 THEN
    RETURN FALSE
  END IF
  LET signature AS List OF Integer = [137, 80, 78, 71, 13, 10, 26, 10]
  MUT i AS Integer = 0
  WHILE i < 8
    IF toInt(collections::getOr(bytes, i, toByte(0))) <> collections::getOr(signature, i, 0 - 1) THEN
      RETURN FALSE
    END IF
    i = i + 1
  END WHILE
  RETURN TRUE
END FUNC

FUNC __canvas_pngChunkIs(bytes AS List OF Byte, at AS Integer, tag AS List OF Integer) AS Boolean
  MUT i AS Integer = 0
  WHILE i < 4
    IF toInt(collections::getOr(bytes, at + i, toByte(0))) <> collections::getOr(tag, i, 0 - 1) THEN
      RETURN FALSE
    END IF
    i = i + 1
  END WHILE
  RETURN TRUE
END FUNC

FUNC __canvas_pngSlice(bytes AS List OF Byte, at AS Integer, count AS Integer, into AS List OF Byte) AS List OF Byte
  MUT out AS List OF Byte = into
  MUT i AS Integer = 0
  WHILE i < count
    out = collections::append(out, collections::getOr(bytes, at + i, toByte(0)))
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

/// The scanline defilter of RFC 2083 §6, which is also the step a wrong `bpp` corrupts
/// most quietly.
///
/// `bpp` is the number of **bytes** per pixel rounded up (1 for anything under 8 bits a
/// pixel), and it is the distance the Sub/Average/Paeth filters look back. Every read of
/// a neighbouring byte goes through `getOr` with a zero default, which is exactly the
/// specification's rule for the first row and the first pixel — so the edge cases need no
/// branch of their own.
#[rustfmt::skip]
const PNG_UNFILTER: &str =
r#"FUNC __canvas_paeth(a AS Integer, b AS Integer, c AS Integer) AS Integer
  LET p AS Integer = a + b - c
  LET pa AS Integer = __canvas_absI(p - a)
  LET pb AS Integer = __canvas_absI(p - b)
  LET pc AS Integer = __canvas_absI(p - c)
  IF pa <= pb AND pa <= pc THEN
    RETURN a
  END IF
  IF pb <= pc THEN
    RETURN b
  END IF
  RETURN c
END FUNC

FUNC __canvas_absI(v AS Integer) AS Integer
  IF v < 0 THEN
    RETURN 0 - v
  END IF
  RETURN v
END FUNC

FUNC __canvas_pngUnfilter(raw AS List OF Byte, width AS Integer, height AS Integer, bpp AS Integer, stride AS Integer) AS List OF Byte
  MUT out AS List OF Byte = []
  MUT row AS Integer = 0
  WHILE row < height
    LET at AS Integer = row * (stride + 1)
    LET filter AS Integer = toInt(collections::getOr(raw, at, toByte(0)))
    LET base AS Integer = row * stride
    MUT i AS Integer = 0
    WHILE i < stride
      LET x AS Integer = toInt(collections::getOr(raw, at + 1 + i, toByte(0)))
      MUT a AS Integer = 0
      IF i >= bpp THEN
        a = toInt(collections::getOr(out, base + i - bpp, toByte(0)))
      END IF
      MUT b AS Integer = 0
      IF row > 0 THEN
        b = toInt(collections::getOr(out, base - stride + i, toByte(0)))
      END IF
      MUT c AS Integer = 0
      IF row > 0 AND i >= bpp THEN
        c = toInt(collections::getOr(out, base - stride + i - bpp, toByte(0)))
      END IF
      MUT value AS Integer = x
      IF filter = 1 THEN
        value = x + a
      ELSEIF filter = 2 THEN
        value = x + b
      ELSEIF filter = 3 THEN
        value = x + (a + b) / 2
      ELSEIF filter = 4 THEN
        value = x + __canvas_paeth(a, b, c)
      END IF
      out = collections::append(out, toByte(value MOD 256))
      i = i + 1
    END WHILE
    row = row + 1
  END WHILE
  RETURN out
END FUNC"#;

/// Sample extraction and the conversion to RGBA8.
///
/// A sample is read by bit offset rather than by byte, which is what lets one routine
/// serve depths 1, 2, 4, 8 and 16. Sub-8-bit samples are **scaled**, not shifted into the
/// low bits: a 1-bit greyscale 1 is white, not 1/255th of white, and the standard's
/// scaling (`value * 255 / max`) is what makes that true for every depth at once.
///
/// 16-bit samples are reduced to 8 by taking the high byte. That is lossy and it is the
/// right loss: the destination is RGBA8, and rounding would introduce a difference
/// between this decoder and every other one for no visible gain.
#[rustfmt::skip]
const PNG_SAMPLES: &str =
r#"FUNC __canvas_pngSample(row AS List OF Byte, index AS Integer, depth AS Integer) AS Integer
  IF depth = 8 THEN
    RETURN toInt(collections::getOr(row, index, toByte(0)))
  END IF
  IF depth = 16 THEN
    RETURN toInt(collections::getOr(row, index * 2, toByte(0)))
  END IF
  LET perByte AS Integer = 8 / depth
  LET byte AS Integer = toInt(collections::getOr(row, index / perByte, toByte(0)))
  LET shift AS Integer = 8 - depth * ((index MOD perByte) + 1)
  RETURN (byte / __canvas_pow2(shift)) MOD __canvas_pow2(depth)
END FUNC

FUNC __canvas_pngScale(value AS Integer, depth AS Integer) AS Integer
  IF depth >= 8 THEN
    RETURN value
  END IF
  RETURN value * 255 / (__canvas_pow2(depth) - 1)
END FUNC

FUNC __canvas_pngChannels(colour AS Integer) AS Integer
  IF colour = 0 THEN
    RETURN 1
  END IF
  IF colour = 2 THEN
    RETURN 3
  END IF
  IF colour = 3 THEN
    RETURN 1
  END IF
  IF colour = 4 THEN
    RETURN 2
  END IF
  RETURN 4
END FUNC

FUNC __canvas_pngPixel(row AS List OF Byte, x AS Integer, colour AS Integer, depth AS Integer, palette AS List OF Byte, alphas AS List OF Byte) AS List OF Integer
  LET channels AS Integer = __canvas_pngChannels(colour)
  LET at AS Integer = x * channels
  IF colour = 0 THEN
    LET g AS Integer = __canvas_pngScale(__canvas_pngSample(row, at, depth), depth)
    RETURN [g, g, g, 255]
  END IF
  IF colour = 2 THEN
    RETURN [__canvas_pngScale(__canvas_pngSample(row, at, depth), depth), __canvas_pngScale(__canvas_pngSample(row, at + 1, depth), depth), __canvas_pngScale(__canvas_pngSample(row, at + 2, depth), depth), 255]
  END IF
  IF colour = 3 THEN
    LET index AS Integer = __canvas_pngSample(row, at, depth)
    ' A palette index is NOT scaled: it names an entry, it is not a brightness. An
    ' index past the end of PLTE is a malformed file, but the decode is already past
    ' the point where it can say so, so it reads as transparent black rather than as
    ' whatever byte happens to follow the palette.
    IF index * 3 + 2 >= len(palette) THEN
      RETURN [0, 0, 0, 0]
    END IF
    MUT alpha AS Integer = 255
    IF index < len(alphas) THEN
      alpha = toInt(collections::getOr(alphas, index, toByte(255)))
    END IF
    RETURN [toInt(collections::getOr(palette, index * 3, toByte(0))), toInt(collections::getOr(palette, index * 3 + 1, toByte(0))), toInt(collections::getOr(palette, index * 3 + 2, toByte(0))), alpha]
  END IF
  IF colour = 4 THEN
    LET g AS Integer = __canvas_pngScale(__canvas_pngSample(row, at, depth), depth)
    RETURN [g, g, g, __canvas_pngScale(__canvas_pngSample(row, at + 1, depth), depth)]
  END IF
  RETURN [__canvas_pngScale(__canvas_pngSample(row, at, depth), depth), __canvas_pngScale(__canvas_pngSample(row, at + 1, depth), depth), __canvas_pngScale(__canvas_pngSample(row, at + 2, depth), depth), __canvas_pngScale(__canvas_pngSample(row, at + 3, depth), depth)]
END FUNC"#;

/// Adam7, and the pass geometry it needs.
///
/// Interlacing is not a variant of the layout — it is seven complete sub-images, each
/// filtered independently with its own stride. Handling it by scattering each pass into
/// the final buffer is why `__canvas_pngPass` exists at all: the alternative, special
/// casing the unfiltered walk, would give the interlaced path its own copy of the pixel
/// conversion and therefore its own opportunity to disagree with the other one.
#[rustfmt::skip]
const PNG_INTERLACE: &str =
r#"FUNC __canvas_adam7XOrigin(pass AS Integer) AS Integer
  RETURN collections::getOr([0, 4, 0, 2, 0, 1, 0], pass, 0)
END FUNC

FUNC __canvas_adam7YOrigin(pass AS Integer) AS Integer
  RETURN collections::getOr([0, 0, 4, 0, 2, 0, 1], pass, 0)
END FUNC

FUNC __canvas_adam7XStep(pass AS Integer) AS Integer
  RETURN collections::getOr([8, 8, 4, 4, 2, 2, 1], pass, 1)
END FUNC

FUNC __canvas_adam7YStep(pass AS Integer) AS Integer
  RETURN collections::getOr([8, 8, 8, 4, 4, 2, 2], pass, 1)
END FUNC

FUNC __canvas_adam7Count(total AS Integer, origin AS Integer, interval AS Integer) AS Integer
  ' `interval`, not `step`: `step` is a reserved word and a parameter cannot be named
  ' one -- the parser says only "parameter name must be an identifier".
  IF total <= origin THEN
    RETURN 0
  END IF
  RETURN (total - origin + interval - 1) / interval
END FUNC"#;

/// The decode itself: header, chunk walk, inflate, then one pass (or seven).
///
/// `__canvas_pngDecode` returns RGBA8 pixels and `__canvas_pngSize` the dimensions, as
/// two calls rather than one packed result: a record for a purely internal answer would
/// have to be declared in the package's public type surface, and packing the dimensions
/// into the byte list would have capped an image at whatever the packing allowed. An
/// empty list from either means "not a PNG this decodes", and `canvas::loadImage` — the
/// only caller — turns that into `ErrBadImageFile`.
#[rustfmt::skip]
const PNG_DECODE: &str =
r#"FUNC __canvas_pngPass(pixels AS List OF Byte, header AS List OF Integer, raw AS List OF Byte, rawAt AS Integer, palette AS List OF Byte, alphas AS List OF Byte, xOrigin AS Integer, yOrigin AS Integer, xStep AS Integer, yStep AS Integer, passWidth AS Integer, passHeight AS Integer) AS List OF Byte
  LET width AS Integer = collections::getOr(header, 0, 0)
  LET depth AS Integer = collections::getOr(header, 2, 8)
  LET colour AS Integer = collections::getOr(header, 3, 0)
  LET channels AS Integer = __canvas_pngChannels(colour)
  LET bitsPerPixel AS Integer = channels * depth
  LET stride AS Integer = (passWidth * bitsPerPixel + 7) / 8
  MUT bpp AS Integer = bitsPerPixel / 8
  IF bpp < 1 THEN
    bpp = 1
  END IF

  MUT block AS List OF Byte = []
  block = __canvas_pngSlice(raw, rawAt, passHeight * (stride + 1), block)
  LET flat AS List OF Byte = __canvas_pngUnfilter(block, passWidth, passHeight, bpp, stride)

  MUT out AS List OF Byte = pixels
  MUT row AS Integer = 0
  WHILE row < passHeight
    MUT line AS List OF Byte = []
    line = __canvas_pngSlice(flat, row * stride, stride, line)
    MUT col AS Integer = 0
    WHILE col < passWidth
      LET rgba AS List OF Integer = __canvas_pngPixel(line, col, colour, depth, palette, alphas)
      LET at AS Integer = ((yOrigin + row * yStep) * width + xOrigin + col * xStep) * 4
      out = collections::set(out, at, toByte(collections::getOr(rgba, 0, 0)))
      out = collections::set(out, at + 1, toByte(collections::getOr(rgba, 1, 0)))
      out = collections::set(out, at + 2, toByte(collections::getOr(rgba, 2, 0)))
      out = collections::set(out, at + 3, toByte(collections::getOr(rgba, 3, 255)))
      col = col + 1
    END WHILE
    row = row + 1
  END WHILE
  RETURN out
END FUNC

FUNC __canvas_pngPassBytes(header AS List OF Integer, passWidth AS Integer, passHeight AS Integer) AS Integer
  IF passWidth <= 0 OR passHeight <= 0 THEN
    RETURN 0
  END IF
  LET depth AS Integer = collections::getOr(header, 2, 8)
  LET colour AS Integer = collections::getOr(header, 3, 0)
  LET bitsPerPixel AS Integer = __canvas_pngChannels(colour) * depth
  RETURN passHeight * ((passWidth * bitsPerPixel + 7) / 8 + 1)
END FUNC

FUNC __canvas_pngRawBytes(header AS List OF Integer) AS Integer
  ' The filtered rows the header commits the stream to -- a number a PNG fixes
  ' exactly, which is what makes the inflate cap below a cap and not a guess.
  LET width AS Integer = collections::getOr(header, 0, 0)
  LET height AS Integer = collections::getOr(header, 1, 0)
  IF collections::getOr(header, 4, 0) = 0 THEN
    RETURN __canvas_pngPassBytes(header, width, height)
  END IF
  MUT total AS Integer = 0
  MUT pass AS Integer = 0
  WHILE pass < 7
    LET passWidth AS Integer = __canvas_adam7Count(width, __canvas_adam7XOrigin(pass), __canvas_adam7XStep(pass))
    LET passHeight AS Integer = __canvas_adam7Count(height, __canvas_adam7YOrigin(pass), __canvas_adam7YStep(pass))
    total = total + __canvas_pngPassBytes(header, passWidth, passHeight)
    pass = pass + 1
  END WHILE
  RETURN total
END FUNC

FUNC __canvas_pngHeader(bytes AS List OF Byte, at AS Integer) AS List OF Integer
  LET width AS Integer = __canvas_beU32(bytes, at)
  LET height AS Integer = __canvas_beU32(bytes, at + 4)
  LET depth AS Integer = toInt(collections::getOr(bytes, at + 8, toByte(0)))
  LET colour AS Integer = toInt(collections::getOr(bytes, at + 9, toByte(0)))
  LET compression AS Integer = toInt(collections::getOr(bytes, at + 10, toByte(0)))
  LET filter AS Integer = toInt(collections::getOr(bytes, at + 11, toByte(0)))
  LET interlace AS Integer = toInt(collections::getOr(bytes, at + 12, toByte(0)))
  IF width <= 0 OR height <= 0 THEN
    RETURN []
  END IF
  ' What a header may claim, checked before anything is sized from it (bug-509).
  ' 16384 a side is the largest texture the GPU backends upload, and 2^24 pixels --
  ' 4096x4096, 64 MiB decoded -- is far past any asset a canvas program draws. Without
  ' the caps an 80-byte file naming 40000x40000 cost the decode 6.4 GB before it read
  ' a byte of pixel data. The sides are checked first: the product of two raw u32s
  ' overflows an Integer.
  IF width > 16384 OR height > 16384 THEN
    RETURN []
  END IF
  IF width * height > 16777216 THEN
    RETURN []
  END IF
  IF compression <> 0 OR filter <> 0 OR interlace > 1 THEN
    RETURN []
  END IF
  ' The legal (colour type, bit depth) pairs of RFC 2083 Table 11.1, spelled as the
  ' table does. A pair outside it is a malformed file, not a variant to guess at:
  ' 4-bit truecolour, for instance, has no defined meaning to fall back to.
  IF colour = 0 THEN
    IF depth <> 1 AND depth <> 2 AND depth <> 4 AND depth <> 8 AND depth <> 16 THEN
      RETURN []
    END IF
  ELSEIF colour = 3 THEN
    IF depth <> 1 AND depth <> 2 AND depth <> 4 AND depth <> 8 THEN
      RETURN []
    END IF
  ELSEIF colour = 2 OR colour = 4 OR colour = 6 THEN
    IF depth <> 8 AND depth <> 16 THEN
      RETURN []
    END IF
  ELSE
    RETURN []
  END IF
  RETURN [width, height, depth, colour, interlace]
END FUNC

FUNC __canvas_pngDecode(bytes AS List OF Byte) AS List OF Byte
  IF NOT __canvas_isPng(bytes) THEN
    RETURN []
  END IF
  MUT header AS List OF Integer = []
  MUT palette AS List OF Byte = []
  MUT alphas AS List OF Byte = []
  MUT idat AS List OF Byte = []
  MUT at AS Integer = 8
  LET total AS Integer = len(bytes)
  MUT seenEnd AS Boolean = FALSE
  WHILE at + 8 <= total AND NOT seenEnd
    LET length AS Integer = __canvas_beU32(bytes, at)
    LET body AS Integer = at + 8
    IF length < 0 OR body + length + 4 > total THEN
      RETURN []
    END IF
    IF __canvas_pngChunkIs(bytes, at + 4, [73, 72, 68, 82]) THEN
      header = __canvas_pngHeader(bytes, body)
      IF len(header) = 0 THEN
        RETURN []
      END IF
    ELSEIF __canvas_pngChunkIs(bytes, at + 4, [80, 76, 84, 69]) THEN
      ' The three accumulators grow IN PLACE, appended to by this function rather
      ' than through `__canvas_pngSlice`. That helper takes the list by value and
      ' returns it, so handing it the accumulator copied everything gathered so far
      ' once per chunk -- quadratic in the chunk count, with every intermediate copy
      ' left in the arena: 20,000 eight-byte IDATs cost 2.47 GB (bug-509, DEC-52).
      ' `append` mutates in place only for a local of the function doing the write.
      MUT p AS Integer = 0
      WHILE p < length
        palette = collections::append(palette, collections::getOr(bytes, body + p, toByte(0)))
        p = p + 1
      END WHILE
    ELSEIF __canvas_pngChunkIs(bytes, at + 4, [116, 82, 78, 83]) THEN
      MUT t AS Integer = 0
      WHILE t < length
        alphas = collections::append(alphas, collections::getOr(bytes, body + t, toByte(0)))
        t = t + 1
      END WHILE
    ELSEIF __canvas_pngChunkIs(bytes, at + 4, [73, 68, 65, 84]) THEN
      MUT d AS Integer = 0
      WHILE d < length
        idat = collections::append(idat, collections::getOr(bytes, body + d, toByte(0)))
        d = d + 1
      END WHILE
    ELSEIF __canvas_pngChunkIs(bytes, at + 4, [73, 69, 78, 68]) THEN
      seenEnd = TRUE
    END IF
    at = body + length + 4
  END WHILE
  IF len(header) = 0 OR len(idat) = 0 THEN
    RETURN []
  END IF

  LET width AS Integer = collections::getOr(header, 0, 0)
  LET height AS Integer = collections::getOr(header, 1, 0)
  LET colour AS Integer = collections::getOr(header, 3, 0)
  LET interlace AS Integer = collections::getOr(header, 4, 0)
  IF colour = 3 AND len(palette) = 0 THEN
    RETURN []
  END IF

  ' Three bounds hang off the row count the header fixes (bug-509). The ratio first:
  ' DEFLATE cannot expand its input past 1032:1 (a 258-byte match costs at least two
  ' bits), so a stream too short to ever produce the rows is refused before a byte of
  ' it is inflated -- an 80-byte file claiming 4000x4000 is refused here, in O(1),
  ' where it used to cost 4.98 GB of pixel buffer first. Then the inflate is told the
  ' count and refuses to pass it, so a 1x1 image whose IDAT would inflate to 400 MB
  ' is refused at its fifth byte instead of reported as a pixel. Last, the pixel
  ' buffer is allocated only once the rows are known to be there.
  LET expected AS Integer = __canvas_pngRawBytes(header)
  IF expected > len(idat) * 1032 THEN
    RETURN []
  END IF
  LET raw AS List OF Byte = __canvas_zlibInflate(idat, expected)
  IF len(raw) < expected THEN
    RETURN []
  END IF

  ' Opaque black, then overwritten. Pre-filling rather than appending is what lets the
  ' interlaced passes scatter into the buffer in any order, and it costs one pass over
  ' the pixels either way.
  MUT pixels AS List OF Byte = []
  MUT i AS Integer = 0
  LET count AS Integer = width * height * 4
  WHILE i < count
    pixels = collections::append(pixels, toByte(0))
    i = i + 1
  END WHILE

  IF interlace = 0 THEN
    IF __canvas_pngPassBytes(header, width, height) > len(raw) THEN
      RETURN []
    END IF
    pixels = __canvas_pngPass(pixels, header, raw, 0, palette, alphas, 0, 0, 1, 1, width, height)
  ELSE
    MUT rawAt AS Integer = 0
    MUT pass AS Integer = 0
    WHILE pass < 7
      LET passWidth AS Integer = __canvas_adam7Count(width, __canvas_adam7XOrigin(pass), __canvas_adam7XStep(pass))
      LET passHeight AS Integer = __canvas_adam7Count(height, __canvas_adam7YOrigin(pass), __canvas_adam7YStep(pass))
      LET passBytes AS Integer = __canvas_pngPassBytes(header, passWidth, passHeight)
      IF passBytes > 0 THEN
        IF rawAt + passBytes > len(raw) THEN
          RETURN []
        END IF
        pixels = __canvas_pngPass(pixels, header, raw, rawAt, palette, alphas, __canvas_adam7XOrigin(pass), __canvas_adam7YOrigin(pass), __canvas_adam7XStep(pass), __canvas_adam7YStep(pass), passWidth, passHeight)
        rawAt = rawAt + passBytes
      END IF
      pass = pass + 1
    END WHILE
  END IF

  RETURN pixels
END FUNC

FUNC __canvas_pngSize(bytes AS List OF Byte) AS List OF Integer
  ' IHDR is required to be the first chunk, so its body is at a fixed offset: 8 bytes
  ' of signature, 4 of length, 4 of type. Reading it separately costs one header parse
  ' and saves packing two dimensions into the byte list the pixels come back in --
  ' which would have capped an image at 65535 wide for no reason but the container.
  IF NOT __canvas_isPng(bytes) THEN
    RETURN []
  END IF
  IF NOT __canvas_pngChunkIs(bytes, 12, [73, 72, 68, 82]) THEN
    RETURN []
  END IF
  LET header AS List OF Integer = __canvas_pngHeader(bytes, 16)
  IF len(header) = 0 THEN
    RETURN []
  END IF
  RETURN [collections::getOr(header, 0, 0), collections::getOr(header, 1, 0)]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_pngChunks", PNG_CHUNKS));
    pkg.add_helper(RegistryHelper::always("canvas_pngUnfilter", PNG_UNFILTER));
    pkg.add_helper(RegistryHelper::always("canvas_pngSamples", PNG_SAMPLES));
    pkg.add_helper(RegistryHelper::always("canvas_pngInterlace", PNG_INTERLACE));
    pkg.add_helper(RegistryHelper::always("canvas_pngDecode", PNG_DECODE));
}
