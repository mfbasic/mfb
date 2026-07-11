# bug-42: `datetime::localOffset` ignores `localtime_r`'s NULL return and reads the uninitialized `struct tm` — leaks a raw stack qword to the program

Last updated: 2026-07-09
Effort: small (<1h)

`lower_datetime_helper` emits `localtime_r(&epochSeconds, &tm)` and then
unconditionally loads `tm.tm_gmtoff` from the stack buffer. `localtime_r` returns
`NULL` (and sets `EOVERFLOW`) when the instant cannot be represented as broken-down
time — the year does not fit `tm_year`'s `int`. On that path libc leaves the caller's
`struct tm` **untouched**, so the helper returns whatever 8 bytes happened to be on
the stack at `sp + TM_OFFSET + TM_GMTOFF_OFFSET`.

On macOS/aarch64 today `datetime::localOffset(9223372036854775807)` returns a
*different value on every run* — an ASLR-varying main-thread stack address such as
`6097529168` (`0x16B7A2BD0`). This is both a wrong result on a function the man page
documents as having **"No errors"**, and an **information leak**: a pure-MFBASIC
program with no `LINK`, no `unsafe`, and no native code can read an uninitialized
stack qword and thereby defeat ASLR on the host.

The single correct behavior a fix produces: `datetime::localOffset` checks
`localtime_r`'s return value; on `NULL` it must not read `tm` at all. It either
returns a defined value or raises an error — see [§ Open Decisions](#open-decisions).

References:

- `src/target/shared/code/datetime.rs:85-105` (`lower_datetime_helper`, the
  `"datetime.localOffset"` arm) — `platform.emit_libc_call("localtime_r", …)` is
  followed directly by `abi::load_u64(RESULT_VALUE_REGISTER, sp, TM_OFFSET +
  TM_GMTOFF_OFFSET)`; the call's `x0` result is never tested.
- `src/target/shared/runtime/datetime_specs.rs:39-40` (the
  `_mfb_rt_datetime_datetime_localOffset` spec).
- `src/docs/man/builtins/datetime/localOffset.md:66-68` — "## Errors / No errors",
  and `:58` documents `epochSeconds` as an unconstrained `Integer`.
- POSIX / `localtime_r(3)`: "returns NULL … the contents of the structure pointed to
  by `result` are unspecified" (glibc and Darwin BSD libc both set `EOVERFLOW`).
- Consumers: `datetime::offsetAt` (local zones) and `datetime::toLocal`, per
  `localOffset.md:45-48` and `withZone.md:55`.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

```
mfb init /tmp/loffp
cat > /tmp/loffp/src/main.mfb <<'EOF'
IMPORT io
IMPORT datetime

SUB main()
  io::print(toString(datetime::localOffset(9223372036854775807)))
END SUB
EOF
mfb build /tmp/loffp
/tmp/loffp/loffp.out   # run it three times
```

- Observed (macOS 15 / aarch64, host TZ `Pacific/Honolulu`, three consecutive runs):

  ```
  6097529168
  6169438544
  6133721424
  ```

  Each value is a distinct main-thread stack address (`0x16B…`), varying with ASLR.

- Expected: a defined, run-stable result — either the host's offset (`-36000`) or a
  documented error. Never a value that varies per run.

Contrast cases that work correctly today (regression guards):

- `datetime::localOffset(0)` → `-36000` (`0` under `TZ=UTC`): in range, `localtime_r`
  succeeds and writes `tm`.
- `datetime::localOffset(67768036191676800)` → `-36000`: still representable.
- `datetime::localOffset(9000000000000)` → `-36000`.
- Calling `localOffset(0)` *before* `localOffset(maxint)` in the same program makes
  the bad call return `-36000` "correctly" — the first call left a valid `tm` in the
  same stack slot. This aliasing is exactly what makes the bug silent and
  layout-sensitive, and is why it must not be tested by a multi-call program.
- On a freshly-zeroed frame (`localOffset(2000000000000000000)` as the *first* call)
  the leak reads zeros and returns a stable `0` — wrong, but not obviously so.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS 15 | aarch64, Darwin BSD libc | fails ✗ (verified: varying stack address) |
| Linux | aarch64/x86_64/riscv64, glibc & musl | fails ✗ (unverified; same codegen path, same libc contract) |

## Root Cause

`src/target/shared/code/datetime.rs:lower_datetime_helper`, `"datetime.localOffset"`
arm. The emitted sequence is:

```
str  x0, [sp, #TIME_T_OFFSET]      // stash epochSeconds
add  x0, sp, #TIME_T_OFFSET        // &time_t
add  x1, sp, #TM_OFFSET            // &tm
bl   localtime_r                   // <-- x0 result DISCARDED
ldr  x0, [sp, #TM_OFFSET + 40]     // tm.tm_gmtoff  (garbage when the call failed)
mov  x1, #RESULT_OK_TAG
ret
```

`localtime_r` computes broken-down time and must store the year in `tm_year`, an
`int`. For `|epochSeconds|` beyond roughly `6.7e16` the year exceeds `INT_MAX` and
libc bails out early with `NULL`/`EOVERFLOW` *without writing any field of `tm`*. The
helper never allocates or zeroes that 56-byte buffer (it is raw frame space carved by
`finalize_vreg_body_with_locals`), so the subsequent `ldr` observes whatever the
previous stack frame left at `sp+56`.

The two sibling arms are immune, which bounds the bug precisely: `datetime.nowNanos`
and `datetime.monotonicNanos` call `clock_gettime` with `CLOCK_REALTIME` /
`CLOCK_MONOTONIC`, which cannot fail for those clock ids, and their `timespec` is
fully written on every success. Only `localOffset` takes an **unvalidated
user-supplied argument** into a libc call that has a failure mode.

The result is additionally tagged `RESULT_OK_TAG` unconditionally
(`datetime.rs:113-117`), so no `TRAP`/error path can observe the failure either.

## Goal

- `datetime::localOffset(epochSeconds)` never reads `tm` when `localtime_r` returned
  `NULL`.
- The value returned for an out-of-range `epochSeconds` is defined, documented, and
  identical across runs, hosts, and stack states.
- `datetime::localOffset(9223372036854775807)` produces the same output on three
  consecutive runs.

### Non-goals (must NOT change)

- In-range behavior: `localOffset(0)`, `localOffset(9000000000000)`, and every
  DST-boundary case must keep returning exactly what they return today.
- The `nowNanos` / `monotonicNanos` arms and the shared frame layout constants
  (`TIMESPEC_OFFSET`, `TM_OFFSET`, `LR_OFFSET`) — do not renumber them.
- The `datetime::` package's public signature (`AS Integer`) and the calendar math in
  `datetime_package.mfb`.
- **Forbidden wrong fix:** zeroing the `tm` buffer before the call. That converts an
  obvious garbage value into a *plausible* `0` (UTC) and silently mis-converts times
  for every non-UTC host. The `NULL` return must be branched on.

## Blast Radius

Found by searching for `emit_libc_call` sites whose libc callee has a failure return
that the emitted code then ignores.

- `src/target/shared/code/datetime.rs:93` (`localtime_r`) — **fixed by this bug.**
- `src/target/shared/code/datetime.rs:69` (`clock_gettime`) — unaffected: cannot fail
  for `CLOCK_REALTIME`/`CLOCK_MONOTONIC`, and the `timespec` is fully written.
- `datetime::offsetAt` (local zones) and `datetime::toLocal` in
  `datetime_package.mfb` — **downstream consumers**; they inherit the fix. They pass
  an `Instant`'s seconds through unchanged, so a user-constructed far-future `Instant`
  reaches the same path.
- `src/target/{macos_aarch64,linux_aarch64,linux_x86_64,linux_riscv64}/plan.rs`
  (`"datetime.localOffset"` import rows) — unaffected: import plumbing only, no
  codegen change.

## Fix Design

In the `"datetime.localOffset"` arm, branch on `localtime_r`'s `x0` return before the
`tm_gmtoff` load:

```
bl   localtime_r
cbz  x0, <fail>          // NULL => out of range, tm is unspecified
ldr  x0, [sp, #TM_OFFSET + 40]
mov  x1, #RESULT_OK_TAG
b    <done>
<fail>:
<per the Open Decision: either ERR path, or a defined in-range fallback>
<done>:
```

The risk concentrates in the error-tagging convention: the arm currently ends with a
single unconditional `RESULT_OK_TAG` + `ret` shared by all three calls, so the
`localOffset` arm must grow its own labelled tail without disturbing the other two.
Use `push_error_message_address` + `RESULT_ERR_TAG` exactly as
`fs_helpers.rs:emit_errno_error_mapping` does, if the ERR route is chosen.

Rejected alternatives:

- *Zero the buffer, keep reading it.* Rejected: silently returns `0` (UTC) for
  out-of-range instants on non-UTC hosts. Trades a loud bug for a quiet one, and does
  not fix the "No errors" documentation lie.
- *Range-check `epochSeconds` in the compiler front end.* Rejected: the exact
  representable range is a libc/platform property (it depends on `tm_year` width and
  the zone's transition table), not a constant the compiler can know.
- *Clamp `epochSeconds` into range before the call.* Rejected: returns a confidently
  wrong offset for an instant the caller never asked about.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add a runtime-behavior test that calls `datetime::localOffset(9223372036854775807)`
      as the program's **first** `datetime` call and asserts a defined result. Confirm
      it fails today (non-deterministic value). Note in the test why it must be the
      first call (stack-aliasing, see Contrast).
- [x] Add the in-range contrast cases (`0`, `9000000000000`) as regression guards.
- [x] Blast-radius audit complete (above).

Acceptance: the new test fails for the documented reason; in-range guards pass.
Commit: —

### Phase 2 — the fix

- [x] Resolve the Open Decision below. **Chosen: raise `ErrInvalidArgument`
      (`77050002`, ERR tag).** The call site (`emit_runtime_helper_call`) already
      checks the tag and auto-propagates on error, so `offsetAt`/`toLocal` inherit
      the failure with **no `datetime_package.mfb` change** — the ripple feared in
      the Open Decision does not materialize (the package FUNCs simply propagate).
- [x] Emit the return-NULL branch and the failure tail in
      `src/target/shared/code/datetime.rs`, `"datetime.localOffset"` arm. (Used
      `compare_immediate` + `branch_eq` on the return register rather than a raw
      `cbz`, matching this module's abi vocabulary.)
- [x] Update `src/docs/man/builtins/datetime/localOffset.md` (`## Errors`), the
      `mfb spec` datetime + runtime-helper-abi topics, and the `datetime_specs.rs`
      header comment. `offsetAt`/`toLocal` prose already documents their errors as
      inherited from `localOffset`.

Acceptance: Phase 1 test passes; in-range contrast cases unchanged; the three
`nowNanos`/`monotonicNanos` code paths are byte-identical.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [x] Regenerate `.ncode` / codegen goldens; confirm the only delta is the
      `localOffset` helper body. **No existing fixture carries a native
      (`.ncode`/`.nplan`/`.nir`/`.mir`/`.hex`) golden that includes this helper —
      no datetime fixture uses native goldens, and no golden references
      `localtime_r`/`datetime_localOffset`. The `.ir` goldens are front-end only and
      unaffected. So there is no golden delta to regenerate.**
- [ ] `scripts/test-accept.sh` green. *(Left to the orchestrator. The two new
      fixtures' goldens were verified to match by replaying the exact harness
      commands and diffing; the 23 datetime unit tests pass.)*
- [x] Re-run the reproduction on macOS/aarch64 — prints `outOfRange
      invalidArg=TRUE` on three consecutive runs (was a per-run ASLR stack address).
      *(Linux aarch64 + x86_64 + riscv64 left to the orchestrator's remote matrix;
      same codegen path, same libc contract.)*

Acceptance: full suite green; golden delta confined to the `localOffset` helper; the
reproduction is stable on every environment in the matrix.
Commit: —

## Validation Plan

- Regression test(s): the first-call `localOffset(maxint)` determinism test plus the
  in-range guards, in the runtime-behavior test folder.
- Runtime proof: run the built binary three times on each platform; the out-of-range
  output must be byte-identical each time (today it varies with ASLR).
- Doc sync: `src/docs/man/builtins/datetime/localOffset.md` `## Errors` currently says
  "No errors" — it must be corrected either way the Open Decision lands.
- Full suite: `scripts/test-accept.sh`; `scripts/artifact-gate.sh` for the codegen delta.

## Open Decisions

- **What does an out-of-range `epochSeconds` return?** Recommended: raise a proper
  error (`ErrInvalidArgument`-class, `RESULT_ERR_TAG`), matching how every other
  fallible runtime helper reports, and update the man page's `## Errors` section. This
  makes `localOffset` fallible, so `datetime::offsetAt`/`toLocal` must decide whether
  to propagate or clamp — that is the real cost of this option.
  Alternative: return `0` with an `OK` tag and document "offset is `0` for instants
  outside the host's representable range". Cheaper (keeps `localOffset` infallible and
  leaves consumers untouched) but returns a wrong-looking-plausible value, the same
  objection that disqualifies the zero-the-buffer fix. Prefer the error unless the
  fallibility ripple into the `datetime` package proves large.

## Resolution

Fixed. `src/target/shared/code/datetime.rs` `lower_datetime_helper`,
`"datetime.localOffset"` arm now tests `localtime_r`'s return before touching the
`tm` buffer:

```
bl   localtime_r
cmp  x0, #0                     ; RETURN_REGISTER == RESULT_TAG_REGISTER
b.eq <symbol>_range             ; NULL => tm unspecified, do not read it
ldr  x1, [sp, #TM_OFFSET+40]    ; tm_gmtoff -> RESULT_VALUE_REGISTER
; ... falls through to the shared OK tail (x0 = OK tag; ret)
<symbol>_range:                 ; emitted after the OK return
mov  x1, #77050002              ; ErrInvalidArgument code
mov  x0, #1                     ; RESULT_ERR_TAG
adrp/add x2, _mfb_str_error_invalid_argument
ret
```

**Open Decision resolved: raise `ErrInvalidArgument`.** The key finding that made
this cheap: `emit_runtime_helper_call` (the non-raw runtime-helper call site)
*always* emits `cmp tag, OK; b.eq ok; <propagate>` regardless of the helper's
"cannot fail" documentation. So returning the ERR tag auto-propagates up through
`__datetime_offsetAt` / `__datetime_toLocal` with **zero `datetime_package.mfb`
changes** — the fallibility ripple the doc feared does not exist. `nowNanos` /
`monotonicNanos` and the frame layout constants are byte-identical.

Runtime proof (macOS 15 / aarch64, host TZ `Pacific/Honolulu`):

- `datetime::localOffset(9223372036854775807)` as the first datetime call now
  raises `ErrInvalidArgument`, caught by a `TRAP` → `outOfRange invalidArg=TRUE`
  on three consecutive runs (previously a varying ASLR stack address).
- In-range calls unchanged and run-stable: `localOffset(0)` and
  `localOffset(9000000000000)` return the real host offset, no trap.
- Downstream `datetime::toLocal(instant(maxint))` propagates the same
  `ErrInvalidArgument` (proves the ripple works without package edits).

Files changed:

- `src/target/shared/code/datetime.rs` — the branch + failure tail (the fix).
- `src/target/shared/runtime/datetime_specs.rs` — header comment (localOffset can
  now fail).
- `src/docs/man/builtins/datetime/localOffset.md` — `## Errors` now lists
  `ErrInvalidArgument`.
- `src/docs/spec/stdlib/02_datetime.md`, `src/docs/spec/memory/07_runtime-helper-abi.md`
  — corrected the "infallible" claims for `localOffset`.
- `tests/rt-behavior/datetime/func_datetime_localOffset_valid/**` — new runtime
  test (out-of-range trap as first call, in-range determinism guards, downstream
  propagation). Fails pre-fix (garbage value, no trap → golden mismatch), passes
  post-fix.
- `tests/syntax/datetime/func_datetime_localOffset_invalid/**` — new compile-time
  test (wrong arity 0 and 2, wrong arg type).

Codegen goldens: no `.ncode`/`.nplan`/`.nir`/`.mir` golden shifts (no datetime
fixture carries native goldens; `.ir` is front-end only).

## Summary

A one-instruction omission — the discarded `localtime_r` return — turns a documented
infallible function into an uninitialized-stack read that hands a live stack pointer
to unprivileged MFBASIC code. The fix itself is a `cbz` and a failure tail; the real
engineering risk is the Open Decision, because making `localOffset` fallible ripples
into `datetime::offsetAt` and `toLocal`. The `nowNanos`/`monotonicNanos` arms and all
in-range behavior stay untouched.
