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

/// `app::setMode(mode)` — store the `Mode` discriminant into the presentation-mode
/// word, then invoke the per-backend surface-reconcile seam
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
        type_: "Nothing".to_string(),
        location: Operand::from("void"),
        text: "app.setMode".to_string(),
    })
}

const INTRO: &str = r#"Change the presentation mode of this `--app` program"#;
const DESC: &str = r#"`app::setMode` sets the program's presentation mode. `mode` is one of the two
`Mode` enum members: `Mode.Console` (the terminal-in-a-window surface) or
`Mode.None` (windowless). The call returns nothing.

Switching mode reconciles the window surface to match: entering `Mode.None` tears
the window down and routes `io::print` to standard output; entering `Mode.Console`
brings the transcript window up. A subsequent `app::getMode` reflects the new mode.

Referencing `app::setMode` anywhere in a program also changes that program's
**initial** mode to `Mode.None` — a program that manages its own surface starts
windowless and brings a window up deliberately, rather than flashing the default
terminal window first.

The `Mode` enum is referenced bare, like every other builtin type: write
`Mode.None`, not `app::Mode.None`."#;
const EX: &str = r#"Start windowless (the mere reference to `setMode` makes `None` the initial mode),
then bring the console surface up:

```
IMPORT app
IMPORT io

SUB main
  io::print("no window yet")
  app::setMode(Mode.Console)
END SUB
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
                desc: "The presentation mode to switch to: `Mode.Console` or `Mode.None`. Any \
                       other type is rejected at compile time.",
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
