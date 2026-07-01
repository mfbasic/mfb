# regex

Match, search, and replace text with regular expressions

## Synopsis

```
IMPORT regex
regex::match(value, pattern)
regex::find(value, pattern, start)
regex::findAll(value, pattern, start)
regex::replace(value, pattern, replacement)
```

## Description

The `regex` package searches and rewrites text with a single portable
regular-expression dialect that is MFBASIC's own. Its syntax and semantics are
defined entirely by `mfb spec stdlib regex` and produce byte-for-byte identical
results on every target, never deferring to a host libc, locale, or OS regex
library. `regex` is a built-in package: `IMPORT regex` needs no manifest
dependency. For the full pattern language, run `mfb man regex language`. [[src/builtins/regex.rs:call_return_type_name]]

The package defines no new types. `pattern` and `replacement` are ordinary
runtime `String` values, so they may be literals, built at run time, or read from
input; a pattern is compiled at the moment a function is called. An invalid
pattern fails the call with `ErrInvalidFormat` rather than being silently treated
as "no match". Because MFBASIC `String` literals process their own backslash
escapes, a backslash the regex needs is written `"\\"` in a source literal
(`"\\d"` is the pattern `\d`); a pattern read from a file or user input has no
such doubling. [[src/builtins/regex_package.mfb:__regex_find]]

Matching operates over Unicode scalar values. Every position and index a regex
function accepts or reports is a zero-based Unicode scalar index — never a byte
offset and never a grapheme-cluster index — consistent with `len` and the
`strings` package. A string of `n` scalars has positions `0` through `n`;
position `n` is after the last scalar, so a `start` argument may equal
`len(value)`. All Unicode-dependent behavior (the `\d`/`\w`/`\s` shorthands,
`\p{...}` properties, and `(?i)` case folding) resolves against a single pinned
Unicode version, identical across every target.

The functions differ only in what they report. `match` returns a `Boolean` for
whether the pattern matches anywhere; `find` returns the start index of the first
match at or after `start`, or `-1` when there is none; `findAll` returns a
`List OF Integer` of the start index of every non-overlapping match; and
`replace` returns a new `String` with every non-overlapping match rewritten by a
replacement template. Every search is unanchored and leftmost: the reported match
is the one beginning at the smallest position where any match exists. `find` and
`findAll` take an optional `start` (default `0`) restricting only where a match
may begin — the absolute anchors `\A`, `\z`, and unflagged `^`/`$` are still
evaluated against the whole value. A zero-length match is valid; iteration
advances one scalar past an empty match so it always terminates. [[src/builtins/regex_package.mfb:__regex_findAll]]

No `regex` function fails on the absence of a match: `match` returns `FALSE`,
`find` returns `-1`, `findAll` returns an empty list, and `replace` returns
`value` unchanged. `ErrNotFound` is never raised by this package. None of the
functions mutate their arguments or have side effects.

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | raised by every function when `pattern` is not a valid regular expression: an unbalanced or unterminated group or class, a quantifier with no atom or stacked quantifiers, a counted quantifier with `m > n`, a class range whose low endpoint exceeds its high endpoint, an empty class, a backslash escape outside the defined set, a `\x...`/`\x{...}` value that is not a valid scalar, an unknown `\p{...}` property, a malformed flag or group head, or a non-goal construct (backreference or look-around) [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050001` | `ErrIndexOutOfRange` | raised by `find` and `findAll` when `start` is less than `0` or greater than the scalar length of `value`; `ErrInvalidFormat` takes precedence when the pattern is also invalid [[src/builtins/regex_package.mfb:__regex_find]] [[src/target/shared/code/error_constants.rs:ERR_INDEX_OUT_OF_RANGE_CODE]] |
