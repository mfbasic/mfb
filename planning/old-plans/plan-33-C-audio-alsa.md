# plan-33-C: Linux Backend — ALSA

Last updated: 2026-07-12
Effort: large (4h-8h)
Depends on: plan-33-A

This sub-plan lands the Linux backend for `audio` on `linux-aarch64`,
`linux-x86_64`, and `linux-riscv64`, for both the glibc and musl flavors, using
ALSA's blocking PCM API resolved at runtime through `dlopen`.

It depends on plan-33-A only. It does not depend on plan-33-B and can land
before, after, or in parallel with it.

References:

- `planning/plan-33-A-audio-surface.md` — public API, `AudioHandle`/`AudioState`
  layout, parameter validation, concurrency contract (§6), and the error-code
  registry (§7): `ErrAudioUnavailable = 7-705-0017` (raised here when
  `libasound.so.2` or a `dlsym` fails, §3.1/§4), `ErrAudioDevice = 7-705-0018`
  (open/configure/stream failure), §3.5 violations reuse `ErrInvalidArgument`.
  This backend adds the matching `ERR_AUDIO_*` triples to
  `src/target/shared/code/error_constants.rs` per plan-33-A §7 (shared with
  plan-33-B — identical names and values) and raises with them.
- `planning/plan-33-B-audio-macos.md` — the sibling backend whose observable
  behavior this one must match exactly.
- `src/target/linux_aarch64/plan.rs:runtime_imports`,
  `src/target/linux_x86_64/plan.rs:runtime_imports`,
  `src/target/linux_riscv64/plan.rs:runtime_imports` — per-spec Linux imports.
- `src/target/linux_x86_64/plan.rs:27` — `LinuxFlavor`; every Linux console
  binary is emitted twice, once against `libc.so.6` and once against
  `libc.musl-x86_64.so.1`.
- `src/os/linux/object.rs:73` — `PlatformImport.library` is a free-form `String`,
  so no linker table edit is needed for a new `.so`.
- `src/target/shared/code/tls/openssl.rs` — precedent for driving an external C
  library from an emitted runtime helper.
- `.ai/remote_systems.md` — the Alpine/musl riscv64 validation host.

## 1. Goal

On each Linux target and flavor, with `libasound.so.2` installed and a default
PCM device present, `audio::devices()`, `audio::openOutput(48000, 2, 512)`,
`audio::write`, `audio::read`, `audio::poll`, and `audio::close` behave exactly
as they do on macOS. Without `libasound.so.2`, every `audio::` call raises
`ErrAudioUnavailable` at the point of use — the binary still loads and every
non-audio program is entirely unaffected.

### Non-goals (explicit constraints)

- No PulseAudio, PipeWire, JACK, OSS, or abstraction layer. ALSA only.
- No virtual/test/null backend, and no silent success when `libasound.so.2` or
  the device is missing.
- No change to the plan-33-A public API, and no Linux-only behavior of any kind.
- No silent resampling and no silent channel conversion. See §3.3.
- No partial-frame reads or writes.
- No `snd_pcm_*` symbol linked at load time. See §3.1.

## 2. Current State

Linux targets import libc symbols per helper spec through `libc_import`
(`linux_aarch64/plan.rs:29`), which stamps the current flavor's libc soname onto
the import. `PlatformImport.library` is an ordinary `String`
(`src/os/linux/object.rs:73`), so naming a third-party `.so` needs no table
edit — unlike macOS, which has closed framework tables.

Every Linux console binary is emitted in two flavors, glibc and musl
(`linux_x86_64/plan.rs:6`). This matters more than it first appears: a direct
`libasound.so.2` `DT_NEEDED` entry would make **both** flavors fail to `exec` on
a host without alsa-lib. The riscv64 validation host is Alpine/musl, which does
not ship alsa-lib by default, so the one Linux target that can actually be
exercised end-to-end is the one most likely to lack the library.

No ALSA symbol is imported today, and no `audio` helper body exists before this
sub-plan. `src/arch/riscv64` has no atomic instructions, consistent with the
other two backends and with plan-33-A §6.

## 3. Design Overview

### 3.1 `dlopen`, not a `DT_NEEDED`

Resolve `libasound.so.2` lazily, on the first `audio::` call, with
`dlopen("libasound.so.2", RTLD_NOW | RTLD_LOCAL)` followed by one `dlsym` per
symbol into a fixed-layout function-pointer table. Cache the table in a
process-wide slot next to the other runtime globals; `dlopen` on an
already-loaded library is cheap and refcounted, so no locking is required for
the cache — the worst case is two threads each resolving it once and storing
identical pointers, and plan-33-A §6 forbids relying on atomics for anything
stronger.

Rationale, in order of weight:

1. **The musl/Alpine reality.** A `DT_NEEDED` on `libasound.so.2` means every
   Linux `mfb` binary that so much as mentions `audio` refuses to start on a host
   without alsa-lib — including the riscv64 box the project actually tests on.
   `dlopen` turns that into a precise `ErrAudioUnavailable` raised at the call
   site, which is a diagnosable runtime error rather than a dynamic-linker fatal.
2. **It is testable in CI without hardware.** "No alsa-lib installed" becomes a
   deterministic, assertable error path (§5.3) instead of an untested branch.
3. **It matches the macOS contract.** There, a missing device also raises
   `ErrAudioUnavailable` at the call. Symmetry is free here.

The cost is that `dlopen`/`dlsym` must be imported. On glibc ≥ 2.34 both live in
`libc.so.6`; on older glibc they live in `libdl.so.2`; on musl both are in libc.
Import them from the flavor's libc soname via `libc_import`, and add a
plan-level test per target that pins which library each resolves from. If the
project's minimum glibc predates 2.34, add `libdl.so.2` as a second
`PlatformImport` library for the glibc flavor only — `PlatformImport.library`
being a free string makes this a one-line change, not a table edit.

Rejected alternative — direct imports for transparent dependency plans: the
transparency is real but is bought with a load-time failure on every host lacking
the library, on a project whose primary Linux test host lacks it.

### 3.2 No callback, no ring

ALSA's blocking `snd_pcm_readi` / `snd_pcm_writei` are called directly from the
MFBASIC thread that invoked `audio::read` / `audio::write`. There is no OS
callback thread, so `AudioState.ring`, `mutex`, and `cond` from plan-33-A §5.1
are **unused on Linux**. The mapping is still created (one page) so that
`AudioHandle` has a single layout across platforms; `ringCapacity` is 0 and the
mutex/cond bytes are left uninitialized and never touched.

This is the whole reason plan-33-A §6's constraint is cheap on Linux: with no
concurrent producer there is nothing to synchronize, and the absence of atomic
instructions never comes up.

`AudioState.osObject` holds the `snd_pcm_t*`. `AudioState.xruns` is a plain `u64`
incremented by the owning thread only.

### 3.3 Configuration is exact or it fails

`snd_pcm_hw_params_set_rate_near` and `snd_pcm_hw_params_set_period_size_near`
adjust their arguments toward what the hardware supports. After
`snd_pcm_hw_params` commits, read back the configured rate and channel count and
**raise `ErrAudioDevice` if either differs from the request**. ALSA's `plughw`
default device will usually satisfy any sane rate by inserting its own
conversion, so this rarely fires — but when it does, the alternative is a program
that silently plays at the wrong speed. Silent resampling is not part of this
API on either platform.

The period size may differ from `bufferFrames`; that is latency, not
correctness, and is accepted without complaint. Set the buffer size to
`bufferFrames * 4` frames to mirror the four-buffer depth of the macOS backend
(plan-33-B §3.2), so `available()` returns comparable numbers on both platforms.

### 3.4 Read, write, poll — matching macOS exactly

The one place the previous draft of this plan went wrong was letting each
platform expose its natural semantics. Blocking `snd_pcm_readi` returns exactly
the requested frame count or an error; a macOS ring naturally short-reads. Those
are different observable behaviors for the same program. plan-33-A §3.2 settles
it: `read` returns exactly `frames`, or fewer only when a `timeoutMs` expires,
and always a whole number of frames. Linux implements that rule rather than
ALSA's:

- **`read(stream, frames)`** — loop `snd_pcm_readi` into the result buffer at the
  current offset for the remaining frame count until all `frames` are read.
  Short reads are normal and simply continue the loop.
- **`read(stream, frames, timeoutMs)`** — compute an absolute deadline once from
  `clock_gettime(CLOCK_MONOTONIC)`. Loop: `snd_pcm_avail_update`; read
  `min(remaining, avail)` frames if `avail > 0`; if more are still needed, call
  `snd_pcm_wait(pcm, remainingMs)` and recompute `remainingMs` from the deadline.
  On expiry, return the whole frames gathered so far — possibly an empty list.
  `timeoutMs = 0` skips `snd_pcm_wait` entirely and returns what is already
  buffered.
- **`write(stream, bytes)`** — validate whole-frame length, then loop
  `snd_pcm_writei` until every frame is accepted.
- **`poll(stream)`** — `snd_pcm_avail_update(pcm) > 0`.
  **`poll(stream, timeoutMs)`** — `snd_pcm_wait(pcm, timeoutMs)`, which returns
  `1` when the device is ready, `0` on timeout, negative on error.
- **`available(stream)`** — `snd_pcm_avail_update(pcm)`, clamped at 0. For
  playback this is frames writable without blocking; for capture, frames
  readable. Both match the macOS meanings.

### 3.5 Recovery and `xruns`

`snd_pcm_readi`, `snd_pcm_writei`, `snd_pcm_avail_update`, and `snd_pcm_wait`
can each return `-EPIPE` (overrun on capture, underrun on playback) or
`-ESTRPIPE` (the device was suspended). On either:

1. Increment `AudioState.xruns` by **one** — one event.
2. Call `snd_pcm_recover(pcm, err, 1)` (silent).
3. If it returns 0, continue the loop. Otherwise raise `ErrAudioDevice` with
   `snd_strerror(err)` text in the message.

Counting events rather than lost frames is not a shortcut, it is the only honest
option: ALSA does not report how many frames an xrun destroyed. plan-33-A §3.3
defines `xruns` as an event count for exactly this reason, and plan-33-B counts
the same way.

`-EINTR` retries without touching `xruns`. Any other negative return raises
`ErrAudioDevice`.

### 3.6 `close`

Set `closed` first, so the path is idempotent and drop-cleanup can re-enter it.
Then `snd_pcm_drain` for playback (blocks until queued frames have played) or
`snd_pcm_drop` for capture (discards), then `snd_pcm_close` exactly once, then
`munmap` the state page. A `snd_pcm_drain` failure is reported but must not skip
`snd_pcm_close`.

### 3.7 Device enumeration

`snd_device_name_hint(-1, "pcm", &hints)` yields a `NULL`-terminated `void**`.
For each hint, `snd_device_name_get_hint(h, "NAME")` gives the `id`,
`"DESC"` gives the `name` (newline-separated; take the first line), and
`"IOID"` gives `"Input"`, `"Output"`, or `NULL` meaning both. Each returned
string is `malloc`'d by ALSA and must be `free`'d — this is the one place the
Linux backend needs `free`, imported from libc. Finish with
`snd_device_name_free_hint(hints)`.

`isDefaultInput` / `isDefaultOutput` are set on the hint whose `NAME` is exactly
`"default"` and whose `IOID` permits that direction. ALSA has no richer notion of
a default, and inventing one would diverge from macOS in a way no program could
rely on.

## 4. Symbols

Resolved by `dlsym` from `libasound.so.2`, never imported:

`snd_pcm_open`, `snd_pcm_close`, `snd_pcm_hw_params_malloc`,
`snd_pcm_hw_params_free`, `snd_pcm_hw_params_any`, `snd_pcm_hw_params_set_access`,
`snd_pcm_hw_params_set_format`, `snd_pcm_hw_params_set_channels`,
`snd_pcm_hw_params_set_rate_near`, `snd_pcm_hw_params_set_period_size_near`,
`snd_pcm_hw_params_set_buffer_size_near`, `snd_pcm_hw_params_get_rate`,
`snd_pcm_hw_params_get_channels`, `snd_pcm_hw_params`, `snd_pcm_prepare`,
`snd_pcm_readi`, `snd_pcm_writei`, `snd_pcm_avail_update`, `snd_pcm_wait`,
`snd_pcm_drain`, `snd_pcm_drop`, `snd_pcm_recover`, `snd_strerror`,
`snd_device_name_hint`, `snd_device_name_get_hint`, `snd_device_name_free_hint`.

A `dlsym` returning `NULL` for any of these raises `ErrAudioUnavailable` naming
the missing symbol — a wrong-ABI `libasound` must not be tolerated.

Constants (verify against `alsa/pcm.h` before landing):
`SND_PCM_STREAM_PLAYBACK = 0`, `SND_PCM_STREAM_CAPTURE = 1`,
`SND_PCM_ACCESS_RW_INTERLEAVED = 3`, `SND_PCM_FORMAT_S16_LE = 2`.

Imported per audio spec from the flavor's libc (`libc_import`):
`dlopen`, `dlsym`, `free`, `clock_gettime`. `mmap`/`munmap` go through the
existing platform hook.

## 5. Phases

### Phase 1 - Import planning and the unavailable path

This lands the load-bearing negative case first, because it is the one CI can
prove on every target.

- [ ] Add per-spec `dlopen`/`dlsym`/`free`/`clock_gettime` imports to all three
      Linux `plan.rs` files.
- [ ] Emit the `libasound.so.2` resolver: `dlopen`, the `dlsym` table, the
      cached slot, and `ErrAudioUnavailable` on any failure.
- [ ] Tests: plan/object tests proving **no** `libasound.so.2` `DT_NEEDED` entry
      appears on any target or flavor, for audio and non-audio programs alike;
      a plan test pinning which library `dlopen` resolves from per flavor.
- [ ] Tests: `tests/rt-error/audio/func_audio_devices_unavailable` runs with a
      poisoned loader path and asserts `ErrAudioUnavailable`.

Acceptance: on all three Linux targets, both flavors, an audio binary links and
`exec`s on a host with no alsa-lib, and its first `audio::` call raises
`ErrAudioUnavailable` naming the library.
Commit: -

### Phase 2 - Device enumeration

- [ ] Extend `src/target/shared/code/audio/` with `alsa.rs`, dispatched by
      `CodegenPlatform::target()`.
- [ ] Emit `_mfb_rt_audio_audio_devices` per §3.7, `free`ing every hint string.
- [ ] Tests: `func_audio_devices_valid` (host-gated) asserts a nonempty list
      containing an entry with `id = "default"`.

Acceptance: on a Linux host with alsa-lib, the device list matches the `NAME`
column of `aplay -L` / `arecord -L`.
Commit: -

### Phase 3 - Open, write, close

- [ ] Emit `openOutput` (both overloads) per §3.3, `write`, `available`, `poll`
      (both overloads), `xruns`, `close` (§3.6).
- [ ] Emit the §3.5 recovery loop shared by every PCM entry point.
- [ ] Tests: `rt-behavior` open/write/close; `rt-error` for every plan-33-A §3.5
      parameter violation, non-whole-frame `write`, `write` after `close`, and
      double `close`. (Swapped-direction calls are compile errors, covered once
      in plan-33-A Phase 1.)

Acceptance: a native program writes 200 ms of a 440 Hz `s16le` tone to the
default playback device, closes, exits 0 — audibly, on real Linux hardware.
`valgrind` reports no leak of the hw_params object or the state page.
Commit: -

### Phase 4 - Read and poll

- [ ] Emit `openInput` (both overloads), `read` (both overloads) implementing the
      §3.4 deadline loop, and capture `available`/`poll`.
- [ ] Tests: `func_audio_read_valid` (host-gated), `func_audio_read_timeout_valid`
      asserting whole-frame alignment and that `timeoutMs = 0` returns
      immediately, `rt-error` for `read` after `close`.

Acceptance: `audio::read(in, 4800)` on a 1-channel 48 kHz stream returns exactly
9600 bytes with at least one nonzero sample from a live capture device; a
`timeoutMs = 0` read on a just-opened stream returns an empty list; a
`timeoutMs = 50` read that cannot be satisfied returns a whole-frame-aligned
short list rather than blocking.
Commit: -

## Compatibility / Format Impact

Nothing beyond plan-33-A, now available on Linux. Critically, **no Linux binary
gains a `DT_NEEDED` entry for `libasound.so.2`**, in either flavor, whether or
not it uses `audio`. The dependency is entirely dynamic and lazy. Programs that
never call `audio::` never `dlopen` anything.

## Validation Plan

- Tests: the negative path (Phase 1) runs everywhere, on every target and
  flavor, with no audio hardware. The positive paths are host-gated.
- Runtime proof: on at least one glibc Linux host with alsa-lib and real
  hardware — tone playback audible, capture nonzero, `xruns` zero across a
  5-second loop, open-by-`id` reaching the named device. On the Alpine/musl
  riscv64 host, either install alsa-lib and repeat, or record
  `ErrAudioUnavailable` as the *expected and asserted* result and state plainly
  that musl/riscv64 playback is **unverified**, not "working".
- Doc sync: deferred to plan-33-D.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- `dlopen` source library — **decided (2026-07-12):** `libc_import("dlopen")`.
  The tree already resolves `dlopen`/`dlsym` from libc for the crypto backend
  (`src/target/linux_aarch64/plan.rs:74` — "glibc ≥ 2.34 folds dlopen/dlsym into
  libc"), and every glibc host in `.ai/remote_systems.md` (Arch, Kali, Debian 12,
  Ubuntu) ships glibc ≥ 2.34. No `libdl.so.2` fallback is needed; mirror the
  crypto backend's import path exactly.
- Device string for `open` — recommended: pass the `AudioDevice.id` hint `NAME`
  straight to `snd_pcm_open`, and `"default"` for the no-device overloads;
  alternative: prefer `"plughw:N"`, which exposes hardware indices that mean
  nothing on macOS.
- Buffer sizing — recommended: `bufferFrames * 4` total buffer to mirror the
  macOS four-buffer depth so `available()` agrees across platforms; alternative:
  let ALSA choose, which makes `available()` platform-dependent.

## Summary

Linux is the easy backend precisely because ALSA's blocking API needs no callback
thread, so plan-33-A §6's no-atomics constraint costs nothing here. The two
decisions that carry weight are `dlopen` over `DT_NEEDED` — which keeps every
Linux binary runnable on hosts without alsa-lib, including the project's own
riscv64 test box — and implementing plan-33-A's exact-or-timeout `read` rule on
top of ALSA rather than exposing ALSA's own semantics.
