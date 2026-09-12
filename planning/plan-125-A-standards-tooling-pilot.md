# plan-125-A: the two audience standards, the spec tooling, the parallel review harness, and the pilot

Last updated: 2026-09-04
Overall Effort: huge (> 3d) — plan-125 spans A–N and drives 897 `codex exec` runs
Effort: large (3h–1d)
Depends on: nothing (first letter)

plan-108 (archived `49f665a23`, 2026-08-31) took every `mfb man` builtin page
from a half-filled state to authored + verified + cross-model-reviewed. It
worked. But it was **one pass**, it **excluded the narrative guide topics**,
and the surface has moved substantially since: **27 of the 31 renderable
packages have had commits under `src/codegen/builtins/` since it closed**
(`git log --since=2026-08-31 --name-only --format='' -- src/codegen/builtins |
grep -oE 'builtins/[a-zA-Z]+/' | sort -u | wc -l` → 27), `color` (28 function
pages) is an entirely new package that no plan-108 letter ever saw, `canvas`
(19) was missed by every letter of plan-108 (recorded in its own Corrections),
and `collections` grew from 24 pages to 49.

plan-125 re-runs the review, at greater depth, over **two surfaces with two
different audiences**, and it fixes the thing plan-108 never had: a
**three-iteration** structure where each iteration reads the material at a
different granularity, so a defect that is invisible at one zoom level is
caught at another.

- **`mfb man`** — for **the MFBASIC developer**: someone *using and learning
  the language*. Compiler internals must not leak here. This surface goes
  first (letters B–H), because verifying it discovers, by probe, what the
  compiler actually does.
- **`src/docs/spec/**`** (`mfb spec`) — for **the compiler contributor**, and
  for the developer who wants the internal detail. Internals are not merely
  permitted here, they are the *point*, and every non-obvious one carries a
  `[[path:Symbol]]` provenance citation. This surface goes second (I–N),
  consuming what B–H learned.

The two standards are deliberate mirror images, and that is the plan's most
useful property: **a sentence cut from a man page for being too internal is a
candidate spec obligation.** Letters B–H therefore emit a *"belongs in spec"*
ledger, and letters I–N consume it as a coverage checklist.

This letter builds everything the other thirteen run on: the two content
standards, the missing spec tooling (there is none today), the parallel
Codex fan-out harness, the eight reviewer prompts, and a pilot that takes one
man package, one guide topic, and one spec package through all three
iterations end to end to calibrate the cost per unit before A–N commit to it.

References:

- `planning/completed/plan-108-A-census-standard-pilot.md` §3 — the
  four-step per-package workflow (accuracy → scope → cross-model review →
  apply), the memory-vocabulary ban §3 (2a) with its rewrite table and two
  carve-outs, and the rejected alternatives. **plan-125 inherits all of it
  unchanged for the man surface**; this letter extends rather than replaces.
- `planning/completed/plan-108-F-certification-tooling-closeout.md` — the
  certification pattern: a certificate is a re-runnable measured sweep, not
  an assembly of ticked boxes.
- `.ai/man-content.md` (318 lines) — the man content standard. Lives; this
  letter extends it to the narrative topics and restates the audience.
- `.ai/specifications.md` — the spec's existing rules (single source of
  truth, `[[ ]]` provenance, `PACKAGE_ORDER`, the error-code registry as
  build input). The new `.ai/spec-content.md` is the *review* standard that
  sits on top of it, not a replacement.
- `src/cli/man.rs:render_all_markdown` — `mfb man --all` **deliberately skips
  `unqualified_global` packages** (`testing`, `general`) because they have no
  writable `IMPORT` spelling. It also renders no guide topic. See §4.1: the
  final gate in letter H cannot be raw `mfb man --all`.
- `src/docs/spec/mod.rs:PACKAGE_ORDER` — the 12 spec packages, in reading
  order.
- `src/docs/spec/diagnostics/02_error-codes.md` — **build input.** `build.rs`
  generates the `errorCode::` constants from its Constant Registry table and
  `src/codegen/builtins/errorcode/mod.rs:table_matches_registry` guards the
  drift. Any spec letter touching this file must run `cargo build` and that
  test; this is the one place the spec is not inert prose.
- `scripts/man-census.sh`, `scripts/man-run-examples.sh` — plan-108's
  instruments; extended here, not rewritten.
- Memory `doc-sync-means-man-and-spec`, `man-content-standard`,
  `plan-line-citations-decay-silently`, `completeness-claims-need-an-audit`,
  `example-harness-cwd-and-timeout`, `man-page-count-scrape-overcounts`,
  `diagnostic-harness-must-record-exit-and-unlocated-errors`,
  `subagent-edits-can-silently-vanish`, `peer-sessions-share-main-checkout`.

## Prerequisites

These are a precondition on the whole of plan-125, not a dependency to
negotiate. Letters B–N point here.

| Must be true | Command | Status |
|---|---|---|
| release `mfb` at current HEAD (every unit of every letter renders pages and compiles probes with it) | `cargo build --release`; `ls -l target/release/mfb` mtime ≥ HEAD commit date | **MET** 2026-09-12 — was stale (binary `Sep 8 07:36`, HEAD `b49ca1610` is `Sep 11 13:06`), rebuilt in the `P-125` worktree; binary `Sep 12 06:15`. |
| `codex` CLI installed and non-interactive | `~/local/bin/codex --version` → `codex-cli 0.153.0` | **MET** 2026-09-12 — still `codex-cli 0.153.0` (plan-108 ran `0.150.0`; not pinned — see plan-108-A Open Decisions, "do NOT pin") |
| the working tree is clean of unrelated in-flight work, or the in-flight work is on another branch | `git status --porcelain` | **MET** 2026-09-12 — `git status --porcelain` in the main checkout is empty; the `term` work that blocked this at plan-writing has landed. All plan-125 work is on `worktree-P-125`, forked from `main` at `b49ca1610`. |
| no peer session is mid-flight in `src/codegen/builtins/**` or `src/docs/spec/**` | `git worktree list` + `git -C <wt> status --porcelain` filtered to those paths, per memory `peer-sessions-share-main-checkout` | **MET** 2026-09-12 — swept all 16 live `.claude/worktrees/*`. Exactly one hit: `P-covdev` carries `src/codegen/builtins/canvas/mod.rs` **+2 lines** (`#[cfg(test)] mod tests_codegen;`) plus an untracked `tests_codegen.rs`, last touched `Sep 5 06:19`. That is a test-module declaration, not a prose field — disjoint from every edit plan-125 makes, and a week cold. No other worktree touches either surface. |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you.

## 1. Goal

- **`.ai/man-content.md` states the audience in one line and is extended to
  cover the narrative topics.** Today it governs registry prose only; the 10
  guide topics under `src/docs/man/**` (32 markdown pages, 3,924 lines —
  `find src/docs/man -name '*.md' | wc -l` → 32) are in scope for plan-125
  and need the same MUST/MUST-NOT list, the same memory-vocabulary ban, and
  the same example rules.
- **`.ai/spec-content.md` exists** — the contributor-audience review standard
  for `src/docs/spec/**`: what a spec topic must contain (the normative
  contract, stated precisely, with `[[ ]]` provenance), what it must not
  (tutorial prose, marketing, duplicated bodies, unverifiable claims,
  aspirational behavior), and the accuracy rule — **the spec describes the
  compiler as-is at HEAD, not as designed**.
- **`scripts/spec-census.sh` exists** with the modes in §4.2, and its
  `--citations` mode reproduces this letter's measured baseline: **61 of
  1,280 symbol citations do not resolve, 2 citations are malformed**
  (commands in §2). Citation rot is invisible today because nothing checks
  it.
- **`scripts/man-census.sh` covers the narrative topics** (`--topics`), so
  the man census denominator is the whole developer surface, not 31/41 of it.
- **`scripts/man-manual.sh` exists** and emits the *complete* developer
  manual as one deterministic artifact — `mfb man --all` plus the two
  `unqualified_global` packages it skips plus every guide topic. Letter H's
  gate reads this, not raw `mfb man --all` (§4.1).
- **`scripts/doc-review-fanout.sh` exists** (§4.3): N concurrent `codex exec`
  runs across N reusable detached worktrees, one unit per run, findings to
  one file per unit, plus a `manifest.tsv` recording exit status and findings
  count for **every** unit so a letter can prove zero unaccounted units.
- **The eight reviewer prompts are written verbatim into §5 of this file** —
  man iterations 1/2/3, man final; spec iterations 1/2/3, spec final — so
  every later letter runs the identical prompt and findings stay comparable
  across 897 runs.
- **The pilot is complete**: `color` (30 units: 28 function pages + overview +
  types — the package no plan-108 letter reviewed), the `variable` guide
  topic, and the `unicode` spec package (3 files, 508 lines) have each been
  through **all three iterations plus apply**, and this file records the
  measured wall-clock and findings-yield per unit for each iteration. Those
  numbers, not an estimate, size letters B–N.
- **The `mfb man --all` coverage hole is resolved** — measured, decided, and
  either fixed in the renderer or worked around by `scripts/man-manual.sh`,
  with the decision recorded here (§4.1 / Open Decisions).

### Non-goals (explicit constraints)

- **No compiler test gates for the man surface.** Prose fields are
  `&'static str` the compiler never reads; `artifact-gate`, the cargo suite
  and `test-accept` can neither catch a doc error nor fail on one. Do not run
  them for letters B–H. (Two exceptions, both narrow:
  `tests/cli/cli_man_summary_plain.rs` and `tests/cli/cli_canvas_man_examples_compile.rs`
  pin rendered text — if a letter changes text those tests pin, update the
  pin in the same commit.)
- **The spec surface is different and DOES have gates** (letters I–N):
  `cargo build` (the embedded table is generated), `cargo test --bin mfb spec`,
  and — only when `src/docs/spec/diagnostics/02_error-codes.md` changes —
  `cargo test errorcode` for `table_matches_registry`. Per memory
  `scope-the-test-run-to-the-blast-radius`, that is the blast radius; no full
  suite for prose.
- **Prose fields and markdown only.** Never touch a descriptor type, an error
  table, a byte-significant MFBASIC body (e.g.
  `src/codegen/builtins/collections/func_sort_by.rs:2` "Body byte-significant
  … do not reformat"), or any non-prose code. Every commit's `git diff` is
  checked to be string-literal / markdown changes only.
- **Exactly one renderer change is permitted in the whole of plan-125**, and
  only if §4.1 decides for it: rendering guide topics in `mfb man --all`.
  No other change to `src/cli/man.rs` or `src/docs/spec/mod.rs`; no registry
  schema change; no new descriptor fields.
- **No wording churn.** A page or topic that passes its iteration's lens is
  left byte-for-byte alone. This is an audit, not a rewrite.
- **The reviewer never commits.** Codex runs `workspace-write` inside a
  disposable worktree so it can build probes; every fix is applied by the
  main thread in the primary checkout. Per memory
  `subagent-edits-can-silently-vanish`, the harness runs
  `git -C <worktree> status --porcelain` after each unit and records any
  reviewer edit as a harness violation.
- **`planning/old_man/**` stays archived** and is never cited as an
  authoring surface; its `[[path:symbol]]` citations pre-date the plan-102/103
  code motion and are stale.
- Plan-125 does not renumber, reorder, or add spec packages/topics; it does
  not add a permanent example-running test harness (rejected by the user in
  plan-108 and still rejected).

## 2. Current State

### The man surface

`mfb man` renders builtin package pages straight from the clean-room registry
descriptors in `src/codegen/builtins/**` (`src/cli/man.rs:1-15`), and the 10
narrative guide topics from markdown under `src/docs/man/**`
(`src/docs/man/mod.rs`, embedded at build time). Declaration / Parameters /
Errors tables are derived from the same descriptors the compiler executes, so
those are correct by construction; **prose is the audit target**.

plan-108 left it fully filled and, at that time, clean.

### The spec surface

`src/docs/spec/**` is 12 packages of markdown embedded in the binary
(`src/docs/spec/mod.rs:PACKAGE_ORDER`), rendered by `mfb spec <pkg> [<topic>]
[--all]`. **There is no tooling of any kind**: `ls scripts/ | grep -i spec` →
no matches; `grep -rl 'docs/spec' scripts/ tests/` → no matches. Nothing
checks a citation, a cross-link, or a stale claim. `.ai/specifications.md`
states the rules; nothing measures compliance.

### Measured populations

All commands **re-run 2026-09-12 at HEAD `b49ca1610`** (Phase 1) with a
`target/release/mfb` rebuilt from it. The 2026-09-04 figures at `90f6c1357`
are kept in the right-hand column wherever they moved, because a drifting
denominator is itself a result: eight days and ~50 commits shifted the man
surface by +7 pages and the spec's broken-citation count by +1.

| What | Count (2026-09-12, `b49ca1610`) | Command | Was (2026-09-04, `90f6c1357`) |
|---|---|---|---|
| renderable man packages | **31** | `./scripts/man-census.sh --fill` → 31 package rows. 33 dirs under `src/codegen/builtins/` minus `perf` **and `tests`**, neither of which is an MFB package (`mfb man perf`, `mfb man tests` → `unknown package`) | 31 (32 dirs minus `perf`) |
| man function pages | **544** | `./scripts/man-census.sh --fill` → `TOTAL 544 544 544 544 903/903` | 538 |
| man parameter descriptions | **903** | same row | 884 |
| man package overview pages | 31 | one per renderable package (`PKGDOC` column `11` for all 31) | 31 |
| man `types` pages | **21** | `awk 'NR>2 && /^-----/{exit} NR>2 && $NF!="-"{n++} END{print n}'` over the `--fill` table | 20 |
| narrative guide topics | 10 | `ls src/docs/man/` minus `mod.rs` | 10 |
| narrative guide **pages** (topic + subtopics) | 32 | `find src/docs/man -name '*.md' \| wc -l`; corroborated by `./scripts/man-manual.sh --count` (flow 8, types 10, tour 6, tooling 2, six single-page topics) | 32 |
| narrative guide lines | **4,015** | `find src/docs/man -name '*.md' -exec cat {} + \| wc -l` | 3,924 |
| **man review units, iteration 1 / 3** | **41** | 31 packages + 10 topics | 41 |
| **man review units, iteration 2** | **628** | 544 function + 31 overview + 21 types + 32 guide pages; independently confirmed as the `═`-rule count of the complete manual (`./scripts/man-manual.sh --count` → `PAGES 628`) | 621 |
| complete manual artifact | **61,181 lines / 628 pages** | `./scripts/man-manual.sh --count` | — |
| function pages **missing** from `mfb man --all` | 30 | `testing` 12 + `general` 18 — `render_all_markdown` filters `is_unqualified_global()`; deliberate, kept (§4.1). `scripts/man-manual.sh` supplies them | 30 |
| guide pages missing from `mfb man --all` | **0** | the §4.1 decision was taken: `render_all_markdown` now appends every guide topic. `./scripts/man-manual.sh \| grep -cE '^(TESTING\|GENERAL\|A TOUR OF MFBASIC)$'` → 3 | 32 |
| man fill state | 100% | census: `pages with neither Description nor Examples: 0` | 100% |
| man memory-vocabulary hits, **registry packages only** | **0 unclassified** (15 carve-out 1, **40** carve-out 2) | `./scripts/man-census.sh --memory-scope <31 packages>` | 0 unclassified (15 / 23) |
| man internals-vocabulary hits, **registry packages only** | 0 | `./scripts/man-census.sh --scope <31 packages>` | 0 |
| man memory-vocabulary hits, **whole surface (incl. guide topics)** | **109 unclassified** | `./scripts/man-census.sh --memory-scope` (no args now sweeps the 10 topics too) — `tour` 45, `types` 39, `optimizations` 8, `link` 8, `lambda` 7, `errors` 2 | never measured: plan-108 excluded the topics |
| man internals-vocabulary hits, **whole surface (incl. guide topics)** | **9** (+27 carve-out 3) | `./scripts/man-census.sh --scope` → `internals-vocabulary hits: 9` — `optimizations` 5, `tour` 3, `types` 1 | never measured |
| spec packages | **12** | `./scripts/spec-census.sh --fill` → 12 rows, in `PACKAGE_ORDER` | 12 |
| spec files | **146** | `--fill` `TOTAL` row, `FILES` column | 146 |
| spec lines / words | **27,261 / 230,854** | `--fill` `TOTAL` row | 26,482 / 223,085 |
| **spec review units, iteration 1 / 3** | **12** | one per package | 12 |
| **spec review units, iteration 2** | **146** | one per file | 146 |
| spec `[[ ]]` citations (total / unique) | **2,021 / 1,456** | `--fill` `CITES` column; `--citations` `TOTAL unique=` | 1,970 / 1,414 |
| unique citations that are file/dir only (no suffix) | 115 | `--citations` → `nosuffix=115` | 115 |
| unique citations with a line-range suffix | 19, **0 out of range** | `--citations` → `line=19`, `MISS-LINE 0` | 19, 0 |
| unique citations with a symbol suffix | **1,322** | `--citations` → `symbol=1322` | 1,280 |
| **symbol citations whose symbol is not in the cited file** | **62** — **49 stale-by-move, 13 stale-by-deletion** | `--citations` → `MISS-SYMBOL 62  (stale-by-move 49, stale-by-deletion 13)` | 61 (split never measured) |
| citations with an unresolvable path | **2** (`finalize_vreg_body_with_locals`, `run_register_allocation` — malformed: symbol with no path, both on `src/docs/spec/memory/08_program-startup.md:228`) | `--citations` → `MISS-PATH 2` | 2 |
| spec code fences | **366** | `--fill` `FENCES` column | 363 |
| spec `mfb spec`/`mfb man` cross-links | **928** | `--fill` `LINKS` column | — |
| `mfb spec --all` (global) | **renders 0 lines** | `./target/release/mfb spec --all \| wc -l` → 0; only per-package `mfb spec <pkg> --all` works | 0 |
| **review units across the whole plan** | **876** | man 706 (A pilot 31 + B 39 + C–F 590→**597** + G 39) + spec 170. The +7 is iteration 2's man page growth; §"Corrections" records which letters absorb it | 869 |
| **total `codex exec` runs plan-125 will make** | **904** | 876 unit runs + 16 consistency runs (4 each in B, G, I, M) + 12 final-lens runs (6 in H, 6 in N) | 897 |

### Verified properties

- **`mfb man --all` was not the whole developer manual — now it is, bar 30
  pages by design.** VERIFIED at plan-writing by reading
  `src/cli/man.rs:render_all_markdown` (filters `is_unqualified_global()`) and
  by grep. **RESOLVED in Phase 1**: the §4.1 decision was taken and
  `render_all_markdown` now appends every guide topic in the index's sorted
  order, so the 32 guide pages are in `--all`. `testing` (12) and `general`
  (18) stay filtered for the documented reason and are supplied by
  `scripts/man-manual.sh`. The complete artifact is 628 pages / 61,181 lines,
  cross-checked three ways (census `544+31+21=596` registry pages; the
  artifact's own `═`-rule count `628`; per-topic renders summing to `32`).
- **The unresolved symbol citations are two distinct rot classes, and the
  split is now MEASURED, not spot-checked** — `./scripts/spec-census.sh
  --citations` → `MISS-SYMBOL 62  (stale-by-move 49, stale-by-deletion 13)`.
  The 13 stale-by-deletion citations are the **suspect-claim list** letters
  I–N triage as claims, not links; they are enumerated in §2.1 below. The
  original spot-check that motivated the split stands:
  `__http_dechunk` is cited as
  `src/codegen/builtins/http/mod.rs:__http_dechunk` but now lives in
  `helper_dechunk_bytes.rs` (`grep -rl '__http_dechunk'
  src/codegen/builtins/http/`) — **stale by move**, the package.mfb split
  described in memory `splitting-package-mfb-render-order-doc-asymmetry`.
  `lower_io_write_helper` appears **only in the spec**
  (`grep -rl 'lower_io_write_helper' src/` → `src/docs/spec/memory/07_runtime-helper-abi.md`
  alone) — **stale by deletion**, and the surrounding claim is therefore
  suspect, not just the citation. A citation fixer that only re-points paths
  would silently ratify class two.
- **The renderer omits empty prose sections** (`src/cli/man.rs` `is_empty()`
  gates) — carried forward from plan-108-A, still true; rendered output, not
  a source grep, is the honest census surface.
- **`perf` is not an MFB package** — VERIFIED: `mfb man perf` errors
  `unknown package`; `src/codegen/builtins/perf/perf.rs:1-6` says so. 31, not
  32, is the package denominator.
- **`tests` is not an MFB package either, and the census did not know that** —
  VERIFIED in Phase 1: `mfb man tests` → ``unknown package `tests` ``;
  `src/codegen/builtins/tests/` is a `#[cfg(test)]` Rust module tree
  (`abi_inline.rs`, `app_surface.rs`, …) added after `man-census.sh`'s filter
  was written. Before the fix the census printed a **32nd package row with
  `PKGDOC 00`** — a row that reads exactly like an unfilled overview and is
  really a directory with no man surface at all. Fixed in the same commit
  (`grep -vE '^(perf|tests)$'`), and the denominator is 31 again.
- **The `datetime` `borrow` hits are arithmetic, not memory** — VERIFIED by
  `--memory-scope` classification (15 CARVE-1 rows, all "borrows a whole
  second"). Carried forward from plan-108-A carve-out 1.
- **UNVERIFIED — whether any man page has *regressed* since plan-108.** The
  census says fill is 100% and the vocabulary sweeps are 0, but 27 packages
  changed and neither sweep can see a *false* sentence. That is precisely
  what letters B–H measure; no assumption either way is made here.
- **UNVERIFIED — spec accuracy at HEAD.** Nothing has ever checked it. The 61
  broken citations are the only measurable proxy and they are a lower bound:
  a claim can be wrong with a perfectly resolving citation.

### 2.1 The suspect-claim list — 13 stale-by-DELETION citations

Measured in Phase 1 by `./scripts/spec-census.sh --citations` (the
`ELSEWHERE=no` column). These are **not broken links**. The symbol each one
cites exists nowhere under `src/`, `build.rs` or `repository/src`, so the
thing the surrounding sentence describes has been deleted or renamed out of
existence — and the *claim*, not just the marker, is suspect. Re-pointing the
path would silently ratify a sentence about a compiler that no longer exists.

Letters I–N triage every row here as a **claim** (verify it still holds at
HEAD, then re-cite it; or cut it), never as a link fix.

| Spec site | Citation |
|---|---|
| `src/docs/spec/diagnostics/02_error-codes.md:161` | `[[build.rs:generate_errorcode_table]]` |
| `src/docs/spec/diagnostics/01_rule-codes.md:155` | `[[src/cli/dispatch.rs:exit_after_diagnostics]]` |
| `src/docs/spec/architecture/14_aarch64-instruction-set.md:247` | `[[src/arch/aarch64/encode/sizing.rs:wide_imm_word_count]]` |
| `src/docs/spec/memory/07_runtime-helper-abi.md:103` | `[[src/codegen/builtins/datetime/mod.rs:DATETIME_NOW_NANOS_SPEC]]` |
| `src/docs/spec/stdlib/02_datetime.md:143` | `[[src/codegen/builtins/datetime/mod.rs:NOW_NANOS]]` |
| `src/docs/spec/app/03_console-io.md:32` **and** `:153` | `[[src/codegen/builtins/io/func_read_byte.rs:lower_io_read_byte_helper]]` |
| `src/docs/spec/memory/07_runtime-helper-abi.md:80` | `[[src/codegen/builtins/io/func_write.rs:lower_io_write_helper]]` |
| `src/docs/spec/stdlib/04_json.md:66` | `[[src/codegen/builtins/json/mod.rs:is_json_value_type]]` |
| `src/docs/spec/memory/05_collections.md:249` **and** `09_closures.md:122` | `[[src/codegen/engine/types/type_utils.rs:is_function_type]]` |
| `src/docs/spec/language/04_types.md:109` | `[[src/codegen/error/emission/builder_error_emission.rs:emit_float_domain_return]]` |
| `src/docs/spec/threading/06_thread-runtime-helpers.md:34` | `[[src/codegen/runtime/thread/runtime_helpers.rs:lower_thread_helper]]` |
| `src/docs/spec/linker/06_macos-aarch64.md:56` | `[[src/os/macos/link/macho.rs:write_load_commands]]` |
| `src/docs/spec/app/02_linux-runtime.md:291` **and** `04_term-backend.md:682` | `[[src/target/linux_gtk/mod.rs:ARENA_REG]]` |

13 unique citations across 16 spec sites. The remaining **49** `MISS-SYMBOL`
rows are stale-by-move: the symbol is still in `src/`, at another path, and a
re-point is the correct repair (the `http` and `net` families dominate — the
`package.mfb` split recorded in memory
`splitting-package-mfb-render-order-doc-asymmetry`).

## 3. Design Overview

### 3.1 Two audiences, stated once

| | `mfb man` | `mfb spec` |
|---|---|---|
| Reader | a developer **using and learning MFBASIC** | a **compiler contributor**, or a developer who wants the internal detail |
| Answers | "what does this do, how do I call it, what goes wrong" | "what is the exact contract, and where in the compiler is it implemented" |
| Internals | **banned** — no IR, no lowering, no ABI, no codegen, no plan/bug numbers, no mangled symbols | **required**, and cited with `[[path:Symbol]]` |
| Memory words | the four permitted only: copy, mutate, value, alias (plan-108-A §3 (2a)) | the precise contract in its own vocabulary — the ban does **not** apply |
| Examples | runnable MFBASIC a developer would write; compiled and run during review | illustrative fragments; correctness of the *claim* outranks runnability |
| Voice | second person, task-first | normative, precise, no tutorial scaffolding |

**The bridge:** when a man page states something that only belongs in spec,
the man letter cuts it and writes one line into
`planning/plan-125-belongs-in-spec.md` (unit, the cut sentence, the spec
package it belongs to). Letters I–N open that ledger as a coverage checklist:
every entry is either already covered by a spec topic (record where) or is a
spec gap to fill. This is why the man surface goes first.

### 3.2 The three iterations — each a different lens

Three passes are only worth 3× the cost if each one *can see something the
others cannot*. They are therefore defined by granularity, not by repetition.

**Iteration 1 — the package as a unit.** The reviewer reads the whole
rendered package (`mfb man <pkg> --all` + `types`, or the whole guide topic
with its subtopics). It is the only iteration that can see:
*coverage* (a thing a developer needs that no page mentions), *internal
consistency* (siblings describing the same concept two ways), *the overview's
promises vs. what the functions deliver*, and *ordering / discoverability*.
It ends with a **cross-package consistency review** (§3.4) — the one review
that sees all 41 units at once.

**Iteration 2 — the page as a unit.** The reviewer is given **one page and no
siblings**. It is the only iteration that can afford, per page, to verify
*every sentence* against the implementation by reading the code and running
probe programs, to compile and run the example, and to check every parameter
description and error row. It is the depth pass, and it is 621 of the 709 man
runs for that reason.

**Iteration 3 — the package as a unit, again, after the surgery.**
Iteration 2 edits 621 pages *independently*; that reliably introduces
divergence (two pages now explain one concept differently), redundancy, and
broken cross-references. Iteration 3 is the **re-integration** pass and the
reviewer's first sight of each package in its final form. Its findings are
expected to be about *seams*, not facts — if iteration 3 returns many factual
findings, iteration 2 under-performed and that is itself a recorded result.

The spec surface runs the identical three lenses at package / file / package
granularity, with contributor-appropriate content (§5.5–5.7).

### 3.3 The per-unit workflow (inherited from plan-108-A §3, extended)

1. **My pass** — read the unit; check every claim against the implementation;
   fix or excise. Apply the audience standard (`.ai/man-content.md` or
   `.ai/spec-content.md`). For man: run `scripts/man-census.sh --memory-scope`
   and `--scope` on the package; compile and run every example. For spec:
   run `scripts/spec-census.sh --citations <pkg>` and resolve every hit.
2. **Cross-model review** — one `codex exec` per unit, from the iteration's
   verbatim prompt (§5), in a fan-out worktree.
3. **Apply** — triage on the main thread: confirmed → fix; rejected → record
   **with the disproving command** in the letter's ledger. Never apply a
   reviewer edit from the worktree; the reviewer's output is text.
4. **Re-measure** the unit and record before/after in the ledger.

### 3.4 The consistency reviews

Two kinds, and they are not the same thing:

- **Cross-package consistency (end of iteration 1, letters B and I).** One
  concept, one vocabulary, across the whole surface. Run as a small number of
  dimension-scoped runs over a *condensed* artifact (every overview + every
  `types` page, not every function page — that is the only way it fits) plus
  targeted greps for competing spellings of the same idea.
- **The final full-surface sweep (letters H and N).** Over the complete
  manual artifact, decomposed into **6 lenses** (§4.4), because no single run
  can read 51,540 lines usefully. Each lens is one run over the whole
  artifact with one question.

### 3.4b The split rule, and why seven letters are x-large

The write-plan split rule wants sub-plans at medium/large. Letters C–F and J–L
total x-large because their *unit counts* are large (142–159 man pages, 45–51
spec files), not because they are one indivisible change. **Each Phase inside
them is independently landable and carries its own `Commit:` line**, and a
phase is one package or one small group — the same size plan-108's letters
were. Splitting them further would produce 25 planning files that each restate
the same workflow and standard, which is worse for the implementer than seven
letters with four landable phases apiece. The pilot (Phase 5) re-checks this:
if measured per-unit cost is more than 2x the estimate, C–F and J–L are
re-batched before they start.

### 3.5 Where the risk concentrates

- **The 897-run accounting.** The single most likely failure of this plan is
  a unit that silently never ran and is counted as clean. Held by the
  harness's `manifest.tsv` recording exit status and findings count for every
  unit, by a per-letter reconciliation (`units listed == units in manifest ==
  units with a findings file`), and by memory
  `diagnostic-harness-must-record-exit-and-unlocated-errors`: a failed run
  must never read as "same".
- **Parallel worktrees writing the repo.** Held by: reviewers never commit,
  the main thread is the only writer, and a post-run
  `git -C <wt> status --porcelain` check per unit.
- **Iteration 2 fragmenting the surface.** 621 independent edits will
  introduce divergence. This is *expected* and is exactly what iteration 3
  exists to repair — it is not a reason to weaken iteration 2.
- **The spec's accuracy pass being unbounded.** Verifying a spec claim can
  mean reading a compiler pass. Held by: citation-first triage (a claim with
  a resolving symbol citation is checked *at that symbol*; a claim with none
  is either given one or cut), and by the rule below.
- **Found compiler bugs.** A spec/code disagreement is triaged, never
  averaged: *spec stale* → fix the spec; *code wrong* → per AGENTS.md the bug
  is not left, it goes through `write-bug` (small → fix now; large → a
  `bug-NN` document with a repro), recorded in the letter's ledger either
  way. A doc plan is allowed to find compiler bugs; it is not allowed to
  ignore them.
- **The Codex sandbox cannot bind sockets** (plan-108-C's recorded lesson).
  Any probe for `tcp`/`udp`/`tls`/`net`/`http` must be run by the main thread
  in the primary checkout; the prompts say so explicitly (§5).

### Rejected alternatives

- **Batch iteration 2 (~5 pages per run).** Rejected by the user: it would
  cut man iteration 2 from 621 runs to ~125, but per-page isolation is the
  entire mechanism of the depth pass — a reviewer holding five pages
  proofreads instead of verifying.
- **Run the man and spec tracks concurrently.** Rejected by the user: the
  surfaces overlap on the memory model, resources and stdlib semantics, and
  would be reviewed against each other's stale state, producing contradictory
  fixes to reconcile later. Man first also feeds the "belongs in spec"
  ledger.
- **Per-package interleave of the three iterations.** Rejected by the user:
  iteration 1's consistency review and iteration 3's re-integration lens both
  require the pass to be complete across the surface.
- **Use another Claude tier as the reviewer.** Rejected in plan-108-A and
  still rejected: the value is independence from Claude, which only a
  different vendor's model gives.
- **Pin the Codex model.** Rejected in plan-108-A ("do NOT pin"); the banner
  is recorded per letter instead. `0.153.0` today vs `0.150.0` in plan-108.
- **Add a permanent example-running test harness.** Rejected by the user in
  plan-108 and unchanged.
- **Make `mfb man --all` render `testing` and `general`.** Rejected: the
  filter is deliberate and documented (no writable `IMPORT` spelling), and
  advertising an unwritable spelling is a worse defect than the coverage
  hole. `scripts/man-manual.sh` covers them instead (§4.1).

## 4. Detailed Design

### 4.1 The complete-manual artifact, and the `mfb man --all` hole

The user's final gate is "one final review of `mfb man --all` as a final full
developer doc consistency check". Measured, `mfb man --all` is **missing 62 of
the 621 pages**: `testing` (12) and `general` (18) function pages by a
deliberate `is_unqualified_global()` filter, and all 32 guide pages because
`--all` walks the registry only.

Resolution, in two parts:

1. **`scripts/man-manual.sh`** emits the artifact letter H reviews:
   `mfb man --all`, then `mfb man testing --all`, `mfb man general --all`,
   then `mfb man <topic> --all` for each of the 10 topics, concatenated with
   the renderer's own rule separator, deterministic ordering, no timestamps.
   This is the complete developer manual and it is what H's six lenses read.
2. **Open Decision (below): should `mfb man --all` itself render the guide
   topics?** Recommendation: **yes** — a developer running `mfb man --all`
   reasonably expects the whole manual, and H's headline gate should be
   honest when a human runs it by hand. It is a contained change in
   `render_all_markdown` (append the topics after the packages, in the index's
   sorted order), it needs `tests/cli/cli_man_summary_plain.rs` re-checked, and it
   is the **only** renderer change plan-125 permits. `testing`/`general` stay
   filtered for the documented reason and remain script-only.

### 4.2 `scripts/spec-census.sh`

Modelled on `scripts/man-census.sh` (deterministic output, no timestamps, no
paths that vary, `LC_ALL` set — its header comments explain why). Modes:

- *(default)* **`--fill`** — per-package inventory: files, lines, words, code
  fences, citation count, cross-link count; a `TOTAL` row. The denominator
  every spec letter reconciles against.
- **`--citations [pkg…]`** — the instrument that does not exist today.
  For each unique `[[…]]`: split on the **first** `:` (Correction C-3 — the
  last-colon rule shreds a Rust symbol containing `::`); verify the path exists
  (file or directory); for a numeric suffix verify the line is within the
  file; for a symbol suffix `grep -F` the symbol in the cited file. Emit
  `OK` / `MISS-PATH` / `MISS-LINE` / `MISS-SYMBOL` with **the spec file and
  line the citation was written on**, so a finding is actionable. Must
  reproduce the baseline: `MISS-SYMBOL 61`, `MISS-PATH 2`, `MISS-LINE 0`.
  It must also, for every `MISS-SYMBOL`, report whether the symbol exists
  **anywhere** in `src/` — that single column separates *stale by move*
  (fixable by re-pointing) from *stale by deletion* (the claim itself is
  suspect), the distinction §2 verified by hand.
- **`--links [pkg…]`** — resolve every `mfb spec <pkg> [<topic>]` and
  `mfb man <pkg> [<fn>|types]` reference in the spec text against
  `PACKAGE_ORDER` / the topic files / the registry. Report unresolvable
  targets with their source line.
- **`--render [pkg…]`** — render each package `--all`, assert it is non-empty,
  and grep the rendered output for leaked `[[` (the renderer strips them, so
  any hit is a malformed marker).
- **`--fences [pkg…]`** — inventory the 363 code fences by language tag, so
  letters J–L know which are MFBASIC and worth compiling.

`--citations` and `--links` are the two that turn "the spec is probably fine"
into a number. Both are re-run at the top and bottom of every spec letter.

### 4.3 `scripts/doc-review-fanout.sh`

The harness that makes 897 runs tractable.

- **Input**: a unit-list file (one unit per line, e.g. `man-pkg:color`,
  `man-page:color/mix`, `man-topic:flow/if`, `spec-pkg:memory`,
  `spec-file:memory/07_runtime-helper-abi.md`), a prompt template path, and a
  concurrency `N` (default 6).
- **Worktrees**: `N` detached worktrees created once per letter at the
  letter's base commit, reused round-robin, removed at letter close.
  Measured cost: 228M each (`du -sh .claude/worktrees/research`), 953Gi free
  — 6 worktrees ≈ 1.4G. They carry **no `target/`**: reviewers use the
  primary checkout's prebuilt binary via `MFB=<primary>/target/release/mfb`,
  so the harness never triggers six cargo builds. Per memory
  `enterworktree-absolute-path-edits-main` the prompt uses
  worktree-relative paths only.
- **Per unit**: substitute the unit into the template, run
  `codex exec -C <worktree> -s workspace-write - < prompt`, capture stdout to
  `planning/plan-125-findings/<letter>/<unit>.md` (path-safe slug), then run
  `git -C <worktree> status --porcelain`; a non-empty result is recorded as
  `DIRTY` in the manifest and the worktree is reset before reuse.
- **`manifest.tsv`**: `unit  exit  seconds  findings_count  dirty  banner`.
  A unit with `exit != 0` or a zero-byte findings file is **re-queued once**,
  then recorded `FAILED` — never dropped. A letter may not close while any
  unit is `FAILED` or absent from the manifest.
- **Scratch discipline**: probe programs go in `/tmp/plan-125/<unit>/`, never
  in the worktree, and the prompt sets that as the scratch cwd (memory
  `example-harness-cwd-and-timeout`: a scratch *project* is the cwd, and
  every run is time-bounded). Per-run timeout, recorded on expiry.
- **Never** takes a real directory as a scratch argument (memory
  `test-accept-second-arg-is-rm-rf-scratch`).

### 4.4 The six lenses of a final sweep (letters H and N)

One `codex exec` per lens, each over the whole artifact, each with exactly one
question:

**Man (H), over `scripts/man-manual.sh` output:**
1. **Terminology** — is one concept spelled one way everywhere (handle vs
   resource, fails vs errors, index vs position, byte vs character)?
2. **Example style** — do examples across packages look like they came from
   one manual (imports shown or not, naming, output shown or not, error
   handling shown or not)?
3. **Audience/scope** — read end-to-end for any sentence that requires a
   compiler mental model, including ones no grep can find (plan-108-F's
   recorded blind spot: a page can teach a borrow model with no banned word).
4. **Error documentation** — is every failure a developer can hit documented,
   and consistently, across sibling functions?
5. **Cross-links and discoverability** — does every `mfb man X` reference in
   the prose resolve, and can a developer find the right page from the index?
6. **Memory vocabulary** — the plan-108-A §3 (2a) ban, 0 unclassified, plus
   the "did anyone delete a true contract to pass the grep" check.

**Spec (N), over the 12 concatenated `mfb spec <pkg> --all` renderings:**
1. **Contract completeness** — is every externally observable contract the
   compiler has covered by some topic?
2. **Single source of truth** — duplicated or contradicting bodies across
   topics (`.ai/specifications.md`'s first convention).
3. **Citation integrity** — 0 `MISS-*` from `--citations`, and no claim left
   uncited that needed one.
4. **Accuracy at HEAD** — spot-verified claims against the code, weighted to
   the areas that changed most since each topic was last touched.
5. **Reading order and cross-links** — `PACKAGE_ORDER`, per-package reading
   prose, `## See Also`, and every `mfb spec`/`mfb man` link resolving.
6. **Man↔spec agreement** — the two surfaces must not contradict each other;
   this lens reads the "belongs in spec" ledger from B–H as its checklist.

## Compatibility / Format Impact

- **Rendered `mfb man` output changes** wherever prose is corrected — that is
  the plan. `tests/cli/cli_man_summary_plain.rs` and
  `tests/cli/cli_canvas_man_examples_compile.rs` pin some rendered text; a letter
  that changes pinned text updates the pin in the same commit.
- **`mfb man --all` gains the guide topics** if the §4.1 Open Decision goes
  that way. No package page's content or order changes.
- **Rendered `mfb spec` output changes** wherever a claim is corrected. If
  `src/docs/spec/diagnostics/02_error-codes.md`'s Constant Registry table is
  touched, the generated `errorCode::` constants change — that is a
  **compiler-visible** edit and gets `cargo build` + `cargo test errorcode`.
- Descriptor types, registry schema, `.mfp` format, ABI, and every MFBASIC
  body are unchanged.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` **in the same
> commit as the work it describes**. Use `- [~]` for partially done with one
> line on what remains. Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting it. Fill each phase's `Commit:` line the moment it lands.
> **An unticked box means NOT DONE.**

### Phase 1 — Re-census both surfaces and resolve the `--all` hole

Establishes the denominators every later letter reconciles against, before any
standard or tool is written to the wrong shape.

- [x] Rebuild `cargo build --release`; confirm the binary post-dates HEAD.
      Was stale (`Sep 8 07:36` vs HEAD `b49ca1610` at `Sep 11 13:06`); rebuilt
      in the `P-125` worktree → `Sep 12 06:15`.
- [x] Re-run `./scripts/man-census.sh --fill`, `--functions`, `--memory-scope`,
      `--scope`; paste all four outputs into §2 of this file, replacing the
      2026-09-04 figures if they moved. All four moved — §2's table now carries
      the measured numbers with the 2026-09-04 values beside them, and
      Corrections C-1 records which letters absorb the +7 pages.
- [x] **(added)** Give `man-census.sh` the `--topics` mode §1 requires, and
      widen `--memory-scope` / `--scope` to the guide topics on a whole-surface
      run. No Phase checkbox carried this and §1 states it as a goal; without
      it the census denominator is 596 of 628 pages. `--topics` reconciles
      rendered pages against markdown files per topic (32 == 32) and fails if
      they disagree.
- [x] Write the §2 citation-measurement script into `scripts/spec-census.sh`
      as `--citations` (§4.2) and confirm it reproduces `MISS-SYMBOL 61`,
      `MISS-PATH 2`, `MISS-LINE 0`. A different number is a Correction, not a
      quiet edit. **Reproduces exactly** at the plan's own commit — see C-4 for
      the run and for the +1 drift to 62 at HEAD; C-3 records the one rule in
      §4.2 that had to change (first colon, not last).
- [x] Classify all 61 `MISS-SYMBOL` hits into *stale by move* vs *stale by
      deletion* using the "exists anywhere in `src/`" column; record both
      counts here. The deletion class is a list of **suspect claims** handed
      to letters I–N, not just broken links. Measured: **49 stale-by-move, 13
      stale-by-deletion** (62 at HEAD). The 13 are enumerated in §2.1 with
      their spec sites.
- [x] Decide §4.1: does `mfb man --all` render guide topics? Record the
      decision and, if yes, make the one permitted renderer change and re-run
      `cargo test --bin mfb man` + `tests/cli/cli_man_summary_plain.rs`.
      **DECIDED: yes.** `render_all_markdown` now appends every guide topic in
      the index's sorted order. `cargo test --release --bin mfb man` → 291
      passed, 0 failed (including `all_renders_the_whole_registry_manual`);
      `cargo test --release --test cli_man_summary_plain` → 1 passed.
      `testing`/`general` stay filtered for the documented reason. This is the
      only renderer change plan-125 permits and it is now spent.
- [x] Write `scripts/man-manual.sh`; confirm its output covers all ~~621~~ 628
      man pages (assert the page count against the census).
      `./scripts/man-manual.sh --count` → `PAGES 628`, and the guide-page count
      agrees across two independent measurements (32 summed per topic; 32 as
      artifact-total minus registry-total). Registry pages 596 = the census's
      `544 fn + 31 overview + 21 types`. Three instruments, one number.

Acceptance: `scripts/spec-census.sh --citations` runs and prints the measured
baseline; `scripts/man-manual.sh` output contains a header for all 31
packages **and** `testing`, `general`, and all 10 topics
(`./scripts/man-manual.sh | grep -cE '^(TESTING|GENERAL|A TOUR OF MFBASIC)$'`
→ 3); §2's tables in this file are the numbers those commands just printed.
**MET** — the acceptance grep returns exactly `3`; `--citations` prints
`MISS-PATH 2 / MISS-LINE 0 / MISS-SYMBOL 62 (49 move, 13 deletion)`; §2 and
§2.1 are those outputs.
Commit: `2a51694d4`

### Phase 2 — The two content standards

- [x] Extend `.ai/man-content.md`: state the audience in one line at the top
      (the §3.1 row); add a section governing the **narrative topics** — the
      same MUST/MUST-NOT list, the same memory ban, the same rule that every
      code block is compiled and run; note that topics have subtopic pages and
      that `mfb man <topic>` is the verification command.
- [x] Author `.ai/spec-content.md` — the contributor-audience review standard:
      what a topic must contain (normative contract, `[[ ]]` provenance at
      claim-cluster granularity, the as-is rule), what it must not (tutorial
      prose, marketing, a second full copy of another topic's body,
      unverifiable or aspirational claims), how to triage a spec/code
      disagreement (§3.5), and the two rot classes from Phase 1 with the rule
      that a *stale-by-deletion* citation makes the claim suspect.
- [x] Cross-reference the two standards at the seam: `.ai/man-content.md`
      points at `.ai/spec-content.md` for "this belongs in spec", and back.
- [x] Create `planning/plan-125-belongs-in-spec.md` with its header and empty
      table (§3.1); letters B–H append, I–N consume.
- [x] Update AGENTS.md's "Creating or updating `mfb man` content" section to
      name `.ai/spec-content.md` alongside `.ai/man-content.md`, and to say
      which audience each serves.

Acceptance: both standards exist and each states its audience in its first 10
lines; `grep -n 'spec-content' AGENTS.md .ai/man-content.md` returns hits in
both; the ledger file exists.
**MET** — `.ai/man-content.md` line 3 and `.ai/spec-content.md` line 3 each
open with an `> **Audience: …**` blockquote; `grep -n 'spec-content' AGENTS.md
.ai/man-content.md` returns AGENTS.md:107, :109, :158 and man-content.md:6,
:262; `planning/plan-125-belongs-in-spec.md` exists with its row format,
resolution vocabulary (COVERED / FILLED / REJECTED) and re-derivable counters.
Commit: `bd566a610`

### Phase 3 — The fan-out harness

- [x] Write `scripts/doc-review-fanout.sh` per §4.3: unit list, prompt
      template, concurrency, N reusable detached worktrees with no `target/`,
      `MFB=` pointing at the primary release binary, per-unit findings file,
      `manifest.tsv` with exit/seconds/findings/dirty/banner, one re-queue then
      `FAILED`, per-run timeout, ~~`/tmp/plan-125/<unit>/`~~
      `/tmp/plan-125-scratch/<letter>/<slug>/` scratch (per-letter, so two
      letters cannot collide on a unit name).
- [x] Self-test it on a 3-unit list including **one unit that must fail**
      (~~a nonexistent package~~ — see C-8: a nonexistent package does *not*
      fail, so the failure is induced by the watchdog) and confirm the manifest
      records `FAILED` rather than dropping it — memory
      `diagnostic-harness-must-record-exit-and-unlocated-errors`.
- [x] Self-test the dirty-worktree path: have a run touch a file, confirm
      `DIRTY` is recorded and the worktree is reset before reuse — memory
      `subagent-edits-can-silently-vanish`. The reviewer really did create
      `HARNESS_SELFTEST_MARKER.txt` (its `diff --git` is in the transcript);
      the manifest row reads `DIRTY`; `git status --porcelain` in that worktree
      is empty afterwards and the file is gone.
- [x] Add a `--reconcile` mode: given a unit list and a manifest, print any
      unit missing, `FAILED`, or without a findings file; exit non-zero if any.
      Every letter runs this before it closes. Also reports `ORPHAN` — a
      manifest row whose unit is *not* in the list, which means the list moved
      under the run and a review was made that no letter's accounting covers.

Acceptance: the 3-unit self-test manifest has one `FAILED` row and
`--reconcile` exits non-zero on it and zero after the re-run; a deliberately
dirty run is recorded `DIRTY`.

**MET**, measured (letter `SELFTEST`, `planning/plan-125-findings/SELFTEST/manifest.tsv`):

```
unit                    exit    seconds findings_lines  dirty   banner
man-pkg:NOSUCHPACKAGE   FAILED  5       1               clean   OpenAI Codex v0.153.0/gpt-5.6-terra
man-pkg:NOSUCHPACKAGE   0       8       1               clean   OpenAI Codex v0.153.0/gpt-5.6-terra
selftest-dirty:one      0       8       0               DIRTY   OpenAI Codex v0.153.0/gpt-5.6-terra
```

- **FAILED path**: `--timeout 5` → `exit=124`, re-queued exactly once, second
  attempt also 124, row recorded `FAILED`. `--reconcile` → `unaccounted=1`,
  exit 1.
- **Re-run**: same unit at `--timeout 600` → exit 0 in 8s; `--reconcile` →
  `unaccounted=0`, exit 0. (This is what C-9 fixed: reconcile read the *first*
  row and would have reported the stale `FAILED` forever.)
- **DIRTY path**: recorded, worktree reset, marker file gone.
- **MISSING path**: reconciling the 3-unit manifest against a 4-unit list →
  `MISSING man-pkg:neverran`, exit 1.
- **Parallel path** (letter `SELFTEST3`, 3 units at `--jobs 3`): all three ran
  concurrently at 7–8s each, `unaccounted=0 orphans=0`.

Banner for the record, per plan-108's practice: `OpenAI Codex v0.153.0`,
model `gpt-5.6-terra`.
Commit: `7bf6060ee`

### Phase 4 — The eight reviewer prompts

- [x] Write all eight prompts verbatim into §5 of this file (man 1/2/3 +
      final; spec 1/2/3 + final), each stating: the audience, the lens, the
      standard file to read, the rendering command, the verification duty
      (read the code; run probes; **do not** attempt to bind a socket — the
      main thread runs network probes), the structured findings format
      (`unit / claim / verdict / evidence / suggested wording`), and the
      instruction to make **no repository edits**. §5.1–§5.8, generated.
- [x] Store each prompt also as a file under `planning/plan-125-prompts/` so
      the harness can pass it with `-`; the file and §5 must match (a
      `diff` check in the pilot). The files are the **source** and §5 is
      generated from them by `scripts/plan-125-prompts-sync.sh` — see the
      note above §5 for why two hand-maintained copies were refused.

Each prompt additionally carries two things the plan did not list, both of
which exist to make a *silent* failure loud:

- **A required no-findings report.** "NO FINDINGS" must be followed by a
  paragraph naming the probe that was run and what it printed. An unaudited
  "looks fine" from a reviewer is indistinguishable from a reviewer that did
  not look, and ~900 of those would be the plan's worst outcome.
- **A named category for the iteration-3 success test.** A factual error found
  in iteration 3 is tagged `fact-escaped-iter2`, so §3.2's claim that
  iteration 3 finds *seams* rather than *facts* is measured rather than
  asserted.

Acceptance: eight prompt files exist; `diff` between each file and its §5
block is empty.
**MET** — `ls planning/plan-125-prompts/` → 8 files;
`./scripts/plan-125-prompts-sync.sh --check` → `§5 matches all 8 prompt files`,
exit 0.
Commit: `281b4ae50`

### Phase 5 — Pilot: `color`, the `variable` topic, and the `unicode` spec package

The calibration run. `color` is chosen because **no plan-108 letter ever
reviewed it** (it did not exist), so it is the most likely to yield real
findings; `variable` because it is the topic every package page links to and
was authored, never independently reviewed; `unicode` because it is the
smallest spec package (3 files, 508 lines) and can absorb a tooling mistake.

- [ ] Iteration 1 on `color` (1 unit) and `variable` (1 unit): my pass →
      Codex → apply. Record the ledger (finding / verdict / evidence /
      disproving command for every rejection).
- [ ] Iteration 2 on all 30 `color` pages and both `variable` pages
      (`package.md` is the only file; 1 page) — 31 units through the harness
      at `N=6`. Every example compiled and run. Record wall-clock per unit.
- [ ] Iteration 3 on `color` and `variable` (2 units): the re-integration
      lens. Record whether its findings are seams or facts (§3.2's success
      test for iteration 2).
- [ ] The same three iterations on the `unicode` spec package (1 + 3 + 1 = 5
      units), including `--citations` before and after.
- [ ] Record in this file, as a table: **units, wall-clock, findings raised,
      findings confirmed, findings rejected — per iteration, per surface.**
      These numbers size letters B–N; if the per-unit cost is more than 2× the
      estimate, re-batch C–F and J–L before starting them.
- [ ] Record the Codex banner (`codex --version` and the model it reports) for
      the pilot, as plan-108 did.
- [ ] Sweep the pilot's changes: `./scripts/man-census.sh --memory-scope color`
      and `--scope color` → 0 unclassified;
      `./scripts/spec-census.sh --citations unicode` → 0 `MISS-*`.
- [ ] `--reconcile` clean for every pilot unit.

Acceptance: 36 pilot units all present in the manifest with `exit 0`, no
`FAILED`, no `DIRTY`; the per-iteration cost table is filled with measured
numbers; `color`, `variable` and `unicode` are through all three iterations
with their ledgers recorded here; `git diff` on the pilot commits shows
string-literal and markdown changes only.
Commit: —

## 5. The reviewer prompts

<!-- Filled in by Phase 4, verbatim, and mirrored into
     planning/plan-125-prompts/*.txt. Every later letter runs these unchanged;
     a prompt edit mid-plan makes findings incomparable and is recorded as a
     Correction.

     The mirror is GENERATED, not hand-copied: the files are the source and
     `scripts/plan-125-prompts-sync.sh --check` is the diff Phase 4's
     acceptance asks for. Two hand-maintained copies of a thousand lines is a
     copy that drifts, which is the same reason `.ai/man-content.md` §4.2
     refuses to re-type the banned-word list. -->

<!-- BEGIN GENERATED PROMPTS -- edit planning/plan-125-prompts/*.txt, then scripts/plan-125-prompts-sync.sh --write -->

The eight prompts every one of plan-125's reviewer runs uses, verbatim.
They are generated from `planning/plan-125-prompts/*.txt`, which is what
`doc-review-fanout.sh --prompt` actually passes to `codex exec`; run
`./scripts/plan-125-prompts-sync.sh --check` to prove this section matches.

The harness substitutes `{{UNIT}}`, `{{KIND}}`, `{{TARGET}}`, `{{MFB}}` and
`{{SCRATCH}}` per run; everything else is identical across all ~900 runs, so
findings stay comparable between letters.

### 5.1 Man, iteration 1 — the package or topic as a whole

`planning/plan-125-prompts/man-iter1-package.txt`

```
You are reviewing MFBASIC's developer documentation as an independent second
opinion. You are not the author. Be specific and be hard to please.

UNIT: {{UNIT}}

## Who this documentation is for

`mfb man` is written for **the MFBASIC developer at the terminal** — someone
using and learning the language. It is NOT written for a compiler contributor.
The test for any sentence: *if it only matters to someone reading compiler
source, it does not belong on the page.*

Read the standard before you start:

    cat .ai/man-content.md

## Your lens — ITERATION 1: the unit as a WHOLE

This pass is the only one that sees the unit all at once, so it is the only
one that can find these four things. Look for them specifically; a later pass
reads one page at a time and is structurally blind to all of them.

1. **Coverage.** Something a developer needs that no page mentions at all.
   What question would they arrive with that this unit never answers?
2. **Internal consistency.** Two sibling pages describing the same concept in
   two different ways, or using two different words for one thing.
3. **The overview's promises versus what the functions deliver.** Does the
   overview describe a package that these functions actually add up to?
4. **Ordering and discoverability.** Can a developer find the right page from
   the overview? Is the reading order sensible?

Do NOT do a sentence-by-sentence proofread. A later pass does that with far
more time per page than you have. If you find yourself listing wording nits,
you are in the wrong pass — go back up to the four questions.

## Render the unit

For a package unit (`man-pkg:<name>`):

    {{MFB}} man {{TARGET}} --all
    {{MFB}} man {{TARGET}} types

For a guide-topic unit (`man-topic:<name>`):

    {{MFB}} man {{TARGET}} --all

## Verification duty

Every factual claim you challenge must be challenged **with evidence**, not
with an impression:

- Read the implementation. Builtin pages are rendered from the clean-room
  registry descriptors under `src/codegen/builtins/<pkg>/`; guide topics are
  markdown under `src/docs/man/<topic>/`.
- Run probe programs. Write them under {{SCRATCH}} — **never inside the
  repository** — and compile and run them with {{MFB}}.
- **Do not attempt to bind or connect a socket.** Your sandbox cannot, and a
  failure there tells you nothing. If a claim about `tcp`, `udp`, `tls`, `net`
  or `http` needs a live socket to check, say so in the finding and mark it
  `NEEDS-NETWORK-PROBE`; the main thread will run it.

A claim you cannot check is reported as `UNVERIFIED` with what you tried. Do
not guess, and do not pad the list to look thorough — a short list of real
findings is worth far more than a long list of maybes.

## Output format

Reply with findings only, one block each, in this exact shape:

    ### <n>. <short title>
    UNIT:      {{UNIT}}
    PAGE:      <the specific page, or "package-wide">
    CATEGORY:  coverage | consistency | overview-mismatch | discoverability
    CLAIM:     <the sentence or gap, quoted verbatim where it is a sentence>
    VERDICT:   wrong | misleading | missing | inconsistent
    EVIDENCE:  <the command you ran and what it printed, or the file:symbol you read>
    SUGGESTED: <the wording you would use instead, or what to add>

If you find nothing, reply with exactly:

    NO FINDINGS for {{UNIT}}

followed by one paragraph on what you checked, so the absence is auditable.

## Rules

- **Make no edits to any file in the repository.** Your output is text. Every
  fix is applied by the main thread; an edit here is discarded and recorded as
  a harness violation.
- Scratch files go under {{SCRATCH}} only.
- Do not commit anything.
```

### 5.2 Man, iteration 2 — one page, every sentence verified

`planning/plan-125-prompts/man-iter2-page.txt`

```
You are reviewing MFBASIC's developer documentation as an independent second
opinion. You are not the author. Be specific and be hard to please.

UNIT: {{UNIT}}

## Who this documentation is for

`mfb man` is written for **the MFBASIC developer at the terminal** — someone
using and learning the language. It is NOT written for a compiler contributor.
The test for any sentence: *if it only matters to someone reading compiler
source, it does not belong on the page.*

Read the standard before you start:

    cat .ai/man-content.md

## Your lens — ITERATION 2: ONE page, and the time to verify EVERY sentence

You have been given exactly one page and no siblings. That is deliberate. This
is the depth pass, and it is the only pass that can afford to check every
sentence against the implementation.

Your duty, per sentence, is to answer: **is this true of the binary at HEAD?**
Not plausible. Not what the author intended. True.

Work through all of:

1. **Every sentence of the intro and description.** Each one is a claim.
2. **Every parameter description.** Units, valid range, and what happens at
   zero, empty, and negative. Whether a range bound is inclusive or exclusive
   — the parameter's NAME does not say, so the description must.
3. **Every row of the Errors table**, against the condition that actually
   raises it.
4. **The example: compile it and run it.** Copy it to {{SCRATCH}} and build it
   with {{MFB}}. An example that does not compile is a finding. An example
   whose output differs from what the page implies is a finding.
5. **The sharp edges.** Does it clamp or raise? Unicode scalar or grapheme?
   Mutation or a new value? Is ordering stable? A page that omits a sharp edge
   has failed even if every sentence on it is true — `strings::mid` raises
   where `left` and `right` clamp, and a page that does not say so is wrong by
   omission.
6. **Scope.** Any sentence that requires a compiler mental model
   (`.ai/man-content.md` §3), and any use of the banned memory vocabulary
   (§4 — the only permitted words are **copy**, **mutate**, **value**, and
   **alias**, the last for a `RES` handle only).
7. **The subtler §4 failure, which no grep can find**: a sentence that teaches
   a borrow/ownership mental model *without using a banned word*. Flag it.

## Render the page

    {{MFB}} man {{TARGET}}

(For a `man-page:<pkg>/<fn>` unit run `{{MFB}} man <pkg> <fn>`; for
`man-page:<pkg>/types` run `{{MFB}} man <pkg> types`; for a `man-topic-page`
unit render the topic and read the named page.)

## Verification duty

- Read the implementation for this member: the registry descriptor under
  `src/codegen/builtins/<pkg>/`, and the lowering it names.
- Run probe programs. Write them under {{SCRATCH}} — **never inside the
  repository** — and compile and run them with {{MFB}}.
- **Do not attempt to bind or connect a socket.** Your sandbox cannot, and a
  failure there tells you nothing. If a claim about `tcp`, `udp`, `tls`, `net`
  or `http` needs a live socket to check, say so in the finding and mark it
  `NEEDS-NETWORK-PROBE`; the main thread will run it.

A claim you cannot check is reported as `UNVERIFIED` with what you tried. Do
not guess.

## Output format

Reply with findings only, one block each, in this exact shape:

    ### <n>. <short title>
    UNIT:      {{UNIT}}
    CLAIM:     <the sentence, quoted verbatim>
    VERDICT:   wrong | misleading | incomplete | out-of-scope | banned-vocabulary | example-broken
    EVIDENCE:  <the probe program and its output, or the file:symbol you read>
    SUGGESTED: <the wording you would use instead>

If you find nothing, reply with exactly:

    NO FINDINGS for {{UNIT}}

followed by one paragraph naming the probe you ran and what it printed, so the
absence is auditable. "I read it and it looked fine" is not an acceptable
no-findings report on this pass.

## Rules

- **Make no edits to any file in the repository.** Your output is text. Every
  fix is applied by the main thread; an edit here is discarded and recorded as
  a harness violation.
- Scratch files go under {{SCRATCH}} only.
- Do not commit anything.
```

### 5.3 Man, iteration 3 — re-integration after the page pass

`planning/plan-125-prompts/man-iter3-package.txt`

```
You are reviewing MFBASIC's developer documentation as an independent second
opinion. You are not the author. Be specific and be hard to please.

UNIT: {{UNIT}}

## Who this documentation is for

`mfb man` is written for **the MFBASIC developer at the terminal** — someone
using and learning the language. It is NOT written for a compiler contributor.

Read the standard before you start:

    cat .ai/man-content.md

## Your lens — ITERATION 3: RE-INTEGRATION, after the surgery

Every page in this unit has just been edited **independently of its
siblings**, one page at a time, by a pass that could not see any of the
others. That reliably introduces three things, and you are the first reader to
see the unit in its final form:

1. **Divergence.** Two pages now explain one concept in two different ways, or
   use two different words for the same thing. Both may be individually
   correct. That is still a defect: the reader meets both.
2. **Redundancy.** The same explanation restated on five pages, where one page
   should own it and the others should link. `mfb man variable` is the page
   that owns the value model; a package page that re-explains it is a defect
   even if the explanation is right.
3. **Broken seams.** A cross-reference to a page that no longer says what the
   referring page claims it says; a "see also" that now points at the wrong
   sibling; an overview that promises something a rewritten page no longer
   delivers.

**Findings on this pass should mostly be about SEAMS, not facts.** That is the
success test for the pass before you: if you find many *factual* errors here,
the per-page pass under-performed, and that is itself a result worth
reporting. So: if a finding is a plain factual error, still report it, and
mark its CATEGORY `fact-escaped-iter2` so the plan can count them.

## Render the unit

For a package unit (`man-pkg:<name>`):

    {{MFB}} man {{TARGET}} --all
    {{MFB}} man {{TARGET}} types

For a guide-topic unit (`man-topic:<name>`):

    {{MFB}} man {{TARGET}} --all

Read it end to end, in order, as a developer meeting this package for the
first time would.

## Verification duty

- For a seam finding, the evidence is the two places that disagree, quoted.
- For any factual finding, the evidence is a probe program under {{SCRATCH}}
  compiled and run with {{MFB}}, or the `file:symbol` you read.
- **Do not attempt to bind or connect a socket.** Mark such a finding
  `NEEDS-NETWORK-PROBE`; the main thread will run it.

## Output format

Reply with findings only, one block each, in this exact shape:

    ### <n>. <short title>
    UNIT:      {{UNIT}}
    PAGES:     <the two or more pages involved>
    CATEGORY:  divergence | redundancy | broken-seam | fact-escaped-iter2
    QUOTE-A:   <the first wording, verbatim, with its page>
    QUOTE-B:   <the second wording, verbatim, with its page>
    VERDICT:   <which should win, and why>
    SUGGESTED: <the single wording both should use, or the link that replaces one>

If you find nothing, reply with exactly:

    NO FINDINGS for {{UNIT}}

followed by one paragraph on what you compared, so the absence is auditable.

## Rules

- **Make no edits to any file in the repository.** Your output is text. Every
  fix is applied by the main thread; an edit here is discarded and recorded as
  a harness violation.
- Scratch files go under {{SCRATCH}} only.
- Do not commit anything.
```

### 5.4 Man, final sweep — one lens over the complete manual

`planning/plan-125-prompts/man-final-lens.txt`

```
You are reviewing MFBASIC's complete developer manual as an independent second
opinion. You are not the author. Be specific and be hard to please.

UNIT: {{UNIT}}
LENS: {{TARGET}}

## Who this documentation is for

`mfb man` is written for **the MFBASIC developer at the terminal** — someone
using and learning the language. It is NOT written for a compiler contributor.

Read the standard before you start:

    cat .ai/man-content.md

## The artifact

Generate the complete manual — all 628 pages, including the two packages
`mfb man --all` deliberately omits:

    ./scripts/man-manual.sh > {{SCRATCH}}/manual.txt
    ./scripts/man-manual.sh --count

It is about 61,000 lines. **You cannot read it end to end with useful
attention, and you are not being asked to.** You have been given exactly ONE
question. Answer that one question across the whole artifact, and ignore
everything else — five other runs are asking the other five questions.

## Your lens

**{{TARGET}}** — the question you are answering:

- **terminology** — Is one concept spelled one way everywhere? Check at least:
  handle vs resource; fails vs errors vs raises; index vs position vs offset;
  byte vs character vs code point vs grapheme; empty vs blank vs missing.
  Report each concept that has more than one spelling, with a count of each
  and the pages using the minority form.

- **example-style** — Do the examples across packages look like they came from
  one manual? Are imports shown, or assumed? Is output shown? Is error
  handling shown? Are variables named consistently? Report each axis on which
  packages disagree, naming the packages on each side.

- **audience-scope** — Read for any sentence that requires a compiler mental
  model. This lens exists because a grep cannot find the important cases: a
  page can teach a borrow-and-ownership model without using a single banned
  word. That is the failure recorded from the previous plan's certification,
  and it is what you are here for. Quote each one.

- **errors** — Is every failure a developer can actually hit documented, and
  documented consistently across sibling functions? Two functions that raise
  the same error should describe it the same way. A function that can fail and
  documents no failure is the more serious finding.

- **cross-links** — Does every `mfb man <x>` reference in the prose resolve?
  Check by running it. Can a developer reach the right page from the index
  (`mfb man` with no arguments)? Report every dead link and every page that is
  reachable only by already knowing its name.

- **memory-vocabulary** — The hard ban in `.ai/man-content.md` §4. Run
  `./scripts/man-census.sh --memory-scope` and confirm it reports 0
  unclassified. Then do the thing the script cannot: check that nobody passed
  the grep by **deleting a true contract**. For every resource-returning or
  resource-taking function, does the page still say whether the handle stays
  open or is closed by the call? A page that is silent on that has been
  sanitized, not fixed, and that is a worse defect than the banned word was.

## Verification duty

- Evidence is a command and its output. `grep -c` over the artifact, a probe
  program under {{SCRATCH}} compiled with {{MFB}}, or a rendered page.
- **Do not attempt to bind or connect a socket.** Mark such a finding
  `NEEDS-NETWORK-PROBE`; the main thread will run it.

## Output format

Reply with findings only, one block each, in this exact shape:

    ### <n>. <short title>
    LENS:      {{TARGET}}
    SCOPE:     <the pages or packages involved>
    CLAIM:     <the inconsistency or gap, with quotes from each side>
    VERDICT:   <which form should win, and why>
    EVIDENCE:  <the command you ran and what it printed>
    SUGGESTED: <the single form to standardize on, or the text to add>

Order your findings by how many pages each affects, most first.

If you find nothing, reply with exactly:

    NO FINDINGS for {{UNIT}}

followed by one paragraph naming the commands you ran across the artifact.

## Rules

- **Make no edits to any file in the repository.** Your output is text.
- Scratch files go under {{SCRATCH}} only.
- Do not commit anything.
```

### 5.5 Spec, iteration 1 — the package as a whole

`planning/plan-125-prompts/spec-iter1-package.txt`

```
You are reviewing MFBASIC's compiler specification as an independent second
opinion. You are not the author. Be specific and be hard to please.

UNIT: {{UNIT}}

## Who this documentation is for

`mfb spec` is written for **the compiler contributor**, and for the developer
who wants the internal detail. Internals are not merely permitted here, they
are the point — and every non-obvious one carries a `[[path:Symbol]]`
provenance citation.

This is the mirror image of `mfb man`, which bans all of that. Do not apply
man-page rules here.

Read both standards before you start:

    cat .ai/spec-content.md
    cat .ai/specifications.md

## Your lens — ITERATION 1: the package as a WHOLE

This pass is the only one that sees the package all at once, so it is the only
one that can find these four things. A later pass reads one file at a time and
is structurally blind to all of them.

1. **Contract completeness.** An externally observable contract the compiler
   has that no topic in this package covers. What would a contributor arrive
   needing to know that this package never states?
2. **Single source of truth.** Two topics carrying full bodies of the same
   fact, or — worse — carrying two bodies that disagree. Each fact has one
   canonical topic; others summarize and link.
3. **The overview versus the topics.** Does the package overview describe a
   package these topics actually add up to? Is the stated reading order the
   order that works?
4. **Guaranteed versus incidental.** A contributor cannot tell by inspection
   whether a stated behaviour is a promise or an accident of the current
   implementation. Flag every place the package states a behaviour without
   saying which it is — silently promoting an accident to a contract is how
   the next contributor gets trapped into preserving it.

Do NOT proofread sentence by sentence. A later pass does that per file.

## Render the unit

    {{MFB}} spec {{TARGET}} --all

and measure it:

    ./scripts/spec-census.sh --citations {{TARGET}}
    ./scripts/spec-census.sh --links {{TARGET}}
    ./scripts/spec-census.sh --fences {{TARGET}}

## The accuracy rule, and the two rot classes

**The spec describes the compiler AS IT IS AT HEAD** — not as designed, not as
it will be. A planned contract that does not exist yet is a lie with a
citation attached; flag it.

`--citations` splits its failures into two classes and they need different
repairs. Respect the difference:

- **STALE BY MOVE** (`ELSEWHERE=yes`) — the symbol still exists at another
  path. Re-point the link; the claim is probably still fine.
- **STALE BY DELETION** (`ELSEWHERE=no`) — the symbol exists nowhere in
  `src/`. **The CLAIM is suspect, not just the link.** The thing the sentence
  describes has been deleted or renamed out of existence. Verify the behaviour
  from scratch. A reviewer who only re-points paths here turns the link green
  and leaves the sentence wrong, and the surface then looks audited.

## Verification duty

Verifying a spec claim can mean reading a whole compiler pass, so triage:

1. **Citation first** — a claim with a resolving `[[path:Symbol]]` is checked
   at that symbol. Read it.
2. **Probe second** — if the claim is externally observable, build a program
   under {{SCRATCH}} and run it with {{MFB}}.
3. **Flag third** — a claim with no citation that you cannot locate in the
   code and cannot observe is not a contract. Report it as `UNCITED-UNCHECKABLE`.

- **Do not attempt to bind or connect a socket.** Mark such a finding
  `NEEDS-NETWORK-PROBE`; the main thread will run it.

## Output format

Reply with findings only, one block each, in this exact shape:

    ### <n>. <short title>
    UNIT:      {{UNIT}}
    TOPIC:     <the file, or "package-wide">
    CATEGORY:  missing-contract | duplicate-body | contradiction | reading-order | guarantee-unstated | stale-by-deletion | uncited-uncheckable
    CLAIM:     <the sentence or gap, quoted verbatim where it is a sentence>
    VERDICT:   wrong | misleading | missing | duplicated | aspirational
    EVIDENCE:  <the command you ran and what it printed, or the file:symbol you read>
    SUGGESTED: <the wording you would use instead, or what to add, with its citation>

If you find nothing, reply with exactly:

    NO FINDINGS for {{UNIT}}

followed by one paragraph on what you checked, so the absence is auditable.

## Rules

- **Make no edits to any file in the repository.** Your output is text. Every
  fix is applied by the main thread; an edit here is discarded and recorded as
  a harness violation.
- Scratch files go under {{SCRATCH}} only.
- Do not commit anything.
```

### 5.6 Spec, iteration 2 — one file, every claim verified

`planning/plan-125-prompts/spec-iter2-file.txt`

```
You are reviewing MFBASIC's compiler specification as an independent second
opinion. You are not the author. Be specific and be hard to please.

UNIT: {{UNIT}}
FILE: src/docs/spec/{{TARGET}}

## Who this documentation is for

`mfb spec` is written for **the compiler contributor**, and for the developer
who wants the internal detail. Internals are not merely permitted here, they
are the point — and every non-obvious one carries a `[[path:Symbol]]`
provenance citation.

This is the mirror image of `mfb man`, which bans all of that. Do not apply
man-page rules here.

Read both standards before you start:

    cat .ai/spec-content.md
    cat .ai/specifications.md

## Your lens — ITERATION 2: ONE file, and the time to verify EVERY claim

You have been given exactly one file and no siblings. That is deliberate. This
is the depth pass, and it is the only pass that can afford to check every
claim against the code.

Your duty, per claim, is to answer: **is this true of the compiler at HEAD?**
Not plausible. Not what the author intended. Not what the design says. True.

Work through all of:

1. **Every normative statement.** Exact values, exact ranges, exact ordering,
   exact error behaviour. "Generally returns quickly" is not a contract.
2. **Every `[[path:Symbol]]` citation.** Does the symbol exist in that file?
   Does the code there actually say what the surrounding sentence claims? A
   resolving citation over a wrong claim is the failure mode this pass exists
   to catch — the link being green proves nothing about the sentence.
3. **Every uncited non-obvious claim** — a magic number, an offset, an ABI
   register, an enum variant, a capability list, a pass ordering. It needs a
   citation. Find the symbol and propose one, or report it as
   `UNCITED-UNCHECKABLE`.
4. **Guaranteed versus incidental.** Where the file states a behaviour, does
   it say whether that is a promise or an implementation accident? A
   contributor cannot tell by inspection.
5. **Backend qualification.** A claim true only on one target and stated
   unqualified is a wrong claim on the others. Check which targets it holds
   for.
6. **Failure modes.** Overflow, empty input, boundary, concurrency. A topic
   that documents only the success path is incomplete.
7. **Scope.** Tutorial prose, marketing ("blazingly fast", "simply",
   "elegant"), a second full copy of another topic's body, or aspirational
   behaviour that does not exist yet.
8. **Code fences.** Spec fences are legitimately illustrative and often
   deliberately partial, so a non-compiling fence is a finding to triage, NOT
   an automatic defect. But a fence tagged as MFBASIC that reads like a whole
   program and does not compile IS a defect — copy it to {{SCRATCH}} and build
   it with {{MFB}}.

## Render it

    {{MFB}} spec <package> <topic>

and measure the file's own citations:

    ./scripts/spec-census.sh --citations <package>

## The two rot classes

- **STALE BY MOVE** (`ELSEWHERE=yes`) — the symbol still exists at another
  path. Re-point the link; the claim is probably still fine.
- **STALE BY DELETION** (`ELSEWHERE=no`) — the symbol exists nowhere in
  `src/`. **The CLAIM is suspect, not just the link.** Verify the behaviour
  from scratch. Re-pointing a path here turns the link green and leaves the
  sentence wrong.

## Verification duty

1. **Citation first** — read the cited symbol.
2. **Probe second** — if the claim is externally observable, build a program
   under {{SCRATCH}} and run it with {{MFB}}.
3. **Flag third** — `UNCITED-UNCHECKABLE`.

- **Do not attempt to bind or connect a socket.** Mark such a finding
  `NEEDS-NETWORK-PROBE`; the main thread will run it.

If the spec says X and the code does Y, **do not average them.** Say which you
believe is wrong and why. If you believe the CODE is wrong, say so explicitly
and loudly — a documentation pass is allowed to find compiler bugs and is not
allowed to ignore them.

## Output format

Reply with findings only, one block each, in this exact shape:

    ### <n>. <short title>
    UNIT:      {{UNIT}}
    LINE:      <line number in src/docs/spec/{{TARGET}}>
    CATEGORY:  wrong-claim | stale-citation-move | stale-citation-deletion | uncited-uncheckable | guarantee-unstated | backend-unqualified | missing-failure-mode | out-of-scope | aspirational | broken-fence | SUSPECTED-COMPILER-BUG
    CLAIM:     <the sentence, quoted verbatim>
    EVIDENCE:  <the file:symbol you read and what it says, or the probe and its output>
    SUGGESTED: <the wording you would use instead, with its citation>

If you find nothing, reply with exactly:

    NO FINDINGS for {{UNIT}}

followed by one paragraph naming the symbols you read and any probe you ran,
so the absence is auditable. "I read it and it looked fine" is not an
acceptable no-findings report on this pass.

## Rules

- **Make no edits to any file in the repository.** Your output is text. Every
  fix is applied by the main thread; an edit here is discarded and recorded as
  a harness violation.
- Scratch files go under {{SCRATCH}} only.
- Do not commit anything.
```

### 5.7 Spec, iteration 3 — re-integration after the file pass

`planning/plan-125-prompts/spec-iter3-package.txt`

```
You are reviewing MFBASIC's compiler specification as an independent second
opinion. You are not the author. Be specific and be hard to please.

UNIT: {{UNIT}}

## Who this documentation is for

`mfb spec` is written for **the compiler contributor**, and for the developer
who wants the internal detail. Internals are the point here, and every
non-obvious one carries a `[[path:Symbol]]` provenance citation.

Read the standard before you start:

    cat .ai/spec-content.md

## Your lens — ITERATION 3: RE-INTEGRATION, after the surgery

Every topic in this package has just been edited **independently of its
siblings**, one file at a time, by a pass that could not see any of the
others. That reliably introduces three things, and you are the first reader to
see the package in its final form:

1. **Divergence.** Two topics now state one contract in two different ways, or
   use two different terms for one construct. Both may be individually
   correct. That is still a defect: a contributor meets both and cannot tell
   which is normative.
2. **Redundancy and lost ownership.** The same contract restated in full in
   three topics, where one should own it and the others should summarize and
   link. Worse: the per-file pass may have *added* a full body to a topic that
   previously linked, quietly creating a second source of truth.
3. **Broken seams.** A `mfb spec <pkg> <topic>` link to a topic that no longer
   says what the referring topic claims; a `## See Also` that now points
   somewhere wrong; a package overview whose reading-order prose describes an
   order the topics no longer follow.

**Findings on this pass should mostly be about SEAMS, not facts.** That is the
success test for the pass before you: if you find many *factual* errors here,
the per-file pass under-performed, and that is itself a result worth
reporting. So: if a finding is a plain factual error, still report it, and
mark its CATEGORY `fact-escaped-iter2` so the plan can count them.

## Render and measure the unit

    {{MFB}} spec {{TARGET}} --all
    ./scripts/spec-census.sh --citations {{TARGET}}
    ./scripts/spec-census.sh --links {{TARGET}}
    ./scripts/spec-census.sh --render {{TARGET}}

`--render` must report 0 leaked `[[` markers: the renderer strips them
everywhere, so any that survives into rendered output is malformed.

Read the package end to end, in its stated reading order, as a contributor
meeting it for the first time would.

## Verification duty

- For a seam finding, the evidence is the two places that disagree, quoted,
  with their files.
- For any factual finding, the evidence is the `file:symbol` you read or a
  probe under {{SCRATCH}} run with {{MFB}}.
- **Do not attempt to bind or connect a socket.** Mark such a finding
  `NEEDS-NETWORK-PROBE`; the main thread will run it.

## Output format

Reply with findings only, one block each, in this exact shape:

    ### <n>. <short title>
    UNIT:      {{UNIT}}
    TOPICS:    <the two or more topics involved>
    CATEGORY:  divergence | redundancy | lost-ownership | broken-seam | leaked-marker | fact-escaped-iter2
    QUOTE-A:   <the first wording, verbatim, with its file and line>
    QUOTE-B:   <the second wording, verbatim, with its file and line>
    VERDICT:   <which should win, and why — which topic OWNS this fact>
    SUGGESTED: <the single wording, or the link that replaces the duplicate body>

If you find nothing, reply with exactly:

    NO FINDINGS for {{UNIT}}

followed by one paragraph on what you compared, so the absence is auditable.

## Rules

- **Make no edits to any file in the repository.** Your output is text. Every
  fix is applied by the main thread; an edit here is discarded and recorded as
  a harness violation.
- Scratch files go under {{SCRATCH}} only.
- Do not commit anything.
```

### 5.8 Spec, final sweep — one lens over the whole spec

`planning/plan-125-prompts/spec-final-lens.txt`

```
You are reviewing MFBASIC's complete compiler specification as an independent
second opinion. You are not the author. Be specific and be hard to please.

UNIT: {{UNIT}}
LENS: {{TARGET}}

## Who this documentation is for

`mfb spec` is written for **the compiler contributor**, and for the developer
who wants the internal detail. Internals are the point here.

Read the standard before you start:

    cat .ai/spec-content.md

## The artifact

Generate the whole spec — all 12 packages, in `PACKAGE_ORDER`:

    for p in architecture language memory linker threading package \
             diagnostics tooling package-manager unicode app stdlib; do
      echo "===== $p ====="
      {{MFB}} spec $p --all
    done > {{SCRATCH}}/spec.txt

(Note `mfb spec --all` with no package renders 0 lines; only the per-package
form works. That is why the loop is spelled out.)

It is about 27,000 lines. **You cannot read it end to end with useful
attention, and you are not being asked to.** You have been given exactly ONE
question. Answer that one question across the whole artifact, and ignore
everything else — five other runs are asking the other five questions.

## Your lens

**{{TARGET}}** — the question you are answering:

- **contract-completeness** — Is every externally observable contract the
  compiler has covered by some topic? Work from the compiler outward, not from
  the spec inward: enumerate the surfaces (CLI flags, manifest keys, `.mfp`
  fields, error codes, ABI registers, IR ops, target backends) and ask which
  have no topic. An uncovered contract is invisible if you only read the spec.

- **single-source-of-truth** — Duplicated or contradicting bodies across
  topics. Two topics stating one fact is a defect even when they agree,
  because they will not agree forever. Report each duplicated fact with all
  the places it lives and which topic should own it.

- **citation-integrity** — Run `./scripts/spec-census.sh --citations` and
  confirm 0 `MISS-*`. Then do the thing the script cannot: sample claims that
  have a *resolving* citation and check the cited code actually says what the
  sentence claims. A green link over a wrong claim is invisible to every
  instrument. Also report non-obvious claims that carry no citation at all.

- **accuracy-at-head** — Spot-verify claims against the code, weighted to the
  areas that have changed most since each topic was last touched (use
  `git log --since` over `src/` versus over `src/docs/spec/`). The spec
  describes the compiler AS IT IS — flag anything aspirational.

- **reading-order** — `PACKAGE_ORDER`, each package's reading-order prose,
  every `## See Also`, and every `mfb spec` / `mfb man` link. Run
  `./scripts/spec-census.sh --links` and then judge the order a human would
  actually need: does a contributor reading in this order ever meet a term
  before it is defined?

- **man-spec-agreement** — The two surfaces must not contradict each other.
  Your checklist is `planning/plan-125-belongs-in-spec.md`: every row there is
  a sentence cut from a man page for being too internal, and every row must be
  resolved to COVERED (name the topic), FILLED, or REJECTED (with disproving
  evidence). Then, separately, look for places where `mfb man` and `mfb spec`
  state the same thing incompatibly — the overlap is the memory model,
  resources, and stdlib semantics. Render both and compare.

## Verification duty

- Evidence is a command and its output. A `grep -c` over the artifact, a
  `file:symbol` you read, or a probe under {{SCRATCH}} run with {{MFB}}.
- **Do not attempt to bind or connect a socket.** Mark such a finding
  `NEEDS-NETWORK-PROBE`; the main thread will run it.
- If you believe the CODE is wrong rather than the spec, say so explicitly and
  loudly.

## Output format

Reply with findings only, one block each, in this exact shape:

    ### <n>. <short title>
    LENS:      {{TARGET}}
    SCOPE:     <the topics or packages involved>
    CLAIM:     <the gap or contradiction, with quotes from each side>
    VERDICT:   <what is authoritative, and why>
    EVIDENCE:  <the command you ran and what it printed>
    SUGGESTED: <the text to add or change, with its citation>

Order your findings by how many topics each affects, most first.

If you find nothing, reply with exactly:

    NO FINDINGS for {{UNIT}}

followed by one paragraph naming the commands you ran across the artifact.

## Rules

- **Make no edits to any file in the repository.** Your output is text.
- Scratch files go under {{SCRATCH}} only.
- Do not commit anything.
```

<!-- END GENERATED PROMPTS -->

## Validation Plan

- **Tests**: none for man prose (Non-goals). For any spec letter:
  `cargo build`, `cargo test --bin mfb spec`; plus `cargo test errorcode` if
  `diagnostics/02_error-codes.md` changed. For the §4.1 renderer change (if
  taken): `cargo test --bin mfb man` and `tests/cli/cli_man_summary_plain.rs`.
- **Coverage check**: not a code-coverage question here — the analogue is
  `--reconcile`: a green letter means *every listed unit ran*, and that is
  checked, not assumed (memory `completeness-claims-need-an-audit`).
- **Runtime proof**: rendering. `mfb man <pkg> [<fn>|types|--all]`,
  `mfb man <topic>`, `mfb spec <pkg> [<topic>|--all]`,
  `./scripts/man-manual.sh`. Plus every man example compiled and run with the
  release binary during its iteration-2 unit.
- **Doc sync**: this plan *is* the doc sync. AGENTS.md gains
  `.ai/spec-content.md` (Phase 2). Memory gains only durable lessons, never
  plan status.
- **Acceptance**: `./scripts/doc-review-fanout.sh --reconcile` exits 0 for the
  letter's unit list; the letter's census/citation sweeps are at their target;
  every ledger row has a verdict and, for rejections, a disproving command.

## Open Decisions

- ~~**Should `mfb man --all` render the guide topics?**~~ — **SETTLED YES,
  Phase 1.** `render_all_markdown` appends every guide topic in the index's
  sorted order; `mfb man --all` is now 628 pages minus the 30 deliberately
  filtered `testing`/`general` pages. `cargo test --release --bin mfb man`
  (291 passed) and `cargo test --release --test cli_man_summary_plain`
  (1 passed) are green. The plan's one permitted renderer change is spent.
  Original reasoning follows.
- **Should `mfb man --all` render the guide topics?** — **Recommend yes**
  (§4.1): the user's final gate is literally `mfb man --all` as the full
  developer doc, and today it omits 62 of 621 pages. One contained change in
  `render_all_markdown`, the only renderer change plan-125 permits.
  Alternative: leave the renderer alone and let `scripts/man-manual.sh` be
  the only complete artifact — cheaper, but leaves a real product gap that
  this plan measured and chose not to fix.
- **Fan-out concurrency `N`.** — Recommend **6**, from the pilot's measured
  wall-clock; raise only if the pilot shows the main thread (the sole writer)
  is the bottleneck rather than the reviewers.
- **Worktree isolation vs scratch-cwd.** — Recommend **worktrees** (plan-108's
  proven shape, 228M each). Alternative: run Codex with a `/tmp` scratch cwd
  and the repo read-only; cheaper on disk, but unproven with this CLI version
  and it removes the `git status` violation check. Settle it in the Phase 5
  pilot and record which was used.
- **Should letters J–L compile the MFBASIC code fences in the spec?** —
  Recommend **inventory in Phase 1 (`--fences`), compile only those tagged as
  MFBASIC**, and treat a non-compiling fence as a finding rather than a gate;
  spec fragments are legitimately illustrative and often deliberately partial.

## Corrections

<!-- Filled in DURING execution: every place this letter turned out to be
     wrong — the claim, what was actually true, the evidence, and whether
     another letter's scope was derived from the wrong number. -->

### C-1 (Phase 1) — every population drifted; D and F absorb +7 man pages

**Claimed** (2026-09-04, `90f6c1357`): 538 man function pages, 884 parameter
descriptions, 20 `types` pages, 621 iteration-2 man units; guide topics 3,924
lines.

**Actually true** (2026-09-12, `b49ca1610`, `./scripts/man-census.sh --fill`):
544 / 903 / 21 / **628**; guide topics 4,015 lines
(`find src/docs/man -name '*.md' -exec cat {} + | wc -l`). Eight days and ~50
commits.

**Whose scope was derived from the wrong number.** Measured per batch with
`./target/release/mfb man <pkg> --all | grep -c '^═'` summed over each
letter's package list:

| Letter | Packages | Plan said | Measured | Δ |
|---|---|---|---|---|
| A (pilot) | `color` + `variable` | 31 | 30 + 1 = **31** | 0 |
| C | collections, datetime, encoding, math | 150 | **150** | 0 |
| D | fs, strings, term, astrings, io | 142 | **144** | **+2** |
| E | crypto, canvas, http, vector, os, general, bits | 142 | **142** | 0 |
| F | 14 small/resource packages + 31 guide pages | 156 | 130 + 31 = **161** | **+5** |

`30+150+144+142+130+32 = 628`, reconciling against the artifact's own page
count. **D and F are re-scoped in place to 144 and 161**; C and E are
unchanged and A's pilot is unchanged. The feature is not re-split — this is
the "correct the count, re-scope in place" row of the skill's table, not a
re-batching trigger.

Plan-wide totals move with it: unit runs 869 → **876**, total `codex exec`
runs 897 → **904**.

### C-2 (Phase 1) — `tests/` was censusing as a 32nd, unfilled man package

**Claimed**: "31 renderable packages — 32 dirs under `src/codegen/builtins/`
minus `perf`".

**Actually true**: there are now **33** dirs. `src/codegen/builtins/tests/` is
a `#[cfg(test)]` Rust module tree added after `man-census.sh`'s filter was
written, and `mfb man tests` → ``unknown package `tests` ``. The census
printed it as a package row with `PKGDOC 00` — indistinguishable from a real
package whose overview has neither intro nor description. The conclusion (31)
was right; the instrument was wrong, which is worse, because every later
letter reconciles against the instrument.

Fixed in `scripts/man-census.sh:packages()` (`grep -vE '^(perf|tests)$'`) with
the reason recorded in its header comment beside `perf`'s.

### C-3 (Phase 1) — the citation split rule is FIRST colon, not last

§4.2 specified "split on the **last** `:`". That shreds
`[[src/codegen/engine/value/builder_values.rs:NirValue::FunctionRef]]` — a
Rust symbol legitimately containing `::` — into a nonexistent path plus a bare
`FunctionRef`, and reports `MISS-PATH` on a citation that is perfectly fine.
Measured: last-colon gave `MISS-PATH 3`, first-colon gives `MISS-PATH 2`,
which is the plan's own baseline. No path anywhere in the tree contains `:`
(`grep -rhoE '\[\[[^]]+\]\]' src/docs/spec --include='*.md' | sort -u |
awk -F: 'NF>2'` → exactly one hit, that one), so the first colon is always the
path/suffix seam. `scripts/spec-census.sh` implements first-colon and says why.

### C-4 (Phase 1) — the citation baseline reproduces exactly, and has since drifted +1

The acceptance was "reproduce `MISS-SYMBOL 61`, `MISS-PATH 2`, `MISS-LINE 0`".
Run against a detached worktree at the plan's own commit
(`git worktree add --detach /tmp/plan125-base 90f6c1357`, then
`./scripts/spec-census.sh --citations`):

```
TOTAL unique=1411  nosuffix=115  line=19  symbol=1277
MISS-PATH 2
MISS-LINE 0
MISS-SYMBOL 61  (stale-by-move 49, stale-by-deletion 12)
```

**Exact match on all three MISS counters.** Two smaller discrepancies in the
plan's own §2 figures, both harmless and both recorded rather than quietly
edited: the plan wrote unique **1,414** where the measurement is **1,411**,
and symbol **1,280** where it is **1,277** (the plan was measured against a
tree carrying uncommitted `term` work — see the Prerequisites row that was
`NOT MET` at plan-writing).

At HEAD `b49ca1610` the same command gives **62** `(49 move, 13 deletion)`.
The delta is three moves, identified by diffing the two runs' citation lists:
`[[src/cli/dispatch.rs:exit_after_diagnostics]]` and
`[[src/codegen/builtins/strings/gen_strings_support.rs:static_strings_package_string]]`
newly broke; `[[src/codegen/builtins/regex/mod.rs:resolve_call]]` was fixed.

### C-5 (Phase 1) — the man surface was never certified clean; it was certified over 31 of 41 units

This is the most consequential correction in Phase 1, and it is a *finding*,
not a scope change.

plan-108 and §2's "man memory-vocabulary hits: **0 unclassified**; man
internals-vocabulary hits: **0**" were measured over the 31 registry packages
only, because `man-census.sh` had no notion of the guide topics at all. §1
required a `--topics` mode; no Phase checkbox carried it, so one was added
(see Phase 1's appended task) and the sweeps were widened.

Whole-surface, at HEAD:

- `./scripts/man-census.sh --memory-scope` → **109 unclassified** memory-
  vocabulary hits, every one of them in a guide topic: `tour` 45, `types` 39,
  `optimizations` 8, `link` 8, `lambda` 7, `errors` 2. They are not marginal —
  `tour` opens with "built around value **ownership**: every value has a single
  **owner**", `types` says a Map "stores its keys and values in one contiguous
  **allocation**", `lambda` teaches an explicit **borrow** model.
- `./scripts/man-census.sh --scope` → **9** internals-vocabulary hits:
  `optimizations` 5, `tour` 3, `types` 1 (e.g. `types` line 26,
  "monomorphized before code is generated").

These are handed to the letters that own those topics (B iteration 1, F
iteration 2, G iteration 3), not fixed here. Phase 1's job is the denominator.

### C-6 (Phase 1) — carve-out 3: the generated optimizer-catalog table

Widening `--scope` to the topics turned up 36 hits on `optimizations`, of
which **27 are not page prose**: `src/cli/man.rs:render_topic_overview`
substitutes `{{optimizer-catalog}}` with
`optimizer::catalog::render_markdown_table()` at display time, exactly so the
page and the compiler cannot disagree about which passes exist. Its Stage
column is literally `NIR` / `MIR` / `regalloc` / `codegen`, and no page author
can edit any of it — the same shape as plan-108-E's carve-out 2 for derived
`Errors` rows.

Added as **carve-out 3**, counted and printed separately, never dropped. It is
bounded by the two rendered headings around the marker and carves only
box-drawing table ROWS inside that region, so the authored sentences in the
same section ("Stage says where the pass runs: NIR …") remain HITs — which is
the point, since those are prose a reviewer can rewrite. 36 → 9 real hits.

### C-8 (Phase 3) — a nonexistent package does NOT make a reviewer run fail

Phase 3's self-test called for "one unit that must fail (a nonexistent
package)". Measured, it does not: `codex exec` on `man-pkg:NOSUCHPACKAGE` runs
`mfb man NOSUCHPACKAGE`, sees the error, reports it, and **exits 0** with a
perfectly good findings file:

```
UNIT: man-pkg:NOSUCHPACKAGE
FIRST: ERROR
```

That is the reviewer behaving correctly, and it is worth knowing for a
different reason: **a unit whose target does not exist will sail through this
harness as a clean review.** The harness cannot detect it, because the harness
is target-agnostic by design. The defence is the unit LIST: every letter
derives its units from a census command, never by hand, and `--reconcile`
proves list and manifest agree. A typo in a hand-written unit list is the one
way a page can be skipped and still look reviewed.

The self-test's deliberate failure is therefore induced by the watchdog
(`--timeout 5` → exit 124), which exercises the same path the real failures
will take: a run that dies, is re-queued once, and is recorded `FAILED`.

### C-9 (Phase 3) — two harness defects the self-test caught

Both were found by running the self-test, not by reading the script, and both
would have silently corrupted a letter's accounting:

1. **`--reconcile` read the FIRST manifest row for a unit.** A re-run appends
   rather than rewriting, so a unit that failed and was then re-run
   successfully keeps its stale `FAILED` row — and reconcile would report it
   forever, with no way to close the letter. Now reads the last row.
2. **macOS has no `timeout`(1)** (`command -v timeout` → empty) and ships
   **bash 3.2**, where `wait -n` does not exist. The first draft used both. A
   missing `wait -n` does not error — it falls through to a bare `wait` that
   blocks on *all* jobs, which would have collapsed the fan-out into lockstep
   batches of N with nothing to notice. Both are now spelled out: an explicit
   watchdog subshell (reporting 124, the code GNU `timeout` uses, so the
   manifest reads the same on either platform), and a polled free-slot table.

A third was designed out rather than found: the first draft assigned unit *n*
to worktree *n % JOBS*. That is not the same as a slot pool — when one review
runs long, round-robin still hands the next unit to its worktree, so two
`codex` processes share one working tree, interleave their probe builds, and
each sees the other's files in `git status`. The `DIRTY` check would report
noise and the reset would wipe a live run's scratch. Slots are now released by
PID.

### C-7 (Phase 1) — the spec is bigger than the plan measured

`./scripts/spec-census.sh --fill` at HEAD: **146 files** (unchanged),
**27,261 lines / 230,854 words** (plan: 26,482 / 223,085), **366 code fences**
(plan: 363), **2,021 citations** (plan: 1,970). The `unicode` pilot package is
**3 files / 571 lines**, not 508. Iteration-2 spec units are still **146**, so
no spec letter is re-scoped.

## Summary

The engineering risk is not in any single page — it is in **accounting across
897 runs**, which is why the harness's manifest and `--reconcile` are built
and self-tested (Phase 3) *before* a single review unit is dispatched, and why
the pilot (Phase 5) produces measured per-unit costs rather than an estimate.
The second risk is the spec's 61 broken symbol citations, one third of which
are stale *by deletion* — meaning the claim, not just the link, is suspect;
Phase 1 splits those two classes so letters I–N triage them differently.

Untouched by this letter: every man page, every spec topic, and every line of
compiler code except the one optional `render_all_markdown` change.
