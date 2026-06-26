# plan-02 — Built-in `bits` and `encoding` Packages

Last updated: 2026-06-26

This document is the **normative definition and implementation plan** for two
foundational built-in packages, lowest layer first:

- **`bits`** (§A) — integer bitwise/shift/rotate operations. MFBASIC today has
  *no* bitwise integer ops (the `AND`/`OR`/`XOR` operators are logical/boolean),
  so byte-level codecs and any future bit-twiddling cannot be written without it.
  Each function lowers to a single AArch64 instruction
  (`AND`/`ORR`/`EOR`/`MVN`/`LSLV`/`LSRV`/`RORV` — `RORV` already exists from
  PCG64) and is trivially supported on the Binary Representation (BR) path.
  Independently useful beyond any single consumer.
- **`encoding`** (§B) — hex and Base64/Base64url byte↔text codecs, the Unicode
  transforms (`utf8`/`utf16`/`utf32` encode/decode), and `percent` (URL)
  escaping. Raw `List OF Byte` values (hashes, MACs, keys, random bytes, binary
  file contents) are unusable as text without these, and the stdlib has none
  today. Implemented in source on `bits` (the codecs are trivial and benefit from
  a single uniform implementation incl. Base64url, which host libraries do not
  all expose). The `utf8Encode` `List OF Byte` / `List OF Integer` overload pair
  relies on **return-type-distinguished overload resolution**, a language
  prerequisite specified in `plan-01-overload.md` §F.

Outputs are standardized, so the **native and Binary Representation (BR)
execution paths produce identical results** — a hex string is a hex string and a
left shift is a left shift regardless of which path computed it.

> **Origin.** `bits` and `encoding` were originally specified inside
> `plan-04-crypto.md` as the `crypto` package's two companions. They are split
> out here because both are general-purpose, have no dependency on the
> cryptographic surface, and form the lowest layer of that stack: `crypto`
> depends on `encoding` (for stringifying digests/keys) and on `bits` (for its
> portable software cores), so they are most naturally delivered first and on
> their own. `plan-04-crypto.md` now consumes both from this plan.

It complements:

- `specifications/standard_package.md` §3 (universal `toString`/`toInt`), §10.1
  (the PCG64 RNG, the source of the existing `RORV` encoder)
- `specifications/error_codes.md` (`ErrInvalidArgument` `77050002` and
  `ErrInvalidFormat` `77050003`, both reused — this plan reserves **no** new
  codes; §C)
- `specifications/mfbasic.md` (the reserved logical operators `AND`/`OR`/`XOR`/
  `NOT`; `TRAP`/`RECOVER`/`FAIL`; `qualifiedIdent = ident "::" ident`)
- `specifications/plan-01-overload.md` §F (return-type-distinguished overload
  resolution — the language prerequisite for `utf8Encode`'s overload pair)
- `specifications/plan-03-http.md` / the `csv`/`json` source packages (the
  source-package shim and wiring template `encoding` mirrors)
- `specifications/plan-04-crypto.md` (the downstream consumer of both packages)

---

# Part A — `bits` package

Integer bitwise operations on the 64-bit `Integer`. No types or enums. Each lowers
to a single native instruction inline (like `math::abs`) and is interpreted
directly on the BR path. All are **total** except shift/rotate count validation.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `bits::band` | `FUNC band(a AS Integer, b AS Integer) AS Integer` | Bitwise AND of all 64 bits. |
| `bits::bor` | `FUNC bor(a AS Integer, b AS Integer) AS Integer` | Bitwise OR. |
| `bits::bxor` | `FUNC bxor(a AS Integer, b AS Integer) AS Integer` | Bitwise XOR. |
| `bits::bnot` | `FUNC bnot(a AS Integer) AS Integer` | One's-complement (all 64 bits inverted). |
| `bits::shiftLeft` | `FUNC shiftLeft(value AS Integer, count AS Integer) AS Integer` | Logical left shift; vacated low bits are zero; bits past bit 63 are discarded. Fails `ErrInvalidArgument` (`77050002`) when `count` is outside `0 .. 63`. |
| `bits::shiftRight` | `FUNC shiftRight(value AS Integer, count AS Integer) AS Integer` | **Logical** (unsigned) right shift; vacated high bits are zero. Fails `77050002` when `count` is outside `0 .. 63`. |
| `bits::rotateLeft32` | `FUNC rotateLeft32(value AS Integer, count AS Integer) AS Integer` | Rotate the low 32 bits left by `count MOD 32`; result zero-extended into bits 32..63. |
| `bits::rotateRight32` | `FUNC rotateRight32(value AS Integer, count AS Integer) AS Integer` | Rotate the low 32 bits right by `count MOD 32`; zero-extended. |
| `bits::rotateLeft64` | `FUNC rotateLeft64(value AS Integer, count AS Integer) AS Integer` | Rotate all 64 bits left by `count MOD 64`. |
| `bits::rotateRight64` | `FUNC rotateRight64(value AS Integer, count AS Integer) AS Integer` | Rotate all 64 bits right by `count MOD 64`. |

> The boolean ops are `band`/`bor`/`bxor`/`bnot` because `and`/`or`/`xor`/`not`
> are reserved logical operators (case-insensitive keywords, `mfbasic.md` §…) and
> cannot be package member identifiers (`qualifiedIdent = ident "::" ident`).
> Operands are raw two's-complement bit patterns; `bits` functions do not interpret
> sign. The 32-bit rotates target word-oriented algorithms (e.g. ChaCha20); the
> 64-bit rotates target 64-bit arithmetic cores. The four named rotate variants map
> one-to-one to hardware (`RORV`, 32-/64-bit forms) and avoid a `width` parameter.

---

# Part B — `encoding` package

Byte↔text and Unicode codecs. No types or enums. Decoders fail with
`ErrInvalidFormat` (`77050003`). Pure source on `bits`.

| Function | Signature | Behavior |
|----------|-----------|----------|
| `encoding::utf8Encode` | `FUNC utf8Encode(value AS String) AS List OF Byte` | Encodes `value` to its UTF-8 bytes, one element per byte (`0 .. 255`) — the raw bytes binary/crypto consumers need. Selected when the call's expected result type is `List OF Byte` (return-type overloading, plan-01 §F). Total. |
| `encoding::utf8Encode` | `FUNC utf8Encode(value AS String) AS List OF Integer` | Same UTF-8 bytes as `0 .. 255` Integers, for arithmetic on code units. Selected when the call's expected result type is `List OF Integer`; identical numeric values to the `List OF Byte` form. Total. |
| `encoding::utf8Decode` | `FUNC utf8Decode(value AS List OF Byte) AS String` · `FUNC utf8Decode(value AS List OF Integer) AS String` | Decodes a UTF-8 byte/code-unit sequence to a String (distinct **parameter** types, so ordinary positional overloading selects). Fails `77050003` on an element outside `0 .. 255` or an invalid UTF-8 sequence. |
| `encoding::utf16Encode` | `FUNC utf16Encode(value AS String) AS List OF Integer` | Encodes `value` to UTF-16 code units, one element per 16-bit unit (`0 .. 65535`; astral codepoints become surrogate pairs). Numeric code units, **not** a byte serialization — endianness does not apply. Total. |
| `encoding::utf16Decode` | `FUNC utf16Decode(value AS List OF Integer) AS String` | Decodes a UTF-16 code-unit sequence to a String. Fails `77050003` on an element outside `0 .. 65535` or an unpaired surrogate. |
| `encoding::utf32Encode` | `FUNC utf32Encode(value AS String) AS List OF Integer` | Encodes `value` to UTF-32 code units, one element per Unicode scalar value (`0 .. 0x10FFFF`, surrogates excluded). Total. |
| `encoding::utf32Decode` | `FUNC utf32Decode(value AS List OF Integer) AS String` | Decodes a UTF-32 codepoint sequence to a String. Fails `77050003` on a codepoint outside `0 .. 0x10FFFF` or inside the surrogate range `0xD800 .. 0xDFFF`. |
| `encoding::hexEncode` | `FUNC hexEncode(data AS List OF Byte) AS String` | Lowercase hex (two chars/byte, no separators). Total. `strings::upper` for uppercase. |
| `encoding::hexDecode` | `FUNC hexDecode(text AS String) AS List OF Byte` | Decodes hex (upper or lower). Fails `77050003` on a non-hex character or odd length. |
| `encoding::base64Encode` | `FUNC base64Encode(data AS List OF Byte) AS String` | Standard Base64 (RFC 4648 §4), `+`/`/`, `=` padding. Total. |
| `encoding::base64Decode` | `FUNC base64Decode(text AS String) AS List OF Byte` | Decodes standard Base64; padding required. Fails `77050003` on invalid alphabet/length/padding. |
| `encoding::base64UrlEncode` | `FUNC base64UrlEncode(data AS List OF Byte) AS String` | URL-safe Base64 (RFC 4648 §5), `-`/`_`, **no** padding. Total. |
| `encoding::base64UrlDecode` | `FUNC base64UrlDecode(text AS String) AS List OF Byte` | Decodes URL-safe Base64; accepts input with or without `=` padding. Fails `77050003`. |
| `encoding::percentEncode` | `FUNC percentEncode(text AS String) AS String` | Percent-encodes (URL-encodes) `text` per RFC 3986: the unreserved set `A–Z a–z 0–9 - . _ ~` passes through; every other byte of `text`'s UTF-8 encoding becomes `%XX` with uppercase hex. Total. |
| `encoding::percentDecode` | `FUNC percentDecode(text AS String) AS String` | Decodes percent-encoded `text`, interpreting the decoded bytes as UTF-8. Fails `77050003` on a malformed `%XX` escape or on invalid UTF-8. |

> **Naming directions.** `hex`/`base64` *Encode* serialize raw bytes **to** text
> (and *Decode* back); the Unicode `utf*` *Encode* transform a String **to** its
> code units (and *Decode* back); `percentEncode`/`percentDecode` are String→String
> escape/unescape. The signatures are authoritative — read them for the exact
> input/output types of each call.

---

# Part C — Error codes

This plan reserves **no** new runtime codes. Failures reuse existing codes from
`error_codes.md`. (The `utf8Encode` overload pair relies on the
`TYPE_OVERLOAD_AMBIGUOUS` compile-time diagnostic introduced by
`plan-01-overload.md` §F, not a `7-705` runtime code.)

| Canonical | Integer | Name | Used by |
|-----------|---------|------|---------|
| `7-705-0002` | `77050002` | `ErrInvalidArgument` | `bits::shiftLeft`/`shiftRight` when `count` is outside `0 .. 63`. |
| `7-705-0003` | `77050003` | `ErrInvalidFormat` | `encoding` decode failures: non-hex/odd-length hex; invalid Base64 alphabet/length/padding; malformed `%XX` or bad UTF-8 in `percentDecode`; out-of-range or surrogate code units in the `utf*Decode` family. |

---

# Part D — Implementation Plan

## Phase 0 — `bits` foundation

- **`src/builtins/bits.rs`** (new shim, modeled on `math.rs`): the ten functions,
  arities, param names, `resolve_call` (`Integer`-typed), `implementation_name`.
- **Codegen** in a new `src/target/shared/code/builder_bits.rs` (peer of
  `builder_math.rs`): lower each to a single instruction inline —
  `AND`/`ORR`/`EOR` (register form), `MVN`, `LSLV`, `LSRV`, `RORV` (reuse
  `emit_rorv`; 32-bit rotates use the `W` form, zero-extended). Shift-count
  validation (`0..63`) emits the `ErrInvalidArgument` range check.
- **BR path:** add the ten ops to the BR interpreter's integer op set.
- Tests: per-op golden values incl. sign-bit/boundary counts; native↔BR equality.

> **Prerequisite (plan-01 §F).** `encoding::utf8Encode`'s two overloads need
> **return-type-distinguished overload resolution**, specified and implemented in
> `plan-01-overload.md` §F (resolver duplicate key, typecheck contextual tie-break,
> `TYPE_OVERLOAD_AMBIGUOUS`, the `mfbasic.md` §6 amendment). That work must land
> before Phase 1 here. No overload-resolution work in this plan.

## Phase 1 — `encoding` source package

- **`src/builtins/encoding.rs`** + **`src/builtins/encoding_package.mfb`**
  (source-package idiom from `json.rs`/`csv`). `IMPORT bits`, `IMPORT strings`,
  `IMPORT collections`. Implements the `utf8`/`utf16`/`utf32` encode/decode pairs,
  hex, the Base64 family, and `percent` encode/decode on `bits` + byte lists. No
  codegen. `utf8Encode`'s two return-type overloads rely on the plan-01 §F
  prerequisite above.

## Phase 2 — Man pages

- `mfb man bits`, `mfb man encoding` via the existing
  `man_pages`/`write_pages`/`parse_package` pipeline (`build.rs`, `src/man/mod.rs`).
  Cite RFC 4648 (Base64/Base64url) in `encoding`; note the logical-operator naming
  rationale in `bits`.

## Phase 3 — User documentation

- `standard_package.md`: new sections for `bits` and `encoding` (mirroring §10
  `math` / §12 `json`); note in §10.1 that `bits` supplies the integer bitwise
  operations the operator set intentionally omits.
- `mfbasic.md`: list the two packages; note `bits` provides the integer bitwise
  operations the operator set intentionally omits (because `AND`/`OR`/`XOR`/`NOT`
  are reserved logical operators). (The §6 return-type-overload amendment is owned
  by plan-01 §F.)

## Phase 4 — Tests (golden)

- **Known-answer vectors:** RFC 4648 (Base64 / Base64url, with and without
  padding), hex round-trips, RFC 3986 percent-encoding, and the `utf8`/`utf16`/
  `utf32` codecs against ASCII, 2-/3-byte, and astral (surrogate-pair) codepoints.
- **`bits` golden values:** per-op including sign-bit/boundary shift and rotate
  counts; `MOD`-reduced rotate counts; the two shift-count range failures.
- **Overload resolution (plan-01 §F):** `utf8Encode` resolves to `List OF Byte` vs
  `List OF Integer` by annotation / argument-slot / `RETURN` context; the
  unannotated inferred-`LET` case reports `TYPE_OVERLOAD_AMBIGUOUS`; `utf8Decode`
  selects by parameter type with no annotation needed. (Resolution machinery is
  tested in plan-01; this is the encoding integration case.)
- **Negative:** malformed hex (non-hex char, odd length) → `ErrInvalidFormat`;
  malformed Base64 (bad alphabet, bad length, bad padding) → `ErrInvalidFormat`;
  malformed `%XX` / bad UTF-8 in `percentDecode` and out-of-range or surrogate
  code units in the `utf*Decode` family → `ErrInvalidFormat`; out-of-range shift
  count → `ErrInvalidArgument`.
- **Equality matrices:** native↔BR identical for every `bits` op and every codec;
  round-trip identity (`decode(encode(x)) = x`) across the byte space and over a
  sample of multi-plane Unicode text.

---

# Part E — Worked examples

```basic
IMPORT bits
IMPORT encoding

' Bitwise ops the language operators intentionally omit.
LET masked = bits::band(0xFF00, 0x0FF0)        ' 0x0F00
LET packed = bits::bor(bits::shiftLeft(r, 16), bits::shiftLeft(g, 8))
LET mixed  = bits::rotateLeft32(state, 7)

' Hex and Base64 round-trips on raw bytes. The annotation picks utf8Encode's
' List OF Byte overload; without it the call is TYPE_OVERLOAD_AMBIGUOUS (plan-01 §F.3).
LET raw AS List OF Byte = encoding::utf8Encode("hello world")
io::print(encoding::hexEncode(raw))             ' 68656c6c6f20776f726c64
io::print(encoding::base64Encode(raw))          ' aGVsbG8gd29ybGQ=
io::print(encoding::base64UrlEncode(raw))       ' aGVsbG8gd29ybGQ (no padding)

' Unicode code units and URL escaping.
LET units AS List OF Integer = encoding::utf16Encode("héllo")  ' [104,233,108,108,111]
io::print(encoding::percentEncode("a b/c"))     ' a%20b%2Fc

' Decoders fail closed on malformed input.
LET bytes = encoding::hexDecode("zz") TRAP(e)
  IF e.code = errorCode::ErrInvalidFormat THEN RECOVER []
  FAIL e
END TRAP
```

---

# Part F — Divergences and non-goals

## F.1 Divergences from the source-package template

- `bits` is the first package to add the integer bitwise operations the language
  operators omit (each lowering to a single native instruction inline, like
  `math::abs`), rather than pure source over existing builtins.
- `encoding` is conventional pure source, in the `json`/`csv` mould, built on
  `bits` + byte lists — but it has a **language prerequisite** owned by
  `plan-01-overload.md` §F: the `utf8Encode` overload pair is the first consumer of
  return-type-distinguished overload resolution, a narrow extension of
  `mfbasic.md` §6.

## F.2 Non-goals for this version

- **No additional codecs** (Base32, Base58, quoted-printable, yEnc) — may be added
  later if a consumer needs them.
- **No streaming/incremental codecs** (one-shot byte-list/String API only).
- **No bit-set / bitfield abstractions** in `bits` beyond the ten scalar ops; no
  arbitrary-width or signed (arithmetic) right shift — operands are raw
  two's-complement bit patterns.
- **No text encodings beyond UTF-8/16/32 and percent-escaping** — no Latin-1,
  other codepage transcoding, or punycode/IDNA.
- **No general return-type-based overloading sugar beyond plan-01 §F** — return
  type is a tie-break only among otherwise-indistinguishable candidates and requires
  a known expected type; it does not add implicit conversions or candidate ranking.
