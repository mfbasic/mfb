//! `strings.stripSuffix` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_strip;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Remove one trailing occurrence of a suffix from a string."#;

const DESC: &str = r#"`strings::stripSuffix` returns `value` with one trailing occurrence of `suffix`
removed when `value` ends with `suffix`, and returns `value` unchanged otherwise.

The match is an exact byte comparison of the trailing bytes of `value` against
every byte of `suffix`, with no normalization and no case folding. Because both
operands are well-formed UTF-8 and UTF-8 is self-synchronizing, a matching byte
suffix is always a whole-scalar suffix, so the remainder is always a valid
string.

Exactly one copy is removed. If `value` ends with `suffix` repeated, only the
last copy is stripped and the earlier ones remain — call `stripSuffix` in a loop
to remove them all. An empty `suffix` removes no bytes, a `suffix` longer than
`value` cannot match, and a non-matching `suffix` leaves `value` alone; all three
return an equal string.

The function is total and never fails. Neither operand is modified, and you
always get a new `String` back, even on the unchanged path.

To test for the suffix without removing it, use `strings::endsWith`. To remove a
*set* of trailing scalars rather than a fixed substring, use
`strings::trimChars`.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed exactly as the `String` overload's
and whose attribute spans are remapped by the same edit."#;

const EX: &str = r#"Remove a file extension; a non-matching suffix changes nothing:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::stripSuffix("photo.png", ".png"))
  io::print(strings::stripSuffix("photo.png", ".jpg"))
  RETURN 0
END FUNC
```

Only one copy is removed:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::stripSuffix("foobarbar", "bar"))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.stripSuffix: no native lowering for these arguments".to_string());
    }
    gen_strip::lower_strings_strip(builder, &args[0], &args[1], true)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "stripSuffix",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string to strip from. May be empty. Returned as an equal copy when it does not end with `suffix`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "suffix",
                    desc: "The trailing substring to remove. May be empty, in which case `value` is returned unchanged.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
