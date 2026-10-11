//! `canvas::AutoPaint`'s state (plan-172-K): which of its paints a shape shows, from
//! where the pointer is and which buttons are held — and the graphics thread's frame
//! step that keeps it.
//!
//! * [`AUTO_STEP`] is the whole rule as one pure function — no globals, no clock of its
//!   own, no drawing — so every rule in `planning/new_canvas.md` "AutoPaint states" is a
//!   row of inputs and an expected output (`tests/canvas/rt_canvas_auto_step.rs`, which
//!   compiles this text and runs it).
//! * [`RESOLVE`] is what calls it: the instances a frame's walk reached, the table that
//!   carries their state from frame to frame, and the swap of a draw entry for the
//!   record of the paint its instance shows.
//!
//! Both are one chunk gated on the program constructing an `AutoPaint`
//! ([`HelperGate::WhenUsedOrConstructed`]). The unconditional source calls into it by
//! name — the walks, the render loop, the `Text` builder — so a program with no
//! `AutoPaint` gets [`ABSENT`] instead ([`HelperGate::WhenNotConstructed`]), the same
//! names with the bodies that are right when there is nothing to resolve. A canvas
//! program therefore pays for this file only if it can hold an `AutoPaint`.
//!
//! **Threads.** [`RESOLVE`]'s globals are the graphics thread's: it is the only thread
//! that walks a frame (`.ai/canvas-threading.md` section 2). The worker never resolves
//! a state and `present` never waits on one.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

/// The record types whose construction opens this chunk.
pub(super) const CONSTRUCTED: &[&str] = &["AutoPaint"];

/// One instance, one frame.
///
/// **What is kept between frames** is a phase and, while a flash is running, the time
/// it ends:
///
/// | Phase | Meaning |
/// |---|---|
/// | 0 `OUT` | the pointer is not on the shape |
/// | 1 `IN` | it is, and any enter flash is over |
/// | 2 `ENTER` | it arrived less than a window ago |
/// | 3 `EXIT` | it left less than a window ago |
///
/// and whether this instance owns the press in progress: it does when the first button
/// went down with the pointer on it, and until every button is up.
///
/// **What is shown** follows from those, in the design's priority order — pressed,
/// enter, hover, exit, the shape's own paint — and a state whose `use…` flag is off is
/// never entered, which is what "falls through" means: with `useEnter` off an arrival
/// goes straight to `IN`, with `useExit` off a departure goes straight to `OUT`, and
/// neither starts a window, so neither asks for a frame 50 ms later.
///
/// The answer is packed, because a function returns one value and a list would be an
/// allocation an instance a frame: the phase, the ownership, the paint shown, and
/// whether a window started on this step. A window's end is never packed — it is a
/// clock reading — so the caller asks `__canvas_autoDeadline` for it, handing back the
/// one it held.
#[rustfmt::skip]
pub(super) const AUTO_STEP: &str =
r#"' The phases an instance is kept in between frames.
LET __CANVAS_AUTO_OUT AS Integer = 0
LET __CANVAS_AUTO_IN AS Integer = 1
LET __CANVAS_AUTO_ENTER AS Integer = 2
LET __CANVAS_AUTO_EXIT AS Integer = 3

' The paint an instance shows: 0 is the shape's own.
LET __CANVAS_AUTO_SHOW_ENTER AS Integer = 1
LET __CANVAS_AUTO_SHOW_EXIT AS Integer = 2
LET __CANVAS_AUTO_SHOW_HOVER AS Integer = 3
LET __CANVAS_AUTO_SHOW_PRESSED AS Integer = 4

' One instance, one frame. `state` is its phase and `deadline` the end of the window it
' is in, both as last answered; `pressOwner` is whether it owned the press; `inside` is
' whether it is the topmost item under the pointer NOW; `flags` is `useEnter` +
' 2 * `useExit` + 4 * `useHover` + 8 * `usePressed`.
'
' The answer is `phase + 4 * owns + 8 * shown + 64 * restart`. `restart` says a window
' started on this step, so its end is `now` plus the window (`__canvas_autoDeadline`).
FUNC __canvas_autoStep(state AS Integer, deadline AS Integer, pressOwner AS Boolean, inside AS Boolean, buttons AS Integer, wasButtons AS Integer, flags AS Integer, now AS Integer) AS Integer
  LET useEnter AS Boolean = flags MOD 2 = 1
  LET useExit AS Boolean = (flags / 2) MOD 2 = 1
  LET useHover AS Boolean = (flags / 4) MOD 2 = 1
  LET usePressed AS Boolean = (flags / 8) MOD 2 = 1
  MUT phase AS Integer = state
  MUT restart AS Integer = 0
  ' A window that has run out is over -- and so is one whose paint is no longer in use,
  ' which a re-`present` with the flag turned off can leave behind.
  IF phase = __CANVAS_AUTO_ENTER THEN
    IF now >= deadline OR NOT useEnter THEN
      phase = __CANVAS_AUTO_IN
    END IF
  ELSEIF phase = __CANVAS_AUTO_EXIT THEN
    IF now >= deadline OR NOT useExit THEN
      phase = __CANVAS_AUTO_OUT
    END IF
  END IF
  ' Arriving and leaving. Arriving during an exit window cancels it and starts an enter
  ' window; leaving during an enter window cancels that and starts an exit window. A
  ' flash that is not in use is skipped, with no window.
  IF inside THEN
    IF phase = __CANVAS_AUTO_OUT OR phase = __CANVAS_AUTO_EXIT THEN
      IF useEnter THEN
        phase = __CANVAS_AUTO_ENTER
        restart = 1
      ELSE
        phase = __CANVAS_AUTO_IN
      END IF
    END IF
  ELSEIF phase = __CANVAS_AUTO_IN OR phase = __CANVAS_AUTO_ENTER THEN
    IF useExit THEN
      phase = __CANVAS_AUTO_EXIT
      restart = 1
    ELSE
      phase = __CANVAS_AUTO_OUT
    END IF
  END IF
  ' The press belongs to whatever the pointer was on when the first button went down,
  ' until every button is up. A press that began elsewhere is never this instance's,
  ' however far it is dragged.
  MUT owns AS Boolean = pressOwner
  IF buttons = 0 THEN
    owns = FALSE
  ELSEIF wasButtons = 0 THEN
    owns = inside
  END IF
  ' What is shown: pressed, enter, hover, exit, the shape's own -- highest first. A
  ' phase is only ever one its flag allows, so only `hover` and `pressed` ask theirs.
  MUT shown AS Integer = 0
  IF phase = __CANVAS_AUTO_EXIT THEN
    shown = __CANVAS_AUTO_SHOW_EXIT
  ELSEIF phase = __CANVAS_AUTO_ENTER THEN
    shown = __CANVAS_AUTO_SHOW_ENTER
  ELSEIF phase = __CANVAS_AUTO_IN AND useHover THEN
    shown = __CANVAS_AUTO_SHOW_HOVER
  END IF
  IF inside AND owns AND usePressed THEN
    shown = __CANVAS_AUTO_SHOW_PRESSED
  END IF
  MUT packed AS Integer = phase + 8 * shown + 64 * restart
  IF owns THEN
    packed = packed + 4
  END IF
  RETURN packed
END FUNC

FUNC __canvas_autoPhase(packed AS Integer) AS Integer
  RETURN packed MOD 4
END FUNC

FUNC __canvas_autoOwns(packed AS Integer) AS Boolean
  RETURN (packed / 4) MOD 2 = 1
END FUNC

FUNC __canvas_autoShown(packed AS Integer) AS Integer
  RETURN (packed / 8) MOD 8
END FUNC

' The end of the window an instance is in after a step, or 0 when it is in none:
' `window` from `now` when the step started one, the end it already had when it is
' still inside one.
FUNC __canvas_autoDeadline(packed AS Integer, deadline AS Integer, now AS Integer, window AS Integer) AS Integer
  IF packed / 64 = 1 THEN
    RETURN now + window
  END IF
  LET phase AS Integer = packed MOD 4
  IF phase = __CANVAS_AUTO_ENTER OR phase = __CANVAS_AUTO_EXIT THEN
    RETURN deadline
  END IF
  RETURN 0
END FUNC"#;

/// The graphics thread's half: every `AutoPaint` the frame's walk reached, its state
/// advanced, and the draw entries of the ones showing a state paint swapped for that
/// paint's record.
///
/// **An instance is a place, not a value.** Its key is where the walk found it — the
/// index of its top-level item, and for an item inside a group the chain of group
/// nodes and child indices that led to it (`__canvas_autoPath`). No `id` is read
/// (owner's decision: an `id` is what `overID` reports, and it need not be unique), so
/// two uses of one group are two instances whatever they are called, and re-presenting
/// an item in place — moved, restyled, or unchanged — keeps its state. Inserting or
/// removing an item ahead of it in the same list moves it to a new place, and it
/// starts again from the shape's own paint.
///
/// **The table** is four parallel lists, rebuilt each frame from the instances the
/// frame saw, so a row whose instance is gone is dropped by not being copied.
///
/// **The hit test runs before any swap**, against the entries the walk appended —
/// each shape with its OWN paint — so a state paint never changes where the shape is
/// hit.
///
/// **A swap is a new record under a new hash**: the shape with the state's paint,
/// cached under the item's hash stepped by the state, and the draw hash stepped the
/// same way. That is what makes the damage diff repaint exactly the instance's old and
/// new bounds, and what makes a frame in which no instance changed paint compare equal
/// to the one before it and be skipped like any other unchanged frame.
///
/// Nothing here can raise: it runs on the graphics thread, where a `FAIL` has no
/// `present` to reach.
#[rustfmt::skip]
const RESOLVE: &str =
r#"' The instances of the frame being walked, in walk order.
MUT __CANVAS_AUTO_ITEMS AS List OF AutoPaint = []
MUT __CANVAS_AUTO_HASH AS List OF Integer = []
' Where the instance's entry is in the walk's own list (`side` in
' `__canvas_sceneOffsets`); `__canvas_autoResolve` turns it into a frame entry.
MUT __CANVAS_AUTO_SIDE AS List OF Integer = []
MUT __CANVAS_AUTO_KEY AS List OF Integer = []
' The path of the list that holds it: 0 for the scene, a group use's otherwise.
MUT __CANVAS_AUTO_PARENT AS List OF Integer = []
' Its scene index when it is a top-level item, otherwise -1.
MUT __CANVAS_AUTO_TOP AS List OF Integer = []

' The table kept across frames, one row per instance the last frame saw: its key, its
' phase (+ 4 when it owns the press), the end of the window it is in, the paint it is
' showing, and its parent path.
MUT __CANVAS_AUTO_T_KEY AS List OF Integer = []
MUT __CANVAS_AUTO_T_STATE AS List OF Integer = []
MUT __CANVAS_AUTO_T_DEADLINE AS List OF Integer = []
MUT __CANVAS_AUTO_T_SHOWN AS List OF Integer = []
MUT __CANVAS_AUTO_T_PARENT AS List OF Integer = []
' And what `__canvas_autoLight` needs to step a row without the item: its draw entry
' and its four `use...` flags, as `__canvas_autoStep` takes them.
MUT __CANVAS_AUTO_T_ENTRY AS List OF Integer = []
MUT __CANVAS_AUTO_T_FLAGS AS List OF Integer = []
' The last walked frame's draw entries as the walk left them -- every `AutoPaint` at its
' shape's OWN paint -- which is what a hit test reads, in that frame and in every wake
' after it that walks nothing.
MUT __CANVAS_AUTO_BASE AS List OF Integer = []
' The buttons held at the last frame that had an instance.
MUT __CANVAS_AUTO_WAS_BUTTONS AS Integer = 0
' Whether the last frame had an instance -- what `canvas::autoPublish` was last told.
MUT __CANVAS_AUTO_LIVE AS Boolean = FALSE
' The earliest window end among the instances, on `canvas::frameNanos`' clock; 0 = none.
MUT __CANVAS_AUTO_DEADLINE AS Integer = 0
' The pointer moved under the frame that first published "live": resolve again at once.
MUT __CANVAS_AUTO_AGAIN AS Boolean = FALSE

' plan-172-K: the shape an `AutoPaint` decorates, as the `DrawItem` it would be on its
' own. An `AutoPaint` IS its shape wherever a shape is what is asked about -- what it
' draws, where it is hit, what its `id` is, which font it names -- so every `MATCH` over
' `DrawItem` answers its `AutoPaint` arm by asking this and going round again. A
' `Shape` cannot hold an `AutoPaint`, so that never goes round twice.
'
' It copies the shape. An `AutoPaint` is for the handful of items a program interacts
' with, and none of the paths that call this run per pixel.
FUNC __canvas_autoItem(a AS AutoPaint) AS DrawItem
  MATCH a.shape
    CASE Rectangle(r)
      LET item AS DrawItem = r
      RETURN item
    CASE Line(l)
      LET item AS DrawItem = l
      RETURN item
    CASE Polygon(p)
      LET item AS DrawItem = p
      RETURN item
    CASE Circle(c)
      LET item AS DrawItem = c
      RETURN item
    CASE Arc(arc)
      LET item AS DrawItem = arc
      RETURN item
    CASE Text(t)
      LET item AS DrawItem = t
      RETURN item
    CASE Ellipse(e)
      LET item AS DrawItem = e
      RETURN item
    CASE PolyLine(pl)
      LET item AS DrawItem = pl
      RETURN item
  END MATCH
  RETURN __canvas_noItem()
END FUNC

' An `AutoPaint`'s item hash: its shape's hash, its four flags and its four paints -- so
' an item that differs only in a state paint is a different item to `present`, to the
' damage diff and to the geometry cache.
FUNC __canvas_autoHash(a AS AutoPaint) AS Integer
  MUT h AS Integer = __canvas_hashStart(11)
  h = __canvas_hashStep(h, __canvas_hashItem(__canvas_autoItem(a)))
  MUT flags AS Integer = 0
  IF a.useEnter THEN
    flags = flags + 1
  END IF
  IF a.useExit THEN
    flags = flags + 2
  END IF
  IF a.useHover THEN
    flags = flags + 4
  END IF
  IF a.usePressed THEN
    flags = flags + 8
  END IF
  h = __canvas_hashStep(h, flags)
  h = __canvas_hashPaint(h, a.enterPaint, __canvas_noTransform())
  h = __canvas_hashPaint(h, a.exitPaint, __canvas_noTransform())
  h = __canvas_hashPaint(h, a.hoverPaint, __canvas_noTransform())
  RETURN __canvas_hashPaint(h, a.pressedPaint, __canvas_noTransform())
END FUNC

' The name of a list of items: the group node at `index` of the list named `path`,
' resolved to `slot`. `slot + 2` keeps an unresolved name (-1) apart from an instance.
FUNC __canvas_autoPath(path AS Integer, index AS Integer, slot AS Integer) AS Integer
  RETURN __canvas_hashStep(__canvas_hashStep(__canvas_hashStep(1469598103, path), index), slot + 2)
END FUNC

' The key of the instance at `index` of the list named `path`.
FUNC __canvas_autoKey(path AS Integer, index AS Integer) AS Integer
  RETURN __canvas_hashStep(__canvas_hashStep(__canvas_hashStep(1469598103, path), index), 0)
END FUNC

' The geometry-cache key of an instance's shape under the paint it shows (0: its own).
' Never the item's own hash: the scene's hashes name `AutoPaint`s, and a record cached
' under one would let the native resolve take the item without the walk seeing it.
FUNC __canvas_autoGeoHash(hash AS Integer, shown AS Integer) AS Integer
  RETURN __canvas_hashStep(hash, shown + 16)
END FUNC

FUNC __canvas_autoPaintFor(a AS AutoPaint, shown AS Integer) AS Paint
  IF shown = __CANVAS_AUTO_SHOW_ENTER THEN
    RETURN a.enterPaint
  END IF
  IF shown = __CANVAS_AUTO_SHOW_EXIT THEN
    RETURN a.exitPaint
  END IF
  IF shown = __CANVAS_AUTO_SHOW_HOVER THEN
    RETURN a.hoverPaint
  END IF
  RETURN a.pressedPaint
END FUNC

' The shape of an `AutoPaint` with `p` for its paint, as a `DrawItem`. Everything else
' about the shape -- where it is, its size, its transform, its `id` -- is its own.
FUNC __canvas_autoItemWith(a AS AutoPaint, p AS Paint) AS DrawItem
  MATCH a.shape
    CASE Rectangle(r)
      LET changed AS Rectangle = WITH r { paint := p }
      LET item AS DrawItem = changed
      RETURN item
    CASE Line(l)
      LET changed AS Line = WITH l { paint := p }
      LET item AS DrawItem = changed
      RETURN item
    CASE Polygon(poly)
      LET changed AS Polygon = WITH poly { paint := p }
      LET item AS DrawItem = changed
      RETURN item
    CASE Circle(c)
      LET changed AS Circle = WITH c { paint := p }
      LET item AS DrawItem = changed
      RETURN item
    CASE Arc(arc)
      LET changed AS Arc = WITH arc { paint := p }
      LET item AS DrawItem = changed
      RETURN item
    CASE Text(t)
      LET changed AS Text = WITH t { paint := p }
      LET item AS DrawItem = changed
      RETURN item
    CASE Ellipse(e)
      LET changed AS Ellipse = WITH e { paint := p }
      LET item AS DrawItem = changed
      RETURN item
    CASE PolyLine(pl)
      LET changed AS PolyLine = WITH pl { paint := p }
      LET item AS DrawItem = changed
      RETURN item
  END MATCH
  RETURN __canvas_noItem()
END FUNC

' The record an instance draws from when it shows `shown`.
FUNC __canvas_autoGeometry(a AS AutoPaint, hash AS Integer, shown AS Integer) AS Integer
  IF shown = 0 THEN
    RETURN __canvas_geometryFor(__canvas_autoItem(a), __canvas_autoGeoHash(hash, 0))
  END IF
  RETURN __canvas_geometryFor(__canvas_autoItemWith(a, __canvas_autoPaintFor(a, shown)), __canvas_autoGeoHash(hash, shown))
END FUNC

' `__canvas_appendDraw`'s `AutoPaint` arm: note the instance, then append its shape's
' entry -- with the shape's OWN paint, which is what the hit test must read.
FUNC __canvas_autoAppend(offsets AS List OF Integer, a AS AutoPaint, hash AS Integer, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  LET key AS Integer = __canvas_autoKey(path, index)
  LET side AS Integer = len(offsets)
  MUT top AS Integer = 0 - 1
  IF depth = 0 THEN
    top = index
  END IF
  __CANVAS_AUTO_ITEMS = collections::append(__CANVAS_AUTO_ITEMS, a)
  __CANVAS_AUTO_HASH = collections::append(__CANVAS_AUTO_HASH, hash)
  __CANVAS_AUTO_SIDE = collections::append(__CANVAS_AUTO_SIDE, side)
  __CANVAS_AUTO_KEY = collections::append(__CANVAS_AUTO_KEY, key)
  __CANVAS_AUTO_PARENT = collections::append(__CANVAS_AUTO_PARENT, path)
  __CANVAS_AUTO_TOP = collections::append(__CANVAS_AUTO_TOP, top)
  RETURN __canvas_appendDraw(offsets, __canvas_autoItem(a), __canvas_autoGeoHash(hash, 0), gdx, gdy, mat, depth, path, index)
END FUNC

' The table row of `key`, or -1.
FUNC __canvas_autoRow(key AS Integer) AS Integer
  MUT i AS Integer = 0
  WHILE i < len(__CANVAS_AUTO_T_KEY)
    IF collections::getOr(__CANVAS_AUTO_T_KEY, i, 0) = key THEN
      RETURN i
    END IF
    i = i + 1
  END WHILE
  RETURN 0 - 1
END FUNC

' The topmost of the last walked frame's draw entries under a surface point, or -1: the
' test `canvas::pollMouse` makes on the worker (`__canvas_hitEntry`), over this thread's
' own entries (`__CANVAS_AUTO_BASE`). Last entry first -- later entries are drawn over
' earlier ones.
FUNC __canvas_autoHitAt(x AS Float, y AS Float) AS Integer
  MUT i AS Integer = len(__CANVAS_AUTO_BASE) - 1
  WHILE i >= 0
    LET offset AS Integer = collections::getOr(__CANVAS_AUTO_BASE, i, 0)
    LET mat AS Integer = collections::getOr(__CANVAS_DRAW_MAT, i, 0)
    LET gdx AS Float = collections::getOr(__CANVAS_DRAW_DX, i, 0.0)
    LET gdy AS Float = collections::getOr(__CANVAS_DRAW_DY, i, 0.0)
    IF __canvas_hitEntry(offset, mat, gdx, gdy, x, y) THEN
      RETURN i
    END IF
    i = i - 1
  END WHILE
  RETURN 0 - 1
END FUNC

' Is there anything for `__canvas_autoResolve` to do this frame: an instance to
' resolve, or a table to clear because the last frame had one.
FUNC __canvas_autoPending() AS Boolean
  RETURN len(__CANVAS_AUTO_KEY) > 0 OR __CANVAS_AUTO_LIVE
END FUNC

FUNC __canvas_autoForget() AS Nothing
  __CANVAS_AUTO_T_KEY = []
  __CANVAS_AUTO_T_STATE = []
  __CANVAS_AUTO_T_DEADLINE = []
  __CANVAS_AUTO_T_SHOWN = []
  __CANVAS_AUTO_T_PARENT = []
  __CANVAS_AUTO_T_ENTRY = []
  __CANVAS_AUTO_T_FLAGS = []
  __CANVAS_AUTO_BASE = []
  __CANVAS_AUTO_WAS_BUTTONS = 0
  __CANVAS_AUTO_DEADLINE = 0
END FUNC

' One frame's resolve. `offsets` is the frame's draw entries and `spans` what
' `__canvas_sceneOffsets` gave `canvas::sceneLayout` -- (scene index, first entry in the
' walk's list, entry count) for every index the walk expanded -- from which an
' instance's place in the walk's list becomes its place in `offsets`.
FUNC __canvas_autoResolve(offsets AS List OF Integer, spans AS List OF Integer) AS List OF Integer
  LET count AS Integer = len(__CANVAS_AUTO_KEY)
  IF count = 0 THEN
    ' The scene holds no `AutoPaint` any more: nothing is kept, and a pointer change
    ' stops being a redraw.
    __canvas_autoForget()
    __CANVAS_AUTO_LIVE = FALSE
    canvas::autoPublish(FALSE)
    RETURN offsets
  END IF
  __CANVAS_AUTO_BASE = offsets

  ' 1. The pointer and the clock, once for the whole frame.
  LET pointer AS List OF Integer = canvas::pointerSnapshot()
  LET buttons AS Integer = collections::getOr(pointer, 2, 0)
  LET now AS Integer = canvas::frameNanos()
  LET window AS Integer = __canvas_autoWindow()

  ' 2. The topmost entry under the pointer, whatever kind of item it is. The point is
  ' the pixel's centre, as it is for `overID`.
  MUT over AS Integer = 0 - 1
  IF collections::getOr(pointer, 3, 0) <> 0 THEN
    over = __canvas_autoHitAt(toFloat(collections::getOr(pointer, 0, 0)) + 0.5, toFloat(collections::getOr(pointer, 1, 0)) + 0.5)
  END IF

  ' 3-5. Each instance: its entry, its state, its record. The new table is built beside
  ' the old one, which is read until the loop ends.
  MUT out AS List OF Integer = offsets
  MUT keys AS List OF Integer = []
  MUT states AS List OF Integer = []
  MUT deadlines AS List OF Integer = []
  MUT showns AS List OF Integer = []
  MUT parents AS List OF Integer = []
  MUT entries AS List OF Integer = []
  MUT flagWords AS List OF Integer = []
  MUT earliest AS Integer = 0
  MUT spanAt AS Integer = 0
  MUT extra AS Integer = 0
  MUT n AS Integer = 0
  FOR EACH a IN __CANVAS_AUTO_ITEMS
    ' The instance's frame entry: its span's first entry -- the span's scene index plus
    ' one less than the count of every span before it -- and its place within the span.
    LET side AS Integer = collections::getOr(__CANVAS_AUTO_SIDE, n, 0)
    WHILE spanAt * 3 + 5 < len(spans) AND collections::getOr(spans, spanAt * 3 + 1, 0) + collections::getOr(spans, spanAt * 3 + 2, 0) <= side
      extra = extra + collections::getOr(spans, spanAt * 3 + 2, 0) - 1
      spanAt = spanAt + 1
    END WHILE
    LET entry AS Integer = collections::getOr(spans, spanAt * 3, 0) + extra + side - collections::getOr(spans, spanAt * 3 + 1, 0)

    LET key AS Integer = collections::getOr(__CANVAS_AUTO_KEY, n, 0)
    LET row AS Integer = __canvas_autoRow(key)
    MUT state AS Integer = 0
    MUT deadline AS Integer = 0
    IF row >= 0 THEN
      state = collections::getOr(__CANVAS_AUTO_T_STATE, row, 0)
      deadline = collections::getOr(__CANVAS_AUTO_T_DEADLINE, row, 0)
    END IF
    MUT flags AS Integer = 0
    IF a.useEnter THEN
      flags = flags + 1
    END IF
    IF a.useExit THEN
      flags = flags + 2
    END IF
    IF a.useHover THEN
      flags = flags + 4
    END IF
    IF a.usePressed THEN
      flags = flags + 8
    END IF
    LET packed AS Integer = __canvas_autoStep(state MOD 4, deadline, (state / 4) MOD 2 = 1, entry = over, buttons, __CANVAS_AUTO_WAS_BUTTONS, flags, now)
    deadline = __canvas_autoDeadline(packed, deadline, now, window)
    LET shown AS Integer = __canvas_autoShown(packed)
    IF deadline > 0 THEN
      IF earliest = 0 OR deadline < earliest THEN
        earliest = deadline
      END IF
    END IF
    keys = collections::append(keys, key)
    states = collections::append(states, packed MOD 8)
    deadlines = collections::append(deadlines, deadline)
    showns = collections::append(showns, shown)
    parents = collections::append(parents, collections::getOr(__CANVAS_AUTO_PARENT, n, 0))
    entries = collections::append(entries, entry)
    flagWords = collections::append(flagWords, flags)

    ' A state paint: the entry becomes the shape under that paint, and its draw hash
    ' says so. The shape's own paint is the entry the walk already appended.
    IF shown <> 0 THEN
      LET offset AS Integer = __canvas_autoGeometry(a, collections::getOr(__CANVAS_AUTO_HASH, n, 0), shown)
      out = collections::set(out, entry, offset)
      LET drawHash AS Integer = __canvas_hashStep(collections::getOr(__CANVAS_DRAW_HASHES, entry, 0), shown + 16)
      __CANVAS_DRAW_HASHES = collections::set(__CANVAS_DRAW_HASHES, entry, drawHash)
      LET top AS Integer = collections::getOr(__CANVAS_AUTO_TOP, n, 0 - 1)
      IF top >= 0 THEN
        __CANVAS_TOP_OFFSETS = collections::set(__CANVAS_TOP_OFFSETS, top, offset)
      END IF
    END IF
    n = n + 1
  NEXT
  __CANVAS_AUTO_T_KEY = keys
  __CANVAS_AUTO_T_STATE = states
  __CANVAS_AUTO_T_DEADLINE = deadlines
  __CANVAS_AUTO_T_SHOWN = showns
  __CANVAS_AUTO_T_PARENT = parents
  __CANVAS_AUTO_T_ENTRY = entries
  __CANVAS_AUTO_T_FLAGS = flagWords
  __CANVAS_AUTO_WAS_BUTTONS = buttons
  __CANVAS_AUTO_DEADLINE = earliest
  __CANVAS_AUTO_ITEMS = []
  __CANVAS_AUTO_HASH = []
  __CANVAS_AUTO_SIDE = []
  __CANVAS_AUTO_KEY = []
  __CANVAS_AUTO_PARENT = []
  __CANVAS_AUTO_TOP = []

  ' 6. From now on a pointer change is a redraw. The platforms' handlers read that word
  ' AFTER they publish, so a change that landed between this frame's snapshot and the
  ' store below told nobody: look again, and if the pointer is not where this frame
  ' saw it, resolve once more without waiting to be asked.
  IF NOT __CANVAS_AUTO_LIVE THEN
    __CANVAS_AUTO_LIVE = TRUE
    canvas::autoPublish(TRUE)
    LET after AS List OF Integer = canvas::pointerSnapshot()
    MUT i AS Integer = 0
    WHILE i < 4
      IF collections::getOr(after, i, 0) <> collections::getOr(pointer, i, 0) THEN
        __CANVAS_AUTO_AGAIN = TRUE
      END IF
      i = i + 1
    END WHILE
  END IF
  RETURN out
END FUNC

' A wake that owes nothing -- the pointer changed, or a window ended -- answered WITHOUT
' walking the scene: does any instance now show a different paint? Only then is there a
' frame to draw (`__canvas_renderFrame`).
'
' Nothing is owed, so nothing but the pointer and the clock has changed since the last
' walked frame: its draw entries (`__CANVAS_AUTO_BASE`), their group transforms and the
' geometry they point at are all still as that frame left them, and the table holds
' each instance's entry and flags. So each row is stepped exactly as
' `__canvas_autoResolve` would step it, and the table is left as that would leave it.
' When the answer is TRUE the frame is walked and resolved in full, moments later, and
' that second step of the same rows changes nothing: a window this one started is
' still running, and a press it took is still held.
'
' This is what a pointer crossing a large scene costs -- one hit test -- instead of a
' walk of the scene per pointer event.
FUNC __canvas_autoLight() AS Boolean
  LET count AS Integer = len(__CANVAS_AUTO_T_KEY)
  IF count = 0 THEN
    RETURN FALSE
  END IF
  LET pointer AS List OF Integer = canvas::pointerSnapshot()
  LET buttons AS Integer = collections::getOr(pointer, 2, 0)
  LET now AS Integer = canvas::frameNanos()
  LET window AS Integer = __canvas_autoWindow()
  MUT over AS Integer = 0 - 1
  IF collections::getOr(pointer, 3, 0) <> 0 THEN
    over = __canvas_autoHitAt(toFloat(collections::getOr(pointer, 0, 0)) + 0.5, toFloat(collections::getOr(pointer, 1, 0)) + 0.5)
  END IF
  MUT changed AS Boolean = FALSE
  MUT earliest AS Integer = 0
  MUT n AS Integer = 0
  WHILE n < count
    LET state AS Integer = collections::getOr(__CANVAS_AUTO_T_STATE, n, 0)
    LET held AS Integer = collections::getOr(__CANVAS_AUTO_T_DEADLINE, n, 0)
    LET entry AS Integer = collections::getOr(__CANVAS_AUTO_T_ENTRY, n, 0 - 2)
    LET flags AS Integer = collections::getOr(__CANVAS_AUTO_T_FLAGS, n, 0)
    LET packed AS Integer = __canvas_autoStep(state MOD 4, held, (state / 4) MOD 2 = 1, entry = over, buttons, __CANVAS_AUTO_WAS_BUTTONS, flags, now)
    LET deadline AS Integer = __canvas_autoDeadline(packed, held, now, window)
    LET shown AS Integer = __canvas_autoShown(packed)
    LET kept AS Integer = packed MOD 8
    IF shown <> collections::getOr(__CANVAS_AUTO_T_SHOWN, n, 0) THEN
      changed = TRUE
    END IF
    IF deadline > 0 THEN
      IF earliest = 0 OR deadline < earliest THEN
        earliest = deadline
      END IF
    END IF
    __CANVAS_AUTO_T_STATE = collections::set(__CANVAS_AUTO_T_STATE, n, kept)
    __CANVAS_AUTO_T_DEADLINE = collections::set(__CANVAS_AUTO_T_DEADLINE, n, deadline)
    __CANVAS_AUTO_T_SHOWN = collections::set(__CANVAS_AUTO_T_SHOWN, n, shown)
    n = n + 1
  END WHILE
  __CANVAS_AUTO_WAS_BUTTONS = buttons
  __CANVAS_AUTO_DEADLINE = earliest
  RETURN changed
END FUNC

' How long the render loop may wait for a redraw, in nanoseconds; 0 is "until one is
' owed". Not 0 only while an instance is inside an enter or exit window -- then, the
' time left of the earliest -- or, in a `--debug` build, while a scripted pointer entry
' is still to come and the scene has an instance to show it to.
FUNC __canvas_autoTimeout() AS Integer
  IF __CANVAS_AUTO_AGAIN THEN
    __CANVAS_AUTO_AGAIN = FALSE
    RETURN 1
  END IF
  IF NOT __CANVAS_AUTO_LIVE THEN
    RETURN 0
  END IF
  MUT due AS Integer = __CANVAS_AUTO_DEADLINE
  LET script AS Integer = __canvas_pointerNextDue()
  IF script > 0 THEN
    IF due = 0 OR script < due THEN
      due = script
    END IF
  END IF
  IF due = 0 THEN
    RETURN 0
  END IF
  LET left AS Integer = due - canvas::frameNanos()
  IF left < 1 THEN
    RETURN 1
  END IF
  RETURN left
END FUNC

' The memo variant of one use of a group (`__canvas_memoGroup`): 0 when none of the
' `AutoPaint`s the group holds is showing a state paint in this use, so the use shares
' the group's one run; otherwise a number that names which are showing what.
FUNC __canvas_autoVariant(path AS Integer) AS Integer
  MUT variant AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < len(__CANVAS_AUTO_T_KEY)
    LET shown AS Integer = collections::getOr(__CANVAS_AUTO_T_SHOWN, i, 0)
    IF shown <> 0 THEN
      IF collections::getOr(__CANVAS_AUTO_T_PARENT, i, 0) = path THEN
        variant = __canvas_hashStep(__canvas_hashStep(variant, collections::getOr(__CANVAS_AUTO_T_KEY, i, 0)), shown) + 1
      END IF
    END IF
    i = i + 1
  END WHILE
  RETURN variant
END FUNC

' The block an `AutoPaint` child at `index` of the group use `path` lays out: its shape
' under the paint the frame's resolve chose for that instance.
FUNC __canvas_autoBlock(a AS AutoPaint, hash AS Integer, path AS Integer, index AS Integer) AS Integer
  LET row AS Integer = __canvas_autoRow(__canvas_autoKey(path, index))
  MUT shown AS Integer = 0
  IF row >= 0 THEN
    shown = collections::getOr(__CANVAS_AUTO_T_SHOWN, row, 0)
  END IF
  RETURN __canvas_autoGeometry(a, hash, shown)
END FUNC

' The renderer's record for a `Text`, with the run's layout box written into it -- what
' the hit test answers a `Text` on (`__canvas_hitIsTextBox`). The box is the one the
' worker's `__canvas_textBoxHeader` computes: the pen's travel by the font's ascent and
' descent. A record that draws nothing is handed back as it is.
FUNC __canvas_autoTextBox(header AS List OF Float, t AS Text) AS List OF Float
  LET kind AS Integer = toInt(collections::getOr(header, 0, 0.0))
  IF kind <> __CANVAS_GEO_TEXT AND kind <> __CANVAS_GEO_POLYGON THEN
    RETURN header
  END IF
  IF toInt(collections::getOr(header, __CANVAS_GEO_GRADIENT_COUNT, 0.0)) >= 2 THEN
    RETURN header
  END IF
  LET font AS Integer = canvas::fontHandle(t.font)
  LET b AS List OF Byte = __canvas_fontBlob(font)
  IF len(b) = 0 THEN
    RETURN header
  END IF
  LET upem AS Integer = __canvas_fontUnitsPerEm(b)
  IF upem <= 0 THEN
    RETURN header
  END IF
  LET shaped AS List OF Integer = __canvas_shapeRun([font], t.text)
  LET scale AS Float = t.fontSize / toFloat(upem)
  LET x0 AS Float = __canvas_textX(t)
  LET baseline AS Float = __canvas_textY(t)
  MUT pen AS Float = x0
  MUT c AS Integer = 0
  WHILE c * 3 < len(shaped)
    pen = pen + toFloat(collections::getOr(shaped, c * 3 + 2, 0)) * scale
    c = c + 1
  END WHILE
  MUT out AS List OF Float = header
  IF kind = __CANVAS_GEO_POLYGON THEN
    out = collections::set(out, __CANVAS_GEO_GRADIENT_KIND, toFloat(__CANVAS_GEO_TEXT_BOX_MARK))
  END IF
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX, x0)
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX + 1, baseline - toFloat(__canvas_fontAscent(b)) * scale)
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX + 2, pen)
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX + 3, baseline + toFloat(0 - __canvas_fontDescent(b)) * scale)
  RETURN out
END FUNC

' The paint each instance of the last frame shows, one digit each in walk order, for a
' `--debug` build's stats line: 0 its own, 1 enter, 2 exit, 3 hover, 4 pressed.
FUNC __canvas_autoText() AS String
  IF len(__CANVAS_AUTO_T_SHOWN) = 0 THEN
    RETURN " auto=-"
  END IF
  MUT text AS String = " auto="
  FOR EACH shown IN __CANVAS_AUTO_T_SHOWN
    text = text & toString(shown)
  NEXT
  RETURN text
END FUNC"#;

/// What the unconditional source calls in a program that constructs no `AutoPaint`
/// ([`HelperGate::WhenNotConstructed`]). Such a program holds none — a record value
/// exists only where a constructor made it — so there is no instance to note, no state
/// to keep and no window to wait for, and these bodies say exactly that. The four that
/// are handed an `AutoPaint` cannot be reached; they are here because the item unions
/// have the variant in every program, and a `MATCH` over them must have its arm.
///
/// The converter and the hash are in this pair, not in the unconditional source,
/// because of what they cost there: with both compiled into every canvas program a
/// program with no `AutoPaint` carried 38,196 more bytes of code than it did before
/// the type existed, and with them here, 13,628 (`otool -l`, `__text`, macOS AArch64).
#[rustfmt::skip]
const ABSENT: &str =
r#"IMPORT canvas
IMPORT collections

FUNC __canvas_autoItem(a AS AutoPaint) AS DrawItem
  RETURN __canvas_noItem()
END FUNC

FUNC __canvas_autoHash(a AS AutoPaint) AS Integer
  RETURN 0
END FUNC

FUNC __canvas_autoPath(path AS Integer, index AS Integer, slot AS Integer) AS Integer
  RETURN 0
END FUNC

FUNC __canvas_autoAppend(offsets AS List OF Integer, a AS AutoPaint, hash AS Integer, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  RETURN __canvas_appendDraw(offsets, __canvas_autoItem(a), hash, gdx, gdy, mat, depth, path, index)
END FUNC

FUNC __canvas_autoBlock(a AS AutoPaint, hash AS Integer, path AS Integer, index AS Integer) AS Integer
  RETURN __canvas_geometryFor(__canvas_autoItem(a), hash)
END FUNC

FUNC __canvas_autoVariant(path AS Integer) AS Integer
  RETURN 0
END FUNC

FUNC __canvas_autoPending() AS Boolean
  RETURN FALSE
END FUNC

FUNC __canvas_autoResolve(offsets AS List OF Integer, spans AS List OF Integer) AS List OF Integer
  RETURN offsets
END FUNC

FUNC __canvas_autoLight() AS Boolean
  RETURN FALSE
END FUNC

FUNC __canvas_autoTimeout() AS Integer
  RETURN 0
END FUNC

FUNC __canvas_autoTextBox(header AS List OF Float, t AS Text) AS List OF Float
  RETURN header
END FUNC

FUNC __canvas_autoText() AS String
  RETURN ""
END FUNC"#;

/// The names the unconditional source calls: each is defined once in [`RESOLVE`] and
/// once in [`ABSENT`], and exactly one of the two chunks is in any canvas program.
#[cfg(test)]
const HOOKS: [&str; 12] = [
    "__canvas_autoItem",
    "__canvas_autoHash",
    "__canvas_autoPath",
    "__canvas_autoAppend",
    "__canvas_autoBlock",
    "__canvas_autoVariant",
    "__canvas_autoPending",
    "__canvas_autoResolve",
    "__canvas_autoLight",
    "__canvas_autoTimeout",
    "__canvas_autoTextBox",
    "__canvas_autoText",
];

static BODY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!("IMPORT canvas\nIMPORT collections\n\n{AUTO_STEP}\n\n{RESOLVE}")
});

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "canvas_autopaint",
        gate: HelperGate::WhenUsedOrConstructed(&[], CONSTRUCTED),
        body: Some(BODY.as_str()),
        import_name: None,
        natively_called: false,
    });
    pkg.add_helper(RegistryHelper {
        name: "canvas_autopaint_absent",
        gate: HelperGate::WhenNotConstructed(CONSTRUCTED),
        body: Some(ABSENT),
        import_name: None,
        natively_called: false,
    });
}

#[cfg(test)]
mod tests {
    use super::{ABSENT, HOOKS, RESOLVE};

    /// The signature line of `FUNC name(` in `source`.
    fn signature<'a>(source: &'a str, name: &str) -> &'a str {
        let opens = format!("FUNC {name}(");
        let at = source
            .find(&opens)
            .unwrap_or_else(|| panic!("no `{opens}` in the chunk"));
        source[at..].lines().next().expect("a signature line")
    }

    /// The two chunks are alternatives: every hook is declared in both, with the same
    /// parameters and the same return type, so the unconditional source compiles
    /// against either. A hook added to one and not the other would build for one kind
    /// of program and fail for the other.
    #[test]
    fn canvas_auto_hooks_are_declared_alike_in_both_chunks() {
        for hook in HOOKS {
            assert_eq!(signature(RESOLVE, hook), signature(ABSENT, hook), "{hook}");
        }
        let declared = ABSENT.matches("\nFUNC ").count() + usize::from(ABSENT.starts_with("FUNC "));
        assert_eq!(
            declared,
            HOOKS.len(),
            "ABSENT declares a function that is not a hook"
        );
    }

    /// The hit test reads the entries the walk appended, before any is swapped for a
    /// state paint's record — the rule that a state paint never changes where a shape
    /// is hit. In the source that is an order: the one call to `__canvas_autoHitAt`
    /// comes before the first `collections::set(out, …)`.
    #[test]
    fn canvas_auto_the_hit_test_runs_before_any_entry_is_swapped() {
        let resolve = &RESOLVE[RESOLVE.find("FUNC __canvas_autoResolve(").expect("resolve")..];
        let hit = resolve
            .find("over = __canvas_autoHitAt(")
            .expect("the hit test");
        let swap = resolve
            .find("out = collections::set(out,")
            .expect("the swap");
        assert!(hit < swap);
    }
}
