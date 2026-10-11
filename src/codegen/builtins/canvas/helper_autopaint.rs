//! `canvas::AutoPaint`'s state (plan-172-K): which of its paints a shape shows, from
//! where the pointer is and which buttons are held.
//!
//! [`AUTO_STEP`] is the whole rule as one pure function — no globals, no clock of its
//! own, no drawing — so every rule in `planning/new_canvas.md` "AutoPaint states" is a
//! row of inputs and an expected output (`tests/canvas/rt_canvas_auto_step.rs`, which
//! compiles this text and runs it).
//!
//! The chunk is gated on the program constructing an `AutoPaint`
//! ([`HelperGate::WhenUsedOrConstructed`]): a canvas program with none compiles none of
//! it.

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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "canvas_autopaint",
        gate: HelperGate::WhenUsedOrConstructed(&[], CONSTRUCTED),
        body: Some(AUTO_STEP),
        import_name: None,
        natively_called: false,
    });
}
