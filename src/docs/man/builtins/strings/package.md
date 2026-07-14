# strings

Unicode-aware helpers for `String` values

## Synopsis

```
IMPORT strings
strings::trim(value)
strings::upper(value)
strings::split(value, delimiter)
strings::find(value, needle[, start])
strings::mid(value, start, count)
strings::padLeft(value, width[, padChar])
```

## Description

The `strings` package provides package-qualified helpers for `String` values:
trimming and case mapping (`trim`, `trimStart`, `trimEnd`, `trimChars`, `upper`,
`lower`, `caseFold`), Unicode normalization and segmentation (`normalizeNfc`,
`graphemes`, `graphemeAt`, `graphemesCount`), tests and search (`startsWith`,
`endsWith`, `contains`, `startsWithAny`, `endsWithAny`, `find`, `count`), slicing
and reshaping (`left`, `right`, `mid`, `stripPrefix`, `stripSuffix`, `split`,
`join`, `replace`, `repeat`, `padLeft`, `padRight`), length and byte queries
(`byteLen`, `toBytes`), and the Unicode-scalar seam (`toScalars`, `fromScalars`,
and the `Scalar` classifiers `isLetter`, `isDigit`, `isWhitespace`, `isUpper`,
`isLower`). [[src/builtins/strings.rs:is_strings_call]]

These helpers do not mutate their arguments. Functions that transform text return
a new `String`; `graphemes` and `split` return a `List OF String`, `toBytes`
returns a `List OF Byte`, `toScalars` returns a `List OF Scalar`, and the
original value is left unchanged. The scalar seam bridges `String` and the
`Scalar` primitive: `toScalars` walks a string one Unicode scalar at a time and
`fromScalars` rebuilds one, an exact round trip; the five `isX(Scalar)`
predicates classify a single scalar by its Unicode general category.
[[src/builtins/strings.rs:call_return_type_name]]

Index- and count-based functions (`find`, `mid`, `left`, `right`) measure
positions in zero-based Unicode scalar values, not bytes or graphemes. The
grapheme helpers `graphemes`, `graphemeAt`, and `graphemesCount` are the
exception: they operate on user-perceived extended grapheme clusters. `byteLen`
reports the length of the UTF-8 encoding in bytes, and `toBytes` returns those
raw UTF-8 bytes one element per byte. Case-insensitive comparison should use
`caseFold` rather than `upper` or `lower`, and content that may combine
characters differently can be normalized with `normalizeNfc` before comparison.

Several functions accept an optional or defaulted argument: `find` takes an
optional `start` position, and `padLeft` and `padRight` take an optional
`padChar` that defaults to a single space. The pad character, when supplied, must
be exactly one Unicode scalar value.

`strings` is a built-in package: `IMPORT strings` needs no manifest dependency.

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050001` | `ErrIndexOutOfRange` | raised by `find` when `start` is outside `0` through the scalar length of `value`, by `mid` when `start` or `count` is negative or `start + count` exceeds the scalar length of `value`, and by `graphemeAt` when `index` is negative or not less than the grapheme-cluster count of `value` [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] |
| `77050002` | `ErrInvalidArgument` | raised by `left`, `right`, and `repeat` when the `count` or `times` argument is negative, by `padLeft` and `padRight` when `width` is negative or `padChar` is not exactly one Unicode scalar value, by `split` when the delimiter is the empty `String`, and by `count` when the needle is the empty `String` [[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050004` | `ErrNotFound` | raised by `find` when no occurrence of `needle` exists at or after `start` [[src/target/shared/code/error_constants.rs:ERR_NOT_FOUND_CODE]] |
