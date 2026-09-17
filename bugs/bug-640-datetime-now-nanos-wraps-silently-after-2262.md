# bug-640: `datetime::nowNanos` / `datetime::now` wrap to a negative reading after 2262 instead of raising

Last updated: 2026-09-15
Effort: small
Severity: LOW
Class: Correctness (silent integer wraparound)

Status: Open
Regression Test: none yet — see Phase 1

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
so it is in scope too; the Windows `monotonicNanos` QPC fold is described as
overflow-safe and must be confirmed.

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
paths and record the result here. Commit:

Phase 2 — lower the scaling with checked multiply/add and a fail tail, declare
`ErrOverflow` on `nowNanos` and `now` (and on monotonic if shared), update the two
man pages from "wraps" to "raises `ErrOverflow`" (GREEN); full suite. Commit:
