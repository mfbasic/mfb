# Debugger review (`mfb dbg`)

**Review date:** 2026-10-24  
**Platform:** macOS 15 / Darwin 24.6.0, arm64 (`uname -a`)  
**Review status:** macOS and shared-layer review completed; two input-triggered defects were reproduced and fixed. An extended stress pass covered the remaining requested workflows.
**Scope:** the native `mfb dbg` CLI, shared debugger behavior, and macOS process/thread backend, as used on macOS. Linux and other platform backends were not exercised or reviewed as platform implementations. The two reproduced defects were fixed in shared debugger code, with regression checks and spec updates in the same working tree.

## Executive assessment

On macOS arm64, `mfb dbg` is a capable instruction-level debugger for MFBASIC executables. I exercised launch-and-pause, named and conditional breakpoints, register inspection, stepping and finish, backtraces, watchpoints, break-on-error, typed list/map inspection, multi-thread inspection, the debug exit report, and normal exit. The debugger remained useful with `--debug` binaries built at `-O0`, `-O1`, `-O2`, and `-O3`; function names and backtraces were available at every tested optimization level. The hands-on pass found two defects in shared command handling, both fixed and recorded below.

For a non-`--debug` build, it still launches, runs, reports process exit, and supports raw-address breakpoints, register inspection, and instruction disassembly. I read the initial `pc`, set a breakpoint four bytes later, stopped there with `F: -`, removed it, then ran the program to exit 0. It loses MFB user function names, arena inspection, and error-function-based break-on-error; `bp main` is rejected because no such function is present. This remains useful for low-level triage when an address is known, but is materially less useful without symbols or the runtime debug registry. The prompt correctly explains the rebuild requirement.

The product is a low-level debugger, not a source debugger. It exposes instructions, registers, addresses, and runtime data structures; the tested workflow did not provide source lines, source variables, or source expressions. Users seeking ordinary source-level debugging will need the `mfb man debug`/external debugger workflow. The distinction should remain prominent in user-facing help.

## Scope and method

Reviewed the debugger specification at `src/docs/spec/tooling/10_debugger.md`, the CLI at `src/cli/dbg.rs`, and the shared `src/debugger/` paths for prompt parsing/actions, breakpoints, disassembly, backtraces, runtime metadata, typed values, naming, and environment handling. Read the macOS backend paths for task lifecycle, Mach exception routing, instruction patching, watchpoint registers, stepping, thread state, and dyld library discovery. The review is scoped to macOS plus shared code; the Linux implementation was explicitly excluded. Existing test modules were consulted as contracts. This is a code review of the listed paths plus scenario probes, not a proof of every race/failure branch.

The runtime fixture was the repository's `examples/debug` MFB project, supplemented with one-off MFB projects under `/tmp` for worker errors, environment restarts, nested collections, short-lived workers, optimization, and attach cases. Builds and debugger runs used the freshly rebuilt `target/debug/mfb`; the signed debugger copy and probe project executables were placed under `/tmp/mfb-debug-review*` and `/tmp/mfb-dbg-*`. `scripts/sign-debugger.sh` was applied to the temporary compiler copy as required by macOS task-port access. No probe fixture or generated output was added to the repository.

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
- The extended hard-thread sequence once emitted `cannot read a thread's registers: thread_get_state: (os/kern) object terminated` around `step-all`/thread inspection. Five full-sequence repeats and five step/resume repeats produced no such diagnostic; an eight-worker exit-timing probe repeated 8 times also did not reproduce it. Treat this as an intermittent race signal that merits a dedicated reproducer; this pass did not establish a stable repro or justify a code change.
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

`src/cli/dbg.rs` and `src/debugger/mod.rs` define the host check, executable validation, launch/attach dispatch, and common inferior contract. The public help clearly separates binary launch from PID attach and documents argument forwarding after `--`. For macOS use, the signing prerequisite is unusually important and is called out in help and the signing script. PID attach was exercised against a separate MFB process; hardened-target task-port denial and a timed attach/target-kill attempt were also checked. The exact task-port acquisition race and executable replacement race remain unobservable or unstaged.

The main success path behaved coherently: the target paused before its first instruction, accepted commands through stdin, resumed under `run`, and reported exit. A piped command stream also worked. The interface is scriptable, but it is still a prompt protocol rather than a structured machine interface; that is acceptable for the documented CLI, though a stable machine-readable mode could be useful for automation.

### Breakpoints, stepping, and stop actions

Shared breakpoint resolution correctly found named MFB functions in all four debug optimization builds. Function entry breakpoints and `finish` were exercised at O0 through O3. `step-all` produced an intelligible per-thread result when one thread was waiting in a system call and another was runnable. This is a good distinction: the output did not pretend that the waiting main thread had executed instructions.

The extended probe catalog below covers watchpoint size/capacity/removal, unreadable conditions, Ctrl-C, worker error delivery through `thread::waitFor`, and multi-worker stepping. Adjacent-instruction watch writes and races in the exact snapshot interval remain untested.

### Thread and backtrace inspection

The worker example demonstrated that macOS reports a stable debugger thread index for the worker and that `bt all` distinguishes the blocked main thread from the user worker. The backtrace crossed MFB code, the runtime trampoline, and pthread frames. This is useful for debugging MFB thread handoff without claiming native language-level locals.

The extended probes exercised four concurrent workers, stepped them into pthread cleanup, and tested eight workers timed near a breakpoint. A transient `thread_get_state: object terminated` error occurred once in a broader sequence but did not recur in five exact-sequence repeats, five step/resume repeats, or 8 timing probes; the precise termination-snapshot race remains a follow-up.

### Values, runtime metadata, and reports

The help/spec describe memory, register, arena, typed-value, thread-error, and exit-report views. The debug report was retained and retrievable after exit in the O1 debug run. In the non-debug run, the explicit diagnostics for arenas and break-on-error were actionable and prescribed a rebuild. This is a good degraded-mode behavior.

Nested inline list/map/set decoding, invalid address reads, and worker/main error-record ownership were exercised in the extended probes below. Arena registry overflow and corrupt in-arena collection headers were not deliberately synthesized; those remain useful adversarial follow-ups.

### macOS process control and architecture

The host is Apple Silicon, so the AArch64 instruction/register backend and Darwin process-control implementation were exercised together. The temporary ad-hoc signature allowed launch-and-debug. A separate hardened-runtime MFB executable was launched successfully but rejected at `task_for_pid`; x86-64 macOS binaries, Rosetta execution, and a debugger lacking its entitlement were not tested. Since macOS task-port access is permission-sensitive, retain explicit diagnostics for entitlement/root refusal and ensure any failed launched child is killed and reaped, as the spec states.

## Extended macOS probe catalog

The temporary MFB programs and logs are under `/tmp/mfb-dbg-*`. The commands below identify each run and its observed outcome.

| Scenario and command | Result |
| --- | --- |
| Ctrl-C: in a PTY, launch `mfb dbg /tmp/mfb-dbg-exhaustive-O0.out -- 1 100000000 0`, enter `run`, then send terminal Ctrl-C. | The prompt returned with `--[CC] User Break`; `bt` showed `isPrime -> countPrimes -> runRound -> runRounds -> main -> _main`. `q` killed the launched child. A subsequent `pgrep -alf 'mfb-dbg-exhaustive-O0|mfb-dbg-churn.out|mfb-dbg-exit-race.out|mfb-dbg-worker-error'` found no target process (the only match was the `pgrep` command itself). |
| Restart arguments/environment: `/tmp/mfb-dbg-env` prints `os::args()` and `os::getEnvOr("DBG_REVIEW", "missing")`; set `DBG_REVIEW=value with spaces`, restart/run, change it, remove `PATH`, restart/run, then `env reset`, restart/run. | Each process retained `argc=2` and `arg0=original-argument`; it saw the value with spaces, then the changed value. After removing `PATH`, it still ran. `env reset` restored the launch environment, so `DBG_REVIEW` printed `missing`. |
| Empty/cleared environment: `env clear`, `env s DBG_REVIEW` (empty value), `list env`, `restart`, `run`. | `list env` showed only debugger-side `HOME`, `PATH`, `TERM`, and empty `DBG_REVIEW`; the restarted program saw `env=` and retained its argument. `env reset` restored the initial environment. `env s BAD=NAME value` and `env r MISSING` returned their documented diagnostics without ending the prompt. |
| Watchpoint capacity/input: `sysctl -n hw.optional.watchpoint` returned `4`; at a paused O0 target issue `wp sp`, `wp sp+8`, `wp sp+16`, `wp sp+24`, and `wp sp+32`, then remove one with `wp r sp+8`. | Four valid stack watchpoints installed and listed; the fifth was refused with `all 4 of the processor's watchpoints are in use`. Removal freed a slot and the listing reflected three watches. Size 0, unaligned/cross-boundary spans, and overflow-crossing spans were refused before installation. |
| Unreadable condition: `bpi [0xffffffffffffffff] eq 3 at countPrimes`, `run`, `list breakpoint`. | The conditional registered, the unreadable operand evaluated as a non-match, the program exited 0, and the breakpoint count stayed 0. No debugger panic or target exception occurred. |
| Invalid typed/memory locations: at `summarize`, request typed values at `0xffffffffffffffff` and `show mem 0xffffffffffffffff 16`. | Typed views rejected the pointer as outside an arena block; `show mem` returned the Mach invalid-address diagnostic. The prompt remained usable, and neither operation crashed the debugger or target. |
| Nested runtime values: `/tmp/mfb-dbg-nested` stops in `inspect(nested, grouped, texts)` and runs `show type list x0 0..`, `show type map x1 values`, `show type set x2 string 0..`. | The debugger decoded two inline nested integer lists, map values containing inline lists (lengths 2 and 1), and both set strings (`alpha`, `beta`). |
| Worker-thread runtime error: `/tmp/mfb-dbg-worker-error-untrapped.out`, with `be ErrInvalidFormat`, `run`, `show error`, `bt all`, `run`, `show error`, `bt all`, `run`, `show report`. | The first `--[BE]` stop was thread 1 in `worker`; `bt all` showed main blocked in `thread::waitFor` and the worker call chain, and `show error` decoded code, message, and source location. Continuing stopped again on main thread 0 when `waitFor` propagated the error. The final continuation exited 255, and `show report` returned worker/main arena and process metrics. |
| Concurrent thread view/step: `/tmp/mfb-dbg-churn.out` starts four workers together, each sleeping then executing `work`; run `bp work`, `run`, `bt all`, `step-all 50`, `list thread`, `bt all`, `run`. | The debugger reported worker breakpoints, showed main waiting and four worker stacks, stepped each worker 50 instructions while reporting main as waiting, and resumed to `total=200`/exit 0. `step-all 10000` advanced workers into pthread cleanup/exit paths; `bt all` rendered cleanup frames and a subsequent `run` reached exit 0. The full `step-all 2000`/`list thread`/`bt all`/resume sequence passed 5 repeats; a further 5 `step-all 2000`/resume runs also passed. The single `thread_get_state` diagnostic noted above was not reproduced. |
| Thread exits timed near inspection: `/tmp/mfb-dbg-exit-race.out` starts eight workers with 500 ms sleeps, sleeps 497 ms in main, then stops at `snapshot`; run `bp snapshot`, `run`, `bt all`, `list thread`. | Repeated 8 times. The breakpoint/backtrace were valid; by the stop all eight workers had left the task, so only main was listed. This did not force a thread to disappear between `task_threads` and `thread_get_state`; no termination error reproduced. |
| Bad/stale PID and protected system PID: `mfb dbg 99999999`; `mfb dbg 1`. | The nonexistent PID returned `no process with PID 99999999`. PID 1 was rejected at executable validation because `/sbin/launchd` is not an MFBASIC executable; this did not exercise task-port denial. |
| Attach denial: copy `/tmp/mfb-dbg-attach/build/debug.out`, ad-hoc sign it with `codesign -f -s - --options runtime --timestamp=none`, launch it, then attach with the debugger signed by `scripts/sign-debugger.sh`. | The hardened MFB process printed its PID and completed normally. Attach failed cleanly with `task_for_pid: (os/kern) failure` and signing/root guidance. |
| Target exits during an attached session: attach to the 5-second `/tmp/mfb-dbg-attach/build/debug.out`, externally `kill -KILL` that test PID while the debugger is paused, then enter `run`, `bt`, `q`. | `run` reported `--[EX] Program exit`; `bt` refused because the program had exited; `q` reported that the process had already exited. |
| Attach/exit timing attempt: start the 5-second `/tmp/mfb-dbg-attach-race.out`, start `mfb dbg 73382`, and issue `kill -KILL 73382` 40 ms after starting the debugger command. | The debugger printed `Attached to process 73382; paused`; the next `run` reported `--[EX] Program exit` and `bt` refused because the target had exited. This exercised a target killed during the attach attempt window, though CLI timing cannot prove whether the kill landed just before or just after task-port acquisition. A separate 350 ms self-exiting target was already gone when attach began and returned `no process with PID 72934`. |
| Debugger dies while attached: attach to the same target, send `kill -KILL` to the debugger process, then observe the target PTY. | The target resumed after the debugger died, printed `finished`, emitted its report, and exited 0. |
| Non-MFB launch target: `mfb dbg /bin/true`. | Refused before launch with `'/bin/true' is neither a file nor a process ID`; no unrelated executable ran under the debugger. |
| Optimized-away calls: `/tmp/mfb-dbg-opt` builds at `--debug -O0/-O1/-O2/-O3`; set `bp pureValue` or `bp constantValue`, then `run`. | `bp pureValue` stopped at O0, but O1/O2/O3 ran to `5,11` and exit 0 without hitting it. `bp constantValue` stopped at O0; O1/O2/O3 skipped it. `dis constantValue` showed arithmetic/error-check instructions at O0 and just `mov x1, #5; mov x0, #0; ret` at O3. Symbolic names survive while optimization removes callsites; function breakpoints do not imply a call will occur. |

The attach-versus-exit race was exercised with a 40 ms timed kill after debugger invocation and with a 350 ms self-exiting target. The timed kill produced a clean attach banner followed by clean exit detection; the shorter target was already gone when attach began. The exact internal point at which the timed kill landed remains unobservable from the CLI, so this is cataloged as a timing probe rather than proof of either side of the `task_for_pid`/`task_suspend` boundary.

## Post-fix validation

`cargo build --bin mfb` passed. The rebuilt compiler was signed with `scripts/sign-debugger.sh`, then both original repros were rerun against it: `bp main+1` was refused with the instruction-boundary diagnostic before installation, and `show type list x0 18446744073709551615` returned the normal invalid-index diagnostic while the prompt remained alive. The two focused tests passed (`cargo test --bin mfb breakpoint_addresses_must_start_on_instruction_boundaries`; `cargo test --bin mfb views_parse_their_types_indexes_and_ranges`), then the full debugger module suite passed: `cargo test --bin mfb debugger::` reported 205 passed and 0 failed. Its first run found a legacy prompt fixture attempting to set/list/remove unaligned AArch64 address `0x1012`; `git log`/`git blame` traced it to the test-splitting commit, its only in-tree use was this prompt test, and the test's behavior is address-breakpoint listing/removal, not acceptance of misaligned instructions. The existing `jump`/`dis` contract, AArch64's 4-byte alignment, and the live `bp main+1` corruption repro prove that input invalid; only the corresponding fixture address references were corrected to aligned `0x1014`. The earlier pre-fix repro outputs are retained above to show the debugger defects were real.

## Remaining untested edges

1. Pin the exact attach-versus-exit phase inside `task_for_pid`/`task_suspend`; the timed kill raced the attach command but CLI output cannot show which side of task-port acquisition it landed on. A 350 ms self-exiting target was already gone when attach began.
2. Attach as an independent macOS user or with a debugger lacking its entitlement; the hardened-target refusal was tested using the properly signed debugger.
3. Stress hundreds of simultaneously live workers and deliberately malformed in-arena collection headers, arena registry overflow, or watchpoint writes spanning adjacent instructions.
4. Test macOS x86-64/Rosetta and attach failures for protected non-MFB tasks; this host and the review targets were Apple Silicon arm64, and PID 1 failed MFB executable validation before task-port acquisition.

## Relevant paths

Shared/CLI and macOS implementation reviewed or consulted:

- `src/cli/dbg.rs`
- `src/debugger/mod.rs`, `arch.rs`, `arm64/`, `backtrace/`, `breakpoints/`, `disassemble.rs`, `environment.rs`, `location.rs`, `names.rs`, `prompt/`, `runtime.rs`, `values/`
- `src/debugger/macos/`
- `src/docs/spec/tooling/10_debugger.md`
- `scripts/sign-debugger.sh`
- `examples/debug/`

The debugger specification is the implementation map and command contract; this review records observations and review judgments without duplicating its full command reference.
