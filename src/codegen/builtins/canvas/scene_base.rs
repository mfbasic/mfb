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
