# plan-87: Benchmark coverage — critical-feature hot paths to add

Last updated: 2026-08-04
Effort: medium (each benchmark is a self-contained `test_*` in all three languages)
Companion to `planning/plan-86-benchmark-perf.md` (the fix plan).

The current suite (`benchmark/{mfb,c,python}`) is now **very broad**: the bug-430
collection-container matrix (list/map/set × `Fixed`/`Dynamic` × plain/`Record`/`State`,
each op split one-per-function), the plan-40 pattern-throughput groups
(listchurn/mapchurn/strbuild/regexbench/mathpipe/arena/scalarbench), the plan-45
extensions, and the plan-65 crypto/serialize groups. Between them it exercises every
`collections::`/`math::`/`vector::`/`bits::`/`strings::`/`encoding::`/`datetime::`/
`crypto::`/`json::`/`csv::` member, union `MATCH` + inline `TRAP`, sustained churn, the
`Scalar` primitive, and the `Set OF T` type. The remaining gaps are narrow and specific:

1. **`strings::displayWidth`** (plan-70) — a new built-in with **zero benchmark coverage**.
   Terminal display-width is a distinct hot path (UAX #29 EGC segmentation + a per-cluster
   utf8proc charwidth lookup), separate from `len`/`byteLen`/`graphemesCount`.
2. **Chained collection HOF pipelines** — the suite benches `filter`/`transform`/`reduce`
   each *alone* (one call per member) but never a **chained pipeline**
   (`filter → transform → reduce`), which is what real programs run and which stresses the
   distinct cost of intermediate-list materialization between stages.
3. **In-memory number conversion throughput** — `toString`/`toInt`/`toFloat` are exercised
   only *inside* the IO-bound `io format`/`readnum` rows; a pure conversion loop (parse +
   render, no file IO) is a distinct hot path (logging, serialization inner loops).

This plan adds throughput benchmarks for those hot paths without duplicating the existing
per-member or pattern-throughput rows.

## Prerequisites

All three themes use members that ship today (verified against `mfb man`):

| # | Prerequisite | Check | Status (2026-08-04) |
|---|---|---|---|
| P1 | `strings::displayWidth` exists | `mfb man strings displayWidth` resolves | **MET** — landed by plan-70-A (`strings::displayWidth(value AS String) AS Integer`) |
| P2 | `collections::filter`/`transform`/`reduce` exist | `mfb man collections filter` / `transform` / `reduce` resolve | **MET** — all ship |
| P3 | `toString`/`toInt`/`toFloat` exist | `mfb man general toString` / `toInt` / `toFloat` resolve | **MET** — always-in-scope general functions |

## Design rules (match the existing suite)

- One self-contained workload per language (`benchmark/mfb/src/*.mfb`, `benchmark/c/*.c`,
  `benchmark/python/*.py`), timed internally with `datetime::monotonicNanos()` (mfb) /
  matching clock, printing a checksum on stderr so all three agree. Register each in
  `main.*`'s driver + the group table + the README coverage table.
- **New group per theme** (or extend an existing group file). Keep the C/Python mirrors doing
  the *same materialized work* (README parity contract). Where mfb has no cross-language peer,
  mark the row mfb-only (like `math fixed`), printed as `--`.
- **The arena mixed-transient-churn quadratic is largely retired** (plan-64-A landed:
  `datetime civil` 973→0.78 flat, `parse csv` 447→5.01 near-linear), so most churn rows can now
  be authored at realistic size. **The one residual is the grapheme-transient-churn quadratic**
  (`[[arena-transient-churn-quadratic-graphemes]]`): `strings::graphemes`/`graphemesCount`/
  `graphemeAt`/`normalizeNfc`/`toBytes` churned in one loop still degrades. A new row that
  segments **many fresh strings into graphemes** (Theme 1's realistic tier) must be authored
  tiny with a `TODO(arena grapheme-churn)` marker until that residual is fixed; the same-buffer
  single-pass variant is fine at realistic size.

## What's already covered (do NOT duplicate)

Read from the current suite (`benchmark/{mfb,c,python}` + `benchmark/README.md`):

- **list / map / set (bug-430 matrix):** every `collections::` list/map/set op split
  one-per-function, over `Fixed` (Integer) and `Dynamic` (String) elements, in a plain local, a
  record field, and a `File STATE` field (the plain `Fixed`/`Dynamic` + map `key-*` groups are
  cross-language; Record/State are mfb-only). `sort_asc`/`sort_desc`/`sort_rand` adaptivity.
- **listchurn / mapchurn:** build-by-append, prepend front-shift, nested build+flatten+groupBy;
  map grow/rehash, steady-state churn, keys/values/mapValues/merge iterate.
- **math / mathpipe / float:** every transcendental + int/fixed/simd; leibniz/nbody/mandelbrot/
  matmul; dft/stats/memo/finance/money.
- **vector:** math/float/fixed/int families. **bits:** every op, one row per op. **bignum:**
  modmul/modexp.
- **strings / strbuild:** `&` concat, case, search, slice, `unicode`/`unibig` smoke; join/
  splitjoin/clean. **encoding:** base64/hex/percent. **datetime:** civil arithmetic + iso
  round-trip. **dispatch:** union+MATCH, inline TRAP.
- **crypto** (plan-65): sha256/sha512/hmac/pbkdf2/cte/churn/ed25519. **serialize** (plan-65):
  json/roundtrip/csv stringify. **parse:** csv/json/regex. **regexbench:** compile/capture/
  alternation/replace. **io:** write/read/readnum/buf_on/buf_off/format/binary.
- **scalarbench:** roundtrip/classify/transform/listchurn. **recurse:** fib/ackermann.
  **primes. thread** sum. **record** update. **arena:** transient/mixed/growshrink.

Deliberately-tiny arena-gated rows (README arena caveat): `string unicode`/`unibig`, `io binary`,
the whole `arena` group, `scalarbench roundtrip/transform/listchurn`, the crypto `sha512`/`hmac`/
`pbkdf2`/`churn`/`ed25519` rows, the whole `serialize` group, and the collection-matrix
set-producing / String-reshape rows (still authored small). **Do not re-benchmark any of the above.**
Every list/map/set op is already matrix-covered — **do not add another per-member row**; this plan
is strictly *pattern throughput* + one *new-feature* group.

## Proposed new benchmarks

Grouped by hot-path theme. Each row: **why it's a distinct hot path**, the **workload**, and the
**real API members** it exercises (all verified against `mfb man` — no invented surface).

### Theme 1 — Unicode display width (new group `width`) — tracks `strings::displayWidth` (zero coverage)

`strings::displayWidth` (plan-70) has **no** benchmark. It is a distinct fourth string measure
(vs `len` = scalars, `byteLen` = UTF-8 bytes, `graphemesCount` = clusters): it segments the string
into UAX #29 extended grapheme clusters and sums each cluster's terminal column width (0/1/2) from
the vendored utf8proc charwidth table (`mfb man strings displayWidth`). That EGC-segmentation +
per-cluster width lookup is exactly the layout hot path a terminal renderer runs.

1. **width ascii** — `strings::displayWidth` over a fixed multi-KB **ASCII** buffer, in a loop.
   *Distinct:* the all-narrow fast path (segmentation with no wide/zero-width clusters) — measures
   raw EGC-walk throughput. *Workload:* one fixed ~4 KB ASCII string, `reps` folds summing the
   width. **Realistic N** (single fixed buffer, same-buffer walk — not grapheme *churn*). API:
   `strings::displayWidth`. Cross-language: C `wcswidth` over the same bytes, Python a hand-rolled
   ASCII=1 width (both trivially match on pure ASCII). Checksum = total columns.
2. **width mixed** — `strings::displayWidth` over a fixed buffer mixing ASCII, CJK wide ideographs
   (2 cols), combining marks (0), and a ZWJ emoji family (1 cluster), in a loop. *Distinct:* the
   wide/zero-width/emoji branches + real EGC boundaries — the slow path the terminal grid uses.
   *Workload:* one fixed curated string, `reps` folds. **Same-buffer, realistic N.** API:
   `strings::displayWidth` (+ `strings::graphemesCount` for a companion count checksum). C/Python
   peers hand-roll a small width table over the *controlled* corpus so the column total matches
   bit-for-bit (the corpus is chosen so a fixed table suffices; follows the `string unicode`
   precedent where the peer approximates — mark mfb-only if exact parity proves infeasible).
3. **width churn (arena-gated)** — `strings::displayWidth` over **many fresh** short strings built
   per iteration (the realistic "measure each row of a table" pattern — per-call grapheme
   segmentation over fresh String temporaries). *Distinct:* the grapheme-transient-churn path.
   **Arena-gated** on the residual grapheme-churn quadratic (`[[arena-transient-churn-quadratic-graphemes]]`):
   author tiny with a `TODO(arena grapheme-churn)` marker; raise N when that residual lands, doubling
   as its regression gate. API: `strings::displayWidth` + `toString` (to build fresh rows).

### Theme 2 — chained collection HOF pipeline (new group `pipeline`)

The matrix benches each HOF alone; no row chains them. A `filter → transform → reduce` pipeline is
the canonical collection-processing shape and stresses a cost the per-member rows never show: the
**intermediate list materialized between stages** (filter's output list, transform's output list),
plus back-to-back indirect FUNC dispatch. It is also a natural future fusion target, so it is worth
tracking as its own regression row.

4. **pipeline int** — over a `List OF Integer` of ~N: `collections::filter` (keep even) →
   `collections::transform` (×3+1) → `collections::reduce` (sum), repeated `reps`. *Distinct:* two
   intermediate lists + three HOF passes vs the single-op rows. **Realistic N** (Integer, fixed-width
   — not arena-sensitive). API: `collections::filter`/`transform`/`reduce`. Cross-language (Python
   `filter`/`map`/`functools.reduce` or comprehensions; C hand-rolled three-pass). Checksum = the sum.
5. **pipeline groupagg** — `collections::groupBy` a `List OF Integer` by `n MOD K` → `mapValues`
   each bucket to its `reduce`-sum → `values`/`reduce` the bucket sums. *Distinct:* the map-of-list
   aggregate-then-fold ETL shape (distinct from `listchurn nested`, which builds+flattens but does
   not fold buckets). **Realistic N** (Integer). API: `collections::groupBy`/`mapValues`/`reduce`/
   `values`. Cross-language. Checksum = the folded total.
6. **pipeline str (arena-gated)** — the same `filter → transform → reduce` chain over a
   `List OF String` (transform = `strings::upper`; reduce = concat-length fold). *Distinct:* the
   String-element pipeline exercises plan-86 sub-plan A (String native lowering) end-to-end.
   **Arena-gated / reduced-size** — String reshape churn; author small (mirrors the matrix
   `Dynamic` reshape rows) with a `TODO(plan-86-A)` marker. API: `collections::filter`/`transform`/
   `reduce`, `strings::upper`.

### Theme 3 — in-memory number conversion (extend `scalarbench` or new group `convert`)

`toString`/`toInt`/`toFloat` are exercised only inside the file-IO-bound `io format`/`readnum` rows,
so their cost is entangled with `fs` writes. A pure in-memory parse+render loop isolates the
conversion hot path (logging/serialization inner loops), which is genuinely distinct.

7. **convert int** — a loop that renders `toString(i)` and re-parses `toInt(s)` for a range of
   integers, folding a checksum. *Distinct:* the integer↔text formatter/parser with no IO. **Realistic
   N.** API: `toString(Integer)`, `toInt(String)`. Cross-language (Python `str`/`int`, C `snprintf`/
   `strtoll`). Checksum = folded round-trip sum.
8. **convert float** — `toString(f, 6)` render + `toFloat(s)` re-parse over a deterministic float
   sequence, folding a bit-checksum of the reparsed values. *Distinct:* the float formatter
   (`float_format.rs`) + parser round-trip — the intrinsic cost plan-86 sub-plan L flagged for
   `io format`, isolated. **Realistic N.** API: `toString(Float, decimals)`, `toFloat(String)`.
   Cross-language (Python `f"{x:.6f}"`/`float`, C `snprintf %.6f`/`strtod`). Checksum = a
   round-trip-stable integer fold (choose a rendering both parsers reproduce exactly).

## Rollout / phasing

- **Phase 1 (now, safe — not arena-sensitive):** `width ascii` (1), `width mixed` (2) [same-buffer],
  the whole `pipeline` Integer theme (`pipeline int` 4, `pipeline groupagg` 5), and the whole
  `convert` theme (7, 8). These measure real gaps immediately and give plan-86 more signal (Theme 2
  is a direct regression row for sub-plan A's Integer twins; `width mixed` for the grapheme path).
- **Phase 2 (with the residual grapheme-churn arena fix / plan-86 sub-plan A):** the arena-gated rows
  — `width churn` (3) and `pipeline str` (6) — authored tiny in Phase 1 with their `TODO` markers,
  bumped to realistic N in the commit that lands the fix, doubling as its acceptance gate (must jump
  from tiny to realistic and stay linear).
- Each new row lands in all three languages simultaneously with a matching checksum, updates
  `benchmark/README.md`'s coverage table, and keeps the git-ignored logs regenerable via
  `benchmark/run.sh --run 10`.

## Non-goals

- No network/tls/http/audio/app/term benchmarks (non-deterministic or external-dependency).
- **No new per-member collection rows** — list/map/set are already exhaustively matrix-covered; this
  plan is pattern-throughput + the one new-feature group only.
- No new language surface — every benchmark uses existing, documented members (`strings::displayWidth`,
  `collections::filter`/`transform`/`reduce`/`groupBy`/`mapValues`, `toString`/`toInt`/`toFloat` — all
  verified against `mfb man`).
- Not a replacement for the per-member or pattern-throughput rows — this is *additive* (chained-pattern
  throughput + a zero-coverage feature); the existing suite stays as the surface + churn check.
