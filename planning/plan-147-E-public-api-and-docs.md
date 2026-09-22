# plan-147-E: Docs and the final gate

Last updated: 2026-09-21
Effort: medium (1h–2h)
Depends on: plan-147-D

Prerequisites: see plan-147-A. Additionally plan-147-D is archived
(`ls planning/completed/plan-147-D-*` → one file). If not, this letter cannot start,
full stop.

Documents the two public members (added in plan-147-B on top of `canvas::systemFaces`
and the `canvas::loadFont(path, face)` overload from A), rewrites the "no font
discovery" contract, and runs the feature's one full-suite gate. Checkable outcome: on this Mac,
`canvas::listSystemFonts()` contains `"Helvetica"` and is sorted with no duplicates, and
`canvas::loadSystemFont("Helvetica")` draws text; `loadSystemFont("No Such Font")`
fails `ErrNotFound` (77050004).

## 1. Goal

- `canvas::listSystemFonts() AS List OF String` — the `name` of every `systemFaces`
  entry, sorted (byte order, the `collections` sort), duplicates removed.
- `canvas::loadSystemFont(name AS String) AS canvas::Font` — among `systemFaces`
  entries with `name = name` (exact, case-sensitive), the first after sorting by
  `(path, postScript)` — deterministic when two files carry the same full name — is
  loaded with `canvas::loadFont(path, postScript)` (or `name` when the OS gave no
  PostScript name). None →
  `FAIL error(77050004, "no system font named: " & name)`. Errors: `ErrNotFound`,
  `ErrBadFontFile`, `ErrOutOfMemory`, and whatever `fs::readBytes` raises
  (`ErrPathNotFound` if the file vanished between listing and loading).
- Both are `Body::mfb` in a new `func_system_fonts.rs`; no new native code.
- No `Mode.Canvas` check (like `loadFont`).

### Non-goals

- No fuzzy/family matching, no fallback font, no default font.
- Text goldens keep using `loadFont` with fixture fonts; system fonts are never used in
  an exact-match golden.

## 2. Current State

- `loadFont`'s `DESC` (`func_load_font.rs`) ends: "There is no font *discovery*: `path`
  names a file … which is what makes text goldens exact-match." — replaced here.
- `src/docs/spec/app/06_canvas.md` has no font-discovery text
  (`grep -n -i 'discovery\|system font' src/docs/spec/app/06_canvas.md` → nothing).
- `ErrNotFound` = 77050004 (`src/codegen/errorcode/mod.rs`, md row) — no new code.

## Phases

> **NOTE — keep the checkboxes current as you go. An unticked box means NOT DONE.**

### Phase 1 — Members

- [x] ~~`func_system_fonts.rs`: both members, and their tests~~ — moot here: moved into
      plan-147-B (its Phase 1 tasks and Corrections), because the internal backend is
      unreachable from a test program without them.
- [x] Man prose for `listSystemFonts` / `loadSystemFont` finished against
      `.ai/man-content.md`: the list depends on the machine; read once per run; empty
      list without fontconfig; text in a system font can look different on another
      machine; `loadFont` is the reproducible path; every raisable error stated
      (`ErrNotFound`, `ErrBadFontFile`, `ErrPathNotFound`); no memory vocabulary. The
      `loadSystemFont` example no longer names `Helvetica` (macOS-only) — it loads the
      first listed name, and a second example shows `ErrNotFound`.
- [x] `loadFont`'s Errors table lacked `ErrPathNotFound`, which it raises for a
      missing path (`rt_canvas_font.rs` `a_missing_path_is_not_reported_as_a_bad_font`
      asserts 77030001) — added to both overloads and named in the prose.

Acceptance: Check: `scripts/man-census.sh --memory-scope` → 0 unclassified;
`mfb man canvas loadSystemFont` renders (est. 2 min).
  **Observed:** `unclassified memory-vocabulary hits: 0`; the page renders the
  declaration, parameter, description and a four-row Errors table.
Commit: e1194b769

### Phase 2 — Docs

- [x] `func_load_font.rs` `DESC`: replace the "no font discovery" paragraph — `loadFont`
      is the reproducible path (same file, same pixels everywhere);
      `loadSystemFont` trades that for convenience.
- [x] `06_canvas.md`: section *System fonts are an OS query, then an ordinary load* —
      the one-`String` table, per-thread caching, per-OS backends and filters (glyf,
      not variable, not simulated, not a named instance, not a private `.` face), face
      resolution by PostScript name, empty list without fontconfig, the companion
      `collections` rule; `[[path:Symbol]]` provenance. `bash scripts/spec-census.sh
      --citations` → `MISS-SYMBOL 0`.
- [x] Verify: `mfb man canvas listSystemFonts`, `mfb man canvas loadSystemFont`,
      `scripts/man-census.sh --memory-scope` → 0 unclassified,
      `scripts/man-run-examples.sh canvas --run` → examples compile and run.
      Observed: `examples: 28 built: 28 ran: 28 failed: 0`; after the example rewrite,
      `… canvas --run loadSystemFont listSystemFonts loadFont` → `4 built, 4 ran,
      0 failed` (`not installed: 77050004`).

Acceptance: Check: `cargo test --bin mfb spec` → pass; the man renders above show the
new pages (est. 5 min).
  **Observed:** `cargo test --bin mfb spec` → `43 passed; 0 failed`.
Commit: e1194b769

### Phase 3 — Final gate and landing

- [x] `cargo test --no-fail-fast > /tmp/p147_full.log 2>&1; echo EXIT=$?` → `EXIT=0`,
      `grep -c '^failures:'` → 0 (includes `artifact_gate_all`, the full cross-target
      golden sweep). Observed on the tree with main merged in (`/tmp/p147_full2.log`):
      `EXIT=0`, `failures:` 0, 216 `test result: ok`, 0 FAILED; unit tests `4287
      passed; 0 failed`; `artifact-gate [all]: 1480 tests, 1655 build(s), 2092
      golden(s) checked, 0 diff(s)`. (The first run, before the fix recorded in
      Corrections, had 1 failure.)
- [x] `cargo clippy --all-targets` → no warnings in files plan-147 touched. Observed:
      no diagnostic in any plan-147 file after the `rt_canvas_font.rs` fixture-builder
      fixes (`div_ceil`, `sort_by_key`). The run exits 101 on
      `rt_math_fixed_trig_accuracy`'s five `FRAC_PI_2` errors, which predate plan-147
      and are not in its files.
- [ ] Merge `worktree-system-fonts` into main from the main checkout (clean merge,
      main tree not entangled), then archive plan-147-E.

Commit: —

## Validation Plan

- Final gate (once, for all of plan-147): `cargo test` (full suite, est. per
  `.ai/testing-gates.md`) plus `cargo clippy --all-targets` → clean; C's and D's remote
  proofs recorded in their Corrections stand as the Linux/Windows runtime proof.
- Archive plan-147-A…E to `planning/completed/`.

## Open Decisions

- Name match exact and case-sensitive (recommended: the list is the vocabulary) vs.
  case-insensitive.

## Corrections

- **Final gate, first run: 1 failure, and it exposed a real leak.**
  `every_string_returning_runtime_helper_is_marked_fresh` (bug-576 audit) — the new
  `canvas.systemFontTable` returns a `String` and was not in `STRING_RESULT_HELPERS`.
  Checking the audit's condition (caller-arena block, only pointer) found the macOS
  backend returned CoreFoundation's worst-case-sized conversion block with a shorter
  length stamped, but a String is freed as `byteLength + 9`
  (`builder_owned_cleanup.rs`, bug-560), so the spare bytes were orphaned on every
  drop. Fixed: convert into a scratch block, `memcpy` into an exact `length + 9`
  String, free the scratch at its own size. Linux and Windows already allocate
  exactly. Then added the call to the audit list. Re-checked:
  `cargo test --bin mfb raw_result_block_ownership` → 4 passed;
  `rt_canvas_system_fonts` → `system fonts: 563 loaded: 563`, 4 passed; macOS
  `.app.nplan` diff = exactly `+ _memcpy` for `systemFontTable`.
- **A too-broad `pkill -f artifact-gate.sh` was run** to stop this worktree's
  poisoned gate run; other sessions were running suites on the machine at the time,
  and whether one of theirs had an artifact gate in flight is unknown. Kill by pid.
- The members themselves and their tests moved to plan-147-B (see B's Corrections);
  this letter keeps the prose, the spec, and the final gate.

## Summary

Thin MFBASIC layer over A–D plus the documentation turn from "no discovery" to "two
ways to get a font, one reproducible".
