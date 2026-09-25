//! `app::setFullscreen` — put the app window into or out of fullscreen.

// --- codegen tier imports (migration) ---
use super::gen_window::APP_FULLSCREEN_SYMBOL;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `app::setFullscreen(on)` — store the `Boolean` into the process-global
/// fullscreen word, then run the backend window sync
/// ([`CodegenPlatform::emit_app_window_sync`]), which makes the window match the
/// word on the UI thread. The store lands first so the sync (and any `getFullscreen`
/// racing it) reads the new request.
pub(crate) fn lower_set_fullscreen(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let on = vregs.next();
    let addr = vregs.next();
    builder
        .instructions
        .push(abi::move_register(&on, abi::c_arg(0)));
    push_symbol_address(
        &symbol,
        APP_FULLSCREEN_SYMBOL,
        &addr,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.instructions.push(abi::store_u64(&on, &addr, 0));
    if let Some(result) = ctx.platform.emit_app_window_sync(
        &symbol,
        ctx.platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    ) {
        result?;
    }
    builder.instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "app.setFullscreen".to_string(),
    })
}

const INTRO: &str = r#"Put the app window into or out of fullscreen"#;
const DESC: &str = r#"`app::setFullscreen` makes the program's window fill the screen when `on` is
`TRUE`, and returns it to an ordinary window when `on` is `FALSE`. It returns
nothing and always succeeds. Asking for the state the window is already in does
nothing.

On macOS the window goes into its own fullscreen space, exactly as if the user
had pressed its green button. On Linux the window manager is asked to make the
window fullscreen. On Windows the window loses its border and covers the monitor
it is on; `FALSE` puts back the border, size and position it had before.

The change is started before `app::setFullscreen` returns, but the window may
take a moment to finish it — macOS animates the transition. `app::getFullscreen`
reports the requested state straight away.

In `app::Mode.None` there is no window to change. The request is remembered, and
the window opens fullscreen when a later `app::setMode` shows it. The same goes
for switching between `app::Mode.Console` and `app::Mode.Canvas`: the window keeps
the fullscreen state across the switch."#;
const EX: &str = r#"Show the transcript fullscreen, then go back to a window:

```
IMPORT app
IMPORT io

SUB main
  app::setMode(app::Mode.Console)
  app::setFullscreen(TRUE)
  io::print("fullscreen: " & toString(app::getFullscreen()))
  app::setFullscreen(FALSE)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setFullscreen",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "on",
                desc: "`TRUE` to make the window fullscreen, `FALSE` to return it to an ordinary \
                       window.",
                aliases: &[],
                ty: ParameterType::Boolean,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_set_fullscreen),
        }],
    });
}
