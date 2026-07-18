# bug-296: x86-64 LINK native calls pass integer args 7–8 in rax/rbp; the SysV C callee expects them on the stack

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (ABI / platform divergence)

Status: Open
Regression Test: tests/ (new) — a LINK with 7 integer ABI slots either rejects on x86 or passes args 7+ on the stack per SysV

The x86 backend's `CALL_ARGS` register list is `[…, "rax", "rbp"]` for slots 6–7,
documented as an INTERNAL extension justified by "libc calls never exceed 6 integer
args". But the LINK thunk stages one `%argN` per integer ABI slot (capped only at 8)
and calls the dlopened C function via `blr` — an external SysV callee, which takes
its 7th+ integer arguments from the *stack*, not rax/rbp. Nothing in
manifest/rules/thunk rejects a 7- or 8-integer-slot `ABI (...)`; on x86 the 7th
argument lands in rax (and al is left as staged garbage for variadic callees) and
the 8th in rbp, so the callee reads whatever is on its stack. aarch64/riscv64 (8
argument registers) are correct, making this a silent x86-only wrong-arguments call
with no diagnostic.

The single correct behavior a fix produces: a LINK native function with ≥7 integer
ABI slots on an x86 target either passes those arguments per the SysV C ABI (stack
for args 7+) or is rejected at build time with a clear diagnostic — never called
with garbage in rax/rbp.

References:

- `bugs/completed-bugs/bug-08-parameter-count-limit.md` (>8 params for
  MFBASIC-internal calls; explicitly kept the ≤8-register model for LINK prologues).
- Found during goal-06 review of `src/arch/x86_64/select.rs`.

## Failing Reproduction

A `LINK` block declaring a native function with ≥7 non-float ABI slots (real APIs:
`XCreateWindow`, several ALSA/ffmpeg entry points), built for linux-x86_64.

- Observed: the native call receives garbage trailing arguments (args 7/8 in
  rax/rbp); no diagnostic.
- Expected: correct SysV argument passing, or a build-time rejection.

## Root Cause

`src/arch/x86_64/select.rs:57-67` (`CALL_ARGS` = `[…, "rax", "rbp"]`) combined with
`src/target/shared/code/link_thunk.rs:673-698`, which stages up to `%arg7` and calls
via `blr` an external SysV function that expects args 7+ on the stack. The internal
6-register assumption is valid for the compiler's own libc helper calls but not for
external LINK callees.

## Goal

- Either reject >6 integer slots (and >8 FP) for LINK on x86 targets with a
  `NATIVE_*` diagnostic (loud, minimal), or teach the thunk/select to place
  external-call integer args 7+ on the stack per SysV.

### Non-goals (must NOT change)

- The internal 6-register model for the compiler's own libc helper calls.
- aarch64/riscv64 LINK argument passing (correct).

## Blast Radius

- `select.rs:CALL_ARGS` (for external calls) + `link_thunk.rs` staging — fixed here.
- Internal helper calls that legitimately use the rax/rbp extension — must be left
  unchanged; distinguish external LINK calls from internal ones during the fix.

## Fix Design

Full SysV stack-arg support for external
calls is the complete fix but larger. Rejected: silently keeping rax/rbp — it
miscompiles real APIs.

## Phases

### Phase 1 — failing test + audit
- [ ] Test a 7-integer-slot LINK on x86; audit whether any bundled binding hits it.
### Phase 2 — the fix
- [ ] Diagnostic (interim) and/or SysV stack-arg staging for external calls.
### Phase 3 — validation
- [ ] Full suite green; aarch64/riscv64 unaffected; x86 rejects or correctly passes.

## Validation Plan

- Regression: 7-slot LINK test (reject or correct-pass).
- Doc sync: note the x86 LINK integer-slot limit in language/17_native-libraries.md.

## Summary

The internal 6-integer-register shortcut leaks into external SysV LINK calls on x86,
miscompiling ≥7-arg native functions. A build-time rejection is the low-risk
immediate fix; full stack-arg support is the complete one.
