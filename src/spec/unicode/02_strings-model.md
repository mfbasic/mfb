# String Indexing Model

The three indexing units a `String` exposes and which `strings::` operations use
each. A `String` is an immutable, UTF-8-encoded byte sequence; the runtime never
mutates it in place. The unit a builtin counts in is a deliberate, fixed property
of that builtin — callers must know which.

The per-function `strings::` API (arguments, return types, error codes) is owned
by `./mfb man strings`; this topic specifies only the *indexing model and slice
semantics* a faithful reimplementation must reproduce.

## Three indexing units

| Unit | Definition | Backing | Used by |
| --- | --- | --- | --- |
| Scalar | One Unicode scalar value (Rust `char`, a single code point) | `char_indices` / `chars().count()` | `mid` (start + length), `find` (start + return), `scalar_count` |
| Grapheme | One user-perceived character (extended grapheme cluster) | `unicode-segmentation` | `graphemes`, `graphemesCount`, `graphemeAt` |
| Byte | One UTF-8 code unit | `&str` length | `byteLen`, raw slice bounds |

Scalar and grapheme indices differ whenever a cluster spans multiple scalars: a
combining sequence (`"a\u{301}"` = 2 scalars, 1 grapheme), a ZWJ emoji sequence
(`"👨‍👩‍👧‍👦"` = 7 scalars, 1 grapheme), or a regional-indicator flag (`"🇺🇸"` = 2
scalars, 1 grapheme). Byte indices differ from scalar indices for any non-ASCII
scalar (`é` = 2 bytes, `日` = 3, `😀` = 4). [[src/unicode_backend.rs:graphemes]]

## Scalar / byte mapping

The runtime converts between a scalar index and a byte offset on demand; it never
caches a per-scalar offset table. Mapping is the dominant cost in `mid`/`find`,
each direction being O(n) in the bytes scanned.

`byte_offset_for_scalar_index(value, scalar_index)` walks `char_indices` to the
`scalar_index`-th boundary. The one-past-the-end index (`scalar_index ==
scalar_count`) is accepted and maps to `value.len()` (the end-of-string offset);
this is what lets a zero-length `mid` at the end, and `find` at the end, succeed.
Any index strictly past that returns `"scalar index out of range"`.
[[src/unicode_backend.rs:byte_offset_for_scalar_index]]

`scalar_index_for_byte_offset(value, byte_offset)` is the inverse. A byte offset
equal to `value.len()` maps to the full scalar count. Any other offset must fall
on a `char` boundary (`is_char_boundary`); an offset that is out of range or lands
inside a multi-byte scalar returns `"byte offset is not a scalar boundary"`.
[[src/unicode_backend.rs:scalar_index_for_byte_offset]]

```text
value = "a é 日"  (spaces shown for clarity; actual: "aé日")
  scalar idx:  0    1        2
  bytes:      [61] [c3 a9]  [e6 97 a5]
  byte off:    0    1        3          6 (== len, one-past-end)
```

A scalar index is *not* validated against the byte offsets of any other string —
it is purely an ordinal position within `value`. `is_scalar_boundary` is a thin
wrapper over `str::is_char_boundary`. [[src/unicode_backend.rs:is_scalar_boundary]]

## `mid` semantics — scalar slicing

`mid(value, start, length)` slices by **scalar index**, not byte or grapheme.
[[src/unicode_backend.rs:mid]]

- `start` is mapped to a byte offset via `byte_offset_for_scalar_index`.
- `end = start + length`, computed with a checked add; overflow returns
  `"scalar index overflow"`.
- `end` is mapped to a byte offset the same way.
- The result is `value[start_offset..end_offset]` as an owned `String`.

Because both ends go through the one-past-end-accepting mapper:

- `mid(value, scalar_count, 0)` is the empty string (start at end, zero length).
- A `start` past `scalar_count`, or an `end` past `scalar_count`, returns
  `"scalar index out of range"` — `mid` does **not** clamp `length` to the
  remaining scalars. The caller must size `length` to the string.

The byte slice is always taken on scalar boundaries, so the result is always
valid UTF-8. `mid` is *not* grapheme-aware: slicing through a combining sequence
splits the cluster.

## `find` semantics — scalar in, scalar out

`find(value, needle, start)` searches for the first occurrence of `needle` at or
after scalar index `start`, and returns the match position as a **scalar index**.
[[src/unicode_backend.rs:find]]

- `start` maps to a byte offset (`start` may equal `scalar_count`).
- An empty `needle` short-circuits and returns `start` unchanged (the empty
  string is found at the search origin).
- Otherwise a byte-level substring search runs on the suffix
  `value[start_offset..]`; the relative byte hit is added back to `start_offset`
  and converted to a scalar index via `scalar_index_for_byte_offset`.
- No match returns the error `"not found"`.

The underlying `str::find` is byte-exact, so matching is on raw UTF-8 bytes with
**no normalization and no case folding**: `"é"` (NFC, one scalar) does not match
`"e\u{301}"` (NFD, two scalars). Normalize with `strings::normalizeNfc` or fold
with `strings::caseFold` first when logical equality is required. Because the
needle is whole UTF-8, a byte hit always lands on a scalar boundary, so the
back-conversion never fails.

## Unicode-whitespace trimming

The three trims classify a code point with Rust `char::is_whitespace`, which is
the Unicode `White_Space` property — broader than ASCII. This includes the ASCII
set (`\t \n \x0b \x0c \r` space), plus NBSP `U+00A0`, the various Unicode spaces
`U+2000`–`U+200A`, line/paragraph separators `U+2028`/`U+2029`, and ideographic
space `U+3000`, among others. [[src/unicode_backend.rs:is_whitespace]]

| Function | Strips from | Backing |
| --- | --- | --- |
| `trim` | both ends | `str::trim_matches(is_whitespace)` |
| `trim_start` | leading | `str::trim_start_matches(is_whitespace)` |
| `trim_end` | trailing | `str::trim_end_matches(is_whitespace)` |

Trimming operates scalar by scalar from the end(s); it is not grapheme-aware (it
cannot strip a whitespace scalar buried inside a cluster, but no standard cluster
begins with a `White_Space` scalar). Zero-width characters (e.g. ZWSP `U+200B`,
ZWJ) are **not** `White_Space` and are never trimmed. [[src/unicode_backend.rs:trim]]

## `split` and the empty-delimiter error

`split(value, delimiter)` splits on a **byte-exact** delimiter substring and
returns the parts. An empty `delimiter` is rejected with the error
`"delimiter must not be empty"` — there is no per-scalar or per-grapheme split
mode. [[src/unicode_backend.rs:split]]

Splitting delegates to `str::split`, so it follows Rust semantics: a leading or
trailing delimiter yields an empty leading/trailing part, and N non-overlapping
matches produce N+1 parts. The delimiter match is on raw UTF-8 bytes with no
normalization. The inverse `join(parts, delimiter)` concatenates with the
delimiter between parts and never errors. [[src/unicode_backend.rs:join]]

## See Also

* ./mfb spec unicode tables-and-algorithms — grapheme segmentation, NFC/NFD, case-fold, and the embedded property tables
* ./mfb spec language types — the `String` type and lexicographic-by-scalar comparison/ordering
* ./mfb man strings — the per-function `strings::` API reference
* ./mfb spec memory heap-values — the in-memory `String` byte layout
