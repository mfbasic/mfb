//! `strings::find` — descriptor entry (`Body::Intrinsic`).
//!
//! The `String` overload of `find` shares its bare native lowering with the
//! `collections::` `List` overload through `builtins::native_builtin_target`
//! (`lower_find` etc.), so its `Body` is
//! [`Body::Intrinsic`]; the descriptor exists for return-type resolution, arity,
//! errors, and parameter names.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Locate the first occurrence of a substring, by Unicode scalar index."#;

const DESC: &str = r#"`strings::find` searches `value` for the first occurrence of `needle` at or after
the scalar position `start`, and returns the zero-based scalar index where that
occurrence begins.

Positions are measured in Unicode scalar values — not UTF-8 bytes and not
grapheme clusters. A multi-byte scalar such as `é` or `😀` counts as one
position even though it occupies several bytes. Both `start` and the returned
index are scalar indexes, so `find("a😀é", "😀")` is `1`. Matching itself is an
exact byte comparison with no normalization and no case folding, so a
precomposed `é` does not match a decomposed one.

`start` defaults to `0` when the two-argument form is used. It must lie in `0`
through the scalar length of `value` *inclusive*; the upper bound equals the
length so a search may begin at the very end of the string, where only an empty
needle can match. A negative `start`, or one past the scalar length, raises
`ErrIndexOutOfRange`. An empty `needle` matches immediately and returns `start`.

`find` always returns a valid index on success and never reports absence with a
sentinel such as `-1`. When `needle` does not occur at or after `start` it raises
`ErrNotFound`. When absence is an ordinary, expected outcome, guard the
two-argument form with `strings::contains` and call `find` only once a match is
known to exist.

**That guard does not carry over to the three-argument form.**
`strings::contains` searches the whole string, so it can answer `TRUE` for a
match that lies *before* `start`, and `find` will still raise. In
`strings::contains("abcabc", "a")` the answer is `TRUE` while
`strings::find("abcabc", "a", 5)` raises `ErrNotFound`. When you pass `start`,
either handle the failure with a `TRAP` or search the suffix beginning at
`start` instead.

`find` does not mutate either operand. The bare `find` name is also defined for
lists; see `mfb man collections find` for the `List` form.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"Find the first occurrence, then resume after it:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::find("hello", "l")))
  io::print(toString(strings::find("hello", "l", 3)))
  RETURN 0
END FUNC
```

Indexes are scalar positions, not byte offsets:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::find("a😀é", "😀")))
  io::print(toString(strings::find("aé日é", "日")))
  RETURN 0
END FUNC
```

Guard with `contains`, or catch the absence with `TRAP`:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET at AS Integer = strings::find("hello", "z") TRAP(e)
    io::print("absent")
    RETURN 0
  END TRAP
  io::print(toString(at))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "find",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string to search. May be empty.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "needle",
                    desc: "The substring to locate. An empty `needle` matches at `start`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "start",
                    desc: "Optional. The zero-based scalar index at which to begin searching. Defaults to `0`. Must be in `0` through the scalar length of `value` inclusive.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Optional,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec!["ErrIndexOutOfRange", "ErrNotFound"],
            body: Body::Intrinsic,
        }],
    });
}
