# plan-70-G: Docs/spec sync, golden regeneration, cross-platform runtime proof

Last updated: 2026-07-27
Overall Effort (AI): huge (>3d)
Effort (Human): medium (1h–2h)
Effort (AI): medium (1h–2h)   — mechanical doc/golden work, but converges with Human on the acceptance runs + three hardware boxes
Depends on: plan-70-A, plan-70-B, plan-70-C, plan-70-D, plan-70-E, plan-70-F
Produces: the synchronized spec/man docs, the regenerated goldens (one reviewable
commit), and the end-to-end cross-platform proof that closes the feature.

This letter lands last, behind every other letter's tests, and holds the two
things that must not be scattered across letters: the **golden regeneration** (a
big generated-output churn that would bury per-letter review diffs) and the
**cross-platform runtime proof** (macOS host + Linux box + Windows box 2230).

## Prerequisites

See umbrella. G cannot start until A–F are landed and green individually — that is
the Depends-on edge. All three hardware targets must be reachable (`.ai/remote_systems.md`).

## 1. Goal

- Every spec/man doc the feature touched matches the shipped code.
- All goldens (artifact-gate `.ncodesum` for width-using fixtures; acceptance TUI
  goldens) are regenerated and the diff is reviewed as intentional.
- The `wide-demo` fixture renders aligned CJK/emoji/combining content in a
  bordered box on macOS, Linux, and Windows — the notcurses-parity proof.

### Non-goals

- No code behavior change beyond what A–F shipped; G is docs + goldens + proof
  only (any behavior bug found here is fixed in the owning letter, not patched in G).

## 2. Current State

- Docs to sync: `src/docs/spec/unicode/01_tables-and-algorithms.md` (PackedProperty
  flags table `:88-101` — add width bits 4–6), `02_strings-model.md` (add the
  display-column unit alongside scalar/grapheme/byte `:13-24`), and
  `04_term-backend.md` (shared cell model + console header block + macOS/Linux
  sections; **add a Windows app section — currently absent**). Man:
  `strings/displayWidth.md` (full page), and wide-glyph notes on
  `term/drawText.md`/`drawGlyph.md`.
- Golden mechanics: `scripts/artifact-gate.sh` reads `.ncodesum` byte-identity
  goldens; the width table changes the `flags` column so width-using fixtures'
  sums shift (see the fast-codegen-gate + acceptance-golden-harness conventions).
  Acceptance TUI goldens are seeded per the acceptance-golden-harness mechanics
  (RELEASE build; pre-created empty golden placeholders for new rt-behavior).
- The citation tests are strict: a file/symbol move must sweep `[[path:symbol]]`
  in both man (symbol-level) and spec (file-level).

### Measured populations

| What | Count | Command |
|---|---|---|
| Spec files to update | 3 | `01_tables-and-algorithms.md`, `02_strings-model.md`, `04_term-backend.md` |
| Man pages to add/update | ≥3 | `strings/displayWidth.md` (new), `term/drawText.md`, `term/drawGlyph.md` |
| Golden families that shift | **A already regenerated EVERY table-embedding byte-identity fixture** (the `flags` change ships in the unicode table embedded in essentially every binary — far wider than the plan's "width-using fixtures" estimate; see plan-70-A Corrections), so the gate stayed green through B–F; G re-measures only for *additional* shift from B–F's own codegen + the new acceptance TUI goldens | `scripts/artifact-gate.sh target/release/mfb all` diff after A–F; count families that move beyond A's regen |

## 3. Design

- Sync each spec/man doc to the shipped code; re-run the man + spec citation
  tests. Add the missing Windows app-backend spec section (F built the backend the
  spec never documented).
- Regenerate `.ncodesum` and acceptance goldens in one commit; confirm the delta
  is only the width `flags` column + the intended new TUI frames (not an unrelated
  drift) — mirror bug-388's "diffs are stale goldens, not a codegen bug"
  discipline.
- Author the `wide-demo` fixture (draws `"日本語 | 👨‍👩‍👧‍👦 | café"` in NFD inside a
  `drawBox`), add its acceptance golden, and run it on all three platforms.

## Phases

### Phase 1 — doc/spec + man sync

- [ ] Update the 3 spec files (flags bits, display-column unit, cell model + EGC
      pool + Windows section); add/update the man pages; re-run man + spec
      citation tests.

Acceptance: `cargo test` citation tests green; docs describe the shipped model.
Commit: —

### Phase 2 — golden regeneration

- [ ] Measure the golden families that shifted; regenerate `.ncodesum` +
      acceptance goldens; confirm the diff is intentional (width flags + new TUI
      frames only).

Acceptance: `scripts/artifact-gate.sh` diffs=0 after regen; the golden diff is
reviewed and attributed. Commit: —

### Phase 3 — cross-platform runtime proof

- [ ] Add the `wide-demo` fixture + golden; run it on the macOS host, a Linux box,
      and Windows box 2230; confirm aligned CJK/emoji/combining rendering (borders
      square around the wide content).
- [ ] Run the `browser` example on all three; confirm no regression.

Acceptance: `wide-demo` borders stay aligned around wide content on all three
platforms; full `cargo test` + artifact-gate + acceptance green. Commit: —

## Validation Plan

- Tests: citation tests, acceptance TUI goldens, the `wide-demo` fixture.
- Coverage check: the `wide-demo` fixture exercises width==2 + a pooled cluster on
  each backend's render path.
- Runtime proof: `wide-demo` + `browser` on macOS host + Linux box + box 2230.
- Doc sync: the 3 spec files + man pages (this letter is the doc sync).
- Acceptance: `cargo test` + `scripts/artifact-gate.sh` + acceptance goldens.

## Corrections

<Filled in during execution.>
