//! `canvas::waitFrame` — pace an animation loop to the display (plan-155-C).
//!
//! An MFBASIC wrapper over the native `canvas::awaitPickup`, which is where the wait
//! and its reasoning live (`emit_await_pickup`). `present` stays non-blocking; this is
//! the one call an animation loop adds to be paced.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str =
    r#"Wait until the next frame is due, so an animation loop runs at the display's pace."#;

const DESC: &str = r#"`waitFrame` is the last call in an animation loop's body. It returns once **both**
of these hold:

1. the frame your last `canvas::present` asked for has started drawing, and
2. at least one display refresh interval has passed since `waitFrame` last returned
   — `1 / displayHz` seconds, or 1/60 s when the display rate is not known.

So a loop of `present` then `waitFrame` runs at the display's refresh rate, whether its
scene changes every time or not at all, and never faster: an unchanged scene or a cheap
one still waits out the interval rather than spinning. And because it waits only for the
frame to *start*, your program builds its next scene while that frame is being drawn.

`canvas::present` itself never waits. A program that presents progress while it
computes, and never calls `waitFrame`, is not slowed down at all.

`waitFrame` never causes a frame: calling it with nothing presented just waits out the
interval. Read the step to advance your animation by from `canvas::getFrameStats().dt`,
which is the actual time between your last two presents — do not assume one interval
passed, because a slow frame takes longer.

Requires `app::Mode.Canvas`; elsewhere it raises the trappable `ErrWrongMode`.

`ErrOutOfMemory` means the program ran out of memory."#;

const EX: &str = r#"A square sliding across the window at 100 pixels a second, whatever the display's
refresh rate. The step is clamped so that one long pause does not make it jump:

```
IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT math

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT x AS Float = 0.0
  MUT i AS Integer = 0
  WHILE i < 60
    LET s AS canvas::FrameStats = canvas::getFrameStats()
    x = x + 100.0 * math::min(s.dt, 0.05)
    LET box AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := x, y := 40.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(0, 150, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
    canvas::present([box])
    canvas::waitFrame()
    i = i + 1
  END WHILE
END FUNC
```

At 60 Hz this runs for about a second, and at 120 Hz for about half a second — the
square covers the same distance per second either way."#;

/// The interval is `1e12 / displayMilliHz` nanoseconds, or 1/60 s when the rate is
/// unknown. The mode gate comes first: `ensureGraphics` would otherwise start a render
/// thread for a program that is not in canvas mode.
const BODY: &str = r#"FUNC __canvas_waitFrame() AS Nothing
  canvas::requireCanvasMode()
  __canvas_ensureGraphics()
  LET mhz AS Integer = canvas::displayMilliHz()
  MUT interval AS Integer = 16666667
  IF mhz > 0 THEN
    interval = 1000000000000 / mhz
  END IF
  canvas::awaitPickup(interval)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "waitFrame",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrWrongMode", "ErrOutOfMemory"],
            body: Body::mfb(BODY, "__canvas_waitFrame"),
        }],
    });
}
