# graphemes

Split a string into its extended grapheme clusters.

## Synopsis

```
strings::graphemes(value AS String) AS List OF String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::graphemes` splits `value` into Unicode extended grapheme clusters and
returns them, in order, as a `List OF String`.
[[src/target/shared/code/builder_strings_builtins.rs:lower_strings_graphemes]]

An extended grapheme cluster is one user-perceived character, and it may be built
from several Unicode scalar values: a base letter followed by combining marks, a
flag formed from a pair of regional indicators, or an emoji built from a base
symbol joined to modifiers by zero-width joiners. `graphemes` groups all the
scalars of such a cluster into a single element, so `"👨‍👩‍👧‍👦x"` yields two
elements, not eight. Cluster boundaries follow the Unicode extended
grapheme-cluster rules embedded in the runtime.
[[src/target/shared/code/private/unicode.rs:emit_grapheme_break_branch]]

This is a third way of counting a string, distinct from both of the others:
`len(value)` counts Unicode scalar values and `strings::byteLen(value)` counts
UTF-8 bytes. For text with combining marks, emoji, or flags all three can differ.

The clusters appear in the same left-to-right order as in `value`, and
concatenating them reproduces `value` exactly — no scalar is dropped or
reordered. The empty string yields the empty list. `value` is not mutated; the
returned list and its elements are fresh owned values.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to split. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF String` | The extended grapheme clusters of `value`, in order, one per element. The empty string yields the empty list. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

An emoji ZWJ sequence and a flag each count as one cluster:

```
IMPORT io
IMPORT strings
IMPORT collections

FUNC main() AS Integer
  LET parts AS List OF String = strings::graphemes("👨‍👩‍👧‍👦x")
  io::print(toString(len(parts)))
  io::print(collections::get(parts, 0))
  RETURN 0
END FUNC
```

Iterate over user-perceived characters:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  FOR EACH g IN strings::graphemes("héllo")
    io::print(g)
  NEXT
  RETURN 0
END FUNC
```

## See also

- `mfb man strings graphemesCount`
- `mfb man strings graphemeAt`
- `mfb man strings toScalars`
- `mfb man strings byteLen`
- `mfb man general len`
- `mfb man unicode`
- `mfb man strings`
