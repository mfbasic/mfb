//! `big::equals` — whether two `big::Int` values are the same number.

use super::gen_big::{emit_compare_int, emit_load_int, emit_spill_args};
use super::INT_TYPE_ID;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Test whether two `big::Int` values are the same number."#;
const DESC: &str = r#"`big::equals(a, b)` returns `TRUE` when `a` and `b` are the same number and
`FALSE` otherwise. It is the test `=` would perform if it applied to a `big::Int`,
which it does not.

Equality compares the numbers, not how the records were spelled: a `big::Int` built
by hand with trailing zero bytes in its `magnitude`, or with `negative` set on zero,
equals the canonical value it denotes. `big::equals(a, b)` is `TRUE` exactly when
`big::compare(a, b)` is `0`.

The call never raises."#;
const EX: &str = r#"A hand-built value with a trailing zero byte still equals the canonical one:

```
IMPORT big
IMPORT io

SUB main()
  LET spelled AS big::Int = big::Int[[7, 0], FALSE]
  io::print(toString(big::equals(spelled, big::fromInteger(7))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "equals",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The first value.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The second value.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_equals),
        }],
    });
}

/// `big::equals`: `emit_compare_int`'s order, tested against zero.
pub(crate) fn lower_equals(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let b = emit_load_int(builder, &mut vregs, arg_slots[1], "b");
    let order = vregs.next();
    emit_compare_int(builder, &mut vregs, &a, &b, &order, "eq");
    let answer = vregs.next();
    let differ = format!("{symbol}_differ");
    builder.instructions.extend([
        abi::move_immediate(&answer, "Boolean", "0"),
        abi::compare_immediate(&order, "0"),
        abi::branch_ne(&differ),
        abi::move_immediate(&answer, "Boolean", "1"),
        abi::label(&differ),
        abi::move_register(RESULT_VALUE_REGISTER, &answer),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from("void"),
        text: "big.equals".to_string(),
    })
}
