//! The shared body behind `canvas::present` and `canvas::presentLayers`.
//!
//! The two calls differ only in the element type they copy and which pair of scene
//! slots they publish into. Everything that makes a publish *correct* — the mode
//! gate before the allocation, the deep copy, the exact frame-skip comparison, and
//! the store ordering that keeps a half-written scene unobservable — is identical,
//! so it lives here once rather than being written twice and drifting.

// --- codegen tier imports (migration) ---
use super::scene_base::scene_base;
use crate::codegen::app::hook::app::{prepend_wrong_mode_gate, ModeRequirement};
use crate::codegen::collection::layout::list_entry_stride;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::push_symbol_address;
use crate::codegen::registry::AbiCtx;
use crate::codegen::runtime::canvas::{GRAPHICS_OFFSET_FRAMES, GRAPHICS_STATE_SYMBOL};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Which of the two published shapes a call installs.
///
/// A scene is exactly one shape at a time, and the *other* shape's pointer and count
/// are zeroed on every publish. That is what lets a reader decide with one test
/// (`layers != 0`) instead of carrying a separate discriminant that could disagree
/// with the pointers.
#[derive(Clone, Copy)]
pub(crate) enum SceneShape {
    /// `canvas::present(items AS List OF DrawItem)`.
    Flat,
    /// `canvas::presentLayers(layers AS List OF DrawLayer)`.
    Layered,
}

impl SceneShape {
    /// The element type of the list this shape installs.
    fn list_type(self) -> ParameterType {
        match self {
            SceneShape::Flat => ParameterType::list_of(ParameterType::named("DrawItem")),
            SceneShape::Layered => ParameterType::list_of(ParameterType::named("DrawLayer")),
        }
    }

    /// `(pointer, count)` offsets this shape publishes into, then the pair it clears.
    fn slots(self) -> ((usize, usize), (usize, usize)) {
        let flat = (CANVAS_SCENE_ITEMS_OFFSET, CANVAS_SCENE_COUNT_OFFSET);
        let layered = (CANVAS_SCENE_LAYERS_OFFSET, CANVAS_SCENE_LAYER_COUNT_OFFSET);
        match self {
            SceneShape::Flat => (flat, layered),
            SceneShape::Layered => (layered, flat),
        }
    }

    /// A label prefix, so the two bodies' local labels cannot collide if a program
    /// uses both calls.
    fn tag(self) -> &'static str {
        match self {
            SceneShape::Flat => "canvas_present",
            SceneShape::Layered => "canvas_present_layers",
        }
    }
}

/// Deep-copy the incoming list into the arena's canvas scene region and publish it,
/// skipping the publish when the content is unchanged.
///
/// Returns **TRUE when it published** and FALSE when it skipped, so the caller can
/// gate the render on it. That is what makes the frame skip worth anything: the
/// publish is three stores, the render is the whole scene, and skipping only the
/// stores would save nothing measurable.
///
/// **Why a copy at all**: the renderer reads the installed scene at arbitrary times
/// after the call returns, with no further involvement from the program. A scene
/// pointing at caller storage would be read after that storage was reused.
///
/// **Why one copy suffices**: an MFBASIC collection is a self-contained flat block —
/// strings, records and nested collections are inlined into it, not referenced from
/// it — so `copy_flat_block` is already the transitive deep copy, per its own
/// contract ("because a flat block has no internal pointers, the byte copy **is** a
/// deep copy"). No per-variant walk is needed.
///
/// The copy lands in the **arena**, not the caller's frame: the arena is a growing
/// region owned by the execution context, so the block outlives this call.
pub(crate) fn emit_publish(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
    shape: SceneShape,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scene = scene_base(builder);
    let incoming = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the scene list argument"))?
        .location
        .clone();

    let list_type = shape.list_type();
    let ((ptr_offset, count_offset), (other_ptr, other_count)) = shape.slots();
    let tag = shape.tag();

    // Hold the incoming list pointer across the copy's calls: `copy_flat_block`
    // allocates, and an argument register does not survive a call.
    let source_slot = builder.allocate_stack_object("canvas_publish_source", 8);
    builder.emit(abi::store_u64(&incoming, abi::stack_pointer(), source_slot));

    let copy = builder.copy_flat_block(&list_type, &incoming)?;
    let copy_slot = builder.allocate_stack_object("canvas_publish_copy", 8);
    builder.emit(abi::store_u64(&copy, abi::stack_pointer(), copy_slot));

    // count = source.count. Read from the SOURCE rather than the copy only because
    // the source pointer is already parked; both carry the same count (the copy is
    // shrink-to-fit, which drops capacity, never entries).
    let count = builder.temporary_vreg();
    let source = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), source_slot));
    builder.emit(abi::load_u64(&count, &source, COLLECTION_OFFSET_COUNT));

    // ---- Frame skip: an identical re-present publishes nothing ----------------
    //
    // Compares the **data region**, not the whole block, plus `count` and
    // `dataLength`. Those three together are the scene's content: the data region
    // holds every element's bytes, and for a tight copy the entry offsets are
    // sequential over it.
    //
    // The whole-block compare this replaced never once reported "same" (measured:
    // three identical presents produced three frames, on the pre-graphics-thread
    // build too). Both sides *are* shrink-to-fit — `copy_flat_block` dispatches to
    // `copy_collection_tight` for a collection — so that was not the problem. The
    // problem is that a lookup entry is 40 bytes of which a **list** writes only
    // some: `keyOffset` and `keyLength` are meaningless without keys and are never
    // written, so they hold whatever the arena handed out. Two allocations, two
    // different values, one spurious "changed".
    //
    // Still no hash, so still no collisions.
    //
    // The copy still happens on a skipped frame. That is the design (plan-98-A
    // invariant 2 charges the deep copy to the caller's frame budget); what the skip
    // buys is not re-publishing, which is what would make the renderer redraw.
    let skip = builder.label(&format!("{tag}_skip"));
    let publish = builder.label(&format!("{tag}_publish"));
    let size_slot = builder.allocate_stack_object("canvas_publish_size", 8);
    let installed_size_slot = builder.allocate_stack_object("canvas_publish_prev_size", 8);
    let installed_slot = builder.allocate_stack_object("canvas_publish_prev", 8);

    let installed = builder.temporary_vreg();
    builder.emit(abi::load_u64(&installed, &scene, ptr_offset));
    builder.emit(abi::store_u64(
        &installed,
        abi::stack_pointer(),
        installed_slot,
    ));
    // Nothing installed in THIS shape yet — publish. (Switching shapes therefore
    // always publishes, which is correct: the scene really did change.)
    builder.emit(abi::compare_immediate(&installed, "0"));
    builder.emit(abi::branch_eq(&publish));

    let fresh = builder.temporary_vreg();
    let previous = builder.temporary_vreg();
    let new_count = builder.temporary_vreg();
    let old_count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&fresh, abi::stack_pointer(), copy_slot));
    builder.emit(abi::load_u64(
        &previous,
        abi::stack_pointer(),
        installed_slot,
    ));
    builder.emit(abi::load_u64(&new_count, &fresh, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::load_u64(
        &old_count,
        &previous,
        COLLECTION_OFFSET_COUNT,
    ));
    builder.emit(abi::compare_registers(&new_count, &old_count));
    builder.emit(abi::branch_ne(&publish));

    let new_size = builder.temporary_vreg();
    let old_size = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &new_size,
        &fresh,
        COLLECTION_OFFSET_DATA_LENGTH,
    ));
    builder.emit(abi::load_u64(
        &old_size,
        &previous,
        COLLECTION_OFFSET_DATA_LENGTH,
    ));
    builder.emit(abi::compare_registers(&new_size, &old_size));
    builder.emit(abi::branch_ne(&publish));
    builder.emit(abi::store_u64(&new_size, abi::stack_pointer(), size_slot));
    builder.emit(abi::store_u64(
        &old_size,
        abi::stack_pointer(),
        installed_size_slot,
    ));

    // Data base = block + HEADER + capacity * entryStride. Both sides are tight, so
    // `capacity == count`; deriving it from each block's own capacity rather than
    // assuming that keeps this correct if a future copy stops being tight.
    let element = ParameterType::named(match shape {
        SceneShape::Flat => "DrawItem",
        SceneShape::Layered => "DrawLayer",
    });
    let stride = list_entry_stride(&element);
    let left = builder.temporary_vreg();
    let right = builder.temporary_vreg();
    for (block, out) in [(&fresh, &left), (&previous, &right)] {
        let capacity = builder.temporary_vreg();
        builder.emit(abi::load_u64(&capacity, block, COLLECTION_OFFSET_CAPACITY));
        let bytes = builder.temporary_vreg();
        builder.emit(abi::move_immediate(&bytes, "Integer", &stride.to_string()));
        builder.emit(abi::multiply_registers(&capacity, &capacity, &bytes));
        builder.emit(abi::add_registers(out, block, &capacity));
        builder.emit(abi::add_immediate(out, out, COLLECTION_HEADER_SIZE));
    }
    let length = builder.temporary_vreg();
    builder.emit(abi::load_u64(&length, abi::stack_pointer(), size_slot));
    builder.emit_compare_bytes_branch(
        &left,
        &right,
        &length,
        &skip,
        &publish,
        &format!("{tag}_same"),
    );

    // Skipped: report FALSE so the caller does not re-render. That is what makes the
    // skip worth anything — the publish itself is cheap, the render is not.
    builder.emit(abi::label(&skip));
    // ...and give the comparison's copy back (bug-683). The copy is made before the
    // comparison because the comparison needs something to compare, and this exit used
    // to return without freeing it — so every re-present of an UNCHANGED scene leaked a
    // whole scene copy, the case `mfb man canvas` calls a no-op. Measured at 16,048
    // bytes per present for a 40-rectangle scene, linear in the frame count.
    //
    // Unambiguously safe, and the only free on this side of the retirement gate: the
    // block was allocated in this call and control reaches here only when it was never
    // stored into the scene region, so no renderer can have seen it.
    emit_free_block(builder, &list_type, copy_slot, &symbol)?;
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    builder.emit(abi::label(&publish));

    // ---- Reclaim, then retire (plan-98-D Phase 3) -----------------------------
    //
    // A publish displaces the block the renderer may be *reading right now*, so it
    // cannot be freed here. It is retired instead, stamped with the frame counter,
    // and reclaimed by a **later** publish once a frame has completed since — at
    // which point no render can still hold it.
    //
    // Without this every publish abandoned its predecessor: a 200-frame animation
    // grew by ~0.11 MB a frame with nothing ever reclaiming it.
    //
    // Retirement is a **list**, not one slot (bug-683). Presents are not rate-limited
    // to one per rendered frame, and every block displaced since the last frame tick is
    // one a render in flight may be reading — so the number that has to be held is the
    // number the schedule produced. The single slot this replaced was overwritten by
    // the second present inside a frame, at one whole scene copy lost per present.
    //
    // The free happens on the worker, which is also what allocated the block. An
    // arena is per-thread, so the graphics thread must never do it.
    emit_reclaim_retired(builder, &scene, &symbol)?;
    let oom = builder.label(&format!("{tag}_retire_oom"));
    emit_retire_displaced(builder, &scene, &oom, &symbol)?;

    // Publish: this shape's pointer and count, the other shape's pair cleared, then
    // the revision. The revision is written LAST and is what a reader gates on, so a
    // reader can never observe a bumped revision alongside a half-written scene.
    let published = builder.temporary_vreg();
    builder.emit(abi::load_u64(&published, abi::stack_pointer(), copy_slot));
    builder.emit(abi::store_u64(&published, &scene, ptr_offset));
    builder.emit(abi::store_u64(&count, &scene, count_offset));
    builder.emit(abi::store_u64(abi::ZERO, &scene, other_ptr));
    builder.emit(abi::store_u64(abi::ZERO, &scene, other_count));
    let revision = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &revision,
        &scene,
        CANVAS_SCENE_REVISION_OFFSET,
    ));
    builder.emit(abi::add_immediate(&revision, &revision, 1));
    builder.emit(abi::store_u64(
        &revision,
        &scene,
        CANVAS_SCENE_REVISION_OFFSET,
    ));

    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "1"));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    // The cold path, laid out after the publish exit: the retirement node could not be
    // allocated, so this call publishes nothing and raises. Out of line because it ends
    // in a `ret`, and a second `ret` between the publish label and the exit reporting
    // TRUE would make "what does the publish return" ambiguous to read — in the emitted
    // code and in `the_skip_reports_false_and_the_publish_reports_true`.
    //
    // The fresh copy goes back first. This is the one path that leaves `emit_publish`
    // between the copy and the publish, and leaving it without either publishing the
    // copy or freeing it would trade bug-683's leak for a rarer one. The scene region is
    // untouched, so the installed scene is still whole.
    builder.emit(abi::label(&oom));
    emit_free_block(builder, &list_type, copy_slot, &symbol)?;
    builder.raise_error_bare("ErrOutOfMemory")?;

    // The mode gate is spliced in at the very top, before the manual prologue, so a
    // wrong-mode call returns before allocating anything at all.
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

/// Load the graphics thread's completed-frame counter.
///
/// Read without the mutex, and that is sound: it is a single aligned word, it only
/// ever increases, and the retirement gate is a `>` comparison. A stale read makes
/// the free happen one publish later than it could — never earlier, which is the only
/// direction that would be a use-after-free.
pub(crate) fn emit_load_frame_counter(
    builder: &mut CodeBuilder,
    dst: &VirtualRegister,
    symbol: &str,
) {
    let base = builder.temporary_vreg();
    push_symbol_address(
        symbol,
        GRAPHICS_STATE_SYMBOL,
        &base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::load_u64(dst, &base, GRAPHICS_OFFSET_FRAMES));
}

/// Free one block named by a pointer **stack slot**, and zero the slot.
///
/// The size comes from the block's own header — the same computation
/// `copy_flat_block` used to allocate it. Getting that size wrong frees the wrong
/// number of bytes and corrupts the arena free list, which is why it is derived rather
/// than remembered (bug-560).
///
/// A zero pointer frees nothing, so the caller does not have to prove the slot is
/// occupied.
fn emit_free_block(
    builder: &mut CodeBuilder,
    type_: &ParameterType,
    slot: usize,
    symbol: &str,
) -> Result<(), String> {
    let empty = builder.label("canvas_free_block_empty");
    let block = builder.temporary_vreg();
    builder.emit(abi::load_u64(&block, abi::stack_pointer(), slot));
    builder.emit(abi::compare_immediate(&block, "0"));
    builder.emit(abi::branch_eq(&empty));

    let size_slot = builder.allocate_stack_object("canvas_free_block_size", 8);
    builder.emit_inlined_block_size_from_ptr_slot(type_, slot, size_slot)?;
    builder.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), slot));
    builder.emit(abi::load_u64(
        abi::c_arg(1),
        abi::stack_pointer(),
        size_slot,
    ));
    emit_arena_free(symbol, &mut builder.instructions, &mut builder.relocations);
    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
    builder.emit(abi::label(&empty));
    Ok(())
}

/// The three block types a retirement node can hold, paired with the node offset each
/// is stored at. A node carries whichever of them the publish it displaced had
/// installed; the others are zero and cost only their `emit_free_block` guard.
fn retired_block_types() -> [(usize, ParameterType); 3] {
    [
        (
            CANVAS_RETIRE_NODE_ITEMS,
            ParameterType::list_of(ParameterType::named("DrawItem")),
        ),
        (
            CANVAS_RETIRE_NODE_HASHES,
            ParameterType::list_of(ParameterType::Integer),
        ),
        (
            CANVAS_RETIRE_NODE_LAYERS,
            ParameterType::list_of(ParameterType::named("DrawLayer")),
        ),
    ]
}

/// Drain the retirement list once a frame has completed since its **newest** node.
///
/// The gate is unchanged from the single slot this replaces — `frame_now > stamped`,
/// emitted as "branch away when `frame_now <= stamped`" — and it is the one thing here
/// that must not be relaxed. Retirement exists because `__canvas_sceneDraws` and
/// `__canvas_sceneOffsets` re-read the installed pointer through
/// `canvas::installedItems` at arbitrary points inside a frame, so a block displaced
/// during the frame in progress may be mid-copy right now. Only a *completed* frame
/// proves otherwise.
///
/// Testing the head alone is what the list's newest-first order buys: the head carries
/// the largest stamp, so `frame_now > head.frame` proves the same thing about every
/// node behind it and the whole chain goes at once. That is never more eager than a
/// per-node gate — no node is freed before its own stamp would allow — and it holds an
/// older node at most one extra frame.
fn emit_reclaim_retired(
    builder: &mut CodeBuilder,
    scene: &VirtualRegister,
    symbol: &str,
) -> Result<(), String> {
    let done = builder.label("canvas_reclaim_done");
    let loop_top = builder.label("canvas_reclaim_loop");

    let head = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &head,
        scene,
        CANVAS_SCENE_RETIRED_HEAD_OFFSET,
    ));
    builder.emit(abi::compare_immediate(&head, "0"));
    builder.emit(abi::branch_eq(&done));

    let retired_frame = builder.temporary_vreg();
    let frame_now = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &retired_frame,
        &head,
        CANVAS_RETIRE_NODE_FRAME,
    ));
    emit_load_frame_counter(builder, &frame_now, symbol);
    builder.emit(abi::compare_registers(&frame_now, &retired_frame));
    builder.emit(abi::branch_ls(&done));

    // Detach the whole chain BEFORE freeing any of it, so the head never names a block
    // that has already gone back to the arena.
    builder.emit(abi::store_u64(
        abi::ZERO,
        scene,
        CANVAS_SCENE_RETIRED_HEAD_OFFSET,
    ));

    // The cursor lives in a stack slot rather than a register: each iteration makes up
    // to four `_mfb_arena_free` calls, the last of which frees the very node being
    // walked, so `next` has to be read out and parked before that happens.
    let cursor = builder.allocate_stack_object("canvas_reclaim_cursor", 8);
    let next = builder.allocate_stack_object("canvas_reclaim_next", 8);
    let block = builder.allocate_stack_object("canvas_reclaim_block", 8);
    builder.emit(abi::store_u64(&head, abi::stack_pointer(), cursor));

    builder.emit(abi::label(&loop_top));
    let node = builder.temporary_vreg();
    builder.emit(abi::load_u64(&node, abi::stack_pointer(), cursor));
    builder.emit(abi::compare_immediate(&node, "0"));
    builder.emit(abi::branch_eq(&done));

    let successor = builder.temporary_vreg();
    builder.emit(abi::load_u64(&successor, &node, CANVAS_RETIRE_NODE_NEXT));
    builder.emit(abi::store_u64(&successor, abi::stack_pointer(), next));

    for (offset, type_) in retired_block_types() {
        let walking = builder.temporary_vreg();
        let held = builder.temporary_vreg();
        builder.emit(abi::load_u64(&walking, abi::stack_pointer(), cursor));
        builder.emit(abi::load_u64(&held, &walking, offset));
        builder.emit(abi::store_u64(&held, abi::stack_pointer(), block));
        emit_free_block(builder, &type_, block, symbol)?;
    }

    // Then the node itself. Its size is a compile-time constant rather than a header
    // read — a node is not a collection block, so `_mfb_arena_free` has to be handed
    // exactly the `CANVAS_RETIRE_NODE_SIZE` the matching alloc asked for (bug-560).
    builder.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), cursor));
    builder.emit(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        &CANVAS_RETIRE_NODE_SIZE.to_string(),
    ));
    emit_arena_free(symbol, &mut builder.instructions, &mut builder.relocations);

    let advance = builder.temporary_vreg();
    builder.emit(abi::load_u64(&advance, abi::stack_pointer(), next));
    builder.emit(abi::store_u64(&advance, abi::stack_pointer(), cursor));
    builder.emit(abi::branch(&loop_top));

    builder.emit(abi::label(&done));
    Ok(())
}

/// Push the blocks this publish displaces onto the retirement list, stamped with the
/// frame counter.
///
/// Nothing installed yet — the first publish of a program, and the first of each shape
/// — displaces nothing and allocates no node, so a program that presents once pays for
/// none of this.
///
/// A failed node allocation branches to `oom`, which the caller lays out **after** its
/// own `ret`: it is the cold path, and inlining it here would put a second `ret` inside
/// the publish body between the publish label and the exit that reports TRUE.
fn emit_retire_displaced(
    builder: &mut CodeBuilder,
    scene: &VirtualRegister,
    oom: &str,
    symbol: &str,
) -> Result<(), String> {
    let nothing = builder.label("canvas_retire_nothing");
    let allocated = builder.label("canvas_retire_allocated");

    let live = [
        (CANVAS_RETIRE_NODE_ITEMS, CANVAS_SCENE_ITEMS_OFFSET),
        (CANVAS_RETIRE_NODE_HASHES, CANVAS_SCENE_HASHES_OFFSET),
        (CANVAS_RETIRE_NODE_LAYERS, CANVAS_SCENE_LAYERS_OFFSET),
    ];

    // `items | hashes | layers == 0` is "this publish displaces nothing".
    let any = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&any, "Integer", "0"));
    for (_, offset) in live {
        let installed = builder.temporary_vreg();
        builder.emit(abi::load_u64(&installed, scene, offset));
        builder.emit(abi::or_registers(&any, &any, &installed));
    }
    builder.emit(abi::compare_immediate(&any, "0"));
    builder.emit(abi::branch_eq(&nothing));

    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &CANVAS_RETIRE_NODE_SIZE.to_string(),
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_ne(oom));
    builder.emit(abi::label(&allocated));

    let node = builder.temporary_vreg();
    builder.emit(abi::move_register(&node, abi::mfb_return(1)));

    // The displaced pointers are read AFTER the allocation: the call clobbers the
    // argument and return banks, and reading them first would park three values across
    // it for nothing.
    for (node_offset, scene_offset) in live {
        let displaced = builder.temporary_vreg();
        builder.emit(abi::load_u64(&displaced, scene, scene_offset));
        builder.emit(abi::store_u64(&displaced, &node, node_offset));
    }
    let frame_now = builder.temporary_vreg();
    emit_load_frame_counter(builder, &frame_now, symbol);
    builder.emit(abi::store_u64(&frame_now, &node, CANVAS_RETIRE_NODE_FRAME));

    // Linked LAST: the node is fully written before it is reachable from the scene
    // region, so a drain can never walk into a half-built node.
    let head = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &head,
        scene,
        CANVAS_SCENE_RETIRED_HEAD_OFFSET,
    ));
    builder.emit(abi::store_u64(&head, &node, CANVAS_RETIRE_NODE_NEXT));
    builder.emit(abi::store_u64(
        &node,
        scene,
        CANVAS_SCENE_RETIRED_HEAD_OFFSET,
    ));

    builder.emit(abi::label(&nothing));
    Ok(())
}
