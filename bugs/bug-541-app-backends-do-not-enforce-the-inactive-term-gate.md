# bug-541: the "does nothing while TUI mode is off" gate is not enforced by the Linux or Windows app backends

Last updated: 2026-09-04
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: — (none exists; Phase 1 adds one)

`term::` has one module-wide rule: while TUI mode is off, every call except
`term::on`, `term::isOn` and `term::didResize` short-circuits — the setters and
drawing calls do nothing, the getters return inert defaults, and
`term::terminalSize` raises `ErrUnsupported`. The console and macOS backends
enforce it on every member. The **Linux GTK** and **Windows** app backends do
not, and they fail differently:

- **GATE-01 (Windows)** `term::terminalSize` before `term::on`, or after
  `term::off`, returns a `term::TermSize` instead of raising `ErrUnsupported`.
  This is the one that changes a program's control flow: code written to
  `TRY term::terminalSize()` and fall back never takes the fallback on Windows
  app mode, and a program that uses the raise to detect "TUI not entered" gets a
  plausible-looking 80x25 instead.
- **GATE-02 (Windows)** `term::off` does not release the drawing surface, so
  every positioned drawing call, `term::clear`, `term::moveTo` and the colour and
  attribute setters still reach it afterwards and mutate state that should be
  inert. The Windows helpers gate on the live `TUI_MEMDC` handle rather than on
  the shared `active` slot, and `term::off` never clears that handle.
- **GATE-03 (Linux GTK)** `term::off` while TUI mode is already off is not a
  no-op: it schedules the present and hide callbacks unconditionally, so a
  redundant `term::off` still asks the window to restore itself.

**The single correct behavior a fix produces:** on every backend, a `term::` call
made while TUI mode is off behaves exactly as the console backend makes it
behave — inert for the setters, drawing calls, `clear`, `moveTo` and `sync`;
inert defaults from the getters; `ErrUnsupported` from `terminalSize`; and
`on`/`isOn`/`didResize` answering either way.

Severity is MEDIUM rather than HIGH because a well-formed program reaches
`term::off` last and never observes GATE-02 or GATE-03. GATE-01 is observable by
a correct program.

References:

- `mfb man term` — the no-op rule and gap 3, which currently DISCLOSES this
  rather than promising it; `mfb man term isOn` for the three ungated calls;
  `mfb man term terminalSize` for the Windows `ErrUnsupported` exception;
  `mfb man term off` for the app-mode caveat.
- `mfb spec app term-backend` — the opening paragraph, which since `fc1860141`
  states plainly that the inactive gate is NOT uniform and names which backend
  tests what.
- Found during the `term::` row/column coordinate migration, commit `fc1860141`.
- Sibling gaps filed at the same time: bug-539 (GTK draws nothing), bug-540
  (Windows reduced implementation). GATE-02 becomes broader in scope once
  bug-539 lands, because GTK will then have real drawing writers to gate.

## Failing Reproduction

```
cat > /tmp/termgate/src/main.mfb <<'MFB'
IMPORT term
IMPORT io
IMPORT color
FUNC main() AS Integer
  ' GATE-01: before term::on, terminalSize must raise.
  TRY
    LET s AS term::TermSize = term::terminalSize()
    io::print("GATE-01 FAIL: returned " & toString(s.columns) & "x" & toString(s.rows))
  CATCH e
    io::print("GATE-01 ok: raised")
  END TRY

  term::on()
  term::drawText(1, 1, "before off")
  term::sync()
  term::off()

  ' GATE-02: after term::off these must all be inert.
  term::setForeground(color::rgb(255, 0, 0))
  term::drawText(3, 1, "AFTER OFF - MUST NOT APPEAR")
  term::fillRect(term::FillStyle.Filled, 5, 1, 6, 20)
  term::sync()

  ' GATE-03: a redundant off must do nothing at all.
  term::off()
  RETURN 0
END FUNC
MFB
mfb build --app -target windows-x86_64 /tmp/termgate   # ship to 2230
mfb build --app -target linux-x86_64   /tmp/termgate   # ship to 2228
mfb build /tmp/termgate                                 # console oracle, any box
```

- Observed, Windows app (2230): `GATE-01 FAIL: returned 80x25`; the "AFTER OFF"
  text and the filled block appear on the restored surface.
- Observed, Linux GTK app (2228): `GATE-01 ok`; the drawing calls are inert only
  because GTK does not implement them at all (bug-539) — GATE-03 is visible as
  the redundant `term::off` still driving the hide/present idles.
- Expected, every backend: `GATE-01 ok: raised`; nothing painted after
  `term::off`; the second `term::off` a complete no-op.

Contrast cases that work today and bound the bug:

| Environment | Build | GATE-01 | GATE-02 | GATE-03 |
| --- | --- | --- | --- | --- |
| console (any platform) | `mfb build` | ✓ | ✓ | ✓ |
| macOS app | `mfb build --app` | ✓ | ✓ | ✓ |
| Linux GTK app (2228) | `mfb build --app` | ✓ | n/a (bug-539) | ✗ |
| Windows app (2230) | `mfb build --app` | ✗ | ✗ | ✓ |

## Root Cause

The gate has three different implementations and only two of them are complete.

**Console** — every writer opens with
`src/codegen/term/core/term.rs:emit_gate_inactive`, which loads
`term_state_offset + TERM_STATE_ACTIVE_OFFSET` and branches past the body when it
is zero; the getters take a default through `emit_get_color`/`emit_get_attr`; and
`term::terminalSize` has its own arm that emits `ErrUnsupported` on the inactive
branch.

**macOS** — every app body opens with
`src/target/macos_aarch64/app/app_io.rs:emit_term_active_gate`, reading the same
shared slot. `emit_app_terminal_size` branches to its `unsupported` label from
the same test.

**GATE-01 / GATE-02 (Windows)** — `src/target/win_x86_64/app/mod.rs` has no
equivalent of `emit_term_active_gate`. Its drawing bodies instead open by loading
`TUI_MEMDC_SYM` and branching to their done label when it is null, which works as
a gate only for as long as the memDC's lifetime matches TUI mode's. It does not:
`emit_term_off` stores zero into the shared `active` slot, re-shows the edit
window and invalidates the client, but never destroys or clears `TUI_MEMDC_SYM`.
The handle stays live for the life of the process, so after `term::off` every
memDC-gated body proceeds. `emit_term_move_to` and `emit_term_size` do not even
have that much — neither reads the memDC nor the active slot, so `moveTo` mutates
the cursor globals while off, and `emit_term_size` unconditionally allocates a
record and stores the `TUI_COLS`/`TUI_ROWS` constants into it, which is GATE-01.

**GATE-03 (Linux GTK)** — `src/target/linux_gtk/app_io.rs:emit_app_term_off`
writes the inactive state and then calls `g_idle_add` twice (the final present,
then the hide) with no preceding active test, unlike its sibling
`emit_app_term_move_to`, which opens with `emit_gtk_term_active_gate`.

Note the shared `active` slot is written correctly by all four backends — this is
not a state-tracking bug. Every backend can see that TUI mode is off; two of them
do not consult it on every member.

## Goal

- **GATE-01** Windows `emit_term_size` raises `ErrUnsupported` while the shared
  `active` slot is zero, matching the console and macOS arms.
- **GATE-02** every Windows `term::` body except `on`, `isOn` and `didResize`
  tests the shared `active` slot — not the memDC handle — and short-circuits when
  it is zero. `emit_term_move_to` gains a gate it currently has none of.
- **GATE-03** GTK `emit_app_term_off` opens with `emit_gtk_term_active_gate`, so
  a redundant `term::off` schedules nothing.
- The reproduction above prints `GATE-01 ok` and paints nothing after
  `term::off`, identically on the console, macOS app, Linux app and Windows app.

### Non-goals (must NOT change)

- The three ungated calls. `term::on`, `term::isOn` and `term::didResize` must
  stay ungated on every backend; `didResize` in particular must keep reading
  `FALSE` (not raising, not no-oping into an undefined value) before any
  `term::on`.
- The memDC's own lifetime and the `WM_PAINT`/`BitBlt` present path. The fix adds
  an `active` test; it does not need to destroy the memDC, and destroying it
  would interact with the mode-switch case that
  `src/target/win_x86_64/app/mod.rs` documents around `TUI_MEMDC_SYM` (a program
  that leaves `Console` for `Canvas` deliberately leaves the handle live).
- The `prepend_wrong_mode_gate` presentation-mode gate in
  `src/codegen/builtins/term/gen_shared.rs`. That is a different, orthogonal gate
  (`ModeRequirement::Console`) and is correct; do not fold the two together.
- The coordinate work landed in `fc1860141`.
- **Tempting wrong fix, explicitly forbidden:** clearing `TUI_MEMDC_SYM` in
  `emit_term_off` and calling GATE-02 fixed. That makes the *drawing* bodies
  inert by accident while leaving `moveTo`, the setters and `emit_term_size`
  ungated, and it couples the gate to a resource lifetime that the mode-switch
  case deliberately keeps live. Gate on the `active` slot, like the other two
  backends.

## Blast Radius

Found with `grep -n 'emit_term_active_gate\|emit_gtk_term_active_gate\|emit_gate_inactive\|TERM_STATE_ACTIVE_OFFSET\|TUI_MEMDC_SYM' src/codegen/term/core/term.rs src/target/macos_aarch64/app/app_io.rs src/target/linux_gtk/app_io.rs src/target/win_x86_64/app/mod.rs`
and by walking every arm of each backend's `emit_app_term_helper`.

- `src/target/win_x86_64/app/mod.rs:emit_term_size` — GATE-01; fixed here.
- `src/target/win_x86_64/app/mod.rs:emit_term_move_to` — GATE-02, no gate at all;
  fixed here.
- `src/target/win_x86_64/app/mod.rs` — `emit_term_clear`, `emit_term_sync`, the
  colour/attribute/cursor setters, `emit_term_draw_line`, `emit_term_draw_box`,
  `emit_term_fill_rect`, `emit_term_draw_text_at`, `emit_term_draw_glyph_at`:
  GATE-02, memDC-gated only; all fixed here — the gate must be added uniformly or
  the module rule stays half-true.
- `src/target/linux_gtk/app_io.rs:emit_app_term_off` — GATE-03; fixed here.
- `src/target/linux_gtk/app_io.rs` — every other arm already opens with
  `emit_gtk_term_active_gate`; unaffected, and is the local model for GATE-03.
- `src/target/macos_aarch64/app/app_io.rs` — unaffected: every body opens with
  `emit_term_active_gate`. It is the oracle for GATE-01/GATE-02.
- `src/codegen/term/core/term.rs:emit_gate_inactive` — unaffected; the console
  oracle.
- `src/codegen/builtins/term/gen_shared.rs:lower_term_helper` — unaffected: the
  `prepend_wrong_mode_gate` it applies is the presentation-mode gate, a different
  contract.
- Six GTK positioned drawing helpers — do not exist yet (bug-539). When they
  land they must open with `emit_gtk_term_active_gate`; cross-linked so bug-539
  Phase 2 does not reintroduce GATE-02's shape on a third backend.

## Fix Design

Three small, independent changes, each modelled on an existing sibling in the
same file.

GATE-03 is one line: `emit_app_term_off` gains the `emit_gtk_term_active_gate`
call its siblings already have.

GATE-02 is mechanical but must be uniform: add a Windows
`emit_win_term_active_gate` (load `ARENA_STATE_REGISTER +
term_state_offset + TERM_STATE_ACTIVE_OFFSET`, compare to zero, branch to the
body's existing done label) and call it as the first instruction of every Windows
`term::` body except `on`, `isOn` and `didResize`. The memDC null test stays — it
is still the right guard for "the surface was never created" — but it is no
longer load-bearing as the mode gate. Note `emit_term_move_to` is a frameless
leaf and its done label must be introduced along with the gate.

GATE-01 is the only one needing an error path: `emit_term_size` must emit an
`ErrUnsupported` raise on the inactive branch. The macOS
`emit_app_terminal_size` arm is the shape to copy — it takes the code and message
symbol from `runtime_error_emission("ErrUnsupported")` and loads them into the
shared `RESULT_*` registers — and that helper is arch-neutral.

Rejected: gating centrally in `gen_shared::lower_term_helper` by prepending an
active test to every app-mode body. It cannot distinguish the three ungated
members from the rest without a name list that would then be a second source of
truth beside each backend's dispatcher, and it would double-gate the console and
macOS bodies that already test the slot.

Expected generated-output shift: the `windows-x86_64.app.ncodesum` golden on
`tests/syntax/app/macos-app-mode-term`, and any Linux GTK app golden that exists
by then. Confirm the delta is only the added gate instructions.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add the reproduction as an end-to-end case in `scripts/test-winapp.sh`
      (2230) and the Linux app harness (2228), asserting the `GATE-01 ok` line
      and that nothing paints after `term::off`. Confirm it fails today.
- [ ] Add a console rt-behavior fixture pinning the same three behaviours, so the
      oracle is a committed test rather than a reading of the emitters.
- [ ] Walk every arm of all four `emit_app_term_helper`s and record, in this
      file, which members gate on what — the table above is the audit's output.

Acceptance: the end-to-end cases fail for the documented reasons; the per-member
gate table is complete for all four backends.
Commit: —

### Phase 2 — the fix

- [ ] GATE-03: add `emit_gtk_term_active_gate` to
      `src/target/linux_gtk/app_io.rs:emit_app_term_off`.
- [ ] GATE-02: add `emit_win_term_active_gate` and call it first in every
      Windows `term::` body except `on`, `isOn`, `didResize`; give
      `emit_term_move_to` a done label.
- [ ] GATE-01: emit the `ErrUnsupported` raise on
      `emit_term_size`'s inactive branch, following
      `src/target/macos_aarch64/app/app_io.rs:emit_app_terminal_size`.

Acceptance: the Phase 1 cases pass on 2230 and 2228; the console fixture is
unchanged; the three ungated members still answer while off.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/regen-ncodesum.sh`; confirm the delta is only the added gate
      instructions in the app bodies.
- [ ] Delete gap 3 from the `mfb man term` overview, the app-mode caveat on
      `mfb man term off`, the "both counts" Windows exception on
      `mfb man term terminalSize`, and the standard-gated-sentence caveat added
      to 14 member pages by `fc1860141`; correct the spec's opening paragraph.
- [ ] `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
      `scripts/artifact-gate.sh all`, `scripts/man-census.sh --fill term`.
- [ ] Re-run the reproduction on 2230 and 2228.

Acceptance: full suite green; the docs state the gate as a promise again rather
than disclosing an exception; the reproduction passes on every backend.
Commit: —

## Validation Plan

- Regression test(s): a console rt-behavior fixture for the three behaviours
  (the oracle) plus the app-mode end-to-end cases on 2230 and 2228.
- Runtime proof: the reproduction on 2230 and 2228 — GATE-02 and GATE-03 are
  about what reaches a live surface and cannot be observed from a `.ncode` dump.
- Doc sync: `mfb man term` gap 3; `mfb man term off`;
  `mfb man term terminalSize`; the gated-sentence caveat on 14 member pages;
  `src/docs/spec/app/04_term-backend.md`'s opening paragraph.
- Full suite: `cargo test --release --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`.

## Open Decisions

- GATE-01 changes Windows app mode's `term::terminalSize` from
  always-returning to sometimes-raising. That is the documented contract and the
  behaviour of every other backend, but it can turn a working Windows app program
  into one that traps if it calls `terminalSize` outside TUI mode. Recommended:
  make the change and note it — the raise is the contract, and a program relying
  on the current answer is relying on a documented gap.
- Whether to land GATE-03 alone first. It is one line and independent of the
  Windows work; splitting it lets the GTK half close without waiting on 2230.

## Summary

No new mechanism is needed — all four backends already maintain the shared
`active` slot correctly, and two of them already consult it on every member. The
work is making the Windows bodies consult that slot instead of a resource handle
whose lifetime does not match TUI mode, and giving GTK's `term::off` the gate its
siblings have. The risk is in uniformity, not difficulty: a partially-gated
Windows backend leaves the module rule half-true, which is the state this bug
records. The `ErrUnsupported` raise in GATE-01 is the only piece with a real
error path, and macOS already has the shape to copy.
