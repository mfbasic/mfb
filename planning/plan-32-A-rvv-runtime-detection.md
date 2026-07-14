# plan-32-A: RVV runtime detection (startup HWCAP probe → global flag)

Last updated: 2026-07-08
Overall Effort: x-large (1d–3d)
Effort: medium (1h–2h)
Depends on: nothing (plan-99 rv64 backend is landed on branch `riscv64`)

**Goal of the whole plan-32 feature: a *single* `linux-riscv64` binary that runs
on both V-capable and non-V RISC-V chips**, using native RVV vector code where
the hardware has it and the existing scalar path where it doesn't — chosen at
run time, not build time. This sub-plan lands the detection half: at program
start, probe the ELF aux vector for the "V" ISA bit and record it in a global
byte `_mfb_rt_has_rvv`, which the dual-path v128 lowering (sub-plan C) branches
on.

The single behavioral outcome: the same executable, run under
`qemu-riscv64 -cpu rv64,v=true` vs. `v=false` (and on real V / non-V silicon),
sets `_mfb_rt_has_rvv` to 1 vs. 0 respectively — proven by a probe program whose
exit code is the flag.

References:

- `src/target/linux_riscv64/code.rs:63` `entry_args_in_registers` (false — raw
  ELF entry: `argc` at `[sp]`, `argv` at `sp+8`; `envp` and the **aux vector**
  follow on the initial stack), `emit_program_entry` (`:148`).
- `src/target/shared/code/entry_and_arena.rs:19` `lower_program_entry` — the
  shared entry that reads `argc`/`argv` off the stack and initializes the arena
  before the language entry runs; the auxv scan slots in here (or a riscv hook).
- Linux `AT_HWCAP` (auxv key 16); RISC-V ISA letters map to HWCAP bits by
  `1 << (letter - 'A')`, so **`V` = bit 21** (`COMPAT_HWCAP_ISA_V`). (The newer
  `riscv_hwprobe(2)` syscall is an alternative; auxv is simpler and needs no
  syscall.)
- `.ai/remote_systems.md` (`ssh -p 2229` Alpine riscv64 musl); `.ai/compiler.md`.

## 1. Goal

- A one-byte global `_mfb_rt_has_rvv` (default 0), emitted as a data/BSS symbol
  by the linux_riscv64 module.
- Entry-time code (before the language entry) that walks the initial stack —
  past `argc`, the `argv` vector, the `envp` vector, to the auxv key/value
  pairs — finds `AT_HWCAP`, tests bit 21, and stores 0/1 into `_mfb_rt_has_rvv`.
  Pure loads + a syscall-free scan; no libc `getauxval`.
- Detection runs exactly once, at startup, so any later v128 dispatch is a cheap
  load of a settled byte.

### Non-goals (explicit constraints)

- **No v128 codegen change here.** The flag has no consumer yet; C wires it in.
  So all real output stays byte-identical except the added entry scan + symbol.
- **Do not touch other backends** or the shared entry's behavior for
  aarch64/x86_64 (guard the auxv scan to the riscv64 entry, or a no-op elsewhere).
- No `riscv_hwprobe` syscall dependency (keep to portable auxv); no per-thread
  re-detection (HWCAP is process-global).

## 2. Current State

- The riscv64 entry is a raw ELF entry: `entry_args_in_registers()` is false
  (`src/target/linux_riscv64/code.rs:63`), and `lower_program_entry`
  (`src/target/shared/code/entry_and_arena.rs:19`) already reads `argc` at
  `[sp]` and computes `argv` at `sp+8` before carving the frame — proving the
  initial-stack layout is reachable and the pattern for reading it exists.
- `envp` follows `argv`'s NULL terminator; the **aux vector** (key/value `u64`
  pairs, terminated by key `AT_NULL`=0) follows `envp`'s NULL — standard SysV
  layout, all reachable by loads from the entry `sp`.
- No CPU-feature detection exists anywhere in the codebase (plan-99's `Zbb`
  "feature flag" was never built — the encoder always expands). This is the
  first runtime capability probe.
- v128 currently always scalarizes (`src/arch/riscv64/v128.rs`); there is no RVV
  path to select yet.

## 3. Design Overview

Two pieces, both isolated to the riscv64 target:

1. **The flag symbol.** A 1-byte (padded) data object `_mfb_rt_has_rvv`,
   default 0, emitted by the linux_riscv64 module lowering when the entry
   references it (mirroring how `_mfb_rt_v128_slots` is emitted on demand).
2. **The auxv scan**, emitted into the riscv64 program entry after arena init,
   before the language entry:
   - `t = sp` (entry sp, before the frame is carved — capture it first).
   - `argc = [t]`; advance `t` past `argc` and the `argc` `argv` words and the
     `argv` NULL; then scan `envp` to its NULL.
   - Loop the auxv pairs: load `key,val`; if `key==AT_NULL` stop; if
     `key==AT_HWCAP(16)`, `has = (val >> 21) & 1`; store `has` byte to
     `_mfb_rt_has_rvv`.
   - Uses only entry-scratch GPRs (the entry already treats `x9/x10` as free;
     pick scratch that doesn't collide with the live `argc/argv` the language
     entry consumes).

**Risk:** low, but the scan must be exactly right about the stack layout
(off-by-one on the `argv`/`envp` NULL terminators would misread auxv) and must
not clobber the argc/argv the language entry still needs. Mitigation: an
exit-code **probe program** (below) run under both QEMU cpu profiles gives a
direct, end-to-end yes/no — the scan is either reading HWCAP correctly or it
isn't.

**Why auxv, not a build flag or IFUNC:** a build flag can't make one binary work
on both chips (the whole goal). IFUNC/function-pointer multiversioning needs a
callable kernel to swap, but the SIMD kernels are **inlined** into user code
(`builder_simd_float_math.rs:312` emits into the current function), so there is
no symbol to redirect. A settled global byte + an in-lowering branch (sub-plan
C) is the model that fits this codebase. (See C for the dispatch design.)

## Compatibility / Format Impact

- **Changed:** riscv64 binaries gain a startup auxv scan and a
  `_mfb_rt_has_rvv` data byte. Additive; no format/ABI change.
- **Unchanged:** aarch64/x86_64 entries; all non-riscv output; runtime behavior
  (nothing reads the flag yet).

## Phases

### Phase 1 — the flag symbol + a testable scan routine

Land the data symbol and the auxv-scan as an emitted entry step, wired to a
temporary exit-code probe so it is verifiable alone.

- [ ] Emit `_mfb_rt_has_rvv` (1 byte, default 0) from the linux_riscv64 module
      on demand (mirror `_mfb_rt_v128_slots` emission).
- [ ] Emit the auxv scan in the riscv64 program entry
      (`src/target/linux_riscv64/code.rs` `emit_program_entry`, or a guarded hook
      in `entry_and_arena.rs`): capture entry `sp`, walk argc/argv/envp to auxv,
      find `AT_HWCAP`, store bit 21 to `_mfb_rt_has_rvv`.
- [ ] Tests: a probe build whose program exits with the flag value; a selection/
      encoder unit test asserting the entry references `_mfb_rt_has_rvv` and the
      scan uses only entry-scratch registers (no argc/argv clobber).

Acceptance: the probe program, run under `qemu-riscv64 -cpu rv64,v=true`, exits
1; under `-cpu rv64,v=false`, exits 0. `scripts/artifact-gate.sh` byte-identical
for all non-riscv64 targets.
Commit: —

### Phase 2 — non-regression of the existing riscv64 suite

Prove the added entry step breaks nothing on the scalar path.

- [ ] Run the rt-behavior suite for `linux-riscv64` under QEMU (and `ssh -p 2229`
      if available) — the scan runs at startup for every program; confirm no
      regression in argc/argv-consuming programs.
- [ ] Tests: an argv-reading acceptance program still sees correct arguments
      (the scan must not disturb the argc/argv the language entry reads).

Acceptance: full riscv64 rt-behavior suite green with the scan present; argv
programs unaffected.
Commit: —

## Validation Plan

- Tests: the exit-code probe (both QEMU cpu profiles); an argv-integrity
  acceptance program; a unit test on the emitted entry (symbol reference +
  scratch discipline).
- Runtime proof: same binary, two cpu profiles, two exit codes — the direct
  demonstration that detection works and is runtime, not build-time.
- Doc sync: none yet (C/D document the user-visible portability guarantee).
- Acceptance: probe passes under both profiles; riscv64 rt-behavior suite green;
  non-riscv64 byte-identical.

## Open Decisions

- **Scan placement** — a guarded step inside shared `lower_program_entry`
  (one code path, `if arch==riscv64`) vs. a riscv64-only addition in
  `linux_riscv64/code.rs::emit_program_entry`. Recommend the latter (keeps the
  shared entry untouched for other arches). (§3)
- **`AT_HWCAP` vs. `riscv_hwprobe`** — recommend `AT_HWCAP` bit 21 (no syscall,
  works on every Linux that runs the binary). Revisit only if a needed sub-feature
  isn't reflected in HWCAP. (§1)

## Summary

The detection half of the one-binary-for-both goal: a syscall-free startup auxv
probe setting a global byte, verifiable by itself via a two-profile exit-code
probe. Low risk, and it gives sub-plan C the single settled bit it branches on.
