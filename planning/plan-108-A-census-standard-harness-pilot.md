# plan-108-A: man-content census, the developer-doc standard, the example harness, and the workflow pilot

Last updated: 2026-08-24
Effort: large (3h–1d)
Overall Effort: huge (> 3d) — plan-108 spans A–F
Depends on: nothing (first letter)

`mfb man` renders every builtin package straight from the clean-room registry
descriptors in `src/codegen/builtins/**` (`src/cli/man.rs:1-15` — the old
`src/docs/man` Markdown tree was retired to `planning/old_man`). The prose
fields on those descriptors are the product's developer documentation, and
they are in a measured half-state: **466 function pages render, 272 carry a
Description + Example, 194 carry neither** — nine whole packages are bare
auto-derived skeletons. Nothing anywhere compiles or runs the examples the
field's own doc comment calls "runnable" (`src/codegen/registry/mod.rs:405`).

Plan-108's end state (delivered across A–F): every builtin man page is
**accurate to the actual code, written for the MFBASIC developer (never
compiler-internals spec), carries a runnable harness-verified example, and has
survived an independent cross-model review**.

This letter builds the machinery every later letter runs on: the exact
census, the content standard, the example harness, and the four-step
per-package workflow (accuracy pass → scope pass → cross-model subagent
review → apply findings), piloted end-to-end on one filled package (`bits`)
and one empty package (`thread`).

References:

- `src/cli/man.rs` — the renderer; empty sections are omitted
  (`man.rs:357,385,393` — `intro/desc/example.is_empty()` gates), so a
  rendered census is ground truth for fill state.
- `src/codegen/registry/mod.rs:401-406` — `RegistryFunction { intro, desc,
  example }`; `:147-148` `Parameter::desc`; `:531,556,607` — record / prop /
  resource `description` (the `mfb man <pkg> types` page content).
- `planning/old_man/builtins/**` — 543 retired pages
  (`find planning/old_man/builtins -name '*.md' | wc -l` → 543): the prose
  source material AND a warning — their `[[path:symbol]]` citations pre-date
  the plan-102/103 code motion and must never be trusted or leaked.
- `.ai/man_template.md` / `scripts/update_man.sh` — tooling for the RETIRED
  tree; F retires/replaces it. AGENTS.md's man-page guidance points at the
  retired tree too (F updates it).
- `tests/cli_man_summary_plain.rs` — the one existing man test (plain
  rendering of listing summaries).
- Memory `resources-in-collections-yes-records-no` — a KNOWN accuracy defect:
  the `mfb man process` blurb claiming a resource "cannot be stored as a
  collection element" contradicts spec §15.6. E fixes it; A's standard uses
  it as the canonical accuracy-failure example.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| release `mfb` at current HEAD | `cargo build --release` then `ls -l target/release/mfb` mtime ≥ HEAD commit date | re-verify at kickoff |
| no concurrent letter of plan-104-C/D touching the same builtins package | check active worktrees/sessions | coordinate at kickoff |

Interaction with plans 104–107: plan-108 edits ONLY prose string fields
(`intro`/`desc`/`example`/param `desc`/type `description`) in files that
plan-104-C/D will separately rewrite for typed internals. The edits are
disjoint in content and usually merge clean, but do not work the SAME package
in both plans at the same moment. 108 has no ordering dependency on 104–107.

## 1. Goal

- **The census is committed** (a table in this file): for each of the ~28
  registry packages — function-page count, and per page the fill state of
  `intro`, `desc`, `example`, per-parameter `desc`, plus the types-page
  description fill; produced by a script over rendered output (renderer
  omission gates make rendering the truth), kept as
  `scripts/man-census.sh` so every later letter re-runs it verbatim.
- **The content standard exists** as `.ai/man-content.md`: what a man page
  must contain and what it must never contain (see Design). Every later
  letter's accuracy/scope passes and every cross-model reviewer cite it.
- **The example harness exists and is wired**: `tests/man_examples.rs`
  iterates the registry; for every function in an *enforced* package it
  asserts a non-empty example that **compiles**, and (for packages on the
  run-list) **runs to exit 0** via the release binary (`mfb_exe()`
  convention — tests run `target/release/mfb`, see memory
  `mfb-exe-tests-use-release-binary`). Enforcement is an explicit per-package
  list that grows as letters land; at A's end it contains `bits` + `thread`.
- **The pilot is complete**: `bits` (17 filled pages) has been through the
  full accuracy + scope + cross-model review + fix cycle; `thread` (13 empty
  pages) has been authored from scratch (old_man as source material) through
  the same cycle. Both packages' overview and types pages included.
- **The cross-model review workflow is written down** (in this file, §3) and
  calibrated by the pilot: reviewer model, prompt, structured-findings
  format, and the triage rule.
- The `errorcode` and `perf` anomaly is resolved: both rendered **0 function
  pages** in the census run (`mfb man perf --all`) — determine whether they
  are prose-guide topics, empty packages, or a renderer gap, and record the
  answer here (it sets whether any letter owns them).

### Non-goals (explicit constraints)

- **Codegen untouched**: prose fields are `&'static str` never read by
  lowering; `artifact-gate.sh all` must stay byte-identical every letter.
  Several builtins files carry byte-significant MFBASIC bodies
  ("Body byte-significant … do not reformat", e.g.
  `src/codegen/builtins/collections/func_sort_by.rs:2`) — prose edits never
  touch bodies, and the gate is the guard.
- No renderer changes except what the harness strictly needs (none
  expected); no registry schema changes (no new fields).
- `src/docs/man/**` prose guides (tour, errors, link, lambda, …) are OUT of
  plan-108's scope — this plan is the builtins registry prose only.
- No wording churn for its own sake on accurate, in-scope pages.

## 2. Current State

`mfb man <pkg> [fn]` renders package `intro`/`desc`, each function's
`intro`/`desc`/`example` + auto-derived Declaration/Parameters/Errors tables
(from the `Implementation` descriptors — those tables are correct by
construction), and `mfb man <pkg> types` renders record/resource
descriptions.

### Measured populations

| What | Count | Command |
|---|---|---|
| function pages rendered | 466 | python census over `./target/release/mfb man <pkg> --all` for all 28 packages, splitting on function-page headers (2026-08-24 run, HEAD 254506f9b) |
| pages with Description + Example | 272 | same census (`\nDescription\n`/`\nExample` section presence; renderer omits empty — `man.rs:385,393`) |
| pages with NEITHER | 194 | 466 − 272 |
| all-empty packages | 9: strings 39, term 25, net 23, http 19, astrings 18, general 18, vector 17, thread 13, testing 12 (= 184; remaining 10 empties are stragglers inside filled packages) | same census, desc column = 0 |
| filled packages needing verification | 16: datetime 45, fs 42, encoding 28, collections 24, math 21, bits 17, crypto 17, os 15, io 15, process 15, audio 12, tls 10, json 5, csv 5, money 4, regex 4, app 3 (−1 straggler each in several) | same census |
| function-level `intro` fill | UNMEASURED precisely (crude source greps conflict: 122 `intro: ""` literals in func files; rendered `strings::mid` shows no intro line) — the census script's first output nails it | Phase 1 |
| packages rendering 0 function pages | 2 (`errorcode`, `perf`) | same census — anomaly, resolved in Phase 1 |
| retired source-material pages | 543 | `find planning/old_man/builtins -name '*.md' \| wc -l` |
| tests executing examples | 0 | `rg -rn 'example' tests/ src/cli/man.rs` — only `cli_man_summary_plain.rs` touches man at all |
| builtins func files | 419 (some are impl-only, e.g. `collections/func_sort_by.rs` holds a native fast path, its descriptor prose lives elsewhere) | `ls src/codegen/builtins/*/func_*.rs \| wc -l` |

### Verified properties

- The renderer omits empty prose sections (`src/cli/man.rs:357,385,393`) —
  VERIFIED by read; this is why rendered output is the honest census surface
  (per memory `rename-census-by-grep-underreports`: grep RENDERED output,
  not source, when spellings vary).
- Declaration/Parameters/Errors tables derive from the same descriptors the
  compiler executes — VERIFIED (`src/cli/man.rs` module doc + render fns);
  so *those* are accurate by construction and the audit targets PROSE claims.
- old_man prose quality is high but citation-stale — VERIFIED by sample:
  `planning/old_man/builtins/strings/mid.md` is excellent developer prose
  (and matches the known `mid` raises-not-clamps behavior) but cites
  `src/target/shared/code/builder_search.rs:lower_mid`, a pre-migration
  location.

## 3. Design Overview

Four artifacts, then the pilot proves them.

**(1) `scripts/man-census.sh`** — renders every package (`--all` + `types`),
emits a per-package, per-function fill table (intro/desc/example/param-desc).
Deterministic output committed alongside each letter's close.

**(2) `.ai/man-content.md` — the standard.** Contents (drafted here, finalized
in Phase 2):

- *A man page is written for the MFBASIC developer at the terminal.* Test:
  if a sentence only matters to someone reading compiler source, it belongs
  in `src/docs/spec/**` or `.ai/**`, not man.
- MUST contain: what the function does in MFBASIC terms; parameter semantics
  (units, valid ranges, zero/empty behavior); return semantics; every
  raisable error and the condition that raises it (consistent with the
  auto-derived Errors table); sharp edge cases (clamps vs raises, Unicode
  scalar vs grapheme, mutation vs new-value, ordering/stability); one
  runnable example; cross-references to sibling functions where a developer
  would reach for the wrong one (`left`/`right` clamp, `mid` raises).
- MUST NOT contain: registry/descriptor/lowering/monomorph/ABI vocabulary;
  helper or mangled symbols (`#pkg_…`, `__pkg_…`, `$T` suffixes);
  `Body::`/`abi_inline`/NIR/IR/`.ncode`/codegen mechanics; Rust
  implementation details; plan/bug numbers; `[[path:symbol]]` citation
  markers (old_man artifacts — strip on port, never re-derive into prose).
- Accuracy rule: **every behavioral claim is verified by running a program
  against the release binary or by the descriptor tables** — old_man text is
  source material, never authority (behavior may have changed since
  retirement; citations there are stale by construction).
- The canonical failure example: the `process` overview's
  resources-in-collections claim (wrong vs spec §15.6).

**(3) `tests/man_examples.rs` — the harness.** Iterates `registry()`; for
each enforced package: assert non-empty `example` per function; write it to
a temp project; compile with the release binary; run-list packages also
execute (exit 0). Env-dependent packages (net, http, term, audio, io-stdin,
process-spawn, tls, fs where sandboxed) are compile-only — the per-package
run/compile classification is an explicit table in the test, extended by
each letter. Temp dirs under `/tmp` (never a real dir — memory
`test-accept-second-arg-is-rm-rf-scratch` class of trap).

**(4) The four-step per-package workflow** (used by A's pilot and every
later letter):

1. **Accuracy pass** — for each page: check every prose claim against the
   implementation and by running probe programs; fix or excise. Port from
   old_man where the page is empty, re-verifying each ported claim.
2. **Scope pass** — apply `.ai/man-content.md`'s MUST-NOT list; rewrite
   internals-speak into developer terms or delete it.
3. **Cross-model review** — spawn an independent subagent **on a different
   model than the authoring session** (this session is Fable; reviewers run
   `model: opus`), one per package, `subagent_type: general-purpose`. Input:
   the package name only. Instructions: render `mfb man <pkg> --all` and
   `mfb man <pkg> types`; independently verify every factual claim against
   the code and by running MFBASIC programs; flag (a) inaccuracies with
   evidence, (b) internals leakage per `.ai/man-content.md`, (c) missing
   developer-critical information (unlisted error conditions, surprising
   edge cases). Output: a structured findings list
   (function / claim / verdict / evidence).
4. **Apply** — triage each finding on the main thread: confirmed → fix;
   rejected → record WITH the disproving command in the letter's ledger.
   Re-run the census + harness + `cargo test --no-fail-fast` for the
   touched package.

**Risk concentration:** (a) porting old_man prose without re-verification —
held by the accuracy rule + the cross-model reviewer being prompted to
verify, not proofread; (b) prose edits straying into byte-significant
descriptor/body code — held by `artifact-gate.sh all` byte-identity every
letter.

### Rejected alternatives

- **Restore the old Markdown tree instead of filling registry fields.**
  Rejected: the registry renderer is the shipped design (`man.rs` module
  doc); two sources would immediately diverge and the tables are already
  derived from the descriptors.
- **One giant end-of-plan review instead of per-package review in each
  letter.** Rejected: findings arrive after the author-context is gone;
  per-package review keeps the fix loop short and lets the reviewer verdicts
  calibrate the very next package.
- **Trust old_man citations to locate implementations.** Rejected: they
  pre-date the builtins migration (sampled stale).

## Compatibility / Format Impact

None to codegen/wire (prose strings only). `mfb man` rendered output changes
are the deliverable. `cli_man_summary_plain.rs` may need its pinned summary
text updated ONLY if a pinned package summary is itself corrected (record
per the 4-question gate — proving the old text wrong is the required
evidence).

## Phases

### Phase 1 — census tooling + the exact census

- [ ] Write `scripts/man-census.sh`; run it; commit the per-package
      fill table into this file (replacing the UNMEASURED intro row).
- [ ] Resolve the `errorcode`/`perf` zero-page anomaly; record the answer
      and assign ownership (a later letter's package list, or out of scope
      with the reason).
- [ ] Tests: script is deterministic (two runs, identical output).

Acceptance: census table in this file; anomaly resolved in writing.
Commit: —

### Phase 2 — the standard + the harness

- [ ] Author `.ai/man-content.md` per §3 (2).
- [ ] Author `tests/man_examples.rs` per §3 (3), enforced list = `[]`
      initially; prove the plumbing on one hand-written example.
- [ ] Tests: `cargo test --no-fail-fast` green (harness inert but wired).

Acceptance: standard committed; harness runs in the suite.
Commit: —

### Phase 3 — pilot: `bits` (verify-filled) + `thread` (author-empty)

- [ ] `bits`: accuracy pass + scope pass over its 17 pages + overview +
      types page; examples onto the harness run-list (bits is pure —
      run-enforced).
- [ ] `thread`: author all 13 pages (+ overview check, types page) from
      code + old_man source material, every claim behavior-verified;
      examples run-enforced (thread examples spawn and join).
- [ ] Cross-model review (opus, one agent per package) + apply findings;
      record the findings ledger (confirmed/rejected + evidence) here.
- [ ] Add `bits`, `thread` to the harness enforced list.
- [ ] Tests: full `cargo test --no-fail-fast`;
      `scripts/artifact-gate.sh <exe> all` byte-identical; census re-run
      shows both packages at 100% fill.

Acceptance: both pilot packages 100% filled/verified/reviewed; harness
enforces them; gate byte-identical; workflow §3 amended with anything the
pilot taught (reviewer prompt fixes, triage rules).
Commit: —

## Validation Plan

- Tests: `cargo test --no-fail-fast` (memory: plain `cargo test` fail-fast
  skips `rt_*`); `tests/man_examples.rs` for the enforced list.
- Coverage check: census script output = the denominator; pilot packages at
  100% fill.
- Runtime proof: examples execute via the release binary; behavior probes
  run during the accuracy pass.
- Doc sync: `.ai/man-content.md` is NEW doc; AGENTS.md/template retirement
  deferred to F (recorded there).
- Acceptance: full suite; `artifact-gate all`; `test-accept.sh` no NEW
  mismatch; `rustup run 1.96.0 cargo fmt --all` + repository crate fmt.

## Open Decisions

- **Reviewer model**: `opus` recommended (strongest non-Fable tier
  available to the Agent tool; genuine cross-model independence). `sonnet`
  is the fallback if opus availability/cost is a problem — record the
  choice after the pilot.
- **Function-level `intro` policy**: the one-line intro under the title is
  empty nearly everywhere; recommend REQUIRED (one sentence, distinct from
  desc's first line) — decide in Phase 2 when writing the standard, and the
  harness then enforces non-empty intro too.

## Corrections

<Filled in during execution.>

## Summary

The machinery letter: an honest rendered census, a written standard for
"developer docs, not compiler spec", a harness that makes every example a
tested artifact, and a four-step workflow proven on one filled and one empty
package — so B–F are production-line letters, not design work.
