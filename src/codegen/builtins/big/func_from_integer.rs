//! `big::fromInteger` — the `big::Int` equal to an `Integer`.

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

const INTRO: &str = r#"Build a `big::Int` holding the same value as an `Integer`."#;
const DESC: &str = r#"`big::fromInteger` returns a `big::Int` equal to `value`.

Every `Integer` fits, including the most negative one, `-9223372036854775808`, whose
absolute value is one past the largest `Integer`. The call therefore never raises.

The result is in canonical form: zero has an empty `magnitude` and a `FALSE`
`negative`, and no value carries trailing zero bytes in its `magnitude`.

`big::toInteger` converts back, and raises `ErrOverflow` for a `big::Int` outside the
`Integer` range."#;
const EX: &str = r#"Round-trip a value through `big::Int`:

```
IMPORT big
IMPORT io

SUB main()
  LET x AS big::Int = big::fromInteger(-42)
  io::print(toString(big::toInteger(x)))
END SUB
```

Zero has an empty magnitude:

```
IMPORT big
IMPORT io

SUB main()
  LET zero AS big::Int = big::fromInteger(0)
  io::print(toString(len(zero.magnitude)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fromInteger",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The value to convert. Any `Integer` is accepted.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_from_integer),
        }],
    });
}

/// `big::fromInteger`: an 8-byte magnitude from the absolute value, then
/// `emit_build_int` trims it. A failed allocation raises `ErrOutOfMemory`, which is not a
/// declared error (the `io::readChar` / `crypto::hash` convention).
pub(crate) fn lower_from_integer(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let capacity = builder.allocate_stack_object("big_capacity", 8);
    let count = builder.allocate_stack_object("big_count", 8);
    let negative = builder.allocate_stack_object("big_negative", 8);
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let positive = format!("{symbol}_positive");
    let write_loop = format!("{symbol}_write");
    let write_done = format!("{symbol}_write_done");

    let eight = vregs.next();
    builder.instructions.extend([
        abi::move_immediate(&eight, "Integer", "8"),
        abi::store_u64(&eight, abi::stack_pointer(), capacity),
        abi::store_u64(&eight, abi::stack_pointer(), count),
    ]);
    let result = emit_alloc_magnitude(builder, &mut vregs, capacity, "r", &alloc_fail);

    let value = vregs.next();
    let flag = vregs.next();
    let zero = vregs.next();
    let cursor = vregs.next();
    let index = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&value, abi::stack_pointer(), arg_slots[0]),
        abi::move_immediate(&flag, "Integer", "0"),
        abi::compare_immediate(&value, "0"),
        abi::branch_ge(&positive),
        abi::move_immediate(&flag, "Integer", "1"),
        // Two's-complement negation read as an unsigned word: exact for every
        // `Integer`, including the minimum, whose absolute value 2^63 has no signed form.
        abi::move_immediate(&zero, "Integer", "0"),
        abi::subtract_registers(&value, &zero, &value),
        abi::label(&positive),
        abi::store_u64(&flag, abi::stack_pointer(), negative),
        abi::load_u64(&cursor, abi::stack_pointer(), result.data),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&write_loop),
        abi::compare_immediate(&index, "8"),
        abi::branch_eq(&write_done),
        abi::store_u8(&value, &cursor, 0),
        abi::shift_right_immediate(&value, &value, 8),
        abi::add_immediate(&cursor, &cursor, 1),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&write_loop),
        abi::label(&write_done),
    ]);
    emit_build_int(builder, &mut vregs, &result, count, negative, "r");
    builder.instructions.extend([
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
        text: "big.fromInteger".to_string(),
    })
}
