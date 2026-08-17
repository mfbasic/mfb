//! `app::getMode` — read the presentation mode currently in effect.
//!
//! Descriptor + docs migrated from `src/docs/man/builtins/app/getMode.md`; the
//! native lowering (the per-arena presentation-mode load) lives in
//! [`super::native::lower_app_helper`].

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

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

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getMode",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Named("Mode"),
            errors: vec![],
            body: Body::native(
                Some(super::native::lower_app_helper),
                Some(super::native::lower_app_helper),
                None,
            ),
        }],
    });
}
