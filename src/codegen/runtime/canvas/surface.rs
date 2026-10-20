//! plan-176-V: the surface — its startup size, pointer and size publication, display rate and
//! override, resize detection and the GPU mode. The shared imports and `GraphicsScratch` live
//! in `runtime/canvas/mod.rs`.

use super::*;

/// The surface's startup size, and the value a zero field reads as.
///
/// Matches what all three platform surfaces are created at
/// (`emit_reconcile_canvas_helper`, `RECONCILE_BUILD_SYMBOL`, the Win32
/// `CreateWindowExW`), so a program that never resizes is unaffected by the resize
/// path existing.
pub(crate) const DEFAULT_SURFACE_WIDTH: usize = 900;
pub(crate) const DEFAULT_SURFACE_HEIGHT: usize = 640;

/// One of the surface's two dimensions: where its word is in the graphics state, and
/// what a zero there reads as.
#[derive(Clone, Copy)]
pub(crate) enum SurfaceDimension {
    Width,
    Height,
}

impl SurfaceDimension {
    fn offset(self) -> usize {
        match self {
            SurfaceDimension::Width => GRAPHICS_OFFSET_WIDTH,
            SurfaceDimension::Height => GRAPHICS_OFFSET_HEIGHT,
        }
    }

    fn startup(self) -> usize {
        match self {
            SurfaceDimension::Width => DEFAULT_SURFACE_WIDTH,
            SurfaceDimension::Height => DEFAULT_SURFACE_HEIGHT,
        }
    }

    /// Load this dimension of the graphics state at `base` into `dst`, as the program
    /// sees it: a zero word is the startup size.
    ///
    /// The ONE reading of a size word. `canvas::getSize` answers through it
    /// ([`emit_surface_dimension`]) and the resize publisher decides "did the size
    /// change" through it ([`emit_publish_surface_size_tagged`]), so the two cannot
    /// give 0 different meanings (bug-970). `ready` is a label the caller owns.
    fn emit_load(
        self,
        dst: abi::Reg,
        base: abi::Reg,
        ready: &str,
        instructions: &mut Vec<CodeInstruction>,
    ) {
        instructions.push(abi::load_u64(dst, base, self.offset()));
        instructions.push(abi::compare_immediate(dst, "0"));
        instructions.push(abi::branch_ne(ready));
        instructions.push(abi::move_immediate(
            dst,
            "Integer",
            &self.startup().to_string(),
        ));
        instructions.push(abi::label(ready));
    }
}

/// Read one published surface dimension, falling back to the startup size.
pub(crate) fn emit_surface_dimension(
    symbol: &str,
    scratch: &GraphicsScratch,
    dimension: SurfaceDimension,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let ready = format!("{symbol}_dim_ready_{}", dimension.offset());
    state_base(symbol, &scratch.base, instructions, relocations);
    dimension.emit_load(
        (&scratch.scratch).into(),
        (&scratch.base).into(),
        &ready,
        instructions,
    );
    instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &scratch.scratch));
}

/// plan-172-K, trigger 6: branch to `skip` unless the scene the graphics thread last
/// drew holds a `canvas::AutoPaint` ([`GRAPHICS_OFFSET_AUTOPAINT_LIVE`]).
///
/// Every platform's pointer handler emits this after its publish and then calls
/// `canvas::signalRedraw` with 1 in the first argument register — "the pointer
/// changed", not "a redraw is owed" ([`emit_signal_redraw`]) — so a pointer change
/// wakes the render loop exactly when something in the scene could change its paint
/// for it. `base` is overwritten. It is not part of
/// [`emit_publish_pointer`] because the signal is a call — it takes the graphics mutex
/// — and that writer is emitted into leaf functions with no frame.
pub(crate) fn emit_skip_unless_autopaint_live(
    symbol: &str,
    base: impl Into<abi::Reg>,
    skip: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let base: abi::Reg = base.into();
    state_base(symbol, base, instructions, relocations);
    instructions.push(abi::load_u64(base, base, GRAPHICS_OFFSET_AUTOPAINT_LIVE));
    instructions.push(abi::compare_immediate(base, "0"));
    instructions.push(abi::branch_eq(skip));
}

/// The argument each call to `canvas::signalRedraw` in `instructions` is made with, in
/// order: the immediate moved into the first argument register by the instruction
/// right before the call. A call with anything else in front of it is a panic — a
/// platform handler that signals without saying which kind of signal it is.
#[cfg(test)]
pub(crate) fn signal_redraw_arguments(instructions: &[CodeInstruction]) -> Vec<String> {
    let first_argument = abi::Reg::from(abi::mfb_arg(0)).to_string();
    let mut out = Vec::new();
    for (index, instruction) in instructions.iter().enumerate() {
        if instruction.get("target").as_deref() != Some("_mfb_rt_canvas_canvas_signalRedraw") {
            continue;
        }
        let before = index
            .checked_sub(1)
            .map(|at| &instructions[at])
            .expect("a signal is never a function's first instruction");
        assert_eq!(
            before.get("dst").as_deref(),
            Some(first_argument.as_str()),
            "canvas::signalRedraw is called with its argument register unset"
        );
        out.push(before.get("value").expect("an immediate argument"));
    }
    out
}

/// The registers [`emit_publish_pointer`] is given: the four values to publish, and
/// three it may overwrite. A struct because every caller is a hand-written platform
/// handler that spells its own registers, and seven positional registers of one type
/// is seven chances to swap two.
pub(crate) struct PointerRegs {
    pub(crate) x: abi::Reg,
    pub(crate) y: abi::Reg,
    pub(crate) buttons: abi::Reg,
    pub(crate) inside: abi::Reg,
    /// Overwritten: the graphics-state base.
    pub(crate) base: abi::Reg,
    /// Overwritten: the sequence word's value.
    pub(crate) seq: abi::Reg,
    /// Overwritten: an address, where the stores are store-releases.
    pub(crate) addr: abi::Reg,
}

/// Publish the pointer from a platform's pointer event (plan-172-J).
///
/// Called on the **main** thread — or, for the debug script, on the graphics thread
/// through `canvas::publishPointer`; never both at once in a run that means anything —
/// with the four values in registers. One writer at a time, so the sequence word is a
/// plain increment, not a compare-and-swap: odd, the four stores, even.
///
/// `ordered` is whether the target needs the accesses ordered explicitly
/// (`scene_base::orders_scene_accesses`: AArch64). There every store is a
/// store-release, which is visible only after every earlier access of this thread —
/// so no data word can be seen before the odd mark, nor the even mark before the data
/// — and the reader's loads are load-acquires. x86-64 keeps stores in order and loads
/// in order by itself.
///
/// Makes no call and touches no register outside [`PointerRegs`]. Signals nothing: a
/// pointer change is a redraw only while the scene holds a `canvas::AutoPaint`, and the
/// caller decides that ([`emit_skip_unless_autopaint_live`]).
pub(crate) fn emit_publish_pointer(
    symbol: &str,
    ordered: bool,
    regs: &PointerRegs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let store = |value: abi::Reg, offset: usize, ins: &mut Vec<CodeInstruction>| {
        if ordered {
            ins.push(abi::add_immediate(regs.addr, regs.base, offset));
            ins.push(abi::store_release_u64(value, regs.addr));
        } else {
            ins.push(abi::store_u64(value, regs.base, offset));
        }
    };
    state_base(symbol, regs.base, instructions, relocations);
    instructions.push(abi::load_u64(
        regs.seq,
        regs.base,
        GRAPHICS_OFFSET_POINTER_SEQ,
    ));
    instructions.push(abi::add_immediate(regs.seq, regs.seq, 1));
    store(regs.seq, GRAPHICS_OFFSET_POINTER_SEQ, instructions);
    for (value, offset) in [regs.x, regs.y, regs.buttons, regs.inside]
        .into_iter()
        .zip(GRAPHICS_POINTER_WORDS)
    {
        store(value, offset, instructions);
    }
    instructions.push(abi::add_immediate(regs.seq, regs.seq, 1));
    store(regs.seq, GRAPHICS_OFFSET_POINTER_SEQ, instructions);
}

/// Publish a new surface size from the platform's resize event.
///
/// Called on the **main** thread with the new dimensions already in the two given
/// registers. Two plain aligned stores, no lock: the graphics thread reads them at
/// frame start and a torn pair would at worst render one frame at a mixed size,
/// which the next frame corrects — whereas taking the render mutex from a resize
/// callback would block the UI thread behind a frame.
///
/// The caller signals a redraw afterwards; publishing without signalling would leave
/// the new size unused until something else asked for a frame.
pub(crate) fn emit_publish_surface_size(
    symbol: &str,
    base: impl Into<abi::Reg>,
    width: impl Into<abi::Reg>,
    height: impl Into<abi::Reg>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    emit_publish_surface_size_tagged(symbol, "", base, width, height, instructions, relocations);
}

/// [`emit_publish_surface_size`] for a function that publishes from more than one
/// site: `tag` keeps each site's labels distinct (bug-688 V6, the Windows window
/// procedure publishes from `WM_SIZE` and from the canvas reconcile).
pub(crate) fn emit_publish_surface_size_tagged(
    symbol: &str,
    tag: &str,
    base: impl Into<abi::Reg>,
    width: impl Into<abi::Reg>,
    height: impl Into<abi::Reg>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let base: abi::Reg = base.into();
    let height: abi::Reg = height.into();
    let width: abi::Reg = width.into();
    let unchanged = format!("{symbol}{tag}_resize_same");
    let changed = format!("{symbol}{tag}_resize_changed");
    state_base(symbol, base, instructions, relocations);
    // Count the resize only when the size actually changed. Every platform re-publishes
    // on events that are not resizes — AppKit sends `setFrameSize:` with the size it
    // already has, and the headless scripted resize does the same — and
    // `canvas::didResize` promises the program a *change*, not an event.
    //
    // "Changed" is against the size the program SEES, which is the startup size while
    // the words are still zero (bug-970): a window that opens at that size publishes
    // what `canvas::getSize` already answered, and one that opens at another size
    // publishes a change. Nothing here knows whether a publish is the first.
    //
    // The compare reads the old values before the stores overwrite them, and uses the
    // scratch the caller already gave us rather than a fifth register: this runs in a
    // platform resize callback, where the register budget is whatever the caller had.
    SurfaceDimension::Width.emit_load(
        SCRATCH_RESIZE,
        base,
        &format!("{symbol}{tag}_resize_old_width"),
        instructions,
    );
    instructions.push(abi::compare_registers(SCRATCH_RESIZE, width));
    instructions.push(abi::branch_ne(&changed));
    SurfaceDimension::Height.emit_load(
        SCRATCH_RESIZE,
        base,
        &format!("{symbol}{tag}_resize_old_height"),
        instructions,
    );
    instructions.push(abi::compare_registers(SCRATCH_RESIZE, height));
    instructions.push(abi::branch_eq(&unchanged));
    instructions.push(abi::label(&changed));
    instructions.push(abi::load_u64(SCRATCH_RESIZE, base, GRAPHICS_OFFSET_RESIZES));
    instructions.push(abi::add_immediate(SCRATCH_RESIZE, SCRATCH_RESIZE, 1));
    instructions.push(abi::store_u64(
        SCRATCH_RESIZE,
        base,
        GRAPHICS_OFFSET_RESIZES,
    ));
    instructions.push(abi::label(&unchanged));
    instructions.push(abi::store_u64(width, base, GRAPHICS_OFFSET_WIDTH));
    instructions.push(abi::store_u64(height, base, GRAPHICS_OFFSET_HEIGHT));
}

/// Publish the display's refresh rate, in millihertz, from the platform's main thread
/// (plan-155-A). `mhz` holds the value; `base` is a register the caller can spare.
///
/// One plain aligned store and nothing else — no lock, for `emit_publish_surface_size`'s
/// reason, and **no redraw**: a new rate changes no pixel, and `.ai/canvas-threading.md`
/// §4's triggers stay five. Readers want "some recent value", and a store of a word is
/// never torn.
///
/// Takes operands rather than register names so a caller can pass an ABI role token
/// (the Windows helper uses the Win64 argument registers, spelled `abi::mfb_arg(n)`):
/// rendering such a token to a string loses the role the selector realizes it from.
pub(crate) fn emit_publish_display_rate(
    symbol: &str,
    base: impl Into<abi::Reg>,
    mhz: impl Into<abi::Reg>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let base: abi::Reg = base.into();
    push_symbol_address(
        symbol,
        GRAPHICS_STATE_SYMBOL,
        &base,
        instructions,
        relocations,
    );
    instructions.push(abi::store_u64(mhz, &base, GRAPHICS_OFFSET_DISPLAY_MHZ));
}

/// `canvas::displayMilliHz() AS Integer` — the rate, override first; `0` when unknown.
pub(crate) fn emit_display_milli_hz(
    symbol: &str,
    scratch: &GraphicsScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let chosen = format!("{symbol}_display_override");
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_DISPLAY_OVERRIDE,
    ));
    instructions.push(abi::compare_immediate(&scratch.scratch, "0"));
    instructions.push(abi::branch_ne(&chosen));
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_DISPLAY_MHZ,
    ));
    instructions.push(abi::label(&chosen));
    instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &scratch.scratch));
}

/// `canvas::setDisplayOverride(mhz)` — record `MFB_CANVAS_DISPLAY_HZ`, read from
/// MFBASIC beside `setSyncMode` for the same reason: the environment is portably
/// readable there.
pub(crate) fn emit_set_display_override(
    symbol: &str,
    scratch: &GraphicsScratch,
    value: &Operand,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::store_u64(
        value,
        &scratch.base,
        GRAPHICS_OFFSET_DISPLAY_OVERRIDE,
    ));
}

/// The one scratch register `emit_publish_surface_size` needs for its compare.
///
/// Named here rather than taken as a parameter because every caller is a hand-written
/// platform resize callback that already spells its own registers, and adding a fifth
/// argument to all of them to pass a register they all have spare is churn. `SCRATCH[5]`
/// is above the four each caller stages its own values in.
const SCRATCH_RESIZE: abi::Reg = abi::SCRATCH[5];

/// `canvas::didResize()` — has the surface changed size since this was last asked?
///
/// Read-and-acknowledge: the worker compares the platform's resize counter against the
/// value it last reported and, if they differ, records the new one and answers TRUE. So
/// the answer is TRUE exactly once per resize however many resizes happened in between,
/// and no lock is involved — the two words have one writer each.
pub(crate) fn emit_did_resize(
    symbol: &str,
    scratch: &GraphicsScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let same = format!("{symbol}_resize_seen");
    let done = format!("{symbol}_resize_done");
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_RESIZES,
    ));
    instructions.push(abi::load_u64(
        &scratch.arena,
        &scratch.base,
        GRAPHICS_OFFSET_RESIZES_SEEN,
    ));
    instructions.push(abi::compare_registers(&scratch.scratch, &scratch.arena));
    instructions.push(abi::branch_eq(&same));
    instructions.push(abi::store_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_RESIZES_SEEN,
    ));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    instructions.push(abi::branch(&done));
    instructions.push(abi::label(&same));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    instructions.push(abi::label(&done));
}

/// `canvas::setGpuMode(on)` — record whether a GPU renderer was asked for.
///
/// Read from MFBASIC at first present, next to `setSyncMode`, for the same reason:
/// the environment is portably readable there.
pub(crate) fn emit_set_gpu_mode(
    symbol: &str,
    scratch: &GraphicsScratch,
    value: &Operand,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::store_u64(value, &scratch.base, GRAPHICS_OFFSET_GPU));
}

/// `canvas::useGpu()` — did the program ask for a GPU renderer?
///
/// The flag and nothing else, on every target. It used to hard-return FALSE off
/// macOS, back when it was named `useMetal` and meant "is Metal selected"; by the
/// time a second backend existed that early return silently made `MFB_CANVAS_GPU=1`
/// a no-op on Linux (plan-98-F Correction 4). "Was a GPU asked for" and "is one
/// usable here" are different questions, and `canvas::metalReady` /
/// `canvas::vulkanReady` answer the second.
pub(crate) fn emit_use_gpu(
    symbol: &str,
    scratch: &GraphicsScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    state_base(symbol, &scratch.base, instructions, relocations);
    instructions.push(abi::load_u64(
        &scratch.scratch,
        &scratch.base,
        GRAPHICS_OFFSET_GPU,
    ));
    instructions.push(abi::move_register(RESULT_VALUE_REGISTER, &scratch.scratch));
}
