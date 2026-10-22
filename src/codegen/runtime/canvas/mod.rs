//! The canvas graphics thread (plan-98-D Phase 2).
//!
//! One extra thread per canvas program, spawned lazily on the first `present`, which
//! owns the render loop. `canvas::present` stops rendering inline and instead
//! publishes the scene and signals; the graphics thread wakes, renders, and blits.
//!
//! `.ai/canvas-threading.md` is the normative protocol; this module implements §1
//! (the thread), §4 trigger 1 (a scene was published) and the redraw condition the
//! other triggers will also use.
//!
//! ## Why the thread runs MFBASIC
//!
//! The rasteriser is MFBASIC source (plan-98-C), which means the graphics thread
//! needs a real MFB execution context: an arena-state block for its allocations and
//! its module globals — including the sRGB transfer table `__COLOR_SRGB` (which
//! plan-122-B moved out of canvas into the `color` package, where it backs
//! `color::toLinear`), whose absence would silently render every antialiased pixel
//! black. It gets both the same way a `thread::start`
//! worker does: the spawner allocates and zeroes a child arena-state block, and the
//! trampoline runs the module's `LINK` and global initializers on the new thread
//! before entering the loop.
//!
//! Its globals are therefore **its own** — in particular its own geometry cache,
//! which is exactly right: the cache is renderer state, and the worker has no
//! business in it.
//!
//! ## Why the sync primitives are spelled `pthread_*`
//!
//! `emit_thread_external_call` translates each POSIX primitive to its Win32
//! equivalent (SRWLOCK / CONDITION_VARIABLE, both pointer-sized and valid when
//! zeroed), so the mutex and condition code below is written once and works on all
//! three families.

pub(crate) mod curve_shader;
pub(crate) mod metal;
pub(crate) mod vulkan;
pub(crate) mod vulkan_handoff;

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::runtime::thread::{emit_thread_external_call, thread_symbol};
use crate::target::shared::abi;
use std::collections::HashMap;

/// Scratch registers for the graphics emitters, allocated by the caller.
///
/// The caller must own the numbering: a fresh `Vregs::new()` starts at `%v0`, and
/// inside a `CodeBuilder`-managed body that is a register the builder has already
/// handed out. The two streams then overwrite each other's values — a corruption
/// with no local symptom, which cost an afternoon of chasing a libmalloc abort. So
/// the scratch is taken from whichever allocator owns the function being emitted
/// into, and taken *before* the call so the borrow does not outlive it.
pub(crate) struct GraphicsScratch {
    base: VirtualRegister,
    scratch: VirtualRegister,
    verdict: VirtualRegister,
    arena: VirtualRegister,
    cursor: VirtualRegister,
    end: VirtualRegister,
}

impl GraphicsScratch {
    pub(crate) fn new(vreg: &mut dyn FnMut() -> VirtualRegister) -> Self {
        GraphicsScratch {
            base: vreg(),
            scratch: vreg(),
            verdict: vreg(),
            arena: vreg(),
            cursor: vreg(),
            end: vreg(),
        }
    }
}

mod entry;
mod frame_sync;
mod scene_layout;
mod state_layout;
mod surface;
mod thread_lifecycle;

pub(crate) use entry::*;
pub(crate) use frame_sync::*;
pub(crate) use scene_layout::*;
pub(crate) use state_layout::*;
pub(crate) use surface::*;
pub(crate) use thread_lifecycle::*;

#[cfg(test)]
mod tests;
