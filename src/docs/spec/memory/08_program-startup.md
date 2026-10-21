# Program Startup and Teardown

Every native program has one compiler-generated entry function that sets up the
main thread's arena, installs runtime hooks, runs the language entry, routes its
result, and tears the process down. This topic owns that sequence for **console
mode**. App-mode builds diverge: the standard sequence runs on a worker thread
under the app program symbol while `_main` is the toolkit bootstrap — see
`./mfb spec architecture native`.

The entry is emitted by `lower_program_entry` under the supplied entry symbol
with an internal `entry` label. App mode supplies the app-program symbol.
Physical `x` register names in this topic denote AArch64 realizations; shared
code uses neutral roles, mapped as in `./mfb spec memory native-calling-convention`.
[[src/codegen/engine/function/entry/mod.rs:lower_program_entry]]

## Entry Frame

The entry manages its own frame rather than using `finalize_frame`. It
subtracts `entry_stack_size + shadow + call_parity` and points the arena role
at `sp + shadow`. Windows reserves 32 bytes of shadow space and corrects the
loader entry's stack alignment first. An entry reached as a function call
also uses the backend's call-parity padding (8 on x86-64, 0 on AArch64/RISC-V).
The entry exits rather than restoring this frame and returning.

On macOS and a called entry, argc/argv arrive in argument registers; a raw
Linux entry reads them from the initial stack. When needed, they are preserved
before frame initialization in `SCRATCH[17]`/`SCRATCH[18]`. Windows captures
command-line arguments through its platform hook after arena initialization. [[src/codegen/engine/function/entry/mod.rs:lower_program_entry]]

Because the arena-state lives on the stack (not zero-filled memory), the shim
clears it before the first allocation — **the whole range**, in a loop from the
arena-state base to base + `ARENA_STATE_SIZE`, not a chosen list of header words.
Every program global slot is zeroed after it.

The totality is load-bearing, not defensive: this initializer must stay in
lockstep with the thread-spawn child-state zeroing. This path zeroes
`ARENA_STATE_SIZE` and then every global slot; the thread-spawn path zeroes its
whole worker-arena block in one loop, which covers exactly the same words.
Growing the arena state — adding quick bins, say — or the globals region can then
never leave a new field as garbage in one path but initialized in the other. A
spec that named individual offsets here would invite exactly that divergence.
The full arena-state byte layout is owned by `./mfb spec memory arenas`.
[[src/codegen/error/constants/error_constants.rs:ARENA_STATE_SIZE]] [[src/codegen/engine/function/entry/mod.rs:lower_program_entry]]

Slot layout in the entry frame:

```text
entry arena region (base = arena register = sp + shadow)
  +0                      ArenaState      ; ARENA_STATE_SIZE = 3776 bytes, the main arena-state
  +3776 ENTRY_SEED_SCRATCH getentropy buf ; ENTRY_SEED_SCRATCH_OFFSET = ARENA_STATE_SIZE (RNG-seed scratch word)
  +3784 ENTRY_GLOBALS[0..]               ; ENTRY_STACK_SIZE = 3784; globals, LINK slots, term:: state
  +entry_stack_size-48  args region (arg-accepting entries only)
           +0 argc  +8 argv  +16 args List ptr  +24 data length  +32 saved count
```

`ENTRY_STACK_SIZE` is `3784`; globals begin there. The in-frame
scratch word at `ARENA_STATE_SIZE` (`3776`) is the RNG-seed `getentropy` buffer.
(The arena-state's 128 quick bins begin at `ARENA_QUICK_BIN_BASE_OFFSET = 104`.)
For an arg-accepting entry, a dedicated 48-byte args region (argc, argv, args
`List` pointer, data length, saved count) is appended ABOVE the globals at the
top of the frame — the slots must not overlap the globals (they are written
after global initialization, while the globals are live) and must not spill
past the frame (at a raw Linux ELF entry the words above the frame are the OS
`argc`/`argv` vector itself). `entry_stack_size` is `ENTRY_STACK_SIZE + (globals +
module-resource head + LINK + term:: state + presentation-mode + mouse-state + input-buffer slots) * 8`, rounded up to
16, plus `ENTRY_ARGS_REGION_SIZE` (`48`) when the entry accepts args.
Shadow space and call-parity padding are additional to this size.
[[src/codegen/error/constants/error_constants.rs:ENTRY_ARGS_REGION_SIZE]]

### The conditionally-reserved region chain

Directly after the program's own globals, a program one of whose globals can
reach a resource (a `RES` field or element at any depth) reserves one more word:
the head of its instance's module-owned resource list (`$module_resources`,
`./mfb spec memory heap-values`, "The owner word"). Every other program reserves
nothing there, so its `LINK` slots and later regions keep their offsets.
[[src/codegen/resource/module_owner.rs:MODULE_RESOURCES_GLOBAL]]

Past the program globals and `LINK` slots the frame carries four conditional
regions, each reserved only when the program actually uses the feature, and each
**appended** after the last — never inserted:

```
globals | module-resource head (a global reaches a resource) | LINK | term:: state (uses_term) | presentation mode (uses_app) | mouse state (uses_mouse) | input buffer (uses_input_buffer)
```

Each region's offset is computed from the sizes of the preceding enabled
regions. The same offsets are passed to their emitters.
[[src/codegen/engine/builder/module_layout.rs:plan_arena_layout]]

- **`term::` state** — `TERM_STATE_SLOTS` (`u64` each, 27 in all: the leading
  style slots plus the raw-termios save area), when the program uses `term::`.
  [[src/codegen/error/constants/error_constants.rs:TERM_STATE_SLOTS]]
- **Presentation mode** — `PRESENTATION_MODE_SLOTS` (one word, the `app::Mode`
  discriminant), when the program uses `app::`.
  [[src/codegen/error/constants/error_constants.rs:PRESENTATION_MODE_SLOTS]]
- **Mouse state** — `MOUSE_STATE_SLOTS` (8 words: the decoded-event ring pointer,
  its head and tail cursors, and the stdin decoder's partial-sequence buffer),
  when the program uses `enableMouse`/`pollMouse` from either `term::` or
  `canvas::`. Keyed on those member symbols rather than on a package prefix, so a
  `term::` program that only draws pays nothing; and reserved independently of
  `term::` state, so a canvas-only program gets its ring without carrying 27 slots
  of terminal state it will never read.
  [[src/codegen/error/constants/error_constants.rs:MOUSE_STATE_SLOTS]]

- **Input buffer** — `INPUT_STATE_SLOTS` words, enabled by
  `program_uses_input_buffer` (an `io` reader, or a mouse poll —
  `program_reads_mouse`): decoder state, a pending token and a 4096-byte
  byte buffer for look-ahead input readers.
  [[src/codegen/io/input/mod.rs:INPUT_STATE_SLOTS]]
  [[src/codegen/io/input/mod.rs:program_uses_input_buffer]]

The global-slot clear zero-initializes these regions. Presentation mode is
then seeded to `Mode.None` when that is the program's static default;
otherwise zero denotes `Mode.Console`.
[[src/codegen/engine/function/entry/mod.rs:lower_program_entry]]

## Publishing the Arena Address

After zero-init, the shim writes `x19` (the arena-state address) into the writable
8-byte global `_mfb_rt_main_arena` (`MAIN_ARENA_GLOBAL_SYMBOL`). This lets the
signal handler and `_mfb_shutdown` find the main arena without the pinned `x19`,
which is unavailable on a signal frame. The publication uses a scratch role; needed argc/argv values have already
been preserved. Only the main arena is tracked here — worker
arenas are never freed by the entry. [[src/codegen/error/constants/error_constants.rs:MAIN_ARENA_GLOBAL_SYMBOL]]

## Reserving the Standard Descriptors

Before anything in the program can open a descriptor, a POSIX console entry makes
sure fds 0, 1 and 2 are open. It runs after the arena address is published and the
thread's stack limit is set, and before any `--debug` feature's entry work. For each
of 0, 1, 2 in order, `fcntl(fd, F_GETFD)` is called with the C `int` result
sign-extended. A negative result means the process was started with that descriptor
closed (`prog <&-`, `>&-`, `2>&-`). In that case `open("/dev/null", O_RDONLY)` reserves
fd 0, and `open("/dev/null", O_WRONLY)` reserves fd 1 or 2. Every lower number is open
by then, so POSIX's lowest-free-number rule makes the `open` return exactly that
descriptor. If even `/dev/null` cannot be opened, the entry calls `abort()` rather than
run with the hazard. This is what Rust std and Go do. The path is the read-only C string
`_mfb_rt_dev_null_path` (`DEV_NULL_PATH_SYMBOL`), emitted for every POSIX module with an
entry. [[src/codegen/engine/function/entry/process_setup.rs:emit_reserve_standard_fds]]

Without this, the kernel handed the lowest free number to the program's first
`fs::open`. `io::readLine` then read that file as standard input, and
`io::print`/`io::printError` and the error banners wrote into a file the program had
opened (bug-902). With the numbers reserved, a closed stdin reads as end of input
(`io::readLine` raises `ErrEndOfFile`), and a closed stdout or stderr swallows writes.

An app-mode entry is called on the toolkit's worker thread after the bootstrap has
opened its display connection and window pipe, so it skips this step. The macOS and GTK
`_main` bootstraps run the same reservation as their first act instead
(`./mfb spec app macos-runtime`, `./mfb spec app linux-runtime`). Windows allocates no
standard handle by lowest free number and emits none of it.

## The Arena Memory Cap Check

Unless the project sets `options.maxArenaMemoryDisable: true`, the entry calls
`_mfb_rt_arena_cap_init` next: after the standard descriptors are reserved (so its
messages reach fd 2) and before any `--debug` feature's entry work and the first
allocation. The keys are `./mfb spec tooling project-manifest`, *Arena memory cap*.
A disabled build emits no call, no helper, no limit word and no probe imports.
[[src/codegen/runtime/arena_cap/mod.rs:lower_arena_cap_init]]
[[src/codegen/runtime/arena_cap/mod.rs:arena_cap_enabled]]

1. **Probe the RAM.** macOS calls `sysctlbyname("hw.memsize")`; Linux calls
   `sysinfo` and multiplies `totalram` by `mem_unit`; Windows calls
   `GlobalMemoryStatusEx` and reads `ullTotalPhys`. The call failing is a
   *probe failure* (step 5).
   [[src/codegen/engine/types/types.rs:emit_host_memory_probe]]
2. **Linux: apply the cgroup limit.** The entry reads all of `/proc/self/cgroup`
   (up to 4095 bytes). For the cgroup v2 line (`0::<path>`) it tries
   `/sys/fs/cgroup<path>/memory.max`; for a v1 line whose controller list has
   `memory` it tries `/sys/fs/cgroup/memory<path>/memory.limit_in_bytes`. Each walk
   tries the file at the path, then at every parent up to the root, and the
   effective RAM is the smallest of the host RAM and every limit found. A missing
   or unreadable file, `max`, or a value of 10^18 or more is no limit, never a probe
   failure: a process outside any memory-limited cgroup keeps the host RAM. The
   parent steps cover a limit set on a parent slice and a container that sees its
   own cgroup mounted at `/sys/fs/cgroup` while `/proc/self/cgroup` names a deeper
   host path. A level whose file path would exceed 1023 bytes is skipped.
   [[src/codegen/runtime/arena_cap/cgroup.rs:emit_cgroup_limit]]
3. **Compute the cap.** `requested = RAM × percent / 100`, rounded down, or
   `maxArenaMemoryMiB × 1048576` when that is set and smaller. A `requested` of 0
   (a cgroup limit under 100 bytes) becomes 1, because 0 means "no enforcement".
   The `threshold` is `RAM × 90 / 100`, rounded down. Both products are computed
   as `(RAM / 100) × n + (RAM % 100) × n / 100`, so they cannot overflow.
4. **The 90% check.** If `requested` is above `threshold`, one line goes to
   stderr naming the key that produced `requested` and both byte counts:
   `error: maxArenaMemoryPercent: the arena memory cap of N bytes is above 90% of
   physical memory (T bytes); the program did not start.` Under `"halt"` the entry
   then exits with status 1 through the ordinary exit path (module teardown and
   `_mfb_shutdown` run over the empty arena); no program code, global initializer
   or `LINK` initializer runs. Under `"warn"` the line starts `warning:`, ends
   `(T bytes).`, and the program runs with `requested` as its cap.
5. **Probe failure.** Under `"halt"`: one line naming the failed call
   (`error: maxArenaMemoryPercent: could not read physical memory on this platform
   (sysinfo failed); the program did not start.`) and exit status 1, whatever
   `maxArenaMemoryMiB` says. Under `"warn"`: one `warning:` line, and the cap is
   `maxArenaMemoryMiB × 1048576` when that is set (the 90% check is skipped, having
   no RAM to compare with), or 0 (not enforced) when it is not.
6. **Store the cap** in the writable u64 `_mfb_rt_arena_map_limit`. `0` means no
   enforcement, and only step 5 stores it. The allocator's use of it is in
   `./mfb spec memory arenas`.
   [[src/codegen/runtime/arena_cap/mod.rs:ARENA_MAP_LIMIT_SYMBOL]]

The helper is a vreg-allocated runtime helper with a 5376-byte frame buffer, so
the entry stub itself only calls it and tests the result. With the default
`maxArenaMemoryPercent: 50`, `requested` never exceeds `threshold`, so a default
build prints nothing and behaves as before.

## Signal Dispositions

On non-Windows console entries, the shim
installs `_mfb_rt_signal_handler` (`SIGNAL_HANDLER_SYMBOL`) for `SIGINT` (`2`) and
`SIGTERM` (`15`) via libc `signal()`. `signal()` clobbers `x0`/`x1`, so argc/argv
are parked 16 bytes below the frame across the two calls (`x19` pins the frame, so
lowering `sp` temporarily is safe) and restored afterward.
[[src/codegen/error/constants/error_constants.rs:SIGNAL_HANDLER_SYMBOL]]

The handler is `void handler(int signo)`: it runs the shared `_mfb_shutdown`
teardown and then the platform process-exit with `128 + signo` — exit code `130` for `SIGINT`, `143` for
`SIGTERM`. It never returns, so it preserves no interrupted context, and it
locates the arena through `_mfb_shutdown`'s global read rather than the
interrupted `x19`. Its frame is produced by `finalize_vreg_helper`, which preserves
`signo` across the teardown call and maintains target call alignment. App-mode builds skip handler installation but still share `_mfb_shutdown`
for normal-exit cleanup. [[src/codegen/os/process/process_lifecycle.rs:lower_signal_handler]]

Every POSIX entry, including app mode, installs `SIG_IGN` for `SIGPIPE`.
Socket writes can therefore report errors instead of terminating the process.
The stdout broken-pipe path restores and raises SIGPIPE to preserve pipeline
termination. Windows emits no POSIX signal setup.
[[src/codegen/engine/function/entry/mod.rs:lower_program_entry]]
[[src/codegen/os/syscall/syscall_io.rs:emit_sigpipe_restore_and_raise]]

## RNG Seeding and Start Time

The `math::rand` stream is seeded when the program uses it. Its state words (arena
offsets `88`/`96`) and the stream algorithm are owned by `./mfb spec memory arenas`;
this topic owns only the startup seeding. The arena's memory fill is a zero fill and
needs no seed.

When the program uses `math::rand`/`math::seed` (`seed_rng`), the shim draws 8
entropy bytes from the OS (`emit_random_bytes`) into the dedicated
`ENTRY_SEED_SCRATCH_OFFSET` slot immediately after the arena state — pre-filled with the arena address so a `getentropy` failure still
yields a varying seed — then calls `_mfb_rng_seed_at` (`RNG_SEED_SYMBOL`) with
`x0 = x19` and the seed in `x1`. [[src/codegen/error/constants/error_constants.rs:RNG_SEED_SYMBOL]] [[src/codegen/error/constants/error_constants.rs:ENTRY_SEED_SCRATCH_OFFSET]]

Unconditionally, the shim keeps needed argc/argv in the preserved scratch roles,
captures the arena start time at arena-state offset `40` through the platform
start-time hook (the default uses `clock_gettime(CLOCK_REALTIME)` converted to
nanoseconds), then retains the saved argc/argv for later materialization. No OS
entropy is drawn for a program that does not use `math::rand`/`math::seed`.
[[src/codegen/error/constants/error_constants.rs:ARENA_START_TIME_OFFSET]]

## Running the Program

With the arena live, the shim runs the program body and routes the four-register
fallible result (`x0` tag, `x1` value/code, `x2` message, `x3` source — owned by
`./mfb spec memory fallible-call-abi`):

1. **Closure-descriptor init.** Call `_mfb_closure_desc_init`, which writes each
   no-capture function value's `code` word with `&func` in its static BSS
   descriptor. It runs once, before everything below and before any thread is
   spawned, and cannot fail — there is no tag to check. This is what lets a
   `FunctionRef` be an address rather than an allocation (see
   `./mfb spec memory closures`). [[src/codegen/error/constants/error_constants.rs:CLOSURE_DESC_INIT_SYMBOL]]
2. **Native `LINK` init.** If the program has `LINK` bindings, call the link-init
   symbol (dlopen/dlsym) first; a non-`RESULT_OK_TAG` result jumps to the error
   path, so a load failure aborts before `main`. [[src/codegen/engine/function/entry/mod.rs:lower_program_entry]]
3. **Global initializer.** If present, call it. A `RESULT_PROGRAM_EXIT_TAG`
   (`2`) routes `x1` to the process exit register and jumps to the exit path; any
   non-`RESULT_OK_TAG` jumps to the error path. [[src/codegen/error/constants/error_constants.rs:RESULT_PROGRAM_EXIT_TAG]]
4. **Stdin subscription.** When required, subscribe the main thread to the
   stdin broadcast log after global initialization and before the entry call.
   [[src/codegen/engine/function/entry/mod.rs:lower_program_entry]]
5. **Argument list.** If the language entry accepts args, save argc/argv to their
   slots and materialize the argv strings into an in-arena `List OF String`
   (`emit_entry_args_list_materialization`, via `arena_alloc`); the resulting list
   pointer is loaded into `x0` as the entry's argument. [[src/codegen/engine/function/entry/entry_args.rs:emit_entry_args_list_materialization]]
6. **Language entry.** Call the language entry FUNC. The result is routed the
   same way: `RESULT_PROGRAM_EXIT_TAG` → exit with `x1`; non-OK → error path. On
   success, a `Nothing` return exits `0`; an `Integer` return becomes the exit
   code only in the range `0..255`; an unsigned comparison rejects both
   negative values and values above `255` with `ErrOverflow`.
   [[src/codegen/engine/function/entry/mod.rs:lower_program_entry]]

## Error Path

The error path (`entry_error`) calls the shared failed-thread reporter with the
main MFB identity (`0`), error code, message and optional source. Its labeled
stderr format is specified by `./mfb spec language error-model`. The source and
message remain alive across the call; the reporter does not allocate from the
arena. Native entry return-range and argument-allocation failures explicitly
supply a null source. When enabled, the cleanup-failure audit follows the error
report. The process exit code is then forced to `255`.
[[src/codegen/engine/function/entry/mod.rs:lower_program_entry]]
[[src/codegen/engine/function/entry/entry_args.rs:emit_entry_args_list_materialization]]
[[src/codegen/error/report.rs:lower_error_report]]

## Teardown and Exit

The exit path (`entry_exit`) parks the exit code in the arena-state scratch word
at offset `32` (which lives in the stack-resident entry frame, not the mmap'd
arena blocks that teardown unmaps), calls `_mfb_shutdown` (`SHUTDOWN_SYMBOL`),
reloads the exit code, and calls the platform process-exit. [[src/codegen/error/constants/error_constants.rs:SHUTDOWN_SYMBOL]]

`_mfb_shutdown` reads the arena address from `_mfb_rt_main_arena`, **clears that
global first** (so a signal arriving mid-teardown re-enters as a no-op), and skips
the main cleanup if it was already null. When enabled, it drains stdout,
restores terminal state (`_mfb_rt_term_term_off`), and stops and joins the
graphics thread, in that order. It then frees the main arena
(`_mfb_arena_destroy`) — unless
the program embeds a `thread.` runtime call, in which case the free is skipped on
every platform because a detached worker may still be reading arena memory
(`./mfb spec threading os-integration`). It
preserves `x19` and the link register across the call, so the entry's reloaded
exit code is valid on return. Because the global gate is idempotent, the
SIGINT/SIGTERM handler racing the normal exit path cannot double-free.
[[src/codegen/os/process/process_lifecycle.rs:lower_shutdown]]

In a `--debug` build the last call `_mfb_shutdown` makes, after its `shutdown_done`
label and before it restores `x19`, is `_mfb_debug_shutdown`, which writes the debug
report to stderr; it reads only its own globals, so the reloaded exit code and `x19`
are unaffected. See `./mfb spec tooling debug-report`.
[[src/codegen/debug/shutdown.rs:lower_debug_shutdown]]

## See Also

* ./mfb spec memory arenas — arena-state layout, `x19`, RNG state words, `arena_destroy`
* ./mfb spec memory fallible-call-abi — the four-register result and the program-exit tag
* ./mfb spec memory heap-values — the in-arena layout of the materialized argv `List OF String`
* ./mfb spec architecture native — app-mode entry divergence and native codegen
* ./mfb spec threading thread-runtime-helpers — per-worker arena seeding and reclamation

## The entry stub scratch is machine-floor, but still architecture-neutral

The process **entry stub** (`lower_program_entry`,
`emit_entry_args_list_materialization`) and the panic-path integer formatter
(`emit_write_integer_to_stderr`) are *machine-floor*: they manage frame and scratch state directly, including code that runs
before arena initialization. They manipulate `sp` and the arena role with
explicit offsets, and manage their own stack, so the register
allocator cannot run over them and their scratch cannot be a `%vN` virtual
register the allocator colors. The same is true of the thread trampoline
(`lower_thread_trampoline`), which additionally pins the arena / current-thread /
closure registers across the worker and `pthread_*` calls.

Even so, shared lowering names **no physical register** here: this hand-assigned
scratch is spelled through the neutral `abi::SCRATCH` token pool (`%scratch0`…),
which `abi::realize_abi_token` maps to the AArch64 scratch bank (`x9`–`x18`,
`x20`–`x28`) and each backend maps the roles to its own register file. The pinned
registers are neutral tokens too: `abi::ARENA`, `%thread` (current-thread),
`%closure_env`, `%sysnr`, and the convention-explicit `%argMFB`/`%retMFB`, `%argC`/`%retC` and
`%argSys`/`%retSys` banks.

The same holds for **every register class and every stream**:
the float builders' and SIMD kernels' scratch banks are the `abi::FP_SCRATCH`
(`%fscratch0`…`%fscratch7` → `d0`–`d7`) and `abi::VEC_SCRATCH` (`%vscratch0`…
`%vscratch7` → `v0`–`v7`, the 128-bit lane view of the same file) token pools;
the SIMD math-kernel constant-pool pin is `%mathpool` (→ `x2`); runtime-helper
parameter locations use convention-explicit ABI operands; and the per-platform emitters that
inject libc/syscall staging into shared streams use the role banks as well
(Darwin's syscall number is `%sysnr_darwin` → `x16`, since the seam is ISA-wide
and Linux's `%sysnr` realizes `x8`). Because `d0`–`d7` sit *inside* the FP
allocatable file, the register allocator's occupancy analysis parses the
scratch tokens directly, at exactly the index of their
realization. [[src/codegen/engine/regalloc/analysis.rs]]

The source guard `shared_lowering_names_no_physical_register` scans
instruction-emission contexts in `src/codegen` and `src/target`, excluding
register-namespace definitions and test fixtures. It rejects quoted physical
register operands and constructed physical names. A separate guard rejects
hand-numbered virtual registers across `src/`.
[[tests/guards/architecture_guards.rs:shared_lowering_names_no_physical_register]]
[[tests/guards/architecture_guards.rs:no_hand_picked_vreg]]

Shared-stream finalization also checks for physical operands before selection:
the function allocator, helper finalizer, entry and thread-trampoline paths
reject such operands. Target selection realizes the neutral roles into the
machine register file.
[[src/codegen/engine/regalloc/mod.rs:find_physical_operand]]
[[src/codegen/engine/regalloc/builder_registers.rs:run_register_allocation]]
[[src/codegen/engine/util/vreg_frame.rs:finalize_vreg_body_with_locals]]
