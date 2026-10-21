# plan-174-E: `mfb dbg` on riscv64 — tests, man topic, closeout

Last updated: 2026-10-12
Effort: large (estimate: 3h–1d, most of it waiting on riscv64 builds)
Depends on: plan-174-D

Closes plan-174. `tests/cli/cli_dbg.rs` drives real debugger sessions and runs
today on macOS and Linux AArch64 only; this sub-plan makes it run on riscv64
Linux, adds the man topic the owner chose for riscv64, and runs the final gate
for the whole feature. The outcome: `cargo test --test cli_dbg` passes on box
2229 with 16 tests, the same suite still passes on macOS and box 2226, and
`mfb man debug riscv64` renders.

References:

- `tests/cli/cli_dbg.rs`; `tests/common/mod.rs:mfb_exe` (the release compiler
  the tests drive, and its `MFB_TEST_EXE` override).
- `.ai/man-content.md` (§4 the memory vocabulary, §9 guide topics, §9.2a2 the
  debug carve-out, §9.3 verifying a topic).
- `src/docs/man/debug/package.md` (the topic list) and `01_start.md`.
- `.ai/testing-gates.md` (the full suite runs in a dedicated worktree).

## Prerequisites

The prerequisites are stated once in plan-174-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-174-D is complete | `ls planning/plan-174-D-*` → no match | NOT MET (2026-10-12) |
| A release `mfb` builds on box 2229 | on 2229: `cargo build --release --bin mfb` → `Finished` | NOT MET (2026-10-12: started on 2229 at `c0a48dd4f` and still compiling dependencies when this plan was written; the debug build took 45 min 36 s) |

If plan-174-D is not complete, this plan cannot start.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue. If you stop, report all of them.

## 1. Goal

- `cargo test --test cli_dbg` on box 2229 → 16 passed, 0 failed.
- The same command on macOS → 18 passed, and on box 2226 → 16 passed.
- `mfb man debug riscv64` renders the new topic and is in the topic list.
- The full suite passes once, and plan-174-A to E are archived.

### Non-goals (explicit constraints)

- No debugger behaviour changes. A test that fails on riscv64 because the
  debugger is wrong is fixed in the debugger with a test first; an expectation
  is changed only where it names an AArch64 register or instruction.
- No expectation that macOS or Linux AArch64 checks today is weakened: every
  string those hosts assert stays asserted on them.
- The existing man pages keep their AArch64 samples and wording. The two
  sentences that say where `mfb dbg` runs were changed in plan-174-D.
- `scripts/man-debug-examples.sh` is not extended to riscv64. Its Linux box is
  AArch64 and it runs fenced blocks tagged `sh linux`; the new topic's session
  is captured by hand from box 2229 and carries no tag.

## 2. Current State

`tests/cli/cli_dbg.rs` has 19 tests:

| Gate | Tests |
|---|---|
| none | `help_and_usage_errors_on_every_host` |
| `not(any(macos, all(linux, aarch64)))` | `the_debugger_is_not_supported_off_macos_and_linux_aarch64` |
| `mod native`, `any(macos, all(linux, aarch64))` | 17 |
| of those, macOS only | `an_unsigned_mfb_explains_how_to_sign_itself`, `an_app_s_threads_are_the_app_and_mfb_main` |

On a riscv64 host only the first two compile in today.

AArch64 spellings in the 15 tests Linux runs, each in a test body or helper:

- `c_library`: `ld-musl-aarch64.so.1`; `host_image`: `ELF aarch64`.
- `list_shows_a_launched_debug_build_s_functions_and_registers`: the system
  register list `fp, lr, sp, pc, nzcv, fpsr, fpcr`, `v0`–`v31` at 32 hex digits,
  and the role table `x0`–`x7`, `x19`, `x20`, `x28`.
- `show_type_reads_a_returned_string_from_the_arena`: `show type string x1`.
- `a_watchpoint_stops_when_a_thread_changes_the_watched_bytes`: the whole test.
- `a_breakpoint_with_commands_runs_them_and_runs_on`: `list reg x1` and its row.
- `a_tested_breakpoint_stops_once_and_a_jump_returns_early`: `bpi x0 eq 824084`,
  `j lr x0=0 x1=7`.
- `run_stops_at_breakpoints_and_at_the_end`: ` sub    sp, sp, #0x`, `ret`,
  `list reg x0 sp`, `: x0 = 0x… (0), x1 = 0x`.
- `show_error_names_the_fault_a_thread_stopped_on`: `j pc sp=16`, which is the
  same on riscv64 (`sp` is `sp`, and the prologue stores through it).

The tests drive a release compiler: `mfb_exe` builds `cargo build --release
--bin mfb` once, or takes `MFB_TEST_EXE`.

`package.md`'s "Topics" list has eight entries, one per numbered file
`01_start.md` to `08_report.md`. The pages' sentence about where `mfb dbg` runs
names riscv64 after plan-174-D.

### Measured populations

| What | Count | Command |
|---|---|---|
| Tests in `cli_dbg.rs` | 19 | `grep -c '#\[test\]' tests/cli/cli_dbg.rs` → 19 |
| Tests that pass on macOS | 18 | `cargo test --test cli_dbg` → `18 passed` (2026-10-12, 135 s) |
| Lines of the man debug pages that use an AArch64 spelling | 107 | `grep -cE '\bx[0-9]+\b\|nzcv\|AArch64\|\blr\b' src/docs/man/debug/*.md` summed → 107 |
| Topic files and topic list entries | 8 and 8 | `ls src/docs/man/debug/0*.md \| wc -l`; the "Topics" list in `package.md` |
| A clean debug build of `mfb` on box 2229 | 45 min 36 s | `cargo build` there, 2026-10-12 |

### Verified properties

- **`MFB_TEST_EXE` lets the tests use a compiler built beforehand** (read
  `tests/common/mod.rs:mfb_exe`: it returns the variable's value before looking
  for or building anything), so the long release build on riscv64 runs once, by
  hand, outside the test's timeout.
- **The fault test's trick holds on riscv64.** `_mfb_fn_isPrime` begins `addi
  sp,sp,-208` then `sd ra,0(sp)` (plan-174-C §2), so with `sp` set to 16 the
  second instruction stores to an unmapped address.
- **The man renderer finds topics by file name and the census checks the
  list.** `.ai/man-content.md` §9.3: `scripts/man-census.sh --topics` fails when
  a topic's rendered page count and markdown file count disagree.

## 3. Design Overview

**Tests.** `mod native`'s gate widens to `any(macos, all(linux, any(aarch64,
riscv64)))`, and the not-supported test's narrows by the same term. Inside, one
small helper module names the host's registers and instruction spellings:

```rust
mod cpu {
    #[cfg(target_arch = "aarch64")]
    pub const ARG0: &str = "x0";      // riscv64: "a0"
    pub const ARG1: &str = …;         // "x1" / "a1"
    pub const LINK: &str = …;         // "lr" / "ra"
    pub const SYS: &[&str] = …;       // ["fp","lr","sp","pc","nzcv","fpsr","fpcr"] / ["sp","pc","fcsr"]
    pub const ROLES: &[(&str, &str)] = …; // the bare `list reg` table
    pub const PROLOGUE: &str = …;     // " sub    sp, sp, #0x" / the riscv64 `addi` spelling
    pub fn names() -> Vec<String>;    // every register `list reg gpr sys simd` prints, in order
    pub fn digits(name: &str) -> usize; // 32 for an AArch64 vector register, else 16
}
```

Each test that spelled a register or instruction uses these. On AArch64 every
constant is the literal the test has today, so those hosts assert exactly what
they assert now.

The watchpoint test is compiled out on riscv64, and a riscv64-only test asserts
the refusal. That keeps the count explicit: 15 native tests on Linux AArch64, 15
on riscv64 (14 shared plus the refusal), plus `help_and_usage_errors_on_every_host`.

**Man topic.** `src/docs/man/debug/09_riscv64.md`, reachable as
`mfb man debug riscv64`, for the MFBASIC developer on a riscv64 machine: the
register names, which registers carry arguments and results, what `list reg`
shows, that instructions are 2 or 4 bytes, that `wp` is not available and what
to use in its place, and one real session.

**Correctness risk** is low and lies in the test edits: a helper constant with
the wrong AArch64 literal would silently change what macOS and box 2226 assert.
The gate is that both still pass and that `git diff` for the test file shows
each replaced literal reappearing as an AArch64 constant.

**Byte-identity is not this sub-plan's gate.** It is gated by the three test
runs and the rendered topic.

Rejected alternatives:

- **A separate `cli_dbg_riscv64.rs`.** It would duplicate 15 session scripts and
  let the two drift.
- **Run the unit tests on box 2229 as well.** They are host-independent and run
  on the Mac; a test build of the whole compiler there costs another build of
  the size measured above for no added coverage.

## 4. Detailed Design

### 4.1 `tests/cli/cli_dbg.rs`

- `c_library`: `ld-musl-riscv64.so.1` under musl on riscv64. `host_image`: `ELF
  riscv64`.
- `the_debugger_is_not_supported_…`: renamed
  `the_debugger_is_not_supported_off_its_three_hosts`, gate as §3, message as
  plan-174-D §4.3. (Done in plan-174-D Phase 3, with the message it tests.)
- `list_shows_…_functions_and_registers`: names from `cpu::names()`, widths from
  `cpu::digits`, roles from `cpu::ROLES`.
- `show_type_…`: `show type string {ARG1}`.
- `a_breakpoint_with_commands_…`: `list reg {ARG1}` and a row built from it.
- `a_tested_breakpoint_…`: `bpi {ARG0} eq 824084`, `j {LINK} {ARG0}=0 {ARG1}=7`.
- `run_stops_at_breakpoints_…`: `cpu::PROLOGUE`, `ret`, `list reg {ARG0} sp`,
  `: {ARG0} = 0x… (0), {ARG1} = 0x`.
- `a_watchpoint_stops_…`: `#[cfg(not(target_arch = "riscv64"))]`.
- New, `#[cfg(target_arch = "riscv64")]`: `watchpoints_are_refused_on_riscv64`,
  which sends `wp sp 8` and `list watchpoint` and expects the refusal of
  plan-174-D §4.1 and `no watchpoints`.

### 4.2 The man topic

`09_riscv64.md` follows the other topics' shape (Description, Examples, See
also) and `.ai/man-content.md`: it says "pool", never "arena", outside program
output; its session is real output from box 2229. Content:

- the registers by group, as `list reg gpr`, `sys` and `simd` print them;
- a table of the registers a call uses (`a0`–`a7`, results in `a0` and `a1`,
  and the three MFBASIC keeps: `s11`, `s2`, `s10`), matching bare `list reg`;
- `ra` in place of `lr` wherever another page says `lr`, with `j ra` as the
  riscv64 form of "return at once";
- instructions are 2 or 4 bytes, so `dis` addresses do not always advance by 4;
- `wp` answers that the system has no watchpoints; to find what changes a
  value, break on the functions that can change it and compare with `show mem`;
- one session: `bp isPrime`, `run`, `list reg`, `bt`, `finish`.

`package.md` gains the ninth topic line.

### 4.3 Box notes

`.ai/remote_systems.md` gains, under the riscv64 boxes: `mfb dbg` runs there;
neither kernel has single-step or watchpoints; box 2232 gets its `mfb` as a
static build from 2229 (the command of plan-174-D's Prerequisites).

## Compatibility / Format Impact

None beyond plan-174-D's. This sub-plan changes tests and documentation.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Fill the phase's `Commit:` line when it lands. An unticked
> box means NOT DONE.

### Phase 1 — `cli_dbg` on three hosts

- [x] `tests/cli/cli_dbg.rs`: the gates and `mod cpu` of §3, and the edits of
      §4.1. (The edits are in `tests/cli/cli_dbg/session_launch_and_list.rs` and
      `threads_watch_and_run.rs`, where plan-176 moved the sessions. Two
      watchpoint tests now exist, `a_watchpoint_stops_…` and
      `a_watched_write_under_the_breakpoint_…`; both are compiled out on
      riscv64. The refusal test reads stderr, so it pipes its whole input
      instead of using `Session`. `mod cpu` has no `ARCH`: `host_image` names
      the processor itself. `cargo check --test cli_dbg` for
      `riscv64gc-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` → no
      warnings.)
- [ ] On box 2229: `cargo build --release --bin mfb` (by hand, once), then
      `MFB_TEST_EXE=$PWD/target/release/mfb cargo test --test cli_dbg`.

Acceptance: the suite passes on all three hosts with the counts of §1, and
macOS and box 2226 assert what they asserted before.
  Check: macOS `cargo test --test cli_dbg` → `18 passed; 0 failed` (est. 3 min);
  box 2226, by plan-174-A's Prerequisites command → `16 passed; 0 failed` (est. 10
  min); box 2229 → `16 passed; 0
  failed` (est. 15 min after the release build; the build itself is the
  Prerequisites row and runs once). `git diff -- tests/cli/cli_dbg.rs | grep
  '^-.*"' ` lists only lines whose literal reappears in `mod cpu`'s AArch64
  constants. A real session on riscv64 is the only check that can fail if the
  backend is wrong there, which is why the box run is not replaced by something
  smaller.
Commit: —

### Phase 2 — the man topic and box notes

- [ ] Add `src/docs/man/debug/09_riscv64.md` per §4.2, with its session captured
      on box 2229 from `printf 'bp isPrime\nrun\nlist reg\nbt\nfinish\nq\n' |
      mfb dbg debug-musl.out -- 2 500`.
- [ ] `src/docs/man/debug/package.md`: the ninth topic line.
- [ ] `.ai/remote_systems.md` per §4.3.

Acceptance: the topic renders, is listed, and passes the man standard's sweeps.
  Check: `mfb man debug riscv64` prints the page; `./scripts/man-census.sh
  --topics debug` → 10 pages, 10 files; `./scripts/man-census.sh --memory-scope`
  → `unclassified memory-vocabulary hits: 0`; `./scripts/man-census.sh --scope`
  → `internals-vocabulary hits: 0` (est. 5 min).
Commit: —

### Phase 3 — final gate and archive

- [ ] In a dedicated worktree (`.ai/testing-gates.md`: a full `cargo test` in
      the shared checkout blocks landing), run the full suite once.
- [ ] Move `planning/plan-174-A` to `E` into `planning/completed/`.

Acceptance: the whole suite is green with plan-174 in it.
  Check: `cargo test --no-fail-fast > /tmp/plan-174.log 2>&1; echo EXIT=$?` →
  `EXIT=0` (est. over an hour; a guess, since this plan did not time the full
  suite. Nothing smaller covers the compiler's other tests, and it runs once).
Commit: —

## Validation Plan

- Tests: `cli_dbg` on macOS, box 2226 and box 2229; the refusal test on riscv64.
- Coverage check: on box 2229 the run's summary line must read `16 passed`. A
  count of 2 would mean `mod native` was still compiled out there and the suite
  proved nothing.
- Runtime proof: every `cli_dbg` test is a real `mfb dbg` session on a real
  process. glibc on riscv64 is covered by plan-174-D Phase 4's sessions on box
  2232, which has no `cargo` to run this suite.
- Doc sync: the man topic (Phase 2). The spec and the where-it-runs sentences
  were updated in plan-174-D with the code.
- Final gate (run ONCE, Phase 3): the full `cargo test`, which includes the
  artifact gate through `tests/gate/golden.rs`.

## Decisions

- riscv64 is documented in one new man topic, `mfb man debug riscv64` — decided
  by the owner, 2026-10-12. Existing pages stay AArch64. (§4.2)
- `wp` is refused on riscv64 — decided by the owner, 2026-10-12. The watchpoint
  test is compiled out there and a refusal test takes its place. (§4.1)
- Registers by ABI name only — decided by the owner, 2026-10-12. `mod cpu` uses
  `a0`, `a1`, `ra`. (§3)

## Corrections

## Summary

The engineering risk of plan-174 is behind this sub-plan; what remains is making
the proof run where it has to. The one thing to protect is that macOS and Linux
AArch64 keep asserting exactly what they assert today. Debugger behaviour is not
touched here.
