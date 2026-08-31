//! `canvas::newSurface` — an opaque-black RGBA8 frame buffer, in one allocation.
//!
//! Internal-only. It replaces an MFBASIC loop that built the buffer with
//! `collections::append`, which for a 900x640 surface is **2.3 million appends per
//! frame** — measured at ~116 ms, i.e. the renderer spent longer clearing the buffer
//! than drawing into it.
//!
//! There is no bulk-fill in `collections::`, so this cannot be expressed in MFBASIC
//! source at all: `append` is the only way to grow a list there. That is exactly the
//! case an `abi_function` exists for — one arena allocation and one fill loop over
//! bytes rather than a call per element.
//!
//! Opaque black rather than transparent: the canvas is a window's whole content, so
//! there is nothing behind it to show through, and a transparent clear would make
//! every unpainted pixel depend on whatever the compositor put there.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::emit_alloc_byte_list;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `canvas::newSurface(width, height) AS List OF Byte`.
pub(crate) fn lower_new_surface(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let width = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the width argument"))?
        .location
        .clone();
    let height = args
        .get(1)
        .ok_or_else(|| format!("'{symbol}' expects the height argument"))?
        .location
        .clone();

    let count_slot = builder.allocate_stack_object("canvas_surface_count", 8);
    let list_slot = builder.allocate_stack_object("canvas_surface_list", 8);
    let fail = builder.label("canvas_surface_alloc_fail");
    let done = builder.label("canvas_surface_done");

    // count = width * height * 4
    let count = builder.temporary_vreg();
    let four = builder.temporary_vreg();
    builder.emit(abi::multiply_registers(&count, &width, &height));
    builder.emit(abi::move_immediate(&four, "Integer", "4"));
    builder.emit(abi::multiply_registers(&count, &count, &four));
    builder.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

    emit_alloc_byte_list(
        &symbol,
        "canvas_surface",
        count_slot,
        list_slot,
        &fail,
        &mut builder.instructions,
        &mut builder.relocations,
    );

    // Fill: R=G=B=0, A=255. Writing all four bytes rather than zeroing and then
    // stamping alpha keeps it one pass, and arena memory is not zero-filled anyway.
    let fill_loop = builder.label("canvas_surface_fill");
    let fill_done = builder.label("canvas_surface_fill_done");
    let block = builder.temporary_vreg();
    let cursor = builder.temporary_vreg();
    let end = builder.temporary_vreg();
    let zero = builder.temporary_vreg();
    let opaque = builder.temporary_vreg();
    builder.emit(abi::load_u64(&block, abi::stack_pointer(), list_slot));
    builder.emit(abi::add_immediate(&cursor, &block, COLLECTION_HEADER_SIZE));
    builder.emit(abi::load_u64(&end, abi::stack_pointer(), count_slot));
    builder.emit(abi::add_registers(&end, &cursor, &end));
    builder.emit(abi::move_immediate(&zero, "Integer", "0"));
    builder.emit(abi::move_immediate(&opaque, "Integer", "255"));
    // Enter only if there is anything to fill: a zero-sized surface must not run the
    // body once. There is no unsigned "not lower" branch, so the guard is spelled as
    // its positive form with a jump past the loop.
    builder.emit(abi::compare_registers(&cursor, &end));
    builder.emit(abi::branch_lo(&fill_loop));
    builder.emit(abi::branch(&fill_done));
    builder.emit(abi::label(&fill_loop));
    builder.emit(abi::store_u8(&zero, &cursor, 0));
    builder.emit(abi::store_u8(&zero, &cursor, 1));
    builder.emit(abi::store_u8(&zero, &cursor, 2));
    builder.emit(abi::store_u8(&opaque, &cursor, 3));
    builder.emit(abi::add_immediate(&cursor, &cursor, 4));
    builder.emit(abi::compare_registers(&cursor, &end));
    builder.emit(abi::branch_lo(&fill_loop));
    builder.emit(abi::label(&fill_done));

    let result = builder.temporary_vreg();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), list_slot));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&fail));
    builder.emit(abi::label(&done));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: symbol,
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "newSurface",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "width",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "height",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_new_surface),
        }],
    });
}
