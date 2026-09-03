//! `canvas::setGroup` — install a named sub-scene.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Install a list of items under a name, to be drawn by `canvas::Group`."#;

const DESC: &str = r#"`setGroup` installs `items` under `name`. A scene then draws them by including a
`canvas::Group` node — two offsets and a name — instead of the items themselves:

```
canvas::setGroup("panel", panelItems)
canvas::present([canvas::Group[dx := 10.0, dy := 20.0, name := "panel"]])
```

`canvas::present` copies the whole scene every time it is called. A large static
sub-picture — a map, a sprite sheet, a UI panel — is therefore copied on every frame
even though nothing about it changed. Installing it once and referencing it means each
`present` copies the *node* and not the picture, and the same group can be referenced
from several scenes and at several positions without being copied for each.

**It takes effect at the next `canvas::present`, not immediately.** Calling `setGroup`
does not repaint; `present` is the install point for everything a scene draws, and a
group is no exception. Presenting the same scene list after a `setGroup` *does* redraw,
even though the list is unchanged — the change is inside the group, and it is seen.

Calling it again with the same name replaces what that name draws. A name you have not
installed, or one you removed, draws nothing and does not raise — so a scene may
reference a group before it is built.

Items in the list may themselves be `canvas::Group` nodes. A nested group stays a
reference: replacing the inner group changes what the outer one draws, without
reinstalling the outer.

The items are copied, so the list you pass is yours to change afterwards — the
installed group does not follow it. The group stays installed until you replace it or
call `canvas::removeGroup`."#;

const EX: &str = r#"A panel installed once and drawn at two positions:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET body AS canvas::DrawItem = canvas::RoundedRect[x := 0.0, y := 0.0, w := 160.0, h := 90.0, cornerRadius := 12.0, paint := canvas::fillStroke(canvas::rgb(40, 60, 90), canvas::rgb(120, 170, 230), 3.0)]
  LET dot AS canvas::DrawItem = canvas::Circle[x := 24.0, y := 24.0, radius := 8.0, paint := canvas::fill(canvas::rgb(255, 200, 60))]
  canvas::setGroup("panel", [body, dot])

  LET left AS canvas::DrawItem = canvas::Group[dx := 40.0, dy := 40.0, name := "panel"]
  LET right AS canvas::DrawItem = canvas::Group[dx := 260.0, dy := 40.0, name := "panel"]
  canvas::present([left, right])
END SUB
```

Changing what the name draws. The scene list is not rebuilt and `present` is handed
exactly what it was handed before — the redraw happens because the group changed:

```
IMPORT app
IMPORT canvas
IMPORT os

SUB main()
  app::setMode(app::Mode.Canvas)
  LET node AS canvas::DrawItem = canvas::Group[dx := 100.0, dy := 100.0, name := "light"]

  LET off AS canvas::DrawItem = canvas::Circle[x := 0.0, y := 0.0, radius := 30.0, paint := canvas::fill(canvas::rgb(60, 60, 60))]
  canvas::setGroup("light", [off])
  canvas::present([node])
  os::sleep(500)

  LET on AS canvas::DrawItem = canvas::Circle[x := 0.0, y := 0.0, radius := 30.0, paint := canvas::fill(canvas::rgb(255, 220, 80))]
  canvas::setGroup("light", [on])
  canvas::present([node])
END SUB
```"#;

// plan-116-G Phase 2 registers the surface and nothing else; Phase 3 lands the group
// table and this body against it, and Phase 4 the resolution that makes a `Group` node
// draw. The body is empty rather than partial on purpose — a half-installed group
// would be a wrong picture, and an empty one is a `Group` that resolves no name, which
// is the documented behaviour of a name that was never installed.
//
// What proves the end of this state is not a reading of this comment: the two
// `#[ignore]`d tests in `tests/rt_canvas_rasteriser.rs` fail while it holds, and
// Phase 4 un-ignores them.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_setGroup(name AS String, items AS List OF DrawItem) AS Nothing
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setGroup",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "name",
                    desc: "What to call this group. A `canvas::Group` naming it draws \
                           these items; installing again under the same name replaces \
                           them.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "items",
                    desc: "The sub-scene, drawn in list order — later items paint over \
                           earlier ones, exactly as in `canvas::present`. Copied, so \
                           the list you pass stays yours to change.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::named("DrawItem")),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            // plan-116-G Phase 2 (G14): EMPTY, not `ErrWrongMode`. The body here is
            // inert and cannot raise anything; a declared error a member cannot
            // produce is a documented promise the renderer does not keep, and it
            // renders into the man page's Errors table. Phase 3 adds the errors its
            // native call actually raises.
            errors: vec![],
            body: Body::mfb(BODY, "__canvas_setGroup"),
        }],
    });
}
