//! `strings.graphemes` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_graphemes;
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const INTRO: &str = r#"Split a string into its extended grapheme clusters."#;

const DESC: &str = r#"`strings::graphemes` splits `value` into Unicode extended grapheme clusters and
returns them, in order, as a `List OF String`.

An extended grapheme cluster is one user-perceived character, and it may be built
from several Unicode scalar values: a base letter followed by combining marks, a
flag formed from a pair of regional indicators, or an emoji built from a base
symbol joined to modifiers by zero-width joiners. `graphemes` groups all the
scalars of such a cluster into a single element, so `"👨‍👩‍👧‍👦x"` yields two
elements, not eight. Cluster boundaries follow the Unicode extended
grapheme-cluster rules.

This is a third way of counting a string, distinct from both of the others:
`len(value)` counts Unicode scalar values and `strings::byteLen(value)` counts
UTF-8 bytes. For text with combining marks, emoji, or flags all three can differ.

The clusters appear in the same left-to-right order as in `value`, and
concatenating them reproduces `value` exactly — no scalar is dropped or
reordered. The empty string yields the empty list. `value` is not mutated; the
returned list and its elements are their own values.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"An emoji ZWJ sequence and a flag each count as one cluster:

```
IMPORT io
IMPORT strings
IMPORT collections

FUNC main() AS Integer
  LET parts AS List OF String = strings::graphemes("👨‍👩‍👧‍👦x")
  io::print(toString(len(parts)))
  io::print(collections::get(parts, 0))
  RETURN 0
END FUNC
```

Iterate over user-perceived characters:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  FOR EACH g IN strings::graphemes("héllo")
    io::print(g)
  NEXT
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() == 1 {
        if let Some(value) = builder.static_string_value_vr(&args[0]) {
            let values = crate::unicode::backend::graphemes(&value)
                .into_iter()
                .map(|value| NirValue::Const {
                    type_: ParameterType::String,
                    value,
                })
                .collect::<Vec<_>>();
            return builder
                .lower_list_literal(&ParameterType::list_of(ParameterType::String), &values);
        }
    }
    if args.len() != 1 {
        return Err("strings.graphemes: no native lowering for these arguments".to_string());
    }
    gen_graphemes::lower_strings_graphemes(builder, &args[0])
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "graphemes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string to split. Any `String` is accepted, including the empty string.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
