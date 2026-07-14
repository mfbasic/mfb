# plan-40: Benchmark coverage — critical-feature hot paths to add

Last updated: 2026-07-12
Effort: medium (each benchmark is a self-contained `test_*` in all three languages)
Companion to `planning/plan-39-benchmark-perf.md` (the fix plan).

The current suite (`benchmark/{mfb,c,python}`) is an **API-surface coverage
check**: it calls *every* `collections::`, `math::`, `vector::`, `bits::`,
`strings::` member once over each element type. That proves the surface works and
catches single-op regressions, but it does **not** exercise the *patterns real
programs hit* — sustained churn, mixed pipelines, compile-once/run-many. This plan
adds throughput benchmarks for those hot paths, tracked as **critical MFB
features**, without duplicating the existing per-member rows.

## Design rules (match the existing suite)

- One self-contained workload per language (`benchmark/mfb/src/*.mfb`,
  `benchmark/c/*.c`, `benchmark/python/*.py`), timed internally with
  `datetime::monotonicNanos()` (mfb) / matching clock, printing a checksum on
  stderr so all three agree. Register each in `main.*`'s driver + the group table.
- **New group per theme** (or extend an existing group file). Keep the C/Python
  mirrors doing the *same materialized work* so the cross-language comparison is
  fair (README's parity contract). Where mfb has no cross-language peer (Money,
  Fixed pipelines) mark the row mfb-only, like `math fixed`.
- **Arena-sensitive rows must wait on plan-39-A.** The README already documents
  that `string unicode` and `liststr reshape` use *deliberately tiny* counts
  because the arena free-list degrades quadratically under mixed-size transient
  churn. Any new churn/arena benchmark authored at realistic size will hang the
  suite until plan-39-A lands. Author them now at tiny "smoke" counts with a
  `TODO(plan-39-A): raise N once arena fixed` marker, then bump N in the same
  commit that closes A. This makes the arena-stress rows a **regression gate** for
  A: they should jump from tiny to realistic and stay linear.

## What's already covered (do NOT duplicate)

- Every `collections::` list op once over Integer + String lists; map `set/lookup`
  + `int_ops`/`str_ops` aggregates.
- Each transcendental (`sin/cos/tan/atan2/asin/acos/atan/exp/log/log10/pow/sqrt`)
  over an array; `math float/int/fixed/simd` families; `vector math/float/int/fixed`.
- `strings::` concat/case/search/slice + a tiny unicode smoke row.
- `io write/read`; `parse csv/json/regex` (one shape each); `bits ops`; `bignum
  modmul/modexp`; `recurse fib/ackermann`; `primes`; `thread sum`; `record update`.

## Proposed new benchmarks

Grouped by the six requested hot-path themes. Each row: **why it's a distinct hot
path**, the **workload**, and the **API** it exercises (all real members — verified
against the current suite's usage).

### Theme 1 — map / collection churn (new group `mapchurn`, `listchurn`)

The suite only touches each op once on a pre-built collection. Real code *grows,
mutates, and rehashes* collections in a loop.

1. **map grow+rehash** — insert N distinct keys one-by-one into an initially-empty
   map (forces repeated rehash/grow), then look each up. Contrast with the current
   `map int_ops` which pre-sizes. API: `collections::set`/`get`/`hasKey` in a loop.
2. **map insert/delete churn** — steady-state add/`removeKey` cycling so the map
   stays ~fixed size while churning buckets (tombstone/rehash stress). API:
   `set`/`removeKey`/`hasKey`.
3. **map iterate** — `keys`/`values`/`mapValues`/`merge` over a large map in a loop
   (materialization cost). API: `keys`/`values`/`mapValues`/`merge`.
4. **list build-by-append** — build a large list via repeated `append` in a loop
   (amortized growth path) and a `prepend` variant (O(n) shift) — distinct from the
   current single `append`/`prepend` micro-rows which use tiny N.
5. **nested collections** — `List OF List OF Integer` build + `flatten` + `Map OF
   List` group/append (the value-semantics copy path at depth). API:
   `append`/`flatten`/`groupBy`.

### Theme 2 — float & transcendental pipelines (extend `float`, new `mathpipe`)

The suite isolates each kernel; real numeric code chains them.

6. **matmul** — dense N×N `Float` matrix multiply (FMA-heavy, cache/reg pressure).
   Cross-language. Distinct from `nbody` (fixed 5-body) — tests scaling.
7. **fft / dft** — a small radix-2 FFT or naive DFT (sin/cos + complex arithmetic
   in a tight loop). Exercises transcendentals *interleaved* with float ops, not in
   isolation.
8. **stats reduction** — mean/variance/stddev over a large `Float` array
   (`math::sqrt` + accumulation) — the reduction pattern the per-kernel rows miss.
9. **fixed-point finance (mfb-only)** — a `Money`/`Fixed` running-balance /
   interest calc (mfb-only, like `math fixed`); flagship exact-decimal feature with
   no C/Python peer. Marks Money as a tracked feature.

### Theme 3 — string / unicode (extend `string`, new `strbuild`)

10. **string builder** — accumulate a large string via repeated `&` concat and via
    `strings::join` of a list; contrast the two (the classic O(n²)-vs-O(n) trap).
    Distinct from the current one-shot `concat` row.
11. **split/join round-trip** — `strings::split` a large CSV-ish line then
    `strings::join` back, in a loop (tokenizer hot path). API: `split`/`join`.
12. **replace/trim pipeline** — `replace`/`trimChars`/`stripPrefix`/`padLeft` chain
    over many short strings (text-cleaning hot path).
13. **unicode at realistic size** — `graphemes`/`graphemeAt`/`normalizeNfc`/
    `caseFold` over a multi-KB mixed-script string. **Arena-gated (plan-39-A):**
    author tiny now, raise N when A lands (this is the regression gate for the
    unicode-churn half of E).

### Theme 4 — io buffering (extend `io`)

14. **line read loop** — write a many-line file, then read it back line-by-line
    (`fs::readLine`/equivalent) — the buffered-read hot path the current bulk
    `read` misses.
15. **buffered vs unbuffered write** — the same N writes with `io::setBuffered`
    on vs off, as two rows, to *quantify* the buffering win (and guard plan-39-F).
16. **stdout formatting throughput** — a loop of `toString`+`print` of mixed
    Int/Float/String (the console-output hot path; today only `io write` to a file).
17. **binary round-trip** — `strings::toBytes` + `fs` write/read of a byte buffer
    (binary io path; arena-sensitive via `toBytes`, gate N on plan-39-A).

### Theme 5 — regex (extend `parse`, new `regexbench`)

The current single `regex` row's cross-language parity is **suspect** (mfb 15.7 ms
vs python 0.016 ms — plan-39-G must verify the workloads match). These add
controlled, parity-checked shapes:

18. **compile-once, match-many** — compile one pattern, run it over N lines
    (separates compile cost from match cost; the realistic usage). Ensure C/Python
    also compile once.
19. **capture groups** — extract 2–3 groups per match over many lines (the
    tokenizing/parsing hot path).
20. **alternation / find-all** — `|`-heavy pattern, find-all matches in a large
    text (backtracking stress). Match counts checksum across languages.
21. **regex replace** — pattern-driven substitution over a large string.

### Theme 6 — arena stress (new group `arena`) — regression gate for plan-39-A

Explicit, isolated measurement of the mixed-size transient-churn path the README
calls out. These exist **to catch the quadratic regression** and prove A fixed it.

22. **transient mixed churn** — a loop that allocates and immediately drops many
    *mixed-size* short-lived `List`/`String` temporaries (e.g. build a small list,
    slice a string, discard). Author tiny; raising N to realistic and staying
    linear is A's acceptance criterion.
23. **long-lived + short-lived mix** — hold a large long-lived structure while
    churning short-lived temporaries around it (fragmentation stress).
24. **grow-shrink** — repeatedly grow a collection then `take`/`drop` it back down
    (free-list coalescing stress). API: `append`/`take`/`drop`.

## Rollout / phasing

- **Phase 1 (now, safe):** themes 1–5 rows that are *not* arena-sensitive (map
  grow/rehash, matmul, fft, stats, string builder, split/join, line-read, buffered
  vs unbuffered, stdout, regex compile-once/capture/alternation/replace). These
  measure real gaps immediately and give plan-39 more signal.
- **Phase 2 (with plan-39-A):** all arena-gated rows (unicode realistic, binary
  round-trip, arena-stress group, transient churn) — authored tiny in Phase 1 with
  `TODO(plan-39-A)`, bumped to realistic N in the commit that lands A, doubling as
  its acceptance gate.
- Each new row lands in all three languages simultaneously with a matching
  checksum, updates `benchmark/README.md`'s coverage table, and keeps the
  git-ignored logs regenerable via `benchmark/run.sh`.

## Non-goals

- No network/tls/http benchmarks (non-deterministic, external-dependency; not a
  fair micro-bench).
- No new language surface — every benchmark uses existing, documented members.
- Not a replacement for the per-member coverage rows — this is *additive* (pattern
  throughput), the existing suite stays as the surface check.
