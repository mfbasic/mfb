# normalizeNfc

Normalize a string to Unicode Normalization Form C.

## Synopsis

```
strings::normalizeNfc(value AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::normalizeNfc` returns a new `String` holding the Normalization Form C
(NFC) of `value`. NFC canonically decomposes each scalar, reorders combining
marks by canonical combining class, and then recomposes, so combining sequences
collapse into precomposed scalars wherever a canonical composition exists. Hangul
is composed algorithmically rather than from the table.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_normalize_nfc]] [[src/target/shared/code/private/unicode.rs:emit_hangul_composition_attempt]]

NFC is the canonical form to use when storing, comparing, or transmitting text:
two strings that are canonically equivalent but encoded differently become
byte-for-byte equal once both are normalized. A base letter followed by
`U+0301` COMBINING ACUTE ACCENT recomposes to the single scalar `U+00E9` `é`, and
`"A"` followed by `U+030A` recomposes to `Å`. Scalars already in composed form,
and scalars with no canonical composition, are carried through unchanged.
[[src/target/shared/code/private/unicode.rs:emit_unicode_u32_mapping_lookup]]

NFC applies canonical equivalence only. It performs no compatibility
decomposition, so ligatures, full-width forms, and superscripts are preserved
rather than expanded. Normalization can change the length of the string in both
scalars and bytes, since a base scalar plus its combining marks may collapse into
one scalar.

Normalization is independent of case: it neither folds nor changes case, so apply
`strings::caseFold` in addition when matching must ignore both normalization and
case. The transformation is deterministic and locale-independent. `value` is not
mutated; the result is a new owned `String`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to normalize. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` holding the NFC form of `value`. The empty string yields `""`; a string already in NFC yields an equal string. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

A decomposed and a precomposed spelling normalize to the same value:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET decomposed AS String = "e" & "́"
  LET precomposed AS String = "é"
  io::print(toString(strings::normalizeNfc(decomposed) = strings::normalizeNfc(precomposed)))
  RETURN 0
END FUNC
```

Normalization can shorten the scalar count:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET decomposed AS String = "Cafe" & "́"
  io::print(toString(len(decomposed)))
  io::print(toString(len(strings::normalizeNfc(decomposed))))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings caseFold`
- `mfb man strings upper`
- `mfb man strings lower`
- `mfb man strings graphemes`
- `mfb man unicode`
- `mfb man strings`
