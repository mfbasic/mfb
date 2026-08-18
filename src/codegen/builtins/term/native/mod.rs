//! Native code generation for the built-in `term` package (clean-room registry
//! migration).
//!
//! `term` is a structured-terminal-surface package: cursor / color / attribute
//! control, screen clearing, box / line / fill drawing, glyph / text stamping,
//! size / resize queries, and the full-screen TUI-mode toggle. Every member lowers
//! to an OS-seam runtime helper whose body is the heavy console terminal emitter
//! (`code::lower_term_helper`) or, in an `--app` build, the platform's synthesized
//! `TermView` surface (`CodegenPlatform::emit_app_term_helper`).
//!
//! The 24 members share one dispatcher, [`lower_term_helper`] — the verbatim term
//! block relocated from `code/mod.rs`'s `lower_runtime_helper`. Each member's
//! `func_*.rs` registers it in *both* the `posix` and `win` slots of a
//! `Body::native_os_seam`; the generic OS-seam dispatch (`crate::codegen::os`) reaches
//! it by `platform.family()`, and the dispatcher branches app-vs-console internally
//! off the per-compilation [`OsLowerCtx`](crate::codegen::registry::OsLowerCtx)
//! (`build_mode` / `term_state_offset` / `presentation_mode_offset`). `term` has no
//! posix/win difference and no `os_aliases`.
//!
//! The heavy emitters stay in the shared code layer (like the `strings` / `vector`
//! codegen carriers — relocating the 150 KB of terminal-grid emission would be
//! byte-identity-risky): `code::lower_term_helper` (the console backend),
//! `CodegenPlatform::emit_app_term_helper` (the per-platform app backend), and the
//! cross-package `code::prepend_wrong_mode_gate` (`app` owns `Mode`, so the gate stays
//! shared), reached here through the `code` re-exports.

use crate::codegen::app::hook::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
/// Family-generic OS-seam dispatcher for every `term` member — the verbatim term
/// block relocated from `src/codegen/engine/builder/mod.rs`. Registered in both the
/// `posix` and `win` slots of each member's `Body::native_os_seam`. Branches
/// app-vs-console off the per-compilation
/// [`OsLowerCtx`](crate::codegen::registry::OsLowerCtx).
// --- codegen tier imports (migration) ---
use crate::codegen::term::core::lower_term_helper as console_lower_term_helper;
use std::collections::HashMap;
pub(crate) fn lower_term_helper(
    call: &str,
    symbol: &str,
    ctx: &crate::codegen::registry::OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    let term_state_offset = ctx
        .term_state_offset
        .ok_or_else(|| format!("native code plan emits '{symbol}' without reserving term state"))?;
    // App mode drives the synthesized TermView surface (plan-01-term.md §6.3):
    // `emit_app_term_helper` dispatches EVERY term:: helper — the mode toggle plus
    // clear/sync/moveTo/color/attr/cursor/size — to the platform's app backend. It
    // falls through to the shared console backend below only in non-app builds.
    let app_term_helper = if ctx.build_mode.is_app() {
        platform.emit_app_term_helper(call, symbol, term_state_offset)
    } else {
        None
    };
    match app_term_helper {
        Some(result) => {
            // plan-62-E: every app-mode `term::` helper (including `on`) is gated on the
            // `Console` presentation mode — outside it, `term::` raises the trappable
            // `ErrWrongMode` before touching the (absent) grid. No-op when the program
            // cannot leave `Console` (`presentation_mode_offset` is `None`), so a program
            // that never uses `app::` is unchanged.
            let mut body = pad_no_slots(result?);
            prepend_wrong_mode_gate(
                &mut body.1,
                &mut body.2,
                symbol,
                ctx.presentation_mode_offset,
            );
            Ok(body)
        }
        None => {
            console_lower_term_helper(call, symbol, term_state_offset, platform_imports, platform)
        }
    }
}
