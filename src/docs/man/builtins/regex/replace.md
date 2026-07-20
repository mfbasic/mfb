# replace

Replace every non-overlapping regular-expression match using a replacement template.

## Synopsis

```
regex::replace(value AS String, pattern AS String, replacement AS String) AS String
```

## Package

regex

## Imports

```
IMPORT regex
```

## Description

`regex::replace` compiles `pattern` as a regular expression and returns a new
`String` in which every non-overlapping match in `value` is replaced by the
expansion of `replacement`. The text before, between, and after matches is copied
unchanged. It is the rewriting form of the package: `regex::match` reports only
whether a match exists, `regex::find` reports where the first one begins,
`regex::findAll` reports the start of every non-overlapping match, and `replace`
produces the rewritten text. [[src/builtins/regex.rs:call_return_type_name]]

Matches are found left to right by the same leftmost, unanchored search
`regex::findAll` exposes. At each match the engine resolves it by preference
order (earlier alternatives, greedy quantifiers as long as possible, lazy ones as
short as possible), and after each match the scan resumes at the position just
past the end of that match, so the matches are non-overlapping. A zero-length
match is valid; the iterator then advances one scalar so iteration always
terminates and the same empty match is never rewritten twice at one position.
Consequently an empty or empty-matching pattern inserts the replacement before
each scalar and once at the end: `regex::replace("abc", "", "-")` is `"-a-b-c-"`.
[[src/builtins/regex_package.mfb:__regex_replace]]

Positions are Unicode scalar values, never UTF-8 bytes and never grapheme
clusters, consistent with `len` and the `strings` package.

`replacement` is literal text interleaved with capture references: `$N` or `${N}`
inserts capturing group `N` (`$0` is the whole match), `$name` or `${name}`
inserts a named group, and `$$` inserts a literal `$`. An unbraced reference
consumes the longest valid run, so use the braced form to butt a reference
against following text: `${1}0` is group `1` then `"0"`, whereas `$10` is group
`10`. A reference to a group that did not participate in the match, or to an
unknown name or an out-of-range number, expands to the empty string. Replacement
content is therefore always well-formed and is never a source of failure; only an
invalid pattern fails. [[src/builtins/regex_package.mfb:__regex_expand]]

`pattern` is an ordinary runtime `String`, so it may be built or read at run
time; it uses MFBASIC's own portable regex dialect, defined in
`mfb spec stdlib regex` (run `mfb man regex` for the language overview), which
produces identical results on every target and never defers to a host regex
library. Because `String` literals process backslash escapes, a literal backslash
is written `"\\"` — `regex::replace(value, "\\d", "#")` rewrites every digit. An
invalid pattern fails with `ErrInvalidFormat`. When `pattern` matches nothing in
`value`, `replace` does not fail; it returns a fresh `String` equal to `value`.

`replace` does not mutate `value`, `pattern`, or `replacement` and has no side
effects.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The subject text to rewrite. It is never modified. |
| `pattern` | `String` | The regular expression to compile and search for. It must be a valid pattern in the MFBASIC regex dialect; otherwise the call fails with `ErrInvalidFormat`. |
| `replacement` | `String` | The replacement template: literal text plus `$` capture references as described above. Always well-formed; never a source of failure. |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` with every non-overlapping match replaced by the expansion of `replacement`. Equal to `value` when `pattern` matches nothing. [[src/builtins/regex.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | `pattern` is not a valid regular expression: an unbalanced or unterminated group or class, a quantifier with no atom or stacked quantifiers, a counted quantifier with `m > n`, a class range whose low endpoint exceeds its high endpoint, an empty class, a backslash escape outside the defined set, a `\x...`/`\x{...}` value that is not a valid scalar, an unknown `\p{...}` property, a malformed flag or group head, or a non-goal construct (backreference or look-around). Replacement content never causes a failure. [[src/builtins/regex_package.mfb:__regex_replace]] [[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |

## Examples

Replace every match, and reorder capture groups (note the doubled backslashes):

```
IMPORT regex

SUB main()
  LET masked AS String = regex::replace("a1b2", "\\d", "#")
  LET ymd AS String = regex::replace("2024-06-24", "(\\d+)-(\\d+)-(\\d+)", "$3/$2/$1")
END SUB
```

`$$` inserts a literal dollar sign:

```
IMPORT regex

SUB main()
  LET price AS String = regex::replace("5", "5", "$$")
END SUB
```

## See also

- `mfb man regex match`
- `mfb man regex find`
- `mfb man regex findAll`
- `mfb man strings replace`
