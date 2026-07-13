# bug-180 — ALSA/macOS audio low-severity batch (readTimeout, device-id clamp, macOS pollTimeout parity, open-error leaks, dead labels)

Last updated: 2026-07-13
Severity: LOW — latent/feature-level; carved out of bug-167 (whose two MEDIUM findings A and B are fixed).
Class: Correctness + Memory-safety (batched).
Status: FIXED (2026-07-13; goal: resolve bugs 170-180; full acceptance suite green)

## Finding

These are the batched LOW items originally catalogued under bug-167. bug-167's
two headline MEDIUM defects (uninitialized `pollTimeout` timeout; `devices()`
hint-pointer clobber → SIGSEGV) are fixed; these remain:

- **`readTimeout` ignores the timeout** (`src/target/shared/code/audio/alsa.rs`,
  `let _ = timeout` in the read path) and blocks until `frames` are full,
  diverging from the macOS partial-result backend. Fix: honor the timeout and
  return the partial frames read so far, mirroring macOS.
- **Unbounded device-id copy into a fixed buffer** — `emit_device_cstring`
  (`alsa.rs`, 128-byte `NAME_BUF`) and `emit_select_device`
  (`macos.rs`, 256-byte `UID_CSTR`) copy `device.id`'s bytes with no length
  clamp → buffer overrun for an oversized id. Normally short `devices()` output,
  but unenforced. Fix: clamp the copy count to `buffer_len - 1`.
- **macOS has no `audio.pollTimeout` dispatch** (`macos.rs` match omits it →
  codegen `Err`), so the API compiles on Linux but hard-errors on macOS. Fix:
  add a macOS `PollTimeout` arm (AudioQueue-based) or reject it uniformly in the
  resolver.
- **Open-error leaks** — `dev_fail` on both backends (`macos.rs`, `alsa.rs`)
  jumps to `emit_fail` without disposing the created AudioQueue/`snd_pcm` or
  munmapping the state page; the macOS timed partial read abandons the oversized
  pre-alloc'd list; dead labels `tw_ready`/`no_wrap` (`macos.rs`).

## Fix

Address each item as above. The device-id clamp is the only memory-safety item
(latent — real device ids are short); the rest are correctness/feature parity and
cleanup.

## Prior art

Carved out of bug-167 (findings A and B fixed).
