//! `strings.toBytes` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

const INTRO: &str = r#"Return the raw UTF-8 bytes backing a string, one element per byte."#;

const DESC: &str = r#"`strings::toBytes` returns the UTF-8 octets that back `value` as a
`List OF Byte`, one element per byte, in encoding order. It is the byte-level
view of a string: no decoding, validation, or transformation is performed, and
the bytes are copied verbatim into a freshly built list.

The result length is exactly `strings::byteLen(value)`, which is generally larger
than `len(value)`: an ASCII scalar contributes one element, while a non-ASCII
scalar contributes the two, three, or four bytes of its UTF-8 encoding. For
`"héllo"` the list has six elements, because `é` encodes as the two bytes `195`
and `169`. The empty string yields the empty list.

`toBytes` is the inverse of `toString(List OF Byte)` and the foundation the
`encoding` package's Unicode codecs are built on; `encoding::utf8EncodeBytes`
produces the same octets for the same string.

`value` is not mutated. The returned `List OF Byte` is its own value, so
mutating it does not affect the string it came from.

`value` may also be an `astrings::AttributedString`: the query runs on its visible
text and returns exactly what the `String` overload returns (same value, type, and
errors)."#;

const EX: &str = r#"A non-ASCII scalar contributes more than one byte:

```
IMPORT io
IMPORT strings
IMPORT collections

FUNC main() AS Integer
  LET bytes AS List OF Byte = strings::toBytes("héllo")
  io::print(toString(len(bytes)))
  io::print(toString(collections::get(bytes, 1)))
  RETURN 0
END FUNC
```

Round-trip a string through its bytes:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET bytes AS List OF Byte = strings::toBytes("hi")
  io::print(toString(bytes))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() == 1 {
        if let Some(value) = builder.static_string_value_vr(&args[0]) {
            let values = value
                .as_bytes()
                .iter()
                .map(|byte| NirValue::Const {
                    type_: ParameterType::Byte,
                    value: byte.to_string(),
                })
                .collect::<Vec<_>>();
            return builder
                .lower_list_literal(&ParameterType::list_of(ParameterType::Byte), &values);
        }
    }
    if args.len() != 1 {
        return Err("strings.toBytes: no native lowering for these arguments".to_string());
    }
    let value = &args[0];

    let scratch16 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch20 = builder.temporary_vreg();
    let scratch21 = builder.temporary_vreg();
    let scratch22 = builder.temporary_vreg();
    let scratch23 = builder.temporary_vreg();
    let scratch25 = builder.temporary_vreg();
    let scratch24 = builder.temporary_vreg();
    let scratch26 = builder.temporary_vreg();
    let scratch27 = builder.temporary_vreg();
    let scratch28 = builder.temporary_vreg();
    let value = value.clone();
    builder.require_string("strings.toBytes value", &value)?;
    let value_slot = builder.spill_to_slot("strings_to_bytes_value", &value.location);
    let count_slot = builder.allocate_stack_object("strings_to_bytes_count", 8);
    let result_slot = builder.allocate_stack_object("strings_to_bytes_result", 8);
    let layout = CollectionTypeLayout::from_type(&ParameterType::list_of(ParameterType::Byte))
        .ok_or_else(|| "native strings.toBytes cannot resolve List OF Byte layout".to_string())?;

    let alloc_ok = builder.label("strings_to_bytes_alloc_ok");
    let write_loop = builder.label("strings_to_bytes_write_loop");
    let write_done = builder.label("strings_to_bytes_write_done");

    // count = byteLen( [strptr + 0] ); spill across the allocation call.
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(&scratch9, &scratch16, 0));
    builder.emit(abi::store_u64(&scratch9, abi::stack_pointer(), count_slot));

    // alloc size = HEADER + count * (ENTRY_SIZE) + count (one payload byte each).
    // The size multiply/add is checked (audit-unicode #8): the count is an
    // arena-bounded string length so a wrap is unreachable on real hardware,
    // but every arena-size computation shares the same self-defending shape.
    let size_overflow = builder.label("strings_to_bytes_size_overflow");
    builder.emit(abi::move_immediate(
        &scratch13,
        "Integer",
        &(byte_list_entry_stride() + 1).to_string(),
    ));
    builder.emit_checked_size_multiply(&scratch13, &scratch9, &scratch13, &size_overflow);
    builder.emit_checked_size_add_immediate(
        abi::return_register(),
        &scratch13,
        COLLECTION_HEADER_SIZE,
        &size_overflow,
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    // A size wrap reports the same 77010001 an impossible allocation would;
    // it cannot share the register-based return above (x0 holds the failed
    // call's tag there, not an error code, before the call ever runs).
    builder.emit(abi::label(&size_overflow));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));
    // x1 holds the new collection pointer.
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    builder.emit(abi::move_register(&scratch20, abi::mfb_return(1)));
    builder.emit(abi::load_u64(&scratch9, abi::stack_pointer(), count_slot));
    // Header: count == capacity == dataLength == dataCapacity == count.
    builder.emit_write_list_header_from_registers(&layout, &scratch20, &scratch9, &scratch9);

    // payload base = collection + HEADER + capacity * ENTRY_SIZE.
    builder.emit(abi::move_immediate(
        &scratch13,
        "Integer",
        &byte_list_entry_stride().to_string(),
    ));
    builder.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch13));
    builder.emit(abi::add_immediate(
        &scratch21,
        &scratch20,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::add_registers(&scratch21, &scratch21, &scratch13));

    // x22 = string data pointer (strptr + 8); x23 = i (0).
    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
    builder.emit(abi::add_immediate(&scratch22, &scratch16, 8));
    builder.emit(abi::move_immediate(&scratch23, "Integer", "0"));

    builder.emit(abi::label(&write_loop));
    builder.emit(abi::compare_registers(&scratch23, &scratch9));
    builder.emit(abi::branch_ge(&write_done));
    // entry_addr (x24) = collection + HEADER + i * ENTRY_SIZE.
    builder.emit(abi::move_immediate(
        &scratch25,
        "Integer",
        &byte_list_entry_stride().to_string(),
    ));
    builder.emit(abi::multiply_registers(&scratch25, &scratch23, &scratch25));
    builder.emit(abi::add_immediate(
        &scratch24,
        &scratch20,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::add_registers(&scratch24, &scratch24, &scratch25));
    // flags = USED; key offset/length = 0.
    builder.emit(abi::move_immediate(
        &scratch26,
        "Byte",
        &COLLECTION_ENTRY_FLAG_USED.to_string(),
    ));
    if byte_list_entry_stride() != 0 {
        builder.emit(abi::store_u8(
            &scratch26,
            &scratch24,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
    }
    if byte_list_entry_stride() != 0 {
        builder.emit(abi::store_u64(
            abi::ZERO,
            &scratch24,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
    }
    if byte_list_entry_stride() != 0 {
        builder.emit(abi::store_u64(
            abi::ZERO,
            &scratch24,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
    }
    // value offset = i; value length = 1.
    if byte_list_entry_stride() != 0 {
        builder.emit(abi::store_u64(
            &scratch23,
            &scratch24,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
    }
    builder.emit(abi::move_immediate(&scratch26, "Integer", "1"));
    if byte_list_entry_stride() != 0 {
        builder.emit(abi::store_u64(
            &scratch26,
            &scratch24,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
    }
    // payload[i] = string byte[i].
    builder.emit(abi::add_registers(&scratch27, &scratch22, &scratch23));
    builder.emit(abi::load_u8(&scratch26, &scratch27, 0));
    builder.emit(abi::add_registers(&scratch28, &scratch21, &scratch23));
    builder.emit(abi::store_u8(&scratch26, &scratch28, 0));
    builder.emit(abi::add_immediate(&scratch23, &scratch23, 1));
    builder.emit(abi::branch(&write_loop));
    builder.emit(abi::label(&write_done));

    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::list_of(ParameterType::Byte),
        location: Operand::from(result.render()),
        text: "strings.toBytes".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "toBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc: "The string whose UTF-8 storage is returned. Any `String` is accepted, including the empty string.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: Body::abi_inline(lower),
        }],
    });
}
