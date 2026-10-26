//! `strings.graphemesCount` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_graphemes::{emit_grapheme_count_pass, GraphemeCountLabels, GraphemeCountRegs};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Count the extended grapheme clusters in a string."#;

const DESC: &str = r#"`strings::graphemesCount` returns the number of Unicode extended grapheme
clusters in `value`; the result always equals `len(strings::graphemes(value))`.
Unlike `strings::graphemes`, which builds a list and can raise `ErrOutOfMemory`,
`graphemesCount` builds nothing and never fails.

An extended grapheme cluster is one user-perceived character, and it may be built
from several Unicode scalar values: a base letter followed by combining marks, a
flag formed from a pair of regional indicators, or an emoji built from a base
symbol joined to modifiers by zero-width joiners. Each such cluster counts as
one. The pair `"\r\n"` is also a single cluster, so text with Windows line
endings counts one cluster per line break, not two.

The count is therefore a third measure, distinct from both `len(value)`, which
counts Unicode scalar values, and `strings::byteLen(value)`, which counts UTF-8
bytes. A cluster and a scalar differ only when several scalars combine into one
user-perceived character — combining marks, flags, emoji modifier and joiner
sequences, Hangul jamo; a single emoji outside the Basic Multilingual Plane is
still one scalar and one cluster, and only its byte length differs. `"e"` plus
`U+0301` plus `"fg"` has three clusters but four scalars.

The empty string yields `0`. `value` is not mutated and the call never fails.
Counting means walking the whole string, so unlike `strings::byteLen` the cost
grows with its length — when you need both the clusters and their count, call
`strings::graphemes` once and take `len` of the result.

`value` may also be an `AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns: the same value and
type, and like it, it never fails."#;

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

A combining sequence (`e` followed by `U+0301`) counts as one cluster but two
scalars:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET text AS String = "e\u{301}fg"
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

    // bug-992: the count pass alone. It reads the string and allocates nothing;
    // building the whole `List OF String` to read its count word left that list
    // with no owner, one block per call.
    builder.require_string("strings.graphemesCount value", value)?;
    let value_slot = builder.spill_to_slot("strings_graphemes_count_value", &value.location);
    let regs = GraphemeCountRegs {
        value: builder.temporary_vreg(),
        byte_len: builder.temporary_vreg(),
        bytes: builder.temporary_vreg(),
        count: builder.temporary_vreg(),
        scalar: builder.temporary_vreg(),
        width: builder.temporary_vreg(),
        props: builder.temporary_vreg(),
        prev_bc: builder.temporary_vreg(),
        prev_icb: builder.temporary_vreg(),
        cursor: builder.temporary_vreg(),
        scan: builder.temporary_vreg(),
        bc: builder.temporary_vreg(),
        icb: builder.temporary_vreg(),
    };
    let labels = GraphemeCountLabels::mint(builder);
    emit_grapheme_count_pass(builder, value_slot, &regs, &labels);
    let result = builder.allocate_register();
    builder.emit(abi::move_register(&result, &regs.count));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(result),
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
            // bug-992: the count pass allocates nothing, so nothing can fail.
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
