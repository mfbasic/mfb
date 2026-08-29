//! `strings.trimEnd` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_trim;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Remove trailing Unicode whitespace from a string."#;

const DESC: &str = r#"`strings::trimEnd` returns a new `String` equal to `value` with every trailing
whitespace scalar removed. Leading whitespace is left in place; it is the
one-sided form of `strings::trim`, which trims both ends.

Whitespace is recognized by Unicode scalar, not by byte, and the recognized set
is exactly the Unicode `White_Space` property: `U+0009`–`U+000D`, `U+0020`,
`U+0085`, `U+00A0`, `U+1680`, `U+2000`–`U+200A`, `U+2028`, `U+2029`, `U+202F`,
`U+205F`, and `U+3000`. Multi-byte whitespace scalars are matched whole, so
trimming never splits a scalar.

Removal stops at the last scalar that is not whitespace, so leading and interior
content, including embedded spaces and line breaks, is preserved byte for byte. A
`value` that is entirely whitespace yields the empty string, and the empty string
yields the empty string. `value` is not mutated; the result is a newly allocated
`String`, even when nothing was trimmed.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed exactly as the `String` overload's
and whose attribute spans are remapped by the same edit."#;

const EX: &str = r#"Remove trailing spaces while keeping the leading ones:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print("[" & strings::trimEnd("  Hello  ") & "]")
  RETURN 0
END FUNC
```

Strip a trailing newline from a read line:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print("[" & strings::trimEnd("value\n") & "]")
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 1 {
        return Err("strings.trimEnd: no native lowering for these arguments".to_string());
    }
    gen_trim::lower_strings_trim(builder, &args[0], false, true)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "trimEnd",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string to trim at the end. Any `String` is accepted, including the empty string.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
