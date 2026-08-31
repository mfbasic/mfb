//! `canvas::present` — install a scene as the canvas's current content.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Install a list of `DrawItem`s as the canvas's current content."#;

const DESC: &str = r#"`present` **installs** a scene. It is not a per-frame draw call: the runtime keeps
rendering the installed scene — on vsync, on resize, on damage — until the next
`present` replaces it. A static picture is therefore presented once and costs
nothing thereafter, and a program that never changes its content never calls
`present` again.

`present` **deep-copies the scene transitively**. Every reachable byte — the item
fields, a `Polygon`'s point list, a `Text`'s string, the `Paint` values — is copied
into runtime-owned storage, so once `present` returns, nothing in the installed
scene points at anything the caller owns. The program is free to mutate or drop
whatever it built the list from, and the renderer can read the scene at any later
moment without coordinating with the program.

**Re-presenting an identical scene does nothing.** `present` compares the incoming
content against what is already installed and returns without republishing when
they match, so an animation loop that redraws an unchanged frame costs a
comparison rather than a re-render.

An item names an image or font through an `ImageRef`/`FontRef` — an id, not the
resource — so an installed scene has no opinion about any resource's lifetime.
Destroying an image that a scene still names is safe: the runtime defers freeing
the backing texture until the GPU has finished with it.

Requires `Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`."#;

const EX: &str = r#"A yellow face with green eyes and a smile. Note that each item is bound first —
a list literal does not span source lines:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(Mode.Canvas)
  LET yellow AS Color = canvas::rgb(255, 255, 0)
  LET green AS Color = canvas::rgb(0, 160, 0)

  LET face AS DrawItem = Circle[x := 200.0, y := 200.0, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS DrawItem = Circle[x := 150.0, y := 160.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS DrawItem = Circle[x := 250.0, y := 160.0, radius := 22.0, paint := canvas::fill(green)]
  ' 0 -> PI sweeps downward under a Y-down origin, so this is a smile.
  LET smile AS DrawItem = Arc[x := 200.0, y := 215.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := canvas::stroke(green, 14.0)]

  canvas::present([face, eyeL, eyeR, smile])
END SUB
```"#;

/// Publish, then render only if the publish actually changed anything.
///
/// The two steps are separate calls because the skip has to gate the *render* to be
/// worth anything: publishing is three stores, rendering is the whole scene.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_present(items AS List OF DrawItem) AS Nothing
  IF canvas::publishScene(items) THEN
    canvas::publishHashes(__canvas_hashScene(items))
    canvas::startGraphics()
    canvas::signalRedraw()
  END IF
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "present",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "items",
                desc: "The scene to install, drawn in list order — later items paint \
                       over earlier ones.",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::named("DrawItem")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrWrongMode"],
            body: Body::mfb(BODY, "__canvas_present"),
        }],
    });
}
