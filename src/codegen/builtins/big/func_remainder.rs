//! `big::remainder` — what is left after dividing one `big::Int` by another.

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

const INTRO: &str = r#"Return what is left after dividing one `big::Int` by another."#;
const DESC: &str = r#"`big::remainder(a, b)` returns what is left after dividing `a` by `b` with the quotient
truncated toward zero. **The remainder takes the sign of the dividend `a`**:
`big::remainder(-7, 2)` is `-1` and `big::remainder(7, -2)` is `1`. This is the rule
`Integer`'s `MOD` follows, so for values that fit an `Integer` the two agree.

The remainder's absolute value is always smaller than `b`'s, and
`a = b * big::divide(a, b) + big::remainder(a, b)` holds for every `a` and every non-zero
`b`.

Dividing by zero raises `ErrInvalidArgument`; nothing else can fail, at any size.
`big::divMod` returns the quotient and remainder from one division."#;
const EX: &str = r#"The remainder follows the dividend's sign:

```
IMPORT big
IMPORT io

SUB main()
  io::print(toString(big::toInteger(big::remainder(big::fromInteger(-7), big::fromInteger(2)))))
  io::print(toString(big::toInteger(big::remainder(big::fromInteger(7), big::fromInteger(-2)))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "remainder",
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
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_function(lower_remainder),
        }],
    });
}

/// `big::remainder`: `emit_div_mod_int`, keep the remainder, release the quotient.
pub(crate) fn lower_remainder(
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
    emit_release_int(builder, &mut vregs, quotient);
    builder.instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), remainder),
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
        text: "big.remainder".to_string(),
    })
}
