# plan-179-B: Arena memory cap — startup RAM probe and startup check

Last updated: 2026-10-19
Effort: medium (1h–2h)
Depends on: plan-179-A

This sub-plan makes the program probe its physical RAM at startup, compute the
requested limit, and apply the 90% check: `halt` refuses to start, `warn` prints a
warning and runs. The computed limit is stored in a global. It is not enforced yet;
plan-179-C enforces it on the allocator.

References:

- `planning/plan-179-A-arena-cap-settings.md` — the keys, defaults and rules.
- `src/docs/spec/memory/08_program-startup.md` — the startup sequence the probe joins.
- `src/docs/spec/memory/04_arenas.md` — Arena-State Layout (`exitStatus` at +32; the entry shim zeroes `ARENA_STATE_SIZE`).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-179-A landed | `grep -n "^Commit:" planning/plan-179-A-arena-cap-settings.md` → a hash, not `—` | MET (2026-10-20: `Commit: 0ed6f7b2b`) |

Everything below is written against the world where this holds.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every command before continuing.

## 1. Goal

- A program built with defaults runs, and its output is unchanged.
- With `maxArenaMemoryPercent: 95` and `"halt"`, the program exits with status 1 before any program code runs, and stderr names the key and both byte counts.
- With `maxArenaMemoryPercent: 95` and `"warn"`, the program runs and stderr carries one warning line naming the key.
- The computed limit is in the global `_mfb_rt_arena_map_limit` (u64 bytes) for plan-179-C to read.

### Non-goals (explicit constraints)

- No allocator enforcement (plan-179-C). `_mfb_rt_arena_map_limit` is written, not read.
- No change to `ArenaState` layout. The new global is a data symbol.
- No change to the value of any existing program output.

## 2. Current State

- The entry shim is `lower_program_entry` (`src/codegen/engine/function/entry.rs:13`). It zeroes the whole `ARENA_STATE_SIZE` range before the first allocation (`docs/spec/memory/04_arenas.md`, Arena-State Layout). Existing startup helpers: `emit_default_arena_start_time` (`entry.rs:928`), `emit_riscv_hwcap_probe` (`entry.rs:857`), `emit_write_string_object` (`entry.rs:1225`).
- The shim's exit label is `entry_exit` (`entry.rs:228`). Teardown reads `exitStatus` at `ArenaState+32` (`04_arenas.md`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Real `emit_arena_map` platform implementations | 5 (macOS aarch64, Linux aarch64, Linux x86-64, Linux riscv64, Windows x86-64) plus the shared Linux helper | `grep -rln "fn emit_arena_map" src/target --include='*.rs'` → `macos_aarch64/code.rs`, `linux_aarch64/code.rs`, `win_x86_64/code.rs`, `linux_riscv64/code.rs`, `linux_x86_64/code.rs`, `linux_common/code.rs` |
| `GlobalMemoryStatusEx` references in the tree | 0 | `grep -rn GlobalMemoryStatusEx src \| wc -l` → 0 |
| `sysctlbyname` import on macOS | 1 (existing, used by `os.version`) | `grep -n "_sysctlbyname" src/target/macos_aarch64/plan.rs` → line 422 |
| `sysinfo` import on Linux | 1 (existing, used by `os.uptime`) | `grep -n '"os.uptime"' src/target/linux_common/plan.rs` → line 291 |
| `open`/`read`/`close` imports on Linux | present | `grep -n 'libc_import("close"' src/target/linux_common/plan.rs` → lines 317, 535 |

### Verified properties

- **The macOS probe can use an existing import with the existing call shape.** Read `os.version` (`src/codegen/builtins/os/func_version.rs:78`, call through `_sysctlbyname` imported at `src/target/macos_aarch64/plan.rs:422`). The probe calls `sysctlbyname("hw.memsize", &u64, &len, NULL, 0)` with the same shape.
- **The Linux probe can use the existing `sysinfo` buffer staging.** Read `os.uptime` (`src/codegen/builtins/os/func_uptime.rs:24–30`): `sysinfo(struct sysinfo *)` fills a caller-owned buffer that is staged from the stack pointer immediately before the call. The probe reads `totalram` (8 bytes at offset 32) and `mem_unit` (4 bytes at offset 104) from `struct sysinfo` (x86-64 and aarch64 LP64 layout; the layout is the kernel's `sysinfo(2)` struct, and the acceptance run on Linux confirms the value).
- **The Windows import needs one new entry, and the file format accepts it.** `src/target/win_x86_64/plan.rs:60–74` shows `import("Name", KERNEL32, "_start")`; `GlobalMemoryStatusEx` is a kernel32 export and has no existing reference in the tree (measured above).
- **The mutex primitive is available on all five targets** for plan-179-C: `emit_thread_external_call` maps `pthread_mutex_lock`/`unlock` to SRWLOCK on Windows (`src/codegen/runtime/thread/runtime_helpers.rs:61–67`).
- **The probe must run after the arena state is zeroed.** The main ArenaState lives on the entry stack and is zeroed by the shim (`04_arenas.md`, Arena-State Layout). The probe is therefore emitted after that zeroing loop and before the first allocation, so it can write `exitStatus` and use the stderr helpers.

## 3. Design Overview

Startup sequence after this change:

1. Existing shim: zero ArenaState, set up the arena, start time, and so on.
2. **New:** if `maxArenaMemoryDisable` is false, probe host RAM (per platform, §4.1), take the Linux cgroup minimum if present, compute `requested` and the 90% threshold, and apply `maxArenaMemoryCheck` (§4.2).
3. Existing: continue into the program.

Decisions made by the author (recorded under Decisions):

- Probe code is emitted only when the cap is enabled. A disabled build has no probe, no global writes, and no check.
- Integer math: the percent limit is `RAM × percent / 100` (floor); the threshold is `RAM × 90 / 100` (floor); the MiB limit is `MiB × 1048576`.
- A missing cgroup file is not a probe failure. It means the process is not in a memory-limited cgroup, and the host RAM stands.
- The percent key is always set (it has a default), so `requested` is always the percent limit, or the MiB limit if that is lower.

Rejected alternatives:

- Probe at build time. Rejected: the binary may run on a different machine than the one that built it.
- Probe via the `sysinfo` `freeram` field. Rejected: available RAM changes constantly, so the cap would shift under the program (decided in the earlier design discussion).
- Read `/proc/meminfo` on Linux. Rejected: it reports host RAM inside a container, so `sysinfo` plus the cgroup file is used instead.

## 4. Detailed Design

### 4.1 Per-platform RAM probe

- **macOS aarch64:** `sysctlbyname("hw.memsize", buf, &len, NULL, 0)` with `buf` an 8-byte u64 on the stack and `len = 8`. Success is return 0.
- **Linux (all three ISAs):** `sysinfo(buf)` with a 112-byte stack-staged buffer. Host RAM = `totalram × mem_unit`. Success is return 0.
- **Linux cgroup (all three ISAs):** open `/sys/fs/cgroup/memory.max` (cgroup v2), and if that is absent open `/sys/fs/cgroup/memory/memory.limit_in_bytes` (cgroup v1). Read the decimal text and parse it. The text `max`, or a number at or above host RAM, means no cgroup limit. Effective RAM = `min(host RAM, cgroup limit)`. A missing file means no cgroup limit.
- **Windows x86-64:** `GlobalMemoryStatusEx(buf)` with a 64-byte `MEMORYSTATUSEX` on the stack, `dwLength` (offset 0, u32) set to 64 before the call. Total RAM = `ullTotalPhys` (offset 8, u64). Nonzero return is success.

Probe failure: `sysinfo`, `sysctlbyname` or `GlobalMemoryStatusEx` returns failure. (A missing cgroup file is not a failure, §3.)

### 4.2 The check

Let `ram` be the effective RAM from §4.1.

- `requested = min(ram × percent / 100, mib × 1048576)`, where the MiB term is dropped when `mib` is `-1`.
- `threshold = ram × 90 / 100`.
- If `requested > threshold`:
  - `halt`: write one stderr line naming `maxArenaMemoryPercent` or `maxArenaMemoryMiB` (whichever produced `requested`), `requested` and `threshold` in bytes; set `exitStatus = 1` at `ArenaState+32`; branch to `entry_exit`. No program code runs.
  - `warn`: write one stderr line with the same content, prefixed with "warning:"; continue with `requested` as the limit.
- Otherwise the limit is `requested`.
- Store the limit in `_mfb_rt_arena_map_limit`.

Probe failure:

- `halt`: write one stderr line naming the probe that failed (for example `maxArenaMemoryPercent: could not read physical memory on this platform (sysinfo returned an error)`); exit 1 (as above), regardless of `maxArenaMemoryMiB`.
- `warn`: if `mib` is set, the limit is `mib × 1048576`, and the 90% check is not run; if `mib` is `-1`, the limit is `0` (no enforcement). Either way, write one warning line.

`0` is a valid stored value meaning "no enforcement". It cannot be a real computed limit, because `percent ≥ 1` and `ram` is at least a page, so `requested` is never `0` from a successful probe. (Verified by the range checks in plan-179-A.)

Messages use `emit_write_string_object` (`entry.rs:1225`). Integer formatting uses the existing decimal helper used by `emit_write_integer_to_stderr_with_labels` (`entry.rs:1259`).

## Compatibility / Format Impact

- Default builds gain a startup probe. Output and exit status are unchanged on a machine with ample RAM, because `requested` (50% of RAM) never exceeds the 90% threshold.
- The new global `_mfb_rt_arena_map_limit` is internal and does not appear in any public contract.

## Phases

### Phase 1 — Probe and startup check, all five targets

- [x] `src/codegen/engine/function/entry.rs`: new emitter for the probe, called from `lower_program_entry` after the ArenaState zero loop and before the first allocation, gated on `!arena_memory.disable` (§3). The probe is the vreg helper `_mfb_rt_arena_cap_init` (`src/codegen/runtime/arena_cap/mod.rs`); the entry (`entry/mod.rs`) calls it after the standard-fd reservation and before the `--debug` entry work, gated by `ProgramEntrySpec::arena_cap_init` (Correction 2).
- [x] `src/codegen/engine/function/entry.rs`: data symbol `_mfb_rt_arena_map_limit` (u64), written by the probe path. Emitted by `arena_cap::data_objects` from `module_layout.rs`.
- [x] macOS: `_sysctlbyname("hw.memsize")` probe (§4.1). Import already exists at `src/target/macos_aarch64/plan.rs:422`. That import is `os.version`'s; the entry now declares `_sysctlbyname` itself when the cap is on.
- [x] Linux (aarch64, x86-64, riscv64): `sysinfo` probe with the staging of `src/codegen/builtins/os/func_uptime.rs:24–30`. Import exists at `src/target/linux_common/plan.rs:291`. That is `os.uptime`'s; `entry_imports` now adds `sysinfo`/`read`/`close` when the cap is on.
- [x] Linux: cgroup v2 then v1 reads via `open`/`read`/`close`. Imports exist at `src/target/linux_common/plan.rs:315–317` and `:530–535`. Implemented as the `/proc/self/cgroup` walk of the first attempt's notes (`arena_cap/cgroup.rs`, Correction 3).
- [x] Platform call: a new `CodegenPlatform` method `emit_host_memory_probe` beside `emit_arena_map` (trait declared at `src/codegen/engine/types/types.rs:952–957`), implemented in `macos_aarch64/code.rs`, `linux_aarch64/code.rs`, `linux_x86_64/code.rs`, `linux_riscv64/code.rs` and `win_x86_64/code.rs`. `linux_common/code.rs` holds the shared Linux body, as it does for `emit_arena_map`. Plus `host_memory_probe_name`; the three Linux ISAs share the one `Platform<A>` impl in `linux_common/code.rs`, so no per-ISA file changed (Correction 4).
- [x] Windows: add `GlobalMemoryStatusEx` to the kernel32 import list in `src/target/win_x86_64/plan.rs` (style of lines 60–74). Gated on the cap being on.
- [x] `halt` and `warn` messages and exit path (§4.2). Probe failure checked for real on 2223 with an `LD_PRELOAD` `sysinfo` that returns -1: halt → the `could not read physical memory … (sysinfo failed)` line, rc=1; warn → warning, `hello`, rc=0; warn + MiB 64 → `the arena memory cap is maxArenaMemoryMiB, 64 MiB.`, rc=0.
- [x] `src/docs/spec/memory/08_program-startup.md`: add the probe step to the startup sequence and the halt/warn/probe-failure rules. New section "The Arena Memory Cap Check".
- [x] `src/docs/spec/memory/04_arenas.md`: note that `_mfb_rt_arena_map_limit` is written at startup (enforcement is in plan-179-C).

Acceptance: default build runs with unchanged output on all five targets; `halt` at 95% exits 1 with the message; `warn` at 95% runs with a warning.
  Check (macOS, local): `./mfb build <project> && <exe>; echo $?` with defaults → output as before, `0`.
  Check (macOS, local): project with `maxArenaMemoryPercent: 95`, `"halt"` → `echo $?` → `1`; stderr names `maxArenaMemoryPercent` and both byte counts.
  Check (macOS, local): same project with `"warn"` → runs, one warning line on stderr, `echo $?` → `0`. Manual, per the owner's decision: halt/warn are checked by hand.
  Check (Linux aarch64): box 2223 (`ssh -p 2223 test@127.0.0.1`, cargo at `/usr/bin/cargo`, `.ai/remote_systems.md`) — the same three checks, on a build made there.
  Check (Linux x86-64 musl): box 2227 — the same three checks.
  Check (Windows x86-64): the Win11 box at 192.168.1.206 (the only Windows box, `.ai/remote_systems.md`) — the same three checks.
  Check (goldens): `scripts/artifact-gate.sh <mfb> all` → diffs confined to the startup sequence of every fixture, on all five targets. Expected, because every default program now runs the probe. Inspect one fixture (`objdump` or the `.ncode` output) to localize the diff to the probe; then re-baseline only the entry-shim lines, with the one-fixture inspection in the commit message (AGENTS.md).
  Result (macOS, 96 GiB): defaults → `hello`, rc=0; 95% halt → `error: maxArenaMemoryPercent: the arena memory cap of 97925254348 bytes is above 90% of physical memory (92771293593 bytes); the program did not start.`, rc=1; warn → the `warning:` line, `hello`, rc=0; `maxArenaMemoryPercent: 100, maxArenaMemoryMiB: 95000` → the line names `maxArenaMemoryMiB`; `maxArenaMemoryMiB: 64` → runs.
  Result (Linux aarch64, 2223 Kali glibc): the same three → rc 0 / 1 / 0 with the line (RAM 8113708 kB). Under `systemd-run --user --scope -p MemoryMax=600M` the cap is 597688320 (= 600 MiB × 95%); with `MemoryMax=700M` set only on the parent `p179.slice` (the leaf reads `max`) it is 697303040 (= 700 MiB × 95%), so the parent walk works.
  Result (Linux x86-64 musl, 2227): the same three → 0 / 1 / 0 (RAM 16377832 kB; this box has no `/proc/self/cgroup` lines). Result (Linux riscv64 musl, 2229): 0 / 1 / 0.
  Result (Windows x86-64, 192.168.1.206): `p179_def.exe` → `hello`, rc=0; `p179_halt.exe` → the error line, rc=1; `p179_warn.exe` → `hello` + the warning, rc=0.
  Result (goldens, one fixture): `tests/byte-identity/strings`, macos-aarch64 `-ncode`: with `maxArenaMemoryDisable: true` the sha is the golden's (`409e026e…`); the default build differs only by additions — the `_sysctlbyname` import, the new data objects, the five-instruction entry call and the `_mfb_rt_arena_cap_init` function. The golden re-baseline is done once, after plan-179-C (Correction 5).
Commit: —

Landing tier (once, at the end of this sub-plan): `scripts/artifact-gate.sh <mfb> all` and a full `scripts/test-accept.sh`, with the golden diff described above.

## Validation Plan

- Tests: manual per the owner's decision (halt/warn checked by hand). The probe failure path is checked by reading the emitted code and by the one-fixture inspection.
- Coverage check: `artifact-gate all` covers the entry shim of every golden fixture, so the expected entry-shim diff must appear across the fixture set on all five targets. The landing run records the count of changed fixtures per target.
- Runtime proof: the eleven checks above, on macOS, Linux aarch64, Linux x86-64 and Windows x86-64.
- Doc sync: `08_program-startup.md`, `04_arenas.md`.
- Final gate (once, at the end): `scripts/artifact-gate.sh <mfb> all` and `scripts/test-accept.sh`.

## Decisions

- Halt and warn are tested by hand, not by an automated test — decided by the owner, 2026-10-19 (the startup-check test question).
- Probe failure follows `maxArenaMemoryCheck` — decided by the owner, 2026-10-19.
- `warn` honors the requested limit and prints a warning; it does not clamp — decided by the owner, 2026-10-19 (the "they can change it to warn and blow out the memory" reply).
- Cgroup v1 and v2 are both read; a missing file means no cgroup limit — author's call, recorded here. A probe that reads only v2 would silently ignore memory limits on v1 containers.
- Under `halt`, a probe failure exits regardless of `maxArenaMemoryMiB` — author's call. `halt` means "do not run when the cap cannot be checked".
- `_mfb_rt_arena_map_limit` is a data symbol, not an `ArenaState` field — author's call. It keeps `ARENA_STATE_SIZE` unchanged.

## Notes from the abandoned first attempt (2026-10-20)

A first attempt implemented this sub-plan on branch `worktree-P-179`; the owner discarded it (nothing merged). Findings for the redo:

- **Stale citations.** `src/codegen/engine/function/entry.rs` no longer exists; the shim is `src/codegen/engine/function/entry/mod.rs`, so every `entry.rs:NNN` line cite above must be re-measured.
- **The cgroup lookup in §4.1 is wrong both ways.** Root-only files (`/sys/fs/cgroup/memory.max`) miss a process in a nested cgroup (a systemd scope or user slice, as on box 2223). The attempt's replacement, the path from `/proc/self/cgroup` appended to `/sys/fs/cgroup`, misses containers that share the host's cgroup namespace (Docker on v1, `--cgroupns=host`, Kubernetes on v1): `/proc/self/cgroup` names `/docker/<id>`, but the container's own cgroup is mounted at `/sys/fs/cgroup`, so the composed path does not exist and host RAM is used silently. Recommended design: take the path from `/proc/self/cgroup`, then walk from that path up to `/`, trying the limit file at every level and keeping the smallest limit. That also picks up parent limits (a `MemoryMax` on a parent slice), and the final `/` step is the root file, which covers the container case. Run the v2 walk (`0::<path>`, `memory.max`) and the v1 walk (`N:memory:<path>`, `memory/…/memory.limit_in_bytes`) unconditionally: pure v2 has no `:memory:` line, pure v1 has no `0::` line, and on hybrid hosts the memory controller is on v1. The v1 root reports `9223372036854771712` (unlimited); treat any value of 10^18 or more as no limit.
- **Read all of `/proc/self/cgroup`.** The attempt read 511 bytes. On v1 and hybrid hosts the file has 12–13 lines, and the `:memory:` line (and, in hybrid mode, the trailing `0::` line) is often past byte 511. Use a buffer of at least 4 KiB, placed at the end of the probe frame so the scalar slots keep small offsets (riscv64 loads and stores encode offsets up to 2047 directly; larger ones are materialized).
- **"`requested` is never 0" (§4.2) is false.** A cgroup limit under 100 bytes, or 0, gives `ram × percent / 100 = 0`, which stores 0, meaning "no enforcement". Clamp the computed limit to at least 1, or treat it as a failure.
- **Order relative to `--debug` entry work.** The attempt's spec said the check runs before the `--debug` entry work, while the code ran it after. Decide which, and make the spec and the code match.
- **Unit-test fixtures carry hand-written entry import lists.** The inline tests in `src/target/linux_common/code.rs` (the `probe_entry` fixture used by `riscv64_entry_carries_hwcap_probe_others_do_not`) call `emit_program_entry` with their own import list. Adding `sysinfo`/`read`/`close` to the entry breaks them with "runtime helper requires sysinfo import". `scripts/test-accept.sh` and the artifact gate build through the real import plan and do not catch this, so run `scripts/test-fast.sh` before landing.
- **Expected golden churn:** the probe changes every executable fixture. The attempt regenerated 207 golden files (mostly `.ncodesum`); one inspected macOS fixture changed only by the added `_sysctlbyname` import and its relocation. Record per-target counts before regenerating; "0 diffs after regeneration" is not evidence.
- **Remote validation cost.** Box 2227 (Alpine x86-64 musl) is QEMU-emulated on this Mac. A `cargo test` build there was still compiling the `mfb` crate after about 78 minutes of rustc CPU time, and the attempt dropped that check. Budget for it, or plan an x86-64 row on another box. The attempt's cgroup scan ran only on aarch64 (2223), and the probe-failure path never ran on any target.
- **Shipping a tree from macOS:** plain `tar` adds AppleDouble `._*` files, which break `include_str!` on Linux ("wasn't a utf-8 file"). Use `git archive` or `COPYFILE_DISABLE=1 tar`.

## Corrections

1. **Stale citations.** The entry shim is `src/codegen/engine/function/entry/mod.rs`; the stderr helpers cited (`emit_write_string_object`, `emit_write_integer_to_stderr_with_labels`) are private to `entry/cleanup_audit.rs` and work in entry-stub scratch registers, so the helper has its own vreg versions (`Emit::write_string`, `Emit::write_decimal`).
2. **The probe is a vreg runtime helper, not entry-stub code.** The cgroup walk needs ~25 live values, a 4 KiB read buffer and a 1 KiB path buffer, which the scratch-register-only entry stub cannot hold. `_mfb_rt_arena_cap_init` is a vreg-allocated helper with a 5376-byte local buffer (`finalize_vreg_body_with_locals`); the entry calls it and turns a return of 1 into exit status 1 via `entry_exit`. Order: after the standard-descriptor reservation (so fd 2 exists) and before the `--debug` entry work — the open question in the notes, decided this way and stated in `08_program-startup.md`.
3. **Cgroup lookup (§4.1) replaced by the walk the notes recommend.** `/proc/self/cgroup` is read whole (up to 4095 bytes); the v2 line (`0::`) and any v1 line with a `memory` controller token are both walked from the process's path up to the root, keeping the smallest limit; `max`, a missing file, or ≥ 10^18 is no limit. Proven on 2223 for the leaf and the parent-slice case.
4. **Per-ISA Linux files unchanged.** `linux_aarch64/code.rs`, `linux_x86_64/code.rs` and `linux_riscv64/code.rs` implement `LinuxArch`, not `CodegenPlatform`; the one `impl CodegenPlatform for Platform<A>` in `linux_common/code.rs` carries the probe for all three. The probe has to be the trait's only per-platform piece; the cgroup walk is shared code in `arena_cap/cgroup.rs`, because it is the same on every ISA.
5. **Golden re-baseline deferred to the end of plan-179-C, measured here.** Every executable fixture changes again in plan-179-C (the grow path in `_mfb_arena_alloc`), so re-baselining here would rewrite the same files twice; the one-fixture inspection above localizes B's part of the diff. The `artifact-gate all` count for B is recorded below when the run finishes.
6. **"`requested` is never 0" (§4.2) was false**, as the notes said: a computed 0 is clamped to 1. `0` is stored only on a `warn` probe failure with the MiB key unset.
7. **Remote checks are cross-built.** Each Linux/Windows program was built on the Mac with `mfb build -target <t>` and copied over (`git archive`/cargo builds are not needed to run a program; box 2227 needs hours for a cargo build). The emitted code is the same either way: codegen does not depend on the build host.
8. **`test-fast.sh` passed** (fmt, check, build, skips, tests: all `ok`); the entry fixtures in `linux_common/code.rs` were not broken because their `ProgramEntrySpec` sets `arena_cap_init: false`.

## Summary

Highest risk: the Windows probe, which needs a new import and a stack struct in the entry path, and the cross-platform golden churn, because every program's startup changes. The churn is expected; the one-fixture inspection is what proves the diff is only the probe.
