# plan-62-C: macOS `None`-mode bootstrap + the `setMode` surface transition

Last updated: 2026-07-24
Effort (Human): medium (1h–2h)
Effort (AI): medium (1h–2h)
<!-- Converges: dominated by cross-thread window code that needs iterate-and-verify turns plus
     on-device macOS proof. The AI writes it faster but does not think past the proof loop, so
     the two land at the same band. -->

Depends on: plan-62-B. Feature-wide precondition: plan-62-A §Prerequisites.
Produces: an AppKit `--app` binary that (a) starts **windowless** when the initial mode is
`None`, keeping `[NSApp run]` alive; and (b) reconciles the window surface at runtime when
`app::setMode` switches between `Console` and `None`, including an implicit `term::off()`.
Fans out from B alongside plan-62-D (GTK); C and D share no code.

**The single behavioral outcome:** on macOS, `mfb build --app` of a program whose first act is
`app::setMode(app::Mode::None)` presents **no window** and keeps running; a program that then
does `app::setMode(app::Mode::Console)` brings up the transcript window live; switching back to
`None` tears it down. `io::print` follows automatically — it hits the transcript while a view
is attached (Console) and falls through to the fd sink while none is (None), because that
branch already exists (`app_io.rs:111-116`).

References (read first):

- `src/target/macos_aarch64/app/bootstrap.rs` — `emit_main_bootstrap` (`:6`), the
  unconditional `NSWindow` (`:32-50`), the `MFB_MACAPP_HEADLESS` transcript skip (`:72-73` →
  `after_show` `:486`), the worker/run-loop split (`:495-528`), `[NSApp run]` (`:523`),
  `applicationShouldTerminateAfterLastWindowClosed:` YES (`:307-312`).
- `src/target/macos_aarch64/app/app_io.rs` — the io-write three-way branch (TUI grid `:47-55`;
  transcript append `:60-109`; **nil-view fd fallback `:111-116`**), `emit_app_term_helper`
  (`:588`), `emit_app_term_on_helper` (`:331`).
- plan-62-B — `AppEntrySpec.initial_mode`, `PRESENTATION_MODE_OFFSET`, and the **no-op
  `emit_app_mode_reconcile` seam** this letter fills.

## Prerequisites

See plan-62-A §Prerequisites. Additionally:

| Must be true | Command | Status 2026-07-24 |
|---|---|---|
| plan-62-B landed (state slot + helpers + `AppEntrySpec.initial_mode` + reconcile seam) | `rg -n 'initial_mode' src/target/shared/code/types.rs` | **NOT MET (B pending)** |
| A macOS aarch64 device is reachable for on-device proof | host is macOS (`darwin`) | **MET** |

> **NOTE — re-run every command before continuing and before stopping; report every row if you
> stop.**

## 1. Goal

- Thread `AppEntrySpec.initial_mode` from `emit_app_program_entry` (`mod.rs:542`) into
  `emit_main_bootstrap` (`bootstrap.rs:6`), which takes no `spec` today.
- Make the `NSWindow` + transcript block conditional on `initial_mode`: `None` skips window
  creation but **still reaches `[NSApp run]`** (not the headless `pause()` path). `Console`
  keeps today's behavior byte-for-byte.
- Fill B's `emit_app_mode_reconcile` seam for macOS: on `setMode`, marshal to the main thread
  and build or tear down the transcript window to match the new mode; implicit `term::off()`
  first (restore cooked state / clear `TERM_STATE_ACTIVE_OFFSET`, `term.rs:499`) so raw/grid
  state never leaks across a mode switch.

### Non-goals (explicit constraints)

- **Do not disturb `Console` startup.** A no-`setMode` program must produce the exact same
  window and goldens as today (`uses_term` and the transcript path unchanged).
- **Do not reuse the `pause()` headless path for `None`.** `None` must run the real event loop;
  the existing `MFB_MACAPP_HEADLESS` skip is coupled to a no-`[NSApp run]` test path
  (`bootstrap.rs:507`) and is for tests only.
- **No GTK.** That is plan-62-D. No shared code between them.
- **No `term::`/`io::input` wrong-mode errors** — that is E. C only wires the implicit
  `term::off()` that `setMode` performs.

## 2. Current State

`emit_main_bootstrap()` (`bootstrap.rs:6`) builds the `NSWindow` unconditionally (`:32-50`) and
then, gated only on the `MFB_MACAPP_HEADLESS` env var (`:64-73`), either builds the transcript
`MFBTextView` (`:106-137`, stashed as an NSApp associated object `:183-189`, shown `:473-480`)
or jumps to `after_show` (`:486`). The run split at `:495-528`: headless spawns the worker
inline and `pause()`es (no `[NSApp run]`); GUI calls `[NSApp run]` (`:523`) and defers the
worker to `applicationDidFinishLaunching:` (`:875`, `pthread_create` `:896`).

`[NSApp run]` with **no window** runs the event loop and never self-terminates:
`applicationShouldTerminateAfterLastWindowClosed:` returns YES (`:307`) but only fires on a
window *close*, so a never-created window never triggers it (verified by research). The io
helpers already fall back to the fd sink when the transcript associated object is nil
(`app_io.rs:111-116`) — so windowless `io::print` → stdout requires no new code, only that
`None` leaves the associated object unset.

### Measured populations

| What | Count | Command |
|---|---|---|
| `emit_main_bootstrap` args today | **0** | `rg -n 'fn emit_main_bootstrap' src/target/macos_aarch64/app/bootstrap.rs` |
| Existing transcript skip gate | env var only | `rg -n 'MFB_MACAPP_HEADLESS' src/target/macos_aarch64/app/bootstrap.rs` |
| io nil-view fd fallback exists | yes | `rg -n 'fd_path\|fall.*fd\|nil' src/target/macos_aarch64/app/app_io.rs` |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `[NSApp run]` stays alive windowless | **CONFIRMED** | terminate-after-last-window fires on close only (`bootstrap.rs:307`); research |
| A nil transcript view already routes `io::print` to the fd | **CONFIRMED** | `app_io.rs:111-116`; comment `bootstrap.rs:69` |
| The window/transcript builder receives no `spec` today | **CONFIRMED** | `emit_main_bootstrap` arg count = 0 |
| The headless skip is coupled to the no-run-loop path | **CONFIRMED** | skip target `after_show` leads to `pause()` (`:507`) not `[NSApp run]` for the headless branch |

## 3. Design Overview

The macOS surface work has two entry points that must agree: **startup** (build the right
surface for `initial_mode`) and **runtime `setMode`** (reconcile to the new mode on the main
thread). Correctness risk concentrates in the runtime transition — it crosses the worker→main
thread boundary and mutates the window/associated-object state that the io helpers read — so it
lands after the (simpler, inert-when-Console) startup change and is proven on-device.

### 3.1 Startup: conditional surface

Pass `initial_mode` into `emit_main_bootstrap`. Keep the `NSApplication` setup unconditional.
Guard the window+transcript construction on `initial_mode == Console` (fold the existing
`MFB_MACAPP_HEADLESS` test-skip into the same guard, but keep its distinct `pause()` run path
for the test env var only). For `None`, skip window/transcript creation and jump to a new label
that still reaches `[NSApp run]` with the worker deferred to `applicationDidFinishLaunching:`.

### 3.2 Runtime: the reconcile hook

Fill `emit_app_mode_reconcile` (B's seam) for macOS. `_mfb_rt_app_set_mode` runs on the worker;
the actual view mutation must marshal to the main thread (`performSelectorOnMainThread:` — the
established discipline, plan-13 master §2.4). The hook:

1. **Implicit `term::off()`** — if `TERM_STATE_ACTIVE_OFFSET` is set, run the off sequence
   (present final frame, restore content view / cooked state, clear the flag) so no raw/grid
   state survives the switch.
2. **Console → None:** order the window out / release it and clear the transcript associated
   object (so io falls to the fd). 
3. **None → Console:** build the window + transcript exactly as startup does, set the
   associated object, show it.

Because startup and the reconcile both need "build the Console surface," factor that into one
emitted routine both call — avoiding two drifting copies (the lifetime failure mode plan-13-J
warns about).

**Rejected alternative — tear down / rebuild `NSApplication` itself on each switch:** rejected;
only the *window* is per-mode. `NSApp` and the run loop persist across modes.

## Compatibility / Format Impact

- **Changed:** `emit_main_bootstrap` gains an `initial_mode` parameter; the window/transcript
  block becomes conditional. A macOS `--app` binary can now run windowless.
- **Unchanged:** `Console` startup output and goldens (assert byte-identical); the
  `MFB_MACAPP_HEADLESS` test path; `io::print`/`io::input` code (behavior shifts only because
  the associated object is now conditionally set).

## Phases

> **NOTE — tick `- [x]` in the same commit as the work. Unticked means NOT DONE.**

### Phase 1 — thread `initial_mode` into the bootstrap; conditional startup surface

- [ ] Give `emit_main_bootstrap` an `initial_mode` param; pass it from `emit_app_program_entry`
      (`mod.rs:542`).
- [ ] Guard window+transcript creation on `initial_mode == Console`; add a `None` path that
      skips them and reaches `[NSApp run]` with the worker deferred to
      `applicationDidFinishLaunching:`.
- [ ] Factor "build the Console window+transcript" into one emitted routine (startup calls it;
      the reconcile hook will too).

Acceptance: a `--app` program starting in `Console` (no `setMode`) is byte-identical to today
(goldens unchanged); a program whose static default is `None` (references `setMode`) launches,
shows no window, and stays alive; `io::print` before any `setMode` reaches stdout. Proven
on-device.
Commit: —

### Phase 2 — the runtime `setMode` reconcile (largest blast radius: cross-thread window mutation)

- [ ] Fill `emit_app_mode_reconcile` for macOS: implicit `term::off()`, then main-thread build
      or teardown of the transcript window to match the target mode; reuse the Phase-1 routine.
- [ ] Marshal the view mutation via `performSelectorOnMainThread:`; the worker blocks until the
      main thread confirms (so `getMode` post-`setMode` is coherent).

Acceptance: on-device, a program does `setMode(None)` → window disappears and `io::print` goes
to stdout; `setMode(Console)` → transcript window appears and `io::print` lands in it;
`term::on()` then `setMode(None)` leaves no raw/grid state (implicit off ran). A runtime golden
plus manual on-device confirmation.
Commit: —

## Validation Plan

- Tests: `tests/rt-behavior/app/` runtime goldens for the Console↔None transitions and the
  windowless-print-to-stdout path. Confirm they land in the gate denominator.
- Runtime proof: **on-device macOS** — visually confirm window appears/disappears and stdout
  routing; this is the falsifying proof that unit goldens cannot give for a real window.
- Coverage check: assert the `Console`-startup goldens are unchanged (byte-identical) — the
  guardrail that C did not regress today's behavior.
- Doc sync: note the `None` mode's windowless semantics in the `app::` spec/man pages.
- Acceptance: `scripts/test-accept.sh` green; the app-mode acceptance harness (~15min, per
  memory `bug-workflow-mechanics`).

## Open Decisions

1. **Does `setMode` block the worker until the main thread finishes reconciling?** Recommended
   **yes** (`waitUntilDone:YES`) so `getMode` and subsequent I/O see a coherent surface. The
   alternative (async) races the io helpers against a half-built window.
2. **`None`→worker timing.** Recommended keep the worker deferred to
   `applicationDidFinishLaunching:` in both modes, so startup ordering is identical Console vs
   None and only the window differs.

## Corrections

<!-- Filled in during execution. -->

## Summary

macOS `None` mode is unusually cheap because two mechanisms already exist: `[NSApp run]` is
happy windowless, and the io helpers already fall through to the fd when the transcript view is
nil. The real work is one conditional at startup and one cross-thread reconcile hook, and the
real risk is the reconcile's thread-boundary mutation — hence it lands last and is proven
on-device. Untouched: `Console` startup goldens, the test headless path, and the io helper code
itself.
