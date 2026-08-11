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

const ENCODING_FUNCTIONS: &[BuiltinFunction] = &[
    // The two overloaded names: Custom implementation (resolved by the resolver).
    ef(
        UTF8_ENCODE,
        "utf8Encode",
        &[ov(&[p("value", VALTEXT, "String")], BYTES)],
        Implementation::Custom,
    ),
    ef(
        UTF8_DECODE,
        "utf8Decode",
        &[ov(&[p("value", &[], BYTES)], "String")],
        Implementation::Custom,
    ),
    // The 4 monomorph targets.
    ef(
        UTF8_ENCODE_BYTES,
        "utf8EncodeBytes",
        &[ov(&[p("value", &[], "String")], BYTES)],
        Implementation::Mfb {
            body: UTF8_ENCODE_BYTES_BODY,
            fast_path: None,
        },
    ),
    ef(
        UTF8_ENCODE_INTS,
        "utf8EncodeInts",
        &[ov(&[p("value", &[], "String")], INTS)],
        Implementation::Mfb {
            body: UTF8_ENCODE_INTS_BODY,
            fast_path: None,
        },
    ),
    ef(
        UTF8_DECODE_BYTES,
        "utf8DecodeBytes",
        &[ov(&[p("value", &[], BYTES)], "String")],
        Implementation::Mfb {
            body: UTF8_DECODE_BYTES_BODY,
            fast_path: None,
        },
    ),
    ef(
        UTF8_DECODE_INTS,
        "utf8DecodeInts",
        &[ov(&[p("value", &[], INTS)], "String")],
        Implementation::Mfb {
            body: UTF8_DECODE_INTS_BODY,
            fast_path: None,
        },
    ),
    // Non-overloaded codecs.
    ef(
        UTF16_ENCODE,
        "utf16Encode",
        &[ov(&[p("value", VALTEXT, "String")], INTS)],
        Implementation::Mfb {
            body: UTF16_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        UTF16_DECODE,
        "utf16Decode",
        &[ov(&[p("value", &[], INTS)], "String")],
        Implementation::Mfb {
            body: UTF16_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        UTF32_ENCODE,
        "utf32Encode",
        &[ov(&[p("value", VALTEXT, "String")], INTS)],
        Implementation::Mfb {
            body: UTF32_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        UTF32_DECODE,
        "utf32Decode",
        &[ov(&[p("value", &[], INTS)], "String")],
        Implementation::Mfb {
            body: UTF32_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        HEX_ENCODE,
        "hexEncode",
        &[ov(&[p("data", &[], BYTES)], "String")],
        Implementation::Mfb {
            body: HEX_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        HEX_DECODE,
        "hexDecode",
        &[ov(&[p("text", &[], "String")], BYTES)],
        Implementation::Mfb {
            body: HEX_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        BASE32_ENCODE,
        "base32Encode",
        &[ov(&[p("data", &[], BYTES)], "String")],
        Implementation::Mfb {
            body: BASE32_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        BASE32_DECODE,
        "base32Decode",
        &[ov(&[p("text", &[], "String")], BYTES)],
        Implementation::Mfb {
            body: BASE32_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        BASE64_ENCODE,
        "base64Encode",
        &[ov(&[p("data", &[], BYTES)], "String")],
        Implementation::Mfb {
            body: BASE64_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        BASE64_DECODE,
        "base64Decode",
        &[ov(&[p("text", &[], "String")], BYTES)],
        Implementation::Mfb {
            body: BASE64_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        BASE64URL_ENCODE,
        "base64UrlEncode",
        &[ov(&[p("data", &[], BYTES)], "String")],
        Implementation::Mfb {
            body: BASE64_URL_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        BASE64URL_DECODE,
        "base64UrlDecode",
        &[ov(&[p("text", &[], "String")], BYTES)],
        Implementation::Mfb {
            body: BASE64_URL_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        PERCENT_ENCODE,
        "percentEncode",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: PERCENT_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        PERCENT_DECODE,
        "percentDecode",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: PERCENT_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        HTML_ESCAPE,
        "htmlEscape",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: HTML_ESCAPE_BODY,
            fast_path: None,
        },
    ),
    ef(
        HTML_UNESCAPE,
        "htmlUnescape",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: HTML_UNESCAPE_BODY,
            fast_path: None,
        },
    ),
    ef(
        FORM_URL_ENCODE,
        "formUrlEncode",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: FORM_URL_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        FORM_URL_DECODE,
        "formUrlDecode",
        &[ov(&[p("value", VALTEXT, "String")], "String")],
        Implementation::Mfb {
            body: FORM_URL_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        PUNYCODE_ENCODE,
        "punycodeEncode",
        &[ov(&[p("domain", &[], "String")], "String")],
        Implementation::Mfb {
            body: PUNYCODE_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        PUNYCODE_DECODE,
        "punycodeDecode",
        &[ov(&[p("asciiDomain", &[], "String")], "String")],
        Implementation::Mfb {
            body: PUNYCODE_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        ULEB128_ENCODE,
        "uleb128Encode",
        &[ov(&[p("value", &[], "Integer")], BYTES)],
        Implementation::Mfb {
            body: ULEB128_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        ULEB128_DECODE,
        "uleb128Decode",
        &[ov(&[p("data", &[], BYTES)], "Integer")],
        Implementation::Mfb {
            body: ULEB128_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        SLEB128_ENCODE,
        "sleb128Encode",
        &[ov(&[p("value", &[], "Integer")], BYTES)],
        Implementation::Mfb {
            body: SLEB128_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        SLEB128_DECODE,
        "sleb128Decode",
        &[ov(&[p("data", &[], BYTES)], "Integer")],
        Implementation::Mfb {
            body: SLEB128_DECODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        VARINT_ENCODE,
        "varintEncode",
        &[ov(&[p("value", &[], "Integer")], BYTES)],
        Implementation::Mfb {
            body: VARINT_ENCODE_BODY,
            fast_path: None,
        },
    ),
    ef(
        VARINT_DECODE,
        "varintDecode",
        &[ov(&[p("data", &[], BYTES)], "Integer")],
        Implementation::Mfb {
            body: VARINT_DECODE_BODY,
            fast_path: None,
        },
    ),
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
