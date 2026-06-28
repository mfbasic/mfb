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
  +0                      ArenaState      ; ARENA_STATE_SIZE = 104 bytes, the main arena-state
  +104  ENTRY_ARGC        argc scratch    ; ARENA_STATE_SIZE
  +112  ENTRY_ARGV        argv scratch    ; ENTRY_ARGC + 8  (also ENTRY_STACK_SIZE)
  +120  ENTRY_ARGS_LIST   args List ptr   ; ENTRY_ARGV + 8
  +128  ENTRY_ARGS_DATA_LENGTH            ; ENTRY_ARGS_LIST + 8
  +136  ENTRY_ARGS_COUNT_SAVED            ; ENTRY_ARGS_DATA_LENGTH + 8
  +112  ENTRY_GLOBALS[0..]               ; ENTRY_STACK_SIZE = 112; globals, LINK slots, term:: state
```

`ENTRY_STACK_SIZE` is `112`; globals begin there. The `ENTRY_ARG*` scratch
offsets are computed from `ARENA_STATE_SIZE` (`104`), so `ENTRY_ARGC` sits in the
8 bytes just below the globals region and the remaining arg scratch slots overlap
the start of the globals area. They are transient: `argc`/`argv` are saved and the
args `List` is materialized only after global initialization, immediately before
the language entry consumes it, so the overlap with global slots is never live at
the same time. The total frame is `ENTRY_STACK_SIZE + (globals + LINK + term::
state slots) * 8`, rounded up to 16. [[src/target/shared/code/error_constants.rs:ENTRY_ARGC_OFFSET]]

When the program uses `term::`, eight `TERM_STATE_SLOTS` (`u64` each) are reserved
just past the program globals and `LINK` slots; the entry's global-slot clear
zero-initializes them, which is the inert (TUI-off) default.
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
