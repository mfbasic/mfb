//! The geometry cache's store and the per-frame scene resolve, native (bug-686).
//!
//! Internal-only, graphics thread, and every member is `Body::abi_inline`: it is emitted
//! inside the MFBASIC function that calls it, because only there are the program's
//! globals in scope (a shared `_mfb_rt_*` helper is lowered with none). The cache is
//! still MFBASIC globals — every reader
//! (`__canvas_geoAt`, the software rasteriser, both emitters, the glyph cache) keeps
//! reading `__CANVAS_GEO_DATA` as the `List OF Float` it always was — but the members
//! here read and write those globals **directly through their arena-state slots**
//! rather than through arguments and a return. That is what lets one call per frame
//! append thousands of records into `__CANVAS_GEO_DATA` without the list round-tripping
//! through a value boundary (an argument is borrowed, a return is a fresh owner; neither
//! can be the global's own block). It is sound for the same reason the in-place
//! self-update of a global (`.ai/collections.md` site S2) is: the global is the block's
//! only owner, every MFBASIC read of a global re-loads its slot, and no MFBASIC code is
//! running while a native call is.
//!
//! The store, all fixed-width lists grown geometrically in place:
//!
//! * `__CANVAS_GEO_DATA` — the records, header then tail, back to back.
//! * `__CANVAS_GEO_SLOTS` — four words per slot: `hash, offset, count, lastUsed`.
//!   `lastUsed` is the frame (`__CANVAS_GEO_FRAME`) that last used the slot, or `-1`
//!   for a slot the glyph cache FORGOT (its glyph indices were renumbered under it).
//! * `__CANVAS_GEO_TABLE` — the index, an open-addressing table of `(hash, slot)`
//!   buckets, linear probing, a power-of-two bucket count kept at least twice the slot
//!   count. An empty bucket's key is `-1` (no hash is negative). Forgetting a slot
//!   leaves its key in place with slot `-1`, a tombstone a later insert of the same key
//!   reuses. It replaced a `Map OF Integer TO Integer` whose per-frame rebuild was a
//!   large share of a moving scene's frame.
//!
//! The lifetime rules are `.ai/canvas-threading.md` §14's and are unchanged: nothing is
//! evicted inside a frame (`canvas::geoBeginFrame` is the only place a slot is dropped),
//! glyph eviction pins only this frame's slots, and a `Picture` is never trusted on its
//! hash alone — `canvas::sceneResolve` refers every picture hit back to MFBASIC, which
//! re-reads the image.

use super::func_geo_build::{
    emit_field_block, emit_fill_record, emit_list_element, emit_list_view, emit_record_size,
    emit_refresh_record, record_type,
};
use super::func_item_hash::emit_item_hash;
use super::scene_base::{emit_scene_load, scene_base};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::codegen::runtime::canvas::{
    CANVAS_DRAW_ENTRY_COUNT_SHIFT, CANVAS_DRAW_ENTRY_MODE, CANVAS_DRAW_ENTRY_WORDS,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

mod layout;
mod resolve;
mod snapshot;
mod table;
use layout::{lower_scene_draws_flat, lower_scene_layout};
use resolve::{emit_hash_step_zero, lower_scene_resolve};
use snapshot::{
    lower_scene_snapshot, lower_snapshot_hashes, lower_snapshot_items, lower_snapshot_layers,
};
use table::{
    emit_add_slot, emit_find, emit_refreshed, emit_stamp, finish, lower_geo_begin_frame,
    lower_geo_find, lower_geo_forget, lower_geo_insert, lower_geo_refresh, lower_geo_repaint,
};

/// Words per slot in `__CANVAS_GEO_SLOTS`.
const SLOT_WORDS: usize = 4;
/// The smallest table, in buckets.
const MIN_BUCKETS: usize = 64;
/// `__CANVAS_GEO_PICTURE`, as the bit pattern of the float in a record's slot 0.
const PICTURE_KIND_BITS: u64 = 0x4022_0000_0000_0000; // 9.0
/// `__CANVAS_GEO_TILE` the same way (plan-173). A Tile samples an image exactly as a
/// picture does, so its cached record names a pixel block that `setBytes` replaces
/// without changing the item.
const TILE_KIND_BITS: u64 = 0x402A_0000_0000_0000; // 13.0

fn int_list() -> ParameterType {
    ParameterType::list_of(ParameterType::Integer)
}

fn float_list() -> ParameterType {
    ParameterType::list_of(ParameterType::Float)
}

/// The arena-state offsets of the cache's globals.
struct Globals {
    data: usize,
    slots: usize,
    table: usize,
    frame: usize,
    used: usize,
    last: usize,
    generations: usize,
    compactions: usize,
    native_builds: usize,
    built: usize,
    /// plan-175-A: `__CANVAS_GEO_DRAWS`, the damage identity `D` each slot's record
    /// holds, one word a slot in slot order.
    draws: usize,
    /// plan-175-A: `__CANVAS_GEO_REFRESHES`, records rewritten in place.
    refreshes: usize,
}

fn global(builder: &CodeBuilder, name: &str) -> Result<usize, String> {
    builder
        .globals
        .get(&format!("#{name}"))
        .map(|g| g.offset)
        .ok_or_else(|| format!("canvas: the global __{name} is not declared"))
}

impl Globals {
    fn of(builder: &CodeBuilder) -> Result<Self, String> {
        Ok(Globals {
            data: global(builder, "CANVAS_GEO_DATA")?,
            slots: global(builder, "CANVAS_GEO_SLOTS")?,
            table: global(builder, "CANVAS_GEO_TABLE")?,
            frame: global(builder, "CANVAS_GEO_FRAME")?,
            used: global(builder, "CANVAS_GEO_USED_FLOATS")?,
            last: global(builder, "CANVAS_GEO_LAST_FLOATS")?,
            generations: global(builder, "CANVAS_GEO_GENERATIONS")?,
            compactions: global(builder, "CANVAS_GEO_COMPACTIONS")?,
            native_builds: global(builder, "CANVAS_GEO_NATIVE_BUILDS")?,
            built: global(builder, "CANVAS_GEO_BUILT")?,
            draws: global(builder, "CANVAS_GEO_DRAWS")?,
            refreshes: global(builder, "CANVAS_GEO_REFRESHES")?,
        })
    }
}

fn gload(builder: &mut CodeBuilder, offset: usize) -> VirtualRegister {
    let v = builder.temporary_vreg();
    builder.emit(abi::load_u64(&v, ARENA_STATE_REGISTER, offset));
    v
}

fn gstore(builder: &mut CodeBuilder, value: &VirtualRegister, offset: usize) {
    builder.emit(abi::store_u64(value, ARENA_STATE_REGISTER, offset));
}

/// `global += delta` for an `Integer` global.
fn gadd(builder: &mut CodeBuilder, offset: usize, delta: &VirtualRegister) {
    let v = gload(builder, offset);
    builder.emit(abi::add_registers(&v, &v, delta));
    gstore(builder, &v, offset);
}

fn imm(builder: &mut CodeBuilder, value: i64) -> VirtualRegister {
    let v = builder.temporary_vreg();
    if value < 0 {
        // The immediate encoder takes no negative literal.
        builder.emit(abi::move_immediate(&v, "Integer", "0"));
        builder.emit(abi::subtract_immediate(
            &v,
            &v,
            value.unsigned_abs() as usize,
        ));
    } else {
        builder.emit(abi::move_immediate(&v, "Integer", &value.to_string()));
    }
    v
}

/// `dst` = the address of word `index` of the fixed-width list at `list`.
fn word_addr(
    builder: &mut CodeBuilder,
    list: &VirtualRegister,
    index: &VirtualRegister,
) -> VirtualRegister {
    let a = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&a, index, 3));
    builder.emit(abi::add_registers(&a, list, &a));
    builder.emit(abi::add_immediate(&a, &a, COLLECTION_HEADER_SIZE));
    a
}

fn load_word(
    builder: &mut CodeBuilder,
    list: &VirtualRegister,
    index: &VirtualRegister,
    field_word: usize,
) -> VirtualRegister {
    let a = word_addr(builder, list, index);
    let v = builder.temporary_vreg();
    builder.emit(abi::load_u64(&v, &a, field_word * 8));
    v
}

/// Allocate a fixed-width 8-byte list block with `count = capacity = words` (its words
/// left unwritten), returning a stack slot holding it.
fn alloc_words(
    builder: &mut CodeBuilder,
    words: &VirtualRegister,
    capacity: &VirtualRegister,
    list_type: &ParameterType,
) -> Result<usize, String> {
    let layout = CollectionTypeLayout::from_type(list_type)
        .ok_or_else(|| format!("canvas: no layout for {list_type}"))?;
    let words_slot = builder.spill_to_slot("canvas_geo_words", words);
    let cap_slot = builder.spill_to_slot("canvas_geo_cap", capacity);
    let result_slot = builder.allocate_stack_object("canvas_geo_block", 8);
    let overflow = builder.label("canvas_geo_alloc_overflow");
    let ok = builder.label("canvas_geo_alloc_ok");
    let cap = builder.temporary_vreg();
    let eight = imm(builder, 8);
    let bytes = builder.temporary_vreg();
    builder.emit(abi::load_u64(&cap, abi::stack_pointer(), cap_slot));
    builder.emit_checked_size_multiply(&bytes, &cap, &eight, &overflow);
    builder.emit_checked_size_add_immediate(
        abi::return_register(),
        &bytes,
        COLLECTION_HEADER_SIZE,
        &overflow,
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&overflow));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    let base = builder.temporary_vreg();
    let n = builder.temporary_vreg();
    let c = builder.temporary_vreg();
    let nb = builder.temporary_vreg();
    let cb = builder.temporary_vreg();
    builder.emit(abi::load_u64(&base, abi::stack_pointer(), result_slot));
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), words_slot));
    builder.emit(abi::load_u64(&c, abi::stack_pointer(), cap_slot));
    builder.emit(abi::shift_left_immediate(&nb, &n, 3));
    builder.emit(abi::shift_left_immediate(&cb, &c, 3));
    builder.emit_write_collection_header_full(&layout, &base, &n, &c, &nb, &cb);
    Ok(result_slot)
}

/// Free the list block in stack slot `slot` (whose type is `list_type`).
fn free_block(
    builder: &mut CodeBuilder,
    slot: usize,
    list_type: &ParameterType,
) -> Result<(), String> {
    builder.emit_free_pre_grow_buffer(slot, list_type)
}

/// Ensure the fixed-width list in the global at `goff` has room for `extra` more words,
/// growing it geometrically (at least doubling, at least 64 words) when it does not.
/// Its words are kept; nothing but its block moves. Every pointer into the list taken
/// before this call is stale after it.
fn emit_reserve(
    builder: &mut CodeBuilder,
    goff: usize,
    extra: &VirtualRegister,
    list_type: &ParameterType,
) -> Result<(), String> {
    let done = builder.label("canvas_geo_reserve_done");
    let list = gload(builder, goff);
    let count = builder.temporary_vreg();
    let cap = builder.temporary_vreg();
    let need = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &list, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::load_u64(&cap, &list, COLLECTION_OFFSET_CAPACITY));
    builder.emit(abi::add_registers(&need, &count, extra));
    builder.emit(abi::compare_registers(&need, &cap));
    builder.emit(abi::branch_ls(&done));
    // newCap = max(2 * cap, need, 64).
    let new_cap = builder.temporary_vreg();
    builder.emit(abi::add_registers(&new_cap, &cap, &cap));
    let keep1 = builder.label("canvas_geo_reserve_k1");
    builder.emit(abi::compare_registers(&new_cap, &need));
    builder.emit(abi::branch_hi(&keep1));
    builder.emit(abi::move_register(&new_cap, &need));
    builder.emit(abi::label(&keep1));
    let keep2 = builder.label("canvas_geo_reserve_k2");
    builder.emit(abi::compare_immediate(&new_cap, "64"));
    builder.emit(abi::branch_hi(&keep2));
    builder.emit(abi::move_immediate(&new_cap, "Integer", "64"));
    builder.emit(abi::label(&keep2));
    let old_slot = builder.spill_to_slot("canvas_geo_old", &list);
    let fresh_slot = alloc_words(builder, &count, &new_cap, list_type)?;
    // Copy the live words.
    let old = builder.temporary_vreg();
    let fresh = builder.temporary_vreg();
    let bytes = builder.temporary_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit(abi::load_u64(&old, abi::stack_pointer(), old_slot));
    builder.emit(abi::load_u64(&fresh, abi::stack_pointer(), fresh_slot));
    builder.emit(abi::load_u64(&bytes, &old, COLLECTION_OFFSET_DATA_LENGTH));
    builder.emit(abi::add_immediate(&old, &old, COLLECTION_HEADER_SIZE));
    builder.emit(abi::add_immediate(&fresh, &fresh, COLLECTION_HEADER_SIZE));
    builder.emit_block_copy_advance(&fresh, &old, &bytes, &scratch, "canvas_geo_reserve_copy");
    free_block(builder, old_slot, list_type)?;
    let fresh = builder.temporary_vreg();
    builder.emit(abi::load_u64(&fresh, abi::stack_pointer(), fresh_slot));
    gstore(builder, &fresh, goff);
    builder.emit(abi::label(&done));
    Ok(())
}

/// Append one word to the fixed-width list in the global at `goff`, which must have
/// room (`emit_reserve`).
fn emit_push(builder: &mut CodeBuilder, goff: usize, value: &VirtualRegister) {
    let list = gload(builder, goff);
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &list, COLLECTION_OFFSET_COUNT));
    let at = word_addr(builder, &list, &count);
    builder.emit(abi::store_u64(value, &at, 0));
    builder.emit(abi::add_immediate(&count, &count, 1));
    builder.emit(abi::store_u64(&count, &list, COLLECTION_OFFSET_COUNT));
    let dl = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&dl, &count, 3));
    builder.emit(abi::store_u64(&dl, &list, COLLECTION_OFFSET_DATA_LENGTH));
}

/// Set the count of the fixed-width list in the global at `goff` (within capacity).
fn emit_set_count(builder: &mut CodeBuilder, goff: usize, count: &VirtualRegister) {
    let list = gload(builder, goff);
    builder.emit(abi::store_u64(count, &list, COLLECTION_OFFSET_COUNT));
    let dl = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&dl, count, 3));
    builder.emit(abi::store_u64(&dl, &list, COLLECTION_OFFSET_DATA_LENGTH));
}

fn function(
    name: &'static str,
    params: Vec<Parameter>,
    return_type: ParameterType,
    errors: Vec<&'static str>,
    lower: crate::codegen::registry::AbiInline,
) -> RegistryFunction {
    RegistryFunction {
        name,
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params,
            return_type,
            errors,
            body: Body::abi_inline(lower),
        }],
    }
}

fn param(name: &'static str, ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases: &[],
        ty,
        default: DefaultValue::None,
    }
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(function(
        "geoFind",
        vec![param("hash", ParameterType::Integer)],
        ParameterType::Integer,
        vec![],
        lower_geo_find,
    ));
    pkg.add_function(function(
        "geoInsert",
        vec![
            param("hash", ParameterType::Integer),
            param("draw", ParameterType::Integer),
            param("header", float_list()),
            param("tail", float_list()),
        ],
        ParameterType::Integer,
        vec!["ErrOutOfMemory"],
        lower_geo_insert,
    ));
    pkg.add_function(function(
        "geoRefresh",
        vec![
            param("slot", ParameterType::Integer),
            param("draw", ParameterType::Integer),
            param("header", float_list()),
            param("tail", float_list()),
        ],
        ParameterType::Integer,
        vec!["ErrOutOfMemory"],
        lower_geo_refresh,
    ));
    pkg.add_function(function(
        "geoRepaint",
        vec![
            param("slot", ParameterType::Integer),
            param("draw", ParameterType::Integer),
            param("header", float_list()),
            param("stops", float_list()),
        ],
        ParameterType::Integer,
        vec![],
        lower_geo_repaint,
    ));
    pkg.add_function(function(
        "geoForget",
        vec![param("slot", ParameterType::Integer)],
        ParameterType::Nothing,
        vec![],
        lower_geo_forget,
    ));
    pkg.add_function(function(
        "geoBeginFrame",
        vec![],
        ParameterType::Nothing,
        vec!["ErrOutOfMemory"],
        lower_geo_begin_frame,
    ));
    pkg.add_function(function(
        "sceneResolve",
        vec![],
        ParameterType::Integer,
        vec!["ErrOutOfMemory"],
        lower_scene_resolve,
    ));
    pkg.add_function(function(
        "sceneSnapshot",
        vec![],
        ParameterType::Boolean,
        vec![],
        lower_scene_snapshot,
    ));
    pkg.add_function(function(
        "snapshotItems",
        vec![],
        ParameterType::list_of(ParameterType::named("DrawItem")),
        vec!["ErrOutOfMemory"],
        lower_snapshot_items,
    ));
    pkg.add_function(function(
        "snapshotLayers",
        vec![],
        ParameterType::list_of(ParameterType::named("DrawLayer")),
        vec!["ErrOutOfMemory"],
        lower_snapshot_layers,
    ));
    pkg.add_function(function(
        "snapshotHashes",
        vec![],
        int_list(),
        vec!["ErrOutOfMemory"],
        lower_snapshot_hashes,
    ));
    pkg.add_function(function(
        "sceneDrawsFlat",
        vec![],
        ParameterType::Boolean,
        vec!["ErrOutOfMemory"],
        lower_scene_draws_flat,
    ));
    pkg.add_function(function(
        "sceneLayout",
        vec![param("spans", int_list()), param("side", int_list())],
        int_list(),
        vec!["ErrOutOfMemory"],
        lower_scene_layout,
    ));
}

#[cfg(test)]
mod tests {
    use super::{PICTURE_KIND_BITS, TILE_KIND_BITS};
    use crate::codegen::runtime::canvas::{GEO_KIND_PICTURE, GEO_KIND_TILE};

    /// `emit_hit` compares a record's slot 0 against these as raw bits, so each must be
    /// exactly the float its kind constant names.
    #[test]
    fn image_kind_bits_are_their_kinds_as_floats() {
        for (bits, kind) in [
            (PICTURE_KIND_BITS, GEO_KIND_PICTURE),
            (TILE_KIND_BITS, GEO_KIND_TILE),
        ] {
            let kind: f64 = kind.parse().expect("a numeric kind");
            assert_eq!(bits, kind.to_bits(), "kind {kind}");
        }
    }
}
