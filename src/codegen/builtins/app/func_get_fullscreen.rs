//! `app::getFullscreen` — report whether the window is fullscreen.

// --- codegen tier imports (migration) ---
use super::gen_window::APP_FULLSCREEN_SYMBOL;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `app::getFullscreen()` — load the process-global fullscreen word (`0`/`1`) as
/// the `Boolean` result. The word is written by `setFullscreen` and by each
/// backend's UI thread when the user changes the window's state, so this is a
/// plain load with no marshal to the UI thread.
pub(crate) fn lower_get_fullscreen(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut vregs = Vregs::new();
    let addr = vregs.next();
    push_symbol_address(
        &symbol,
        APP_FULLSCREEN_SYMBOL,
        &addr,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, &addr, 0),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::return_(),
    ]);
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "app.getFullscreen".to_string(),
    })
}

const INTRO: &str = r#"Report whether the app window is fullscreen"#;
const DESC: &str = r#"`app::getFullscreen` returns `TRUE` when the program's window is fullscreen and
`FALSE` when it is not. It takes no arguments and always succeeds.

The answer follows the window, not only the program: if the user leaves
fullscreen themselves — with the window's fullscreen button or a keyboard
shortcut — `app::getFullscreen` returns `FALSE` from then on, and `TRUE` again if
they go back in. A program that wants to know the current state asks rather than
remembering what it last passed to `app::setFullscreen`.

While there is no window — in `app::Mode.None` — it returns the state the window
will take when one is shown, which is `FALSE` until the program calls
`app::setFullscreen(TRUE)`."#;
const EX: &str = r#"Toggle fullscreen from whatever state the window is in now:

```
IMPORT app

SUB main
  app::setFullscreen(NOT app::getFullscreen())
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getFullscreen",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_get_fullscreen),
        }],
    });
}
