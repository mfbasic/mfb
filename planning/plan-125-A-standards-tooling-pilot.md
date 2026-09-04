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
| release `mfb` at current HEAD (every unit of every letter renders pages and compiles probes with it) | `cargo build --release`; `ls -l target/release/mfb` mtime ≥ HEAD commit date | **UNVERIFIED at plan-writing** — binary is `2026-09-04 10:36`, HEAD `90f6c1357` is `2026-09-04 09:05`, but the working tree carries uncommitted `term` changes. Rebuild and re-check before Phase 1. |
| `codex` CLI installed and non-interactive | `~/local/bin/codex --version` → `codex-cli 0.153.0` | **MET** 2026-09-04 (plan-108 ran `0.150.0`; not pinned — see plan-108-A Open Decisions, "do NOT pin") |
| the working tree is clean of unrelated in-flight work, or the in-flight work is on another branch | `git status --porcelain` | **NOT MET** 2026-09-04 — 40+ modified files from an in-flight `term` change. plan-125 edits prose string fields in the same `src/codegen/builtins/term/` files; land or park that work first, full stop. |
| no peer session is mid-flight in `src/codegen/builtins/**` or `src/docs/spec/**` | `git worktree list` (12 worktrees exist as of 2026-09-04) + ask each peer session directly, per memory `peer-sessions-share-main-checkout` | **NOT MET** — 12 worktrees exist; ask before starting. |

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
  `tests/cli_man_summary_plain.rs` and `tests/cli_canvas_man_examples_compile.rs`
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

All commands run 2026-09-04 at HEAD `90f6c1357` with `target/release/mfb`
(mtime 10:36).

| What | Count | Command |
|---|---|---|
| renderable man packages | 31 | `./scripts/man-census.sh --fill` → 31 package rows (32 dirs under `src/codegen/builtins/` minus `perf`, which is not an MFB package — `mfb man perf` → `unknown package`, resolved in plan-108-A §"the errorcode/perf anomaly") |
| man function pages | **538** | `./scripts/man-census.sh --fill` → `TOTAL 538 538 538 538 884/884` |
| man parameter descriptions | 884 | same row |
| man package overview pages | 31 | one per renderable package (`PKGDOC` column `11` for all 31) |
| man `types` pages | 20 | census `TYPES` column non-`-` |
| narrative guide topics | 10 | `ls src/docs/man/` minus `mod.rs` |
| narrative guide **pages** (topic + subtopics) | 32 | `find src/docs/man -name '*.md' \| wc -l` → 32 (flow 8, types 10, tour 6, tooling 2, and 6 single-page topics) |
| narrative guide lines | 3,924 | `cat $(find src/docs/man -name '*.md') \| wc -l` |
| **man review units, iteration 1 / 3** | **41** | 31 packages + 10 topics |
| **man review units, iteration 2** | **621** | 538 function + 31 overview + 20 types + 32 guide pages |
| rendered `mfb man --all` | 51,540 lines | `./target/release/mfb man --all \| wc -l` |
| function pages **missing** from `mfb man --all` | 30 | `testing` 12 + `general` 18 — `render_all_markdown` filters `is_unqualified_global()`; `mfb man --all \| grep -cE '^(TESTING\|GENERAL)$'` → 0 |
| guide pages missing from `mfb man --all` | 32 | all of them; `--all` renders registry packages only |
| man fill state | 100% | census: `pages with neither Description nor Examples: 0` |
| man memory-vocabulary hits | **0 unclassified** (15 carve-out 1 datetime arithmetic borrow, 23 carve-out 2 derived Errors rows) | `./scripts/man-census.sh --memory-scope` |
| man internals-vocabulary hits | 0 | `./scripts/man-census.sh --scope` |
| leaked `[[` in rendered man | 0 real (5 hits, all nested-list literals `[["name","age"]]`) | `./target/release/mfb man --all \| grep -n '\[\['` — read all 5 |
| packages changed since plan-108 closed | 27 of 31 | `git log --since=2026-08-31 --name-only --format='' -- src/codegen/builtins \| grep -oE 'builtins/[a-zA-Z]+/' \| sort -u \| wc -l` |
| spec packages | **12** | `sed -n '43,56p' src/docs/spec/mod.rs` (`PACKAGE_ORDER`) |
| spec files | **146** | `find src/docs/spec -name '*.md' \| wc -l` |
| spec lines / words | 26,482 / 223,085 | per-dir `cat */*.md \| wc -l`; `cat $(find src/docs/spec -name '*.md') \| wc -w` |
| **spec review units, iteration 1 / 3** | **12** | one per package |
| **spec review units, iteration 2** | **146** | one per file |
| spec `[[ ]]` citations (total / unique) | 1,970 / 1,414 | `grep -rhoE '\[\[[^]]+\]\]' src/docs/spec --include='*.md' \| wc -l`; `… \| sort -u \| wc -l` |
| unique citations that are file/dir only (no suffix) | 115 | §2 citation script, `nosuffix` counter |
| unique citations with a line-range suffix | 19, **0 out of range** | same script, `lin`/`linbad` |
| unique citations with a symbol suffix | 1,280 | same script, `sym` |
| **symbol citations whose symbol is not in the cited file** | **61** | same script, `symbad` — the rot baseline plan-125 drives to 0 |
| citations with an unresolvable path | 2 (`finalize_vreg_body_with_locals`, `run_register_allocation` — malformed: symbol with no path) | path-existence loop over the unique list |
| spec code fences | 363 | `grep -rc '^```' src/docs/spec --include='*.md' \| awk -F: '{s+=$2} END {print s/2}'` |
| distinct `mfb spec <pkg>` link targets, all resolving | 12 / 12 | loop over `grep -rhoE 'mfb spec [a-zA-Z-]+'` targets, `-d src/docs/spec/$t` |
| `mfb spec --all` (global) | **renders 0 lines** | `./target/release/mfb spec --all \| wc -l` → 0; only per-package `mfb spec <pkg> --all` works (`mfb spec language --all \| wc -l` → 5,604) |
| **review units across the whole plan** | **869** | man 699 (A pilot 31 + B 39 + C 150 + D 142 + E 142 + F 156 + G 39) + spec 170 (A pilot 5 + I 11 + J 47 + K 45 + L 51 + M 11) |
| **total `codex exec` runs plan-125 will make** | **897** | 869 unit runs + 16 consistency runs (4 each in B, G, I, M) + 12 final-lens runs (6 in H, 6 in N) |

### Verified properties

- **`mfb man --all` is not the whole developer manual** — VERIFIED by reading
  `src/cli/man.rs:render_all_markdown` (filters `is_unqualified_global()`) and
  by grep: `TESTING`/`GENERAL` headers appear 0 times in the output, and no
  guide-topic body text appears. 62 of the 621 man pages (10%) are outside it.
  This directly affects the final gate the user asked for; §4.1 resolves it.
- **The 61 unresolved symbol citations are two distinct rot classes** —
  VERIFIED by spot-check, not inferred: `__http_dechunk` is cited as
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
   sorted order), it needs `tests/cli_man_summary_plain.rs` re-checked, and it
   is the **only** renderer change plan-125 permits. `testing`/`general` stay
   filtered for the documented reason and remain script-only.

### 4.2 `scripts/spec-census.sh`

Modelled on `scripts/man-census.sh` (deterministic output, no timestamps, no
paths that vary, `LC_ALL` set — its header comments explain why). Modes:

- *(default)* **`--fill`** — per-package inventory: files, lines, words, code
  fences, citation count, cross-link count; a `TOTAL` row. The denominator
  every spec letter reconciles against.
- **`--citations [pkg…]`** — the instrument that does not exist today.
  For each unique `[[…]]`: split on the **last** `:`; verify the path exists
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
  the plan. `tests/cli_man_summary_plain.rs` and
  `tests/cli_canvas_man_examples_compile.rs` pin some rendered text; a letter
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

- [ ] Rebuild `cargo build --release`; confirm the binary post-dates HEAD.
- [ ] Re-run `./scripts/man-census.sh --fill`, `--functions`, `--memory-scope`,
      `--scope`; paste all four outputs into §2 of this file, replacing the
      2026-09-04 figures if they moved.
- [ ] Write the §2 citation-measurement script into `scripts/spec-census.sh`
      as `--citations` (§4.2) and confirm it reproduces `MISS-SYMBOL 61`,
      `MISS-PATH 2`, `MISS-LINE 0`. A different number is a Correction, not a
      quiet edit.
- [ ] Classify all 61 `MISS-SYMBOL` hits into *stale by move* vs *stale by
      deletion* using the "exists anywhere in `src/`" column; record both
      counts here. The deletion class is a list of **suspect claims** handed
      to letters I–N, not just broken links.
- [ ] Decide §4.1: does `mfb man --all` render guide topics? Record the
      decision and, if yes, make the one permitted renderer change and re-run
      `cargo test --bin mfb man` + `tests/cli_man_summary_plain.rs`.
- [ ] Write `scripts/man-manual.sh`; confirm its output covers all 621 man
      pages (assert the page count against the census).

Acceptance: `scripts/spec-census.sh --citations` runs and prints the measured
baseline; `scripts/man-manual.sh` output contains a header for all 31
packages **and** `testing`, `general`, and all 10 topics
(`./scripts/man-manual.sh | grep -cE '^(TESTING|GENERAL|A TOUR OF MFBASIC)$'`
→ 3); §2's tables in this file are the numbers those commands just printed.
Commit: —

### Phase 2 — The two content standards

- [ ] Extend `.ai/man-content.md`: state the audience in one line at the top
      (the §3.1 row); add a section governing the **narrative topics** — the
      same MUST/MUST-NOT list, the same memory ban, the same rule that every
      code block is compiled and run; note that topics have subtopic pages and
      that `mfb man <topic>` is the verification command.
- [ ] Author `.ai/spec-content.md` — the contributor-audience review standard:
      what a topic must contain (normative contract, `[[ ]]` provenance at
      claim-cluster granularity, the as-is rule), what it must not (tutorial
      prose, marketing, a second full copy of another topic's body,
      unverifiable or aspirational claims), how to triage a spec/code
      disagreement (§3.5), and the two rot classes from Phase 1 with the rule
      that a *stale-by-deletion* citation makes the claim suspect.
- [ ] Cross-reference the two standards at the seam: `.ai/man-content.md`
      points at `.ai/spec-content.md` for "this belongs in spec", and back.
- [ ] Create `planning/plan-125-belongs-in-spec.md` with its header and empty
      table (§3.1); letters B–H append, I–N consume.
- [ ] Update AGENTS.md's "Creating or updating `mfb man` content" section to
      name `.ai/spec-content.md` alongside `.ai/man-content.md`, and to say
      which audience each serves.

Acceptance: both standards exist and each states its audience in its first 10
lines; `grep -n 'spec-content' AGENTS.md .ai/man-content.md` returns hits in
both; the ledger file exists.
Commit: —

### Phase 3 — The fan-out harness

- [ ] Write `scripts/doc-review-fanout.sh` per §4.3: unit list, prompt
      template, concurrency, N reusable detached worktrees with no `target/`,
      `MFB=` pointing at the primary release binary, per-unit findings file,
      `manifest.tsv` with exit/seconds/findings/dirty/banner, one re-queue then
      `FAILED`, per-run timeout, `/tmp/plan-125/<unit>/` scratch.
- [ ] Self-test it on a 3-unit list including **one unit that must fail**
      (a nonexistent package) and confirm the manifest records `FAILED` rather
      than dropping it — memory
      `diagnostic-harness-must-record-exit-and-unlocated-errors`.
- [ ] Self-test the dirty-worktree path: have a run touch a file, confirm
      `DIRTY` is recorded and the worktree is reset before reuse — memory
      `subagent-edits-can-silently-vanish`.
- [ ] Add a `--reconcile` mode: given a unit list and a manifest, print any
      unit missing, `FAILED`, or without a findings file; exit non-zero if any.
      Every letter runs this before it closes.

Acceptance: the 3-unit self-test manifest has one `FAILED` row and
`--reconcile` exits non-zero on it and zero after the re-run; a deliberately
dirty run is recorded `DIRTY`.
Commit: —

### Phase 4 — The eight reviewer prompts

- [ ] Write all eight prompts verbatim into §5 of this file (man 1/2/3 +
      final; spec 1/2/3 + final), each stating: the audience, the lens, the
      standard file to read, the rendering command, the verification duty
      (read the code; run probes; **do not** attempt to bind a socket — the
      main thread runs network probes), the structured findings format
      (`unit / claim / verdict / evidence / suggested wording`), and the
      instruction to make **no repository edits**.
- [ ] Store each prompt also as a file under `planning/plan-125-prompts/` so
      the harness can pass it with `-`; the file and §5 must match (a
      `diff` check in the pilot).

Acceptance: eight prompt files exist; `diff` between each file and its §5
block is empty.
Commit: —

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
     Correction. -->

## Validation Plan

- **Tests**: none for man prose (Non-goals). For any spec letter:
  `cargo build`, `cargo test --bin mfb spec`; plus `cargo test errorcode` if
  `diagnostics/02_error-codes.md` changed. For the §4.1 renderer change (if
  taken): `cargo test --bin mfb man` and `tests/cli_man_summary_plain.rs`.
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
