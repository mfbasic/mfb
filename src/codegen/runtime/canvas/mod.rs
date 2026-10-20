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
mod tests {
    use super::{
        emit_publish_surface_size, DEFAULT_SURFACE_HEIGHT, DEFAULT_SURFACE_WIDTH,
        GRAPHICS_OFFSET_HEIGHT, GRAPHICS_OFFSET_RESIZES, GRAPHICS_OFFSET_WIDTH,
    };
    use super::{graphics_trampoline_saved, graphics_trampoline_shadow, thread_entry_frame};

    /// The graphics entry's frame: the shared frame arithmetic at its own save count.
    fn graphics_trampoline_frame(arch: &str, windows: bool) -> usize {
        thread_entry_frame(
            arch,
            windows,
            graphics_trampoline_saved(arch, windows).len(),
        )
    }
    use crate::arch::ops::CodeOp;
    use crate::target::shared::abi;
    use std::collections::HashMap;

    /// Run the size publisher's own instructions over a graphics state held as
    /// `offset -> word`, publishing `width` by `height`.
    ///
    /// The publisher is a straight run of loads, compares, branches and stores on the
    /// registers it was handed, the same at every platform's resize callback, so it can
    /// be executed here rather than read: what it leaves in the state is the contract.
    /// An instruction this does not know fails the test.
    fn publish(state: &mut HashMap<usize, u64>, width: u64, height: u64) {
        let mut instructions = Vec::new();
        let mut relocations = Vec::new();
        emit_publish_surface_size(
            "resize_test",
            abi::LOCAL[3],
            abi::LOCAL[1],
            abi::LOCAL[2],
            &mut instructions,
            &mut relocations,
        );
        let name = |reg: abi::Reg| {
            abi::move_register(reg, reg)
                .get("dst")
                .expect("a register renders")
        };
        let base = name(abi::LOCAL[3]);
        let mut registers: HashMap<String, u64> =
            HashMap::from([(name(abi::LOCAL[1]), width), (name(abi::LOCAL[2]), height)]);
        let field = |at: usize, key: &str| -> String {
            instructions[at]
                .get(key)
                .unwrap_or_else(|| panic!("instruction {at} has no `{key}`"))
        };
        let label = |target: &str| -> usize {
            instructions
                .iter()
                .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(target))
                .unwrap_or_else(|| panic!("no label `{target}`"))
        };
        // The address of the state is the first thing emitted; everything after it
        // reaches the state through `base`.
        let mut at = instructions
            .iter()
            .position(|i| i.op == CodeOp::LdrU64)
            .expect("the publisher reads the old size");
        let mut flags = (0u64, 0u64);
        while at < instructions.len() {
            let op = instructions[at].op;
            let mut next = at + 1;
            match op {
                CodeOp::Label => {}
                CodeOp::LdrU64 => {
                    assert_eq!(field(at, "base"), base, "a load not from the state");
                    let offset: usize = field(at, "offset").parse().expect("an offset");
                    let value = state.get(&offset).copied().unwrap_or(0);
                    registers.insert(field(at, "dst"), value);
                }
                CodeOp::StrU64 => {
                    assert_eq!(field(at, "base"), base, "a store not to the state");
                    let offset: usize = field(at, "offset").parse().expect("an offset");
                    state.insert(offset, registers[&field(at, "src")]);
                }
                CodeOp::Cmp => flags = (registers[&field(at, "lhs")], registers[&field(at, "rhs")]),
                CodeOp::CmpImm => {
                    flags = (
                        registers[&field(at, "lhs")],
                        field(at, "rhs").parse().expect("an immediate"),
                    )
                }
                CodeOp::MovImm => {
                    registers.insert(
                        field(at, "dst"),
                        field(at, "value").parse().expect("an immediate"),
                    );
                }
                CodeOp::AddImm => {
                    let sum = registers[&field(at, "src")]
                        + field(at, "imm").parse::<u64>().expect("an immediate");
                    registers.insert(field(at, "dst"), sum);
                }
                CodeOp::BranchEq => {
                    if flags.0 == flags.1 {
                        next = label(&field(at, "target"));
                    }
                }
                CodeOp::BranchNe => {
                    if flags.0 != flags.1 {
                        next = label(&field(at, "target"));
                    }
                }
                other => panic!("the size publisher emitted `{other:?}`, which this does not run"),
            }
            at = next;
        }
    }

    /// bug-970. A size the program already sees is not a resize, and the first publish
    /// is not special.
    ///
    /// A zero size word reads as the startup size (`canvas::getSize` answers 900 by 640
    /// before any window exists), so a window that opens AT that size publishes no
    /// change, and `canvas::didResize` must not answer TRUE for it. A window that opens
    /// at another size (Windows' first client area is 384 by 261) did change what the
    /// program sees, and counts.
    #[test]
    fn publishing_the_size_the_program_already_sees_counts_no_resize() {
        let (w, h) = (DEFAULT_SURFACE_WIDTH as u64, DEFAULT_SURFACE_HEIGHT as u64);
        let resizes =
            |state: &HashMap<usize, u64>| state.get(&GRAPHICS_OFFSET_RESIZES).copied().unwrap_or(0);

        let mut state = HashMap::new();
        publish(&mut state, w, h);
        assert_eq!(
            resizes(&state),
            0,
            "the first publish of the startup size is the size `getSize` already reported"
        );
        assert_eq!(state[&GRAPHICS_OFFSET_WIDTH], w);
        assert_eq!(state[&GRAPHICS_OFFSET_HEIGHT], h);
        publish(&mut state, w, h);
        assert_eq!(resizes(&state), 0, "a repeat of the size it has");
        // Real resizes count once each, one dimension or both.
        publish(&mut state, 640, h);
        assert_eq!(resizes(&state), 1, "the width changed");
        publish(&mut state, 640, 480);
        assert_eq!(resizes(&state), 2, "the height changed");
        publish(&mut state, 640, 480);
        assert_eq!(resizes(&state), 2, "a repeat of the size it has");
        publish(&mut state, w, h);
        assert_eq!(resizes(&state), 3, "back to the startup size IS a change");

        // A first publish at any other size is a change from what the program saw.
        for (width, height) in [(384, 261), (w, 261), (384, h)] {
            let mut state = HashMap::new();
            publish(&mut state, width, height);
            assert_eq!(resizes(&state), 1, "first publish of {width} by {height}");
        }
    }

    /// bug-479. The graphics thread ran eight bytes out of alignment on Windows for as
    /// long as canvas mode existed there, because this frame skipped the x86-64 realign
    /// on the (false) premise that `BaseThreadInitThunk` enters a start routine already
    /// 16-aligned. Nothing noticed until `SleepConditionVariableSRW`, which builds its
    /// wait block on the caller's stack and tags the pointer in its low 4 bits — eight
    /// bytes out, ntdll walked a garbage list and faulted with every argument correct.
    ///
    /// The invariant is the same on every x86-64 OS, which is the point: a thread entry
    /// is reached by a `call`, so `rsp % 16 == 8` on arrival, and a frame of `8 (mod 16)`
    /// is what puts the next `call` back on an aligned boundary.
    #[test]
    fn every_x86_64_graphics_trampoline_frame_realigns_the_stack() {
        for windows in [false, true] {
            let frame = graphics_trampoline_frame("x86_64", windows);
            assert_eq!(
                frame % 16,
                8,
                "x86_64 (windows={windows}) trampoline frame {frame} leaves every call \
                 the render loop makes misaligned by 8"
            );
        }
    }

    /// AArch64 is entered with a 16-aligned `sp` and the return address in `lr`, so it
    /// takes no realign — asserted so a future "just make it uniform" does not silently
    /// skew the Mac and the Linux arm64 boxes to fix Windows.
    #[test]
    fn aarch64_takes_no_realign() {
        assert_eq!(graphics_trampoline_frame("aarch64", false) % 16, 0);
    }

    /// The saves must sit ABOVE the shadow space, or a callee spilling its register
    /// arguments overwrites the saved return address and registers. The emitter
    /// stores `lr` at `shadow` and each saved register in the slot after it, so the
    /// frame has to hold all of them beyond it.
    #[test]
    fn the_frame_holds_every_save_above_the_shadow_space() {
        for (arch, windows) in [("x86_64", true), ("x86_64", false), ("aarch64", false)] {
            let frame = graphics_trampoline_frame(arch, windows);
            let shadow = graphics_trampoline_shadow(windows);
            let saves = 8 * (1 + graphics_trampoline_saved(arch, windows).len());
            assert!(
                frame >= shadow + saves,
                "{arch} (windows={windows}): frame {frame} cannot hold {saves} bytes of \
                 saves above its {shadow}-byte shadow space"
            );
        }
    }

    /// bug-982 audit: the entry saves every callee-saved register the MFB render
    /// loop may leave changed — on Win64 that includes `rdi`/`rsi`, which are
    /// argument registers inside MFB code and callee-saved to the thread library.
    #[test]
    fn win64_saves_the_internal_argument_registers_it_owes_the_thread_library() {
        use crate::target::shared::abi;
        let saved = graphics_trampoline_saved("x86_64", true);
        for register in [abi::mfb_arg(4), abi::mfb_arg(5), abi::mfb_arg(7)] {
            assert!(
                saved.contains(&register),
                "{register} is not saved: {saved:?}"
            );
        }
        assert!(!graphics_trampoline_saved("x86_64", false).contains(&abi::mfb_arg(4)));
    }

    /// Windows owes its callees 32 bytes; nothing else does.
    #[test]
    fn only_windows_reserves_shadow_space() {
        assert_eq!(graphics_trampoline_shadow(true), 32);
        assert_eq!(graphics_trampoline_shadow(false), 0);
    }
}
