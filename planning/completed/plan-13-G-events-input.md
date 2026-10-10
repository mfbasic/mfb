# plan-13-G: events, pacing, and Input I/O

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-13-E (a live window and the event pipe). Feature-wide precondition:
plan-13 master §Prerequisites.
Produces: click/double-click/close/resize events, `app::poll`, the `Input` round-trip.
Consumed by 13-H (TextArea reuses the Input shape) and 13-I (cell activation).

Native events drained at `sync`, the `poll` wait primitive, and the `Input` value
round-trip.

The single behavioral outcome: `clicked`/`doubleClicked`/`isOpen` behave correctly under
an event-driven `poll(win)` loop with no busy-spin; `setValue` does not echo as
`valueChanged`; `submitted` latches on Enter independently; and a stalled worker never
freezes the window or loses a click.

References (read first):

- `planning/old-plans/superseded-plan-13-A-app-builtin.md` §7 and §9 — the locked pacing decisions this
  preserves: non-blocking `sync`, `poll` returns FALSE before that window's first `sync`
  and never parks.
- `src/target/macos_aarch64/app/` and `src/target/linux_gtk/` — the `O_NONBLOCK` pipe with
  bug-114 backpressure, the pattern this reuses for a second pipe.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-13-E has landed (live window + event pipe) | `rg -n 'host_wait_events' src/` | **NOT MET** |
| A macOS machine is available for on-device proof | build host | **MET** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> before continuing and again before deciding to stop; report every row if you stop.

## 1. Goal

- Single/double click records: the button's target/action (macOS) and `GtkGestureClick`
  (GTK) classify each press by native `clickCount`/`n_press` and push a `Click` or
  `DoubleClick` record; `sync` folds them into per-node shadow counters.
- Optional `ClickMode.Exclusive`: defer the single via a main-thread timer of the
  **system** double-click interval — read from the toolkit, never hardcoded. The default
  path stays timer-free.
- Window-close record → `app::isOpen`; resize record → the window-size shadow;
  frame-report records → the `app::frame` mirror.
- `app::poll` / `host_wait_events` as `poll(2)`/`ppoll(2)` on the event pipe's read fd;
  returns FALSE **immediately** before that window's first `sync` and never parks;
  `ErrInvalidArgument` on a negative timeout.
- `Input` I/O: `host_input_drain` pulls text + user-edited + Enter-submit latches into the
  shadow at `sync`, with **program-set-wins-this-frame** precedence on a dirty `value`.

### Non-goals (explicit constraints)

- **No callbacks.** There is no native→MFBASIC callback mechanism anywhere in the tree
  (master §2.4); events are polled by design, not by preference.
- **No busy-spin.** `poll` is the wait primitive; a loop that spins is a bug, not a
  performance note.
- **No hardcoded double-click interval.** Read it from the toolkit.
- **Do not block the UI on a slow worker.** The write end is `O_NONBLOCK`.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| Event record kinds | **5** (Click, DoubleClick, Close, Resize, FrameReport) | §1 |
| Native→MFBASIC callback mechanisms available | **0** | master §2.4 |
| Existing pipe-with-backpressure precedents | **2** (macOS, GTK; bug-114) | `rg -n 'O_NONBLOCK\|non-blocking' src/target/*/app*/bootstrap.rs src/target/linux_gtk/bootstrap.rs` |

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| Events must be polled, not delivered | **CONFIRMED** | zero native→MFBASIC callback mechanisms exist; the audio callbacks are producer/consumer shims where MFBASIC polls down (master §2.4) |
| The `O_NONBLOCK` + coalescing-fallback pattern is proven | **CONFIRMED** | bug-114, both platforms |
| The system double-click interval is a toolkit value | **CONFIRMED** | `NSEvent.doubleClickInterval` / `gtk-double-click-time`. The draft correctly says do not hardcode |
| `poll` must return FALSE before the first `sync` | **LOCKED DESIGN** | plan-13-C §9 — prevents parking on a window that has never presented |
| A stalled worker cannot freeze the UI or lose clicks | **UNVERIFIED — an acceptance criterion** | proven by filling the pipe, not by reasoning |

## 3. Design Overview

Events flow one way: native callback → fixed-size record → pipe → `sync` drains → per-node
shadow counters → MFBASIC reads via `clicked`/`doubleClicked`/`getValue`.

**Where design uncertainty concentrates: double-click classification.** The draft's own
acceptance names the trap — `doubleClicked` must be correct "where 'two clicks per frame'
never happens", i.e. when the two clicks land in *different* `sync` frames. Classifying by
native `clickCount`/`n_press` at the source rather than by counting singles at the drain is
what makes that work, and it is the one place where a plausible-looking implementation
(count clicks per frame) is wrong in a way that only shows up under slow frames.

**Where correctness risk concentrates:** backpressure. If the write end blocks, a worker
that stops calling `sync` freezes the **UI**, because the native callback is on the main
thread. `O_NONBLOCK` plus a main-thread coalescing fallback on `EAGAIN` is the fix, and its
proof is filling the pipe deliberately — not reasoning about capacity.

**Rejected alternative:** *deliver events by calling an MFBASIC handler.* Impossible today
(master §2.4) and rejected on design grounds regardless: a retained tree with polled events
has no re-entrancy hazard, and this codebase has no mechanism to call up.

**Rejected alternative:** *count singles at the drain to synthesize double-clicks.*
Rejected — it breaks whenever two clicks straddle a frame boundary, which is exactly when
the worker is slow.

**Rejected alternative:** *make `poll` park before the first `sync`.* Rejected and locked:
it would wait for events on a window that has never presented.

## Compatibility / Format Impact

- **New:** 5 event record kinds; `app::poll`; the `Input` drain protocol.
- **Unchanged:** `sync`'s non-blocking contract; the transcript path; the solver.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — records, drain, and `poll`

- [ ] Click/DoubleClick records classified at the source by native `clickCount`/`n_press`.
- [ ] Close → `isOpen`; Resize → window-size shadow; FrameReport → the `app::frame` mirror.
- [ ] `app::poll` / `host_wait_events` over `poll(2)`/`ppoll(2)`; FALSE immediately before
      the first `sync`; `ErrInvalidArgument` on a negative timeout.
- [ ] Tests: `tests/syntax/app/` for the event functions' arity and types.

Acceptance: on-device, an event-driven `poll(win)` loop reacts to clicks with **no
busy-spin**, and `doubleClicked` is correct when the two clicks land in different frames.
Commit: —

### Phase 2 — `Input` round-trip and `ClickMode.Exclusive`

- [ ] `host_input_drain` → text + user-edited + Enter-submit latches at `sync`, with
      program-set-wins-this-frame precedence on a dirty `value`.
- [ ] `ClickMode.Exclusive` via a main-thread timer of the **toolkit-reported** interval;
      the default path stays timer-free.

Acceptance: `setValue` does **not** echo as `valueChanged`; `submitted` latches on Enter
independently of `valueChanged`; `Exclusive` drops the stray single.
Commit: —

### Phase 3 — backpressure (largest blast radius last)

The failure here freezes the user's window, so it is proven by force, not by argument.

- [ ] Fill the pipe from a program that **stops calling `sync`**.
- [ ] Confirm the UI stays responsive and that **no click is lost** once `sync` resumes.

Acceptance: with the pipe full and the worker stalled, the window still drags, resizes and
closes; when `sync` resumes, every click that occurred is accounted for. A test that never
fills the pipe does not satisfy this.
Commit: —

## Validation Plan

- Tests: syntax fixtures for the surface; the behavioral proofs are on-device.
- Coverage check: `tests/syntax/app/` is golden-backed. The event behavior is not
  golden-testable — it is proven on-device, and that is stated rather than papered over.
- Runtime proof: macOS on-device for all three phases; the click/Input behavior is
  re-proven on GTK when 13-F lands.
- Doc sync: the event and `poll` surface in `src/docs/spec/stdlib/` + man pages.
- Acceptance: the project's full suite.

## Open Decisions

1. **Whether `ClickMode.Exclusive` ships in v1.** Recommended yes but off by default — the
   timer path is the only place a UI-thread timer exists, and defaulting to it would give
   every program latency it did not ask for.
2. **Whether resize events coalesce.** Recommended yes, at the main-thread fallback: a drag
   produces hundreds and only the last matters to the shadow.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The no-callbacks design is forced, not chosen.** There are zero
  native→MFBASIC callback mechanisms in the tree (master §2.4). Recorded here so a future
  reader does not "improve" the polled model by inventing a mechanism that does not exist
  at any layer.
- 2026-07-20 — The system double-click interval stays a toolkit read. The draft's
  illustrative "~250–500 ms" and "~75 ms is far too short" are OS constants, not codebase
  facts, and are not restated as numbers here.

## Summary

The engineering risk is backpressure, and its failure mode is the worst kind: the native
callback runs on the main thread, so a blocking write freezes the *user's window* because
of something the *worker* did. `O_NONBLOCK` plus main-thread coalescing is the fix and
Phase 3 proves it by deliberately filling the pipe.

The subtle risk is double-click classification, where the naive implementation — count
singles per frame — is correct exactly until the worker gets slow, which is when it
matters.

What is left untouched: `sync`'s non-blocking contract, the solver, the transcript path,
and the polled-event model, which is a consequence of the seam rather than a preference.
