//! `big::toBytes` — the magnitude of a `big::Int` as a byte list, in either order.

use super::gen_big::{emit_load_int, emit_spill_args};
use super::{ENDIAN_TYPE_ID, INT_TYPE_ID};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::emit_fail;
use crate::codegen::memory::marshal::emit_build_byte_list;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Return the bytes of a `big::Int`'s absolute value, in either byte order."#;
const DESC: &str = r#"`big::toBytes` returns the absolute value of `value` as a `List OF Byte`, one
byte per element, with no zero bytes at the most significant end. Zero returns an
empty list. The sign is not included: read it with `big::sign`, or from the
`negative` field.

`endian` chooses the order. `big::Endian.Little` (the default) puts the lowest byte
first, matching `magnitude`; `big::Endian.Big` puts the highest byte first, the order
most wire formats and key encodings use.

The returned list is a new value; changing it does not change `value`. The call never
raises. `big::fromBytes` is the reverse."#;
const EX: &str = r#"Write a value big-endian, as a wire format would carry it:

```
IMPORT big
IMPORT collections
IMPORT io

SUB main()
  LET wire AS List OF Byte = big::toBytes(big::fromInteger(258), big::Endian.Big)
  io::print(toString(collections::get(wire, 0)) & " " & toString(collections::get(wire, 1)))
END SUB
```

Zero has no bytes:

```
IMPORT big
IMPORT io

SUB main()
  io::print(toString(len(big::toBytes(big::fromInteger(0)))))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The value whose absolute value is returned. Its sign is not part of the result.",
                    aliases: &[],
                    ty: ParameterType::named(INT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "endian",
                    desc: "The order of the returned bytes. Optional, defaulting to `big::Endian.Little`.",
                    aliases: &[],
                    ty: ParameterType::named(ENDIAN_TYPE_ID),
                    default: DefaultValue::Fill {
                        type_name: ParameterType::named(ENDIAN_TYPE_ID),
                        expr: "0",
                    },
                },
            ],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: Body::abi_function(lower_to_bytes),
        }],
    });
}

/// `big::toBytes`: the trimmed magnitude copied into an exact `List OF Byte`, then
/// reversed in place for `Big`.
pub(crate) fn lower_to_bytes(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let arg_slots = emit_spill_args(builder, args);
    let operand = emit_load_int(builder, &mut vregs, arg_slots[0], "v");
    let list_slot = builder.allocate_stack_object("big_list", 8);
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");
    let reverse_loop = format!("{symbol}_reverse");
    let reverse_done = format!("{symbol}_reverse_done");

    // `emit_build_byte_list` writes its own fixed scratch registers, so nothing is held
    // in a register across it: the source and count travel in frame slots.
    emit_build_byte_list(
        &symbol,
        &format!("{symbol}_copy"),
        &format!("{symbol}_copy_done"),
        operand.data,
        operand.count,
        Some(list_slot),
        abi::mfb_return(1),
        &alloc_fail,
        &mut builder.instructions,
        &mut builder.relocations,
    );

    let order = vregs.next();
    let low = vregs.next();
    let high = vregs.next();
    let count = vregs.next();
    let a = vregs.next();
    let b = vregs.next();
    builder.instructions.extend([
        abi::load_u64(&order, abi::stack_pointer(), arg_slots[1]),
        abi::compare_immediate(&order, "0"),
        abi::branch_eq(&reverse_done),
        abi::load_u64(&low, abi::stack_pointer(), list_slot),
        abi::add_immediate(&low, &low, COLLECTION_HEADER_SIZE),
        abi::load_u64(&count, abi::stack_pointer(), operand.count),
        abi::add_registers(&high, &low, &count),
        abi::label(&reverse_loop),
        // Swap inward from both ends until they meet: `high` is one past the byte to
        // swap, so the loop stops once `low + 1 >= high`.
        abi::subtract_immediate(&high, &high, 1),
        abi::compare_registers(&low, &high),
        abi::branch_ge(&reverse_done),
        abi::load_u8(&a, &low, 0),
        abi::load_u8(&b, &high, 0),
        abi::store_u8(&b, &low, 0),
        abi::store_u8(&a, &high, 0),
        abi::add_immediate(&low, &low, 1),
        abi::branch(&reverse_loop),
        abi::label(&reverse_done),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), list_slot),
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
        type_: ParameterType::list_of(ParameterType::Byte),
        location: Operand::from("void"),
        text: "big.toBytes".to_string(),
    })
}
