# plan-125-F: man iteration 2, batch 4 — the resource family, the small packages, and the 31 guide pages (156 pages)

Last updated: 2026-09-04
Effort: x-large (1d–3d)
Depends on: plan-125-E (iteration 2 is one ordered pass).

Landing unit: **each Phase below is independently landable and gets its own
commit.** The letter totals x-large; it is never landed as one change, and a
session that lands one phase and stops has left the tree consistent.

Iteration 2, final batch. Three groups, deliberately together:

1. **The `RES` handle family** — `process`, `tcp`, `udp`, `tls`, `net`,
   `audio`, `thread`. These pages share one contract (open / stays open /
   closes) and plan-108-F recorded them as the sharpest divergence test in the
   surface. Reviewing them in one letter is the only way the wording lands
   identical.
2. **The small packages** — `testing`, `csv`, `json`, `money`, `regex`, `app`,
   `errorCode`.
3. **The 31 guide pages** (10 topics minus `variable`, done in plan-125-A's
   pilot) — the pages a learner reads *first*, and the least-audited material
   in the product: plan-108 excluded them entirely.

The unit is one page. See plan-125-A §3.2.

References:

- plan-125-A §3.2/§3.3/§4.3/§5; plan-125-B's terminology table and its
  guide-topic findings from Phase 1.
- `.ai/man-content.md` as extended by plan-125-A Phase 2 to govern the
  narrative topics.
- plan-108-A §3 (2a) rewrite table — this batch is where all 25 of the
  original "Borrowed, not consumed" parameter descriptions lived (process 7,
  tcp 6, audio 4, tls 4, udp 4). They are gone; verify the *replacements* say
  what happens to the handle.
- `.ai/net-tls.md` — foil for `tcp`/`udp`/`tls`/`net`.
- `.ai/resources-packages.md` — foil for the `RES` model; the man pages state
  the developer-visible half and link `mfb man variable`.
- Memory `process-waitfor-drains-into-spill-blocks`,
  `ignored-signal-disposition-survives-exec` — `process` behaviors a page can
  get subtly wrong.
- Memory `audio-play-is-48k-mono-and-never-converts` — 48 kHz mono only, never
  resampled; if the `audio` pages do not say this plainly, that is the
  highest-value single finding available in this letter.
- Memory `arena-state-is-per-thread`, `spawned-thread-entry-must-save-callee-saved`
  — `thread` internals (foil), and the developer-visible consequence (data
  published from one thread is not visible to another the way a developer
  might assume) which the page *must* state.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-E complete | `grep -c '^- \[ \]' planning/plan-125-E-man-iter2-batch3.md` → `0` | — |

## 1. Goal

- **All 156 pages** through my per-page pass, one `codex exec` each, and apply.
- **The handle contract is stated identically across all seven resource
  packages** — verified by extracting the contract sentence from every one and
  diffing them, not by grepping for banned words.
- **Every network probe run by the main thread**, never from a reviewer
  worktree (plan-108-C: the Codex sandbox cannot bind sockets). The ledger
  records which claims that covered.
- **All 31 guide pages verified and every code block in them compiled and
  run** — plan-125-A Phase 2 put them under the standard; this is the first
  time anyone checks them.
- Every finding has a verdict; every rejection a disproving command.
- `--reconcile` exits 0 over the 156-unit list; sweeps clean.
- "Belongs in spec" cuts appended.
- **Iteration 2 is complete across the whole man surface**: 590 pages in
  C–F plus 31 in A's pilot = 621, reconciled against the census.

### Non-goals (explicit constraints)

- Per plan-125-A. No cross-page reconciliation (letter G owns it).
- **No listening socket is bound from a reviewer worktree.**
- No wording churn on verified prose.
- The guide topics are reviewed, not rewritten; `tour` in particular is a
  deliberate narrative and its shape is not a defect.

## 2. Current State

The resource family was plan-108-C's and -E's work and is where the memory
ban did its heaviest rewriting; `testing` and `general` are invisible to
`mfb man --all`; the guide topics have never been reviewed by anyone.

### Measured populations

| What | Count | Command |
|---|---|---|
| `process` units | 16 | 14 pages + overview + types — `./scripts/man-census.sh --fill` |
| `audio` / `tcp` / `testing` / `thread` / `tls` units | 13 each | 11+1+1, 11+1+1, 12+1, 12+1, 11+1+1 |
| `udp` units | 10 | 8 + overview + types |
| `net` units | 7 | 5 + overview + types |
| `csv` / `json` units | 6 each | 4 + overview + types |
| `money` / `regex` units | 5 each | 3+1+1, 4+1 |
| `app` units | 4 | 2 + overview + types |
| `errorCode` units | 1 | overview only — it renders 0 function pages (constants, no functions) |
| package subtotal | **125** | sum |
| guide pages | **31** | 32 markdown files minus `variable` (A's pilot): tour 6, types 10, flow 8, tooling 2, errors/lambda/link/optimizations/unicode 1 each |
| **batch total** | **156** | 125 + 31 |
| parameter descriptions in batch | 208 | census sum over the 14 packages |
| type descriptions in batch | 75 | census: net 19, audio 17, json 12, csv 8, process 7, app 3, udp 3, tcp 2, tls 2, money 2 |
| `process` file-touches since plan-108 | 44 | `git log --since=2026-08-31 --name-only --format='' -- src/codegen/builtins/process \| wc -l` |
| `json` / `tls` file-touches since plan-108 | 41 / 33 | same |
| guide-page lines to verify | 3,676 | 3,924 total minus `variable`'s 248 |
| packages invisible to `mfb man --all` | `testing`, `general` | `render_all_markdown` filters `is_unqualified_global()`; `general` was reviewed in plan-125-E, `testing` here |

### Verified properties

- **The guide topics have never been reviewed** — VERIFIED by reading
  plan-108-A's Non-goals (they are explicitly out of scope, `variable` the
  sole carve-out). Letter B gave them a whole-topic pass; this is their first
  per-page one, and the first time their code blocks are compiled.
- **`errorCode` renders an overview and no function pages** — VERIFIED:
  `mfb man errorCode` renders; census `PAGES 0`, `PKGDOC 11`. Its single unit
  is the overview, and the thing to verify is that the constants it describes
  match `src/docs/spec/diagnostics/02_error-codes.md`'s registry — which is
  build input, so a mismatch is a real defect, not a wording nit.
- **The reviewer cannot bind sockets** — VERIFIED as a recorded lesson in
  plan-108-C ("the Codex sandbox cannot bind sockets", ~30 attempts). Designed
  around rather than rediscovered.

## 3. Design Overview

Per page: my pass → one `codex exec` → apply, `N` concurrency, main thread
sole writer.

**Order:** the resource family first, together, so the shared contract is
settled before anything else drifts from it; then the small packages; then the
guide pages last, because by then the whole package surface is final and a
guide page can be checked against it.

1. `net` 7 → `tcp` 13 → `udp` 10 → `tls` 13 → `process` 16 → `audio` 13 →
   `thread` 13 (75 units)
2. `errorCode` 1 → `app` 4 → `money` 5 → `regex` 5 → `csv` 6 → `json` 6 →
   `testing` 13 (40 units)
3. the 31 guide pages

**Risk concentration:**
- **Handle-contract divergence.** Seven packages, one contract. The check is
  mechanical and must be done as one act: extract the contract sentence from
  each of the seven overviews and each affected function page, put them in one
  table in this letter, and make them agree. plan-108-F recorded exactly this
  as the failure mode of a per-package process.
- **Deleting a true contract to pass a grep.** `--memory-scope` returning 0 is
  necessary and not sufficient; each of the seven packages is read for whether
  it still tells the developer when the handle closes.
- **Guide pages teaching outdated language.** `tour`, `types` and `flow`
  describe syntax and semantics that have moved; every code block is compiled
  and run, and every claim about the language is checked against the current
  compiler, not against the page's own confidence.
- **`errorCode` vs the build-input registry.** A mismatch between the overview
  and `src/docs/spec/diagnostics/02_error-codes.md` is a real defect on a real
  build input; if the *spec table* is the wrong side, that is a note for
  letters I–N, not an edit here.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial; `- [x] ~~text~~ — moot: <evidence>`
> rather than deleting; fill `Commit:` on landing. **Unticked means NOT DONE.**

### Phase 1 — the resource family (75 units)

- [ ] `net` 7, `tcp` 13, `udp` 10, `tls` 13, `process` 16, `audio` 13,
      `thread` 13 — one review unit per page.
- [ ] Build the **contract table** in this file: for each of the seven
      packages, the exact sentence(s) stating what happens to a handle on
      open, on pass, on close, and at scope exit. Make them agree; record the
      before/after.
- [ ] Verify the `audio` pages state the 48 kHz mono constraint plainly
      (memory `audio-play-is-48k-mono-and-never-converts`) — a format mismatch
      shows up as ~1.8× playback speed, which is exactly the surprise a man
      page exists to prevent.
- [ ] Verify the `thread` pages state the developer-visible consequence of
      per-thread arena state without describing arenas.
- [ ] All network probes run by the main thread; the ledger records which.
- [ ] Every example compiled and run; ledgers recorded.

Acceptance: 75 units `exit 0`; the contract table shows all seven packages
agreeing; `--memory-scope` 0 unclassified across all seven **and** a
read-through confirming each still states its close contract; sweeps clean.
Commit: —

### Phase 2 — the small packages (40 units)

- [ ] `errorCode` 1, `app` 4, `money` 5, `regex` 5, `csv` 6, `json` 6,
      `testing` 13.
- [ ] Reconcile `json`'s 41 file-touches since plan-108 against its pages
      first.
- [ ] Verify `errorCode`'s overview against
      `src/docs/spec/diagnostics/02_error-codes.md`'s Constant Registry; a
      mismatch is recorded and, if the spec side is wrong, handed to letters
      I–N rather than edited here.
- [ ] Verify the `regex` pattern-vs-escape boundary is stated precisely
      (plan-108-E's recorded finding on this package).
- [ ] Every example compiled and run; ledgers recorded.

Acceptance: 40 units `exit 0`; sweeps clean; `errorCode`'s overview
reconciled against the registry table with the result recorded either way.
Commit: —

### Phase 3 — the 31 guide pages

- [ ] `tour` 6, `types` 10, `flow` 8, `tooling` 2, and `errors`, `lambda`,
      `link`, `optimizations`, `unicode` (1 each).
- [ ] **Every code block in all 31 pages compiled and run**; the ledger
      accounts for each as ran or compile-only-with-reason.
- [ ] Every language claim checked against the current compiler — these pages
      predate a great deal of language work and nothing has ever checked them.
- [ ] `tooling` checked against the actual CLI surface (`mfb --help` and the
      subcommands), since it documents commands rather than language.
- [ ] `optimizations` checked against the real `-O` dial behavior; it is the
      guide topic most likely to have drifted into internals.
- [ ] Every cut appended to `planning/plan-125-belongs-in-spec.md`.

Acceptance: 31 units `exit 0`; every code block on every guide page compiled
and run with the result in the ledger, 0 unaccounted; `mfb man <topic>` and
`mfb man <topic> <subtopic>` render for all of them; `--reconcile` exits 0
over the whole 156-unit batch.
Commit: —

### Phase 4 — iteration 2 completeness

- [ ] Reconcile the union of A's pilot (31), C (150), D (142), E (142) and F
      (156) against the census: **621 units, 0 unaccounted**.
- [ ] Reconcile the union of all four letters' example ledgers against the
      census function list plus the guide code blocks: 0 unaccounted.
- [ ] Run the whole-surface sweeps: `--fill` 100%, `--memory-scope` 0
      unclassified, `--scope` 0.

Acceptance: the reconciliation prints 621 and 0; the three sweeps are at
target; this letter records the totals as the entry state for letter G.
Commit: —

## Validation Plan

- Tests: none (man prose); pinned-text tests updated in the same commit if
  touched.
- Coverage check: `--reconcile` over the 156-unit list, then the Phase 4
  cross-letter reconciliation to 621.
- Runtime proof: `scripts/man-run-examples.sh <pkg> --run` for all 14
  packages; every guide code block compiled and run; `mfb man <topic>` renders.
- Doc sync: `planning/plan-125-belongs-in-spec.md` appended.
- Acceptance: `--fill` 100%; `--memory-scope` 0 unclassified; `--scope` 0;
  `--reconcile` 0; 621 units reconciled.

## Open Decisions

- **Does `tour` get held to the same example rule as a function page?** —
  Recommend yes for *runnability* (every block compiles and runs) and no for
  *shape* (a tour block is deliberately partial and comparative; that is the
  topic working, not a defect).
- **If `errorCode`'s overview and the spec registry disagree, which is
  authoritative?** — The spec table is build input, so it is authoritative by
  construction; the man overview is corrected here, and whether the *table* is
  right about the compiler is a letter I–N question, recorded not resolved.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

Two things in this letter have never been checked by anyone: the 31 guide
pages, which are the first thing a learner reads, and the agreement of the
seven resource packages on a single handle contract, which plan-108 could not
achieve because it worked one package at a time. Everything else is
verification of material that has been reviewed before.
