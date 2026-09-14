//! `big::shiftLeft` — multiply a `big::Int` by a power of two.

use super::gen_big::{
    emit_build_int, emit_load_int, emit_reject_negative, emit_shift_left_magnitude, emit_spill_args,
};
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

const INTRO: &str = r#"Shift a `big::Int`'s bits toward the high end."#;
const DESC: &str = r#"`big::shiftLeft(a, count)` returns `a` multiplied by 2 raised to `count`: the
absolute value's bits move `count` places toward the high end, and the sign is kept.

`count` must be zero or more; a negative `count` raises `ErrInvalidArgument`. A `count`
of zero returns `a` unchanged, and zero shifted by any amount is zero. The result grows
to hold every shifted bit, so nothing is lost off the top.

`big::shiftRight` moves bits the other way."#;
const EX: &str = r#"Shift one past the `Integer` range:

```
IMPORT big
IMPORT io

SUB main()
  LET wide AS big::Int = big::shiftLeft(big::fromInteger(-3), 100)
  io::print(toString(big::bitLength(wide)) & " " & toString(big::sign(wide)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "shiftLeft",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The value to shift. Its sign is kept.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "count",
                    desc: "How many bit places to shift. Zero or more; a negative count raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_function(lower_shift_left),
        }],
    });
}

/// `big::shiftLeft`: reject a negative count, shift the magnitude, keep the sign.
pub(crate) fn lower_shift_left(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    emit_reject_negative(builder, &mut vregs, arg_slots[1], &invalid);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let (result, count) =
        emit_shift_left_magnitude(builder, &mut vregs, &a, arg_slots[1], "r", &alloc_fail);
    emit_build_int(builder, &mut vregs, &result, count, a.negative, "r");
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid),
    ]);
    emit_fail(
        &symbol,
        "ErrInvalidArgument",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );
    builder.instructions.push(abi::label(&alloc_fail));
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
        text: "big.shiftLeft".to_string(),
    })
}
