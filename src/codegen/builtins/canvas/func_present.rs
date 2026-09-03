//! `canvas::present` — install a scene as the canvas's current content.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Install a list of `canvas::DrawItem`s as the canvas's current content."#;

const DESC: &str = r#"`present` **installs** a scene. It is not a per-frame draw call: the runtime keeps
rendering the installed scene — on vsync, on resize, on damage — until the next
`present` replaces it. A static picture is therefore presented once and costs
nothing thereafter, and a program that never changes its content never calls
`present` again.

`present` **copies the whole scene**. Everything it reaches — the item fields, a
`canvas::Polygon`'s point list, a `canvas::Text`'s string, the `canvas::Paint` values — is copied, so
once `present` returns the installed scene is entirely its own. You are free to
change or discard whatever you built the list from, and the renderer can read
the scene at any later moment without coordinating with your program.

**Re-presenting an identical scene does nothing.** `present` compares the incoming
content against what is already installed and returns without republishing when
they match, so an animation loop that redraws an unchanged frame costs a
comparison rather than a re-render.

An item names an image or font through a `canvas::ImageRef`/`canvas::FontRef` — an id, not the
resource itself — so an installed scene never keeps an image open. Destroying an
image a scene still names is safe: the scene holds its id, not the image.

Requires `app::Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`."#;

const EX: &str = r#"A yellow face with green eyes and a smile. Note that each item is bound first —
a list literal does not span source lines:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET yellow AS canvas::Color = canvas::rgb(255, 255, 0)
  LET green AS canvas::Color = canvas::rgb(0, 160, 0)

  LET face AS canvas::DrawItem = canvas::Circle[x := 200.0, y := 200.0, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS canvas::DrawItem = canvas::Circle[x := 150.0, y := 160.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS canvas::DrawItem = canvas::Circle[x := 250.0, y := 160.0, radius := 22.0, paint := canvas::fill(green)]
  ' 0 -> PI sweeps downward under a Y-down origin, so this is a smile.
  LET smile AS canvas::DrawItem = canvas::Arc[x := 200.0, y := 215.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Butt, paint := canvas::stroke(green, 14.0)]

  canvas::present([face, eyeL, eyeR, smile])
END SUB
```

The eyes as ellipses instead, half-closed and tilted — the case a `canvas::Circle`
cannot express and a `canvas::Polygon` would only approximate. `angle` turns each about
its own centre, clockwise from +X, so the two lean towards each other:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET yellow AS canvas::Color = canvas::rgb(255, 255, 0)
  LET green AS canvas::Color = canvas::rgb(0, 160, 0)

  LET face AS canvas::DrawItem = canvas::Circle[x := 200.0, y := 200.0, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS canvas::DrawItem = canvas::Ellipse[x := 150.0, y := 160.0, radiusX := 30.0, radiusY := 12.0, angle := 0.0 - 0.35, paint := canvas::fill(green)]
  LET eyeR AS canvas::DrawItem = canvas::Ellipse[x := 250.0, y := 160.0, radiusX := 30.0, radiusY := 12.0, angle := 0.35, paint := canvas::fill(green)]
  LET smile AS canvas::DrawItem = canvas::Arc[x := 200.0, y := 215.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, cap := canvas::CapStyle.Round, paint := canvas::stroke(green, 14.0)]

  canvas::present([face, eyeL, eyeR, smile])
END SUB
```

Give an ellipse equal radii and it is a circle — exactly, not nearly — so an animation
that squashes a circle can hold one item type throughout rather than switching at the
moment the radii happen to match."#;

/// Publish, then render only if the publish actually changed anything.
///
/// The two steps are separate calls because the skip has to gate the *render* to be
/// worth anything: publishing is three stores, rendering is the whole scene.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_present(items AS List OF DrawItem) AS Nothing
  IF canvas::publishScene(items) THEN
    canvas::publishHashes(__canvas_hashScene(items))
    __canvas_ensureGraphics()
    canvas::signalRedraw()
    canvas::syncFrame()
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
