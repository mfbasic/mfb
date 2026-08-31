//! `strings.left` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_left_right;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Return the leading Unicode scalars of a string."#;

const DESC: &str = r#"`strings::left` returns a new `String` holding the first `count` Unicode scalar
values of `value`, taken from the start of the string toward the end.

Lengths are measured in Unicode scalar values — not UTF-8 bytes and not grapheme
clusters. A multi-byte scalar such as `é` or `😀` counts as one even though it
occupies several bytes, and `left` never splits a scalar, so the result is always
well-formed UTF-8. Note that a grapheme cluster made of a base scalar plus
combining marks counts as more than one, so `left` can cut a cluster in half; use
`strings::graphemes` when user-perceived characters are what matters.

`left` clamps rather than failing on an over-long request: when `count` is
greater than or equal to the scalar length of `value`, the whole string is
returned, with no padding and no error. A `count` of `0` returns the empty
string. A negative `count` is rejected with `ErrInvalidArgument`.

This clamping is the difference from `strings::mid`, which raises
`ErrIndexOutOfRange` when the requested window runs past the end.

`value` is not mutated; the result is a new `String`.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed exactly as the `String` overload's
and whose attribute spans are remapped by the same edit."#;

const EX: &str = r#"Take a prefix; an over-long count clamps to the whole string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::left("hello", 3))
  io::print(strings::left("hi", 5))
  io::print("[" & strings::left("hi", 0) & "]")
  RETURN 0
END FUNC
```

Multi-byte scalars count as one position each:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::left("a😀bc", 2))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.left: no native lowering for these arguments".to_string());
    }
    gen_left_right::lower_strings_left_right(builder, &args[0], &args[1], false)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "left",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string whose leading scalars are returned. May be empty.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "count",
                    desc: "The number of leading Unicode scalar values to take. Must be `0` or greater; values at or above the scalar length of `value` yield the whole string.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
