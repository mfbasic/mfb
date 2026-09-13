//! `big::isZero` — whether a `big::Int` is zero.

use super::gen_big::{emit_load_int, emit_spill_args};
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

const INTRO: &str = r#"Test whether a `big::Int` is zero."#;
const DESC: &str = r#"`big::isZero(a)` returns `TRUE` when `a` is zero and `FALSE` otherwise.

A `magnitude` made only of zero bytes is zero, whatever its length, and so is a
hand-built value with `negative` set on it. A `big::Int` declared with `MUT` and no
initializer is zero.

The call never raises. It gives the same answer as
`big::equals(a, big::fromInteger(0))` without building the second value."#;
const EX: &str = r#"A `big::Int` declared with `MUT` and no initializer is zero:

```
IMPORT big
IMPORT io

SUB main()
  MUT x AS big::Int
  io::print(toString(big::isZero(x)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isZero",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "a",
                desc: "The value to test.",
                aliases: &[],
                ty: ParameterType::named(INT_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_is_zero),
        }],
    });
}

/// `big::isZero`: the trimmed significant count is zero.
pub(crate) fn lower_is_zero(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let count = vregs.next();
    let answer = vregs.next();
    let nonzero = format!("{symbol}_nonzero");
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), a.count),
        abi::move_immediate(&answer, "Boolean", "0"),
        abi::compare_immediate(&count, "0"),
        abi::branch_ne(&nonzero),
        abi::move_immediate(&answer, "Boolean", "1"),
        abi::label(&nonzero),
        abi::move_register(RESULT_VALUE_REGISTER, &answer),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from("void"),
        text: "big.isZero".to_string(),
    })
}
