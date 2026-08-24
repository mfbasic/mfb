//! `app::getMode` — read the presentation mode currently in effect.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `app::getMode()` — load the presentation-mode word (`0`/`1`) into the result
/// value register as a `Mode` value (the enum is i64-carried by its discriminant).
/// The `abi_function` wrapper seeds the entry label and finalizes; this body sets
/// the result registers and returns. `ctx.presentation_mode_offset` is the arena
/// slot, `Some` only in an `--app` build, so `None` here is an internal error.
pub(crate) fn lower_get_mode(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let presentation_mode_offset = ctx.presentation_mode_offset.ok_or_else(|| {
        format!(
            "native code plan emits '{}' without reserving the presentation-mode slot",
            builder.current_symbol
        )
    })?;
    builder.instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        ARENA_STATE_REGISTER,
        presentation_mode_offset,
    ));
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
        text: "app.getMode".to_string(),
    })
}
const INTRO: &str = r#"Read the presentation mode currently in effect for this `--app` program"#;
const DESC: &str = r#"`app::getMode` returns the program's current presentation mode as a `Mode` value.
It takes no arguments and always succeeds.

The mode reported is the value most recently written by `app::setMode`, or — if
the program has never called `app::setMode` — the statically decided initial mode.
That initial mode is `Mode.Console` for a program that references `app::setMode`
nowhere, and `Mode.None` for a program that references it anywhere (even on a
never-taken branch: the decision is a static, whole-program one, not a runtime
flow analysis).

The mode is not a call into a runtime helper you can see — it is lowered to a
single load of the per-execution-context presentation-mode word held in the arena
state region, reserved only in an `--app` build.

The `Mode` enum is referenced bare, like every other builtin type: write
`Mode.Console`, not `app::Mode.Console`."#;
const EX: &str = r#"Branch on the mode currently in effect:

```
IMPORT app
IMPORT io

SUB main
  IF app::getMode() = Mode.None THEN
    io::print("running windowless")
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getMode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named("Mode"),
            errors: vec![],
            body: Body::abi_function(lower_get_mode),
        }],
    });
}
