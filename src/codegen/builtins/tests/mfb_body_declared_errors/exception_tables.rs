//! plan-176-H: the measured exception tables of the `mfb_body_declared_errors` sweeps
//! (`UNREACHABLE_VIA_CALLEE`, `UNREACHABLE_GENERAL_CALL`, `UNREACHABLE_QUALIFIED_CALL`,
//! `DECLARED_EXTRA`). Data only, so no `use` line.

/// A code this guard's own function-level (not per-overload, not
/// argument-sensitive) call propagation reports as reachable for `member`,
/// but which a human reading `member`'s own source has proven can never
/// actually fire — the `Body::Mfb` counterpart of
/// `declared_errors_sweep.rs`'s `unreachable_own_raise`/
/// `unreachable_via_callee` (same evidentiary bar: cite the exact loop
/// invariant or argument shape that makes the callee's check unreachable,
/// not "it seems unlikely"). Phase 2 adds an entry here instead of declaring
/// a code a member cannot actually raise, and instead of leaving a
/// permanently-red baseline entry for a false positive.
///
/// `astrings::toMarkdown` calls `astrings::getAttributes`/`strings::mid`
/// (via `__astrings_mdStateAt`) with an index `i` bounded by its own `WHILE i
/// < n` loop, where `n = astrings::scalarLen(a)` is the exact same bound
/// both callees range-check against (`getAttributes`'s `index < 0 OR index
/// >= n` and `mid`'s scalar-unit bound, confirmed matching units: both
/// `scalarLen` and `strings::mid` count Unicode scalars, not bytes or UTF-16
/// units) — so the index can never be out of range for either call.
///
/// **Keyed by `(package, member, code)`, never bare `member`** — this sweep
/// is whole-registry, and a bare member name collides across packages that
/// happen to share one (`datetime::parse`/`csv::parse`/`json::parse`): an
/// entry proven for one would silently also suppress a REAL gap on another
/// package's same-named member. Coordinator correction, 2026-10-03.
pub(super) const UNREACHABLE_VIA_CALLEE: &[(&str, &str, &str)] = &[
    ("astrings", "toMarkdown", "ErrIndexOutOfRange"),
    // audio (`render`'s ErrInvalidArgument row is gone: since bug-810 render
    // raises it itself, for a note past the ten-minute limit)
    ("audio", "play", "ErrFloatInf"),
    ("audio", "play", "ErrFloatNaN"),
    ("audio", "play", "ErrOverflow"),
    ("audio", "play", "ErrIndexOutOfRange"),
    ("audio", "play", "ErrNotFound"),
    // canvas (the loadImage ErrInvalidFormat/ErrTooLarge and loadFont
    // ErrInvalidFormat findings the canvas agent reported are TRAP-absorbed
    // via __canvas_pngDecode's/__canvas_faceName's inline `TRAP(err) RETURN
    // ... END TRAP` -- the TRAP-aware resolve() now excludes them on its own,
    // so they are deliberately NOT listed here, per the coordinator's "prune
    // redundant entries" instruction)
    ("canvas", "present", "ErrIndexOutOfRange"),
    ("canvas", "present", "ErrInvalidArgument"),
    ("canvas", "present", "ErrResourceClosed"),
    ("canvas", "presentLayers", "ErrInvalidArgument"),
    ("canvas", "loadImage", "ErrIndexOutOfRange"),
    ("canvas", "loadFont", "ErrIndexOutOfRange"),
    ("canvas", "listSystemFonts", "ErrIndexOutOfRange"),
    ("canvas", "listSystemFonts", "ErrInvalidFormat"),
    ("canvas", "loadSystemFont", "ErrIndexOutOfRange"),
    ("canvas", "loadSystemFont", "ErrInvalidFormat"),
    ("canvas", "measureText", "ErrIndexOutOfRange"),
    // plan-162-D: `caretStops` reaches the same `collections::set` calls `measureText`
    // does, through the shaped run and `__canvas_measureHandles`. Each writes an index
    // the code just bounded: `__canvas_shapeRun` writes `c * __CANVAS_RUN_STRIDE + col`
    // for `c < n` into the run it built with exactly `n` records; `__canvas_kernRun`
    // writes records `i` and `i + 1` under `WHILE i + 1 < n`; `__canvas_measureHandles`
    // writes `sums`/`used` at a run's `fontIndex`, which is always an index into the
    // `handles` list both were sized from. `caretStops` itself only appends.
    ("canvas", "caretStops", "ErrIndexOutOfRange"),
    // color
    ("color", "toPacked", "ErrInvalidArgument"),
    ("color", "fromPacked", "ErrInvalidArgument"),
    ("color", "fromHex", "ErrIndexOutOfRange"),
    ("color", "fromHex", "ErrNotFound"),
    ("color", "toHex", "ErrIndexOutOfRange"),
    ("color", "toHexAlpha", "ErrIndexOutOfRange"),
    ("color", "toHsl", "ErrInvalidArgument"),
    ("color", "toHsl", "ErrOverflow"),
    ("color", "saturate", "ErrInvalidArgument"),
    ("color", "saturate", "ErrOverflow"),
    ("color", "desaturate", "ErrInvalidArgument"),
    ("color", "desaturate", "ErrOverflow"),
    ("color", "rotateHue", "ErrInvalidArgument"),
    ("color", "rotateHue", "ErrOverflow"),
    ("color", "hsl", "ErrOverflow"),
    ("color", "hsla", "ErrOverflow"),
    ("color", "fromName", "ErrIndexOutOfRange"),
    ("color", "nameOf", "ErrIndexOutOfRange"),
    // csv
    ("csv", "parse", "ErrIndexOutOfRange"),
    ("csv", "parse", "ErrNotFound"),
    ("csv", "stringify", "ErrIndexOutOfRange"),
    ("csv", "stringify", "ErrNotFound"),
    ("csv", "stringify", "ErrInvalidArgument"),
    // bug-809: parse(String)'s only `fs::openData` call passes the literal mode
    // "r", one of the four the mode check accepts, so its ErrInvalidArgument
    // (an unknown mode) cannot fire.
    ("csv", "parse", "ErrInvalidArgument"),
    ("csv", "readRow", "ErrNotFound"),
    // readRow's reads are `__csv_scan`'s: every `collections::get(chars, i)` has
    // `i` in `0 .. count - 1` (the `WHILE index < count` loop,
    // `__csv_isDoubledQuote`/`__csv_separatorLength`'s own `index + 1 < count`
    // guards), `__csv_decodeRange` runs over `fieldStart .. index`, a sub-range of
    // the same bounds, and `count` is re-read with every new `chars`. Its
    // `collections::get(<rows>, 0)` reads a scan of a non-empty line, which yields
    // a record (a separator ends one; otherwise the end of input flushes the
    // started field) or FAILs first.
    ("csv", "readRow", "ErrIndexOutOfRange"),
    // crypto
    ("crypto", "uuid7", "ErrIndexOutOfRange"),
    ("crypto", "uuid7", "ErrNotFound"),
    ("crypto", "uuid7", "ErrInvalidArgument"),
    ("crypto", "ulid", "ErrNotFound"),
    ("crypto", "ulid", "ErrInvalidArgument"),
    // json
    ("json", "get", "ErrIndexOutOfRange"),
    ("json", "getOr", "ErrIndexOutOfRange"),
    ("json", "getOr", "ErrNotFound"),
    ("json", "parse", "ErrIndexOutOfRange"),
    ("json", "parse", "ErrNotFound"),
    ("json", "stringify", "ErrIndexOutOfRange"),
    ("json", "stringify", "ErrInvalidArgument"),
    ("json", "stringify", "ErrNotFound"),
    // regex
    ("regex", "find", "ErrInvalidArgument"),
    ("regex", "findAll", "ErrInvalidArgument"),
    ("regex", "match", "ErrInvalidArgument"),
    ("regex", "findMatch", "ErrInvalidArgument"),
    ("regex", "findAllMatches", "ErrInvalidArgument"),
    ("regex", "findAll", "ErrNotFound"),
    ("regex", "match", "ErrNotFound"),
    ("regex", "replace", "ErrNotFound"),
    ("regex", "findMatch", "ErrNotFound"),
    ("regex", "findAllMatches", "ErrNotFound"),
    ("regex", "count", "ErrNotFound"),
    ("regex", "split", "ErrNotFound"),
    ("regex", "match", "ErrIndexOutOfRange"),
    ("regex", "replace", "ErrIndexOutOfRange"),
    ("regex", "split", "ErrIndexOutOfRange"),
    // `__encoding_fromCodepoint` encodes a surrogate (55296 .. 57343) as three
    // bytes `toString` rejects with ErrEncoding (bug-807); these callers never
    // pass one. `utf16Decode` passes either a paired scalar (>= 65536) or a unit
    // it has checked is not a surrogate; `codepageDecode` passes only `n < 128`.
    // (csv's `parse`/`readRow` used to be listed here too; since bug-809 they read
    // a `File` through `fs::readLineRaw`, which really raises ErrEncoding for a
    // line that is not UTF-8, so they declare it.)
    ("encoding", "utf16Decode", "ErrEncoding"),
    ("encoding", "utf32Decode", "ErrEncoding"),
    ("encoding", "codepageDecode", "ErrEncoding"),
    // datetime
    // parseIso's own `__datetime_readOffset` reads every field through
    // `__datetime_peek`, which range-checks `start + count <= len(value)`
    // before ever calling `strings::mid` (helper_peek.rs's own doc comment:
    // "the ErrIndexOutOfRange a bare mid past the end would raise" is
    // exactly what the guard precedes) -- so the ErrIndexOutOfRange
    // `strings::mid` declares can never fire through this caller.
    ("datetime", "parseIso", "ErrIndexOutOfRange"),
    // parseIso calls `__datetime_fixedOffset1` (the ONE-argument overload of
    // `datetime::fixedOffset`, which alone declares only ErrInvalidArgument)
    // with a magnitude `readOffset` has already bounded under 24h (hours
    // 00-23, minutes/seconds 00-59) -- exactly fixedOffset1's own `<=
    // -86400 OR >= 86400` check, which can therefore never fail here. The
    // guard's function-level (not per-overload) union pulls in
    // ErrInvalidArgument/ErrOverflow from fixedOffset's UNRELATED
    // two-argument overload (`__datetime_fixedOffset2`), which parseIso
    // never calls.
    ("datetime", "parseIso", "ErrInvalidArgument"),
    ("datetime", "parseIso", "ErrOverflow"),
    ("datetime", "toUtc", "ErrInvalidArgument"),
    ("datetime", "toUtc", "ErrOverflow"),
    // format's only collections::get is `__datetime_monthName`'s
    // `names[month - 1]`, reached solely from `__datetime_formatToken`'s
    // month-name arm, which first FAILs ErrInvalidArgument for a `month`
    // outside 1 .. 12 -- so the index is always 0 .. 11 of a 12-element list.
    // (`__datetime_weekdayName`'s index is `__datetime_floorMod(_, 7)`, always
    // 0 .. 6 of a 7-element list.)
    ("datetime", "format", "ErrIndexOutOfRange"),
    ("datetime", "format", "ErrNotFound"),
    ("datetime", "parse", "ErrIndexOutOfRange"),
    ("datetime", "parse", "ErrNotFound"),
    ("datetime", "toIso", "ErrIndexOutOfRange"),
    ("datetime", "formatDuration", "ErrInvalidArgument"),
    // net
    ("net", "toUrl", "ErrIndexOutOfRange"),
    ("net", "toUrl", "ErrNotFound"),
    ("net", "parseQuery", "ErrIndexOutOfRange"),
    ("net", "parseQuery", "ErrInvalidArgument"),
    ("net", "parseQuery", "ErrNotFound"),
    // vector
    ("vector", "length", "ErrFloatDomain"),
    ("vector", "length", "ErrInvalidArgument"),
    ("vector", "distance", "ErrFloatDomain"),
    ("vector", "distance", "ErrInvalidArgument"),
    ("vector", "normalize", "ErrFloatDomain"),
    ("vector", "normalize", "ErrOverflow"),
    ("vector", "angle", "ErrFloatDomain"),
    ("vector", "angle", "ErrOverflow"),
    ("vector", "lerp", "ErrInvalidArgument"),
    ("vector", "slerp", "ErrFloatDomain"),
    ("vector", "slerp", "ErrFloatNaN"),
    ("vector", "clamp_length", "ErrFloatDomain"),
    ("vector", "clamp_length", "ErrOverflow"),
    ("vector", "min", "ErrInvalidArgument"),
    ("vector", "max", "ErrInvalidArgument"),
    ("vector", "rotate_2d", "ErrFloatNaN"),
    ("vector", "rotate_2d", "ErrOverflow"),
    // encoding (subset A)
    ("encoding", "utf16Encode", "ErrIndexOutOfRange"),
    ("encoding", "utf16Encode", "ErrInvalidArgument"),
    ("encoding", "utf16Encode", "ErrNotFound"),
    ("encoding", "utf16Decode", "ErrIndexOutOfRange"),
    ("encoding", "utf16Decode", "ErrInvalidArgument"),
    ("encoding", "utf16Decode", "ErrNotFound"),
    ("encoding", "utf32Encode", "ErrIndexOutOfRange"),
    ("encoding", "utf32Encode", "ErrNotFound"),
    ("encoding", "hexEncode", "ErrIndexOutOfRange"),
    ("encoding", "hexDecode", "ErrIndexOutOfRange"),
    ("encoding", "hexDecode", "ErrNotFound"),
    ("encoding", "base32Encode", "ErrIndexOutOfRange"),
    ("encoding", "base32Encode", "ErrInvalidArgument"),
    ("encoding", "base32Decode", "ErrIndexOutOfRange"),
    ("encoding", "base32Decode", "ErrInvalidArgument"),
    ("encoding", "base32Decode", "ErrNotFound"),
    ("encoding", "base64Encode", "ErrIndexOutOfRange"),
    ("encoding", "base64Encode", "ErrInvalidArgument"),
    ("encoding", "base64Decode", "ErrIndexOutOfRange"),
    ("encoding", "base64Decode", "ErrInvalidArgument"),
    ("encoding", "base64Decode", "ErrNotFound"),
    ("encoding", "base64UrlEncode", "ErrIndexOutOfRange"),
    ("encoding", "base64UrlEncode", "ErrInvalidArgument"),
    ("encoding", "base64UrlDecode", "ErrIndexOutOfRange"),
    ("encoding", "base64UrlDecode", "ErrInvalidArgument"),
    ("encoding", "base64UrlDecode", "ErrNotFound"),
    ("encoding", "percentEncode", "ErrIndexOutOfRange"),
    ("encoding", "percentDecode", "ErrIndexOutOfRange"),
    ("encoding", "percentDecode", "ErrNotFound"),
    // encoding (subset B)
    ("encoding", "htmlEscape", "ErrIndexOutOfRange"),
    ("encoding", "htmlUnescape", "ErrIndexOutOfRange"),
    ("encoding", "htmlUnescape", "ErrNotFound"),
    ("encoding", "formUrlEncode", "ErrIndexOutOfRange"),
    ("encoding", "formUrlDecode", "ErrIndexOutOfRange"),
    ("encoding", "formUrlDecode", "ErrNotFound"),
    ("encoding", "punycodeEncode", "ErrIndexOutOfRange"),
    ("encoding", "punycodeEncode", "ErrNotFound"),
    ("encoding", "punycodeEncode", "ErrInvalidArgument"),
    ("encoding", "punycodeDecode", "ErrIndexOutOfRange"),
    ("encoding", "punycodeDecode", "ErrNotFound"),
    ("encoding", "punycodeDecode", "ErrInvalidArgument"),
    ("encoding", "uleb128Encode", "ErrInvalidArgument"),
    ("encoding", "uleb128Decode", "ErrIndexOutOfRange"),
    ("encoding", "uleb128Decode", "ErrNotFound"),
    ("encoding", "uleb128Decode", "ErrInvalidArgument"),
    ("encoding", "sleb128Encode", "ErrInvalidArgument"),
    ("encoding", "sleb128Decode", "ErrIndexOutOfRange"),
    ("encoding", "sleb128Decode", "ErrNotFound"),
    ("encoding", "sleb128Decode", "ErrInvalidArgument"),
    ("encoding", "varintEncode", "ErrInvalidArgument"),
    ("encoding", "varintDecode", "ErrIndexOutOfRange"),
    ("encoding", "varintDecode", "ErrNotFound"),
    ("encoding", "varintDecode", "ErrInvalidArgument"),
    ("encoding", "codepageDecode", "ErrIndexOutOfRange"),
    ("encoding", "codepageDecode", "ErrNotFound"),
    ("encoding", "codepageEncode", "ErrIndexOutOfRange"),
    ("encoding", "codepageEncode", "ErrNotFound"),
    // collections (subset A)
    ("collections", "sort", "ErrIndexOutOfRange"),
    ("collections", "sort", "ErrNotFound"),
    ("collections", "sortBy", "ErrIndexOutOfRange"),
    ("collections", "sortBy", "ErrNotFound"),
    ("collections", "take", "ErrIndexOutOfRange"),
    ("collections", "take", "ErrNotFound"),
    ("collections", "drop", "ErrIndexOutOfRange"),
    ("collections", "drop", "ErrNotFound"),
    ("collections", "any", "ErrIndexOutOfRange"),
    ("collections", "any", "ErrNotFound"),
    ("collections", "all", "ErrIndexOutOfRange"),
    ("collections", "all", "ErrNotFound"),
    ("collections", "groupBy", "ErrIndexOutOfRange"),
    ("collections", "groupBy", "ErrNotFound"),
    ("collections", "mapValues", "ErrIndexOutOfRange"),
    ("collections", "flatten", "ErrIndexOutOfRange"),
    ("collections", "flatten", "ErrNotFound"),
    ("collections", "zip", "ErrIndexOutOfRange"),
    ("collections", "zip", "ErrNotFound"),
    // collections (subset B)
    ("collections", "chunks", "ErrIndexOutOfRange"),
    ("collections", "chunks", "ErrNotFound"),
    ("collections", "window", "ErrIndexOutOfRange"),
    ("collections", "window", "ErrNotFound"),
    ("collections", "distinct", "ErrIndexOutOfRange"),
    ("collections", "distinct", "ErrNotFound"),
    ("collections", "partition", "ErrIndexOutOfRange"),
    ("collections", "partition", "ErrNotFound"),
    ("collections", "merge", "ErrIndexOutOfRange"),
    // http
    ("http", "ok", "ErrIndexOutOfRange"),
    ("http", "status", "ErrIndexOutOfRange"),
    ("http", "json", "ErrIndexOutOfRange"),
    ("http", "withHeader", "ErrIndexOutOfRange"),
    ("http", "route", "ErrIndexOutOfRange"),
    ("http", "route", "ErrNotFound"),
    ("http", "respondFile", "ErrIndexOutOfRange"),
    ("http", "respondPath", "ErrIndexOutOfRange"),
    ("http", "respondPath", "ErrNotFound"),
];

/// A code a bare `general` call's overload can raise that no call of that
/// member in that one `FUNC` (a member body or a private helper) can actually
/// raise, keyed `(FUNC, general member, code)` and applied where the call is,
/// so every member that reaches the `FUNC` inherits the proof (bug-807). The
/// proof must cover every call of that member in the `FUNC`, from its
/// arguments' ranges; a proof that depends on the caller names the callers.
pub(super) const UNREACHABLE_GENERAL_CALL: &[(&str, &str, &str)] = &[
    // Every toByte argument is in 0 .. 255: `value` after the `< 0` / `> 255`
    // guards above it, and the literals 0 and 255.
    ("__color_clampByte", "toByte", "ErrOverflow"),
    // `255 - toInt(<Byte channel>)` is in 0 .. 255.
    ("__color_invert", "toByte", "ErrOverflow"),
    // `lo` starts at 0 and `hi` at 255; `lo = mid + 1` runs only while `lo < hi`
    // with `mid < hi`, so `lo` ends in 0 .. 255.
    ("__color_fromLinear", "toByte", "ErrOverflow"),
    // Every `digits` entry is a `__color_hexValue` result checked `>= 0`, so
    // 0 .. 15: `d * 17` and `d * 16 + e` are at most 255.
    ("__color_fromHex", "toByte", "ErrOverflow"),
    // `math::floor` of a Float is an Integer, and toFloat of an Integer cannot
    // fail.
    ("__color_hslToColor", "toFloat", "ErrInvalidFormat"),
    ("__color_hslToColor", "toFloat", "ErrOverflow"),
    ("__audio_mmlSynth", "toFloat", "ErrInvalidFormat"),
    ("__audio_mmlSynth", "toFloat", "ErrOverflow"),
    // `t` is `__color_clampFraction(amount)`, so 0.0 .. 1.0: `aa + toInt((ba -
    // aa) * t)` lies between the two Byte alphas, and the product fits an
    // Integer.
    ("__color_mix", "toByte", "ErrOverflow"),
    ("__color_mix", "toInt", "ErrOverflow"),
    // The channel helpers' only callers (`__color_brighten`, `__color_darken`,
    // `__color_mix`) pass `amount` through `__color_clampFraction` (0.0 .. 1.0),
    // and `color::toLinear` is in 0 .. 65535, so each product is at most 65535.
    ("__color_brightenChannel", "toInt", "ErrOverflow"),
    ("__color_darkenChannel", "toInt", "ErrOverflow"),
    ("__color_mixChannel", "toInt", "ErrOverflow"),
    // `c` is a `color::Color` from a PolyLine's `colors`: toInt of a Byte channel.
    ("__canvas_hashItem", "toInt", "ErrInvalidFormat"),
    ("__canvas_hashItem", "toInt", "ErrOverflow"),
    // Every `rgba` channel from `__canvas_pngPixel` is in 0 .. 255: an 8-bit
    // sample, the high byte of a 16-bit one, a sub-byte sample `MOD 2^depth`
    // scaled by `255 / (2^depth - 1)`, or a palette/tRNS byte — the depth is one
    // `__canvas_pngHeader` accepts (1, 2, 4, 8, 16), and the sole caller,
    // `__canvas_pngDecode`, passes that checked header.
    ("__canvas_pngPass", "toByte", "ErrOverflow"),
    // `x`, `a`, `b`, `c` are bytes and Paeth picks one of `a`/`b`/`c`, so `value`
    // is in 0 .. 510 and `value MOD 256` in 0 .. 255.
    ("__canvas_pngUnfilter", "toByte", "ErrOverflow"),
    // The sole caller, `__canvas_extractFace`, passes `cursor` (12 plus padded
    // table sizes) and `size` (`__canvas_beU32`, two unsigned 16-bit halves):
    // both are non-negative, so each `(v / k) MOD 256` is in 0 .. 255.
    ("__canvas_beBytes32", "toByte", "ErrOverflow"),
    // `token` is 1 .. 18 ASCII digits with no leading zero (checked above it):
    // a well-formed Integer below 10^18.
    ("__json_arrayIndex", "toInt", "ErrInvalidFormat"),
    ("__json_arrayIndex", "toInt", "ErrOverflow"),
    // The exponent text is the digits after `json::sciParts`'s `e` for a finite,
    // non-zero magnitude (`__json_requireFiniteNumberText` runs first): an
    // optional sign and at most three digits.
    ("__json_stringifyNumber", "toInt", "ErrInvalidFormat"),
    ("__json_stringifyNumber", "toInt", "ErrOverflow"),
    // `n > 0` (the `n <= 0` early return), so `math::sqrt(toFloat(n))` is below
    // 3.1e9 and fits an Integer.
    ("__vector_isqrtFloor", "toInt", "ErrOverflow"),
    // `value` is checked `>= 0` first and each branch bounds it, so every toByte
    // argument is in 0 .. 244 (a lead byte) or 128 .. 191 (a continuation).
    ("__encoding_fromCodepoint", "toByte", "ErrOverflow"),
    // The `List OF Integer` overload checks each `unit` is in 0 .. 255 before
    // its toByte, and both overloads call toString only after
    // `__encoding_utf8Valid` accepted the bytes.
    ("__encoding_utf8Decode", "toByte", "ErrOverflow"),
    ("__encoding_utf8Decode", "toString", "ErrEncoding"),
    // Every caller passes ASCII: `__encoding_formUrlEncode`/`percentEncode` an
    // `__encoding_isAlphaNum`/`isUnreserved` byte, `__encoding_punyEncodeLabel` a
    // code point `< 128` or `__encoding_punyDigit` of a digit in 0 .. 35 (97 ..
    // 122 or 48 .. 57). One ASCII byte fits a Byte and is valid UTF-8.
    ("__encoding_byteChar", "toByte", "ErrOverflow"),
    ("__encoding_byteChar", "toString", "ErrEncoding"),
    // `c` is a byte, `hi`/`lo` are `__encoding_hexValue` results checked `>= 0`
    // (0 .. 15), and the bytes are decoded only after `__encoding_utf8Valid`.
    ("__encoding_percentDecodeBytes", "toByte", "ErrOverflow"),
    ("__encoding_percentDecodeBytes", "toString", "ErrEncoding"),
    // `bits::band(_, 255)`.
    ("__encoding_baseDecodeBits", "toByte", "ErrOverflow"),
    // `hi`/`lo` are `__encoding_hexValue` results checked `>= 0` (0 .. 15).
    ("__encoding_hexDecode", "toByte", "ErrOverflow"),
    // `chunk = bits::band(_, 127)`, so `chunk` and `chunk + 128` fit a Byte.
    ("__encoding_leb128Emit", "toByte", "ErrOverflow"),
    ("__encoding_sleb128Encode", "toByte", "ErrOverflow"),
    // `point` is a code point below 128 (checked), and the table-point scan's
    // `idx` is 0 .. 127 (the `idx < 0` FAIL above it, and `tablePoints` is
    // always the fixed 128-scalar codepage table), so `idx + 128` is 128 .. 255.
    ("__encoding_codepageEncode", "toByte", "ErrOverflow"),
    // bug-814. Each value is `bits::band(_, 255)` bound to a name first, a
    // decoded literal symbol `0 <= sym < 256`, `endPos MOD 256` of a
    // non-negative position, `bits::sr(c, 8)` of `0 <= c <= 65535`, a zlib header
    // byte of a value below 31000, or the gzip XFL 0/2/4.
    ("__compress_inflateCore", "toByte", "ErrOverflow"),
    ("__compress_deflateCore", "toByte", "ErrOverflow"),
    ("__compress_zlibEncode", "toByte", "ErrOverflow"),
    ("__compress_gzipEncode", "toByte", "ErrOverflow"),
    ("__crypto_appendBeWord", "toByte", "ErrOverflow"),
    ("__crypto_appendBeWord64", "toByte", "ErrOverflow"),
    ("__crypto_pad512", "toByte", "ErrOverflow"),
    // `bits::bxor`/`bits::bor` of two bytes, or of a byte and 128 / the pad 54/92,
    // stays a byte; `bits::sl(parity, 7)` of a parity bit is 0 or 128; `clampScalar`'s
    // `(b AND 127) OR 64` is at most 127; the sponge suffix is 6 or 31 at every call.
    ("__crypto_xorPad", "toByte", "ErrOverflow"),
    ("__crypto_xorBytes", "toByte", "ErrOverflow"),
    ("__crypto_keccakSponge", "toByte", "ErrOverflow"),
    ("__crypto_clampScalar", "toByte", "ErrOverflow"),
    ("__crypto_clampScalar448", "toByte", "ErrOverflow"),
    ("__crypto_ed448Prune", "toByte", "ErrOverflow"),
    ("__crypto_ed448Encode", "toByte", "ErrOverflow"),
    ("__crypto_packPoint", "toByte", "ErrOverflow"),
    // `counter <= ceil(length / L) <= 255`: `crypto::hkdf` rejects `length > 255
    // * L`, and HPKE's LabeledExpand asks at most 64 bytes of a 32/64-byte HMAC.
    ("__crypto_hkdfExpand", "toByte", "ErrOverflow"),
    // `uuid4`'s version/variant bytes are `(b AND 15) OR 64 <= 79` and `(b AND 63)
    // OR 128 <= 191`; `randomIntBig`'s top byte is a byte `MOD 2^topBits`, topBits
    // in 1 .. 8.
    ("__crypto_uuid4", "toByte", "ErrOverflow"),
    ("__crypto_randomIntBig", "toByte", "ErrOverflow"),
];

/// A code a qualified `pkg::member(` call can raise that no call of that member
/// in that one `FUNC` (a private helper or a rewrite target) can actually raise,
/// keyed `(FUNC, "pkg::member", code)` and applied where the call is, so every
/// member that reaches the `FUNC` inherits the proof — the qualified-call
/// counterpart of [`UNREACHABLE_GENERAL_CALL`] (bug-814), with the same bar: the
/// proof covers every call of that member in the `FUNC`, from its arguments'
/// ranges, and a proof that depends on the callers names them. One entry per
/// line (`rustfmt::skip`), so a FUNC's entries read as one block under its proof.
#[rustfmt::skip]
pub(super) const UNREACHABLE_QUALIFIED_CALL: &[(&str, &str, &str)] = &[
    // --- compress: the inflate core (RFC 1951 decoder) ---
    // Every `data` read is `IF pos < n` guarded (`pos` starts at the caller's
    // `start`: 0, 2 after a zlib header checked `n >= 2`, or a gzip header end
    // checked `<= n`) or a stored-block copy after `pos + remaining > n` FAILs.
    // `litT`/`distT`/`clT` come from `__compress_buildTable` with roots 9/6/7, so
    // they hold at least 512/64/128 entries for the `bits::band(bitBuf, 511/63/127)`
    // lookups; a sub-table index is `off + (bits below 2^subBits)`, inside the
    // sub-table buildTable appended at `off`. `li` is checked `< 29` and `dsym <
    // 30` before indexing the 29-entry length and 30-entry distance tables;
    // `ci < hclen <= 19` indexes the 19-entry CL_ORDER; `lens` holds exactly
    // `hlit + hdist >= 258` entries when `lens[256]` is read and `have >= 1` when
    // `lens[have - 1]` is; `clens` is 19 long and CL_ORDER's values are 0 .. 18.
    // A copy appends `mid(out, from, chunk)` with `1 <= dist <= outLen` checked
    // and `outLen = len(out)` throughout, so `from = outLen - dist` is in
    // `0 .. outLen - 1` and `chunk = min(outLen - from, left)` is at least 1 with
    // `from + chunk <= outLen` (bug-774).
    // Shifts: a refill shifts a byte by `bitCount <= 48`; every drop is a code
    // length `bits::band(entry, 15)` (0 .. 15), an extra-bit count from the
    // length/distance tables (0 .. 5 / 0 .. 13), `drop = bitCount MOD 8` of a
    // non-negative count, or a literal; `bits::sl(1, sub)` takes a
    // `bits::band(entry, 15)` of an entry checked `>= 1048576`.
    ("__compress_inflateCore", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_inflateCore", "collections::mid", "ErrIndexOutOfRange"),
    ("__compress_inflateCore", "collections::set", "ErrIndexOutOfRange"),
    ("__compress_inflateCore", "bits::sl", "ErrInvalidArgument"),
    ("__compress_inflateCore", "bits::sr", "ErrInvalidArgument"),
    // `bl`/`nextCode` are 16 long and indexed by a code length 0 .. 15 (every
    // caller's lengths are a fixed 5/7/8/9, or a decoded 3-bit or `cl < 16`
    // length) or by `bitLen`/`maxLen` in 0 .. 15. `lens[start + i]` with `i <
    // count` stays inside every caller's list: (0, 288) of 288, (0, 32) of 32,
    // (0, 19) of 19, (0, hlit) and (hlit, hdist) of `hlit + hdist`. The table
    // starts `2^root` long (root 9/6/7 from every caller); a direct entry is
    // written at `rev + k * 2^l < 2^root`, and a long code's sub-table entry at
    // `off + sj`, `sj < 2^subBits`, inside the sub-table appended at `off` — a
    // prefix-free code (over-subscription FAILs; an incomplete code is only
    // accepted with `maxLen = 1`, which has no long codes) never finds a direct
    // entry at a long code's root prefix. Shifts are by `root`, `l`, `b < l`,
    // `l - root` and `maxLen - root`, all in 0 .. 15 where evaluated.
    ("__compress_buildTable", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_buildTable", "collections::set", "ErrIndexOutOfRange"),
    ("__compress_buildTable", "bits::sl", "ErrInvalidArgument"),
    // The inflate core always appends its 8-byte end position, so `withEnd`
    // holds at least 8 bytes: `last - k` is in range and `len - 8 >= 0`.
    ("__compress_endPosition", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_stripEnd", "collections::mid", "ErrIndexOutOfRange"),
    ("__compress_inflate", "collections::mid", "ErrIndexOutOfRange"),
    // `data[0]`/`data[1]` after `n < 2` FAILs; `data[endPos + k]`, k < 4, after
    // `endPos + 4 > n` FAILs.
    ("__compress_zlibDecode", "collections::get", "ErrIndexOutOfRange"),
    // Every header read follows a `pos + 10 > n` / `p + 2 > n` / `p >= n` FAIL;
    // the header CRC covers `pos .. p` with `p <= n`; `__compress_le32` reads
    // `endPos .. endPos + 7` after `endPos + 8 > n` FAILs; `member[k]`, k < its
    // length.
    ("__compress_gzipDecode", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_gzipDecode", "collections::mid", "ErrIndexOutOfRange"),
    ("__compress_le32", "collections::get", "ErrIndexOutOfRange"),
    // `i + 7 <= n - 1` in the eight-byte loop (where `i` is a multiple of 8, so
    // `bits::bor(i, k)` is `i + k`) and `i < n` in the tail; the 2048-entry
    // slicing table `t` (`__compress_crc32` passes `__COMPRESS_CRC32_TABLES`) is
    // read at `bits::bor(k * 256, b)` = `k * 256 + b`, k in 0 .. 7, for `b` a
    // data byte, `bits::band(_, 255)`, or `bits::sr(x, 24)` of `x`, a 32-bit
    // value (the running CRC is checked or derived in 0 .. 2^32 - 1, and every
    // table entry is a 32-bit CRC). Adler-32 reads `data[i]`, `i < n`.
    ("__compress_crc32Run", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_adler32", "collections::get", "ErrIndexOutOfRange"),
    // --- compress: the deflate encoder ---
    // Every caller rejects a level outside 0 .. 9 first, so the 10-entry
    // configuration lists are in range. `head` has 32768 entries indexed by a
    // 15-bit hash; `prev` has `min(n, 32768)` indexed by `bits::band(p, 32767)` of
    // a position `p < n`. A match compares `data[cand + k]`/`data[pos + k]` only
    // while `k < maxLen <= n - pos` (`AND` short-circuits) with `cand < pos`, and
    // `data[pos + bestLen]` while `bestLen < maxLen` (a match reaching `maxLen`
    // exits). `pos - 1` is read only after `pos` advanced. LEN_SYM (259 entries)
    // is indexed by a match length 3 .. 258, DIST_CODE (512) by `d1 < 256` or `256
    // + bits::sr(d1, 7)` with `d1 = dist - 1 <= 32767`; LEN/DIST tables by a
    // `li <= 28` / `dsym <= 29` read off them; `litFreq` (286) at a literal, 256 or
    // `257 + li`, `distFreq` (30) at `dsym`; the code lists (288/286 and 30) at
    // the same symbols. The bit writer flushes to `bitCount < 8` after every
    // symbol, so a shift count is at most 10 + 15 + 5 + 15 = 45.
    ("__compress_deflateCore", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_deflateCore", "collections::set", "ErrIndexOutOfRange"),
    ("__compress_deflateCore", "bits::sl", "ErrInvalidArgument"),
    // `freqs`/`lens` are equal-length (`__compress_codeLengths` returns one length
    // per frequency); `litFreq` (286) is shorter than FIXED_LIT (288); `i < len`.
    ("__compress_treeBits", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_fixedLitBits", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_headerBits", "collections::get", "ErrIndexOutOfRange"),
    // `lens[0]` and `lens[n + 1]` with `n + 1 < codes`, `codes` (hlit <= 286 /
    // hdist <= 30) at most the list's length.
    ("__compress_treeRle", "collections::get", "ErrIndexOutOfRange"),
    // `litLens[hlit - 1]`/`distLens[hdist - 1]` with hlit in 258 .. 286 and hdist
    // in 2 .. 30; `clFreq`/`clLens`/`clCodes` are 19 long and indexed by a
    // code-length symbol `entry MOD 32` in 0 .. 18 (zlib's send_tree invariant:
    // a repeat count is decremented only when it is >= 4, so every `count - 3`
    // and `count - 11` is >= 0), or by a CL_ORDER value 0 .. 18.
    ("__compress_dynamicHeader", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_dynamicHeader", "collections::set", "ErrIndexOutOfRange"),
    // Package-merge over `count` (19, 30 or 286) symbols: `f[node]`, node 0 .. 2;
    // the insertion sort reads `keys[j - 1]` only for `j > 0` (`AND`
    // short-circuits); pool indices are pool entries created earlier; `cur` holds
    // at least `2n - 2` items after `limit - 1` merges because `n <= 2^limit` (286 <
    // 2^15, 30 < 2^15, 19 < 2^7); the stack is read and removed at `top`, its
    // last index; `lens[sym]` with `sym < count`.
    ("__compress_codeLengths", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_codeLengths", "collections::set", "ErrIndexOutOfRange"),
    ("__compress_codeLengths", "collections::removeAt", "ErrIndexOutOfRange"),
    // Code lengths are 1 .. 15 (package-merge with limit 15 or 7), indexing the
    // 16-entry `blCount`/`nextCode`; `b - 1` is 0 .. 14.
    ("__compress_canonicalCodes", "collections::get", "ErrIndexOutOfRange"),
    ("__compress_canonicalCodes", "collections::set", "ErrIndexOutOfRange"),
    // --- crypto: shared byte-list helpers ---
    // Each reads index `i < len` of its own argument (`concat`'s `b`,
    // `concatInt`'s `b`, `isAllZero`'s `b`, `xorPad`'s `data`), `0 .. len(data)`
    // (`copyBytes`), or the shorter length (`constantTimeEqual`).
    ("__crypto_concat", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_concatInt", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_isAllZero", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_xorPad", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_constantTimeEqual", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_copyBytes", "collections::mid", "ErrIndexOutOfRange"),
    // `xorBytes(a, b)` reads `b[i]`, `i < len(a)`: its callers pass equal-length
    // lists (`__crypto_gcmTag`: two 16-byte blocks; `__crypto_pbkdf2Block`: two
    // outputs of the same HMAC).
    ("__crypto_xorBytes", "collections::get", "ErrIndexOutOfRange"),
    // `slice(data, start, stop)` / `truncate(data, n)`: every caller passes `0 <=
    // start <= stop <= len(data)` / `0 <= n <= len(data)` — fixed digests and
    // seeds (a SHA-512 64, a SHAKE256-114 or ChaCha20 64-byte block cut at 32/57,
    // an HPKE key schedule's 44 bytes cut at 32/44, a length-checked 64/114-byte
    // signature), `__crypto_decrypt`'s box after `total < nenc + 16` FAILs, the
    // sponge's `off <= n`, and output loops that run until `len(okm) >= length`.
    ("__crypto_slice", "collections::mid", "ErrIndexOutOfRange"),
    ("__crypto_truncate", "collections::mid", "ErrIndexOutOfRange"),
    // --- crypto: SHA-1 / SHA-2 ---
    // `pad512`/`pad1024` leave a multiple of 64/128 bytes, and each block loop
    // runs `base < total`, so a schedule's `beWord`/`beWord64` reads `base .. base
    // + 63/127`; the other `beWord64` callers read a 16-byte GCM subkey block at
    // 0/8, GHASH data (a multiple of 16) at `o`/`o + 8`, `beWords64`'s
    // multiple-of-8 hex constants and the 192-byte Keccak RC table at `t * 8`, t
    // < 24. The schedules index their own 80/64-entry `w` at `t - 2 .. t - 16`
    // for `t >= 16`; the cores read `w`/K256 (64) or K512 (80) at `t` below the
    // round count and their 8-word IV at 0 .. 7.
    ("__crypto_beWord", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_beWord64", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha1Schedule", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha256Schedule", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha512Schedule", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha1_bytes", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha2_32", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_sha2_64", "collections::get", "ErrIndexOutOfRange"),
    // Shift counts `56 - i * 8` / `i * 8` for `i < 8`, and `__crypto_shr32`'s
    // count from its only callers, `ssig0` (3) and `ssig1` (10).
    ("__crypto_pad512", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_appendBeWord64", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_appendLeLane", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_shr32", "bits::sr", "ErrInvalidArgument"),
    // --- crypto: Keccak-f[1600] ---
    // The state is always 25 lanes (`__crypto_keccakZero`, then `set`s and
    // rounds that return 25); `lane`/`fl`/`sq < rateLanes <= 18`. An absorb
    // reads `off + rate <= n`; the padded tail is exactly `rate` bytes (the tail
    // is `n - off < rate` bytes plus the suffix byte), so `last = rate - 1` and
    // `fl * 8 + 7` are in it. A round reads `c` (5) at `MOD 5`, `a`/`t`/`b`/`out`
    // (25) at `x + 5y < 25`, and the 25-entry RHO/PI tables, whose PI values are
    // 0 .. 24; `keccakF` reads the 24 round constants at `round < 24`.
    // `leLane(data, o)` reads `o .. o + 7`: the sponge's checked offsets above,
    // BLAKE2b's 128-byte blocks at `base + i * 8`, i < 16, and Argon2's 1024-byte
    // H' output at `k * 8`, k < 128; its shift `i * 8` is at most 56.
    ("__crypto_keccakSponge", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_keccakSponge", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_keccakRound", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_keccakRound", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_keccakF", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_leLane", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_leLane", "bits::sl", "ErrInvalidArgument"),
    // --- crypto: Curve25519 / Ed25519 (16 limbs) ---
    // Every field element is 16 limbs (`gf0`/`gf1`/`unpack25519` build 16, and
    // `edA`/`edZ`/`edM`/`sel25519`/`car25519` map 16 to 16), so the limb loops'
    // `i, j, k < 16` and `car25519`'s `i + 1` (for `i < 15`) are in range. A point
    // is four elements (64 limbs; `gfAt(p, 0 .. 3)`), a ladder pair two
    // (128; `first64`/`last64`/`cswap128` read `i` and `64 + i`, i < 64).
    // `unpack25519`/`clampScalar` read 32 bytes; every caller passes 32: a key
    // length-checked by `__crypto_exchange`/`__crypto_hpkeRecipientPub`/
    // `__crypto_ed25519Verify`, `crypto::randomBytes(32)`, a 32-byte truncated
    // SHA-512, a `pack25519` output (32), the 32-byte X25519 base, or HPKE's
    // `enc` (`nenc` = 32). The ladders index the 32-byte scalar at `bits::sr(i, 3)`
    // / `i / 8` for `i <= 255`, and shift by `bits::band(i, 7)`.
    ("__crypto_sel25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_edA", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_edZ", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_edM", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_par25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gfAt", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_first64", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_last64", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_car25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_car25519", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_pack25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_pack25519", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_unpack25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_unpack25519", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_clampScalar", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_clampScalar", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_packPoint", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_packPoint", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_cswap128", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_cswap128", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_gf1", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_gf121665", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_x25519", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_x25519", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_scalarmult", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_scalarmult", "bits::sr", "ErrInvalidArgument"),
    // --- crypto: Curve448 / Ed448 (16 limbs of 28 bits) ---
    // As for Curve25519: every `gf448` value is 16 limbs, a product's `c` is 31
    // (`n + 16 <= 30` for `n <= 14`, `n + 24 <= 30` for `n <= 6`), a point 48 limbs
    // (`ed448PointAt(p, 0 .. 2)`) and a ladder pair 96 (`ed448Cswap` reads `i`,
    // `48 + i`; the ladder cuts it at 0/48). `gf448Unpack`/`clampScalar448` read 56
    // bytes: a length-checked 56-byte key, `crypto::randomBytes(56)`, a 56-byte
    // SHAKE256 output, a `gf448Pack` output (56), the 56-byte base, a 57-byte Ed448
    // key cut to 56, or the two 56-byte hex constants of the Ed448 base point.
    // `ed448Prune` reads 0/55/56 of a 57-byte truncated SHAKE256 output, and
    // `ed448Encode` byte 0 of a 56-byte pack. The ladders index the 56/57-byte
    // scalar at `bits::sr(i, 3)` for `i <= 447`; shifts are `i * 8`/`k * 8` for `i,
    // k < 7` and `bits::band(i, 7)`.
    ("__crypto_gf448Add", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Sub", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Mul", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448MulSmall", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Select", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_ed448Encode", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Carry", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Carry", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_clampScalar448", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_clampScalar448", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_ed448Prune", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_ed448Prune", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_ed448Cswap", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_ed448Cswap", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_gf448Unpack", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Unpack", "bits::sl", "ErrInvalidArgument"),
    ("__crypto_gf448Pack", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_gf448Pack", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_x448", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_x448", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_ed448Scalarmult", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_ed448Scalarmult", "collections::mid", "ErrIndexOutOfRange"),
    ("__crypto_ed448Scalarmult", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_ed448PointAt", "collections::mid", "ErrIndexOutOfRange"),
    ("__crypto_ed448PubToX448", "collections::mid", "ErrIndexOutOfRange"),
    // --- crypto: BLAKE2b and Argon2id ---
    // BLAKE2b's chain `h` is 8 words and the IV 8; the working vector `v` 16 and
    // the message `m` 16, read at SIGMA values 0 .. 15; SIGMA holds 12 rounds of 16
    // (`s + 15 <= 191`); `blake2bG`'s four indices are literals 0 .. 15 at every
    // call; the digest loop reads `h[i / 8]`, `i < outLen <= 64` (H0 asks 64, H'
    // asks `outLen <= 64` or 64, or its last `outLen - 32r` in 33 .. 64), shifting
    // by `(i MOD 8) * 8 <= 56`. `gather3` reads each of its three lists below that
    // list's length, from `i < stop <= n`.
    ("__crypto_blake2b3", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_blake2b3", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_blake2b3", "bits::sr", "ErrInvalidArgument"),
    ("__crypto_blake2bCompress", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_blake2bCompress", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_blake2bG", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_blake2bG", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_gather3", "collections::get", "ErrIndexOutOfRange"),
    // Argon2id: `mem` holds `blocks + 4` blocks of 128 words (`blocks >= 8 *
    // parallelism` after the range checks), and every offset is a block number
    // times 128 plus `k < 128`: a lane's first two blocks, the current block
    // `curr` and its predecessor `prev` (both inside the current lane), the
    // reference block (`refLane = (rnd >> 32) MOD parallelism`, `refIdx` reduced
    // `MOD laneLen`), the last column, or the four scratch blocks past the matrix
    // (`inputOff + 6`, `addrOff + idx MOD 128`). The fill reads three such 128-word
    // blocks, its `r`/`tmp` are 128 words (`2g + 16k + 1 <= 127`), and the
    // permutation takes and returns 16 words. `argon2Le32` shifts by `i * 8`, i < 4.
    ("__crypto_argon2id", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_argon2id", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_argon2Fill", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_argon2Fill", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_argon2Fill", "collections::mid", "ErrIndexOutOfRange"),
    ("__crypto_argon2P", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_argon2P", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_argon2P", "collections::mid", "ErrIndexOutOfRange"),
    ("__crypto_argon2Le32", "bits::sr", "ErrInvalidArgument"),
    // --- crypto: random draws ---
    // `crypto::randomBytes(8)`/`(16)` is a literal count inside 0 .. 16 MiB, and
    // the result is exactly that many bytes: `rand62`/`rand63` read 0 .. 7,
    // `uuid4` reads and writes 6 and 8 and cuts its 32-character hex string at
    // 0 .. 32. `randomIntBig` draws `byteCount = (width + 7) / 8 >= 1` bytes
    // (`max > min`, so the span is at least 2) and reads/writes `byteCount - 1`.
    ("__crypto_rand62", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_rand62", "crypto::randomBytes", "ErrInvalidArgument"),
    ("__crypto_rand63", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_rand63", "crypto::randomBytes", "ErrInvalidArgument"),
    ("__crypto_uuid4", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_uuid4", "collections::set", "ErrIndexOutOfRange"),
    ("__crypto_uuid4", "strings::mid", "ErrIndexOutOfRange"),
    ("__crypto_uuid4", "crypto::randomBytes", "ErrInvalidArgument"),
    ("__crypto_randomIntBig", "collections::get", "ErrIndexOutOfRange"),
    ("__crypto_randomIntBig", "collections::set", "ErrIndexOutOfRange"),
    // --- net: toUrl's path backslash rewrite (bug-979) ---
    // The one `strings::replace` call is `replace(path, "\\", "/")`: its `old`
    // argument is a one-character literal, and replace raises ErrInvalidArgument
    // only for an empty `old`.
    ("__net_toUrl", "strings::replace", "ErrInvalidArgument"),
];

/// The mirror image of [`UNREACHABLE_VIA_CALLEE`]: a code this sweep's model
/// CANNOT see as reachable, but that really is — the `Body::Mfb` counterpart
/// of `declared_errors_all.rs`'s `IO_DECLARED_EXTRA`/`TERM_DECLARED_EXTRA`
/// ("a code this sweep's harness configuration cannot make reachable, but
/// that a real build does raise"). `encoding::htmlUnescape`/`punycodeDecode`'s
/// ErrEncoding used to be carried here; since bug-807 this guard sees the bare
/// `toString(<List OF Byte>)` call in `__encoding_fromCodepoint` that raises it.
/// `http::finish`'s eight transport codes used to be carried here too, because
/// it re-raised the code VALUE in `s.state.err`; since bug-808 it raises each
/// one from a literal `FAIL error(<code>, …)`, which this guard sees.
pub(super) const DECLARED_EXTRA: &[(&str, &str, &str)] = &[
    // Pre-existing, correct `ErrOverflow`/`ErrOutOfMemory`/etc. declarations
    // on members whose only raise site is an IMPLICIT checked-arithmetic
    // operator (`+`/`-`/`*` on Integer, a MOD/divide) in their own Mfb
    // source, never a literal `FAIL error(...)` -- this guard's own top-of-
    // file doc comment scopes it to explicit FAILs and callee propagation,
    // so an implicit operator raise is a documented blind spot, not a wrong
    // declaration. Found by the whole-registry sweep (none of these were in
    // Phase 1's original 136-member list; they predate bug-698 and were
    // simply never measured until this guard's first whole-registry run).
    ("audio", "render", "ErrInvalidFormat"),
    ("audio", "render", "ErrOverflow"),
    ("canvas", "getSize", "ErrOutOfMemory"),
    ("canvas", "getSize", "ErrResourceClosed"),
    ("datetime", "instant", "ErrOverflow"),
    ("datetime", "duration", "ErrOverflow"),
    ("datetime", "inZone", "ErrOverflow"),
    ("datetime", "resolve", "ErrOverflow"),
    ("datetime", "add", "ErrOverflow"),
    ("datetime", "subtract", "ErrOverflow"),
    ("datetime", "between", "ErrOverflow"),
    ("datetime", "negate", "ErrOverflow"),
    ("datetime", "plus", "ErrOverflow"),
    ("datetime", "minus", "ErrOverflow"),
    ("datetime", "weekday", "ErrOverflow"),
    ("datetime", "dayOfYear", "ErrOverflow"),
    ("datetime", "toMillis", "ErrOverflow"),
    ("datetime", "toNanos", "ErrOverflow"),
    ("datetime", "format", "ErrOverflow"),
    ("datetime", "formatDuration", "ErrOverflow"),
];
