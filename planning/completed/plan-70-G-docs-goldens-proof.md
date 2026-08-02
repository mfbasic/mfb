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

- [x] Updated the 3 spec files: `01_tables-and-algorithms.md` (flags charwidth bits
      4–5 + `AMBIGUOUS` bit 6 in the PackedProperty table + how the runtime reads
      `(flags>>4)&3`); `02_strings-model.md` (a "Display columns" measure alongside
      scalar/grapheme/byte); `04_term-backend.md` (shared cell model width byte +
      `WIDE_TRAIL` + 64 B EGC pool; console `POOL_BYTES_PER_CELL`/`OUTBUF_PER_CELL`=136;
      macOS plan-70-D + Linux plan-70-E Pango subsections; a NEW Windows GDI
      immediate-mode section). The man pages were already synced by A (new
      `displayWidth.md`) and C (wide-glyph notes on `drawText.md`/`drawGlyph.md`). The
      man + spec citation tests (`spec_citations_resolve`, `spec_links_resolve`,
      `man_citations_resolve`) pass.

Acceptance: citation tests green; docs describe the shipped model. Commit: e13c7ae03

### Phase 2 — golden regeneration

- [x] Ran the full `scripts/artifact-gate.sh target/release/mfb all` (1128 tests,
      1523 goldens) → exactly **3 diffs**, all the `windows-x86_64.app` `.ncodesum`
      goldens of `macos-app-mode-{io,plumbing,term}`. Root cause: plan-70-F grew the
      Windows io-write helper FRAME `0x88→0x98` (UTF-16 decode slots), changing every
      app's Windows codegen, plus the term fixture's new font/width/draw-helper
      codegen — all intentional. Regenerated from the current release binary and
      verified byte-for-byte; no other golden moved (A had already regenerated every
      table-embedding fixture, B–E per-letter).

Acceptance: the only gate diffs were the 3 attributed Windows goldens; regenerated,
diff reviewed and intentional. Commit: a9fda8ba1

### Phase 3 — cross-platform runtime proof

- [x] Added `examples/wide-demo` (a `drawBox` around CJK / astral emoji / NFD
      combining / ZWJ-family content in a 24-column inner field) with a built-in
      `strings::displayWidth` self-check. Ran it on: macOS host **console** — box
      borders align (right `│` at column 26 on every row) and the self-check is
      correct (`日本語 中文 한국어`=18, `cafe`+combining=4, ZWJ family=2); macOS app
      (headless, clean + correct width output); Linux GTK (2226, Xvfb — clean);
      Windows (2230 — RC=0). The deterministic width proof (console alignment +
      self-check) is autonomous; the GUI pixel proof on each platform is the
      human-convergence step (headless GUI does not composite — established D–F).
- [x] The `browser` example builds clean for all three targets (macOS `.out`, Linux
      AppImage, Windows `.exe`) after its package prep — no plan-70 codegen
      regression (its only build error in isolation is the pre-existing missing-
      sibling-package issue, unrelated to term/unicode).

Acceptance: `wide-demo` borders stay aligned around wide content (console-verified
on the macOS host; runs clean on all three); full `cargo test` + artifact-gate green;
`browser` unregressed. Commit: 2590c3e73

## Validation Plan

- Tests: citation tests, acceptance TUI goldens, the `wide-demo` fixture.
- Coverage check: the `wide-demo` fixture exercises width==2 + a pooled cluster on
  each backend's render path.
- Runtime proof: `wide-demo` + `browser` on macOS host + Linux box + box 2230.
- Doc sync: the 3 spec files + man pages (this letter is the doc sync).
- Acceptance: `cargo test` + `scripts/artifact-gate.sh` + acceptance goldens.

## Corrections

- **2026-08-02 — the man pages were already synced by A + C; only the 3 spec files
  needed work.** `strings/displayWidth.md` shipped with plan-70-A and the wide-glyph
  notes on `term/drawText.md`/`drawGlyph.md` with plan-70-C, so G Phase 1 reduced to
  the spec prose (the flags table, the display-column measure, and the term-backend
  cell-model/pool/macOS/Linux/Windows sections). Man pages live under
  `src/docs/man/builtins/`, not `src/docs/man/`.

- **2026-08-02 — the gate's only drift was the 3 Windows app goldens, and the width
  `flags` churn never appeared (A already absorbed it).** The plan anticipated a
  broad `flags`-column shift across width-using fixtures; but plan-70-A's table change
  regenerated EVERY table-embedding byte-identity golden at the time, so B–F stayed
  green against those, and the full gate at G found only the 3
  `windows-x86_64.app.ncodesum` goldens that drifted from plan-70-F's Windows codegen
  (never re-generated for the Windows target during D's macOS-only regen). Regenerated;
  everything else was already current.

- **2026-08-02 — the pixel-level cross-platform GUI proof is the human-convergence
  step (consistent with D–F).** A headless GUI cannot composite: GTK4 needs a
  compositor/GL session (bare Xvfb stays black), a Windows GUI window over
  non-interactive ssh does not render to a capturable desktop, and the macOS app
  window does not composite in an automated session. So `wide-demo`'s pixel alignment
  is verified autonomously ONLY on the macOS host console (the escape stream shows the
  right border at a fixed column + a correct `displayWidth` self-check); the app-window
  rendering on each of the three platforms is the documented human verify loop this
  umbrella (and each of D/E/F) calls out. The autonomous bar met here: `wide-demo`
  builds + runs clean on all three, and `browser` builds clean on all three.

- **2026-08-02 — `wide-demo` is an EXAMPLE, not a golden fixture.** The plan said "add
  the wide-demo fixture + golden", but an `examples/` program has no golden harness,
  and its determinism is already proven by the built-in `strings::displayWidth`
  self-check (18/4/2) plus the console escape-stream alignment. The per-backend width
  RENDER paths are golden-covered by the `func_term_draw_wide_valid` /
  `func_term_cluster_pool_valid` rt-behavior fixtures (plan-70-C) and the app-mode
  `.ncodesum` goldens; a separate `wide-demo` golden would duplicate that without
  adding coverage.
