# plan-62-D: GTK4 `None`-mode bootstrap + the `setMode` surface transition

Last updated: 2026-07-24
Effort (Human): medium (1h–2h)
Effort (AI): medium (1h–2h)
<!-- Converges: dominated by hold/release balancing plus a cross-compile→ship→verify loop on the
     GTK box (box 2232). The AI authors faster but pays the same hardware round-trip, so the two
     land at the same band. -->

Depends on: plan-62-B. Feature-wide precondition: plan-62-A §Prerequisites.
Produces: a GTK4 `--app` binary that starts **windowless** when the initial mode is `None`
(holding the `GApplication` alive with `g_application_hold`) and reconciles its window at
runtime on `app::setMode`, with an implicit `term::off()`. Fans out from B alongside
plan-62-C (macOS); C and D share no code.

**The single behavioral outcome:** on the Debian aarch64 GTK4 box, `mfb build --app` of a
program that starts in `None` runs with no window and does not exit; `app::setMode(Console)`
presents the transcript window live; switching back to `None` hides it and keeps the app
running. `io::print` follows the same view-presence fallback as macOS.

References (read first):

- `src/target/linux_gtk/bootstrap.rs` — `emit_main_bootstrap` (`:43`), `gtk_application_new`
  (`:80`), the `"activate"` connect (`:83-90`), `g_application_run` (`:107`);
  `emit_activate_handler` (`:121`) with the **unconditional** `gtk_application_window_new`
  (`:135`) / `gtk_text_view_new` (`:184`) / `gtk_window_present` (`:235`) and worker spawn
  (`:281`).
- `src/target/linux_gtk/app_io.rs` — the GTK io/term app bodies (`emit_app_term_helper` `:10`),
  the counterpart of macOS's nil-view fd fallback.
- plan-62-B — `AppEntrySpec.initial_mode` and the `emit_app_mode_reconcile` seam this letter
  fills; plan-62-C — the macOS twin, for parity of the transition semantics (not shared code).

## Prerequisites

See plan-62-A §Prerequisites. Additionally:

| Must be true | Command | Status 2026-07-24 |
|---|---|---|
| plan-62-B landed (state slot + helpers + `initial_mode` + reconcile seam) | `rg -n 'initial_mode' src/target/shared/code/types.rs` | **NOT MET (B pending)** |
| The GTK4 box is reachable (`.ai/remote_systems.md`, box 2232) | `grep -n 'GTK4' .ai/remote_systems.md` | **MET** |
| No `g_application_hold` exists yet (confirming D must add it) | `rg -n 'g_application_hold' src/` → 0 | **MET (0 hits — must add)** |

> **NOTE — re-run every command before continuing and before stopping; report every row if you
> stop.** Linux boxes have no Rust toolchain except box 2229 (memory `linux-boxes-have-no-rust-toolchain`):
> cross-compile + ship; fixtures use repo-root-relative paths.

## 1. Goal

- Thread `AppEntrySpec.initial_mode` from `emit_app_program_entry` (`mod.rs:418`, and the
  `_x86` variant `:450`) into `emit_main_bootstrap` (`bootstrap.rs:43`) and
  `emit_activate_handler` (`:121`), which take no `spec` today.
- In `activate`, build the window+transcript only when `initial_mode == Console`. For `None`,
  **add `g_application_hold(app)`** so `GApplication` does not exit with zero windows (there is
  no hold call today — the app would exit immediately otherwise), and still spawn the worker.
- Fill B's `emit_app_mode_reconcile` seam for GTK: on `setMode`, marshal to the main loop
  (`g_idle_add` — the established GTK discipline, plan-13 master §2.4) and build/destroy the
  window to match the new mode, balancing `g_application_hold`/`g_application_release` so the
  app stays alive across `None` and self-terminates correctly; implicit `term::off()` first.

### Non-goals (explicit constraints)

- **Do not disturb `Console` startup.** A no-`setMode` program must produce today's window and
  goldens unchanged.
- **No macOS.** That is plan-62-C. No shared code.
- **No `term::`/`io::input` wrong-mode errors** — that is E; D only wires the implicit
  `term::off()`.

## 2. Current State

`emit_main_bootstrap` (`bootstrap.rs:43`) creates the `GtkApplication` (`:80`), connects
`"activate"` (`:83`), and calls `g_application_run` (`:107`). `emit_activate_handler` (`:121`)
**unconditionally** creates the window (`gtk_application_window_new` `:135`), the transcript
`GtkTextView` (`:184`, editable FALSE `:188`, monospace `:191`), scrolls it in (`:196-202`),
presents it (`gtk_window_present` `:235`), and spawns the worker (`pthread_create` `:281`,
`pthread_detach` `:283`). No env gate, no presentation branch, and **no `g_application_hold`
anywhere in `src/`** (`rg` → 0). `GApplication` exits when it holds zero windows, so a
windowless `activate` that just returns causes an immediate exit — the hold call is mandatory
for `None`.

Neither `emit_main_bootstrap` nor `emit_activate_handler` receives `spec` today; only
`emit_worker_shim` does (`mod.rs:426`). Threading `initial_mode` into the activate handler is
the enabling change.

### Measured populations

| What | Count | Command |
|---|---|---|
| `g_application_hold` calls today | **0** | `rg -n 'g_application_hold' src/` |
| `emit_activate_handler` presentation gates today | **0** | `rg -n 'fn emit_activate_handler' src/target/linux_gtk/bootstrap.rs` (body is unconditional) |
| GTK app-mode LOC (scale) | 3417 | `wc -l src/target/linux_gtk/*.rs` (plan-13 §2.1) |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| GTK exits with zero windows unless held | **CONFIRMED (framework)** | `GApplication` semantics; no `g_application_hold` in tree to prevent it |
| The transcript is created unconditionally in `activate` | **CONFIRMED** | `bootstrap.rs:135/184/235`, no gate (research; plan-13 master §2.4 claim verified) |
| The activate handler receives no `spec` today | **CONFIRMED** | `mod.rs:418/450` pass `spec` only to `emit_worker_shim` |

## 3. Design Overview

Symmetric with macOS (plan-62-C) but with GTK's lifecycle wrinkle: **the app's aliveness is
window-count-driven**, so `None` must actively hold the application. Correctness risk
concentrates in balancing hold/release across transitions (an unbalanced hold leaks the process
alive after the last intended window; an over-release exits mid-run) — so the runtime reconcile
lands after the inert-when-Console startup change and is proven on the GTK box.

### 3.1 Startup: conditional surface + hold

Thread `initial_mode` into `emit_activate_handler`. For `Console`, unchanged. For `None`: skip
window/transcript creation, call `g_application_hold(app)` (import it in the GTK symbol
surface — note the flavor-aware import rules, plan-13 master §2.5 / plan-56), and spawn the
worker as today. The app now stays alive windowless.

### 3.2 Runtime: the reconcile hook

Fill `emit_app_mode_reconcile` for GTK, marshalling via `g_idle_add`:

1. **Implicit `term::off()`** (clear `TERM_STATE_ACTIVE_OFFSET`, restore state) as on macOS.
2. **Console → None:** destroy/hide the window; ensure a `g_application_hold` is in effect so
   the app survives the window's disappearance.
3. **None → Console:** if a hold was taken at startup, `g_application_release` it as the window
   takes over aliveness; build + present the window+transcript (reuse one emitted routine, as
   macOS does, to avoid two drifting copies).

Track the hold state so the app has exactly one aliveness source at all times (a window *or* a
hold, never zero, never a leaked extra hold).

**Rejected alternative — never release the hold, rely only on it:** rejected; then the app
never self-terminates on the last window close in a Console-only session, changing today's exit
behavior.

## Compatibility / Format Impact

- **Changed:** `emit_activate_handler`/`emit_main_bootstrap` gain `initial_mode`; the window
  block becomes conditional; `g_application_hold`/`release` are newly imported and used.
- **Unchanged:** `Console` startup output and goldens (assert byte-identical); the io/term app
  bodies (behavior shifts only via view presence).

## Phases

> **NOTE — tick `- [x]` in the same commit as the work. Unticked means NOT DONE.**

### Phase 1 — thread `initial_mode`; conditional startup surface + `g_application_hold`

- [ ] Thread `initial_mode` from `emit_app_program_entry` (`mod.rs:418`/`:450`) into
      `emit_main_bootstrap` (`:43`) and `emit_activate_handler` (`:121`).
- [ ] Guard window+transcript creation on `initial_mode == Console`. For `None`, add and import
      `g_application_hold(app)` (flavor-aware import surface, plan-56) and spawn the worker.
- [ ] Factor "build the Console window+transcript" into one emitted routine (startup + reconcile
      both call it).

Acceptance: on the GTK box, a `Console`-default program is byte-identical to today (goldens
unchanged); a `None`-default program launches, shows no window, and does not exit;
`io::print` before `setMode` reaches stdout.
Commit: —

### Phase 2 — the runtime `setMode` reconcile + hold balancing (largest blast radius)

- [ ] Fill `emit_app_mode_reconcile` for GTK via `g_idle_add`: implicit `term::off()`, then
      build/destroy the window to match the target mode; reuse the Phase-1 routine.
- [ ] Balance `g_application_hold`/`g_application_release` so aliveness has exactly one source
      at all times; track the hold state.

Acceptance: on the GTK box, `setMode(None)` hides the window and keeps the app alive with
`io::print` → stdout; `setMode(Console)` presents the window with `io::print` → transcript; a
Console-only program still self-terminates on window close (hold balanced). Runtime golden +
on-box confirmation.
Commit: —

## Validation Plan

- Tests: `tests/rt-behavior/app/` runtime goldens for GTK Console↔None; confirm gate
  denominator inclusion.
- Runtime proof: **on the Debian aarch64 GTK4 box (box 2232)** — window appears/disappears, app
  stays alive windowless, Console-only exit unchanged. Cross-compile + ship (memory
  `linux-boxes-have-no-rust-toolchain`).
- Coverage check: assert `Console`-startup goldens unchanged.
- Doc sync: shared with plan-62-C — the `None` windowless semantics in `app::` spec/man.
- Acceptance: `scripts/test-accept.sh` green; app-mode acceptance harness.

## Open Decisions

1. **Hold granularity** — recommended a single tracked hold taken on entering `None` and
   released on leaving it, so hold count is always 0 (window present) or 1 (windowless). Simpler
   than reference-counting per transition.
2. **`g_application_hold` import flavoring** — recommended follow plan-56's flavor-aware GTK
   import surface; verify the symbol resolves under both glibc and musl AppImages (plan-51).

## Corrections

<!-- Filled in during execution. -->

## Summary

GTK `None` mode is macOS's twin plus one framework wrinkle: aliveness is window-count-driven, so
`None` must hold the `GApplication` and every transition must keep exactly one aliveness source.
That balancing is the real risk and lands last, proven on the GTK box. Untouched: `Console`
startup goldens and the io/term app bodies.
