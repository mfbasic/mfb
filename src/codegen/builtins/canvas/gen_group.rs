//! The named-group table: `canvas::setGroup` and `canvas::removeGroup` (plan-116-G).
//!
//! Both run on the **worker**, which is the only thread that may allocate or free
//! (`.ai/canvas-threading.md` §3). The table itself is a process-global fixed array —
//! see `CANVAS_GROUPS_SYMBOL` for why it is neither arena state nor growable.
//!
//! A slot is claimed by writing everything **except** the name, then the name. A
//! graphics thread that sees a non-zero name therefore sees a slot whose `items` and
//! `count` are already there; one that sees zero skips the slot and never follows a
//! pointer that is half-written. Dropping a slot reverses it: the name goes first.

// --- codegen tier imports (migration) ---
use super::gen_present::emit_load_frame_counter;
use crate::codegen::app::hook::app::{prepend_wrong_mode_gate, ModeRequirement};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::native_arena::emit_arena_free;
use crate::codegen::memory::data::push_symbol_address;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// The group table's base address, in a fresh register.
///
/// The twin of `scene_base`; a separate function rather than a parameterised one
/// because the two symbols are reached identically and naming them apart is what makes
/// a call site readable.
pub(crate) fn groups_base(builder: &mut CodeBuilder) -> VirtualRegister {
    let base = builder.temporary_vreg();
    let symbol = builder.current_symbol.clone();
    push_symbol_address(
        &symbol,
        CANVAS_GROUPS_SYMBOL,
        &base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    base
}

/// Walk the table looking for `name`, leaving the matching slot's address in
/// `found_slot` or `0`, and the first free slot's address in `free_slot` or `0`.
///
/// One pass for both because they are the same walk: `setGroup` needs the match if
/// there is one and a free slot otherwise, and doing it twice would let the table
/// change between them. A slot matches when its name is non-zero and its bytes equal
/// `name`'s — length first, since that rejects almost every mismatch without a loop.
fn emit_slot_scan(
    builder: &mut CodeBuilder,
    name_slot: usize,
    found_slot: usize,
    free_slot: usize,
    prefix: &str,
) {
    let head = builder.label(&format!("{prefix}_scan_head"));
    let next = builder.label(&format!("{prefix}_scan_next"));
    let done = builder.label(&format!("{prefix}_scan_done"));
    let occupied = builder.label(&format!("{prefix}_scan_occupied"));
    let hit = builder.label(&format!("{prefix}_scan_hit"));

    let zero = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&zero, "Integer", "0"));
    builder.emit(abi::store_u64(&zero, abi::stack_pointer(), found_slot));
    builder.emit(abi::store_u64(&zero, abi::stack_pointer(), free_slot));

    let cursor = builder.allocate_stack_object("canvas_group_cursor", 8);
    let index = builder.allocate_stack_object("canvas_group_index", 8);
    let base = groups_base(builder);
    builder.emit(abi::store_u64(&base, abi::stack_pointer(), cursor));
    builder.emit(abi::store_u64(&zero, abi::stack_pointer(), index));

    builder.emit(abi::label(&head));
    let i = builder.temporary_vreg();
    builder.emit(abi::load_u64(&i, abi::stack_pointer(), index));
    builder.emit(abi::compare_immediate(&i, &CANVAS_MAX_GROUPS.to_string()));
    builder.emit(abi::branch_ge(&done));

    let slot = builder.temporary_vreg();
    let slot_name = builder.temporary_vreg();
    builder.emit(abi::load_u64(&slot, abi::stack_pointer(), cursor));
    builder.emit(abi::load_u64(&slot_name, &slot, CANVAS_GROUP_NAME));
    builder.emit(abi::compare_immediate(&slot_name, "0"));
    builder.emit(abi::branch_ne(&occupied));

    // Free. Remember the FIRST one only — `store` unconditionally would keep the last,
    // and reusing the lowest free slot keeps the table dense so the scan stays short.
    let seen_free = builder.temporary_vreg();
    builder.emit(abi::load_u64(&seen_free, abi::stack_pointer(), free_slot));
    builder.emit(abi::compare_immediate(&seen_free, "0"));
    builder.emit(abi::branch_ne(&next));
    builder.emit(abi::store_u64(&slot, abi::stack_pointer(), free_slot));
    builder.emit(abi::branch(&next));

    builder.emit(abi::label(&occupied));
    // Length first: two `String` blocks lead with a u64 byte length, and comparing it
    // rejects nearly every mismatch without entering the byte loop.
    let want = builder.temporary_vreg();
    let want_len = builder.temporary_vreg();
    let have_len = builder.temporary_vreg();
    builder.emit(abi::load_u64(&want, abi::stack_pointer(), name_slot));
    builder.emit(abi::load_u64(&want_len, &want, 0));
    builder.emit(abi::load_u64(&have_len, &slot_name, 0));
    builder.emit(abi::compare_registers(&want_len, &have_len));
    builder.emit(abi::branch_ne(&next));

    let want_bytes = builder.temporary_vreg();
    let have_bytes = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&want_bytes, &want, 8));
    builder.emit(abi::add_immediate(&have_bytes, &slot_name, 8));
    builder.emit_compare_bytes_branch(
        &want_bytes,
        &have_bytes,
        &want_len,
        &hit,
        &next,
        &format!("{prefix}_name"),
    );

    builder.emit(abi::label(&hit));
    builder.emit(abi::store_u64(&slot, abi::stack_pointer(), found_slot));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&next));
    let advance = builder.temporary_vreg();
    builder.emit(abi::load_u64(&advance, abi::stack_pointer(), cursor));
    builder.emit(abi::add_immediate(
        &advance,
        &advance,
        CANVAS_GROUP_SLOT_BYTES,
    ));
    builder.emit(abi::store_u64(&advance, abi::stack_pointer(), cursor));
    let bumped = builder.temporary_vreg();
    builder.emit(abi::load_u64(&bumped, abi::stack_pointer(), index));
    builder.emit(abi::add_immediate(&bumped, &bumped, 1));
    builder.emit(abi::store_u64(&bumped, abi::stack_pointer(), index));
    builder.emit(abi::branch(&head));

    builder.emit(abi::label(&done));
}

/// `canvas::setGroup(name, items)`.
pub(crate) fn emit_set_group(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let name_in = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the name argument"))?
        .location
        .clone();
    let items_in = args
        .get(1)
        .ok_or_else(|| format!("'{symbol}' expects the items argument"))?
        .location
        .clone();

    let items_type = ParameterType::list_of(ParameterType::named("DrawItem"));

    // plan-116-J will have to replace this `copy_flat_block`, not add to it. It is
    // correct today because no `DrawItem` carries a resource; plan-116-I puts a
    // `RES canvas::Image` in `Picture` and a `RES canvas::Font` in `Text`, and
    // `a_res_collection_does_not_diverge` in `builder_collection_layout.rs` pins that a
    // resource-carrying collection must stay out of `copy_flat_block` — and out of
    // `is_freeable_flat_value`, which is what `emit_free_items_block` below relies on.
    // Recorded here as well as in that plan because this is the call site, and a reader
    // arriving from I rather than from J would otherwise find nothing.

    // Both copies happen BEFORE the table is touched, and both are parked on the
    // stack across each other's calls: an argument register does not survive a call,
    // and `copy_flat_block` allocates.
    //
    // The name is copied too, not just the items. The table outlives the caller's
    // binding, and a slot holding the caller's `String` block would be a pointer into
    // a value the program is free to drop — the graphics thread would then read a
    // name out of reclaimed memory when it scans.
    let name_slot = builder.allocate_stack_object("canvas_group_name", 8);
    let items_slot = builder.allocate_stack_object("canvas_group_items", 8);
    let count_slot = builder.allocate_stack_object("canvas_group_count", 8);

    builder.emit(abi::store_u64(&items_in, abi::stack_pointer(), items_slot));
    let name_copy = builder.copy_flat_block(&ParameterType::String, &name_in)?;
    builder.emit(abi::store_u64(&name_copy, abi::stack_pointer(), name_slot));

    let incoming = builder.temporary_vreg();
    builder.emit(abi::load_u64(&incoming, abi::stack_pointer(), items_slot));
    // The count comes off the SOURCE before the copy replaces the slot: both carry the
    // same count, since a shrink-to-fit copy drops capacity and never entries.
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &count,
        &incoming,
        COLLECTION_OFFSET_COUNT as usize,
    ));
    builder.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

    let incoming2 = builder.temporary_vreg();
    builder.emit(abi::load_u64(&incoming2, abi::stack_pointer(), items_slot));
    let items_copy = builder.copy_flat_block(&items_type, &incoming2)?;
    builder.emit(abi::store_u64(
        &items_copy,
        abi::stack_pointer(),
        items_slot,
    ));

    // Charge the copy to the table's owned-bytes total, which is what `groupBytes=`
    // reports. Nothing subtracts from it yet: a replaced or removed group's block is
    // retired rather than freed until Phase 5's drain gate, so the total is a running
    // measure of exactly that leak.
    let size_slot = builder.allocate_stack_object("canvas_group_size", 8);
    builder.emit_inlined_block_size_from_ptr_slot(&items_type, items_slot, size_slot)?;
    let owned_base = groups_base(builder);
    let owned = builder.temporary_vreg();
    let added = builder.temporary_vreg();
    builder.emit(abi::load_u64(&owned, &owned_base, CANVAS_GROUP_OWNED_BYTES));
    builder.emit(abi::load_u64(&added, abi::stack_pointer(), size_slot));
    builder.emit(abi::add_registers(&owned, &owned, &added));
    // The NAME copy is charged too, so `groupBytes=` covers everything the table owns.
    // Charging only the items is what made a name leak invisible: the counter reported
    // a table owning nothing while 200 name copies sat unreachable.
    let name_size_slot = builder.allocate_stack_object("canvas_group_name_size", 8);
    builder.emit_inlined_block_size_from_ptr_slot(
        &ParameterType::String,
        name_slot,
        name_size_slot,
    )?;
    builder.emit(abi::load_u64(&added, abi::stack_pointer(), name_size_slot));
    builder.emit(abi::add_registers(&owned, &owned, &added));
    builder.emit(abi::store_u64(
        &owned,
        &owned_base,
        CANVAS_GROUP_OWNED_BYTES,
    ));

    let found_slot = builder.allocate_stack_object("canvas_group_found", 8);
    let free_slot = builder.allocate_stack_object("canvas_group_free", 8);
    emit_slot_scan(
        builder,
        name_slot,
        found_slot,
        free_slot,
        "canvas_set_group",
    );

    let claim = builder.label("canvas_set_group_claim");
    let install = builder.label("canvas_set_group_install");
    let full = builder.label("canvas_set_group_full");

    // A name already in the table replaces in place, keeping the slot — so a
    // `canvas::Group` node that already resolved to this slot keeps resolving to it,
    // and the revision bump is what tells `present` the contents moved.
    let target = builder.allocate_stack_object("canvas_group_target", 8);
    let found = builder.temporary_vreg();
    builder.emit(abi::load_u64(&found, abi::stack_pointer(), found_slot));
    builder.emit(abi::compare_immediate(&found, "0"));
    builder.emit(abi::branch_eq(&claim));
    builder.emit(abi::store_u64(&found, abi::stack_pointer(), target));
    builder.emit(abi::branch(&install));

    builder.emit(abi::label(&claim));
    let free = builder.temporary_vreg();
    builder.emit(abi::load_u64(&free, abi::stack_pointer(), free_slot));
    builder.emit(abi::compare_immediate(&free, "0"));
    builder.emit(abi::branch_eq(&full));
    builder.emit(abi::store_u64(&free, abi::stack_pointer(), target));
    builder.emit(abi::branch(&install));

    // Raising rather than evicting: dropping some other group to make room would draw
    // a picture the program did not describe, and silently. See `CANVAS_MAX_GROUPS`
    // for why 256 rather than a number a program is likely to reach.
    builder.emit(abi::label(&full));
    builder.raise_error_bare("ErrCanvasGroupLimit")?;

    builder.emit(abi::label(&install));
    let dst = builder.temporary_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit(abi::load_u64(&dst, abi::stack_pointer(), target));

    // Replacing a live name RETIRES the buffer it displaces rather than freeing it: the
    // graphics thread may be mid-copy of exactly that block. Stamping the frame is what
    // lets `__canvas_groupReclaim` know when no render can still be inside it.
    //
    // A slot can hold one retired buffer, and that is sufficient rather than lucky: a
    // second replacement before the first drains would find this word occupied, and the
    // drain gate runs at the top of every `present` — so reaching here twice without an
    // intervening drain means no frame completed in between, and the two `setGroup`
    // calls are then indistinguishable to any reader. The first block is freed here in
    // that case, because nothing can have started reading it.
    emit_retire_current_items(builder, &dst, &symbol)?;

    // Everything but the name first. A graphics thread scanning concurrently either
    // sees the old name (and the old, still-valid pointers, since Phase 5 is what
    // frees them) or the new name with everything already in place.
    builder.emit(abi::load_u64(&scratch, abi::stack_pointer(), items_slot));
    builder.emit(abi::store_u64(&scratch, &dst, CANVAS_GROUP_ITEMS));
    builder.emit(abi::load_u64(&scratch, abi::stack_pointer(), count_slot));
    builder.emit(abi::store_u64(&scratch, &dst, CANVAS_GROUP_COUNT));

    // One reference: the table's own. Scenes and parent groups add theirs in Phase 5.
    builder.emit(abi::move_immediate(&scratch, "Integer", "1"));
    builder.emit(abi::store_u64(&scratch, &dst, CANVAS_GROUP_REFS));
    // `CANVAS_GROUP_RETIRED_FRAME` is deliberately NOT written here. The retire above
    // has just stamped it with the frame this slot's displaced buffer must outlive, and
    // an "unretired" sentinel written afterwards destroys exactly that: with -1 the
    // gate `frame_now <= stamped` is true against every unsigned frame number, so a
    // replaced buffer was never freed while a removed one was. Measured as
    // `groupBytes=2112` holding across six frames.
    //
    // No sentinel is needed. `RETIRED_ITEMS` is the discriminator the drain reads
    // first, and this word means nothing while that one is zero.

    // The revision bump is LAST of the payload words and before the name, so a
    // resolver that reads a name reads a revision at least as new as the items.
    builder.emit(abi::load_u64(&scratch, &dst, CANVAS_GROUP_REVISION));
    builder.emit(abi::add_immediate(&scratch, &scratch, 1));
    builder.emit(abi::store_u64(&scratch, &dst, CANVAS_GROUP_REVISION));

    builder.emit(abi::load_u64(&scratch, abi::stack_pointer(), name_slot));
    builder.emit(abi::store_u64(&scratch, &dst, CANVAS_GROUP_NAME));

    // The epilogue is explicit, as it is in every `abi_function` lowering here: the
    // builder does not append one, so a body that just returns its `ValueResult` falls
    // off the end of its own code into whatever was emitted next.
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    // The mode gate is spliced in at the very top, before anything allocates, so a
    // wrong-mode call returns having touched neither the arena nor the table.
    prepend_wrong_mode_gate(
        &mut builder.instructions,
        &mut builder.relocations,
        &symbol,
        ctx.presentation_mode_offset,
        ModeRequirement::Canvas,
    );

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: symbol,
    })
}

/// `canvas::removeGroup(name)`.
///
/// Clears the slot's name and drops the table's reference. It does **not** free the
/// items: a graphics thread may be mid-frame over them, and the free waits for the
/// drain gate (§4.3). Phase 5 lands that gate; until then this leaks by construction,
/// which is what `groups=`/`groupBytes=` measures.
pub(crate) fn emit_remove_group(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let name_in = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the name argument"))?
        .location
        .clone();

    let name_slot = builder.allocate_stack_object("canvas_group_rm_name", 8);
    builder.emit(abi::store_u64(&name_in, abi::stack_pointer(), name_slot));

    let found_slot = builder.allocate_stack_object("canvas_group_rm_found", 8);
    let free_slot = builder.allocate_stack_object("canvas_group_rm_free", 8);
    emit_slot_scan(
        builder,
        name_slot,
        found_slot,
        free_slot,
        "canvas_remove_group",
    );

    // A name that is not installed is the documented no-op, symmetric with a `Group`
    // node naming an absent group — a program that cannot know whether a group is
    // installed should not have to check before removing it.
    let done = builder.label("canvas_remove_group_done");
    let found = builder.temporary_vreg();
    builder.emit(abi::load_u64(&found, abi::stack_pointer(), found_slot));
    builder.emit(abi::compare_immediate(&found, "0"));
    builder.emit(abi::branch_eq(&done));

    // The name has to be SAVED before it is cleared, and the two cannot be reordered:
    // clearing first is the concurrency invariant (the name is the discriminator, so a
    // concurrent scan must stop seeing this slot before anything else about it changes),
    // while `emit_retire_current_items` reads the live name to retire it — and by then
    // it is zero. Capturing it here and writing it into the retired word afterwards is
    // what lets both be true. Without the save the name block leaked, silently, because
    // `groupBytes=` charges only the items.
    let saved_name = builder.allocate_stack_object("canvas_group_rm_saved_name", 8);
    let live_name = builder.temporary_vreg();
    builder.emit(abi::load_u64(&live_name, &found, CANVAS_GROUP_NAME));
    builder.emit(abi::store_u64(&live_name, abi::stack_pointer(), saved_name));

    // Name first: it is the discriminator, so clearing it is what makes the slot
    // invisible to a concurrent scan. The pointers stay valid behind it for whatever
    // frame is still drawing them.
    let zero = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&zero, "Integer", "0"));
    builder.emit(abi::store_u64(&zero, &found, CANVAS_GROUP_NAME));

    let refs = builder.temporary_vreg();
    builder.emit(abi::load_u64(&refs, &found, CANVAS_GROUP_REFS));
    builder.emit(abi::subtract_immediate(&refs, &refs, 1));
    builder.emit(abi::store_u64(&refs, &found, CANVAS_GROUP_REFS));

    // The buffer is retired, not freed: a frame may be mid-copy of it. The drain gate
    // at the top of `present` frees it once a frame has completed.
    emit_retire_current_items(builder, &found, &symbol)?;

    // ...and the saved name goes into the retired word. `emit_retire_current_items`
    // read the slot's live name to retire it and found the zero this function had
    // already written, so it retired nothing; this is where the real pointer lands.
    let restore_name = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &restore_name,
        abi::stack_pointer(),
        saved_name,
    ));
    builder.emit(abi::store_u64(
        &restore_name,
        &found,
        CANVAS_GROUP_RETIRED_NAME,
    ));

    builder.emit(abi::label(&done));
    // The epilogue is explicit, as it is in every `abi_function` lowering here: the
    // builder does not append one, so a body that just returns its `ValueResult` falls
    // off the end of its own code into whatever was emitted next.
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    // The mode gate is spliced in at the very top, before anything allocates, so a
    // wrong-mode call returns having touched neither the arena nor the table.
    prepend_wrong_mode_gate(
        &mut builder.instructions,
        &mut builder.relocations,
        &symbol,
        ctx.presentation_mode_offset,
        ModeRequirement::Canvas,
    );

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: symbol,
    })
}

/// Move a slot's live `items` into its retired word and stamp the frame.
///
/// The buffer is not freed here and must not be: `canvas::groupItems` copies out of it
/// on the graphics thread, so a free at this moment races a reader. This is the same
/// retire-then-drain rule `.ai/canvas-threading.md` §3 gives for scene blocks and §7
/// for textures, which is why it is spelled the same way — one drain rule in the
/// subsystem rather than three.
///
/// If the slot already had a retired buffer, that one is freed first. It is safe by
/// construction rather than by luck: the drain gate runs at the top of every `present`,
/// so a still-occupied retired word means no frame has completed since it was retired,
/// which means no render can have started reading it.
fn emit_retire_current_items(
    builder: &mut CodeBuilder,
    slot: &VirtualRegister,
    symbol: &str,
) -> Result<(), String> {
    let list_type = ParameterType::list_of(ParameterType::named("DrawItem"));
    let no_prior = builder.label("canvas_group_retire_no_prior");
    let done = builder.label("canvas_group_retire_done");

    let prior = builder.temporary_vreg();
    builder.emit(abi::load_u64(&prior, slot, CANVAS_GROUP_RETIRED_ITEMS));
    builder.emit(abi::compare_immediate(&prior, "0"));
    builder.emit(abi::branch_eq(&no_prior));
    emit_free_items_block(builder, &prior, symbol, &list_type)?;
    builder.emit(abi::label(&no_prior));

    let live = builder.temporary_vreg();
    builder.emit(abi::load_u64(&live, slot, CANVAS_GROUP_ITEMS));
    builder.emit(abi::store_u64(&live, slot, CANVAS_GROUP_RETIRED_ITEMS));
    builder.emit(abi::store_u64(abi::ZERO, slot, CANVAS_GROUP_ITEMS));

    // The NAME is retired too, and forgetting it was a real leak: `setGroup` copies the
    // caller's name into the arena, and a `removeGroup` that only zeroed the pointer —
    // or a replacing `setGroup` that only overwrote it — left that copy unreachable
    // forever. It went unmeasured as well as unfreed, because `groupBytes=` charges only
    // the item block, so a 200-cycle install/remove test passed with 200 names leaked.
    //
    // Retired rather than freed for the same reason the items are, and it is the
    // sharper case of the two: `__canvas_appendDraw` resolves a group node on the
    // GRAPHICS thread by calling `canvas::groupResolve`, which scans the table comparing
    // name bytes. A name freed the instant a slot is cleared is a block a live scan may
    // be reading.
    let prior_name = builder.temporary_vreg();
    let no_prior_name = builder.label("canvas_group_retire_no_prior_name");
    builder.emit(abi::load_u64(&prior_name, slot, CANVAS_GROUP_RETIRED_NAME));
    builder.emit(abi::compare_immediate(&prior_name, "0"));
    builder.emit(abi::branch_eq(&no_prior_name));
    emit_free_name_block(builder, &prior_name, symbol)?;
    builder.emit(abi::label(&no_prior_name));

    let live_name = builder.temporary_vreg();
    builder.emit(abi::load_u64(&live_name, slot, CANVAS_GROUP_NAME));
    builder.emit(abi::store_u64(&live_name, slot, CANVAS_GROUP_RETIRED_NAME));

    let frame_now = builder.temporary_vreg();
    emit_load_frame_counter(builder, &frame_now, symbol);
    builder.emit(abi::store_u64(&frame_now, slot, CANVAS_GROUP_RETIRED_FRAME));

    builder.emit(abi::label(&done));
    Ok(())
}

/// Free one item block and take its bytes back off the table's owned total.
///
/// The size comes from the block itself rather than from anything remembered, which is
/// what keeps `groupBytes=` honest across a replace: the number that goes back is the
/// one that was added, because both are `emit_inlined_block_size_from_ptr_slot` of the
/// same block.
fn emit_free_items_block(
    builder: &mut CodeBuilder,
    block: &VirtualRegister,
    symbol: &str,
    list_type: &ParameterType,
) -> Result<(), String> {
    let ptr_slot = builder.allocate_stack_object("canvas_group_free_ptr", 8);
    let size_slot = builder.allocate_stack_object("canvas_group_free_size", 8);
    builder.emit(abi::store_u64(block, abi::stack_pointer(), ptr_slot));
    builder.emit_inlined_block_size_from_ptr_slot(list_type, ptr_slot, size_slot)?;

    let base = groups_base(builder);
    let owned = builder.temporary_vreg();
    let taken = builder.temporary_vreg();
    builder.emit(abi::load_u64(&owned, &base, CANVAS_GROUP_OWNED_BYTES));
    builder.emit(abi::load_u64(&taken, abi::stack_pointer(), size_slot));
    builder.emit(abi::subtract_registers(&owned, &owned, &taken));
    builder.emit(abi::store_u64(&owned, &base, CANVAS_GROUP_OWNED_BYTES));

    builder.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), ptr_slot));
    builder.emit(abi::load_u64(
        abi::c_arg(1),
        abi::stack_pointer(),
        size_slot,
    ));
    emit_arena_free(symbol, &mut builder.instructions, &mut builder.relocations);
    Ok(())
}

/// Free one interned name block, and take its bytes back off the owned total.
///
/// **Names are charged to `groupBytes=` deliberately, and an earlier version of this
/// function did not charge them** — which is exactly why a name leak went unnoticed:
/// with only the items counted, a run that leaked 200 name copies reported a table
/// owning nothing, and a 200-cycle churn test passed against the bug. A counter that
/// does not cover everything the table owns cannot detect the table owning too much.
///
/// So `groupBytes=` is "every byte this table is responsible for", not "what the items
/// cost". The items dominate it in any real program, so the number still reads the way
/// a caller expects.
fn emit_free_name_block(
    builder: &mut CodeBuilder,
    block: &VirtualRegister,
    symbol: &str,
) -> Result<(), String> {
    let ptr_slot = builder.allocate_stack_object("canvas_group_free_name_ptr", 8);
    let size_slot = builder.allocate_stack_object("canvas_group_free_name_size", 8);
    builder.emit(abi::store_u64(block, abi::stack_pointer(), ptr_slot));
    builder.emit_inlined_block_size_from_ptr_slot(&ParameterType::String, ptr_slot, size_slot)?;

    let base = groups_base(builder);
    let owned = builder.temporary_vreg();
    let taken = builder.temporary_vreg();
    builder.emit(abi::load_u64(&owned, &base, CANVAS_GROUP_OWNED_BYTES));
    builder.emit(abi::load_u64(&taken, abi::stack_pointer(), size_slot));
    builder.emit(abi::subtract_registers(&owned, &owned, &taken));
    builder.emit(abi::store_u64(&owned, &base, CANVAS_GROUP_OWNED_BYTES));

    builder.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), ptr_slot));
    builder.emit(abi::load_u64(
        abi::c_arg(1),
        abi::stack_pointer(),
        size_slot,
    ));
    emit_arena_free(symbol, &mut builder.instructions, &mut builder.relocations);
    Ok(())
}

/// `canvas::groupReclaim()` — free every retired buffer a frame has completed past.
///
/// Internal-only, called from `__canvas_present` **before** the content comparison and
/// on every present, not on the publish path (**G7**). `emit_reclaim_retired` — the
/// scene ring's equivalent — sits after the publish label, so it runs only when the
/// scene actually changed; a group free placed beside it would never run for
/// `removeGroup("panel")` followed by presents of an unchanged scene. The frame skip
/// would be working correctly and the memory would be held anyway.
///
/// A scan of at most `CANVAS_MAX_GROUPS` slots with no allocation, which is why it can
/// be unconditional. A memory bound that depends on the scene changing is not a bound.
pub(crate) fn emit_group_reclaim(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let list_type = ParameterType::list_of(ParameterType::named("DrawItem"));
    let head = builder.label("canvas_group_reclaim_head");
    let next = builder.label("canvas_group_reclaim_next");
    let done = builder.label("canvas_group_reclaim_done");

    let cursor = builder.allocate_stack_object("canvas_group_reclaim_cursor", 8);
    let index = builder.allocate_stack_object("canvas_group_reclaim_index", 8);
    let base = groups_base(builder);
    builder.emit(abi::store_u64(&base, abi::stack_pointer(), cursor));
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index));

    builder.emit(abi::label(&head));
    let i = builder.temporary_vreg();
    builder.emit(abi::load_u64(&i, abi::stack_pointer(), index));
    builder.emit(abi::compare_immediate(&i, &CANVAS_MAX_GROUPS.to_string()));
    builder.emit(abi::branch_ge(&done));

    let slot = builder.temporary_vreg();
    let retired = builder.temporary_vreg();
    builder.emit(abi::load_u64(&slot, abi::stack_pointer(), cursor));
    builder.emit(abi::load_u64(&retired, &slot, CANVAS_GROUP_RETIRED_ITEMS));
    builder.emit(abi::compare_immediate(&retired, "0"));
    builder.emit(abi::branch_eq(&next));

    // The gate: a frame must have COMPLETED since the retirement. `branch_ls` is the
    // same unsigned comparison the scene ring's reclaim uses, so the two agree on the
    // boundary case rather than differing by one.
    let stamped = builder.temporary_vreg();
    let frame_now = builder.temporary_vreg();
    builder.emit(abi::load_u64(&stamped, &slot, CANVAS_GROUP_RETIRED_FRAME));
    emit_load_frame_counter(builder, &frame_now, &symbol);
    builder.emit(abi::compare_registers(&frame_now, &stamped));
    builder.emit(abi::branch_ls(&next));

    emit_free_items_block(builder, &retired, &symbol, &list_type)?;
    let slot_again = builder.temporary_vreg();
    builder.emit(abi::load_u64(&slot_again, abi::stack_pointer(), cursor));
    builder.emit(abi::store_u64(
        abi::ZERO,
        &slot_again,
        CANVAS_GROUP_RETIRED_ITEMS,
    ));

    // The retired name drains on the same tick and behind the same gate. Guarded
    // separately because a slot can hold a retired name with no retired items — a
    // `removeGroup` on a group whose items were already drained does exactly that.
    let retired_name = builder.temporary_vreg();
    let no_name = builder.label("canvas_group_reclaim_no_name");
    builder.emit(abi::load_u64(
        &retired_name,
        &slot_again,
        CANVAS_GROUP_RETIRED_NAME,
    ));
    builder.emit(abi::compare_immediate(&retired_name, "0"));
    builder.emit(abi::branch_eq(&no_name));
    emit_free_name_block(builder, &retired_name, &symbol)?;
    builder.emit(abi::store_u64(
        abi::ZERO,
        &slot_again,
        CANVAS_GROUP_RETIRED_NAME,
    ));
    builder.emit(abi::label(&no_name));

    builder.emit(abi::label(&next));
    let advance = builder.temporary_vreg();
    builder.emit(abi::load_u64(&advance, abi::stack_pointer(), cursor));
    builder.emit(abi::add_immediate(
        &advance,
        &advance,
        CANVAS_GROUP_SLOT_BYTES,
    ));
    builder.emit(abi::store_u64(&advance, abi::stack_pointer(), cursor));
    let bumped = builder.temporary_vreg();
    builder.emit(abi::load_u64(&bumped, abi::stack_pointer(), index));
    builder.emit(abi::add_immediate(&bumped, &bumped, 1));
    builder.emit(abi::store_u64(&bumped, abi::stack_pointer(), index));
    builder.emit(abi::branch(&head));

    builder.emit(abi::label(&done));
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
        text: symbol,
    })
}

/// `canvas::groupCount()` — how many names the table currently holds.
///
/// Internal-only, and read by `__canvas_presentSurface`'s `MFB_CANVAS_STATS` line.
/// `.ai/canvas-threading.md` §11: the stats line is the only window a test has onto
/// worker-owned state, and the group table is worker-owned state.
///
/// A scan rather than a maintained counter. 256 loads is nothing next to a frame, and
/// a counter is a second copy of a fact that can disagree with the table it describes —
/// which is exactly the sort of drift this letter's other guards exist to catch.
pub(crate) fn emit_group_count(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let head = builder.label("canvas_group_count_head");
    let next = builder.label("canvas_group_count_next");
    let done = builder.label("canvas_group_count_done");

    let cursor = builder.allocate_stack_object("canvas_group_count_cursor", 8);
    let index = builder.allocate_stack_object("canvas_group_count_index", 8);
    let total = builder.allocate_stack_object("canvas_group_count_total", 8);

    let zero = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&zero, "Integer", "0"));
    builder.emit(abi::store_u64(&zero, abi::stack_pointer(), index));
    builder.emit(abi::store_u64(&zero, abi::stack_pointer(), total));
    let base = groups_base(builder);
    builder.emit(abi::store_u64(&base, abi::stack_pointer(), cursor));

    builder.emit(abi::label(&head));
    let i = builder.temporary_vreg();
    builder.emit(abi::load_u64(&i, abi::stack_pointer(), index));
    builder.emit(abi::compare_immediate(&i, &CANVAS_MAX_GROUPS.to_string()));
    builder.emit(abi::branch_ge(&done));

    let slot = builder.temporary_vreg();
    let name = builder.temporary_vreg();
    builder.emit(abi::load_u64(&slot, abi::stack_pointer(), cursor));
    builder.emit(abi::load_u64(&name, &slot, CANVAS_GROUP_NAME));
    builder.emit(abi::compare_immediate(&name, "0"));
    builder.emit(abi::branch_eq(&next));
    let seen = builder.temporary_vreg();
    builder.emit(abi::load_u64(&seen, abi::stack_pointer(), total));
    builder.emit(abi::add_immediate(&seen, &seen, 1));
    builder.emit(abi::store_u64(&seen, abi::stack_pointer(), total));

    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&slot, &slot, CANVAS_GROUP_SLOT_BYTES));
    builder.emit(abi::store_u64(&slot, abi::stack_pointer(), cursor));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::store_u64(&i, abi::stack_pointer(), index));
    builder.emit(abi::branch(&head));

    builder.emit(abi::label(&done));
    builder.emit(abi::load_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        total,
    ));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(RESULT_VALUE_REGISTER),
        text: symbol,
    })
}

/// `canvas::groupBytes()` — the bytes the group table currently owns.
///
/// Straight out of the header word `setGroup` maintains. Until Phase 5's drain gate
/// this only ever rises, which is the leak this phase ships deliberately and the number
/// that phase's acceptance has to move.
pub(crate) fn emit_group_bytes(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let base = groups_base(builder);
    builder.emit(abi::load_u64(
        RESULT_VALUE_REGISTER,
        &base,
        CANVAS_GROUP_OWNED_BYTES,
    ));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(RESULT_VALUE_REGISTER),
        text: symbol,
    })
}

/// The slot index a name resolves to, or `-1`.
///
/// `canvas::groupResolve(name) AS Integer`, internal-only. This is the **only** string
/// lookup in the group machinery, and it happens on the worker inside `present` — the
/// graphics thread never does one, because the resolution pass records what every group
/// node resolved to and the renderer reads that back by index (§4.4).
///
/// `-1` rather than a raise for a name that is not installed: a `canvas::Group` naming
/// an absent group is a documented silent no-op, and it is the caller's job to make
/// that mean "draw nothing", not this function's to refuse.
pub(crate) fn emit_group_resolve(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let name_in = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the name argument"))?
        .location
        .clone();

    let name_slot = builder.allocate_stack_object("canvas_group_res_name", 8);
    builder.emit(abi::store_u64(&name_in, abi::stack_pointer(), name_slot));
    let found_slot = builder.allocate_stack_object("canvas_group_res_found", 8);
    let free_slot = builder.allocate_stack_object("canvas_group_res_free", 8);
    emit_slot_scan(
        builder,
        name_slot,
        found_slot,
        free_slot,
        "canvas_group_resolve",
    );

    let miss = builder.label("canvas_group_resolve_miss");
    let done = builder.label("canvas_group_resolve_done");
    let found = builder.temporary_vreg();
    builder.emit(abi::load_u64(&found, abi::stack_pointer(), found_slot));
    builder.emit(abi::compare_immediate(&found, "0"));
    builder.emit(abi::branch_eq(&miss));

    // The scan yields an ADDRESS; the caller wants an index, because that is what
    // travels in the published signature list as a plain integer.
    let base = groups_base(builder);
    let index = builder.temporary_vreg();
    builder.emit(abi::subtract_registers(&index, &found, &base));
    builder.emit(abi::shift_right_immediate(
        &index,
        &index,
        CANVAS_GROUP_SLOT_SHIFT,
    ));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &index));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&miss));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    builder.emit(abi::subtract_immediate(
        RESULT_VALUE_REGISTER,
        RESULT_VALUE_REGISTER,
        1,
    ));

    builder.emit(abi::label(&done));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(RESULT_VALUE_REGISTER),
        text: symbol,
    })
}

/// A slot's revision word, or `0` for an out-of-range index.
///
/// `canvas::groupRevision(slot) AS Integer`, internal-only. Paired with the slot index
/// in the published signature, this is what lets `publishScene` see a group's *contents*
/// change when the scene list it is handed is byte-identical to the published one.
pub(crate) fn emit_group_revision(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let slot_in = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the slot argument"))?
        .location
        .clone();

    let out = builder.label("canvas_group_rev_out");
    let done = builder.label("canvas_group_rev_done");
    let index = builder.temporary_vreg();
    builder.emit(abi::move_register(&index, &slot_in));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    builder.emit(abi::compare_immediate(&index, "0"));
    builder.emit(abi::branch_lt(&out));
    builder.emit(abi::compare_immediate(
        &index,
        &CANVAS_MAX_GROUPS.to_string(),
    ));
    builder.emit(abi::branch_ge(&out));

    let base = groups_base(builder);
    let addr = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(
        &addr,
        &index,
        CANVAS_GROUP_SLOT_SHIFT,
    ));
    builder.emit(abi::add_registers(&addr, &base, &addr));
    builder.emit(abi::load_u64(
        RESULT_VALUE_REGISTER,
        &addr,
        CANVAS_GROUP_REVISION,
    ));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&out));
    builder.emit(abi::label(&done));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(RESULT_VALUE_REGISTER),
        text: symbol,
    })
}

/// A slot's installed items.
///
/// `canvas::groupItems(slot) AS List OF DrawItem`, internal-only.
///
/// A **copy**, for the same reason `canvas::installedItems` returns one: an MFBASIC
/// collection is a value, and handing back the table's own block would alias storage a
/// later `setGroup` replaces — and, once Phase 5's drain gate lands, storage that gets
/// freed. The copy is charged to the frame that draws, not to `present`, which is the
/// cost this letter set out to remove from the per-present path.
///
/// An empty list for an out-of-range index or an empty slot, so "this name was never
/// installed" and "this group is empty" render identically — which is what makes an
/// absent name a silent no-op rather than something the renderer has to branch on.
pub(crate) fn emit_group_items(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let slot_in = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the slot argument"))?
        .location
        .clone();

    let list_type = ParameterType::list_of(ParameterType::named("DrawItem"));
    let empty = builder.label("canvas_group_items_empty");
    let done = builder.label("canvas_group_items_done");

    let index = builder.temporary_vreg();
    builder.emit(abi::move_register(&index, &slot_in));
    builder.emit(abi::compare_immediate(&index, "0"));
    builder.emit(abi::branch_lt(&empty));
    builder.emit(abi::compare_immediate(
        &index,
        &CANVAS_MAX_GROUPS.to_string(),
    ));
    builder.emit(abi::branch_ge(&empty));

    let base = groups_base(builder);
    let addr = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(
        &addr,
        &index,
        CANVAS_GROUP_SLOT_SHIFT,
    ));
    builder.emit(abi::add_registers(&addr, &base, &addr));
    let items = builder.temporary_vreg();
    builder.emit(abi::load_u64(&items, &addr, CANVAS_GROUP_ITEMS));
    builder.emit(abi::compare_immediate(&items, "0"));
    builder.emit(abi::branch_eq(&empty));

    let copy = builder.copy_flat_block(&list_type, &items)?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &copy));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&empty));
    let fresh = builder.lower_empty_collection(&list_type)?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &fresh.location));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));

    builder.emit(abi::label(&done));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: symbol,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slot size and its shift are two spellings of one number.
    ///
    /// Every index-to-address conversion in this file uses the shift, and every table
    /// walk uses the size. If they disagree, a resolve returns an index that addresses
    /// the middle of a neighbouring slot — reading a `revision` out of someone else's
    /// `refs` and drawing a plausible wrong picture rather than failing.
    #[test]
    fn the_group_slot_size_is_a_power_of_two_matching_its_shift() {
        assert_eq!(
            1usize << CANVAS_GROUP_SLOT_SHIFT,
            CANVAS_GROUP_SLOT_BYTES,
            "CANVAS_GROUP_SLOT_SHIFT must be log2(CANVAS_GROUP_SLOT_BYTES): the \
             conversion is a shift, so a slot size that is not a power of two \
             silently addresses the wrong slot",
        );
        // Every named word, not just the last one this test happened to know about.
        // The bound was written against `RETIRED_FRAME` when that was the highest
        // offset; G32 then added `RETIRED_NAME` above it, and a fixed reference to one
        // word stops being a bound on the layout the moment the layout grows. Taking
        // the max is what makes this a check rather than a coincidence.
        let highest = [
            CANVAS_GROUP_NAME,
            CANVAS_GROUP_ITEMS,
            CANVAS_GROUP_COUNT,
            CANVAS_GROUP_REVISION,
            CANVAS_GROUP_REFS,
            CANVAS_GROUP_RETIRED_FRAME,
            CANVAS_GROUP_RETIRED_ITEMS,
            CANVAS_GROUP_RETIRED_NAME,
        ]
        .into_iter()
        .max()
        .expect("a non-empty word list");
        assert!(
            highest + 8 <= CANVAS_GROUP_SLOT_BYTES,
            "a slot word at +{highest} overflows a {CANVAS_GROUP_SLOT_BYTES}-byte slot: \
             it would be written into the NEXT slot, which reads as one group quietly \
             overwriting another's state",
        );
    }

    /// The owned-bytes header sits past every slot, so slot addressing never reaches it.
    #[test]
    fn the_owned_bytes_header_is_past_the_last_slot() {
        assert_eq!(
            CANVAS_GROUP_OWNED_BYTES,
            CANVAS_MAX_GROUPS * CANVAS_GROUP_SLOT_BYTES,
            "the header must begin exactly where the slot array ends, or a write to \
             slot 255 and a write to the header are the same word",
        );
        assert!(
            CANVAS_GROUP_TABLE_BYTES >= CANVAS_GROUP_OWNED_BYTES + 8,
            "the table must be big enough to hold the header word it ends with",
        );
    }
}
