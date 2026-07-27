# plan-32-A: RVV runtime detection (startup HWCAP probe → global flag)

Last updated: 2026-07-27
Overall Effort: x-large (1d–3d)
Effort: medium (1h–2h)
Depends on: nothing (plan-99 rv64 backend is landed on `main`)

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

- `src/target/linux_common/code.rs:328` `entry_args_in_registers` (false — the
  raw-ELF linux entry shared by all three linux arches incl. riscv64: `argc` at
  `[sp]`, `argv` at `sp+8`; `envp` and the **aux vector** follow on the initial
  stack), `emit_program_entry` (`:414`). `linux_riscv64/code.rs` defines no entry
  of its own — it shares `linux_common::code`, so there is no riscv64-only
  `emit_program_entry` to extend.
- `src/target/shared/code/entry.rs:4` `lower_program_entry` — the shared entry
  that reads `argc`/`argv` off the stack and initializes the arena before the
  language entry runs; the auxv scan slots in here (or a riscv hook). (Was
  `entry_and_arena.rs` until bug-327 split it into 5 files.)
- Linux `AT_HWCAP` (auxv key 16); RISC-V ISA letters map to HWCAP bits by
  `1 << (letter - 'A')`, so **`V` = bit 21** (`COMPAT_HWCAP_ISA_V`). (The newer
  `riscv_hwprobe(2)` syscall is an alternative; auxv is simpler and needs no
  syscall.)
- `.ai/remote_systems.md` (`ssh -p 2229` Alpine riscv64 musl); `.ai/compiler.md`.

## 1. Goal

- A one-byte process-global `_mfb_rt_has_rvv` (default 0), emitted as a plain
  data/BSS symbol by the linux_riscv64 module. (HWCAP is process-global, so
  unlike the *per-thread* v128 slot region this is a genuine global — see §3.)
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
  (`src/target/linux_common/code.rs:328`, shared by every linux arch), and
  `lower_program_entry` (`src/target/shared/code/entry.rs:4`) already reads
  `argc` at `[sp]` and computes `argv` at `sp+8` before carving the frame —
  proving the initial-stack layout is reachable and the pattern for reading it
  exists.
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

1. **The flag symbol.** A 1-byte (padded) process-global data/BSS object
   `_mfb_rt_has_rvv`, default 0, emitted by the linux_riscv64 module lowering
   when the entry references it. (Note: the v128 slot region is *no longer* a
   global — bug-122 moved it into the per-thread arena state off `s11`, so the
   old `_mfb_rt_v128_slots` symbol is gone and there is nothing to "mirror". This
   flag is a genuine global because HWCAP is process-wide.)
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
(`builder_simd_float_math.rs` emits them into the current function — see the
`float_kernel_regs` / kernel-emission region), so there is
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

- [x] Emit `_mfb_rt_has_rvv` (1 byte, padded to 8, default 0) as a process-global
      writable data symbol, gated `module.entry.is_some() && module.target ==
      "linux-riscv64"` in `shared/code/mod.rs` (mirrors the `perf_state` gate);
      `HAS_RVV_GLOBAL_SYMBOL` const in `error_constants.rs`.
- [x] Emit the auxv scan in the riscv64 program entry — a riscv64-guarded step in
      the shared `lower_program_entry` (`entry.rs`), gated on
      `platform.arch() == "riscv64"` (Corrections A1): capture entry `sp`, walk
      argc/argv/envp to auxv, find `AT_HWCAP` (16), store bit 21 to
      `_mfb_rt_has_rvv`. Register-neutral `abi::` ops (scratch `t3`–`t6` + a
      transient `t0`; no callee-saved, no `ARG` argc/argv token).
- [x] Tests: a standalone exit-code probe faithfully reproducing the emitted scan
      (Corrections A4); a unit test (`riscv64_entry_carries_hwcap_probe_others_do_not`)
      asserting the riscv64 entry references `_mfb_rt_has_rvv`, shifts HWCAP by 21,
      stores the byte, touches no `ARG` token, and that aarch64/x86-64 entries
      contain none of it.

Acceptance: the probe program, run under `qemu-riscv64 -cpu rv64,v=true`, exits
1; under `-cpu rv64,v=false`, exits 0. `scripts/artifact-gate.sh` byte-identical
for all non-riscv64 targets. **MET** — probe exits 1/0 on 2232 (Corrections A3);
gate shows 0 non-riscv64 diffs from this change (24 macos-aarch64 diffs are
pre-existing stale goldens; 24 linux-riscv64 goldens regenerated for the scan).
Commit: <A1>

### Phase 2 — non-regression of the existing riscv64 suite

Prove the added entry step breaks nothing on the scalar path.

- [x] Run a `linux-riscv64` binary under QEMU (2232, `~/qemuroot` qemu-user) with
      the scan present — a trivial program exits 0 under both `v=true` and
      `v=false`; no crash/regression. Full `cargo test` green (the one macOS-TLS
      flake is bug-386, unrelated).
- [x] Tests: an argv-reading program (`os::args` echo) prints the correct
      `argc=N` and each argument under `v=true` (3 args), `v=false` (2 args), and
      native (1 arg) — the scan does not disturb the argc/argv the entry reads.

Acceptance: riscv64 binaries run green with the scan present; argv programs
unaffected. **MET** (Corrections A3).
Commit: <A1>

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

- **Scan placement** — a riscv64-guarded step inside shared `lower_program_entry`
  (`entry.rs`, one code path, `if arch==riscv64`) vs. a guarded step in
  `linux_common::code::emit_program_entry` (`:414`). The old third option (a
  riscv64-only addition in `linux_riscv64/code.rs::emit_program_entry`) is **gone**
  — that file no longer defines its own entry; it shares `linux_common`. Recommend
  the linux_common entry step, riscv64-guarded, so other arches stay untouched. (§3)
- **`AT_HWCAP` vs. `riscv_hwprobe`** — recommend `AT_HWCAP` bit 21 (no syscall,
  works on every Linux that runs the binary). Revisit only if a needed sub-feature
  isn't reflected in HWCAP. (§1)

## Corrections

- **A1 — scan placement: shared `lower_program_entry`, riscv64-gated.** Of the two
  Open-Decision options, `linux_common::code::emit_program_entry` (`:414`) only
  forwards to `shared/code/entry.rs::lower_program_entry`, so the scan lives in the
  latter (`emit_riscv_hwcap_probe`, called `if platform.arch() == "riscv64" &&
  !args_in_registers`). That is the one code path where the initial `sp` is still
  the kernel stack, and it keeps every other arch byte-identical. `CodegenPlatform`
  already exposes `arch()`, so no new trait hook was needed.
- **A2 — global emission site.** The plan said "from the linux_riscv64 module"; the
  runtime-global data objects are actually all emitted in `shared/code/mod.rs`
  (next to `MAIN_ARENA_GLOBAL_SYMBOL`/`PERF_STATE_SYMBOL`), so `_mfb_rt_has_rvv`
  is added there, gated `module.target == "linux-riscv64"`. Size 8 (padded), byte
  0 is the flag (the scan's `str_u8` / C's `lb`).
- **A3 — runtime affordance + golden regen.** No `qemu-riscv64` user-mode on the
  Mac (qemu-user is Linux-host-only) and **both** remote riscv64 boxes lack the V
  extension (`/proc/cpuinfo isa` has no `v`). Fetched qemu-user without root on
  2232 (Debian): `apt-get download qemu-user` → `dpkg -x … ~/qemuroot`. It emulates
  V and sets `AT_HWCAP` bit 21 under `-cpu rv64,v=true` (verified with a
  `getauxval` probe: native `0x112d`, v=true `0x20112d`). The scan legitimately
  changes riscv64 entry codegen, so the **24 `linux-riscv64` byte-identity
  `.ncodesum` goldens were regenerated** (single-cause: they were clean at HEAD,
  unlike the 24 `macos-aarch64` ones); the gate then shows only the pre-existing
  macos-aarch64 stale diffs. See [[rvv-two-profile-qemu-oracle]].
- **A4 — the exit-code probe.** `_mfb_rt_has_rvv` has no mfb-level consumer until
  C, so the "probe whose exit code is the flag" is a standalone riscv64 assembly
  program reproducing the emitted scan verbatim (the `.ncode` dump confirms mfb
  emits exactly that walk), run under both profiles (v=true→1, v=false→0). The
  real mfb binary's flag *value* is sealed transitively by C's two-profile parity;
  here the real binary is proven to run + keep argv intact under both profiles.

## Summary

The detection half of the one-binary-for-both goal: a syscall-free startup auxv
probe setting a global byte, verifiable by itself via a two-profile exit-code
probe. Low risk, and it gives sub-plan C the single settled bit it branches on.
