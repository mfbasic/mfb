use std::borrow::Cow;

use crate::codegen::registry::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    DefaultResolver, DefaultValue, Implementation, InjectionRule, Lowering, Parameter,
    ParameterType, ReturnType,
};

// Byte<->text and Unicode codecs, implemented in MFBASIC source over `bits`,
// `strings`, and `collections` (see `encoding_package.mfb`). Public names map to
// internal `__encoding_*` helpers via `implementation_name`; the two overloaded
// names (`utf8Encode` return-type overload, `utf8Decode` parameter overload) are
// resolved in the type checker and monomorphizer (see `resolve_overload_target`).
// See `plan-02-encoding.md` Part B.

const UTF8_ENCODE: &str = "encoding.utf8Encode";
const UTF8_DECODE: &str = "encoding.utf8Decode";
const UTF16_ENCODE: &str = "encoding.utf16Encode";
const UTF16_DECODE: &str = "encoding.utf16Decode";
const UTF32_ENCODE: &str = "encoding.utf32Encode";
const UTF32_DECODE: &str = "encoding.utf32Decode";
const HEX_ENCODE: &str = "encoding.hexEncode";
const HEX_DECODE: &str = "encoding.hexDecode";
const BASE32_ENCODE: &str = "encoding.base32Encode";
const BASE32_DECODE: &str = "encoding.base32Decode";
const BASE64_ENCODE: &str = "encoding.base64Encode";
const BASE64_DECODE: &str = "encoding.base64Decode";
const BASE64URL_ENCODE: &str = "encoding.base64UrlEncode";
const BASE64URL_DECODE: &str = "encoding.base64UrlDecode";
const PERCENT_ENCODE: &str = "encoding.percentEncode";
const PERCENT_DECODE: &str = "encoding.percentDecode";
const HTML_ESCAPE: &str = "encoding.htmlEscape";
const HTML_UNESCAPE: &str = "encoding.htmlUnescape";
const FORM_URL_ENCODE: &str = "encoding.formUrlEncode";
const FORM_URL_DECODE: &str = "encoding.formUrlDecode";
const PUNYCODE_ENCODE: &str = "encoding.punycodeEncode";
const PUNYCODE_DECODE: &str = "encoding.punycodeDecode";
const ULEB128_ENCODE: &str = "encoding.uleb128Encode";
const ULEB128_DECODE: &str = "encoding.uleb128Decode";
const SLEB128_ENCODE: &str = "encoding.sleb128Encode";
const SLEB128_DECODE: &str = "encoding.sleb128Decode";
const VARINT_ENCODE: &str = "encoding.varintEncode";
const VARINT_DECODE: &str = "encoding.varintDecode";

// The concrete dispatch targets the overloaded `utf8Encode`/`utf8Decode` names
// resolve to during monomorphization. They are package-qualified (so the
// post-monomorph resolver accepts them as built-in members) and map to their
// internal implementation in `implementation_name`, exactly like the other
// non-overloaded functions.
const UTF8_ENCODE_BYTES: &str = "encoding.utf8EncodeBytes";
const UTF8_ENCODE_INTS: &str = "encoding.utf8EncodeInts";
const UTF8_DECODE_BYTES: &str = "encoding.utf8DecodeBytes";
const UTF8_DECODE_INTS: &str = "encoding.utf8DecodeInts";

const BYTES: &str = "List OF Byte";
const INTS: &str = "List OF Integer";

// plan-72-I: `ENCODING` is the descriptor authority. Every function is unary
// with a fixed return, so `is_encoding_call`/`arity`/`call_return_type_name`/
// `implementation_name` derive from the descriptor. Non-overloaded functions (and
// the 4 monomorph targets) carry `Implementation::Rewrite(__encoding_*)`; the two
// overloaded names `utf8Encode`/`utf8Decode` are `Implementation::Custom`
// (`is_overloaded`), resolved by `EncodingResolver::resolve_overload_target`.
// `resolve_call` argument validation is also resolver-owned. `WhenImported` source.
const fn p(name: &'static str, aliases: &'static [&'static str], ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::None,
    }
}
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}
const fn ef(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
    implementation: Implementation,
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        doc_intro: "",
        doc_desc: "",
        errors: &[],
        overloads,
        doc_example: "",
        implementation,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}
const VALTEXT: &[&str] = &["text"];

// --- MFBASIC bodies (moved out of encoding_package.mfb; each replaces a
// '@@MFB_BODY:<slug>@@ marker in package.mfb via assembled_source). Byte-
// significant: the 2-space indentation feeds source columns into .ncode.
#[rustfmt::skip]
const UTF8_ENCODE_BYTES_BODY: &str =
r#"FUNC __encoding_utf8EncodeBytes(value AS String) AS List OF Byte
  RETURN strings::toBytes(value)
END FUNC"#;

#[rustfmt::skip]
const UTF8_ENCODE_INTS_BODY: &str =
r#"FUNC __encoding_utf8EncodeInts(value AS String) AS List OF Integer
  LET data AS List OF Byte = strings::toBytes(value)
  MUT result AS List OF Integer = []
  FOR EACH b IN data
    result = collections::append(result, toInt(b))
  NEXT
  RETURN result
END FUNC"#;

#[rustfmt::skip]
const UTF8_DECODE_BYTES_BODY: &str =
r#"FUNC __encoding_utf8DecodeBytes(value AS List OF Byte) AS String
  IF __encoding_utf8Valid(value) = FALSE THEN
    FAIL error(77050003, "invalid utf-8")
  END IF
  RETURN toString(value)
END FUNC"#;

#[rustfmt::skip]
const UTF8_DECODE_INTS_BODY: &str =
r#"FUNC __encoding_utf8DecodeInts(value AS List OF Integer) AS String
  MUT data AS List OF Byte = []
  FOR EACH unit IN value
    IF unit < 0 OR unit > 255 THEN
      FAIL error(77050003, "invalid utf-8 code unit")
    END IF
    data = collections::append(data, toByte(unit))
  NEXT
  IF __encoding_utf8Valid(data) = FALSE THEN
    FAIL error(77050003, "invalid utf-8")
  END IF
  RETURN toString(data)
END FUNC"#;

#[rustfmt::skip]
const UTF16_ENCODE_BODY: &str =
r#"FUNC __encoding_utf16Encode(value AS String) AS List OF Integer
  LET points AS List OF Integer = __encoding_codepoints(value)
  MUT result AS List OF Integer = []
  MUT scalar AS Integer = 0
  MUT high AS Integer = 0
  MUT low AS Integer = 0
  FOR EACH cp IN points
    IF cp <= 65535 THEN
      result = collections::append(result, cp)
    ELSE
      scalar = cp - 65536
      high = 55296 + bits::sr(scalar, 10)
      low = 56320 + bits::band(scalar, 1023)
      result = collections::append(result, high)
      result = collections::append(result, low)
    END IF
  NEXT
  RETURN result
END FUNC"#;

#[rustfmt::skip]
const UTF16_DECODE_BODY: &str =
r#"FUNC __encoding_utf16Decode(value AS List OF Integer) AS String
  LET n AS Integer = len(value)
  MUT out AS String = ""
  MUT i AS Integer = 0
  MUT unit AS Integer = 0
  MUT low AS Integer = 0
  MUT scalar AS Integer = 0
  WHILE i < n
    unit = collections::get(value, i)
    IF unit < 0 OR unit > 65535 THEN
      FAIL error(77050003, "invalid utf-16 code unit")
    END IF
    IF unit >= 55296 AND unit <= 56319 THEN
      IF i + 1 >= n THEN
        FAIL error(77050003, "unpaired surrogate")
      END IF
      low = collections::get(value, i + 1)
      IF low < 56320 OR low > 57343 THEN
        FAIL error(77050003, "unpaired surrogate")
      END IF
      scalar = 65536 + bits::sl(unit - 55296, 10) + (low - 56320)
      out = out & __encoding_fromCodepoint(scalar)
      i = i + 2
    ELSE
      IF unit >= 56320 AND unit <= 57343 THEN
        FAIL error(77050003, "unpaired surrogate")
      END IF
      out = out & __encoding_fromCodepoint(unit)
      i = i + 1
    END IF
  END WHILE
  RETURN out
END FUNC"#;

#[rustfmt::skip]
const UTF32_ENCODE_BODY: &str =
r#"FUNC __encoding_utf32Encode(value AS String) AS List OF Integer
  RETURN __encoding_codepoints(value)
END FUNC"#;

#[rustfmt::skip]
const UTF32_DECODE_BODY: &str =
r#"FUNC __encoding_utf32Decode(value AS List OF Integer) AS String
  MUT out AS String = ""
  FOR EACH cp IN value
    IF cp < 0 OR cp > 1114111 THEN
      FAIL error(77050003, "invalid code point")
    END IF
    IF cp >= 55296 AND cp <= 57343 THEN
      FAIL error(77050003, "surrogate code point")
    END IF
    out = out & __encoding_fromCodepoint(cp)
  NEXT
  RETURN out
END FUNC"#;

#[rustfmt::skip]
const HEX_ENCODE_BODY: &str =
r#"FUNC __encoding_hexEncode(data AS List OF Byte) AS String
  MUT out AS String = ""
  MUT v AS Integer = 0
  FOR EACH b IN data
    v = toInt(b)
    out = out & __encoding_hexDigit(v / 16) & __encoding_hexDigit(v - (v / 16) * 16)
  NEXT
  RETURN out
END FUNC"#;

#[rustfmt::skip]
const HEX_DECODE_BODY: &str =
r#"FUNC __encoding_hexDecode(text AS String) AS List OF Byte
  LET data AS List OF Byte = strings::toBytes(text)
  LET n AS Integer = len(data)
  IF n - (n / 2) * 2 <> 0 THEN
    FAIL error(77050003, "odd-length hex")
  END IF
  MUT result AS List OF Byte = []
  MUT i AS Integer = 0
  MUT hi AS Integer = 0
  MUT lo AS Integer = 0
  WHILE i < n
    hi = __encoding_hexValue(toInt(collections::get(data, i)))
    lo = __encoding_hexValue(toInt(collections::get(data, i + 1)))
    IF hi < 0 OR lo < 0 THEN
      FAIL error(77050003, "invalid hex digit")
    END IF
    result = collections::append(result, toByte(hi * 16 + lo))
    i = i + 2
  END WHILE
  RETURN result
END FUNC"#;

#[rustfmt::skip]
const BASE32_ENCODE_BODY: &str =
r#"FUNC __encoding_base32Encode(data AS List OF Byte) AS String
  RETURN __encoding_baseEncode(data, "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567", 5, 8, TRUE)
END FUNC"#;

#[rustfmt::skip]
const BASE32_DECODE_BODY: &str =
r#"FUNC __encoding_base32Decode(text AS String) AS List OF Byte
  LET data AS List OF Byte = strings::toBytes(text)
  LET total AS Integer = len(data)
  IF total - (total / 8) * 8 <> 0 THEN
    FAIL error(77050003, "invalid base32 length")
  END IF
  MUT values AS List OF Integer = []
  MUT i AS Integer = 0
  MUT seenPad AS Boolean = FALSE
  MUT c AS Integer = 0
  MUT v AS Integer = 0
  WHILE i < total
    c = toInt(collections::get(data, i))
    IF c = 61 THEN
      seenPad = TRUE
    ELSE
      IF seenPad THEN
        FAIL error(77050003, "invalid base32 padding")
      END IF
      v = __encoding_base32Value(c)
      IF v < 0 THEN
        FAIL error(77050003, "invalid base32 character")
      END IF
      values = collections::append(values, v)
    END IF
    i = i + 1
  END WHILE
  LET symbols AS Integer = len(values)
  LET tail AS Integer = symbols - (symbols / 8) * 8
  IF tail = 1 OR tail = 3 OR tail = 6 THEN
    FAIL error(77050003, "invalid base32 length")
  END IF
  RETURN __encoding_baseDecodeBits(values, 5)
END FUNC"#;

#[rustfmt::skip]
const BASE64_ENCODE_BODY: &str =
r#"FUNC __encoding_base64Encode(data AS List OF Byte) AS String
  RETURN __encoding_baseEncode(data, "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/", 6, 4, TRUE)
END FUNC"#;

#[rustfmt::skip]
const BASE64_DECODE_BODY: &str =
r#"FUNC __encoding_base64Decode(text AS String) AS List OF Byte
  LET data AS List OF Byte = strings::toBytes(text)
  LET total AS Integer = len(data)
  IF total - (total / 4) * 4 <> 0 THEN
    FAIL error(77050003, "invalid base64 length")
  END IF
  LET values AS List OF Integer = __encoding_base64Symbols(text, FALSE)
  LET symbols AS Integer = len(values)
  IF symbols - (symbols / 4) * 4 = 1 THEN
    FAIL error(77050003, "invalid base64 length")
  END IF
  RETURN __encoding_baseDecodeBits(values, 6)
END FUNC"#;

#[rustfmt::skip]
const BASE64_URL_ENCODE_BODY: &str =
r#"FUNC __encoding_base64UrlEncode(data AS List OF Byte) AS String
  RETURN __encoding_baseEncode(data, "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_", 6, 4, FALSE)
END FUNC"#;

#[rustfmt::skip]
const BASE64_URL_DECODE_BODY: &str =
r#"FUNC __encoding_base64UrlDecode(text AS String) AS List OF Byte
  LET values AS List OF Integer = __encoding_base64Symbols(text, TRUE)
  LET symbols AS Integer = len(values)
  IF symbols - (symbols / 4) * 4 = 1 THEN
    FAIL error(77050003, "invalid base64 length")
  END IF
  RETURN __encoding_baseDecodeBits(values, 6)
END FUNC"#;

#[rustfmt::skip]
const PERCENT_ENCODE_BODY: &str =
r#"FUNC __encoding_percentEncode(text AS String) AS String
  LET data AS List OF Byte = strings::toBytes(text)
  MUT out AS String = ""
  MUT c AS Integer = 0
  FOR EACH b IN data
    c = toInt(b)
    IF __encoding_isUnreserved(c) THEN
      out = out & __encoding_byteChar(c)
    ELSE
      out = out & __encoding_percentByte(c)
    END IF
  NEXT
  RETURN out
END FUNC"#;

#[rustfmt::skip]
const PERCENT_DECODE_BODY: &str =
r#"FUNC __encoding_percentDecode(text AS String) AS String
  RETURN __encoding_percentDecodeBytes(text, FALSE)
END FUNC"#;

#[rustfmt::skip]
const HTML_ESCAPE_BODY: &str =
r#"FUNC __encoding_htmlEscape(text AS String) AS String
  MUT out AS String = text
  out = strings::replace(out, "&", "&amp;")
  out = strings::replace(out, "<", "&lt;")
  out = strings::replace(out, ">", "&gt;")
  out = strings::replace(out, "\"", "&quot;")
  out = strings::replace(out, "'", "&apos;")
  RETURN out
END FUNC"#;

#[rustfmt::skip]
const HTML_UNESCAPE_BODY: &str =
r##"FUNC __encoding_htmlUnescape(text AS String) AS String
  LET chars AS List OF String = strings::graphemes(text)
  LET n AS Integer = len(chars)
  MUT out AS String = ""
  MUT i AS Integer = 0
  MUT ch AS String = ""
  MUT body AS String = ""
  MUT j AS Integer = 0
  MUT found AS Boolean = FALSE
  MUT code AS Integer = 0
  WHILE i < n
    ch = collections::get(chars, i)
    IF ch = "&" THEN
      body = ""
      j = i + 1
      found = FALSE
      WHILE j < n AND found = FALSE
        IF collections::get(chars, j) = ";" THEN
          found = TRUE
        ELSE
          body = body & collections::get(chars, j)
          j = j + 1
        END IF
      END WHILE
      IF found = FALSE THEN
        FAIL error(77050003, "malformed entity")
      END IF
      IF strings::startsWith(body, "#x") OR strings::startsWith(body, "#X") THEN
        code = __encoding_parseHex(strings::mid(body, 2, len(body) - 2))
      ELSE
        IF strings::startsWith(body, "#") THEN
          code = __encoding_parseDecimal(strings::mid(body, 1, len(body) - 1))
        ELSE
          code = __encoding_htmlEntity(body)
        END IF
      END IF
      IF code < 0 THEN
        FAIL error(77050003, "unknown entity")
      END IF
      out = out & __encoding_fromCodepoint(code)
      i = j + 1
    ELSE
      out = out & ch
      i = i + 1
    END IF
  END WHILE
  RETURN out
END FUNC"##;

#[rustfmt::skip]
const FORM_URL_ENCODE_BODY: &str =
r#"FUNC __encoding_formUrlEncode(text AS String) AS String
  LET data AS List OF Byte = strings::toBytes(text)
  MUT out AS String = ""
  MUT c AS Integer = 0
  FOR EACH b IN data
    c = toInt(b)
    IF __encoding_isAlphaNum(c) THEN
      out = out & __encoding_byteChar(c)
    ELSE
      IF c = 32 THEN
        out = out & "+"
      ELSE
        out = out & __encoding_percentByte(c)
      END IF
    END IF
  NEXT
  RETURN out
END FUNC"#;

#[rustfmt::skip]
const FORM_URL_DECODE_BODY: &str =
r#"FUNC __encoding_formUrlDecode(text AS String) AS String
  RETURN __encoding_percentDecodeBytes(text, TRUE)
END FUNC"#;

#[rustfmt::skip]
const PUNYCODE_ENCODE_BODY: &str =
r#"FUNC __encoding_punycodeEncode(domain AS String) AS String
  LET labels AS List OF String = strings::split(domain, ".")
  MUT out AS String = ""
  MUT first AS Boolean = TRUE
  FOR EACH label IN labels
    IF first THEN
      first = FALSE
    ELSE
      out = out & "."
    END IF
    LET points AS List OF Integer = __encoding_codepoints(label)
    IF __encoding_labelHasNonAscii(points) THEN
      out = out & "xn--" & __encoding_punyEncodeLabel(points)
    ELSE
      out = out & label
    END IF
  NEXT
  RETURN out
END FUNC"#;

#[rustfmt::skip]
const PUNYCODE_DECODE_BODY: &str =
r#"FUNC __encoding_punycodeDecode(asciiDomain AS String) AS String
  LET labels AS List OF String = strings::split(asciiDomain, ".")
  MUT out AS String = ""
  MUT first AS Boolean = TRUE
  FOR EACH label IN labels
    IF first THEN
      first = FALSE
    ELSE
      out = out & "."
    END IF
    IF strings::startsWith(label, "xn--") THEN
      out = out & __encoding_punyDecodeLabel(strings::mid(label, 4, len(label) - 4))
    ELSE
      out = out & label
    END IF
  NEXT
  RETURN out
END FUNC"#;

#[rustfmt::skip]
const ULEB128_ENCODE_BODY: &str =
r#"FUNC __encoding_uleb128Encode(value AS Integer) AS List OF Byte
  IF value < 0 THEN
    FAIL error(77050003, "negative value")
  END IF
  RETURN __encoding_leb128Emit(value)
END FUNC"#;

#[rustfmt::skip]
const ULEB128_DECODE_BODY: &str =
r#"FUNC __encoding_uleb128Decode(data AS List OF Byte) AS Integer
  LET n AS Integer = len(data)
  IF n = 0 THEN
    FAIL error(77050003, "truncated leb128")
  END IF
  MUT result AS Integer = 0
  MUT shift AS Integer = 0
  MUT i AS Integer = 0
  MUT byteValue AS Integer = 0
  MUT done AS Boolean = FALSE
  WHILE done = FALSE
    IF i >= n THEN
      FAIL error(77050003, "truncated leb128")
    END IF
    IF shift > 63 THEN
      FAIL error(77050003, "leb128 overflow")
    END IF
    byteValue = toInt(collections::get(data, i))
    result = bits::bor(result, bits::sl(bits::band(byteValue, 127), shift))
    shift = shift + 7
    i = i + 1
    IF byteValue < 128 THEN
      done = TRUE
    END IF
  END WHILE
  RETURN result
END FUNC"#;

#[rustfmt::skip]
const SLEB128_ENCODE_BODY: &str =
r#"FUNC __encoding_sleb128Encode(value AS Integer) AS List OF Byte
  MUT result AS List OF Byte = []
  MUT remaining AS Integer = value
  MUT chunk AS Integer = 0
  MUT more AS Boolean = TRUE
  MUT signBit AS Integer = 0
  WHILE more
    chunk = bits::band(remaining, 127)
    remaining = bits::sra(remaining, 7)
    signBit = bits::band(chunk, 64)
    IF remaining = 0 AND signBit = 0 THEN
      more = FALSE
    ELSE
      IF remaining = -1 AND signBit <> 0 THEN
        more = FALSE
      END IF
    END IF
    IF more THEN
      result = collections::append(result, toByte(chunk + 128))
    ELSE
      result = collections::append(result, toByte(chunk))
    END IF
  END WHILE
  RETURN result
END FUNC"#;

#[rustfmt::skip]
const SLEB128_DECODE_BODY: &str =
r#"FUNC __encoding_sleb128Decode(data AS List OF Byte) AS Integer
  LET n AS Integer = len(data)
  IF n = 0 THEN
    FAIL error(77050003, "truncated leb128")
  END IF
  MUT result AS Integer = 0
  MUT shift AS Integer = 0
  MUT i AS Integer = 0
  MUT byteValue AS Integer = 0
  MUT done AS Boolean = FALSE
  WHILE done = FALSE
    IF i >= n THEN
      FAIL error(77050003, "truncated leb128")
    END IF
    IF shift > 63 THEN
      FAIL error(77050003, "leb128 overflow")
    END IF
    byteValue = toInt(collections::get(data, i))
    result = bits::bor(result, bits::sl(bits::band(byteValue, 127), shift))
    shift = shift + 7
    i = i + 1
    IF byteValue < 128 THEN
      done = TRUE
      IF shift < 64 AND bits::band(byteValue, 64) <> 0 THEN
        result = bits::bor(result, bits::sl(-1, shift))
      END IF
    END IF
  END WHILE
  RETURN result
END FUNC"#;

#[rustfmt::skip]
const VARINT_ENCODE_BODY: &str =
r#"FUNC __encoding_varintEncode(value AS Integer) AS List OF Byte
  LET zigzag AS Integer = bits::bxor(bits::sl(value, 1), bits::sra(value, 63))
  RETURN __encoding_leb128Emit(zigzag)
END FUNC"#;

#[rustfmt::skip]
const VARINT_DECODE_BODY: &str =
r#"FUNC __encoding_varintDecode(data AS List OF Byte) AS Integer
  LET zigzag AS Integer = __encoding_uleb128Decode(data)
  RETURN bits::bxor(bits::sr(zigzag, 1), 0 - bits::band(zigzag, 1))
END FUNC"#;

// --- authored docs migrated from src/docs/man/builtins/encoding/*.md
// (intro/description/examples; citations stripped). Metadata only.
const INTRO_UTF8_ENCODE: &str = r#"Encode a `String` to its UTF-8 bytes."#;
const DESC_UTF8_ENCODE: &str = r#"`encoding::utf8Encode` returns the UTF-8 encoding of `value` — the exact bytes
that make up the string's storage — one element per byte. Because MFBASIC strings
are always UTF-8 text, the result is the string's raw octets in order, with each
element in the range `0..255`.

The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error. The
byte form is exactly `strings::toBytes(value)`; the integer form contains the
identical numeric values widened to `Integer`.

`utf8Encode` is a **return-type overload**: the same `String` argument produces
either a `List OF Byte` or a `List OF Integer`, chosen by the expected
(contextual) type at the call site. A call with no type context to select the
overload is a compile-time `TYPE_OVERLOAD_AMBIGUOUS` error, not a runtime failure;
the overload is resolved during monomorphization.

The inverse operation is `encoding::utf8Decode`, which accepts either a
`List OF Byte` or a `List OF Integer` and validates it as well-formed UTF-8."#;
const EX_UTF8_ENCODE: &str = r#"Encode a string to raw UTF-8 bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("héllo")
  io::print(toString(len(raw)))
END SUB
```

Encode to the `List OF Integer` form and round-trip it back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8Encode("hi")
  io::print(encoding::utf8Decode(units))
END SUB
```"#;
const INTRO_UTF8_DECODE: &str = r#"Decode a UTF-8 byte or code-unit sequence to a `String`."#;
const DESC_UTF8_DECODE: &str = r#"`encoding::utf8Decode` interprets `value` as a UTF-8 byte sequence and returns the
corresponding text. Because MFBASIC strings are always well-formed UTF-8, the
input is validated in full before the string is produced: `utf8Decode` accepts
only a well-formed UTF-8 sequence and rejects an invalid lead byte, a missing or
stray continuation byte, a truncated multi-byte sequence, an overlong encoding, a
surrogate code point (`U+D800`–`U+DFFF`), and any scalar above `U+10FFFF`. The
empty list decodes to the empty string.


`utf8Decode` is a **parameter overload** selected by the argument's element type:
a `List OF Byte` is decoded directly, while a `List OF Integer` is first checked
element by element — each unit must lie in `0..255` — then decoded. The overload
is resolved during monomorphization, so the selection is a compile-time decision,
not a runtime dispatch.

It is the inverse of `encoding::utf8Encode`: decoding the bytes (or integers)
that `utf8Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
const EX_UTF8_DECODE: &str = r#"Decode raw UTF-8 bytes back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("héllo")
  io::print(encoding::utf8Decode(raw))
END SUB
```

Decode from a `List OF Integer` code-unit list:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = [104, 105]
  io::print(encoding::utf8Decode(units))
END SUB
```"#;
const INTRO_UTF8_ENCODE_BYTES: &str =
    r#"Encode a `String` to its UTF-8 bytes as a `List OF Byte`."#;
const DESC_UTF8_ENCODE_BYTES: &str = r#"`encoding::utf8EncodeBytes` returns the UTF-8 encoding of `value` — the exact
bytes that make up the string's storage — as a `List OF Byte`, one element per
byte. Because MFBASIC strings are always UTF-8 text, the result is the string's
raw octets in order, each element in the range `0..255`. The result is exactly
`strings::toBytes(value)`.

This is the byte-typed form of `encoding::utf8Encode`. `utf8Encode` is a
return-type overload that selects between `List OF Byte` and `List OF Integer`
from the call's contextual type; `utf8EncodeBytes` is the concrete, non-overloaded
name that always yields `List OF Byte`, so no type context is needed to
disambiguate it. The integer-typed counterpart is `encoding::utf8EncodeInts`.


The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error.

The inverse operation is `encoding::utf8DecodeBytes`, which accepts a
`List OF Byte` and validates it as well-formed UTF-8."#;
const EX_UTF8_ENCODE_BYTES: &str = r#"Encode a string to raw UTF-8 bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8EncodeBytes("héllo")
  io::print(toString(len(raw)))
END SUB
```

Round-trip a string through its UTF-8 bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8EncodeBytes("hi")
  io::print(encoding::utf8DecodeBytes(raw))
END SUB
```"#;
const INTRO_UTF8_ENCODE_INTS: &str =
    r#"Encode a `String` to its UTF-8 bytes as a `List OF Integer`."#;
const DESC_UTF8_ENCODE_INTS: &str = r#"`encoding::utf8EncodeInts` returns the UTF-8 encoding of `value` — the exact
bytes that make up the string's storage — as a `List OF Integer`, one element per
byte. Because MFBASIC strings are always UTF-8 text, the result is the string's
raw octets in order, each element widened to `Integer` and in the range `0..255`.
The elements are exactly the values of `strings::toBytes(value)` converted with
`toInt`.

This is the integer-typed form of `encoding::utf8Encode`. `utf8Encode` is a
return-type overload that selects between `List OF Byte` and `List OF Integer`
from the call's contextual type; `utf8EncodeInts` is the concrete, non-overloaded
name that always yields `List OF Integer`, so no type context is needed to
disambiguate it. The byte-typed counterpart is `encoding::utf8EncodeBytes`.


The function is **total**: every string, including the empty string (which yields
an empty list), encodes successfully, and it never raises a runtime error.

The inverse operation is `encoding::utf8DecodeInts`, which accepts a
`List OF Integer` and validates it as well-formed UTF-8."#;
const EX_UTF8_ENCODE_INTS: &str = r#"Encode a string to its UTF-8 code units:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8EncodeInts("héllo")
  io::print(toString(len(units)))
END SUB
```

Round-trip a string through its UTF-8 code units:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8EncodeInts("hi")
  io::print(encoding::utf8DecodeInts(units))
END SUB
```"#;
const INTRO_UTF8_DECODE_BYTES: &str = r#"Decode a `List OF Byte` of UTF-8 octets to a `String`."#;
const DESC_UTF8_DECODE_BYTES: &str = r#"`encoding::utf8DecodeBytes` interprets `value` as a UTF-8 byte sequence and
returns the corresponding text. Because MFBASIC strings are always well-formed
UTF-8, the input is validated in full before the string is produced: the bytes
must form a well-formed UTF-8 sequence, with no invalid, overlong, or truncated
byte sequence. If validation succeeds, the octets are returned verbatim as the
string's storage. The empty list decodes to the empty string.


This is the byte-typed form of `encoding::utf8Decode`. `utf8Decode` is a
parameter overload that selects between a `List OF Byte` and a `List OF Integer`
argument at compile time; `utf8DecodeBytes` is the concrete, non-overloaded name
that always takes a `List OF Byte`, so no overload resolution is involved. The
integer-typed counterpart is `encoding::utf8DecodeInts`, which additionally
requires every element to be in `0..255` before decoding.


It is the inverse of `encoding::utf8EncodeBytes`: decoding the bytes that
`utf8EncodeBytes` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
const EX_UTF8_DECODE_BYTES: &str = r#"Decode raw UTF-8 bytes back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8EncodeBytes("héllo")
  io::print(encoding::utf8DecodeBytes(raw))
END SUB
```

Round-trip a string through its UTF-8 bytes:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8EncodeBytes("hi")
  io::print(encoding::utf8DecodeBytes(raw))
END SUB
```"#;
const INTRO_UTF8_DECODE_INTS: &str =
    r#"Decode a `List OF Integer` of UTF-8 code units to a `String`."#;
const DESC_UTF8_DECODE_INTS: &str = r#"`encoding::utf8DecodeInts` interprets `value` as a UTF-8 byte sequence held one
octet per integer element and returns the corresponding text. Each element is
first range-checked and narrowed to a byte: every unit must lie in `0..255`
(0 through 255 inclusive), and the assembled bytes must form a well-formed UTF-8
sequence, with no invalid, overlong, or truncated byte sequence. If both checks
pass, the octets become the string's storage. The empty list decodes to the
empty string.

This is the integer-typed form of `encoding::utf8Decode`. `utf8Decode` is a
parameter overload that selects between a `List OF Byte` and a `List OF Integer`
argument at compile time; `utf8DecodeInts` is the concrete, non-overloaded name
that always takes a `List OF Integer`, so no overload resolution is involved.
The byte-typed counterpart is `encoding::utf8DecodeBytes`, which takes a
`List OF Byte` and therefore performs no per-element range check.


It is the inverse of `encoding::utf8EncodeInts`: decoding the integers that
`utf8EncodeInts` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
const EX_UTF8_DECODE_INTS: &str = r#"Decode UTF-8 code units back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8EncodeInts("héllo")
  io::print(encoding::utf8DecodeInts(units))
END SUB
```

Round-trip a string through its UTF-8 code units:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf8EncodeInts("hi")
  io::print(encoding::utf8DecodeInts(units))
END SUB
```"#;
const INTRO_UTF16_ENCODE: &str = r#"Encode a `String` to its UTF-16 code units."#;
const DESC_UTF16_ENCODE: &str = r#"`encoding::utf16Encode` returns the UTF-16 encoding of `value` as a list of
numeric code units, one element per 16-bit unit. Each Unicode scalar in `value`
is examined in order: a scalar in the Basic Multilingual Plane (`0..65535`)
becomes a single code unit, and an astral scalar (above `65535`) is split into a
surrogate pair — a high surrogate in `55296..56319` followed by a low surrogate
in `56320..57343`.

The surrogate split subtracts `65536` from the scalar, then takes the top ten
bits (offset by `55296`) as the high unit and the low ten bits (offset by
`56320`) as the low unit, so every returned element lies in `0..65535`.


These are UTF-16 *code units*, not a byte serialization: the result is a
sequence of numbers, so no byte order (endianness) or byte-order mark applies.
The function is **total** — every string, including the empty string (which
yields an empty list), encodes successfully, and it never raises a runtime
error. The inverse operation is `encoding::utf16Decode`, which turns a
`List OF Integer` of code units back into a `String` and rejects unpaired
surrogates and out-of-range units."#;
const EX_UTF16_ENCODE: &str = r#"Encode a string to its UTF-16 code units:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf16Encode("hello")
  io::print(toString(len(units)))
END SUB
```

Round-trip an astral scalar (an emoji) through UTF-16:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf16Encode("😀")
  io::print(encoding::utf16Decode(units))
END SUB
```"#;
const INTRO_UTF16_DECODE: &str =
    r#"Decode a `List OF Integer` of UTF-16 code units to a `String`."#;
const DESC_UTF16_DECODE: &str = r#"`encoding::utf16Decode` interprets `value` as a sequence of UTF-16 code units and
returns the corresponding text. Each element is examined in order: a unit in the
Basic Multilingual Plane decodes to a single Unicode scalar, while a high
surrogate in `55296..56319` is combined with the following low surrogate in
`56320..57343` to reconstruct one astral scalar. The empty list decodes to the
empty string.

A surrogate pair is recombined by subtracting the surrogate offsets, shifting the
high unit up by ten bits, adding the low ten bits, and adding `65536`, yielding a
scalar above `65535`.

Every element must lie in `0..65535`; a value outside that range is rejected. A
high surrogate that is the last element, or is followed by a unit that is not a
low surrogate, is an unpaired surrogate, as is a low surrogate that does not
follow a high surrogate — all of these fail rather than producing replacement
text. The units are treated as numeric code units, not a byte serialization, so
no byte order (endianness) or byte-order mark applies.


`utf16Decode` is the inverse of `encoding::utf16Encode`: decoding the code units
that `utf16Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
const EX_UTF16_DECODE: &str = r#"Decode UTF-16 code units back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::utf16Decode([104, 105]))
END SUB
```

Round-trip an astral scalar (an emoji) through UTF-16:

```
IMPORT encoding
IMPORT io

SUB main()
  LET units AS List OF Integer = encoding::utf16Encode("😀")
  io::print(encoding::utf16Decode(units))
END SUB
```"#;
const INTRO_UTF32_ENCODE: &str = r#"Encode a `String` to its UTF-32 code points."#;
const DESC_UTF32_ENCODE: &str = r#"`encoding::utf32Encode` returns the UTF-32 encoding of `value` as a list of
numeric code points, one element per Unicode scalar value. Each scalar is a
number in the range `0..1114111` (`0x10FFFF`); because a valid `String` holds no
surrogate scalars, the result never contains a value in the surrogate range
`55296..57343`.

The scalars are produced by decoding the string's UTF-8 bytes in order: each
1-to-4-byte sequence contributes exactly one code point, so the returned list
has one element per Unicode scalar in `value` (which may be fewer than its byte
length).

These are UTF-32 *code points*, not a byte serialization: the result is a
sequence of numbers, so no byte order (endianness) or byte-order mark applies.
The function is **total** — every string, including the empty string (which
yields an empty list), encodes successfully, and it never raises a runtime
error. The inverse operation is `encoding::utf32Decode`, which turns a
`List OF Integer` of code points back into a `String` and rejects out-of-range
or surrogate code points."#;
const EX_UTF32_ENCODE: &str = r#"Encode a string to its UTF-32 code points:

```
IMPORT encoding
IMPORT io

SUB main()
  LET points AS List OF Integer = encoding::utf32Encode("hello")
  io::print(toString(len(points)))
END SUB
```

Round-trip an astral scalar (an emoji) through UTF-32:

```
IMPORT encoding
IMPORT io

SUB main()
  LET points AS List OF Integer = encoding::utf32Encode("😀")
  io::print(encoding::utf32Decode(points))
END SUB
```"#;
const INTRO_UTF32_DECODE: &str =
    r#"Decode a `List OF Integer` of UTF-32 code points to a `String`."#;
const DESC_UTF32_DECODE: &str = r#"`encoding::utf32Decode` interprets `value` as a sequence of UTF-32 code points
and returns the corresponding text. Each element is a full Unicode scalar value:
because UTF-32 is a fixed-width encoding, one list element decodes directly to
one scalar, with no multi-unit sequences or surrogate pairs to combine. The empty
list decodes to the empty string.

Every element must be a valid Unicode scalar. A code point is rejected when it is
negative or greater than `1114111` (`0x10FFFF`), or when it lies in the surrogate
range `55296..57343` (`0xD800..0xDFFF`) — surrogates are not scalar values and
cannot appear on their own in UTF-32. Any such element fails rather than
producing replacement text. The elements are treated as numeric code points, not
a byte serialization, so no byte order (endianness) or byte-order mark applies.


`utf32Decode` is the inverse of `encoding::utf32Encode`: decoding the code points
that `utf32Encode` produced reconstructs the original string, and any string
round-trips losslessly through the two functions."#;
const EX_UTF32_DECODE: &str = r#"Decode UTF-32 code points back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::utf32Decode([104, 105]))
END SUB
```

Round-trip an astral scalar (an emoji) through UTF-32:

```
IMPORT encoding
IMPORT io

SUB main()
  LET points AS List OF Integer = encoding::utf32Encode("😀")
  io::print(encoding::utf32Decode(points))
END SUB
```"#;
const INTRO_HEX_ENCODE: &str = r#"Encode a `List OF Byte` to a lowercase hexadecimal `String`."#;
const DESC_HEX_ENCODE: &str = r#"`encoding::hexEncode` returns the base-16 representation of `data`, emitting two
lowercase hexadecimal characters for every input byte with no separators, prefix,
or padding. Bytes are encoded in order: byte value `v` becomes the digit for
`v / 16` followed by the digit for the low nibble, drawn from the alphabet
`0123456789abcdef`.

The result length is always exactly twice the number of input bytes. An empty
list yields the empty string. Use `strings::upper` on the result if uppercase hex
is required.

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. The inverse operation
is `encoding::hexDecode`, which parses a hex string (accepting upper- or
lowercase digits) back into a `List OF Byte`."#;
const EX_HEX_ENCODE: &str = r#"Encode bytes to lowercase hex:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::hexEncode(raw))
END SUB
```

Round-trip through `hexDecode`, and uppercase the digits:

```
IMPORT encoding
IMPORT strings
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  LET hex AS String = encoding::hexEncode(raw)
  io::print(strings::upper(hex))
  io::print(encoding::utf8Decode(encoding::hexDecode(hex)))
END SUB
```"#;
const INTRO_HEX_DECODE: &str = r#"Decode a hexadecimal `String` into a `List OF Byte`."#;
const DESC_HEX_DECODE: &str = r#"`encoding::hexDecode` parses `text` as base-16 and returns the bytes it encodes.
Every two hexadecimal characters produce one byte: the first character is the
high nibble and the second is the low nibble, so the byte value is
`high * 16 + low`. Characters are consumed in order with no separators, prefix,
or padding recognized.

Both cases are accepted for the letter digits: `0`–`9`, `a`–`f`, and `A`–`F` are
valid, and lowercase and uppercase may be mixed freely within the same string.
Any other character is rejected.

The input length must be even, because each byte needs a pair of digits. The
empty string decodes to the empty list. The result always contains exactly half
as many bytes as there are input characters. This is the inverse of
`encoding::hexEncode`, which emits lowercase hex; decoding then re-encoding a
valid string reproduces its lowercase form."#;
const EX_HEX_DECODE: &str = r#"Decode a hex string to bytes and back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::hexDecode("68656c6c6f")
  io::print(encoding::utf8Decode(bytes))
END SUB
```

Round-trip through `hexEncode`, mixing digit case on input:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  LET hex AS String = encoding::hexEncode(raw)
  io::print(hex)
  io::print(encoding::utf8Decode(encoding::hexDecode("6869")))
END SUB
```"#;
const INTRO_BASE32_ENCODE: &str = r#"Encode a `List OF Byte` to a standard Base32 `String`."#;
const DESC_BASE32_ENCODE: &str = r#"`encoding::base32Encode` returns the standard Base32 representation of `data`
as defined by RFC 4648 §6. Input bytes are consumed as a continuous bit stream,
most-significant bit first, and emitted five bits at a time; each 5-bit group
selects one character from the uppercase alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZ234567`.

Encoding operates on 40-bit (5-byte) groups, each producing eight Base32
characters. When the final group is short, its remaining bits become the high
bits of a last symbol and are zero-filled at the low end, then the output is
padded with `=` characters until its length is a multiple of eight, so the
result length is always a multiple of eight. An empty list yields the empty
string.

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. The inverse operation
is `encoding::base32Decode`, which parses a Base32 string back into a
`List OF Byte`."#;
const EX_BASE32_ENCODE: &str = r#"Encode bytes to Base32:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::base32Encode(raw))
END SUB
```

Round-trip through `base32Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base32Encode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base32Decode(text)))
END SUB
```"#;
const INTRO_BASE32_DECODE: &str = r#"Decode a standard Base32 `String` into a `List OF Byte`."#;
const DESC_BASE32_DECODE: &str = r#"`encoding::base32Decode` parses `text` as standard Base32 (RFC 4648 §6) and
returns the bytes it encodes. Each character selects a 5-bit value from the
alphabet `ABCDEFGHIJKLMNOPQRSTUVWXYZ234567`; the values are concatenated
most-significant bit first into a continuous bit stream and emitted eight bits at
a time, so leftover bits that do not fill a final byte are discarded. This is the
inverse of `encoding::base32Encode`.

Decoding is case-insensitive: `A`–`Z` and `a`–`z` map to the same values `0`–`25`,
and the digits `2`–`7` map to `26`–`31`. The `=` character is treated as padding
and may appear only as a trailing run; once a `=` is seen, any later non-padding
character is rejected. Padding characters are otherwise ignored and do not
contribute bits.

The total input length (including padding) must be a multiple of eight
characters. In addition, the number of non-padding symbols must correspond to a
valid Base32 group boundary: a symbol count whose remainder modulo eight is `1`,
`3`, or `6` cannot occur in any well-formed Base32 encoding and is rejected. The
empty string decodes to the empty list."#;
const EX_BASE32_DECODE: &str = r#"Decode a Base32 string back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::base32Decode("NBSWY3DP")
  io::print(encoding::utf8Decode(bytes))
END SUB
```

Round-trip through `base32Encode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base32Encode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base32Decode(text)))
END SUB
```"#;
const INTRO_BASE64_ENCODE: &str = r#"Encode a `List OF Byte` to a standard Base64 `String`."#;
const DESC_BASE64_ENCODE: &str = r#"`encoding::base64Encode` returns the standard Base64 representation of `data`
as defined by RFC 4648 §4. Input bytes are consumed as a continuous bit stream,
most-significant bit first, and emitted six bits at a time; each 6-bit group
selects one character from the alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`, so the
result uses `+` and `/` for the final two symbols.

Encoding operates on 24-bit (3-byte) groups, each producing four Base64
characters. When the final group is short, the remaining data bits occupy the
high-order bits of the last symbol and the low-order bits are zero-filled, and
the output is then padded with `=` characters until its length is a multiple of
four, so the result length is always a multiple of four. An empty list yields
the empty string.

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. For the URL- and
filename-safe variant that uses `-` and `_` without `=` padding, use
`encoding::base64UrlEncode`. The inverse operation is `encoding::base64Decode`,
which parses a Base64 string back into a `List OF Byte`."#;
const EX_BASE64_ENCODE: &str = r#"Encode bytes to Base64:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::base64Encode(raw))
END SUB
```

Round-trip through `base64Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base64Encode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base64Decode(text)))
END SUB
```"#;
const INTRO_BASE64_DECODE: &str = r#"Decode a standard Base64 `String` into a `List OF Byte`."#;
const DESC_BASE64_DECODE: &str = r#"`encoding::base64Decode` parses `text` as standard Base64 (RFC 4648 §4) and
returns the bytes it encodes. Each character selects a 6-bit value from the
alphabet `ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/`; the
values are concatenated most-significant bit first into a continuous bit stream
and emitted eight bits at a time, so leftover bits that do not fill a final byte
are discarded. This is the inverse of `encoding::base64Encode`.

The alphabet is the standard variant using `+` and `/` for values `62` and `63`;
it is case-sensitive (`A`–`Z` map to `0`–`25`, `a`–`z` to `26`–`51`, `0`–`9` to
`52`–`61`). The `=` character is treated as padding: once a `=` is seen, any
later non-padding character is rejected. Padding characters are otherwise ignored
and contribute no bits.

The total input length (including padding) must be a multiple of four
characters. In addition, the number of non-padding symbols cannot be exactly one
more than a multiple of four (a symbol count whose remainder modulo four is `1`),
because no well-formed Base64 group ends on a single 6-bit symbol. The empty
string decodes to the empty list. For the URL- and filename-safe variant that
uses `-` and `_`, use `encoding::base64UrlDecode`."#;
const EX_BASE64_DECODE: &str = r#"Decode a Base64 string back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::base64Decode("aGVsbG8=")
  io::print(encoding::utf8Decode(bytes))
END SUB
```

Round-trip through `base64Encode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base64Encode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base64Decode(text)))
END SUB
```"#;
const INTRO_BASE64_URL_ENCODE: &str =
    r#"Encode a `List OF Byte` to a URL- and filename-safe Base64 `String`."#;
const DESC_BASE64_URL_ENCODE: &str = r#"`encoding::base64UrlEncode` returns the URL- and filename-safe Base64
representation of `data` as defined by RFC 4648 §5. Input bytes are consumed as
a continuous bit stream, most-significant bit first, and emitted six bits at a
time; each 6-bit group selects one character from the alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_`, so the
result uses `-` and `_` for the final two symbols instead of the `+` and `/`
used by the standard variant.

Encoding operates on 24-bit (3-byte) groups, each producing four Base64
characters. When the final group is short, the remaining data bits occupy the
high-order bits of the last symbol and the low-order bits are zero-filled, but
**no** `=` padding characters are appended, so the output length is not rounded
up to a multiple of four. This is the difference from `encoding::base64Encode`,
which pads with `=`. An empty list yields the empty
string.

The function is **total**: every `List OF Byte`, including the empty list,
encodes successfully, and it never raises a runtime error. The inverse
operation is `encoding::base64UrlDecode`, which parses a URL-safe Base64 string
back into a `List OF Byte`."#;
const EX_BASE64_URL_ENCODE: &str = r#"Encode bytes to URL-safe Base64:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hi")
  io::print(encoding::base64UrlEncode(raw))
END SUB
```

Round-trip through `base64UrlDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base64UrlEncode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base64UrlDecode(text)))
END SUB
```"#;
const INTRO_BASE64_URL_DECODE: &str =
    r#"Decode a URL- and filename-safe Base64 `String` into a `List OF Byte`."#;
const DESC_BASE64_URL_DECODE: &str = r#"`encoding::base64UrlDecode` parses `text` as URL- and filename-safe Base64
(RFC 4648 §5) and returns the bytes it encodes. Each character selects a 6-bit
value from the alphabet
`ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_`; the values
are concatenated most-significant bit first into a continuous bit stream and
emitted eight bits at a time, so leftover bits that do not fill a final byte are
discarded. This is the inverse of `encoding::base64UrlEncode`.

The alphabet is the URL-safe variant using `-` and `_` for values `62` and `63`;
it is case-sensitive (`A`–`Z` map to `0`–`25`, `a`–`z` to `26`–`51`, `0`–`9` to
`52`–`61`). The `=` character is treated as padding: once a `=` is seen, any
later non-padding character is rejected. Padding characters are otherwise ignored
and contribute no bits.

Unlike `encoding::base64Decode`, this function does **not** require the total
input length to be a multiple of four, so URL-safe text produced without `=`
padding decodes directly; text that does carry `=` padding is also accepted. The
only length constraint is that the number of non-padding symbols cannot be
exactly one more than a multiple of four (a symbol count whose remainder modulo
four is `1`), because no well-formed Base64 group ends on a single 6-bit symbol.
The empty string decodes to the empty list. For the standard variant that uses
`+` and `/`, use `encoding::base64Decode`."#;
const EX_BASE64_URL_DECODE: &str = r#"Decode a URL-safe Base64 string (no padding) back to text:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::base64UrlDecode("aGVsbG8")
  io::print(encoding::utf8Decode(bytes))
END SUB
```

Round-trip through `base64UrlEncode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET raw AS List OF Byte = encoding::utf8Encode("hello")
  LET text AS String = encoding::base64UrlEncode(raw)
  io::print(text)
  io::print(encoding::utf8Decode(encoding::base64UrlDecode(text)))
END SUB
```"#;
const INTRO_PERCENT_ENCODE: &str = r#"Percent-encode (URL-encode) a `String` per RFC 3986."#;
const DESC_PERCENT_ENCODE: &str = r#"`encoding::percentEncode` percent-encodes `text` following the RFC 3986 rules for
the *unreserved* character set. The input is first converted to its UTF-8 byte
sequence, then each byte is emitted in order.

A byte passes through unchanged when it is a member of the unreserved set:
the ASCII letters `A`–`Z` (65–90) and `a`–`z` (97–122), the digits `0`–`9`
(48–57), and the four marks `-` (45), `.` (46), `_` (95), and `~` (126). Every
other byte — including space, reserved and sub-delimiter characters, control
bytes, and every continuation byte of a multi-byte UTF-8 character — is emitted
as a three-character escape `%XX`, where `XX` is the byte value in **uppercase**
hexadecimal.

Because non-ASCII characters are encoded from their UTF-8 bytes, a single such
character expands to one `%XX` escape per byte (two escapes for most Latin and
symbol characters, three or four for higher code points). The function is
**total**: every `String`, including the empty string (which yields the empty
string), encodes successfully and it never raises a runtime error.

The inverse operation is `encoding::percentDecode`, which parses `%XX` escapes
back into text. For `application/x-www-form-urlencoded` data, where space is
encoded as `+`, use `encoding::formUrlEncode` instead."#;
const EX_PERCENT_ENCODE: &str = r#"Encode a path segment containing reserved characters:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::percentEncode("a b/c"))
END SUB
```

Round-trip through `percentDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET enc AS String = encoding::percentEncode("café & tea")
  io::print(enc)
  io::print(encoding::percentDecode(enc))
END SUB
```"#;
const INTRO_PERCENT_DECODE: &str =
    r#"Decode a percent-encoded (URL-encoded) `String` back into text."#;
const DESC_PERCENT_DECODE: &str = r#"`encoding::percentDecode` reverses `encoding::percentEncode`, expanding every
`%XX` escape in `text` back into the byte it names. The input is scanned as its
raw byte sequence: each `%` (byte 37) introduces a two-digit hexadecimal escape
whose value becomes a single output byte, and every other byte is copied through
unchanged. The accumulated bytes are then interpreted as UTF-8 to produce the
returned `String`.

The two hex digits after a `%` accept either case (`0`–`9`, `a`–`f`, `A`–`F`) and
may be mixed. Unlike `encoding::formUrlDecode`, a literal `+` (byte 43) is *not*
translated to a space — it passes through verbatim — because plus-as-space is an
`application/x-www-form-urlencoded` convention, not part of RFC 3986 percent
encoding.

The empty string decodes to the empty string. The function is a strict decoder:
a `%` with fewer than two following bytes, a `%` followed by a non-hex digit, or
a decoded byte sequence that is not valid UTF-8 all raise an error rather than
being passed through or replaced."#;
const EX_PERCENT_DECODE: &str = r#"Decode a percent-encoded string containing a space escape:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::percentDecode("a%20b"))
END SUB
```

Round-trip through `percentEncode`, including a non-ASCII character:

```
IMPORT encoding
IMPORT io

SUB main()
  LET enc AS String = encoding::percentEncode("café & tea")
  io::print(enc)
  io::print(encoding::percentDecode(enc))
END SUB
```"#;
const INTRO_HTML_ESCAPE: &str = r#"Escape the five HTML/XML metacharacters in a `String`."#;
const DESC_HTML_ESCAPE: &str = r#"`encoding::htmlEscape` produces a form of `text` that is safe to embed inside
HTML/XML element content and attribute values. It replaces each of the five
metacharacters with its named character reference:


- `&` (ampersand) becomes `&amp;`
- `<` (less-than) becomes `&lt;`
- `>` (greater-than) becomes `&gt;`
- `"` (double quote) becomes `&quot;`
- `'` (apostrophe) becomes `&apos;`

The ampersand is substituted **first**, before the other four, so that the `&`
introduced by each replacement entity is not escaped a second time; the result
is therefore a single, correct level of escaping.


Every other character — including whitespace, digits, letters, and non-ASCII
code points — passes through unchanged; only the five characters above are
rewritten. The function is **total**: every `String`, including the empty
string (which yields the empty string), escapes successfully, and it never
raises a runtime error.

The inverse operation is `encoding::htmlUnescape`, which parses named and
numeric character references back into text."#;
const EX_HTML_ESCAPE: &str = r#"Escape a fragment before placing it in element content:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::htmlEscape("<a href='#'>Tom & Jerry</a>"))
END SUB
```

Round-trip through `htmlUnescape`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET esc AS String = encoding::htmlEscape("5 > 3 & 2 < 4")
  io::print(esc)
  io::print(encoding::htmlUnescape(esc))
END SUB
```"#;
const INTRO_HTML_UNESCAPE: &str =
    r#"Decode HTML/XML named and numeric character references in a `String` back to text."#;
const DESC_HTML_UNESCAPE: &str = r#"`encoding::htmlUnescape` scans `text` grapheme by grapheme and replaces each
character reference — a run that begins with `&` and ends at the next `;` — with
the character it denotes. Every other character, including `&` characters that
are part of a valid reference's expansion, passes through unchanged.


Three reference forms are recognized, distinguished by the text between `&`
and `;`:

- A **hexadecimal numeric** reference `&#x…;` or `&#X…;` (for example
  `&#xE9;`), where the digits after `#x`/`#X` are parsed as base 16.

- A **decimal numeric** reference `&#…;` (for example `&#233;`), where the
  digits after `#` are parsed as base 10.

- A **named** reference `&…;` (for example `&eacute;`), looked up in the
  built-in entity table.

The resolved code point is emitted as UTF-8 text. Any code point in the range
`0`–`1114111` (`0x10FFFF`) is accepted, including surrogate values, which are
not screened out.

The function is **not total**: it fails on a reference that has no `;`
terminator, on a numeric reference whose digits are empty or non-numeric, on an
unknown entity name, and on a numeric reference whose value exceeds `1114111`.
The empty string yields the empty string. `encoding::htmlUnescape` is the
inverse of `encoding::htmlEscape`."#;
const EX_HTML_UNESCAPE: &str = r#"Decode named references:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::htmlUnescape("&lt;a&gt;"))
END SUB
```

Decode decimal and hexadecimal numeric references:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::htmlUnescape("caf&#233; / caf&#xE9;"))
END SUB
```

Round-trip through `htmlEscape`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET esc AS String = encoding::htmlEscape("5 > 3 & 2 < 4")
  io::print(encoding::htmlUnescape(esc))
END SUB
```"#;
const INTRO_FORM_URL_ENCODE: &str =
    r#"Encode a `String` as `application/x-www-form-urlencoded` data."#;
const DESC_FORM_URL_ENCODE: &str = r#"`encoding::formUrlEncode` encodes `text` using the
`application/x-www-form-urlencoded` rules that HTML forms apply to query-string
values. The input is first converted to its UTF-8 byte sequence, then each byte
is emitted in order.

A byte passes through unchanged only when it is an ASCII alphanumeric: the
letters `A`–`Z` (65–90) and `a`–`z` (97–122) and the digits `0`–`9` (48–57).
The space byte (32) is emitted as a single `+`. Every other byte — including
`-`, `.`, `_`, `~`, reserved and sub-delimiter characters, control bytes, and
every continuation byte of a multi-byte UTF-8 character — is emitted as a
three-character escape `%XX`, where `XX` is the byte value in **uppercase**
hexadecimal.

This differs from `encoding::percentEncode`, which leaves the four unreserved
marks `-`, `.`, `_`, and `~` untouched and escapes space as `%20` rather than
`+`. Because non-ASCII characters are encoded from their UTF-8 bytes, a single
such character expands to one `%XX` escape per byte (two escapes for most Latin
and symbol characters, three or four for higher code points).

The function is **total**: every `String`, including the empty string (which
yields the empty string), encodes successfully and it never raises a runtime
error. The inverse operation is `encoding::formUrlDecode`, which parses `%XX`
escapes and `+` back into text."#;
const EX_FORM_URL_ENCODE: &str = r#"Encode a form field value containing a space and reserved characters:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::formUrlEncode("name = a b & c"))
END SUB
```

Round-trip through `formUrlDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET enc AS String = encoding::formUrlEncode("café & tea")
  io::print(enc)
  io::print(encoding::formUrlDecode(enc))
END SUB
```"#;
const INTRO_FORM_URL_DECODE: &str =
    r#"Decode `application/x-www-form-urlencoded` text back into a `String`."#;
const DESC_FORM_URL_DECODE: &str = r#"`encoding::formUrlDecode` reverses `encoding::formUrlEncode`, parsing
`application/x-www-form-urlencoded` data — the format HTML forms apply to
query-string values — back into text. The input is read as its UTF-8 byte
sequence and scanned left to right, producing a sequence of decoded bytes.


Each byte is handled as follows:

- A `%` (byte 37) begins a three-character escape `%XX`, where `XX` is two
  hexadecimal digits. The two digits are decoded (case-insensitively) into a
  single byte and the scan advances past all three characters.
- A `+` (byte 43) is replaced by a single space (byte 32). This is the one
  behavior that distinguishes form decoding from `encoding::percentDecode`,
  which leaves `+` untouched.
- Every other byte is copied through unchanged.

After the whole input has been decoded, the resulting byte sequence is
validated as UTF-8 and returned as a `String`. The empty string decodes to the
empty string. Hexadecimal digits in escapes may be upper- or lowercase."#;
const EX_FORM_URL_DECODE: &str = r#"Decode a form field value, turning `+` into a space:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::formUrlDecode("name+%3D+a+b"))
END SUB
```

Round-trip through `formUrlEncode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET enc AS String = encoding::formUrlEncode("café & tea")
  io::print(enc)
  io::print(encoding::formUrlDecode(enc))
END SUB
```"#;
const INTRO_PUNYCODE_ENCODE: &str = r#"Encode a Unicode hostname to its ASCII Punycode form."#;
const DESC_PUNYCODE_ENCODE: &str = r#"`encoding::punycodeEncode` converts a Unicode hostname `domain` to the ASCII
representation used by internationalized domain names (IDNA), applying the
Punycode Bootstring algorithm of RFC 3492. The hostname is split on `.` into
labels, and each label is processed independently; the results are rejoined with
`.` so the dot structure of the input is preserved.

Each label is examined for non-ASCII code points. A label whose code points are
all below `128` is emitted verbatim, unchanged. A label containing any code
point at or above `128` is Punycode-encoded and prefixed with the ACE marker
`xn--`, producing the standard `xn--<encoding>` form.

Within an encoded label, the basic (ASCII) code points are copied out first,
followed by a `-` delimiter when any basic code points are present, and then the
generalized variable-length integers that describe the non-ASCII code points in
ascending order. The algorithm uses the RFC 3492 parameters (initial `n` = 128,
initial bias 72, base 36) and the standard bias-adaptation function. The input
`String` is decoded to Unicode scalar values through the package's UTF-8
decoder before encoding.

The function is **total**: every `String`, including the empty string and
all-ASCII hostnames, encodes successfully, and it never raises a runtime error.
The inverse operation is `encoding::punycodeDecode`, which converts an ASCII
Punycode hostname back to its Unicode form."#;
const EX_PUNYCODE_ENCODE: &str = r#"Encode a Unicode hostname to Punycode:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::punycodeEncode("bücher.example"))
END SUB
```

Round-trip through `punycodeDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET ace AS String = encoding::punycodeEncode("münchen.de")
  io::print(ace)
  io::print(encoding::punycodeDecode(ace))
END SUB
```"#;
const INTRO_PUNYCODE_DECODE: &str =
    r#"Decode an ASCII Punycode hostname back to its Unicode form."#;
const DESC_PUNYCODE_DECODE: &str = r#"`encoding::punycodeDecode` converts an ASCII hostname in the internationalized
domain name (IDNA) representation back to Unicode, reversing the Punycode
Bootstring algorithm of RFC 3492. It is the inverse of
`encoding::punycodeEncode`.

The hostname is split on `.` into labels, and each label is processed
independently; the results are rejoined with `.` so the dot structure of the
input is preserved. A label that begins with the ACE marker `xn--` is decoded:
the `xn--` prefix is stripped and the remainder is run through the Punycode
label decoder. A label without the `xn--` prefix is emitted verbatim, unchanged.


Within an encoded label, the basic (ASCII) code points up to and including the
last `-` delimiter are copied out first, and the trailing generalized
variable-length integers are decoded to reconstruct the non-ASCII code points and
their insertion positions. The decoder uses the RFC 3492 parameters (initial
`n` = 128, initial bias 72, base 36) and the standard bias-adaptation function.
The reconstructed code points are re-encoded to a UTF-8 `String` on return.


The input is expected to be well-formed Punycode. Malformed input — a basic
(pre-delimiter) byte at or above `128`, a variable-length integer that is
truncated before it terminates, a byte that is not a valid base-36 digit, or a
decoded scalar value outside the Unicode range — raises a runtime error rather
than producing a partial result."#;
const EX_PUNYCODE_DECODE: &str = r#"Decode a Punycode label to Unicode:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(encoding::punycodeDecode("xn--mnchen-3ya.de"))
END SUB
```

Round-trip through `punycodeEncode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET ace AS String = encoding::punycodeEncode("bücher.example")
  io::print(ace)
  io::print(encoding::punycodeDecode(ace))
END SUB
```"#;
const INTRO_ULEB128_ENCODE: &str =
    r#"Encode a non-negative `Integer` as an unsigned LEB128 `List OF Byte`."#;
const DESC_ULEB128_ENCODE: &str = r#"`encoding::uleb128Encode` returns the unsigned [LEB128](https://en.wikipedia.org/wiki/LEB128)
representation of `value`, a base-128 little-endian variable-length encoding.
The value is split into 7-bit groups, least-significant group first. Each output
byte carries one group in its low seven bits; the high bit (`0x80`) is set on
every byte except the last, where it is clear, marking the end of the sequence.


At least one byte is always emitted: `0` encodes as the single byte `[0]`.
Because groups are produced until the remaining value reaches zero, the output
length grows by one byte for every additional seven significant bits — for
example values in `0`–`127` produce one byte, `128`–`16383` produce two bytes,
and so on.

`value` must be non-negative; unsigned LEB128 has no representation for negative
numbers. Use `encoding::sleb128Encode` for signed values. The inverse operation
is `encoding::uleb128Decode`, which reads one unsigned LEB128 sequence back into
an `Integer`."#;
const EX_ULEB128_ENCODE: &str = r#"Encode a value and round-trip it back through `uleb128Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::uleb128Encode(624485)
  io::print(toString(encoding::uleb128Decode(bytes)))
END SUB
```

Small values fit in a single byte:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(toString(len(encoding::uleb128Encode(0))))
  io::print(toString(len(encoding::uleb128Encode(127))))
  io::print(toString(len(encoding::uleb128Encode(128))))
END SUB
```"#;
const INTRO_ULEB128_DECODE: &str =
    r#"Decode an unsigned LEB128 `List OF Byte` back into an `Integer`."#;
const DESC_ULEB128_DECODE: &str = r#"`encoding::uleb128Decode` reads one unsigned [LEB128](https://en.wikipedia.org/wiki/LEB128)
sequence from `data` and returns the `Integer` it represents. It is the inverse
of `encoding::uleb128Encode`.

Bytes are consumed least-significant group first. The low seven bits of each
byte contribute the next 7-bit group; the high bit (`0x80`) is the continuation
flag. Decoding accumulates groups — shifting each successive group left by seven
more bits — and stops at the first byte whose high bit is clear (byte value
below `128`), which terminates the sequence. Any bytes after that terminator are
ignored.

`data` must contain at least one byte, and the sequence must be terminated
within it: if the bytes run out before a byte with a clear high bit is seen, the
input is treated as truncated. The accumulated shift may not exceed 63 bits;
a sequence encoding more than 64 significant bits overflows. `data` carries only
magnitude, so the result is always non-negative — use `encoding::sleb128Decode`
for signed values."#;
const EX_ULEB128_DECODE: &str = r#"Round-trip a value through `uleb128Encode` and back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::uleb128Encode(624485)
  io::print(toString(encoding::uleb128Decode(bytes)))
END SUB
```

Decode a literal two-byte sequence (`300` = `[0xAC, 0x02]`):

```
IMPORT encoding
IMPORT collections
IMPORT io

SUB main()
  MUT bytes AS List OF Byte = []
  bytes = collections::append(bytes, toByte(172))
  bytes = collections::append(bytes, toByte(2))
  io::print(toString(encoding::uleb128Decode(bytes)))
END SUB
```"#;
const INTRO_SLEB128_ENCODE: &str =
    r#"Encode a signed `Integer` as a signed LEB128 `List OF Byte`."#;
const DESC_SLEB128_ENCODE: &str = r#"`encoding::sleb128Encode` returns the signed [LEB128](https://en.wikipedia.org/wiki/LEB128)
representation of `value`, a base-128 little-endian variable-length encoding
that carries the sign. The value is split into 7-bit groups, least-significant
group first. Each output byte holds one group in its low seven bits; the high
bit (`0x80`) is set on every byte except the last, where it is clear, marking
the end of the sequence.

Unlike unsigned LEB128, encoding continues by arithmetic (sign-extending) shift
rather than logical shift: after each group `value` is shifted right by seven
bits with the sign preserved. The sequence terminates only when the remaining
bits are all sign bits *and* the sign bit of the final group (`0x40`) matches —
that is, when the remaining value is `0` and the group's sign bit is clear, or
the remaining value is `-1` and the group's sign bit is set. This guarantees the
top byte sign-extends correctly on decode.

At least one byte is always emitted: `0` encodes as the single byte `[0]` and
`-1` encodes as the single byte `[0x7F]`. Both non-negative and negative values
are accepted; use `encoding::uleb128Encode` when the value is known to be
non-negative and the sign byte is unwanted. The inverse operation is
`encoding::sleb128Decode`, which reads one signed LEB128 sequence back into an
`Integer`."#;
const EX_SLEB128_ENCODE: &str = r#"Encode a value and round-trip it back through `sleb128Decode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::sleb128Encode(-123456)
  io::print(toString(encoding::sleb128Decode(bytes)))
END SUB
```

Small values fit in a single byte:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(toString(len(encoding::sleb128Encode(0))))
  io::print(toString(len(encoding::sleb128Encode(-1))))
  io::print(toString(len(encoding::sleb128Encode(-64))))
END SUB
```"#;
const INTRO_SLEB128_DECODE: &str =
    r#"Decode a signed LEB128 `List OF Byte` back into an `Integer`."#;
const DESC_SLEB128_DECODE: &str = r#"`encoding::sleb128Decode` reads one signed [LEB128](https://en.wikipedia.org/wiki/LEB128)
sequence from `data` and returns the `Integer` it represents. It is the inverse
of `encoding::sleb128Encode`.

Bytes are consumed least-significant group first. The low seven bits of each
byte contribute the next 7-bit group; the high bit (`0x80`) is the continuation
flag. Decoding accumulates groups — shifting each successive group left by seven
more bits — and stops at the first byte whose high bit is clear (byte value
below `128`), which terminates the sequence. Any bytes after that terminator are
ignored.

Unlike `encoding::uleb128Decode`, the terminating group carries the sign. When
the final byte's sign bit (`0x40`) is set and the accumulated shift is still
below `64`, the result is sign-extended by filling every higher bit with ones, so
the value decodes as negative. A clear `0x40` leaves the value non-negative. This
mirrors the arithmetic (sign-extending) shift used by `encoding::sleb128Encode`.


`data` must contain at least one byte, and the sequence must be terminated
within it: if the bytes run out before a byte with a clear high bit is seen, the
input is treated as truncated. The accumulated shift may not exceed `63` bits;
a sequence encoding more than 64 significant bits overflows."#;
const EX_SLEB128_DECODE: &str = r#"Round-trip a signed value through `sleb128Encode` and back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::sleb128Encode(-123456)
  io::print(toString(encoding::sleb128Decode(bytes)))
END SUB
```

Decode a single terminating byte whose `0x40` sign bit is set (`-2` = `[0x7E]`):

```
IMPORT encoding
IMPORT collections
IMPORT io

SUB main()
  MUT bytes AS List OF Byte = []
  bytes = collections::append(bytes, toByte(126))
  io::print(toString(encoding::sleb128Decode(bytes)))
END SUB
```"#;
const INTRO_VARINT_ENCODE: &str = r#"Encode a signed `Integer` as a ZigZag varint `List OF Byte`."#;
const DESC_VARINT_ENCODE: &str = r#"`encoding::varintEncode` returns the ZigZag [varint](https://protobuf.dev/programming-guides/encoding/#varints)
representation of `value`. It first maps the signed value onto an unsigned one
with ZigZag encoding — `(value << 1) XOR (value >> 63)`, an arithmetic
right shift — so that small-magnitude negatives become small unsigned numbers,
then writes that unsigned result as base-128 [LEB128](https://en.wikipedia.org/wiki/LEB128).


The ZigZag mapping interleaves signs: `0` maps to `0`, `-1` to `1`, `1` to `2`,
`-2` to `3`, and so on. The mapped value is then split into 7-bit groups,
least-significant group first. Each output byte carries one group in its low
seven bits; the high bit (`0x80`) is set on every byte except the last, where it
is clear, marking the end of the sequence. Because the intermediate value is
shifted right logically, encoding always terminates and at least one byte is
always emitted: `0` encodes as the single byte `[0]`.


Unlike `encoding::uleb128Encode`, `value` may be negative — ZigZag gives every
signed value a compact unsigned form, so no value is rejected. The inverse
operation is `encoding::varintDecode`, which reads one ZigZag varint sequence
back into a signed `Integer`."#;
const EX_VARINT_ENCODE: &str = r#"Encode a signed value and round-trip it back through `varintDecode`:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::varintEncode(-75)
  io::print(toString(encoding::varintDecode(bytes)))
END SUB
```

Small-magnitude values, positive or negative, fit in a single byte:

```
IMPORT encoding
IMPORT io

SUB main()
  io::print(toString(len(encoding::varintEncode(0))))
  io::print(toString(len(encoding::varintEncode(-1))))
  io::print(toString(len(encoding::varintEncode(63))))
END SUB
```"#;
const INTRO_VARINT_DECODE: &str =
    r#"Decode a ZigZag varint `List OF Byte` back into a signed `Integer`."#;
const DESC_VARINT_DECODE: &str = r#"`encoding::varintDecode` reads one ZigZag [varint](https://protobuf.dev/programming-guides/encoding/#varints)
sequence from `data` and returns the signed `Integer` it represents. It is the
inverse of `encoding::varintEncode`.

Decoding proceeds in two steps. First the bytes are read as an unsigned
[LEB128](https://en.wikipedia.org/wiki/LEB128) sequence — least-significant 7-bit
group first, with the high bit (`0x80`) of each byte marking continuation and the
first byte with a clear high bit terminating the sequence. Then the ZigZag
mapping is reversed — `(u >> 1) XOR -(u AND 1)` — turning the unsigned value back
into the original signed value, so that small-magnitude negatives round-trip
correctly. Because the ZigZag reversal is pure arithmetic on the decoded value,
it never fails on its own; every error surfaces from the underlying LEB128 read.


`data` must contain at least one byte, and the sequence must be terminated within
it: if the bytes run out before a byte with a clear high bit is seen, the input
is treated as truncated. The accumulated shift may not exceed 63 bits; a sequence
encoding more than 64 significant bits overflows. Any bytes after the terminator
are ignored."#;
const EX_VARINT_DECODE: &str = r#"Round-trip a signed value through `varintEncode` and back:

```
IMPORT encoding
IMPORT io

SUB main()
  LET bytes AS List OF Byte = encoding::varintEncode(-75)
  io::print(toString(encoding::varintDecode(bytes)))
END SUB
```

Decode a literal two-byte sequence (`-75` = `[0x95, 0x01]`):

```
IMPORT encoding
IMPORT collections
IMPORT io

SUB main()
  MUT bytes AS List OF Byte = []
  bytes = collections::append(bytes, toByte(149))
  bytes = collections::append(bytes, toByte(1))
  io::print(toString(encoding::varintDecode(bytes)))
END SUB
```"#;

const ENCODING_FUNCTIONS: &[BuiltinFunction] = &[
    // The two overloaded names: Custom implementation (resolved by the resolver).
    ef(
        UTF8_ENCODE,
        "utf8Encode",
        &[ov(&[p("value", VALTEXT, "String")], BYTES)],
        Implementation::Custom,
    )
    .with_intro(INTRO_UTF8_ENCODE)
    .with_desc(DESC_UTF8_ENCODE)
    .with_example(EX_UTF8_ENCODE),
    ef(
        UTF8_DECODE,
        "utf8Decode",
        &[ov(&[p("value", &[], BYTES)], "String")],
        Implementation::Custom,
    )
    .with_intro(INTRO_UTF8_DECODE)
    .with_desc(DESC_UTF8_DECODE)
    .with_example(EX_UTF8_DECODE),
    // The 4 monomorph targets.
    ef(
        UTF8_ENCODE_BYTES,
        "utf8EncodeBytes",
        &[ov(&[p("value", &[], "String")], BYTES)],
        Implementation::Mfb {
            body: UTF8_ENCODE_BYTES_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_UTF8_ENCODE_BYTES)
    .with_desc(DESC_UTF8_ENCODE_BYTES)
    .with_example(EX_UTF8_ENCODE_BYTES),
    ef(
        UTF8_ENCODE_INTS,
        "utf8EncodeInts",
        &[ov(&[p("value", &[], "String")], INTS)],
        Implementation::Mfb {
            body: UTF8_ENCODE_INTS_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_UTF8_ENCODE_INTS)
    .with_desc(DESC_UTF8_ENCODE_INTS)
    .with_example(EX_UTF8_ENCODE_INTS),
    ef(
        UTF8_DECODE_BYTES,
        "utf8DecodeBytes",
        &[ov(&[p("value", &[], BYTES)], "String")],
        Implementation::Mfb {
            body: UTF8_DECODE_BYTES_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_UTF8_DECODE_BYTES)
    .with_desc(DESC_UTF8_DECODE_BYTES)
    .with_example(EX_UTF8_DECODE_BYTES),
    ef(
        UTF8_DECODE_INTS,
        "utf8DecodeInts",
        &[ov(&[p("value", &[], INTS)], "String")],
        Implementation::Mfb {
            body: UTF8_DECODE_INTS_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_UTF8_DECODE_INTS)
    .with_desc(DESC_UTF8_DECODE_INTS)
    .with_example(EX_UTF8_DECODE_INTS),
    // Non-overloaded codecs.
    ef(
        UTF16_ENCODE,
        "utf16Encode",
        &[ov(&[p("value", VALTEXT, "String")], INTS)],
        Implementation::Mfb {
            body: UTF16_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_UTF16_ENCODE)
    .with_desc(DESC_UTF16_ENCODE)
    .with_example(EX_UTF16_ENCODE),
    ef(
        UTF16_DECODE,
        "utf16Decode",
        &[ov(&[p("value", &[], INTS)], "String")],
        Implementation::Mfb {
            body: UTF16_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_UTF16_DECODE)
    .with_desc(DESC_UTF16_DECODE)
    .with_example(EX_UTF16_DECODE),
    ef(
        UTF32_ENCODE,
        "utf32Encode",
        &[ov(&[p("value", VALTEXT, "String")], INTS)],
        Implementation::Mfb {
            body: UTF32_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_UTF32_ENCODE)
    .with_desc(DESC_UTF32_ENCODE)
    .with_example(EX_UTF32_ENCODE),
    ef(
        UTF32_DECODE,
        "utf32Decode",
        &[ov(&[p("value", &[], INTS)], "String")],
        Implementation::Mfb {
            body: UTF32_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_UTF32_DECODE)
    .with_desc(DESC_UTF32_DECODE)
    .with_example(EX_UTF32_DECODE),
    ef(
        HEX_ENCODE,
        "hexEncode",
        &[ov(&[p("data", &[], BYTES)], "String")],
        Implementation::Mfb {
            body: HEX_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_HEX_ENCODE)
    .with_desc(DESC_HEX_ENCODE)
    .with_example(EX_HEX_ENCODE),
    ef(
        HEX_DECODE,
        "hexDecode",
        &[ov(&[p("text", &[], "String")], BYTES)],
        Implementation::Mfb {
            body: HEX_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_HEX_DECODE)
    .with_desc(DESC_HEX_DECODE)
    .with_example(EX_HEX_DECODE),
    ef(
        BASE32_ENCODE,
        "base32Encode",
        &[ov(&[p("data", &[], BYTES)], "String")],
        Implementation::Mfb {
            body: BASE32_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_BASE32_ENCODE)
    .with_desc(DESC_BASE32_ENCODE)
    .with_example(EX_BASE32_ENCODE),
    ef(
        BASE32_DECODE,
        "base32Decode",
        &[ov(&[p("text", &[], "String")], BYTES)],
        Implementation::Mfb {
            body: BASE32_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_BASE32_DECODE)
    .with_desc(DESC_BASE32_DECODE)
    .with_example(EX_BASE32_DECODE),
    ef(
        BASE64_ENCODE,
        "base64Encode",
        &[ov(&[p("data", &[], BYTES)], "String")],
        Implementation::Mfb {
            body: BASE64_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_BASE64_ENCODE)
    .with_desc(DESC_BASE64_ENCODE)
    .with_example(EX_BASE64_ENCODE),
    ef(
        BASE64_DECODE,
        "base64Decode",
        &[ov(&[p("text", &[], "String")], BYTES)],
        Implementation::Mfb {
            body: BASE64_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_BASE64_DECODE)
    .with_desc(DESC_BASE64_DECODE)
    .with_example(EX_BASE64_DECODE),
    ef(
        BASE64URL_ENCODE,
        "base64UrlEncode",
        &[ov(&[p("data", &[], BYTES)], "String")],
        Implementation::Mfb {
            body: BASE64_URL_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_BASE64_URL_ENCODE)
    .with_desc(DESC_BASE64_URL_ENCODE)
    .with_example(EX_BASE64_URL_ENCODE),
    ef(
        BASE64URL_DECODE,
        "base64UrlDecode",
        &[ov(&[p("text", &[], "String")], BYTES)],
        Implementation::Mfb {
            body: BASE64_URL_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_BASE64_URL_DECODE)
    .with_desc(DESC_BASE64_URL_DECODE)
    .with_example(EX_BASE64_URL_DECODE),
    ef(
        PERCENT_ENCODE,
        "percentEncode",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: PERCENT_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_PERCENT_ENCODE)
    .with_desc(DESC_PERCENT_ENCODE)
    .with_example(EX_PERCENT_ENCODE),
    ef(
        PERCENT_DECODE,
        "percentDecode",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: PERCENT_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_PERCENT_DECODE)
    .with_desc(DESC_PERCENT_DECODE)
    .with_example(EX_PERCENT_DECODE),
    ef(
        HTML_ESCAPE,
        "htmlEscape",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: HTML_ESCAPE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_HTML_ESCAPE)
    .with_desc(DESC_HTML_ESCAPE)
    .with_example(EX_HTML_ESCAPE),
    ef(
        HTML_UNESCAPE,
        "htmlUnescape",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: HTML_UNESCAPE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_HTML_UNESCAPE)
    .with_desc(DESC_HTML_UNESCAPE)
    .with_example(EX_HTML_UNESCAPE),
    ef(
        FORM_URL_ENCODE,
        "formUrlEncode",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: FORM_URL_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_FORM_URL_ENCODE)
    .with_desc(DESC_FORM_URL_ENCODE)
    .with_example(EX_FORM_URL_ENCODE),
    ef(
        FORM_URL_DECODE,
        "formUrlDecode",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: FORM_URL_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_FORM_URL_DECODE)
    .with_desc(DESC_FORM_URL_DECODE)
    .with_example(EX_FORM_URL_DECODE),
    ef(
        PUNYCODE_ENCODE,
        "punycodeEncode",
        &[ov(&[p("domain", &[], "String")], "String")],
        Implementation::Mfb {
            body: PUNYCODE_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_PUNYCODE_ENCODE)
    .with_desc(DESC_PUNYCODE_ENCODE)
    .with_example(EX_PUNYCODE_ENCODE),
    ef(
        PUNYCODE_DECODE,
        "punycodeDecode",
        &[ov(&[p("asciiDomain", &[], "String")], "String")],
        Implementation::Mfb {
            body: PUNYCODE_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_PUNYCODE_DECODE)
    .with_desc(DESC_PUNYCODE_DECODE)
    .with_example(EX_PUNYCODE_DECODE),
    ef(
        ULEB128_ENCODE,
        "uleb128Encode",
        &[ov(&[p("value", &[], "Integer")], BYTES)],
        Implementation::Mfb {
            body: ULEB128_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_ULEB128_ENCODE)
    .with_desc(DESC_ULEB128_ENCODE)
    .with_example(EX_ULEB128_ENCODE),
    ef(
        ULEB128_DECODE,
        "uleb128Decode",
        &[ov(&[p("data", &[], BYTES)], "Integer")],
        Implementation::Mfb {
            body: ULEB128_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_ULEB128_DECODE)
    .with_desc(DESC_ULEB128_DECODE)
    .with_example(EX_ULEB128_DECODE),
    ef(
        SLEB128_ENCODE,
        "sleb128Encode",
        &[ov(&[p("value", &[], "Integer")], BYTES)],
        Implementation::Mfb {
            body: SLEB128_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_SLEB128_ENCODE)
    .with_desc(DESC_SLEB128_ENCODE)
    .with_example(EX_SLEB128_ENCODE),
    ef(
        SLEB128_DECODE,
        "sleb128Decode",
        &[ov(&[p("data", &[], BYTES)], "Integer")],
        Implementation::Mfb {
            body: SLEB128_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_SLEB128_DECODE)
    .with_desc(DESC_SLEB128_DECODE)
    .with_example(EX_SLEB128_DECODE),
    ef(
        VARINT_ENCODE,
        "varintEncode",
        &[ov(&[p("value", &[], "Integer")], BYTES)],
        Implementation::Mfb {
            body: VARINT_ENCODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_VARINT_ENCODE)
    .with_desc(DESC_VARINT_ENCODE)
    .with_example(EX_VARINT_ENCODE),
    ef(
        VARINT_DECODE,
        "varintDecode",
        &[ov(&[p("data", &[], BYTES)], "Integer")],
        Implementation::Mfb {
            body: VARINT_DECODE_BODY,
            fast_path: None,
        },
    )
    .with_intro(INTRO_VARINT_DECODE)
    .with_desc(DESC_VARINT_DECODE)
    .with_example(EX_VARINT_DECODE),
];

/// Argument-dependent resolution for encoding: `resolve_call` validation and the
/// overloaded `utf8Encode`/`utf8Decode` monomorph-target selection. Both delegate
/// to the retained `dispatch_*` helpers.
struct EncodingResolver;
impl BuiltinResolver for EncodingResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_resolve(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }

    fn resolve_overload_target(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
        expected_type: Option<&str>,
    ) -> Result<Option<String>, ()> {
        dispatch_overload_target(name, arg_types, expected_type).map(|opt| opt.map(str::to_string))
    }
}
static ENCODING_RESOLVER: EncodingResolver = EncodingResolver;

pub(crate) static ENCODING: BuiltinModule = BuiltinModule {
    name: "encoding",
    doc_intro: "",
    doc_desc: "",
    functions: ENCODING_FUNCTIONS,
    types: &[],
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&ENCODING_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_encoding_call(name: &str) -> bool {
    DefaultResolver::contains(&ENCODING, name)
}

// `call_param_names`/`expected_arguments`/`argument_types` return `&'static`
// borrowed shapes the owned `DefaultResolver` cannot produce (and the latter two
// use bespoke phrasing). They stay static: `call_param_names` PINNED equal to
// `ENCODING` by the parity test; the others verified by the existing tests. BB
// removes them.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        UTF8_ENCODE | UTF16_ENCODE | UTF32_ENCODE | PERCENT_ENCODE | PERCENT_DECODE
        | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE | FORM_URL_DECODE => {
            Some(&[&["value", "text"]])
        }
        UTF8_DECODE | UTF16_DECODE | UTF32_DECODE => Some(&[&["value"]]),
        HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE => Some(&[&["data"]]),
        HEX_DECODE | BASE32_DECODE | BASE64_DECODE | BASE64URL_DECODE => Some(&[&["text"]]),
        PUNYCODE_ENCODE => Some(&[&["domain"]]),
        PUNYCODE_DECODE => Some(&[&["asciiDomain"]]),
        ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE => Some(&[&["value"]]),
        ULEB128_DECODE | SLEB128_DECODE | VARINT_DECODE => Some(&[&["data"]]),
        _ => None,
    }
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        UTF8_ENCODE | UTF8_ENCODE_BYTES | UTF8_ENCODE_INTS | UTF16_ENCODE | UTF32_ENCODE
        | PERCENT_ENCODE | PERCENT_DECODE | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE
        | FORM_URL_DECODE | PUNYCODE_ENCODE | PUNYCODE_DECODE | HEX_DECODE | BASE32_DECODE
        | BASE64_DECODE | BASE64URL_DECODE => Some("String"),
        UTF8_DECODE => Some("List OF Byte or List OF Integer"),
        UTF8_DECODE_BYTES => Some(BYTES),
        UTF8_DECODE_INTS | UTF16_DECODE | UTF32_DECODE => Some(INTS),
        HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE | ULEB128_DECODE
        | SLEB128_DECODE | VARINT_DECODE => Some(BYTES),
        ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE => Some("Integer"),
        _ => None,
    }
}

/// The machine-readable positional argument-type signature (bug-340 A1). Every
/// `encoding::` member is unary, so each entry is a one-element slice — except
/// `utf8Decode`, which is overloaded on `List OF Byte | List OF Integer` and so
/// has no single positional signature (`None`, as before). IR lowering reads this
/// directly instead of parsing the `expected_arguments` diagnostic string.
pub(crate) fn argument_types(name: &str) -> Option<&'static [&'static str]> {
    match name {
        UTF8_ENCODE | UTF8_ENCODE_BYTES | UTF8_ENCODE_INTS | UTF16_ENCODE | UTF32_ENCODE
        | PERCENT_ENCODE | PERCENT_DECODE | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE
        | FORM_URL_DECODE | PUNYCODE_ENCODE | PUNYCODE_DECODE | HEX_DECODE | BASE32_DECODE
        | BASE64_DECODE | BASE64URL_DECODE => Some(&["String"]),
        UTF8_DECODE => None,
        UTF8_DECODE_BYTES => Some(&[BYTES]),
        UTF8_DECODE_INTS | UTF16_DECODE | UTF32_DECODE => Some(&[INTS]),
        HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE | ULEB128_DECODE
        | SLEB128_DECODE | VARINT_DECODE => Some(&[BYTES]),
        ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE => Some(&["Integer"]),
        _ => None,
    }
}

/// The argument-validating return-type resolution, invoked through the descriptor
/// resolver by `resolve_call`. Every member is unary; `utf8Decode` accepts either
/// `List OF Byte` or `List OF Integer`.
fn dispatch_resolve<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    let arg = arg_types[0].as_str();
    let return_type: Cow<'a, str> = match name {
        // utf8Encode: String -> List OF Byte | List OF Integer (return overload).
        // Resolved precisely via the expected type; default to List OF Byte here.
        UTF8_ENCODE if arg == "String" => Cow::Borrowed(BYTES),
        UTF8_ENCODE_BYTES if arg == "String" => Cow::Borrowed(BYTES),
        UTF8_ENCODE_INTS if arg == "String" => Cow::Borrowed(INTS),
        UTF8_DECODE if arg == BYTES || arg == INTS => Cow::Borrowed("String"),
        UTF8_DECODE_BYTES if arg == BYTES => Cow::Borrowed("String"),
        UTF8_DECODE_INTS if arg == INTS => Cow::Borrowed("String"),
        UTF16_ENCODE | UTF32_ENCODE if arg == "String" => Cow::Borrowed(INTS),
        UTF16_DECODE | UTF32_DECODE if arg == INTS => Cow::Borrowed("String"),
        HEX_ENCODE | BASE32_ENCODE | BASE64_ENCODE | BASE64URL_ENCODE if arg == BYTES => {
            Cow::Borrowed("String")
        }
        HEX_DECODE | BASE32_DECODE | BASE64_DECODE | BASE64URL_DECODE if arg == "String" => {
            Cow::Borrowed(BYTES)
        }
        PERCENT_ENCODE | PERCENT_DECODE | HTML_ESCAPE | HTML_UNESCAPE | FORM_URL_ENCODE
        | FORM_URL_DECODE | PUNYCODE_ENCODE | PUNYCODE_DECODE
            if arg == "String" =>
        {
            Cow::Borrowed("String")
        }
        ULEB128_ENCODE | SLEB128_ENCODE | VARINT_ENCODE if arg == "Integer" => Cow::Borrowed(BYTES),
        ULEB128_DECODE | SLEB128_DECODE | VARINT_DECODE if arg == BYTES => Cow::Borrowed("Integer"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

/// The internal `__encoding_*` symbol each non-overloaded public member (and the
/// four `utf8Encode`/`utf8Decode` monomorph targets) rewrites to during IR
/// lowering. These members now carry [`Implementation::Mfb`], whose descriptor
/// `implementation_name` is `None` (the body is assembled into the injected
/// package rather than named by a fixed rewrite symbol), so the rewrite target is
/// provided here explicitly. The two overloaded names (`utf8Encode`/`utf8Decode`,
/// `Implementation::Custom`) are absent: they resolve to a concrete target via
/// `resolve_overload_target` first, and that target then rewrites through here.
const IMPL_NAMES: &[(&str, &str)] = &[
    ("encoding.utf8EncodeBytes", "__encoding_utf8EncodeBytes"),
    ("encoding.utf8EncodeInts", "__encoding_utf8EncodeInts"),
    ("encoding.utf8DecodeBytes", "__encoding_utf8DecodeBytes"),
    ("encoding.utf8DecodeInts", "__encoding_utf8DecodeInts"),
    ("encoding.utf16Encode", "__encoding_utf16Encode"),
    ("encoding.utf16Decode", "__encoding_utf16Decode"),
    ("encoding.utf32Encode", "__encoding_utf32Encode"),
    ("encoding.utf32Decode", "__encoding_utf32Decode"),
    ("encoding.hexEncode", "__encoding_hexEncode"),
    ("encoding.hexDecode", "__encoding_hexDecode"),
    ("encoding.base32Encode", "__encoding_base32Encode"),
    ("encoding.base32Decode", "__encoding_base32Decode"),
    ("encoding.base64Encode", "__encoding_base64Encode"),
    ("encoding.base64Decode", "__encoding_base64Decode"),
    ("encoding.base64UrlEncode", "__encoding_base64UrlEncode"),
    ("encoding.base64UrlDecode", "__encoding_base64UrlDecode"),
    ("encoding.percentEncode", "__encoding_percentEncode"),
    ("encoding.percentDecode", "__encoding_percentDecode"),
    ("encoding.htmlEscape", "__encoding_htmlEscape"),
    ("encoding.htmlUnescape", "__encoding_htmlUnescape"),
    ("encoding.formUrlEncode", "__encoding_formUrlEncode"),
    ("encoding.formUrlDecode", "__encoding_formUrlDecode"),
    ("encoding.punycodeEncode", "__encoding_punycodeEncode"),
    ("encoding.punycodeDecode", "__encoding_punycodeDecode"),
    ("encoding.uleb128Encode", "__encoding_uleb128Encode"),
    ("encoding.uleb128Decode", "__encoding_uleb128Decode"),
    ("encoding.sleb128Encode", "__encoding_sleb128Encode"),
    ("encoding.sleb128Decode", "__encoding_sleb128Decode"),
    ("encoding.varintEncode", "__encoding_varintEncode"),
    ("encoding.varintDecode", "__encoding_varintDecode"),
];

pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    IMPL_NAMES
        .iter()
        .find(|(public, _)| *public == name)
        .map(|(_, internal)| *internal)
}

/// Resolve the overloaded `utf8Encode`/`utf8Decode` public calls to a concrete
/// internal implementation, using the call's argument types and the expected
/// (contextual) type. Returns `Ok(Some(name))` on a unique match, `Ok(None)`
/// when the callee is not an overloaded encoding name, and `Err(())` when a
/// return-type overload cannot be resolved without an expected type
/// (`utf8Encode` with no `List OF Byte`/`List OF Integer` context). Invoked
/// through the descriptor resolver by `builtins::resolve_overload_target`.
fn dispatch_overload_target(
    callee: &str,
    arg_types: &[String],
    expected_type: Option<&str>,
) -> Result<Option<&'static str>, ()> {
    match callee {
        UTF8_ENCODE if arg_types == ["String"] => match expected_type {
            Some(BYTES) => Ok(Some(UTF8_ENCODE_BYTES)),
            Some(INTS) => Ok(Some(UTF8_ENCODE_INTS)),
            _ => Err(()),
        },
        UTF8_DECODE if arg_types == [BYTES] => Ok(Some(UTF8_DECODE_BYTES)),
        UTF8_DECODE if arg_types == [INTS] => Ok(Some(UTF8_DECODE_INTS)),
        _ => Ok(None),
    }
}

/// Whether `callee` is one of the overloaded encoding public names: derived from
/// the descriptor (an overloaded name carries `Implementation::Custom`).
pub(crate) fn is_overloaded(callee: &str) -> bool {
    ENCODING
        .function(callee)
        .is_some_and(|function| matches!(function.implementation, Implementation::Custom))
}

/// Synthetic path label for the injected encoding source. `parse_source_internal`
/// records it as the file path; `AstProject::to_json` filters this sentinel out of
/// `-ast` output. Preserved byte-for-byte from the pre-migration
/// `package_source_glue!` invocation so the injected AST is unchanged.
const SOURCE_LABEL: &str = "<builtin-encoding>";
const SOURCE_DOC: &str = "builtins/encoding.mfb";

/// Parses the built-in `encoding` package source (dual path: the `package.mfb`
/// companion plus every [`Implementation::Mfb`] member's body, spliced in by
/// [`assembled_source`]).
pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        std::path::Path::new(SOURCE_LABEL),
        SOURCE_DOC,
        &assembled_source(),
    )
}

/// The `encoding` package source, assembled from the dual path: the external
/// `package.mfb` companion is the base, and every member carrying
/// [`Implementation::Mfb`] contributes its `FUNC __encoding_<name> ... END FUNC`
/// body in place of a one-line `'@@MFB_BODY:<slug>@@` marker at the body's
/// original position. Splicing at the original position keeps every helper's
/// source line numbers unchanged, so the injected AST — and every derived golden —
/// is byte-identical to the pre-migration companion. Mirrors
/// `collections::assembled_source`.
fn assembled_source() -> String {
    let mut source = String::from(include_str!("package.mfb"));
    for func in ENCODING_FUNCTIONS {
        if let Implementation::Mfb { body, .. } = func.implementation {
            let marker = format!("'@@MFB_BODY:{}@@", func.doc_slug);
            debug_assert!(
                source.contains(&marker),
                "encoding package.mfb is missing the '{marker}' body marker",
            );
            source = source.replacen(&marker, body, 1);
        }
    }
    source
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "encoding")
    })
}

pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    if !uses_package(ast) {
        return Ok(ast.clone());
    }
    let mut augmented = ast.clone();
    augmented.files.push(source_file()?);
    Ok(augmented)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    const ALL_PUBLIC: &[&str] = &[
        UTF8_ENCODE,
        UTF8_DECODE,
        UTF16_ENCODE,
        UTF16_DECODE,
        UTF32_ENCODE,
        UTF32_DECODE,
        HEX_ENCODE,
        HEX_DECODE,
        BASE32_ENCODE,
        BASE32_DECODE,
        BASE64_ENCODE,
        BASE64_DECODE,
        BASE64URL_ENCODE,
        BASE64URL_DECODE,
        PERCENT_ENCODE,
        PERCENT_DECODE,
        HTML_ESCAPE,
        HTML_UNESCAPE,
        FORM_URL_ENCODE,
        FORM_URL_DECODE,
        PUNYCODE_ENCODE,
        PUNYCODE_DECODE,
        ULEB128_ENCODE,
        ULEB128_DECODE,
        SLEB128_ENCODE,
        SLEB128_DECODE,
        VARINT_ENCODE,
        VARINT_DECODE,
    ];

    #[test]
    fn is_call_recognizes_and_rejects() {
        for n in ALL_PUBLIC {
            assert!(is_encoding_call(n), "{n}");
        }
        for n in [
            UTF8_ENCODE_BYTES,
            UTF8_ENCODE_INTS,
            UTF8_DECODE_BYTES,
            UTF8_DECODE_INTS,
        ] {
            assert!(is_encoding_call(n), "{n}");
        }
        assert!(!is_encoding_call("encoding.nope"));
        assert!(!is_encoding_call("other.utf8Encode"));
    }

    #[test]
    fn param_names_branches() {
        assert_eq!(
            call_param_names(UTF8_ENCODE),
            Some(&[&["value", "text"][..]][..])
        );
        assert_eq!(call_param_names(UTF8_DECODE), Some(&[&["value"][..]][..]));
        assert_eq!(call_param_names(HEX_ENCODE), Some(&[&["data"][..]][..]));
        assert_eq!(call_param_names(HEX_DECODE), Some(&[&["text"][..]][..]));
        assert_eq!(
            call_param_names(PUNYCODE_ENCODE),
            Some(&[&["domain"][..]][..])
        );
        assert_eq!(
            call_param_names(PUNYCODE_DECODE),
            Some(&[&["asciiDomain"][..]][..])
        );
        assert_eq!(
            call_param_names(ULEB128_ENCODE),
            Some(&[&["value"][..]][..])
        );
        assert_eq!(call_param_names(ULEB128_DECODE), Some(&[&["data"][..]][..]));
        assert!(call_param_names("encoding.nope").is_none());
    }

    #[test]
    fn expected_arguments_branches() {
        assert_eq!(expected_arguments(UTF8_ENCODE), Some("String"));
        assert_eq!(expected_arguments(HEX_DECODE), Some("String"));
        assert_eq!(
            expected_arguments(UTF8_DECODE),
            Some("List OF Byte or List OF Integer")
        );
        assert_eq!(expected_arguments(UTF8_DECODE_BYTES), Some(BYTES));
        assert_eq!(expected_arguments(UTF8_DECODE_INTS), Some(INTS));
        assert_eq!(expected_arguments(UTF16_DECODE), Some(INTS));
        assert_eq!(expected_arguments(HEX_ENCODE), Some(BYTES));
        assert_eq!(expected_arguments(ULEB128_DECODE), Some(BYTES));
        assert_eq!(expected_arguments(ULEB128_ENCODE), Some("Integer"));
        assert!(expected_arguments("encoding.nope").is_none());
    }

    #[test]
    fn argument_types_machine_table() {
        // bug-340 A1: the machine-readable positional signature IR lowering reads.
        // Every member is unary, so each is a one-element slice — except the
        // overloaded `utf8Decode`, which has no single signature.
        assert_eq!(argument_types(UTF8_ENCODE), Some(&["String"][..]));
        assert_eq!(argument_types(UTF8_DECODE), None);
        assert_eq!(argument_types(UTF8_DECODE_BYTES), Some(&[BYTES][..]));
        assert_eq!(argument_types(UTF32_DECODE), Some(&[INTS][..]));
        assert_eq!(argument_types(HEX_ENCODE), Some(&[BYTES][..]));
        assert_eq!(argument_types(ULEB128_ENCODE), Some(&["Integer"][..]));
        assert!(argument_types("encoding.nope").is_none());
    }

    #[test]
    fn implementation_name_flat_map() {
        assert_eq!(
            implementation_name(UTF8_ENCODE_BYTES),
            Some("__encoding_utf8EncodeBytes")
        );
        assert_eq!(
            implementation_name(UTF8_ENCODE_INTS),
            Some("__encoding_utf8EncodeInts")
        );
        assert_eq!(
            implementation_name(UTF8_DECODE_BYTES),
            Some("__encoding_utf8DecodeBytes")
        );
        assert_eq!(
            implementation_name(UTF8_DECODE_INTS),
            Some("__encoding_utf8DecodeInts")
        );
        assert_eq!(
            implementation_name(HEX_ENCODE),
            Some("__encoding_hexEncode")
        );
        assert_eq!(
            implementation_name(VARINT_DECODE),
            Some("__encoding_varintDecode")
        );
        assert_eq!(
            implementation_name(PUNYCODE_ENCODE),
            Some("__encoding_punycodeEncode")
        );
        assert_eq!(
            implementation_name(FORM_URL_DECODE),
            Some("__encoding_formUrlDecode")
        );
        // overloaded names are not in the flat map
        assert_eq!(implementation_name(UTF8_ENCODE), None);
        assert_eq!(implementation_name(UTF8_DECODE), None);
        assert_eq!(implementation_name("encoding.nope"), None);
    }

    #[test]
    fn resolve_overload_target_all_paths() {
        // Route through the generic descriptor entry point (which delegates to
        // `EncodingResolver::resolve_overload_target`), the same path monomorph
        // uses. Results are owned `String`s.
        let target = |callee: &str, args: &[&str], expected: Option<&str>| {
            crate::builtins::resolve_overload_target(callee, &strings(args), expected)
        };
        assert_eq!(
            target(UTF8_ENCODE, &["String"], Some(BYTES)),
            Ok(Some(UTF8_ENCODE_BYTES.to_string()))
        );
        assert_eq!(
            target(UTF8_ENCODE, &["String"], Some(INTS)),
            Ok(Some(UTF8_ENCODE_INTS.to_string()))
        );
        // no expected type -> Err
        assert_eq!(target(UTF8_ENCODE, &["String"], None), Err(()));
        assert_eq!(target(UTF8_ENCODE, &["String"], Some("String")), Err(()));
        // utf8Encode with wrong arg types is not the overload arm -> Ok(None)
        assert_eq!(target(UTF8_ENCODE, &["Integer"], Some(BYTES)), Ok(None));
        assert_eq!(
            target(UTF8_DECODE, &[BYTES], None),
            Ok(Some(UTF8_DECODE_BYTES.to_string()))
        );
        assert_eq!(
            target(UTF8_DECODE, &[INTS], None),
            Ok(Some(UTF8_DECODE_INTS.to_string()))
        );
        // non-overloaded callee -> Ok(None)
        assert_eq!(target(HEX_ENCODE, &[BYTES], None), Ok(None));
    }

    #[test]
    fn is_overloaded_only_utf8() {
        assert!(is_overloaded(UTF8_ENCODE));
        assert!(is_overloaded(UTF8_DECODE));
        assert!(!is_overloaded(UTF16_ENCODE));
        assert!(!is_overloaded(HEX_ENCODE));
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT encoding\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("a").files.len(),
            ast.files.len() + 1
        );
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("a").files.len(),
            ast.files.len()
        );
    }

    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `p`/`ov`/`ef` are const fns used only in const context, so their
        // bodies never run at runtime and show as uncovered. Call them at
        // runtime to exercise (and pin the shape of) each constructor.
        let param = p("value", VALTEXT, "String");
        assert_eq!(param.name, "value");
        assert_eq!(param.aliases, VALTEXT);
        assert_eq!(param.ty, ParameterType::Named("String"));
        assert_eq!(param.default, DefaultValue::None);

        // E0716: `ov`/`ef` borrow `&'static` slices, so they must be named consts.
        const PARAMS: &[Parameter] = &[p("value", &[], "String")];
        let overload = ov(PARAMS, BYTES);
        assert_eq!(overload.params.len(), 1);
        assert_eq!(overload.params[0].name, "value");
        assert_eq!(overload.return_type, ReturnType::Fixed(BYTES));

        const OV_CUSTOM: &[BuiltinOverload] = &[ov(&[p("value", VALTEXT, "String")], BYTES)];
        let custom = ef(UTF8_ENCODE, "utf8Encode", OV_CUSTOM, Implementation::Custom);
        assert_eq!(custom.name, UTF8_ENCODE);
        assert_eq!(custom.doc_slug, "utf8Encode");
        assert_eq!(custom.overloads.len(), 1);
        assert_eq!(custom.implementation, Implementation::Custom);
        assert_eq!(custom.lowering, Lowering::Helper);
        assert!(!custom.flags.internal_only);
        assert!(!custom.flags.return_type_overloaded);

        const OV_REWRITE: &[BuiltinOverload] = &[ov(&[p("data", &[], BYTES)], "String")];
        let rewrite = ef(
            HEX_ENCODE,
            "hexEncode",
            OV_REWRITE,
            Implementation::Rewrite("__encoding_hexEncode"),
        );
        assert_eq!(
            rewrite.implementation,
            Implementation::Rewrite("__encoding_hexEncode")
        );
    }

    #[test]
    fn dispatch_resolve_all_branches() {
        // `dispatch_resolve` is reached in production only through the descriptor
        // resolver; call it directly to exercise every return-type arm and the
        // arity guard.
        let ret = |name: &str, args: &[&str]| {
            dispatch_resolve(name, &strings(args)).map(|r| r.return_type.into_owned())
        };
        let resolve = ret;

        // Arity guard: only unary calls resolve.
        assert!(resolve(UTF8_ENCODE, &["String", "String"]).is_none());
        assert!(resolve(UTF8_ENCODE, &[]).is_none());
        // utf8 family (monomorph targets + overloaded names).
        assert_eq!(ret(UTF8_ENCODE, &["String"]).as_deref(), Some(BYTES));
        assert_eq!(ret(UTF8_ENCODE_BYTES, &["String"]).as_deref(), Some(BYTES));
        assert_eq!(ret(UTF8_ENCODE_INTS, &["String"]).as_deref(), Some(INTS));
        assert_eq!(ret(UTF8_DECODE, &[BYTES]).as_deref(), Some("String"));
        assert_eq!(ret(UTF8_DECODE, &[INTS]).as_deref(), Some("String"));
        assert_eq!(ret(UTF8_DECODE_BYTES, &[BYTES]).as_deref(), Some("String"));
        assert_eq!(ret(UTF8_DECODE_INTS, &[INTS]).as_deref(), Some("String"));
        // utf16/utf32.
        assert_eq!(ret(UTF16_ENCODE, &["String"]).as_deref(), Some(INTS));
        assert_eq!(ret(UTF32_ENCODE, &["String"]).as_deref(), Some(INTS));
        assert_eq!(ret(UTF16_DECODE, &[INTS]).as_deref(), Some("String"));
        assert_eq!(ret(UTF32_DECODE, &[INTS]).as_deref(), Some("String"));
        // hex/base32/base64 encode -> String, decode -> Bytes.
        assert_eq!(ret(BASE64_ENCODE, &[BYTES]).as_deref(), Some("String"));
        assert_eq!(ret(BASE64URL_DECODE, &["String"]).as_deref(), Some(BYTES));
        // percent/html/formUrl/punycode String -> String.
        assert_eq!(ret(PERCENT_ENCODE, &["String"]).as_deref(), Some("String"));
        assert_eq!(ret(PUNYCODE_DECODE, &["String"]).as_deref(), Some("String"));
        // leb128/varint.
        assert_eq!(ret(VARINT_ENCODE, &["Integer"]).as_deref(), Some(BYTES));
        assert_eq!(ret(VARINT_DECODE, &[BYTES]).as_deref(), Some("Integer"));

        // Wrong argument type falls through to the `_ => None` arm.
        assert!(resolve(UTF8_ENCODE, &["Integer"]).is_none());
        assert!(resolve("encoding.nope", &["String"]).is_none());
    }
}
