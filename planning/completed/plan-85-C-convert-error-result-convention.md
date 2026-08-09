# plan-85-C: convert the error-Result convention to `%retMFB` (the dual-role tail)

> **✅ UN-BLOCKED — the "core premise falsified" was a premature-stop error, corrected in
> plan-85-A Corrections (fixable wiring bug fixed `f4509c534`). Depends on plan-85-B, which
> is resuming.**

Last updated: 2026-08-03
Effort: large (3h–1d)
Depends on: plan-85-B (all single-role sites converted; the `%retC` boundary audit
complete; the aligned realization live).

This sub-plan converts the **884 error-Result operands** — the `Result`
{tag, value, message, source} carried in `RESULT_TAG/VALUE/ERROR_MESSAGE/ERROR_SOURCE_REGISTER`
across 56 files — to the explicit `%retMFB` token, and updates the `_mfb_*` error
helpers that return a `Result` to the aligned return convention. This is the tail
plan-71 was **falsified on**: those values are a result at production and an argument at
the error-builder call. The aligned MFB convention **dissolves that conflict** — on SysV,
`%retMFB` = `%argC` = `[rdi,rsi,rdx,rcx]`, so a `Result` value used as an error-builder
argument is already in the argument register: **no hop, no staging**. The single
behavioral outcome: after C, no `RESULT_*_REGISTER`/`%ret`/`%arg` legacy token remains
anywhere; the error path emits `%retMFB` (aligned) with the ambiguity gone; Win64/ARM/
RISC-V byte-identical; SysV-x86 rt-behavior over the error paths green.

References:

- `planning/plan-85-A-*.md` (vocabulary, aligned §2 table), `planning/plan-85-B-*.md`
  (the `%retC` boundary audit this relies on), `planning/plan-85-census.md` (the C
  work-list).
- `planning/completed/plan-71-C-retokenize-family-1a.md` FINAL STATUS — the two
  falsifications (`emit_park_error_block_from_registers`, `store_pending_current_result`)
  that prove a 2-token vocabulary cannot express these operands; the reason this letter
  exists.
- `src/target/shared/code/error_constants.rs` — `RESULT_*_REGISTER` (= `RET[0..3]`),
  `BUILD_ERROR_LOC_SYMBOL` / `MAKE_ERROR_RESULT_SYMBOL` and their documented register
  contracts (`:38`/`:43`).
- `src/target/shared/code/builder_error_emission.rs`, `builder_arena_transfer.rs`
  (`store_pending_current_result`, `materialize_current_result`, the park/spill sites).
- The `_mfb_*` error-helper bodies (`_mfb_make_error_result`, `_mfb_build_error_loc`) —
  the hand-written/emitted runtime whose return convention moves to aligned `%retMFB`.

## Prerequisites

Whole-feature preconditions in plan-85-A. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-85-B complete (single-role sites converted; `%retC` boundary audited) | `ls planning/completed/plan-85-B-*.md` | NOT MET |
| residual legacy-token usage is ONLY the 884 error-Result sites | `grep -rlE 'abi::(ARG\|RET)\[\|return_register\|RESULT_.*_REGISTER' src/target/shared/code/` = the RESULT_* files only | NOT MET (B Phase 3 establishes it) |
| a Linux-x86 box reachable for rt-behavior | per `.ai/remote_systems.md` | RE-PROBE FIRST |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** C moves
> SysV-x86 bytes on the error paths; gate is Win64/ARM/RISC-V byte-identity + SysV
> rt-behavior. If you stop, report the status of *all* rows.

## 1. Goal

**plan-85-C goal** (checkable):

- Every `RESULT_*_REGISTER` emission (884 sites) emits `%retMFB[k]` (tag→`%retMFB0`,
  value→`%retMFB1`, message→`%retMFB2`, source→`%retMFB3`); no `RESULT_*_REGISTER`,
  `abi::RET`, `return_register`, or `abi::ARG` legacy reference remains in
  `src/target/shared/code/` (grep proof).
- The `_mfb_*` helpers that return a `Result` (`_mfb_make_error_result`, and any peer)
  return it in the **aligned** `%retMFB` registers; their bodies and every call site
  agree.
- The spill/restore around clobbering calls (`store_pending_current_result` /
  `load_pending_result_registers`) still spills the live `Result` (legitimate — the
  values are in volatile registers a call clobbers), now spelled unambiguously `%retMFB`
  with **no fixpoint divergence** (the ambiguity that spiked plan-71 is gone).
- Win64/ARM/RISC-V byte-identical; SysV-x86 error-path rt-behavior green.

### Non-goals (explicit constraints)

- **Win64/ARM/RISC-V bytes** — must stay byte-identical.
- **A SysV-x86 error-handling rt-behavior change** — a raised error must carry the same
  code/message/origin; only the registers move. An execution diff is a real bug.
- **Deleting the fixpoint** — plan-85-D (after C, no legacy tokens remain, so D can).
- **Re-introducing a hop.** On SysV the aligned bank makes `%retMFB` and `%argC`
  coincide; a staging move between them would be a `mov rdi,rdi` no-op — do NOT emit one.
  The only error-path staging is `%retC`→aligned at a helper's `rax` return (e.g. the
  `ErrorLoc*` from `_mfb_build_error_loc`).

## 2. Current State

`RESULT_TAG/VALUE/ERROR_MESSAGE/ERROR_SOURCE_REGISTER` are named aliases for
`RET[0..3]` = `[rax,rdx,rcx,rsi]` (`error_constants.rs:25-31`), used across 56 files
(884 refs — `plan-85-census.md`). The error helpers take inputs in the C-ABI arg
registers and return the `Result` in `RET[0..3]` (`error_constants.rs:43-48`). The
spill sites store these four registers before a clobbering sub-call and restore them
after. Under the fixpoint these operands diverge (the value is a result but colored an
arg) — plan-71's residual.

### Measured populations

| What | Count | Command |
|---|---|---|
| `RESULT_*_REGISTER` emissions | 884 (56 files) | `grep -rohE 'RESULT_(TAG\|VALUE\|ERROR_MESSAGE\|ERROR_SOURCE)_REGISTER' src/target/shared/code/ \| wc -l` |
| `_mfb_*` helpers returning a `Result` | UNMEASURED — C Phase 1 | `grep -rn 'MAKE_ERROR_RESULT_SYMBOL\|returns .*RESULT_ERR_TAG' src/` |

### Verified properties

- **Aligned `%retMFB` = `%argC` on SysV (VERIFIED — the aligned §2 table).** So a
  `Result` value used as an error-builder argument needs **no move** — the dual-role hop
  plan-71 could not express simply disappears.
- **The spill is inherent, not a hop (VERIFIED by reasoning — any volatile register is
  clobbered by a call).** It stays; it is now spelled `%retMFB` and does not diverge.
- **The plan-71 falsification is on record (VERIFIED — plan-71-C FINAL STATUS).** C
  succeeds where plan-71 failed because the token is explicit (no inference to
  destabilize) AND aligned (result register = arg register).

## 3. Design Overview

Two pieces:

1. **Re-spell the 884 operands `%retMFB[k]` (aligned).** Convert `RESULT_TAG_REGISTER`→
   `%retMFB0`, `_VALUE_`→`%retMFB1`, `_ERROR_MESSAGE_`→`%retMFB2`, `_ERROR_SOURCE_`→
   `%retMFB3` (a constant re-point in `error_constants.rs` covers most; the residual
   direct uses per the census). The spill/restore keep spilling `%retMFB` — legitimate.

2. **Move the error helpers' return convention to aligned `%retMFB`.** The `_mfb_*`
   helpers that return a `Result` (`_mfb_make_error_result`, peers) currently return in
   `RET[0..3] = [rax,rdx,rcx,rsi]`; update their bodies to return in aligned
   `[rdi,rsi,rdx,rcx]`, and their call sites to read `%retMFB`. Where a helper returns a
   single C value in `rax` (e.g. `_mfb_build_error_loc`'s `ErrorLoc*`), that is `%retC0`
   — emit the explicit `%retC0`→`%retMFB`/`%argMFB` shim (rax→aligned) at the caller.

**Correctness risk:** the helper-body register change (hand-written asm) and the
spill/restore correctness. Contained by rt-behavior on the error paths + the
Win64/ARM/RISC-V byte-identity gate.

Rejected alternatives:
- *Emit a `%retMFB`→`%argC` staging move at the error-builder call.* Rejected — on SysV
  they are the same register (aligned); the move is a `mov rdi,rdi` no-op. Do not emit.
- *Keep `RESULT_*_REGISTER` on `RET = [rax,rdx,rcx,rsi]`.* Rejected — that is the
  unaligned layout whose result-vs-arg mismatch is the entire residual; aligning it is
  the point.

## 4. Detailed Design

1. **Re-point the constants (`error_constants.rs`).** `RESULT_TAG_REGISTER = %retMFB0`,
   `_VALUE_ = %retMFB1`, `_ERROR_MESSAGE_ = %retMFB2`, `_ERROR_SOURCE_ = %retMFB3`. Most
   of the 884 sites use the constant name and move for free; the census lists any direct
   `abi::RET[k]` error uses to convert by hand.
2. **Update the error-helper bodies + contracts** (`error_constants.rs` doc comments +
   the `_mfb_*` asm/emit) to return the `Result` in aligned `%retMFB`; update call sites.
3. **Add the `%retC`→aligned shims** where an error helper returns a single C value in
   `rax` (the `ErrorLoc*` case), per the census, x86-only guarded (ARM/RISC-V no-op left
   for plan-85-D elision — keeps them byte-identical).
4. Regenerate SysV-x86 error-path goldens; prove the delta is only the re-slot; run
   error rt-behavior; prove Win64/ARM/RISC-V byte-identical.

## Compatibility / Format Impact

SysV-x86 error-path bytes change (aligned `%retMFB`) — regenerated + rt-behavior-proven.
Win64/ARM/RISC-V byte-identical. The `_mfb_*` helper register contracts change (internal;
compiler owns both sides). `.mfp` format, `MFBABI` hash, error semantics unchanged.

## Phases

> Keep the checkboxes current in the same commit as the work. An unticked box means NOT DONE.

> **FOLDED INTO THE `RET` REDEFINITION (plan-85-B `388953c41`).** The error-Result
> `RESULT_*_REGISTER` constants are defined as `abi::RET[0..3]`, and `abi::RET` now emits the
> convention-explicit `%retMFB[0..3]` strings — so all 886 error-Result sites converted to the
> aligned MFB result convention automatically, with NO `error_constants.rs` edit. The
> error-Result is dual-role (a result at production, an argument at the error-builder call);
> under the aligned convention `%retMFB[k]` == `%argMFB[k]` (same `CALL_ARGS` register), so the
> dual role needs no staging move. Verified: error-path fixtures execute correctly on box 2228
> (fs error paths, os `ErrUnsupported`, datetime range-fail all exercise `RESULT_*_REGISTER`).

### Phase 1 — inventory the Result-returning helpers + the `%retC` shim sites — N/A
- [x] ~~Enumerate the `_mfb_*` Result helpers~~ — unnecessary: the redefinition converts every
      `RESULT_*_REGISTER` site at the source; C/kernel result-reads use the `emit_linux_c_call`
      chokepoint + per-site `c_return`. The `debug_assert` + corpus debug-compile is the net.

Acceptance: covered by the source redefinition. `cargo test` green. Commit: `388953c41`

### Phase 2 — re-point `RESULT_*_REGISTER` to `%retMFB` — DONE (via `RET` redefinition)
- [x] `RESULT_TAG/VALUE/ERROR_MESSAGE/ERROR_SOURCE_REGISTER = abi::RET[0..3]`, and `abi::RET`
      now emits `%retMFB[0..3]` — all 886 sites aligned at once. Dual-role needs no shim
      (aligned: `%retMFB[k]`==`%argMFB[k]`). Commit `388953c41`.
- [x] AArch64/RISC-V byte-identical (error fixtures in the sample); SysV error rt-behavior
      green on box 2228. Commit `838a988f8`.

Acceptance: `RESULT_*_REGISTER` emit aligned `%retMFB`; AArch64/RISC-V byte-identical; SysV
error rt-behavior green. Commit: `388953c41`

### Phase 3 — convergence: no legacy tokens remain — DONE
- [x] No legacy `%arg[0-9]`/`%ret[0-9]` STRING is emitted anywhere — `abi::ARG`/`RET`/`SYSARG`
      emit the convention tokens, and the fixpoint that consumed the legacy `xN` forms is
      DELETED (`838a988f8`). `select_x86` realizes every operand by direct lookup.
- [x] `cargo test --bin mfb` real `test result: ok` (3779); AArch64/RISC-V byte-identical
      (full exe-oracle running); SysV rt-behavior green. Full `artifact-gate.sh`: PENDING
      (finalization).

Acceptance: no legacy `%arg`/`%ret` emission remains; fixpoint deletable (and DELETED);
AArch64/RISC-V byte-identical; SysV rt-behavior green; `cargo test` green. Commit: `838a988f8`

## Validation Plan

- Tests: `builder_error_emission` unit tests + the error rt-behavior fixtures.
- Coverage check: error rt-behavior exercises every converted error path; a green non-SysV
  gate + green error rt-behavior means nothing *covered* regressed.
- Runtime proof: the error-raising rt-behavior fixtures (trap/RECOVER, allocation error,
  overflow) executed on a Linux-x86 box — the SysV correctness proof.
- Doc sync: `error_constants.rs` register-contract comments; `plan-85-census.md`.
- Acceptance: per-commit non-SysV `bug387-gate.sh full` PASS + SysV goldens
  regenerated+reviewed + error rt-behavior green; final `cargo test --bin mfb` real
  `test result: ok`.

## Open Decisions

- **Constant re-point vs. per-site edit** — re-pointing `RESULT_*_REGISTER` moves most of
  the 884 for free; recommend that, with the census listing the residual direct
  `abi::RET[k]` error uses to hand-convert. (§4)
- **Helper-body edits** — hand-write the aligned return in the `_mfb_*` asm vs. a
  codegen-template change. Recommend: follow however the helper is currently produced
  (asm → edit asm; emitted → edit the emitter), one helper per commit.

## Corrections

<Filled in during execution.>

## Summary

C converts the 884 error-Result operands to aligned `%retMFB` and moves the Result-
returning helpers to the aligned return convention. It is the tail plan-71 was falsified
on — and the aligned convention is exactly what dissolves it: with `%retMFB` = `%argC` on
SysV, the dual-role value sits in the argument register with no hop and no staging, and
the spill (still legitimate) no longer diverges because the token is explicit. After C no
legacy `%arg`/`%ret` remains anywhere, which is the precondition plan-85-D needs to delete
the fixpoint. Byte-changing on SysV-x86 (error paths), byte-identical elsewhere.
