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
/// `__CANVAS_GEO_DATA`, with `draw` (the damage identity its record holds) beside it in
/// `__CANVAS_GEO_DRAWS`, count it as used and generated, and index it.
pub(super) fn emit_add_slot(
    builder: &mut CodeBuilder,
    g: &Globals,
    hash: &VirtualRegister,
    draw: &VirtualRegister,
    offset: &VirtualRegister,
    count: &VirtualRegister,
) -> Result<VirtualRegister, String> {
    let hash_slot = builder.spill_to_slot("canvas_geo_add_hash", hash);
    let draw_slot = builder.spill_to_slot("canvas_geo_add_draw", draw);
    let offset_slot = builder.spill_to_slot("canvas_geo_add_offset", offset);
    let count_slot = builder.spill_to_slot("canvas_geo_add_count", count);
    let one = imm(builder, 1);
    emit_reserve(builder, g.draws, &one, &int_list())?;
    let d = builder.temporary_vreg();
    builder.emit(abi::load_u64(&d, abi::stack_pointer(), draw_slot));
    emit_push(builder, g.draws, &d);
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

/// `canvas::geoInsert(hash, draw, header, tail) AS Integer`: append `header` then `tail`
/// to `__CANVAS_GEO_DATA` as a new slot used by this frame, holding the damage identity
/// `draw`, index it under `hash`, and answer its offset. Counts one generation.
pub(super) fn lower_geo_insert(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let g = Globals::of(builder)?;
    let hash_slot = builder.spill_to_slot("canvas_geo_ins_hash", &args[0].location);
    let draw_slot = builder.spill_to_slot("canvas_geo_ins_draw", &args[1].location);
    let header_slot = builder.spill_to_slot("canvas_geo_ins_header", &args[2].location);
    let tail_slot = builder.spill_to_slot("canvas_geo_ins_tail", &args[3].location);
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
    let draw = builder.temporary_vreg();
    builder.emit(abi::load_u64(&hash, abi::stack_pointer(), hash_slot));
    builder.emit(abi::load_u64(&draw, abi::stack_pointer(), draw_slot));
    emit_add_slot(builder, &g, &hash, &draw, &offset, &n)?;
    let offset = builder.temporary_vreg();
    builder.emit(abi::load_u64(&offset, abi::stack_pointer(), offset_slot));
    Ok(finish(builder, Some(&offset), "canvas.geoInsert"))
}

/// `canvas::geoRefresh(slot, draw, header, tail) AS Integer` (plan-175-A): rewrite
/// `slot`'s record IN PLACE with `header` then `tail` — the record a fresh build of the
/// item makes now — mark it as holding `draw`, stamp it, and answer its offset. No
/// allocation, no slot, no generation: `__CANVAS_GEO_REFRESHES` counts it.
///
/// The caller found `slot` by the item's geometry identity, so the fresh record has the
/// slot's length by construction (`func_item_lanes.rs`: everything that sizes a record
/// is geometry). It is checked anyway, because a record written past its slot would
/// overwrite the next one: a length that differs takes the insert instead, under the
/// slot's own key, which the index then maps to the new slot.
pub(super) fn lower_geo_refresh(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let g = Globals::of(builder)?;
    let slot_slot = builder.spill_to_slot("canvas_geo_ref_slot", &args[0].location);
    let draw_slot = builder.spill_to_slot("canvas_geo_ref_draw", &args[1].location);
    let header_slot = builder.spill_to_slot("canvas_geo_ref_header", &args[2].location);
    let tail_slot = builder.spill_to_slot("canvas_geo_ref_tail", &args[3].location);
    let offset_slot = builder.allocate_stack_object("canvas_geo_ref_offset", 8);
    let header = builder.temporary_vreg();
    let tail = builder.temporary_vreg();
    builder.emit(abi::load_u64(&header, abi::stack_pointer(), header_slot));
    builder.emit(abi::load_u64(&tail, abi::stack_pointer(), tail_slot));
    let n = builder.temporary_vreg();
    let nt = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, &header, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::load_u64(&nt, &tail, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::add_registers(&n, &n, &nt));
    let slot = builder.temporary_vreg();
    builder.emit(abi::load_u64(&slot, abi::stack_pointer(), slot_slot));
    let slots = gload(builder, g.slots);
    let base = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&base, &slot, 2));
    let count = load_word(builder, &slots, &base, 2);
    let offset = load_word(builder, &slots, &base, 1);
    builder.emit(abi::store_u64(&offset, abi::stack_pointer(), offset_slot));
    let insert = builder.label("canvas_geo_ref_insert");
    let done = builder.label("canvas_geo_ref_done");
    builder.emit(abi::compare_registers(&n, &count));
    builder.emit(abi::branch_ne(&insert));
    let data = gload(builder, g.data);
    let dst = word_addr(builder, &data, &offset);
    for list_slot in [header_slot, tail_slot] {
        let list = builder.temporary_vreg();
        let bytes = builder.temporary_vreg();
        let scratch = builder.temporary_vreg();
        builder.emit(abi::load_u64(&list, abi::stack_pointer(), list_slot));
        builder.emit(abi::load_u64(&bytes, &list, COLLECTION_OFFSET_COUNT));
        builder.emit(abi::shift_left_immediate(&bytes, &bytes, 3));
        builder.emit(abi::add_immediate(&list, &list, COLLECTION_HEADER_SIZE));
        builder.emit_block_copy_advance(&dst, &list, &bytes, &scratch, "canvas_geo_ref_copy");
    }
    let slot = builder.temporary_vreg();
    let draw = builder.temporary_vreg();
    builder.emit(abi::load_u64(&slot, abi::stack_pointer(), slot_slot));
    builder.emit(abi::load_u64(&draw, abi::stack_pointer(), draw_slot));
    emit_refreshed(builder, &g, &slot, &draw);
    builder.emit(abi::branch(&done));
    // A length the slot does not have: a new slot under the slot's own key.
    builder.emit(abi::label(&insert));
    {
        let n = builder.temporary_vreg();
        let nt = builder.temporary_vreg();
        let header = builder.temporary_vreg();
        let tail = builder.temporary_vreg();
        builder.emit(abi::load_u64(&header, abi::stack_pointer(), header_slot));
        builder.emit(abi::load_u64(&tail, abi::stack_pointer(), tail_slot));
        builder.emit(abi::load_u64(&n, &header, COLLECTION_OFFSET_COUNT));
        builder.emit(abi::load_u64(&nt, &tail, COLLECTION_OFFSET_COUNT));
        builder.emit(abi::add_registers(&n, &n, &nt));
        let n_slot = builder.spill_to_slot("canvas_geo_ref_n", &n);
        emit_reserve(builder, g.data, &n, &float_list())?;
        let data = gload(builder, g.data);
        let offset = builder.temporary_vreg();
        builder.emit(abi::load_u64(&offset, &data, COLLECTION_OFFSET_COUNT));
        builder.emit(abi::store_u64(&offset, abi::stack_pointer(), offset_slot));
        let dst = word_addr(builder, &data, &offset);
        for list_slot in [header_slot, tail_slot] {
            let list = builder.temporary_vreg();
            let bytes = builder.temporary_vreg();
            let scratch = builder.temporary_vreg();
            builder.emit(abi::load_u64(&list, abi::stack_pointer(), list_slot));
            builder.emit(abi::load_u64(&bytes, &list, COLLECTION_OFFSET_COUNT));
            builder.emit(abi::shift_left_immediate(&bytes, &bytes, 3));
            builder.emit(abi::add_immediate(&list, &list, COLLECTION_HEADER_SIZE));
            builder.emit_block_copy_advance(&dst, &list, &bytes, &scratch, "canvas_geo_ref_ins_copy");
        }
        let n = builder.temporary_vreg();
        let offset = builder.temporary_vreg();
        builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        builder.emit(abi::load_u64(&offset, abi::stack_pointer(), offset_slot));
        let total = builder.temporary_vreg();
        builder.emit(abi::add_registers(&total, &offset, &n));
        emit_set_count(builder, g.data, &total);
        let slot = builder.temporary_vreg();
        builder.emit(abi::load_u64(&slot, abi::stack_pointer(), slot_slot));
        let slots = gload(builder, g.slots);
        let base = builder.temporary_vreg();
        builder.emit(abi::shift_left_immediate(&base, &slot, 2));
        let key = load_word(builder, &slots, &base, 0);
        let draw = builder.temporary_vreg();
        builder.emit(abi::load_u64(&draw, abi::stack_pointer(), draw_slot));
        let offset = builder.temporary_vreg();
        builder.emit(abi::load_u64(&offset, abi::stack_pointer(), offset_slot));
        let n = builder.temporary_vreg();
        builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
        emit_add_slot(builder, &g, &key, &draw, &offset, &n)?;
    }
    builder.emit(abi::label(&done));
    let offset = builder.temporary_vreg();
    builder.emit(abi::load_u64(&offset, abi::stack_pointer(), offset_slot));
    Ok(finish(builder, Some(&offset), "canvas.geoRefresh"))
}

/// The header slots `__canvas_paintHeader` writes from appearance fields
/// (`func_item_lanes.rs`): both colours (8–15), the blend mode (26), the gradient's kind
/// and points — or a text record's layout box, which its builder writes in their place
/// (42–46) — and the ramp's spread, apply, inverse and focal words (55–66).
const APPEARANCE_SLOTS: [std::ops::Range<usize>; 4] = [8..16, 26..27, 42..47, 55..67];

/// `canvas::geoRepaint(slot, draw, header, stops) AS Integer` (plan-175-A Phase 3): a
/// cheaper refresh of a record the MFBASIC builders made. `header` is that kind's header
/// built afresh from the item (its header builder alone, none of its tail), and
/// `stops` the gradient's stop floats as the builder appends them (empty when it has
/// none). The record takes the header's [`APPEARANCE_SLOTS`] and, when there are stops,
/// those `5 * count` floats at the end of its tail, where they always are
/// (`__canvas_gradientStopBase`); everything else it holds is geometry. Answers the
/// record's offset, or -1 — nothing written — when `stops` is not the record's stop
/// count (the caller then rebuilds it whole).
pub(super) fn lower_geo_repaint(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let g = Globals::of(builder)?;
    let slot = builder.temporary_vreg();
    let draw = builder.temporary_vreg();
    let header = builder.temporary_vreg();
    let stops = builder.temporary_vreg();
    builder.emit(abi::move_register(&slot, &args[0].location));
    builder.emit(abi::move_register(&draw, &args[1].location));
    builder.emit(abi::move_register(&header, &args[2].location));
    builder.emit(abi::move_register(&stops, &args[3].location));
    let result = imm(builder, -1);
    let done = builder.label("canvas_geo_repaint_done");
    let slots = gload(builder, g.slots);
    let base = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&base, &slot, 2));
    let offset = load_word(builder, &slots, &base, 1);
    let data = gload(builder, g.data);
    let record = word_addr(builder, &data, &offset);
    // The stops: as many as the record holds, or nothing is written.
    let floats = builder.temporary_vreg();
    builder.emit(abi::load_u64(&floats, &stops, COLLECTION_OFFSET_COUNT));
    let count_f = builder.temporary_fp_vreg();
    builder.emit(abi::load_double(
        &count_f,
        &record,
        8 * 41, // __CANVAS_GEO_GRADIENT_COUNT
    ));
    let count = builder.temporary_vreg();
    builder.emit(abi::float_convert_to_signed_x(&count, &count_f));
    let want = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&want, "Integer", "5"));
    builder.emit(abi::multiply_registers(&want, &want, &count));
    builder.emit(abi::compare_registers(&want, &floats));
    builder.emit(abi::branch_ne(&done));
    // The header's appearance slots over the record's.
    let src = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&src, &header, COLLECTION_HEADER_SIZE));
    let v = builder.temporary_vreg();
    for range in APPEARANCE_SLOTS {
        for s in range {
            builder.emit(abi::load_u64(&v, &src, 8 * s));
            builder.emit(abi::store_u64(&v, &record, 8 * s));
        }
    }
    // The stops, at the end of the record: `length - 5 * count`.
    let length_f = builder.temporary_fp_vreg();
    builder.emit(abi::load_double(&length_f, &record, 8));
    let length = builder.temporary_vreg();
    builder.emit(abi::float_convert_to_signed_x(&length, &length_f));
    builder.emit(abi::subtract_registers(&length, &length, &want));
    builder.emit(abi::shift_left_immediate(&length, &length, 3));
    let dst = builder.temporary_vreg();
    builder.emit(abi::add_registers(&dst, &record, &length));
    let from = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&from, &stops, COLLECTION_HEADER_SIZE));
    let bytes = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&bytes, &floats, 3));
    let scratch = builder.temporary_vreg();
    builder.emit_block_copy_advance(&dst, &from, &bytes, &scratch, "canvas_geo_repaint_stops");
    emit_refreshed(builder, &g, &slot, &draw);
    builder.emit(abi::move_register(&result, &offset));
    builder.emit(abi::label(&done));
    Ok(finish(builder, Some(&result), "canvas.geoRepaint"))
}

/// A record just rewritten in place: `slot` holds `draw`, is stamped as used, and one
/// more refresh is counted.
pub(super) fn emit_refreshed(
    builder: &mut CodeBuilder,
    g: &Globals,
    slot: &VirtualRegister,
    draw: &VirtualRegister,
) {
    let draws = gload(builder, g.draws);
    let at = word_addr(builder, &draws, slot);
    builder.emit(abi::store_u64(draw, &at, 0));
    emit_stamp(builder, g, slot);
    let one = imm(builder, 1);
    gadd(builder, g.refreshes, &one);
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
    // plan-175-A: the slot's draw identity moves with it (`k <= s`, so forward is safe).
    let draws = gload(builder, g.draws);
    let draw = load_word(builder, &draws, &s, 0);
    let draw_at = word_addr(builder, &draws, &k);
    builder.emit(abi::store_u64(&draw, &draw_at, 0));
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
    emit_set_count(builder, g.draws, &k);
    emit_rebuild(builder, &g)?;

    builder.emit(abi::label(&advance));
    let frame = gload(builder, g.frame);
    builder.emit(abi::add_immediate(&frame, &frame, 1));
    gstore(builder, &frame, g.frame);
    Ok(finish(builder, None, "canvas.geoBeginFrame"))
}
