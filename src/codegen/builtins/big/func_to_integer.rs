//! `big::toInteger` — the `Integer` equal to a `big::Int`, or `ErrOverflow`.

use super::gen_big::{emit_load_int, emit_spill_args};
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

/// 2^63 as an unsigned word: the one magnitude above the `Integer` maximum that a
/// negative value may still have (it is `Integer`'s minimum).
const INTEGER_MIN_MAGNITUDE: &str = "9223372036854775808";

const INTRO: &str = r#"Convert a `big::Int` to an `Integer`, raising when it does not fit."#;
const DESC: &str = r#"`big::toInteger` returns the `Integer` equal to `value`.

An `Integer` holds `-9223372036854775808` through `9223372036854775807`. A `big::Int`
outside that range raises `ErrOverflow`; the check is exact at both ends, so the most
negative `Integer` converts and one below it raises.

A `big::Int` built by hand with trailing zero bytes in its `magnitude`, or with
`negative` set on a zero magnitude, converts as the number it spells.

`big::fromInteger` converts the other way and never raises, so
`big::toInteger(big::fromInteger(n))` is `n` for every `Integer`."#;
const EX: &str = r#"Convert back to an `Integer`:

```
IMPORT big
IMPORT io

SUB main()
  io::print(toString(big::toInteger(big::fromInteger(9000))))
END SUB
```

A value past the `Integer` range raises `ErrOverflow`:

```
IMPORT big
IMPORT io

FUNC describe(value AS big::Int) AS String
  RETURN toString(big::toInteger(value))
  TRAP(e)
    RETURN "does not fit"
  END TRAP
END FUNC

SUB main()
  LET twoTo64 AS List OF Byte = [0, 0, 0, 0, 0, 0, 0, 0, 1]
  io::print(describe(big::fromBytes(twoTo64, FALSE)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toInteger",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The value to convert. Must lie in the `Integer` range, or the call raises `ErrOverflow`.",
                aliases: &[],
                ty: ParameterType::named(INT_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec!["ErrOverflow"],
            body: Body::abi_function(lower_to_integer),
        }],
    });
}

/// `big::toInteger`: reject more than eight significant bytes, assemble the word from
/// the high byte down, then apply the sign with the exact range check.
pub(crate) fn lower_to_integer(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let operand = emit_load_int(builder, &mut vregs, arg_slots[0], "v");
    let overflow = format!("{symbol}_overflow");
    let done = format!("{symbol}_done");
    let gather = format!("{symbol}_gather");
    let gathered = format!("{symbol}_gathered");
    let negative = format!("{symbol}_negative");
    let store = format!("{symbol}_store");

    let data = vregs.next();
    let index = vregs.next();
    let acc = vregs.next();
    let byte = vregs.next();
    let cursor = vregs.next();
    let flag = vregs.next();
    let limit = vregs.next();
    let zero = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&data, abi::stack_pointer(), operand.data),
        abi::load_u64(&index, abi::stack_pointer(), operand.count),
        abi::compare_immediate(&index, "8"),
        abi::branch_gt(&overflow),
        abi::move_immediate(&acc, "Integer", "0"),
        abi::label(&gather),
        abi::compare_immediate(&index, "0"),
        abi::branch_eq(&gathered),
        abi::subtract_immediate(&index, &index, 1),
        abi::add_registers(&cursor, &data, &index),
        abi::load_u8(&byte, &cursor, 0),
        abi::shift_left_immediate(&acc, &acc, 8),
        abi::or_registers(&acc, &acc, &byte),
        abi::branch(&gather),
        abi::label(&gathered),
        abi::load_u64(&flag, abi::stack_pointer(), operand.negative),
        abi::compare_immediate(&flag, "0"),
        abi::branch_ne(&negative),
        // Non-negative: the top bit must be clear.
        abi::compare_immediate(&acc, "0"),
        abi::branch_lt(&overflow),
        abi::branch(&store),
        abi::label(&negative),
        // Negative: a magnitude below 2^63 negates normally; exactly 2^63 is already
        // `Integer`'s minimum bit pattern; anything larger does not fit.
        abi::compare_immediate(&acc, "0"),
        abi::branch_lt(&format!("{symbol}_top_bit")),
        abi::move_immediate(&zero, "Integer", "0"),
        abi::subtract_registers(&acc, &zero, &acc),
        abi::branch(&store),
        abi::label(&format!("{symbol}_top_bit")),
        abi::move_immediate(&limit, "Integer", INTEGER_MIN_MAGNITUDE),
        abi::compare_registers(&acc, &limit),
        abi::branch_ne(&overflow),
        abi::label(&store),
        abi::move_register(RESULT_VALUE_REGISTER, &acc),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&overflow),
    ]);
    emit_fail(
        &symbol,
        "ErrOverflow",
        &mut builder.instructions,
        &mut builder.relocations,
        &done,
    );
    builder
        .instructions
        .extend([abi::label(&done), abi::return_()]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from("void"),
        text: "big.toInteger".to_string(),
    })
}
