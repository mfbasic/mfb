//! plan-176-R: the native geometry cache's hash table — buckets, find/put/rebuild, stamps and
//! the geo find/insert/forget/begin-frame lowerings. The globals, word helpers and registration
//! live in `builtins/canvas/func_geo_cache/mod.rs`.

use super::*;

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
pub(super) fn emit_find(
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
pub(super) fn emit_stamp(builder: &mut CodeBuilder, g: &Globals, slot: &VirtualRegister) {
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
pub(super) fn emit_add_slot(
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
pub(super) fn finish(
    builder: &mut CodeBuilder,
    value: Option<&VirtualRegister>,
    text: &str,
) -> ValueResult {
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
        location: Operand::from(result),
        text: text.to_string(),
    }
}

/// `canvas::geoFind(hash) AS Integer`: the slot `hash` is cached in, or -1.
pub(super) fn lower_geo_find(
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
pub(super) fn lower_geo_insert(
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
pub(super) fn lower_geo_forget(
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
pub(super) fn lower_geo_begin_frame(
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
