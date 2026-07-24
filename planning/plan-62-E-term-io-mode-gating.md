# plan-62-E: `term::` / `io::` mode gating (the wrong-mode errors)

Last updated: 2026-07-24
Effort (Human): medium (1h–2h)
Effort (AI): small (<1h)
<!-- Corrected 2026-07-24: the prior Human small / AI medium was inverted (an AI is never slower
     than a human at the same task). The mechanism is tiny (one error code + two guarded checks);
     authoring diverges (AI faster), and the 2-platform golden/acceptance proof converges. Net:
     Human medium, AI small. -->

Depends on: plan-62-C + plan-62-D. Feature-wide precondition: plan-62-A §Prerequisites.
Produces: the mode-gating semantics — `term::*` and `io::input`/`io::readLine`/`io::readChar`
raise a **trappable "wrong mode" runtime error** when the current presentation mode is not
`Console` in an `--app` build; and confirmation that `io::print`/`io::write` route to the
transcript in `Console` and to stdout otherwise (already true via view presence — E asserts it
against the mode state). This is the last letter; it consumes a working `None` mode from C/D.

**The single behavioral outcome:** in an `--app` build, calling `term::moveTo(...)` or
`io::input(...)` while in `None` mode raises a runtime error a program can `TRAP`; the same
calls in `Console` mode work exactly as today. In a plain `NativeBuildMode::Console` (non-app)
build there is no gate — `term::` and `io::` behave exactly as today, because there is no
`app::Mode` there at all.

References (read first):

- `src/target/shared/code/mod.rs:1481-1512` — the `term::` dispatch (`is_term_call`,
  `emit_app_term_helper`), and `:1577-1660` — the `io::print`/`write`/`input`/`readLine`
  dispatch, both branching on `app_mode = build_mode.is_app()`.
- `src/target/shared/code/term.rs` — `emit_gate_inactive` (`:127`), the existing TUI-active
  gate every term helper except `on`/`isOn` already runs; the natural place to add the
  presentation-mode check.
- `src/target/shared/code/error_constants.rs` — the runtime error code table; a new
  `WRONG_MODE` code goes here beside the term/io codes.
- plan-62-B — `PRESENTATION_MODE_OFFSET` (the slot E reads).

## Prerequisites

See plan-62-A §Prerequisites. Additionally:

| Must be true | Command | Status 2026-07-24 |
|---|---|---|
| plan-62-C landed (macOS None mode works) | `rg -n 'initial_mode' src/target/macos_aarch64/app/bootstrap.rs` | **NOT MET (C pending)** |
| plan-62-D landed (GTK None mode works) | `rg -n 'g_application_hold' src/target/linux_gtk/` | **NOT MET (D pending)** |
| The runtime error mechanism is trappable | `rg -n 'TRAP' src/docs/spec/` — confirm runtime errors are catchable | **verify before Phase 1** |

> **NOTE — re-run every command before continuing and before stopping; report every row if you
> stop.** E is testable only once a non-`Console` mode actually exists (C + D) — gating against
> a mode that cannot be entered proves nothing.

## 1. Goal

- A new trappable runtime error `WRONG_MODE` (`error_constants.rs`).
- In `--app` builds only: `term::*` (all functions) and `io::input`/`io::readLine`/`io::readChar`
  check `PRESENTATION_MODE_OFFSET`; if it is not `Console`, raise `WRONG_MODE` before doing any
  work. In console builds the check is absent (no `app::Mode`).
- Confirm (and lock with a test) that `io::print`/`io::write` reach the transcript in `Console`
  and stdout in `None` — the view-presence fallback from C/D, now asserted against the mode
  state rather than incidentally.

### Non-goals (explicit constraints)

- **No gate in console builds.** The presentation-mode check is emitted only when
  `build_mode.is_app()`. A `NativeBuildMode::Console` binary keeps today's `term::`/`io::`
  behavior with zero added branches.
- **`io::print`/`io::write` never error.** They are universal and degrade gracefully
  (transcript vs stdout). Only the *reading* side (`io::input` family) and `term::` hard-fail
  outside `Console` (§3.1 rationale).
- **Do not change the implicit `term::off()` on `setMode`** — that lives in C/D. E only adds
  the outside-Console *rejection* of term calls.

## 2. Current State

Every `term::` helper except `on`/`isOn` already begins with `emit_gate_inactive` (`term.rs:127`),
which no-ops the call when the TUI grid is inactive. That is a *within-Console* gate (is the
grid on?), not a *mode* gate (are we in Console at all?). E adds the mode gate one level up:
outside `Console`, the call is not a silent no-op — it is a `WRONG_MODE` error.

`io::input` in app mode already renders a prompt to the transcript and reads a line
(`app_io.rs:245`); `io::readLine`/`readChar` read fd 0, which in app mode is the window pipe
(`bootstrap.rs:369` dup2). In `None` mode there is no window feeding that pipe, so an ungated
read would block forever — which is exactly why the reading side must hard-fail with
`WRONG_MODE` rather than hang (§3.1).

`io::print`/`io::write` in app mode route by view presence (`app_io.rs:47-116`): TUI grid →
transcript → **fd fallback when the view is nil**. C/D make the view presence track the mode,
so this already yields "transcript in Console, stdout otherwise" — E only pins it with a test.

### Measured populations

| What | Count | Command |
|---|---|---|
| `term::` functions to gate | **17** | `rg -c 'const [A-Z_]+: &str = "term\.' src/builtins/term.rs` |
| `io::` reading functions to gate | **3** (`input`, `readLine`, `readChar`) | `rg -n '"io.(input\|readLine\|readChar)"' src/builtins/io.rs` |
| `io::` writing functions (must NOT gate) | print/write/printError/writeError | `rg -n '"io.(print\|write)' src/builtins/io.rs` |

### Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| term helpers already have a per-call gate seam (`emit_gate_inactive`) | **CONFIRMED** | `term.rs:127`, run by all but `on`/`isOn` |
| An ungated read in `None` would block forever (window pipe has no producer) | **CONFIRMED** | fd 0 is the window pipe (`bootstrap.rs:369`); no window in `None` → no writer |
| `io::print` already falls to fd when the view is nil | **CONFIRMED** | `app_io.rs:111-116` (plan-62-C §2) |
| Runtime errors are trappable | **UNVERIFIED** | Phase 1 task — confirm `WRONG_MODE` can be `TRAP`ped, else it is a fatal abort (Open Decision 1) |

## 3. Design Overview

One new error code and two gate insertions (term side, io-read side), both emitted only under
`is_app()` and both reading the same slot. Low design uncertainty; the only genuine decision is
graceful-vs-hard-fail per surface, resolved by a principle below.

### 3.1 The gating principle: universal I/O degrades, specialized I/O hard-fails

- **`io::print`/`io::write` are universal** — every program writes output. Outside `Console`
  they degrade gracefully to stdout (no error). This is what makes "compile a CLI app, get a
  window" and "print debug from a windowless app" both work.
- **`term::` is specialized** — it needs the character-cell grid that only `Console` provides.
  Outside `Console` there is nothing for it to address, so it hard-fails with `WRONG_MODE`.
- **`io::input`/`readLine`/`readChar` read the console** — outside `Console` the window input
  pipe has no producer, so a read would hang. Hard-fail with `WRONG_MODE` (do not silently EOF:
  a program blocking on input in `None` is a bug the error surfaces immediately).

This asymmetry is the user's decision, and the principle above is what keeps it from reading as
arbitrary.

### 3.2 Where the check goes

- **term:** add a presentation-mode check ahead of `emit_gate_inactive` in the shared term
  lowering (`term.rs`), guarded on `app_mode`. Emitted for **all** term calls including `on`
  (calling `term::on()` in `None` is itself a wrong-mode error). The app-backend
  `emit_app_term_helper` bodies (C/D) inherit the same check via the shared path or replicate it.
- **io read:** add the check at the head of `lower_io_read_line_helper` / `read_char` /
  `emit_app_io_input_helper` (`io_stdin.rs`, `app_io.rs`), guarded on `app_mode`.

## Compatibility / Format Impact

- **New:** a `WRONG_MODE` runtime error; mode checks in `term::` and the io-read helpers,
  **app builds only**.
- **Unchanged:** all console-build behavior (no check emitted); `io::print`/`io::write` (never
  gated); the implicit `term::off()` on `setMode` (owned by C/D).

## Phases

> **NOTE — tick `- [x]` in the same commit as the work. Unticked means NOT DONE.**

### Phase 1 — the `WRONG_MODE` error + `term::` gate

- [ ] Confirm the runtime-error path is trappable; if not, decide fatal-abort vs. adding
      trappability (Open Decision 1) and record it.
- [ ] Add `WRONG_MODE` to `error_constants.rs`.
- [ ] Emit a `PRESENTATION_MODE_OFFSET != Console` check (app builds only) at the head of the
      shared term lowering, covering all 17 term functions including `on`.

Acceptance: in a `--app` build, `term::moveTo(1,1)` while in `None` raises `WRONG_MODE`
(trappable); the same call in `Console` behaves as today; a console (non-app) build is
byte-identical to today (no check emitted). Runtime golden + a `TRAP` test.
Commit: —

### Phase 2 — the io-read gate + the io-write assertion

- [ ] Emit the same `!= Console` check (app builds only) at the head of `io::input`/`readLine`/
      `readChar`.
- [ ] Add a test asserting `io::print` reaches the transcript in `Console` and stdout in `None`
      (locking the C/D view-presence behavior against the mode state).

Acceptance: `io::input()` in `None` raises `WRONG_MODE` rather than hanging; in `Console` it
reads as today; `io::print` routing is proven for both modes. Runtime goldens on macOS device
and the GTK box.
Commit: —

## Validation Plan

- Tests: `tests/rt-behavior/app/` — wrong-mode rejection for term + io-read, a `TRAP`-recovers
  case, and the io-write routing assertion. Negative cases are the substance.
- Runtime proof: macOS device + GTK box — a program that traps `WRONG_MODE` from `term::moveTo`
  in `None`, then `setMode(Console)` and succeeds.
- Coverage check: confirm console-build term/io goldens are byte-identical (the check must not
  leak into non-app builds).
- Doc sync: document the wrong-mode error and the per-surface gating principle (§3.1) in the
  `app::`/`term::`/`io::` spec and man pages.
- Acceptance: `scripts/test-accept.sh` green; app-mode acceptance harness.

## Open Decisions

1. **Trappable vs fatal `WRONG_MODE`** — recommended **trappable**, so a program can probe a
   surface and adapt. Verify the runtime-error mechanism supports `TRAP` (Phase 1 task); if it
   only supports fatal aborts, that is a larger change and becomes its own prerequisite.
2. **Does `term::on()` in `None` error, or implicitly `setMode(Console)`?** Recommended
   **error** (`WRONG_MODE`) — implicit mode changes from `term::` would reintroduce exactly the
   undefined cross-mode behavior plan-62 exists to remove. Mode changes go through `setMode`
   only.

## Corrections

<!-- Filled in during execution. -->

## Summary

E is small in mechanism — one error code and two guarded checks reading one slot — but it is
where the model's semantics become observable: `term::` and console *input* hard-fail outside
`Console`, while `io::print` degrades to stdout. The guiding principle (universal I/O degrades,
specialized I/O hard-fails) is what makes the asymmetry principled rather than arbitrary.
Untouched: every console-build path, and `io::print`/`io::write` themselves.
