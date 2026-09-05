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

An item that draws an image or text holds the image or font itself. Destroying one a
scene still names is safe: that item draws nothing, and the frame is unaffected. An
installed scene does not keep an image open — closing it is still yours to do, and
still takes effect immediately.

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
r#"' plan-116-G: the resolved-groups signature of the LAST present, so a `setGroup` under
' a name this scene already referenced is seen as a change.
'
' A worker-thread global, which is what it must be: `present` runs on the worker, and
' MFBASIC globals are per-thread (`.ai/canvas-threading.md` section 2). The graphics
' thread has its own zeroed copy and never reads this one.
MUT __CANVAS_LAST_GROUP_SIG AS List OF Integer = []

FUNC __canvas_present(items AS List OF DrawItem) AS Nothing
  ' Two independent reasons to redraw, and both must be consulted.
  '
  ' `publishScene` compares the raw bytes of the item list, which catches a scene whose
  ' ITEMS changed. It cannot catch a scene whose items are identical while a group's
  ' contents were replaced -- a `Group` node is two floats and a string pointer, all
  ' three unchanged by `setGroup` under the same name.
  '
  ' The signature is that second reason. Note the ordering: `publishScene` is called
  ' FIRST and unconditionally, because it is what installs the scene; the signature is
  ' then compared, and either one being new makes this a frame. The published items do
  ' not need re-publishing when only the signature moved -- a group's contents live in
  ' the group table and the renderer reads them from there -- so this asks for a
  ' RE-RENDER, not a re-publish.
  ' plan-116-G Phase 5: the drain gate, FIRST and unconditional. It frees every group
  ' buffer a frame has completed past. It runs here rather than beside the scene ring's
  ' own reclaim because that one sits on the publish path (G7): `removeGroup("panel")`
  ' followed by presents of an unchanged scene would then never free anything -- the
  ' frame skip would be working exactly as designed and the memory would be held anyway.
  ' A memory bound that depends on the scene changing is not a bound.
  '
  ' plan-116-J: a group owns the images and fonts its items name, so the buffer being
  ' freed is where they are closed. The walk is a MATCH rather than an open-coded step
  ' over the DrawItem union's layout in codegen -- a MATCH a new variant must handle is a
  ' compile error, a hand-written tag offset a new variant must not break is a hope (J13).
  '
  ' The gate stays in `nextReclaimableGroup`, which frees nothing. A FOR over all 256
  ' slots here instead would put 256 builtin calls on the per-present path, which is the
  ' exact axis plan-116-G optimised; with the scan in native code a present with nothing
  ' due costs one call and this loop never runs.
  MUT due AS Integer = canvas::nextReclaimableGroup()
  WHILE due >= 0
    __canvas_closeRetired(canvas::retiredItems(due), items)
    canvas::groupReclaim(due)
    due = canvas::nextReclaimableGroup()
  END WHILE
  LET installed AS Boolean = canvas::publishScene(items)
  LET sig AS List OF Integer = __canvas_groupSignature(items, 0)
  LET moved AS Boolean = NOT __canvas_intListEquals(sig, __CANVAS_LAST_GROUP_SIG)
  __CANVAS_LAST_GROUP_SIG = sig
  IF installed OR moved THEN
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
            // plan-116-G: `ErrDepthExceeded` is REUSED rather than minted. Its existing
            // definition — "structural nesting exceeds the implementation depth limit;
            // the text is well-formed, it is just nested deeper than the reader will
            // descend" — describes a group cycle exactly, and the caller's response is
            // the same one it names: raise the limit or fix the structure. A
            // canvas-specific twin would be a second code for one mistake.
            errors: vec!["ErrWrongMode", "ErrDepthExceeded", "ErrOutOfMemory"],
            body: Body::mfb(BODY, "__canvas_present"),
        }],
    });
}
