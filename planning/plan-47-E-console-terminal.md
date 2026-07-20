# plan-47-E: the Windows console/terminal surface

Last updated: 2026-07-20
Effort: small (<1h) for E1; medium (1h–2h) for E2
Depends on: **E1 depends on nothing** (inert chokepoint refactor, lands before 47-A).
**E2 depends on 47-S** (the raised raw-mode seam) and E1.
Feature-wide precondition: master §Prerequisites.
Produces: a `term_symbol` chokepoint (E1); the Console API implementations and
`term.*`/terminal `io.*` in `runtime_calls` (E2).

Implements `term::*` and `io::`'s terminal queries over the Windows Console API:
`GetConsoleMode`/`SetConsoleMode` for raw mode and VT processing,
`GetConsoleScreenBufferInfo` for size.

The single behavioral outcome: a program that enters raw mode, reads keystrokes, queries
the terminal size and restores the terminal on exit behaves the same on Windows as on
linux-x86_64 — including restoring the console correctly when the program traps.

**This sub-plan corrects a claim in plan-47-F.** F:21 says it is "different in kind from
47-D/E/G: those add *new* methods to the Windows `CodegenPlatform`." E adds almost no
methods — it rewrites **6 hardcoded POSIX symbol literals in shared lowering** and
answers 3 branch sites. It is F's shape at 7% the scale.

References (read first):

- `src/target/shared/code/io_helpers.rs:825` (`"isatty"`), `:838` (`"tcgetattr"`),
  `:911`/`:952`/`:1034` (`"tcsetattr"`) — 5 of the 6 literals.
- `src/target/shared/code/term.rs:470` (`"tcsetattr"`) — the 6th; and `:233`, `:316`,
  `:800` — the three `TIOCGWINSZ` branches.
- `planning/plan-47-F-threads.md` §Phase 1 — **the technique this sub-plan clones.**
  Collapse to one chokepoint, prove zero-byte diff, then add the Windows arm.
- `planning/plan-47-S-raise-the-posix-seam.md` §4.1 — `emit_set_raw_mode`, which replaces
  the 8 `termios_*` constants E2 would otherwise need.

## Prerequisites

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| *(E1 only)* Byte-identity goldens for all four targets | `find tests -path '*/golden/*' -name '*.ncode*' \| while read f; do b="${f##*/}"; b="${b%.*}"; echo "${b##*.}"; done \| sort -u` | **NOT MET — `linux-riscv64` has 0** |
| *(E2)* plan-47-S has landed | `rg -n 'fn emit_set_raw_mode' src/` | **NOT MET** |
| *(E2)* plan-47-C has landed | `ls src/target/win_x86_64/code.rs` | **NOT MET** |
| *(E2)* The Win11 box answers | `ssh -p 2230 test@127.0.0.1 true` | **UNVERIFIED — run it** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before continuing and again before deciding to stop. If you stop, report all
> four statuses.

**E1 needs only row 1.** It is an inert refactor that lands before 47-A and blocks on
nothing else — which is the point of splitting it out.

## 1. Goal

- **E1:** every terminal-related libc call in shared lowering goes through one
  `term_symbol`-style chokepoint. Zero behavior change; all four targets byte-identical.
- **E2:** raw mode via `SetConsoleMode` (`ENABLE_VIRTUAL_TERMINAL_PROCESSING` on,
  `ENABLE_LINE_INPUT`/`ENABLE_ECHO_INPUT` off), terminal size via
  `GetConsoleScreenBufferInfo`, is-a-terminal via `GetConsoleMode` succeeding.
- The terminal is **restored on every exit path**, including a trap — the same guarantee
  the POSIX path gives.
- `term.*` advertised in `runtime_calls` only after E2.

### Non-goals (explicit constraints)

- **No VT emulation.** Windows 10+ supports VT sequences via
  `ENABLE_VIRTUAL_TERMINAL_PROCESSING`; this sub-plan turns that on and emits the same
  sequences every other target does. It does not implement a fallback renderer for older
  consoles — see §Open Decisions 1.
- **No app-mode terminal helpers.** Those 8 methods are defaulted (master §2.1) and
  Windows is console-only.
- **E1 adds no Windows behavior.** It is a pure refactor; a Windows arm in E1 would
  destroy the byte-identity signal that is its entire proof.
- **Do not reimplement `emit_is_terminal`** — it already exists on the trait and works;
  E2 fills its Windows arm.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| Hardcoded POSIX terminal symbol literals in shared lowering | **6** | `rg -n '"(isatty\|tcgetattr\|tcsetattr)"' src/target/shared/code/io_helpers.rs src/target/shared/code/term.rs` |
| — in `io_helpers.rs` | 5 | `:825`, `:838`, `:911`, `:952`, `:1034` |
| — in `term.rs` | 1 | `:470` |
| Branch sites in terminal shared lowering (converted by 47-P) | **3** | `term.rs:233`, `:316`, `:800` — all `== "macos-aarch64"` |
| `termios_*` trait constants E2 would need without 47-S | **8** | master §2.1 |
| Terminal-related trait methods | 3 (`emit_is_terminal`, `emit_terminal_size`, `emit_poll_input`) | `awk … \| grep -E 'terminal\|poll_input'` |

For scale: F rewrites ~85 pthread literals, G rewrites 37 socket literals, E rewrites 6.
Same technique, three very different sizes.

### 2.2 Why E cannot be method-only

`io_helpers.rs:866` computes `slots.modified + platform.termios_lflag_offset()` and then
calls `tcsetattr` — it *builds a `struct termios` inline*. Windows raw mode is
`SetConsoleMode(handle, DWORD)`: a bitmask on a handle, no struct, no per-field offsets.
No integer returned from `termios_lflag_offset()` makes that consumer correct.

That is the whole argument for 47-S, and E2 is its first consumer. Without S, E2 would
have to fork the consumer — which is the option S rejected.

### 2.3 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| 6 terminal symbol literals live in shared lowering | **CONFIRMED** | the 6 sites listed in §2.1 |
| The raw-mode consumer builds a POSIX struct inline | **CONFIRMED** | `io_helpers.rs:866` |
| `SetConsoleMode` is a bitmask on a handle, not a struct | **CONFIRMED** | Win32 API contract |
| The 3 `TIOCGWINSZ` branches give Windows `LINUX_TIOCGWINSZ` today | **CONFIRMED** | `term.rs:233`, `:316`, `:800` are binary `== "macos-aarch64"` tests; 47-P converts them |
| `ENABLE_VIRTUAL_TERMINAL_PROCESSING` exists on supported Windows | **CONFIRMED** | Windows 10 1511+; the Win11 test box qualifies |
| Restoring the console on a trap works the same way | **UNVERIFIED — Phase E2-3 proves it** | the POSIX path restores via the shutdown hook; the Windows equivalent is untested |

## 3. Design Overview

Two units with very different risk profiles, which is why they are split.

**E1 — the chokepoint (inert, blocks on nothing).** Route all 6 literals through one
function that maps an intent to a symbol name, exactly as 47-F Phase 1 does for
`sync_symbol`. Nothing else changes; the proof is 0-diff goldens on all four targets.
This is deliberately landable before 47-A, so that when Windows arrives there is one
place to add its arm rather than 6.

**E2 — the Windows arms.** Fill `emit_set_raw_mode` (from 47-S), `emit_terminal_size`,
`emit_is_terminal` and `emit_poll_input` with Console API calls, and answer 47-P's three
`TIOCGWINSZ` matches.

**Where design uncertainty concentrates:** console restoration on abnormal exit. On
POSIX the shutdown hook (`entry_and_arena.rs:1868` `lower_shutdown`) turns the terminal
off. Windows needs the saved `DWORD` mode restored on the same path — but a Windows
console mode is per-handle process-wide state that *survives the process*, so leaving it
wrong corrupts the user's shell. **E2 Phase 3 is a spike against exactly that**: trap
mid-raw-mode and confirm the shell is usable afterwards.

**Where correctness risk concentrates:** the mode bitmask. `SetConsoleMode` takes the
*whole* mode, so raw mode is read-modify-write: `GetConsoleMode`, clear
`ENABLE_LINE_INPUT|ENABLE_ECHO_INPUT|ENABLE_PROCESSED_INPUT`, set
`ENABLE_VIRTUAL_TERMINAL_INPUT`, write back — and the *saved original* must be stored for
restoration. Clobbering an unrelated bit is invisible until a user notices their shell
behaves oddly.

Note also that input and output are **separate handles with separate modes**:
`ENABLE_VIRTUAL_TERMINAL_PROCESSING` is an *output* flag, `ENABLE_VIRTUAL_TERMINAL_INPUT`
is an *input* flag. A single "raw mode" call must touch both, which POSIX's single
`termios` does not prepare you for.

**Rejected alternative:** *skip 47-S and fork `io_helpers.rs:866` on `PlatformFamily`.*
Rejected: it is the option S explicitly rejected, it leaves POSIX struct knowledge in
shared code permanently, and it grows a second fork the moment G needs the same treatment.

**Rejected alternative:** *implement a VT-sequence renderer for pre-1511 consoles.*
Rejected as scope: the supported-Windows floor is the test box, and a renderer is a
feature, not a port.

## 4. Detailed Design

| Method | Win32 |
|---|---|
| `emit_is_terminal` | `GetConsoleMode(h, &mode)` succeeding |
| `emit_set_raw_mode(true)` | save mode; clear `ENABLE_LINE_INPUT\|ENABLE_ECHO_INPUT\|ENABLE_PROCESSED_INPUT` on the **input** handle, set `ENABLE_VIRTUAL_TERMINAL_INPUT`; set `ENABLE_VIRTUAL_TERMINAL_PROCESSING` on the **output** handle |
| `emit_set_raw_mode(false)` | restore both saved modes |
| `emit_terminal_size` | `GetConsoleScreenBufferInfo` → `srWindow` (right−left+1, bottom−top+1). **Not** `dwSize`, which is the buffer, not the window |
| `emit_poll_input` | `WaitForSingleObject(stdin, timeout)` |

## Compatibility / Format Impact

- **New:** `term.*` in the Windows `runtime_calls`; kernel32 gains ~6 imports.
- **Changed (shared, E1):** the 6 literals route through one chokepoint. Byte-identical
  for all four existing targets.
- **Unchanged:** the `term::` language surface; every other backend's terminal behavior.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### E1 Phase 1 — the chokepoint (inert; blocks on nothing; land early)

- [ ] Add a `term_symbol(intent)` chokepoint and route all 6 literals
      (`io_helpers.rs:825,838,911,952,1034`; `term.rs:470`) through it.
- [ ] No Windows arm. No behavior change.

Acceptance: `scripts/artifact-gate.sh` 0 diffs on all four existing targets. Any diff
means the refactor changed emission — fix it, do not rebaseline.
Commit: —

### E2 Phase 1 — size and is-terminal (the safe half)

- [ ] `emit_is_terminal` over `GetConsoleMode`; `emit_terminal_size` over
      `GetConsoleScreenBufferInfo` using `srWindow`, not `dwSize`.
- [ ] Answer 47-P's three `TIOCGWINSZ` matches (`term.rs:233`, `:316`, `:800`).
- [ ] Runtime: a program printing the terminal size, and one printing whether stdout is
      a terminal (checked both piped and interactive).

Acceptance: size matches what the Windows console reports; is-terminal is correct both
piped and interactive.
Commit: —

### E2 Phase 2 — raw mode

- [ ] `emit_set_raw_mode` per §4, touching **both** the input and output handles and
      saving both original modes.
- [ ] Runtime: a keystroke-reading program behaves as on linux-x86_64.

Acceptance: individual keystrokes are delivered without line buffering or echo, and VT
sequences render.
Commit: —

### E2 Phase 3 — restoration on every exit path (largest blast radius last)

Console mode is process-wide state that **outlives the process**. Getting this wrong
corrupts the user's shell after the program exits, which no unit test will catch.

- [ ] Restore both saved modes from the shutdown path (`lower_shutdown`,
      `entry_and_arena.rs:1868`).
- [ ] Runtime: enter raw mode then (a) exit normally, (b) TRAP, (c) Ctrl-C — and after
      **each**, confirm the shell still echoes and line-buffers correctly.
- [ ] Advertise `term.*` in `runtime_calls`.

Acceptance: after all three exit paths the console is fully restored. This is verified by
using the shell afterwards, not by reading the code.
Commit: —

## Validation Plan

- Tests: E1's proof is byte-identity. E2's is runtime behavior on the Win11 box —
  raw-mode input cannot be unit-tested meaningfully.
- Coverage check: E1 edits shared lowering, so `linux-riscv64`'s zero goldens make its
  0-diff vacuous (master §Prerequisites row 3). Seed them before E1.
- Runtime proof: the Win11 box for every E2 phase. **Phase 3's proof is using the shell
  afterwards** — the failure mode is invisible to the program itself.
- Doc sync: if VT support requires documenting a Windows version floor, that is a spec
  change (`src/docs/spec/stdlib/` terminal section).
- Acceptance: full suite plus `scripts/artifact-gate.sh` 0 diffs.

## Open Decisions

1. **Minimum Windows version for VT processing.** Recommended: require Windows 10 1511+
   and document it, rather than implementing a fallback renderer. The test box is Win11.
2. **Whether `emit_poll_input` uses `WaitForSingleObject` or `PeekConsoleInput`.**
   Recommended `WaitForSingleObject` on the stdin handle — it matches the POSIX `poll`
   semantics the shared caller expects. `PeekConsoleInput` reports console *events*
   (including window resizes), which is a different question.
3. **Whether raw mode should also disable `ENABLE_PROCESSED_INPUT`** (which handles
   Ctrl-C). Recommended: **no** — leave Ctrl-C working, matching the POSIX path, which
   leaves `ISIG` on. Disabling it would silently change signal behavior between targets.

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **plan-47-F's claim that E "adds new methods to the Windows
  `CodegenPlatform`" is false** (F:21). E rewrites 6 hardcoded POSIX symbol literals in
  shared lowering and answers 3 branch sites — F's shape at 7% the scale. E is split into
  an inert E1 that blocks on nothing, mirroring F's own Phase 1 technique.
- 2026-07-20 — **Input and output are separate handles with separate modes on Windows.**
  `ENABLE_VIRTUAL_TERMINAL_PROCESSING` is an output flag; `ENABLE_VIRTUAL_TERMINAL_INPUT`
  is an input flag. A single POSIX `termios` write becomes two `SetConsoleMode` calls.
- 2026-07-20 — **`GetConsoleScreenBufferInfo` reports two different sizes.** `dwSize` is
  the scrollback buffer; `srWindow` is the visible window. `TIOCGWINSZ` means the window.

## Summary

The engineering risk is not in the terminal code — it is in what the terminal code leaves
behind. Windows console mode is per-handle, process-wide state that survives the process,
so a missed restoration corrupts the user's shell after exit, and no test the program can
run will detect it. E2 Phase 3 exists solely for that, and its acceptance is using the
shell afterwards.

The structural point is smaller but worth carrying to G: E is not a "just add methods"
surface, and splitting the inert chokepoint (E1) out lets it land before 47-A alongside
F1 and G1.

What is left untouched: the `term::` language surface, VT sequence generation (unchanged
— Windows just enables processing of the same sequences), and the app-mode terminal
helpers, which stay defaulted.
