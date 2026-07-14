# plan-45: Benchmark coverage — critical-feature hot paths to add

Last updated: 2026-07-14
Effort: medium (each benchmark is a self-contained `test_*` in all three languages)
Companion to `planning/plan-44-benchmark-perf.md` (the fix plan).

The current suite (`benchmark/{mfb,c,python}`) is an **API-surface coverage check** plus
the plan-40 **pattern-throughput** groups (mapchurn/listchurn/mathpipe/strbuild/
regexbench/arena/scalarbench). Between them the suite now exercises every `collections::`/
`math::`/`vector::`/`bits::`/`strings::` member, sustained collection churn, chained
string/regex pipelines, and the `Scalar` primitive. But three kinds of hot path are still
**untouched**, and two whole packages have **zero** coverage:

1. **`encoding::`** (base64/base32/hex/percent/html/formUrl/punycode) — a flagship
   package with **no benchmark at all**, yet it is a classic throughput hot path (serialize/
   deserialize) with direct C/Python peers.
2. **`datetime::`** civil arithmetic + format/parse — the package is used only as the
   suite's *timer* (`monotonicNanos`); its calendar math and formatting are unbenchmarked.
3. **Control-flow dispatch** — union + `MATCH` tag dispatch and inline-`TRAP` error
   recovery are core language features with no throughput row.

This plan adds throughput benchmarks for those hot paths, tracked as critical MFB
features, without duplicating the existing per-member or plan-40 rows.

## Design rules (match the existing suite)

- One self-contained workload per language (`benchmark/mfb/src/*.mfb`,
  `benchmark/c/*.c`, `benchmark/python/*.py`), timed internally with
  `datetime::monotonicNanos()` (mfb) / matching clock, printing a checksum on stderr so
  all three agree. Register each in `main.*`'s driver + the group table + the README
  coverage table.
- **New group per theme** (or extend an existing group file). Keep the C/Python mirrors
  doing the *same materialized work* (README parity contract). Where mfb has no
  cross-language peer (Money), mark the row mfb-only (like `math fixed` / `mathpipe finance`).
- **Arena-sensitive rows must wait on plan-44-J** (the arena mixed-transient-churn
  sub-plan — successor to plan-39-A). The runtime arena free list still degrades
  quadratically under mixed-size transient churn, so any new churn/arena benchmark
  authored at realistic size will hang the suite until J lands. Author them now at tiny
  "smoke" counts with a `TODO(plan-44-J): raise N once arena fixed` marker, then bump N in
  the commit that closes J — making these rows a regression gate for it.

## What's already covered (do NOT duplicate)

Read from the current suite (`benchmark/mfb/src/*.mfb` + README):

- **list / liststr:** every `collections::` list op (append/prepend/insert/removeAt/get/
  set/copy/distinct/groupby/sort/sortBy/all/any/chunks/contains/drop/filter/find/findIndex/
  findLastIndex/flatten/forEach/mid/partition/reduce/reduceRight/replace/sum/take/transform/
  window/zip) over Integer **and** String lists.
- **listchurn / mapchurn:** build-by-append, prepend front-shift, nested `List OF List`
  build+flatten+groupBy; map grow/rehash, steady-state insert/`removeKey` churn,
  keys/values/mapValues/merge iterate.
- **map:** set/lookup + int_ops/str_ops aggregates over **String-keyed** maps.
- **math:** every transcendental (sin/cos/tan/atan2/asin/acos/atan/exp/log/log10/pow/sqrt)
  over an array; `math float/int/fixed/simd` families.
- **float / mathpipe:** leibniz, nbody, mandelbrot, matmul; dft, stats, `finance` (mfb-only
  Money running-balance).
- **vector:** math/float/fixed/int families.
- **strings / strbuild:** concat/case/search/slice + unicode smoke (`unicode`/`unibig`);
  `&`-concat vs join, split/join round-trip, replace/trim clean chain.
- **bits:** every bitwise/shift/rotate op incl popCount. **bignum:** modmul/modexp.
- **parse:** csv/json/regex (one shape each). **regexbench:** compile/capture/alternation/
  replace. **io:** write/read/readnum/buf_on/buf_off/format/binary.
- **scalarbench:** roundtrip/classify/transform/listchurn (the `Scalar` primitive).
- **recurse:** fib/ackermann. **primes. thread** sum. **record** update. **arena:**
  transient/mixed/growshrink (the plan-44-J regression gate).

Deliberately-tiny arena-gated rows (README:104-123): `string unicode`, `liststr reshape`,
`string unibig`, `io binary`, whole `arena` group, `scalarbench roundtrip/transform/
listchurn`. **Do not re-benchmark any of the above.**

## Proposed new benchmarks

Grouped by hot-path theme. Each row: **why it's a distinct hot path**, the **workload**,
and the **real API members** it exercises (all verified against the source packages —
`src/builtins/encoding_package.mfb`, `datetime_package.mfb`, `money_package.mfb`).

### Theme 1 — `encoding::` serialize/deserialize (new group `encoding`) — tracks the encoding package (zero coverage)

The encoding package has **no** benchmark today, yet encode/decode is a canonical
throughput hot path with exact C/Python peers, and it stresses the `List OF Byte` ↔ String
seam the string rows never hit.

1. **base64 round-trip** — encode a multi-KB byte buffer to base64 then decode back, in a
   loop. Distinct: the 3-byte→4-char bit regrouping over a `List OF Byte`. Cross-language
   (Python `base64.b64encode/b64decode`, C vendored or hand-rolled). API:
   `encoding::base64Encode`/`base64Decode` (+ `strings::toBytes`/`fromBytes` for the seam).
   **Arena-gated (plan-44-J):** allocates a fresh byte list + String per call — author tiny,
   raise N when J lands.
2. **hex round-trip** — `hexEncode`/`hexDecode` over the same buffer (nibble mapping, a
   different inner loop from base64). Cross-language (Python `bytes.hex`/`fromhex`). API:
   `encoding::hexEncode`/`hexDecode`.
3. **percent/URL encode** — `percentEncode`/`percentDecode` over a URL-ish string with
   reserved chars (the web hot path; per-byte reserved-set check + `%XX` expansion).
   Cross-language (Python `urllib.parse.quote/unquote`). API: `encoding::percentEncode`/
   `percentDecode` (or `formUrlEncode`/`formUrlDecode`).

### Theme 2 — `datetime::` civil arithmetic & formatting (new group `datetime`) — tracks the datetime package (zero coverage)

The suite calls `datetime::monotonicNanos` only as a stopwatch; the calendar math and
formatting — the actual user-facing work — are unbenchmarked.

4. **civil date arithmetic** — over a loop, `addDays`/`addMonths` walk a date forward and
   `between`/`daysFromCivil` compute spans (the days↔civil conversion is the hot kernel).
   Cross-language (C `<time.h>` `mktime`/day math, Python `datetime` + `timedelta`). API:
   `datetime::civil`/`addDays`/`addMonths`/`between`/`daysFromCivil`/`daysInMonth`.
5. **ISO format / parse round-trip** — format a datetime to an ISO string then parse it
   back, in a loop (the serialization hot path; token expansion + integer→String). Cross-
   language (Python `strftime`/`strptime`, C `strftime`/`strptime`). API: `datetime::format`
   + the parse entry (verify the public parse member name against `datetime_package.mfb`
   before authoring). **Arena-gated (plan-44-J):** per-call String churn — author tiny,
   raise N when J lands.

### Theme 3 — control-flow dispatch (new group `dispatch`)

Union `MATCH` tag dispatch and inline-`TRAP` recovery are core language features with no
throughput coverage; both are hot in real interpreters/parsers.

6. **union + MATCH eval** — build a small expression tree as a `List` of a tagged union
   (e.g. `Num`/`Add`/`Mul`), then evaluate it many times via `MATCH` on the variant tag
   (the tag-dispatch hot path — distinct from `record update` and from `fib`'s call
   overhead). Cross-language (C tagged-union `switch`, Python `match`/isinstance). API:
   union types + `MATCH`.
7. **inline-TRAP recovery** — a loop that runs a fallible op (e.g. `toInt` of mixed
   valid/invalid tokens, or a divide that can trap) with an inline `TRAP (err)` recovering
   per iteration, so the error path is taken a fixed fraction of the time — quantifies the
   error-route cost the happy-path rows never pay. Cross-language (C errno-check, Python
   `try/except`). API: inline `TRAP` on a fallible builtin (see memory
   `plan-21-inline-trap-on-builtins`).

### Theme 4 — map key-type & collection shape (extend `map` / `listchurn`)

The map rows are all **String-keyed**; the Integer-key hash path is a distinct code path,
and it doubles as a regression gate for plan-44-A/B (in-place removeKey / native merge).

8. **integer-keyed map churn** — build/lookup/`removeKey` over an **Integer-keyed** map
   (distinct FNV/probe path from the String-keyed `mapchurn` rows; also exercises plan-44-A's
   in-place removeKey on integer keys). Cross-language (C open-addressing int map, Python
   `dict[int]`). API: `collections::set`/`get`/`hasKey`/`removeKey` on `Map OF Integer` keys.
   **Arena-gated (plan-44-J)** for the churn variant — author tiny, raise N when J lands.
9. **Map OF List aggregation** — group N items into `Map OF List` by key and append into
   the buckets in a loop (the group-and-append pattern at map depth; ties to plan-44-B's
   native groupBy). Cross-language (C hash-of-vectors, Python `defaultdict(list)`). API:
   `collections::hasKey`/`get`/`set`/`append` on a `Map OF List`.

### Theme 5 — numeric & exact-decimal pipelines (extend `mathpipe`)

10. **memoized DP recursion** — a memoized recurrence (e.g. Fibonacci or coin-change) over
    a `Map`/`List` memo table (tree recursion + memo lookups — distinct from raw `fib`,
    which has no memo). Cross-language. API: `collections::hasKey`/`get`/`set` + recursion.
11. **Money pipeline (mfb-only)** — over a `List OF Money`, run a tax/allocate/split
    computation (extends `mathpipe finance`; flagship exact base-10 decimal with no C/Python
    peer). Marks more of the Money surface as a tracked feature. **mfb-only**, like
    `mathpipe finance` / `math fixed`. API: Money arithmetic (`+`/`*`) + `money::` rounding/
    allocation members (verify names against `money_package.mfb` before authoring).

### Theme 6 — sort adaptivity (extend `list`)

12. **sort by input shape** — `collections::sort` over **pre-sorted**, **reverse-sorted**,
    and **random** Integer lists as three rows (the merge sort's best/worst/average cases).
    Today only the descending-key `sortBy` worst case is covered; these catch an
    adaptive-sort regression and prove O(n log n) holds across shapes. Cross-language
    (`qsort` / Python `sorted`, same materialized order). API: `collections::sort`.

## Rollout / phasing

- **Phase 1 (now, safe — not arena-sensitive):** encoding hex/percent (2, 3), datetime
  civil arithmetic (4), dispatch union-MATCH + TRAP (6, 7), integer-map **lookup** portion
  (8, non-churn), Map OF List aggregation (9), memoized DP (10), Money pipeline (11), sort
  adaptivity (12). These measure real gaps immediately and give plan-44 more signal.
- **Phase 2 (with plan-44-J):** the arena-gated rows — encoding base64 round-trip (1),
  datetime ISO format/parse (5), integer-map **churn** (8) — authored tiny in Phase 1 with
  `TODO(plan-44-J)`, bumped to realistic N in the commit that lands J, doubling as its
  acceptance gate (must jump from tiny to realistic and stay linear).
- Each new row lands in all three languages simultaneously with a matching checksum,
  updates `benchmark/README.md`'s coverage table, and keeps the git-ignored logs
  regenerable via `benchmark/run.sh`.

## Non-goals

- No network/tls/http benchmarks (non-deterministic, external-dependency).
- No new language surface — every benchmark uses existing, documented members (encoding/
  datetime/money members verified against the source packages; union/`MATCH`/`TRAP` are
  core language features).
- Not a replacement for the per-member coverage rows or the plan-40 pattern-throughput
  rows — this is *additive*; the existing suite stays as the surface + churn check.
