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
    record_type,
};
use super::func_item_hash::emit_item_hash;
use super::scene_base::scene_base;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Words per slot in `__CANVAS_GEO_SLOTS`.
const SLOT_WORDS: usize = 4;
/// The smallest table, in buckets.
const MIN_BUCKETS: usize = 64;
/// `__CANVAS_GEO_PICTURE`, as the bit pattern of the float in a record's slot 0.
const PICTURE_KIND_BITS: u64 = 0x4022_0000_0000_0000; // 9.0

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

/// The first bucket a hash probes: its bits folded down, masked to the table.
fn emit_bucket(
    builder: &mut CodeBuilder,
    hash: &VirtualRegister,
    mask: &VirtualRegister,
) -> VirtualRegister {
    let b = builder.temporary_vreg();
    builder.emit(abi::shift_right_immediate(&b, hash, 29));
    builder.emit(abi::exclusive_or_registers(&b, &b, hash));
    builder.emit(abi::and_registers(&b, &b, mask));
    b
}

/// `slot` = the slot the table maps `hash` to, or -1 (absent or forgotten).
fn emit_find(
    builder: &mut CodeBuilder,
    g: &Globals,
    hash: &VirtualRegister,
    slot: &VirtualRegister,
) {
    let miss = builder.label("canvas_geo_find_miss");
    let done = builder.label("canvas_geo_find_done");
    let head = builder.label("canvas_geo_find_probe");
    let table = gload(builder, g.table);
    let words = builder.temporary_vreg();
    builder.emit(abi::load_u64(&words, &table, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&words, "0"));
    builder.emit(abi::branch_eq(&miss));
    let mask = builder.temporary_vreg();
    builder.emit(abi::shift_right_immediate(&mask, &words, 1));
    builder.emit(abi::subtract_immediate(&mask, &mask, 1));
    let b = emit_bucket(builder, hash, &mask);
    let empty = imm(builder, -1);
    let key = builder.temporary_vreg();
    let at = builder.temporary_vreg();
    builder.emit(abi::label(&head));
    builder.emit(abi::shift_left_immediate(&at, &b, 4));
    builder.emit(abi::add_registers(&at, &table, &at));
    builder.emit(abi::load_u64(&key, &at, COLLECTION_HEADER_SIZE));
    builder.emit(abi::compare_registers(&key, &empty));
    builder.emit(abi::branch_eq(&miss));
    let next = builder.label("canvas_geo_find_next");
    builder.emit(abi::compare_registers(&key, hash));
    builder.emit(abi::branch_ne(&next));
    builder.emit(abi::load_u64(slot, &at, COLLECTION_HEADER_SIZE + 8));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&b, &b, 1));
    builder.emit(abi::and_registers(&b, &b, &mask));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&miss));
    builder.emit(abi::move_immediate(slot, "Integer", "0"));
    builder.emit(abi::subtract_immediate(slot, slot, 1));
    builder.emit(abi::label(&done));
}

/// Map `hash` to `slot` in a table with room (at most half full). A bucket already
/// holding `hash` is overwritten, so the newest slot for a key wins.
fn emit_put(
    builder: &mut CodeBuilder,
    g: &Globals,
    hash: &VirtualRegister,
    slot: &VirtualRegister,
) {
    let head = builder.label("canvas_geo_put_probe");
    let write = builder.label("canvas_geo_put_write");
    let table = gload(builder, g.table);
    let words = builder.temporary_vreg();
    builder.emit(abi::load_u64(&words, &table, COLLECTION_OFFSET_COUNT));
    let mask = builder.temporary_vreg();
    builder.emit(abi::shift_right_immediate(&mask, &words, 1));
    builder.emit(abi::subtract_immediate(&mask, &mask, 1));
    let b = emit_bucket(builder, hash, &mask);
    let empty = imm(builder, -1);
    let key = builder.temporary_vreg();
    let at = builder.temporary_vreg();
    builder.emit(abi::label(&head));
    builder.emit(abi::shift_left_immediate(&at, &b, 4));
    builder.emit(abi::add_registers(&at, &table, &at));
    builder.emit(abi::load_u64(&key, &at, COLLECTION_HEADER_SIZE));
    builder.emit(abi::compare_registers(&key, &empty));
    builder.emit(abi::branch_eq(&write));
    builder.emit(abi::compare_registers(&key, hash));
    builder.emit(abi::branch_eq(&write));
    builder.emit(abi::add_immediate(&b, &b, 1));
    builder.emit(abi::and_registers(&b, &b, &mask));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&write));
    builder.emit(abi::store_u64(hash, &at, COLLECTION_HEADER_SIZE));
    builder.emit(abi::store_u64(slot, &at, COLLECTION_HEADER_SIZE + 8));
}

/// Rebuild the table from `__CANVAS_GEO_SLOTS`: size it to at least twice the slot
/// count (a power of two, at least `MIN_BUCKETS`), clear it, and insert every slot not
/// forgotten, in slot order.
fn emit_rebuild(builder: &mut CodeBuilder, g: &Globals) -> Result<(), String> {
    let slots = gload(builder, g.slots);
    let nslots = builder.temporary_vreg();
    builder.emit(abi::load_u64(&nslots, &slots, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::shift_right_immediate(&nslots, &nslots, 2));
    // buckets = MIN; while buckets < 2 * nslots + 2: buckets *= 2.
    let want = builder.temporary_vreg();
    builder.emit(abi::add_registers(&want, &nslots, &nslots));
    builder.emit(abi::add_immediate(&want, &want, 2));
    let buckets = imm(builder, MIN_BUCKETS as i64);
    let grow = builder.label("canvas_geo_rebuild_size");
    let sized = builder.label("canvas_geo_rebuild_sized");
    builder.emit(abi::label(&grow));
    builder.emit(abi::compare_registers(&buckets, &want));
    builder.emit(abi::branch_ge(&sized));
    builder.emit(abi::add_registers(&buckets, &buckets, &buckets));
    builder.emit(abi::branch(&grow));
    builder.emit(abi::label(&sized));
    let words = builder.temporary_vreg();
    builder.emit(abi::add_registers(&words, &buckets, &buckets));
    // Reuse the block when it already has room for that many words.
    let table = gload(builder, g.table);
    let cap = builder.temporary_vreg();
    builder.emit(abi::load_u64(&cap, &table, COLLECTION_OFFSET_CAPACITY));
    let reuse = builder.label("canvas_geo_rebuild_reuse");
    let cleared = builder.label("canvas_geo_rebuild_clear");
    builder.emit(abi::compare_registers(&words, &cap));
    builder.emit(abi::branch_ls(&reuse));
    let old_slot = builder.spill_to_slot("canvas_geo_old_table", &table);
    let fresh_slot = alloc_words(builder, &words, &words, &int_list())?;
    free_block(builder, old_slot, &int_list())?;
    let fresh = builder.temporary_vreg();
    builder.emit(abi::load_u64(&fresh, abi::stack_pointer(), fresh_slot));
    gstore(builder, &fresh, g.table);
    builder.emit(abi::branch(&cleared));
    builder.emit(abi::label(&reuse));
    emit_set_count(builder, g.table, &words);
    builder.emit(abi::label(&cleared));
    // Clear to -1.
    let table = gload(builder, g.table);
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, &table, COLLECTION_OFFSET_COUNT));
    let empty = imm(builder, -1);
    let cursor = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&cursor, &table, COLLECTION_HEADER_SIZE));
    let fill = builder.label("canvas_geo_rebuild_fill");
    let filled = builder.label("canvas_geo_rebuild_filled");
    builder.emit(abi::label(&fill));
    builder.emit(abi::compare_immediate(&n, "0"));
    builder.emit(abi::branch_eq(&filled));
    builder.emit(abi::store_u64(&empty, &cursor, 0));
    builder.emit(abi::add_immediate(&cursor, &cursor, 8));
    builder.emit(abi::subtract_immediate(&n, &n, 1));
    builder.emit(abi::branch(&fill));
    builder.emit(abi::label(&filled));
    // Insert every live slot.
    let s = imm(builder, 0);
    let head = builder.label("canvas_geo_rebuild_insert");
    let done = builder.label("canvas_geo_rebuild_done");
    let skip = builder.label("canvas_geo_rebuild_skip");
    builder.emit(abi::label(&head));
    let slots = gload(builder, g.slots);
    let total = builder.temporary_vreg();
    builder.emit(abi::load_u64(&total, &slots, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::shift_right_immediate(&total, &total, 2));
    builder.emit(abi::compare_registers(&s, &total));
    builder.emit(abi::branch_ge(&done));
    let base = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&base, &s, 2));
    let used = load_word(builder, &slots, &base, 3);
    let empty = imm(builder, -1);
    builder.emit(abi::compare_registers(&used, &empty));
    builder.emit(abi::branch_eq(&skip));
    let hash = load_word(builder, &slots, &base, 0);
    emit_put(builder, g, &hash, &s);
    builder.emit(abi::label(&skip));
    builder.emit(abi::add_immediate(&s, &s, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));
    Ok(())
}

/// Index `slot` (the last one appended) under `hash`, rebuilding a bigger table first
/// when it would pass half full.
fn emit_index(
    builder: &mut CodeBuilder,
    g: &Globals,
    hash: &VirtualRegister,
    slot: &VirtualRegister,
) -> Result<(), String> {
    let put = builder.label("canvas_geo_index_put");
    let done = builder.label("canvas_geo_index_done");
    let table = gload(builder, g.table);
    let words = builder.temporary_vreg();
    builder.emit(abi::load_u64(&words, &table, COLLECTION_OFFSET_COUNT));
    // Room while 2 * (slot + 1) <= buckets, i.e. 4 * (slot + 1) <= words.
    let need = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&need, slot, 1));
    builder.emit(abi::shift_left_immediate(&need, &need, 2));
    builder.emit(abi::compare_registers(&need, &words));
    builder.emit(abi::branch_ls(&put));
    // The rebuild re-inserts every slot, the one just appended included.
    emit_rebuild(builder, g)?;
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&put));
    emit_put(builder, g, hash, slot);
    builder.emit(abi::label(&done));
    Ok(())
}

/// Stamp `slot` as used by the frame in progress, counting its floats once per frame
/// (`__canvas_geoStamp`).
fn emit_stamp(builder: &mut CodeBuilder, g: &Globals, slot: &VirtualRegister) {
    let done = builder.label("canvas_geo_stamp_done");
    let slots = gload(builder, g.slots);
    let base = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&base, slot, 2));
    let at = word_addr(builder, &slots, &base);
    let used = builder.temporary_vreg();
    builder.emit(abi::load_u64(&used, &at, 24));
    let frame = gload(builder, g.frame);
    builder.emit(abi::compare_registers(&used, &frame));
    builder.emit(abi::branch_eq(&done));
    builder.emit(abi::store_u64(&frame, &at, 24));
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &at, 16));
    gadd(builder, g.used, &count);
    builder.emit(abi::label(&done));
}

/// Append a slot `(hash, offset, count, FRAME)` for floats already in
/// `__CANVAS_GEO_DATA`, count it as used and generated, and index it.
fn emit_add_slot(
    builder: &mut CodeBuilder,
    g: &Globals,
    hash: &VirtualRegister,
    offset: &VirtualRegister,
    count: &VirtualRegister,
) -> Result<VirtualRegister, String> {
    let hash_slot = builder.spill_to_slot("canvas_geo_add_hash", hash);
    let offset_slot = builder.spill_to_slot("canvas_geo_add_offset", offset);
    let count_slot = builder.spill_to_slot("canvas_geo_add_count", count);
    let four = imm(builder, SLOT_WORDS as i64);
    emit_reserve(builder, g.slots, &four, &int_list())?;
    let slots = gload(builder, g.slots);
    let index = builder.temporary_vreg();
    builder.emit(abi::load_u64(&index, &slots, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::shift_right_immediate(&index, &index, 2));
    let index_slot = builder.spill_to_slot("canvas_geo_add_index", &index);
    for slot in [hash_slot, offset_slot, count_slot] {
        let v = builder.temporary_vreg();
        builder.emit(abi::load_u64(&v, abi::stack_pointer(), slot));
        emit_push(builder, g.slots, &v);
    }
    let frame = gload(builder, g.frame);
    emit_push(builder, g.slots, &frame);
    let c = builder.temporary_vreg();
    builder.emit(abi::load_u64(&c, abi::stack_pointer(), count_slot));
    gadd(builder, g.used, &c);
    let one = imm(builder, 1);
    gadd(builder, g.generations, &one);
    let h = builder.temporary_vreg();
    let s = builder.temporary_vreg();
    builder.emit(abi::load_u64(&h, abi::stack_pointer(), hash_slot));
    builder.emit(abi::load_u64(&s, abi::stack_pointer(), index_slot));
    emit_index(builder, g, &h, &s)?;
    let s = builder.temporary_vreg();
    builder.emit(abi::load_u64(&s, abi::stack_pointer(), index_slot));
    Ok(s)
}

/// The inline call's value: an `Integer` (or the layout's list) in a fresh register, or
/// `Nothing`.
fn finish(builder: &mut CodeBuilder, value: Option<&VirtualRegister>, text: &str) -> ValueResult {
    let result = builder.allocate_register();
    let type_ = match value {
        Some(v) => {
            builder.emit(abi::move_register(&result, v));
            if text == "canvas.sceneLayout" {
                int_list()
            } else {
                ParameterType::Integer
            }
        }
        None => {
            builder.emit(abi::move_immediate(&result, "Integer", "0"));
            ParameterType::Nothing
        }
    };
    ValueResult {
        origin: None,
        type_,
        location: Operand::from(result.render()),
        text: text.to_string(),
    }
}

/// `canvas::geoFind(hash) AS Integer`: the slot `hash` is cached in, or -1.
fn lower_geo_find(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let g = Globals::of(builder)?;
    let hash = builder.temporary_vreg();
    builder.emit(abi::move_register(&hash, &args[0].location));
    let slot = builder.temporary_vreg();
    emit_find(builder, &g, &hash, &slot);
    Ok(finish(builder, Some(&slot), "canvas.geoFind"))
}

/// `canvas::geoInsert(hash, header, tail) AS Integer`: append `header` then `tail` to
/// `__CANVAS_GEO_DATA` as a new slot used by this frame, index it under `hash`, and
/// answer its offset. Counts one generation.
fn lower_geo_insert(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let g = Globals::of(builder)?;
    let hash_slot = builder.spill_to_slot("canvas_geo_ins_hash", &args[0].location);
    let header_slot = builder.spill_to_slot("canvas_geo_ins_header", &args[1].location);
    let tail_slot = builder.spill_to_slot("canvas_geo_ins_tail", &args[2].location);
    let header = builder.temporary_vreg();
    let tail = builder.temporary_vreg();
    builder.emit(abi::load_u64(&header, abi::stack_pointer(), header_slot));
    builder.emit(abi::load_u64(&tail, abi::stack_pointer(), tail_slot));
    let n = builder.temporary_vreg();
    let nt = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, &header, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::load_u64(&nt, &tail, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::add_registers(&n, &n, &nt));
    let n_slot = builder.spill_to_slot("canvas_geo_ins_n", &n);
    emit_reserve(builder, g.data, &n, &float_list())?;
    let data = gload(builder, g.data);
    let offset = builder.temporary_vreg();
    builder.emit(abi::load_u64(&offset, &data, COLLECTION_OFFSET_COUNT));
    let offset_slot = builder.spill_to_slot("canvas_geo_ins_offset", &offset);
    let dst = word_addr(builder, &data, &offset);
    for list_slot in [header_slot, tail_slot] {
        let list = builder.temporary_vreg();
        let bytes = builder.temporary_vreg();
        let scratch = builder.temporary_vreg();
        builder.emit(abi::load_u64(&list, abi::stack_pointer(), list_slot));
        builder.emit(abi::load_u64(&bytes, &list, COLLECTION_OFFSET_COUNT));
        builder.emit(abi::shift_left_immediate(&bytes, &bytes, 3));
        builder.emit(abi::add_immediate(&list, &list, COLLECTION_HEADER_SIZE));
        builder.emit_block_copy_advance(&dst, &list, &bytes, &scratch, "canvas_geo_ins_copy");
    }
    let n = builder.temporary_vreg();
    let offset = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    builder.emit(abi::load_u64(&offset, abi::stack_pointer(), offset_slot));
    let total = builder.temporary_vreg();
    builder.emit(abi::add_registers(&total, &offset, &n));
    emit_set_count(builder, g.data, &total);
    let hash = builder.temporary_vreg();
    builder.emit(abi::load_u64(&hash, abi::stack_pointer(), hash_slot));
    emit_add_slot(builder, &g, &hash, &offset, &n)?;
    let offset = builder.temporary_vreg();
    builder.emit(abi::load_u64(&offset, abi::stack_pointer(), offset_slot));
    Ok(finish(builder, Some(&offset), "canvas.geoInsert"))
}

/// `canvas::geoForget(slot)`: drop `slot` from the index for good — the glyph cache is
/// renumbering the glyph indices its record holds. It is never hit again; its floats go
/// at the next frame boundary that compacts.
fn lower_geo_forget(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let g = Globals::of(builder)?;
    let slot = builder.temporary_vreg();
    builder.emit(abi::move_register(&slot, &args[0].location));
    let slots = gload(builder, g.slots);
    let base = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&base, &slot, 2));
    let hash = load_word(builder, &slots, &base, 0);
    let at = word_addr(builder, &slots, &base);
    let gone = imm(builder, -1);
    builder.emit(abi::store_u64(&gone, &at, 24));
    // The bucket keeps its key (a tombstone) and loses its slot, if it names this one.
    let mapped = builder.temporary_vreg();
    emit_find(builder, &g, &hash, &mapped);
    let done = builder.label("canvas_geo_forget_done");
    builder.emit(abi::compare_registers(&mapped, &slot));
    builder.emit(abi::branch_ne(&done));
    emit_put(builder, &g, &hash, &gone);
    builder.emit(abi::label(&done));
    Ok(finish(builder, None, "canvas.geoForget"))
}

/// `canvas::geoBeginFrame()`: the frame boundary (the rules `__canvas_geoBeginFrame` had in MFBASIC).
/// When the floats the previous frame did NOT use reach the ones it did, keep only the
/// slots it used — their floats moved down in place, slot order kept, the list's
/// capacity kept — and rebuild the index. Then advance the frame.
fn lower_geo_begin_frame(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let g = Globals::of(builder)?;
    let used = gload(builder, g.used);
    gstore(builder, &used, g.last);
    let zero = imm(builder, 0);
    gstore(builder, &zero, g.used);
    let data = gload(builder, g.data);
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &data, COLLECTION_OFFSET_COUNT));
    let stale = builder.temporary_vreg();
    builder.emit(abi::subtract_registers(&stale, &count, &used));
    let advance = builder.label("canvas_geo_begin_advance");
    builder.emit(abi::compare_immediate(&stale, "0"));
    builder.emit(abi::branch_le(&advance));
    builder.emit(abi::compare_registers(&stale, &used));
    builder.emit(abi::branch_lt(&advance));

    let one = imm(builder, 1);
    gadd(builder, g.compactions, &one);
    let frame = gload(builder, g.frame);
    let s = imm(builder, 0);
    let k = imm(builder, 0);
    let w = imm(builder, 0);
    let head = builder.label("canvas_geo_compact");
    let next = builder.label("canvas_geo_compact_next");
    let done = builder.label("canvas_geo_compact_done");
    let slots = gload(builder, g.slots);
    let data = gload(builder, g.data);
    let total = builder.temporary_vreg();
    builder.emit(abi::load_u64(&total, &slots, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::shift_right_immediate(&total, &total, 2));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&s, &total));
    builder.emit(abi::branch_ge(&done));
    let base = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&base, &s, 2));
    let src_at = word_addr(builder, &slots, &base);
    let last_used = builder.temporary_vreg();
    builder.emit(abi::load_u64(&last_used, &src_at, 24));
    builder.emit(abi::compare_registers(&last_used, &frame));
    builder.emit(abi::branch_ne(&next));
    let hash = builder.temporary_vreg();
    let from = builder.temporary_vreg();
    let owned = builder.temporary_vreg();
    builder.emit(abi::load_u64(&hash, &src_at, 0));
    builder.emit(abi::load_u64(&from, &src_at, 8));
    builder.emit(abi::load_u64(&owned, &src_at, 16));
    // Move the floats down: slots sit in the list in slot order, so `w <= from` and a
    // forward copy is safe.
    let moved = builder.label("canvas_geo_compact_moved");
    builder.emit(abi::compare_registers(&from, &w));
    builder.emit(abi::branch_eq(&moved));
    let dst = word_addr(builder, &data, &w);
    let src = word_addr(builder, &data, &from);
    let bytes = builder.temporary_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&bytes, &owned, 3));
    builder.emit_block_copy_advance(&dst, &src, &bytes, &scratch, "canvas_geo_compact_copy");
    builder.emit(abi::label(&moved));
    let kbase = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&kbase, &k, 2));
    let dst_at = word_addr(builder, &slots, &kbase);
    builder.emit(abi::store_u64(&hash, &dst_at, 0));
    builder.emit(abi::store_u64(&w, &dst_at, 8));
    builder.emit(abi::store_u64(&owned, &dst_at, 16));
    builder.emit(abi::store_u64(&frame, &dst_at, 24));
    builder.emit(abi::add_registers(&w, &w, &owned));
    builder.emit(abi::add_immediate(&k, &k, 1));
    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&s, &s, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));
    emit_set_count(builder, g.data, &w);
    let kwords = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&kwords, &k, 2));
    emit_set_count(builder, g.slots, &kwords);
    emit_rebuild(builder, &g)?;

    builder.emit(abi::label(&advance));
    let frame = gload(builder, g.frame);
    builder.emit(abi::add_immediate(&frame, &frame, 1));
    gstore(builder, &frame, g.frame);
    Ok(finish(builder, None, "canvas.geoBeginFrame"))
}

/// `canvas::sceneResolve() AS Integer`: resolve every item of the installed scene — the
/// flat items, then each layer's, in `canvas::installedHashes()` order — into
/// `__CANVAS_TOP_OFFSETS`, and answer how many it left at -1 for MFBASIC.
///
/// Per index, by the published hash: a hit is stamped and resolved, except a
/// `Picture`'s (its pixels can change under an unchanged item); a miss of a kind
/// `canvas::geoBuild` builds is built straight into `__CANVAS_GEO_DATA`; anything else —
/// a picture, a `Text`, a `Group`, a declined paint or kind, or an index past the items —
/// is left at -1.
///
/// A built record is indexed under the item's OWN hash (`canvas::itemHash`), not the
/// published one. They differ only when a frame runs between `publishScene` and
/// `publishHashes`, and keying the new item's geometry under the old item's hash would
/// hand it to that old item forever after.
///
/// `__CANVAS_GEO_BUILT` gets `(index, slot)` for each record built here, for the
/// `--debug` geometry check.
fn lower_scene_resolve(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let g = Globals::of(builder)?;
    let top_off = global(builder, "CANVAS_TOP_OFFSETS")?;
    let item_list_type = ParameterType::list_of(ParameterType::named("DrawItem"));
    let layer_type = record_type(builder, &ParameterType::named("DrawLayer"))?;
    let layer_list_type = ParameterType::list_of(ParameterType::named("DrawLayer"));

    // The published hashes, and how many indices this frame has.
    let scene = scene_base(builder);
    let hashes = builder.temporary_vreg();
    builder.emit(abi::load_u64(&hashes, &scene, CANVAS_SCENE_HASHES_OFFSET));
    let hcount = imm(builder, 0);
    let no_hashes = builder.label("canvas_resolve_no_hashes");
    builder.emit(abi::compare_immediate(&hashes, "0"));
    builder.emit(abi::branch_eq(&no_hashes));
    builder.emit(abi::load_u64(&hcount, &hashes, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::label(&no_hashes));
    let hashes_slot = builder.spill_to_slot("canvas_resolve_hashes", &hashes);
    let hcount_slot = builder.spill_to_slot("canvas_resolve_hcount", &hcount);

    // __CANVAS_TOP_OFFSETS = hcount x -1; __CANVAS_GEO_BUILT = [].
    let zero = imm(builder, 0);
    emit_set_count(builder, top_off, &zero);
    emit_set_count(builder, g.built, &zero);
    let hcount = builder.temporary_vreg();
    builder.emit(abi::load_u64(&hcount, abi::stack_pointer(), hcount_slot));
    emit_reserve(builder, top_off, &hcount, &int_list())?;
    let hcount = builder.temporary_vreg();
    builder.emit(abi::load_u64(&hcount, abi::stack_pointer(), hcount_slot));
    emit_set_count(builder, top_off, &hcount);
    {
        let top = gload(builder, top_off);
        let cursor = builder.temporary_vreg();
        let n = builder.temporary_vreg();
        builder.emit(abi::add_immediate(&cursor, &top, COLLECTION_HEADER_SIZE));
        builder.emit(abi::move_register(&n, &hcount));
        let empty = imm(builder, -1);
        let fill = builder.label("canvas_resolve_fill");
        let filled = builder.label("canvas_resolve_filled");
        builder.emit(abi::label(&fill));
        builder.emit(abi::compare_immediate(&n, "0"));
        builder.emit(abi::branch_eq(&filled));
        builder.emit(abi::store_u64(&empty, &cursor, 0));
        builder.emit(abi::add_immediate(&cursor, &cursor, 8));
        builder.emit(abi::subtract_immediate(&n, &n, 1));
        builder.emit(abi::branch(&fill));
        builder.emit(abi::label(&filled));
    }

    // Walk the segments: the flat items (segment -1), then each layer's items.
    let gi_slot = builder.spill_to_slot("canvas_resolve_gi", &zero);
    let resolved_slot = builder.spill_to_slot("canvas_resolve_resolved", &zero);
    let minus = imm(builder, -1);
    let seg_slot = builder.spill_to_slot("canvas_resolve_seg", &minus);
    let list_slot = builder.allocate_stack_object("canvas_resolve_list", 8);
    let j_slot = builder.allocate_stack_object("canvas_resolve_j", 8);
    let item_slot = builder.allocate_stack_object("canvas_resolve_item", 8);
    let key_slot = builder.allocate_stack_object("canvas_resolve_key", 8);
    let n_slot = builder.allocate_stack_object("canvas_resolve_n", 8);
    let off_slot = builder.allocate_stack_object("canvas_resolve_off", 8);

    let next_segment = builder.label("canvas_resolve_segment");
    let layer_segment = builder.label("canvas_resolve_layer");
    let walk = builder.label("canvas_resolve_walk");
    let item_head = builder.label("canvas_resolve_item");
    let item_next = builder.label("canvas_resolve_next");
    let all_done = builder.label("canvas_resolve_done");

    builder.emit(abi::label(&next_segment));
    {
        let seg = builder.temporary_vreg();
        builder.emit(abi::load_u64(&seg, abi::stack_pointer(), seg_slot));
        builder.emit(abi::compare_immediate(&seg, "0"));
        builder.emit(abi::branch_ge(&layer_segment));
        // Segment -1: the flat items.
        let zero = imm(builder, 0);
        builder.emit(abi::store_u64(&zero, abi::stack_pointer(), seg_slot));
        let scene = scene_base(builder);
        let items = builder.temporary_vreg();
        builder.emit(abi::load_u64(&items, &scene, CANVAS_SCENE_ITEMS_OFFSET));
        builder.emit(abi::compare_immediate(&items, "0"));
        builder.emit(abi::branch_eq(&next_segment));
        builder.emit(abi::store_u64(&items, abi::stack_pointer(), list_slot));
        builder.emit(abi::branch(&walk));
    }
    builder.emit(abi::label(&layer_segment));
    {
        let scene = scene_base(builder);
        let layers = builder.temporary_vreg();
        builder.emit(abi::load_u64(&layers, &scene, CANVAS_SCENE_LAYERS_OFFSET));
        builder.emit(abi::compare_immediate(&layers, "0"));
        builder.emit(abi::branch_eq(&all_done));
        let seg = builder.temporary_vreg();
        let lcount = builder.temporary_vreg();
        builder.emit(abi::load_u64(&seg, abi::stack_pointer(), seg_slot));
        builder.emit(abi::load_u64(&lcount, &layers, COLLECTION_OFFSET_COUNT));
        builder.emit(abi::compare_registers(&seg, &lcount));
        builder.emit(abi::branch_ge(&all_done));
        let view = emit_list_view(builder, &layers, &layer_list_type)?;
        let layer = builder.temporary_vreg();
        emit_list_element(builder, &layer, &view, &seg);
        let items = builder.temporary_vreg();
        emit_field_block(builder, &items, &layer, &layer_type, "items")?;
        builder.emit(abi::store_u64(&items, abi::stack_pointer(), list_slot));
        builder.emit(abi::add_immediate(&seg, &seg, 1));
        builder.emit(abi::store_u64(&seg, abi::stack_pointer(), seg_slot));
    }
    builder.emit(abi::label(&walk));
    let zero = imm(builder, 0);
    builder.emit(abi::store_u64(&zero, abi::stack_pointer(), j_slot));
    builder.emit(abi::label(&item_head));
    {
        let list = builder.temporary_vreg();
        let j = builder.temporary_vreg();
        let lcount = builder.temporary_vreg();
        builder.emit(abi::load_u64(&list, abi::stack_pointer(), list_slot));
        builder.emit(abi::load_u64(&j, abi::stack_pointer(), j_slot));
        builder.emit(abi::load_u64(&lcount, &list, COLLECTION_OFFSET_COUNT));
        builder.emit(abi::compare_registers(&j, &lcount));
        builder.emit(abi::branch_ge(&next_segment));
        let gi = builder.temporary_vreg();
        let hcount = builder.temporary_vreg();
        builder.emit(abi::load_u64(&gi, abi::stack_pointer(), gi_slot));
        builder.emit(abi::load_u64(&hcount, abi::stack_pointer(), hcount_slot));
        builder.emit(abi::compare_registers(&gi, &hcount));
        builder.emit(abi::branch_ge(&all_done));
        let hashes = builder.temporary_vreg();
        builder.emit(abi::load_u64(&hashes, abi::stack_pointer(), hashes_slot));
        let hash = load_word(builder, &hashes, &gi, 0);
        let slot = builder.temporary_vreg();
        emit_find(builder, &g, &hash, &slot);
        let miss = builder.label("canvas_resolve_miss");
        builder.emit(abi::compare_immediate(&slot, "0"));
        builder.emit(abi::branch_lt(&miss));
        // A hit: resolved, unless it is a picture.
        emit_hit(
            builder,
            &g,
            &slot,
            top_off,
            gi_slot,
            resolved_slot,
            &item_next,
        )?;
        builder.emit(abi::label(&miss));
        // A miss: build it if it is a kind geoBuild builds.
        let list = builder.temporary_vreg();
        let j = builder.temporary_vreg();
        builder.emit(abi::load_u64(&list, abi::stack_pointer(), list_slot));
        builder.emit(abi::load_u64(&j, abi::stack_pointer(), j_slot));
        let view = emit_list_view(builder, &list, &item_list_type)?;
        let item = builder.temporary_vreg();
        emit_list_element(builder, &item, &view, &j);
        builder.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        let n = builder.temporary_vreg();
        emit_record_size(builder, &item, &n)?;
        builder.emit(abi::compare_immediate(&n, "0"));
        builder.emit(abi::branch_eq(&item_next));
        builder.emit(abi::store_u64(&n, abi::stack_pointer(), n_slot));
        let item = builder.temporary_vreg();
        builder.emit(abi::load_u64(&item, abi::stack_pointer(), item_slot));
        let key = builder.temporary_vreg();
        emit_item_hash(builder, &item, &key)?;
        builder.emit(abi::store_u64(&key, abi::stack_pointer(), key_slot));
        // A stale published hash can still name geometry cached under the item's own.
        let build = builder.label("canvas_resolve_build");
        let hashes = builder.temporary_vreg();
        let gi = builder.temporary_vreg();
        builder.emit(abi::load_u64(&hashes, abi::stack_pointer(), hashes_slot));
        builder.emit(abi::load_u64(&gi, abi::stack_pointer(), gi_slot));
        let published = load_word(builder, &hashes, &gi, 0);
        builder.emit(abi::compare_registers(&published, &key));
        builder.emit(abi::branch_eq(&build));
        let own = builder.temporary_vreg();
        emit_find(builder, &g, &key, &own);
        builder.emit(abi::compare_immediate(&own, "0"));
        builder.emit(abi::branch_lt(&build));
        emit_hit(
            builder,
            &g,
            &own,
            top_off,
            gi_slot,
            resolved_slot,
            &item_next,
        )?;
        builder.emit(abi::label(&build));
        let n = builder.temporary_vreg();
        builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        emit_reserve(builder, g.data, &n, &float_list())?;
        let data = gload(builder, g.data);
        let off = builder.temporary_vreg();
        builder.emit(abi::load_u64(&off, &data, COLLECTION_OFFSET_COUNT));
        builder.emit(abi::store_u64(&off, abi::stack_pointer(), off_slot));
        let out = word_addr(builder, &data, &off);
        let item = builder.temporary_vreg();
        builder.emit(abi::load_u64(&item, abi::stack_pointer(), item_slot));
        emit_fill_record(builder, &item, &out)?;
        let n = builder.temporary_vreg();
        let off = builder.temporary_vreg();
        builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        builder.emit(abi::load_u64(&off, abi::stack_pointer(), off_slot));
        let end = builder.temporary_vreg();
        builder.emit(abi::add_registers(&end, &off, &n));
        emit_set_count(builder, g.data, &end);
        let key = builder.temporary_vreg();
        builder.emit(abi::load_u64(&key, abi::stack_pointer(), key_slot));
        let new_slot = emit_add_slot(builder, &g, &key, &off, &n)?;
        let new_slot_slot = builder.spill_to_slot("canvas_resolve_new_slot", &new_slot);
        let one = imm(builder, 1);
        gadd(builder, g.native_builds, &one);
        let two = imm(builder, 2);
        emit_reserve(builder, g.built, &two, &int_list())?;
        let gi = builder.temporary_vreg();
        builder.emit(abi::load_u64(&gi, abi::stack_pointer(), gi_slot));
        emit_push(builder, g.built, &gi);
        let s = builder.temporary_vreg();
        builder.emit(abi::load_u64(&s, abi::stack_pointer(), new_slot_slot));
        emit_push(builder, g.built, &s);
        let off = builder.temporary_vreg();
        builder.emit(abi::load_u64(&off, abi::stack_pointer(), off_slot));
        emit_resolved(builder, top_off, gi_slot, resolved_slot, &off);
    }
    builder.emit(abi::label(&item_next));
    {
        for slot in [gi_slot, j_slot] {
            let v = builder.temporary_vreg();
            builder.emit(abi::load_u64(&v, abi::stack_pointer(), slot));
            builder.emit(abi::add_immediate(&v, &v, 1));
            builder.emit(abi::store_u64(&v, abi::stack_pointer(), slot));
        }
        builder.emit(abi::branch(&item_head));
    }
    builder.emit(abi::label(&all_done));
    let hcount = builder.temporary_vreg();
    let resolved = builder.temporary_vreg();
    builder.emit(abi::load_u64(&hcount, abi::stack_pointer(), hcount_slot));
    builder.emit(abi::load_u64(
        &resolved,
        abi::stack_pointer(),
        resolved_slot,
    ));
    builder.emit(abi::subtract_registers(&hcount, &hcount, &resolved));
    Ok(finish(builder, Some(&hcount), "canvas.sceneResolve"))
}

/// `__CANVAS_TOP_OFFSETS[gi] = offset`, one more resolved.
fn emit_resolved(
    builder: &mut CodeBuilder,
    top_off: usize,
    gi_slot: usize,
    resolved_slot: usize,
    offset: &VirtualRegister,
) {
    let top = gload(builder, top_off);
    let gi = builder.temporary_vreg();
    builder.emit(abi::load_u64(&gi, abi::stack_pointer(), gi_slot));
    let at = word_addr(builder, &top, &gi);
    builder.emit(abi::store_u64(offset, &at, 0));
    let r = builder.temporary_vreg();
    builder.emit(abi::load_u64(&r, abi::stack_pointer(), resolved_slot));
    builder.emit(abi::add_immediate(&r, &r, 1));
    builder.emit(abi::store_u64(&r, abi::stack_pointer(), resolved_slot));
}

/// A cache hit on `slot`: a picture's record branches to `next` unresolved; anything
/// else is stamped and resolved, then branches to `next`.
fn emit_hit(
    builder: &mut CodeBuilder,
    g: &Globals,
    slot: &VirtualRegister,
    top_off: usize,
    gi_slot: usize,
    resolved_slot: usize,
    next: &str,
) -> Result<(), String> {
    let slots = gload(builder, g.slots);
    let base = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&base, slot, 2));
    let off = load_word(builder, &slots, &base, 1);
    let data = gload(builder, g.data);
    let kind = load_word(builder, &data, &off, 0);
    let picture = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &picture,
        "Integer",
        &PICTURE_KIND_BITS.to_string(),
    ));
    builder.emit(abi::compare_registers(&kind, &picture));
    builder.emit(abi::branch_eq(next));
    emit_stamp(builder, g, slot);
    emit_resolved(builder, top_off, gi_slot, resolved_slot, &off);
    builder.emit(abi::branch(next));
    Ok(())
}

/// `__canvas_hashStep(acc, 0)`, natively. The value's three pieces are all zero, so a
/// lane is `lane * multiplier` reduced modulo 2^31 - 1 twice.
fn emit_hash_step_zero(builder: &mut CodeBuilder, acc: &VirtualRegister) {
    let m = imm(builder, 2_147_483_647);
    let a = builder.temporary_vreg();
    let b = builder.temporary_vreg();
    builder.emit(abi::shift_right_immediate(&a, acc, 31));
    builder.emit(abi::and_registers(&b, acc, &m));
    let reduce = |builder: &mut CodeBuilder, lane: &VirtualRegister, mult: u64| {
        let k = builder.temporary_vreg();
        builder.emit(abi::move_immediate(&k, "Integer", &mult.to_string()));
        let x = builder.temporary_vreg();
        builder.emit(abi::multiply_registers(&x, lane, &k));
        let hi = builder.temporary_vreg();
        let r = builder.temporary_vreg();
        builder.emit(abi::and_registers(&r, &x, &m));
        builder.emit(abi::shift_right_immediate(&hi, &x, 31));
        builder.emit(abi::add_registers(&r, &r, &hi));
        builder.emit(abi::and_registers(lane, &r, &m));
        builder.emit(abi::shift_right_immediate(&hi, &r, 31));
        builder.emit(abi::add_registers(lane, lane, &hi));
        let ok = builder.label("canvas_hash_step_ok");
        builder.emit(abi::compare_registers(lane, &m));
        builder.emit(abi::branch_lt(&ok));
        builder.emit(abi::subtract_registers(lane, lane, &m));
        builder.emit(abi::label(&ok));
    };
    reduce(builder, &a, 131);
    reduce(builder, &b, 257);
    builder.emit(abi::shift_left_immediate(acc, &a, 31));
    builder.emit(abi::add_registers(acc, acc, &b));
}

/// `canvas::sceneLayout(spans, side) AS List OF Integer`: the frame's draw entries, in
/// scene order, from `__CANVAS_TOP_OFFSETS` and what MFBASIC resolved.
///
/// `spans` is `(index, start, count)` per index MFBASIC handled, ascending; its entries
/// are `side[start..start+count]` with their group offsets and draw hashes at the same
/// positions of `__CANVAS_DRAW_DX`, `__CANVAS_DRAW_DY` and `__CANVAS_DRAW_HASHES`. Every
/// other index is one entry: its offset at no group offset, whose draw hash is
/// `__canvas_hashFloat(__canvas_hashFloat(hash, 0.0), 0.0)` of its published hash —
/// exactly what `__canvas_appendDraw` records for it. The three `__CANVAS_DRAW_*`
/// globals are replaced by the whole frame's lists, and the offsets are the answer.
fn lower_scene_layout(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let top_off = global(builder, "CANVAS_TOP_OFFSETS")?;
    let dx_off = global(builder, "CANVAS_DRAW_DX")?;
    let dy_off = global(builder, "CANVAS_DRAW_DY")?;
    let dh_off = global(builder, "CANVAS_DRAW_HASHES")?;
    let spans_slot = builder.spill_to_slot("canvas_layout_spans", &args[0].location);
    let side_slot = builder.spill_to_slot("canvas_layout_side", &args[1].location);

    // total = tcount + sum(count - 1) over the spans.
    let top = gload(builder, top_off);
    let total = builder.temporary_vreg();
    builder.emit(abi::load_u64(&total, &top, COLLECTION_OFFSET_COUNT));
    let spans = builder.temporary_vreg();
    builder.emit(abi::load_u64(&spans, abi::stack_pointer(), spans_slot));
    let nspan_words = builder.temporary_vreg();
    builder.emit(abi::load_u64(&nspan_words, &spans, COLLECTION_OFFSET_COUNT));
    let p = imm(builder, 0);
    let sum_head = builder.label("canvas_layout_sum");
    let sum_done = builder.label("canvas_layout_summed");
    builder.emit(abi::label(&sum_head));
    builder.emit(abi::compare_registers(&p, &nspan_words));
    builder.emit(abi::branch_ge(&sum_done));
    let c = load_word(builder, &spans, &p, 2);
    builder.emit(abi::add_registers(&total, &total, &c));
    builder.emit(abi::subtract_immediate(&total, &total, 1));
    builder.emit(abi::add_immediate(&p, &p, 3));
    builder.emit(abi::branch(&sum_head));
    builder.emit(abi::label(&sum_done));
    let total_slot = builder.spill_to_slot("canvas_layout_total", &total);

    let out_slot = alloc_words(builder, &total, &total, &int_list())?;
    let t = builder.temporary_vreg();
    builder.emit(abi::load_u64(&t, abi::stack_pointer(), total_slot));
    let odx_slot = alloc_words(builder, &t, &t, &float_list())?;
    let t = builder.temporary_vreg();
    builder.emit(abi::load_u64(&t, abi::stack_pointer(), total_slot));
    let ody_slot = alloc_words(builder, &t, &t, &float_list())?;
    let t = builder.temporary_vreg();
    builder.emit(abi::load_u64(&t, abi::stack_pointer(), total_slot));
    let odh_slot = alloc_words(builder, &t, &t, &int_list())?;

    // The walk. Nothing below calls anything until the frees.
    let top = gload(builder, top_off);
    let tcount = builder.temporary_vreg();
    builder.emit(abi::load_u64(&tcount, &top, COLLECTION_OFFSET_COUNT));
    let scene = scene_base(builder);
    let hashes = builder.temporary_vreg();
    builder.emit(abi::load_u64(&hashes, &scene, CANVAS_SCENE_HASHES_OFFSET));
    let hcount = imm(builder, 0);
    let no_hashes = builder.label("canvas_layout_no_hashes");
    builder.emit(abi::compare_immediate(&hashes, "0"));
    builder.emit(abi::branch_eq(&no_hashes));
    builder.emit(abi::load_u64(&hcount, &hashes, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::label(&no_hashes));
    let spans = builder.temporary_vreg();
    let side = builder.temporary_vreg();
    builder.emit(abi::load_u64(&spans, abi::stack_pointer(), spans_slot));
    builder.emit(abi::load_u64(&side, abi::stack_pointer(), side_slot));
    let sdx = gload(builder, dx_off);
    let sdy = gload(builder, dy_off);
    let sdh = gload(builder, dh_off);
    let out = builder.temporary_vreg();
    let odx = builder.temporary_vreg();
    let ody = builder.temporary_vreg();
    let odh = builder.temporary_vreg();
    builder.emit(abi::load_u64(&out, abi::stack_pointer(), out_slot));
    builder.emit(abi::load_u64(&odx, abi::stack_pointer(), odx_slot));
    builder.emit(abi::load_u64(&ody, abi::stack_pointer(), ody_slot));
    builder.emit(abi::load_u64(&odh, abi::stack_pointer(), odh_slot));
    let nspan_words = builder.temporary_vreg();
    builder.emit(abi::load_u64(&nspan_words, &spans, COLLECTION_OFFSET_COUNT));
    let i = imm(builder, 0);
    let p = imm(builder, 0);
    let e = imm(builder, 0);
    let zero = imm(builder, 0);
    let head = builder.label("canvas_layout_walk");
    let own = builder.label("canvas_layout_own");
    let next = builder.label("canvas_layout_next");
    let done = builder.label("canvas_layout_done");
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, &tcount));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::compare_registers(&p, &nspan_words));
    builder.emit(abi::branch_ge(&own));
    let span_index = load_word(builder, &spans, &p, 0);
    builder.emit(abi::compare_registers(&span_index, &i));
    builder.emit(abi::branch_ne(&own));
    // MFBASIC's entries for this index.
    let start = load_word(builder, &spans, &p, 1);
    let count = load_word(builder, &spans, &p, 2);
    let k = imm(builder, 0);
    let copy = builder.label("canvas_layout_copy");
    let copied = builder.label("canvas_layout_copied");
    builder.emit(abi::label(&copy));
    builder.emit(abi::compare_registers(&k, &count));
    builder.emit(abi::branch_ge(&copied));
    let src = builder.temporary_vreg();
    builder.emit(abi::add_registers(&src, &start, &k));
    for (from, to) in [(&side, &out), (&sdx, &odx), (&sdy, &ody), (&sdh, &odh)] {
        let v = load_word(builder, from, &src, 0);
        let at = word_addr(builder, to, &e);
        builder.emit(abi::store_u64(&v, &at, 0));
    }
    builder.emit(abi::add_immediate(&e, &e, 1));
    builder.emit(abi::add_immediate(&k, &k, 1));
    builder.emit(abi::branch(&copy));
    builder.emit(abi::label(&copied));
    builder.emit(abi::add_immediate(&p, &p, 3));
    builder.emit(abi::branch(&next));
    // One entry of its own.
    builder.emit(abi::label(&own));
    let offset = load_word(builder, &top, &i, 0);
    let at = word_addr(builder, &out, &e);
    builder.emit(abi::store_u64(&offset, &at, 0));
    for list in [&odx, &ody] {
        let at = word_addr(builder, list, &e);
        builder.emit(abi::store_u64(&zero, &at, 0));
    }
    let h = imm(builder, 0);
    let fold = builder.label("canvas_layout_fold");
    builder.emit(abi::compare_registers(&i, &hcount));
    builder.emit(abi::branch_ge(&fold));
    let published = load_word(builder, &hashes, &i, 0);
    builder.emit(abi::move_register(&h, &published));
    builder.emit(abi::label(&fold));
    for _ in 0..4 {
        emit_hash_step_zero(builder, &h);
    }
    let at = word_addr(builder, &odh, &e);
    builder.emit(abi::store_u64(&h, &at, 0));
    builder.emit(abi::add_immediate(&e, &e, 1));
    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));

    // Replace the three globals.
    for (goff, slot, ty) in [
        (dx_off, odx_slot, float_list()),
        (dy_off, ody_slot, float_list()),
        (dh_off, odh_slot, int_list()),
    ] {
        let old = gload(builder, goff);
        let old_slot = builder.spill_to_slot("canvas_layout_old", &old);
        free_block(builder, old_slot, &ty)?;
        let fresh = builder.temporary_vreg();
        builder.emit(abi::load_u64(&fresh, abi::stack_pointer(), slot));
        gstore(builder, &fresh, goff);
    }
    let result = builder.temporary_vreg();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), out_slot));
    Ok(finish(builder, Some(&result), "canvas.sceneLayout"))
}

/// `toInt(__CANVAS_GEO_DATA[off + slot])` — the float truncated toward zero.
fn geo_int(
    builder: &mut CodeBuilder,
    data: &VirtualRegister,
    off: &VirtualRegister,
    slot: usize,
) -> VirtualRegister {
    let at = word_addr(builder, data, off);
    let f = builder.temporary_fp_vreg();
    builder.emit(abi::load_double(&f, &at, slot * 8));
    let v = builder.temporary_vreg();
    builder.emit(abi::float_convert_to_signed_x(&v, &f));
    v
}

/// `n` = `__canvas_blockInstances(off)`: a text run's glyph count; 2 for a blended
/// item that both strokes (`strokeHalf > 0.0`) and fills (alpha > 0); else 1.
fn emit_block_instances(
    builder: &mut CodeBuilder,
    data: &VirtualRegister,
    off: &VirtualRegister,
    n: &VirtualRegister,
) {
    let done = builder.label("canvas_draws_inst_done");
    let one = builder.label("canvas_draws_inst_one");
    let not_text = builder.label("canvas_draws_inst_shape");
    let kind = geo_int(builder, data, off, 0);
    builder.emit(abi::compare_immediate(&kind, "6"));
    builder.emit(abi::branch_ne(&not_text));
    let glyphs = geo_int(builder, data, off, 20);
    builder.emit(abi::move_register(n, &glyphs));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&not_text));
    let blend = geo_int(builder, data, off, 26);
    builder.emit(abi::compare_immediate(&blend, "0"));
    builder.emit(abi::branch_eq(&one));
    let at = word_addr(builder, data, off);
    let half = builder.temporary_fp_vreg();
    builder.emit(abi::load_double(&half, &at, 7 * 8));
    let zero = builder.temporary_fp_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit_f64_const(&zero, &scratch, 0.0);
    let strokes = builder.label("canvas_draws_inst_strokes");
    builder.emit(abi::float_compare_d(&half, &zero));
    builder.emit(abi::branch_gt(&strokes));
    builder.emit(abi::branch(&one));
    builder.emit(abi::label(&strokes));
    let alpha = geo_int(builder, data, off, 11);
    builder.emit(abi::compare_immediate(&alpha, "0"));
    builder.emit(abi::branch_le(&one));
    builder.emit(abi::move_immediate(n, "Integer", "2"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&one));
    builder.emit(abi::move_immediate(n, "Integer", "1"));
    builder.emit(abi::label(&done));
}

/// `canvas::sceneDrawsFlat() AS Boolean`: `__canvas_sceneDraws` for a frame with no
/// group node, natively — FALSE, having touched nothing, when `__CANVAS_TOP_OFFSETS`
/// holds a group (-1) and the MFBASIC walk must lay the frame out.
///
/// Without a group the frame is one run of blocks at no offset, so the MFBASIC walk
/// reduces to: every offset into `__CANVAS_DRAW_BLOCKS`, the running instance count
/// into `__CANVAS_DRAW_INST` (`__canvas_blockInstances`), and one draw entry per maximal
/// stretch `__canvas_drawsJoin` joins — split where the blend mode changes or at a
/// `Text` block — each eight words `(instBase, instCount, 0, 0, mode, 0, 0, 0)`, skipped
/// when it publishes no instance (`__canvas_pushOneDraw`). The memo lists are emptied,
/// as the walk empties them.
fn lower_scene_draws_flat(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let top_off = global(builder, "CANVAS_TOP_OFFSETS")?;
    let data_off = global(builder, "CANVAS_GEO_DATA")?;
    let draws_off = global(builder, "CANVAS_DRAWS")?;
    let blocks_off = global(builder, "CANVAS_DRAW_BLOCKS")?;
    let inst_off = global(builder, "CANVAS_DRAW_INST")?;
    let next_off = global(builder, "CANVAS_DRAW_NEXT_INST")?;
    let memo = [
        global(builder, "CANVAS_DRAW_MEMO_SLOT")?,
        global(builder, "CANVAS_DRAW_MEMO_BASE")?,
        global(builder, "CANVAS_DRAW_MEMO_COUNT")?,
    ];
    let result = builder.temporary_vreg();
    let done = builder.label("canvas_draws_done");

    // Any group node sends the frame to the MFBASIC walk.
    let top = gload(builder, top_off);
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, &top, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::move_immediate(&result, "Boolean", "0"));
    let i = imm(builder, 0);
    let scan = builder.label("canvas_draws_scan");
    let flat = builder.label("canvas_draws_flat");
    builder.emit(abi::label(&scan));
    builder.emit(abi::compare_registers(&i, &n));
    builder.emit(abi::branch_ge(&flat));
    let off = load_word(builder, &top, &i, 0);
    builder.emit(abi::compare_immediate(&off, "0"));
    builder.emit(abi::branch_lt(&done));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&scan));
    builder.emit(abi::label(&flat));
    let n_slot = builder.spill_to_slot("canvas_draws_n", &n);

    let zero = imm(builder, 0);
    for goff in [draws_off, blocks_off, inst_off, memo[0], memo[1], memo[2]] {
        emit_set_count(builder, goff, &zero);
    }
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    emit_reserve(builder, blocks_off, &n, &int_list())?;
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    emit_reserve(builder, inst_off, &n, &int_list())?;
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    let words = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&words, &n, 3));
    emit_reserve(builder, draws_off, &words, &int_list())?;

    // Nothing below allocates.
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    let top = gload(builder, top_off);
    let data = gload(builder, data_off);
    let blocks = gload(builder, blocks_off);
    let inst = gload(builder, inst_off);
    let draws = gload(builder, draws_off);
    let next = imm(builder, 0);
    let i = imm(builder, 0);
    let fill = builder.label("canvas_draws_blocks");
    let filled = builder.label("canvas_draws_blocks_done");
    builder.emit(abi::label(&fill));
    builder.emit(abi::compare_registers(&i, &n));
    builder.emit(abi::branch_ge(&filled));
    let off = load_word(builder, &top, &i, 0);
    let at = word_addr(builder, &blocks, &i);
    builder.emit(abi::store_u64(&off, &at, 0));
    let at = word_addr(builder, &inst, &i);
    builder.emit(abi::store_u64(&next, &at, 0));
    let count = builder.temporary_vreg();
    emit_block_instances(builder, &data, &off, &count);
    builder.emit(abi::add_registers(&next, &next, &count));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&fill));
    builder.emit(abi::label(&filled));
    gstore(builder, &next, next_off);
    emit_set_count(builder, blocks_off, &n);
    emit_set_count(builder, inst_off, &n);

    // The runs: i from 1 to n inclusive, a run ending at i when i = n or the two blocks
    // do not join.
    let d = imm(builder, 0);
    let run_start = imm(builder, 0);
    let runs = builder.label("canvas_draws_runs");
    let runs_done = builder.label("canvas_draws_runs_done");
    let end_run = builder.label("canvas_draws_end_run");
    let continue_run = builder.label("canvas_draws_continue");
    builder.emit(abi::compare_immediate(&n, "0"));
    builder.emit(abi::branch_eq(&runs_done));
    builder.emit(abi::move_immediate(&i, "Integer", "1"));
    builder.emit(abi::label(&runs));
    builder.emit(abi::compare_registers(&i, &n));
    builder.emit(abi::branch_gt(&runs_done));
    builder.emit(abi::branch_eq(&end_run));
    {
        let prev_index = builder.temporary_vreg();
        builder.emit(abi::subtract_immediate(&prev_index, &i, 1));
        let a = load_word(builder, &blocks, &prev_index, 0);
        let b = load_word(builder, &blocks, &i, 0);
        let ka = geo_int(builder, &data, &a, 0);
        builder.emit(abi::compare_immediate(&ka, "6"));
        builder.emit(abi::branch_eq(&end_run));
        let kb = geo_int(builder, &data, &b, 0);
        builder.emit(abi::compare_immediate(&kb, "6"));
        builder.emit(abi::branch_eq(&end_run));
        let ma = geo_int(builder, &data, &a, 26);
        let mb = geo_int(builder, &data, &b, 26);
        builder.emit(abi::compare_registers(&ma, &mb));
        builder.emit(abi::branch_eq(&continue_run));
    }
    builder.emit(abi::label(&end_run));
    {
        // instBase = inst[runStart]; instEnd = inst[i] (or the total at i = n).
        let base = load_word(builder, &inst, &run_start, 0);
        let end = builder.temporary_vreg();
        let at_end = builder.label("canvas_draws_at_end");
        let have_end = builder.label("canvas_draws_have_end");
        builder.emit(abi::compare_registers(&i, &n));
        builder.emit(abi::branch_ge(&at_end));
        let e = load_word(builder, &inst, &i, 0);
        builder.emit(abi::move_register(&end, &e));
        builder.emit(abi::branch(&have_end));
        builder.emit(abi::label(&at_end));
        builder.emit(abi::move_register(&end, &next));
        builder.emit(abi::label(&have_end));
        let count = builder.temporary_vreg();
        builder.emit(abi::subtract_registers(&count, &end, &base));
        let skip = builder.label("canvas_draws_empty_run");
        builder.emit(abi::compare_immediate(&count, "0"));
        builder.emit(abi::branch_le(&skip));
        let first = load_word(builder, &blocks, &run_start, 0);
        let mode = geo_int(builder, &data, &first, 26);
        let zero = imm(builder, 0);
        for (k, value) in [&base, &count, &zero, &zero, &mode, &zero, &zero, &zero]
            .iter()
            .enumerate()
        {
            let at = word_addr(builder, &draws, &d);
            builder.emit(abi::store_u64(*value, &at, k * 8));
        }
        builder.emit(abi::add_immediate(&d, &d, 8));
        builder.emit(abi::label(&skip));
        builder.emit(abi::move_register(&run_start, &i));
    }
    builder.emit(abi::label(&continue_run));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&runs));
    builder.emit(abi::label(&runs_done));
    emit_set_count(builder, draws_off, &d);
    builder.emit(abi::move_immediate(&result, "Boolean", "1"));
    builder.emit(abi::label(&done));
    let out = builder.allocate_register();
    builder.emit(abi::move_register(&out, &result));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from(out.render()),
        text: "canvas.sceneDrawsFlat".to_string(),
    })
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
            param("header", float_list()),
            param("tail", float_list()),
        ],
        ParameterType::Integer,
        vec!["ErrOutOfMemory"],
        lower_geo_insert,
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
