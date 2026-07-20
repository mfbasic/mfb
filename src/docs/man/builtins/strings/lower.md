# lower

Map a string to lowercase using Unicode full case mapping.

## Synopsis

```
strings::lower(value AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::lower` returns a new `String` in which every scalar of `value` has been
mapped to its lowercase form. The mapping is applied per Unicode scalar value
across the whole string, using the lowercase table embedded in the runtime.
Scalars with no lowercase mapping — digits, punctuation, symbols, and
already-lowercase letters — are copied through unchanged.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_case_map]]

The mapping is *full*, not simple: one scalar may expand into several. `U+0130`
LATIN CAPITAL LETTER I WITH DOT ABOVE (`İ`) lowercases to `i` followed by
`U+0307` COMBINING DOT ABOVE, two scalars, so `lower` can return a string longer
than its input. Never assume `len` is preserved across a case mapping.
[[src/target/shared/code/private/unicode.rs:emit_unicode_u32_mapping_lookup]]

The mapping is deterministic and locale-independent: it always uses the default
Unicode case conventions and applies no language-specific tailoring, so no
Turkish dotted/dotless-i tailoring is performed. `lower` does not normalize, so
combining sequences stay decomposed; apply `strings::normalizeNfc` first when
that matters. [[src/unicode_backend.rs:lower]]

For caseless *comparison*, prefer `strings::caseFold` over lowercasing both
operands. `value` is not mutated; the result is a new owned `String`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to lowercase. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` holding the lowercase mapping of `value`. The empty string yields `""`; a string with no cased scalars yields an equal string. May be longer than `value`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Lowercase a word; uncased scalars pass through:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::lower("HELLO"))
  io::print(strings::lower("ABC-123"))
  RETURN 0
END FUNC
```

Full case mapping can lengthen the string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET dotted AS String = "İ"
  io::print(toString(len(dotted)))
  io::print(toString(len(strings::lower(dotted))))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings upper`
- `mfb man strings caseFold`
- `mfb man strings normalizeNfc`
- `mfb man unicode`
- `mfb man strings`
