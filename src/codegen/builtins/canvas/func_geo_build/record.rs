//! plan-176-R: the native geometry builder's record and list views — record types and fields,
//! variants, list views and elements. The slot constants, float helpers and `lower_geo_build`
//! live in `builtins/canvas/func_geo_build/mod.rs`.

use super::*;

// ---- Layout, read from the type model ----------------------------------------------

/// A canvas nominal as the type model keys it. The program's model registers the
/// builtin records package-qualified (`canvas.Paint`, `color.Color`); a helper compiled
/// inside the package may still spell one bare, so both are tried — the
/// package-qualified spelling FIRST. A bare spelling in the program's model is a USER
/// type (`.ai/codegen-invariants.md`, "Inside the compiler a BARE type spelling is
/// always a USER type"): a program's own `TYPE Circle` must not stand in for
/// `canvas.Circle`, whose layout the helper reads.
pub(in crate::codegen::builtins::canvas) fn record_type(
    builder: &CodeBuilder,
    ty: &ParameterType,
) -> Result<ParameterType, String> {
    let name = ty.name().into_owned();
    let bare = name.strip_prefix("canvas.").unwrap_or(&name).to_string();
    for spelling in [format!("canvas.{bare}"), name.clone(), bare] {
        let candidate = ParameterType::named(&spelling);
        if builder.type_model.record_fields.contains_key(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "canvas: the record type '{name}' has no layout in the type model"
    ))
}

/// The fields of a record type, in slot order.
pub(in crate::codegen::builtins::canvas) fn record_fields(
    builder: &CodeBuilder,
    record: &ParameterType,
) -> Result<Vec<(String, ParameterType)>, String> {
    builder
        .type_model
        .record_fields
        .get(record)
        .cloned()
        .ok_or_else(|| format!("canvas: the record type '{record}' has no layout"))
}

/// The slot index and declared type of field `name` of `record`.
pub(super) fn field(
    builder: &CodeBuilder,
    record: &ParameterType,
    name: &str,
) -> Result<(usize, ParameterType), String> {
    record_fields(builder, record)?
        .into_iter()
        .enumerate()
        .find(|(_, (field, _))| field == name)
        .map(|(index, (_, ty))| (index, ty))
        .ok_or_else(|| format!("canvas: the record '{record}' has no field '{name}'"))
}

/// The byte offset of scalar field `name` within its record.
pub(super) fn scalar_offset(
    builder: &CodeBuilder,
    record: &ParameterType,
    name: &str,
) -> Result<usize, String> {
    let (index, ty) = field(builder, record, name)?;
    if builder.record_field_is_inlined(&ty) || builder.record_field_is_pointer(&ty) {
        return Err(format!(
            "canvas: '{record}.{name}' is a composite ({ty}), not a scalar slot"
        ));
    }
    Ok(8 * index)
}

/// `dst` = the address of composite field `name` of the record at `base`: `base +
/// [base + 8*i]` when the model inlines it, `[base + 8*i]` when it is a pointer. Returns
/// the field's record/list type as the model keys it. `dst` must not be `base`.
pub(in crate::codegen::builtins::canvas) fn emit_field_block(
    builder: &mut CodeBuilder,
    dst: &VirtualRegister,
    base: &VirtualRegister,
    record: &ParameterType,
    name: &str,
) -> Result<ParameterType, String> {
    let (index, ty) = field(builder, record, name)?;
    builder.emit(abi::load_u64(dst, base, 8 * index));
    if builder.record_field_is_inlined(&ty) {
        builder.emit(abi::add_registers(dst, base, dst));
    } else if !builder.record_field_is_pointer(&ty) {
        return Err(format!(
            "canvas: '{record}.{name}' is a scalar ({ty}), not a composite"
        ));
    }
    if matches!(ty, ParameterType::ListOf(_)) {
        return Ok(ty);
    }
    record_type(builder, &ty)
}

/// The union tag of `DrawItem` variant `name`, and its record type. Qualified first, as
/// in [`record_type`]: a bare `Circle` in the model is a user union's variant, with that
/// union's tag.
pub(in crate::codegen::builtins::canvas) fn variant(
    builder: &CodeBuilder,
    name: &str,
) -> Result<(usize, ParameterType), String> {
    for spelling in [format!("canvas.{name}"), name.to_string()] {
        if let Some(tag) = builder
            .type_model
            .union_variant_tags
            .get(&ParameterType::named(&spelling))
        {
            let record = record_type(builder, &ParameterType::named(name))?;
            return Ok((*tag, record));
        }
    }
    Err(format!(
        "canvas: the DrawItem variant '{name}' has no union tag"
    ))
}

/// The ordinal of enum member `member` of the enum type `ty` (a field's declared type).
pub(super) fn enum_ordinal(
    builder: &CodeBuilder,
    ty: &ParameterType,
    member: &str,
) -> Result<usize, String> {
    let name = ty.name().into_owned();
    let bare = name.strip_prefix("canvas.").unwrap_or(&name).to_string();
    // Qualified first, as in [`record_type`].
    for spelling in [format!("canvas.{bare}"), name.clone(), bare] {
        if let Some(ordinal) = builder
            .type_model
            .enum_members
            .get(&(ParameterType::named(&spelling), member.to_string()))
        {
            return Ok(*ordinal);
        }
    }
    Err(format!(
        "canvas: the enum '{name}' has no member '{member}'"
    ))
}

/// A list block's element addressing: its entry array, its data base (capacity-based)
/// and whether an element's payload is a pointer to it rather than the element itself.
pub(in crate::codegen::builtins::canvas) struct ListView {
    pub(in crate::codegen::builtins::canvas) entries: VirtualRegister,
    pub(in crate::codegen::builtins::canvas) data: VirtualRegister,
    /// The element type as the list's own type spells it.
    pub(in crate::codegen::builtins::canvas) element: ParameterType,
    /// `Some(width)` for an entry-free fixed-width list: element `i` is at
    /// `data + i * width`.
    pub(in crate::codegen::builtins::canvas) fixed_width: Option<usize>,
    pub(in crate::codegen::builtins::canvas) pointer_payload: bool,
}

/// Compute the addressing for the list block at `list` whose type is `list_type`.
pub(in crate::codegen::builtins::canvas) fn emit_list_view(
    builder: &mut CodeBuilder,
    list: &VirtualRegister,
    list_type: &ParameterType,
) -> Result<ListView, String> {
    let ParameterType::ListOf(element) = list_type else {
        return Err(format!("canvas: '{list_type}' is not a list"));
    };
    let element = (**element).clone();
    let stride = list_entry_stride(&element);
    let entries = builder.temporary_vreg();
    let data = builder.temporary_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&entries, list, COLLECTION_HEADER_SIZE));
    builder.emit(abi::load_u64(&scratch, list, COLLECTION_OFFSET_CAPACITY));
    let stride_v = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &stride_v,
        "Integer",
        &stride.to_string(),
    ));
    builder.emit(abi::multiply_registers(&scratch, &scratch, &stride_v));
    builder.emit(abi::add_registers(&data, &entries, &scratch));
    let fixed_width = list_element_is_fixed_width(&element);
    let pointer_payload = builder.is_pointer_collection_payload_type(&element);
    Ok(ListView {
        entries,
        data,
        element,
        fixed_width,
        pointer_payload,
    })
}

/// `dst` = the address of element `index`'s payload (for a record element, the record's
/// own base). `dst` must not be `index`.
pub(in crate::codegen::builtins::canvas) fn emit_list_element(
    builder: &mut CodeBuilder,
    dst: &VirtualRegister,
    view: &ListView,
    index: &VirtualRegister,
) {
    if let Some(width) = view.fixed_width {
        let w = builder.temporary_vreg();
        builder.emit(abi::move_immediate(&w, "Integer", &width.to_string()));
        builder.emit(abi::multiply_registers(dst, index, &w));
        builder.emit(abi::add_registers(dst, &view.data, dst));
        return;
    }
    let entry_size = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &entry_size,
        "Integer",
        &COLLECTION_ENTRY_SIZE.to_string(),
    ));
    builder.emit(abi::multiply_registers(dst, index, &entry_size));
    builder.emit(abi::add_registers(dst, &view.entries, dst));
    builder.emit(abi::load_u64(
        dst,
        dst,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    ));
    builder.emit(abi::add_registers(dst, &view.data, dst));
    if view.pointer_payload {
        builder.emit(abi::load_u64(dst, dst, 0));
    }
}

/// Reserve a `List OF Float` of exactly the count in `n_slot` (count = capacity), and
/// return the stack slot holding it. The only call either function makes.
pub(super) fn reserve_float_list(
    builder: &mut CodeBuilder,
    n_slot: usize,
) -> Result<usize, String> {
    let layout = CollectionTypeLayout::from_type(&ParameterType::list_of(ParameterType::Float))
        .ok_or_else(|| "canvas: no List OF Float layout".to_string())?;
    let n = builder.temporary_vreg();
    let eight = builder.temporary_vreg();
    let bytes = builder.temporary_vreg();
    let result_slot = builder.allocate_stack_object("canvas_geo_list", 8);
    let overflow = builder.label("canvas_geo_list_overflow");
    let alloc_ok = builder.label("canvas_geo_list_ok");
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    builder.emit(abi::move_immediate(&eight, "Integer", "8"));
    builder.emit_checked_size_multiply(&bytes, &n, &eight, &overflow);
    builder.emit_checked_size_add_immediate(
        abi::return_register(),
        &bytes,
        COLLECTION_HEADER_SIZE,
        &overflow,
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&overflow));
    let (code, message) =
        crate::codegen::registry::runtime_error("ErrOutOfMemory").expect("errorCode name");
    builder.emit_error_code_return(code, message)?;
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    let base = builder.temporary_vreg();
    let nn = builder.temporary_vreg();
    let bb = builder.temporary_vreg();
    builder.emit(abi::load_u64(&base, abi::stack_pointer(), result_slot));
    builder.emit(abi::load_u64(&nn, abi::stack_pointer(), n_slot));
    builder.emit(abi::shift_left_immediate(&bb, &nn, 3));
    builder.emit_write_collection_header_full(&layout, &base, &nn, &nn, &bb, &bb);
    Ok(result_slot)
}
