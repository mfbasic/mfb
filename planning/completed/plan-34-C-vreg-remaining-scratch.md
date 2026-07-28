# plan-34-C: Virtualize the remaining scratch, and forbid physical registers outright

Last updated: 2026-07-10
Effort: large (3h–1d) — **split at the Phase 3/4 boundary** (see Open Decisions)
Depends on: nothing hard. Interacts with plan-34-B (its Phase 1 census may reclassify ~800
`"x1"` scratch sites into this plan's scope) and subsumes the root cause of `bug-56`.

> **STATUS: FULLY COMPLETE — ZERO physical registers in shared lowering (2026-07-10).** The
> §2.6 "documented exclusion" was rejected: the MIR is architecture-neutral, so it must name NO
> physical register at all, not even in machine-floor code. Final scan of
> `src/target/shared/code/` production code: **0** physical `x#`/`w#` literals. How the last
> machine-floor holdouts were neutralized (this session, on top of the earlier Phase-1–3 work):
> - **`x20` current-thread → `%thread` token** (`abi::CURRENT_THREAD`): a program-wide PINNED
>   register like `arena_base`. Added `RegisterModel::current_thread()` (AArch64 `x20` / x86-64
>   `rbx` / riscv64 `s2`) and **removed each realization from `INT_ALLOCATABLE`**. This also fixed
>   a LATENT bug the trampoline notes couldn't isolate: `x20`/`rbx`/`s2` were wrongly
>   *allocatable*, so a worker body could color scratch onto the current-thread register. (Only 5
>   host goldens shifted `x20`→`x21`; low blast radius.)
> - **Entry stub + trampoline + panic formatter scratch → `abi::SCRATCH` token pool** (`%scratch0`…
>   `%scratch18`, realized in `abi.rs` to the AArch64 scratch bank, BYTE-IDENTICAL). These are
>   genuinely machine-floor (entry reads argc/argv off raw `sp` pre-`finalize_frame`; trampoline
>   hand-pins registers across the worker + `pthread_*` calls), so their scratch cannot be a `%vN`
>   — but it is now spelled neutrally, not physically. The trampoline was NOT vregged (the prior
>   unisolated x86-cancellation segfault); machine-floor + `%thread` token is the correct shape.
> - **`tls/openssl.rs` `x1` scratch → `%v14`** (the last stray physical in an allocator-run fn).
> - **Phase-5 guard: allowlist REMOVED.** `shared_lowering_names_no_physical_scratch_register`
>   now enforces zero physical scratch everywhere with no exceptions.
>
> Validated: 2555 unit tests green; artifact gate byte-identical except the 5 `x20`-allocation
> goldens (refreshed); runtime on host(aarch64) + x86 box(2227) + riscv box(2229): hello-world,
> thread cancellation (`thread-drop-cleanup`), control-flow, fs-writeAll EINTR, and **live TLS
> handshakes** all correct. Pre-existing riscv thread bug surfaced + filed (bug-86, NOT this plan
> — identical at the 0ba52fee baseline).
>
> Earlier commits (Phase 1–3): §2.5 `e6c4075b`; Phase-3 scratch `1f89339d`/`e56822d9`/`afddaaae`/
> `5b942367`/`77d290c8`; x86 r14 freed `5dfb87ef`; Phase-5 guard `803b6c6f`; spec `0ba52fee`.
> The plan-34-B Phase-4 x86 regression (bug-85) was found + reverted earlier (`a23aee06`).
>
> **One follow-up (not required for this plan's outcome):** the trampoline's `x13`/`x14` *scratch*
> (distinct from the pinned `x20`) could in principle vreg now that x86 has 5 GPRs (r14 freed),
> but a vregged trampoline segfaults x86 cancellation for an unisolated reason (simple threads
> pass, so the frame is sound — the control block just isn't reaching the worker's `is_cancelled`);
> all attempts were reverted. See [[bug-85-x86-arg-staging-revert]] neighbor notes in memory.
>
> **Superseded earlier status:** The two LIVE hazards are fixed; the remaining scratch
> vregging is the multi-session byte-changing grind this plan warns must not be compressed.
>
> **Done + validated:**
> - **Phase 1 census** — complete. Refreshed count: **401** physical `x8`–`x30` string-literal
>   operands in `src/target/shared/code/` (its `crypto_ec/`, `net/`, `tls/` subdirs included).
>   Of these, ~53 are `#[cfg(test)]` fixtures (`mir.rs` ≥ line 124, `peephole.rs` ≥ 335,
>   `regalloc/tests.rs`) — out of scope. The in-scope remainder is classified by disposition in
>   the "Remaining" list below: (a) vreg-able caller-saved `x9`–`x18` (errno cluster + the
>   already-done self-contained scratch), (b) callee-saved-across-call `x20`–`x27` (thread
>   trampoline), (d) syscall/linkage plumbing `x8`/`x16`/`x17`/`x30` (stays), (e) entry-stub
>   special (`entry_and_arena.rs`, likely permanently physical). Class (c) pinned `x19` is the
>   arena base (plan-34-A token). Zero sites unclassified.
> - **bug-56 (Phase 2)** — already fixed before this plan: `emit_link_expr`
>   (`link_thunk.rs:941`) mints vregs (`%v{vreg}`), not `x{base}`; the reproducer
>   `native-link-nested-success-rt` (`SUCCESS_ON status <> 1 AND (status <> 2 AND status <> 3)`)
>   is a committed regression test.
> - **§2.5 closure-env hazard (Phase 2)** — FIXED (`e6c4075b`): `RegisterModel::closure_env()`
>   added; each ISA's realization (AArch64 `x28`, x86 `r13`, riscv `s10`) removed from
>   `INT_ALLOCATABLE`, so the allocator cannot color a body vreg onto it. Byte-identical on the
>   whole corpus (artifact-gate 0 diffs → §2.5 was latent, never observed). Runtime-validated on
>   macos-aarch64 + linux-riscv64 (a new register-pressure closure test + `call-function-value-rt`).
> - **Phase 3, self-contained scratch** — `function_lowering.rs` entry-zeroing → `xzr`
>   (`1f89339d`); `os.rs` executablePath + `fs_helpers_io.rs` write-all-bytes constant scratch →
>   vregs (`e56822d9`). Validated on host + riscv64.
>
> **Remaining (deferred — byte-changing, per-file 4-target runtime validation required):**
> - **Phase 3 errno cluster** (~22 `x9`): `emit_errno` is a platform-trait method that writes
>   errno to `x9`, consumed by `emit_errno_error_mapping` / `emit_fs_path_errno_error_mapping`
>   (fs_helpers.rs) and `emit_eintr_retry` (fs_helpers_io.rs). Refactor = thread a `dst` vreg
>   through `emit_errno` (trait + 4 ISA impls + 2 mocks) and an `errno_reg` param through the two
>   snippets, then a vreg at each of ~24 callers. **Not a live bug**: `emit_errno` does a
>   `bl __errno_location`/`___error` that clobbers all caller-saved regs, so no live vreg can
>   cross the `x9` write — this is Phase-5 purity, not correctness.
> - **`emit_branch_if_ascii_literal` `x12`** (fs_helpers_io.rs:2231, a snippet) — thread a scratch vreg.
> - **Phase 4 thread trampoline** (`runtime_helpers.rs` `x13`/`x14`/`x20` ≈41+40, `runtime_helpers_thread.rs` `x20`×6):
>   `x20` holds the thread record live across `bl`/`svc` (callee-saved-across-call, class b) —
>   **verify against `bug-54` first**; a mis-spill silently corrupts the trampoline. Highest risk.
> - **Phase 4 entry stub** (`entry_and_arena.rs` ≈192): runs before the arena/frame exist (class e);
>   the plan's Open Decision recommends assuming the allocator cannot run here and documenting a
>   physical-register exclusion in `08_program-startup.md`. Likely permanently physical.
> - **`tls/mod.rs`** ≈24 scratch (`x12` and siblings).
> - **Phase 5 guard test** — blocked until the above are vregged or allowlisted.
>
> **Validation constraint discovered this session:** the x86-64 boxes run, but **x86 closures and
> x86 scope-drop already segfault at the pre-plan baseline** (a minimal no-capture lambda and
> `scope-drop-free-rt` both crash on 2227, byte-identical to HEAD `98f25463`) — separate,
> pre-existing x86 bring-up bugs. So the plan's "execute on all four targets" gate is
> unachievable for those paths on x86 regardless of this plan; the linux-aarch64 boxes
> (2222–2226) are also down. Runtime validation this session used macos-aarch64 (host, full
> acceptance) + linux-riscv64 (2229) + x86-64 (2227) where the path is not pre-broken.

`src/target/shared/code/` mints virtual registers at 928 sites and lets the linear-scan
allocator color them. In the same files it also names roughly **383 physical registers**
outright. Some must stay: `x19` is the pinned arena base, `x8`/`x16`/`x17` carry syscall and
linkage plumbing, and `x0`–`x7` are the call boundary (plan-34-B's tokens, uncolorable by
design). The rest is hand-picked scratch — `x9`, `x10`, `x13`, `x20`–`x27` — chosen by a human
counting on their fingers.

That is not a stylistic complaint. `bug-56` is an open, live crash: `emit_link_expr` assigns
scratch by an uncapped index (`x{base}`, rhs at `base+2`, `And`/`Or` rhs at `base+4`, from
`base = 9`), so a legal three-level expression like `SUCCESS_ON r <> 1 AND (r <> 2 AND r <> 3)`
walks the index up through `x17` into **`x19`, the arena base**, clobbers it in a thunk that
never restores it, and corrupts every subsequent allocation program-wide. §2.5 below describes a
second, unfiled instance of the same shape at every closure call site.

The plan's final act is what makes the fix permanent: once every role has a token (plan-34-A,
plan-34-B) and every scratch value is a vreg, **no MIR operand may name a physical register at
all** — an invariant a guard test enforces, in the same spirit as `mir.md` §5's ban on
`smulh`/`rorv`/`adrp` and `ir::verify`'s sole-rejecter role. Under that rule `bug-56` is not
fixed; it is *unrepresentable*.

The single behavioral outcome: **no shared lowering path picks a register by hand; every
non-token operand is a virtual register; `bug-56`'s reproducer runs clean at any expression
depth; and a guard test rejects any physical register name in a shared instruction stream.**

References:

- `planning/bug-56-link-expr-physical-register-escalation.md` — the live crash this plan
  eliminates by construction rather than by capping an index.
- `planning/bug-54-regalloc-callee-saved-scratch-not-saved.md` — a prior allocator defect in
  exactly the callee-saved territory Phase 4 enters.
- `src/docs/spec/memory/07_runtime-helper-abi.md`, `08_program-startup.md`, `09_closures.md`.
- `.ai/compiler.md` § "Native Codegen Register Lifetimes" — *"Treat any value held in a
  caller-saved scratch register as destroyed across the call unless you have proven otherwise
  from the callee's source."*

## 1. Goal

- Shared lowering names no register in `x9`–`x18` or `x20`–`x27`. Those operands are `%vN`.
- `bug-56`'s reproducer — a LINK binding with a right-nested `SUCCESS_ON` expression — compiles
  and runs without corrupting the arena base, at *any* depth (spilling, not escalating, when
  registers run out).
- The closure-call sites (§2.5) cannot allocate a vreg onto the closure-environment register.
- A guard test rejects **any** physical register name in a shared instruction stream, with an
  explicit, documented allowlist for syscall/linkage plumbing.

### Non-goals (explicit constraints)

- **The call boundary stays physical-by-token.** `codegen_utils.rs:~455` states the contract:
  argument and result registers *"stay physical (the allocator never colors them)"*. plan-34-B
  answers that with tokens, not vregs. Coloring a boundary register is wrong.
- **The arena base stays pinned.** `ARENA_STATE_REGISTER` is a program-wide invariant, reserved
  from allocation via `RegisterModel::arena_base()` (`aarch64/regmodel.rs:69-72`). Virtualizing
  it is `bug-56` with extra steps.
- **No change to the fallible-call ABI, the arena layout, or the closure layout.**
- **Emitted bytes WILL change.** Unlike plan-34-A and plan-34-B, this plan cannot use the
  byte-identical gate as its proof: the point is that the allocator picks different registers
  than the human did. Goldens move. That is the defining risk, and it dictates the whole
  Validation Plan.

## 2. Current State

### 2.1 The machinery already exists

`src/target/shared/code/codegen_utils.rs`:

- `:~455 finalize_vreg_helper` — `regalloc::allocate` + `finalize_frame` for a helper body, *"so
  the shared allocator places its registers per-ISA, which is what makes the helpers portable
  (plan-00-G Phase 2)"*.
- `:481 finalize_vreg_body(instructions, reserved)` — the building block; `reserved` is the
  escape hatch for registers the allocator must not touch.
- `:~497 finalize_vreg_body_with_locals(…, local_size)` — same, plus a fixed `sp`-relative
  buffer for a `stat`/`getcwd`/`readdir` struct a syscall fills.

Adoption is real but partial: `vreg_body_with_locals` has 56 call sites, `vreg_body` 41,
`vreg_helper` 17, and `vreg(` 928. Per the project's notes, plan-00-G Phase 2 migrated *"every
vreg-able runtime helper."* **"vreg-able" is load-bearing in that sentence**, and this plan is
about establishing what it excluded and why.

Note also that **every `finalize_vreg_body` call site inspected passes `reserved = &[]`** (e.g.
`os.rs:331,384,476,529,750,868,895,940`). The escape hatch exists and is unused.

### 2.2 What is left

Approximately 383 physical register operands in `src/target/shared/code/` including its
`crypto_ec/`, `net/`, `tls/` subdirectories. Exact counts are Phase 1's job — a top-level-only
glob returns 193 caller-saved uses and misses the subdirectories, so **the numbers below are
shape, not census**:

| register | ≈count | assessment |
|---|---|---|
| `x9` | 85 | caller-saved scratch — **vreg-able** |
| `x13` | 44 | caller-saved scratch — **vreg-able** |
| `x10`, `x11`, `x12`, `x14`, `x15` | ~74 | caller-saved scratch — **vreg-able** |
| `x20`–`x27` | 136 | callee-saved scratch held across calls — vreg-able, higher risk |
| `x8`, `x16`, `x17` | ~36 | syscall number / platform scratch (IP0/IP1) — **stays** |
| `x19` | 3 | `ARENA_STATE_REGISTER` — **stays pinned** (plan-34-A retires the literal) |
| `x28` | 5 | `CLOSURE_ENV_REGISTER` — **plan-34-B's token**, and see §2.5 |

By file, the caller-saved (`x9`–`x18`) uses concentrate in ten places:

| file | uses | vreg mentions |
|---|---|---|
| `entry_and_arena.rs` | 96 | 157 |
| `runtime_helpers.rs` | 41 | 3 |
| `mir.rs` | 25 | 4 |
| `fs_helpers.rs` | 11 | **0** |
| `os.rs` | 5 | 113 |
| `function_lowering.rs` | 4 | 11 |
| `fs_helpers_paths.rs` | 4 | 141 |
| `fs_helpers_io.rs` | 4 | 106 |
| `link_thunk.rs` | 2 | 8 — **and see bug-56** |
| `peephole.rs` | 1 | 0 |

The pattern is informative. The heavily-vregged files (`os.rs`, `fs_helpers_paths.rs`,
`fs_helpers_io.rs`) have a handful of physical stragglers, probably ABI or syscall plumbing and
correctly physical. The problems are `runtime_helpers.rs` (41 physical, 3 vreg), `fs_helpers.rs`
(11 physical, 0 vreg), and `entry_and_arena.rs`.

### 2.3 `link_thunk.rs` — bug-56

`emit_link_expr` (`:916-985`) computes a boolean expression into `x{base}`, recursing rhs at
`base+2` (`:954`) and `base+4` (`:976,981`), from `base = 9` at both call sites (`:512-519`,
`:554-563`), with no cap and no spilling. It walks into `x19`. The thunk is finalized with an
empty stack-slot list, so `x19` is never restored.

This is a hand-rolled register allocator with an off-by-infinity. The fix is to delete it and
call the real one.

### 2.4 `runtime_helpers.rs` — callee-saved values live across calls

`:629-648` loads a thread record into `x20` and calls through `x13`
(`abi::branch_link_register("x13")`), relying on `x20` surviving. That is a *correct* use of a
callee-saved register. Virtualizing it depends on the allocator honoring what
`finalize_vreg_helper`'s doc promises — *"the call clobber model spills any vreg live across a
`bl`/`svc`"* — and `bug-54` (`regalloc-callee-saved-scratch-not-saved`) is a prior defect in
precisely that machinery.

### 2.5 The closure-call sites — bug-56's shape, unfiled

`builder_emit_helpers.rs:205-229`:

```rust
let code_register = self.allocate_register()?;   // :206 — a vreg
let env_register  = self.allocate_register()?;   // :207 — a vreg
…
self.emit(abi::move_register(CLOSURE_ENV_REGISTER, &env_register));  // :223 — writes x28
self.emit(abi::branch_link_register(&code_register));                // :224 — reads code_register
```

`x28` is **allocatable on all three ISAs**: AArch64 `INT_ALLOCATABLE` (`regmodel.rs:101-104`)
ends `"x27", "x28"`; x86 realizes it via `map_scratch_register(28)` to `r13`, which is in
`INT_ALLOCATABLE`; riscv64 to `s10`, likewise. Nothing reserves it — every `finalize_vreg_body`
call site passes `&[]` (§2.1), and unlike `arena_base` there is no `RegisterModel::closure_env()`
accessor and no realization test.

**If linear scan colors `code_register` to `x28`, line 223 overwrites the code pointer with the
environment pointer and line 224 calls through the environment.** `builder_collection_queries.rs:1268-1277`
has the same shape. `x28` is last in the AArch64 allocatable list, so this should require real
register pressure — the layout-sensitive profile this codebase has been bitten by repeatedly.

**This is a hypothesis, not a confirmed defect.** Verified: `x28` is allocatable on all three
ISAs; nothing reserves it; the hardcoded write at `:223` sits between the vreg definition at
`:206` and its use at `:224`. Not verified: whether the allocator can actually reach `x28` at
these sites, or whether some liveness interaction makes it unreachable. Phase 1 must settle it
with a pressure-forcing reproducer. If confirmed, file it as a bug in its own right — it is a
wrong indirect call, not a cleanup.

### 2.6 `entry_and_arena.rs` — the process entry stub

96 caller-saved uses in code that runs *before the arena exists*, with no frame to spill into.
`:47-64` manipulates `argc`/`argv` off the raw stack. Whether the allocator can run here at all
is an open question Phase 1 must answer. The project's notes record a prior incident — an
arg-accepting `main` plus `math::rand` crashed at startup because argc/argv were clobbered by
RNG seeding — so this file has form.

### 2.7 A physical-register list in the ABI

`src/arch/aarch64/abi.rs:4` — `IO_PRINT_CLOBBERS: &[&str] = &["x0","x1","x2","x9","x16"]`. A
hardcoded clobber set naming a scratch register. It must be re-expressed once `x9` is no longer
a name shared code can say.

## 3. Design Overview

Five layers, ordered so the highest-value/lowest-risk work lands first and the entry stub — the
one place the allocator may genuinely not be able to run — lands last or not at all.

1. **Census and feasibility.** Classify all ~383 sites; settle §2.5 with a reproducer; decide
   whether the entry stub can run the allocator. Nothing moves until every site has a class.

2. **`link_thunk.rs` first.** Smallest surface, fixes a live crash, and cleanly demonstrates
   that `finalize_vreg_body` handles an arbitrarily deep expression by spilling. This phase
   alone justifies the plan.

3. **The caller-saved bulk.** `runtime_helpers.rs`, `fs_helpers.rs`, the stragglers. Mint vregs,
   call `finalize_vreg_body`, let the allocator place them.

4. **The callee-saved population and the entry stub.** Highest risk, behind the tests the
   earlier phases build.

5. **The invariant.** A guard test rejecting any physical register name in a shared instruction
   stream. This is what stops the problem from growing back, and it is only *possible* once
   plan-34-A's and plan-34-B's tokens exist — which is why this plan closes the feature.

**Where the correctness risk concentrates**, bluntly: *this plan cannot be validated the way A
and B are.* The byte-identical gate is the safety net for a rename; here byte changes are the
expected outcome, and a wrong allocation looks exactly like a right one until it corrupts memory
at runtime. The gate this plan needs is `.ai/compiler.md`'s completion gate — **execution** on
all four targets — plus leak checks, because the failure modes are `bug-56`'s arena corruption
and `bug-54`'s unsaved callee-saved register, neither of which fails a compile.

**Rejected alternative — cap the index in `emit_link_expr`.** The obvious `bug-56` fix: bound
`base` and error past some depth. It converts memory corruption into an arbitrary compile-time
limit on a legal expression, and leaves the hand-rolled allocator for the next person to walk
off. The allocator exists; use it.

**Rejected alternative — virtualize `x19`/`x28` and let the allocator pin them.** Superficially
uniform. `arena_base` is pinned *across function boundaries* — a program-wide invariant no
linear-scan allocator can express; it stays reserved. `closure_env` is a call-boundary register
and gets a token (plan-34-B), not a vreg.

**Rejected alternative — land before plan-34-B.** Tempting, since B's Phase 1 census will likely
reclassify ~800 `"x1"` scratch sites into this plan. But B's Phase 1 is cheap, lands nothing, and
its output determines this plan's true size. Run B's census first, then decide. Recorded as an
Open Decision in both plans.

## 4. Detailed Design

### 4.1 `emit_link_expr` (bug-56)

Replace the `base`-indexed physical scheme with `CodeBuilder::allocate_register`:

- The node's value, a `Compare`'s rhs, and an `And`/`Or`'s rhs each become a fresh vreg.
- The thunk body is finalized through `finalize_vreg_body(&mut instructions, RESERVED)` (§4.2).
- Depth becomes unbounded: the allocator spills when it runs out of colors, and `finalize_frame`
  sizes the thunk's frame from the allocation outcome. The empty-stack-slot-list finalization
  that `bug-56` identifies as the reason `x19` is never restored disappears.

### 4.2 The `reserved` set

`finalize_vreg_body(instructions, reserved)` takes the escape hatch and every call site passes
`&[]` today. Define it once, and pass it:

```rust
/// Registers the allocator must never color: program-wide invariants only.
/// NOT the call boundary — that is named with %arg/%ret/%sysarg tokens
/// (plan-34-B) and is uncolorable by construction, not by reservation.
const RESERVED: &[&str] = &[abi::ARENA];
```

`closure_env` is deliberately absent: plan-34-B §4.3 gives it a `RegisterModel::closure_env()`
accessor mirroring `arena_base()`, so each ISA excludes its own realization from
`INT_ALLOCATABLE` rather than reserving an AArch64 register number x86 would misread. **If
plan-34-B has not landed when Phase 2 starts, add the closure register to `RESERVED` as an
interim measure and note it as debt** — §2.5's hazard is live either way.

Syscall-number registers (`x8` Linux, `x16` macOS) are set immediately before the `svc` and read
by it; they are part of the syscall ABI and are handled like argument registers, not by
reservation.

### 4.3 `IO_PRINT_CLOBBERS`

`abi.rs:4` names `x9` and `x16`. After Phase 3, `x9` is not a name shared code uses. The clobber
set becomes a property of the call — the allocator's model already treats `bl`/`svc` as
destroying the caller-saved bank (`finalize_vreg_helper`'s doc; `regalloc/analysis.rs:57` —
*"Caller-saved integer registers `x0`–`x17` (clobbered by any call per the PCS)"*). Confirm
`io::print` routes through that model and delete the bespoke list.

### 4.4 The invariant (Phase 5)

A guard test over every shared `CodeInstruction` stream:

- Every register-valued operand is a vreg (`%vN`/`%fN`), a role token (`%arg`/`%ret`/`%sysarg`/
  `%sysnr`/`%sysret`/`%closure_env`), or an invariant token (`sp`, `lr`, `xzr`, `arena_base`).
- Anything else — any `xN`, any `rN`, any `aN` — fails, with an allowlist for the class-(d)
  syscall/linkage plumbing sites, each carrying a written reason.

`format!("x{base}")` then cannot produce a legal operand. That is the difference between fixing
`bug-56` and making it impossible.

## Compatibility / Format Impact

**Emitted machine code changes on all four targets.** `.ncode` goldens across `tests/` need
regeneration, and every regeneration must be justified by inspection rather than accepted
wholesale — per `.ai/compiler.md`: *"Do not assume an acceptance mismatch is a test issue."*

No change to: the `.mfp` package format, the fallible-call ABI, the arena layout, the closure
layout, the diagnostic surface, or the language.

## Phases

### Phase 1 — Census, feasibility, and the §2.5 reproducer (no code change)

Classify every physical register operand; settle two open questions. Lands nothing.

- [ ] Enumerate all physical `x8`–`x30` operands in `src/target/shared/code/` **and its
      `crypto_ec/`, `net/`, `tls/` subdirectories** — the top-level-only count of 193
      caller-saved uses misses them and is not the real number.
- [ ] Class each: (a) vreg-able caller-saved, (b) vreg-able callee-saved-across-a-call,
      (c) pinned (`x19`), (d) syscall/linkage plumbing (`x8`/`x16`/`x17`/`x30`), (e)
      entry-stub-special.
- [ ] **Settle §2.5.** Construct a closure call under enough register pressure to exhaust
      `x8`–`x27` and observe whether `code_register` is colored `x28`. If it is, file the bug
      and fix it in Phase 2 alongside `bug-56` — same shape, same remedy.
- [ ] For `entry_and_arena.rs`: determine whether `finalize_vreg_body` can run before the arena
      and frame exist. If not, record class (e) as permanently physical, with the reason.
- [ ] Read `bug-54` and confirm whether the callee-saved spill path it reports is fixed. Phase 4
      depends on it.

Acceptance: a census table in this document accounting for every site with zero unclassified; a
written yes/no on entry-stub feasibility; a yes/no on §2.5 backed by a reproducer or by a cited
reason it cannot occur. Per the project's "completeness claims need an audit" rule, this is an
exhaustive enumeration, not a sample.
Commit: —

### Phase 2 — `link_thunk.rs` and the closure sites: vreg the hand-rolled allocators

Smallest surface, live crash, and the proof that spilling replaces escalation.

- [ ] Rewrite `emit_link_expr` (`link_thunk.rs:916-985`) to mint vregs instead of `x{base}` /
      `base+2` / `base+4`; drop the `base` argument from both call sites (`:512-519`, `:554-563`).
- [ ] Finalize the thunk through `finalize_vreg_body` with `RESERVED` (§4.2), so the frame is
      sized from the allocation outcome rather than an assumed-empty slot list.
- [ ] If Phase 1 confirmed §2.5: reserve or token-ize the closure-environment register at
      `builder_emit_helpers.rs:205-229` and `builder_collection_queries.rs:1268-1277`.
- [ ] Tests: `tests/rt-behavior/` cases for bug-56's reproducer (`SUCCESS_ON r <> 1 AND (r <> 2
      AND r <> 3)`) and a deeper nest that forces a spill; a pressure-forcing closure call if
      §2.5 was confirmed; `tests/rt-error/` counterparts per `.ai/compiler.md`'s mandatory
      valid/invalid pairing.

Acceptance: bug-56's reproducer **executes** and the arena base is intact afterwards (a
subsequent allocation succeeds) on macos-aarch64, linux-aarch64, and linux-riscv64
(`ssh -p 2229`). A depth-10 expression compiles and runs. `test-accept.sh` green with justified
golden updates.
Commit: —

### Phase 3 — The caller-saved bulk

Mechanical once Phase 2 proves the pattern.

- [ ] `runtime_helpers.rs` (41 physical, 3 vreg) — the largest offender outside the entry stub.
      Exclude the thread-trampoline callee-saved uses (`:629-648`); those are Phase 4.
- [ ] `fs_helpers.rs` (11 physical, 0 vreg).
- [ ] `mir.rs` (25) — confirm these are fused-op expansion / test fixtures rather than lowering;
      if fixtures, they are out of scope and the census should have said so.
- [ ] `function_lowering.rs` (4), `peephole.rs` (1), and the stragglers in `os.rs`,
      `fs_helpers_paths.rs`, `fs_helpers_io.rs` (~13 total) — expect most to be class (d).
- [ ] Re-express `abi.rs:4 IO_PRINT_CLOBBERS` per §4.3.

Acceptance: `tests/acceptance/` **executes** green on macos-aarch64, linux-aarch64, and
linux-riscv64; goldens regenerated with each diff inspected. Memory-leak check clean — this
touches `fs_helpers`, and `bug-63` is a prior fs error-path resource-leak cluster.
Commit: —

### Phase 4 — Callee-saved scratch and the entry stub (highest-risk work last)

Only after Phase 3, and only where Phase 1 said the allocator can run.

- [ ] `runtime_helpers.rs:629-648` and the rest of the `x20`–`x27` population (~136 uses):
      convert to vregs and rely on the allocator's call-clobber model to spill values live across
      `bl`/`svc`. **Verify against `bug-54` first** — an unsaved callee-saved register here
      silently corrupts the thread trampoline.
- [ ] `entry_and_arena.rs` (96 uses): only if Phase 1 concluded the allocator can run before the
      arena and frame exist. Otherwise document class (e) as permanently physical in
      `src/docs/spec/memory/08_program-startup.md` and close this task as scoped out — a
      documented, reasoned exclusion, not a silent one.

Acceptance: full `tests/acceptance/` execution on all four targets, including thread tests
(`thread::` trampoline) and startup tests with an arg-accepting `main` + `math::rand` (the prior
argc/argv-clobber incident). `test-accept.sh` green.
Commit: —

### Phase 5 — Forbid physical registers in shared streams

The invariant. Requires plan-34-A and plan-34-B to have landed.

- [ ] Implement the guard test of §4.4 over every shared `CodeInstruction` stream.
- [ ] Write the class-(d) allowlist explicitly, one line of justification per entry. An
      allowlist with an unexplained entry is a failing test, not a passing one.
- [ ] Document the invariant in `src/docs/spec/memory/07_runtime-helper-abi.md` beside the
      helper-body rules.

Acceptance: the guard test passes; reintroducing `format!("x{base}")` into `emit_link_expr`
makes it fail. `test-accept.sh` green.
Commit: —

## Validation Plan

- **Tests:** bug-56's reproducer and a spill-forcing deep nest (`tests/rt-behavior/`,
  `tests/rt-error/`); the §2.5 pressure-forcing closure test if confirmed; the Phase 5 guard test
  with its documented allowlist; per `.ai/compiler.md`, `tests/…_valid/**` and
  `tests/…_invalid/**` for every built-in whose lowering moves.
- **Byte gate:** *does not apply as pass/fail.* `scripts/artifact-gate.sh` is still worth running
  as a **change detector** — to see which functions moved and confirm nothing moved that
  shouldn't — but a diff here is expected, not a failure.
- **Runtime proof:** the primary gate, per `.ai/compiler.md`'s hard completion rule. Execute
  `tests/acceptance/` on macos-aarch64, linux-aarch64, linux-riscv64 (`ssh -p 2229`, Alpine
  riscv64 musl), and linux-x86_64. If the x86 boxes are still down (they were as of plan-31),
  this plan is **blocked at its acceptance criterion**, not "done pending hardware."
- **Leak proof:** the arena-corruption failure mode is silent. Run the leak checks; `bug-56`
  corrupts allocation program-wide, and `bug-01`'s value-semantic collection leaks are the
  precedent for how such damage surfaces late.
- **Doc sync:** `src/docs/spec/memory/07_runtime-helper-abi.md` (helper bodies use vregs; the
  reserved set; the Phase 5 invariant); `08_program-startup.md` if the entry stub is scoped out;
  `09_closures.md` if §2.5 changes the closure-environment register's status.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Effort exceeds the one-sitting bar.** *Recommended:* split at the Phase 3/4 boundary —
  `plan-34-C1` (census + link_thunk/closures + caller-saved) and `plan-34-C2` (callee-saved +
  entry stub + invariant). C1 is independently valuable because it closes `bug-56`. (§Phases)
- **Sequencing against plan-34-B.** B's Phase 1 census may reclassify ~800 `"x1"` scratch
  operands into this plan, roughly tripling it. *Recommended:* run B's Phase 1 first — it is
  cheap and lands nothing — then size this plan honestly. (§3)
- **Should bug-56 be fixed standalone first?** A capped-`base` patch is a one-hour fix for a live
  memory-corruption bug. *Recommended: no* — Phase 2 is not much larger and fixes it by
  construction. But if this plan slips, land the cap as a stopgap and say plainly in the commit
  that it is one. (§3)
- **§2.5's closure hazard: fix now or wait for plan-34-B?** *Recommended: fix in Phase 2* via
  `RESERVED`, and let plan-34-B replace the reservation with the `%closure_env` token later. A
  live wrong-indirect-call should not wait on an x-large plan. (§4.2)
- **`entry_and_arena.rs`.** Can the allocator run in the entry stub? *Recommended: assume no*
  until Phase 1 proves otherwise, and treat a documented physical-register exclusion as an
  acceptable outcome. 96 of the ~383 sites may simply be correct as they are. (§2.6)

## Summary

Two of this plan's phases are ordinary refactors, one is a live bug fix, one is a suspected
second instance of the same bug, and the last is what stops both from recurring. The engineering
risk is concentrated where the safety net is thinnest: unlike plan-34-A and plan-34-B, byte
identity cannot prove this plan correct, because changing which register holds what *is* the
change. A misallocation does not fail to compile — it corrupts the arena base, or drops a
callee-saved value across a `bl`, and shows up as a crash three allocations later. Execution on
all four targets, plus leak checks, is the only real gate. `bug-54` and `bug-56` are prior
instances of exactly these two failure modes, which is the argument for doing this and also the
reason to do it carefully.

The Phase 5 invariant is the point of the whole `plan-34` feature. Tokens (A, B) and vregs (C)
are only worth the churn if, at the end, a physical register name is something shared lowering
*cannot say*.

What this leaves untouched: the call boundary (plan-34-B's tokens, uncolorable by construction),
the pinned arena register, the arena and closure layouts, the fallible-call ABI, and — very
possibly — the 96 physical registers in the process entry stub, which may be correct exactly as
they are.

## Follow-up: freeing an x86 GPR (the blocker for the machine-floor scratch) — 2026-07-10

The machine-floor code (thread trampoline, process entry stub, arena helper bodies —
~272 physical operands) cannot be made arch-neutral by token-spelling: its **scratch**
registers (x13/x14, x9–x17) have no ABI role, so the only neutral spelling is a vreg, and
vregging needs a free x86 GPR. x86 has only 4 allocatable GPRs (r10/r11/rbx/r12; rax..r9
are ABI, rbp=8th-arg, rsp, r15=arena_base, r14=zero, r13=closure_env), and reserving the
current-thread register (rbx) drops it to 3 → the allocator panics ("more operands than
registers"). So plan-00-H §7 (free r15 by moving arena_base off the pinned register) is a
hard prerequisite.

**Scope of freeing r15 on x86 (a from-scratch subsystem, NOT a bolt-on):**
- The x86 **console path is a static, interpreter-less, no-libc ELF** (raw syscalls). A
  static binary has no TLS runtime: `fs` is unset, there is no TCB. So real TLS
  (`mov reg, fs:[arena@tpoff]`) requires mfb to **build the static TLS runtime itself**:
  the entry stub must `arch_prctl(ARCH_SET_FS, tcb)` after allocating a TCB + TLS block,
  and the worker trampoline must set each thread's `fs`/TLS (workers on the dynamic/libc
  path get it from `pthread_create`, but the static path does not).
- The ELF emitter (`src/os/linux/link/elf.rs`) must emit a `PT_TLS` program header + a
  `.tbss` slot for the arena pointer, and resolve `R_X86_64_TPOFF32` (initial-exec).
- `select_x86`'s `ARENA_BASE → r15` realization (`src/arch/x86_64/select.rs:687`) becomes
  a per-function TLS load into an allocator-placed vreg (live where the arena is touched),
  and `r15` joins `INT_ALLOCATABLE` (`x86_64/regmodel.rs`).
- Then the machine-floor scratch can vreg (5 GPRs; reserve the current-thread register via
  a `RegisterModel::current_thread()` accessor — x20/rbx/s2 — validated in this session's
  reverted experiment), and the current-thread `x20` becomes a `%thread` pinned token.

Only after r15 is freed can the trampoline/entry-stub/arena-helper scratch be virtualized
and the Phase-5 "no physical register in a shared stream" guard test pass. This is a
dedicated effort (its own plan), gated on runtime execution of the full suite on all
targets (the arena hot path + thread-arena isolation are the silent-corruption surface).
