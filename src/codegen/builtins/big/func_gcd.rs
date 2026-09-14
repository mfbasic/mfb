//! `big::gcd` — the greatest common divisor of two `big::Int` values.

use super::gen_big::{
    emit_copy_int, emit_div_mod_int, emit_load_int, emit_release_int, emit_spill_args,
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

const INTRO: &str = r#"Return the greatest common divisor of two `big::Int` values."#;
const DESC: &str = r#"`big::gcd(a, b)` returns the largest non-negative value that divides both `a` and `b`
exactly.

The result is **never negative**, whatever the signs of `a` and `b`: `big::gcd(-12, 18)`
is `6`. The call is total and never raises. It is defined for every pair, including
zero: `big::gcd(a, 0)` is the absolute value of `a`, and `big::gcd(0, 0)` is `0`.

The work is Euclid's algorithm over `big::remainder`, so it grows with the size of the
operands."#;
const EX: &str = r#"Signs do not matter, and zero is handled:

```
IMPORT big
IMPORT io

SUB main()
  io::print(big::toString(big::gcd(big::fromInteger(-12), big::fromInteger(18))))
  io::print(big::toString(big::gcd(big::fromInteger(0), big::fromInteger(-7))))
  io::print(big::toString(big::gcd(big::fromInteger(0), big::fromInteger(0))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "gcd",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The first value. Its sign does not affect the result.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "The second value. Its sign does not affect the result.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_gcd),
        }],
    });
}

/// `big::gcd`: Euclid over non-negative copies — `(x, y) = (y, x mod y)` until `y` is
/// zero. Every value here is this body's own copy, released as soon as it is replaced, so
/// only the final `x` survives.
pub(crate) fn lower_gcd(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let zero_sign = builder.allocate_stack_object("big_zero_sign", 8);
    let x_slot = builder.allocate_stack_object("big_x", 8);
    let y_slot = builder.allocate_stack_object("big_y", 8);
    let quotient_slot = builder.allocate_stack_object("big_quotient", 8);
    let remainder_slot = builder.allocate_stack_object("big_remainder", 8);
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let step = format!("{symbol}_step");
    let finished = format!("{symbol}_finished");

    builder
        .instructions
        .push(abi::store_u64(abi::ZERO, abi::stack_pointer(), zero_sign));
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    emit_copy_int(builder, &mut vregs, &a, zero_sign, "x", &alloc_fail);
    builder.instructions.push(abi::store_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        x_slot,
    ));
    let b = emit_load_int(builder, &mut vregs, arg_slots[1], "b");
    emit_copy_int(builder, &mut vregs, &b, zero_sign, "y", &alloc_fail);
    builder.instructions.extend([
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), y_slot),
        abi::label(&step),
    ]);
    let x = emit_load_int(builder, &mut vregs, x_slot, "x_step");
    let y = emit_load_int(builder, &mut vregs, y_slot, "y_step");
    let count = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&count, abi::stack_pointer(), y.count),
        abi::compare_immediate(&count, "0"),
        abi::branch_eq(&finished),
    ]);
    // `y` was just checked non-zero, so the zero-divisor exit is never taken; it leads to
    // the same place a zero `y` does.
    emit_div_mod_int(
        builder,
        &mut vregs,
        &x,
        &y,
        quotient_slot,
        remainder_slot,
        "euclid",
        &finished,
        &alloc_fail,
    );
    emit_release_int(builder, &mut vregs, quotient_slot);
    emit_release_int(builder, &mut vregs, x_slot);
    let (moved, remainder) = (vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&moved, abi::stack_pointer(), y_slot),
        abi::store_u64(&moved, abi::stack_pointer(), x_slot),
        abi::load_u64(&remainder, abi::stack_pointer(), remainder_slot),
        abi::store_u64(&remainder, abi::stack_pointer(), y_slot),
        abi::branch(&step),
        abi::label(&finished),
    ]);
    emit_release_int(builder, &mut vregs, y_slot);
    builder.instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), x_slot),
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
        text: "big.gcd".to_string(),
    })
}
