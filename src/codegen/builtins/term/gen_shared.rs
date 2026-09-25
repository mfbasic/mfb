//! Native code generation for the built-in `term` package (clean-room registry
//! migration).
//!
//! `term` is a structured-terminal-surface package: cursor / color / attribute
//! control, screen clearing, box / line / fill drawing, glyph / text stamping,
//! size / resize queries, and the full-screen TUI-mode toggle. Every member lowers
//! to a runtime helper whose body is the heavy console terminal emitter
//! (`code::lower_term_helper`) or, in an `--app` build, the platform's synthesized
//! `TermView` surface (`CodegenPlatform::emit_app_term_helper`).
//!
//! Each of the 26 members owns its `Body::abi_function` body (`lower_<name>`) in its
//! own `func_*.rs`: the `abi_function` wrapper seeds the entry label, binds the
//! incoming ABI argument registers, and finalizes; the body calls the one
//! genuinely-shared family-generic [`lower_term_helper`] with its own runtime-call
//! name, which branches app-vs-console off the
//! [`AbiCtx`](crate::codegen::registry::AbiCtx) (`build_mode` / `term_state_offset` /
//! `presentation_mode_offset`) and appends the pre-finalize [`TermBodyParts`] the
//! wrapper finalizes. `term` has no posix/win difference and no `os_aliases`.
//!
//! The heavy emitters stay in the shared code layer (like the `strings` / `vector`
//! codegen carriers — relocating the 150 KB of terminal-grid emission would be
//! byte-identity-risky): `code::lower_term_helper` (the console backend),
//! `CodegenPlatform::emit_app_term_helper` (the per-platform app backend, now
//! append-shaped), and the cross-package `code::prepend_wrong_mode_gate` (`app`
//! owns `Mode`, so the gate stays shared), reached here through the `code`
//! re-exports.

// --- codegen tier imports (migration) ---
use crate::codegen::app::hook::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::term::core::lower_term_helper as console_lower_term_helper;
use crate::types::ParameterType;
use std::collections::HashMap;

/// The `(instructions, relocations, stack_size)` a `term` OS-seam body emits before
/// the `abi_function` wrapper finalizes it — the successor to the finalized
/// `HelperResult` tuple (see `net`'s `NetBodyParts` / `tls`'s `TlsBodyParts`).
/// `stack_size` is the sp-relative locals region the body reserves; the wrapper
/// passes it to `finalize_vreg_body_with_locals`, byte-identical to the console
/// body's former self-finalize.
pub(crate) type TermBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The `void` result every native `term.*` member returns from its per-member
/// `abi_function` body: every term body emits its own fallible ABI, so the wrapper
/// appends no epilogue. `type_` is `Nothing`; `text` carries the runtime-call name.
pub(crate) fn void_result(call: &str) -> ValueResult {
    ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: call.to_string(),
    }
}

/// The single family-generic OS-seam entry for every `term::` member — reached from
/// the shared [`lower_term_os_seam`] `abi_function` body. Branches app-vs-console off
/// the per-compilation build mode and returns the pre-finalize [`TermBodyParts`] the
/// wrapper finalizes.
///
/// App mode drives the synthesized TermView surface (plan-01-term.md §6.3):
/// `emit_app_term_helper` appends EVERY term:: helper it implements — the mode toggle
/// plus clear/sync/moveTo/color/attr/cursor/size — into the caller's stream. It falls
/// through to the shared console backend when the platform returns `None` (a call the
/// app surface keeps on the console backend, or a non-app build).
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_term_helper(
    call: &str,
    symbol: &str,
    term_state_offset: Option<usize>,
    presentation_mode_offset: Option<usize>,
    // plan-94-B: `Some` only for a program that uses `enableMouse`/`pollMouse`.
    // The two mouse members demand it; every other member ignores it, which is why
    // it is threaded rather than folded into `term_state_offset` — a `term::`
    // program that never touches the mouse reserves no mouse region at all.
    mouse_state_offset: Option<usize>,
    build_mode: crate::target::NativeBuildMode,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<TermBodyParts, String> {
    let term_state_offset = term_state_offset
        .ok_or_else(|| format!("native code plan emits '{symbol}' without reserving term state"))?;

    if build_mode.is_app() {
        let mut instructions: Vec<CodeInstruction> = Vec::new();
        let mut relocations: Vec<CodeRelocation> = Vec::new();
        if let Some(result) = platform.emit_app_term_helper(
            call,
            symbol,
            term_state_offset,
            &mut instructions,
            &mut relocations,
        ) {
            result?;
            // plan-62-E: every app-mode `term::` helper (including `on`) is gated on the
            // `Console` presentation mode — outside it, `term::` raises the trappable
            // `ErrWrongMode` before touching the (absent) grid. No-op when the program
            // cannot leave `Console` (`presentation_mode_offset` is `None`), so a program
            // that never uses `app::` is unchanged.
            //
            // plan-98-A Phase 2 relaxed the *console-read `io::*`* gate to "any mode with
            // a window", but `term::` deliberately keeps `ModeRequirement::Console`: it
            // needs the transcript view's character grid, which `Canvas` does not have —
            // a canvas surface is pixels, not cells. So `term::` traps in `Canvas` too.
            prepend_wrong_mode_gate(
                &mut instructions,
                &mut relocations,
                symbol,
                presentation_mode_offset,
                ModeRequirement::Console,
            );
            // Reserve exactly the sp-relative scratch the platform body actually
            // addresses. This used to hard-code 0 on the belief that "the app
            // bodies hold every cross-call value in allocator vregs (no
            // addressable stack scratch)" — untrue on Win64, where the appended
            // (frameless) bodies name raw slots: `term::on` stores hdcScreen at
            // `sp+0x70`, and draw_text/draw_box/fill_rect/draw_line/draw_glyph_at
            // each carry their own slot set. With 0 declared locals those stores
            // landed above the finalized frame, in the caller's — the exact
            // bug-360 shape. Measuring the emitted body keeps the reservation and
            // the offsets from drifting again, and is 0 (byte-identical) for the
            // macOS/Linux bodies, which genuinely use no sp scratch.
            let locals =
                crate::codegen::engine::util::vreg_frame::required_sp_local_bytes(&instructions);
            return Ok((instructions, relocations, locals));
        }
    }

    let parts = console_lower_term_helper(
        call,
        symbol,
        term_state_offset,
        mouse_state_offset,
        build_mode.is_app(),
        platform_imports,
        platform,
    );
    // plan-94-B, closing the §4.5 Open Decision for `term::`: the two mouse
    // members are `Console`-gated in an app build, like every other `term::`
    // member.
    //
    // They reach this fall-through because no backend's `emit_app_term_helper`
    // claims them (plan-94-C/D/E add those arms), so the gate `lower_term_helper`
    // normally prepends when a backend returns `Some` never runs for them. Left
    // ungated, `term::pollMouse` in `Mode.Canvas` would answer "where is the mouse,
    // in cells" about a surface that has no cells — and would do it silently, which
    // is worse than the trap its siblings raise. A canvas program asks
    // `canvas::pollMouse`, which is `Canvas`-gated for the mirror-image reason.
    //
    // A no-op outside app builds (`presentation_mode_offset` is `None`), so a
    // console program is untouched.
    if build_mode.is_app() && matches!(call, "term.enableMouse" | "term.pollMouse") {
        let (mut instructions, mut relocations, locals) = parts?;
        prepend_wrong_mode_gate(
            &mut instructions,
            &mut relocations,
            symbol,
            presentation_mode_offset,
            ModeRequirement::Console,
        );
        return Ok((instructions, relocations, locals));
    }
    parts
}
