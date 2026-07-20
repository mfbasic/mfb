# plan-13-M: the macOS/AppKit backend

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-13-S (the solver and the shadow tree). Feature-wide precondition:
plan-13 master §Prerequisites.
Produces: the host-protocol seam contract, the AppKit implementation of its 26 ops, mode
selection (GUI vs transcript), and the event pipe. Consumed by 13-E, 13-D, 13-B, 13-C.

Brings up a real macOS window through the seam and the 13-S solver.

The single behavioral outcome: a `window` + `Column`/`Row` + `Button`/`Label` program lays
out to **the same frames the headless host produced for the same tree** (asserted via
`app::frame`), stays live under `[NSApp run]`, and re-flows on drag-resize **without the
worker running** — while no transcript window appears and `io::print` goes to console
stdio.

References (read first):

- `src/target/macos_aarch64/app/bootstrap.rs` — the runtime Obj-C class recipe
  (`objc_allocateClassPair` + `class_addMethod` + `objc_registerClassPair`), proven twice
  for `MFBTextView` and `TermView`. Find with `rg -n 'objc_allocateClassPair'`.
- `src/target/macos_aarch64/app/mod.rs` — `performSelectorOnMainThread:…waitUntilDone:`
  and the `object_getIndexedIvars` per-instance state path. Find by symbol.
- `src/target/shared/code/mod.rs` — the `uses_term` whole-program runtime-symbol scan that
  mode selection mirrors. Find with `rg -n 'let uses_term'`; note it also drives
  `term_state_offset`/`term_state_slots`, a coupling the 2026-07-09 draft did not account
  for.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-13-S has landed (solver + headless frames to match) | `rg -n '_mfb_rt_app_layout' src/` | **NOT MET** |
| A macOS machine is available for on-device proof | build host | **MET** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> before continuing and again before deciding to stop; report every row if you stop.
> Locate every symbol with `rg`, not by line — master §2.3.

## 1. Goal

- The **host-protocol seam** defined once, as a contract, against the `Asm`/`abi` builder:
  26 ops (§2.1) that 13-G and the headless host implement identically.
- The AppKit backend: `NSWindow` + plain `NSView` containers + `NSButton` + `NSTextField`
  (non-editable = `Label`, editable = `Input`).
- **Mode selection**: static whole-program detection of an `app::window` call picks GUI vs
  transcript sub-mode at build time; in GUI sub-mode the transcript window is skipped and
  `io::`/`term::` keep console lowering.
- The **event pipe**: its own fd pair, **never `dup2`'d onto fd 0**, `O_NONBLOCK` write
  end, fixed-size records, main-thread coalescing fallback on `EAGAIN`.
- `app::sync` is **non-blocking**: drain the pipe, post an owned command batch with
  `performSelectorOnMainThread:…waitUntilDone:NO`, main thread frees it.
- `host_present` calls `_mfb_rt_app_layout` with `fittingSize`/`intrinsicContentSize` as
  the measure fn-ptr, sets every node's frame, and returns immediately.
- Native resize re-invokes the solver **autonomously on the main thread** — layout
  ownership is native; no worker `sync` is involved.

### Non-goals (explicit constraints)

- **No events, no Input round-trip.** 13-E. This sub-plan proves layout and liveness only.
- **No GTK.** 13-G implements the same seam.
- **Do not change the transcript path** for programs that do not call `app::window`.
- **Do not block in `sync`.** `waitUntilDone:YES` would inherit today's blocking
  round-trip and stall the worker on every frame.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| Host-seam ops this sub-plan defines and implements | **26** | counted from plan-13-A §8 |
| — plus 13-B's TextArea ops | +5 | plan-13-B §4.3 |
| — plus 13-C's Table ops | +8 | plan-13-C §5.3 |
| **Family total × 3 backends** | **39 × 3 = 117 implementations** | master §2.1 |
| Existing macOS app-mode code to extend | **4372 LOC** | `wc -l src/target/macos_aarch64/app/*.rs` |
| Runtime Obj-C classes already created from codegen | **2** (`MFBTextView`, `TermView`) | `rg -c 'objc_allocateClassPair' src/target/macos_aarch64/app/bootstrap.rs` |

**"Keep the seam small and stable" is not a measurement.** 26 ops here become 39
family-wide and 117 implementations across three backends. That is the feature's real cost
driver and no 2026-07-09 document totals it.

### 2.2 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| Runtime Obj-C class creation from codegen is proven | **CONFIRMED** | done twice already (`MFBTextView` `keyDown:`, `TermView` with 5 `class_addMethod` calls) |
| Per-instance state via indexed ivars is proven | **CONFIRMED** | `object_getIndexedIvars` path in `app/mod.rs` |
| `waitUntilDone:NO` is required for a non-blocking `sync` | **CONFIRMED** | the existing transcript append uses the same selector; `YES` would make `sync` a blocking round-trip |
| The existing input pipe is `dup2`'d onto fd 0 | **CONFIRMED** | so the event pipe **must** be a separate fd pair — the draft flags this correctly |
| The `O_NONBLOCK` write end + backpressure pattern is proven | **CONFIRMED** | bug-114 established it on both platforms |
| There is no native→MFBASIC callback mechanism | **CONFIRMED** | master §2.4 — which is why events are polled, not delivered |
| Layout can be driven from the native resize handler | **UNVERIFIED — this is the acceptance criterion** | the whole native-owned-layout claim rests on it |

## 3. Design Overview

**Where design uncertainty concentrates: native-owned layout.** Every other piece has
prior art in the existing 4372 LOC. But "the resize handler re-invokes the emitted solver
on the main thread with no worker involvement" has none — it requires the emitted helper to
be callable from a context the worker does not control, with a measure fn-ptr into AppKit.
**Phase 2 is a spike on exactly that**: resize a window with the worker deliberately parked
and confirm the layout still re-flows.

**Where correctness risk concentrates:** two places.

1. **Mode selection coupling.** The `uses_term` scan this mirrors also drives
   `term_state_offset`/`term_state_slots`. Adding a second whole-program flag next to it
   without accounting for that coupling is how a GUI program ends up allocating terminal
   state, or a `term::` program ends up without it.
2. **The event pipe's fd.** If it is `dup2`'d onto fd 0 like the transcript's, GUI-mode
   stdin breaks — and in GUI sub-mode fd 0 is the **real** stdin, which is the point.

**Rejected alternative:** *let the worker own layout and push frames at `sync`.* Rejected:
a drag-resize would then be as slow as the worker's loop, and a parked worker would freeze
the window's layout entirely. Native-owned layout is why resize works without the worker.

**Rejected alternative:** *`waitUntilDone:YES` for simpler ordering.* Rejected — it makes
every `sync` a blocking main-thread round-trip.

**Rejected alternative:** *reuse the transcript's pipe for events.* Rejected: it is
`dup2`'d onto fd 0.

## Compatibility / Format Impact

- **New:** the seam contract; the AppKit widget backend; a second fd pair; a GUI sub-mode.
- **Unchanged:** transcript behavior for programs that do not call `app::window`; console
  `io::`/`term::` lowering in GUI sub-mode; the existing input pipe.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — the seam contract and mode selection

- [ ] Define the 26-op seam once, against the `Asm`/`abi` builder, as the contract 13-G and
      the headless host implement. Write it down as a contract, not as macOS's shape.
- [ ] Static whole-program `app::window` detection selecting GUI vs transcript sub-mode.
      **Account for the `uses_term` coupling** (§3) — do not add a flag beside it blindly.
- [ ] Stand up the event pipe: separate fd pair, `O_NONBLOCK` write end, fixed-size
      records, main-thread coalescing fallback on `EAGAIN`.

Acceptance: a GUI-sub-mode program builds, opens no transcript, and `io::print` reaches
console stdio; a non-`app::` program's emitted bytes are unchanged
(`scripts/artifact-gate.sh` 0 diffs).
Commit: —

### Phase 2 — spike: native-owned layout under resize (the unproven premise)

- [ ] `host_present` → `_mfb_rt_app_layout` with `fittingSize`/`intrinsicContentSize` as
      the measure fn-ptr; `setFrame` per node; return immediately.
- [ ] Native resize handler re-invokes the solver autonomously on the main thread.
- [ ] **Park the worker deliberately** and drag-resize the window.

Acceptance: the window re-flows correctly on drag-resize **while the worker is parked**.
If it cannot, native-owned layout does not work and §3 must be redesigned before the widget
set is built.
Commit: —

### Phase 3 — the widget set and `sync`

- [ ] `NSWindow` + `NSView` containers + `NSButton` + `NSTextField` (non-editable =
      `Label`, editable = `Input`), using the proven runtime-class recipe with the node id
      in indexed ivars.
- [ ] `app::sync`: non-blocking pipe drain + `performSelectorOnMainThread:…waitUntilDone:NO`
      of an owned command batch, freed by the main thread.
- [ ] Runtime: the canonical `window` + `Column`/`Row` + `Button`/`Label` program.

Acceptance: on-device, the program's frames — read back via `app::frame` — **match the
headless host's frames for the same tree**, and the window stays live under `[NSApp run]`.
Commit: —

## Validation Plan

- Tests: frame equality against 13-S's headless goldens is the test. A screenshot is not.
- Coverage check: mode selection touches shared lowering, so `scripts/artifact-gate.sh`
  must show 0 diffs for non-`app::` programs on every existing target.
- Runtime proof: on-device macOS. The load-bearing one is **Phase 2 with the worker
  parked** — it is the only proof that layout is genuinely native-owned.
- Doc sync: none here; the `app::` surface docs land with 13-A/13-Z.
- Acceptance: the project's full suite.

## Open Decisions

1. **Whether the seam is a Rust trait or a table of emitters.** Recommended a trait with
   three impls (macOS, GTK, headless) so the compiler enforces that all 26 ops exist
   everywhere — 117 implementations is too many to police by convention.
2. **Whether `Label` and `Input` share `NSTextField`.** Recommended yes (editable flag), as
   the draft says — two classes would double the runtime-class work for no behavior gain.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The seam is 26 ops here, 39 family-wide, 117 implementations across three
  backends.** plan-13-A §8 calls it "small and stable" and never totals it. It is the
  feature's largest cost driver after the solver.
- 2026-07-20 — **Mode selection's citation and mechanism were both wrong in the draft.**
  It pointed at a `module_uses_call` scan while describing a runtime-symbol scan. The
  actual `uses_term` scan also drives `term_state_offset`/`term_state_slots` — a coupling
  the draft did not account for.
- 2026-07-20 — **The seam's proposed location (`src/target/<platform>/widgets`) does not
  match the shipped layout.** Existing app code lives in `macos_aarch64/app/` and the
  *shared* `linux_gtk/`. Decide the location against what exists, not against a directory
  that has never existed.

## Summary

The engineering risk is one premise with no prior art: that the emitted solver can be
re-invoked from AppKit's resize handler with the worker uninvolved. Everything else in this
sub-plan has been done before in the existing 4372 LOC — runtime Obj-C classes, indexed
ivars, main-thread marshalling, non-blocking pipes. Phase 2 parks the worker and drags the
window, and that is the whole proof.

The cost is not in the risk, though — it is in the 26 seam ops, which become 117
implementations family-wide. That number belongs in the effort estimate and was never
written down.

What is left untouched: the transcript path for non-`app::` programs, console I/O lowering
in GUI sub-mode, and the existing input pipe.
