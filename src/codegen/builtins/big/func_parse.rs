//! `big::parse` — a `big::Int` from its digits in a radix.

use super::gen_big::{emit_alloc_magnitude, emit_build_int, emit_spill_args};
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

const INTRO: &str = r#"Read a `big::Int` from its digits, in base 10 or another radix."#;
const DESC: &str = r#"`big::parse(text, radix)` reads `text` as a whole number written in base `radix` and
returns it as a `big::Int`.

**The accepted text** is one optional leading `-` followed by one or more digits, with
nothing else: no `+`, no spaces, no separators, no prefix such as `0x`. A digit is `0`–`9`,
then `a`–`z` (or `A`–`Z`, either case) for the values 10 to 35, and every digit must be
below `radix`. Leading zeros are allowed, and `-0` is zero. Anything else — an empty
string, a lone `-`, `12x`, `+-1` — raises `ErrInvalidFormat`.

`radix` defaults to 10 and must be from 2 to 36; any other radix raises
`ErrInvalidArgument`, checked before the text is read.

There is no size limit. `big::toString` and `big::toRadixString` write the text this reads,
so `big::parse(big::toString(a))` is `a` for every value."#;
const EX: &str = r#"Read a number past the `Integer` range and one in hexadecimal:

```
IMPORT big
IMPORT io

SUB main()
  LET huge AS big::Int = big::parse("-123456789012345678901234567890")
  io::print(toString(big::sign(huge)) & " " & toString(big::bitLength(huge)))
  io::print(toString(big::toInteger(big::parse("ff", 16))))
END SUB
```

Text with a leading `+` is rejected:

```
IMPORT big
IMPORT io

FUNC tryParse(text AS String) AS String
  RETURN toString(big::toInteger(big::parse(text)))
  TRAP(e)
    RETURN "rejected " & toString(e.code)
  END TRAP
END FUNC

SUB main()
  io::print(tryParse("+5"))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "parse",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "text",
                    desc: "An optional `-` and at least one digit valid in `radix`, with nothing else.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "radix",
                    desc: "The base the digits are written in, 2 to 36. Optional, defaulting to 10.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::Integer,
                        expr: "10",
                    },
                },
            ],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec!["ErrInvalidFormat", "ErrInvalidArgument"],
            body: Body::abi_function(lower_parse),
        }],
    });
}

/// `big::parse`: check the radix, validate every character in one pass (so a malformed
/// text is rejected before anything is made), then accumulate
/// `magnitude = magnitude * radix + digit` over byte limbs. A digit adds less than one
/// byte (`radix <= 36 < 256`), so `length + 1` bytes always hold the result, and each
/// step's carry stays below `255 * 36 + 35 = 9215`.
pub(crate) fn lower_parse(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let (text_slot, radix_slot) = (arg_slots[0], arg_slots[1]);
    let capacity = builder.allocate_stack_object("big_capacity", 8);
    let negative = builder.allocate_stack_object("big_negative", 8);
    let start = builder.allocate_stack_object("big_start", 8);
    let used = builder.allocate_stack_object("big_used", 8);
    let invalid_radix = format!("{symbol}_invalid_radix");
    let invalid_text = format!("{symbol}_invalid_text");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    // Radix first: 2..=36.
    let radix = vregs.next();
    let text = vregs.next();
    let length = vregs.next();
    let byte = vregs.next();
    let flag = vregs.next();
    let position = vregs.next();
    let unsigned = format!("{symbol}_unsigned");
    builder.instructions.extend([
        abi::load_u64(&radix, abi::stack_pointer(), radix_slot),
        abi::compare_immediate(&radix, "2"),
        abi::branch_lt(&invalid_radix),
        abi::compare_immediate(&radix, "36"),
        abi::branch_gt(&invalid_radix),
        // The sign: one optional leading '-'.
        abi::load_u64(&text, abi::stack_pointer(), text_slot),
        abi::load_u64(&length, &text, 0),
        abi::compare_immediate(&length, "0"),
        abi::branch_eq(&invalid_text),
        abi::move_immediate(&flag, "Integer", "0"),
        abi::move_immediate(&position, "Integer", "0"),
        abi::load_u8(&byte, &text, 8),
        abi::compare_immediate(&byte, "45"),
        abi::branch_ne(&unsigned),
        abi::move_immediate(&flag, "Integer", "1"),
        abi::move_immediate(&position, "Integer", "1"),
        // A lone '-' has no digits.
        abi::compare_registers(&position, &length),
        abi::branch_eq(&invalid_text),
        abi::label(&unsigned),
        abi::store_u64(&flag, abi::stack_pointer(), negative),
        abi::store_u64(&position, abi::stack_pointer(), start),
        abi::add_immediate(&length, &length, 1),
        abi::store_u64(&length, abi::stack_pointer(), capacity),
    ]);

    // Pass 1: every remaining byte must be a digit below the radix.
    let check_loop = format!("{symbol}_check");
    let checked = format!("{symbol}_checked");
    emit_digit_value(builder, &mut vregs, text_slot, radix_slot, start, &check_loop, &checked, &invalid_text, None, &symbol);

    let result = emit_alloc_magnitude(builder, &mut vregs, capacity, "r", &alloc_fail);
    builder
        .instructions
        .push(abi::store_u64(abi::ZERO, abi::stack_pointer(), used));

    // Pass 2: accumulate. The digits are known valid, so nothing here can fail.
    let fold_loop = format!("{symbol}_fold");
    let folded = format!("{symbol}_folded");
    emit_digit_value(
        builder,
        &mut vregs,
        text_slot,
        radix_slot,
        start,
        &fold_loop,
        &folded,
        &invalid_text,
        Some((&result, used)),
        &symbol,
    );

    emit_build_int(builder, &mut vregs, &result, used, negative, "r");
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&invalid_radix),
    ]);
    emit_fail(
        &symbol,
        "ErrInvalidArgument",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );
    builder.instructions.push(abi::label(&invalid_text));
    emit_fail(
        &symbol,
        "ErrInvalidFormat",
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
        text: "big.parse".to_string(),
    })
}

/// One walk over the digits from `start_slot` to the end of the `String` in `text_slot`.
/// Each byte is mapped to its value (`0`-`9`, `a`-`z`, `A`-`Z`); a byte that is not a
/// digit, or a digit not below the radix, branches to `invalid`. With `accumulate` set
/// to a result and its used-byte slot, the walk also folds each digit in:
/// `magnitude = magnitude * radix + digit`.
#[allow(clippy::too_many_arguments)]
fn emit_digit_value(
    builder: &mut CodeBuilder,
    vregs: &mut Vregs,
    text_slot: usize,
    radix_slot: usize,
    start_slot: usize,
    walk: &str,
    walked: &str,
    invalid: &str,
    accumulate: Option<(&super::gen_big::ResultSlots, usize)>,
    symbol: &str,
) {
    let tag = if accumulate.is_some() { "fold" } else { "check" };
    let lower = format!("{symbol}_{tag}_lower");
    let upper = format!("{symbol}_{tag}_upper");
    let valued = format!("{symbol}_{tag}_valued");
    let text = vregs.next();
    let length = vregs.next();
    let position = vregs.next();
    let byte = vregs.next();
    let radix = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&text, abi::stack_pointer(), text_slot),
        abi::load_u64(&length, &text, 0),
        abi::load_u64(&position, abi::stack_pointer(), start_slot),
        abi::load_u64(&radix, abi::stack_pointer(), radix_slot),
        abi::label(walk),
        abi::compare_registers(&position, &length),
        abi::branch_eq(walked),
        abi::add_registers(&byte, &text, &position),
        abi::load_u8(&byte, &byte, 8),
        // '0'..'9' -> 0..9, 'a'..'z' -> 10..35, 'A'..'Z' -> 10..35.
        abi::compare_immediate(&byte, "48"),
        abi::branch_lt(invalid),
        abi::compare_immediate(&byte, "57"),
        abi::branch_gt(&upper),
        abi::subtract_immediate(&byte, &byte, 48),
        abi::branch(&valued),
        abi::label(&upper),
        abi::compare_immediate(&byte, "65"),
        abi::branch_lt(invalid),
        abi::compare_immediate(&byte, "90"),
        abi::branch_gt(&lower),
        abi::subtract_immediate(&byte, &byte, 55),
        abi::branch(&valued),
        abi::label(&lower),
        abi::compare_immediate(&byte, "97"),
        abi::branch_lt(invalid),
        abi::compare_immediate(&byte, "122"),
        abi::branch_gt(invalid),
        abi::subtract_immediate(&byte, &byte, 87),
        abi::label(&valued),
        abi::compare_registers(&byte, &radix),
        abi::branch_ge(invalid),
    ]);
    if let Some((result, used_slot)) = accumulate {
        let times = format!("{symbol}_{tag}_times");
        let timed = format!("{symbol}_{tag}_timed");
        let no_carry = format!("{symbol}_{tag}_no_carry");
        let used = vregs.next();
        let index = vregs.next();
        let dst = vregs.next();
        let cursor = vregs.next();
        let limb = vregs.next();
        builder.instructions.extend([
            // byte is the incoming carry: the digit.
            abi::load_u64(&used, abi::stack_pointer(), used_slot),
            abi::load_u64(&dst, abi::stack_pointer(), result.data),
            abi::move_immediate(&index, "Integer", "0"),
            abi::label(&times),
            abi::compare_registers(&index, &used),
            abi::branch_eq(&timed),
            abi::add_registers(&cursor, &dst, &index),
            abi::load_u8(&limb, &cursor, 0),
            abi::multiply_registers(&limb, &limb, &radix),
            abi::add_registers(&limb, &limb, &byte),
            abi::store_u8(&limb, &cursor, 0),
            abi::shift_right_immediate(&byte, &limb, 8),
            abi::add_immediate(&index, &index, 1),
            abi::branch(&times),
            abi::label(&timed),
            abi::compare_immediate(&byte, "0"),
            abi::branch_eq(&no_carry),
            abi::add_registers(&cursor, &dst, &used),
            abi::store_u8(&byte, &cursor, 0),
            abi::add_immediate(&used, &used, 1),
            abi::store_u64(&used, abi::stack_pointer(), used_slot),
            abi::label(&no_carry),
        ]);
    }
    builder.instructions.extend([
        abi::add_immediate(&position, &position, 1),
        abi::branch(walk),
        abi::label(walked),
    ]);
}
