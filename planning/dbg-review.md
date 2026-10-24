# Debugger review (`mfb dbg`)

**Review date:** 2026-10-24  
**Platform:** macOS 15 / Darwin 24.6.0, arm64 (`uname -a`)  
**Review status:** macOS and shared-layer review completed; two input-triggered defects were reproduced and fixed during review.  
**Scope:** the native `mfb dbg` CLI, shared debugger behavior, and macOS process/thread backend, as used on macOS. Linux and other platform backends were not exercised or reviewed as platform implementations. The two reproduced defects were fixed in shared debugger code, with regression checks and spec updates in the same working tree.

## Executive assessment

On macOS arm64, `mfb dbg` is a capable instruction-level debugger for MFBASIC executables. I exercised launch-and-pause, named and conditional breakpoints, register inspection, stepping and finish, backtraces, watchpoints, break-on-error, typed list/map inspection, multi-thread inspection, the debug exit report, and normal exit. The debugger remained useful with `--debug` binaries built at `-O0`, `-O1`, `-O2`, and `-O3`; function names and backtraces were available at every tested optimization level. The hands-on pass found two defects in shared command handling, both fixed and recorded below.

For a non-`--debug` build, it still launches, runs, reports process exit, and supports raw-address breakpoints, register inspection, and instruction disassembly. I read the initial `pc`, set a breakpoint four bytes later, stopped there with `F: -`, removed it, then ran the program to exit 0. It loses MFB user function names, arena inspection, and error-function-based break-on-error; `bp main` is rejected because no such function is present. This remains useful for low-level triage when an address is known, but is materially less useful without symbols or the runtime debug registry. The prompt correctly explains the rebuild requirement.

The product is a low-level debugger, not a source debugger. It exposes instructions, registers, addresses, and runtime data structures; the tested workflow did not provide source lines, source variables, or source expressions. Users seeking ordinary source-level debugging will need the `mfb man debug`/external debugger workflow. The distinction should remain prominent in user-facing help.

## Scope and method

Reviewed the debugger specification at `src/docs/spec/tooling/10_debugger.md`, the CLI at `src/cli/dbg.rs`, and the shared `src/debugger/` paths for prompt parsing/actions, breakpoints, disassembly, backtraces, runtime metadata, typed values, naming, and environment handling. Read the macOS backend paths for task lifecycle, Mach exception routing, instruction patching, watchpoint registers, stepping, thread state, and dyld library discovery. The review is scoped to macOS plus shared code; the Linux implementation was explicitly excluded. Existing test modules were consulted as contracts. This is a code review of the listed paths plus scenario probes, not a proof of every race/failure branch.

The runtime fixture was the repository's `examples/debug` MFB project. It exercises nested user functions, deterministic output, collection values, file I/O, and optional worker threads. Builds and debugger runs used the existing `target/debug/mfb`; the signed debugger copy and probe project executables were placed under `/tmp/mfb-debug-review*`. `scripts/sign-debugger.sh` was applied to the temporary compiler copy as required by macOS task-port access. The repository already had an unrelated untracked `planning/shaderfunc.md`; it was left unchanged.

Commands and observations:

- `uname -a` identified Darwin 24.6.0 on arm64.
- `target/debug/mfb build --debug -O0 /tmp/mfb-debug-review`, followed by `mfb dbg ...`: `bp main`, `run`, `bt`, `list func`, `step 4`, and continued `run` all worked; the program printed the expected two rounds and exited 0.
- The same fixture built with `--debug -O1`, `--debug -O2`, and `--debug -O3`. `bp runRound`/`bp isPrime` resolved, `run` stopped at the function, and `bt` displayed MFB function names and callers at each level. At `-O3`, `finish` returned from `isPrime` to `countPrimes` and reported the return register.
- The `-O1` debug run's `show report` returned the report retained from process exit, including `mfb.debug.begin/end`, performance counters, arena counters, and process RSS. The report was for the small one-round fixture and is not a performance comparison.
- A two-round worker-thread run stopped at `roundWorker` on thread 1. `bt all` showed the main thread blocked in `thread::waitFor` and the worker in MFB code; `step-all 3` reported the main thread waiting and advanced the worker three instructions; a later `run` resumed execution and stopped at the next worker breakpoint.
- A plain `-O1` binary launched successfully. `list func` showed imported system-linked functions but no MFB user functions; `bp main` returned `error: no function is named 'main'`; `list arena` and `breakerror` explained that the program was not built with `--debug`. In a PTY session, a raw breakpoint at `pc+4` stopped with `F: -`; after removal the program ran to exit 0, without a debug-report hint.
- A conditional breakpoint `bpi x0 eq 824084 at countPrimes` skipped the first call, stopped on the second, and `list reg x0, x1` showed the matching argument value.
- A hardware watchpoint at `sp-0xf0` in `countPrimes` stopped first when the slot was initialized to zero and again when the count changed to one, reporting the write instruction and before/after values.
- On a real `ErrOverflow`, `be ErrOverflow` stopped at the raiser; `show error` decoded the code, message, and `src/main.mfb:3:10`, `bt` showed the caller chain, and continuing reached exit status 255.
- At `summarize`, `show type list x0 0..` decoded both counts and `show type map x1 get round 2` found the expected map entry. `list arena` found the main arena; `show arena 0` decoded the arena state and two block headers. `finish` then printed an arbitrary-looking `x1` value for the `Nothing`-returning function; see usability note 5.
- A command list `bp countPrimes list reg x0, x1; bt 2; run` printed arguments and a two-frame backtrace at each hit, then resumed automatically through both rounds to exit.
- A pending system-symbol breakpoint `bp _exit` was placed after dyld loaded libraries. Removing the MFB `isPrime` breakpoint let the program run to `_exit`, then one more `run` reported exit 0.
- A separately running `--debug -O0` process was attached by PID. `list thread` identified its current `isPrime+0x1b8` PC; `bt all` showed `isPrime -> countPrimes -> runRound -> runRounds -> main -> _main`; `list reg` exposed the current argument/arena/thread registers. `q` printed `Detached ...; it continues running.` The detached process was then terminated by its own PID.
- `dis countPrimes 6` showed six decoded AArch64 instructions. `dis pc -4 8` crossed from the preceding function into `countPrimes`, inserted headers for both functions, and marked the current PC. `step 3` advanced exactly three instructions; `bt` retained the caller chain.
- With `env -i HOME=/tmp TERM=xterm PATH=/usr/bin:/bin`, `env s REVIEW_ENV value with spaces`, `list env`, and `restart` accepted the spaced value, showed it under “After restart”, then moved it to “Current” after restart. The relaunched program exited 0. An earlier run with the full development environment produced a very wide listing, so a minimal launch environment is better for this workflow.
- `bp main+1` was accepted, patched unaligned bytes into an AArch64 instruction, and `run` stopped at the original PC with `X: an illegal instruction`. The breakpoint path had omitted the architecture alignment check already used by `dis` and `jump`. Fixed by rejecting unaligned resolved addresses before installation; a focused regression test was added.
- `show type list x0 18446744073709551615` panicked the debugger with `attempt to add with overflow` at `Span::parse`; the single-index parser computed `u64::MAX + 1`. Fixed using `checked_add`; a regression assertion now expects the normal invalid-span result.
- `target/debug/mfb --help` failed because the repository-root `./mfb` launcher is absent; the built compiler's `target/debug/mfb --help` and `target/debug/mfb dbg --help` were used instead. No conclusion about the launcher is part of this review.

## Findings

### Confirmed defects reproduced and fixed

1. **Unaligned breakpoint offsets corrupt instructions (P2, fixed).** On the macOS arm64 fixture, `bp main+1` printed a successful “Set a breakpoint” line. `Breakpoints::set` called `Inferior::insert_breakpoint` without checking `Arch::alignment`; the backend wrote the four-byte `brk #0` at that unaligned address. The subsequent `run` produced `--[XC] ... X: an illegal instruction` at the function's original start. The breakpoint code now validates every resolved address before installing any of them. A unit test pins the AArch64 alignment result; validation occurs before any backend insertion or session mutation. The debugger spec now states the alignment requirement.
2. **Maximum list/set index panics the debugger (P2, fixed).** `show type list x0 18446744073709551615` reached `Span::parse`, where `at + 1` overflowed in the debug compiler and panicked the prompt process. The parser now uses `checked_add(1)` and maps overflow to its regular invalid-index/range response. A regression assertion covers `u64::MAX`; the spec records the refusal behavior.

The original repros were run before the fixes. Both were repeated against a freshly rebuilt compiler after the fixes; no unrelated fixtures or generated outputs were changed.

### Usability and product gaps

1. **No source-level mapping or source-variable view.** Breakpoints resolve functions and addresses, and stepping is by machine instruction. The register/memory/type inspection is powerful but requires knowledge of generated code and runtime layouts. This is the largest usability limit for application debugging. Consider source locations, source-line breakpoints, and scoped variable locations as a separate feature effort; until then, keep the low-level nature explicit in `mfb man debug` and command help.
2. **Non-debug builds lose the principal MFB-level handles.** Hands-on `-O1` testing confirmed that MFB function names disappear, so symbolic breakpoints such as `bp main` fail; arena listing and `breakerror` are also unavailable. Address-level debugging can remain possible, but the value is sharply reduced. The existing rebuild diagnostic is useful. Consider an early, concise session banner summarizing exactly which capabilities this binary has (user symbols, arena registry, error stops, exit report), rather than relying on later command errors.
3. **Optimization still changes the meaning of stepping.** Names and backtraces worked through `-O3`, but the debugger operates on emitted instructions; source statements can be folded, reordered, or removed. A function breakpoint at `-O3` establishes entry into the function, not source-level execution fidelity. The help should continue to distinguish optimizer level from debug metadata availability and avoid implying that `--debug` disables optimization.
4. **Large `list func` output is noisy.** The O0 fixture's table included user functions, many runtime helpers, and system-linked imports. That is useful for symbol discovery but makes user code harder to scan in a real program. A filter by origin/name and a default user-only view, with an explicit all-functions option, would improve navigation. Preserve the current origin distinction for users who need runtime internals.

5. **`finish` prints a meaningless result register for `Nothing` functions.** At the `summarize` breakpoint in `examples/debug`, `finish` reported `x0 = 0` and a non-deterministic-looking pointer-sized `x1` although `summarize` is declared `AS Nothing`. `finish_lines` always prints both ABI result registers and has no return-type metadata with which to distinguish a `Nothing` result. This is observable and potentially misleading; consider printing the raw registers as such or suppressing the value field when the return type is known to be `Nothing`. This is a usability defect, not yet fixed in this review.

These are product gaps and suggestions, not claims that the current implementation violates its documented contract.

## Capability matrix from macOS runs

| Program build | User function names / symbolic breakpoints | Backtrace | Runtime arena / error inspection | Run and exit | Observed result |
|---|---|---|---|---|---|
| `--debug -O0` | Yes | Yes | Arena listing/state and error inspection exercised | Yes | Break at `main`, step, inspect functions, resume to exit |
| `--debug -O1` | Yes | Yes | `show report` worked; arena listing not explicitly exercised | Yes | Break at `runRound`, inspect nested backtrace, exit report |
| `--debug -O2` | Yes | Yes | Available by build mode; not deeply exercised | Yes | Break at `runRound`, backtrace, resume to exit |
| `--debug -O3` | Yes | Yes | Available by build mode; not deeply exercised | Yes | Break at `isPrime`, backtrace, `finish`, resume |
| `-O1` without `--debug` | No MFB user names; system imports remain | Reduced to available symbols/addresses | Arena and `breakerror` explicitly unavailable | Yes | `bp main` failed; raw `pc+4` breakpoint/disassembly worked; run exited successfully |

The `--debug` switch is what controls debugger metadata and runtime debug registries; `-O0` is not required for function-name availability. `-O0` is still the recommended choice when investigating optimizer-sensitive code.

## Review notes by subsystem

### CLI and session lifecycle

`src/cli/dbg.rs` and `src/debugger/mod.rs` define the host check, executable validation, launch/attach dispatch, and common inferior contract. The public help clearly separates binary launch from PID attach and documents argument forwarding after `--`. For macOS use, the signing prerequisite is unusually important and is called out in help and the signing script. PID attach was also exercised against a separate, long-running MFB process; task-port denial, executable replacement races, and protected/hardened targets were not exercised.

The main success path behaved coherently: the target paused before its first instruction, accepted commands through stdin, resumed under `run`, and reported exit. A piped command stream also worked. The interface is scriptable, but it is still a prompt protocol rather than a structured machine interface; that is acceptable for the documented CLI, though a stable machine-readable mode could be useful for automation.

### Breakpoints, stepping, and stop actions

Shared breakpoint resolution correctly found named MFB functions in all four debug optimization builds. Function entry breakpoints and `finish` were exercised at O0 through O3. `step-all` produced an intelligible per-thread result when one thread was waiting in a system call and another was runnable. This is a good distinction: the output did not pretend that the waiting main thread had executed instructions.

Not exercised in this review: watchpoint address/span edge cases, delivery of a watched error through `TRAP` or `thread::waitFor`, Ctrl-C, or concurrent stop races. Conditional breakpoints, breakpoint removal, command lists that auto-resume, and a live restart with environment changes were exercised. Those are high-value follow-up probes because they combine shared breakpoint state with OS thread coordination.

### Thread and backtrace inspection

The worker example demonstrated that macOS reports a stable debugger thread index for the worker and that `bt all` distinguishes the blocked main thread from the user worker. The backtrace crossed MFB code, the runtime trampoline, and pthread frames. This is useful for debugging MFB thread handoff without claiming native language-level locals.

The result does not establish behavior for rapidly created/destroyed threads, a thread exiting while a stop is being assembled, or a process with many simultaneous threads. Those deserve stress coverage, especially if the backend's thread enumeration is cached across stop/resume cycles.

### Values, runtime metadata, and reports

The help/spec describe memory, register, arena, typed-value, thread-error, and exit-report views. The debug report was retained and retrievable after exit in the O1 debug run. In the non-debug run, the explicit diagnostics for arenas and break-on-error were actionable and prescribed a rebuild. This is a good degraded-mode behavior.

This session did not deeply validate typed decoding for nested collections, malformed runtime pointers, arena registry overflow, or error-record ownership across threads. These are correctness-sensitive areas: malformed addresses must remain bounded to the target's readable mappings, and every typed display should fail with a localized diagnostic rather than turn corrupt metadata into an uncontrolled read. The code/spec describe bounds and fault behavior; a dedicated adversarial value-inspection pass would be useful.

### macOS process control and architecture

The host is Apple Silicon, so the AArch64 instruction/register backend and Darwin process-control implementation were exercised together. The temporary ad-hoc signature allowed launch-and-debug. x86-64 macOS binaries, Rosetta execution, attach to protected or hardened-runtime processes, and failures due to missing entitlement were not tested. Since macOS task-port access is permission-sensitive, retain explicit diagnostics for entitlement/root refusal and ensure any failed launched child is killed and reaped, as the spec states.

## Post-fix validation

`cargo build --bin mfb` passed. The rebuilt compiler was signed with `scripts/sign-debugger.sh`, then both original repros were rerun against it: `bp main+1` was refused with the instruction-boundary diagnostic before installation, and `show type list x0 18446744073709551615` returned the normal invalid-index diagnostic while the prompt remained alive. `git diff --check` passes. Focused Rust regression assertions were added but not run. The earlier pre-fix repro outputs are retained above to show the defects were real.

## Suggested follow-up probes

These are not blockers to this review; they are the most valuable unexercised cases for a future focused validation pass:

1. Trigger a real MFB runtime error and verify `breakerror`, `show error`, per-thread attribution, continued execution/exit, and `show report` together.
2. Exercise a watchpoint and conditional breakpoint on mutable local/arena memory, including removal, an unreadable condition operand, and a watched write spanning adjacent instructions.
3. Exercise `restart` and every `env` operation on a launched process; compare initial and restarted arguments/environment, including values with spaces and empty values.
4. Send Ctrl-C while the target is running, and test `quit` both before and after exit; verify the child is paused or killed as documented and never left as an orphan.
5. Stress thread creation, completion, and `step-all` with multiple workers, including a worker that exits while the debugger is collecting stops.
6. Probe attach and launch refusal cases: missing debugger entitlement, protected system process, stale PID, and target exit during attach; confirm clear diagnostics and resource cleanup.
7. Inspect representative nested `List`, `Map`, `Set`, `String`, `Fixed`, and `Money` values plus invalid addresses and registry edge cases.
8. Compare source function/instruction breakpoint placement and `dis` output between `-O0`, `-O1`, `-O2`, and `-O3` on a fixture whose optimizer removes or folds work; document which stepping guarantees are intentionally absent.

## Relevant paths

Shared/CLI and macOS implementation reviewed or consulted:

- `src/cli/dbg.rs`
- `src/debugger/mod.rs`, `arch.rs`, `arm64/`, `backtrace/`, `breakpoints/`, `disassemble.rs`, `environment.rs`, `location.rs`, `names.rs`, `prompt/`, `runtime.rs`, `values/`
- `src/debugger/macos/`
- `src/docs/spec/tooling/10_debugger.md`
- `scripts/sign-debugger.sh`
- `examples/debug/`

The debugger specification is the implementation map and command contract; this review records observations and review judgments without duplicating its full command reference.
