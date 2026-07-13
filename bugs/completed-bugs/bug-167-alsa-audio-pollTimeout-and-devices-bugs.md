# bug-167 — ALSA audio: `pollTimeout` passes an uninitialized stack slot as the timeout, and `devices()` clobbers the hint pointer (SIGSEGV)

Last updated: 2026-07-12
Severity: MEDIUM — two Linux/ALSA audio defects: wrong/garbage poll timeout, and a crashing `devices()` enumeration.
Class: Correctness + Memory-safety.
Status: FIXED (findings A and B — the two named MEDIUM defects)
Resolution: Finding A — `lower_query` now spills the incoming `timeoutMs`
(`ARG[1]`) to `FRAMES_OFF` at entry before any dlopen/libc call clobbers it, so
`PollTimeout` reads the real timeout instead of uninitialized stack. Finding B —
`lower_devices` reloads the hint by dereferencing `HINT_PTR_OFF` for the DESC
`get_hint`, since `emit_string_from_cstr` reused `N_OFF` (which held the hint) as
strlen scratch while building the id String; the DESC lookup no longer passes an
integer as `const void* hint` (SIGSEGV). The batched LOW-severity items
(readTimeout partial-result semantics, the unbounded device-id copy clamp, macOS
`pollTimeout` dispatch parity, open-error resource leaks, dead labels) are
tracked separately as bug-180 — they are latent/feature-level, not the crash/
garbage-timeout defects that define this bug.

## Finding A — `pollTimeout` uses uninitialized `FRAMES_OFF` as the timeout

`src/target/shared/code/audio/alsa.rs:997`. `lower_query` entry (:945-952) stores
only the handle (HANDLE_OFF) and state (STATE_OFF); the incoming `timeoutMs` in
`abi::ARG[1]` is never spilled, and `emit_dlopen` then makes a libc call that
clobbers `ARG[1]`. The `Query::PollTimeout` arm loads `abi::ARG[1]` from
`FRAMES_OFF` (:997) as the `snd_pcm_wait` timeout, but `FRAMES_OFF` is never
written in this function — it is uninitialized stack. The trailing comment
(:1024-1026) even says "Stage the timeout into FRAMES_OFF", but that store is
absent.
- Trigger: `audio.pollTimeout(input, timeoutMs)` on Linux — waits for a garbage
  duration.
- Fix: In `lower_query` entry, add `store_u64(abi::ARG[1], sp, FRAMES_OFF)` before
  the closed-guard (mirroring `lower_read`'s TIMEOUT store).

## Finding B — `devices()` clobbers the hint pointer, corrupting the DESC lookup (SIGSEGV)

`src/target/shared/code/audio/alsa.rs:1118`. `lower_devices` keeps the current
hint pointer in `N_OFF` (stored :1229) and reads it for both
`snd_device_name_get_hint(hint, "NAME")` (:1233) and `...(hint, "DESC")` (:1243).
But `emit_string_from_cstr` (building the id String at :1237) reuses `N_OFF` as
scratch for the computed strlen (`store_u64("%v10", sp, N_OFF)` at :1118). After
the id String is built, `N_OFF` holds the id length (a small integer), so the
DESC `get_hint` at :1243 is called with that integer as the `const void* hint` —
a garbage pointer libasound dereferences (SIGSEGV) or returns NULL (empty device
name).
- Trigger: `audio.devices()` on Linux with at least one PCM hint.
- Fix: Reload the hint from `HINT_PTR_OFF` (deref) before the DESC lookup, or give
  `emit_string_from_cstr` a scratch slot that does not collide with `N_OFF`.

## Related lower-severity ALSA/macOS audio items (batched, LOW)

- `readTimeout` ignores the timeout and blocks until `frames` full
  (`alsa.rs:848`, `let _ = timeout`), diverging from the macOS partial-result
  backend.
- Unbounded device-id copy into a fixed buffer:
  `emit_device_cstring` (`alsa.rs:344`, 128-byte NAME_BUF) and
  `emit_select_device` (`macos.rs:420`, 256-byte UID_CSTR) copy `device.id`'s
  bytes with no length clamp → buffer overrun for an oversized id (normally short
  `devices()` output, but unenforced). Fix: clamp to `buffer_len - 1`.
- macOS has no `audio.pollTimeout` dispatch (`macos.rs:70` match omits it →
  codegen `Err`), so the API compiles on Linux but hard-errors on macOS. Fix: add
  a macOS `PollTimeout` arm or reject it uniformly in the resolver.
- Open-error leaks: `dev_fail` on both backends (`macos.rs:384`, `alsa.rs:487`)
  jumps to `emit_fail` without disposing the created AudioQueue/`snd_pcm` or
  munmapping the state page; macOS timed partial read abandons the oversized
  pre-alloc'd list; dead labels `tw_ready`/`no_wrap` (`macos.rs:1537,1737`).
