//! `strings.trim` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_trim;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Remove leading and trailing Unicode whitespace from a string."#;

const DESC: &str = r#"`strings::trim` returns a new `String` equal to `value` with every leading and
trailing whitespace scalar removed. Both ends are trimmed in one call; to trim
only one end use `strings::trimStart` or `strings::trimEnd`.

Whitespace is recognized by Unicode scalar, not by byte, and the recognized set
is exactly the Unicode `White_Space` property: `U+0009`–`U+000D` (tab, line
feed, vertical tab, form feed, carriage return), `U+0020` space, `U+0085` next
line, `U+00A0` no-break space, `U+1680` ogham space mark, `U+2000`–`U+200A` the
en/em quad and space family, `U+2028` line separator, `U+2029` paragraph
separator, `U+202F` narrow no-break space, `U+205F` medium mathematical space,
and `U+3000` ideographic space. Multi-byte whitespace scalars are matched whole,
so trimming never splits a scalar.

Only the contiguous runs of whitespace at the very start and the very end are
removed. Whitespace between non-whitespace scalars is interior and is preserved
byte for byte. A `value` that is entirely whitespace trims to the empty string,
and the empty string trims to the empty string. `value` is not mutated; the
result is a new `String`, even when nothing was trimmed.

The trim is locale-independent and performs no normalization or case folding. To
strip a specific set of scalars instead of whitespace, use `strings::trimChars`.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed exactly as the `String` overload's
and whose attribute spans are remapped by the same edit."#;

const EX: &str = r#"Remove surrounding spaces:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::trim("  Hello  "))
  RETURN 0
END FUNC
```

Interior whitespace is preserved, and non-ASCII whitespace is trimmed:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::trim("\n  a b  \n"))
  io::print(strings::trim("　wide　"))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 1 {
        return Err("strings.trim: no native lowering for these arguments".to_string());
    }
    gen_trim::lower_strings_trim(builder, &args[0], true, true)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "trim",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string to trim. Any `String` is accepted, including the empty string and a string that is entirely whitespace.",
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
