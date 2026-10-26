//! plan-176-V: the graphics thread's entry — its trampoline, entry frame, the graphics-state
//! and profile data objects, and `emit_thread_entry`. The shared imports and `GraphicsScratch`
//! live in `runtime/canvas/mod.rs`.

use super::*;

/// The Win64 shadow space this trampoline owes its callees: 32 bytes on Windows, none
/// elsewhere. It sits at the BOTTOM of the frame, so the saves above it are out of reach
/// of a callee that spills its register arguments into it.
pub(crate) fn graphics_trampoline_shadow(windows: bool) -> usize {
    usize::from(windows) * 32
}

/// A canvas thread entry's frame size for `saved` saved registers, split out from the
/// emitter so the alignment invariant it exists to satisfy can be asserted directly
/// (bug-479).
///
/// **Every x86-64 thread entry is reached by a `call`** — `_pthread_start` on Unix and
/// `BaseThreadInitThunk` on Windows alike — so the routine begins at `rsp % 16 == 8`.
/// A callee must in turn be entered at `rsp % 16 == 8`, which means `rsp % 16 == 0`
/// immediately before the `call`. This trampoline only subtracts (it pushes nothing), so
/// the frame itself has to carry the odd 8: `frame % 16 == 8` is the whole requirement.
///
/// AArch64 needs no such thing — the entry `sp` is 16-aligned and the return address
/// arrives in `lr` rather than on the stack.
pub(crate) fn thread_entry_frame(arch: &str, windows: bool, saved: usize) -> usize {
    let realign = usize::from(arch == "x86_64") * 8;
    // `lr`, then one slot per saved register, rounded to the 16 every frame keeps.
    let saves = 8 * (1 + saved);
    saves.div_ceil(16) * 16 + realign + graphics_trampoline_shadow(windows)
}

/// The callee-saved registers the graphics thread entry saves for its caller, in
/// slot order above `lr` (bug-982 audit).
///
/// The caller is `_pthread_start` / `start_thread` / `BaseThreadInitThunk`, which
/// keeps its own state in the callee-saved registers across the call, and the
/// body runs compiled MFB code. MFB code preserves only the callee-saved registers
/// the allocator hands out; the rest of the bank is the convention's to change, and
/// every one of them must reach the thread library intact:
///
/// - the arena-state, current-thread and closure-environment registers MFB code
///   pins (`x19`/`x20`/`x28` on AArch64, `r15`/`rbx`/`r13` on x86-64; `rbx`/`r13`
///   are also the x86 homes of the hand-written runtime helpers' scratch, which
///   the render loop reaches);
/// - the 8th internal argument register, `rbp` on x86-64 (see the emitter);
/// - on Win64, `rdi`/`rsi`, the internal 5th and 6th argument registers, which
///   Win64 (unlike SysV) makes callee-saved.
///
/// The arch parameter is unused today because `canvas` needs app mode and app mode
/// has no RISC-V target; it stays in the signature beside `graphics_trampoline_frame`'s.
pub(crate) fn graphics_trampoline_saved(_arch: &str, windows: bool) -> Vec<abi::Reg> {
    let mut saved = vec![
        ARENA_STATE_REGISTER,
        abi::CURRENT_THREAD,
        CLOSURE_ENV_REGISTER,
        abi::mfb_arg(7),
    ];
    if windows {
        saved.extend([abi::mfb_arg(4), abi::mfb_arg(5)]);
    }
    saved
}

/// The trampoline `pthread_create` starts: establishes the MFB context, then loops.
pub(crate) const GRAPHICS_TRAMPOLINE_SYMBOL: &str = "_mfb_rt_canvas_graphics_entry";

/// The graphics thread's shared-state data object: all zero but
/// [`GRAPHICS_OFFSET_AUTO_WINDOW_MS`], which holds `auto_paint_ms` little-endian.
pub(crate) fn graphics_state_data_object(auto_paint_ms: u64) -> CodeDataObject {
    let mut bytes = vec![0u8; GRAPHICS_STATE_SIZE];
    bytes[GRAPHICS_OFFSET_AUTO_WINDOW_MS..GRAPHICS_OFFSET_AUTO_WINDOW_MS + 8]
        .copy_from_slice(&auto_paint_ms.to_le_bytes());
    let value: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    CodeDataObject {
        symbol: GRAPHICS_STATE_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "mfb.runtime.canvas_graphics.v1 { u64 started, pending; mutex; cond; \
                 u64 arenaState }"
            .to_string(),
        align: 8,
        size: GRAPHICS_STATE_SIZE,
        value,
    }
}

/// A `--debug` build's GPU frame profile: where a GPU frame's time goes inside the
/// backend's own renderer, which `phaseRenderMs` reports as one number.
///
/// [`CANVAS_PROFILE_WORDS`] cumulative 64-bit words, written by the renderer on the
/// graphics thread and read back on that same thread by `canvas::profileWord` for the
/// `MFB_CANVAS_STATS` line, so there is no lock and no ordering. Its own data object
/// and not words of the graphics state: a normal build has neither the object nor a
/// reader, and its graphics state is the size it always was.
///
/// Only the Metal renderer fills it (`emit_metal_draw` in the macOS app module, and
/// `emit_metal_layer_apply` for the display-sync word); on every other target the words
/// stay zero.
pub(crate) const CANVAS_PROFILE_SYMBOL: &str = "_mfb_rt_canvas_profile";
/// Nanoseconds preparing the frame's target: taking the window's drawable (and the
/// main-thread hop that sizes its layer). Zero for a frame that is read back.
pub(crate) const CANVAS_PROFILE_ACQUIRE_NS: usize = 0;
/// Nanoseconds writing the frame buffer: every item's block, edges, bands, stops,
/// dashes, and glyph and picture texels.
pub(crate) const CANVAS_PROFILE_FILL_NS: usize = 1;
/// Nanoseconds recording commands: the render pass and its bindings, one draw per
/// draw-list entry, and the copy into the drawable.
pub(crate) const CANVAS_PROFILE_ENCODE_NS: usize = 2;
/// Nanoseconds from submitting the command buffer to its completion being reported —
/// how long the graphics thread was blocked on the GPU.
pub(crate) const CANVAS_PROFILE_WAIT_NS: usize = 3;
/// Nanoseconds the GPU spent executing the command buffer, by the device's own clock.
pub(crate) const CANVAS_PROFILE_GPU_NS: usize = 4;
/// Nanoseconds copying the finished frame to the CPU surface and reordering its
/// channels. Zero for a frame presented straight to the window.
pub(crate) const CANVAS_PROFILE_READBACK_NS: usize = 5;
/// Nanoseconds presenting the drawable. Zero for a frame that is read back.
pub(crate) const CANVAS_PROFILE_PRESENT_NS: usize = 6;
/// Passes submitted. A frame is one pass unless it does not fit the frame buffer.
pub(crate) const CANVAS_PROFILE_PASSES: usize = 7;
/// Frame-buffer bytes written, summed over the regions' cursors at the end of each fill.
pub(crate) const CANVAS_PROFILE_BYTES: usize = 8;
/// Glyph and picture texels copied into the frame buffer.
pub(crate) const CANVAS_PROFILE_TEXELS: usize = 9;
/// Draw calls issued: one per draw-list entry that draws — a node-only row (repeat 0,
/// plan-175-C) issues none.
pub(crate) const CANVAS_PROFILE_DRAWS: usize = 10;
/// `1` once `MFB_CANVAS_VSYNC=0` has turned the window layer's display sync off, else
/// `0`. Not cumulative: the main thread's layer apply sets it once, when it first
/// configures the layer, before the frame that asked for the apply takes a drawable.
pub(crate) const CANVAS_PROFILE_VSYNC_OFF: usize = 11;
pub(crate) const CANVAS_PROFILE_WORDS: usize = 12;

/// The profile block, zeroed. Emitted only for a `--debug` build of a canvas program.
pub(crate) fn profile_data_object() -> CodeDataObject {
    CodeDataObject {
        symbol: CANVAS_PROFILE_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "mfb.runtime.canvas_profile.v1 { u64 acquireNs, fillNs, encodeNs, waitNs, \
                 gpuNs, readbackNs, presentNs, passes, bytes, texels, draws, vsyncOff }"
            .to_string(),
        align: 8,
        size: CANVAS_PROFILE_WORDS * 8,
        value: "00".repeat(CANVAS_PROFILE_WORDS * 8),
    }
}

/// `canvas::profileWord(index) AS Integer` — one word of the profile block, or `0` for
/// an index it does not have.
pub(crate) fn emit_profile_word(
    symbol: &str,
    scratch: &GraphicsScratch,
    index: &Operand,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let held = format!("{symbol}_profile_held");
    let done = format!("{symbol}_profile_done");
    instructions.push(abi::move_register(&scratch.scratch, index));
    // Unsigned, so a negative index is out of range too.
    instructions.push(abi::compare_immediate(
        &scratch.scratch,
        &CANVAS_PROFILE_WORDS.to_string(),
    ));
    instructions.push(abi::branch_lo(&held));
    instructions.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    instructions.push(abi::branch(&done));
    instructions.push(abi::label(&held));
    push_symbol_address(
        symbol,
        CANVAS_PROFILE_SYMBOL,
        &scratch.base,
        instructions,
        relocations,
    );
    instructions.push(abi::shift_left_immediate(
        &scratch.scratch,
        &scratch.scratch,
        3,
    ));
    instructions.push(abi::add_registers(
        &scratch.base,
        &scratch.base,
        &scratch.scratch,
    ));
    instructions.push(abi::load_u64(RESULT_VALUE_REGISTER, &scratch.base, 0));
    instructions.push(abi::label(&done));
}

/// The MFBASIC render loop's native symbol.
///
/// Derived from the helper's own name rather than hard-coded, so a rename of
/// `__canvas_renderLoop` breaks the build here instead of producing a trampoline
/// that jumps to a symbol nothing defines.
pub(crate) fn render_loop_symbol() -> String {
    crate::target::shared::nir::function_symbol(&crate::internal_name::internalize(
        "__canvas_renderLoop",
    ))
}

/// `_mfb_rt_canvas_graphics_entry` — the graphics thread's first instruction.
///
/// The thread argument (`c_arg(0)`) is the child arena-state block the spawner
/// allocated and zeroed. This pins it into the arena-state register, runs the
/// module's `LINK` and global initializers **on this thread** — which is what gives
/// the loop its own geometry cache and, critically, a populated `__COLOR_SRGB` (the
/// sRGB transfer table, in the `color` package since plan-122-B) — and then enters
/// `__canvas_renderLoop`, which never returns.
///
/// The loop returns when shutdown asks it to (`canvas::waitForRedraw` reports
/// `FALSE`), and this returns straight after — which is how a pthread entry exits,
/// and what the shutdown join is waiting for.
pub(crate) fn emit_graphics_trampoline(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    arena_init: ArenaInitSymbols,
) -> Result<CodeFunction, String> {
    let from = GRAPHICS_TRAMPOLINE_SYMBOL;
    let _ = platform_imports;
    let windows = platform.family() == PlatformFamily::Windows;
    Ok(emit_thread_entry(
        from,
        "canvas.graphics_entry",
        platform,
        graphics_trampoline_saved(platform.arch(), windows),
        |instructions, relocations| {
            // Pin the child arena state. Every MFB global access on this thread — including
            // the geometry cache and the sRGB table — is addressed off it.
            instructions.push(abi::move_register(ARENA_STATE_REGISTER, abi::c_arg(0)));

            // The two initializers, in the order the entry runs them (bug-369): `LINK` first,
            // because a global's initializer may call a `LINK` symbol.
            for symbol in [arena_init.link_init, arena_init.global_init]
                .into_iter()
                .flatten()
            {
                instructions.push(abi::branch_link(symbol));
                relocations.push(internal_branch(from, symbol));
            }

            let render_loop = render_loop_symbol();
            instructions.push(abi::branch_link(&render_loop));
            relocations.push(internal_branch(from, &render_loop));
            // bug-806: this thread ran the module's global initializer into its own
            // instance; close what that instance's module-level bindings own.
            if let Some(symbol) = arena_init.teardown {
                instructions.push(abi::branch_link(symbol));
                relocations.push(internal_branch(from, symbol));
            }
        },
    ))
}

/// A thread start routine: the frame every canvas thread entry needs, around `body`.
///
/// The graphics thread's entry and (bug-996) the Vulkan builder's are both called by
/// the thread library, so they owe it the same things: the stack alignment a `call`
/// leaves, Windows' shadow space, and every register in `saved` handed back as it
/// arrived. Which registers that is depends on what the body runs, so the caller says
/// ([`graphics_trampoline_saved`], `vulkan_handoff::builder_entry_saved`). `body` runs
/// with the thread argument still in `c_arg(0)`; the routine returns 0.
pub(crate) fn emit_thread_entry(
    from: &str,
    name: &str,
    platform: &dyn CodegenPlatform,
    saved: Vec<abi::Reg>,
    body: impl FnOnce(&mut Vec<CodeInstruction>, &mut Vec<CodeRelocation>),
) -> CodeFunction {
    let mut instructions = Vec::new();
    let mut relocations = Vec::new();
    // **The x86-64 stack realign.** Every x86-64 thread library reaches a
    // start-routine through a `call`, so the routine begins at `sp % 16 == 8` — the
    // return address has been pushed and nothing has re-aligned. A frame that is a
    // multiple of 16 therefore leaves every call this trampoline makes misaligned,
    // and the first callee that uses an aligned SSE store faults. It does not fault
    // here: it faults far away, inside `calloc` under `g_idle_add`, as heap
    // corruption in a thread whose own frames are all correct.
    //
    // `lower_thread_trampoline` has carried this same +8 since bug-408, box-proven on
    // both libcs (glibc 2228: the 88-byte frame runs, the 80-byte frame SIGSEGVs 5/5;
    // musl 2227: 88 runs, 96 SIGSEGVs 5/5). The graphics thread is a second
    // start-routine and needs it for exactly the same reason — it did not have it,
    // which is why canvas mode segfaulted on Linux from the moment plan-98-D moved
    // rendering onto a thread. AArch64 takes no realign: `pthread_create` enters with
    // a 16-aligned `sp` and the return address in `lr`.
    //
    // **Windows takes the realign too — measured, not assumed.** This comment used to
    // claim `BaseThreadInitThunk` enters a `CreateThread` start routine ALREADY
    // 16-aligned, and skipped the `+8` on that basis. It is false: the thunk reaches
    // the start routine through an ordinary `call`, so the routine begins at
    // `rsp % 16 == 8` exactly like `_pthread_start`.
    //
    // The skew is invisible until something on the graphics thread calls a Win32
    // function that cares. `SleepConditionVariableSRW` cares, because ntdll builds its
    // wait block on the caller's stack and tags the pointer in the block's low 4 bits
    // (`and rdx,0FFFFFFFFFFFFFFF0h` at `RtlSleepConditionVariableSRW+0x13d`). Eight
    // bytes out, that mask yields a pointer into the middle of the block, the wait-list
    // walk below it loads a NULL `Next`, and `mov [rcx+10h],rax` faults with `rcx=0` —
    // with every argument correct, on the very first wait. That is bug-479.
    //
    // Measured on box 2230 with a breakpoint on `ntdll!RtlAcquireSRWLockExclusive`
    // printing `@rsp` per thread, both acquiring the SAME canvas mutex:
    //
    //     worker   t=1728  rsp=0x332de68  -> % 16 == 8   correct
    //     graphics t=1a28  rsp=0x3b2fd30  -> % 16 == 0   skewed
    //
    // The worker is right because its entry pushes a register before its frame; this
    // trampoline only subtracts, so the frame itself has to carry the odd 8. And
    // because a body's alignment is its caller's call site, the skew was inherited by
    // every Win32 call the render loop ever made (bug-478 is the same mistake in
    // `win_x86_64/app/mod.rs`).
    //
    // Windows also needs the callee's **shadow space**: 32 bytes at the bottom of this
    // frame that any callee may spill its register arguments into. The shared
    // `finalize_frame` reserves it for every *allocated* function
    // (`outgoing_args_base_offset`), but this trampoline is hand-built and gets none —
    // so without it the saves below sit exactly where a callee is entitled to write.
    let windows = platform.family() == PlatformFamily::Windows;
    let shadow = graphics_trampoline_shadow(windows);
    let frame = thread_entry_frame(platform.arch(), windows, saved.len());
    instructions.push(abi::label("entry"));
    instructions.push(abi::subtract_stack(frame));
    instructions.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        shadow,
    ));
    // **Save the arena register.** It is callee-saved, and the caller here is
    // `_pthread_start`, which has its own live state in it — clobbering it corrupts
    // pthread's frame and the thread dies at exit inside `_pthread_terminate` with a
    // pointer-authentication failure, having run the whole body correctly first. The
    // thread runtime's own trampoline saves the same register for the same reason;
    // "a thread entry has nobody to return to" is wrong, because pthread is the
    // caller.
    // (Every register `graphics_trampoline_saved` lists is stored below, after
    // the two that cost the most to find.)
    //
    // **Save the 8th internal argument register too.**
    //
    // MFBASIC functions take up to 8 parameters and AArch64 has 8 argument
    // registers, but SysV x86-64 has only six — so the internal convention extends
    // the list with `rax` and `rbp` for arguments 7 and 8 (bug-296,
    // `CALL_ARGS` in `x86_64/select.rs`). That is self-consistent between MFB
    // caller and MFB callee, and wrong at a boundary: `rbp` is CALLEE-saved under
    // SysV, and the caller here is glibc's `start_thread`, which keeps its own frame
    // pointer in it across the call to this routine.
    //
    // So any MFB function that stages an 8th argument destroys it. `__canvas_
    // drawGeometry` does exactly that, six times, calling `__canvas_geoDistance`:
    //
    //     mov r8, r10 / mov r9, r10 / mov rax, r10 / mov rbp, r10
    //     bl _mfb_ifn_canvas_5FgeoDistance
    //
    // and its own frame saves only `["r12", "r14", "lr"]`, because the frame's
    // callee-saved set is computed from ALLOCATED registers and this `rbp` is an
    // ABI-staged one the allocator never assigned.
    //
    // On AArch64 the 8th argument is `x7`, which is caller-saved, so saving it here
    // costs one store and changes nothing.
    //
    // Measured on an x86_64 ubuntu-24.04 runner under gdb: the graphics thread
    // returns with `rbp = 0x404e000000000000`, which is not a pointer at all — it is
    // the double `60.0`, the `radius` of the circle in the scene being drawn.
    // `start_thread` then executes `mov -0x98(%rbp),%rax` and dies with SIGBUS. That
    // is the whole of the canvas failure on the x86_64 Linux rows: 68 of ~90 canvas
    // tests, 55 SIGBUS and 13 SIGSEGV, the two alternating exactly as one wild
    // address does depending on where it lands.
    //
    // It reproduces on no machine I can reach — boxes 2228/2227/2223 and an
    // ubuntu:24.04 container with the same GTK run the same binaries clean, dozens
    // of times — because whether a clobbered `rbp` is ever *dereferenced* depends on
    // the libc's own code path after the start routine returns.
    for (index, register) in saved.iter().enumerate() {
        instructions.push(abi::store_u64(
            *register,
            abi::stack_pointer(),
            shadow + 8 + 8 * index,
        ));
    }
    body(&mut instructions, &mut relocations);

    // Return, which is how a pthread entry exits. It used to park here on the
    // theory that a render loop returning was a bug — but shutdown asks it to
    // return, and parking made the shutdown join wait forever on a thread that
    // was spinning two instructions away from finishing.
    instructions.push(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    // Every saved register, the arena register (`r15` on x86-64) and the 8th
    // argument register (`rbp`) among them: all must reach `start_thread` intact.
    for (index, register) in saved.iter().enumerate().rev() {
        instructions.push(abi::load_u64(
            *register,
            abi::stack_pointer(),
            shadow + 8 + 8 * index,
        ));
    }
    instructions.push(abi::load_u64(
        abi::link_register(),
        abi::stack_pointer(),
        shadow,
    ));
    instructions.push(abi::add_stack(frame));
    instructions.push(abi::return_());

    CodeFunction {
        name: name.to_string(),
        symbol: from.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    }
}
