//! `big::multiply` — the product of two `big::Int` values.

use super::gen_big::{emit_load_int, emit_mul_int, emit_spill_args};
use super::INT_TYPE_ID;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Multiply two `big::Int` values."#;
const DESC: &str = r#"`big::multiply(a, b)` returns `a * b`.

The result is exact at every size and the call never raises: the product grows to hold
as many bytes as the two operands together. It is negative exactly when the operand
signs differ, and multiplying by zero gives plain zero, never a negative zero.

The cost grows with the product of the two operand sizes. Neither operand is changed.
`big::product` multiplies a whole list in one call."#;
const EX: &str = r#"Square 2 raised to 32, which leaves the `Integer` range, and check it against
2 raised to 64 built by adding:

```
IMPORT big
IMPORT io

SUB main()
  LET twoTo32 AS big::Int = big::fromInteger(4294967296)
  LET square AS big::Int = big::multiply(twoTo32, twoTo32)
  LET top AS big::Int = big::fromInteger(9223372036854775807)
  LET twoTo64 AS big::Int = big::add(big::add(top, top), big::fromInteger(2))
  io::print(toString(big::equals(square, twoTo64)) & " " & toString(len(square.magnitude)))
  io::print(toString(big::sign(big::multiply(twoTo32, big::fromInteger(-3)))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "multiply",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The first factor.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The second factor.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_multiply),
        }],
    });
}

/// `big::multiply`: load both operands and multiply with `emit_mul_int`.
pub(crate) fn lower_multiply(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let b = emit_load_int(builder, &mut vregs, arg_slots[1], "b");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    emit_mul_int(builder, &mut vregs, &a, &b, "r", &alloc_fail);
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&alloc_fail),
    ]);
    emit_fail(
        &symbol,
        "ErrOutOfMemory",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );
    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::named(INT_TYPE_ID),
        location: Operand::from("void"),
        text: "big.multiply".to_string(),
    })
}
