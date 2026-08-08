# plan-85-A: convention-explicit ABI token vocabulary + aligned per-target realization + census

Last updated: 2026-08-03
Overall Effort: huge (>3d — ~4,900 emission sites re-classified across 72 files onto a
typed `Operand::Abi` variant, 4 native backends re-wired, the 646-line x86 fixpoint +
legacy string-token path deleted, SysV-x86 goldens regenerated for the aligned convention)
Effort: large (3h–1d)
Depends on: nothing (plan-71 is complete/archived — it landed the prerequisites
this feature builds on: the `@src` source-instrumentation tool, the
`selfmove_probe`, and the value-level Category-1/2 partition proof).

This is the lead sub-plan of plan-85. It **replaces the two overloaded ABI role
tokens `%arg`/`%ret` (plus `%sysarg`) with six convention-explicit tokens** —
`%argMFB`/`%retMFB` (MFB's internal convention), `%argC`/`%retC` (the platform C
ABI), `%argSys`/`%retSys` (the syscall convention) — so that every operand in the
shared MIR stream **names the calling convention it belongs to** instead of leaving
x86 to *infer* it. The single behavioral outcome of the whole plan-85 feature
(delivered across A→D): the 646-line `remap_x86_abi` fixpoint is **deleted**, MFB's
internal convention is **aligned** (`%retMFB` = `%argMFB` = `[rdi,rsi,rdx,rcx]` on
SysV, no `rax`), and every operand's register is a direct lookup keyed on its explicit
token.

**This is a byte-CHANGING feature on SysV-x86** (linux/macos-x86): the aligned MFB
convention deliberately reassigns registers, so SysV-x86 goldens regenerate.
Win64-x86, AArch64, and RISC-V are **byte-identical** (they are already aligned — args
and results coincide there), and their byte-identity is the migration's cross-target
gate. SysV-x86 correctness is gated on **rt-behavior** (real execution), not
byte-identity.

The six tokens are a **typed `Operand::Abi { convention, role, index }` variant** — not
string constants. Today the ABI role tokens are the last register category still
funnelling through `Operand::Raw(Box<str>)` (`operand.rs:28`: *"physical registers
reaching the code layer as a bare `&str` … still funnel through `Raw`"*), a
heap-allocated boxed string per emission on the **allocation-bound** acceptance compile
(memory `codeinstruction-operand-typing-and-regalloc-perf`: 808M allocs, malloc-dominated).
Because plan-85 already re-touches every one of the ~4,900 ABI-token sites and rewrites
their realization, it is the leverage point to **finish plan-82's `Raw`→typed migration
for tokens**: a `Copy` typed arm (zero allocation) realized by a typed match, replacing
the `Raw(Box<str>)` strings. This rides along with the ABI work at no extra site-touch cost.

plan-85-A itself changes **no emitted byte** — it is a pure primitive addition +
measurement: (1) the six tokens exist as `Operand::Abi` and realize to their **final
aligned** physical registers on all four backends; (2) `select_x86` is taught to realize
an explicit token **directly** (bypassing the fixpoint), the seam the deletion needs;
(3) the per-operand **classification census** assigning every current emission site to one
of the six conventions. No site emits the new tokens yet, so A alone stays byte-identical
on all five targets (a typed operand that renders to the same register name emits the same
byte — plan-82's premise).

## Why this exists — bug-387, and why plan-71 stopped short

The shared lowering emits `%arg`/`%ret` role tokens that `realize_abi_token`
(`src/target/shared/abi.rs:329`) maps to **AArch64's** register file, where the k-th
argument and k-th result are the *same* register (`%arg0`|`%ret0` → `x0`, line 331).
x86 alone splits those roles, so it runs the 646-line `remap_x86_abi` CFG fixpoint
(`src/arch/x86_64/select.rs:210`) to *re-infer* each operand's role from control flow.
That is bug-387: the "neutral" stream carries an ARM assumption, and x86 pays for it.

plan-71 tried to delete the fixpoint by re-tokenizing `%ret`→`%arg`. It succeeded for
the **index-0 single-role** sites but was **falsified for the indices-1..3 dual-role
error-Result values** (`planning/completed/plan-71-C-retokenize-family-1a.md` FINAL
STATUS): those values are a *result* and an *argument* at different points, and no
single overloaded token can express that. plan-85 fixes the vocabulary so the
ambiguity cannot arise — `%retMFB` where it is a result, `%argC` where it is an
argument, with an explicit staging move at the transition (plan-85-D) — and then
**aligns** MFB's convention so those transitions are hop-free at every index (only the
genuine C/kernel boundary keeps `rax`).

References:

- `src/target/shared/abi.rs` — `ARG`/`RET`/`SYSARG` token arrays (`:139`/`:146`/`:155`),
  `argument_register`/`return_register` (`:12`/`:95`), `realize_abi_token` (`:329`).
- `src/arch/x86_64/select.rs` — `map_token_direct` (`:168`), `remap_x86_abi` /
  `remap_x86_abi_inner` (`:210`/`:231`, ~646 lines), `select_x86` (`:917`), the banks
  `CALL_ARGS`/`RETS`/`SYS_ARGS`/`CALL_ARGS_WIN64`/`RETS_WIN64` (`:82`/`:90`/`:83`/`:116`/`:120`).
- `src/arch/aarch64/select.rs:106`, `src/arch/riscv64/select.rs:732` — the other two
  token-realizing backends.
- `src/target/shared/code/operand.rs` — the `Operand` enum (plan-78/79/82): the typed
  `VReg`/`Phys`/`Imm` arms and the `Raw(Box<str>)` fallback the ABI tokens still use
  (`:28`, `:71`); `Cow` rendering (`:114`). The `Operand::Abi` arm lands here.
- `planning/completed/` plan-78 / plan-79 / plan-82 — the typed-operand refactor
  (`CodeInstruction`/`MirInstruction.fields: Vec<(_, Operand)>`, −28.6% compile
  allocations) this finishes for the token category. Memory:
  `codeinstruction-operand-typing-and-regalloc-perf` (the compile is allocation-bound).
- `src/target/shared/code/selfmove_probe.rs` — the `MFB_BUG387_SELFMOVE` probe
  (plan-71-B) the staging-move elision (plan-85-D) relies on.
- `planning/completed/plan-71-*.md` — the byte-identity oracle mechanics
  (`scripts/bug387-gate.sh`, `scripts/exe-oracle.sh`), the `@src` tool, and the FINAL
  STATUS of why re-tokenization stalled (the design this supersedes).
- `planning/completed/plan-80-unified-resource-record.md` — the precedent for a
  byte-CHANGING codegen migration gated by "regenerate goldens + prove the delta is
  only the intended change + rt-behavior" (this plan's SysV-x86 gate model).
- `.ai/compiler.md` (silent-wrong-register is the worst class), `.ai/remote_systems.md`
  (the GTK Linux boxes for rt-behavior execution proof).

## Prerequisites

Stated once here for the whole plan-85 feature; every later letter points here.

| Must be true | Command | Status |
|---|---|---|
| plan-71 complete & archived (its tool + probe + arena work landed) | `ls planning/plan-71-*.md 2>/dev/null` → none; `ls src/target/shared/code/selfmove_probe.rs` exists | MET (re-verified c0c30e70a) — no `plan-71-*.md` in `planning/`; `selfmove_probe.rs` present |
| Repo builds clean; full byte-identity gate green at HEAD | `cargo build --release --bin mfb && bash scripts/bug387-gate.sh target/release/mfb full` (record fresh SERIAL baselines first) | MET (build) + IN-PROGRESS (gate): release rebuilt clean (38.62s); `app-ncode: byte-identical`; exe-oracle ×4 compare running against the fresh baselines (job `bvp6y25j6`) |
| exe-oracle baselines re-recorded from clean `main` **serially** | `for t in linux-x86_64 windows-x86_64 linux-riscv64 linux-aarch64; do bash scripts/exe-oracle.sh <exe> $t record /tmp/bug387/oracle-$t.txt; done` (one at a time) | MET — all 4 `/tmp/bug387/oracle-*.txt` recorded serially from the c0c30e70a-forked worktree (linux-x86_64 1354, windows 644, riscv64 1352, aarch64) |
| A Linux x86 box reachable for SysV-x86 rt-behavior execution proof | per `.ai/remote_systems.md` | MET — box 2227 (Alpine musl x86_64) reachable (`ssh -p 2227` → `Linux x86_64`); 2228 (glibc) timed out, recorded (plan-71 stance: available box stands) |
| No concurrent artifact-gate / exe-oracle running | `pgrep -f 'artifact-gate\|exe-oracle'` → empty | MET at gate start (no foreign gate); this plan's own finalize gate is the only exe-oracle now |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** The
> `/tmp/bug387/*` baselines are ephemeral and MUST be re-recorded from clean `main`
> **serially, one target at a time** — concurrent `exe-oracle` runs share
> `tests/*/build` and silently drop entries → phantom DIFF (memory
> `exe-oracle-concurrent-clobber`). If you stop, report the status of *all* rows.

Everything below is written against the world where these hold.

## 1. Goal

**plan-85-A goal** (checkable):

- The six tokens `%argMFB[0..7]`, `%retMFB[0..3]`, `%argC[0..7]`, `%retC[0..1]`,
  `%argSys[0..5]`, `%retSys` are defined in `abi.rs` and `realize_abi_token` (plus the
  riscv/x86 remaps) map every one to its **final aligned** physical register on all
  four backends (§2 table), verified by a unit test per token per backend.
- `select_x86` realizes an explicit token **directly** through `map_token_direct`
  (bypassing `remap_x86_abi`); legacy `%arg`/`%ret` still flow through the fixpoint.
  The two coexist (a within-plan migration seam, deleted in plan-85-D).
- A complete **classification census** (`planning/plan-85-census.md`) maps every
  emission site to one of the six conventions, counts stated with commands, no `~`.
- No emission site converted yet → `bug387-gate.sh full` byte-identical on all five
  targets (A is a dormant primitive).

**plan-85 overall goal (context, delivered across A–D):** `remap_x86_abi` deleted;
`%retMFB`=`%argMFB`=`[rdi,rsi,rdx,rcx]` on SysV; every operand a direct token lookup;
byte-identical on Win64/AArch64/RISC-V; byte-changed on SysV-x86 and proven
rt-behavior-equivalent.

### Non-goals (explicit constraints)

- **Any emitted byte, in plan-85-A.** A adds dormant tokens + the direct-realization
  seam + the census; nothing emits the new tokens, so all five targets stay
  byte-identical. (The feature's byte change begins in plan-85-B and is confined to
  SysV-x86.)
- **Converting emission sites (B/C), deleting the fixpoint (D).**
- **Win64/AArch64/RISC-V register assignments.** They are already aligned; the six
  tokens collapse onto their existing `xN`/`aN` there. Those targets never move.
- **The `.mfp` format, the `MFBABI` metadata hash, runtime semantics.** The `MFBABI\0`
  string (`src/binary_repr/sections.rs`) is the package-compat hash — unrelated;
  untouched.
- **The syscall register assignment.** `%argSys`/`%retSys` reproduce `SYS_ARGS`/`rax`
  exactly; the kernel ABI is fixed.

## 2. Current State

Shared builders emit `abi::ARG[k]`, `abi::RET[k]`/`return_register()`, `abi::SYSARG[k]`,
realized to AArch64 spellings by `realize_abi_token` (`abi.rs:329`), where `%argK`,
`%retK`, `%sysargK` collide to one `xN` (`:331`). AArch64 uses that directly; RISC-V
remaps `xN`→`aN`; x86 realizes to `xN` then runs the 646-line `remap_x86_abi` to
re-derive SysV/Win64 roles. The `%arg`/`%ret` overloading is the defect the six-token
vocabulary removes.

### Measured populations

| What | Count | Command |
|---|---|---|
| files emitting `abi::ARG[`/`abi::RET[` | 72 | `grep -rlE 'abi::(ARG\|RET)\[' src/target/shared/code/ \| wc -l` |
| MFB error-Result convention (`RESULT_*_REGISTER`, dual-role tail) | 884 (56 files) | `grep -rohE 'RESULT_(TAG\|VALUE\|ERROR_MESSAGE\|ERROR_SOURCE)_REGISTER' src/target/shared/code/ \| wc -l` |
| single-role emissions (arg 1,609 + ret 2,383 + sys 16, mechanical bulk) | ~4,008 | `planning/plan-85-census.md` |
| `remap_x86_abi` span (fixpoint to delete) | ~646 lines | `awk '/^fn remap_x86_abi/{s=NR} s&&/^fn /&&NR>s{print NR-s; exit}' src/arch/x86_64/select.rs` |

The **split-deciding census is complete** (`planning/plan-85-census.md`): the 884-site
error-Result convention is the dual-role tail → **plan-85-C**; the ~4,008 single-role
sites are the uniform bulk → **plan-85-B**; then **plan-85-D** (staging + elision +
fixpoint deletion). There is no separate alignment letter — the aligned realization
lands in A and takes effect as B/C convert (byte-changing SysV-x86).

### The register mapping the six tokens realize to (final, aligned)

| token family | SysV (linux/mac-x86) | Win64 (win-x86) | AArch64 | RISC-V |
|---|---|---|---|---|
| `%argMFB[0..3]` | rdi, rsi, rdx, rcx | rcx, rdx, r8, r9 | x0..x3 | a0..a3 |
| `%retMFB[0..3]` | **rdi, rsi, rdx, rcx** | rcx, rdx, r8, r9 | x0..x3 | a0..a3 |
| `%argC[0..3]` | rdi, rsi, rdx, rcx | rcx, rdx, r8, r9 | x0..x3 | a0..a3 |
| `%retC[0..1]` | **rax**, rdx | **rax**, rdx | x0, x1 | a0, a1 |
| `%argSys[0..5]` | rdi, rsi, rdx, r10, r8, r9 | (n/a) | x0..x5 | a0..a5 |
| `%retSys` | rax | (n/a) | x0 | a0 |

On SysV this collapses to **one aligned bank `[rdi,rsi,rdx,rcx]`** (every MFB arg, MFB
return, and C-call arg) plus **`rax`** (appears only in `%retC` and the syscall return).
`%retMFB` on SysV = `[rdi,rsi,rdx,rcx]` (no `rax`) is the byte-changing choice — it
differs from today's `RETS = [rax,rdx,rcx,rsi]`, so MFB result registers move.

### Verified properties

- **The `%arg`/`%ret` collision to one `xN` is the ARM assumption (VERIFIED —
  `abi.rs:331`).**
- **Win64 already aligns `RET[1..3]` with `ARG[1..3]` (VERIFIED — `select.rs:116/120`);
  the census shows Windows has ZERO index-1..3 divergences.** So Win64 (and ARM/RISC-V)
  do not move — the existence proof that aligned works and the migration's cross-target
  gate.
- **`%retMFB` never crosses to C as a 2-register value without an explicit `%retC`
  (UNVERIFIED — plan-85-B must confirm).** The SysV `rax:rdx` two-value C return, FFI,
  and `main`'s return must all use `%retC` (rax-exact); an MFB value that reaches those
  boundaries needs an explicit `%retC`→`%retMFB`/`%argMFB` shim. Auditing that no
  boundary silently relies on the old `RETS` layout is plan-85-B's first task.
- **The `@src` tool + `selfmove_probe` exist on main (VERIFIED — f3b62d29a).**

## 3. Design Overview

Three pieces; A builds the first and measures the third.

1. **The token vocabulary + aligned realization + the direct-realize seam (A).** Add
   the six families to `abi.rs`; teach `realize_abi_token`/`map_token_direct`/the riscv
   remap to map each per-target per §2 (aligned). Teach `select_x86` to realize an
   explicit token directly via `map_token_direct`, leaving `remap_x86_abi` to handle
   only the legacy `%arg`/`%ret` still present during migration. Dormant until B emits
   the tokens.

2. **The staged conversion (B single-role, C error-Result).** Convert emission sites to
   explicit tokens. On SysV this **changes bytes** (aligned registers), so each
   subsystem regenerates its SysV-x86 goldens and proves rt-behavior; Win64/ARM/RISC-V
   stay byte-identical (gate). Where an MFB value consumes a C/syscall return (`%retC`
   in `rax`), the conversion emits an explicit `%retC`→`%argMFB`/`%retMFB` staging move
   (rax→rdi). Where an MFB result feeds a C call, it is already in the aligned bank =
   the C arg register, so no move.

3. **The deletion (D).** Once no `%arg`/`%ret` remain, `remap_x86_abi` is deleted;
   `select_x86` realizes every token through `map_token_direct` in one pass. The
   explicit staging moves that land as `mov xN,xN` no-ops on AArch64/RISC-V are removed
   by the `selfmove_probe`-guided elision.

**Where design uncertainty concentrates (schedule FIRST — plan-85-B Phase 1):** the
`%retC` boundary audit — does anything (FFI, entry, a runtime helper, a 2-register C
return) silently depend on MFB returning in `RETS = [rax,rdx,rcx,rsi]`? If yes, those
sites need an explicit `%retC` shim before the aligned convention is safe. The audit is
the cheapest experiment that could falsify "the C boundary is cleanly separable."

**Where correctness risk concentrates (schedule LAST — plan-85-C/D):** the error-Result
staging and the fixpoint deletion, on the codegen path every x86 program uses, gated by
rt-behavior on a real Linux-x86 box + Win64/ARM/RISC-V byte-identity.

Rejected alternatives:

- *Keep `%arg`/`%ret`, re-tokenize (plan-71).* Falsified for dual-role values.
- *A disjoint MFB register set from C.* Impossible — 9 volatile registers, C claims 7,
  and arg/return registers are inherently volatile. Separate the conventions (tokens),
  not the registers.
- *A byte-IDENTICAL intermediate (two realizations, align later).* Considered and
  **dropped** (project decision): the aligned convention is adopted directly and gated
  on rt-behavior, accepting SysV-x86 golden regen from B onward rather than carrying a
  migration-only realization + `MFB_ALIGNED` switch.
- *Separate `%argWin`/`%retWin`.* Redundant — "C on Windows" is Win64; one `%argC`/
  `%retC` realizes per-platform.

## 4. Detailed Design

### 4.1 Token definitions — the typed `Operand::Abi` variant (`operand.rs` + `abi.rs`)
Add to the `Operand` enum (`operand.rs`) a `Copy` typed arm, and the two small enums it
carries:
```
enum AbiConvention { Mfb, C, Sys }        // Copy, Eq
enum AbiRole       { Arg, Ret }           // Copy, Eq
// in enum Operand:
Abi { convention: AbiConvention, role: AbiRole, index: u8 },   // Copy — no allocation
```
`render()`/`rendered()` resolve the spelling through a **static token-string table**
(a `&'static str` per (convention, role, index), like `Operand::Phys` carries its static
`name` — `operand.rs:123`), so `rendered()` returns `Cow::Borrowed` with **no
allocation**. This keeps `Abi` allocation-free on any read path and makes plan-85
complementary to **plan-83** (which eliminates owned-`render()` reads) rather than
re-introducing an allocating operand kind. Add
`abi::mfb_arg(k)`/`mfb_return(k)`/`c_arg(k)`/`c_return(k)`/`sys_arg(k)`/`sys_return()`
accessors returning `Operand::Abi{…}`. Keep the legacy `ARG`/`RET`/`SYSARG`/
`argument_register`/`return_register` (which still yield `Raw` strings) during migration;
they are deleted in plan-85-D once no site emits them.

### 4.2 Realization (typed match, final aligned registers)
Realization moves from string matching to a **typed** function
`realize_abi(convention, role, index, target) -> &'static str` returning the §2 register
(a `&'static [&'static str]` bank index — no allocation). Each backend matches
`Operand::Abi{…}` and calls it: AArch64/RISC-V collapse every family to `xN`/`aN`; x86's
`map_token_direct` (`select.rs:168`) returns the SysV/Win64 register per §2 (`%retMFB[k]`
→ aligned `[rdi,rsi,rdx,rcx]` on SysV). The legacy `realize_abi_token(&str)` stays for the
not-yet-converted `Raw` `%arg`/`%ret` until plan-85-D. No `MFB_ALIGNED` switch — aligned
is the only realization.

### 4.3 The direct-realize seam (`select_x86`)
In `select_x86` (`:917`), before deferring an operand to `remap_x86_abi`, check
`is_explicit_convention_token(tok)`; if so realize it immediately via
`map_token_direct(tok, abi)` and do NOT defer it. Legacy `%arg`/`%ret` defer to the
fixpoint as today. This is the seam plan-85-D widens to "everything direct, fixpoint
gone." Dormant in A (no explicit tokens emitted yet).

### 4.4 The census
Recorded in `planning/plan-85-census.md` (done). Per-operand refinement (the target
token + justifying boundary per `file:line`) is appended from the `@src`
`MFB_BUG387_AUDIT` sweep as B/C convert.

## Compatibility / Format Impact

plan-85-A: no emitted-byte or format change (dormant primitive; five-target
byte-identical). It **adds an `Operand::Abi` enum arm** — an internal compiler-data-
structure change with no observable effect (a typed operand renders to the same register
name, so emit is unchanged; plan-82's premise). Whole feature: the **SysV-x86
`.ncode`/executable byte layout changes** for register-using code (the aligned MFB
convention) — additive/mechanical, regenerated per plan-80's precedent and proven
rt-behavior-equivalent; and the ~4,900 ABI tokens stop allocating `Raw(Box<str>)` (a
compile-time allocation reduction, no output effect). Win64/AArch64/RISC-V byte-identical.
`.mfp` format, `MFBABI` hash, and all runtime semantics unchanged.

## Phases

> Keep the checkboxes current in the same commit as the work. An unticked box means
> NOT DONE.

### Phase 1 — the typed `Operand::Abi` variant + accessors (`operand.rs`, `abi.rs`)
- [x] Add the `Operand::Abi{convention, role, index}` arm + the `AbiConvention`/`AbiRole`
      enums (`operand.rs`); implement `render()`/`rendered()` for it; add the
      `mfb_arg`/`c_arg`/`sys_arg`/… accessors returning `Operand::Abi` (`abi.rs`). Leave
      the legacy string tokens in place. — `operand.rs` (enum arm + static token table
      `abi_token` + `Operand::abi`); `abi.rs` accessors; `code/mod.rs` re-exports
      `AbiConvention`/`AbiRole`. `Abi` payload is `Copy`; `rendered()` borrows the static
      spelling (no alloc). Legacy `ARG`/`RET`/`SYSARG` untouched.
- [x] Tests: `operand::tests` — `Operand::Abi` is `Copy`, round-trips through
      `render()`, and each accessor yields the expected variant. — `operand::tests`
      (`abi_tokens_render_and_borrow`, `abi_payload_is_copy_and_clones_without_alloc`,
      `abi_tokens_are_not_confused_with_legacy_or_vregs`) + `abi::tests`
      (`convention_explicit_abi_accessors`); all green.

Acceptance: `cargo test --bin mfb operand:: abi::` green; no emission site changed;
`bug387-gate.sh full` byte-identical (five targets). — cargo tests green (57 passed);
**`bug387-gate.sh target/release/mfb full` = PASS byte-identical on ALL 5 targets**
(app-ncode + linux-x86_64 1354 / windows 644 / riscv64 1352 / aarch64 1354 executables),
verified against the serial fresh baselines. Confirms the dormant primitive emits zero
byte change.
Commit: 817ddd32b

### Phase 2 — aligned typed realization (all four backends) + the direct-realize seam
- [x] Add `realize_abi(convention, role, index, target)` (§4.2) returning the §2 register;
      have each backend match `Operand::Abi` and call it — AArch64 (`select.rs:106`),
      RISC-V (`:732`), and x86's `map_token_direct` (SysV + Win64 columns, aligned `%retMFB`).
      — Shared positional realizer `abi::realize_abi_positional(index) -> &'static str`
      (`x{index}`; AArch64 args==results collapse) called from the AArch64 select loop and
      the RISC-V select loop (which then remaps `xN`→`aN`). x86's `realize_abi_operand`
      (`x86_64/select.rs`) returns the §2 aligned register per (convention, role, index, abi):
      MFB arg/ret + C arg → aligned `CALL_ARGS[k]`; `%retC` → `rax:rdx` (`C_RETS`); syscalls
      unchanged; Win64 on the `*_WIN64` banks. Kept as its own fn rather than folding into
      `map_token_direct` (which stays for the legacy `Raw` `%arg`/`%ret` until plan-85-D).
- [x] Add the `Operand::Abi` direct-realize branch in `select_x86` (§4.3) so an explicit
      token bypasses `remap_x86_abi`; legacy `Raw` `%arg`/`%ret` still defer to the fixpoint.
      — the seam matches `Operand::Abi` first in the per-operand loop, realizes via
      `realize_abi_operand`, and `continue`s before the `is_abi_role_token` deferral.
- [x] Tests: a realization unit test per (convention, role, index) per backend asserting
      the §2 register; a `select_x86` test proving an `Operand::Abi` bypasses the fixpoint.
      — `abi::abi_positional_realization_collapses_every_convention` (AArch64/RISC-V logic);
      `x86_64::select::realize_abi_operand_maps_to_aligned_registers` (full §2 SysV+Win64);
      `x86_64::select::explicit_abi_token_bypasses_the_fixpoint` (`%retMFB0`→`rdi`, no `rax`);
      `{aarch64,riscv64}::select::explicit_abi_tokens_realize_to_positional_{x,a}_registers`
      (end-to-end select path). All green; full `cargo test --bin mfb` = 3792 passed / 0 failed.

Acceptance: realization tests green on all four backends; `bug387-gate.sh full`
byte-identical (five targets — nothing emits `Operand::Abi` yet). — realization tests
green (all four backends); **`bug387-gate.sh full` = PASS byte-identical on all 5 targets**
(same run as Phase 1; Phase 1+2 are both dormant, one gate covers both).
Commit: f19a18bbf

### Phase 3 — per-operand census work-list
- [x] ~~Measure the split-deciding distribution~~ — done during planning
      (`planning/plan-85-census.md`: 884 / ~4,008 / 16, with commands).
- [x] Append the per-`file:line` target token + justifying callee/boundary from the
      `MFB_BUG387_AUDIT=1` `@src` sweep on a release build — the B and C work-lists.
      — Ran the sweep over 135 error-path fixtures on the A-only (byte-identical) release
      binary: 14,748 `BUG387-MISMATCH` lines across **273 distinct `@src` sites**, saved to
      `plan-85-audit-src.txt` and summarized in `plan-85-census.md` (§"MFB_BUG387_AUDIT @src
      sweep results"). The divergences are exactly the byte-changing MFB-result sites
      (`builder_error_emission.rs` dominant → C; entry/io_stdout/collection result staging →
      B); arg sites do not diverge (confirms Correction C1's byte-identical args). Combined
      with the complete grep work-list (`plan-85-worklist.md`), every site has a target token
      justified by its callee/boundary.

Acceptance: `planning/plan-85-census.md` carries a complete per-`file:line` work-list;
every site has a target token justified by its callee/boundary; counts carry commands.
— MET: `plan-85-worklist.md` (complete per-`file:line` enumeration, all categories, with
generating commands) + the `@src` sweep (273 divergent sites confirming the byte-changing
subset) + the deterministic target-token rule + per-file distribution, all in the census.
Commit: ebb9afdac

## Validation Plan

- Tests: `src/target/shared/abi::tests` (token + realization); the `select_x86` direct-
  realize branch has a unit test proving an explicit token bypasses the fixpoint.
- Coverage check: realization tests exercise every new token on every backend;
  `bug387-gate.sh full` PASS means nothing *covered* moved (A is dormant).
- Runtime proof: none for A (byte-identical primitive); the five-target gate IS the
  proof. rt-behavior proof begins in plan-85-B when bytes first move.
- Doc sync: `planning/plan-85-census.md`. Spec register-role text updated in plan-85-D.
- Acceptance: `cargo test --bin mfb` real `test result: ok`; `bug387-gate.sh <exe> full`
  PASS (five targets); `artifact-gate.sh` if no concurrent run.

## Open Decisions

- **`%retMFB` width / Result return** — MFB's `Result` uses 4 result registers; on SysV
  the aligned bank `[rdi,rsi,rdx,rcx]` supplies all four, but `%retC` is width-2 (the C
  ABI returns ≤2). Confirm no path returns a 4-register `Result` across a genuine C
  boundary (it would need marshalling). Recommend: the `%retC` boundary audit
  (plan-85-B Phase 1) settles it.
- **rt-behavior gate scope** — full remote Linux-x86 execution suite per subsystem vs.
  once at plan-85-D. Recommend: per-subsystem smoke on the converted area + one full
  run at D, since bytes move incrementally.
- **Typed `Operand::Abi` tokens vs. string tokens (the string-removal ride-along).**
  Recommend: **typed** — the tokens are the last register category still on
  `Operand::Raw(Box<str>)` (`operand.rs:28`), and plan-85 already re-touches all ~4,900
  sites, so typing them here **finishes plan-82's `Raw`→typed migration for tokens and
  cuts those per-compile boxed-string allocations** on the allocation-bound compile at
  no extra site-touch cost. The trade: it **widens plan-85-A** to the `Operand` enum and
  every `realize`/`map`/`@src`-render path (which take `&str` today), and the B/C/D gates
  now also cover the representation change. Accepted as worth it; recorded here so it is
  a deliberate scope choice, not a silent add. (Alternative: keep string tokens in
  plan-85 and type them in a separate later plan — rejected: that re-touches every site.)

## Corrections

**CORE-PREMISE FALSIFIED (plan-85-B execution) — the "Win64/AArch64/RISC-V byte-identical
cross-target gate" does not hold; the whole-feature verification model is invalid as
written.** This is recorded here as a **Prerequisites defect**: the entry gate proved only
that *A* is byte-identical (trivially true — A emits nothing), and never tested the
load-bearing premise that *converting emission sites* is byte-identical on the non-SysV
targets. A minimal conversion probe would have caught this before B committed to the
incremental structure.

Evidence (full `bug387-gate.sh full`, rebuild + exe-oracle ×4, on the A-only serial
baselines, after converting all `shared/code` single-role args to `%argC`):

| target | result |
|---|---|
| linux-aarch64 | **byte-identical** (OK 1354) |
| linux-x86_64 (SysV) | **534/1354 executables CHANGED** |
| windows-x86_64 (Win64) | **broadly CHANGED** |
| linux-riscv64 | **1352/1352 CHANGED** (universal; unexplained — needs objdump root-cause) |

Root cause (x86): `remap_x86_abi` colors every `xN` token by a **per-function GLOBAL
dataflow** (`defined_since_boundary`/`staged_live`/`param_home`, `select.rs:687-697`).
Converting *any* token makes it a physical register the fixpoint no longer tracks, so its
dataflow shifts and it re-colors the *remaining* legacy tokens — broadly byte-changing on
BOTH x86 ABIs. **Win64 runs the same `select_x86`/`remap_x86_abi`**, so the plan's premise
that Win64 is a byte-identity cross-check (§2, §Non-goals: "Win64-x86 … byte-identical …
their byte-identity is the migration's cross-target gate") is false. (Win64 also diverges
at result index 0 anyway: `%retMFB0`=rcx vs `RETS_WIN64[0]`=rax.)

**Consequence for B/C/D:** the plan as written cannot be executed — its correctness gate
(cross-target byte-identity) is invalid, so a byte-changing conversion has no sound way to
be verified on the two x86 ABIs. This needs **re-planning** (a write-plan task, outside
follow-plan's scope), not an in-place correction. A viable redesign must: (1) root-cause the
riscv64 universal diff (positional realization should be byte-identical like aarch64 — likely
a latent bug in the entry/arena conversion or the `Operand::Abi` riscv path, isolatable by
objdump of one fixture); (2) abandon the incremental "byte-identical args grouping" (there is
none on x86); (3) convert **all** tokens at once (or per-fully-isolated-function) so the
fixpoint has no partial state to perturb / no converted-vs-legacy register collision, which
— because shared results (`arena_alloc`'s `RET[0]/RET[1]` at ~795 sites) interconnect the
token graph — effectively merges B/C/D into one atomic conversion + `remap_x86_abi` deletion;
(4) verify by **rt-behavior on BOTH x86 ABIs** (SysV box 2227, Win64 box 2230) plus AArch64
(and RISC-V once fixed) byte-identity — NOT Win64 byte-identity.

plan-85-A itself (the typed `Operand::Abi` vocabulary + aligned realization + direct-realize
seam + census) stands and is byte-identical/verified; it is the correct primitive for the
redesigned conversion. The failed B args conversion was reverted (`fb54a6e61`); its findings
(C1–C4, inventories, worklist, `@src` sweep) are preserved in `planning/` for the redesign.

## Summary

plan-85-A adds the six-token vocabulary as a **typed `Operand::Abi` variant** (finishing
plan-82's `Raw`→typed migration for the last register category, so tokens stop
heap-allocating on the allocation-bound compile), realizes it to the final aligned
registers on every backend, installs the direct-realize seam the fixpoint deletion needs,
and records the complete classification census. It changes no emitted byte (dormant until
B). The risk it *removes* is plan-71's: with `%retMFB` and `%argC` distinct — and MFB's
convention aligned so they coincide on SysV except at the `rax` C boundary — a dual-role
value is expressible and hop-free, so the fixpoint deletion stops being blocked by the
error-Result residual. Untouched in A: every emitted byte, the `.mfp` format, the
`MFBABI` hash, and all runtime behavior.
