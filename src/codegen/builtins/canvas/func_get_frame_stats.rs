//! `canvas::getFrameStats` — how often the program presents and the backend draws
//! (plan-155-B).
//!
//! The MFBASIC body turns one native snapshot, `canvas::frameCounters`, into the
//! record. One snapshot rather than a call per field is the point: `presents` and
//! `frames` are written by different threads, and `pps` against `fps` is only a
//! comparison when both were read at the same instant.

use crate::codegen::registry::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::codegen::runtime::canvas::{FRAME_COUNTER_HEADER_WORDS, FRAME_RING_SLOTS};
use crate::types::ParameterType;
use std::sync::LazyLock;

const INTRO: &str = r#"Measure how fast your program presents, how fast frames are drawn, and how fast the display refreshes."#;

const DESC: &str = r#"`getFrameStats` returns a `canvas::FrameStats` record: one snapshot of the canvas's
timing, taken at the moment of the call. Every field comes from the same instant,
so comparing two of them is meaningful.

* `dt` is the time in seconds between your last two `canvas::present` calls — the
  step to advance an animation by. It is `0.0` until you have presented twice.
* `pps` is how many times you called `present` in the last second, and `fps` how
  many frames were actually drawn in it. Both fall to `0.0` within a second of the
  activity stopping.
* `presents` and `frames` are the totals since the program started.
* `frameSeconds` is how long the last drawn frame took to draw, `0.0` before the
  first.
* `renderer` is what drew the last frame, and `Software` before the first.
* `displayHz` is the refresh rate of the monitor the window is on, `0.0` when it
  is not known (no window yet, or the system does not say).

**`fps` is not `pps`.** A `present` of a scene identical to the one on screen draws
nothing, so a program re-presenting an unchanged picture reports a high `pps` and an
`fps` of `0.0` — that is the canvas doing no work, not a stall. A static scene reports
`fps = 0.0` for the same reason. When the scene *does* change every time and `fps`
is still below `pps`, you are presenting faster than frames are drawn, and the
presents in between are never shown.

Calling `getFrameStats` never causes a frame, and it works in any presentation
mode. Before the first `present` every count is `0`."#;

const EX: &str = r#"An animation loop that moves a square by the time that has actually passed, so it
travels at the same speed whatever the frame rate, and reports its rates at the end:

```
IMPORT app
IMPORT canvas
IMPORT color
IMPORT io
IMPORT math
IMPORT os

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT x AS Float = 0.0
  MUT i AS Integer = 0
  WHILE i < 30
    LET s AS canvas::FrameStats = canvas::getFrameStats()
    x = x + 120.0 * math::min(s.dt, 0.05)
    LET box AS canvas::DrawItem = canvas::Rectangle[x := x, y := 40.0, w := 40.0, h := 40.0, paint := canvas::fill(color::rgb(240, 160, 0))]
    canvas::present([box])
    os::sleep(10)
    i = i + 1
  END WHILE
  LET s AS canvas::FrameStats = canvas::getFrameStats()
  io::print("presents: " & toString(s.presents))
END FUNC
```

This prints `presents: 30`. The square moves by `dt`, not by one fixed step per loop,
so a slow frame moves it further rather than making the animation run slow."#;

/// The rate over the trailing second, and the step between the last two presents.
///
/// `__canvas_rate` counts the ring entries newer than a second. Until all
/// `FRAME_RING_SLOTS` are that new, that count is the rate exactly; once they all are,
/// the ring spans less than a second and the rate is `(slots - 1)` intervals over the
/// ring's span — so a loop presenting thousands of times a second is measured rather
/// than reported as `slots`.
static BODY: LazyLock<String> = LazyLock::new(body);

fn body() -> String {
    let ring = FRAME_RING_SLOTS;
    let present_ring = FRAME_COUNTER_HEADER_WORDS;
    let drawn_ring = FRAME_COUNTER_HEADER_WORDS + FRAME_RING_SLOTS;
    format!(
        r#"FUNC __canvas_rate(counters AS List OF Integer, first AS Integer, total AS Integer, now AS Integer) AS Float
  MUT held AS Integer = total
  IF held > {ring} THEN
    held = {ring}
  END IF
  MUT recent AS Integer = 0
  MUT newest AS Integer = 0
  MUT oldest AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < held
    LET t AS Integer = collections::getOr(counters, first + i, 0)
    IF t > now - 1000000000 THEN
      recent = recent + 1
    END IF
    IF i = 0 OR t > newest THEN
      newest = t
    END IF
    IF i = 0 OR t < oldest THEN
      oldest = t
    END IF
    i = i + 1
  END WHILE
  IF recent < {ring} OR newest <= oldest THEN
    RETURN toFloat(recent)
  END IF
  RETURN toFloat({ring} - 1) * 1000000000.0 / toFloat(newest - oldest)
END FUNC

FUNC __canvas_getFrameStats() AS FrameStats
  __canvas_readDisplayOverride()
  LET c AS List OF Integer = canvas::frameCounters(canvas::frameNanos())
  LET now AS Integer = collections::getOr(c, 0, 0)
  LET presents AS Integer = collections::getOr(c, 1, 0)
  LET presentHead AS Integer = collections::getOr(c, 2, 0)
  LET drawn AS Integer = collections::getOr(c, 3, 0)
  LET nanos AS Integer = collections::getOr(c, 5, 0)
  LET ordinal AS Integer = collections::getOr(c, 6, 0)
  MUT dt AS Float = 0.0
  IF presents >= 2 THEN
    LET last AS Integer = collections::getOr(c, {present_ring} + (presentHead + {ring} - 1) MOD {ring}, 0)
    LET before AS Integer = collections::getOr(c, {present_ring} + (presentHead + {ring} - 2) MOD {ring}, 0)
    dt = toFloat(last - before) / 1000000000.0
  END IF
  MUT drewWith AS Renderer = Renderer.Software
  IF ordinal = 1 THEN
    drewWith = Renderer.Metal
  END IF
  IF ordinal = 2 THEN
    drewWith = Renderer.Vulkan
  END IF
  RETURN FrameStats[dt := dt, pps := __canvas_rate(c, {present_ring}, presents, now), fps := __canvas_rate(c, {drawn_ring}, drawn, now), displayHz := toFloat(collections::getOr(c, 7, 0)) / 1000.0, frameSeconds := toFloat(nanos) / 1000000000.0, presents := presents, frames := drawn, renderer := drewWith]
END FUNC"#
    )
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getFrameStats",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::named("FrameStats"),
            errors: vec!["ErrOutOfMemory"],
            body: Body::mfb(BODY.as_str(), "__canvas_getFrameStats"),
        }],
    });
}
