//! `strings.graphemesCount` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_graphemes::lower_strings_graphemes;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Count the extended grapheme clusters in a string."#;

const DESC: &str = r#"`strings::graphemesCount` returns the number of Unicode extended grapheme
clusters in `value`. It is defined as the element count of
`strings::graphemes(value)`, and is computed by performing that same
segmentation and reading the resulting list's length.

An extended grapheme cluster is one user-perceived character, and it may be built
from several Unicode scalar values: a base letter followed by combining marks, a
flag formed from a pair of regional indicators, or an emoji built from a base
symbol joined to modifiers by zero-width joiners. Each such cluster counts as
one.

The count is therefore a third measure, distinct from both `len(value)`, which
counts Unicode scalar values, and `strings::byteLen(value)`, which counts UTF-8
bytes. For text with combining marks, emoji, or characters outside the Basic
Multilingual Plane, all three can differ: `"e"` plus `U+0301` plus `"fg"` has
three clusters but four scalars.

The empty string yields `0`. `value` is not mutated and the call never fails.
Because the count is derived by segmenting the whole string, it is a linear scan,
not a stored field — prefer calling `strings::graphemes` once when you need both
the clusters and their count.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"Count user-perceived characters:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(toString(strings::graphemesCount("abc")))
  io::print(toString(strings::graphemesCount("a😀b")))
  RETURN 0
END FUNC
```

A combining sequence counts as one cluster but two scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET text AS String = "e" & "́" & "fg"
  io::print(toString(strings::graphemesCount(text)))
  io::print(toString(len(text)))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 1 {
        return Err("strings.graphemesCount: no native lowering for these arguments".to_string());
    }
    let value = &args[0];

    let scratch16 = builder.temporary_vreg();
    let list = lower_strings_graphemes(builder, value)?;
    let list_slot = builder.spill_to_slot("strings_graphemes_count_list", &list.location);
    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), list_slot));
    builder.emit(abi::load_u64(&result, &scratch16, COLLECTION_OFFSET_COUNT));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(result.render()),
        text: "strings.graphemesCount".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "graphemesCount",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string whose clusters are counted. Any `String` is accepted, including the empty string.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
