# plan-183-A: Arena — fast zeroing, no fill of fresh memory

Last updated: 2026-10-24
Overall Effort: huge (> 3d) — plan-183 is split A → E, landed in letter order
Effort: medium (1h–2h)
Depends on: nothing

## plan-183 overview (shared by every letter)

The arena does not work the way it was designed to. It maps a block per request
(`max(4096, size + slack)`), so a large list gets one OS mapping per growth step;
blocks never merge, nothing is returned, and fresh blocks are zero-filled by hand.
`examples/weather` maps 3.64 GB for a 1.78 GB peak and spends 629 GB of zeroing in 70 s.

plan-183 replaces the block layer while keeping one invariant: **every byte not in
active use by the program is zero.** The letters, in order:

| Letter | Delivers |
|---|---|
| A | fast per-target zeroing; fresh OS memory is never filled by hand |
| B | `maxThreads`, `ErrThreadLimit`, one startup reservation cut into equal arena slices |
| C | three regions per arena, side bitmaps, free-list bookkeeping out of free memory, the new arena-state layout, block headers removed, committed-byte cap |
| D | `arena_extend`, so collections grow in place |
| E | decommit of long-free ranges, frontier retreat, `--debug` report rows, weather proof |

Owner decisions (2026-10-24), binding on every letter:
- Zero-on-free stays.
- No allocation directly from the OS per request; the runtime manages one reservation.
- Arena size is `min(20 GiB, reservation ÷ slices)`. Slices = `1 + graphics + 2 × maxThreads`, and all arenas are equal.
- `options.maxThreads` defaults to 10. A new error, `ErrThreadLimit`, is raised past it.
- Three regions per arena: small fixed, medium and huge growing toward each other.
- The arena-state layout may change as needed.
- Tuning values (confirmed by the owner):
  - 64 KiB small pages;
  - a huge tier at 32 MiB and up;
  - 4 MiB commit steps;
  - a 4 MiB minimum decommit run;
  - a 256 MiB per-arena reservation floor;
  - `maxThreads` 1–4096.

Remote boxes (`.ai/remote_systems.md`):

| Box | Platform | Use |
|---|---|---|
| 2223 | aarch64 glibc | preferred Linux box |
| 2227 | x86_64 musl | emulated; only for x86_64-specific checks, keep runs short |
| 2229 | riscv64 musl | riscv64 |
| `test@192.168.1.206` | Windows | the only Windows box |

## Goal

Freed memory is zeroed by a fast per-target routine, and freshly mapped memory is
never zeroed by hand. The visible effect: `fill_grow_bytes` is 0, and freeing costs
less CPU.

### Non-goals

- Allocation and free algorithms, block layout and the cap are unchanged (B–E change them).
- Zero-on-free stays.

## Current state (read 2026-10-24)

- `src/codegen/memory/arena/arena/mod.rs:lower_arena_fill_zero` stores 8 bytes at a time, 32 bytes per iteration.
- `src/codegen/memory/arena/arena/alloc.rs:lower_arena_alloc` grow path calls it on every freshly mapped block (`04_arenas.md` step 5, "calls `arena_fill_zero` to zero the freshly mapped block").
- `free.rs:lower_arena_free` calls it over `[ptr+16, ptr+size)` on every free.
- Weather (`mfb build --debug`, 70 s): `fill_grow_bytes` 3,641,109,184; `fill_free_calls` 260,347,971; `fill_free_bytes` 629,284,695,184.
- Fresh anonymous memory is zero on every target: `mmap(MAP_ANONYMOUS)` on Linux and macOS, `VirtualAlloc(MEM_COMMIT)` on Windows.

## Design

- **AArch64** (macOS, Linux aarch64):
  - At startup, read `DCZID_EL0` once into a runtime word, `_mfb_rt_dczva_bytes`. If its DZP bit (bit 4) is set, `dc zva` is prohibited and the word stays 0. Otherwise the word is `4 << BS` (the block size).
  - `fill_zero` with a non-zero word: plain stores up to block alignment, `dc zva` per block, plain stores for the tail. With 0: a 64-byte `stp xzr, xzr` loop.
- **x86-64** (Linux, Windows): `rep stosb`.
- **riscv64:** an unrolled loop of 8 `sd zero` stores, 64 bytes per iteration.
- **Grow path:** drop the `arena_fill_zero` call; the frame stays, because other calls remain.

## Phase A1 — Fast zero routine, no fresh-block fill

- [x] `mod.rs:lower_arena_fill_zero`: the three per-arch bodies above. New ops `mrs_dczid`/`dc_zva` (AArch64) and `rep_stosb_zero` (x86-64) through `ops.rs`, `mir.rs`, `code_impl.rs`, both encoders. `cargo test --bin mfb zero_fill_tests` → 3 passed; `cargo test --bin mfb -- encodes_rep_stosb_zero encodes_mrs_dczid mnemonic_round_trips` → 3 passed (AArch64 words checked against Apple clang 17 `otool -t`: `d53b00e3 d53b00f5 d50b7425 d50b743c`).
- [x] Startup: the `DCZID_EL0` read into `_mfb_rt_dczva_bytes` (AArch64 entry and the `--app` program entry), with the DZP check. One site, `lower_program_entry`, serves both (`module_layout.rs` passes `entry_called_as_function: true` for app mode). Proof the `dc zva` path runs: a one-off mutation that stepped two blocks per `dc zva` made the reuse program crash on macOS, and `/tmp/dczid` reads `DCZID_EL0=0x4 DZP=0 block_bytes=64` on the macOS host and on box 2226.
- [x] `alloc.rs`: remove the grow-path fill. Smoke run: `arena.0.grow 7`, `arena.0.fill_grow_bytes 0`.
- [x] `04_arenas.md` *Zero Fill* and step 5: the OS-zero guarantee per target; the new routine. Also `09_debug-report.md`, the instruction-set topics 14/15/17 and `mfb man debug report`. `cargo test --bin mfb spec` → 72 passed (citations resolve).
- [x] Test: new `tests/runtime/rt_arena_zeroing.rs` — free chunks of 24 B, 3 KiB and 5 MiB, reallocate the same sizes, and assert every byte past the first 16 is zero (C tightens this to all bytes); assert the `--debug` report's `fill_grow_bytes` is 0. The program frees a 32 B string chunk, a 360-element and a 655,360-element list each iteration (`hit_quick_bin + hit_large_bin ≥ 117`). A mutation that dropped the free-path fill gave `zero_violations 118`.
- [x] Added: the `MFB_ARENA_CHECK_ZERO` test switch of a `--debug` build (read once at the main arena's registration with `getenv`/`GetEnvironmentVariableA`; `arena_alloc` scans every chunk it hands out; report rows `zero_checks`, `zero_violations`). It is how a test sees freed bytes at all; `rt_debug_arena.rs`'s counter list gains the two rows.
- [x] Added: `tests/common/remote.rs` — `MFB_TEST_REMOTE=<target>,<ssh dest>[,<port>[,<libc>]]` cross-builds a test's program and runs it on a box (or `docker:<image>` in a container of the target architecture), with the assertions on the Mac. `debug_report::build_debug*`/`run_ok` and `rt_arena_large_block_reuse`'s runs route through it.
- [ ] Added (from Validation): the goldens — baseline gate at main, the full gate on this branch, `scripts/regen-native-goldens.sh`, the gate back to 0, and a full `scripts/test-accept.sh` with no `.run`/`build.log` change.

Acceptance: `cargo test --test rt_arena_zeroing --test rt_arena_large_block_reuse` passes on macOS, 2223, 2227 (x86_64 for `rep stosb`; short run) and the Windows box (est. 3 min each, 2227 longer: emulated).
Measured 2026-10-24 (`MFB_TEST_EXE=target/debug/mfb`): macOS 2+3 passed; box 2226 (`MFB_TEST_REMOTE=linux-aarch64,test@127.0.0.1,2226,glibc`) 2+3 passed; x86_64 musl (`MFB_TEST_REMOTE=linux-x86_64,docker:alpine,,musl`) 2+3 passed; Windows box (`MFB_TEST_REMOTE=windows-x86_64,test@192.168.1.206`) 2+3 passed. Box 2227, once back up (`MFB_TEST_REMOTE=linux-x86_64,test@127.0.0.1,2227,musl`): 2+3 passed. 2223 stayed down — see Corrections.
Commit: —

## Validation

- `bash scripts/test-fast.sh`.
- Codegen goldens: every binary links `fill_zero`, so every `.ncodesum` is expected to move. `scripts/artifact-gate.sh target/release/mfb all`, then regenerate. Per AGENTS.md, the full acceptance run must pass with no `.run`/`build.log` change before the re-baseline commit, run when the host is free.
- Doc sync: `04_arenas.md`.

## Corrections

1. **Boxes 2223 and 2227 were unreachable** (2026-10-24: `ssh -p 2223` and `-p 2227` → `Connection refused`; 2228 too; `utmctl` cannot start UTM VMs from this session). Substitutes, same architecture and libc: box 2226 (Debian 12 aarch64 glibc) for 2223, and a local `linux/amd64` Alpine container under qemu-user (`docker run --platform linux/amd64 alpine uname -m` → `x86_64`; the same TCG emulation class as 2227) for 2227's x86_64 musl row, run through the new `MFB_TEST_REMOTE` harness. The real x86_64 `rep stosb` row is the Windows box (Coffee Lake hardware). 2227 came back later the same day and the pair passed there too; 2223 was still refusing connections.
2. **The no-`dc zva` fallback is eight `str xzr` per 64 bytes, not `stp xzr, xzr`.** The code-op vocabulary has no store-pair op. The fallback runs only where `DCZID_EL0` has DZP set or a block outside 16–2048 bytes; measured `DCZID_EL0=0x4` (DZP 0, 64-byte blocks) on the macOS host and on box 2226, so no supported AArch64 box runs it. riscv64 uses the same generic loop, as the design says.
3. **How the test sees freed bytes.** Nothing could read a reused chunk's bytes from outside the program, so A added the `MFB_ARENA_CHECK_ZERO` `--debug` switch (two report rows) and the remote-run harness — both listed as tasks above.
4. **A-era limit of the "past the first 16 bytes" property.** A flush that merges two free chunks leaves the absorbed chunk's 16-byte free-list node inside the merged one, so a chunk carved from a merged run can carry a non-zero word in its middle. The smoke program `/tmp/p183/smoke` (2,000 growing lists) measured `arena.0.flushes 4`, `zero_violations 1`. The A test's program never flushes (`flushes 0`). C removes free-list nodes from free memory and tightens the test to every byte of every chunk.
5. **`rt_arena_large_block_reuse` assumed `os::args` costs no arena memory.** On Windows the entry builds `os::args` in the arena (`emit_build_argv_utf8`), so the run with an argument held 32 more peak live bytes (`left: 69296, right: 69328` on the Windows box) and `varying_large_sizes_map_no_more_than_a_fixed_size`'s equal-peak assertion failed there; it had never run on Windows before. Both runs now pass one five-letter argument (`drift`/`fixed`), so the assertion is unchanged and true on every target.
6. **`mfb man debug report` still said fills write random bytes** (the fill became a zero fill in `4bb55ea8c`). Corrected with the new rows.
