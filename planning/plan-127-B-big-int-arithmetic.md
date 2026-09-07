# plan-127-B: `big::Int` arithmetic, bit operations, and text

Last updated: 2026-09-06
Effort: large (3h–1d)
Depends on: plan-127-A

Addition, subtraction and multiplication over the byte-limb magnitude, the bit-level
members, and the base-10 text seams. Twelve members, all natively lowered.

Behavioral outcome: a program can add, subtract and multiply `big::Int` values of
arbitrary size with **no error path at all**, parse one from a decimal string, and print
one back — with the round trip `big::parse(big::toString(x))` equal to `x` for every
`x`.

References — read these first:

- plan-127-A §4.2 (the pinned runtime layout) and §4.3 (the shared emitters).
- `src/codegen/builtins/bits/func_clz.rs` — the arch-neutral native lowering shape.
- `.ai/codegen-invariants.md` — register lifetimes, vreg allocation order, clobbers.
- `.ai/arch-abi.md` — per-architecture traps the `abi::` layer does not hide.

## Prerequisites

Stated once in plan-127-A and unchanged. This letter adds one:

| Must be true | Command | Status |
|---|---|---|
| plan-127-A is complete: Phases 1–5 ticked and their commits recorded | `grep -c '^- \[ \]' planning/plan-127-A-big-int-foundation.md` → `0` | NOT MET |
| `gen_big.rs` exposes the three shared emitters | `grep -c "fn emit_load_int\|fn emit_alloc_magnitude\|fn emit_build_int" src/codegen/builtins/big/gen_big.rs` → `3` | NOT MET |

If plan-127-A is not complete, this letter cannot start, full stop. Its emitters and its
pinned offsets are this letter's entire foundation; there is no partial mode.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. **If you stop, report the current status of *all* prerequisites.**

## 1. Goal

- `big::add`, `big::subtract`, `big::multiply`, `big::sum`, `big::product` are **total**
  — none declares an error, at any magnitude.
- `big::parse(big::toString(x))` equals `x` for every `x`, including negatives and zero.
- `big::bitLength(x)` equals the number of significant bits in `x`'s magnitude, and is
  `0` for zero.
- A 4096-bit × 4096-bit `multiply` produces the mathematically correct product, verified
  against an independently computed expectation.

### Non-goals

Every non-goal in plan-127-A §1 applies unchanged. Additionally:

- **No full division.** `divide`/`remainder`/`divMod` are plan-127-C. This letter lands
  only `__big_divSmall` (division by a single-byte divisor), and only as a private
  emitter serving `toString`.
- **No `and`/`or`/`xor` on `big::Int`.** Defining them requires two's-complement
  semantics over a sign-magnitude representation ("what is `-5 AND 3`?"), which is a
  real decision with no customer in this feature. Not shipped, not stubbed.
- **No Karatsuba or sub-quadratic multiplication.** Schoolbook only. An asymptotic
  improvement is a measured optimization, not part of getting correct arithmetic to
  land.

## 2. Current State

`big::Int` exists as a value record with a pinned runtime layout and three shared
native emitters (plan-127-A §4.2, §4.3). It can be built, converted to and from bytes
and `Integer`, compared, negated and tested for zero. It cannot yet be added, and it has
no text representation — a `big::Int` cannot be printed at all, which is why this
letter's `toString` is the first point the type becomes usable in ordinary code.

`big::compare` already implements magnitude ordering (plan-127-A Phase 5), which
subtraction reuses to decide result sign and operand order.

### Measured populations

| What | Count | Command |
|---|---|---|
| Members this letter adds | 12 | §4 |
| Members existing after plan-127-A | 11 | plan-127-A §4.4 |
| Private emitters this letter adds | 4 | §4.1 |

### Verified properties

- **Every byte-limb partial product fits in an `Integer`.** VERIFIED arithmetically:
  the maximum is `255 × 255 + 255 + 255 = 65535`, four orders of magnitude inside a
  signed 64-bit `Integer`. No widening-multiply primitive is required, and no
  intermediate can overflow — which is what makes `multiply` total.
- **`big::compare`'s magnitude ordering is reusable by subtraction.** VERIFIED by
  reading plan-127-A's `func_compare.rs`: it orders by magnitude length then by
  descending byte index, independent of sign, which is exactly the predicate
  subtraction needs to choose the larger operand.
- **UNMEASURED — schoolbook multiply cost at 4096 bits.** Phase 1 measures it. The
  number decides Open Decision 1 (limb width) and nothing else; correctness does not
  depend on it.

## 3. Design Overview

Four private emitters in `gen_big.rs`, then twelve members built on them.

**Where correctness risk concentrates: carry and borrow propagation.** Every defect in
this letter is a wrong byte at a boundary — a carry dropped at the high end, a borrow
not propagated through a run of zero bytes, a result buffer one byte short. These are
silent wrong answers, not crashes, and they hide at magnitudes the obvious tests do not
reach. Phase 2's tests therefore include explicit carry-chain cases (`0xFF...FF + 1`,
`0x100...00 - 1`) rather than only random values.

**Where design uncertainty concentrates: none that blocks.** The algorithms are
schoolbook and settled. The one open number is performance, measured in Phase 1 and
consequential only for Open Decision 1.

**Byte-identity is not this letter's gate.** Same narrow non-disturbance role as
plan-127-A §3: a program that does not `IMPORT big` must stay byte-identical.

### Rejected alternatives

- **Implementing `subtract` as `add(a, negate(b))`.** Rejected: it still needs magnitude
  subtraction underneath, so it saves nothing and adds an allocation. Sign dispatch at
  the top of one shared emitter is simpler and allocates once.
- **A separate `toDecimalString` fast path using repeated `Integer` division.** Rejected:
  it would be a second implementation of base conversion, divergent from `__big_divSmall`
  and separately wrong. One path.

## 4. Detailed Design

### 4.1 Private emitters (`gen_big.rs`)

- `emit_add_magnitude(a, b) -> (data, count)` — byte-wise addition with carry; result
  capacity `max(lenA, lenB) + 1`.
- `emit_sub_magnitude(larger, smaller) -> (data, count)` — byte-wise subtraction with
  borrow. **Caller guarantees `larger >= smaller`**; the emitter does not check, and the
  sign dispatch in §4.2 is what establishes it.
- `emit_mul_magnitude(a, b) -> (data, count)` — schoolbook: for each byte of `a`,
  multiply-accumulate across `b` into the result at the right offset. Result capacity
  `lenA + lenB`.
- `emit_div_small(a, divisor AS Integer) -> (data, count, remainder)` — long division by
  a single-byte divisor, walking from the high byte down. Serves `toString` only.

All four return raw `(data, count)` and never construct a record; every member routes
its result through plan-127-A's `emit_build_int`, so normalization stays single-sited.

### 4.2 Sign dispatch

`add` and `subtract` share one rule. With `sa`/`sb` the operand signs:

- **Same effective operation** (`add` with `sa = sb`, or `subtract` with `sa ≠ sb`):
  magnitudes add; the result sign is `sa`.
- **Opposing** (`add` with `sa ≠ sb`, or `subtract` with `sa = sb`): compare magnitudes;
  subtract smaller from larger; the result sign is the sign of the operand with the
  larger magnitude, and `emit_build_int` forces it false when the result is zero.

That last clause is why `1 + (-1)` yields `{[], FALSE}` and not negative zero.

### 4.3 Members (12)

```basic
big::add(a AS big::Int, b AS big::Int) AS big::Int                    ' total
big::subtract(a AS big::Int, b AS big::Int) AS big::Int               ' total
big::multiply(a AS big::Int, b AS big::Int) AS big::Int               ' total
big::sum(values AS List OF big::Int) AS big::Int                      ' total
big::product(values AS List OF big::Int) AS big::Int                  ' total

big::bitLength(a AS big::Int) AS Integer                              ' total
big::shiftLeft(a AS big::Int, count AS Integer) AS big::Int           ' ErrInvalidArgument (count < 0)
big::shiftRight(a AS big::Int, count AS Integer) AS big::Int          ' ErrInvalidArgument (count < 0)
big::testBit(a AS big::Int, index AS Integer) AS Boolean              ' ErrInvalidArgument (index < 0)

big::parse(text AS String, radix AS Integer = 10) AS big::Int         ' ErrInvalidFormat, ErrInvalidArgument
big::toString(value AS big::Int) AS String                            ' total, base 10
big::toRadixString(value AS big::Int, radix AS Integer) AS String     ' ErrInvalidArgument (radix outside 2..36)
```

Five members with no error path is the headline: overflow-proof accumulation with no
`TRAP`, no `AS Error`, no propagation. That is the reason to reach for `big::Int` over
`Integer`, and it is a property to defend as the surface grows.

`sum` and `product` are not conveniences — they are the answer to "the package is slow".
One native call covers N operations instead of N calls each re-resolving operands. A
million-term `sum` pays the per-call cost once.

`toString` is total and stays separate from `toRadixString` deliberately: a radix needs
range-checking, and folding it into the most-called member in the package would make
that member fallible for a reason base-10 callers never encounter.

`bitLength` returns `0` for zero — the significant-bit count, not a byte count. It is
what reports a key size, so plan-127-D depends on it.

`shiftLeft`/`shiftRight` operate on the **magnitude**, preserving sign; `shiftRight` of
a negative is therefore magnitude truncation toward zero, not an arithmetic shift. This
must be stated in the descriptor `desc`, because the two conventions differ for negative
values and a reader will assume the other one.

## Compatibility / Format Impact

- **Added:** twelve members under `big::`. No new type, no new enum.
- **Unchanged:** `big::Int`'s layout, canonical form and field order; every other
  package's surface; the numeric tower; every file under `packages/`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partial and say what remains. Fill each
> `Commit:` line the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — additive arithmetic

- [ ] `gen_big.rs`: `emit_add_magnitude`, `emit_sub_magnitude` per §4.1.
- [ ] `func_add.rs`, `func_subtract.rs` — the shared sign dispatch of §4.2.
- [ ] Tests — carry and borrow chains explicitly, not only random values:
      `0xFF..FF + 1` (carry out of every byte, result one byte longer);
      `0x0100..00 - 1` (borrow through a run of zeros);
      `x + negate(x) = 0` and the result is `{[], FALSE}`, not negative zero;
      `subtract(a, b) = negate(subtract(b, a))`;
      addition is commutative and associative over a spread covering both signs.
- [ ] Measure and record: 1e6 iterations of a 128-bit `add`, with the command.

Acceptance: both members report `native_member_declares_error` → `false`; the carry and
borrow chain tests pass; the negative-zero case yields canonical zero.
Commit: —

### Phase 2 — multiplication and the aggregates

- [ ] `gen_big.rs`: `emit_mul_magnitude` per §4.1.
- [ ] `func_multiply.rs`, `func_sum.rs`, `func_product.rs` — all total. `sum`/`product`
      over an empty list return the identity (`0` and `1` respectively); state this in
      the descriptors.
- [ ] Tests: `multiply` against hand-computed products at 8, 64, 512 and 4096 bits;
      sign combinations across all four quadrants; `multiply(x, 0)` is canonical zero;
      `sum`/`product` agree with folding `add`/`multiply` over the same list;
      empty-list identities.
- [ ] Measure and record: one 4096-bit × 4096-bit `multiply`, and 1e4 iterations of a
      512-bit `multiply`, with the commands. Resolve Open Decision 1 against these.

Acceptance: the 4096-bit product matches an independently computed expectation
(computed outside MFB and committed as a test constant, not produced by this code);
all three members report `native_member_declares_error` → `false`.
Commit: —

### Phase 3 — bit operations

- [ ] `func_bit_length.rs`, `func_shift_left.rs`, `func_shift_right.rs`,
      `func_test_bit.rs`.
- [ ] Tests: `bitLength` of zero is `0`, of `1` is `1`, of `255` is `8`, of `256` is `9`;
      `shiftLeft(x, 8)` equals `multiply(x, fromInteger(256))`;
      `shiftRight(shiftLeft(x, n), n) = x`;
      `shiftRight` of a negative truncates toward zero and preserves sign;
      `testBit` agrees with `shiftRight`+`isZero` across a spread;
      each of the three fallible members raises `ErrInvalidArgument` on a negative
      count/index.

Acceptance: the shift/multiply equivalence holds for `n` in 1..64 across several
magnitudes, and the three negative-argument cases each raise `ErrInvalidArgument`.
Commit: —

### Phase 4 — text (largest blast radius last: base conversion touches every member)

- [ ] `gen_big.rs`: `emit_div_small` per §4.1.
- [ ] `func_to_string.rs` (total, base 10), `func_to_radix_string.rs`
      (`ErrInvalidArgument` outside radix 2..36), `func_parse.rs`
      (`ErrInvalidFormat` on malformed text, `ErrInvalidArgument` on a bad radix).
- [ ] Tests: `parse(toString(x)) = x` across a spread covering zero, negatives, and
      values well past `Integer` range; `toString` of zero is `"0"` with no sign;
      `toString` of a negative carries exactly one leading `-`;
      `parse` rejects `""`, `"-"`, `"12x"`, `"+-1"` with `ErrInvalidFormat`;
      `parse` accepts a leading `-` and rejects a leading `+` (state which in the
      descriptor either way);
      `toRadixString(x, 16)` agrees with the magnitude bytes for several values;
      radix `1` and radix `37` each raise `ErrInvalidArgument`.

Acceptance: the `parse`∘`toString` round trip holds for every value in the test spread,
and `toString` reports `native_member_declares_error` → `false`.
Commit: —

## Validation Plan

- **Tests:** Rust unit tests in each `func_*.rs` for registry facts; runtime behavior
  extending `tests/rt_big_int.rs`, including every negative/error case listed above.
- **Coverage check:** the new `gen_big.rs` emitters must appear in the coverage
  denominator — `tests/` gives `src/**` zero coverage, so the emitter behavior is
  reached through unit tests inside `src/codegen/builtins/big/`.
- **Runtime proof:** an `.mfb` program computing a 4096-bit product and a 1000-term
  `sum`, printing both via `toString`, run on a release binary and checked against
  independently computed expectations. Lowering is not runtime proof.
- **Doc sync:** `scripts/man-census.sh --fill big` (every new member documented) and
  `scripts/man-run-examples.sh big --run` (every example on the page compiles and runs).
  `scripts/man-census.sh --memory-scope` must report 0 unclassified hits.
- **Acceptance:** `cargo test --no-fail-fast`, then `scripts/test-accept.sh`; regenerate
  `.ir` goldens with `scripts/sync-goldens.sh` and prove the delta is only `big`'s.
- **Formatting:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

1. **Limb width for multiplication.** **Recommend: keep byte limbs** — the storage form
   is bytes, every partial product stays ≤ 65535, and no unpack/repack pass is needed.
   Alternative: unpack to 32-bit limbs for ~16× fewer partial products at the cost of
   two full passes per operation. Phase 2's measurement decides; do not switch on
   intuition. If it switches, the storage form does **not** change — only the
   emitter's internal working representation.
2. **Whether `parse` accepts a leading `+`.** **Recommend: reject it**, so that
   `toString` is the exact inverse of `parse` and there is one spelling per value.
   Alternative: accept it as a convenience, which makes `parse` non-injective on text.
   Either way the descriptor states it explicitly.

## Corrections

<!-- Filled in DURING execution. -->

## Summary

The engineering risk is carry and borrow propagation, and it is the kind that produces
silent wrong answers rather than crashes — hence explicit carry-chain tests rather than
random-value coverage, and hence an independently computed 4096-bit expectation rather
than one this code produced.

Left untouched: division and everything built on it (plan-127-C), the `mfb spec`
chapter, the acceptance fixture, and the `crypto` integration (plan-127-D).
