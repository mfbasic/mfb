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
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::AbiCtx;
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
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    builder.emit(abi::label(&publish));

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
