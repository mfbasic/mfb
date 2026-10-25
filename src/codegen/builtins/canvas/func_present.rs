//! `canvas::present` — install a scene as the canvas's current content.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"Install a list of `canvas::DrawItem`s as the canvas's current content."#;

const DESC: &str = r#"`present` **installs** a scene. It is not a per-frame draw call: the runtime draws
the installed scene, and draws it again only when the window is resized or
uncovered, until the next `present` replaces it. A static picture is therefore presented once and costs
nothing thereafter, and a program that never changes its content never calls
`present` again.

`present` **returns at once** — it does not wait for the frame to be drawn, so a
program that presents progress while it computes is never slowed by it. To run an
animation loop at the display's pace, end each iteration with `canvas::waitFrame`, and
advance the animation by `canvas::getFrameStats().dt`.

`present` **copies the whole scene**. Everything it reaches — the item fields, a
`canvas::Polygon`'s rings, a `canvas::Text`'s string, the `canvas::Paint` values — is copied, so
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

Requires `app::Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`.

A scene holding a `canvas::ParticleSystem` whose `emission.lifetime` is zero or less
raises `ErrCanvasParticleLifetime`, wherever the system is — in the list itself or in a
group it draws.

A reading of the clock `present` times itself against whose nanosecond count does
not fit an `Integer` raises `ErrOverflow`."#;

const EX: &str = r#"A yellow face with green eyes and a smile. Note that each item is bound first —
a list literal does not span source lines:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET yellow AS color::Color = color::rgb(255, 255, 0)
  LET green AS color::Color = color::rgb(0, 160, 0)

  LET face AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 200.0, y := 200.0], radius := 150.0, paint := canvas::fill(yellow), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET eyeL AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 150.0, y := 160.0], radius := 22.0, paint := canvas::fill(green), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET eyeR AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 250.0, y := 160.0], radius := 22.0, paint := canvas::fill(green), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' 0 -> PI sweeps downward under a Y-down origin, so this is a smile.
  LET smile AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 200.0, y := 215.0], radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(green, 14.0) { cap := canvas::CapStyle.Butt }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  canvas::present([face, eyeL, eyeR, smile])
END FUNC
```

The eyes as ellipses instead, half-closed and tilted — the case a `canvas::Circle`
cannot express and a `canvas::Polygon` would only approximate. Each is turned by its
`transform`: a rotation, clockwise from +X. A transform turns a shape about its
`position`, which for an ellipse is its centre, so the two eyes lean towards each other
without moving:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT math

FUNC turn(angle AS Float) AS canvas::Transform
  LET co AS Float = math::cos(angle)
  LET si AS Float = math::sin(angle)
  RETURN canvas::Transform[a := co, b := si, c := 0.0 - si, d := co, tx := 0.0, ty := 0.0]
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET yellow AS color::Color = color::rgb(255, 255, 0)
  LET green AS color::Color = color::rgb(0, 160, 0)

  LET face AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 200.0, y := 200.0], radius := 150.0, paint := canvas::fill(yellow), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET eyeL AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 150.0, y := 160.0], radiusX := 30.0, radiusY := 12.0, paint := canvas::fill(green), transform := turn(0.0 - 0.35)]
  LET eyeR AS canvas::DrawItem = canvas::Ellipse[id := "", position := vector::Float2[x := 250.0, y := 160.0], radiusX := 30.0, radiusY := 12.0, paint := canvas::fill(green), transform := turn(0.35)]
  LET smile AS canvas::DrawItem = canvas::Arc[id := "", position := vector::Float2[x := 200.0, y := 215.0], radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := WITH canvas::stroke(green, 14.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]

  canvas::present([face, eyeL, eyeR, smile])
END FUNC
```

Give an ellipse equal radii and it is a circle — exactly, not nearly — so an animation
that squashes a circle can hold one item type throughout rather than switching at the
moment the radii happen to match.

A button that lights up under the mouse. It is presented once, and from then on the
window shows its `hoverPaint` while the mouse rests on it and its `pressedPaint` while
a mouse button is held on it — with no further code, and even while the program is busy
or waiting. A `canvas::AutoPaint` has no default value, so every field is written out;
the two flashes are switched off here, and their paints are never shown:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT box AS canvas::Rectangle
  box = WITH box { id := "ok", position := vector::Float2[x := 100.0, y := 100.0], size := canvas::Size[width := 200.0, height := 80.0], paint := canvas::fill(color::rgb(40, 90, 200)) }
  LET unused AS canvas::Paint = canvas::fill(color::rgb(0, 0, 0))
  LET button AS canvas::AutoPaint = canvas::AutoPaint[shape := box, useEnter := FALSE, useExit := FALSE, useHover := TRUE, usePressed := TRUE, enterPaint := unused, exitPaint := unused, hoverPaint := canvas::fill(color::rgb(70, 130, 255)), pressedPaint := canvas::fill(color::rgb(20, 50, 140))]
  LET item AS canvas::DrawItem = button

  canvas::present([item])
END FUNC
```

A fountain of sparks: one `canvas::ParticleSystem`, drawn as up to 200 particles. The
system is the shape to copy (`item`), the shape to scatter the copies over (`emitter`,
here a short line), and three records saying how many there are (`canvas::Emission`),
how they move (`canvas::Motion`) and how they look as they age (`canvas::Appearance`).
Nothing moves by itself. The loop is what animates it: each frame it adds the time since
the last one to a clock and presents the system again with that clock as its `time`:

```
IMPORT app
IMPORT canvas
IMPORT color
IMPORT math
IMPORT vector

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT dot AS canvas::Circle
  LET spark AS canvas::AutoShape = WITH dot { radius := 3.0, paint := canvas::fill(color::rgb(255, 255, 255)) }
  MUT spout AS canvas::Line
  LET emitter AS canvas::Shape = WITH spout { startAt := vector::Float2[x := 0.0 - 20.0, y := 0.0], endAt := vector::Float2[x := 20.0, y := 0.0] }
  LET emission AS canvas::Emission = canvas::Emission[mode := canvas::EmissionMode.Continuous, maxParticles := 200, rate := 100.0, burst := 0, burstPeriod := 0.0, lifetime := 2.0, lifetimeVar := 0.2]
  MUT still AS canvas::Motion
  LET motion AS canvas::Motion = WITH still { speed := 260.0, speedVar := 0.2, angle := 0.0 - math::pi / 2.0, spread := 0.3, gravityY := 300.0 }
  LET look AS canvas::Appearance = canvas::Appearance[startColor := color::rgb(255, 230, 120), endColor := color::rgba(255, 60, 0, 0), startScale := 1.0, endScale := 0.4, scaleVar := 0.2]

  MUT clock AS Float = 0.0
  MUT frame AS Integer = 0
  WHILE frame < 120
    LET stats AS canvas::FrameStats = canvas::getFrameStats()
    clock = clock + math::min(stats.dt, 0.1)
    LET fountain AS canvas::DrawItem = canvas::ParticleSystem[position := vector::Float2[x := 450.0, y := 500.0], item := spark, emitter := emitter, emission := emission, motion := motion, appearance := look, seed := 1, time := clock]
    canvas::present([fountain])
    canvas::waitFrame()
    frame = frame + 1
  END WHILE
END FUNC
```

A firework: the other `canvas::EmissionMode`. `Burst` releases `burst` particles
together, here from a single point — a `canvas::Circle` of radius `0.0` — in every
direction (`spread := math::pi`). This draws the burst as it is 0.8 seconds after it
went off; the same `seed` and `time` always draw the same picture, so this is a still:

```
IMPORT app
IMPORT canvas
IMPORT color
IMPORT math
IMPORT vector

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT dot AS canvas::Circle
  LET spark AS canvas::AutoShape = WITH dot { radius := 3.0, paint := canvas::fill(color::rgb(255, 255, 255)) }
  LET point AS canvas::Shape = dot
  MUT none AS canvas::Emission
  LET emission AS canvas::Emission = WITH none { mode := canvas::EmissionMode.Burst, maxParticles := 150, burst := 150, lifetime := 2.0, lifetimeVar := 0.3 }
  MUT still AS canvas::Motion
  LET motion AS canvas::Motion = WITH still { speed := 180.0, speedVar := 0.5, spread := math::pi, gravityY := 120.0, drag := 0.8 }
  LET look AS canvas::Appearance = canvas::Appearance[startColor := color::rgb(255, 255, 255), endColor := color::rgba(120, 220, 255, 0), startScale := 1.0, endScale := 0.3, scaleVar := 0.0]
  LET firework AS canvas::DrawItem = canvas::ParticleSystem[position := vector::Float2[x := 450.0, y := 300.0], item := spark, emitter := point, emission := emission, motion := motion, appearance := look, seed := 7, time := 0.8]

  canvas::present([firework])
END FUNC
```
"#;

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

' plan-172-I: what `canvas::pollMouse` asks about the scene this thread last presented
' before it builds anything for it (`helper_hit.rs`). Worker-thread globals, like the
' signature above.
'
' `__CANVAS_SCENE_REV` counts the presents that changed something -- the scene's items
' or a group they name -- so a draw list built for one revision is known stale at the
' next. `__CANVAS_SCENE_IDS` is `canvas::sceneIdBits` of that scene: 0 when no item has
' an `id` and none is a group node, which is every scene written before `id` existed,
' and such a scene costs a poll one compare.
MUT __CANVAS_SCENE_REV AS Integer = 0
MUT __CANVAS_SCENE_IDS AS Integer = 0

FUNC __canvas_present(items AS List OF DrawItem) AS Nothing
  ' Two independent reasons to redraw, and both must be consulted.
  '
  ' `sceneUnchanged` compares the item list's content, which catches a scene whose
  ' ITEMS changed. It cannot catch a scene whose items are identical while a group's
  ' contents were replaced -- a `Group` node is two floats and a string pointer, all
  ' three unchanged by `setGroup` under the same name.
  '
  ' The signature is that second reason. Note the ordering: the scene is compared and,
  ' if changed, published FIRST, because that is what installs it; the signature is
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
  ' plan-175-D: compare first, against the scene installed NOW, with nothing copied. An
  ' unchanged scene of no group then costs this present the compare and nothing else:
  ' no copy, no free, no carried hashes, and no list built at all.
  LET unchanged AS Boolean = canvas::sceneUnchanged(items)
  ' plan-175-D: the signature walk reads `Group` nodes and `ParticleSystem`s and nothing
  ' else, so a scene with neither at its top level has the empty signature -- without
  ' an MFBASIC pass over every item to find that out.
  MUT moved AS Boolean = FALSE
  IF canvas::sceneNamesGroups(items) THEN
    LET sig AS List OF Integer = __canvas_groupSignature(items, 0)
    moved = NOT __canvas_intListEquals(sig, __CANVAS_LAST_GROUP_SIG)
    __CANVAS_LAST_GROUP_SIG = sig
  ELSE
    IF len(__CANVAS_LAST_GROUP_SIG) > 0 THEN
      moved = TRUE
      __CANVAS_LAST_GROUP_SIG = []
    END IF
  END IF
  ' plan-155-B: every present is counted, identical ones included, so the thread (and
  ' with it the mutex the count is kept under) starts on the first present of any
  ' scene. `ensureGraphics` is idempotent -- one Boolean test after the first call.
  __canvas_ensureGraphics()
  canvas::notePresent(canvas::frameNanos())
  IF NOT unchanged OR moved THEN
    __canvas_presentInstall(items, NOT unchanged)
    __CANVAS_SCENE_REV = __CANVAS_SCENE_REV + 1
    __CANVAS_SCENE_IDS = canvas::sceneIdBits(items)
    canvas::signalRedraw(FALSE)
    canvas::syncFrame()
  END IF
END FUNC

' Install a present's scene (when `changed`) and its hashes.
'
' bug-686: the hashes of the items that did not change, carried from the scene installed
' NOW -- so the carry comes before `publishScene` replaces it. A present whose only news
' is a group (`changed` FALSE) carries against the scene still installed, which has this
' scene's content, and publishes hashes alone. `publishScene` is told not to compare:
' `sceneUnchanged` already did, and found a change.
FUNC __canvas_presentInstall(items AS List OF DrawItem, changed AS Boolean) AS Nothing
  LET carried AS List OF Integer = canvas::carriedHashes(items)
  IF changed THEN
    LET published AS Boolean = canvas::publishScene(items, FALSE)
  END IF
  canvas::publishHashes(__canvas_hashScene(items, carried))
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
            errors: vec![
                "ErrWrongMode",
                "ErrDepthExceeded",
                // plan-150-A: a `canvas::ParticleSystem` whose `lifetime` is zero or
                // less (`__canvas_groupSignature`).
                "ErrCanvasParticleLifetime",
                "ErrOutOfMemory",
                "ErrOverflow",
            ],
            body: Body::mfb(BODY, "__canvas_present"),
        }],
    });
}
