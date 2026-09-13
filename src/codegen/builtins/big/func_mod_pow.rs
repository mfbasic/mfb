//! `big::modPow` — `base` raised to `exponent`, reduced by `modulus`, without the full power.

use super::gen_big::{
    emit_div_mod_int, emit_int_from_integer, emit_load_int, emit_mul_int, emit_release_int,
    emit_spill_args,
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

const INTRO: &str = r#"Raise a `big::Int` to a power modulo another, without building the full power."#;
const DESC: &str = r#"`big::modPow(base, exponent, modulus)` returns `base` raised to `exponent`, reduced
by `modulus`. It gives the same value as
`big::remainder(big::pow(base, exponent), modulus)` — so a non-zero result takes the sign
the full power would have — but reduces after every step, so the numbers never grow past
twice the size of `modulus`. That is what makes a huge `exponent` practical here, and it is
why `exponent` is a `big::Int` where `big::pow` takes an `Integer`.

`modulus` must not be zero, and `exponent` must be zero or more; either mistake raises
`ErrInvalidArgument`. An `exponent` of zero gives `1` reduced by `modulus` (`0` when
`modulus` is `1` or `-1`).

**Not constant-time — never use it with a secret.** How long this call takes, and which
bytes it reads, depend on the bits of `exponent` and on the values involved, so an
observer who can time it learns about a secret exponent or modulus. That is exactly the
leak that breaks RSA and Diffie-Hellman implementations. For anything cryptographic use
`crypto::`, whose key and signature operations are built from constant-time primitives for
this reason."#;
const EX: &str = r#"A power far too large to build, reduced:

```
IMPORT big
IMPORT io

SUB main()
  LET exponent AS big::Int = big::pow(big::fromInteger(10), 30)
  io::print(big::toString(big::modPow(big::fromInteger(3), exponent, big::fromInteger(1000000007))))
  io::print(big::toString(big::modPow(big::fromInteger(-2), big::fromInteger(3), big::fromInteger(5))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "modPow",
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
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "modulus",
                    desc: "The value to reduce by. Zero raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_function(lower_mod_pow),
        }],
    });
}

/// `big::modPow`: square-and-multiply over the exponent's magnitude bits, least significant
/// first, reducing with `emit_div_mod_int` after every product. Truncated remainders keep
/// the sign of the running product, so the result equals `remainder(pow(base, e), m)`.
/// Every intermediate is released once replaced.
pub(crate) fn lower_mod_pow(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let one_slot = builder.allocate_stack_object("big_one", 8);
    let seed_slot = builder.allocate_stack_object("big_seed", 8);
    let acc_slot = builder.allocate_stack_object("big_acc", 8);
    let base_slot = builder.allocate_stack_object("big_base", 8);
    let product_slot = builder.allocate_stack_object("big_product", 8);
    let quotient_slot = builder.allocate_stack_object("big_quotient", 8);
    let remainder_slot = builder.allocate_stack_object("big_remainder", 8);
    let bit_slot = builder.allocate_stack_object("big_bit", 8);
    let bits_slot = builder.allocate_stack_object("big_bits", 8);
    let invalid = format!("{symbol}_invalid");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let step = format!("{symbol}_step");
    let skip_multiply = format!("{symbol}_skip_multiply");
    let finished = format!("{symbol}_finished");

    let exponent = emit_load_int(builder, &mut vregs, arg_slots[1], "exponent");
    let modulus = emit_load_int(builder, &mut vregs, arg_slots[2], "modulus");
    let (flag, one, bits) = (vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        // A negative exponent is rejected; a zero modulus is rejected by the first reduction.
        abi::load_u64(&flag, abi::stack_pointer(), exponent.negative),
        abi::compare_immediate(&flag, "0"),
        abi::branch_ne(&invalid),
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(&one, abi::stack_pointer(), one_slot),
        abi::load_u64(&bits, abi::stack_pointer(), exponent.count),
        abi::shift_left_immediate(&bits, &bits, 3),
        abi::store_u64(&bits, abi::stack_pointer(), bits_slot),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), bit_slot),
    ]);

    // acc = 1 mod m
    emit_int_from_integer(builder, &mut vregs, one_slot, "one", &alloc_fail);
    builder
        .instructions
        .push(abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), seed_slot));
    let seed = emit_load_int(builder, &mut vregs, seed_slot, "seed");
    emit_div_mod_int(builder, &mut vregs, &seed, &modulus, quotient_slot, acc_slot, "reduce_one", &invalid, &alloc_fail);
    emit_release_int(builder, &mut vregs, quotient_slot);
    emit_release_int(builder, &mut vregs, seed_slot);

    // base = base mod m
    let base = emit_load_int(builder, &mut vregs, arg_slots[0], "base");
    emit_div_mod_int(builder, &mut vregs, &base, &modulus, quotient_slot, base_slot, "reduce_base", &invalid, &alloc_fail);
    emit_release_int(builder, &mut vregs, quotient_slot);

    builder.instructions.push(abi::label(&step));
    let (bit, total, byte, mask) = (vregs.next(), vregs.next(), vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&bit, abi::stack_pointer(), bit_slot),
        abi::load_u64(&total, abi::stack_pointer(), bits_slot),
        abi::compare_registers(&bit, &total),
        abi::branch_eq(&finished),
        // exponent bit `bit`: (data[bit >> 3] >> (bit & 7)) & 1
        abi::shift_right_immediate(&byte, &bit, 3),
        abi::load_u64(&total, abi::stack_pointer(), exponent.data),
        abi::add_registers(&byte, &total, &byte),
        abi::load_u8(&byte, &byte, 0),
        abi::move_immediate(&mask, "Integer", "7"),
        abi::and_registers(&bit, &bit, &mask),
        abi::shift_right_variable(&byte, &byte, &bit),
        abi::move_immediate(&mask, "Integer", "1"),
        abi::and_registers(&byte, &byte, &mask),
        abi::compare_immediate(&byte, "0"),
        abi::branch_eq(&skip_multiply),
    ]);
    // acc = acc * base mod m
    let acc = emit_load_int(builder, &mut vregs, acc_slot, "acc");
    let factor = emit_load_int(builder, &mut vregs, base_slot, "factor");
    emit_mul_int(builder, &mut vregs, &acc, &factor, "times", &alloc_fail);
    builder
        .instructions
        .push(abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), product_slot));
    let product = emit_load_int(builder, &mut vregs, product_slot, "product");
    emit_div_mod_int(builder, &mut vregs, &product, &modulus, quotient_slot, remainder_slot, "reduce_times", &invalid, &alloc_fail);
    emit_release_int(builder, &mut vregs, quotient_slot);
    emit_release_int(builder, &mut vregs, product_slot);
    emit_release_int(builder, &mut vregs, acc_slot);
    let moved = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&moved, abi::stack_pointer(), remainder_slot),
        abi::store_u64(&moved, abi::stack_pointer(), acc_slot),
        abi::label(&skip_multiply),
    ]);
    // base = base * base mod m
    let first = emit_load_int(builder, &mut vregs, base_slot, "square_a");
    let second = emit_load_int(builder, &mut vregs, base_slot, "square_b");
    emit_mul_int(builder, &mut vregs, &first, &second, "squared", &alloc_fail);
    builder
        .instructions
        .push(abi::store_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), product_slot));
    let squared = emit_load_int(builder, &mut vregs, product_slot, "squared_value");
    emit_div_mod_int(builder, &mut vregs, &squared, &modulus, quotient_slot, remainder_slot, "reduce_square", &invalid, &alloc_fail);
    emit_release_int(builder, &mut vregs, quotient_slot);
    emit_release_int(builder, &mut vregs, product_slot);
    emit_release_int(builder, &mut vregs, base_slot);
    let (moved, next_bit) = (vregs.next(), vregs.next());
    builder.instructions.extend([
        abi::load_u64(&moved, abi::stack_pointer(), remainder_slot),
        abi::store_u64(&moved, abi::stack_pointer(), base_slot),
        abi::load_u64(&next_bit, abi::stack_pointer(), bit_slot),
        abi::add_immediate(&next_bit, &next_bit, 1),
        abi::store_u64(&next_bit, abi::stack_pointer(), bit_slot),
        abi::branch(&step),
        abi::label(&finished),
    ]);
    emit_release_int(builder, &mut vregs, base_slot);
    builder.instructions.extend([
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
        text: "big.modPow".to_string(),
    })
}
