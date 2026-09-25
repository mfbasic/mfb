//! `canvas::carriedHashes` — the installed scene's hashes, carried forward for every
//! incoming item whose bytes did not change (bug-686 Phase 2).
//!
//! Internal-only. `canvas::present` used to hash every item of every scene it was handed
//! (`__canvas_hashScene`), in MFBASIC, whether or not the item had changed since the last
//! present — ~2 µs a line, which made a static scene of a few thousand items cost the
//! worker more than the whole frame costs the renderer. This walks the incoming list
//! against the scene currently installed, index by index, and answers:
//!
//! * the installed item's hash, when the incoming item at the same index has the same
//!   payload bytes (a byte-identical item is the identical value, so its hash is too);
//! * `-1` otherwise, meaning "hash this one". A real hash is two non-negative 31-bit lanes
//!   (`__canvas_hashStep`), so `-1` is never one.
//!
//! **Two kinds are never carried: `Text` and `Picture`.** Their hash folds in a resource's
//! backend id (`canvas::fontHandle` / `canvas::imageHandle`), which answers `0` once the
//! resource is closed — so an item whose bytes are unchanged can still need a new hash
//! (a text whose font was destroyed must stop drawing). Everything else in a `DrawItem` is
//! plain data inside its own bytes.
//!
//! A byte comparison can only ever say "different" too often, never too rarely: two equal
//! values whose padding differs just get hashed, which is the old cost and the right
//! answer. It must run BEFORE `canvas::publishScene`, which replaces the installed items;
//! the installed hashes are only replaced afterwards, so a carry taken after the publish
//! would pair the new items with the old hashes.

use super::scene_base::scene_base;
use crate::codegen::collection::layout::list_entry_stride;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// The union tag of `DrawItem` variant `name`, as the type model numbers it.
fn variant_tag(builder: &CodeBuilder, name: &str) -> Result<usize, String> {
    for spelling in [name.to_string(), format!("canvas.{name}")] {
        if let Some(tag) = builder
            .type_model
            .union_variant_tags
            .get(&ParameterType::named(&spelling))
        {
            return Ok(*tag);
        }
    }
    Err(format!(
        "canvas::carriedHashes: the DrawItem variant '{name}' has no union tag"
    ))
}

pub(crate) fn lower_carried_hashes(
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
    let text_tag = variant_tag(builder, "Text")?;
    let picture_tag = variant_tag(builder, "Picture")?;

    let items_slot = builder.allocate_stack_object("canvas_carry_items", 8);
    builder.emit(abi::store_u64(&incoming, abi::stack_pointer(), items_slot));
    let n_slot = builder.allocate_stack_object("canvas_carry_n", 8);
    let count = builder.temporary_vreg();
    let items = builder.temporary_vreg();
    builder.emit(abi::load_u64(&items, abi::stack_pointer(), items_slot));
    builder.emit(abi::load_u64(&count, &items, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::store_u64(&count, abi::stack_pointer(), n_slot));

    // The result, one word per incoming item. The only call this function makes, so
    // everything after it lives in registers.
    let out = builder.reserve_integer_index_list(n_slot)?;
    let out_slot = builder.allocate_stack_object("canvas_carry_out", 8);
    builder.emit(abi::store_u64(
        &out.location,
        abi::stack_pointer(),
        out_slot,
    ));

    // m = how many leading indices can be compared: 0 when nothing is installed.
    let scene = scene_base(builder);
    let old = builder.temporary_vreg();
    let old_hashes = builder.temporary_vreg();
    let m = builder.temporary_vreg();
    let n = builder.temporary_vreg();
    let scratch = builder.temporary_vreg();
    let no_carry = builder.label("canvas_carry_none");
    let have_m = builder.label("canvas_carry_have_m");
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    builder.emit(abi::move_immediate(&m, "Integer", "0"));
    builder.emit(abi::load_u64(&old, &scene, CANVAS_SCENE_ITEMS_OFFSET));
    builder.emit(abi::load_u64(
        &old_hashes,
        &scene,
        CANVAS_SCENE_HASHES_OFFSET,
    ));
    builder.emit(abi::compare_immediate(&old, "0"));
    builder.emit(abi::branch_eq(&no_carry));
    builder.emit(abi::compare_immediate(&old_hashes, "0"));
    builder.emit(abi::branch_eq(&no_carry));
    builder.emit(abi::load_u64(&m, &old, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::load_u64(
        &scratch,
        &old_hashes,
        COLLECTION_OFFSET_COUNT,
    ));
    // m = min(old.count, oldHashes.count, n)
    let m_ok1 = builder.label("canvas_carry_m1");
    builder.emit(abi::compare_registers(&scratch, &m));
    builder.emit(abi::branch_ge(&m_ok1));
    builder.emit(abi::move_register(&m, &scratch));
    builder.emit(abi::label(&m_ok1));
    builder.emit(abi::compare_registers(&n, &m));
    builder.emit(abi::branch_ge(&have_m));
    builder.emit(abi::move_register(&m, &n));
    builder.emit(abi::branch(&have_m));
    builder.emit(abi::label(&no_carry));
    builder.emit(abi::move_immediate(&m, "Integer", "0"));
    builder.emit(abi::label(&have_m));

    // Bases. A variable-width list's data region starts past `capacity` lookup
    // entries, never `count` (`.ai/collections.md`): the incoming list is the caller's
    // and may carry headroom. The installed hash list is fixed-width, so its words
    // start right after the header; so does the result's.
    let stride = list_entry_stride(&ParameterType::named("DrawItem"));
    let new_entries = builder.temporary_vreg();
    let new_data = builder.temporary_vreg();
    let old_entries = builder.temporary_vreg();
    let old_data = builder.temporary_vreg();
    let hash_words = builder.temporary_vreg();
    let out_words = builder.temporary_vreg();
    let stride_v = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &stride_v,
        "Integer",
        &stride.to_string(),
    ));
    builder.emit(abi::load_u64(&items, abi::stack_pointer(), items_slot));
    builder.emit(abi::add_immediate(
        &new_entries,
        &items,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::load_u64(&scratch, &items, COLLECTION_OFFSET_CAPACITY));
    builder.emit(abi::multiply_registers(&scratch, &scratch, &stride_v));
    builder.emit(abi::add_registers(&new_data, &new_entries, &scratch));
    let skip_old = builder.label("canvas_carry_skip_old");
    builder.emit(abi::compare_immediate(&m, "0"));
    builder.emit(abi::branch_eq(&skip_old));
    builder.emit(abi::add_immediate(
        &old_entries,
        &old,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::load_u64(&scratch, &old, COLLECTION_OFFSET_CAPACITY));
    builder.emit(abi::multiply_registers(&scratch, &scratch, &stride_v));
    builder.emit(abi::add_registers(&old_data, &old_entries, &scratch));
    builder.emit(abi::add_immediate(
        &hash_words,
        &old_hashes,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::label(&skip_old));
    builder.emit(abi::load_u64(&out_words, abi::stack_pointer(), out_slot));
    builder.emit(abi::add_immediate(
        &out_words,
        &out_words,
        COLLECTION_HEADER_SIZE,
    ));

    // for i in 0..n: out[i] = carried hash, or -1.
    let i = builder.temporary_vreg();
    let result = builder.temporary_vreg();
    let ne = builder.temporary_vreg();
    let oe = builder.temporary_vreg();
    let nlen = builder.temporary_vreg();
    let olen = builder.temporary_vreg();
    let np = builder.temporary_vreg();
    let op = builder.temporary_vreg();
    let lw = builder.temporary_vreg();
    let rw = builder.temporary_vreg();
    let head = builder.label("canvas_carry_head");
    let done = builder.label("canvas_carry_done");
    let store = builder.label("canvas_carry_store");
    let words = builder.label("canvas_carry_words");
    let bytes = builder.label("canvas_carry_bytes");
    let same = builder.label("canvas_carry_same");
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, &n));
    builder.emit(abi::branch_ge(&done));
    // -1, built as 0 - 1: the immediate encoder takes no negative literal.
    builder.emit(abi::move_immediate(&result, "Integer", "0"));
    builder.emit(abi::subtract_immediate(&result, &result, 1));
    builder.emit(abi::compare_registers(&i, &m));
    builder.emit(abi::branch_ge(&store));
    // The two entries: same payload length, or not the same value.
    builder.emit(abi::multiply_registers(&scratch, &i, &stride_v));
    builder.emit(abi::add_registers(&ne, &new_entries, &scratch));
    builder.emit(abi::add_registers(&oe, &old_entries, &scratch));
    builder.emit(abi::load_u64(
        &nlen,
        &ne,
        COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
    ));
    builder.emit(abi::load_u64(
        &olen,
        &oe,
        COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
    ));
    builder.emit(abi::compare_registers(&nlen, &olen));
    builder.emit(abi::branch_ne(&store));
    builder.emit(abi::load_u64(
        &np,
        &ne,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    ));
    builder.emit(abi::add_registers(&np, &new_data, &np));
    builder.emit(abi::load_u64(
        &op,
        &oe,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    ));
    builder.emit(abi::add_registers(&op, &old_data, &op));
    // A data union's tag is its first word; Text and Picture are never carried.
    builder.emit(abi::load_u64(&lw, &np, 0));
    builder.emit(abi::compare_immediate(&lw, &text_tag.to_string()));
    builder.emit(abi::branch_eq(&store));
    builder.emit(abi::compare_immediate(&lw, &picture_tag.to_string()));
    builder.emit(abi::branch_eq(&store));
    // Word compare while at least eight bytes remain, then bytes. `np`/`op`/`nlen`
    // are this iteration's own and are consumed here.
    builder.emit(abi::label(&words));
    builder.emit(abi::compare_immediate(&nlen, "8"));
    builder.emit(abi::branch_lt(&bytes));
    builder.emit(abi::load_u64(&lw, &np, 0));
    builder.emit(abi::load_u64(&rw, &op, 0));
    builder.emit(abi::compare_registers(&lw, &rw));
    builder.emit(abi::branch_ne(&store));
    builder.emit(abi::add_immediate(&np, &np, 8));
    builder.emit(abi::add_immediate(&op, &op, 8));
    builder.emit(abi::subtract_immediate(&nlen, &nlen, 8));
    builder.emit(abi::branch(&words));
    builder.emit(abi::label(&bytes));
    builder.emit_compare_bytes_branch(&np, &op, &nlen, &same, &store, "canvas_carry_tail");
    builder.emit(abi::label(&same));
    builder.emit(abi::shift_left_immediate(&scratch, &i, 3));
    builder.emit(abi::add_registers(&scratch, &hash_words, &scratch));
    builder.emit(abi::load_u64(&result, &scratch, 0));
    builder.emit(abi::label(&store));
    builder.emit(abi::shift_left_immediate(&scratch, &i, 3));
    builder.emit(abi::add_registers(&scratch, &out_words, &scratch));
    builder.emit(abi::store_u64(&result, &scratch, 0));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));

    let reg = builder.allocate_register();
    builder.emit(abi::load_u64(&reg, abi::stack_pointer(), out_slot));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &reg));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "canvas.carriedHashes".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "carriedHashes",
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
            return_type: ParameterType::list_of(ParameterType::Integer),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_carried_hashes),
        }],
    });
}
