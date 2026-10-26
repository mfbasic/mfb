//! `strings.startsWith` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether a string begins with a given prefix."#;

const DESC: &str = r#"`strings::startsWith` returns `TRUE` when `value` begins with `prefix` and
`FALSE` otherwise. The test compares the start of `value` with `prefix` scalar
by scalar, exactly; it succeeds only when every scalar of `prefix` matches.

No normalization, case folding, or other transformation is applied to either
operand, so `startsWith("Hello", "hello")` is `FALSE`. Matching is by Unicode
scalar: a match always covers whole scalars, but not necessarily whole
characters. Because the match starts at the beginning of `value`, it can only
*end* inside a combining sequence, an emoji sequence or a flag, so
`startsWith("e\u{0301}", "e")` is `TRUE` and
`startsWith("\u{1F1FA}\u{1F1F8}x", "\u{1F1FA}")` is `TRUE`. Apply
`strings::normalizeNfc` to both operands first if composed and decomposed
spellings must compare equal.

The boundary cases follow from this comparison. A `prefix` longer than
`value` cannot match and returns `FALSE`. The empty `prefix` matches every
`value`, including the empty string, and returns `TRUE`. A non-empty `prefix`
against an empty `value` returns `FALSE`. Neither operand is modified and the
call never fails.

To test the end of the string use `strings::endsWith`; to test several candidate
prefixes at once use `strings::startsWithAny`; to remove the prefix rather than
test for it use `strings::stripPrefix`.

`value` may also be an `AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns: the same value and
type, and like it, it never fails. `prefix` must be a plain `String`; pass
`toString(p)` to test against an `AttributedString`'s visible text."#;

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
                    desc: "The string whose start is examined. May be empty.",
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
