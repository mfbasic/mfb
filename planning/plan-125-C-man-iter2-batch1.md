# plan-125-C: man iteration 2, batch 1 — collections, datetime, encoding, math (150 pages)

Last updated: 2026-09-04
Effort: x-large (1d–3d) — 150 units at the pilot's measured per-unit cost
Depends on: plan-125-B (iteration 1 complete across the whole man surface,
including the cross-package consistency review whose terminology table this
letter conforms to).

Landing unit: **each Phase below is independently landable and gets its own
commit.** The letter totals x-large; it is never landed as one change, and a
session that lands one phase and stops has left the tree consistent.

Iteration 2 of three, first of four batches. **The review unit is one page.**
The reviewer is given a single page with no siblings in context, and is asked
the only question that needs that isolation: *is every sentence on this page
true, and is this page self-sufficient for a developer who lands on it from a
search?*

This is the depth pass. It is the only iteration that verifies every claim
against the implementation by reading the code and running probes, compiles
and runs every example, and checks every parameter description and error row.
590 of the plan's 709 man runs live in letters C–F for that reason.

Batch 1 is the **core value packages** — the ones every MFBASIC program uses
and the ones whose contracts other packages' pages assume.

References:

- plan-125-A §3.2 (why the page lens differs), §3.3 (workflow), §4.3
  (harness), §5 (the iteration-2 prompt, run verbatim).
- plan-125-B's terminology table — binding on every page in this letter.
- `.ai/man-content.md`; plan-108-A §3 (2a) memory-vocabulary ban.
- `.ai/collections.md` — the **internals foil** for `collections`: HOF
  rewrites, native lowering and in-place mutation mechanics are spec/internals
  and must not appear on a man page. The developer contract ("helpers do not
  mutate their arguments") is what the page states.
- Memory `string-concat-beats-list-join-in-mfb`,
  `collection-set-in-place-only-for-same-function-local`,
  `inline-headroom-growable-record-collection` — behavior a `collections`
  example can get wrong; check any example that mutates or accumulates.
- Memory `tofloat-not-correctly-rounded` — a known precision sharp edge; any
  `math`/`encoding`/`datetime` precision claim is probe-verified, and actual
  behavior is documented honestly.

## Prerequisites

See plan-125-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-125-B complete | `grep -c '^- \[ \]' planning/completed/plan-125-B-man-iter1-packages.md` → `0` | MET 2026-09-13: `0` open and `0` `[~]`; all four Commit lines filled (last `1bcd9d803`) |
| B's terminology table exists and is final | read plan-125-B Phase 4 | MET: 17 rows, the last seven added by Phase 4's consistency review (`1bcd9d803`) |

## 1. Goal

- **All 150 pages in this batch** have been through my per-page pass, one
  `codex exec` per page, and apply.
- **Every example on all 150 pages compiled and run** with the release binary
  during this letter, each recorded in the example ledger as *ran* or
  *compile-only with the reason*. Zero unaccounted.
- **Every parameter description and every prose error claim verified** against
  the descriptor and by probe.
- Every finding has a verdict; every rejection has a disproving command.
- `--reconcile` exits 0 over the 150-unit list.
- Sweeps clean for all four packages after apply.
- "Belongs in spec" cuts appended to `planning/plan-125-belongs-in-spec.md`.

### Non-goals (explicit constraints)

- Per plan-125-A: no compiler test gates; prose fields only; `git diff` per
  commit is string literals only; the reviewer never commits.
- **No cross-page reconciliation here.** If two pages in this batch now
  disagree, record it — do not fix it by rewriting a neighbour you have not
  reviewed. Iteration 3 (letter G) is the re-integration pass and owns that.
- No wording churn on a sentence that survives verification.

## 2. Current State

Entering C, every page in these four packages has been read once as part of a
whole package (letter B) and carries B's terminology. No page has been
verified sentence-by-sentence and **no example in these packages has been
compiled since plan-108-D** (which covered `datetime`, `encoding`,
`collections`, `math` at 24 collections pages; `collections` has since grown
to 49 — 25 pages that plan-108 never saw).

### Measured populations

| What | Count | Command |
|---|---|---|
| `collections` units | 50 | 49 function pages + overview (no types page) — `./scripts/man-census.sh --fill` |
| `datetime` units | 46 | 44 + overview + types |
| `encoding` units | 32 | 30 + overview + types |
| `math` units | 22 | 21 + overview (no types page) |
| **batch total** | **150** | sum |
| parameter descriptions in batch | 236 | census PARAM-DESC: collections 103, datetime 73, encoding 32, math 28 |
| type descriptions in batch | 69 | census TYPES: datetime 40, encoding 29 |
| `collections` pages plan-108 never saw | 25 | 49 today vs 24 in plan-108-D's population table |
| `datetime` commits since plan-108 | 66 file-touches | `git log --since=2026-08-31 --name-only --format='' -- src/codegen/builtins/datetime \| wc -l` |
| `datetime` carve-out-1 borrow lines | 15 | `./scripts/man-census.sh --memory-scope datetime` — arithmetic, not memory; classify once, not per page |

### Verified properties

- **`collections` doubled since plan-108's review** — VERIFIED against
  plan-108-D's own population table (24 pages) versus today's census (49).
  Half this package has never been reviewed at any granularity except
  letter B.
- **The 15 `datetime` `borrow` lines are arithmetic** — VERIFIED by
  `--memory-scope` classification (all CARVE-1, "borrows a whole second").
  Do not rewrite them; classify the set once in this letter's ledger.

## 3. Design Overview

Per page: my pass (read the page rendered, check each sentence against the
implementation, compile and run the example, check every parameter row and
error claim) → one `codex exec` from the iteration-2 prompt → apply on the
main thread. The harness runs the reviews at `N` concurrency while I apply
serially; the main thread is the only writer.

**Order:** `math` (22, simplest claims — calibrates pace) → `encoding` (32) →
`datetime` (46, the largest carve-out surface) → `collections` (50, the half
that is new to review last, with pace known).

**Risk concentration:**
- **Example runtime.** 150 examples compiled and run is the bulk of the
  wall-clock. Every run is time-bounded and uses a scratch project as cwd
  (memory `example-harness-cwd-and-timeout`); `scripts/man-run-examples.sh
  <pkg> --run` is the instrument, and a page whose example cannot run
  standalone is recorded with the reason, never quietly skipped.
- **Precision claims.** `math`, `encoding` and `datetime` make numeric claims
  that read as true and are off by a ULP. Every one is probe-verified against
  the actual binary, not against arithmetic reasoning.
- **`collections` internals pull.** `.ai/collections.md` documents mechanics
  that are genuinely interesting and genuinely forbidden on a man page. The
  test is the audience, not the truth.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains;
> `- [x] ~~text~~ — moot: <evidence>` rather than deleting; fill `Commit:` the
> moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — math (22 units)

- [~] All 21 function pages + the overview, one review unit each.
      Unit list `planning/plan-125-units/C-phase1.txt` (22 units: overview + 21;
      `math` has no types page, and its constants have no pages of their own).
      **Remaining: the 22 Codex reviews.** The 2026-09-13 dispatch hit the Codex
      usage limit on its first units ("try again at 11:13 AM",
      `planning/plan-125-findings/C-phase1/man-page-math-abs.log`) and was
      stopped; re-dispatch after the reset.
- [~] Every example compiled and run; precision claims probe-verified.
      Examples: `math` 21/21 built and ran at `7f4ff371b`. My probes so far
      (`/tmp/p125-ex/mathconst`, `/tmp/p125-ex/mathclaims`):
      - **Wrong, applied:** the overview said every constant "comes in a `Float`
        form and a `Fixed` form with the same value". No pair is equal
        (`toFloat(x) = f` is `FALSE` for all 7); `math::piFixed` prints
        `3.141592653701081`. The overview now says the table shows the `Float`
        forms, and a `Fixed` form is the nearest value `Fixed` can hold, agreeing
        to at least nine significant digits. Measured per pair: `ln10` differs in
        the ninth decimal place, so "ninth decimal" would have been wrong.
      - **Wrong, applied:** `asin`, `acos`, `log` and `log10` said the scalar
        form raises `ErrFloatDomain` and the list form `ErrInvalidArgument`. The
        split is by element type, not by list:
        - `/tmp/p125-ex/mathclaims` and `/tmp/p125-ex/mathlist`:
          `asin`/`acos`/`log`/`log10` on a `Float` or `List OF Float` raise
          `77050012` (`ErrFloatDomain`);
        - `log`/`log10` on a `Fixed` or `List OF Fixed`, and `asin` on a
          `Fixed`, raise `77050002` (`ErrInvalidArgument`);
        - `/tmp/p125-ex/acosfixed`: `acos` on a `Fixed` raises `77050002`.

        All four pages now state the element-type split.
      - Confirmed as documented:
        - `sqrt(-1.0)` raises `ErrFloatDomain`; `sqrt` of a negative `Fixed`
          raises `ErrInvalidArgument`.
        - `abs` of the most negative `Integer` raises `ErrOverflow`.
        - `clamp` with low > high, `rand` with min > max, and `min` with
          mismatched list lengths raise `ErrInvalidArgument`.
        - `ceil(-1.5)` is `-1` and `floor(-1.5)` is `-2`; `round(2.5)` is `3`,
          `round(-2.5)` is `-3`, and `round(0.5)` is `1`.
        - `floor(1.0e30)` raises `ErrOverflow`.
        - `exp(1000.0)` raises `ErrFloatInf`.
        - `pow(-8.0, 0.5)` raises `ErrFloatNaN`. Its page says only "outside the
          domain", which is not wrong, but it names no error; decide when applying.
      - Trig and `atan2`, probed after the Codex reset
        (`/tmp/p125-ex/mathtrig`, `/tmp/p125-ex/mathatan2`,
        `/tmp/p125-ex/tanfixed`):
        - **Wrong, applied:** `tan` said an argument near an odd multiple of
          pi/2 "can overflow to a non-finite result (`ErrFloatInf`/
          `ErrFloatNaN`)". A `Float` never does: `tan(math::pi2)` is
          1.6e16 and `tan(3·pi2)` 5.4e15. A `Fixed` drifts from the `Float`
          result as the angle nears pi/2 (37025580 vs 37262968 at 1.5707963).
          At `math::pi2Fixed` it returns `-1431655767.666667`, no error, while
          the true tangent is about +1.65e10, beyond `Fixed`'s range →
          **bug-615**. The page states the current behavior and points to
          `Float`.
        - **Wrong, applied:** `atan2` said its result is in `(-pi, pi]`;
          `atan2(-0.0, -1.0)` is exactly `-pi`. It is now `[-pi, pi]`, with the
          negative-zero case spelled out.
        - Confirmed as documented:
          - `atan(±1e300)` is ±pi/2;
          - `sin`/`cos`/`tan`/`atan` echo `Fixed`;
          - `atan2`'s four quadrants, its `Fixed` form, and mismatched list
            lengths → `ErrInvalidArgument`.
      - Found while reading `atan2`: declarations render the placeholder
        `AS Arg0`. `./scripts/man-manual.sh | grep -c -E
        '\b(AS|OF|TO) Arg[0-9]+\b'` → 78 lines (66 `math`, 12 `collections`) →
        **bug-616**, a renderer defect, not prose.
      - Bug-number race: 611–614 are already used on other branches (`git log
        --all --name-only`), so the two new bugs are 615 and 616. Another
        session's unmerged t3 checkpoint also uses bug-603, for a `datetime`
        bug added 2026-09-13 06:32. This branch's bug-603 (color hue,
        `17872c5d1`, 2026-09-12 22:35) came first and keeps the number.
- [~] Ledger + example ledger recorded here. **Partial:** 6 of 22 reviews
      completed before the second usage-limit stop (overview, `abs`, `acos`,
      `asin`, `atan`, `atan2`); their 20 findings are triaged below. The other 16
      units remain.

#### Phase 1 ledger — Codex iteration 2, partial (`planning/plan-125-findings/C-phase1/`)

| Page | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| overview | 1 | CONFIRMED | `mfb man math <fn>` declarations: `List OF Fixed` exists for `abs`, `ceil`, `clamp`, `floor`, `log`, `log10`, `max`, `min`, `round`, `sqrt`; not for `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `exp`, `pow`, `atan2` | overview names which forms take only `List OF Float` |
| overview | 2 | NOT A PAGE CLAIM | the quoted text is the `ErrFloatDomain` error code's own message in the derived Errors row (carve-out 2), shared runtime data | — (the per-type split is stated on each function page) |
| overview | 3 | CONFIRMED | no example rendered | example added; `/tmp/p125-ex/mathoverview` printed `1024.00`, `1.77`, `TRUE` |
| abs | 1 | CONFIRMED | reviewer probe: `abs` of the minimum `Money` raised `7-705-0010` | parameter names `Money` too |
| abs | 2 | CONFIRMED → **bug-617** | `gen_math.rs:lower_math_abs` `Float` branch has no raise, yet the Errors row covers overloads 1–7 | none: declared-error data |
| abs | 3 | CONFIRMED | `/tmp/p125-ex/asinfixed`: `abs([-7, 3, 0])` → `7,3,0`, source still `-7` | DESC: new list, same order, input unchanged |
| acos | 1 | CONFIRMED | probe: `acos(-1)` = pi, `acos(0)` = pi/2, `acos(1)` = 0 | parameter: inclusive bounds, 0 → pi/2 |
| acos | 2, 3 | CONFIRMED → **bug-617** | `Float`/list raise `77050012`, `Fixed` raises `77050002`; the table lists both on all overloads | none: declared-error data |
| acos | 4 | CONFIRMED | reviewer probe: new list in input order, input unchanged | DESC |
| acos | 5 | CONFIRMED | range from the same probe | INTRO "from 0 through pi radians" |
| asin | 1, 2 | **REJECTED** | the reviewer's `math::asin(1.1f)` is a `Float` call: `typeName(1.1f)` → `Float`. A typed `LET x AS Fixed = toFixed(1.1)` gives `asin(x)` → `77050002` (`/tmp/p125-ex/asinfixed`). The page's "a `Fixed` argument raises `ErrInvalidArgument`" stands. Its Errors table has the same per-overload defect as `acos`, recorded in bug-617 | — |
| atan | 1 | CONFIRMED | `List OF Fixed` is not an `atan` form (declaration census above) | shared parameter: "or a `List OF Float` of them" |
| atan | 2 | CONFIRMED → **bug-617** | `emit_fixed_atan2` has no raise path, yet `ErrFloatNaN` is listed on the `Fixed` overload | none: declared-error data |
| atan2 | 1 | CONFIRMED (already applied) | my probe, `0399377a5` | range `[-pi, pi]` |
| atan2 | 2 | CONFIRMED | probe: `atan2(±1, 0)` = ±pi/2, `atan2(0, 0)` = 0 | DESC axis and origin cases |
| atan2 | 3 | DEFERRED | the NaN repro, `atan2((big*big)/(big*big), 1.0)`, may raise in its argument expression before `atan2` runs, so it does not isolate `atan2`; re-probe in the page's remaining review | — |
| atan2 | 4 | CONFIRMED | `lower_simd_float_binary` allocates a separate result list | DESC: new list, inputs unchanged |

Acceptance: 22 units `exit 0` in the manifest; `--reconcile` clean for the
phase; `mfb man math --all` renders; `--memory-scope math`/`--scope math` → 0.
Commit: —

### Phase 2 — encoding (32 units)

- [ ] 30 function pages + overview + the 29-description types page.
- [ ] Every example compiled and run; every round-trip claim
      (`hexEncode`/`hexDecode`, `varint`, `punycode`, `codepage`) verified by
      probe in both directions.
- [ ] Ledger recorded.

Acceptance: 32 units `exit 0`; sweeps clean for `encoding`; every type
description verified against the record/resource it describes.
Commit: —

### Phase 3 — datetime (46 units)

- [ ] 44 function pages + overview + the 40-description types page.
- [ ] Classify the 15 carve-out-1 arithmetic-borrow lines **once**, in this
      ledger, not per page.
- [ ] Every example compiled and run; every zone/DST/leap claim probe-verified
      (these are the claims most likely to be true-by-reasoning and false in
      fact).
- [ ] Ledger recorded.

Acceptance: 46 units `exit 0`; `--memory-scope datetime` reports exactly the
15 CARVE-1 rows and 0 unclassified; sweeps otherwise clean.
Commit: —

### Phase 4 — collections (50 units)

- [ ] 49 function pages + overview.
- [ ] Mark, in the ledger, which 25 pages postdate plan-108's review — they
      get the closest reading.
- [ ] Every example compiled and run; every mutation/ordering/identity claim
      probe-verified against the `.ai/collections.md` developer contract
      (never against its internals).
- [ ] Ledger recorded.

Acceptance: 50 units `exit 0`; sweeps clean for `collections`;
`--reconcile` exits 0 over the whole 150-unit batch; the example ledger
accounts for all 150 examples with 0 unaccounted.
Commit: —

## Validation Plan

- Tests: none (man prose); update a pinned-text test in the same commit if a
  fix touches one.
- Coverage check: `--reconcile` over the 150-unit list; and the example ledger
  reconciled against the census function list — 0 unaccounted.
- Runtime proof: `scripts/man-run-examples.sh <pkg> --run` for all four
  packages; `mfb man <pkg> --all` renders.
- Doc sync: `planning/plan-125-belongs-in-spec.md` appended.
- Acceptance: `--fill` still 100%; `--memory-scope` 0 unclassified;
  `--scope` 0; `--reconcile` 0.

## Open Decisions

- **A page whose example cannot be a standalone runnable program** (a
  `collections` helper that only makes sense mid-pipeline) — recommend
  keeping the example as the fragment a developer would actually write and
  recording it *compile-only with the reason* in the example ledger, rather
  than padding it into an artificial full program.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

The risk is concentrated in the 25 `collections` pages that postdate
plan-108's only review of that package, and in the precision claims across
`math`/`encoding`/`datetime` — the class of claim that survives every review
that does not actually run it. Everything else in this batch has been read
once before and is being verified, not authored.
