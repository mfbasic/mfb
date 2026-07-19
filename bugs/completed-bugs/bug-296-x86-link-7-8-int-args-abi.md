# bug-296: x86-64 LINK native calls pass integer args 7–8 in rax/rbp; the SysV C callee expects them on the stack

Last updated: 2026-07-17
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (ABI / platform divergence)

Status: Fixed
Regression Test: link_thunk::tests::seven_integer_slots_are_rejected_on_x86_and_accepted_on_aarch64

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

## Resolution — the build-time rejection

The report offered a rejection or full SysV stack-arg staging and called the
rejection the low-risk immediate fix. That is what landed. Stack-argument staging
for external calls is the complete fix and remains open; it is also a codegen change
whose correctness could not be demonstrated from this host (see bug-295 — both x86
boxes were refusing connections), which makes shipping it here the wrong trade.

The distinction the fix turns on is internal-vs-external, exactly as the report
framed it:

- `RegisterModel::external_int_argument_registers()` — a new capability defaulting to
  the neutral `REGISTER_ARGUMENT_COUNT` (8), which aarch64 and riscv64 genuinely
  have. x86-64 overrides it to 6, the real SysV count. The internal 8-register model
  is untouched, so the compiler's own calls still use the `rax`/`rbp` extension that
  is sound for them.
- `lower_link_thunk` counts non-`CDouble` slots and refuses to lower when they exceed
  that number, naming the function and both counts.

Asking the register model rather than sniffing the architecture keeps shared code
free of physical-register names (the plan-34-D invariant that
`shared_lowering_names_no_physical_register` enforces, and which bug-284 already
tripped over once).

### Verified end to end on both targets

Built a real 7-integer-slot `LINK` project twice from the same source:

- `--target linux-x86_64` → `error: native function 'seven' declares 7 integer ABI
  slots, but this target passes only 6 …`
- host aarch64 → builds and links normally, because AAPCS64 really does pass eight.

Reducing to six slots builds on **both** targets, confirming the rejection is
confined to the range that was actually miscompiled rather than to LINK generally.
The unit test reproduces all four of those cases against the two backends directly.

### Audit: nothing in the tree is affected

Swept every `ABI (...)` block in `bindings/`, `src/` and `tests/` for more than six
non-`CDouble` slots. There are none, so no bundled binding or fixture changes
behaviour — which is also why the artifact gate is clean (1171 goldens across 990
tests, 0 diffs).

`language/17_native-libraries.md` now documents the target-dependent integer-slot
limit, including that `CDouble` slots use a separate register bank and do not count
against it.
