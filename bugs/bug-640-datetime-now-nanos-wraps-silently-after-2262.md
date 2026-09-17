# bug-640: `datetime::nowNanos` / `datetime::now` wrap to a negative reading after 2262 instead of raising

Last updated: 2026-09-17
Effort: small
Severity: LOW
Class: Correctness (silent integer wraparound)

Status: Fix landed on `worktree-B-640` (gate test GREEN); full suite and golden regeneration pending integration
Regression Test: `tests/runtime/rt_datetime_clock_overflow.rs`

On macOS and Linux, `datetime::nowNanos` computes the wall-clock reading as
`tv_sec * 1_000_000_000 + tv_nsec` with an **unchecked** multiply and add. The
64-bit signed nanosecond count runs out at `2262-04-11T23:47:16.854775807Z`.
After that, the reading silently wraps to a large negative number. `datetime::now`
is `nowNanos` split into `seconds`/`nanos`, so it returns a pre-1970 `Instant`
with no error.

Everywhere else in `datetime`, arithmetic that leaves the `Integer` range raises
`ErrOverflow`. Examples: `datetime::toNanos(datetime::instant(9_300_000_000))`,
`datetime::negate` on the most negative seconds, and `datetime::plus` at
`Integer` max (probe `/tmp/p125-ex/dtrest`). `nowNanos` declares no errors at all
(`func_now_nanos.rs:register`, `errors: vec![]`), so a caller cannot trap the
condition.

**The single correct behavior a fix produces:** a wall-clock reading whose
nanosecond count does not fit an `Integer` raises `ErrOverflow` from
`datetime::nowNanos` and `datetime::now`, and both declare it. Readings before
2262 are unchanged.

Found by plan-125-C Phase 3 while reviewing the `nowNanos` man page. That page said
the count "overflows in the year 2262" without saying what a caller sees. **Filed,
not fixed**, by user instruction during a documentation-only plan ("file all bugs,
make no fixes"). The page now documents the as-is wrap.

## Reproduction

This cannot be reproduced by running a program on a correctly set host before 2262.
The defect is in the emitted instructions, shown by the source:

`src/codegen/builtins/datetime/gen_shared.rs:112-121`
(`emit_libc_clock_nanos`):

```rust
// nanos = tv_sec * 1_000_000_000 + tv_nsec.
abi::load_u64(&sec, abi::stack_pointer(), TIMESPEC_OFFSET),
abi::load_u64(&nsec, abi::stack_pointer(), TIMESPEC_OFFSET + 8),
abi::move_immediate(&scale, "Integer", "1000000000"),
abi::multiply_registers(&sec, &sec, &scale),
abi::add_registers(RESULT_VALUE_REGISTER, &sec, &nsec),
```

Neither the multiply nor the add is followed by an overflow check or a fail tail.

A Phase 1 test can reproduce it without touching the host clock. Unit-test the
emitted sequence with `tv_sec = 9_223_372_037` (the first second past the limit).
From the unchecked instructions above, today's result should be negative (not yet
run); the fixed result is the `ErrOverflow` fail path.
Alternatively, an integration test can set the process clock with a test-only
clock override, if one exists; otherwise use the unit test.

## Phase 1 findings (2026-09-17)

RED test `tests/runtime/rt_datetime_clock_overflow.rs` pins the clock with a
`clock_gettime` interposer (`DYLD_INSERT_LIBRARIES` / `LD_PRELOAD`), so it runs the
real emitted code rather than a unit test of the instruction list. On macos-aarch64
at `798870ec2`: a reading of `(9223372036, 854775808)` returns
`nowNanos ok -9223372036854775808` and `now ok -9223372037 145224192`;
`(-9223372037, 0)` wraps the other way to `9223372036709551616`. The in-range
maximum `(9223372036, 854775807)` is exact, and a control reading proves the
interposer is loaded.

Audit: `monotonicNanos` shares `emit_libc_clock_nanos` and wraps identically (same
test). The Windows `nowNanos` path (`(FILETIME - epoch) * 100`, unchecked
`multiply_registers`) has the same wrap past 2262 — a FILETIME reaches year ~60056 —
so it is in scope too.

Windows `monotonicNanos` audit (reading `lower_monotonic_nanos`): the QPC fold
`(counter/freq)*1e9 + ((counter%freq)*1e9)/freq` is "overflow-safe" only for the
fraction. The whole-second product `q*1e9` was an unchecked multiply and the
final add an unchecked add, so a count past `Integer` max (about 292 years of
counter at any frequency) wrapped exactly like the libc path. Latent in
practice, but the same defect, so fixed in the same change. The fraction's
`(counter%freq)*1e9` is exact `u64` arithmetic only while `freq < 2^64/1e9`
(~18.4 GHz); Windows 10+ reports a fixed 10 MHz, so this is a documented
precondition (spec `stdlib datetime`), not a reachable wrong value.

The blast radius includes two callers outside `datetime`: `crypto::uuid7` and
`crypto::ulid` compute `datetime::nowNanos() / 1000000`. Before the fix a
post-2262 clock gave them a silently wrong (negative) millisecond stamp; after
it they propagate `ErrOverflow` (probe: both print `raised 77050010` with the
clock pinned to `(9223372037, 0)`), which their `errors` lists
(`ErrUnknown`, `ErrOutOfMemory`) do not declare. The `http` request readers
difference `monotonicNanos` readings and are unaffected in practice.

## Root cause

`emit_libc_clock_nanos` lowers the seconds-to-nanoseconds scaling with plain
`multiply_registers`/`add_registers`. It does not use the checked
arithmetic the rest of the compiler lowers `Integer` `*` and `+` to. And because
the member is registered with `errors: vec![]` and a `void_int_result` wrapper,
there is no fail tail to branch to (`gen_shared.rs:126-129`, "the body emitted its
own fallible ABI (the OK tail, and for `localOffset` the range-fail tail)").

## Non-goals

- Changing the `nowNanos` result type or the 2262 limit itself (the page documents
  it; an `Instant`-returning clock read that avoids the limit is a design change).
- Any change to `datetime::monotonic`/`monotonicNanos` readings except where they
  share the same helper and the same unchecked scaling (see audit).
- Adding a clamp: saturating at `Integer` max would also be a silent wrong value.

## Blast-radius audit

- `datetime::now`: built on `nowNanos`, so fixed by the same change.
- `datetime::monotonicNanos` / `datetime::monotonic`: check whether they call
  `emit_libc_clock_nanos` too. A monotonic origin is usually boot time, so the wrap
  is roughly 292 years of uptime away. That path is latent but shares the code; fix
  it in the same change if it is the same helper.
- Windows: `gen_shared.rs:57` describes a separate "overflow-safe tick→nanosecond
  conversion" (FILETIME, with a `win_filetime_max_unix_sec_matches_no_wrap_formula`
  test). **Not examined** by this filing. Phase 1 must confirm whether that path
  already fails closed or clamps.

## Fix

Phase 1 — RED test on the emitted scaling sequence (or a clock-override test),
expecting `ErrOverflow` for `tv_sec` past the limit. Audit the monotonic and Windows
paths and record the result here. Commit: `66ee644b0`

Phase 2 — lower the scaling with checked multiply/add and a fail tail, declare
`ErrOverflow` on `nowNanos` and `now` (and on monotonic if shared), update the two
man pages from "wraps" to "raises `ErrOverflow`" (GREEN); full suite. Commits:
`94305cfdc` (fix), `b1be9ca5a` (man pages), `7932c1601` (spec).

### Phase 2 notes (2026-09-17)

**libc fold (`emit_libc_clock_nanos`, shared by both members).** The fold is
computed in 128 bits and range-checked once, instead of checking the multiply
and the add separately:

```
smulh high, sec, 1e9          ; high word of the product (reads sec first)
mul   sec,  sec, 1e9          ; low word
asr   sign, nsec, #63         ; tv_nsec sign-extended = its high word
add_carry sec,  carry, sec,  nsec, xzr
add_carry high, xzr,   high, sign, carry
asr   sign, sec, #63
cmp   high, sign ; b.ne <symbol>_overflow
mov   %retMFB1, sec
```

A separate multiply check would be wrong at one edge: `(-9223372037, 145224192)`
overflows the product but sums to exactly `Integer` min, which the test's own
oracle (`i128` sum, then `i64::try_from`) calls representable. Probed: that
reading returns `-9223372036854775808`; `(-9223372037, 145224191)` raises.

**Windows `nowNanos`.** `cmp ft, #0 ; b.lt overflow` first: a FILETIME `>= 2^63`
has a Unix count of at least `2^63 - epoch` intervals, which overflows once scaled,
and below it the rebase subtract cannot wrap. Then `smulh`/`mul` by 100 and the
same high-word-vs-sign compare.

**Windows `monotonicNanos`.** `umulh high, q, 1e9 ; cmp high, #0 ; b.ne`, then
`mul q, q, 1e9 ; cmp q, #0 ; b.lt` (the unsigned product must fit 63 bits), then
after the fraction `add q, q, rem ; cmp q, #0 ; b.lt` (two non-negative values, so
overflow is exactly a negative sum).

**Fail tail (`emit_clock_overflow_tail`).** One `<symbol>_overflow` label after the
OK return, `raise_error_into(.., "ErrOverflow", ..)`, `ret` — the `localOffset`
`range_fail` shape. Every vreg is allocated after the external call, so nothing
is live in a register across it. The runtime-helper call site already compares
the tag unconditionally and propagates, so no call-site change was needed; the
MFBASIC wrappers `now`/`monotonic` have no `TRAP` and propagate it. All four
members declare `ErrOverflow`, and the `members_declare_the_errors_they_raise`
rows that recorded them as never raising were corrected (disproved by the gate
test).

**Verification.**
- `cargo test --test rt_datetime_clock_overflow`: 4 passed (macos-aarch64).
- Same program + interposer, cross-built and run: linux-x86_64 glibc (box 2228)
  and linux-riscv64 musl (box 2229) print the expected lines for
  `(9223372036, 854775808)`, `(9223372037, 0)`, `(i64::MAX, 999999999)`,
  `(-9223372037, 0)`, `(-9223372037, 145224192)`, `(-9223372036, 0)`,
  `(9223372036, 854775807)` and an in-range reading.
- windows-x86_64 build runs on box 2230 with correct real-clock readings. The
  Windows overflow branches are **not runtime-exercised**: kernel32 has no
  loader-level interposer, so they rest on the reasoning above.
- linux-aarch64 cross-builds; not run.
- `.ncodesum` byte-identity goldens for datetime (and any fixture that calls a
  clock member) change by design and are regenerated at integration.

## Fallout found during the fix (2026-09-17)

- **`crypto::uuid7` and `crypto::ulid` now raise an error they did not declare.**
  Both stamp `datetime::nowNanos() / 1000000`; with the clock pinned past 2262 they
  raise 77050010 (before the fix they encoded a wrapped negative timestamp). Both
  now declare `ErrOverflow` and say so on their man pages. RED unit test
  `crypto::tests::clock_derived_identifiers_declare_the_clock_overflow`; runtime
  case `clock_derived_identifiers_raise_overflow_past_2262`.
- `http`'s request-read helpers call `datetime::monotonicNanos`; `handleRequest`
  already traps any raise from them into a 500, so no declaration changes.
- `src/codegen/debug/clock.rs:emit_debug_monotonic_nanos` (the `--debug` arena-series
  timestamp) has the same unchecked fold on `CLOCK_MONOTONIC`/QPC. It has no error
  path by design, and a monotonic origin is boot time, so the wrap needs ~292 years
  of uptime; recorded here, not changed.

