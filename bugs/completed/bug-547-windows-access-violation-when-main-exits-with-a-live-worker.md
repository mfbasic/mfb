# bug-547: Windows — intermittent `0xC0000005` when `main` returns while an ISOLATED worker thread is still live

Last updated: 2026-09-05
Effort: medium–large (needs a debugger on box 2230)
Severity: HIGH (any Windows program that starts a thread and does not join it can crash ~10% of the time; also flakes the `windows-x86_64` CI row)
Class: correctness / Win64 threading teardown

Status: **FIXED** — `4740507cd`. The main arena is no longer destroyed at exit in a
program that embeds a `thread.` runtime call, on any platform family (Linux
already did this; Windows and macOS did not). Proven on box 2230: 34/40 → 0/40
access violations under `cdb`, 4/75 → 0/75 in the plain ssh loop.

## Symptom

`cli_thread_accept_res_bind::the_repaired_program_runs` on the `windows-x86_64`
CI row:

```
the reproduction must run cleanly, got ExitStatus(ExitStatus(3221225477))
stdout:
started
stderr:
```

`3221225477` = `0xC0000005` = ACCESS_VIOLATION. The program printed `started`,
so it reached the end of `main`; the fault is at or after `main`'s return.

## Reproduction (box 2230, Win11 x86_64)

The test's own program, cross-compiled `-target windows-x86_64`:

```basic
ISOLATED FUNC worker(t AS ThreadWorker OF RES tcp::Socket TO Integer, n AS Integer) AS Integer
  RES s AS tcp::Socket = thread::accept(t, 1000)
  RETURN 1
END FUNC

FUNC main AS Integer
  LET a AS Thread OF RES tcp::Socket TO Integer = thread::start(worker, 0)
  io::print("started")
  RETURN 0
END FUNC
```

`main` returns immediately while the worker is blocked in a 1000 ms
`thread::accept` that will never be satisfied — so the process exits with a
worker thread mid-call. **It is intermittent: 12 runs gave 11 clean exits and one
`0xC0000005`.**

```sh
for i in $(seq 1 12); do ssh -p 2230 test@127.0.0.1 'thr.exe' >/dev/null 2>&1; echo -n "$? "; done
0 0 0 0 0 0 0 0 0 0 0 5
```

**Read `5` as `0xC0000005`** — ssh truncates a remote exit code mod 256, and
`0xC0000005 & 0xFF == 5`. Do not mistake it for a plain exit 5. (Equally, do not
trust `cmd`'s `& echo EXIT=%errorlevel%` on one line: `%errorlevel%` is expanded
at PARSE time, so it reports the PREVIOUS command's code and shows a clean `0`
over a crash. That is what hid this on the first attempt.)

A third harness trap found while fixing it: a `.bat` that loops the exe with
`>nul` **does not reproduce at all** (25/25 clean). The reproducing form is the
one above — one `ssh` per run, with the program's stdout on the ssh **pipe**.
The `io::print` write is what holds the main thread long enough for the worker to
be scheduled into the window.

The same program is clean on macOS and Linux — the test passes on all three Unix
rows.

## It is NOT about `thread::accept`, or resources, or `tcp`

The test that found it uses all three, so the first write-up implied they mattered.
They do not. Same box, a worker that only spins — no `accept`, no `RES`, no
`tcp` import, no resource type parameter:

```basic
ISOLATED FUNC worker(t AS ThreadWorker OF RES Integer TO Integer, n AS Integer) AS Integer
  MUT i AS Integer = 0
  MUT acc AS Integer = 0
  WHILE i < 300000000
    acc = acc + i
    i = i + 1
  END WHILE
  RETURN acc
END FUNC

FUNC main AS Integer
  LET a AS Thread OF RES Integer TO Integer = thread::start(worker, 0)
  io::print("started")
  RETURN 0
END FUNC
```

    15 runs:  0 0 5 0 0 0 0 0 0 5 0 0 0 0 0      <- 2 crashes

**And joining removes it completely.** The identical worker, with
`thread::waitFor(a)` before `RETURN 0`:

    15 runs:  0 0 0 0 0 0 0 0 0 0 0 0 0 0 0      <- 15/15 clean

So the trigger is exactly **process exit while an MFB worker thread is still
running**, and the condition is generic: any Windows MFB program that calls
`thread::start` and returns from `main` without `thread::waitFor` can fault. The
`cli_thread_accept_res_bind` reproduction is one instance, not the shape of the
bug.

Workaround for a user hitting this today: join before returning from `main`.

## Mechanism — the memory, named

`cdb` is installed on 2230 (`C:\Program Files (x86)\Windows Kits\10\Debuggers\x64\cdb.exe`)
and it is a far more sensitive harness than the ssh loop: running the spinning
repro under it faulted in **34 of 40** runs (`findstr /M /C:"c0000005" cdb*.txt`),
because the debugger widens the window. Two consecutive first-chance sites, both
on a non-main thread (`0:003>`):

```
thr+0x6b07  4c8bbb50000000  mov r15, qword ptr [rbx+50h]   ds:00000284`c5a00070=????????????????
thr+0x6b0e  49899f08000000  mov qword ptr [r15+8], rbx     ds:00000282`02f00028=????????????????
            Attempt to write to address 0000028202f00028
            rbx=0000028202ef0020  r15=0000028202f00020
```

Both are in `lower_thread_trampoline`'s prologue, and both operands are named
constants of this repository:

* `rbx` is `abi::CURRENT_THREAD` on x86-64 — the **thread control block**.
* `0x50` = 80 = `THREAD_OFFSET_ARENA_STATE`.
* `r15` is `ARENA_STATE_REGISTER` on x86-64 — the worker's **arena state**.
* `+8` is `ARENA_WORKER_THREAD_OFFSET`, the plan-99 TCB publish.

So the faulting instructions are literally

```rust
abi::load_u64(ARENA_STATE_REGISTER, abi::CURRENT_THREAD, THREAD_OFFSET_ARENA_STATE),
abi::store_u64(abi::CURRENT_THREAD, ARENA_STATE_REGISTER, ARENA_WORKER_THREAD_OFFSET),
```

and `????????????????` on both means both blocks are **unmapped**.

They are unmapped because the main thread freed them. `lower_thread_start_helper`
`arena_alloc`s the thread control block **and** the worker's entire arena-state
block (arena state + the per-arena globals region, `worker_arena_size`) out of the
**spawning** thread's arena — so every worker's two pinned registers point into
the *main* arena's blocks for its whole life. Dropping the `Thread` handle only
runs `thread.drop` (= the Cancel op: set CANCELLED, close the queues, broadcast —
detached semantics, bug-205); it does not join and does not free the TCB. `main`
then returns into `_mfb_shutdown`, which called `_mfb_arena_destroy`, which
`VirtualFree(MEM_RELEASE)`s every main-arena block — including the live worker's
control block and arena state.

The decision was a single flag in `builder::lower_module_for_platform`:

```rust
let family_defers_arena_destroy = match platform.family() {
    PlatformFamily::Linux => true,
    PlatformFamily::MacOS | PlatformFamily::Windows => false,
};
```

Linux had deferred since the beginning, for exactly this reason (`./mfb spec
threading os-integration`: "unmapping shared runtime memory would race that
worker"). Windows was left `false` by plan-47-H pending "when Windows threads
(CreateThread) land" — they landed, and the flag was never revisited. **macOS
carried the identical latent use-after-free** and was only incidentally clean: its
`exit` follows the `munmap` closely enough that the worker rarely gets scheduled in
between (100/100 clean locally on macos-aarch64, so it could not be demonstrated —
but the mechanism is the same code on the same two allocations).

## The fix

`src/codegen/engine/builder/mod.rs`: drop the per-family flag. Every family now
defers, gated exactly as Linux already was — on the program embedding any
`thread.` runtime call:

```rust
let skip_entry_arena_destroy = runtime_symbols.iter().any(|symbol| {
    runtime::spec_for_symbol(symbol)
        .map(|spec| spec.call.starts_with("thread."))
        .unwrap_or(false)
});
```

Why this and not the alternatives:

* **It removes the free, it does not narrow a window.** Windows `ExitProcess`
  terminates the other threads at arbitrary instruction boundaries, so a fix that
  only shortens the race would not be a fix at all. After
  this change *nothing at all* is released between `main`'s return and the platform
  process-exit: `_mfb_shutdown` still drains stdout, still restores the terminal,
  still stops+joins the graphics thread — none of which unmaps memory — and then
  exits. A worker frozen at any instruction cannot fault on memory that is still
  mapped. Residual: zero, for this class.
* **Joining every worker at exit was rejected.** It would change the observable
  semantics of a *correct* program — a detached worker currently does not block
  exit, and `thread::start` + return is a legal shape — which the no-language-
  surface-change constraint forbids. This fix leaves that semantics untouched.
* **Skipping the free costs nothing.** The process is exiting; the OS reclaims
  every block either way. `arena_destroy` at exit is ceremony (it "frees no
  individual values" — `./mfb spec memory arenas`), and the exit code is parked in
  the stack-resident entry frame, not in an arena block.
* **Scope is surgical.** A program with no `thread.` runtime call cannot have a
  worker outlive it and still destroys the arena, byte-identically. The artifact
  gate confirms it: exactly 4 goldens moved.

## Evidence

RED test: `tests/codegen_thread_exit_arena_teardown.rs` (4 tests over 5 targets) —
`runtime.shutdown` must not `bl _mfb_arena_destroy` for a threaded program, must
still do so for a threadless one, and the two must otherwise be identical. Against
the pre-fix compiler 3 of the 4 fail on `windows-x86_64`
(`Calls emitted: ["_mfb_rt_io_stdout_drain", "_mfb_arena_destroy"]`); the
threadless control passes. All 4 pass after.

Runtime, box 2230 (Win11 x86_64, `ver` = 10.0.26100.9168), same source, same
`mfb build --target windows-x86_64`:

| program              | harness                         | pre-fix        | post-fix |
| -------------------- | ------------------------------- | -------------- | -------- |
| spinning worker      | `ssh` loop, one run per ssh     | 4 / 75 faulted | 0 / 75   |
| spinning worker      | under `cdb` (`-g -G -c "g; …"`) | 34 / 40        | 0 / 40   |
| CI shape (`accept`)  | under `cdb`, pre/post INTERLEAVED in one loop | 9 / 30 | 0 / 30 |

The last row is the strongest: it is the exact
`cli_thread_accept_res_bind::the_repaired_program_runs` program, and the pre-fix
and post-fix executables were run alternately inside a single `for /L` loop on the
box, so machine load and scheduling noise are shared between the two arms.

Pre-fix exit-code sequences (`5` = `0xC0000005` truncated mod 256):

```
run 1-25:  0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 5 5 0 0 0 0 0 0 0 0
run 26-75: 0 0 0 0 0 0 0 0 0 0 0 0 0 5 0 0 0 0 0 0 0 0 0 0 0
           0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 5 0 0 0 0 0 0 0 0 0
```

Post-fix exit-code sequence (75 runs):

```
0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0
0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0
0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0
```

Re-verified after merging `main` (bug-537 + bug-545 had landed in between, one of
them in the same `lower_module_for_platform`): full `cargo test --no-fail-fast`
exit 0, `artifact-gate all` 0 diff(s), and the merged-tree executables back on
2230 — CI-shape program 0/30 faults under `cdb`, spinning worker 0/30 in the ssh
loop.

Goldens: `artifact-gate.sh target/release/mfb all` on the branch = **4 diff(s)**,
all four being (thread-using byte-identity fixture) × (family whose flag flipped) —
`byte-identity/thread` and `byte-identity/resource-xfer-slots`, each on
`macos-aarch64` and `windows-x86_64`. No `linux-*` target of the same fixtures
moved (Linux already deferred), and no unrelated fixture moved. Each of those four
was rebuilt with the **pre-fix** compiler and matched its committed golden, so the
baseline was clean and the delta is only this change; regenerated with
`bash scripts/regen-ncodesum.sh target/release/mfb` (bash, not zsh — bug-513) and
the gate is back to 0.

## CI impact

`cli_thread_accept_res_bind::the_repaired_program_runs` was the flake. It is a
REAL crash and the test was doing its job — it was never gated, and it is not
gated now.
