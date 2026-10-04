# bug-803: an app-mode program runs on a thread with the default pthread stack

Last updated: 2026-10-03
Effort: small
Severity: MEDIUM
Class: Correctness (recursion depth, large frames)

Status: Fixed
Regression Test: tests/runtime/rt_app_program_thread_stack.rs

Found while verifying bug-789. The app-mode bootstrap starts the thread that runs
the MFBASIC program with `pthread_create(&tid, NULL, worker, …)`: default attributes.
That is 512 KiB on macOS and 128 KiB on musl, against 8 MiB for a console program
and for every `thread::start` worker (`THREAD_STACK_BYTES`). An app-mode program
therefore gets 1/16 (macOS) or 1/64 (musl) of the stack the same program gets as a
console build.

## Reproduction

`tests/rt-behavior/control-flow/stack-overflow-rt` built with `--app` and run with
`MFB_MACAPP_HEADLESS=1` on macOS: every deep case raises `ErrStackOverflow` as it
should, but the final `recur(10000)` also raises (exit 255). The console build prints
`shallow 10000`. Before bug-789 the same call died by SIGSEGV. On musl, a frame like
the regex engine's (~230 KiB) cannot fit at all.

## Fix

Create the app program thread with an explicit `THREAD_STACK_BYTES` stack
(`pthread_attr_init` / `pthread_attr_setstacksize` / `pthread_attr_destroy`), as
`thread::start` does: `src/target/macos_aarch64/app/bootstrap.rs` (headless
`pthread_create` and `didFinishLaunching`) and `src/target/linux_gtk/bootstrap.rs`
(headless and activate). Each site needs a `pthread_attr_t` (64 bytes) in its frame.

## Resolution

Each bootstrap now spawns the program thread through `emit_spawn_program_thread`:
`pthread_attr_init`, `pthread_attr_setstacksize(THREAD_STACK_BYTES)`,
`pthread_create(&tid, &attr, worker, arg)`, `pthread_attr_destroy`.

- macOS (`src/target/macos_aarch64/app/bootstrap.rs`): the headless site in
  `_main` and `applicationDidFinishLaunching:`. That frame grew to 96 bytes,
  keeping `&argblock` at sp+16 across the attribute calls.
- Linux GTK (`src/target/linux_gtk/bootstrap.rs`): the headless gate in
  `_mfb_gtkapp_main` (frame 112, attr@48) and `activate` (frame 96, attr@32).

The GTK work also found that `_mfb_gtkapp_main` popped 32 bytes of its 48-byte
frame. That is fixed in its own commit.

Verified:
- macOS: `rt_app_program_thread_stack` runs `recur(20000)` headless. It passes
  with the fix; with the staging-5 compiler (no fix) it dies by SIGBUS (wait
  status 10).
- Linux GTK, box 2226 (aarch64 glibc): `stack-overflow-rt` built `--app`, run
  headless from the extracted AppImage under `ulimit -s 256`. glibc sizes a
  default-attribute thread from that rlimit. The fixed build prints every
  `77010002` case and then `shallow 10000`, exit 0. The build without the fix
  dies by SIGSEGV (exit 139).
- Linux x86-64 and musl were not run: no x86-64 or aarch64-musl box is up, and
  riscv64 has no app mode. The emitted sequence is the same arch-neutral `Asm`,
  and both frame growths are multiples of 16, so call alignment is unchanged.
