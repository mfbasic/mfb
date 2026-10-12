//! `canvas::itemHash` and `canvas::sceneHashes` — a `DrawItem`'s content hash, computed
//! natively (bug-686).
//!
//! Internal-only. The item hash is the geometry cache's key and the damage diff's "did
//! this item change"; `canvas::present` computes one for every item whose bytes changed
//! since the installed scene. In MFBASIC (`__canvas_hashItem`, two 31-bit lanes folded
//! a field at a time through `__canvas_hashStep`) that was ~1.3 µs an item — the worker's
//! whole budget for a moving scene of a few thousand items.
//!
//! **Structural, not a byte hash.** Two identically built items are not byte-identical:
//! a list's headroom (`capacity` past `count`) and its payload padding differ with how it
//! was built, so a hash over the block would miss the cache on every rebuilt scene. The
//! walk folds what the item MEANS and nothing else: the variant tag, every scalar field,
//! every nested record's fields in slot order, and a list's COUNT and then its elements
//! (a nested list's the same way) —
//! never its capacity, headroom or padding. A `Float` folds as its exact 64-bit pattern.
//! A `Picture`'s image folds as its backend id, read through the resource with the
//! closed flag first (`canvas::imageHandle`'s rule, `func_handle_bridge.rs`): `0` once
//! the image is closed, so closing it changes the key and the item's geometry is rebuilt
//! as the nothing it now draws.
//!
//! The walk is generated from the type model's record layouts, so it cannot miss a
//! field a variant gains; a field of a type it does not know how to fold fails the BUILD.
//!
//! **A `Picture`'s `shape` is a union, and it is folded by going round again**
//! (plan-172-L). The picture's other fields are folded first; then the shape's tag is
//! folded and the walk branches back to the variant dispatch with the shape's record in
//! place of the item's, which folds that record and finishes. So a picture on a
//! rectangle — a tilemap is thousands of them — is hashed here, in one pass and with no
//! second copy of any variant's fold; and a picture on a shape this walk does not hash
//! (a `Text`, an `AutoPaint`) falls out of the dispatch as `-1`, like that shape would,
//! and takes `__canvas_hashItem`'s MFBASIC arm.
//!
//! **`Text` and `Group` answer `-1`** — "not hashed here" — and keep `__canvas_hashItem`'s
//! MFBASIC arms: their `String`s fold a codepoint at a time and a `Text` also folds its
//! font. A real hash is 62 bits and non-negative (the final mix shifts two bits out), so
//! `-1` is never one; and the two hash spaces never meet, because a kind is hashed by
//! exactly one of them.
//!
//! The mix is a multiply-xorshift per word (`h = (h ^ v) * M1; h ^= h >> 29`) and a
//! finalizer, all within one 64-bit register: nothing here can overflow-trap.

use super::func_geo_build::{
    emit_field_block, emit_list_element, emit_list_view, record_fields, record_type, variant,
};
use super::gen_image::emit_closed_guard;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// The per-word multiplier and the finalizer's, both odd and below 2^63 (the immediate
/// encoder takes no negative literal).
const MIX: u64 = 0x5851_F42D_4C95_7F2D;
const FINAL: u64 = 0x2545_F491_4F6C_DD1D;
/// The seed every item hash starts from.
const SEED: u64 = 0x1B87_3593_CC9E_2D51;

/// The variants this walk hashes. `Text` and `Group` are the MFBASIC path's, and
/// `AutoPaint` (plan-172-K), whose `shape` is a union field the walk does not fold; a
/// variant in neither list fails the build rather than falling to a default nobody
/// chose.
const HASHED: [&str; 10] = [
    "Rectangle",
    "Circle",
    "Line",
    "Arc",
    "Polygon",
    "Picture",
    "Ellipse",
    "PolyLine",
    "PolyCurve",
    "CurvePolygon",
];
const NOT_HASHED: [&str; 3] = ["Text", "Group", "AutoPaint"];

struct Mixer {
    hash: VirtualRegister,
    mix: VirtualRegister,
}

/// `h = (h ^ value) * MIX; h ^= h >> 29`.
fn fold(builder: &mut CodeBuilder, m: &Mixer, value: &VirtualRegister) {
    let t = builder.temporary_vreg();
    builder.emit(abi::exclusive_or_registers(&m.hash, &m.hash, value));
    builder.emit(abi::multiply_registers(&m.hash, &m.hash, &m.mix));
    builder.emit(abi::shift_right_immediate(&t, &m.hash, 29));
    builder.emit(abi::exclusive_or_registers(&m.hash, &m.hash, &t));
}

/// The one union a hashed record may hold: a `Picture`'s `shape`.
fn is_shape_union(ty: &ParameterType) -> bool {
    ty.is_named("AutoShape") || ty.is_named("canvas.AutoShape")
}

/// Fold every field of the record at `base`, in slot order.
fn fold_record(
    builder: &mut CodeBuilder,
    m: &Mixer,
    base: &VirtualRegister,
    record: &ParameterType,
) -> Result<(), String> {
    for (index, (name, ty)) in record_fields(builder, record)?.into_iter().enumerate() {
        fold_field(builder, m, base, record, index, &name, &ty)?;
    }
    Ok(())
}

fn fold_field(
    builder: &mut CodeBuilder,
    m: &Mixer,
    base: &VirtualRegister,
    record: &ParameterType,
    index: usize,
    name: &str,
    ty: &ParameterType,
) -> Result<(), String> {
    let value = builder.temporary_vreg();
    match ty {
        ParameterType::Float
        | ParameterType::Integer
        | ParameterType::Fixed
        | ParameterType::Money => {
            builder.emit(abi::load_u64(&value, base, 8 * index));
            fold(builder, m, &value);
        }
        ParameterType::Byte | ParameterType::Boolean => {
            builder.emit(abi::load_u8(&value, base, 8 * index));
            fold(builder, m, &value);
        }
        ParameterType::Res(_) => {
            fold_resource_handle(builder, &value, base, 8 * index);
            fold(builder, m, &value);
        }
        // plan-172-C: every drawn item has an `id`. A `String` is a block of its byte
        // length at +0 and its bytes from +8; a record holds it inlined (its slot is a
        // block-relative offset) or by pointer. Folded as the length, then each byte —
        // so `"ab"` + `"c"` in two fields cannot hash as `"a"` + `"bc"`.
        ParameterType::String => {
            let text = builder.temporary_vreg();
            builder.emit(abi::load_u64(&text, base, 8 * index));
            if builder.record_field_is_inlined(ty) {
                builder.emit(abi::add_registers(&text, base, &text));
            }
            let length = builder.temporary_vreg();
            builder.emit(abi::load_u64(&length, &text, 0));
            fold(builder, m, &length);
            let i = builder.temporary_vreg();
            let at = builder.temporary_vreg();
            let head = builder.label("canvas_hash_text");
            let done = builder.label("canvas_hash_text_done");
            builder.emit(abi::move_immediate(&i, "Integer", "0"));
            builder.emit(abi::label(&head));
            builder.emit(abi::compare_registers(&i, &length));
            builder.emit(abi::branch_ge(&done));
            builder.emit(abi::add_registers(&at, &text, &i));
            builder.emit(abi::load_u8(&value, &at, 8));
            fold(builder, m, &value);
            builder.emit(abi::add_immediate(&i, &i, 1));
            builder.emit(abi::branch(&head));
            builder.emit(abi::label(&done));
        }
        ParameterType::ListOf(_) => {
            let list = builder.temporary_vreg();
            let list_type = emit_field_block(builder, &list, base, record, name)?;
            fold_list(builder, m, &list, &list_type)?;
        }
        other if builder.is_enum_type(other) => {
            builder.emit(abi::load_u64(&value, base, 8 * index));
            fold(builder, m, &value);
        }
        other if record_type(builder, other).is_ok() => {
            let sub = builder.temporary_vreg();
            let sub_type = emit_field_block(builder, &sub, base, record, name)?;
            fold_record(builder, m, &sub, &sub_type)?;
        }
        other => {
            return Err(format!(
                "canvas::itemHash: '{record}.{name}' has type '{other}', which the \
                 structural hash does not fold"
            ))
        }
    }
    Ok(())
}

/// `dst` = the backend id behind the resource handle in the slot at `base + offset`, or
/// `0` when there is no resource or it is closed — the closed flag read FIRST.
fn fold_resource_handle(
    builder: &mut CodeBuilder,
    dst: &VirtualRegister,
    base: &VirtualRegister,
    offset: usize,
) {
    let record = builder.temporary_vreg();
    let closed = builder.label("canvas_hash_res_closed");
    let done = builder.label("canvas_hash_res_done");
    builder.emit(abi::load_u64(&record, base, offset));
    builder.emit(abi::compare_immediate(&record, "0"));
    builder.emit(abi::branch_eq(&closed));
    emit_closed_guard(builder, &record, &closed);
    builder.emit(abi::load_u64(dst, &record, RESOURCE_OFFSET_HANDLE));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&closed));
    builder.emit(abi::move_immediate(dst, "Integer", "0"));
    builder.emit(abi::label(&done));
}

/// Fold a list's count, then each element in order.
fn fold_list(
    builder: &mut CodeBuilder,
    m: &Mixer,
    list: &VirtualRegister,
    list_type: &ParameterType,
) -> Result<(), String> {
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, list, COLLECTION_OFFSET_COUNT));
    fold(builder, m, &count);
    let view = emit_list_view(builder, list, list_type)?;
    // plan-163-A: a list element may itself be a list (`Polygon.contours`), folded the
    // same way — its count, then its elements.
    let element_is_list = matches!(view.element, ParameterType::ListOf(_));
    let element_record = match view.fixed_width {
        Some(_) => None,
        None if element_is_list => None,
        None => Some(record_type(builder, &view.element).map_err(|_| {
            format!(
                "canvas::itemHash: a list of '{}' is neither fixed-width, a list nor a \
                 record",
                view.element
            )
        })?),
    };
    let i = builder.temporary_vreg();
    let at = builder.temporary_vreg();
    let head = builder.label("canvas_hash_list");
    let done = builder.label("canvas_hash_list_done");
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, &count));
    builder.emit(abi::branch_ge(&done));
    emit_list_element(builder, &at, &view, &i);
    match (view.fixed_width, &element_record) {
        (Some(width), _) => {
            let value = builder.temporary_vreg();
            builder.emit(match width {
                1 => abi::load_u8(&value, &at, 0),
                4 => abi::load_u32(&value, &at, 0),
                _ => abi::load_u64(&value, &at, 0),
            });
            fold(builder, m, &value);
        }
        (None, Some(record)) => fold_record(builder, m, &at, record)?,
        (None, None) => {
            let element = view.element.clone();
            fold_list(builder, m, &at, &element)?
        }
    }
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));
    Ok(())
}

/// Emit `out` = the item hash of the `DrawItem` data union at `item`, or `-1` for a
/// `Text`, a `Group`, or a tag the union does not have.
pub(super) fn emit_item_hash(
    builder: &mut CodeBuilder,
    item: &VirtualRegister,
    out: &VirtualRegister,
) -> Result<(), String> {
    for name in NOT_HASHED {
        variant(builder, name)?;
    }
    let hashed = HASHED
        .iter()
        .map(|name| variant(builder, name))
        .collect::<Result<Vec<_>, String>>()?;
    let tag = builder.temporary_vreg();
    let rec = builder.temporary_vreg();
    let m = Mixer {
        hash: builder.temporary_vreg(),
        mix: builder.temporary_vreg(),
    };
    builder.emit(abi::load_u64(&tag, item, 0));
    builder.emit(abi::add_immediate(&rec, item, 16));
    builder.emit(abi::move_immediate(&m.mix, "Integer", &MIX.to_string()));
    builder.emit(abi::move_immediate(&m.hash, "Integer", &SEED.to_string()));
    fold(builder, &m, &tag);

    let labels: Vec<String> = hashed
        .iter()
        .map(|_| builder.label("canvas_hash_variant"))
        .collect();
    let finish = builder.label("canvas_hash_finish");
    let done = builder.label("canvas_hash_done");
    let dispatch = builder.label("canvas_hash_dispatch");
    builder.emit(abi::label(&dispatch));
    for ((variant_tag, _), label) in hashed.iter().zip(&labels) {
        builder.emit(abi::compare_immediate(&tag, &variant_tag.to_string()));
        builder.emit(abi::branch_eq(label));
    }
    // Text, Group, AutoPaint -- as the item, or as a picture's shape: the MFBASIC
    // path's. -1, built as 0 - 1.
    builder.emit(abi::move_immediate(out, "Integer", "0"));
    builder.emit(abi::subtract_immediate(out, out, 1));
    builder.emit(abi::branch(&done));
    for ((_, record), label) in hashed.iter().zip(&labels) {
        builder.emit(abi::label(label));
        // A union field is folded LAST, by going round the dispatch again with its
        // record (see the module note); every other field in slot order.
        let mut shape = None;
        for (index, (name, ty)) in record_fields(builder, record)?.into_iter().enumerate() {
            if is_shape_union(&ty) {
                if shape.replace((index, ty)).is_some() {
                    return Err(format!(
                        "canvas::itemHash: '{record}' holds two shape unions; the walk \
                         goes round once"
                    ));
                }
                continue;
            }
            fold_field(builder, &m, &rec, record, index, &name, &ty)?;
        }
        let Some((index, ty)) = shape else {
            builder.emit(abi::branch(&finish));
            continue;
        };
        let sub = builder.temporary_vreg();
        builder.emit(abi::load_u64(&sub, &rec, 8 * index));
        if builder.record_field_is_inlined(&ty) {
            builder.emit(abi::add_registers(&sub, &rec, &sub));
        } else if !builder.record_field_is_pointer(&ty) {
            return Err(format!(
                "canvas::itemHash: the union '{ty}' in '{record}' is neither inlined nor \
                 a pointer"
            ));
        }
        // A data union is its tag at +0 and the variant's record at +16.
        builder.emit(abi::load_u64(&tag, &sub, 0));
        builder.emit(abi::add_immediate(&rec, &sub, 16));
        fold(builder, &m, &tag);
        builder.emit(abi::branch(&dispatch));
    }
    builder.emit(abi::label(&finish));
    let t = builder.temporary_vreg();
    let fin = builder.temporary_vreg();
    builder.emit(abi::shift_right_immediate(&t, &m.hash, 32));
    builder.emit(abi::exclusive_or_registers(&m.hash, &m.hash, &t));
    builder.emit(abi::move_immediate(&fin, "Integer", &FINAL.to_string()));
    builder.emit(abi::multiply_registers(&m.hash, &m.hash, &fin));
    builder.emit(abi::shift_right_immediate(&t, &m.hash, 29));
    builder.emit(abi::exclusive_or_registers(&m.hash, &m.hash, &t));
    // 62 bits, non-negative.
    builder.emit(abi::shift_right_immediate(out, &m.hash, 2));
    builder.emit(abi::label(&done));
    Ok(())
}

fn finish_integer(builder: &mut CodeBuilder, value: &VirtualRegister, text: &str) -> ValueResult {
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, value));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: text.to_string(),
    }
}

pub(crate) fn lower_item_hash(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let incoming = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the DrawItem argument"))?
        .location
        .clone();
    let item = builder.temporary_vreg();
    builder.emit(abi::move_register(&item, &incoming));
    let out = builder.temporary_vreg();
    emit_item_hash(builder, &item, &out)?;
    Ok(finish_integer(builder, &out, "canvas.itemHash"))
}

/// `canvas::sceneHashes(items, carried)`: one word per item — `carried[i]` where it is a
/// hash (`canvas::carriedHashes` found the item's bytes unchanged), else the item's
/// `canvas::itemHash`, which is `-1` for a `Text` or a `Group`. `__canvas_hashScene`
/// hashes only those `-1`s in MFBASIC, so a scene of the common kinds never extracts an
/// item from the list at all. The one allocation is the result.
pub(crate) fn lower_scene_hashes(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if args.len() < 2 {
        return Err(format!(
            "'{symbol}' expects the scene and the carried hashes"
        ));
    }
    let items_slot = builder.allocate_stack_object("canvas_scene_hash_items", 8);
    let carried_slot = builder.allocate_stack_object("canvas_scene_hash_carried", 8);
    builder.emit(abi::store_u64(
        &args[0].location,
        abi::stack_pointer(),
        items_slot,
    ));
    builder.emit(abi::store_u64(
        &args[1].location,
        abi::stack_pointer(),
        carried_slot,
    ));
    let n_slot = builder.allocate_stack_object("canvas_scene_hash_n", 8);
    let items = builder.temporary_vreg();
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&items, abi::stack_pointer(), items_slot));
    builder.emit(abi::load_u64(&count, &items, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::store_u64(&count, abi::stack_pointer(), n_slot));
    let result = builder.reserve_integer_index_list(n_slot)?;
    let out_slot = builder.allocate_stack_object("canvas_scene_hash_out", 8);
    builder.emit(abi::store_u64(
        &result.location,
        abi::stack_pointer(),
        out_slot,
    ));

    let items = builder.temporary_vreg();
    let carried = builder.temporary_vreg();
    let n = builder.temporary_vreg();
    let m = builder.temporary_vreg();
    let out_words = builder.temporary_vreg();
    builder.emit(abi::load_u64(&items, abi::stack_pointer(), items_slot));
    builder.emit(abi::load_u64(&carried, abi::stack_pointer(), carried_slot));
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    builder.emit(abi::load_u64(&m, &carried, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::load_u64(&out_words, abi::stack_pointer(), out_slot));
    // Both `List OF Integer`s are entry-free: word i at HEADER + 8*i.
    builder.emit(abi::add_immediate(
        &out_words,
        &out_words,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::add_immediate(
        &carried,
        &carried,
        COLLECTION_HEADER_SIZE,
    ));
    let view = emit_list_view(
        builder,
        &items,
        &ParameterType::list_of(ParameterType::named("DrawItem")),
    )?;

    let i = builder.temporary_vreg();
    let word = builder.temporary_vreg();
    let item = builder.temporary_vreg();
    let hash = builder.temporary_vreg();
    let head = builder.label("canvas_scene_hash_head");
    let store = builder.label("canvas_scene_hash_store");
    let compute = builder.label("canvas_scene_hash_compute");
    let done = builder.label("canvas_scene_hash_done");
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, &n));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::compare_registers(&i, &m));
    builder.emit(abi::branch_ge(&compute));
    builder.emit(abi::shift_left_immediate(&word, &i, 3));
    builder.emit(abi::add_registers(&word, &carried, &word));
    builder.emit(abi::load_u64(&hash, &word, 0));
    builder.emit(abi::compare_immediate(&hash, "0"));
    builder.emit(abi::branch_ge(&store));
    builder.emit(abi::label(&compute));
    emit_list_element(builder, &item, &view, &i);
    emit_item_hash(builder, &item, &hash)?;
    builder.emit(abi::label(&store));
    builder.emit(abi::shift_left_immediate(&word, &i, 3));
    builder.emit(abi::add_registers(&word, &out_words, &word));
    builder.emit(abi::store_u64(&hash, &word, 0));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));

    let reg = builder.allocate_register();
    builder.emit(abi::load_u64(&reg, abi::stack_pointer(), out_slot));
    Ok(finish_integer(builder, &reg, "canvas.sceneHashes"))
}

/// The one variant whose `id` this pass cannot read from its first field: an
/// `AutoPaint` reports its wrapped shape's (plan-172-K).
const UNSEEN_DECORATOR: &str = "AutoPaint";

/// `canvas::sceneIdBits(items)`: what `canvas::pollMouse` needs to know about a scene
/// before it builds anything for it (plan-172-I) — bit 0 when some item of the list has a
/// non-empty `id`, a `Group` node's included; bit 1 when the list holds something this
/// pass does not see into: a `Group` node, whose own items are elsewhere, or an
/// `AutoPaint`, whose `id` is its shape's -- as an item, or as the shape of a `Picture`.
/// `helper_hit.rs:__canvas_reachesId` looks.
///
/// A `Picture` (plan-172-L) has two: its own, and its shape's, which `overID` reports
/// behind it. Both are read here -- the shape is one more load away -- so a scene of
/// pictures with no `id` anywhere, a tilemap, still answers 0 and builds nothing.
///
/// One pass, two loads an item, no allocation. It leans on the one thing every
/// `DrawItem` variant but `AutoPaint` has in common: `id` is its record's FIRST field, a
/// `String`. That is checked here against the type model for each of them, so a variant
/// that ever broke it fails the build instead of answering from the wrong slot.
pub(crate) fn lower_scene_id_bits(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let incoming = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the scene list argument"))?
        .location
        .clone();
    for name in HASHED.iter().chain(NOT_HASHED.iter()) {
        // plan-172-K: an `AutoPaint` has no `id` -- it reports its shape's -- and its
        // first field is the shape, a union. This pass does not look inside it: like a
        // group node, it is counted as "holds something this pass does not see".
        if *name == UNSEEN_DECORATOR {
            continue;
        }
        let (_, record) = variant(builder, name)?;
        match record_fields(builder, &record)?.first() {
            Some((field, ParameterType::String)) if field == "id" => {}
            _ => {
                return Err(format!(
                    "canvas::sceneIdBits: '{record}' does not start with `id AS String`"
                ))
            }
        }
    }
    let (group_tag, _) = variant(builder, "Group")?;
    let (picture_tag, picture) = variant(builder, "Picture")?;
    let (auto_tag, _) = variant(builder, UNSEEN_DECORATOR)?;
    let inlined = builder.record_field_is_inlined(&ParameterType::String);
    // A picture's `shape`: a data union, held inlined or by pointer.
    let (shape_index, shape_type) = record_fields(builder, &picture)?
        .into_iter()
        .enumerate()
        .find(|(_, (name, _))| name == "shape")
        .map(|(index, (_, ty))| (index, ty))
        .ok_or_else(|| format!("canvas::sceneIdBits: '{picture}' has no `shape`"))?;
    if !is_shape_union(&shape_type) {
        return Err(format!(
            "canvas::sceneIdBits: '{picture}.shape' is '{shape_type}', not the shape union"
        ));
    }
    let shape_inlined = builder.record_field_is_inlined(&shape_type);
    if !shape_inlined && !builder.record_field_is_pointer(&shape_type) {
        return Err(format!(
            "canvas::sceneIdBits: '{picture}.shape' is neither inlined nor a pointer"
        ));
    }

    let items = builder.temporary_vreg();
    let n = builder.temporary_vreg();
    builder.emit(abi::move_register(&items, &incoming));
    builder.emit(abi::load_u64(&n, &items, COLLECTION_OFFSET_COUNT));
    let view = emit_list_view(
        builder,
        &items,
        &ParameterType::list_of(ParameterType::named("DrawItem")),
    )?;

    let bits = builder.temporary_vreg();
    let one = builder.temporary_vreg();
    let two = builder.temporary_vreg();
    let i = builder.temporary_vreg();
    let item = builder.temporary_vreg();
    let word = builder.temporary_vreg();
    let text = builder.temporary_vreg();
    let head = builder.label("canvas_id_bits_head");
    let leaf = builder.label("canvas_id_bits_leaf");
    let next = builder.label("canvas_id_bits_next");
    let done = builder.label("canvas_id_bits_done");
    builder.emit(abi::move_immediate(&bits, "Integer", "0"));
    builder.emit(abi::move_immediate(&one, "Integer", "1"));
    builder.emit(abi::move_immediate(&two, "Integer", "2"));
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, &n));
    builder.emit(abi::branch_ge(&done));
    emit_list_element(builder, &item, &view, &i);
    // A data union's tag is its first word; the variant's record follows at +16.
    builder.emit(abi::load_u64(&word, &item, 0));
    // An `AutoPaint`: unseen, and its first slot is not an `id` -- skip the read.
    let not_auto = builder.label("canvas_id_bits_not_auto");
    builder.emit(abi::compare_immediate(&word, &auto_tag.to_string()));
    builder.emit(abi::branch_ne(&not_auto));
    builder.emit(abi::or_registers(&bits, &bits, &two));
    builder.emit(abi::branch(&next));
    builder.emit(abi::label(&not_auto));
    let not_group = builder.label("canvas_id_bits_not_group");
    builder.emit(abi::compare_immediate(&word, &group_tag.to_string()));
    builder.emit(abi::branch_ne(&not_group));
    builder.emit(abi::or_registers(&bits, &bits, &two));
    builder.emit(abi::branch(&leaf));
    builder.emit(abi::label(&not_group));
    builder.emit(abi::compare_immediate(&word, &picture_tag.to_string()));
    builder.emit(abi::branch_ne(&leaf));
    // A picture's shape: an `AutoPaint` is unseen, as it is on its own; any other
    // shape's record starts with its `id`, like every item's.
    let shape = builder.temporary_vreg();
    let shape_word = builder.temporary_vreg();
    let shape_text = builder.temporary_vreg();
    let shape_id = builder.label("canvas_id_bits_shape_id");
    builder.emit(abi::add_immediate(&shape, &item, 16));
    builder.emit(abi::load_u64(&shape_word, &shape, 8 * shape_index));
    if shape_inlined {
        builder.emit(abi::add_registers(&shape, &shape, &shape_word));
    } else {
        builder.emit(abi::move_register(&shape, &shape_word));
    }
    builder.emit(abi::load_u64(&shape_word, &shape, 0));
    builder.emit(abi::compare_immediate(&shape_word, &auto_tag.to_string()));
    builder.emit(abi::branch_ne(&shape_id));
    builder.emit(abi::or_registers(&bits, &bits, &two));
    builder.emit(abi::branch(&leaf));
    builder.emit(abi::label(&shape_id));
    builder.emit(abi::add_immediate(&shape, &shape, 16));
    builder.emit(abi::load_u64(&shape_text, &shape, 0));
    if inlined {
        builder.emit(abi::add_registers(&shape_text, &shape, &shape_text));
    }
    builder.emit(abi::load_u64(&shape_word, &shape_text, 0));
    builder.emit(abi::compare_immediate(&shape_word, "0"));
    builder.emit(abi::branch_eq(&leaf));
    builder.emit(abi::or_registers(&bits, &bits, &one));
    builder.emit(abi::label(&leaf));
    // A `String` is its byte length at +0; a record holds it inlined (the slot is a
    // record-relative offset) or by pointer (`fold_field`).
    builder.emit(abi::add_immediate(&item, &item, 16));
    builder.emit(abi::load_u64(&text, &item, 0));
    if inlined {
        builder.emit(abi::add_registers(&text, &item, &text));
    }
    builder.emit(abi::load_u64(&word, &text, 0));
    builder.emit(abi::compare_immediate(&word, "0"));
    builder.emit(abi::branch_eq(&next));
    builder.emit(abi::or_registers(&bits, &bits, &one));
    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));
    Ok(finish_integer(builder, &bits, "canvas.sceneIdBits"))
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let item_param = || Parameter {
        name: "item",
        desc: "",
        aliases: &[],
        ty: ParameterType::named("DrawItem"),
        default: DefaultValue::None,
    };
    pkg.add_function(RegistryFunction {
        name: "itemHash",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![item_param()],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_item_hash),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "sceneHashes",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "items",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::named("DrawItem")),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "carried",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::Integer),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::Integer),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_scene_hashes),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "sceneIdBits",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "items",
                desc: "",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::named("DrawItem")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_scene_id_bits),
        }],
    });
}
