//! `canvas::presentLayers` — install a layered scene.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Install a list of `DrawLayer`s as the canvas's current content."#;

const DESC: &str = r#"`presentLayers` installs a scene as an ordered stack of layers rather than a flat
list of items. Layers composite in order — later layers paint over earlier ones —
and within a layer the items draw in list order, exactly as `canvas::present`
draws them.

Everything `canvas::present` guarantees holds here: the scene is deep-copied, so
after the call nothing installed points at anything the caller owns; installing an
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

Requires `Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`."#;

const EX: &str = r#"A static backdrop under a moving marker, so redrawing the marker leaves the
backdrop's geometry untouched:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(Mode.Canvas)
  LET sky AS Color = canvas::rgb(20, 30, 60)
  LET dot AS Color = canvas::rgb(255, 200, 0)

  LET backdrop AS DrawItem = Rectangle[x := 0.0, y := 0.0, w := 400.0, h := 300.0, paint := canvas::fill(sky)]
  LET marker AS DrawItem = Circle[x := 100.0, y := 150.0, radius := 12.0, paint := canvas::fill(dot)]

  LET back AS DrawLayer = DrawLayer[items := [backdrop]]
  LET front AS DrawLayer = DrawLayer[items := [marker]]
  canvas::presentLayers([back, front])
END SUB
```"#;

/// The layered twin of `canvas::present`: publish, render only on a real change.
#[rustfmt::skip]
const BODY: &str =
r#"FUNC __canvas_presentLayers(layers AS List OF DrawLayer) AS Nothing
  IF canvas::publishLayers(layers) THEN
    canvas::publishHashes(__canvas_hashLayers(layers))
    canvas::startGraphics()
    canvas::signalRedraw()
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
            errors: vec!["ErrWrongMode"],
            body: Body::mfb(BODY, "__canvas_presentLayers"),
        }],
    });
}
