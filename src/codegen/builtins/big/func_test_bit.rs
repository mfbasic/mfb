//! `big::testBit` — whether one bit of a `big::Int`'s absolute value is set.

use super::gen_big::{emit_load_int, emit_reject_negative, emit_spill_args};
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

const INTRO: &str = r#"Test one bit of a `big::Int`'s absolute value."#;
const DESC: &str = r#"`big::testBit(a, index)` returns `TRUE` when bit `index` of the absolute value of
`a` is `1`. Bit `0` is the lowest; bit `index` is worth 2 raised to `index`.

The bit is read from the absolute value, so `a` and `big::negate(a)` answer the same:
`big::testBit(-5, 0)` is `TRUE`, as it is for `5`. There is no two's-complement
reading of a negative value.

`index` must be zero or more; a negative `index` raises `ErrInvalidArgument`. An `index`
at or past `big::bitLength(a)` returns `FALSE`."#;
const EX: &str = r#"The bits of 5 are `101`:

```
IMPORT big
IMPORT io

SUB main()
  LET five AS big::Int = big::fromInteger(5)
  io::print(toString(big::testBit(five, 0)) & " " & toString(big::testBit(five, 1)) & " " & toString(big::testBit(five, 2)))
  io::print(toString(big::testBit(big::negate(five), 0)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "testBit",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "The value whose absolute value is read.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "index",
                    desc: "The bit to read, `0` being the lowest. Zero or more; a negative index raises `ErrInvalidArgument`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_function(lower_test_bit),
        }],
    });
}

/// `big::testBit`: `FALSE` past the magnitude; otherwise the bit of byte `index / 8`.
pub(crate) fn lower_test_bit(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let invalid = format!("{symbol}_invalid");
    let done = format!("{symbol}_done");
    let answered = format!("{symbol}_answered");
    emit_reject_negative(builder, &mut vregs, arg_slots[1], &invalid);
    let a = emit_load_int(builder, &mut vregs, arg_slots[0], "a");
    let index = vregs.next();
    let byte_index = vregs.next();
    let count = vregs.next();
    let byte = vregs.next();
    let mask = vregs.next();
    let answer = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&index, abi::stack_pointer(), arg_slots[1]),
        abi::shift_right_immediate(&byte_index, &index, 3),
        abi::load_u64(&count, abi::stack_pointer(), a.count),
        abi::move_immediate(&answer, "Boolean", "0"),
        abi::compare_registers(&byte_index, &count),
        abi::branch_ge(&answered),
        abi::load_u64(&byte, abi::stack_pointer(), a.data),
        abi::add_registers(&byte, &byte, &byte_index),
        abi::load_u8(&byte, &byte, 0),
        abi::move_immediate(&mask, "Integer", "7"),
        abi::and_registers(&index, &index, &mask),
        abi::shift_right_variable(&byte, &byte, &index),
        abi::move_immediate(&mask, "Integer", "1"),
        abi::and_registers(&answer, &byte, &mask),
        abi::label(&answered),
        abi::move_register(RESULT_VALUE_REGISTER, &answer),
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
    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from("void"),
        text: "big.testBit".to_string(),
    })
}
