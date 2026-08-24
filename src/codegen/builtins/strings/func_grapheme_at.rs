//! `strings.graphemeAt` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use super::gen_graphemes::lower_strings_graphemes;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.graphemeAt: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
    let index = &args[1];

    let scratch16 = builder.temporary_vreg();
    let scratch10 = builder.temporary_vreg();
    let scratch9 = builder.temporary_vreg();
    let scratch11 = builder.temporary_vreg();
    let scratch12 = builder.temporary_vreg();
    let scratch13 = builder.temporary_vreg();
    let scratch14 = builder.temporary_vreg();
    let scratch15 = builder.temporary_vreg();
    let index = index.clone();
    if index.type_ != ParameterType::Integer {
        return Err(format!(
            "strings.graphemeAt index must be Integer, got {}",
            index.type_
        ));
    }
    let index_slot = builder.spill_to_slot("strings_grapheme_at_index", &index.location);
    let list = lower_strings_graphemes(builder, value)?;
    let list_slot = builder.spill_to_slot("strings_grapheme_at_list", &list.location);
    let ptr_slot = builder.allocate_stack_object("strings_grapheme_at_ptr", 8);
    let len_slot = builder.allocate_stack_object("strings_grapheme_at_len", 8);

    let invalid = builder.label("strings_grapheme_at_invalid");
    let ok = builder.label("strings_grapheme_at_ok");

    builder.emit(abi::load_u64(&scratch16, abi::stack_pointer(), list_slot));
    builder.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
    builder.emit(abi::load_u64(
        &scratch9,
        &scratch16,
        COLLECTION_OFFSET_COUNT,
    ));
    builder.emit(abi::compare_immediate(&scratch10, "0"));
    builder.emit(abi::branch_lt(&invalid));
    builder.emit(abi::compare_registers(&scratch10, &scratch9));
    builder.emit(abi::branch_ge(&invalid));
    // entry = header + index * ENTRY_SIZE.
    builder.emit(abi::move_immediate(
        &scratch11,
        "Integer",
        &COLLECTION_ENTRY_SIZE.to_string(),
    ));
    builder.emit(abi::multiply_registers(&scratch11, &scratch11, &scratch10));
    builder.emit(abi::add_immediate(
        &scratch12,
        &scratch16,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::add_registers(&scratch12, &scratch12, &scratch11));
    // x13 = value offset, x14 = value length.
    builder.emit(abi::load_u64(
        &scratch13,
        &scratch12,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    ));
    builder.emit(abi::load_u64(
        &scratch14,
        &scratch12,
        COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
    ));
    builder.emit_collection_data_pointer_for(&scratch15, &scratch16, "String");
    builder.emit(abi::add_registers(&scratch15, &scratch15, &scratch13));
    builder.emit(abi::store_u64(&scratch15, abi::stack_pointer(), ptr_slot));
    builder.emit(abi::store_u64(&scratch14, abi::stack_pointer(), len_slot));
    builder.emit(abi::branch(&ok));
    builder.emit(abi::label(&invalid));
    builder.raise_error("strings.graphemeAt", "ErrIndexOutOfRange")?;
    builder.emit(abi::label(&ok));
    builder.emit(abi::load_u64(&scratch15, abi::stack_pointer(), ptr_slot));
    builder.emit(abi::load_u64(&scratch14, abi::stack_pointer(), len_slot));
    let result = builder.emit_materialize_string_from_bytes(&scratch15, &scratch14)?;
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::String,
        location: Operand::from(result.render()),
        text: "strings.graphemeAt".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "graphemeAt",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "index",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec!["ErrIndexOutOfRange"],
            body: Body::abi_inline(lower),
        }],
    });
}
