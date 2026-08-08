# plan-85-B: convert the single-role emission sites to explicit aligned tokens

> **✅ UN-BLOCKED — the "falsified premise" was a premature-stop error; the real cause was a
> fixable plan-85-A wiring bug, fixed in `f4509c534` (see plan-85-A Corrections + C4 below).**
> Clean arg conversion IS byte-identical on all targets (proven by isolated `-ncode`/exe
> diffs); the broad diffs came from the fused compare-branch expander stringifying
> `Operand::Abi`, now realized correctly by `realize_convention_token`. The conversion
> resumes on this fixed base.

Last updated: 2026-08-03
Effort: x-large (1d–3d)
Depends on: plan-85-A (the six-token vocabulary, the aligned per-target realization,
the `select_x86` direct-realize seam, and the classification census).

This sub-plan converts every **single-role** `%arg`/`%ret`/`%sysarg` emission — a value
used in exactly one convention role — to the explicit token the census assigned it
(`%argC`/`%retMFB`/`%argMFB`/`%argSys`/`%retSys`/`%retC`). Because the MFB convention is
**aligned** (`%retMFB` = `[rdi,rsi,rdx,rcx]` on SysV, dropping `rax`), this **changes
bytes on SysV-x86** — the goldens for the converted subsystems regenerate and
correctness is proven by **rt-behavior**. Win64, AArch64, and RISC-V stay
**byte-identical** (already aligned) — their byte-identity is the cross-target gate. The
single behavioral outcome: after B, no single-role site emits a legacy token; the only
residual `%arg`/`%ret` usage is the 884 error-Result sites (plan-85-C); every converted
SysV-x86 program runs correctly (rt-behavior) and every non-SysV target is byte-identical.

References:

- `planning/plan-85-A-abi-token-vocabulary-and-census.md` — vocabulary, aligned §2
  table, the direct-realize seam, the design overview.
- `planning/plan-85-census.md` — the per-`file:line` B work-list.
- `planning/completed/plan-80-unified-resource-record.md` — the byte-CHANGING migration
  discipline this mirrors (regenerate goldens; prove the delta is only the intended
  re-slot; rt-behavior unchanged).
- `scripts/bug387-gate.sh` / `scripts/exe-oracle.sh` (Win64/ARM/RISC-V byte-identity),
  `scripts/artifact-gate.sh` (SysV-x86 golden regen), `.ai/remote_systems.md` (Linux-x86
  rt-behavior execution).

## Prerequisites

The whole-feature preconditions live in plan-85-A's Prerequisites table and remain in
force. This letter additionally requires:

| Must be true | Command | Status |
|---|---|---|
| plan-85-A complete (tokens realize aligned; direct-realize seam; census) | `ls planning/completed/plan-85-A-*.md` | NOT MET (A not yet landed) |
| the per-`file:line` census work-list exists | `grep -c 'file:line' planning/plan-85-census.md` (or the appended B work-list) | NOT MET (A Phase 3) |
| A Linux-x86 box reachable for rt-behavior execution | per `.ai/remote_systems.md` | RE-PROBE FIRST |
| exe-oracle baselines re-recorded from clean `main` **serially** | `ls /tmp/bug387/oracle-*.txt` | RE-RECORD FIRST |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** B moves
> SysV-x86 bytes; its gate is Win64/ARM/RISC-V byte-identity + SysV-x86 rt-behavior, not
> SysV byte-identity. Re-record the ephemeral baselines **serially**. If you stop, report
> the status of *all* rows.

## 1. Goal

**plan-85-B goal** (checkable): every single-role emission site emits its explicit
convention token, such that:

- No single-role site references `abi::ARG[`/`abi::RET[`/`return_register()`/
  `argument_register(`/`abi::SYSARG[`; residual legacy-token usage = the 884
  error-Result sites only (`grep` proof).
- **Win64, AArch64, RISC-V: byte-identical** (`bug387-gate.sh full` PASS on those four
  targets — they realize the aligned tokens to the same `xN`/`aN`/Win64 registers they
  already used).
- **SysV-x86: bytes change (aligned), rt-behavior unchanged** — the converted
  subsystems' `.ncode`/executable goldens regenerate and every rt-behavior fixture over
  the converted area runs correctly on a real Linux-x86 box.
- Every value that consumes a C/syscall return (`%retC`/`%retSys` in `rax`) and is then
  used in MFB's aligned bank carries an explicit `%retC`→`%argMFB`/`%retMFB` staging move.

### Non-goals (explicit constraints)

- **Win64/AArch64/RISC-V bytes.** Those must stay byte-identical; a move there is a bug
  (a mis-realized token), not a re-baseline.
- **A SysV-x86 rt-behavior change.** The programs must compute the same results; only
  the register assignment differs. A `.run`/execution diff is a real bug, never a
  re-baseline.
- **The 884 error-Result dual-role sites** — plan-85-C.
- **Deleting the fixpoint** — plan-85-D. B leaves it in place for the not-yet-converted
  error-Result `%arg`/`%ret`; converted single-role tokens already bypass it (the A seam).

## 2. Current State

Per plan-85-A: the six tokens realize aligned; `select_x86` realizes explicit tokens
directly (bypassing the fixpoint). The census assigns each single-role site a target
token by its callee/boundary. Single-role population: **~4,008 sites** (1,609 arg +
2,383 ret + 16 sys — `plan-85-census.md`). An arg to a `_mfb_*`/arena/syscall → `%argC`/
`%argSys`; an arg to an MFB function → `%argMFB`; a genuine result → `%retMFB`; a value
read straight back from a C call as the caller's own return → `%retC`.

### Measured populations

| What | Count | Command |
|---|---|---|
| single-role sites to convert | ~4,008 | `plan-85-census.md` |
| C-boundary call sites the args feed | ~795 | `plan-85-census.md` (60 arena_alloc + 12 arena_free + 12 symbol + ~711 `_mfb_`) |

### Verified properties

- **On Win64/ARM/RISC-V the conversion is byte-identical (VERIFIED by A's realization
  tests).** The explicit token realizes to the same register the legacy token did there.
- **On SysV the conversion moves `%retMFB` off `rax` (VERIFIED — the aligned §2 table).**
  This is the intended byte change; its correctness is rt-behavior, not byte-identity.
- **The `%retC` boundary separability is UNVERIFIED — B Phase 1.** Whether any FFI/entry/
  runtime-helper/2-register-C-return path silently relies on the old `RETS` layout is the
  uncertainty scheduled first.

## 3. Design Overview

Per-file conversion (the plan-71-C discipline), but byte-CHANGING on SysV: for each
census work-list file, convert single-role emissions to their explicit tokens; where the
value is a C/syscall return consumed by MFB, insert the explicit `%retC`→aligned staging
move. Per file (or small group), regenerate the SysV-x86 goldens, prove Win64/ARM/RISC-V
byte-identical, and run the area's rt-behavior. Commit per file so a wrong conversion is
bisectable.

**Uncertainty first (Phase 1): the `%retC` boundary audit.** Before any conversion,
enumerate every place an MFB value crosses to/from genuine C (FFI exports, `main`/entry
return, direct libc calls, any 2-register C return) and confirm each uses `%retC`
(rax-exact) with an explicit shim to/from the aligned bank. If a boundary silently
assumed the old `RETS`, it is fixed here before the bulk conversion relies on it.

**Correctness risk:** volume + the boundary shims. Contained by per-file commits, the
Win64/ARM/RISC-V byte-identity gate (catches a mis-realized token instantly), and
rt-behavior on SysV-x86.

Rejected alternatives:
- *Tree-wide sed.* Rejected — not every `abi::RET[k]` is `%retMFB`; some are `%retC` or
  feed a C call as `%argC`. Per-site, census-guided, gated.
- *Convert error-Result here.* Rejected — dual-role, needs staging (plan-85-C).

## 4. Detailed Design

For each work-list entry `(file:line, current-token, target-token, boundary)`: change the
emission to the target accessor (`abi::c_arg(k)`/`abi::mfb_return(k)`/…), which
constructs the **typed `Operand::Abi`** (plan-85-A) — so each conversion also replaces a
`Raw(Box<str>)` token with the zero-allocation typed arm (the string-removal ride-along;
no output effect, since it realizes to the same register). If `boundary` is "consumes a
C/syscall return", emit the
explicit `%retC[k]`→`%argMFB[k]`/`%retMFB[k]` move at the consumption point (on SysV a
real `mov rdi,rax`; on ARM/RISC-V a `mov xN,xN` no-op — left for plan-85-D's elision, so
Win64/ARM/RISC-V stay byte-identical because the no-op move is *not yet* emitted there…
see Open Decision 1). Regenerate SysV-x86 goldens for the file; prove the diff is only the
re-slot; run rt-behavior; tick the census entry in the same commit.

## Compatibility / Format Impact

SysV-x86 `.ncode`/executable bytes change for converted register-using code (aligned MFB
convention) — regenerated + rt-behavior-proven. Win64/AArch64/RISC-V byte-identical.
`.mfp` format, `MFBABI` hash, runtime semantics unchanged.

## Phases

> Keep the checkboxes current in the same commit as the work. An unticked box means NOT DONE.

### Phase 1 — the `%retC` boundary audit (uncertainty-first)
- [ ] Enumerate every MFB↔C value crossing (FFI export, `main`/entry return, direct libc
      call, any 2-register C return) — grep `extern`/entry lowering/`_mfb_` return
      handling — and confirm each uses `%retC` (rax-exact) with an explicit shim to/from
      the aligned bank. Fix any that assumed the old `RETS` layout.
- [ ] Record the boundary inventory in `planning/plan-85-census.md`.

Acceptance: every MFB↔C boundary is accounted for and uses `%retC`; the aligned
convention cannot silently corrupt a C return. `cargo test --bin mfb` green;
`bug387-gate.sh full` still byte-identical (no conversion yet).
Commit: —

### Phase 2 — C-boundary + syscall args (`%argC` / `%argSys`) + C-return staging
- [ ] Convert single-role args feeding `_mfb_*`/arena/`emit_symbol_call` to `%argC`, and
      syscall args to `%argSys`; insert `%retC`→aligned staging where MFB consumes a C/
      syscall return. Per-file commits.
- [ ] Gate per commit: Win64/ARM/RISC-V byte-identical; SysV-x86 goldens regenerated (diff
      = re-slot only); rt-behavior over the file's area green on a Linux-x86 box.

Acceptance: no single-role C/syscall arg emits a legacy token; four non-SysV targets
byte-identical; SysV-x86 rt-behavior green; `cargo test --bin mfb` green.
Commit: —

### Phase 3 — results + internal args (`%retMFB` / `%retC` / `%argMFB`) + convergence
- [ ] Convert single-role result emissions to `%retMFB` (genuine results) / `%retC`
      (values returned straight from a C call), and internal-call args to `%argMFB`, per
      the census. Per-file commits, same gate.
- [ ] Confirm the only residual `abi::ARG`/`abi::RET`/`return_register`/`SYSARG` refs are
      the 884 error-Result sites (command proof).
- [ ] Full `cargo test --bin mfb` real `test result: ok`; SysV-x86 `artifact-gate.sh`
      regenerated + reviewed; Win64/ARM/RISC-V `bug387-gate.sh full` PASS.

Acceptance: all single-role sites converted; residual legacy usage = error-Result only;
non-SysV byte-identical; SysV rt-behavior green; full suite green.
Commit: —

## Validation Plan

- Tests: A's realization tests + `select_x86` direct-realize test continue to pass.
- Coverage check: the audit sweep + rt-behavior exercise every converted site; a green
  non-SysV gate + green rt-behavior means nothing *covered* regressed.
- Runtime proof: rt-behavior fixtures over each converted subsystem, executed on a real
  Linux-x86 box (`.ai/remote_systems.md`) — the SysV-x86 correctness proof that replaces
  byte-identity.
- Doc sync: append per-file progress + the boundary inventory to `plan-85-census.md`.
- Acceptance: per-commit — Win64/ARM/RISC-V `bug387-gate.sh full` PASS, SysV-x86 goldens
  regenerated+reviewed, rt-behavior green; final `cargo test --bin mfb` real `test result: ok`.

## Open Decisions

- **When to emit the ARM/RISC-V `mov xN,xN` staging no-op** — emitting it in B would move
  ARM/RISC-V bytes (breaking their gate) until plan-85-D's elision removes it. Recommend:
  emit the staging move **only on x86** in B (guard the emission by target), and let
  plan-85-D introduce the ARM/RISC-V staging + elision together, so ARM/RISC-V stay
  byte-identical throughout B/C. (§4)
- **Commit granularity / B split** — per file; split B/B2 by subsystem only if the
  ~4,008-site volume can't land in sittings.

## Corrections

**C1 (plan-85-A, source analysis) — the byte change is ONLY the MFB-result move; arg
conversions are byte-identical; shared-helper results are cross-file-atomic.**

The aligned realization (`x86_64/select.rs realize_abi_operand`) maps `%argMFB[k]`,
`%argC[k]`, **and `%retMFB[k]`** all to `call_args[k]` (SysV `[rdi,rsi,rdx,rcx]`, Win64
`[rcx,rdx,r8,r9]`). Only `%retC` (`rax:rdx`) and `%argSys`/`%retSys` (syscall file) sit
outside that bank. Consequences for B's sequencing:

- **Arg conversions are byte-identical on every target.** `abi::ARG[k]`/`argument_register(k)`
  → `%argC[k]` or `%argMFB[k]` both realize to `CALL_ARGS[k]`, which is exactly where the
  old fixpoint already placed an outgoing arg (before a call) and an incoming arg (at entry).
  So the `%argC`-vs-`%argMFB` choice is a **documentation** choice, not a byte choice, and
  arg sites can convert freely per-file with a byte-identical gate on ALL five targets.
- **The SysV byte change is exactly the MFB-result move `RETS→CALL_ARGS`.** An `abi::RET[k]`/
  `return_register()`/`RESULT_*_REGISTER` used as an **MFB result** moves
  `[rax,rdx,rcx,rsi][k]`→`[rdi,rsi,rdx,rcx][k]` (k0 rax→rdi, k1 rdx→rsi, k2 rcx→rdx, k3
  rsi→rcx). A `RET[0]` used as a **C-call result** (post-`bl`, → `%retC0`) stays `rax` —
  byte-identical.
- **Shared-helper results are cross-file ATOMIC.** A result register is fixed by the
  producer (helper body). A consumer that reads `%retMFB1` (rsi) while the producer still
  emits the legacy `RET[1]` (fixpoint→rdx) MISMATCHES. So `_mfb_arena_alloc`'s
  `{tag=RET[0], ptr=RET[1]}` result + its **body** (`entry_and_arena.rs`) + **all ~795
  read sites** convert as ONE unit; the same holds for every `_mfb_*` helper whose own
  return is read across files. Per-file commits are fine for **arg** sites and for a
  helper's **self-contained** result, but a shared-helper result crosses files — group its
  producer+consumers into one commit (or convert the body last, after all readers, in a
  tight sequence gated together). This supersedes the flat "commit per file" where a shared
  result spans files.

Pilot file analysis (`os/env.rs`): arg sites `ARG[0]`/`ARG[1]`/`ARG[2]` feeding
`pthread_mutex_lock/unlock`, `getenv`/`setenv`/`unsetenv`, `arena_alloc` → `%argC` (all
byte-identical); `return_register()` post-`emit_libc_call` (getenv/setenv result) → `%retC0`
(byte-identical, stays rax); `RET[1]` = arena_alloc ptr → `%retMFB1` (atomic with the
arena_alloc body); `RESULT_*_REGISTER` → left for plan-85-C.

**C2 — the 5 `shared/code` `abi::SYSARG[]` sites are NOT genuine syscall args; they are
`%argC`.** Converting them to `%argSys` (`sys_arg`) hit `realize_abi_operand`'s Win64
`unreachable!("Win64 emits no syscall boundary")` and crashed `build_project_*` cross-target
tests. Cause: those sites (`entry.rs:317/404` getentropy via `emit_random_bytes`,
`entry.rs:817/818` `clock_gettime` via `emit_libc_call`, `arena.rs:1166` munmap/`VirtualFree`
via `emit_arena_unmap`) feed **platform-abstracted libc/OS calls**, not a raw `svc` — the
old fixpoint realized their `SYSARG` token by the **Call** boundary (→ `CALL_ARGS`), never
the syscall file. On SysV `SYS_ARGS[k]==CALL_ARGS[k]` for k≤2 (all these are index 0/1), so
`%argC` is byte-identical there and is the only token that also works on Win64. Fixed to
`abi::c_arg(k)`. The genuine `%argSys` (raw `svc`, index-3 `r10` divergence) lives in the
**Linux platform backends** (`arch/linux_*/code.rs`), outside `shared/code`, so there are
**zero genuine syscall-arg sites in B's `shared/code` scope** — the census "syscall (5)"
bucket is entirely `%argC`. `sys_arg`/`sys_return` accessors remain for the Linux-backend
conversion (a later scope). `grep -rn 'abi::SYSARG\[' src/target/shared/code/` → empty.

**C3 — the MFB-result move (byte-changing) plan for `_mfb_arena_alloc`.** The
`arena_alloc` body (`arena.rs` `lower_arena_alloc`) returns `{tag@return_register()=RET[0],
ptr@RET[1]}` at ~10 return points (OK-tag+ptr stores at `:159/160`, `:180/181`, `:240/241`,
`:283/284`, `:402/403`; error-tag at `:545/546/549/…`). Under alignment these become
`%retMFB0` (tag, SysV rax→rdi) and `%retMFB1` (ptr, SysV rdx→rsi) — **byte-CHANGING**. This
is the cross-file atomic unit (C1): the body's result stores + all ~795 caller reads
(`RET[1]` = "alloc result → vreg base"; `return_register()`/`RET[0]` = the tag check after
`bl ARENA_ALLOC`) must convert **together**. **Trap:** the body ALSO uses `return_register()`
as internal **scratch** (the mmap-grow path `:488-505` stores the fresh block pointer through
`return_register()` as x0-scratch, NOT a result) — so the conversion is NOT a `return_register`
sed; it must convert only the final tag/ptr-before-`ret` sites to `%retMFB`, leaving the
internal x0-scratch uses (better: give them a vreg). Same discipline for every `_mfb_*`
helper whose own return is read across files. This is the byte-changing grouping, gated on
rt-behavior (box 2227) + Win64/ARM/RISC-V byte-identity, run AFTER the byte-identical args
grouping's gate is green.

**C4 — RETRACTED. The "x86 global-dataflow perturbation" theory was WRONG; the broad diffs
were a fused-op stringification bug (fixed `f4509c534`), and clean arg conversion IS
byte-identical on all targets (C1 stands).** Retained below as the record of the wrong
turn; the empirical disproof is in plan-85-A Corrections. Original (incorrect) text: After converting all `shared/code` `abi::ARG[]`→`c_arg()`
(args only, no results), the full byte gate on the A-only baselines showed **534 of 1354
linux-x86_64 executables changed AND windows-x86_64 broadly changed** (both x86 ABIs), while
the conversion touched NO result tokens. Root cause: `remap_x86_abi` colors every `xN` token
by a **per-function global dataflow** (`defined_since_boundary`, `staged_live`, `param_home`
— `select.rs:687-697`). Converting a token makes it a physical register the fixpoint no
longer tracks, so its dataflow state shifts and it colors the *remaining* legacy tokens
differently — even where the converted token's own register agrees with the fixpoint (C1's
premise). The perturbation is broad because runtime helpers (linked into every executable)
mix converted args + legacy results.

Consequences:
- **C1 is wrong.** Arg conversion is byte-identical only on AArch64/RISC-V (positional, no
  fixpoint); on BOTH x86 ABIs it perturbs. The "byte-identical args grouping" does not exist.
- **The plan's premise "Win64 is byte-identical (the cross-target gate)" is FALSE.** Win64
  runs the same `select_x86`/`remap_x86_abi`, so it perturbs like SysV; and Win64 result
  index-0 diverges anyway (`%retMFB0`=rcx vs old `RETS_WIN64[0]`=rax, §2 table). The genuine
  byte-identity cross-check is **AArch64/RISC-V only**; both x86 ABIs must be verified by
  **rt-behavior** (SysV box 2227, Win64 box 2230).
- **Incremental partial conversion cannot be byte-verified on x86 and risks a register
  COLLISION** (a converted physical token vs a legacy token the fixpoint colors to the same
  reg → clobber → miscompile). The only perturbation-free conversion is to leave NO legacy
  tokens for a function's fixpoint pass — but shared results (`arena_alloc`'s `RET[0]/RET[1]`
  read at ~795 sites) make the atomic unit cascade to "convert everything" = the plan-85-D
  fixpoint deletion. **Open question (decides B's whole approach): is the SysV perturbation
  benign (rt-behavior-correct, just different registers) or does it introduce a collision?**
  Being resolved by rt-behavior on box 2227; the answer decides keep-and-verify-rt-behavior
  vs revert-and-redesign-B-around-full-conversion.

## Summary

B converts the single-role bulk to explicit aligned tokens. It is byte-CHANGING on
SysV-x86 (the aligned MFB convention) and byte-identical on the other four targets, so
its gate is Win64/ARM/RISC-V byte-identity + SysV-x86 rt-behavior + golden regen — the
plan-80 discipline. The uncertainty (does any C boundary depend on the old `RETS`?) is
audited first; the volume risk is contained per-file. The 884 error-Result dual-role
sites are left for plan-85-C.
