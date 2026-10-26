//! plan-176-R: the native geometry cache's scene layout and flat draws (`lower_scene_layout`,
//! `lower_scene_draws_flat`). The globals, word helpers and registration live in
//! `builtins/canvas/func_geo_cache/mod.rs`.

use super::*;

/// `canvas::sceneLayout(spans, side) AS List OF Integer`: the frame's draw entries, in
/// scene order, from `__CANVAS_TOP_OFFSETS` and what MFBASIC resolved.
///
/// `spans` is `(index, start, count)` per index MFBASIC handled, ascending; its entries
/// are `side[start..start+count]` with their group offsets and draw hashes at the same
/// positions of `__CANVAS_DRAW_DX`, `__CANVAS_DRAW_DY`, `__CANVAS_DRAW_MAT` (plan-172-G:
/// each entry's group matrix number) and `__CANVAS_DRAW_HASHES`. Every
/// other index is one entry: its offset at no group offset and no matrix, whose draw hash is
/// `__canvas_hashFloat(__canvas_hashFloat(hash, 0.0), 0.0)` of its published hash —
/// exactly what `__canvas_appendDraw` records for it. The four `__CANVAS_DRAW_*`
/// globals are replaced by the whole frame's lists, and the offsets are the answer.
pub(super) fn lower_scene_layout(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let top_off = global(builder, "CANVAS_TOP_OFFSETS")?;
    let dx_off = global(builder, "CANVAS_DRAW_DX")?;
    let dy_off = global(builder, "CANVAS_DRAW_DY")?;
    let dh_off = global(builder, "CANVAS_DRAW_HASHES")?;
    let dm_off = global(builder, "CANVAS_DRAW_MAT")?;
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
    let t = builder.temporary_vreg();
    builder.emit(abi::load_u64(&t, abi::stack_pointer(), total_slot));
    let odm_slot = alloc_words(builder, &t, &t, &int_list())?;

    // The walk. Nothing below calls anything until the frees.
    let top = gload(builder, top_off);
    let tcount = builder.temporary_vreg();
    builder.emit(abi::load_u64(&tcount, &top, COLLECTION_OFFSET_COUNT));
    let hashes = gload(builder, global(builder, "CANVAS_FRAME_HASHES")?);
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
    let sdm = gload(builder, dm_off);
    let out = builder.temporary_vreg();
    let odx = builder.temporary_vreg();
    let ody = builder.temporary_vreg();
    let odh = builder.temporary_vreg();
    let odm = builder.temporary_vreg();
    builder.emit(abi::load_u64(&out, abi::stack_pointer(), out_slot));
    builder.emit(abi::load_u64(&odx, abi::stack_pointer(), odx_slot));
    builder.emit(abi::load_u64(&ody, abi::stack_pointer(), ody_slot));
    builder.emit(abi::load_u64(&odh, abi::stack_pointer(), odh_slot));
    builder.emit(abi::load_u64(&odm, abi::stack_pointer(), odm_slot));
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
    for (from, to) in [
        (&side, &out),
        (&sdx, &odx),
        (&sdy, &ody),
        (&sdh, &odh),
        (&sdm, &odm),
    ] {
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
    // No group offset and no group matrix: an item of the scene itself.
    for list in [&odx, &ody, &odm] {
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

    // Replace the four globals.
    for (goff, slot, ty) in [
        (dx_off, odx_slot, float_list()),
        (dy_off, ody_slot, float_list()),
        (dh_off, odh_slot, int_list()),
        (dm_off, odm_slot, int_list()),
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
/// item that both strokes (`strokeHalf > 0.0`) and fills (alpha > 0, a gradient or a
/// picture); else 1.
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
    // plan-163-D: it fills when the fill alpha is visible, OR it has a gradient (slot 41,
    // two or more stops), OR it is a picture -- `emit_split_or_publish`'s test, whose
    // ramp or image does not come from the fill alpha.
    let fills = builder.label("canvas_draws_inst_fills");
    let alpha = geo_int(builder, data, off, 11);
    builder.emit(abi::compare_immediate(&alpha, "0"));
    builder.emit(abi::branch_gt(&fills));
    let stops = geo_int(builder, data, off, 41);
    builder.emit(abi::compare_immediate(&stops, "2"));
    builder.emit(abi::branch_ge(&fills));
    builder.emit(abi::compare_immediate(&kind, "9"));
    builder.emit(abi::branch_ne(&one));
    builder.emit(abi::label(&fills));
    builder.emit(abi::move_immediate(n, "Integer", "2"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&one));
    builder.emit(abi::move_immediate(n, "Integer", "1"));
    builder.emit(abi::label(&done));
}

/// `visible` = `__canvas_boundsMeetViewport(off, width, height, 0.0, 0.0)` with no group
/// matrix (plan-173): 1 when the block's bounds (slots 16-19) cross the half-open
/// viewport `[0, width) x [0, height)`, else 0. Each test branches on its TRUE outcome,
/// so a NaN bound is offscreen here as it is in MFBASIC, where a comparison with NaN is
/// FALSE.
fn emit_block_visible(
    builder: &mut CodeBuilder,
    data: &VirtualRegister,
    off: &VirtualRegister,
    width: &VirtualRegister,
    height: &VirtualRegister,
) -> VirtualRegister {
    let visible = builder.temporary_vreg();
    let offscreen = builder.label("canvas_draws_offscreen");
    let done = builder.label("canvas_draws_visible_done");
    let zero_i = imm(builder, 0);
    let zero = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&zero, &zero_i));
    let at = word_addr(builder, data, off);
    // x1 > 0, y1 > 0, x0 < width, y0 < height.
    for (slot, limit, below) in [
        (18, &zero, false),
        (19, &zero, false),
        (16, width, true),
        (17, height, true),
    ] {
        let bound = builder.temporary_fp_vreg();
        builder.emit(abi::load_double(&bound, &at, slot * 8));
        let holds = builder.label("canvas_draws_visible_next");
        builder.emit(abi::float_compare_d(&bound, limit));
        builder.emit(if below {
            abi::branch_mi(&holds)
        } else {
            abi::branch_gt(&holds)
        });
        builder.emit(abi::branch(&offscreen));
        builder.emit(abi::label(&holds));
    }
    builder.emit(abi::move_immediate(&visible, "Integer", "1"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&offscreen));
    builder.emit(abi::move_immediate(&visible, "Integer", "0"));
    builder.emit(abi::label(&done));
    visible
}

/// `canvas::sceneDrawsFlat() AS Boolean`: `__canvas_sceneDraws` for a frame with no
/// group node, natively — FALSE, having touched nothing, when `__CANVAS_TOP_OFFSETS`
/// holds a group (-1) and the MFBASIC walk must lay the frame out.
///
/// Without a group the frame is one run of blocks at no offset, so the MFBASIC walk
/// reduces to: every offset into `__CANVAS_DRAW_BLOCKS`, the running instance count
/// into `__CANVAS_DRAW_INST` (`__canvas_blockInstances`), and one draw entry per maximal
/// stretch `__canvas_drawsJoin` joins — split where the blend mode changes or at a
/// `Text` block — each `CANVAS_DRAW_ENTRY_WORDS` words, `(instBase, instCount, 0, 0, mode)`
/// and zeros after (no offset, no matrix) but the repeat, 1 (plan-175-B), skipped when it publishes no instance
/// (`__canvas_pushOneDraw`). plan-173: a run also ends where a block's viewport
/// visibility differs from the one before it (`emit_block_visible`), and an offscreen run
/// publishes no entry — `__canvas_pushDraw`'s split, so the block list and instance bases
/// stay those of every block while the GPU draws only the visible ones. The memo lists
/// are emptied, as the walk empties them.
pub(super) fn lower_scene_draws_flat(
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
    let viewport_w_off = global(builder, "CANVAS_VIEWPORT_W")?;
    let viewport_h_off = global(builder, "CANVAS_VIEWPORT_H")?;
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
    builder.emit(abi::shift_left_immediate(
        &words,
        &n,
        CANVAS_DRAW_ENTRY_COUNT_SHIFT as u8,
    ));
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

    // plan-173: the viewport `__canvas_renderFrame` stored, as floats for the bounds test.
    let viewport_w = builder.temporary_fp_vreg();
    let w = gload(builder, viewport_w_off);
    builder.emit(abi::signed_convert_to_float_d(&viewport_w, &w));
    let viewport_h = builder.temporary_fp_vreg();
    let h = gload(builder, viewport_h_off);
    builder.emit(abi::signed_convert_to_float_d(&viewport_h, &h));

    // The runs: i from 1 to n inclusive, a run ending at i when i = n, the two blocks
    // do not join, or one is in the viewport and the other is not.
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
        builder.emit(abi::branch_ne(&end_run));
        let va = emit_block_visible(builder, &data, &a, &viewport_w, &viewport_h);
        let vb = emit_block_visible(builder, &data, &b, &viewport_w, &viewport_h);
        builder.emit(abi::compare_registers(&va, &vb));
        builder.emit(abi::branch_eq(&continue_run));
    }
    builder.emit(abi::label(&end_run));
    {
        // An offscreen run publishes nothing; its blocks keep their instance bases.
        let offscreen = builder.label("canvas_draws_offscreen_run");
        let first = load_word(builder, &blocks, &run_start, 0);
        let shown = emit_block_visible(builder, &data, &first, &viewport_w, &viewport_h);
        builder.emit(abi::compare_immediate(&shown, "0"));
        builder.emit(abi::branch_eq(&offscreen));
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
        let mode = geo_int(builder, &data, &first, 26);
        let zero = imm(builder, 0);
        let one = imm(builder, 1);
        // A flat frame's entry: base, count, the blend mode in word 4, the repeat 1 in
        // word 13 (plan-175-B: one draw over the row's own node), zeros elsewhere.
        let at = word_addr(builder, &draws, &d);
        for k in 0..CANVAS_DRAW_ENTRY_WORDS {
            let value = match k {
                0 => &base,
                1 => &count,
                k if k * 8 == CANVAS_DRAW_ENTRY_MODE => &mode,
                CANVAS_DRAW_REPEAT_WORD => &one,
                _ => &zero,
            };
            builder.emit(abi::store_u64(value, &at, k * 8));
        }
        builder.emit(abi::add_immediate(&d, &d, CANVAS_DRAW_ENTRY_WORDS));
        builder.emit(abi::label(&skip));
        builder.emit(abi::label(&offscreen));
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
        location: Operand::from(out),
        text: "canvas.sceneDrawsFlat".to_string(),
    })
}
