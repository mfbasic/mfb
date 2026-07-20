# upper

Map a string to uppercase using Unicode full case mapping.

## Synopsis

```
strings::upper(value AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::upper` returns a new `String` in which every scalar of `value` has been
mapped to its uppercase form. The mapping is applied per Unicode scalar value
across the whole string, using the uppercase table embedded in the runtime.
Scalars with no uppercase mapping — digits, punctuation, symbols, and
already-uppercase letters — are copied through unchanged.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_case_map]]

The mapping is *full*, not simple: one scalar may expand into several. The German
sharp s `ß` uppercases to `SS`, so `upper` can return a string that is longer
than its input in both scalars and bytes. Never assume `len` is preserved across
a case mapping. [[src/target/shared/code/private/unicode.rs:emit_unicode_u32_mapping_lookup]]

The mapping is deterministic and locale-independent: it always uses the default
Unicode case conventions and applies no language-specific tailoring, so no
Turkish dotted/dotless-i tailoring is performed. `upper` does not normalize, so
combining sequences stay decomposed; apply `strings::normalizeNfc` first when
that matters. [[src/unicode_backend.rs:upper]]

For caseless *comparison*, prefer `strings::caseFold` over uppercasing or
lowercasing both operands. `value` is not mutated; the result is a new owned
`String`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to uppercase. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` holding the uppercase mapping of `value`. The empty string yields `""`; a string with no cased scalars yields an equal string. May be longer than `value`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Uppercase a word; uncased scalars pass through:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::upper("hello"))
  io::print(strings::upper("abc-123"))
  RETURN 0
END FUNC
```

Full case mapping can lengthen the string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET german AS String = "straße"
  io::print(strings::upper(german))
  io::print(toString(len(german)))
  io::print(toString(len(strings::upper(german))))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings lower`
- `mfb man strings caseFold`
- `mfb man strings normalizeNfc`
- `mfb man unicode`
- `mfb man strings`
