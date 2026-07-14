# plan-99 — rv64 Backend (RVA20 / RV64GC)

Last updated: 2026-07-07

The second new ISA and the one that *most* validates the design (no flags, no native SIMD).
Build a `MIR → rv64` backend targeting **RVA20 / RV64GC** (`G` = IMAFD incl. hardware FMA;
`Zbb` optional; RVV later), Linux first (`planning/old-plans/mir.md §1`, §12.3; §13 step 6 —
the MIR design doc was archived to `old-plans/` after A–G landed).

Depends on plan-00-A–G (all DONE) and reuses the x86_64 work's CI pattern from plan-00-H
(DONE — x86_64 emits **both glibc and musl** flavors, matching aarch64; rv64 must do the
same). Additive — must not touch other backends.

> Backends now span **two** trees (post plan-00-H): `src/arch/<arch>/` holds selection /
> encoder / ops / reloc, while `src/target/<os>_<arch>/` holds the `NativeBackend` impl +
> output shape, registered in the `NATIVE_BACKENDS` array (`src/target.rs`). Both dirs use
> the Rust `env::consts::ARCH` name (`aarch64`, `x86_64`, `riscv64`) — so this ISA is
> `src/arch/riscv64/`. A new ISA needs **both** trees — see §1.

## 1. Goal

- `src/arch/riscv64/`: selection (MIR → RV64GC), encoder, `RegisterModel` (32 GPRs x0–x31,
  32 FP regs f0–f31 — generous, so `arena_base` **pins** a register), the RV calling
  convention (a0–a7/fa0–fa7), ELF relocations, frame/prologue.
- `src/target/linux_riscv64/`: the `NativeBackend` trait impl (output shape — executable,
  NIR/native-plan/object-plan/code-plan/MIR writers) glibc **and** musl, added to the
  `NATIVE_BACKENDS` array in `src/target.rs`.
- **Target string is `-target linux-riscv64`**, not `linux-rv64`: `BuildTarget` derives
  `arch` from Rust's `env::consts::ARCH`, which is `"riscv64"` on RV64GC, and `is_host()` /
  `backend_for()` compare the `os`/`arch` strings exactly — so the backend's `target()` must
  advertise `arch: "riscv64"` for host detection and cross-target builds to work. The
  `src/arch/riscv64/` module name matches, keeping every `arch`/`target` identifier
  `riscv64` (the `rv64` label survives only as the ISA nickname in prose).
- The resolved hazards, rv64 flavor:
  - **Flagless is native:** `br_cc` → `beq`/`bne`/`blt`/`bge`/`bltu`/`bgeu` directly (no
    `cmp`); `set_cc` → `slt`/`sltu`+; `fbr_cc` → `feq.d`/`flt.d`/`fle.d` into a GPR + branch.
    `*_ovf` computed from sign/compare (no carry/overflow flag). *This is where the flagless
    MIR earns its keep.*
  - **`addr_of`** → `auipc; addi`; **`mov_imm`** → `lui; addi`/`li` materialization.
  - **FMA is base `D`:** `fma`/`fms` → `fmadd.d`/`fmsub.d`/`fnmadd.d`/`fnmsub.d` — no
    extension needed (the kernels' ≤1 ULP holds natively).
  - **Exotic ints — `Zbb` optional with base-G fallback:** `clz`→`clz`(Zbb) else loop/table;
    `rotr`→`ror`(Zbb) else shift-or; `rev`→`rev8`(Zbb) else byte-shuffle; `mulhi_*`→
    `mulh`/`mulhu` (base M). A target feature flag selects native-vs-expansion.
  - **`v128` → scalarized:** lower each `v128` op to 2× scalar `f64` (base G); RVV is a
    later optional backend. Correct, slower — accepted (§6).
  - **`arena_base`** → a pinned register (32 GPRs make it cheap); **`syscall`** → `ecall`
    (a7=nr, a0…).

### Non-goals

- RVV vector backend (later/optional), `Zbb` as a hard requirement, non-Linux. Other ISAs.

## 2. Current State

The MIR (after A–G) is neutral; the x86_64 backend (plan-00-H) proved the additive-backend
shape and the QEMU CI lane. rv64's distinctive demands are flagless branches (native) and SIMD
scalarization (no native SIMD on RV64GC).

## 3. Design

Selector + encoder + RegisterModel + ABI consuming MIR (`src/arch/riscv64/`); shared allocator
with the rv64 RegisterModel (pin `arena_base`). A **`v128`-scalarize pass** expands `v128`
ops to scalar `f64` pairs before/within selection. ELF writer + rv64 reloc table
(`R_RISCV_CALL`/`PCREL_HI20`/`PCREL_LO12`/`TPREL_*`). A `Zbb` feature flag (emit native or
expand). The `NativeBackend` impl in `src/target/linux_riscv64/` produces both glibc and musl
executables (mirroring `linux_x86_64`) and is registered in `NATIVE_BACKENDS`. QEMU-user rv64
CI lane covering both libc flavors.

## 4. Phases

1. Scalar core selection + encoder + RV ABI + frame + ELF relocs; `NativeBackend` impl in
   `src/target/linux_riscv64/` (glibc **and** musl flavors) registered in `NATIVE_BACKENDS`;
   `empty` + integer/string/collection under QEMU on both libc flavors.
2. Float + native FMA + flagless branches (`feq/flt/fle` + branch) + `f2i` (`fcvt.l.d`, rm);
   float/trap tests.
3. `v128`-scalarize pass; the kernels + `vector::`; **ULP harness on rv64** (native FMA must
   reproduce the plan-00-E contract).
4. `arena_base` pinned; `Zbb`-optional ints with base-G fallback; threads; signals; full
   suite green on rv64.

## 5. Validation

- Full runtime suite green on rv64 (QEMU-user CI), **both glibc and musl flavors** (matching
  the plan-00-H x86_64 lane) — behavioral parity; `_invalid` traps (codes + locations) match.
- **`runtime_ulp.py` ≤1 ULP on rv64** (base-`D` FMA; scalarized `v128` must still match the
  plan-00-E contract). nbody/mandelbrot/math values bit-identical to AArch64/x86_64.
- `Zbb`-on and `Zbb`-off builds both correct (the feature flag's expansion path is exercised).

## Implementation status (branch `riscv64`)

**Phases 1–2 landed and validated on real rv64 hardware** (Alpine musl, `ssh -p 2229`):
the full `src/arch/riscv64/` backend (regmodel, select, encoder, reloc, backend) +
`src/target/linux_riscv64/` (`NativeBackend`, glibc+musl) + `os/linux/link` rv64 ELF
(EM_RISCV, lp64d float-ABI flag, interpreters, `R_RISCV_*` relocs, `auipc/ld/jr` import
stubs, `auipc/jalr` + pcrel-hi/lo patching). Integers, floats (native FMA, `feq/flt/fle`
branches, `fcvt.l.d` rounding), strings, collections, control flow, functions, recursion,
and the int/float/`toString` formatters all **build and run correctly** (glibc+musl). The
acceptance suite: 207/225 rt-behavior tests build; the runtime failures are mostly
environmental (relative paths / clock / network absent on the remote), not codegen.

Key rv64-specific fixes made along the way: the shared allocator's **call-clobber masks**
were AArch64/x86-hardcoded and had to be generalized per-ISA (rv64 caller-saved live at
different physical indices); `li` 32-bit fast-path `lui` sign-extension; `add_ovf`/`sub_ovf`
must write `dst` before the branch; FP frame saves must use `fsd`/`fld`.

**Remaining (not yet done):**
- **`v128` scalarize (Phase 3).** The transcendental kernels + `vector::` emit ~30 `v128`
  ops on **physical** `v0`–`v31` (register-native), which rv64 (64-bit FP regs) must realize
  as register pairs or 16-byte memory slots — the "correct, slower" memory lowering the plan
  calls for, needing frame (or global-slot) cooperation. Blocks `math::sin/cos/exp/log/pow/
  atan2` and `vector::`. This is the largest remaining piece.
- **Threads (Phase 4).** All thread tests SIGSEGV: `thread::start`'s 120-byte control block
  is handed out by `arena_alloc` crossing past its arena block's page boundary
  (`badaddr` page-aligned = `cb + THREAD_OFFSET_INBOUND_QUEUE`). General allocation is fine
  (3000-item churn verified), so it is a thread-path arena block-boundary bug — needs
  `arena_alloc` debugging.
- **Bare `cmp`/`cmp_imm` (net/link/http).** Hand-written net/link helpers have a `cmp` whose
  flag-reading branch is non-adjacent (flags outlive intervening loads), so fusion misses it;
  rv64 has no flags. Needs a flag-register scheme (save operands at the `cmp`, emit `rv.br`
  at the standalone branch — `gp`/x3 is free). Blocks 17 builds (all need network/libraries
  to run anyway).
- **Exotic ints `clz`/`rbit`/`rev_w`/`rev_x` + `Zbb`.** Base-ISA fallbacks; no current test
  exercises them, so deferred.
- **QEMU/CI lane + `runtime_ulp.py`** (§5): validation was done by scp+run on the physical
  rv64 machine, not a QEMU CI lane; the ULP harness needs the `v128` path first.

## Summary

The backend that proves the MIR is genuinely portable, because rv64 stresses the two design
calls hardest: it has **no flags** (so the flagless MIR is the only reason branches lower at
all) and **no native SIMD** (so `v128` must scalarize). Hardware FMA being in base `D` means
accuracy is free; the only "missing" pieces (`clz`/`rotr`/`rev`) are cheap `Zbb`-or-expand.
If A–H were honest, this is one more `src/arch/riscv64/` selector plus a thin
`src/target/linux_riscv64/` backend registration — and the project is a three-ISA compiler
from a single frontend.
