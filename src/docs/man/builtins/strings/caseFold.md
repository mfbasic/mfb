# caseFold

Case-fold a string to a canonical caseless form for comparison.

## Synopsis

```
strings::caseFold(value AS String) AS String
```

## Package

strings

## Imports

```
IMPORT strings
```

`strings` is a built-in package, so no manifest dependency is required. [[src/builtins/strings.rs:is_strings_call]]

## Description

`strings::caseFold` returns a new `String` produced by applying Unicode full case
folding to `value`. Folding maps scalars to a canonical caseless form so that two
strings differing only in case become equal once both are folded. It is the
intended basis for caseless matching, in preference to uppercasing or lowercasing
both operands. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_case_map]]

Folding is applied per Unicode scalar value across the whole string, using the
case-folding table embedded in the runtime. Scalars with no folded form — digits,
punctuation, and symbols — are copied through unchanged. Folding is *full*: one
scalar may fold to several, so `"Straße"` folds to `"strasse"` and the result can
be longer than the input. Never assume `len` is preserved across a fold.
[[src/target/shared/code/private/unicode.rs:emit_unicode_u32_mapping_lookup]]

Folding is not lowercasing. It is designed for comparison rather than display,
and it also collapses distinctions that lowercasing preserves — `U+212A` KELVIN
SIGN folds to plain `k`. Do not present a folded string to a user; keep the
original for display and use the folded form only as a comparison key.

Folding does not normalize. Strings that differ in Unicode normalization form can
still differ after folding, so apply `strings::normalizeNfc` first when
normalization-insensitive matching is required. The mapping is deterministic and
locale-independent, with no language-specific tailoring. `value` is not mutated;
the result is a new owned `String`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `value` | `String` | The string to fold. Any `String` is accepted, including the empty string. [[src/builtins/strings.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | A new `String` holding the case-folded form of `value`. The empty string yields `""`; a string with no cased scalars yields an equal string. May be longer than `value`. [[src/builtins/strings.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Compare two strings without regard to case:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET same AS Boolean = strings::caseFold("HELLO") = strings::caseFold("hello")
  io::print(toString(same))
  RETURN 0
END FUNC
```

Folding can change the length of the string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET german AS String = "Straße"
  io::print(strings::caseFold(german))
  RETURN 0
END FUNC
```

Normalize first when the inputs may differ in composition:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET a AS String = strings::caseFold(strings::normalizeNfc("CAFÉ"))
  LET b AS String = strings::caseFold(strings::normalizeNfc("café"))
  io::print(toString(a = b))
  RETURN 0
END FUNC
```

## See also

- `mfb man strings lower`
- `mfb man strings upper`
- `mfb man strings normalizeNfc`
- `mfb man unicode`
- `mfb man strings`
