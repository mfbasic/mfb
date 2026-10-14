//! The process-global loaded-font table: register on load, drop on release, look up
//! by the handle a `FontRef` carries.
//!
//! `canvas::loadFont` runs on the **worker** and the geometry cache that needs a
//! glyph's outline runs on the **graphics thread**. The bytes are already reachable
//! from both — an arena block is ordinary process memory, and only the *allocator
//! state* is per-thread — but the graphics thread has no way to get from the integer
//! in a `FontRef` to the block. This table is that map, and it is a process-global for
//! exactly the reason the scene region is one (`.ai/canvas-threading.md` §2).
//!
//! Chunks of sixteen slots, `handle`, `block` and `face`, and a `next` word linking each chunk
//! to the one after it (bug-756; it was one fixed table, and a seventeenth open font
//! silently drew nothing). No count word: a zero handle is the empty slot, and scanning
//! sixteen words is cheaper than keeping a count consistent between two threads without
//! a lock. The first chunk is the process-global symbol; the worker appends an arena
//! chunk only when every slot in the chain is taken. A chunk never moves and is never
//! freed, and arena memory stays mapped for the life of the process
//! (`.ai/canvas-threading.md` §2), so a reader holding a chunk address always holds a
//! valid one — which is why the chain can grow under a lock-free reader when an array
//! that reallocated could not.
//!
//! **Publishing is ordered.** A reader matches on `handle` and then follows `block` or
//! `face`, and walks on through `next`, so a `handle` must never be visible before its
//! `block` and `face`, nor
//! a `next` before the chunk it names is filled. On AArch64 a plain `str` can become
//! visible out of program order, so `handle` and `next` are written with a store-release
//! and read with a load-acquire there (the scene region's rule,
//! `scene_base::orders_scene_accesses`). x86-64 keeps stores and loads in order, and
//! there both are plain. Releasing a font is a single store of zero to its `handle`.

use super::scene_base::orders_scene_accesses;

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::push_symbol_address;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryHelper,
    RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// Park the first chunk's address in a fresh vreg.
fn table_base(builder: &mut CodeBuilder) -> VirtualRegister {
    let base = builder.temporary_vreg();
    let symbol = builder.current_symbol.clone();
    push_symbol_address(
        &symbol,
        CANVAS_FONTS_SYMBOL,
        &base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    base
}

/// Publish `value` at `base + offset`: a store-release where the target needs one.
fn emit_publish(
    builder: &mut CodeBuilder,
    base: &VirtualRegister,
    offset: usize,
    value: &VirtualRegister,
) {
    if orders_scene_accesses(builder) {
        let at = builder.temporary_vreg();
        builder.emit(abi::add_immediate(&at, base, offset));
        builder.emit(abi::store_release_u64(value, &at));
    } else {
        builder.emit(abi::store_u64(value, base, offset));
    }
}

/// Read a published word at `base + offset`: a load-acquire where the target needs one,
/// so the loads after it cannot see memory older than the word.
fn emit_observe(
    builder: &mut CodeBuilder,
    base: &VirtualRegister,
    offset: usize,
    dst: &VirtualRegister,
) {
    if orders_scene_accesses(builder) {
        let at = builder.temporary_vreg();
        builder.emit(abi::add_immediate(&at, base, offset));
        builder.emit(abi::load_acquire_u64(dst, &at));
    } else {
        builder.emit(abi::load_u64(dst, base, offset));
    }
}

/// Claim the first free slot in the chain for `handle` -> (`block`, `face`), appending a
/// chunk when every slot is taken.
///
/// Only the worker registers and releases, so the scan for a free slot races no other
/// writer. The new chunk is filled — zeroed, its first slot written — before the one
/// store that links it in, so a reader either does not reach it yet or sees it whole.
/// An allocation failure raises `ErrOutOfMemory` and branches to `failed`: a font that
/// loads but can never be drawn is the silent failure this chain exists to remove.
pub(crate) fn emit_register_font(
    builder: &mut CodeBuilder,
    handle: &Operand,
    block: &Operand,
    face: &Operand,
    failed: &str,
) -> Result<(), String> {
    let scan_chunk = builder.label("canvas_font_table_chunk");
    let scan = builder.label("canvas_font_table_scan");
    let next = builder.label("canvas_font_table_next");
    let follow = builder.label("canvas_font_table_follow");
    let grow = builder.label("canvas_font_table_grow");
    let alloc_ok = builder.label("canvas_font_table_alloc_ok");
    let done = builder.label("canvas_font_table_done");

    // Parked: the grow path calls the allocator, and nothing survives that in a register.
    let handle_slot = builder.allocate_stack_object("canvas_font_reg_handle", 8);
    let block_slot = builder.allocate_stack_object("canvas_font_reg_block", 8);
    let face_slot = builder.allocate_stack_object("canvas_font_reg_face", 8);
    let chunk_slot = builder.allocate_stack_object("canvas_font_reg_chunk", 8);
    builder.emit(abi::store_u64(handle, abi::stack_pointer(), handle_slot));
    builder.emit(abi::store_u64(block, abi::stack_pointer(), block_slot));
    builder.emit(abi::store_u64(face, abi::stack_pointer(), face_slot));

    let chunk = table_base(builder);
    let slot = builder.temporary_vreg();
    let limit = builder.temporary_vreg();
    let occupant = builder.temporary_vreg();
    let value = builder.temporary_vreg();

    builder.emit(abi::label(&scan_chunk));
    builder.emit(abi::move_register(&slot, &chunk));
    builder.emit(abi::add_immediate(&limit, &chunk, CANVAS_FONT_CHUNK_NEXT));

    builder.emit(abi::label(&scan));
    builder.emit(abi::compare_registers(&slot, &limit));
    builder.emit(abi::branch_ge(&follow));
    builder.emit(abi::load_u64(&occupant, &slot, CANVAS_FONT_SLOT_HANDLE));
    builder.emit(abi::compare_immediate(&occupant, "0"));
    builder.emit(abi::branch_ne(&next));
    // The block and the face go in first, and the handle is published after them.
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), block_slot));
    builder.emit(abi::store_u64(&value, &slot, CANVAS_FONT_SLOT_BLOCK));
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), face_slot));
    builder.emit(abi::store_u64(&value, &slot, CANVAS_FONT_SLOT_FACE));
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), handle_slot));
    emit_publish(builder, &slot, CANVAS_FONT_SLOT_HANDLE, &value);
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&slot, &slot, CANVAS_FONT_SLOT_BYTES));
    builder.emit(abi::branch(&scan));

    // Every slot in this chunk is taken: on to the next chunk, or grow the chain. Only
    // this thread writes `next`, so reading it needs no ordering.
    builder.emit(abi::label(&follow));
    builder.emit(abi::load_u64(&value, &chunk, CANVAS_FONT_CHUNK_NEXT));
    builder.emit(abi::compare_immediate(&value, "0"));
    builder.emit(abi::branch_eq(&grow));
    builder.emit(abi::move_register(&chunk, &value));
    builder.emit(abi::branch(&scan_chunk));

    builder.emit(abi::label(&grow));
    builder.emit(abi::store_u64(&chunk, abi::stack_pointer(), chunk_slot));
    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &CANVAS_FONT_CHUNK_BYTES.to_string(),
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::branch(failed));
    builder.emit(abi::label(&alloc_ok));

    let fresh = builder.temporary_vreg();
    builder.emit(abi::move_register(&fresh, abi::mfb_return(1)));
    for word in 0..CANVAS_FONT_CHUNK_BYTES / 8 {
        builder.emit(abi::store_u64(abi::ZERO, &fresh, word * 8));
    }
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), block_slot));
    builder.emit(abi::store_u64(&value, &fresh, CANVAS_FONT_SLOT_BLOCK));
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), face_slot));
    builder.emit(abi::store_u64(&value, &fresh, CANVAS_FONT_SLOT_FACE));
    builder.emit(abi::load_u64(&value, abi::stack_pointer(), handle_slot));
    builder.emit(abi::store_u64(&value, &fresh, CANVAS_FONT_SLOT_HANDLE));
    // Linking it in is the publish: everything above is visible before the link is.
    let tail = builder.temporary_vreg();
    builder.emit(abi::load_u64(&tail, abi::stack_pointer(), chunk_slot));
    emit_publish(builder, &tail, CANVAS_FONT_CHUNK_NEXT, &fresh);

    builder.emit(abi::label(&done));
    Ok(())
}

/// Clear the slot naming `handle`, wherever in the chain it is.
///
/// Called from `destroyFont`, so that text still naming a released font draws empty
/// rather than reading a block the program has finished with. Clearing the *handle*
/// is what unpublishes it: a reader matches on the handle, so the block word left
/// behind is unreachable, and zeroing it too would be two stores where one does. The
/// chunk stays linked, and the next font loaded reuses the slot.
pub(crate) fn emit_unregister_font(builder: &mut CodeBuilder, handle: &Operand) {
    let scan_chunk = builder.label("canvas_font_drop_chunk");
    let scan = builder.label("canvas_font_drop_scan");
    let next = builder.label("canvas_font_drop_next");
    let follow = builder.label("canvas_font_drop_follow");
    let done = builder.label("canvas_font_drop_done");

    let chunk = table_base(builder);
    let slot = builder.temporary_vreg();
    let limit = builder.temporary_vreg();
    let occupant = builder.temporary_vreg();

    builder.emit(abi::label(&scan_chunk));
    builder.emit(abi::move_register(&slot, &chunk));
    builder.emit(abi::add_immediate(&limit, &chunk, CANVAS_FONT_CHUNK_NEXT));

    builder.emit(abi::label(&scan));
    builder.emit(abi::compare_registers(&slot, &limit));
    builder.emit(abi::branch_ge(&follow));
    builder.emit(abi::load_u64(&occupant, &slot, CANVAS_FONT_SLOT_HANDLE));
    builder.emit(abi::compare_registers(&occupant, handle));
    builder.emit(abi::branch_ne(&next));
    builder.emit(abi::store_u64(abi::ZERO, &slot, CANVAS_FONT_SLOT_HANDLE));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&slot, &slot, CANVAS_FONT_SLOT_BYTES));
    builder.emit(abi::branch(&scan));

    builder.emit(abi::label(&follow));
    builder.emit(abi::load_u64(&chunk, &chunk, CANVAS_FONT_CHUNK_NEXT));
    builder.emit(abi::compare_immediate(&chunk, "0"));
    builder.emit(abi::branch_ne(&scan_chunk));

    builder.emit(abi::label(&done));
}

/// Find the `field` word (`CANVAS_FONT_SLOT_BLOCK` or `CANVAS_FONT_SLOT_FACE`)
/// registered for `handle`, or zero.
///
/// Zero is the answer for a handle that was never registered, one whose font was
/// released, and the `0` a `FontRef` holds when nothing was ever assigned to it. All
/// three mean the same thing to a caller — there is nothing to draw — so they get the
/// same answer rather than three. It runs on the graphics thread, so every `handle` and
/// `next` it follows is read with `emit_observe`.
pub(crate) fn emit_lookup_font(
    builder: &mut CodeBuilder,
    handle: &Operand,
    field: usize,
    out: &VirtualRegister,
) {
    let scan_chunk = builder.label("canvas_font_find_chunk");
    let scan = builder.label("canvas_font_find_scan");
    let next = builder.label("canvas_font_find_next");
    let follow = builder.label("canvas_font_find_follow");
    let done = builder.label("canvas_font_find_done");

    let chunk = table_base(builder);
    let slot = builder.temporary_vreg();
    let limit = builder.temporary_vreg();
    let occupant = builder.temporary_vreg();
    builder.emit(abi::move_immediate(out, "Integer", "0"));
    builder.emit(abi::compare_immediate(handle, "0"));
    builder.emit(abi::branch_eq(&done));

    builder.emit(abi::label(&scan_chunk));
    builder.emit(abi::move_register(&slot, &chunk));
    builder.emit(abi::add_immediate(&limit, &chunk, CANVAS_FONT_CHUNK_NEXT));

    builder.emit(abi::label(&scan));
    builder.emit(abi::compare_registers(&slot, &limit));
    builder.emit(abi::branch_ge(&follow));
    emit_observe(builder, &slot, CANVAS_FONT_SLOT_HANDLE, &occupant);
    builder.emit(abi::compare_registers(&occupant, handle));
    builder.emit(abi::branch_ne(&next));
    builder.emit(abi::load_u64(out, &slot, field));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&slot, &slot, CANVAS_FONT_SLOT_BYTES));
    builder.emit(abi::branch(&scan));

    builder.emit(abi::label(&follow));
    emit_observe(builder, &chunk, CANVAS_FONT_CHUNK_NEXT, &chunk);
    builder.emit(abi::compare_immediate(&chunk, "0"));
    builder.emit(abi::branch_ne(&scan_chunk));

    builder.emit(abi::label(&done));
}

/// `canvas::fontRegistered(id) AS Boolean` — is there a blob for this handle?
///
/// Split from the blob read so the MFBASIC wrapper can answer "no font" with an
/// ordinary `[]` literal. Building an empty collection from an emitter would mean
/// hand-writing a header the language already knows how to write, for the one case
/// that is not a font at all.
pub(crate) fn lower_font_registered(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let handle = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the font handle"))?
        .location
        .clone();

    let found = builder.temporary_vreg();
    emit_lookup_font(builder, &handle, CANVAS_FONT_SLOT_BLOCK, &found);
    let no = builder.label("canvas_font_registered_no");
    let done = builder.label("canvas_font_registered_done");
    builder.emit(abi::compare_immediate(&found, "0"));
    builder.emit(abi::branch_eq(&no));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&no));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
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
        text: "canvas.fontRegistered".to_string(),
    })
}

/// `canvas::fontBlobUnchecked(id) AS List OF Byte` — an owned copy of the font file.
///
/// **It copies, and the first version of this read did not.** Returning the font's own
/// block as a read-only alias is the obvious optimisation — a font file is hundreds of
/// kilobytes, and `measureText` runs per string per frame, so copying makes a read cost
/// the font's size rather than the string's length. It is also wrong: the returned value
/// binds to an ordinary `LET`, and that binding's scope-drop reclaims it, so the
/// *second* call on the same font reads a block the first call freed. Measured, not
/// reasoned: in the retired `canvas::fontBytes` (bug-756 replaced it with this read), the
/// first `measureText` printed correct metrics and the next one segfaulted (exit 139).
///
/// So the cost is real and paid deliberately. What removes it is not an alias but not
/// reading the file at all: plan-162-A's parse-once face index answers glyphs, advances
/// and metrics by handle. Until then `__canvas_shapeRun` binds one copy per font per run,
/// and the draw paths pay it once per geometry-cache **miss**, not per glyph or frame:
/// the cache means a string is only shaped again when it changes.
///
/// `Unchecked` because it must be called with a handle `canvas::fontRegistered` just
/// said yes to; on any other it would copy from a null pointer.
pub(crate) fn lower_font_blob_unchecked(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let handle = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the font handle"))?
        .location
        .clone();

    let block = builder.temporary_vreg();
    emit_lookup_font(builder, &handle, CANVAS_FONT_SLOT_BLOCK, &block);
    let copy = builder.copy_flat_block(&ParameterType::list_of(ParameterType::Byte), &block)?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &copy));
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
        text: "canvas.fontBlobUnchecked".to_string(),
    })
}

/// `canvas::faceAt(id, i) AS Integer` — word `i` of the face index published for a handle.
///
/// The face index (plan-162-A, `helper_font.rs:__canvas_faceIndex`) is the parsed part of
/// a font every string reads — metrics, `cmap` runs, advances. This is the **only** way
/// text code reaches it, and it reads the word **in place**: it never allocates, and it
/// never hands the block to MFBASIC.
///
/// **That is the whole rule, and it has a history.** The block belongs to the worker's
/// resource, and a value MFBASIC binds is a value MFBASIC frees: when the retired
/// `canvas::fontBytes` returned the font's own byte block as an alias, the `LET` holding
/// it reclaimed it at scope end, and the next `measureText` on the same font read freed
/// memory and segfaulted (exit 139; `lower_font_blob_unchecked` records it). So the
/// index is read one word at a time by native code here, on whichever thread asks, and
/// no MFBASIC name ever holds it.
///
/// An unregistered handle — never loaded, released, or the `0` a closed font's
/// `canvas::fontHandle` answers — and an `i` outside the block both answer `0`. Callers
/// test `canvas::fontRegistered` once per run, as `__canvas_fontBlob` does, so a `0` is
/// never taken for data. A released font's slot reads `0` from the moment
/// `destroyFont` clears its handle; the block itself is never freed (§7, "close never
/// frees"), so a reader that found it a moment earlier still reads valid memory.
pub(crate) fn lower_face_at(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let handle = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the font handle"))?
        .location
        .clone();
    let index = args
        .get(1)
        .ok_or_else(|| format!("'{symbol}' expects the word index"))?
        .location
        .clone();

    let done = builder.label("canvas_face_at_done");
    let at = builder.temporary_vreg();
    builder.emit(abi::move_register(&at, &index));
    let face = builder.temporary_vreg();
    emit_lookup_font(builder, &handle, CANVAS_FONT_SLOT_FACE, &face);
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    builder.emit(abi::compare_immediate(&face, "0"));
    builder.emit(abi::branch_eq(&done));
    builder.emit(abi::compare_immediate(&at, "0"));
    builder.emit(abi::branch_lt(&done));
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &face, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_registers(&at, &count));
    builder.emit(abi::branch_ge(&done));
    // A `List OF Integer` is the fixed-width layout: word `i` at `HEADER + 8 * i`.
    let address = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(&address, &at, 3));
    builder.emit(abi::add_registers(&address, &address, &face));
    builder.emit(abi::load_u64(
        RESULT_VALUE_REGISTER,
        &address,
        COLLECTION_HEADER_SIZE,
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
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "canvas.faceAt".to_string(),
    })
}

/// The safe front door the geometry builder calls.
#[rustfmt::skip]
const FONT_BLOB: &str =
r#"FUNC __canvas_fontBlob(id AS Integer) AS List OF Byte
  IF NOT canvas::fontRegistered(id) THEN
    RETURN []
  END IF
  RETURN canvas::fontBlobUnchecked(id)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    for (name, intro, lower) in [(
        "fontRegistered",
        "Is a font blob published for this handle?",
        lower_font_registered
            as fn(&mut CodeBuilder, &[ValueResult], &AbiCtx) -> Result<ValueResult, String>,
    )] {
        pkg.add_function(RegistryFunction {
            name,
            intro,
            desc: "Internal. Reads the process-global font table the worker publishes \
                   into and the graphics thread reads.",
            example: "",
            expected_arguments: None,
            internal_only: true,
            implementations: vec![Implementation {
                params: vec![Parameter {
                    name: "id",
                    desc: "The handle a `canvas::FontRef` carries.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Boolean,
                errors: vec![],
                body: Body::abi_function(lower),
            }],
        });
    }
    pkg.add_function(RegistryFunction {
        name: "fontBlobUnchecked",
        intro: "The published font file for a handle, as an owned copy.",
        desc: "Internal, and only valid for a handle `canvas::fontRegistered` has \
               answered yes to.",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "id",
                desc: "The handle a `canvas::FontRef` carries.",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_font_blob_unchecked),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "faceAt",
        intro: "One word of the face index published for a font handle.",
        desc: "Internal. Reads the word in place and never hands out the block; answers \
               `0` for an unregistered handle or an index outside the block.",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "id",
                    desc: "The handle a `canvas::FontRef` carries.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "i",
                    desc: "The word to read.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_face_at),
        }],
    });
    pkg.add_helper(RegistryHelper::always("canvas_fontBlob", FONT_BLOB));
}
