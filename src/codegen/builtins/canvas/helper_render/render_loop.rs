//! plan-176-S: `helper_render`'s MFBASIC text for the render loop — graphics setup, test switches, the loop macros and their `concat!`s, phase timers and the pointer script. Registration lives in
//! `builtins/canvas/helper_render/mod.rs`.

/// Start the graphics thread on the first present, and settle sync mode and the
/// renderer with it.
///
/// The guard makes this one `os::getEnvOr` per program rather than per frame, and it
/// keeps the environment read in MFBASIC — where it is already portable — instead of
/// putting a per-platform `getenv` on the spawn path.
///
/// **The renderer.** A program in a real window draws on the GPU (Metal on macOS,
/// Vulkan on Linux and Windows, where a pipeline exists); a headless run
/// (`MFB_HEADLESS`) draws in software.
/// `MFB_CANVAS_GPU` overrides both ways: `0` forces software, any other value
/// forces the GPU. The software renderer is the exact-match oracle the goldens and
/// the GPU backends are measured against, and every test that renders runs
/// headless, so it stays the default *there*; a window is where frame rate is
/// felt — a full-window canvas redraw in software runs at about 8 frames a second
/// on Apple silicon, against about 55 on Metal. A GPU that is unavailable, or a
/// scene it declines, still falls back to software per frame
/// (`__canvas_renderFrame`).
#[rustfmt::skip]
pub(super) const ENSURE_GRAPHICS: &str =
r#"MUT __CANVAS_GFX_READY AS Boolean = FALSE

FUNC __canvas_ensureGraphics() AS Nothing
  IF NOT __CANVAS_GFX_READY THEN
    __canvas_readSyncMode()
    canvas::setGpuMode(__canvas_wantGpu())
    __canvas_readDisplayOverride()
    canvas::startGraphics()
    __CANVAS_GFX_READY = TRUE
  END IF
END FUNC

FUNC __canvas_wantGpu() AS Boolean
  LET choice AS String = os::getEnvOr("MFB_CANVAS_GPU", "")
  IF len(choice) > 0 THEN RETURN choice <> "0"
  IF len(os::getEnvOr("MFB_HEADLESS", "")) > 0 THEN RETURN FALSE
  RETURN TRUE
END FUNC

' Make `present` wait for the frame it asked for. Any non-empty value turns it on.
'
' A test switch, yet read by EVERY build rather than only `--debug` like the others
' (`./mfb spec tooling debug-report`): while canvas is in development,
' `tools/canvas-bench`'s `sweep --release` needs one rendered frame per present in a
' release build, to measure fps without a `--debug` build's instrumentation. It moves
' behind `--debug` once canvas settles.
FUNC __canvas_readSyncMode() AS Nothing
  canvas::setSyncMode(len(os::getEnvOr("MFB_CANVAS_SYNC", "")) > 0)
END FUNC"#;

/// The two switches a test sets to make a headless run observable beyond
/// `MFB_CANVAS_SYNC`: report a pinned display rate, and hold the graphics thread
/// inside a frame. A normal build has neither — each body is empty, which leaves the
/// graphics state's display-override word at the zero it starts with.
pub(super) const TEST_SWITCHES: &str = r#"FUNC __canvas_readDisplayOverride() AS Nothing
END FUNC

FUNC __canvas_frameHold() AS Nothing
END FUNC"#;

/// [`TEST_SWITCHES`] for a `--debug` build, which is the only build that reads
/// `MFB_CANVAS_DISPLAY_HZ` and `MFB_CANVAS_FRAME_HOLD_MS`.
#[rustfmt::skip]
pub(super) const TEST_SWITCHES_DEBUG: &str =
r#"' plan-155-A: pin the display rate the runtime reports, since a headless run has no
' monitor to ask. Read once, by whichever asks first: the first present, or
' (plan-155-B) a `canvas::getFrameStats` made before it.
MUT __CANVAS_DISPLAY_OVERRIDE_READ AS Boolean = FALSE

FUNC __canvas_readDisplayOverride() AS Nothing
  IF NOT __CANVAS_DISPLAY_OVERRIDE_READ THEN
    ' bug-807: a malformed or out-of-range override is ignored, as if unset. It is a
    ' harness affordance, so it must never make a present or a frame-stats read fail.
    MUT hz AS Integer = toInt(os::getEnvOr("MFB_CANVAS_DISPLAY_HZ", "0")) TRAP(e)
      RECOVER 0
    END TRAP
    IF hz < 0 OR hz > 1000000 THEN
      hz = 0
    END IF
    canvas::setDisplayOverride(hz * 1000)
    __CANVAS_DISPLAY_OVERRIDE_READ = TRUE
  END IF
END FUNC

' plan-116-G, G9: hold the graphics thread INSIDE a frame, for tests that need a
' worker action to land mid-render. Off unless the variable is set.
FUNC __canvas_frameHold() AS Nothing
  LET holdMs AS Integer = toInt(os::getEnvOr("MFB_CANVAS_FRAME_HOLD_MS", "0"))
  IF holdMs > 0 THEN
    os::sleep(holdMs)
  END IF
END FUNC"#;

// `__canvas_renderLoop`/`__canvas_renderFrame` are one body in both builds except for
// one line: a skipped frame writes the stats line only in a `--debug` build
// (plan-130-E). The shared text is split at that line so it exists once.
macro_rules! render_loop_head {
    () => {
        r#"' plan-172-K: why the render loop woke -- 1, a redraw is owed; 2, nothing is owed but
' the pointer changed or an `AutoPaint` window ended (`canvas::waitForRedraw`).
MUT __CANVAS_WAKE AS Integer = 1

' bug-996: the Vulkan pipeline is never on a frame's critical path. Building it can take
' tens of seconds on a driver that has not seen this executable, so the first frame that
' wants the GPU only STARTS the build -- on a thread of its own (`canvas::vulkanBuild(0)`)
' -- and every frame asks `canvas::vulkanReady()`, which is one load. Frames are drawn in
' software until it answers TRUE, and by Vulkan from the first frame after.
'
' A program that asked for the GPU by name (`MFB_CANVAS_GPU`) builds here and now instead
' (`canvas::vulkanBuild(1)`): its first frame waits for the pipeline, once, so a test's
' every frame is a GPU frame. This is only reached with the GPU selected, so a set
' variable here is one that selected it.
MUT __CANVAS_VULKAN_ASKED AS Boolean = FALSE

FUNC __canvas_vulkanReady() AS Boolean
  IF NOT __CANVAS_VULKAN_ASKED THEN
    __CANVAS_VULKAN_ASKED = TRUE
    IF len(os::getEnvOr("MFB_CANVAS_GPU", "")) > 0 THEN
      canvas::vulkanBuild(1)
    ELSE
      canvas::vulkanBuild(0)
    END IF
  END IF
  RETURN canvas::vulkanReady()
END FUNC

' bug-1087: bug-996's rule, on Metal. macOS compiles the shader the first time an app
' name asks for it -- seconds for a name never run before -- so the first frame that
' wants the GPU only STARTS the build on a thread of its own (`canvas::metalBuild(0)`),
' every frame asks `canvas::metalReady()`, which is one load, and frames are software
' until it answers TRUE. `MFB_CANVAS_GPU` builds here and now instead
' (`canvas::metalBuild(1)`), so a test's every frame is a GPU frame.
MUT __CANVAS_METAL_ASKED AS Boolean = FALSE

FUNC __canvas_metalReady() AS Boolean
  IF NOT __CANVAS_METAL_ASKED THEN
    __CANVAS_METAL_ASKED = TRUE
    IF len(os::getEnvOr("MFB_CANVAS_GPU", "")) > 0 THEN
      canvas::metalBuild(1)
    ELSE
      canvas::metalBuild(0)
    END IF
  END IF
  RETURN canvas::metalReady()
END FUNC

FUNC __canvas_renderLoop() AS Nothing
  ' plan-172-K: the wait is timed while an `AutoPaint` is inside an enter or exit window
  ' -- the one time-based redraw there is -- and untimed (0) otherwise. It answers 0
  ' when the loop is to stop.
  __CANVAS_WAKE = canvas::waitForRedraw(__canvas_autoTimeout())
  WHILE __CANVAS_WAKE <> 0
    ' plan-155-B: time the frame and report whether -- and by which renderer -- it
    ' was drawn, for `canvas::getFrameStats`. `__canvas_renderFrame` sets the two
    ' globals on every path out of it.
    LET started AS Integer = canvas::frameNanos()
    __canvas_renderFrame()
    LET finished AS Integer = canvas::frameNanos()
    canvas::frameDone(__CANVAS_LAST_DREW, __CANVAS_LAST_RENDERER, finished - started, finished)
    __CANVAS_WAKE = canvas::waitForRedraw(__canvas_autoTimeout())
  END WHILE
END FUNC

FUNC __canvas_renderFrame() AS Nothing
  ' Not drawn until a backend says so: the skip below returns with these unchanged.
  __CANVAS_LAST_DREW = FALSE
  __CANVAS_LAST_RENDERER = 0
  ' plan-172-J: a `--debug` build's scripted pointer, applied before anything in the
  ' frame could read the pointer. Empty in a normal build.
  __canvas_pointerScript()
  ' plan-172-K: a frame nobody is owed -- the pointer changed, or an `AutoPaint` window
  ' ended -- is drawn only if some `AutoPaint` now shows a different paint, and that is
  ' asked before anything is walked (`__canvas_autoLight`). A pointer crossing empty
  ' canvas, or moving within a shape, draws nothing and walks nothing, whatever
  ' `MFB_CANVAS_DAMAGE` says. When the answer is yes the frame is an ordinary one from
  ' here on.
  IF __CANVAS_WAKE = 2 THEN
    IF NOT __canvas_autoLight() THEN
      __CANVAS_DAMAGE = []
      __canvas_frameSkipped()
      RETURN
    END IF
  END IF
  ' plan-172-E: `canvas::Size` holds Floats, for a shape's extent. The surface is whole
  ' pixels and every consumer below wants them as Integers, so they are read directly.
  LET surfaceW AS Integer = canvas::surfaceWidth()
  LET surfaceH AS Integer = canvas::surfaceHeight()
  __CANVAS_VIEWPORT_W = surfaceW
  __CANVAS_VIEWPORT_H = surfaceH
  __canvas_phaseMark(0)
  ' The geometry is built once, here, and the offsets are then handed to whichever
  ' backend draws them. It has to happen before the damage diff rather than inside the
  ' renderer: an item's damaged rectangle is its geometry's bounds, so there is no
  ' diff to compute until the geometry exists.
  LET offsets AS List OF Integer = __canvas_sceneOffsets()
  __canvas_phaseMark(1)
  ' plan-116-H Phase 1: build the GPU draw list beside the software one. Nothing
  ' consumes it yet -- both backends still decline a group scene -- but it is built every
  ' frame so `MFB_CANVAS_STATS` can report it, which is the only way a test can see a
  ' structure that exists on the graphics thread and is handed straight to an emitter.
  '
  ' bug-686: it lays the list out from the offsets `__canvas_sceneOffsets` just resolved
  ' rather than resolving every item again.
  LET draws AS List OF Integer = __canvas_sceneDraws()
  __canvas_phaseMark(2)
  ' plan-116-G, G9: hold the graphics thread INSIDE a frame, for tests that need a
  ' worker action to land mid-render.
  '
  ' Here rather than anywhere else in the frame because this is the point after which
  ' the renderer is committed: `__canvas_sceneOffsets` has resolved every group name and
  ' read each group's items block, so a `removeGroup` arriving during the hold is
  ' exactly the race the drain gate exists for -- the block is retired while a frame is
  ' demonstrably still working from it.
  '
  ' A `--debug` build's `MFB_CANVAS_FRAME_HOLD_MS`; nothing in a normal build. Before
  ' this, every "mid-render" row was reached through `MFB_CANVAS_RESIZE_W`/`_H` firing
  ' while the worker slept, which is resize-specific -- so the rows that needed a
  ' different worker action were either untestable (R1) or tested by luck.
  __canvas_frameHold()
  ' plan-116-G: the EXPANDED hashes, written by the walk above, not
  ' `canvas::installedHashes()`. The damage diff pairs hashes with offsets by index, and
  ' expanding one `Group` node into N children makes those two lists different lengths
  ' -- which `__canvas_damageFor` detects and answers with a full redraw, so a group
  ' scene would silently never take the partial path. For a group-free scene this list
  ' is `installedHashes()` passed through unchanged.
  LET hashes AS List OF Integer = __CANVAS_DRAW_HASHES
  LET damage AS List OF Integer = __canvas_damageFor(hashes, offsets, surfaceW, surfaceH)
  __CANVAS_DAMAGE = damage
  __canvas_phaseMark(3)
  IF len(damage) = 0 THEN
    ' Nothing changed, so nothing is presented. The loop still calls `frameDone`, so a
    ' `present` waiting under MFB_CANVAS_SYNC is released -- a skipped frame is a frame
    ' that finished, not one that was lost.
    __CANVAS_SKIPPED = __CANVAS_SKIPPED + 1
"#
    };
}

macro_rules! render_loop_tail {
    () => {
        r#"    RETURN
  END IF
  __CANVAS_FRAMES = __CANVAS_FRAMES + 1
  IF NOT __canvas_damageIsFull(damage, surfaceW, surfaceH) THEN
    __CANVAS_PARTIAL = __CANVAS_PARTIAL + 1
  END IF
  __canvas_rememberScene(hashes, offsets)

  ' A GPU backend renders the whole frame whatever the damage is: it draws into its own
  ' texture and reads the result back, so there is no kept surface for it to preserve.
  ' That is the plan's "capability-absent path falls back to full-frame", and it is the
  ' honest reading here -- the capability the plan named (`VK_KHR_incremental_present`,
  ' Metal dirty rects) belongs to presenting a swapchain drawable, and the direct
  ' present paths (bug-686 Phase 4, bug-688 V6) redraw every drawable whole rather than
  ' track which regions of it are stale (Correction 24).
  ' plan-155-B: the renderer ordinals are `canvas::Renderer`'s -- 0 Software, 1 Metal,
  ' 2 Vulkan. A GPU arm that declines leaves them for the next arm to set.
  ' bug-996, bug-1087: each is FALSE while its pipeline is still being built, so this
  ' frame is software.
  IF canvas::useGpu() AND __canvas_metalReady() THEN
    IF __canvas_renderMetal(surfaceW, surfaceH) THEN
      __CANVAS_LAST_DREW = TRUE
      __CANVAS_LAST_RENDERER = 1
      __canvas_phaseMark(4)
      RETURN
    END IF
  END IF
  IF canvas::useGpu() AND __canvas_vulkanReady() THEN
    IF __canvas_renderVulkan(surfaceW, surfaceH) THEN
      __CANVAS_LAST_DREW = TRUE
      __CANVAS_LAST_RENDERER = 2
      __canvas_phaseMark(4)
      RETURN
    END IF
  END IF
  __canvas_renderScene(offsets, damage, surfaceW, surfaceH)
  __CANVAS_LAST_DREW = TRUE
  __CANVAS_LAST_RENDERER = 0
  __canvas_phaseMark(4)
END FUNC"#
    };
}

/// The graphics thread's whole life: wait, render, repeat — and the renderer seam.
///
/// `__canvas_renderFrame` is the one place that will choose a renderer (plan-98-E).
/// It is deliberately a runtime branch rather than a build-time one, because the
/// choice is a runtime fact: whether a Metal device exists, and whether the program
/// asked for it (`canvas::metalAvailable` and `canvas::useGpu` answer those).
///
/// There are two GPU arms — Metal on macOS, Vulkan on Linux and Windows — and each is
/// taken only when all three of its conditions hold: the program asked for a GPU
/// (`canvas::useGpu`, which despite its name is the one renderer-selection flag and
/// is set by `__canvas_wantGpu`), a pipeline exists (`canvas::metalReady` /
/// `canvas::vulkanReady`), and the *scene* is one that renderer draws correctly.
///
/// The second condition is asked every frame, and on both backends it only reads
/// whether the pipeline exists yet (bug-996 for Vulkan, bug-1087 for Metal):
/// `__canvas_metalReady` / `__canvas_vulkanReady` start the build on its own thread the
/// first time and the frames before it finishes are software frames. So the renderer
/// of a run can change once, from software to the GPU, at the first frame after the
/// pipeline is ready. `MFB_CANVAS_GPU` set builds on the graphics thread at the first
/// GPU frame instead, which then waits for it.
///
/// The two are mutually exclusive in practice — `metalReady` is FALSE off macOS and
/// `vulkanReady` FALSE on it — so the order between them never decides anything;
/// they are written as separate `IF`s rather than an `ELSE` so a future platform with
/// both is a matter of which is listed first, not a restructure.
///
/// That third condition is what keeps `MFB_CANVAS_GPU=1` honest: a backend that
/// drew a circle as its bounding box would still *report* success, which is exactly
/// the lie Correction 3 rejected. Since Phase 2 the shader reproduces every
/// primitive, so the only scene still declined is one carrying a polygon with more
/// edges than a `setFragmentBytes:` payload holds.
///
/// The software path stays the default for headless runs, which is every test that
/// renders: it is the oracle the GPU path is measured against, so it cannot become
/// the thing being measured. A real window asks for the GPU (`__canvas_wantGpu`).
///
/// It never returns. The wait is a real condition wait, so a static scene costs
/// nothing — no timer, no poll, no spin. Time is a redraw trigger in exactly one case
/// (`.ai/canvas-threading.md` §4): a `canvas::AutoPaint` inside its enter or exit
/// window, whose end `__canvas_autoTimeout` hands the wait.
///
/// It renders the *installed* scene rather than being handed one, which is what lets
/// a repaint no `present` caused — a resize, an expose — draw the right picture.
pub(super) const RENDER_LOOP: &str = concat!(render_loop_head!(), render_loop_tail!());

/// bug-686 Phase 0: where a frame's time goes, reported on the `MFB_CANVAS_STATS` line.
///
/// `__canvas_renderFrame` marks five points: the frame's start, then the end of the scene
/// walk, of the draw list, of the damage diff and of the render (the predicate, the
/// backend's own work, readback and present). A normal build's mark is an empty `FUNC … AS Nothing`;
/// a `--debug` build's accumulates the nanoseconds between consecutive marks per phase,
/// and `__canvas_phaseText` reports each total in milliseconds. Cumulative, like
/// `generations=`, because the interesting quantity is the delta between frames.
pub(super) const PHASE_TIMERS: &str = r#"FUNC __canvas_phaseMark(phase AS Integer) AS Nothing
END FUNC"#;

#[rustfmt::skip]
pub(super) const PHASE_TIMERS_DEBUG: &str = r#"MUT __CANVAS_PHASE_AT AS Integer = 0
MUT __CANVAS_PHASE_NS AS List OF Integer = [0, 0, 0, 0]

FUNC __canvas_phaseMark(phase AS Integer) AS Nothing
  LET now AS Integer = canvas::frameNanos()
  IF phase > 0 AND __CANVAS_PHASE_AT > 0 THEN
    LET spent AS Integer = collections::getOr(__CANVAS_PHASE_NS, phase - 1, 0) + (now - __CANVAS_PHASE_AT)
    __CANVAS_PHASE_NS = collections::set(__CANVAS_PHASE_NS, phase - 1, spent)
  END IF
  __CANVAS_PHASE_AT = now
END FUNC

FUNC __canvas_phaseText() AS String
  RETURN " phaseOffsetsMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 0, 0) / 1000000) & " phaseDrawsMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 1, 0) / 1000000) & " phaseDamageMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 2, 0) / 1000000) & " phaseRenderMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 3, 0) / 1000000) & __canvas_profileText()
END FUNC

' The GPU renderer's own split of `phaseRenderMs` (`CANVAS_PROFILE_SYMBOL`), cumulative
' like the phases and in microseconds, because one frame's share of a phase is often
' under a millisecond. Only the Metal renderer fills the block; elsewhere every field is 0.
' `profVsyncOff` is the one that is not cumulative: 1 once `MFB_CANVAS_VSYNC=0` has turned
' the window layer's display sync off.
FUNC __canvas_profileText() AS String
  RETURN " profAcquireUs=" & toString(canvas::profileWord(0) / 1000) & " profFillUs=" & toString(canvas::profileWord(1) / 1000) & " profEncodeUs=" & toString(canvas::profileWord(2) / 1000) & " profWaitUs=" & toString(canvas::profileWord(3) / 1000) & " profGpuUs=" & toString(canvas::profileWord(4) / 1000) & " profReadbackUs=" & toString(canvas::profileWord(5) / 1000) & " profPresentUs=" & toString(canvas::profileWord(6) / 1000) & " profPasses=" & toString(canvas::profileWord(7)) & " profBytes=" & toString(canvas::profileWord(8)) & " profTexels=" & toString(canvas::profileWord(9)) & " profDraws=" & toString(canvas::profileWord(10)) & " profVsyncOff=" & toString(canvas::profileWord(11))
END FUNC"#;

/// plan-172-J: the scripted pointer. A normal build has no script and no reader.
#[rustfmt::skip]
pub(super) const POINTER_SCRIPT: &str = r#"FUNC __canvas_pointerScript() AS Nothing
END FUNC

' plan-172-K: a frame that was woken and not drawn.
FUNC __canvas_frameSkipped() AS Nothing
  __CANVAS_SKIPPED = __CANVAS_SKIPPED + 1
END FUNC

' plan-172-K: when the script's next entry falls due, on `canvas::frameNanos`' clock, or
' 0 for none. There is no script in a normal build.
FUNC __canvas_pointerNextDue() AS Integer
  RETURN 0
END FUNC

' plan-172-K: how long an `AutoPaint`'s enter and exit paints are held, in nanoseconds:
' the project's `options.canvasAutoPaintMS`, 100 unless it says otherwise.
FUNC __canvas_autoWindow() AS Integer
  RETURN canvas::autoWindowMs() * 1000000
END FUNC"#;

/// The `--debug` build's scripted pointer: `MFB_CANVAS_POINTER` holds
/// `ms,x,y,buttons,inside;...`, and at the start of each frame every entry whose time
/// has come is published through `canvas::publishPointer` — the writer the platforms'
/// own pointer handlers call. It is what lets a headless test put the pointer
/// somewhere, on every platform including the one with no scriptable window.
///
/// Time is counted from the first frame, in the monotonic clock the phase timers read.
/// Entries are applied in order and each once; `inside` 0 is the pointer leaving the
/// surface. Anything that is not a digit separates numbers, so the `,` and `;` are for
/// the reader — and a count that is not a multiple of five ignores the tail.
///
/// On the GRAPHICS thread, which is the one deliberate departure from "the main thread
/// publishes": a script has no main-thread event to ride, and the writer is
/// single-writer either way, which a headless run with no real pointer satisfies.
#[rustfmt::skip]
pub(super) const POINTER_SCRIPT_DEBUG: &str = r#"MUT __CANVAS_POINTER_READ AS Boolean = FALSE
MUT __CANVAS_POINTER_SCRIPT AS List OF Integer = []
MUT __CANVAS_POINTER_NEXT AS Integer = 0
MUT __CANVAS_POINTER_T0 AS Integer = 0

FUNC __canvas_pointerNumbers(text AS String) AS List OF Integer
  LET bytes AS List OF Byte = encoding::utf8Encode(text)
  MUT out AS List OF Integer = []
  MUT value AS Integer = 0
  MUT open AS Boolean = FALSE
  MUT i AS Integer = 0
  WHILE i < len(bytes)
    LET b AS Integer = toInt(collections::getOr(bytes, i, toByte(0)))
    IF b >= 48 AND b <= 57 THEN
      value = value * 10 + (b - 48)
      open = TRUE
    ELSEIF open THEN
      out = collections::append(out, value)
      value = 0
      open = FALSE
    END IF
    i = i + 1
  END WHILE
  IF open THEN
    out = collections::append(out, value)
  END IF
  RETURN out
END FUNC

FUNC __canvas_pointerScript() AS Nothing
  IF NOT __CANVAS_POINTER_READ THEN
    __CANVAS_POINTER_READ = TRUE
    __CANVAS_POINTER_SCRIPT = __canvas_pointerNumbers(os::getEnvOr("MFB_CANVAS_POINTER", ""))
    __CANVAS_POINTER_T0 = canvas::frameNanos()
  END IF
  LET count AS Integer = len(__CANVAS_POINTER_SCRIPT) / 5
  IF __CANVAS_POINTER_NEXT >= count THEN
    RETURN
  END IF
  LET elapsedMs AS Integer = (canvas::frameNanos() - __CANVAS_POINTER_T0) / 1000000
  WHILE __CANVAS_POINTER_NEXT < count
    LET at AS Integer = __CANVAS_POINTER_NEXT * 5
    IF collections::getOr(__CANVAS_POINTER_SCRIPT, at, 0) > elapsedMs THEN
      RETURN
    END IF
    LET x AS Integer = collections::getOr(__CANVAS_POINTER_SCRIPT, at + 1, 0)
    LET y AS Integer = collections::getOr(__CANVAS_POINTER_SCRIPT, at + 2, 0)
    LET buttons AS Integer = collections::getOr(__CANVAS_POINTER_SCRIPT, at + 3, 0)
    LET inside AS Integer = collections::getOr(__CANVAS_POINTER_SCRIPT, at + 4, 0)
    canvas::publishPointer(x, y, buttons, inside)
    __CANVAS_POINTER_NEXT = __CANVAS_POINTER_NEXT + 1
  END WHILE
END FUNC

' plan-172-K: a frame that was woken and not drawn -- counted, and in this build it
' writes its stats line like any other frame the graphics thread considered.
FUNC __canvas_frameSkipped() AS Nothing
  __CANVAS_SKIPPED = __CANVAS_SKIPPED + 1
  __canvas_writeStats()
END FUNC

' plan-172-K: when the script's next entry falls due, on `canvas::frameNanos`' clock, or
' 0 when it has none left. A script entry is not a pointer EVENT -- nothing signals for
' it -- so a scene with an `AutoPaint` waits no longer than this (`__canvas_autoTimeout`)
' and the entry is applied at the start of the frame that wakes.
FUNC __canvas_pointerNextDue() AS Integer
  IF NOT __CANVAS_POINTER_READ THEN
    RETURN 0
  END IF
  IF __CANVAS_POINTER_NEXT * 5 + 4 >= len(__CANVAS_POINTER_SCRIPT) THEN
    RETURN 0
  END IF
  RETURN __CANVAS_POINTER_T0 + collections::getOr(__CANVAS_POINTER_SCRIPT, __CANVAS_POINTER_NEXT * 5, 0) * 1000000
END FUNC

' plan-172-K: how long an `AutoPaint`'s enter and exit paints are held, in nanoseconds
' -- the project's `options.canvasAutoPaintMS`, unless `MFB_CANVAS_AUTO_WINDOW_MS` says
' otherwise. The override is a test affordance, off by default: a flash that lasts a
' tenth of a second cannot be photographed by a test whose software frame takes longer
' than that to draw.
MUT __CANVAS_AUTO_WINDOW_NS AS Integer = 0

FUNC __canvas_autoWindow() AS Integer
  IF __CANVAS_AUTO_WINDOW_NS = 0 THEN
    LET built AS Integer = canvas::autoWindowMs()
    MUT ms AS Integer = toInt(os::getEnvOr("MFB_CANVAS_AUTO_WINDOW_MS", "0")) TRAP(e)
      RECOVER 0
    END TRAP
    IF ms < 1 OR ms > 3600000 THEN
      ms = built
    END IF
    __CANVAS_AUTO_WINDOW_NS = ms * 1000000
  END IF
  RETURN __CANVAS_AUTO_WINDOW_NS
END FUNC

' The pointer as the graphics thread sees it, for the stats line: `x,y,buttons,inside`.
FUNC __canvas_pointerText() AS String
  LET p AS List OF Integer = canvas::pointerSnapshot()
  RETURN " pointer=" & toString(collections::getOr(p, 0, 0)) & "," & toString(collections::getOr(p, 1, 0)) & "," & toString(collections::getOr(p, 2, 0)) & "," & toString(collections::getOr(p, 3, 0))
END FUNC"#;

/// [`RENDER_LOOP`] for a `--debug` build: a skipped frame also writes the
/// `MFB_CANVAS_STATS` line.
pub(super) const RENDER_LOOP_DEBUG: &str = concat!(
    render_loop_head!(),
    "    __canvas_writeStats()\n",
    render_loop_tail!()
);
