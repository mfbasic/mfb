//! `strings.startsWith` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a string begins with a given prefix."#;

const DESC: &str = r#"`strings::startsWith` returns `TRUE` when `value` begins with `prefix` and
`FALSE` otherwise. The test is an exact byte comparison of the leading bytes of
`value` against every byte of `prefix`, in order; it succeeds only when all of
them match.

No normalization, case folding, or other transformation is applied to either
operand, so `startsWith("Hello", "hello")` is `FALSE`. Because both operands are
well-formed UTF-8 and UTF-8 is self-synchronizing, a matching byte prefix is
always also a whole-scalar prefix — a match can never land mid-scalar.

The boundary cases follow from the byte comparison. A `prefix` longer than
`value` cannot match and returns `FALSE`. The empty `prefix` matches every
`value`, including the empty string, and returns `TRUE`. A non-empty `prefix`
against an empty `value` returns `FALSE`. Neither operand is modified and the
call never fails.

To test the end of the string use `strings::endsWith`; to test several candidate
prefixes at once use `strings::startsWithAny`; to remove the prefix rather than
test for it use `strings::stripPrefix`.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"Test for a leading prefix, including a multi-byte one:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::startsWith("Hello", "He")))
  io::print(toString(strings::startsWith("😀 Hello", "😀")))
  io::print(toString(strings::startsWith("Hello", "hello")))
  RETURN 0
END FUNC
```

The empty prefix always matches:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::startsWith("anything", "")))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.startsWith: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
    let prefix = &args[1];

    let value = value.clone();
    builder.require_string("strings.startsWith value", &value)?;
    let value_slot = builder.spill_to_slot("strings_starts_with_value", &value.location);
    let prefix = prefix.clone();
    builder.require_string("strings.startsWith prefix", &prefix)?;
    let prefix_slot = builder.spill_to_slot("strings_starts_with_prefix", &prefix.location);
    builder.lower_string_prefix_predicate("strings.startsWith", value_slot, prefix_slot, false)
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "startsWith",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string whose leading bytes are examined. May be empty.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "prefix",
                    desc: "The prefix to look for at the start of `value`. May be empty, in which case the result is always `TRUE`.",
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
