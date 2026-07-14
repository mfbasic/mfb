# plan-34-B: Role-named registers for every call boundary

Last updated: 2026-07-10
Effort: x-large (1d–3d) — **must be split before implementation**; the Phase 2/3 boundary is
the natural seam (see Open Decisions)
Depends on: plan-34-A (retires `"x19"`/`"x30"`/`"x31"`, so the operand namespace this plan
rewrites contains only call-boundary registers and scratch)

> **STATUS: COMPLETE (2026-07-10).** All four phases landed. Commits: Phase 2 hoist
> (earlier); Phase 3b token vocabulary + `realize_abi_token` seam (`f5b9dbb5`); the
> ~1600-site boundary-literal migration (`cdc4129f`); Phase 4 x86 flip + inference
> deletion (`c098504f`); spec (`98f25463`).
>
> **Design divergence from the original Phase 3a/3b split.** The plan expected the
> class-(c) scratch sites (~313) to be **vregged** (byte-changing, multi-session,
> gated on plan-34-C). Instead they were **tokenized** by dataflow role (an
> arena-alloc result pointer reloaded past its call is `%ret1`, an incoming param is
> `%arg[n]`, etc.), which is byte-identical via the seam and needs no runtime
> re-validation. This unblocked Phases 3–4 without waiting on plan-34-C. The genuine
> residual scratch that could not be tokenized — 4 `x1` tv-math temporaries in
> `tls/openssl.rs`, the `x27`/`x28` argc/argv save-registers in `entry_and_arena.rs`,
> and `#[cfg(test)]` fixtures — stays as bare literals; x86 maps a residual bare
> `x0`–`x8` to its `RETS` home (reproducing the old no-boundary arm) and `x9`+ by
> pool. Vregging those residuals remains **plan-34-C** scope.
>
> **Verification (the payoff clause).** Byte-identical to the pre-plan baseline
> (`41578ef3`) on **all four targets**: macos-aarch64 (`artifact-gate.sh` 0 diffs)
> and cross-emitted linux-{aarch64,riscv64,x86_64} `.nobj` (447 programs each, 0
> diffs). The x86 lookup reproduces the deleted CFG inference exactly — **0 x86 byte
> diffs = 0 role misclassifications**. 2479 unit tests pass. The one deferred item is
> the strict "no bare `x0`–`x8`/`x28` in a shared stream" guard test, which cannot
> pass until plan-34-C vregs the residual scratch above.

Shared lowering names call-boundary registers by their **AArch64 register number**. AArch64
collapses three disjoint register banks into one — arguments, results, and syscall arguments
all live in `x0`–`x7` — so the shared layer never had to say which bank it meant. SysV
x86-64 has three genuinely different banks, and the same `x3` operand must become `rcx`,
`r10`, or `rsi` depending on a role the shared layer discarded.

The x86-64 backend therefore carries `remap_x86_abi` (`x86_64/select.rs:108`), a fixpoint
dataflow over the CFG that *reconstructs* the role of every `x0`–`x8` operand. It has a
fallback arm for when inference fails, and that arm's comment names the SIGSEGV it was
invented to fix.

The riscv64 backend states the diagnosis in its own header (`riscv64/select.rs:20-24`):

> *"Unlike x86 — where argument, return, and syscall registers are three disjoint files,
> forcing a control-flow role analysis — RISC-V reuses the `a0`–`a7` bank for arguments and
> results and syscall arguments, exactly as AArch64 reuses `x0`–`x7`. So the remap is a
> simple positional substitution."*

The single behavioral outcome: **every call-boundary register in shared lowering is named by
its role, `remap_x86_abi`'s role-inference dataflow is deleted, and the emitted bytes for all
four targets are unchanged.**

References:

- `src/docs/spec/memory/02_fallible-call-abi.md` — the four-register result form, currently
  specified in AArch64 register names.
- `src/docs/spec/memory/06_native-calling-convention.md` — argument passing, `x0..x7` + stack
  tail (bug-08).
- `src/docs/spec/memory/07_runtime-helper-abi.md`, `09_closures.md` — helper and closure ABI.
- `.ai/compiler.md` — completion gate, acceptance suite, register lifetimes.

## 1. Goal

- Shared lowering contains no `"x0"`–`"x8"` or `"x28"` string literal. Every call-boundary
  operand is named by role: `%arg0..7`, `%ret0..3`, `%sysnr`, `%sysarg0..5`, `%sysret`,
  `%closure_env`.
- `x86_64/select.rs`'s `AbiBoundary` enum (`:24`), `next_after`/`boundary_before` fixpoint
  (`:108-~560`), and `map_abi_register`'s `role`/`is_result` parameters (`:81`) are **deleted**.
  Role mapping becomes a table lookup.
- `scripts/artifact-gate.sh` reports **byte-identical** `.ncode` for all four targets.

### Non-goals (explicit constraints)

- **No emitted-byte change.** This renames operand *tokens*; it does not change which physical
  register anything lands in.
- **No change to the fallible-call ABI.** Tag in the first result register, value in the
  second, message third, source fourth. `src/docs/spec/memory/02_fallible-call-abi.md` stays
  semantically true; only its wording moves from register numbers to roles.
- **No vreg conversion of a boundary register.** `codegen_utils.rs:~455` states the contract:
  ABI registers *"stay physical (the allocator never colors them)"*. A boundary register must
  be uncolorable — that is what makes it a boundary. Tokens, not vregs.
- **No change to the stack-argument tail** (bug-08's `> 8` parameter path).
- `x19` (arena) and the zero/link registers are plan-34-A. Hand-picked scratch is plan-34-C.

## 2. Current State

### 2.1 Three banks, one namespace

`src/arch/aarch64/abi.rs`:

- `:3` — `RETURN_REGISTER: &str = "x0"`
- `:6` — `fn argument_register(index) -> "x{index}"` for `index < 8`
- `:19` — `REGISTER_ARGUMENT_COUNT: usize = 8`
- `:96` — `fn syscall_register() -> "x8"`

`src/target/shared/code/error_constants.rs`:

- `:82-88` — `RESULT_TAG_REGISTER = abi::RETURN_REGISTER` (`"x0"`), `RESULT_VALUE_REGISTER = "x1"`,
  `RESULT_ERROR_MESSAGE_REGISTER = "x2"`, `RESULT_ERROR_SOURCE_REGISTER = "x3"`
- `:124` — `CLOSURE_ENV_REGISTER = "x28"`

The constants exist and are widely bypassed: `src/target/shared/code/` contains **1615** bare
`"x0"`–`"x7"` literals.

| operand | count | dominant role |
|---|---|---|
| `"x1"` | 1091 | result **value** (every fallible call returns through it) |
| `"x0"` | 276 | result **tag** / return value / first argument |
| `"x2"` | 170 | result **error message** / third argument |
| `"x3"` | 48 | result **error source** / fourth argument |
| `"x4"`–`"x7"` | 30 | arguments only |

`"x1"` outnumbering `"x0"` four-to-one is the tell: this is not an argument-register
distribution. It is the result ABI plus — critically — **ad-hoc scratch that happened to pick
`x1`**. `builder_collection_mutate.rs:553,569` reads
`abi::move_immediate("x1", "Integer", "8")` then `abi::store_u64("x1", abi::stack_pointer(), result_slot)`,
and `:597,600` use `"x1"` as a data pointer. Nothing in the source distinguishes "the ABI
result register" from "a scratch register spelled x1".

### 2.2 The three banks, per ISA

`x86_64/select.rs:68-76`:

```rust
const CALL_ARGS: &[&str] = &["rdi", "rsi", "rdx", "rcx", "r8", "r9", "rax", "rbp"];
const SYS_ARGS:  &[&str] = &["rdi", "rsi", "rdx", "r10", "r8", "r9"];
const RETS:      &[&str] = &["rax", "rdx", "rcx", "rsi"];
```

`CALL_ARGS` and `SYS_ARGS` **diverge at index 3** — `rcx` vs `r10` — because the `syscall`
instruction clobbers `rcx`. So one operand name has three homes:

| `x3` used as… | AArch64 | riscv64 | x86-64 |
|---|---|---|---|
| call argument | `x3` | `a3` | `rcx` |
| syscall argument | `x3` | `a3` | `r10` |
| result | `x3` | `a3` | `rsi` |

And `x8` is itself overloaded: the syscall-number register on Linux/AArch64, mapped by
`map_abi_register:87` to `rax` when the boundary is a syscall, and by `riscv64/select.rs:486`
to `a7`.

Note `CALL_ARGS` has eight entries: MFBASIC functions take up to 8 register parameters but
SysV supplies 6, so `select.rs:60-67` extends the bank with `rax` (7th) and `rbp` (8th). `rbp`
is simultaneously the "reserved frame register" — a live constraint any `%arg7` mapping must
preserve.

### 2.3 What the inference costs

`x86_64/select.rs`:

- `:24 abi_boundary_of` — classify each instruction as `Call` / `Syscall` / `Ret`.
- `:81 map_abi_register(n, role: Option<AbiBoundary>, is_result: bool)` — the physical home is
  a **function of inferred role**.
- `:108 remap_x86_abi` — builds `next_after` (nearest boundary a value flows *into*, following
  branch targets) and `boundary_before` (a **fixpoint forward dataflow** over the CFG for the
  result direction).

Its comments catalogue the hazards:

- `:92-95` — a loop back-edge poisons the index; "mapping a leftover `x1` to rax (the OK tag =
  0) gave a null-dst copy → **SIGSEGV**".
- `:210-212` — the error-Result staging block is entered both from `make_error_result` calls
  *and* by fall-through from a success path that "sets x0/x1 manually with no call"; the merge
  lattice must let a call boundary win.
- `:451-453` — a `toString` prologue with two params (`x0`, `x1`) both map to `None` → the
  `rax` fallback → collision.

And the fallback arm itself (`:96-103`):

```rust
// No following boundary: a leftover ABI register used as a plain value …
None => RETS.get(n).copied().unwrap_or("rax"),
```

**There is a code path for "the dataflow could not determine this operand's role."** With role
tokens, that arm cannot exist: an operand cannot fail to have a role, because the role is what
it is named. This is the single clearest statement of why the plan is worth doing.

### 2.4 `%closure_env` belongs here, not with the invariant registers

`CLOSURE_ENV_REGISTER = "x28"` looks like a pinned register. It is not. Its entire lifetime is
two instructions (`builder_emit_helpers.rs:223-224`):

```rust
self.emit(abi::move_register(CLOSURE_ENV_REGISTER, &env_register));
self.emit(abi::branch_link_register(&code_register));
```

It is an **implicit ninth argument register**, set immediately before an indirect call and read
by the callee. Unlike `arena_base` it has no `RegisterModel` trait method, no realization test,
and no reservation — it is in `INT_ALLOCATABLE` on all three ISAs (AArch64
`regmodel.rs:101-104` ends `"x27", "x28"`; x86 realizes it via `map_scratch_register` to `r13`,
allocatable; riscv64 to `s10`, allocatable). See plan-34-C §2.5 for the hazard that creates.

Naming it `%closure_env` here — a call-boundary token that selection places and the allocator
cannot color — is what fixes it by construction. `src/docs/spec/memory/09_closures.md` is the
spec to keep true.

### 2.5 Precedent to mirror

`codegen_utils.rs:~455` (`finalize_vreg_helper`) already documents the invariant in prose:

> "Physical operands the body still names — `arena_base` (the reserved arena register), the ABI
> `x0`–`x7` it loads call args into **and** reads results from — stay physical (the allocator
> never colors them…)"

"loads call args into *and* reads results from" — the doc names two roles over one register
range, in one breath. This plan makes that sentence into two token families. plan-34-A's
`ARENA` token is the same move, already landed for a third.

## 3. Design Overview

Five layers:

1. **Hoist `abi` out of `arch::aarch64`.** `shared/code/mod.rs:4` reads
   `use crate::arch::aarch64::{abi, ops::CodeOp};`. Move `abi.rs` to `src/target/shared/abi.rs`;
   re-export from `arch::aarch64` for AArch64-internal callers. Do the same for
   `RegisterModel` — `shared/code/regalloc/mod.rs:19` imports
   `crate::arch::aarch64::regmodel::RegisterModel` into a file whose own doc-comment calls
   itself the "ISA-neutral register allocator core." Move the trait; leave the AArch64
   *implementation* in place. Pure code motion.

2. **Define the tokens** (§4.1): `ARG[0..8]`, `RET[0..4]`, `SYSNR`, `SYSARG[0..6]`, `SYSRET`,
   `CLOSURE_ENV`. Values are `%`-prefixed sentinels, for the same reason
   `regalloc/mod.rs:26` prefixes vregs: the prefix *"cannot collide with any physical register
   name, immediate, symbol, label, or type name."* Here the sentinel is load-bearing — an
   unmigrated site must fail to compile or fail selection, not silently pass through.

3. **Census and migrate** the 1615 boundary sites, splitting each by role (§4.2). This is the
   work, and it is not mechanical.

4. **Translate at the seam, temporarily.** Phase 3 converts `%argN`/`%retN`/… back to `xN`
   before instruction selection, so all three backends see today's input unchanged and
   AArch64 + riscv64 get a byte-identical proof while x86 is still on the inference path.

5. **Flip the backends** to token lookup and delete the dataflow.

**Where the correctness risk concentrates:** entirely in step 3, and specifically in the `"x1"`
sites that are *scratch*, not the result register. A scratch site renamed to `RET[1]` still
emits `x1` on AArch64 and `a1` on riscv64 — byte-identical, gate green — but maps to `rdx`
instead of `rsi` on x86-64. **A silent miscompile the byte gate cannot catch**, because x86's
bytes were going to change anyway once the inference pass is deleted.

That hazard dictates the phase order. x86 must not switch to token lookup until the shared
migration is complete *and* independently verified on the two ISAs where the byte gate is
meaningful.

**Rejected alternative — `%arg`/`%ret` only, skip the syscall bank.** This was this plan's
original scope and it is wrong. `SYS_ARGS` differs from `CALL_ARGS` at index 3, so
`AbiBoundary::Syscall` and the dataflow that computes it would survive. You would delete half
the pass and keep all of its failure modes.

**Rejected alternative — keep `xN`, add a role field to `CodeInstruction`.** Avoids the
1615-site edit, but the role gets set at the same call sites anyway and every operand consumer
reads two fields instead of one. Strictly worse.

**Rejected alternative — infer roles once in shared lowering, then bake them in.** That is
`remap_x86_abi` moved uphill: same fixpoint, same `None` arm, same bugs. The inference is only
necessary because the information was discarded. Name the roles at the source or don't bother.

**Rejected alternative — land before plan-34-A.** `"x19"`/`"x30"`/`"x31"` would have to be
threaded through the same operand-token machinery and then removed again.

## 4. Detailed Design

### 4.1 The tokens

```rust
/// A call's Nth outgoing argument. Never allocator-colored. Indices 0..8; the
/// custom convention passes 8 in registers (REGISTER_ARGUMENT_COUNT) and the rest
/// in a stack tail (bug-08).
pub(crate) const ARG: [&str; 8] = ["%arg0", …, "%arg7"];

/// A call's Nth result. RET[0..4] are the fallible-call ABI's tag / value /
/// error-message / error-source (spec: memory/02_fallible-call-abi.md). An
/// infallible call uses RET[0] only.
pub(crate) const RET: [&str; 4] = ["%ret0", "%ret1", "%ret2", "%ret3"];

/// The syscall number register. AArch64/Linux `x8`, AArch64/macOS `x16`,
/// riscv64 `a7`, x86-64 `rax`. Four realizations of one role — which is exactly
/// why it cannot be spelled as a register number.
pub(crate) const SYSNR: &str = "%sysnr";

/// A syscall's Nth argument. Distinct from ARG: x86-64 passes syscall arg 3 in
/// `r10`, not `rcx`, because the `syscall` instruction clobbers `rcx`.
pub(crate) const SYSARG: [&str; 6] = ["%sysarg0", …, "%sysarg5"];

/// A syscall's result. AArch64 `x0`, riscv64 `a0`, x86-64 `rax`.
pub(crate) const SYSRET: &str = "%sysret";

/// The closure environment pointer — an implicit argument register, live from its
/// definition to the immediately following indirect call (spec: memory/09_closures.md).
pub(crate) const CLOSURE_ENV: &str = "%closure_env";
```

`error_constants.rs:82-88,124` is re-expressed without changing meaning:

```rust
pub(crate) const RESULT_TAG_REGISTER: &str           = abi::RET[0];
pub(crate) const RESULT_VALUE_REGISTER: &str         = abi::RET[1];
pub(crate) const RESULT_ERROR_MESSAGE_REGISTER: &str = abi::RET[2];
pub(crate) const RESULT_ERROR_SOURCE_REGISTER: &str  = abi::RET[3];
pub(crate) const CLOSURE_ENV_REGISTER: &str          = abi::CLOSURE_ENV;
```

`abi::argument_register(i)` returns `ARG[i]`; `abi::return_register()` returns `RET[0]`;
`abi::syscall_register()` returns `SYSNR`. The unit test at `abi.rs:926` asserting
`syscall_register() == "x8"` becomes a *realization* test per target.

**`SYSNR` is the strongest single argument for this plan.** `syscall_register()` today returns
`"x8"`, which is Linux/AArch64's answer. It is called only from `linux_aarch64/code.rs:655,663`
and `linux_riscv64/code.rs:670,678` — never from macOS or from shared code, which set the
syscall number by other means. One role, four realizations, and an accessor that is only
correct for two of them because nobody else dares call it.

### 4.2 Classifying the 1615 sites

Every site falls into exactly one class. Phase 1 produces the census; nothing migrates until it
is complete, because class (c) is where the miscompile lives.

- **(a) Outgoing call argument** — written before a `call`. → `abi::ARG[n]`.
- **(b) Result** — read after a `call`/`syscall`, or written by a callee about to `ret`.
  → `abi::RET[n]` / `abi::SYSRET`.
- **(c) Ad-hoc scratch spelled `x1`** — neither. `builder_collection_mutate.rs:553-637` is the
  exemplar. → **must become a vreg** (`%vN`), not a token. This is the overlap with plan-34-C
  and the reason this plan is not a `sed`.
- **(d) Syscall argument** — written before an `svc`/`ecall`/`syscall`. → `abi::SYSARG[n]`.
  `mir.rs:183` (`Svc => Syscall`) and the fused `syscall_br` op mean syscalls flow through the
  MIR stream, so these operands are in scope even though the syscall *numbers* are set in
  per-target `code.rs`.
- **(e) Syscall number** — `x8` (Linux) / `x16` (macOS). → `abi::SYSNR`.

The class-(c) population is unknown until the census runs. The 4:1 ratio of `"x1"` to `"x0"`
bounds it crudely: if `x1` were purely the result register it would appear at most as often as
`x0`, so **on the order of 800 `x1` sites are not the result register**. That number is the
single most important output of Phase 1, and if it is large this plan re-sequences behind
plan-34-C.

**macOS `%sysnr` = `x16` is unverified.** It is the standard Darwin/arm64 convention, and
`abi::syscall_register()` is demonstrably never called from macOS code — but the macOS
syscall-number site was not traced. Phase 1 must confirm it from source before `SYSNR`'s
realization table is written. (The `"x16"` literals in `entry_and_arena.rs:530,542,543,564` are
plain scratch, **not** the syscall number — an easy and checked-for false lead.)

### 4.3 Backend mapping, after

- **AArch64** (`arch/aarch64/select.rs`): `ARG[n] → x{n}`; `RET[n] → x{n}`; `SYSARG[n] → x{n}`;
  `SYSRET → x0`; `SYSNR → x8` (Linux) / `x16` (macOS); `CLOSURE_ENV → x28`.
- **riscv64** (`arch/riscv64/select.rs:524`): `ARG[n]`/`RET[n]`/`SYSARG[n] → a{n}`;
  `SYSNR → a7`; `SYSRET → a0`; `CLOSURE_ENV → s10`. Replaces the `n <= 7 => a{n}` arm and the
  `x8 => a7` special case; strictly simpler.
- **x86-64** (`arch/x86_64/select.rs`): `ARG → CALL_ARGS`, `SYSARG → SYS_ARGS`, `RET → RETS`,
  `SYSRET`/`SYSNR → rax`, `CLOSURE_ENV → r13`. Direct lookup. `abi_boundary_of`,
  `remap_x86_abi`'s dataflow, and `map_abi_register`'s `role`/`is_result` parameters all delete.

`SYSNR`'s AArch64 realization is **per-target, not per-arch** — the one place `select_aarch64`
must consult the OS. `CLOSURE_ENV` must additionally be removed from `INT_ALLOCATABLE` on all
three ISAs, or given a `RegisterModel` accessor. See plan-34-C §2.5.

## Compatibility / Format Impact

Emitted machine code, `.mfp` format, and diagnostics are unchanged.

The `-mir` dump text changes (`x0` → `%ret0` / `%arg0` / `%sysarg0`) and its goldens must be
refreshed. This is a feature: the dump becomes readable as a neutral IR for the first time,
which is what `mir.md` §5 already demands of the op vocabulary (no `smulh`/`rorv`/`adrp`).

`src/docs/spec/memory/02_fallible-call-abi.md:6-20` and `06_native-calling-convention.md:6-44`
specify the ABI in AArch64 register names and **must be rewritten** in role terms with a
per-ISA realization table. `09_closures.md` gains `%closure_env`. This is a spec-surface
change, not a doc refresh.

## Phases

### Phase 1 — Census (no code change) — **DONE 2026-07-10; VERDICT: re-sequence behind plan-34-C**

Classify all sites. Lands nothing; determines whether the rest of the plan is viable.

- [x] Exhaustive census of every `"x0"`–`"x8"`/`"x28"` literal in `src/target/shared/code/`
      (incl. `crypto_ec/`, `net/`, `tls/`) — 6 parallel classifiers over ~46 files.
- [x] class-(c) count reported (below), from an exhaustive per-site pass, not a sample.
- [x] macOS syscall-number traced and `%sysnr`'s four realizations cited to source.
- [x] class-(c) **exceeds ~200 → decision recorded: STOP; land plan-34-C's scratch vregging
      first, then return for Phases 3–4.**

#### Census result (all ~1635 in-scope literals classified, zero unclassified)

| class | count | notes |
|---|---:|---|
| (a) call argument | 601 | staged before a `bl`/`blr`/`branch_link`/`emit_libc_call` |
| (b) result / ABI-boundary | 548 | read after a call/`arena_alloc`, staged as return, or incoming-param reads |
| **(c) scratch** | **319** | ad-hoc working register spelled `xN`, no adjacent call/syscall/ret |
| (d) syscall argument | 142 | staged before a raw syscall / `emit_write`/`emit_read_file`/ioctl |
| (e) syscall number | 0 | set in **platform** `code.rs`, not shared lowering |
| (f) meta (tests, role-const defs) | ~25 | `#[cfg(test)]` fixtures + `CLOSURE_ENV_REGISTER` def |

**class-(c) = 319 is concentrated**, not spread: SIMD kernels (`builder_simd_math` 68 +
`builder_simd_float_math` 40 + `builder_simd_fixed_math` 22 = **130**), collections
(`builder_collection_mutate` 62 + `builder_collection_layout` 24 = **86**),
`entry_and_arena` **59**, `tls` 12, the `crypto_ec` byte-copy loop 6, `builder_pow` 2, `net` a
handful. The ABI-heavy helpers (fs/os/tls/runtime/crypto and most builders) already route
scratch through `%vN` vregs — their `xN` literals are genuine (a)/(b)/(d) boundary registers.
(The (b)/(c) split is partly interpretive — `x1`-as-alloc-result-base used far past its call
could be read as (c) — so 319 is a lower-ish bound; it does not fall below ~275 under any
reasonable reading.)

**`%sysnr` — four realizations confirmed from source:** linux-aarch64 `x8`
(`abi::syscall_register()`, `linux_aarch64/code.rs:655,663`), riscv64 `a7`
(`abi::syscall_register()` → remap, `linux_riscv64/code.rs:670,678`), **macOS `x16`**
(`macos_aarch64/code.rs:704,723`, `move_immediate("x16", …, DARWIN_SYSCALL_*)`), linux-x86_64
`rax` (`linux_x86_64/code.rs`). Confirmed the plan's warning: `entry_and_arena.rs`'s `x16`
literals are **scratch** (a byte counter in the args-fill loop), *not* the syscall number.

#### Decision (per this phase's gate)

class-(c) = 319 ≫ 200. **Phases 3–4 are BLOCKED behind plan-34-C**: migrating 319 scratch
sites to vregs is C's scope (byte-CHANGING work), and doing it inside B would triple B and
mix a byte-changing migration into a byte-identical rename. The correct sequence is
plan-34-C's scratch-vregging first (concentrated in SIMD + collections + entry_and_arena),
which shrinks B's boundary namespace to genuine (a)/(b)/(d)/(e) sites, then B's Phases 3–4.
Phase 4's x86 hardware is **available** (port 2227, Alpine x86_64), so hardware is not the
blocker — plan-34-C is.

Acceptance: **met.** Full census in this document; every in-scope literal classified; `%sysnr`
cited to source; the class-(c) gate evaluated and the re-sequencing decision recorded.
Commit: — (Phase 1 lands no code.)

#### Re-assessment of the plan-34-C dependency (2026-07-10)

Sizing the unblock: plan-34-C targets a **different** register range (`x9`–`x18`, `x20`–`x27`)
than B's census (`x0`–`x8`/`x28`); currently **377** such literals in shared/code (of which
`x16`/`x17` ≈26 are IP0/IP1 platform scratch that stays, and `x8` is the syscall number). The
plan's intent (Open Decisions) is that B's 319 class-(c) sites are **added** to C's scope, so
the true unblock is a **~670-site byte-CHANGING scratch→vreg migration** across the compiler's
most correctness-critical code — the arena allocator (`entry_and_arena.rs`, which C §2.6 flags
"runs before the arena exists… whether the allocator can run here at all is an open question"),
the SIMD numerical kernels, closures (C §2.5's unconfirmed `x28` mis-color hazard), and the
callee-saved-across-call cases (C §2.4, entangled with the prior `bug-54`). It cannot use the
byte-identical gate as proof (the allocator picks different registers by design), so each site
needs runtime validation on all four targets.

**bug-56 is already fixed** (its doc is in `old-plans/`; `emit_link_expr` already vregs — plan-34-C
Phase 2 is effectively done), so C is partially landed, but the bulk (the ~670-site migration)
remains. Per `.ai/compiler.md` / "correctness over completeness," this byte-changing migration
into the arena allocator and SIMD kernels must be done incrementally with per-file runtime
validation, not compressed into one pass — it is genuinely multi-session work, and rushing it
risks silent miscompiles the byte gate cannot catch. **plan-34-B Phases 3–4 remain blocked on
that effort.**

### Phase 2 — Hoist `abi` and `RegisterModel` to neutral modules — **DONE (both bullets)**

Pure code motion. Separately valuable: it deletes the most-cited evidence that MIR is not
ISA-neutral.

- [x] Moved `src/arch/aarch64/abi.rs` → `src/target/shared/abi.rs`; added `pub(crate) mod abi`
      to `shared/mod.rs`; `arch::aarch64::mod.rs` now re-exports it
      (`pub(crate) use crate::target::shared::abi`) so AArch64-internal callers are unchanged.
      Updated all ~28 `shared/` importers to `crate::target::shared::abi`.
- [x] Moved `RegClass` + the `RegisterModel` trait to a new `src/target/shared/regmodel.rs`;
      `arch::aarch64::regmodel` re-exports them and keeps `Aarch64RegisterModel` in place;
      updated the `shared/code/regalloc/*` and `mir.rs` importers.

Acceptance: **byte-identical met, name-purity partially met.** `artifact-gate.sh` → 0 diffs;
four-target `.nobj` diff vs. pristine-HEAD → identical; `cargo test` 2479 passed. The
"no `crate::arch::aarch64` in `shared/`" clause is **not fully achievable by these two
bullets** — after the hoist, `shared/` still names `arch::aarch64::{backend, ops::CodeOp,
select::select_aarch64, reloc::reloc_kind}` and the `ARENA_BASE_REGISTER`/`Aarch64RegisterModel`
couplings. Neutralizing those means hoisting the `CodeOp` op enum and the backend dispatch — a
much larger refactor the plan's Phase 2 never scoped. Deferred as a separate follow-on; the two
named modules (`abi`, `RegisterModel`) are hoisted.
Commit: —

### Phase 3 — Introduce tokens; migrate shared lowering; keep the backends on `xN`

The 1615-site edit, gated on byte-identity for AArch64 and riscv64.

**Phase 3a (class-(c) scratch → vreg) — IN PROGRESS, landing incrementally (byte-changing):**
- [x] `crypto_ec.rs` `emit_read_byte_list` byte-copy loop: `x2`/`x3` → `%v16`/`%v17` (6 sites),
      commit `c7479310`. Byte-identical on macos-aarch64 (allocator re-picks the same regs),
      4-target build clean, `crypto-ec-valid` ECDSA runtime matches golden. **Proves the approach
      for self-contained scratch.**
- [ ] Remaining ~313 class-(c) sites. **Not mechanical, and unevenly risky:**
      - SIMD kernels (~130, `builder_simd_*`) are **hand-optimized "register-tight" bodies**
        (`emit_fixed_sqrt_vector`'s own comment: "physical `v1..v7` … register-tight body");
        `x0`/`x1` are call-boundary args/results at the prologue *and* independent scratch in the
        kernel — each scratch use-region is a distinct lifetime to group. Vregging risks allocator
        spills (bytes *and* perf); needs numerical runtime validation on all four targets.
      - collections (~86) reload `x1` as a data-base pointer past its call.
      - `entry_and_arena` (~59) runs **before the arena exists** — C §2.6's open "can the allocator
        run here at all" question must be settled first.
      - Some census-(c) sites are really incoming-args (→ tokens, not vregs) — the (b)/(c) boundary
        needs per-site confirmation, so the true vreg count is ≤ 319.
      This is the multi-session grind; each file is a separate byte-changing commit with 4-target
      runtime validation. Do the clean/self-contained files first, `entry_and_arena` last (or via
      plan-34-C's feasibility work).

- [ ] Add `ARG`/`RET`/`SYSNR`/`SYSARG`/`SYSRET`/`CLOSURE_ENV` to the hoisted `abi` (§4.1).
- [ ] Re-express `error_constants.rs:82-88,124` and `abi::argument_register` /
      `return_register` / `syscall_register` in terms of the tokens.
- [ ] Migrate class-(a), (b), (d), (e) sites.
- [ ] Migrate class-(c) sites to vregs via `CodeBuilder::allocate_register`. **These change
      which physical register is used and therefore change emitted bytes** — land them as a
      separate, earlier commit with goldens refreshed and runtime-validated, so the token commit
      itself stays byte-clean.
- [ ] At the MIR→CodeOp seam, translate the tokens back to `xN` before instruction selection,
      so all three backends see today's input unchanged.
- [ ] Guard test: no shared instruction stream carries a bare `"x0"`–`"x8"` or `"x28"` operand.
- [ ] Refresh `-mir` goldens.

Acceptance: `artifact-gate.sh` byte-identical for macos-aarch64, linux-aarch64, linux-riscv64.
`test-accept.sh` green. `tests/acceptance/` executes on those three (riscv64 via `ssh -p 2229`).
Commit: —

### Phase 4 — Flip the backends to token lookup; delete the inference pass (highest-risk work last)

The payoff. Cannot land before Phase 3 is runtime-verified.

- [ ] Remove the seam translation; pass tokens to selection.
- [ ] `arch/aarch64/select.rs`, `arch/riscv64/select.rs`: table lookup per §4.3, including
      `SYSNR`'s per-target AArch64 realization.
- [ ] `arch/x86_64/select.rs`: table lookup; delete `abi_boundary_of` (`:24`), `remap_x86_abi`'s
      dataflow (`:108-~560`), and `map_abi_register`'s `role`/`is_result` parameters (`:81`) —
      including the `None` fallback arm (`:96-103`), which has no successor.
- [ ] Remove `CLOSURE_ENV`'s realization from `INT_ALLOCATABLE` on all three ISAs, or add a
      `RegisterModel::closure_env()` accessor mirroring `arena_base()` (`aarch64/regmodel.rs:72`).
- [ ] Preserve and re-target the inference pass's regression tests (`x86_64/select.rs:754-1019`)
      — they encode the SIGSEGV and staged-def cases and must keep passing against the lookup.

Acceptance: `artifact-gate.sh` byte-identical for **all four** targets, including linux-x86_64
— a correct lookup reproduces a correct inference exactly. **Any x86 byte diff is a class-(c)
misclassification from Phase 1, not a golden to refresh.** `tests/acceptance/` executes on
linux-x86_64.
Commit: —

## Validation Plan

- **Tests:** the bare-`xN` guard test; the retained `select.rs` inference regression cases,
  re-pointed at the lookup; `%sysnr` realization tests replacing `abi.rs:926`; refreshed `-mir`
  goldens; per `.ai/compiler.md`, `tests/…_valid/**` and `tests/…_invalid/**` for every built-in
  whose lowering moves.
- **Byte gate:** `scripts/artifact-gate.sh` at every phase. Phase 4's x86 byte-identity is the
  strongest single signal in this plan: it proves the deleted dataflow and the new lookup agree
  on every instruction in the corpus.
- **Runtime proof:** `.ai/compiler.md`'s completion gate — byte-identity is not sufficient.
  `tests/acceptance/` must execute on all four targets. **linux-x86_64 is the target that
  matters here**, and per plan-31's notes the x86 boxes were down. If they still are, this plan
  is **blocked at Phase 4**, not "done pending hardware." Say so rather than shipping.
- **Doc sync:** rewrite `src/docs/spec/memory/02_fallible-call-abi.md` and
  `06_native-calling-convention.md` in role terms with a per-ISA realization table; add
  `%closure_env` to `09_closures.md`; update `07_runtime-helper-abi.md`. All have uncommitted
  working-tree edits — coordinate.
- **Acceptance:** `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **This plan is x-large and must be split.** *Recommended:* `plan-34-B1` = Phases 1–2 (census
  + hoist), `plan-34-B2` = Phases 3–4 (migrate + flip). B1 is independently valuable: the census
  sizes plan-34-C, and the hoist removes the `arch::aarch64` imports from `shared/`. (§Phases)
- **Sequencing against plan-34-C.** B's Phase 1 census may reclassify ~800 `"x1"` scratch
  operands into C, roughly tripling it. *Recommended:* run B's Phase 1 first — it is cheap and
  lands nothing — then size C honestly. (§4.2)
- **`%closure_env`: reserve, or give it a `RegisterModel` accessor?** Reserving costs x86 one of
  its five allocatable GPRs. *Recommended: the accessor*, mirroring `arena_base()`, so each ISA
  picks a register it can afford — and see plan-34-C §2.5 for the latent bug that makes this
  urgent rather than cosmetic. (§4.3)
- **`RET` arity.** Four result registers are the fallible ABI; infallible calls use one.
  *Recommended: `RET[0..4]` with a documented rule* — `RESULT_TAG_REGISTER` already aliases
  `RETURN_REGISTER` today (`error_constants.rs:82`), so the identity is pre-existing. (§4.1)
- **`%arg7` → `rbp` on x86.** `CALL_ARGS[7]` is `rbp`, which is also the reserved frame
  register. This works today only because "no vregified builder function names it"
  (`select.rs:60-67`). *Recommended: preserve the constraint and assert it in a test* — the
  token migration is exactly when it could silently break. (§2.2)

## Summary

The real engineering risk is not the 1615-site rename. It is the subset of `"x1"` operands that
are scratch wearing the result register's name. Those are invisible to the byte-identical gate
on AArch64 and riscv64 — both collapse arguments, results, and syscall arguments into one bank
— and surface as a miscompile only on x86-64, the one ISA where the gate would have to be
trusted. Phase 1's census is therefore not a formality; it is the plan.

What this deletes: `AbiBoundary`, a fixpoint CFG dataflow, and a fallback arm for "role
unknown" that exists because roles can be unknown. What it leaves untouched: the fallible-call
ABI's semantics, the stack-argument tail, the arena and zero/link tokens (plan-34-A), the
hand-picked scratch (plan-34-C), and the emitted bytes of every target.
