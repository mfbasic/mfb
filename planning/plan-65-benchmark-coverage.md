# plan-65: Benchmark coverage — critical-feature hot paths to add

Last updated: 2026-07-25
Effort: medium (each benchmark is a self-contained `test_*` in all three languages)
Companion to `planning/plan-64-benchmark-perf.md` (the fix plan).

The current suite (`benchmark/{mfb,c,python}`) is an **API-surface coverage check** plus
the plan-40 pattern-throughput groups and the plan-45 additions (encoding/datetime/
dispatch + map/mathpipe/list extensions). Between them it exercises every `collections::`/
`math::`/`vector::`/`bits::`/`strings::`/`encoding::`/`datetime::` member, union `MATCH` +
inline `TRAP`, sustained churn, and the `Scalar` primitive. But one **flagship package has
zero coverage**, one **whole direction of two covered packages is unbenchmarked**, and one
**new type is on the way**:

1. **`crypto::`** (hashes, HMAC, KDFs, AEAD, Ed25519) — a flagship built-in package with
   **no benchmark at all**, yet hashing/KDF/AEAD is a canonical throughput hot path with
   deterministic, byte-identical output and direct C/Python peers (`hashlib`, `hmac`).
2. **Serialization (encode direction)** — the suite benches `parse csv`/`parse json` but
   never `csv::stringify`/`json::stringify`; serialization is a distinct hot path (tree
   walk + escape + integer→String) from parsing.
3. **`Set OF T`** (plan-63) — a new built-in collection type. **Front-end only today** (no
   literal, no operations, no codegen — plan-63-A), so it is **not yet benchmarkable**;
   tracked here as a deferred group to add when plan-63-B/C land operations.

This plan adds throughput benchmarks for those hot paths, tracked as critical MFB features,
without duplicating the existing per-member or pattern-throughput rows.

## Design rules (match the existing suite)

- One self-contained workload per language (`benchmark/mfb/src/*.mfb`, `benchmark/c/*.c`,
  `benchmark/python/*.py`), timed internally with `datetime::monotonicNanos()` (mfb) /
  matching clock, printing a checksum on stderr so all three agree. Register each in
  `main.*`'s driver + the group table + the README coverage table.
- **New group per theme** (or extend an existing group file). Keep the C/Python mirrors
  doing the *same materialized work* (README parity contract). Where mfb has no
  cross-language peer, mark the row mfb-only (like `math fixed`), printed as `--`.
- **Arena-sensitive rows must wait on plan-64-A** (the arena mixed-transient-churn
  sub-plan — successor to plan-44-J). Any new benchmark that allocates a fresh
  `List OF Byte`/`String`/`Map` per call and runs at realistic N will hang the suite until
  A lands. Author them now at tiny "smoke" counts with a `TODO(plan-64-A): raise N once
  arena fixed` marker, then bump N in the commit that closes A — making these rows a
  regression gate for it.

## What's already covered (do NOT duplicate)

Read from the current suite (`benchmark/mfb/src/*.mfb` + README):

- **list / liststr:** every `collections::` list op over Integer **and** String lists;
  `sort_asc`/`sort_desc`/`sort_rand` adaptivity.
- **listchurn / mapchurn:** build-by-append, prepend front-shift, nested build+flatten+
  groupBy; map grow/rehash, steady-state churn, keys/values/mapValues/merge iterate.
- **map:** set/lookup, int_ops/str_ops, intkey/intchurn/listagg (Integer-key path).
- **math / mathpipe / float:** every transcendental + int/fixed/simd; leibniz/nbody/
  mandelbrot/matmul; dft/stats/memo/finance/money.
- **vector:** math/float/fixed/int families. **bits:** every op. **bignum:** modmul/modexp.
- **strings / strbuild:** concat/case/search/slice + unicode smoke; join/splitjoin/clean.
- **encoding:** base64/hex/percent round-trips. **datetime:** civil arithmetic + iso
  round-trip. **dispatch:** union+MATCH, inline TRAP.
- **parse:** csv/json/regex (**parse only — no stringify**). **regexbench:** compile/
  capture/alternation/replace. **io:** write/read/readnum/buf_on/buf_off/format/binary.
- **scalarbench:** roundtrip/classify/transform/listchurn. **recurse:** fib/ackermann.
  **primes. thread** sum. **record** update. **arena:** transient/mixed/growshrink.

Deliberately-tiny arena-gated rows (README:122-150): `string unicode`, `liststr reshape`,
`string unibig`, `io binary`, whole `arena` group, `scalarbench roundtrip/transform/
listchurn`, and the plan-45 `TODO(plan-44-J)` rows `encoding base64`, `datetime iso`, `map
intchurn` (carry forward to `TODO(plan-64-A)`). **Do not re-benchmark any of the above.**

## Proposed new benchmarks

Grouped by hot-path theme. Each row: **why it's a distinct hot path**, the **workload**, and
the **real API members** it exercises (all verified against `mfb man`: crypto sha256/sha512/
hmacSha256/pbkdf2Sha256/constantTimeEqual/ed25519*, json stringify, csv stringify).

### Theme 1 — `crypto::` hashing / KDF / AEAD (new group `crypto`) — tracks the crypto package (zero coverage)

The crypto package has **no** benchmark today, yet it is a portable software core over the
`bits` package — a pure integer/bit-shuffling throughput hot path — with deterministic,
byte-identical output and exact peers (`hashlib`, `hmac`, `hashlib.pbkdf2_hmac`). Every row
prints the digest/tag as a hex-fold checksum that **matches bit-for-bit across all three
languages** (standard FIPS/RFC algorithms), so parity is exact, not approximate.

1. **sha256 bulk hash** — hash a fixed multi-KB `List OF Byte` buffer in a loop.
   Distinct: the 64-round SHA-2 compression over `bits` rotate/shift/xor — the package's
   core inner loop. Cross-language (Python `hashlib.sha256`, C vendored `sha256.c` — same
   pattern the parse group vendors parson/libcsv). API: `crypto::sha256` (+
   `encoding::hexEncode` for the checksum). **Realistic N** (single hash of a fixed buffer
   per iter is CPU-bound, not arena-churn) — Phase 1.
2. **sha512 bulk hash** — same buffer through the 80-round 64-bit variant (a distinct
   word-size inner loop from sha256). Cross-language (Python `hashlib.sha512`). API:
   `crypto::sha512`.
3. **hmac-sha256** — keyed MAC over the buffer (two hash passes + key schedule — the
   message-authentication hot path). Cross-language (Python `hmac.new(...,sha256)`). API:
   `crypto::hmacSha256`.
4. **pbkdf2-sha256 (work factor)** — derive a 32-byte key with a **fixed salt** and a
   fixed, moderate iteration count (e.g. 4096). Distinct: the deliberately-expensive
   iterated-HMAC KDF — quantifies the per-iteration HMAC cost, deterministic with a fixed
   salt. Cross-language (Python `hashlib.pbkdf2_hmac('sha256',...)`). API:
   `crypto::pbkdf2Sha256`. Realistic (fixed iteration count, CPU-bound) — Phase 1.
5. **constantTimeEqual** — compare two equal-length byte lists many times (the constant-
   time-compare primitive; a tight `bits` loop). Cross-language (Python
   `hmac.compare_digest`). API: `crypto::constantTimeEqual`.
6. **hash churn (arena-gated)** — hash many *fresh* small `List OF Byte` messages built per
   call (the realistic "hash a stream of records" pattern — per-call byte-list allocation).
   **Arena-gated (plan-64-A):** author tiny with `TODO(plan-64-A)`; raise N when A lands.
   API: `crypto::sha256` + `strings::toBytes`.
7. **ed25519 sign+verify (optional, tracked)** — sign a fixed message with a **hardcoded
   test-vector private key** (deterministic RFC 8032 signing) then verify. Distinct: the
   only asymmetric primitive that is fully deterministic. Cross-language where a peer
   exists (Python `cryptography` Ed25519) else **mfb+python only**, C marked `--`. API:
   `crypto::ed25519Sign`/`ed25519Verify`. (Key *generation*, ECDSA, `randomBytes`, `uuid4`
   are non-deterministic — excluded, see Non-goals.)

### Theme 2 — serialization / encode direction (extend `parse`, or new group `serialize`)

The suite parses CSV and JSON but never serializes them back. `stringify` is a distinct hot
path: a recursive tree walk with string-escape and integer/float→String rendering, exactly
the shape `json::stringify`/`csv::stringify` document.

8. **json stringify** — build a Json value tree once (objects/arrays/numbers/strings), then
   `json::stringify` it many times (compact serialize: escape + number formatting + map
   iteration-order emission). Cross-language (Python `json.dumps(...,separators=(',',':'))`,
   C parson `json_serialize_to_string`). API: `json::stringify` (+ `json::parse` to build the
   tree once). Checksum = length of the emitted text (matches across languages for compact
   output). **Arena-gated (plan-64-A):** per-call String building — author tiny with
   `TODO(plan-64-A)`.
9. **json parse→stringify round-trip** — the realistic load/modify/save cycle end-to-end
   (distinct from either half alone; stresses the parse+serialize seam). Cross-language.
   API: `json::parse` + `json::stringify`. Arena-gated (plan-64-A).
10. **csv stringify** — render a `List OF List OF String` grid to RFC-4180 text
    (`csv::stringify`), the quote-when-needed field-join hot path complementary to `parse
    csv`. Cross-language (Python `csv.writer`, C libcsv or hand-rolled). API:
    `csv::stringify`. Arena-gated (plan-64-A) for the per-call String churn.

### Theme 3 — `Set OF T` (deferred group `set`) — tracks plan-63 (not yet operational)

`Set OF T` (plan-63) is a new built-in collection type, but plan-63-A ships **only the
front-end type shape** — no literal, no operations, no codegen. **There is nothing to
benchmark yet.** When plan-63-B/C land the literal + operations (add/contains/remove/union/
intersect/size), add a `set` group mirroring the `map` intkey/intchurn rows: build/`contains`
sweep over Integer and String elements, steady-state add/remove churn, and set-algebra
(union/intersect/difference) over two sets — cross-language (Python `set`, C hash-set), all
checksum-matched. **Do not author these rows until plan-63 operations exist** (verify
member names against the shipped surface first). This theme is a placeholder so the Set
throughput gate is not forgotten.

## Rollout / phasing

- **Phase 1 (now, safe — not arena-sensitive):** crypto sha256 (1) and constantTimeEqual (5).
  These stay in the arena quick bins and their per-call cost is flat across the run loop, so
  they run at realistic N (reps=64 / reps=8192) and measure real gaps immediately (mfb's
  software core vs `hashlib`'s C backend). **CORRECTED — see Corrections C1:** the plan
  originally also listed sha512 (2), hmac (3), pbkdf2 (4), and ed25519 (7) as Phase 1
  "CPU-bound, no per-call churn"; measurement showed they *are* arena-sensitive and they moved
  to Phase 2.
- **Phase 2 (with plan-64-A):** the arena-gated rows — crypto sha512 (2), hmac (3), pbkdf2 (4),
  hash churn (6), ed25519 (7), json stringify (8) + round-trip (9), csv stringify (10) —
  authored tiny with `TODO(plan-64-A)`, bumped to realistic N in the commit that lands A,
  doubling as its acceptance gate (must jump from tiny to realistic and stay linear).
- **Deferred (with plan-63-B/C):** the `set` group (Theme 3) — added when Set operations ship.
  Verified deferred: `plan-63-B`/`plan-63-C` are still in `planning/` (not landed), so Set
  has no literal/operations to benchmark yet.
- Each new row lands in all three languages simultaneously with a matching checksum, updates
  `benchmark/README.md`'s coverage table, and keeps the git-ignored logs regenerable via
  `benchmark/run.sh --run 50`.

## Status (executed 2026-07-25)

All non-deferred rows landed. Themes 1 and 2 (10 benchmarks) are implemented in all three
languages, wired into every driver (`main.mfb` / `main.c` / `main.py`), the C build list
(`run.sh`), and the README coverage table. Theme 3 (Set) remains deferred (prerequisite not
met — see Rollout).

- `[x]` **Theme 1 — crypto** (`benchmark/{mfb/src/crypto.mfb,c/cryptobench.{c,h},python/cryptobench.py}`):
  sha256, sha512, hmac, pbkdf2, cte, churn, ed25519. C hand-rolls the FIPS/RFC cores; ed25519
  is mfb+python (C `--`).
- `[x]` **Theme 2 — serialize** (`benchmark/{mfb/src/serialize.mfb,c/serializebench.{c,h},python/serializebench.py}`):
  json, roundtrip, csv. JSON via vendored parson (C); csv hand-rolled to mfb's rules.
- `[ ]` **Theme 3 — set:** deferred until plan-63-B/C land Set operations. Not authored.

**Verification:** `./benchmark/run.sh` builds all four targets and runs clean (exit 0) at
`--run 1` and `--run 10`. Every new row's checksum is byte-identical across mfb / c-O0 / c-O2 /
python (crypto rows 4×; ed25519 2× = mfb+python, C prints `--`; serialize rows 4×): sha256
320768, sha512 17144, hmac 30216, pbkdf2 4581, cte 8192, churn 67103, ed25519 8105 (= 8104 sig
byte-sum + 1 verify), serialize json/roundtrip 532, csv 236. The mfb `crypto::` software core is
thus proven byte-identical to `hashlib`/`hmac`/pyca and the C reference.

## Corrections

- **C1 — Phase-1 misclassification of arena-sensitive crypto rows.** The plan's Rollout claimed
  crypto sha256–pbkdf2 (+ ed25519) were Phase 1, "CPU-bound, no per-call churn." That is false
  for mfb's implementation: the `crypto::` package is a software core over `bits` that allocates
  transient `List OF Byte` values per operation. Measured (native, `datetime::monotonicNanos`):
  in a clean process the crypto group runs fast (sha512 reps=64 = 26 ms, pbkdf2 iters=4096 =
  224 ms), but run late in the suite — after ~100 arena-churning groups — the same code hits the
  plan-64-A quadratic free-list path and explodes: at `--run 1`, sha512 reps=64 = 1634 ms and
  pbkdf2 iters=4096 = 8069 ms; at `--run 10`, hmac reps=64 climbs 179 ms → 1300 ms. Only sha256
  (flat 9 ms, min≈max across the run loop) and cte (flat 1.6 ms) are genuinely quick-bin. Per
  the plan's own design rule ("arena-sensitive rows must wait on plan-64-A … author them now at
  tiny smoke counts with a `TODO(plan-64-A)` marker"), sha512 (reps 64→2), hmac (reps 64→8),
  pbkdf2 (iters 4096→64), and ed25519 (reps 2→1) were reclassified to Phase 2, authored tiny
  with `TODO(plan-64-A)`, becoming part of the plan-64-A regression gate. Phase 1 now holds only
  sha256 and cte. This is a Rollout defect, not an implementation defect.
- **C2 — ed25519 verify needs a precomputed public key.** The plan's row 7 says "sign+verify with
  a hardcoded test-vector private key," but `crypto::ed25519Sign` takes only the 32-byte seed and
  there is no derive-public-from-seed builtin (only the random `generateEd25519`). The matching
  public key was computed once via pyca for the fixed seed and hardcoded so `ed25519Verify` runs
  deterministically. Signature byte-sum (8104) verified identical between mfb and pyca.
- **C3 — csv Python peer is hand-rolled, not `csv.writer`.** The plan suggested `csv.writer` for
  the Python csv peer, but `csv.writer` appends a line terminator after the final row, which
  breaks length-checksum parity with mfb's no-trailing-newline `csv::stringify`. The Python peer
  hand-rolls the RFC-4180 join to mfb's exact rules instead (the plan permits "hand-rolled").

## Non-goals

- No network/tls/http benchmarks (non-deterministic, external-dependency).
- **No non-deterministic crypto:** `randomBytes`, `randomInt`, `uuid4`, key *generation*
  (`generateEd25519`/`generateP256/384/521`), and ECDSA `p*Sign` (non-deterministic nonce)
  produce non-reproducible output — excluded. Only deterministic primitives (hashes, HMAC,
  KDFs with fixed salt, AEAD with fixed key+nonce, Ed25519 with a fixed key) are benched.
- No new language surface — every benchmark uses existing, documented members (crypto/json/
  csv verified against `mfb man`; Set deferred until its operations ship).
- Not a replacement for the per-member or pattern-throughput rows — this is *additive*; the
  existing suite stays as the surface + churn check.
