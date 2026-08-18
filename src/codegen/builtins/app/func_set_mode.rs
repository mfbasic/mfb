//! `app::setMode` — change the presentation mode of an `--app` program.
//!
//! The native lowering (store the discriminant, then the per-backend surface-reconcile
//! seam) lives in [`super::native::lower_app_helper`].

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

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
                ty: ParameterType::Named("Mode"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native(
                Some(super::native::lower_app_helper),
                Some(super::native::lower_app_helper),
                None,
            ),
        }],
    });
}
