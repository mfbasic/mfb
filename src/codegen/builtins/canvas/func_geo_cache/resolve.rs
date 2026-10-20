//! plan-176-R: the native geometry cache's scene resolve (`lower_scene_resolve`). The globals,
//! word helpers and registration live in `builtins/canvas/func_geo_cache/mod.rs`.

use super::*;

/// `canvas::sceneResolve() AS Integer`: resolve every item of the frame's scene — the
/// snapshot `canvas::sceneSnapshot` took, its flat items then each layer's, keyed by
/// `__CANVAS_FRAME_HASHES` in that order — into `__CANVAS_TOP_OFFSETS`, and answer how
/// many it left at -1 for MFBASIC.
///
/// Per index, by the published hash: a hit is stamped and resolved, except a
/// `Picture`'s (its pixels can change under an unchanged item); a miss of a kind
/// `canvas::geoBuild` builds is built straight into `__CANVAS_GEO_DATA`; anything else —
/// a picture, a `Text`, a `Group`, a declined paint or kind, or an index past the items —
/// is left at -1.
///
/// A built record is indexed under the item's OWN hash (`canvas::itemHash`). Since the
/// frame's hashes are the ones published for the frame's own scene (or computed from
/// it), that is the frame hash; computing it rather than trusting it costs a hash per
/// MISS and keeps a key from ever naming another item's geometry.
///
/// `__CANVAS_GEO_BUILT` gets `(index, slot)` for each record built here, for the
/// `--debug` geometry check.
pub(super) fn lower_scene_resolve(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let g = Globals::of(builder)?;
    let top_off = global(builder, "CANVAS_TOP_OFFSETS")?;
    let item_list_type = ParameterType::list_of(ParameterType::named("DrawItem"));
    let layer_type = record_type(builder, &ParameterType::named("DrawLayer"))?;
    let layer_list_type = ParameterType::list_of(ParameterType::named("DrawLayer"));

    // The frame's hashes (`__CANVAS_FRAME_HASHES`, taken with the frame's snapshot), and
    // how many indices this frame has.
    let hashes = gload(builder, global(builder, "CANVAS_FRAME_HASHES")?);
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
        let items = gload(builder, global(builder, "CANVAS_SNAP_ITEMS")?);
        builder.emit(abi::compare_immediate(&items, "0"));
        builder.emit(abi::branch_eq(&next_segment));
        builder.emit(abi::store_u64(&items, abi::stack_pointer(), list_slot));
        builder.emit(abi::branch(&walk));
    }
    builder.emit(abi::label(&layer_segment));
    {
        let layers = gload(builder, global(builder, "CANVAS_SNAP_LAYERS")?);
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

/// A cache hit on `slot`: a picture's or a tile's record branches to `next` unresolved,
/// so the MFBASIC pass confirms its image block (`__canvas_pictureCached`); anything
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
    let tile = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &tile,
        "Integer",
        &TILE_KIND_BITS.to_string(),
    ));
    builder.emit(abi::compare_registers(&kind, &tile));
    builder.emit(abi::branch_eq(next));
    emit_stamp(builder, g, slot);
    emit_resolved(builder, top_off, gi_slot, resolved_slot, &off);
    builder.emit(abi::branch(next));
    Ok(())
}

/// `__canvas_hashStep(acc, 0)`, natively. The value's three pieces are all zero, so a
/// lane is `lane * multiplier` reduced modulo 2^31 - 1 twice.
pub(super) fn emit_hash_step_zero(builder: &mut CodeBuilder, acc: &VirtualRegister) {
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
