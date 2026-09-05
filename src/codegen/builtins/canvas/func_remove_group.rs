//! `canvas::removeGroup` — drop a named sub-scene.

use super::gen_group::emit_remove_group;
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Drop a group installed by `canvas::setGroup`, so its name draws nothing."#;

const DESC: &str = r#"`removeGroup` drops the name. A `canvas::Group` naming it afterwards draws nothing —
the same as a name that was never installed — and does not raise.

Removing a name that is not installed is a no-op, not an error. That is deliberate and
symmetric with the node's own rule: a program that cannot be sure whether a group is
installed should not have to check before removing it, any more than it has to check
before drawing it.

Like `canvas::setGroup`, this takes effect at the next `canvas::present`. A scene
already on screen keeps its picture until something presents again.

You do not have to call this. A group you replace with another `canvas::setGroup` of
the same name is dropped for you, and everything a program installs is dropped when it
exits. `removeGroup` is for the case where a group is large and the program keeps
running — the same reason `canvas::destroyImage` exists next to letting an image go out
of scope."#;

const EX: &str = r#"A group installed, drawn, and dropped — the second present draws nothing where the
panel was:

```
IMPORT app
IMPORT canvas
IMPORT os

SUB main()
  app::setMode(app::Mode.Canvas)
  LET body AS canvas::DrawItem = canvas::RoundedRect[x := 0.0, y := 0.0, w := 160.0, h := 90.0, cornerRadius := 12.0, paint := canvas::fill(canvas::rgb(40, 60, 90))]
  canvas::setGroup("panel", [body])

  LET node AS canvas::DrawItem = canvas::Group[dx := 40.0, dy := 40.0, name := "panel"]
  LET marker AS canvas::DrawItem = canvas::Circle[x := 400.0, y := 300.0, radius := 20.0, paint := canvas::fill(canvas::rgb(255, 80, 80))]
  canvas::present([node, marker])
  os::sleep(500)

  canvas::removeGroup("panel")
  canvas::present([node, marker])
END SUB
```

The scene still contains the node and it is still legal; it simply resolves to nothing,
so only the marker is drawn. Removing a name that was never installed is the same
no-op:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  canvas::removeGroup("never-installed")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "removeGroup",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc: "The group to drop. A name that is not installed is a no-op.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrWrongMode"],
            body: Body::abi_function(emit_remove_group),
        }],
    });
}
