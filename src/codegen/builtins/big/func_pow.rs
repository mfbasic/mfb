//! `big::pow` — a `big::Int` raised to a non-negative `Integer` power.

use super::gen_big::{
    emit_int_from_integer, emit_load_int, emit_mul_int, emit_reject_negative,
    emit_release_int, emit_spill_args,
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

const INTRO: &str = r#"Raise a `big::Int` to a non-negative whole power."#;
const DESC: &str = r#"`big::pow(base, exponent)` returns `base` multiplied by itself `exponent` times.
`big::pow(base, 0)` is `1` for every `base`, including zero, and `big::pow(base, 1)` is
`base`. A negative `base` gives a negative result exactly when `exponent` is odd.

`exponent` must be zero or more; a negative `exponent` raises `ErrInvalidArgument`. It is
an `Integer`, not a `big::Int`: a power large enough to need one would produce a result no
machine can hold. The result grows with `exponent` times the size of `base`, so large
powers of large values are slow and large.

For a power reduced modulo some value — the case where the result stays small — use
`big::modPow`, which never builds the full power."#;
const EX: &str = r#"Powers of two and a negative base:

```
IMPORT big
IMPORT io

SUB main()
  io::print(big::toString(big::pow(big::fromInteger(2), 100)))
  io::print(big::toString(big::pow(big::fromInteger(-3), 3)))
  io::print(big::toString(big::pow(big::fromInteger(0), 0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pow",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "base",
                    desc: "The value to raise.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "exponent",
                    desc: "The power. Zero or more; a negative exponent raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_function(lower_pow),
        }],
    });
}

/// `big::pow`: square-and-multiply over the exponent's bits. The running square starts as
/// the argument itself — never released — and every value this body makes is released
/// once it is replaced.
pub(crate) fn lower_pow(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let (base_slot, exponent_slot) = (arg_slots[0], arg_slots[1]);
    let one_slot = builder.allocate_stack_object("big_one", 8);
    let acc_slot = builder.allocate_stack_object("big_acc", 8);
    let square_slot = builder.allocate_stack_object("big_square", 8);
    let owned_slot = builder.allocate_stack_object("big_square_owned", 8);
    let remaining_slot = builder.allocate_stack_object("big_remaining", 8);
    let next_slot = builder.allocate_stack_object("big_next", 8);
    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let step = format!("{symbol}_step");
    let skip_multiply = format!("{symbol}_skip_multiply");
    let finished = format!("{symbol}_finished");
    let square_borrowed = format!("{symbol}_square_borrowed");
    let keep_argument = format!("{symbol}_keep_argument");

    emit_reject_negative(builder, &mut vregs, exponent_slot, &invalid);
    let (one, square, exponent) = (vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, abi::stack_pointer(), one_slot),
        abi::load_u64(&square, abi::stack_pointer(), base_slot),
        abi::store_u64(&square, abi::stack_pointer(), square_slot),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), owned_slot),
        abi::load_u64(&exponent, abi::stack_pointer(), exponent_slot),
        abi::store_u64(&exponent, abi::stack_pointer(), remaining_slot),
    ]);
    emit_int_from_integer(builder, &mut vregs, one_slot, "one", &alloc_fail);
    builder.instructions.extend([
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), acc_slot),
        abi::label(&step),
    ]);

    // Multiply the accumulator by the square when the low exponent bit is set.
    let (remaining, bit, mask) = (vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&remaining, abi::stack_pointer(), remaining_slot),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&finished),
        abi::move_immediate(&mask, "Integer", "1"),
        abi::and_registers(&bit, &remaining, &mask),
        abi::compare_immediate(&bit, "0"),
        abi::branch_eq(&skip_multiply),
    ]);
    let acc = emit_load_int(builder, &mut vregs, acc_slot, "acc");
    let square = emit_load_int(builder, &mut vregs, square_slot, "square");
    emit_mul_int(builder, &mut vregs, &acc, &square, "times", &alloc_fail);
    builder.instructions.push(abi::store_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        next_slot,
    ));
    emit_release_int(builder, &mut vregs, acc_slot);
    let moved = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&moved, abi::stack_pointer(), next_slot),
        abi::store_u64(&moved, abi::stack_pointer(), acc_slot),
        abi::label(&skip_multiply),
    ]);

    // Halve the exponent; square again while bits remain.
    let (remaining, owned) = (vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&remaining, abi::stack_pointer(), remaining_slot),
        abi::shift_right_immediate(&remaining, &remaining, 1),
        abi::store_u64(&remaining, abi::stack_pointer(), remaining_slot),
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(&finished),
    ]);
    let first = emit_load_int(builder, &mut vregs, square_slot, "square_a");
    let second = emit_load_int(builder, &mut vregs, square_slot, "square_b");
    emit_mul_int(builder, &mut vregs, &first, &second, "squared", &alloc_fail);
    builder.instructions.extend([
        abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), next_slot),
        abi::load_u64(&owned, abi::stack_pointer(), owned_slot),
        abi::compare_immediate(&owned, "0"),
        abi::branch_eq(&square_borrowed),
    ]);
    emit_release_int(builder, &mut vregs, square_slot);
    let (moved, flag) = (vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::label(&square_borrowed),
        abi::load_u64(&moved, abi::stack_pointer(), next_slot),
        abi::store_u64(&moved, abi::stack_pointer(), square_slot),
        abi::move_immediate(&flag, "Integer", "1"),
        abi::store_u64(&flag, abi::stack_pointer(), owned_slot),
        abi::branch(&step),
        abi::label(&finished),
    ]);

    // The square is released unless it is still the argument.
    let owned = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&owned, abi::stack_pointer(), owned_slot),
        abi::compare_immediate(&owned, "0"),
        abi::branch_eq(&keep_argument),
    ]);
    emit_release_int(builder, &mut vregs, square_slot);
    builder.instructions.extend([
        abi::label(&keep_argument),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), acc_slot),
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
        text: "big.pow".to_string(),
    })
}
