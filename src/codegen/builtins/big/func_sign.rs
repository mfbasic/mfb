//! `big::sign` — the sign of a `big::Int`: `-1`, `0` or `1`.

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

const INTRO: &str = r#"Return the sign of a `big::Int` as `-1`, `0` or `1`."#;
const DESC: &str = r#"`big::sign(a)` returns `-1` when `a` is below zero, `0` when it is zero, and `1`
when it is above zero.

Zero is always `0`, including a hand-built value with `negative` set on a zero
`magnitude`. Reading the `negative` field directly gives `FALSE` for zero and cannot
tell zero from a positive value; `big::sign` can.

The call never raises."#;
const EX: &str = r#"The three signs:

```
IMPORT big
IMPORT io

SUB main()
  io::print(toString(big::sign(big::fromInteger(-9))))
  io::print(toString(big::sign(big::fromInteger(0))))
  io::print(toString(big::sign(big::fromInteger(9))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sign",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "a",
                desc: "The value whose sign is returned.",
                aliases: &[],
                ty: ParameterType::named(INT_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_sign),
        }],
    });
}

/// `big::sign`: `0` for a zero count, otherwise `-1`/`1` from the trimmed sign.
pub(crate) fn lower_sign(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let count = vregs.next();
    let flag = vregs.next();
    let answer = vregs.next();
    let finished = format!("{symbol}_finished");
    let positive = format!("{symbol}_positive");
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), a.count),
        abi::move_immediate(&answer, "Integer", "0"),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(&finished),
        abi::load_u64(&flag, abi::stack_pointer(), a.negative),
        abi::move_immediate(&answer, "Integer", "1"),
        abi::compare_immediate(&flag, "0"),
        abi::branch_eq(&positive),
        abi::move_immediate(&answer, "Integer", "0"),
        abi::subtract_immediate(&answer, &answer, 1),
        abi::label(&positive),
        abi::label(&finished),
        abi::move_register(RESULT_VALUE_REGISTER, &answer),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from("void"),
        text: "big.sign".to_string(),
    })
}
