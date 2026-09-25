//! Addressing the canvas scene region.
//!
//! The scene lives in one writable **process-global** block
//! ([`CANVAS_SCENE_SYMBOL`]), not in any thread's arena state. Every read and write
//! of it goes through [`scene_base`], so there is exactly one place that knows where
//! the scene is.
//!
//! **Why not arena state**, where plan-98-B originally put it: arena state is
//! per-thread. The entry pins `x19` to its own stack frame, and in an `--app` build
//! the *worker* runs the entry, so a scene published into arena state is invisible to
//! the graphics thread plan-98-D spawns — it would read its own zeroed region and
//! render blank frames forever, silently, because a blank frame is a legal frame.
//! `.ai/canvas-threading.md` §2 has the full account.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::push_symbol_address;

/// A register holding the base address of the canvas scene region.
///
/// Materialized fresh at each use rather than cached in a callee-saved register: the
/// canvas bodies are short, and a cached base would have to survive the
/// `copy_flat_block` calls they all make.
pub(crate) fn scene_base(builder: &mut CodeBuilder) -> VirtualRegister {
    let base = builder.temporary_vreg();
    let symbol = builder.current_symbol.clone();
    push_symbol_address(
        &symbol,
        CANVAS_SCENE_SYMBOL,
        &base,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    base
}

/// Whether this target orders the scene region's cross-thread accesses with
/// store-release / load-acquire. AArch64 only: `stlr`/`ldar` exist nowhere else in the
/// backend (`CodeOp::StlrU64`). x86-64's TSO already keeps stores in order and loads in
/// order, which is all the scene's sequence lock needs from a plain `mov`; riscv64 has
/// no app mode, so no canvas program runs there.
pub(crate) fn orders_scene_accesses(builder: &CodeBuilder) -> bool {
    builder.platform.arch() == "aarch64"
}

/// Store `value` at `scene + offset`: a store-release where the target orders the
/// scene's accesses (visible only after every earlier store), a plain store elsewhere.
pub(crate) fn emit_scene_store(
    builder: &mut CodeBuilder,
    scene: &VirtualRegister,
    offset: usize,
    value: impl Into<crate::codegen::engine::operand::Operand>,
) {
    if orders_scene_accesses(builder) {
        let at = builder.temporary_vreg();
        builder.emit(crate::target::shared::abi::add_immediate(
            &at, scene, offset,
        ));
        builder.emit(crate::target::shared::abi::store_release_u64(value, &at));
    } else {
        builder.emit(crate::target::shared::abi::store_u64(value, scene, offset));
    }
}

/// Load `scene + offset` into `dst`: a load-acquire where the target orders the scene's
/// accesses (it happens before every later load), a plain load elsewhere.
pub(crate) fn emit_scene_load(
    builder: &mut CodeBuilder,
    scene: &VirtualRegister,
    offset: usize,
    dst: &VirtualRegister,
) {
    if orders_scene_accesses(builder) {
        let at = builder.temporary_vreg();
        builder.emit(crate::target::shared::abi::add_immediate(
            &at, scene, offset,
        ));
        builder.emit(crate::target::shared::abi::load_acquire_u64(dst, &at));
    } else {
        builder.emit(crate::target::shared::abi::load_u64(dst, scene, offset));
    }
}

/// Mark a publish as in progress: `pending = revision + 1`, released.
pub(crate) fn emit_mark_pending(builder: &mut CodeBuilder, scene: &VirtualRegister) {
    let next = builder.temporary_vreg();
    builder.emit(crate::target::shared::abi::load_u64(
        &next,
        scene,
        CANVAS_SCENE_REVISION_OFFSET,
    ));
    builder.emit(crate::target::shared::abi::add_immediate(&next, &next, 1));
    emit_scene_store(builder, scene, CANVAS_SCENE_PENDING_OFFSET, &next);
}

/// A store barrier made of what the backend has (bug-686): every store before it becomes
/// visible before every store after it. On AArch64 it re-stores `pending` with a
/// store-release (ordered after every earlier store) and loads it back with a
/// load-acquire (ordered after the release, and before every later access), so plain
/// `str`s on either side stay on their side. Nothing on x86-64, whose stores are
/// already observed in program order.
pub(crate) fn emit_scene_barrier(builder: &mut CodeBuilder, scene: &VirtualRegister) {
    if !orders_scene_accesses(builder) {
        return;
    }
    let at = builder.temporary_vreg();
    let value = builder.temporary_vreg();
    builder.emit(crate::target::shared::abi::add_immediate(
        &at,
        scene,
        CANVAS_SCENE_PENDING_OFFSET,
    ));
    builder.emit(crate::target::shared::abi::load_u64(&value, &at, 0));
    builder.emit(crate::target::shared::abi::store_release_u64(&value, &at));
    builder.emit(crate::target::shared::abi::load_acquire_u64(&value, &at));
}
