# plan-35-F: Migration, validation, and docs

Last updated: 2026-07-11
Effort: medium (1h–2h)
Depends on: plan-35-B, plan-35-C, plan-35-D, plan-35-E

Close out the feature: migrate the existing TUI examples to the mandatory-present
model (add `term::sync()` calls — D1), run the full test/acceptance matrix across
all four targets and both `-app` backends, and finish the spec/man sync. After F,
`examples/life` runs flicker-free on the console and both app backends, the
`func_term_*` suite covers grid-write/diff/resize/flush, and the docs describe the
shadow-grid + mandatory-present model end to end.

Shared design of record: `planning/plan-35-shadow-grid-unify.md` (Validation Plan,
Compatibility). Depends on all rendering work (B/C console, D macOS, E GTK) being
landed.

## 1. Goal

- Every bundled `term::` program calls `term::sync()` at the right frame boundary
  and renders correctly on console + macOS app + GTK app.
- The `func_term_*` suite asserts: grid write/wrap/scroll/clamp, `sync` no-op
  while off, minimal-diff output under PTY capture, resize full-repaint, and
  `off` final-frame flush.
- Byte-gate green (all TUI-off programs byte-identical); determinism across the
  four targets; both `-app` builds pass.
- `mfb spec app term-backend` / `console-io` and `mfb man term` fully describe
  the model, including the mandatory-present footgun.

### Non-goals

- No new rendering behavior — F only migrates callers, tests, and docs.

## 2. Current State

`examples/life/src/main.mfb` draws each frame via `term::moveTo`+`io::write`
(`drawGrid:208`, `drawStatus:225`) and has **no `term::sync()` call** — it relied
on immediate rendering, which B/C/D/E removed (D1: mandatory present). It ships an
inline-TRAP workaround for bug-148 (see memory `bug-148 loop-trap`) and reads keys
via `io::readChar` (bug-149/150 context). Validation infra: `scripts/artifact-gate.sh`
(execution-free byte-gate, ~5min), `scripts/test-accept.sh`, `scripts/sync-goldens.sh`,
`scripts/update_man.sh` / `update_man_package.sh`; cross-target hosts via ssh
(x86, riscv on port 2229 — see `.ai/remote_systems.md`); both `-app` backends
(macOS local, GTK on the linux box).

## 3. Design

Mechanical migration + verification. The one judgment call: **where `life` calls
`sync()`.** Place it once per loop iteration after `drawGrid` + `drawStatus` and
before the input read (`readKey`), so each frame is composed fully then presented
once — the canonical retained-mode shape. `term::clear()` on `dirty` stays (it
blanks the back buffer; the next `sync` diffs from the cleared state). No other
`life` logic changes.

## Phases

### Phase 1 — migrate examples

- [ ] `examples/life/src/main.mfb`: add `term::sync()` once per frame (after
      `drawStatus`, before `readKey`). Verify the `dirty`/`clear` path still
      composes correctly against the diff.
- [ ] Grep the tree for any other `term::on` user (examples/tests) and add
      `sync()` where a frame is drawn.

Acceptance: `examples/life` runs flicker-free on the console; a steady frame
emits O(changed cells) (captured escape stream). Commit: —

### Phase 2 — test matrix + docs

- [ ] Consolidate/verify `func_term_*`: grid-write, draw-attrs, `sync` no-op,
      `func_term_diff_minimal`, resize full-repaint, `off`-flush.
- [ ] Run `scripts/artifact-gate.sh` (byte-gate) + `scripts/test-accept.sh` on
      host; cross-target build/run on x86 + riscv (ssh 2229); `mfb build -app`
      run of `life` on macOS and the linux GTK box.
- [ ] Final doc sync: `src/docs/spec/app/04_term-backend.md`,
      `03_console-io.md`, `src/docs/man/builtins/term/{sync.txt,package.md}` —
      describe the shadow grid, mandatory present, and per-backend present.
- [ ] `scripts/sync-goldens.sh` if any golden shifted (TUI-on behavioral goldens
      only — TUI-off must not shift).

Acceptance: all acceptance suites green on all four targets and both app
backends; byte-gate confirms TUI-off unchanged; docs render complete. Commit: —

## Validation Plan

- Tests: full `func_term_*` + `tests/syntax/term/*`.
- Runtime proof: `examples/life` on console (no flicker, minimal diff), macOS
  app, and GTK app (no tearing, resize reflow).
- Byte-gate: `scripts/artifact-gate.sh`; determinism across the four targets
  (bug-87).
- Acceptance: `scripts/test-accept.sh` + cross-target ssh (x86, riscv 2229) +
  both `-app` builds.
- Doc sync: spec `04_term-backend.md` + `03_console-io.md`; man `sync.txt` +
  `package.md`.

## Summary

Feature close-out: migrate `life` (and any other TUI caller) to the mandatory
`sync()`, prove the whole model on all four targets and both app backends, and
finish the docs. No new behavior — the risk is only catching a caller that drew
without presenting, which the flicker/blank runtime proof surfaces immediately.
