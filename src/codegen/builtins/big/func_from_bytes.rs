//! `big::fromBytes` — a `big::Int` from a magnitude byte list, a sign, and a byte order.

use super::gen_big::{emit_alloc_magnitude, emit_build_int, emit_spill_args};
use super::{ENDIAN_TYPE_ID, INT_TYPE_ID};
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

const INTRO: &str = r#"Build a `big::Int` from the bytes of its absolute value and a sign."#;
const DESC: &str = r#"`big::fromBytes` reads `bytes` as an unsigned number and returns it as a
`big::Int`, negated when `negative` is `TRUE`.

`endian` says which end of `bytes` is least significant. `big::Endian.Little` (the
default) reads `bytes[0]` as the lowest byte, the order `magnitude` uses;
`big::Endian.Big` reads `bytes[0]` as the highest, the order most wire formats and key
encodings use.

Any list is accepted, including an empty one, which is zero. Zero bytes at the
most significant end are allowed and do not change the value. `negative` is ignored
for a zero value, so an empty or all-zero list with `negative` set is plain zero, never a
negative zero.
The call never raises.

`big::toBytes` is the reverse: `big::toBytes(big::fromBytes(b, n, e), e)` returns `b`
whenever `b` has no zero bytes at its most significant end."#;
const EX: &str = r#"Read a big-endian magnitude, as a wire format would carry it:

```
IMPORT big
IMPORT io

SUB main()
  LET wire AS List OF Byte = [1, 0]
  LET x AS big::Int = big::fromBytes(wire, FALSE, big::Endian.Big)
  io::print(toString(big::toInteger(x)))
END SUB
```

Little-endian is the default, and a negative sign on zero is dropped:

```
IMPORT big
IMPORT io

SUB main()
  LET zeros AS List OF Byte = [0, 0]
  LET z AS big::Int = big::fromBytes(zeros, TRUE)
  io::print(toString(z.negative))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fromBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "bytes",
                    desc: "The absolute value, one byte per element, in the order `endian` names. An empty list is zero.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::Byte),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "negative",
                    desc: "`TRUE` for a value below zero. Ignored when the value is zero.",
                    aliases: &[],
                    ty: ParameterType::Boolean,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "endian",
                    desc: "Which end of `bytes` is least significant. Optional, defaulting to `big::Endian.Little`.",
                    aliases: &[],
                    ty: ParameterType::named(ENDIAN_TYPE_ID),
                    default: DefaultValue::Fill {
                        type_name: ParameterType::named(ENDIAN_TYPE_ID),
                        expr: "0",
                    },
                },
            ],
            return_type: ParameterType::named(INT_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_from_bytes),
        }],
    });
}

/// `big::fromBytes`: a result sized to the list, filled in list order (`Little`) or
/// reversed (`Big`), then normalized by `emit_build_int`.
pub(crate) fn lower_from_bytes(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let capacity = builder.allocate_stack_object("big_capacity", 8);
    let negative = builder.allocate_stack_object("big_negative", 8);
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let copy_loop = format!("{symbol}_copy");
    let copy_done = format!("{symbol}_copy_done");
    let little = format!("{symbol}_little");
    let placed = format!("{symbol}_placed");

    let list = vregs.next();
    let count = vregs.next();
    let flag = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&list, abi::stack_pointer(), arg_slots[0]),
        abi::load_u64(&count, &list, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, abi::stack_pointer(), capacity),
        abi::load_u64(&flag, abi::stack_pointer(), arg_slots[1]),
        abi::store_u64(&flag, abi::stack_pointer(), negative),
    ]);
    let result = emit_alloc_magnitude(builder, &mut vregs, capacity, "r", &alloc_fail);

    // Reload everything after the allocation call.
    let src = vregs.next();
    let dst = vregs.next();
    let n = vregs.next();
    let index = vregs.next();
    let order = vregs.next();
    let at = vregs.next();
    let byte = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&src, abi::stack_pointer(), arg_slots[0]),
        abi::add_immediate(&src, &src, COLLECTION_HEADER_SIZE),
        abi::load_u64(&dst, abi::stack_pointer(), result.data),
        abi::load_u64(&n, abi::stack_pointer(), capacity),
        abi::load_u64(&order, abi::stack_pointer(), arg_slots[2]),
        abi::move_immediate(&index, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&index, &n),
        abi::branch_eq(&copy_done),
        abi::compare_immediate(&order, "0"),
        abi::branch_eq(&little),
        // Big: magnitude byte `index` is list element `n - 1 - index`.
        abi::subtract_registers(&at, &n, &index),
        abi::subtract_immediate(&at, &at, 1),
        abi::branch(&placed),
        abi::label(&little),
        abi::move_register(&at, &index),
        abi::label(&placed),
        abi::add_registers(&at, &src, &at),
        abi::load_u8(&byte, &at, 0),
        abi::add_registers(&at, &dst, &index),
        abi::store_u8(&byte, &at, 0),
        abi::add_immediate(&index, &index, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
    ]);
    emit_build_int(builder, &mut vregs, &result, capacity, negative, "r");
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
        text: "big.fromBytes".to_string(),
    })
}
