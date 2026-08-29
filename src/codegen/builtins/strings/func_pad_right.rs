//! `strings.padRight` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_pad;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Pad a string on the right to a given scalar width."#;

const DESC: &str = r#"`strings::padRight` returns a new `String` in which copies of `padChar` are
appended to `value` until the whole result spans `width` Unicode scalar values.
The number of copies appended is `width` minus the current scalar length of
`value`.

Width is counted in Unicode scalar values, not in UTF-8 bytes and not in grapheme
clusters, and it counts scalars of the *result*, not of the padding alone. A
multi-byte `padChar` therefore contributes one toward the width per copy while
adding several bytes.

When the scalar length of `value` already equals or exceeds `width`, no padding
is added and the result equals `value`. `padRight` never truncates to fit within
`width`. Note that a new `String` is always allocated, even in that case; the
original is never aliased.

`padChar` is optional and defaults to a single space. When supplied, it must be
exactly one well-formed Unicode scalar value — neither empty nor more than one
scalar — otherwise `ErrInvalidArgument` is raised. A negative `width` raises the
same error, as does a result size that cannot be represented in 64 bits.

Neither argument is mutated.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed exactly as the `String` overload's
and whose attribute spans are remapped by the same edit."#;

const EX: &str = r#"Right-pad with the default space and with an explicit character:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print("[" & strings::padRight("42", 5) & "]")
  io::print(strings::padRight("42", 5, "0"))
  RETURN 0
END FUNC
```

Build an aligned two-column table:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::padRight("name", 10, ".") & "value")
  io::print(strings::padRight("id", 10, ".") & "7")
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if !(args.len() == 2 || args.len() == 3) {
        return Err("strings.padRight: no native lowering for these arguments".to_string());
    }
    gen_pad::lower_strings_pad(builder, args, true)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "padRight",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string to pad. Returned as an equal copy when its scalar length is already at least `width`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "width",
                    desc: "The target total length of the result in Unicode scalar values. Must be `0` or greater; `0` never pads.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "padChar",
                    desc: "Optional. The fill character appended to reach `width`; defaults to a single space. Must be exactly one Unicode scalar value.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Optional,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
