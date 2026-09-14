//! `big::divide` — the quotient of two `big::Int` values, truncated toward zero.

use super::gen_big::{emit_div_mod_int, emit_load_int, emit_release_int, emit_spill_args};
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

const INTRO: &str = r#"Divide one `big::Int` by another, truncating toward zero."#;
const DESC: &str = r#"`big::divide(a, b)` returns the quotient of `a` divided by `b`, **truncated toward
zero**: `big::divide(-7, 2)` is `-3`, not `-4`. This is the rule `Integer`'s `/` follows,
so for values that fit an `Integer` the two agree.

Dividing by zero raises `ErrInvalidArgument`; nothing else can fail, at any size.

`big::remainder` is what is left over, and `big::divMod` returns both from one division.
Calling `big::divide` and `big::remainder` separately divides twice."#;
const EX: &str = r#"Truncation toward zero, and a division by zero:

```
IMPORT big
IMPORT io

FUNC quotient(a AS Integer, b AS Integer) AS String
  RETURN toString(big::toInteger(big::divide(big::fromInteger(a), big::fromInteger(b))))
  TRAP(e)
    RETURN "raised " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print(quotient(-7, 2))
  io::print(quotient(7, 0))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "divide",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The dividend.",
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
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_function(lower_divide),
        }],
    });
}

/// `big::divide`: `emit_div_mod_int`, keep the quotient, release the remainder.
pub(crate) fn lower_divide(
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
    emit_release_int(builder, &mut vregs, remainder);
    builder.instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), quotient),
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
        type_: ParameterType::named(INT_TYPE_ID),
        location: Operand::from("void"),
        text: "big.divide".to_string(),
    })
}
