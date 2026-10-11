//! `canvas::presentLayers` — install a layered scene.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Install a list of `canvas::DrawLayer`s as the canvas's current content."#;

const DESC: &str = r#"`presentLayers` installs a scene as an ordered stack of layers rather than a flat
list of items. Layers composite in order — later layers paint over earlier ones —
and within a layer the items draw in list order, exactly as `canvas::present`
draws them.

Everything `canvas::present` guarantees holds here: the scene is deep-copied, so
after the call the installed scene is entirely its own; installing an
identical scene republishes nothing; and the runtime keeps rendering what was
installed until the next call replaces it.

**Layers exist to separate what changes from what does not.** A game with a static
background and a moving sprite, or a chart with fixed axes and a live series, can
put each in its own layer; a layer whose contents did not change hashes identically
and reuses its cached geometry wholesale, so only the layer that moved costs
anything.

A scene is either flat or layered, never both — installing one shape replaces the
other. Use `canvas::present` when there is nothing to separate; a single-layer
`presentLayers` is the same picture, and the flat form says so more directly.

Requires `app::Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`.

A reading of the clock `presentLayers` times itself against whose nanosecond
count does not fit an `Integer` raises `ErrOverflow`. `ErrOutOfMemory` means the
program ran out of memory."#;

const EX: &str = r#"A static backdrop under a moving marker, so redrawing the marker leaves the
backdrop's geometry untouched:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET sky AS color::Color = color::rgb(20, 30, 60)
  LET dot AS color::Color = color::rgb(255, 200, 0)

  LET backdrop AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 400.0, height := 300.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(sky), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET marker AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 100.0, y := 150.0], radius := 12.0, paint := canvas::fill(dot), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  LET back AS canvas::DrawLayer = canvas::DrawLayer[items := [backdrop]]
  LET front AS canvas::DrawLayer = canvas::DrawLayer[items := [marker]]
  canvas::presentLayers([back, front])
END FUNC
```"#;

/// The layered twin of `canvas::present`: publish, render only on a real change.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_presentLayers(layers AS List OF DrawLayer) AS Nothing
  ' `publishLayers` first: it carries the `Mode.Canvas` gate, which must raise before
  ' anything starts a graphics thread. Then, as in `present` (plan-155-B), start the
  ' thread and count the present whether or not the layers changed.
  LET installed AS Boolean = canvas::publishLayers(layers)
  __canvas_ensureGraphics()
  canvas::notePresent(canvas::frameNanos())
  IF installed THEN
    ' plan-172-I: the revision and the id bits `canvas::pollMouse` reads, as `present`
    ' records them -- the bits of every layer's items together.
    __CANVAS_SCENE_REV = __CANVAS_SCENE_REV + 1
    MUT named AS Integer = 0
    MUT grouped AS Integer = 0
    FOR EACH layer IN layers
      LET bits AS Integer = canvas::sceneIdBits(layer.items)
      IF bits = 1 OR bits = 3 THEN
        named = 1
      END IF
      IF bits >= 2 THEN
        grouped = 2
      END IF
    NEXT
    __CANVAS_SCENE_IDS = named + grouped
    canvas::publishHashes(__canvas_hashLayers(layers))
    canvas::signalRedraw()
    canvas::syncFrame()
  END IF
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "presentLayers",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "layers",
                desc: "The layers to install, composited in order — later layers \
                       paint over earlier ones.",
                aliases: &[],
                ty: ParameterType::list_of(ParameterType::named("DrawLayer")),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrWrongMode", "ErrOutOfMemory", "ErrOverflow"],
            body: Body::mfb(BODY, "__canvas_presentLayers"),
        }],
    });
}
