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

- [x] All 21 function pages + the overview, one review unit each.
      Unit list `planning/plan-125-units/C-phase1.txt` (22 units: overview + 21;
      `math` has no types page, and its constants have no pages of their own).
      The Codex usage limit cut the batch three times, so it ran in rounds:
      - 6 units on the first dispatch;
      - 11 from `C-phase1-retry.txt`;
      - the last 5 from `C-phase1-retry2.txt`.
      `--reconcile --letter C-phase1 --units planning/plan-125-units/C-phase1.txt`
      → `units=22 unaccounted=0 orphans=0`.
- [x] Every example compiled and run; precision claims probe-verified.
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
- [x] Ledger + example ledger recorded here. All 22 reviews are triaged below,
      over three rounds, and every verdict carries its evidence. Examples: `math`
      21/21 built and ran after each round; the overview's new example was run
      separately (`/tmp/p125-ex/mathoverview`).

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

Retry round (user: "send the next round"). The quota probe answered PONG, and
16 units were dispatched from `planning/plan-125-units/C-phase1-retry.txt`:
- 11 completed, exit 0: `ceil`, `clamp`, `cos`, `exp`, `floor`, `log`,
  `log10`, `max`, `min`, `rand`, `round`.
- 5 hit the usage limit again (`pow`, `seed`, `sin`, `sqrt`, `tan`: logs say
  "try again at 11:13 AM"). They remain.

Probes: `/tmp/p125-ex/mathc1b`, `/tmp/p125-ex/mathc1c`.

| Page | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| ceil | 1 | CONFIRMED | "a deliberate dimension exit" is registry design vocabulary | DESC: returns an `Integer`; `Money` gives whole currency units |
| ceil | 2 | CONFIRMED | probe: `ceil(2^63)` → `77050010`; `ceil(-2^63)`, a `Fixed` at 2147483647.5 and `Money` round normally | DESC: only a `Float` can overflow |
| ceil | 3 | CONFIRMED | probe: `ceil([2.1, -1.5])` → `3,-1`, input unchanged; empty → 0 | DESC: new list, same order, empty → empty |
| clamp | 1, 2 | CONFIRMED | probe: `clamp([-3,0,4,9], 0, 4)` → `0..4`, input unchanged; empty → 0 | DESC: scalar bounds for a list; new list |
| clamp | 3 | REJECTED | `ErrOutOfMemory` for a result list that cannot be allocated is exhaustion, not a trappable domain error; plan-125-B (`collections::distinct`) does not document it as a per-call error | — |
| cos | 1 | CONFIRMED | `cos` has only `List OF Float` | parameter: "or a `List OF Float` of them" |
| cos | 2 (empty list), 3 | CONFIRMED | probe: empty → 0; `cos([0, 3.14159…])[1]` → `-1.00`, input unchanged | DESC: new list, empty → empty |
| cos | 2 (non-finite), 4 | DEFERRED | the NaN repro builds NaN in its argument expression, so it does not isolate `cos` (same as atan2 #3) | — |
| exp | 1 | CONFIRMED | probe: `exp(-800.0)` → `0`; `exp(710.0)` → `77050014`; `exp(30.0F)` → `77050010` (`typeName(30.0F)` is `Fixed`: uppercase `F`) | DESC: underflow to 0, `ErrFloatInf` vs `ErrOverflow` |
| exp | 2 | CONFIRMED → **bug-617** | same probe | none: declared-error data |
| floor | 1 | CONFIRMED → **bug-617** | same Float-only range check as `ceil` | none |
| floor | 2, 3, 4 | CONFIRMED | shares `ceil`'s lowering and probe results; the reviewer's probe showed `2,-2,0` | DESC as `ceil`; parameter no longer implies a `Money` list |
| log | 1, 2 | CONFIRMED → **bug-617** | `/tmp/p125-ex/mathlist` | none |
| log | 3, 4 | CONFIRMED | reviewer probe: empty lists → 0; `log([e])` new list, input unchanged | DESC + parameter |
| log10 | 1, 2 | CONFIRMED → **bug-617** | as `log` | none |
| log10 | 3 | CONFIRMED | reviewer probe: empty → 0, order kept, input unchanged | DESC + parameter |
| max | 1, 2, 3 | CONFIRMED | the `min` probe shares `lower_simd_binary`: new list, inputs unchanged, empty+empty → 0; mismatched → `77050002` (`/tmp/p125-ex/mathclaims`) | DESC + both parameters |
| max | 4 | CONFIRMED → **bug-617** | `lower_math_min_max` has no raise path | none |
| min | 1 | CONFIRMED → **bug-617** | as `max` | none |
| min | 2, 3 | CONFIRMED | probe: `min([3,-5,7],[4,-2,1])` → `3,-5,1`, input unchanged; empty+empty → 0 | DESC + both parameters |
| rand | 1, 2 | CONFIRMED | probe: `rand(0,0)` → 0, `rand(-10,-10)` → -10, `rand(-5,-1)` in range, `Money` -1.00 bounds work | both parameters: zero, negative and equal bounds valid |
| round | 1, 2 | CONFIRMED | probe: `round([2.5,-2.5])` → `3,-3`, input unchanged; empty → 0 | DESC: new list, empty → empty; "dimension exit" replaced |

Final round (user: "codex limit reset, continue"). The quota probe answered PONG,
and 5 units were dispatched from `planning/plan-125-units/C-phase1-retry2.txt`;
all 5 completed with exit 0. `--reconcile --letter C-phase1 --units
planning/plan-125-units/C-phase1.txt` → `units=22 unaccounted=0 orphans=0`.
Probe: `/tmp/p125-ex/mathc1d`.

| Page | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| pow | 1, 2 | CONFIRMED | `List OF Fixed` is not a `pow` form (declaration census); mismatched `List OF Float` → `77050002` | both parameters name `List OF Float`; lengths must match |
| pow | 3 | CONFIRMED | probe: `pow(-2.0, 0.5)` → `77050013`; `Fixed` → `77050002`; `pow(0.0, -1.0)` → `77050014`; `Fixed` → `77050010`; `pow(0,0)` → `1.00`; `pow(2,-2)` → `0.25` | DESC: the per-type error split and the edge values |
| seed | 1 | CONFIRMED | my own last-round sentence "unseeded draws differ from run to run" overclaims (`rand(1, 1)` is always 1) | DESC: "not reproducible" |
| seed | 2 | CONFIRMED | probe: `seed(0)` and `seed(-1)` both replay | parameter: any `Integer`, including zero and negative |
| seed | 3 | CONFIRMED | `src/codegen/runtime/thread/runtime_helpers.rs`: a worker's PCG64 stream is seeded from one draw of the spawning thread's generator (`RNG_NEXT_SYMBOL` → `RNG_SEED_SYMBOL`), not inherited | DESC: each thread has its own sequence |
| sin | 1 | CONFIRMED → **bug-618** | probe: `sin(1e20)` → `1.96e29`, `cos(1e20)` → `-3.67e27`; correct-looking through `1e9` | none: the page does not document the garbage; the bug does |
| sin | 2 | CONFIRMED | `List OF Fixed` is not a `sin` form; empty list and a new list per `lower_simd_float_unary` | parameter names `List OF Float`; DESC: new list, empty → empty |
| sin | 3 | DEFERRED | the NaN repro `sin(0.0 / 0.0)` builds NaN in its argument, so it does not isolate `sin` (same as atan2 #3, cos #4) | — |
| sqrt | 1 | CONFIRMED | probe: `sqrt([4.0, -1.0])` → `77050012` (`ErrFloatDomain`); `sqrt` of a `List OF Fixed` with -1 → `77050002`. The page had said the list forms raise `ErrInvalidArgument` | DESC: split by element type, not by list |
| sqrt | 2, 3 | CONFIRMED → **bug-617** | same probe | none: declared-error data |
| sqrt | 4 | CONFIRMED | reviewer probe: `sqrt(0.0)` → 0; empty list → 0; new list, input unchanged | DESC |
| tan | 1 | CONFIRMED | `tan` has only `List OF Float` | parameter names `List OF Float` |
| tan | 2 | CONFIRMED → **bug-617** | `FloatKernel::Tan`'s only error is `ErrFloatNaN`; `tan(math::pi2)` is finite | none: declared-error data |

Acceptance: 22 units `exit 0` in the manifest; `--reconcile` clean for the
phase; `mfb man math --all` renders; `--memory-scope math`/`--scope math` → 0.

Measured at the closing commit:
- every one of the 22 units has an `exit 0` row as its latest;
- `--reconcile` → `unaccounted=0 orphans=0`;
- `mfb man math --all` exits 0;
- `--memory-scope math` 0, `--scope math` 0;
- `math` examples 21/21 ran.

Bugs filed from this phase: bug-615, bug-616, bug-617, bug-618.
Commit: db12d70c3 (unit lists), 21027ab02, 0399377a5, 604bc95c9, 3caf47763, e63a903d3 (closed)

### Phase 2 — encoding (32 units)

- [x] 30 function pages + overview + the 29-description types page.
      Unit list `planning/plan-125-units/C-phase2.txt` (32 units). The Codex usage
      limit split the batch: 22 units on the first dispatch, and the last 10 from
      `C-phase2-retry.txt`. `--reconcile --letter C-phase2 --units
      planning/plan-125-units/C-phase2.txt` → `units=32 unaccounted=0 orphans=0`.
- [x] Every example compiled and run; every round-trip claim
      (`hexEncode`/`hexDecode`, `varint`, `punycode`, `codepage`) verified by
      probe in both directions.
      My pass, done:
      - `./scripts/man-run-examples.sh encoding --run` → 62/62 built and ran;
        `--memory-scope encoding` 0; `--scope encoding` 0.
      - Round-trips, both directions (`/tmp/p125-ex/encroundtrip`):
        - hex `00ff7f80`;
        - `varint` 300 → `d804` → 300, and -1 → -1;
        - `uleb128` 624485 → `e58e26` → 624485;
        - `sleb128` -123456 → `c0bb78` → -123456;
        - `punycode` `bücher.example` ↔ `xn--bcher-kva.example`;
        - `Windows1252` `café€` → `636166e980` → `café€`;
        - `Koi8R` `привет` round-trips.
      - Types page, all counts exact (128 high bytes decoded per codepage):
        - `Iso8859_3` 7 undefined, `Iso8859_6` 83 defined, `Iso8859_7` 3
          undefined, `Iso8859_8` 92 defined;
        - `Iso8859_8I` identical to `Iso8859_8` on 128/128 bytes;
        - `Windows874` 120 defined, `Windows1253` 3 undefined, `Windows1255` 118
          defined, `Windows1257` 2 undefined;
        - `Iso8859_15` 0xA4 is `€` (`Windows1252` has `¤` there).
      - Decoder and escape claims (`/tmp/p125-ex/encclaims`): all confirmed.
      - `varintEncode(300)` is `d804`, the ZigZag form (a plain protobuf varint of
        300 is `ac02`). The pages already say ZigZag (`varintDecode` INTRO and
        DESC), so no page defect. To check against its review: the DESC writes the
        mapping as `(u >> 1) XOR -(u AND 1)`, and MFBASIC's `AND`/`XOR` are
        Boolean-only (the letter-B `bits::ctz` class).
      - Retry-round pages probed as well (`/tmp/p125-ex/encutf`, `ulebneg`,
        `utf8amb`): UTF-16/UTF-32 round-trips, the `varint` ZigZag mapping, and
        the `Integer` extremes.
- [x] Ledger recorded. All 32 reviews are triaged below, over two dispatch rounds;
      every verdict carries its evidence.

#### Phase 2 ledger — Codex iteration 2 (`planning/plan-125-findings/C-phase2/`)

Probes: `/tmp/p125-ex/encclaims`, `encroundtrip`, `encb32`, `encb64`, `utf8err`.
The "input unchanged" suggestions are rejected as a class: no call can change an
argument value (`mfb man variable`), so saying it on one page restates a
language rule.

| Page | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| overview | — | NO FINDINGS | — | — |
| types | 1 | CONFIRMED | `Iso8859_8I` decodes 128/128 high bytes identically to `Iso8859_8`; "differ only in display direction" implied a behavior difference | `Iso8859_8I` description |
| types | 2 | CONFIRMED | probe: `Iso8859_3` byte 165 and `Windows1257` byte 161 → `77050003`; all eight counts exact (encroundtrip) | the 8 counted codepages add "Decoding one of those undefined bytes raises `ErrInvalidFormat`" |
| base32Decode | 1 | CONFIRMED (prose) + **bug-606** | `base32Decode("A=======")` → `77050003`; the descriptor declares `errors: vec![]` | DESC names `ErrInvalidFormat`; bug-606 extended to all 12 decoders |
| base32Encode | 1 | CONFIRMED | probe: `base32Encode(base32Decode("my======"))` → `MY======` | "inverse" replaced |
| base32Encode | 2 | CONFIRMED | probe: `base32Encode([])` → empty | parameter |
| base32Encode | 3 | REJECTED | input-unchanged class (above) | — |
| base64Decode | 1, 2 | CONFIRMED | probe: `base64Decode("====")` → 0 bytes; `"AB=="` → `00` → `base64Encode` `AA==` | DESC: lenient decoding stated; "each non-padding character" |
| base64Decode | 3 | CONFIRMED (prose) + **bug-606** | `base64Decode("QQ")` → `77050003` | DESC names `ErrInvalidFormat` |
| base64Encode | 1 | REJECTED | input-unchanged class | — |
| base64UrlDecode | 1, 2 | CONFIRMED | probe: `"AB=="` → `00` → `base64UrlEncode` `AA`; `"Zg=="` → `66` | DESC |
| base64UrlDecode | 3 | CONFIRMED (prose) + **bug-606** | `base64UrlDecode("Z")` → `77050003` | DESC names `ErrInvalidFormat` |
| base64UrlEncode | 1 | CONFIRMED | probe: `base64UrlEncode([])` → empty | parameter |
| base64UrlEncode | 2 | REJECTED | input-unchanged class | — |
| codepageEncode | 1, 3 | CONFIRMED | probe: `codepageEncode(Utf8, "e\u{0301}")` → `65cc81` | INTRO and `codepage` parameter mention `Codepage.Utf8` |
| codepageEncode | 2 | CONFIRMED | probe: `codepageEncode(Iso8859_7, "A")` → `41` (ASCII is its own byte); `"世"` → `77050003` | example narrowed to non-Greek letters above U+007F |
| codepageDecode | 1 | CONFIRMED | probe: `codepageDecode(Windows874, [0xDB])` → `77050003`; a browser substitutes a replacement character | browser claim limited to defined bytes |
| formUrlDecode | 1 | CONFIRMED (prose) + **bug-606** | probe: `x%4`, `x%G0`, `%FF` → `77050003` | DESC names `ErrInvalidFormat` and its triggers |
| formUrlEncode | 1 | CONFIRMED | probe: `formUrlEncode("*-._~")` → `%2A%2D%2E%5F%7E` (a browser leaves `*-._`); `formUrlDecode` of either spelling → `*-._` | opening no longer claims the browser rule set |
| hexEncode | 1 | REJECTED | input-unchanged class | — |
| hexEncode | 2 | CONFIRMED | probe: `hexEncode([])` → empty | parameter |
| hexDecode | 1, 2 | CONFIRMED (prose) + **bug-606** | probe: `hexDecode("0")` and `hexDecode("zz")` → `77050003` | DESC names `ErrInvalidFormat` for bad digits and odd length |
| htmlEscape | 1 | CONFIRMED | probe: `htmlEscape("x onmouseover=alert(1)")` comes back unchanged, so the result is unsafe in an unquoted attribute | DESC: element content and **quoted** attribute values only |
| htmlEscape | 2 | REJECTED | input-unchanged class | — |
| htmlUnescape | 1 | CONFIRMED | probe: `htmlUnescape("&#55296;")` → `77020004` (`ErrEncoding`); the page said surrogates are accepted | DESC: surrogates raise `ErrEncoding` |
| htmlUnescape | 2 | CONFIRMED (prose) + **bug-606** | probe: `&#;`, `a &amp b`, `&#1114112;`, `&nosuch;` → `77050003`; the descriptor declares `errors: vec![]` | DESC names both errors; bug-606 row added |
| percentDecode | 1 | CONFIRMED (prose) + **bug-606** | probe: `percentDecode("%2")` → `77050003` | DESC names `ErrInvalidFormat` |
| percentEncode | 1 | REJECTED | input-unchanged class | — |
| punycodeEncode | 1 | CONFIRMED | probe: `punycodeEncode("a b.example")` unchanged; `"foo\u{3002}bar"` → `xn--foobar-rr3e` (no IDNA dot mapping) | DESC: plain RFC 3492 per ASCII-`.` label, no IDNA mapping or validation |
| punycodeEncode | 2 | CONFIRMED | probe: `"e\u{0301}.example"` → `xn--e-xbb.example`; "the package's UTF-8 decoder" was an implementation route | DESC: scalars, not graphemes |
| punycodeEncode | 3 | CONFIRMED | probe: empty → empty | parameter |
| punycodeDecode | 1 | CONFIRMED | probe: `punycodeDecode(punycodeEncode("xn--mnchen-3ya.de"))` → `münchen.de` | DESC: inverse only for labels not already beginning `xn--` |
| punycodeDecode | 2 | CONFIRMED | probe: empty → empty | parameter |
| punycodeDecode | 3 | CONFIRMED | probe: `punycodeDecode("xn--ib9b")` → `77020004` (`ErrEncoding`) | DESC: surrogate payload raises `ErrEncoding` |
| punycodeDecode | 4 | CONFIRMED | `func_punycode_decode.rs` strips `xn--` before `helper_puny_decode_label.rs` checks `len > 1024` | DESC: the bound excludes the prefix |
| punycodeDecode | 5 | CONFIRMED | probe: 1023 × `ü` → a 1029-octet label, which `punycodeDecode` rejects with `77050003` | DESC: `punycodeEncode` can exceed the bound |
| sleb128Decode | 1, 2 | CONFIRMED (prose) + **bug-606** | probe: `sleb128Decode([])` and `[0x80]` → `77050003`; 11 bytes → `77050003` | DESC + parameter name the error and the rules |
| sleb128Decode | 3 | CONFIRMED → **bug-619** | probe: nine `0x80` + `0x02` → `0`, no error; `IF shift > 63` is checked before the read; `uleb128Decode` has the same check | DESC no longer promises overflow detection |
| sleb128Encode | 1 | CONFIRMED | probe: the `Integer` minimum round-trips in 10 bytes; `0` → `00`, `-1` → `7f` | parameter and DESC: every `Integer` is valid |
| sleb128Encode | 2 | REJECTED | the shift sentence explains why negative values terminate, the observable contract; not internals | — |
| sleb128Encode | 3 | CONFIRMED | probe: `sleb128Encode(64)` → `c000`, `uleb128Encode(64)` → `40`; no separate sign byte | DESC: sign bit in the last group, so one more byte |

Retry round (user: "codex limit reset, continue"). The quota probe answered PONG,
and 10 units were dispatched from `planning/plan-125-units/C-phase2-retry.txt`.
My probes of those 10 pages, run while the reviews ran: `/tmp/p125-ex/encutf`,
`/tmp/p125-ex/ulebneg`, `/tmp/p125-ex/utf8amb`.

| Page | # | Verdict | Evidence | Applied |
|---|---|---|---|---|
| uleb128Encode | 1 | CONFIRMED (prose) + **bug-606** | probe: `uleb128Encode(-1)` → `77050003`; the descriptor declares `errors: vec![]` (the first encoder in bug-606) | parameter names `ErrInvalidFormat` |
| utf16Decode | 1 | CONFIRMED (prose) + **bug-606** | probe: `[65536]`, `[-1]` and a lone low surrogate → `77050003` | DESC names `ErrInvalidFormat` |
| utf16Encode | 1 | REJECTED | input-unchanged class | — |
| uleb128Decode | 1, 2 | CONFIRMED → **bug-619** | probe: nine `0xFF` then `0x01` → `-1`; nine `0x80` then `0x02` → `0` (the reviewer's `[0x80 x9, 0x01]` → the `Integer` minimum) | DESC no longer promises a non-negative result or overflow detection |
| uleb128Decode | 3 | CONFIRMED (prose) + **bug-606** | reviewer probe: `uleb128Decode([])` → `77050003` | DESC + parameter name `ErrInvalidFormat` |
| utf32Decode | 1, 2 | CONFIRMED (prose) + **bug-606** | probe: `[0x110000]`, `[0xD800]`, `[-1]` → `77050003` | DESC + parameter name the error and the valid range |
| utf32Decode | 3 | REJECTED | input-unchanged class; decoding in list order is already the page's statement | — |
| utf32Encode | 1 | CONFIRMED | reviewer probe: `"e\u{0301}"` → 2 elements (101, 769) | DESC: per scalar, not per user-perceived character |
| utf32Encode | 2 | CONFIRMED | reviewer probe: `utf32Encode("")` → 0 elements | parameter |
| utf8Decode | 1 | CONFIRMED (prose) + **bug-606** | probe: overlong `C0 80`, surrogate `ED A0 80`, above U+10FFFF `F4 90 80 80` → `77050003` | DESC names `ErrInvalidFormat` |
| utf8Decode | 2 | CONFIRMED | probe: `List OF Integer` with 256 → `77050003`; `[104, 105]` → `hi`; reviewer: empty lists → empty | parameter: 0–255, empty → empty |
| utf8Decode | 3 | CONFIRMED | "the selection is a compile-time decision, not a runtime dispatch" is compiler mechanics (`.ai/man-content.md` §3) | sentence cut; the range rule kept |
| utf8Encode | — | NO FINDINGS | my probe: a call with no expected type is `TYPE_OVERLOAD_AMBIGUOUS` at build, as the page says | — |
| varintDecode | 1 | CONFIRMED (prose) + **bug-606** | reviewer probe: `varintDecode([])` → `77050003` | DESC names `ErrInvalidFormat` |
| varintDecode | 2 | CONFIRMED → **bug-619** | probe: nine `0x80` then `0x02` → `0` | DESC no longer promises overflow detection |
| varintEncode | 1 | CONFIRMED | probe: `0`/`-1`/`1`/`-2` → `00`/`01`/`02`/`03`; the `Integer` minimum and maximum round-trip | parameter: every `Integer` valid |

Checked, left unchanged: `varintDecode`/`varintEncode` write the ZigZag mapping as
`(u >> 1) XOR -(u AND 1)`. MFBASIC has no `>>` operator, so this reads as
mathematical notation, not code to copy. That differs from the letter-B `bits::ctz`
case, whose `value AND -value` was offered as a usable idiom. No reviewer flagged it.

Found while applying, not raised by a reviewer, and left for letter G's
re-integration (no cross-page reconciliation here): `base32Decode`'s own
description still calls it "the inverse of `encoding::base32Encode`".

Acceptance: 32 units `exit 0`; sweeps clean for `encoding`; every type
description verified against the record/resource it describes.

Measured at the closing commit:
- every one of the 32 units has an `exit 0` row as its latest;
- `--reconcile` → `unaccounted=0 orphans=0`;
- `--memory-scope encoding` 0, `--scope encoding` 0;
- `encoding` examples 62/62 ran;
- the types page's 29 descriptions are verified: 128 high bytes decoded per
  codepage, all eight counts exact, and `Iso8859_8I` identical to `Iso8859_8`.

Bugs filed or extended from this phase: bug-606 (12 decoders, `uleb128Encode`,
`htmlUnescape`) and bug-619 (LEB128 decoders miss overflow).
Commit: f12fca000 (22 of 32), 52495d384 (closed)

### Phase 3 — datetime (46 units)

- [ ] 44 function pages + overview + the 40-description types page.
      **Progress (2026-09-14):** 27 of 46 reviewed and applied: overview, types,
      add, addDays, addMonths, between, civil, compare, date, dayOfYear,
      daysInMonth, duration, equals, fixedOffset, format, formatDuration,
      fromMillis, inZone, instant, isAfter, isBefore, isLeapYear, local,
      localOffset, minus, monotonic, monotonicNanos. The Codex usage limit stopped
      the second batch too (resets Sep 19, 06:05). The other 19 are in
      `planning/plan-125-units/C-phase3-retry.txt`. Every accepted finding was
      probe-verified: `/tmp/p125-ex/dtraw`, `dtraw2`, `dtgap`, `dtmisc`, `dtfmt`,
      `dtms`, `dtzone`, `dtinst`, `dtlo`, `dtminus`, and `dtmono`, plus `dtdst` under
      `TZ=America/New_York`, `TZ=Pacific/Honolulu`, and `TZ=Not/AZone`, and
      `dthavana` under `TZ=America/Havana`.
      Rejected: overview #4–6, generic derived Errors-table rows; types #6,
      covered by the new overview example; fromMillis #1, already on the page;
      monotonic #3, since `0 .. 999_999_999` is the package-wide inclusive notation.
      Applied beyond the findings, each verified: `startOfDay` in a zone whose
      DST gap starts at midnight returns `01:00` (Havana, 2026-03-08); the "OS
      intrinsic" wording is removed from `withZone` too; the signed-comparison
      sentence is fixed on all four comparison pages.
      Probing found bug-629: a built-in member accepts an argument of the wrong
      record type, e.g. `toMillis(DateTime)` compiles and returns garbage. Also
      as-is and documented, not filed: records built directly are not validated;
      `compare`/`equals` read stored fields; `daysInMonth` returns 31 for any
      month outside 1..12; a quoted pattern literal cannot hold an apostrophe.
- [x] Classify the 15 carve-out-1 arithmetic-borrow lines **once**, in this
      ledger, not per page.
      `./scripts/man-census.sh --memory-scope datetime` → 0 unclassified, 15
      CARVE-1, unchanged after the plan-135 merge. All 15 are the **subtraction
      borrow of nanosecond normalization**: when a nanosecond difference goes
      negative, one whole second is taken from the seconds field so `nanos` stays
      in `0 .. 999_999_999`. None describes memory or ownership, so all stay as
      written. The lines, re-measured after the 27-page edit, now number **16**:
      - `add` 54;
      - `between` 43, 44;
      - `duration` 35 (the new `nanos` parameter row), 66;
      - `fromMillis` 39;
      - `instant` 36 (the new `nanos` parameter row), 65;
      - `minus` 39, 51, 52;
      - `negate` 40;
      - `subtract` 39, 52, 53, 54.
- [ ] Every example compiled and run; every zone/DST/leap claim probe-verified
      (these are the claims most likely to be true-by-reasoning and false in
      fact).
- [ ] Ledger recorded.

Acceptance: 46 units `exit 0`; `--memory-scope datetime` reports exactly the
15 CARVE-1 rows and 0 unclassified; sweeps otherwise clean.
*Correction (2026-09-14):* the expected CARVE-1 count is now **16**. The accepted
`instant` finding #2 rewrote the `nanos` parameter row to state normalization
("a negative value borrows a second"). That is one more instance of the same
nanosecond borrow, so it is classified the same way. `./scripts/man-census.sh
--memory-scope datetime` → 16 CARVE-1, 0 unclassified.
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
