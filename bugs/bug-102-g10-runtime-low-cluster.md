# bug-102 — G10 runtime LOW cluster: temp-file O_CLOEXEC divergence, hardcoded `_main` reloc, TLS int sign-extension, dead arena-state store

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G10). Four independent
LOW/latent findings in the runtime-helper codegen, batched per goal-02.

## 1. macOS `fs::createTempFile` omits O_CLOEXEC (Linux sets it)

`src/target/shared/code/fs_helpers_atomic.rs:246-255` (`temp_file_open_flags`).
Linux flags `524482` = O_RDWR|O_CREAT|O_EXCL|**O_CLOEXEC** (0x80000). macOS
flags `2562` = O_RDWR|O_CREAT|O_EXCL only — macOS O_CLOEXEC (0x1000000) is not
set. The created temp fd is inheritable across exec on macOS, close-on-exec on
Linux. Latent (runtime uses pthreads, no live fork/exec path), but a real
inconsistency worth closing. Fix: OR in 0x1000000 on the macOS temp path.

## 2. `emit_entry_args_list_materialization` hardcodes reloc `from` as `"_main"`

`src/target/shared/code/entry_and_arena.rs:515,527`. `lower_program_entry`
takes an `entry_symbol` that is `"_main"` for the normal entry but a different
symbol for the macOS app entry (line 457). This helper unconditionally builds
relocations with `from: "_main"` (and `push_error_message_address(...,
ERR_ALLOCATION_SYMBOL, ...)`). If an args-accepting entry were emitted under a
non-`_main` symbol (arg-accepting macOS app mode), those relocs would attach to
the wrong function. No live trigger (app-mode programs don't accept argv);
latent. Fix: use `entry_symbol` instead of the literal.

## 3. Linux TLS int-returning libc calls compared signed without sign-extension

`src/target/shared/code/tls/openssl.rs:139-141,200-203,234-237,255-260,
290-292,1373-1375,1668-1670` (socket/connect/poll/getsockopt/accept/SSL_read);
parallel sites in tls/macos.rs. The fs helpers deliberately call
`normalize_c_int_result` (sign-extend w0→x0) before any signed `branch_lt` on a
C-`int` return (the bug-04/bug-44 hazard: upper 32 bits unspecified, `-1` reads
as `+4294967295`, error branch skipped). These TLS helpers compare raw `int`
returns with `branch_lt`/`branch_le` and no sign extension. Empirically benign
on current targets (a broken compare would make connect/accept failures read as
success and crash), so reported as a defensive inconsistency, not a confirmed
miscompile. Same class as bug-04. Fix: route these int returns through
`normalize_c_int_result` too.

## 4. Dead store of `THREAD_OFFSET_PARENT_ARENA_STATE`

`src/target/shared/code/runtime_helpers.rs:392` vs :397-401
(`lower_thread_start_helper`). Line 392 stores `ZERO` into
`%v9[THREAD_OFFSET_PARENT_ARENA_STATE]`; lines 397-401 immediately overwrite
the same slot with `ARENA_STATE_REGISTER`, no intervening read. Redundant
instruction, correctness unaffected. Fix: drop the zero store.
