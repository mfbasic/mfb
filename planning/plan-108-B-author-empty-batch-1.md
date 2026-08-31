# plan-108-B: Author the empty packages, batch 1 — strings, term, testing

Last updated: 2026-08-30
Effort: large (3h–1d)
Depends on: plan-108-A (census script, `.ai/man-content.md` standard, and the
pilot-calibrated four-step workflow all exist; A's Prerequisites and no-test
verification policy carry forward).

Author the man prose for the first batch of all-empty packages — **strings
(39 function pages), term (25), testing (12) = 76 pages** plus each package's
overview and types page — through plan-108-A's four-step workflow: accuracy
pass (author from code + old_man source material, every claim
behavior-verified, every example compiled and run while writing it), scope
pass (developer docs, never compiler internals), cross-model review via the
Codex CLI, apply findings.

Batch composition: strings is the highest-developer-traffic empty package;
term and testing round the batch to ~76 pages, and both stress the standard
in useful ways (term: interactive examples that can only be compile-verified
without a tty; testing: `expect` — prose must describe developer semantics
without leaking the desugar story).

See plan-108-A §3 for the workflow and the standard. Per A: verification is
`mfb man` rendering + ad-hoc example/probe runs — no compiler test gates.

References:

- **plan-108-A §3 (2a) — the memory-vocabulary hard ban.** Permitted:
  **copy**, **mutate**, **value**, **alias** (`RES` handles only).
  Banned from rendered output: `borrow`, `pointer`, `ownership`/`owns`,
  `move`, `free`, `heap`, `refcount`, `lifetime`, `deep/shallow copy`,
  `by reference`, `drop` (memory sense) — use A's rewrite table, and link
  `mfb man variable` instead of re-explaining the model on a package page.
  Run `scripts/man-census.sh --memory-scope <pkg>` before closing each
  package; record before/after counts in the ledger.
  Rendered baseline for this letter's packages (2026-08-30,
  `mfb man <pkg> --all | grep -cEi 'borrow|ownership|\bowns\b|pointer|deep
  copy|shallow copy|by reference|heap|refcount|dangling'`): strings 1,
  term 0, testing 0. Low — but these packages are being AUTHORED, so the
  risk is importing the vocabulary from `planning/old_man/**`, which
  predates the ban: strip it on port, do not carry it across. `term`
  pages describe a handle-free surface; if one needs to say a `RES` is
  shared, say **alias** and link `mfb man variable`.
- `src/codegen/builtins/strings/`, `…/term/`, `…/testing/` — the descriptor
  prose fields being filled.
- `planning/old_man/builtins/strings/` etc. — source material (claims
  re-verified, citations stripped; per A's accuracy rule).
- Known behavior sharp edges the strings pages MUST get right (and which
  make good reviewer bait): `strings::mid` raises `ErrIndexOutOfRange`
  rather than clamping (memory `mfb-strings-mid-raises-not-clamps`;
  old_man/builtins/strings/mid.md describes this correctly); string escapes
  are `\u{HEX}` not `\x{…}` (memory `mfb-string-escape-is-u-not-x`) —
  examples must not use non-escapes.
- `term::on` leaves ISIG enabled (`^C` = SIGINT, runtime restores the
  screen) — same memory; a term-page claim to verify, not assume.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-A complete | A's boxes ticked; census + standard committed | **MET** 2026-08-30 — every box in A's four phases resolved, every acceptance criterion verified, every `Commit:` line filled (`e30c538de`, `870b10e79`, `f816298ea`, `55b7565d8`). `scripts/man-census.sh`, `scripts/man-run-examples.sh`, `.ai/man-content.md` and `mfb man variable` all exist. |

## 1. Goal

- Every function page in strings, term, testing has non-empty `intro` (per
  A's intro policy), `desc`, `example`, and per-parameter `desc`; the
  package overviews and types pages are reviewed/corrected.
- Every claim behavior-verified (probe programs against the release binary
  or descriptor-table-derived); zero internals leakage per
  `.ai/man-content.md`.
- **`scripts/man-census.sh --memory-scope` reports 0** for every package in
  this letter (plan-108-A §3 (2a)): no `borrow`, `pointer`, `ownership`,
  `move`, `free`, `heap`, `lifetime` in rendered output. Where a `RES`
  handle's behavior must be stated, it is stated with **alias** and
  MFBASIC's own verbs (open / close / stays open); anything longer links
  `mfb man variable`.
- Every example compiled and run while authoring; term members that need a
  tty are compile-verified only, noted per function in this letter's
  ledger.
- Cross-model review completed for all three packages; findings ledger
  (confirmed → fixed / rejected → disproving command) recorded here.
- `scripts/man-census.sh` shows all three packages at 100% fill.

### Non-goals (explicit constraints)

- **No new inline explanation of the memory model.** Any page that needs
  more than one sentence about copies or handles links `mfb man variable`
  (authored in A) — it does not re-explain, and never in C/Rust terms.
- Per plan-108-A: no compiler testing (rendering is the verification);
  prose string fields only (never a body, descriptor type, or error table —
  `git diff` per commit shows string-literal prose changes only); no
  renderer or registry-schema changes; `src/docs/man/**` untouched.
- No behavior changes to the builtins themselves. **Exception discipline:**
  if the accuracy pass uncovers an actual code bug (doc says X, code does Y,
  and Y is wrong), that is a found bug — fix it or file it via write-bug per
  AGENTS.md, never paper over it in prose; record it here either way.

## 2. Current State

A's census (re-run at kickoff for exact numbers): strings 39 / term 25 /
testing 12 function pages, 0 with Description or Example
(census 2026-08-24: desc column = 0 for all three). old_man coverage for
these packages exists under `planning/old_man/builtins/<pkg>/` (543 pages
total across all packages).

### Measured populations

| What | Count | Command |
|---|---|---|
| pages to author | 76 (39+25+12) | A's census table; re-run `scripts/man-census.sh` at kickoff |
| old_man source pages available per package | measure at kickoff | `ls planning/old_man/builtins/{strings,term,testing} \| wc -l` |
| term members compile-verified only (no tty) | decided per function during Phase 2 | recorded in this letter's ledger |

## 3. Design Overview

Production-line: one package at a time through A's four steps, one commit
per package per step-pair (author+scope, then review-fixes), census re-run
after each package. strings first (largest, best old_man coverage), then
term, then testing.

**Risk concentration:** silently inheriting a stale old_man claim. Held by
A's accuracy rule (probe programs), the reviewer's verify-not-proofread
prompt, and this letter's ledger requiring evidence per finding.

### Rejected alternatives

- **Author all 76 pages then review once.** Rejected: A's pilot calibration
  works package-by-package; a term-page systemic mistake caught by the
  strings review never gets made.

## Compatibility / Format Impact

None to codegen/wire. `tests/cli_man_summary_plain.rs` pinned text updated
in the same commit only if a pinned summary is itself corrected.

## Phases

> **Re-scoped 2026-08-30 at kickoff — see Corrections.** Two of this
> letter's three packages are no longer empty. Measured with
> `./scripts/man-census.sh strings term testing`:
>
> ```
> strings           39     39     39       39       64/64     11      -
> term              24     24     24       24       34/34     11  13/18
> testing           12     12      0        0       23/23     11      -
> ```
>
> So Phase 1 and Phase 2 are **verification** passes, not authoring, and
> only Phase 3 authors. The four-step workflow is unchanged; only the
> starting point of the first step differs, which is exactly the cheaper
> branch A's §3 already describes. Two populations the original phases did
> not name are added: term's **5 undescribed types-page entries**, and the
> **46 memory-vocabulary hits** (strings 28, term 18) this letter must
> drive to 0.

### Phase 1 — strings (verify)

- [x] Verify 39 pages + overview (accuracy + scope passes); every example
      compiled and run. **No types page** — strings exports no types, so
      `mfb man strings types` renders nothing and the census reads `-`.
      **84 examples build and run** (`./scripts/man-run-examples.sh strings
      --run`). Every documented sharp edge probed: `left`/`right` clamp
      where `mid` raises `77050001`, `len` counts scalars where `byteLen`
      counts bytes and `graphemesCount` counts clusters, `trim` returns a
      new `String` on the unchanged path, `split` never overlaps, `repeat`
      by 0 gives the empty string, `padLeft` never truncates.
- [x] Drive the 28 memory-vocabulary hits to 0. They are one systematic
      phrase — "the result is a new **owned** String" — plus `allocate`
      variants. Keep the fact (the argument is not changed, you get a new
      value back); drop the vocabulary. — **0**.
- [x] Cross-model review (Codex) + apply; ledger recorded here.
      **14 findings, 14 confirmed, 0 rejected.**
- [x] Verify: `mfb man strings --all` reads clean; census 100% for
      strings; `--memory-scope strings` at 0. — 39/39/39, 64/64 params, 0.

Acceptance: strings fully verified and reviewed, 0 memory-scope hits.
— **MET**.
Commit: ce95ab5c5 (verify + scope), plus the review fixes in the same

### Phase 2 — term (verify)

- [x] Verify 24 pages + overview + types page; per-function run vs
      compile-only verification noted in the ledger.
      **43 examples compile; 32 also run headless.** The 11 that do not are
      recorded by name in the ledger below — they are not a blanket skip.
- [x] Author the **5 missing types-page entries** — `TermColor.r/g/b` and
      `TermSize.columns/rows`. — types page now **18/18**.
- [x] Drive term's 18 memory-vocabulary hits to 0. — **0**.
- [x] Cross-model review + apply; ledger.
      **8 findings, 8 confirmed, 0 rejected**, plus 11 further pages swept.
- [x] Verify: rendering + census as Phase 1, types page at 18/18.
      — 24/24/24, 34/34 params, 18/18 types, 0 memory-scope.

Acceptance: term fully verified and reviewed, types page complete, 0
memory-scope hits. — **MET**.
Commit: 7c9fc359a

### Phase 3 — testing (author)

- [x] Author 12 pages + overview; describe `expect` semantics in developer
      terms (what a failed expectation reports; never the desugar story).
      — done, and the **overview's own desugar story was removed**: it said
      the assertions are "compiler-lowered — recognized in the front end and
      desugared to comparison statements — so there is no runtime helper",
      which is exactly what this phase forbids. Split into one `func_*.rs`
      per assertion, matching every other package.
- [x] Cross-model review + apply; ledger. **4 findings, 4 confirmed.**
- [x] Verify: rendering + census as Phase 1. — 12/12/12, 23/23 params, 0
      memory-scope, and all 14 examples checked with `mfb test`.

Acceptance: testing fully authored and reviewed. — **MET**.
Commit: ce95ab5c5 (author), plus the review fixes

## Cross-model review ledger

Reviewer for all three: `codex exec -C <worktree> -s workspace-write - <
prompt.txt`, banner **OpenAI Codex v0.150.0, model `gpt-5.6-terra`**, one run
per package. `git status` clean of reviewer edits after each — Codex worked
only under `/tmp`. **26 findings across the three packages; 26 confirmed, 0
rejected.**

### strings — 14 findings (40 pages checked, 86 programs run)

| Category | Finding | Disposition |
|---|---|---|
| INACCURACY | `find` and `contains` both advise "guard `find` with `contains`". **That guard is unsound for the three-argument form**: `contains` searches the whole string, so it can answer `TRUE` for a match that lies *before* `start`, and `find` raises anyway | CONFIRMED — **re-verified**: `contains("abcabc","a")` is `TRUE` while `find("abcabc","a",5)` raises `77050004`. Both pages now scope the advice to the two-argument form and say what to do instead |
| LEAKAGE ×12 | implementation detail on a dozen pages: `byteLen`'s "read directly from the string's stored byte count", `graphemesCount`'s "linear scan, not a stored field", `toBytes`'s "folded at build time", `displayWidth`'s "vendored utf8proc `charwidth` table", and "table embedded in the runtime/compiler" on `caseFold`, `lower`, `upper`, `graphemes`, `isDigit`, `isLetter`, `isLower`, `isUpper` | CONFIRMED — each replaced by the Unicode rule it implements ("Unicode full case folding", "follows the Unicode general categories"), which is what a developer can actually rely on. `byteLen` keeps the useful half — the answer is immediate however long the string is |

### term — 8 findings (26 pages checked, 4 programs run)

| Category | Finding | Disposition |
|---|---|---|
| INACCURACY | the overview's "every other `term::` call except `term::isOn` is a no-op while TUI mode is off" — `term::terminalSize` **raises `ErrUnsupported`** instead | CONFIRMED — **re-verified**: with TUI off, `clear`/`moveTo`/`setBold` return quietly and `terminalSize()` raises `77050007`. The overview now names both exceptions |
| INACCURACY | `drawGlyph`'s "the cell is **clamped to the surface**: if `(x, y)` is off the grid the call draws nothing" — those are opposite behaviours | CONFIRMED — it is bounds-checked, not clamped. The page says so, and adds the consequence: an off-by-one loses the glyph silently rather than drawing it at an edge |
| LEAKAGE ×6 | `sync`'s front/back-buffer diffing and batched writes, `terminalSize`'s `TIOCGWINSZ`, `didResize`'s `setFrameSize:`/GTK resize-signal hooks, `drawText`'s "maximal runs", `clear`'s "zero-fill", and `~ICANON`/`~ECHO` | CONFIRMED — each replaced by its developer-visible consequence |

Sweeping those six turned up the same vocabulary on **eleven further pages**
Codex had not cited (`back buffer`, `shadow cursor`, `shadow grid`,
`zero-fill`, `grid header`), all rewritten. **`alternate screen` is kept
deliberately** — it is a real terminal concept a developer recognises, not an
implementation detail.

**term's run-vs-compile ledger.** 43 examples; 32 run headless, and these 11
need a tty: `drawBox#2`, `drawGlyph#1`, `drawHLine#1`, `drawText#1`,
`drawVLine#1`, `fillRect#2`, `moveTo#2`, `setBackground#2`, `sync#2`,
`terminalSize#1`, `terminalSize#2`. All 11 fail with the `ErrUnsupported`
their own pages document, so the failure is itself a check rather than a gap.

### testing — 4 findings (14 pages checked, 6 programs run)

| Category | Finding | Disposition |
|---|---|---|
| INACCURACY | `expectFixed`'s "writing `expectFixed(x, 0.1)` compares against whatever `0.1` rounds to" — a bare decimal is a `Float`, so the call does not compare at all, it is rejected. The page **contradicted itself** two sentences later | CONFIRMED — rewritten so the rounding point is made about an annotated `Fixed` value, and the bare-literal case is named as the `TESTING_EXPECT_TYPE_MISMATCH` it is |
| MISSING ×3 | the printable-operand list on the overview, `expectEqual` and `expectNEqual` omitted **`Boolean`** and **`Scalar`** | CONFIRMED — **re-verified**: `expectEqual(TRUE, TRUE)` and `expectNEqual(toScalar("A"), toScalar("B"))` both pass. List corrected in all three places |

### What this letter adds to the pilot's lessons

7. **The scope pass introduced a defect again** — `term::terminalSize` gained
   "it can still fail when memory is exhausted, since it builds a record to
   return" while a leak was being removed from the same sentence. That is two
   letters running (A's `bits` findings 2–5). Treat any *new* sentence written
   during a scope pass as unverified prose that the reviewer must see.
8. **A reviewer finding is usually a class, not an instance.** Codex cited six
   leaking term pages; the same vocabulary was on eleven more. Always grep the
   package for the pattern behind a finding rather than fixing only what was
   quoted.
9. **Sibling pages disagree in pairs.** `find`/`contains` gave the same unsound
   advice from both ends, and `expectFixed` contradicted itself within one
   page. When a finding lands, check the page that points at it.

## Validation Plan

- Verification: `mfb man <pkg> --all`/`types` rendering per package;
  `scripts/man-census.sh` → 100% fill for strings, term, testing; examples
  and probes compiled/run ad hoc during authoring.
- Doc sync: none beyond the man content itself (F owns tooling/AGENTS.md).
- Hygiene: fmt at session end (prose lives in `.rs` files).

## Open Decisions

- None entering the letter — run-vs-compile verification calls for term are
  made per function during Phase 2 and recorded in the ledger, not
  deferred.

## Corrections

**2026-08-30 (kickoff) — strings and term are already filled; this letter
authors one package, not three.** A's Phase 1 census (and this letter's own
kickoff re-run, `./scripts/man-census.sh strings term testing`) measures:

| Package | This letter said | Kickoff measures |
|---|---|---|
| strings | 39 pages, 0 desc | 39 pages, **39** desc + 39 example, 64/64 params |
| term | 25 pages, 0 desc | **24** pages, **24** desc + 24 example, 34/34 params |
| testing | 12 pages, 0 desc | 12 pages, **0** desc — still empty |

The letter's §2 quoted A's 2026-08-24 census (HEAD `254506f9b`); six of the
nine packages it called all-empty have been filled since. So the "76 pages
to author" figure is wrong: **12 pages are authored here, and 63 are
verified.** Per A's §3 the workflow is identical either way — verification
just starts from existing prose rather than a blank field.

The title "Author the empty packages, batch 1" is left as-is: renaming the
file would break every cross-letter reference, and this Correction is where
a reader finds out what the letter actually does.

Two populations the letter never named, both now phase tasks:

- **term's types page is 13/18.** Five record fields render with an empty
  Description — `TermColor.r`, `.g`, `.b` and `TermSize.columns`, `.rows`.
  A page with a bare field list fails this plan's Goal the same way a bare
  function page does.
- **46 memory-vocabulary hits** (strings 28, term 18, testing 0) against a
  §2 baseline that predicted "strings 1, term 0, testing 0". That baseline
  used the old undercounting regex — A's Corrections explain why. In
  `strings` the hits are almost one phrase repeated: "the result is a new
  **owned** String", plus `allocate` variants.

## Summary

The first authoring batch: the most-used empty package (strings) plus two
packages that stress-test the standard's example and internals-leakage
rules, all landed through the calibrated four-step workflow with
per-package review ledgers.
