//! `strings.endsWith` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a string ends with a given suffix."#;

const DESC: &str = r#"`strings::endsWith` returns `TRUE` when `value` ends with `suffix` and `FALSE`
otherwise. The test is an exact byte comparison of the trailing bytes of `value`
against every byte of `suffix`, in order; it succeeds only when all of them
match.

No normalization, case folding, or other transformation is applied to either
operand, so `endsWith("Hello", "LO")` is `FALSE`. Because both operands are
well-formed UTF-8 and UTF-8 is self-synchronizing, a matching byte suffix is
always also a whole-scalar suffix — a match can never land mid-scalar.

The boundary cases follow from the byte comparison. A `suffix` longer than
`value` cannot match and returns `FALSE`. The empty `suffix` matches every
`value`, including the empty string, and returns `TRUE`. A non-empty `suffix`
against an empty `value` returns `FALSE`. Neither operand is modified and the
call never fails.

To test the start of the string use `strings::startsWith`; to test several
candidate suffixes at once use `strings::endsWithAny`; to remove the suffix
rather than test for it use `strings::stripSuffix`.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"Test for a trailing suffix, including a multi-byte one:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::endsWith("Hello", "lo")))
  io::print(toString(strings::endsWith("Hello 😀", "😀")))
  io::print(toString(strings::endsWith("Hi", "Hello")))
  RETURN 0
END FUNC
```

Match a file extension:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::endsWith("photo.png", ".png")))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.endsWith: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
    let suffix = &args[1];

    let value = value.clone();
    builder.require_string("strings.endsWith value", &value)?;
    let value_slot = builder.spill_to_slot("strings_ends_with_value", &value.location);
    let suffix = suffix.clone();
    builder.require_string("strings.endsWith suffix", &suffix)?;
    let suffix_slot = builder.spill_to_slot("strings_ends_with_suffix", &suffix.location);
    builder.lower_string_prefix_predicate("strings.endsWith", value_slot, suffix_slot, true)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "endsWith",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string whose trailing bytes are examined. May be empty.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "suffix",
                    desc: "The suffix to look for at the end of `value`. May be empty, in which case the result is always `TRUE`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
