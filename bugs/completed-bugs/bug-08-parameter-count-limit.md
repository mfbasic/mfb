# MFBASIC bug-08: 8-parameter limit — native backends pass args only in registers, no stack spill

Last updated: 2026-07-07
Effort: large (3h–1d)

> **STATUS: IMPLEMENTED** (2026-07-07). A callable may now take any number of
> parameters on both backends. Verified: 12-/10-/9-parameter integer, string,
> float, `SUB`, and recursion cases run correctly on macOS aarch64, Linux aarch64
> glibc (box 2223), and Linux x86_64 musl (box 2227); the x86_64 glibc offsets are
> disassembly-verified (`incoming k` at `[sp + frame + 8 + k*8]`, outgoing at the
> frame bottom). `scripts/artifact-gate.sh` reports 0 diffs across 907 goldens —
> every ≤8-argument call is byte-identical. All 2311 unit tests green.
>
> **Design refinement vs. the sketch below:** the caller reserves its outgoing
> tail *statically* at the frame bottom (no runtime `sub sp`/`add sp` bracket),
> which avoids the frame-slot-shift hazard a runtime bracket would create. The
> incoming/outgoing offsets are expressed with sentinel bases
> (`incoming_args`/`outgoing_args`) that survive selection + register allocation
> and are resolved to concrete `sp`-relative accesses in `finalize_frame` once the
> frame size is known. Only ONE callee prologue site
> (`function_lowering.rs::lower_function`) needed the stack-parameter path; the
> runtime-helper / `LINK` / builtin-wrapper prologues keep the ≤8 register model
> (they never exceed the register window).

Any `FUNC`/`SUB` (and any internal call) is capped at **8 parameters**. A 9th
parameter is a hard codegen error, not a type-system rule: the native code plan
assigns arguments strictly by position to registers `x0`–`x7` and has **no
stack-argument path**. This is fine for a v1/MVP but will choke user land once
auto-generated code, wide configuration records, or complex native bindings
(`LINK`) push past 8 arguments. The single correct behavior a fix produces: a
callable with N > 8 parameters lowers and runs correctly on **every** target
(aarch64 + x86_64, glibc/musl/macOS), passing arguments 9..N on the stack per a
defined ABI, with observable behavior identical to an 8-or-fewer-parameter call.

It complements:

- `./mfb spec memory 06_native-calling-convention` (the argument-passing
  contract; canonical spec under `src/docs/spec/memory/**`)
- `./mfb spec language 06_functions` (the documented parameter-count limit)

## 1. Goal

- A `FUNC`/`SUB` declared with **more than 8 parameters** compiles, lowers, and
  runs correctly on aarch64 and x86_64 (glibc, musl, macOS). Arguments beyond the
  register set are passed on the stack per a defined, documented calling
  convention; the callee reads them from the correct frame offsets.
- Both the **declaration site** (function prologue param binding) and every
  **call site** (internal calls) handle N > 8 arguments — currently both fail at
  `abi::argument_register`.
- The documented limit is lifted from the spec once the fix lands (or replaced by
  a new, far-higher documented bound if one is deliberately chosen).

### Non-goals (explicit constraints)

- **No change to the ≤8-parameter ABI.** Calls with 8 or fewer arguments must
  keep passing them in `x0`–`x7` (x86_64: `rdi, rsi, rdx, rcx, r8, r9, rax, rbp`)
  with **byte-identical native code** — the entire existing golden corpus for
  ≤8-arg calls must not shift. Only 9+-arg calls gain new codegen.
- **No language-surface change.** No new syntax; `FUNC`/`SUB` param lists are
  already unbounded in the grammar — this is purely a codegen capability.
- **No change to value/copy/move/freeze semantics** or record/closure layout.
  Each argument still occupies one 8-byte slot (raw bits for `Float`/`Fixed`).
- This is **not** an adoption of AAPCS64 / SysV argument classification. The
  existing convention is a custom, non-AAPCS64 ABI (every scalar is one 8-byte GP
  slot); the stack-argument extension must follow the same simple model.

## 2. Current State

The 8-parameter cap has a single enforcement point:

`argument_register` (`src/arch/aarch64/abi.rs:6-14`):

```rust
pub(crate) fn argument_register(index: usize) -> Result<String, String> {
    if index < 8 {
        Ok(format!("x{index}"))
    } else {
        Err(format!(
            "aarch64 code plan cannot pass argument {index}; stack arguments are not implemented"
        ))
    }
}
```

It is called at two lowering sites, both of which fail for a 9th argument:

- **Function prologue / param binding** — `lower_function`
  (`src/target/shared/code/function_lowering.rs:574`): each parameter `index` is
  bound to `abi::argument_register(index)?`, so a function *declared* with > 8
  params fails here.
- **Call sites** — `emit_prepared_call_args`
  (`src/target/shared/code/builder_emit_helpers.rs:28-53`): each argument is
  lowered into a GPR, spilled to a stack *marshalling* slot, reloaded, then moved
  into `abi::argument_register(index)?`. The final destination is always a
  register `x0..x7`; there is **no path that places argument N ≥ 8 on the stack
  as a call argument**. A call with a 9th argument fails here.

The shared code plan is authored in aarch64 mnemonics
(`src/target/shared/code/mod.rs:4` — `use crate::arch::aarch64::{abi, ...}`);
both backends consume the same neutral plan. So the cap is architecture-agnostic:

- **x86_64** re-lowers the `x0..x7` argument registers in selection. SysV has only
  6 integer arg registers, so the backend extends the set with two *internal*
  registers to reach 8 (`src/arch/x86_64/select.rs:60-68`):

  ```rust
  const CALL_ARGS: &[&str] = &["rdi", "rsi", "rdx", "rcx", "r8", "r9", "rax", "rbp"];
  ```

  x86_64 accommodates the full 8 but has no stack-argument path either; the > 8
  rejection still happens upstream in the shared plan.

The limit is documented (to be lifted by this fix):

- `src/docs/spec/memory/06_native-calling-convention.md:12-22` — "There are no
  stack arguments: requesting argument index ≥ 8 is a hard codegen error … A
  native function with more than 8 parameters cannot be lowered."
- `src/docs/spec/language/06_functions.md:19-24` — "a callable may take at most 8
  parameters; a 9th is rejected at code-plan time … This is an implementation
  limit, not a type-system rule."

Unit test pinning the boundary: `src/arch/aarch64/abi.rs:809-811`
(`argument_register(7) == "x7"`, `argument_register(8).is_err()`).

## 3. Design Overview

Extend the custom calling convention with a **stack-argument tail**: arguments
0..7 stay in registers exactly as today; arguments 8..N are passed on the stack in
a defined order, and the callee prologue binds param slots 8..N to the
corresponding incoming frame offsets. The correctness risk concentrates in three
places, each of which must keep ≤8-arg output byte-identical:

1. **ABI definition.** Decide the stack-argument layout (order, alignment, who
   owns cleanup) for the custom convention on each backend. On aarch64 this is
   caller-pushed args above the callee frame; on x86_64 the SysV-style stack tail
   sits above the return address. This is the load-bearing decision — see Open
   Decisions.
2. **Call-site emission** (`emit_prepared_call_args`): for indices ≥ 8, instead of
   `abi::argument_register(index)?`, emit a store to the outgoing-argument area of
   the caller frame at the ABI-defined offset. Registers 0..7 unchanged. Requires
   reserving/sizing the outgoing-arg area in frame layout when any call in the
   function has > 8 args.
3. **Prologue binding** (`lower_function`): for param indices ≥ 8, bind the param
   to an *incoming* stack slot (load from the caller-established offset) instead of
   a register. The register-allocator/frame machinery must treat these as
   pre-spilled incoming values.

`abi::argument_register` becomes `abi::argument_location(index) -> Reg | StackSlot`
(or a sibling `abi::argument_stack_offset(index)`), and both backends grow a
stack-argument encoding path. The x86_64 `CALL_ARGS` "internal register" hack
(`rax`/`rbp` for args 7/8) stays for the 8-register window; args 9+ go to the
stack.

## Layout / ABI Impact

- `mfb spec memory 06_native-calling-convention` gains a **stack-argument tail**
  section (order, alignment, cleanup ownership) and drops the "cannot be lowered"
  language. `mfb spec language 06_functions` drops (or greatly raises) the
  parameter-count limit note.
- **No record/closure/value layout change** — each argument is still one 8-byte
  slot; copy/transfer/freeze output is unaffected.
- **Native code output:** ≤8-arg calls **must stay byte-identical** (the golden
  corpus is the guardrail). Only functions that *declare* or *call* with > 8 args
  emit new code. Golden regeneration is limited to new > 8-arg test fixtures;
  existing goldens must show **zero diff** (verify via `scripts/artifact-gate.sh`).

## Phases

### Phase 1 — ABI decision + failing repro (no codegen change)

Nail the stack-argument convention on paper and lock in a reproduction before
touching codegen.

- [ ] Write a failing repro fixture: a `FUNC` with 9 (and one with, say, 12)
  parameters that sums/returns them, under `tests/rt-behavior/**` (runtime proof)
  and an `_invalid`→`_valid` flip if any diagnostic test currently asserts the
  9-param rejection. Today it fails with "aarch64 code plan cannot pass argument
  8; stack arguments are not implemented".
- [ ] Decide and document the stack-argument ABI for both backends (order,
  8/16-byte alignment, caller-cleanup, outgoing-arg-area placement in frame
  layout). Record it in this file and draft the `mfb spec memory` update.
- [ ] Confirm no other caller of `argument_register` exists beyond
  `function_lowering.rs:574` and `builder_emit_helpers.rs:46-52` (grep).

Acceptance: the 9-param repro fails today with the documented error on aarch64 and
x86_64; the stack-argument ABI is written into §Current State / §3 with per-backend
offsets. Commit: 4c677252

### Phase 2 — aarch64 stack arguments (call site + prologue)

- [ ] Replace `abi::argument_register(index)?` with a location abstraction
  (`abi::argument_location`) at `builder_emit_helpers.rs` and
  `function_lowering.rs`; keep 0..7 emitting the identical `mov x{index}` so ≤8-arg
  code is unchanged.
- [ ] Reserve/size the outgoing-argument area in aarch64 frame layout when a call
  has > 8 args; emit stores to it for args ≥ 8.
- [ ] Bind param slots ≥ 8 to incoming frame offsets in the prologue.
- [ ] Encoder/unit tests for the new stack-store/load offsets;
  `argument_register` boundary test at `abi.rs:809` updated to reflect the new
  contract.

Acceptance: the 9- and 12-param repro builds and runs correctly on macOS
(host aarch64) and on a Linux aarch64 box (`.ai/remote_systems.md`); ≤8-arg
goldens byte-identical (`scripts/artifact-gate.sh` — 0 diffs).
Commit: 4c677252

### Phase 3 — x86_64 stack arguments

- [ ] Extend x86_64 selection (`src/arch/x86_64/select.rs`,
  `src/target/linux_x86_64/code.rs`) so args ≥ 8 lower to SysV-style stack slots
  above the return address; keep `CALL_ARGS` (`rdi..rbp`) for the 8-register
  window unchanged.
- [ ] Prologue binding of param slots ≥ 8 to incoming stack offsets on x86_64.
- [ ] Stack-alignment maintenance (16-byte at call) with an odd/even number of
  stack args.

Acceptance: the repro runs correctly on x86_64 glibc **and** musl
(`.ai/remote_systems.md`); ≤8-arg x86_64 goldens byte-identical.
Commit: 4c677252

### Phase 4 — spec/doc sync + full acceptance (highest-risk last)

- [ ] Update `src/docs/spec/memory/06_native-calling-convention.md` (stack-arg
  tail) and `src/docs/spec/language/06_functions.md` (drop/raise the limit).
- [ ] Add > 8-param fixtures to the acceptance corpus; regenerate only the new
  goldens; confirm the rest are unchanged.
- [ ] `scripts/test-accept.sh target/debug/mfb target/accept-actual` green;
  cross-validate the repro on every box in `.ai/remote_systems.md`.

Acceptance: acceptance suite green; new > 8-arg tests pass on all target boxes;
existing goldens show only the intended additions; spec no longer states an
8-parameter cap. Commit: 4c677252

## Validation Plan

- Runtime proof: a `FUNC`/`SUB` with 9–16 parameters (mixed `Integer`/`Float`/
  `Fixed`/`String`) that returns a checksum of all args; the observed result must
  match the same computation done with ≤8-arg helpers. Prove on aarch64 + x86_64,
  glibc/musl/macOS.
- Function tests: `tests/rt-behavior/**` valid fixtures for 9/12/16 params; flip
  any existing `_invalid` diagnostic fixture that asserted the 9-param rejection.
- Golden guard: `scripts/artifact-gate.sh` must report **0 diffs** on all
  pre-existing (≤8-arg) goldens.
- Doc sync: `mfb spec memory 06_native-calling-convention`,
  `mfb spec language 06_functions`.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Stack-argument ordering & cleanup** — caller pushes args 8..N in
  ascending-index order above the callee frame, caller-cleanup (recommended,
  mirrors the simple one-slot-per-arg model) vs. any callee-cleanup scheme
  (rejected: complicates variadic-free custom ABI). (§3)
- **Whether to keep the x86_64 `rax`/`rbp` internal-register hack** for args 7/8
  or move the whole 7+ tail to the stack for backend uniformity. Recommended: keep
  the hack (preserves ≤8-arg goldens) and only spill 9+. (§Current State)
- **Documented cap after the fix** — remove entirely vs. state a high practical
  bound. Recommended: remove the hard cap; note practical frame-size limits. (§4)

## Non-Goals

- AAPCS64 / SysV argument *classification* (struct-by-value splitting, HFA/HVA,
  register+stack straddling). The custom one-slot-per-arg model is retained.
- Variadic functions.
- Any change to the ≤8-argument code path or its goldens.

## Summary

The real engineering risk is defining and encoding a stack-argument tail on both
backends **without shifting any ≤8-arg native output** — the existing golden
corpus is the guardrail. Everything else (value/record/closure layout, copy/move/
freeze semantics, the register window `x0..x7` / `rdi..rbp`) stays untouched. The
fix turns `argument_register` into a location abstraction and grows one
outgoing-arg store path + one incoming-arg bind path per backend, gated behind
new > 8-parameter tests.
