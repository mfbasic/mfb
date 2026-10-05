//! `big::factorial` — `n!` as a `big::Int`.

use super::gen_big::{emit_alloc_magnitude, emit_build_int, emit_reject_negative, emit_spill_args};
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

const INTRO: &str = r#"Compute `n` factorial as a `big::Int`."#;
const DESC: &str = r#"`big::factorial(n)` returns `n!`, the product `1 * 2 * ... * n`. `0!` and `1!` are both
`1`.

`n` must be zero or more; a negative `n` raises `ErrInvalidArgument`. There is no upper
limit on the result's size — `20!` is the largest factorial an `Integer` holds, and larger
ones simply grow — but the work grows quickly with `n`.

The whole product is computed in one call, rather than a `big::multiply` call per
factor."#;
const EX: &str = r#"A factorial past the `Integer` range:

```
IMPORT big
IMPORT io

FUNC main() AS Nothing
  io::print(big::toString(big::factorial(25)))
  io::print(big::toString(big::factorial(0)))
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "factorial",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "n",
                desc: "How many factors to multiply. Zero or more; a negative `n` raises `ErrInvalidArgument`.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec!["ErrInvalidArgument", "ErrOutOfMemory"],
            body: Body::abi_function(lower_factorial),
        }],
    });
}

/// `big::factorial`: one result block, multiplied in place (bug-775).
///
/// A first pass bounds the result: `n!` has at most `1 + sum(bitLength(k))` bits for
/// `k = 2..n`, so the block is made once at that size and nothing is allocated or
/// released while the product grows. The factors are then packed into words — `k`,
/// `k + 1`, ... multiply into one word while the product still fits 64 bits (`umulh` is
/// zero) — and each packed word multiplies the magnitude in one word-wide pass, so
/// `2000!` takes about 400 passes rather than 2,000 multiplications.
pub(crate) fn lower_factorial(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let n_slot = arg_slots[0];
    let capacity = builder.allocate_stack_object("big_capacity", 8);
    let used = builder.allocate_stack_object("big_used", 8);
    let negative = builder.allocate_stack_object("big_negative", 8);
    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let bound = format!("{symbol}_bound");
    let bounded = format!("{symbol}_bounded");
    let step = format!("{symbol}_step");
    let pack = format!("{symbol}_pack");
    let packed = format!("{symbol}_packed");
    let multiply = format!("{symbol}_multiply");
    let multiplied = format!("{symbol}_multiplied");
    let no_carry = format!("{symbol}_no_carry");
    let finished = format!("{symbol}_finished");

    emit_reject_negative(builder, &mut vregs, n_slot, &invalid);
    let (n, k, sum, zeros) = (vregs.next(), vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), negative),
        abi::load_u64(&n, abi::stack_pointer(), n_slot),
        abi::move_immediate(&k, "Integer", "2"),
        abi::move_immediate(&sum, "Integer", "1"),
        abi::label(&bound),
        abi::compare_registers(&k, &n),
        abi::branch_gt(&bounded),
        // sum += bitLength(k) = 64 - clz(k)
        abi::count_leading_zeros(&zeros, &k),
        abi::add_immediate(&sum, &sum, 64),
        abi::subtract_registers(&sum, &sum, &zeros),
        abi::add_immediate(&k, &k, 1),
        abi::branch(&bound),
        abi::label(&bounded),
        // capacity = 8 * (sum / 64 + 2) bytes: the bound's words plus a spare.
        abi::shift_right_immediate(&sum, &sum, 6),
        abi::add_immediate(&sum, &sum, 2),
        abi::shift_left_immediate(&sum, &sum, 3),
        abi::store_u64(&sum, abi::stack_pointer(), capacity),
    ]);
    let result = emit_alloc_magnitude(builder, &mut vregs, capacity, "r", &alloc_fail);
    let (data, words, factor, high, cursor, end, word, low, carry, carry_out) = (
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
        vregs.next(),
    );
    let (n, k) = (vregs.next(), vregs.next());
    builder.instructions.extend([
        // The product starts as the one-word value 1. The loop is call-free, so its
        // state stays in registers.
        abi::load_u64(&data, abi::stack_pointer(), result.data),
        abi::move_immediate(&word, "Integer", "1"),
        abi::store_u64(&word, &data, 0),
        abi::move_immediate(&words, "Integer", "1"),
        abi::load_u64(&n, abi::stack_pointer(), n_slot),
        abi::move_immediate(&k, "Integer", "2"),
        abi::label(&step),
        abi::compare_registers(&k, &n),
        abi::branch_gt(&finished),
        // factor = k * (k + 1) * ... while it fits one word.
        abi::move_register(&factor, &k),
        abi::add_immediate(&k, &k, 1),
        abi::label(&pack),
        abi::compare_registers(&k, &n),
        abi::branch_gt(&packed),
        abi::unsigned_multiply_high_registers(&high, &factor, &k),
        abi::compare_immediate(&high, "0"),
        abi::branch_ne(&packed),
        abi::multiply_registers(&factor, &factor, &k),
        abi::add_immediate(&k, &k, 1),
        abi::branch(&pack),
        abi::label(&packed),
        // magnitude *= factor, one word-wide pass; the carry out is a new top word.
        abi::move_register(&cursor, &data),
        abi::shift_left_immediate(&end, &words, 3),
        abi::add_registers(&end, &data, &end),
        abi::move_immediate(&carry, "Integer", "0"),
        abi::label(&multiply),
        abi::compare_registers(&cursor, &end),
        abi::branch_eq(&multiplied),
        abi::load_u64(&word, &cursor, 0),
        abi::multiply_registers(&low, &word, &factor),
        abi::unsigned_multiply_high_registers(&high, &word, &factor),
        abi::add_carry(&low, &carry_out, &low, &carry, abi::ZERO),
        abi::add_registers(&carry, &high, &carry_out),
        abi::store_u64(&low, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::branch(&multiply),
        abi::label(&multiplied),
        abi::compare_immediate(&carry, "0"),
        abi::branch_eq(&no_carry),
        abi::store_u64(&carry, &cursor, 0),
        abi::add_immediate(&words, &words, 1),
        abi::label(&no_carry),
        abi::branch(&step),
        abi::label(&finished),
        abi::shift_left_immediate(&words, &words, 3),
        abi::store_u64(&words, abi::stack_pointer(), used),
    ]);
    emit_build_int(builder, &mut vregs, &result, used, negative, "r");
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
        text: "big.factorial".to_string(),
    })
}
