# bug-775: `big::` arithmetic runs 6–57× behind Python's `int`: byte limbs everywhere, one decimal digit per pass, and wasted modPow squarings

Last updated: 2026-09-28
Effort: x-large (1d–3d)
Severity: MEDIUM
Class: Other (Performance)

Status: Open
Regression Test: planned `tests/runtime/rt_big_int_cost.rs` (new), modeled on
`tests/runtime/rt_compress_bounds.rs::decode_time_is_linear_in_output_size` (in-process,
interleaved medians, ratio assertions). Correctness stays pinned by the existing
`tests/runtime/rt_big_int.rs` oracles.

The `bigint` benchmark group (`benchmark/mfb/src/bigint.mfb`) has no C peer, because libc
has no bignum. So `./benchmark/rank.py` does not grade it: `rank.py --csv | grep bigint`
prints nothing. Against its only peer, CPython's native `int`, it is 6–57× slower:

| row (min ms, `benchmark/baseline/*.log`) | mfb -O1 | mfb -O2 | mfb -O3 | Python | × Python |
|---|---|---|---|---|---|
| `bigint pidigits` (400 digits of pi, spigot) | 33.338 | 33.305 | 33.337 | 5.424 | 6.1× |
| `bigint factorial` (`toString(factorial(2000))`) | 30.679 | 30.349 | 30.215 | 0.537 | 57× |
| `bigint modpow` (16 × `modPow(b, 65537, 2^2048 − 1557)`) | 23.098 | 23.094 | 23.046 | 1.001 | 23× |
| `bigint gcd` (200 × parse + gcd + toString, 100–300 digits) | 3.980 | 4.015 | 4.004 | 0.306 | 13× |

`-O2`/`-O3` change nothing. Every `big` member is a hand-emitted native body
(`Body::abi_function`), not an MFB loop the optimizer rewrites.

**The single correct behavior a fix produces:** each of the four rows runs within **5×
of the Python peer at `-O1`**: pidigits ≤ 27.1 ms, factorial ≤ 2.69 ms, modpow ≤ 5.0 ms,
gcd ≤ 1.53 ms. Five times is the B band of `benchmark/RANKING.md` ("visibly slower, but
no structural defect implied"), and Python is the only real peer. Every result, rendered
digit, error and error code is unchanged, and so is `big::Int`'s public representation
(a little-endian `List OF Byte` magnitude in canonical form).

References:

- `mfb spec stdlib big` (`src/docs/spec/stdlib/19_big.md`) and `mfb man big types`: the
  `big::Int` record is **public**. Its fields are `magnitude AS List OF Byte`
  (least-significant byte first, no trailing zero bytes, empty for zero) and
  `negative AS Boolean`. That fixes the storage format (see Non-goals).
- `src/codegen/builtins/big/gen_big.rs` module doc (plan-127-A §4.3): the three
  emitters, and "Everything is kept in frame slots between steps".
- `planning/completed/plan-127-{A,B,C,D}-*.md`: the design. Byte limbs were chosen so
  that "no widening multiply is needed" (`emit_mul_magnitude` doc).
- Found by the benchmark D/F-grade sweep (`benchmark/RANKING.md`), which asked for
  root causes on these rows.

## Failing Reproduction

macos-aarch64, `target/release/mfb` at `7e53c82b9`, `mfb build -O 1 -q .`, best of 5
in-process runs. Python 3 is the system `python3` running the mirror statements
(`/tmp/dbF-big.py`).

### A. per-operation split (`/tmp/dbF-big`)

`src/main.mfb` (standard executable manifest). Each line below comes from a best-of-5
loop of this shape:

```
IMPORT io
IMPORT big
IMPORT datetime

FUNC main() AS Nothing
  MUT best AS Integer = 1000000000000
  MUT f AS big::Int = big::fromInteger(1)
  FOR r = 1 TO 5
    LET t0 AS Integer = datetime::monotonicNanos()
    f = big::factorial(2000)
    LET t1 AS Integer = datetime::monotonicNanos()
    IF t1 - t0 < best THEN
      best = t1 - t0
    END IF
  NEXT
  io::print("factorial(2000) product only: " & toString(best / 1000) & " us")
  ' ... the same harness around:
  '   big::toString(f)                     (2000!, 5736 digits)
  '   big::toString(big::factorial(1000))  (2568 digits)
  '   100 x big::multiply(m, m)     m = 2^2048 - 1557
  '   100 x big::multiply(m2, m2)   m2 = m*m
  '   100 x big::remainder(m2 - 12345, m)
  '   big::modPow(big::fromInteger(3), big::fromInteger(65537), m)
  '   10000 x big::multiply(a, a)   a = 123456789
END FUNC
```

| operation | mfb -O1 | Python | × |
|---|---|---|---|
| `factorial(2000)`, product only | 9,071 µs | 127 µs (`math.factorial`) | 71× |
| `toString(2000!)`, 5,736 digits | 23,757 µs | 422 µs (`str`) | 56× |
| `toString(1000!)`, 2,568 digits | 4,718 µs | 111 µs | 43× |
| 100 × multiply 2048 × 2048 bit | 5,500 µs | 281 µs | 20× |
| 100 × multiply 4096 × 4096 bit | 11,491 µs | 1,192 µs | 9.6× |
| 100 × remainder 4096-bit mod 2048-bit | 6,213 µs | 1,176 µs | 5.3× |
| 1 × `modPow(3, 65537, m)` | 1,558 µs | 71 µs (`pow`) | 22× |
| 10,000 × multiply of 27-bit values | 346 µs | — | 35 ns/call |

### B. the gcd row split (`/tmp/dbF-big2`, the row's exact data)

```
gcd row: 400 parses: 1584 us               (Python int(): 128 us, 12x)
gcd row: 200 gcds: 2459 us                 (Python math.gcd: 175 us, 14x)
gcd row: 200 gcds + toString: 2466 us      (toString of the small gcds is negligible)
```

The 200 pairs take **2,147 Euclid steps in total**, 10.7 per pair (counted in Python with
the same `(x, y) = (y, x mod y)` recurrence). The gcd time is therefore about 1.15 µs per
`divMod`, not a step-count problem.

### C. remainder size split (`/tmp/dbF-mp`, 10,000 reps, three rounds)

```
10000x remainder 68-bit/64-bit:   1036-2571 us   (Python 955-1144 us: parity)
10000x remainder 1123-bit/697-bit: 61682-68359 us (Python 11362-11831 us: 5.5x)
```

Per-call overhead is at parity with Python. The gap grows with operand size.

### D. modPow squares past the exponent's top bit (`/tmp/dbF-mp`)

```
FUNC timeIt(e AS Integer, m AS big::Int) AS Integer   ' best of 5 of one modPow(3, e, m)
...
e=65537 (17 bits, 3 bytes): 2065 us
e=8388609 (24 bits, 3 bytes): 2087 us
e=257 (9 bits, 2 bytes): 836 us
e=65535 (16 bits, 2 bytes): 1630 us
```

- Observed: `e = 65537` (17 significant bits) costs the same as `e = 2^23 + 1`
  (24 bits).
- Expected: the cost follows the exponent's bit length, not its byte count.

Correctness holds. The four `test_bigint_*` checksums in `benchmark/baseline/mfb-O1.sums`
and `python.sums` are identical (888227773, 29118, 37451214999, 2652206916), and
`rt_big_int.rs` covers the members against independent oracles.

## Root Cause

Every member is emitted by `src/codegen/builtins/big/gen_big.rs` over **8-bit limbs**:
one byte of the public magnitude per inner-loop step. That single choice multiplies
every O(n) pass by 8 and every O(n²) kernel by up to 64, compared with 64-bit words.
Compared with CPython's 30-bit digits, the factors are 3.75 and 14. Four mechanisms sit
on top of it.

1. **Multiply** (`gen_big.rs:emit_mul_magnitude`): schoolbook, one `u8 × u8` step per
   inner iteration (`load_u8 b[j]; mul; add; load_u8 dst; add; store_u8; lsr 8`). A
   2048 × 2048-bit product is 256 × 256 = 65,536 steps. At 55 µs that is 0.84 ns per
   step, so the per-step code is fine and the step count is the problem: 64-bit limbs
   need 32 × 32 = 1,024 `mul`/`umulh` steps. The 4096-bit multiply takes only 2.1× the
   2048-bit one because its operand `m²` has many zero low bytes, and the row loop
   skips a zero multiplier byte (`branch_eq(&row_next)`).
2. **Decimal rendering** (`gen_big.rs:emit_int_to_string`, shared by
   `big::toRadixString`): "One pass per digit: divide the working copy by the radix,
   high byte down". Each step is a `udiv` + `msub` on a remainder that carries from
   step to step. 2000! has 5,736 digits over a working copy that shrinks from 2,382
   bytes, which is about 6.8 M dependent `udiv` steps. At 23.8 ms that is ≈ 3.5 ns per
   step, the latency of that dependency chain. The scaling is confirmed quadratic:
   2.23× the digits cost 5.03× the time (4,718 → 23,757 µs; (5736/2568)² = 4.99).
   CPython is quadratic too but extracts 9 digits per pass over 30-bit digits, about
   34× fewer steps (derived: (1/9) × (8/30)). The 2000! rendering is 77% of the
   `factorial` row.
3. **Factorial product** (`func_factorial.rs:lower_factorial`):
   `acc = acc × k` for `k = 2..n`. Each step allocates a new zero-filled result sized
   `count(acc) + count(k)` and a new `big::Int` for `k` (`emit_int_from_integer`), and
   runs the byte-limb schoolbook over roughly 1–2,382 × 1–2 bytes. About 4.8 M byte
   steps in total (derived) plus 4,000 allocations: 9.07 ms. Python uses 30-bit digits
   and a binary-split product (127 µs).
4. **modPow** (`func_mod_pow.rs:lower_mod_pow`) runs its square-and-multiply loop over
   `bits = exponent.count << 3`, every bit of every *byte*, and squares the base on
   each iteration. So `e = 65537` (3 bytes) runs 24 iterations: 7 squarings past the
   top set bit (bit 16), plus the squaring after it, are computed, reduced and thrown
   away. Once the base is full size (after about 11 squarings of 3, since 3^2048 >
   2^2048), those are 2048-bit multiply + `divMod` pairs (≈ 55 + 62 µs each, table A).
   The probe confirms it: 65537 costs the same as 2^23+1. Each reduction also builds
   and releases a quotient it never uses (`emit_div_mod_int` then
   `emit_release_int(quotient_slot)`).
5. **Division / Euclid / parse** (`gen_big.rs:emit_div_mod_magnitude`, Knuth
   Algorithm D over base 256; `func_gcd.rs:lower_gcd`; `func_parse.rs`): byte-limb
   inner loops. 1123/697-bit remainder is 5.5× Python while small operands are at
   parity (C). `parse` does `magnitude = magnitude × radix + digit` over the whole
   magnitude **once per digit**, the mirror image of item 2 (quadratic, one digit per
   pass).

Ranked contribution per row (percentages derived from tables A–D; everything else is a
guess):

- **factorial**: item 2 (77%) > item 3 (≈ 30%). The two overlap because the row is
  ≈ 30.7 ms against 9.07 + 23.8 ms measured separately.
- **modpow**: item 4 (roughly half the full-size squarings are wasted; guess ≈ 45%),
  then items 1 and 5 (byte limbs in multiply and Algorithm D).
- **gcd**: parse (item 5, 1.58 ms, 40%) and divMod (item 5, 2.46 ms, 60%).
- **pidigits**: operands stay small to medium and the operations are
  linear-to-quadratic over byte limbs (item 1/5). Hypothesis, not split by measurement.
  Confirm by timing `bigintPiDigits(400)` before and after the limb change, since the
  row has no other lever.

Contrast: per-call overhead is not the problem. 10,000 multiplies of 27-bit values take
35 ns per call, and 64-bit remainders are at Python parity (table C).

## Goal

- Rows at `-O1` (re-run `benchmark`): pidigits ≤ 27.1 ms, factorial ≤ 2.69 ms, modpow ≤
  5.0 ms, gcd ≤ 1.53 ms (each ≤ 5× the Python min in `benchmark/baseline/python.log`).
- Probe D: `modPow(3, 65537, m)` ≤ 0.8 × `modPow(3, 2^23+1, m)` (it is 0.99 today).
- `tests/runtime/rt_big_int.rs` passes unchanged. Every benchmark checksum is unchanged.

### Non-goals (must NOT change)

**Owner constraint:** no change to the language specification or the memory model. The
following are untouched:

- value semantics (a `big::Int` assigned or passed is a copy; every member returns a
  fresh value);
- `big::` results, errors and error codes;
- syntax, keywords, builtins and signatures (no new `big::` member, parameter or
  overload);
- `mfb man` / `mfb spec language` behavior;
- `mfb spec memory` (the arena and drop contract: a `big::Int` is a copyable value
  record released at scope exit at `16 + 40 + dataCapacity` bytes, per
  `gen_big.rs`'s module doc).

Only the emitted algorithms inside the member bodies may change.

- **The public representation is fixed.** `big::Int.magnitude` is a documented
  `List OF Byte`, little-endian, canonical (no trailing zero bytes, empty for zero,
  `negative = FALSE` for zero), and a hand-built non-canonical value must still read as
  the number it spells (`emit_load_int`'s total decoder). **Switching the stored limb
  width to 32 or 64 bits is a blocker.** It would change a public record field's
  contents, which is a spec change, and it is not proposed. The fix processes the
  *same bytes* in wider words (below).
- `toString`/`toRadixString` output (digit case, `-` sign, `0` for zero) and `parse`
  acceptance and errors (`ErrInvalidFormat`, `ErrInvalidArgument` for radix) are
  unchanged.
- **Do not edit the benchmark rows** or the Python peer.
- **Forbidden wrong fixes:**
  - caching results across calls, such as memoizing `factorial`. That is invisible but
    measures nothing real;
  - special-casing the benchmark's constants;
  - reading 8 bytes past a user-built magnitude's capacity. A hand-built `big::Int`
    can have any capacity, so a word read must stay inside `dataCapacity`.

## Blast Radius

Found by `grep -n "^pub(crate) fn emit_" src/codegen/builtins/big/gen_big.rs` and
`grep -rln "gen_big" src`. The latter hits only `big/*`, `src/docs/spec/stdlib/19_big.md`,
and `crypto/{helper_random_int_big.rs, func_random_int.rs, mod.rs}`.

- `emit_mul_magnitude` → `multiply`, `pow`, `product`, `factorial`, `modPow`: fixed
  by this bug (word-wide multiply).
- `emit_int_to_string` → `toString`, `toRadixString`: fixed (chunked radix
  extraction).
- `func_parse.rs` accumulate walk → `parse`: fixed (chunked radix accumulation).
- `emit_div_mod_magnitude` → `divide`, `remainder`, `divMod`, `gcd`, `modPow`: fixed
  (word-wide Algorithm D; `modPow` stops computing the unused quotient).
- `lower_mod_pow`: fixed (iterate to the exponent's bit length, no trailing squaring).
- `lower_factorial`: fixed as a consequence of the word-wide multiply. A product tree
  is optional (Open Decisions).
- `emit_add_magnitude` / `emit_sub_magnitude` → `add`, `subtract`, `sum`: latent, 8×
  step count on a linear pass. They dominate `pidigits` together with
  multiply-by-small. In scope as the same word-wide change.
- `emit_shift_left_magnitude` / `emit_shift_right_magnitude`, `emit_compare_magnitude`,
  `emit_trim`, `fromBytes`/`toBytes`: linear byte passes. Latent and cheap. Word-wide
  is optional; out of scope unless a probe shows them in a row.
- `crypto::randomInt` big overload (`helper_random_int_big.rs`): shares the load/build
  emitters only. Unaffected in behavior; re-run its uniformity test
  (`rt_big_int.rs::crypto_random_int_big_overload_is_in_range_and_uniform`).
- `money`/`Fixed` (`money/gen_fixed_math.rs`): separate code, which does not use
  `gen_big`. Unaffected.

## Fix Design

This stays inside `mfb spec memory` (a result is still one freshly allocated `big::Int`
block and scratch buffers are still released at their made size, as
`emit_int_to_string` already does) and inside the `big::Int` public layout.

1. **Word-wide kernels over the byte magnitude.** All five targets are little-endian
   (`src/target/{macos_aarch64, linux_aarch64, linux_x86_64, win_x86_64,
   linux_riscv64}`), so the byte magnitude *is* a little-endian `u64` array.
   Read operands as `floor(count / 8)` whole words plus a 1–7-byte tail assembled
   bytewise, so a read never goes past `count`. Round each result's internal capacity up
   to a multiple of 8 so the kernels can store whole words. `emit_build_int` still
   trims to the canonical byte count, and the release size is still the recorded
   `dataCapacity`. Kernels:
   - multiply with `mul` + `umulh` and add-with-carry. x86-64 has `mul` → `rdx:rax`;
     riscv64 has `mulhu`;
   - add/sub with carry/borrow;
   - Algorithm D over base 2^32 with 64-bit intermediates. That avoids 128/64 division,
     which AArch64 and riscv64 lack, so it is the simpler first cut. Or base 2^64 with a
     2-by-1 reciprocal. Base 2^32 keeps D3's estimate in one `udiv`.
2. **Chunked radix conversion.** `emit_int_to_string`: divide the working copy by
   `radix^k` per pass, with the largest `k` such that `radix^k < 2^32`. With 32-bit
   words the step dividend `rem·2^32 + w` fits in 64 bits. That gives 9 decimal digits
   per pass, then `k` digits from the 32-bit remainder (zero-padded except in the top
   chunk). `parse` mirrors it: accumulate up to `k` digits into a word, then
   `magnitude = magnitude × radix^k + chunk`. The same digits and the same errors
   (validation pass unchanged) are produced for radix 2..36. Estimate (guess): 5,736/9
   passes × ≤ 596 words ≈ 190 k steps vs 6.8 M, about 36× fewer.
3. **modPow**: loop to `bitLength(exponent)`, not `count × 8`, and skip squaring after
   the top bit. Reduce with a remainder-only Algorithm D (no quotient block).
   Montgomery/Barrett are optional (Open Decisions).
4. **factorial**: with (1) the sequential product costs about 300 k word steps (guess).
   Also reuse the per-step factor instead of allocating a `big::Int` per `k` (multiply by
   a single word directly).

Rejected:

- **Karatsuba/Toom**. Python itself uses schoolbook below 70 digits, about 2,100 bits,
  and every row's operands are ≤ 4,096 bits. It is complexity with no measured need.
- **Changing the stored limb width.** It is a public-layout change (Non-goals).

Expected output shifts: `.ncode` of every program that calls a `big` member (the
byte-identity fixtures that import `big`). No runtime output changes.

## Phases

### Phase 1: failing tests + audit (no behavior change)

- [ ] `tests/runtime/rt_big_int_cost.rs` (new; one release build, five interleaved
      rounds, medians, like `rt_compress_bounds.rs::decode_time_is_linear_in_output_size`):
      - `mod_pow_cost_follows_bit_length`: `modPow(3, 65537, m)` ≤ 0.8 ×
        `modPow(3, 8388609, m)` (0.99 today, probe D).
      - `to_string_is_not_one_digit_per_pass`: `toString(2000!)` ≤ 60 × the median of
        a 128 KiB `collections::mid` copy, a native copy loop no fix here touches.
        Today it is ≈ 590× (23.8 ms vs 40 µs; `/tmp/dbF-ref` measured the copy at
        40.2–43.4 ms per 1,000).
      - `multiply_is_word_wide`: 100 × 2048-bit multiply ≤ 12 × the same `mid`
        reference (≈ 137× today: 5.5 ms vs 40 µs).
      - `parse_is_not_one_digit_per_pass`: the gcd row's 400 parses ≤ 8 × the
        reference (≈ 40× today: 1.58 ms vs 40 µs).

      Thresholds come from this doc's measurements and the design estimates. Phase 2
      re-derives them from the fixed build with a ≥ 1.5× margin.
- [ ] Extend `tests/runtime/rt_big_int.rs` oracle coverage for the new edges, with
      values computed outside MFB (its rule):
      - magnitudes of 1–17 bytes (word-tail boundaries), in every member;
      - hand-built non-canonical operands whose capacity is **not** a multiple of 8;
      - `toRadixString` for every radix 2..36 at chunk boundaries (for example
        10^9 − 1, 10^9, 10^18);
      - `modPow` with exponents 0, 1 and 2^k ± 1.
- [ ] Confirm the pidigits hypothesis by timing `bigintPiDigits(400)` with only the
      add/sub/multiply kernels widened (Phase 2 step order below allows that).

Acceptance: the new cost tests fail for the documented reasons; the new oracle cases
pass today (they pin behavior, not speed).
Commit: —

### Phase 2: the fix

- [ ] modPow bit-length loop + remainder-only reduction (`func_mod_pow.rs`).
- [ ] Word-wide add/sub/multiply kernels (`gen_big.rs`), with a word-rounded internal
      capacity in `emit_alloc_magnitude`, per target (`.ai/arch-abi.md`).
- [ ] Word-wide Algorithm D (`emit_div_mod_magnitude`).
- [ ] Chunked `emit_int_to_string` and `parse` accumulate.
- [ ] factorial: single-word factor path.

Acceptance: Phase 1 tests pass; `rt_big_int.rs` passes unchanged; the four rows meet
the Goal on the reference machine.
Commit: —

### Phase 3: regenerate expected outputs + full validation

- [ ] Full suite first, then `scripts/sync-goldens.sh` for `big`-importing
      byte-identity fixtures. The diff must be `.ncode` only.
- [ ] Cross-target runs of `rt_big_int.rs` on linux-x86_64 and linux-riscv64
      (`.ai/remote_systems.md`). The word kernels are per-ISA.
- [ ] Re-run `benchmark` to regenerate the baseline by the normal procedure and
      confirm the four rows.
- [ ] Update `src/docs/spec/stdlib/19_big.md` wherever it describes the algorithms
      (limb width, digit extraction), with `[[path:Symbol]]` citations
      (`.ai/specifications.md`).

Acceptance: full suite green on all targets; golden deltas are code-only; the rows meet
the Goal.
Commit: —

## Validation Plan

- Regression tests: `tests/runtime/rt_big_int_cost.rs` (cost ratios) and the extended
  `tests/runtime/rt_big_int.rs` (correctness oracles).
- Runtime proof: `/tmp/dbF-big`, `/tmp/dbF-big2` and `/tmp/dbF-mp` probes (tables A–D)
  and the four benchmark rows.
- Doc sync: `mfb spec stdlib big` (algorithm description). `mfb man big`: none, because
  the representation and behavior are unchanged. `mfb spec memory`: none.
- Full suite: `cargo test` (full), `scripts/artifact-gate.sh target/release/mfb all`, and
  `rt_big_int.rs` on every target.

## Open Decisions

- **Division base.** Recommended: base 2^32 with 64-bit intermediates (one `udiv` per
  D3 estimate on every ISA) vs. base 2^64 with a reciprocal (faster, more code per
  target).
- **factorial product tree.** Recommended: not needed if the word-wide sequential
  product meets 2.69 ms with `toString` fixed. Revisit only if it does not.
- **Montgomery reduction in modPow.** Recommended: no, because bit-length + word-wide
  Algorithm D should reach 5 ms (guess). Montgomery requires an odd modulus and a
  second code path.

## Summary

The rows are slow because every kernel walks the magnitude one byte at a time, not
because of per-call overhead, which is at Python parity. The decimal conversion and
parse also move one digit per pass, and modPow squares through the exponent's padding
bits. All of it is fixable inside the member bodies while keeping the public
`List OF Byte` magnitude. The risk is the per-ISA word kernels: carry chains, the
Algorithm D estimate with word-tail reads that never pass a user-built capacity, and
chunk padding in radix conversion. `rt_big_int.rs`'s independent oracles plus the new
boundary cases are the guard. Spec, memory model, public layout and output are
untouched. Changing the stored limb width would be a spec change and is explicitly not
proposed.
