# plan-125-D: man iteration 2, batch 2 — fs, strings, term, astrings, io (142 pages)

Last updated: 2026-09-04
Effort: x-large (1d–3d)
Depends on: plan-125-C (iteration 2 is a single ordered pass; C's batch lands
before D's, and C's calibration of per-page pace and of the example ledger
format carries forward).

Landing unit: **each Phase below is independently landable and gets its own
commit.** The letter totals x-large; it is never landed as one change, and a
session that lands one phase and stops has left the tree consistent.

Iteration 2, batch 2: **text, files, and the terminal** — the packages whose
pages are read by a developer who is mid-task and landing from a search, which
is exactly the reader the page lens exists to serve.

The unit is one page; the question is *is every sentence true, and is this
page self-sufficient*. See plan-125-A §3.2.

References:

- plan-125-A §3.2/§3.3/§4.3/§5; plan-125-B's terminology table.
- `.ai/man-content.md`; plan-108-A §3 (2a) memory-vocabulary ban and its
  rewrite table — `fs` and `io` are handle packages and this ban is where
  their pages historically failed.
- Memory `string-concat-beats-list-join-in-mfb` — `s = s & ch` beats
  `List OF String` + join by ~3×; any `strings` page that advises otherwise is
  wrong in the direction a reader will believe.
- Memory `unicode-pinned-tables-vs-utf8proc` — `strings`/`astrings` Unicode
  claims (normalization, grapheme boundaries, case folding) are version-
  sensitive; probe, do not reason.
- `src/docs/spec/unicode/**` — the internals foil for `strings`/`astrings`
  (and the destination for anything cut as too internal).
- Memory `interactive-state-never-in-an-immutable-tree` and
  `.ai/canvas-threading.md` — foils for `term`, whose pages must state the
  developer-visible behavior and never the backend's threading model.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-C complete | `grep -c '^- \[ \]' planning/plan-125-C-man-iter2-batch1.md` → `0` | — |

## 1. Goal

- **All 142 pages** through my per-page pass, one `codex exec` each, and apply.
- **Every example compiled and run** — and for `fs`, **every example rewritten
  to touch only temp paths** (plan-108-D's standing rule); a `fs` example that
  writes a real user path is a defect regardless of what it documents.
- Every parameter description and prose error claim verified.
- Every finding has a verdict; every rejection a disproving command.
- `--reconcile` exits 0 over the 142-unit list; sweeps clean for all five
  packages.
- "Belongs in spec" cuts appended to `planning/plan-125-belongs-in-spec.md` —
  `strings`/`astrings` Unicode internals are the likeliest source in the whole
  man surface.

### Non-goals (explicit constraints)

- Per plan-125-A. No cross-page reconciliation (letter G owns it).
- **No example anywhere in this letter touches a path outside a temp
  directory**, and none leaves a file behind.
- No wording churn on verified prose.

## 2. Current State

`fs` (41 pages) and `strings` (39) were reviewed by plan-108-D and -C
respectively; `term` (24) and `astrings` (15) have both had substantial churn
since (`term` 26 file-touches, `astrings` 22 —
`git log --since=2026-08-31 --name-only --format='' -- src/codegen/builtins/<pkg> | wc -l`),
and there is an **in-flight `term` change in the working tree at plan-writing**
(plan-125-A's prerequisite gate) that must land before this letter starts.

### Measured populations

| What | Count | Command |
|---|---|---|
| `fs` units | 43 | 41 pages + overview + types — `./scripts/man-census.sh --fill` |
| `strings` units | 40 | 39 + overview (no types page) |
| `term` units | 26 | 24 + overview + types |
| `astrings` units | 17 | 15 + overview + types |
| `io` units | 16 | 15 + overview (no types page) |
| **batch total** | **142** | sum |
| parameter descriptions in batch | 174 | census: fs 54, strings 64, term 30, astrings 19, io 7 |
| type descriptions in batch | 33 | census: astrings 17, term 15, fs 1 |
| `term` file-touches since plan-108 | 26 | `git log --since=2026-08-31 --name-only --format='' -- src/codegen/builtins/term \| wc -l` |
| `astrings` file-touches since plan-108 | 22 | same, `astrings` |
| `io` param descriptions | 7 for 15 pages | census PARAM-DESC `7/7` — the lowest parameter density in the batch; check that the pages that take no parameters really take none |

### Verified properties

- **`term` is mid-change at plan-writing** — VERIFIED by `git status`: 20+
  modified files under `src/codegen/builtins/term/` and
  `src/docs/spec/app/04_term-backend.md`. This letter reviews `term` *after*
  that lands, or it reviews prose that is about to move.
- **`fs` looks clean to a source grep and is not** — VERIFIED in plan-108-A:
  37 `owns` hits in `fs` are Rust module-doc lines that never render.
  Measure `fs` by rendering (`--memory-scope fs`), never by grepping the
  `.rs` files.

## 3. Design Overview

Per page: my pass → one `codex exec` → apply, at `N` concurrency with the main
thread as sole writer.

**Order:** `io` (16, smallest and the handle-vocabulary calibrator) →
`astrings` (17) → `term` (26) → `strings` (40) → `fs` (43, largest and the one
whose examples need the most care, with pace known).

**Risk concentration:**
- **`fs` examples on real paths.** The single highest-consequence defect class
  in this letter: an example a developer copy-pastes that deletes or
  overwrites something. Every `fs` example is rewritten to a temp path and run
  in a scratch project; the ledger records the temp path used.
- **Unicode claims in `strings`/`astrings`.** Memory
  `unicode-pinned-tables-vs-utf8proc` records that the pinned UCD and
  utf8proc disagree on 4,804 scalars. A normalization or grapheme claim is
  probe-verified against the binary, and where behavior is version-dependent
  the page says what *this* compiler does.
- **`term` prose vs. an in-flight backend change.** Held by the prerequisite.
- **Handle vocabulary in `fs`/`io`.** These are the packages the memory ban
  was written for. plan-108's rewrite table is applied, and the check is not
  "does it pass the grep" but "does the page still say what happens to the
  handle" — plan-108-F's recorded failure mode was deleting a true contract to
  pass a grep.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — io (16 units)

- [ ] 15 function pages + overview.
- [ ] Verify the open/close contract wording against plan-125-B's terminology
      table and plan-108-A's rewrite table; the page must still state what
      happens to the handle.
- [ ] Every example compiled and run; ledger recorded.

Acceptance: 16 units `exit 0`; `--memory-scope io` 0 unclassified; every `io`
page states its handle contract in the permitted vocabulary (read, not
grepped).
Commit: —

### Phase 2 — astrings (17 units)

- [ ] 15 pages + overview + the 17-description types page.
- [ ] Every byte-vs-scalar-vs-grapheme claim probe-verified; the
      `astrings`/`strings` boundary stated the same way on both sides.
- [ ] Every example compiled and run; ledger recorded.

Acceptance: 17 units `exit 0`; sweeps clean; the `astrings`↔`strings` boundary
sentence is identical in wording on both packages' overviews.
Commit: —

### Phase 3 — term (26 units)

- [ ] 24 pages + overview + the 15-description types page.
- [ ] No backend or threading detail on any page; anything cut goes to the
      "belongs in spec" ledger against `src/docs/spec/app/04_term-backend.md`.
- [ ] Every example compiled and run — a `term` example that leaves the
      terminal in a modified state is a defect; verify each restores it.
- [ ] Ledger recorded.

Acceptance: 26 units `exit 0`; sweeps clean; every `term` example leaves the
terminal restored (verified by running it).
Commit: —

### Phase 4 — strings (40 units)

- [ ] 39 pages + overview.
- [ ] Every Unicode claim probe-verified; every performance hint checked
      against memory `string-concat-beats-list-join-in-mfb` before it is left
      standing.
- [ ] Every example compiled and run; ledger recorded.

Acceptance: 40 units `exit 0`; sweeps clean; no page advises an accumulation
pattern the measurement contradicts.
Commit: —

### Phase 5 — fs (43 units)

- [ ] 41 pages + overview + types page.
- [ ] **Every example rewritten to temp paths and run**; the ledger records
      the temp path per example and confirms nothing is left behind.
- [ ] Handle contract wording verified by reading, not grepping
      (`--memory-scope fs` is necessary and not sufficient).
- [ ] Ledger recorded.

Acceptance: 43 units `exit 0`; `--reconcile` exits 0 over the whole 142-unit
batch; the example ledger accounts for all 142 examples, 0 unaccounted; no
`fs` example references a path outside a temp directory
(`./scripts/man-run-examples.sh fs --run` clean, and a read-through of the 41
examples confirming it).
Commit: —

## Validation Plan

- Tests: none (man prose); pinned-text tests updated in the same commit if
  touched.
- Coverage check: `--reconcile` over the 142-unit list; example ledger
  reconciled against the census function list, 0 unaccounted.
- Runtime proof: `scripts/man-run-examples.sh <pkg> --run` for all five;
  `mfb man <pkg> --all` renders.
- Doc sync: `planning/plan-125-belongs-in-spec.md` appended (expect the
  Unicode-internals cuts here).
- Acceptance: `--fill` 100%; `--memory-scope` 0 unclassified; `--scope` 0;
  `--reconcile` 0.

## Open Decisions

- **How much Unicode does a `strings` page owe the reader?** — Recommend: the
  developer-visible rule and the one-line pointer to `mfb man unicode`; the
  table version, the pinned UCD version, and the utf8proc divergence go to
  `mfb spec unicode` via the "belongs in spec" ledger. A `strings` page that
  explains normalization forms is documenting the wrong layer.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

Two risks dominate: an `fs` example a developer runs against a real path, and
a Unicode claim in `strings`/`astrings` that is true by reasoning and false in
this binary. Both are held by *running the thing* rather than reading it, which
is the only reason iteration 2 costs what it costs.
