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
/// neither starts a window, so neither asks for a frame when it would have ended.
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
' plan-172-L: which `Picture` it is the shape of -- the number `helper_picture.rs` gave
' that picture this frame -- or -1 when it is an item of its own.
MUT __CANVAS_AUTO_HOSTS AS List OF Integer = []

' The table kept across frames, one row per instance the last frame saw: its key, its
' phase (+ 4 when it owns the press), the end of the window it is in, the paint it is
' showing, and its parent path.
MUT __CANVAS_AUTO_T_KEY AS List OF Integer = []
' The table's rows by key, and each parent path's rows in table order, rebuilt
' wherever `__CANVAS_AUTO_T_KEY` is (`__canvas_autoIndexTable`). A scan of the table per
' instance (`__canvas_autoRow`) and per group use (`__canvas_autoVariant`) made a frame of
' n instances cost n * n.
MUT __CANVAS_AUTO_T_ROW AS Map OF Integer TO Integer = Map OF Integer TO Integer { }
MUT __CANVAS_AUTO_T_FIRST AS Map OF Integer TO Integer = Map OF Integer TO Integer { }
MUT __CANVAS_AUTO_T_NEXT AS List OF Integer = []
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
    CASE PolyCurve(pc)
      LET item AS DrawItem = pc
      RETURN item
    CASE CurvePolygon(cp)
      LET item AS DrawItem = cp
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
    CASE PolyCurve(pc)
      LET changed AS PolyCurve = WITH pc { paint := p }
      LET item AS DrawItem = changed
      RETURN item
    CASE CurvePolygon(cp)
      LET changed AS CurvePolygon = WITH cp { paint := p }
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

' Note an instance at `index` of the list named `path`: `side` is the entry in the
' walk's list the pointer is tested against, `hash` the hash its records are keyed
' from, and `host` the `Picture` it is the shape of (plan-172-L) or -1.
FUNC __canvas_autoNote(a AS AutoPaint, hash AS Integer, side AS Integer, depth AS Integer, path AS Integer, index AS Integer, host AS Integer) AS Nothing
  MUT top AS Integer = 0 - 1
  IF depth = 0 THEN
    top = index
  END IF
  __CANVAS_AUTO_ITEMS = collections::append(__CANVAS_AUTO_ITEMS, a)
  __CANVAS_AUTO_HASH = collections::append(__CANVAS_AUTO_HASH, hash)
  __CANVAS_AUTO_SIDE = collections::append(__CANVAS_AUTO_SIDE, side)
  __CANVAS_AUTO_KEY = collections::append(__CANVAS_AUTO_KEY, __canvas_autoKey(path, index))
  __CANVAS_AUTO_PARENT = collections::append(__CANVAS_AUTO_PARENT, path)
  __CANVAS_AUTO_TOP = collections::append(__CANVAS_AUTO_TOP, top)
  __CANVAS_AUTO_HOSTS = collections::append(__CANVAS_AUTO_HOSTS, host)
END FUNC

' `__canvas_appendDraw`'s `AutoPaint` arm: note the instance, then append its shape's
' entry -- with the shape's OWN paint, which is what the hit test must read.
'
' Through a local, not the parameter: handing `offsets` straight on to
' `__canvas_appendDraw` copied the walk's whole entry list for every instance (and freed
' and scrubbed the copy), so a frame of n instances cost n * n
' (`an_autopaint_frame_costs_linear_in_its_instances`).
FUNC __canvas_autoAppend(offsets AS List OF Integer, a AS AutoPaint, hash AS Integer, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  MUT out AS List OF Integer = offsets
  __canvas_autoNote(a, hash, len(out), depth, path, index, 0 - 1)
  out = __canvas_appendDraw(out, __canvas_autoItem(a), __canvas_autoGeoHash(hash, 0), gdx, gdy, mat, depth, path, index)
  RETURN out
END FUNC

' plan-172-L: the shape of an `AutoPaint` under the paint it shows (0: its own), for a
' `Picture` that has it for a shape -- and the paint the instance at `index` of the list
' named `path` was last resolved to show.
FUNC __canvas_autoShapeShown(a AS AutoPaint, shown AS Integer) AS DrawItem
  IF shown = 0 THEN
    RETURN __canvas_autoItem(a)
  END IF
  RETURN __canvas_autoItemWith(a, __canvas_autoPaintFor(a, shown))
END FUNC

FUNC __canvas_autoShownAt(path AS Integer, index AS Integer) AS Integer
  LET row AS Integer = __canvas_autoRow(__canvas_autoKey(path, index))
  IF row < 0 THEN
    RETURN 0
  END IF
  RETURN collections::getOr(__CANVAS_AUTO_T_SHOWN, row, 0)
END FUNC

' The table row of `key`, or -1.
FUNC __canvas_autoRow(key AS Integer) AS Integer
  RETURN collections::getOr(__CANVAS_AUTO_T_ROW, key, 0 - 1)
END FUNC

' Index the table just written: each key's FIRST row (what the scan it replaced
' answered), and each parent path's rows as a chain in rising order -- the parent's
' first row, then each row's next row of the same parent (-1 ends it). A chain, not a
' list per parent: every top-level instance has parent 0, and appending to a list held
' in a map copies it, which is the n * n this index exists to remove.
FUNC __canvas_autoIndexTable() AS Nothing
  LET count AS Integer = len(__CANVAS_AUTO_T_KEY)
  __CANVAS_AUTO_T_ROW = Map OF Integer TO Integer { }
  __CANVAS_AUTO_T_FIRST = Map OF Integer TO Integer { }
  __CANVAS_AUTO_T_NEXT = collections::repeat([], 0 - 1, count)
  MUT last AS Map OF Integer TO Integer = Map OF Integer TO Integer { }
  MUT i AS Integer = 0
  WHILE i < count
    LET key AS Integer = collections::getOr(__CANVAS_AUTO_T_KEY, i, 0)
    IF NOT collections::hasKey(__CANVAS_AUTO_T_ROW, key) THEN
      __CANVAS_AUTO_T_ROW = collections::set(__CANVAS_AUTO_T_ROW, key, i)
    END IF
    LET parent AS Integer = collections::getOr(__CANVAS_AUTO_T_PARENT, i, 0)
    LET previous AS Integer = collections::getOr(last, parent, 0 - 1)
    IF previous < 0 THEN
      __CANVAS_AUTO_T_FIRST = collections::set(__CANVAS_AUTO_T_FIRST, parent, i)
    ELSE
      __CANVAS_AUTO_T_NEXT = collections::set(__CANVAS_AUTO_T_NEXT, previous, i)
    END IF
    last = collections::set(last, parent, i)
    i = i + 1
  END WHILE
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
  __CANVAS_AUTO_T_ROW = Map OF Integer TO Integer { }
  __CANVAS_AUTO_T_FIRST = Map OF Integer TO Integer { }
  __CANVAS_AUTO_T_NEXT = []
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
    '
    ' plan-172-L: an instance that is a picture's shape is TWO entries -- the image, then
    ' the outline the pointer was tested against -- and both become the state paint's:
    ' its fill is the image's tint, its stroke the outline's. The picture's top-level
    ' slot stays as the walk left it: not one entry, so the draw walk asks again.
    IF shown <> 0 THEN
      LET hosted AS List OF Integer = __canvas_pictureAutoBlocks(collections::getOr(__CANVAS_AUTO_HOSTS, n, 0 - 1), a, collections::getOr(__CANVAS_AUTO_HASH, n, 0), shown)
      IF len(hosted) >= 4 THEN
        out = collections::set(out, entry - 1, collections::getOr(hosted, 0, 0))
        out = collections::set(out, entry, collections::getOr(hosted, 2, 0))
        LET imageHash AS Integer = __canvas_hashStep(collections::getOr(__CANVAS_DRAW_HASHES, entry - 1, 0), shown + 16)
        __CANVAS_DRAW_HASHES = collections::set(__CANVAS_DRAW_HASHES, entry - 1, imageHash)
        LET outlineHash AS Integer = __canvas_hashStep(collections::getOr(__CANVAS_DRAW_HASHES, entry, 0), shown + 16)
        __CANVAS_DRAW_HASHES = collections::set(__CANVAS_DRAW_HASHES, entry, outlineHash)
      ELSE
        LET offset AS Integer = __canvas_autoGeometry(a, collections::getOr(__CANVAS_AUTO_HASH, n, 0), shown)
        out = collections::set(out, entry, offset)
        LET drawHash AS Integer = __canvas_hashStep(collections::getOr(__CANVAS_DRAW_HASHES, entry, 0), shown + 16)
        __CANVAS_DRAW_HASHES = collections::set(__CANVAS_DRAW_HASHES, entry, drawHash)
        LET top AS Integer = collections::getOr(__CANVAS_AUTO_TOP, n, 0 - 1)
        IF top >= 0 THEN
          __CANVAS_TOP_OFFSETS = collections::set(__CANVAS_TOP_OFFSETS, top, offset)
        END IF
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
  __canvas_autoIndexTable()
  __CANVAS_AUTO_WAS_BUTTONS = buttons
  __CANVAS_AUTO_DEADLINE = earliest
  __CANVAS_AUTO_ITEMS = []
  __CANVAS_AUTO_HASH = []
  __CANVAS_AUTO_SIDE = []
  __CANVAS_AUTO_KEY = []
  __CANVAS_AUTO_PARENT = []
  __CANVAS_AUTO_TOP = []
  __CANVAS_AUTO_HOSTS = []

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
  MUT i AS Integer = collections::getOr(__CANVAS_AUTO_T_FIRST, path, 0 - 1)
  WHILE i >= 0
    LET shown AS Integer = collections::getOr(__CANVAS_AUTO_T_SHOWN, i, 0)
    IF shown <> 0 THEN
      variant = __canvas_hashStep(__canvas_hashStep(variant, collections::getOr(__CANVAS_AUTO_T_KEY, i, 0)), shown) + 1
    END IF
    i = collections::getOr(__CANVAS_AUTO_T_NEXT, i, 0 - 1)
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
  LET handles AS List OF Integer = __canvas_textHandles(t)
  LET scales AS List OF Float = __canvas_fontScales(handles, t.fontSize)
  LET shaped AS List OF Integer = __canvas_shapeRun(handles, t.text, t.letterSpacing)
  ' The line the run sits on; for an empty string, the first drawable font's. No such
  ' font -- every font closed, or none with a scale -- and there is no box.
  LET line AS List OF Float = __canvas_runLine(handles, scales, shaped)
  IF collections::getOr(line, 2, 0.0) = 0.0 THEN
    RETURN header
  END IF
  LET x0 AS Float = __canvas_textX(t)
  LET baseline AS Float = __canvas_textY(t)
  LET pen AS Float = __canvas_runPen(shaped, scales, x0, t.letterSpacing)
  MUT out AS List OF Float = header
  IF kind = __CANVAS_GEO_POLYGON THEN
    out = collections::set(out, __CANVAS_GEO_GRADIENT_KIND, toFloat(__CANVAS_GEO_TEXT_BOX_MARK))
  END IF
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX, x0)
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX + 1, baseline - collections::getOr(line, 0, 0.0))
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX + 2, pen)
  out = collections::set(out, __CANVAS_GEO_TEXT_BOX + 3, baseline + collections::getOr(line, 1, 0.0))
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
END FUNC

FUNC __canvas_autoNote(a AS AutoPaint, hash AS Integer, side AS Integer, depth AS Integer, path AS Integer, index AS Integer, host AS Integer) AS Nothing
END FUNC

FUNC __canvas_autoShapeShown(a AS AutoPaint, shown AS Integer) AS DrawItem
  RETURN __canvas_noItem()
END FUNC

FUNC __canvas_autoShownAt(path AS Integer, index AS Integer) AS Integer
  RETURN 0
END FUNC"#;

/// The names the unconditional source calls: each is defined once in [`RESOLVE`] and
/// once in [`ABSENT`], and exactly one of the two chunks is in any canvas program.
#[cfg(test)]
const HOOKS: [&str; 15] = [
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
    "__canvas_autoNote",
    "__canvas_autoShapeShown",
    "__canvas_autoShownAt",
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
mod tests;
