//! `big::divMod` — the quotient and remainder of one division, as a `big::DivResult`.

use super::gen_big::{emit_build_div_result, emit_div_mod_int, emit_load_int, emit_spill_args};
use super::{DIV_RESULT_TYPE_ID, INT_TYPE_ID};
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

const INTRO: &str = r#"Divide one `big::Int` by another, returning the quotient and remainder together."#;
const DESC: &str = r#"`big::divMod(a, b)` divides `a` by `b` once and returns a `big::DivResult` holding
both answers. Its `quotient` is truncated toward zero and its `remainder` takes the sign
of the dividend `a`, exactly as `big::divide` and `big::remainder` return them, so
`a = b * quotient + remainder` with the remainder's absolute value smaller than `b`'s.

When both answers are needed this does the division once, where calling `big::divide`
and `big::remainder` does it twice.

Dividing by zero raises `ErrInvalidArgument`; nothing else can fail, at any size."#;
const EX: &str = r#"Both answers from one division:

```
IMPORT big
IMPORT io

SUB main()
  LET parts AS big::DivResult = big::divMod(big::fromInteger(-7), big::fromInteger(2))
  io::print(toString(big::toInteger(parts.quotient)) & " " & toString(big::toInteger(parts.remainder)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "divMod",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The dividend. The remainder takes its sign.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The divisor. Zero raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(DIV_RESULT_TYPE_ID),
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_function(lower_div_mod),
        }],
    });
}

/// `big::divMod`: `emit_div_mod_int`, then both records into one `DivResult`.
pub(crate) fn lower_div_mod(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let b = emit_load_int(builder, &mut vregs, arg_slots[1], "b");
    let quotient = builder.allocate_stack_object("big_quotient", 8);
    let remainder = builder.allocate_stack_object("big_remainder", 8);
    let zero_divisor = format!("{symbol}_zero_divisor");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    emit_div_mod_int(
        builder,
        &mut vregs,
        &a,
        &b,
        quotient,
        remainder,
        "r",
        &zero_divisor,
        &alloc_fail,
    );
    emit_build_div_result(builder, &mut vregs, quotient, remainder, "parts", &alloc_fail)?;
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&zero_divisor),
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
        type_: ParameterType::named(DIV_RESULT_TYPE_ID),
        location: Operand::from("void"),
        text: "big.divMod".to_string(),
    })
}
