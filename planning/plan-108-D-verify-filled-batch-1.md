# plan-108-D: Verify the pre-filled packages, batch 1 — datetime, fs, encoding, collections, math

Last updated: 2026-08-30
Effort: large (3h–1d)
Depends on: plan-108-C (all authoring done; the workflow + reviewer prompts
have been through 10 packages — verification letters inherit a settled
standard).

Run the accuracy + scope + cross-model-review + apply cycle over the five
largest **pre-filled** packages — **datetime (45 function pages), fs (42),
encoding (28), collections (24), math (21) = 160 pages** plus overviews and
types pages. These pages already carry prose (filled during the builtins
migration); this letter's job is the user's mandate applied to them:
**verify every claim against the actual code, and verify the prose is
developer documentation, not compiler-internals spec** — then update from
the independent review.

Verification of an existing page is cheaper per page than authoring (B/C):
read the page, check each claim (probe program or descriptor table), compile
and run its example (never done before — A measured zero prior example
verification), apply the MUST-NOT scope list, move on; the cross-model
reviewer then re-verifies independently.

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
  Rendered baseline (2026-08-30): datetime 15, collections 4, fs 0,
  encoding 0, math 0. **All 15 datetime hits are carve-out 1 — arithmetic
  borrow** ("a negative nanos value borrows a second"), NOT memory: keep
  them and classify the whole set once in this letter's ledger rather than
  per page. `fs` looks clean only because its 37 source hits are Rust
  module-doc comments that never render (A's population table) — verify by
  rendering, never by grepping the `.rs` file. `collections`'s 4 are the
  real work here: the overview's copy/mutation contract is exactly what
  `mfb man variable` now owns, so cut and link rather than restate.
- `src/codegen/builtins/{datetime,fs,encoding,collections,math}/` — the
  pages under audit.
- `.ai/collections.md` — internals foil for collections prose (HOF rewrites,
  native lowering, in-place mutation mechanics = spec/internals; the man
  page states the developer contract: "helpers do not mutate their
  arguments" etc., which the collections overview already does well —
  verify, don't rewrite).
- Memory `tofloat-not-correctly-rounded` — a known behavior sharp edge
  (naive float parsing, ~1 ULP off): wherever a datetime/math/encoding page
  makes precision claims, verify them by probe, and document actual
  behavior honestly.
- Memory `inline-headroom-growable-record-collection` — records are
  `WITH`-only (no `a.field = v`): any example using record mutation syntax
  must be checked against real syntax.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-108-C complete | census shows every function page across all 30 packages carrying desc+example (denominator per A's Phase 1 census — the first draft's 466 excluded tcp/udp) | NOT MET until C lands |

## 1. Goal

- All 160 pages + 5 overviews + types pages verified claim-by-claim and
  scope-checked; every inaccuracy fixed; every internals leak rewritten in
  developer terms or removed.
- **`scripts/man-census.sh --memory-scope` reports 0** for every package in
  this letter (plan-108-A §3 (2a)): no `borrow`, `pointer`, `ownership`,
  `move`, `free`, `heap`, `lifetime` in rendered output. Where a `RES`
  handle's behavior must be stated, it is stated with **alias** and
  MFBASIC's own verbs (open / close / stays open); anything longer links
  `mfb man variable`.
- Every example compiled and run during the pass (fs examples against temp
  paths only — rewrite any example that touches a real user path);
  compile-only members, if any, noted in the ledger.
- Cross-model review (Codex) per package; ledgers (confirmed → fixed /
  rejected → disproving command) recorded here.
- Census still 100% for all five.

### Non-goals (explicit constraints)

- **No new inline explanation of the memory model.** Any page that needs
  more than one sentence about copies or handles links `mfb man variable`
  (authored in A) — it does not re-explain, and never in C/Rust terms.
- Per plan-108-A (no compiler testing; prose string fields only with
  per-commit `git diff` check; no renderer/schema changes; no
  `package.mfb` edits; `src/docs/man/**` untouched).
- **No wording churn on accurate, in-scope prose** — this is an audit, not
  a rewrite; a page that passes both passes is left byte-for-byte alone.
- Found code bugs: fix or file via write-bug, recorded here.

## 2. Current State

A's census: these five packages carry desc+example on 160 of their 161
pages (datetime 44/45, fs 41/42, encoding 28/28, collections 24/24, math
21/21 — the missing singletons were authored as C's stragglers). The prose
was written during the builtins migration era; it has never been
independently audited, and no example has ever been compiled or run
(A's measurement).

### Measured populations

| What | Count | Command |
|---|---|---|
| pages to verify | 160 (+5 overviews, 5 types pages) | `scripts/man-census.sh` at kickoff |
| examples never before compiled | 160 | A's measurement (zero prior example verification) |
| claims per page | unbounded prose — the reader and the reviewer, not a grep, are the coverage instrument | — |

## 3. Design Overview

Per-package: verification pass (steps 1+2 of the workflow, page by page,
fixing as found) → cross-model review → apply. Order: collections first
(the overview makes strong behavioral contracts — "do not mutate", ordering
rules — worth auditing early, and its 24 pages calibrate audit pace), then
math, encoding, fs, datetime (largest last, with pace known).

**Risk concentration:** rubber-stamping — an audit pass that reads prose as
plausible instead of checking it. Held by: probe-program discipline for
every behavioral claim that isn't table-derived (clamps/raises, rounding,
timezone/DST claims in datetime, path semantics in fs), and by the
cross-model reviewer whose prompt demands independent verification with
evidence, not proofreading.

### Rejected alternatives

- **Grep-driven claim extraction instead of page-by-page reading.**
  Rejected: prose claims have no uniform spelling (memory
  `census-a-behavior-by-its-effect` — counting by one spelling
  undercounts); the census bounds the page set, the reader/reviewer bound
  the claims.

## Compatibility / Format Impact

None to codegen/wire. Summary-pin update only if a pinned summary is itself
corrected.

## Phases

### Phase 1 — collections, math

- [x] Verify collections **49** + math **21** pages + overviews + types
      pages; every example compiled and run (collections 140/140,
      math 21/21 — `scripts/man-run-examples.sh <pkg> --run`).
      Counts corrected, see Corrections.
- [x] Cross-model review per package + apply; ledgers here.
- [x] Verify: rendering reads clean; census still 100%
      (`scripts/man-census.sh --fill collections math`).

**Ledger — collections (33 findings, all LEAKAGE, all applied).** The
package explained itself in compiler terms throughout. Rewritten by kind:

| Was | Now |
|---|---|
| "It is a **native** member: the compiler emits the <X> loop directly rather than instantiating an MFBASIC generic" (`filter`, `find`, `forEach`, `mid`, `reduce`, `sum`, `transform`) | deleted — a developer cannot act on it |
| "When the compiler can prove the target is a same local being reassigned … it lowers the call to an in-place grow with geometric spare capacity" (`add`, `append`, `set`, `remove`, `prepend`) | kept the FACT, dropped the mechanism: "Assigning straight back to the same local variable — `list = collections::append(list, x)` — is the cheap shape: it grows the list instead of copying it… Appending to something reached another way (a parameter, a module-level `MUT`, or the list a `FOR EACH` is walking) copies the whole list on every call." |
| "the compiler's in-place assignment recognizers cover `append`, bulk `append`, `prepend`, `set`, and string concatenation, not `insert`" (`insert`, `removeAt`, `replace`) | "Unlike `append`, `prepend`, and `set`, there is no cheap in-place shape for `insert` at an arbitrary index: every call copies the list." |
| "hash bucket index … linear scan of the entry table" (`get`, `getOr`, `hasKey`) | "is a direct lookup; other key types are found by scanning the map" |
| "the lookup-entry table … its own storage is not aliased … separate derived metadata" (`keys`, `values`) | "The map keeps its entries in insertion order and `keys` walks them in that order" |
| "a composite payload stored inline in the collection's data region is copied into a standalone arena block before it is handed back, so binding, storing, and freeing the result cannot disturb the source" (`get`) | "you get a copy, so nothing you do with it afterwards can disturb the collection it came from. See `mfb man variable`." |
| "freeing it would turn a leak into a use-after-free. Intermediate accumulators are likewise left unfreed" (`reduce`) | "The reducer may return one of the elements it was given as the new accumulator, and that is safe to do." |
| "the internal slice helper … lowered natively as a bulk range copy" (`chunks`, `drop`, `take`, `window`) | "The remaining elements are copied into a new list" |
| "the compiler's inline-built-in fallibility census classifies `sum` as infallible" | "`sum` is treated as **infallible** for the purpose of inline `TRAP`" |
| "does not share storage with `value`" (`drop`, `take`, `chunks`) | "nothing you do with the result affects `value`" |

**Ledger — math (6 findings: 1 INACCURACY, 1 MISSING, 4 LEAKAGE; all
applied).**

- **INACCURACY + LEAKAGE.** "Every function lowers inline at the call site,
  like `bits::*`, rather than calling a runtime helper, and produces
  identical results on the native and Binary Representation execution
  paths." False (`pow`/`atan2`/`rand`/`seed` call helpers) *and* internals.
  Deleted; replaced with the return-type rules a caller needs.
- **LEAKAGE.** intro "Scalar and vectorized (SIMD) numeric functions" →
  "Numeric functions and constants". SIMD is a mechanism, not behaviour.
- **LEAKAGE.** "this thread's PCG64 generator" (`rand`, `seed`) → "this
  thread's random sequence".
- **MISSING — the biggest of the six.** The overview said "14 compile-time
  constants (`pi`, `e`, `ln2`, and friends)" and stopped. `mfb man math pi`
  answers `error: unknown math function 'pi'`, so 11 of the 14 names were
  undiscoverable from the documentation. The overview now carries the full
  Float/Fixed/value table and says explicitly that a constant has no page of
  its own because it is a value, not a function.

Acceptance: both packages verified and reviewed; ledgers recorded.
Commit: —

### Phase 2 — encoding, fs

- [x] Verify encoding 28 + fs **41** pages + overviews + types pages; fs
      examples rewritten onto paths they create themselves, all compiled
      and run (encoding 57/57, fs 94/94).
- [x] Cross-model review + apply; ledgers.
- [x] Verify: rendering + census as Phase 1.

**Ledger — fs (33 findings; 16 of them one defect).** The review found
**16 examples that do not run**, all the same defect: they read or write
under a directory the example never creates — `target/` in fourteen cases,
which exists in this repository's own checkout but never in a reader's
project. `fs::writeText("target/output.txt", "Hello")` fails with
`ErrNotFound` for anyone who pastes it.

This also exposed a **defect in `scripts/man-run-examples.sh`**: it ran each
built example with the *repository* as the working directory, so `target/`
resolved to cargo's own `target/` and all 16 passed. Fixed (the binary now
runs with the scratch project as cwd) — see Corrections. With the fix the
harness reproduces all 16 failures exactly, and after the content fix reports
94 built / 94 run / 0 failed.

Each example now creates what it needs, which also demonstrates
`fs::createDirectories`:

```
SUB main()
  fs::createDirectories("output")
  fs::writeText("output/report.txt", "Hello")
END SUB
```

The remaining fs findings were the same LEAKAGE classes as collections.

**Ledger — encoding (3 findings, all applied).**

- **INACCURACY.** The overview said "Decoders reject malformed input with
  `ErrInvalidFormat`". Base32/Base64 do **not** reject a non-canonical final
  group. Verified: `encoding::base64Decode("AB==")` and
  `encoding::base32Decode("AB======")` each return a single `0` byte instead
  of raising. The page now says so and warns: "Do not use a decode round-trip
  as a canonical-form check."
- **MISSING.** `htmlUnescape` said a named reference is "looked up in the
  built-in entity table" without saying the table is finite. `&alpha;` raises
  `ErrInvalidFormat` ("unknown entity"). The page now lists all **44**
  recognised names and says any other must be written numerically.
- **LEAKAGE.** `utf8Encode`'s "the exact bytes that make up the string's
  storage" → "the UTF-8 bytes of the string, one list element per byte".

Acceptance: both packages verified and reviewed.
Commit: —

### Phase 3 — datetime

- [x] Verify **44** pages + overview + types page; timezone/DST/precision
      claims probe-verified; examples compiled and run (112/112).
- [x] Cross-model review + apply; ledger.
- [x] Verify: rendering + census as Phase 1.

**Ledger — datetime (12 findings: 3 INACCURACY, 9 LEAKAGE; all applied).**

- **INACCURACY (reproduced here before applying).** The overview said
  "Everything civil — `Date`, `Time`, and `DateTime` — is a projection of an
  instant through a `Zone`." A hand-built `DateTime` is not checked:
  `DateTime[Date[2026,6,26], Time[9,30,0,0], datetime::utc(), 3600]`
  projected back through UTC gives `2026-06-26T08:30:00.000Z`, an hour off
  the `09:30` supplied, because `resolve` believes the offset field. The
  claim is now scoped to values the package *produces*, with the constructor
  caveat spelled out.
- **INACCURACY.** "Only three operations touch the host" omitted the three
  public `*Nanos`/`localOffset` entry points. Now: "the wall clock (`now`
  and `nowNanos`), the monotonic counter (`monotonic` and `monotonicNanos`),
  and local-zone offset resolution".
- **INACCURACY.** `now`'s "the only wall-clock entry point in the package" →
  "the package's wall-clock entry point that returns an `Instant`
  (`datetime::nowNanos` reads the same clock as a bare nanosecond count)".
- **LEAKAGE ×9.** `clock_gettime(CLOCK_REALTIME)`, `tv_sec`, libc `timespec`,
  `localtime_r`/`tm_gmtoff`, "lowers to a libc runtime helper", "returns an
  `Integer` in the result register with the OK tag set", Howard Hinnant's
  civil↔epoch-day algorithm, `daysFromCivil(...) * 86400`, and the
  `ZoneKind::Utc`/tag-`0`/`FixedOffset`/tag-`1`/`Local`/tag-`2` walkthroughs.
  All removed. Note `ZoneKind` is a **public enum of this package**, so its
  variant *names* stay — respelled the MFBASIC way (`ZoneKind.Utc`), without
  the Rust `::` path or the numeric tags.

Acceptance: datetime verified and reviewed.
Commit: —

## Validation Plan

- Verification: `mfb man <pkg> --all`/`types` per package; census still
  100%; examples and probes compiled/run ad hoc with the release binary.
- Doc sync: none beyond content.
- Hygiene: fmt at session end.

## Open Decisions

- None entering the letter.

## Corrections

1. **Page counts were wrong for four of the five packages.** The plan said
   collections 24, math 21, encoding 28, fs 42, datetime 45. Measured with
   `scripts/man-census.sh --fill <pkg>`:

   | Package | Plan said | Actually | Note |
   |---|---|---|---|
   | collections | 24 | **49** | more than double |
   | math | 21 | 21 | correct |
   | encoding | 28 | 28 | correct |
   | fs | 42 | **41** | |
   | datetime | 45 | **44** | |

   Scope corrected in place above. No other letter derived a count from
   these.

2. **`scripts/man-census.sh` counted `math`'s constants as pages.**
   `functions_of` scraped every `│ math::name` cell from the whole overview,
   and the Constants table this letter added put `math::pi` … `math::ln10`
   into that scrape. The census then reported "7 pages with neither
   Description nor Examples" for members that have no page at all
   (`mfb man math pi` → `error: unknown math function 'pi'`). Fixed by
   scoping the scrape to the overview's *Functions* table. After the fix
   `--fill math` reads `21 21 21 21 28/28`, not `28 21 21 21`.

3. **`scripts/man-run-examples.sh` ran examples with the wrong working
   directory** — the repository root rather than the scratch project. A
   relative path in an example therefore resolved against cargo's `target/`,
   and 16 broken `fs` examples reported as passing. This is the defect that
   made the instrument agree with the documentation instead of testing it.
   Fixed: the built binary now runs with the scratch project as cwd. Before
   the fix `fs` reported 94 run / 0 failed; after it, 78 run / 16 failed —
   exactly the 16 the independent review found.

4. **The harness treated a documented failure as a harness failure.**
   `collections::groupBy`'s third example demonstrates error propagation and
   the page states its output ("prints, and exits non-zero: `failed:
   77050002`"). The harness now accepts a non-zero exit when every line the
   program printed appears in the rendered page — which additionally checks
   the page's stated output against the real one.

5. **Two example-only conveniences were added to the harness**, both to
   verify by *running* rather than downgrading a page to compile-only:
   `STDIN_FILE=` feeds real input to the `io` examples that read stdin (22/22
   now run), and the companion `workers` package the `thread` and `os::sleep`
   examples import is built automatically (13/13 now run).

## Summary

The heavy verification batch: the five biggest migrated-prose packages go
under the same evidence discipline the authored packages were born under —
probe-verified claims, scope-checked prose, every example finally compiled
and run, independently reviewed.
