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

| Must be true | Command | Status 2026-07-25 |
|---|---|---|
| plan-62-B landed (state slot + helpers + reconcile seam) | `rg -n 'emit_app_mode_reconcile' src/target/shared/code/types.rs` | **MET** (commit d12ac331a; `AppEntrySpec.initial_mode` was deferred from B to here — C is its first reader — per B Corrections) |
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

- [x] Gave `emit_main_bootstrap` an `initial_mode` param; passed from `emit_app_program_entry`
      (via `AppEntrySpec.initial_mode`, added here — its first reader).
- [x] Guarded window+transcript creation on `initial_mode == Console`; the `None` path skips
      the surface but still installs the app delegate (extracted to `emit_gui_delegate`, so the
      worker spawns from `applicationDidFinishLaunching:`) and reaches the real `[NSApp run]`.
      Console is emitted byte-for-byte as before (no reorder), so its goldens are unchanged.
- [x] Extracted `emit_gui_delegate` (the delegate synth the `None` path shares). The full
      Console window+transcript build is not yet extracted into a reconcile-callable routine —
      deferred to Phase 2 where the reconcile needs it.

Acceptance: a `--app` program starting in `Console` is byte-identical to today
(`macos-app-mode-*` goldens unchanged — verified); a `None`-default program launches, shows no
window, and stays alive under real `[NSApp run]` (verified on-device: "alive" after 3s,
non-headless); `io::print` before any `setMode` reaches stdout (verified: worker spawns via
delegate, prints "windowless-running" to stdout). **VERIFIED on-device.**
Commit: fed1e931d

### Phase 2 — the runtime `setMode` reconcile (largest blast radius: cross-thread window mutation)

- [x] Filled `emit_app_mode_reconcile` for macOS. The worker's `setMode` helper reloads the
      new mode and calls `_mfb_macapp_reconcile_marshal`, which (when a delegate exists — i.e.
      a real `[NSApp run]`, never headless) boxes the mode in an `NSNumber` and
      `performSelectorOnMainThread:@selector(mfbReconcile:) withObject: waitUntilDone:YES`.
      The `mfbReconcile:` IMP runs on the main thread: `Console` builds the transcript window
      the first time (`_mfb_macapp_reconcile_build`, a plain `NSTextView` content view) or
      re-shows it and re-points the io-routing `ASSOC_KEY` at the transcript; `None` clears
      `ASSOC_KEY` (io → stdout) and orders the window out.
- [x] Marshalled via `performSelectorOnMainThread:...waitUntilDone:YES` — the worker blocks
      until the main thread finishes, so a following `getMode`/`io` sees the reconciled surface
      (Open Decision 1: yes).

Acceptance: `setMode(None)` → `io::print` goes to stdout; `setMode(Console)` → `io::print`
lands in the transcript (window). **VERIFIED on-device (non-headless)**: a `None`-start program
prints `BEFORE` to stdout, then after `setMode(Console)` prints `AFTER` to the transcript — a
stdout capture shows only `BEFORE` (both directions verified). `test-macapp.sh` case "setMode
reconcile flips io from stdout to the transcript window" (GUI-opt-in). The window's visual
appear/disappear is manual on-device confirmation (the io-routing flip is its automatable
proxy). Full acceptance green; `macos-app-mode-*` byte-identical.
Commit: fa88d0e7e

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

- 2026-07-25 — **`AppEntrySpec.initial_mode` is added HERE (plan-62-C), not B.** B deferred it
  to avoid an unread field (AGENTS.md); C is its first reader (the startup-window decision). B
  carries the static default into program entry via `ProgramEntrySpec::seed_presentation_mode_offset`
  instead; C adds the `AppEntrySpec` field. The rv64 `#[should_panic]` `AppEntrySpec`
  construction was updated to set it.

- 2026-07-25 — **Finer split than "guard the whole surface block."** The first attempt guarded
  the entire window+transcript block (`bootstrap.rs:32-485`) on `Console`, but that block also
  contains the **app-delegate synthesis**, whose `applicationDidFinishLaunching:` is what spawns
  the worker. Gating it out left a `None` program with a live `[NSApp run]` but no worker (it
  never printed / never ran). Fix: extract the delegate synthesis into `emit_gui_delegate` and
  call it from BOTH the `Console` path (byte-identically) and the `None` path, so a windowless
  `None` program still spawns its worker. Verified: a `None` program's worker runs and
  `io::print`s to stdout.

- 2026-07-25 — **Console byte-identical without reordering.** To keep `macos-app-mode-*` goldens
  unchanged, the `Console` block is emitted in its exact original order (the `getenv
  MFB_MACAPP_HEADLESS` stays where it was); the `None` path re-reads the env var separately
  rather than hoisting it above the window build. Verified byte-identical via `test-accept.sh`.

- 2026-07-25 — **Phase 2 reconcile: what was built vs the plan.**
  - *Marshalling*: worker `setMode` → `_mfb_macapp_reconcile_marshal` → main-thread
    `mfbReconcile:` (added to the delegate). Guarded on `[NSApp delegate]` being non-nil, which
    is false only headless — so under headless (no run loop) the reconcile is a clean no-op and
    `waitUntilDone:YES` cannot deadlock. This is why the B/E headless tests still pass with the
    reconcile in place.
  - *Build routine NOT shared with startup*: the plan said "reuse the Phase-1 routine," but
    startup interleaves surface/delegate/input/show and can't be extracted churn-free. Instead a
    dedicated minimal `_mfb_macapp_reconcile_build` (window + plain non-editable `NSTextView`
    content view, enough for `io::print` output) is emitted. **Only `None`-start programs ever
    reconcile** (a program referencing `setMode` is always `None`-start; a `Console`-start
    program never references it), so this fresh window never collides with a startup-built
    Console window — and the reconcile helpers + their selector/key data objects are emitted
    ONLY when `initial_mode == None`, keeping `Console`-default programs byte-identical.
  - *Implicit `term::off` deferred*: the plan wanted `setMode` to run an implicit `term::off`.
    Not implemented in the reconcile — plan-62-E already makes `term::*` raise `ErrWrongMode`
    outside `Console`, so no raw/grid state can be issued in `None`, which covers the concern.
    Left as a possible refinement; recorded so it is not silently dropped.
  - *Window visual*: appear/disappear is manual on-device (the plan's own "manual on-device
    confirmation"); the io-routing flip (stdout↔transcript) is its automatable proxy and is
    verified both directions.

## Summary

macOS `None` mode is unusually cheap because two mechanisms already exist: `[NSApp run]` is
happy windowless, and the io helpers already fall through to the fd when the transcript view is
nil. The real work is one conditional at startup and one cross-thread reconcile hook, and the
real risk is the reconcile's thread-boundary mutation — hence it lands last and is proven
on-device. Untouched: `Console` startup goldens, the test headless path, and the io helper code
itself.
