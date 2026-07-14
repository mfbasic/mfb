# Program Startup and Teardown

Every native program has one compiler-generated entry function that sets up the
main thread's arena, installs runtime hooks, runs the language entry, routes its
result, and tears the process down. This topic owns that sequence for **console
mode**. App-mode builds diverge: the standard sequence runs on a worker thread
under the app program symbol while `_main` is the toolkit bootstrap — see
`./mfb spec architecture native`.

The entry is emitted by `lower_program_entry`; it carries the assembler label
`entry` and is renamed `_main` at link time (console mode). The macapp variant
emits the same body under the app program symbol instead.
[[src/target/shared/code/entry_and_arena.rs:lower_program_entry]]

## Entry Frame

The entry has no callee-save prologue: it carves its whole frame with one
`sp -= entry_stack_size`, points `x19` (`ARENA_STATE_REGISTER`) at `sp`, and
that stack region **is** the main arena-state for the life of the program. `x19`
and the arena live as long as the process; they are never restored because the
entry exits rather than returns. [[src/target/shared/code/entry_and_arena.rs:lower_program_entry]]

Because the arena-state lives on the stack (not zero-filled memory), the shim
explicitly clears the header words it depends on before the first allocation:
arena-state offsets `0`/`8`/`16`/`24`, the free-list head at offset `48`
(`ARENA_FREE_LIST_HEAD_OFFSET`), the cleanup-failure audit triple at `64`/`72`/`80`
(only when the program can record a cleanup failure), and every program global
slot. The full arena-state byte layout is owned by `./mfb spec memory arenas`.
[[src/target/shared/code/error_constants.rs:ARENA_FREE_LIST_HEAD_OFFSET]]

Slot layout in the entry frame:

```text
entry frame (base = sp = x19)
  +0                      ArenaState      ; ARENA_STATE_SIZE = 3768 bytes, the main arena-state
  +3768 ENTRY_SEED_SCRATCH getentropy buf ; ENTRY_SEED_SCRATCH_OFFSET = ARENA_STATE_SIZE (RNG-seed scratch word)
  +3776 ENTRY_GLOBALS[0..]               ; ENTRY_STACK_SIZE = 3776; globals, LINK slots, term:: state
  +top-48  args region (arg-accepting entries only; base = frame size - 48)
           +0 argc  +8 argv  +16 args List ptr  +24 data length  +32 saved count
```

`ENTRY_STACK_SIZE` is `3776`; globals begin there. The in-frame
scratch word at `ARENA_STATE_SIZE` (`3768`) is the RNG-seed `getentropy` buffer.
(The arena-state's 128 quick bins begin at `ARENA_QUICK_BIN_BASE_OFFSET = 104`.)
For an arg-accepting entry, a dedicated 48-byte args region (argc, argv, args
`List` pointer, data length, saved count) is appended ABOVE the globals at the
top of the frame — the slots must not overlap the globals (they are written
after global initialization, while the globals are live) and must not spill
past the frame (at a raw Linux ELF entry the words above the frame are the OS
`argc`/`argv` vector itself). The total frame is `ENTRY_STACK_SIZE + (globals +
LINK + term:: state slots) * 8`, rounded up to 16, plus `ENTRY_ARGS_REGION_SIZE`
(`48`) when the entry accepts args.
[[src/target/shared/code/error_constants.rs:ENTRY_ARGS_REGION_SIZE]]

When the program uses `term::`, `TERM_STATE_SLOTS` (`u64` each, 27 in all —
leading style slots plus the raw-termios save area) are reserved just past the
program globals and `LINK` slots; the entry's global-slot clear zero-initializes
them, which is the inert (TUI-off) default. [[src/target/shared/code/error_constants.rs:TERM_STATE_SLOTS]]
[[src/target/shared/code/error_constants.rs:TERM_STATE_SLOTS]]

## Publishing the Arena Address

After zero-init, the shim writes `x19` (the arena-state address) into the writable
8-byte global `_mfb_rt_main_arena` (`MAIN_ARENA_GLOBAL_SYMBOL`). This lets the
signal handler and `_mfb_shutdown` find the main arena without the pinned `x19`,
which is unavailable on a signal frame. `x9` is the scratch temporary; `x0`/`x1`
(argc/argv) are left untouched. Only the main arena is tracked here — worker
arenas are never freed by the entry. [[src/target/shared/code/error_constants.rs:MAIN_ARENA_GLOBAL_SYMBOL]]

## Signal Handlers (console only)

In console mode (`module.entry.is_some() && !build_mode.is_app()`) the shim
installs `_mfb_rt_signal_handler` (`SIGNAL_HANDLER_SYMBOL`) for `SIGINT` (`2`) and
`SIGTERM` (`15`) via libc `signal()`. `signal()` clobbers `x0`/`x1`, so argc/argv
are parked 16 bytes below the frame across the two calls (`x19` pins the frame, so
lowering `sp` temporarily is safe) and restored afterward.
[[src/target/shared/code/error_constants.rs:SIGNAL_HANDLER_SYMBOL]]

The handler is `void handler(int signo)`: it runs the shared `_mfb_shutdown`
teardown and then `_exit(128 + signo)` — exit code `130` for `SIGINT`, `143` for
`SIGTERM`. It never returns, so it preserves no interrupted context, and it
locates the arena through `_mfb_shutdown`'s global read rather than the
interrupted `x19`. Its 16-byte frame keeps `sp` aligned across the `bl`s and parks
`signo`. App-mode builds skip handler installation but still share `_mfb_shutdown`
for normal-exit cleanup. [[src/target/shared/code/entry_and_arena.rs:lower_signal_handler]]

## RNG Seeding and Start Time

Two independent PCG64 streams are seeded before any user code runs. Their state
words and the stream algorithm are owned by `./mfb spec memory arenas` (the
`math::rand` stream at arena offsets `88`/`96`, the dedicated memory-fill stream
at `16`/`24`); this topic owns only the startup seeding.

When the program uses `math::rand`/`math::seed` (`seed_rng`), the shim draws 8
entropy bytes from the OS (`emit_random_bytes`) into the as-yet-unused `ENTRY_ARGC`
scratch slot — pre-filled with the arena address so a `getentropy` failure still
yields a varying seed — then calls `_mfb_rng_seed_at` (`RNG_SEED_SYMBOL`) with
`x0 = x19` and the seed in `x1`. [[src/target/shared/code/error_constants.rs:RNG_SEED_SYMBOL]]

The memory-fill stream is seeded unconditionally (entropy fill is always on). The
shim parks argc/argv in callee-saved `x27`/`x28`, captures the arena start time at
arena-state offset `40` via `clock_gettime(CLOCK_REALTIME)` converted to
nanoseconds (`sec * 1e9 + nsec`), draws 8 entropy bytes, XORs them with the start
time and the arena address, calls `_mfb_arena_fill_seed`, then restores
argc/argv. Mixing the start time and arena address keeps two arenas seeding in the
same instant (or after a `getentropy` failure) distinct.
[[src/target/shared/code/error_constants.rs:ARENA_START_TIME_OFFSET]]

## Running the Program

With the arena live, the shim runs the program body and routes the four-register
fallible result (`x0` tag, `x1` value/code, `x2` message, `x3` source — owned by
`./mfb spec memory fallible-call-abi`):

1. **Native `LINK` init.** If the program has `LINK` bindings, call the link-init
   symbol (dlopen/dlsym) first; a non-`RESULT_OK_TAG` result jumps to the error
   path, so a load failure aborts before `main`. [[src/target/shared/code/entry_and_arena.rs:lower_program_entry]]
2. **Global initializer.** If present, call it. A `RESULT_PROGRAM_EXIT_TAG`
   (`2`) routes `x1` to the process exit register and jumps to the exit path; any
   non-`RESULT_OK_TAG` jumps to the error path. [[src/target/shared/code/error_constants.rs:RESULT_PROGRAM_EXIT_TAG]]
3. **Argument list.** If the language entry accepts args, save argc/argv to their
   slots and materialize the argv strings into an in-arena `List OF String`
   (`emit_entry_args_list_materialization`, via `arena_alloc`); the resulting list
   pointer is loaded into `x0` as the entry's argument. [[src/target/shared/code/entry_and_arena.rs:emit_entry_args_list_materialization]]
4. **Language entry.** Call the language entry FUNC/SUB. The result is routed the
   same way: `RESULT_PROGRAM_EXIT_TAG` → exit with `x1`; non-OK → error path. On
   success, a `Nothing` return exits `0`; an `Integer` return becomes the exit
   code but is range-checked against `255` (`ERR_OVERFLOW` if higher).
   [[src/target/shared/code/entry_and_arena.rs:lower_program_entry]]

## Error Path

The error path (`entry_error`) prints the failure to stderr in the form
`Code: <code> Message: <message>\n` (`ENTRY_ERROR_PREFIX` / `ENTRY_ERROR_SEPARATOR`
/ `ENTRY_ERROR_NEWLINE`), stashing the error code in the arena-state exit-status
word at offset `32` and the message pointer in `x20`. When the program can record
cleanup failures, the cleanup-failure audit (count/code/message at arena offsets
`64`/`72`/`80`) is reported next. The process exit code is then forced to `255`.
[[src/target/shared/code/error_constants.rs:ENTRY_ERROR_PREFIX_SYMBOL]]

## Teardown and Exit

The exit path (`entry_exit`) parks the exit code in the arena-state scratch word
at offset `32` (which lives in the stack-resident entry frame, not the mmap'd
arena blocks that teardown unmaps), calls `_mfb_shutdown` (`SHUTDOWN_SYMBOL`),
reloads the exit code, and calls the platform process-exit. [[src/target/shared/code/error_constants.rs:SHUTDOWN_SYMBOL]]

`_mfb_shutdown` reads the arena address from `_mfb_rt_main_arena`, **clears that
global first** (so a signal arriving mid-teardown re-enters as a no-op), and skips
all work if it was already null. It restores the terminal when `term::` was active
(`_mfb_rt_term_term_off`) and frees the main arena (`_mfb_arena_destroy`). It
preserves `x19` and the link register across the call, so the entry's reloaded
exit code is valid on return. Because the global gate is idempotent, the
SIGINT/SIGTERM handler racing the normal exit path cannot double-free.
[[src/target/shared/code/entry_and_arena.rs:lower_shutdown]]

## See Also

* ./mfb spec memory arenas — arena-state layout, `x19`, RNG state words, `arena_destroy`
* ./mfb spec memory fallible-call-abi — the four-register result and the program-exit tag
* ./mfb spec memory heap-values — the in-arena layout of the materialized argv `List OF String`
* ./mfb spec architecture native — app-mode entry divergence and native codegen
* ./mfb spec threading thread-runtime-helpers — per-worker arena seeding and reclamation

## The entry stub scratch is machine-floor, but still architecture-neutral

The process **entry stub** (`lower_program_entry`,
`emit_entry_args_list_materialization`) and the panic-path integer formatter
(`emit_write_integer_to_stderr`) are *machine-floor*: they run **before the arena
and a normal frame exist**, manipulate `sp` and `x19` (the arena base) with
pre-`finalize_frame` offsets, and manage their own stack, so the register
allocator cannot run over them and their scratch cannot be a `%vN` virtual
register the allocator colors. The same is true of the thread trampoline
(`lower_thread_trampoline`), which additionally pins the arena / current-thread /
closure registers across the worker and `pthread_*` calls.

Even so, shared lowering names **no physical register** here: this hand-assigned
scratch is spelled through the neutral `abi::SCRATCH` token pool (`%scratch0`…),
which `abi::realize_abi_token` maps to the AArch64 scratch bank (`x9`–`x18`,
`x20`–`x28`) — the same registers the code has always used, so the emitted code
is byte-identical — and each backend then remaps to its own file. The pinned
registers are neutral tokens too: `abi::ARENA`, `%thread` (current-thread),
`%closure_env`, `%sysnr`, and the `%arg`/`%ret` call-boundary bank.

The same holds for **every register class and every stream**:
the float builders' and SIMD kernels' scratch banks are the `abi::FP_SCRATCH`
(`%fscratch0`…`%fscratch7` → `d0`–`d7`) and `abi::VEC_SCRATCH` (`%vscratch0`…
`%vscratch7` → `v0`–`v7`, the 128-bit lane view of the same file) token pools;
the SIMD math-kernel constant-pool pin is `%mathpool` (→ `x2`); runtime-helper
parameter locations are `%arg` role tokens; and the per-platform emitters that
inject libc/syscall staging into shared streams use the role banks as well
(Darwin's syscall number is `%sysnr_darwin` → `x16`, since the seam is ISA-wide
and Linux's `%sysnr` realizes `x8`). Because `d0`–`d7` sit *inside* the FP
allocatable file, the register allocator's occupancy analysis parses the
scratch tokens directly, at exactly the index of their
realization, so coloring is unchanged. [[src/target/shared/code/regalloc/analysis.rs]]

Two guards enforce the invariant with **no allowlist**:

* a source scan (`shared lowering names no physical register`) — no file in
  shared lowering except the two that define the physical namespace
  (the realization tables and the occupancy
  parsers) may spell a physical register of any class or ISA, quoted or
  dynamically constructed; [[src/target/shared/]] [[src/target/shared/code/abi.rs]] [[src/target/shared/code/regalloc/analysis.rs]]
* an always-on stream assertion at every
  point a shared stream is finished — the pre-selection seam, the hand-built
  helper finalizer, the entry stub, and the thread
  trampoline. A physical name in a shared stream is a build error (an ICE for
  helper bodies), not a silent miscompile. [[src/target/shared/code/regalloc/mod.rs:find_physical_operand]] [[run_register_allocation]] [[finalize_vreg_body_with_locals]]

Standalone per-target streams (the macOS app-mode views, the GTK app
functions, the TLS block trampolines) are target-native machine floor with
hand-built frames — realization-layer code like `arch/`, outside the shared
MIR and outside the invariant.
