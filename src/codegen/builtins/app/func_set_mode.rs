//! `app::setMode` — change the presentation mode of an `--app` program.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// The `term::off` runtime helper. It exists whenever the program has `term::`
/// state: `builder::mod` forces it in for the exit-time auto-restore.
const TERM_OFF_SYMBOL: &str = "_mfb_rt_term_term_off";

/// `app::setMode(mode)` — when the mode changes and the program has `term::` state,
/// run the implicit `term::off` (bug-966); store the `Mode` discriminant into the
/// presentation-mode word; then invoke the per-backend surface-reconcile seam
/// ([`CodegenPlatform::emit_app_mode_reconcile`]). The store lands *before* the
/// reconcile runs, so the reconcile (which in C/D emits register-clobbering `bl`
/// calls to marshal to the UI thread) reads the authoritative mode from the slot
/// rather than a caller-saved register. `ctx.presentation_mode_offset` is `Some`
/// only in an `--app` build, so `None` here is an internal error.
pub(crate) fn lower_set_mode(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let presentation_mode_offset = ctx.presentation_mode_offset.ok_or_else(|| {
        format!("native code plan emits '{symbol}' without reserving the presentation-mode slot")
    })?;
    let mut vregs = Vregs::new();
    let mode = vregs.next();
    builder
        .instructions
        .push(abi::move_register(&mode, abi::c_arg(0)));
    // bug-966: a mode *change* tears the surface down with an implicit `term::off`
    // (`mfb spec app presentation-mode`, "The surface-reconcile seam"), so no raw /
    // grid state survives the switch. This is `term::off`'s own helper — final
    // present, input restore, `active = 0` — called here, once, for every backend,
    // rather than a copy in each backend's reconcile. It runs while the slot still
    // holds the OLD mode: the TUI can only be on in `Console` (`term::on` is
    // `Console`-gated), so an active TUI passes `term::off`'s `ErrWrongMode` gate.
    // When the old mode is not `Console` the TUI is necessarily off; the helper then
    // returns `ErrWrongMode` (a static message, no allocation) without touching
    // anything, and that result is discarded like every other result below — the
    // shutdown auto-restore calls the same helper the same way. A same-mode
    // `setMode` is not a change and keeps the TUI. A program with no `term::` state
    // has no TUI to turn off and emits nothing.
    if ctx.term_state_offset.is_some() {
        let current = vregs.next();
        let unchanged = format!("{symbol}_mode_unchanged");
        builder.instructions.extend([
            abi::load_u64(&current, ARENA_STATE_REGISTER, presentation_mode_offset),
            abi::compare_registers(&current, &mode),
            abi::branch_eq(&unchanged),
            abi::branch_link(TERM_OFF_SYMBOL),
        ]);
        builder
            .relocations
            .push(internal_branch(&symbol, TERM_OFF_SYMBOL));
        builder.instructions.push(abi::label(&unchanged));
    }
    builder.instructions.push(abi::store_u64(
        &mode,
        ARENA_STATE_REGISTER,
        presentation_mode_offset,
    ));
    // plan-62-B seam (no-op default; filled by plan-62-C/D). `None` = state-only.
    if let Some(result) = ctx.platform.emit_app_mode_reconcile(
        &symbol,
        presentation_mode_offset,
        &mut builder.instructions,
        &mut builder.relocations,
    ) {
        result?;
    }
    builder.instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.instructions.push(abi::return_());
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "app.setMode".to_string(),
    })
}

const INTRO: &str = r#"Change the presentation mode of this `--app` program"#;
const DESC: &str = r#"`app::setMode` sets the program's presentation mode. `mode` is one of the three
`app::Mode` enum members: `app::Mode.Console` (the terminal-in-a-window surface),
`app::Mode.Canvas` (the 2D graphics surface the `canvas` package draws on; see
`mfb man canvas`), or `app::Mode.None` (windowless). The call returns nothing.

Switching mode reconciles the window surface to match: entering `app::Mode.None` tears
the window down and routes `io::print` to standard output; entering `app::Mode.Console`
brings the transcript window up. A subsequent `app::getMode` reflects the new mode.

Switching to a *different* mode first leaves TUI mode exactly as `term::off` would
(the last frame is presented, line input is restored), so after coming back to
`app::Mode.Console` the program is in the scrolling transcript, `term::isOn`
reports `FALSE`, and `io::print` output is visible; call `term::on` again to get
the grid back. Setting the mode that is already in effect is not a switch and
leaves an active TUI alone.

Referencing `app::setMode` anywhere in a program also changes that program's
**initial** mode to `app::Mode.None` — a program that manages its own surface starts
windowless and brings a window up deliberately, rather than flashing the default
terminal window first.

Name the enum package-qualified, as in `app::Mode.None`; a bare `Mode` is not a
known type."#;
const EX: &str = r#"Start windowless (the mere reference to `setMode` makes `None` the initial mode),
then bring the console surface up:

```
IMPORT app
IMPORT io

FUNC main AS Nothing
  io::print("no window yet")
  app::setMode(app::Mode.Console)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setMode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "mode",
                desc: "The presentation mode to switch to: `app::Mode.Console`, `app::Mode.Canvas`, or \
                       `app::Mode.None`. Any other type is rejected at compile time.",
                aliases: &[],
                ty: ParameterType::named("Mode"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_mode),
        }],
    });
}
