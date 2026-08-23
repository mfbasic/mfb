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
//! The 24 members share one `Body::abi_function` body, [`lower_term_os_seam`]: the
//! `abi_function` wrapper seeds the entry label, binds the incoming ABI argument
//! registers, and finalizes; this body dispatches by the runtime-call name in
//! [`AbiCtx::call`] to the family-generic [`lower_term_helper`], which branches
//! app-vs-console off the [`AbiCtx`](crate::codegen::registry::AbiCtx)
//! (`build_mode` / `term_state_offset` / `presentation_mode_offset`) and appends
//! the pre-finalize [`TermBodyParts`] the wrapper finalizes. `term` has no
//! posix/win difference and no `os_aliases`.
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
use std::collections::HashMap;

/// The `(instructions, relocations, stack_size)` a `term` OS-seam body emits before
/// the `abi_function` wrapper finalizes it — the successor to the finalized
/// `HelperResult` tuple (see `net`'s `NetBodyParts` / `tls`'s `TlsBodyParts`).
/// `stack_size` is the sp-relative locals region the body reserves; the wrapper
/// passes it to `finalize_vreg_body_with_locals`, byte-identical to the console
/// body's former self-finalize.
pub(crate) type TermBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The `abi_function` body shared by every native `term.*` member (crypto/io/net's
/// clean-room shape). The `abi_function` wrapper seeds the entry label, binds the
/// incoming ABI argument registers, and finalizes; this body dispatches to the
/// family-generic [`lower_term_helper`] by the runtime-call name in [`AbiCtx::call`]
/// and appends its instructions/relocations. All native members register this one
/// body.
pub(crate) fn lower_term_os_seam(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &crate::codegen::registry::AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = lower_term_helper(
        ctx.call,
        &symbol,
        ctx.term_state_offset,
        ctx.presentation_mode_offset,
        ctx.build_mode,
        ctx.platform_imports,
        ctx.platform,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    // A `void` location: every term body emits its own fallible ABI, so the wrapper
    // appends no epilogue.
    Ok(ValueResult {
        origin: None,
        type_: "Nothing".to_string(),
        location: Operand::from("void"),
        text: ctx.call.to_string(),
    })
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
pub(crate) fn lower_term_helper(
    call: &str,
    symbol: &str,
    term_state_offset: Option<usize>,
    presentation_mode_offset: Option<usize>,
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
            prepend_wrong_mode_gate(
                &mut instructions,
                &mut relocations,
                symbol,
                presentation_mode_offset,
            );
            // The app bodies hold every cross-call value in allocator vregs (no
            // addressable stack scratch), so the body reserves no sp-relative locals.
            return Ok((instructions, relocations, 0));
        }
    }

    console_lower_term_helper(call, symbol, term_state_offset, platform_imports, platform)
}
