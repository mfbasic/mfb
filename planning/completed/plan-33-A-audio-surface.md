# plan-33-A: Audio Package Surface, Resources, and Runtime Helper Spine

Last updated: 2026-07-12
Overall Effort: x-large (1d-3d)
Effort: medium (1h-2h)
Depends on: nothing

This sub-plan introduces the `audio` built-in package: two move-only resources
(`AudioInput`, `AudioOutput`), one plain `AudioDevice` record, and the nine
public calls that make up the whole raw-PCM API. It lands the frontend, the
resource registry entries, and the runtime helper ABI rows. It emits no helper
bodies — plan-33-B (macOS) and plan-33-C (Linux) do that.

A correct implementation lets an MFBASIC program name a device, open it for
capture or playback, move raw interleaved `s16le` PCM through it, and close it.
No audio file loading, decoding, encoding, mixing, or resampling exists at any
layer of this feature.

References:

- `.ai/compiler.md` - runtime completion gate, mandatory function tests, runtime
  proof, acceptance command.
- `.ai/specifications.md` - embedded spec sync rules and citation requirements.
- `src/builtins/mod.rs:is_builtin_import` - fixed import-gated builtin package
  registry (currently `bits`, `collections`, `crypto`, `csv`, `datetime`,
  `encoding`, `errorCode`, `fs`, `http`, `io`, `json`, `math`, `money`, `net`,
  `os`, `regex`, `strings`, `term`, `thread`, `tls`, `vector` — twenty-one; the
  `money` package landed with plan-29 after this plan's first draft).
- `src/builtins/net.rs` - the closest precedent: a resource package with raw
  `List OF Byte` read/write, a `poll` with an optional `timeoutMs` overload, a
  single `close` shared across several resource types
  (`net.rs:resource_close_function`), and a plain record type `Address` exposed
  through `builtin_type_fields`.
- `src/builtins/resource.rs:BUILTIN_RESOURCES` - single source of truth for
  built-in resources, carrying `sendable` and `close_may_fail`.
- `src/target/shared/plan/lower.rs:storage_for_type` - resource storage is a
  pointer-sized reference.
- `src/target/shared/runtime/mod.rs:RuntimeHelper`,
  `src/target/shared/runtime/catalog.rs:supported_helper_specs` - helper family
  routing and the fixed ABI catalog.
- `src/target/shared/validate.rs:210` - rejects an emitted call whose helper has
  no complete `RuntimeHelperAbi` row.
- `src/target/shared/code/mod.rs:lower_runtime_helper` - shared helper dispatch;
  errors with `native code plan does not emit runtime helper '<symbol>'` for any
  spec without an emit path.

## 1. Goal

After this sub-plan, this program type-checks, resolves, and produces a native
plan naming the right runtime symbols on every target — and fails to *build*
with the explicit `does not emit runtime helper` error until B or C lands:

```basic
IMPORT audio
IMPORT collections

LET devs AS List OF AudioDevice = audio::devices()
LET out AS AudioOutput = audio::openOutput(48000, 2, 512)
audio::write(out, pcmBytes)
audio::close(out)
```

and this one does **not** compile, which is the point of §3.1:

```basic
LET mic AS AudioInput = audio::openInput(48000, 1, 512)
audio::write(mic, pcmBytes)   ' no overload of audio::write over AudioInput
```

### Non-goals (explicit constraints)

- No audio file APIs: no loading, saving, container parsing, codecs, metadata,
  sample libraries, or path-based functions, in this or any sub-plan.
- No implicit decode/encode, format guessing, resampling, or channel mixing. The
  only public format is raw interleaved signed 16-bit little-endian PCM
  (`s16le`).
- No silent fallback device, dummy stream, generated tones, loopback, or default
  byte result. If the OS device cannot be opened or used, the call raises.
- No helper bodies. `lower_runtime_helper` is not touched here.
- No `Float` sample API, no duplex resource, no device hot-plug notification, no
  volume/gain control, no channel maps beyond mono/stereo.
- No change to existing `io::`, `fs::`, `net::`, `term::`, app-mode console IO,
  collection layout, scalar storage, or resource ABI.

## 2. Current State

There is no `audio` package. `src/builtins/mod.rs:is_builtin_import` lists
twenty-one packages and excludes `audio`;
`src/docs/spec/language/18_builtin-functions.md:42` now carries a
`[[src/builtins/mod.rs:is_builtin_import]]` citation and its list was repaired
for `bits`/`crypto`/`encoding`/`vector` since this plan's first draft, but it
still omits `money` (also plan-29). plan-33-D reconciles that list (adding both
`audio` and `money`); this sub-plan must not silently inherit the stale text.

Resource packages have a settled shape. `src/builtins/net.rs` defines
`SOCKET_TYPE`/`LISTENER_TYPE`/`UDP_SOCKET_TYPE`, maps all three to one
`net.close` through `resource_close_function`, exposes the plain record
`Address` through `builtin_type_fields` (`&[("host", "String"), ("port",
"Integer")]`), and overloads `net.poll` on `(Socket)` and `(Socket, Integer)`
inside `resolve_call`. `src/builtins/resource.rs:BUILTIN_RESOURCES` records
`sendable` and `close_may_fail` per resource; `Listener` is already
`sendable: false`, so a non-sendable built-in resource is precedented.

Runtime helper plumbing is data-driven but needs explicit rows: a
`RuntimeHelper` variant, one `RuntimeHelperSpec` per call in a
`<pkg>_specs.rs` file, and a `helper_for_call` arm.
`src/target/shared/validate.rs:210` only rejects helpers reached by an *emitted
call*, so specs may land before bodies — an audio program will fail at
`lower_runtime_helper` with a precise error rather than miscompiling.

Symbol convention is `_mfb_rt_<family>_<pkg>_<member>` — the doubled package
segment is real (`_mfb_rt_net_net_lookup`, `_mfb_rt_os_os_getEnv`), so audio
symbols are `_mfb_rt_audio_audio_openOutput` and so on.

## 3. Design Overview

### 3.1 Two resources, because direction is static

`AudioInput` and `AudioOutput` are separate move-only resource types. Both are
non-sendable. `audio::read` is defined only over `AudioInput`; `audio::write`
only over `AudioOutput`. Passing the wrong one is a **compile error**, caught by
overload resolution, and never reaches codegen.

This is not a stylistic preference. It follows from a fact about the hardware:
**no OS in scope has a duplex stream handle.** `AudioQueueNewInput` and
`AudioQueueNewOutput` return separate queue objects; ALSA's `snd_pcm_open` takes
`SND_PCM_STREAM_PLAYBACK` *or* `SND_PCM_STREAM_CAPTURE`, one direction per
handle. Full duplex is always two handles. A stream's direction is therefore
fixed at open, known statically at every call site, and never observed to change
— which is precisely the condition under which it belongs in the type rather
than in a runtime field.

Rejected alternative — a single `AudioStream` carrying a runtime `kind`:
rejected because it converts a compile error into a runtime error and buys
nothing back. The claim that "the runtime must check direction anyway" is false:
with two types, `read` and `write` resolve to different symbols and perform no
direction check at all. The unified design would add two runtime error paths,
two `rt-error` tests, and a branch in every hot call, to save one entry in
`BUILTIN_RESOURCES`.

The split is fully precedented. `tls` already ships two resource types
(`TlsSocket`, `TlsListener`), one user-facing `tls::close` overload, and two
internal call names with two symbols; `src/builtins/tls.rs:45`'s
`resource_close_function` returns the internal name per type so scope-drop
dispatches statically, and IR lowering rewrites the surface overload onto the
right target. `audio` follows that shape exactly.

The `kind` field survives in `AudioHandle`, but only for the three calls whose
*bodies* differ by direction while no user error is possible — `poll`,
`available`, `xruns` (§5). Those keep one symbol each and branch internally.

### 3.2 The public API

`Stream` below means "either resource type"; those calls are overloaded on both.

| Function | Signature | Behavior |
| --- | --- | --- |
| `audio::devices()` | `() -> List OF AudioDevice` | Enumerates OS audio devices. Never empty on a working host; raises if the OS enumeration API fails. |
| `audio::openInput(sampleRate, channels, bufferFrames)` | `(Integer, Integer, Integer) -> AudioInput` | Opens the **default** capture device. |
| `audio::openInput(device, sampleRate, channels, bufferFrames)` | `(AudioDevice, Integer, Integer, Integer) -> AudioInput` | Opens the named capture device. |
| `audio::openOutput(sampleRate, channels, bufferFrames)` | `(Integer, Integer, Integer) -> AudioOutput` | Opens the **default** playback device. |
| `audio::openOutput(device, sampleRate, channels, bufferFrames)` | `(AudioDevice, Integer, Integer, Integer) -> AudioOutput` | Opens the named playback device. |
| `audio::read(input, frames)` | `(AudioInput, Integer) -> List OF Byte` | Blocks until exactly `frames` frames are captured, then returns `frames * channels * 2` bytes. Raises on stream failure. |
| `audio::read(input, frames, timeoutMs)` | `(AudioInput, Integer, Integer) -> List OF Byte` | Same, but returns early when `timeoutMs` elapses. Returns **whole frames only**, possibly an empty list. `timeoutMs = 0` polls. |
| `audio::write(output, bytes)` | `(AudioOutput, List OF Byte) -> Nothing` | Blocks until every byte is queued for playback. Length must be a nonzero multiple of `channels * 2`. |
| `audio::poll(stream)` | `(AudioInput) -> Boolean`, `(AudioOutput) -> Boolean` | `TRUE` if at least one frame can be read (input) or written (output) without blocking. |
| `audio::poll(stream, timeoutMs)` | `(AudioInput, Integer) -> Boolean`, `(AudioOutput, Integer) -> Boolean` | Waits up to `timeoutMs` for that condition. |
| `audio::available(stream)` | `(AudioInput) -> Integer`, `(AudioOutput) -> Integer` | Frames readable (input) or writable without blocking (output). |
| `audio::xruns(stream)` | `(AudioInput) -> Integer`, `(AudioOutput) -> Integer` | Cumulative overrun (input) / underrun (output) **event** count since open. |
| `audio::close(stream)` | `(AudioInput) -> Nothing`, `(AudioOutput) -> Nothing` | Drains playback, drops capture, releases the OS stream. Drop cleanup routes here. |

`read` and `write` are **not** overloaded across directions. `audio::write(mic,
bytes)` and `audio::read(speaker, 1024)` are compile errors, not runtime errors
— the whole point of §3.1.

`audio::poll(s)` is exactly `audio::available(s) > 0`; it exists because the
`timeoutMs` overload is the ergonomic way to wait, and because `net::poll`
establishes the name. `audio::available` exists because a caller sizing a
`read` needs the count, not the boolean.

### 3.3 Why `xruns` and not silence

A capture ring that fills, or a playback queue that starves, has lost audio. The
alternatives are to raise on the next call (which makes a transient hardware
hiccup fatal and unrecoverable) or to say nothing (which is the "silent
fallback" this feature forbids). A monotonic per-stream counter is the honest
middle: the OS keeps running, the program can see the damage, and the value is
one integer load. Input overruns drop the *oldest* frames; output underruns emit
silence. Both increment `xruns` by exactly one per event.

`xruns` counts **events, not lost frames**, because ALSA does not report how
many frames an overrun or underrun destroyed — `snd_pcm_recover` restores the
stream without saying what it cost. A frame count would be truthful on macOS and
fabricated on Linux. An event count is exact on both.

### 3.4 `AudioDevice`

A plain record, not a resource — it holds no OS handle and needs no close:

```
AudioDevice {
  id              String    // opaque, platform-specific, stable within one run
  name            String    // human-readable
  canInput        Boolean
  canOutput       Boolean
  isDefaultInput  Boolean
  isDefaultOutput Boolean
}
```

Deliberately no `maxChannels` / supported-rate list: ALSA cannot report either
without opening the device, and a field that is truthful on macOS and zero on
Linux is worse than an absent field. A caller discovers a rate/channel
combination by attempting `openInput`/`openOutput` and handling the error.

`id` is a `CFStringRef` device UID on macOS and an ALSA PCM hint `NAME` on
Linux. It is opaque: programs must obtain it from `devices()`, never construct
it. Opening a device whose `id` no longer exists (unplugged between
`devices()` and `open`) raises `ErrAudioDevice`.

### 3.5 Parameter validation

Enforced in the helper, not just by frontend arity, and identically on both
backends:

- `sampleRate` in `8000..=192000`.
- `channels` is `1` or `2`.
- `bufferFrames` in `64..=8192`, and a power of two is *not* required.
- `read` `frames` in `1..=1_048_576`, so `frames * channels * 2` cannot overflow
  a byte-list length.
- `read`/`poll` `timeoutMs` in `0..=86_400_000`.
- `write` byte length nonzero and divisible by `channels * 2`.

Every violation raises before any OS call.

## 4. Frontend Design

Add `src/builtins/audio.rs` modeled on `net.rs` and `tls.rs`:

- Constants: `AUDIO_INPUT_TYPE = "AudioInput"`, `AUDIO_OUTPUT_TYPE =
  "AudioOutput"`, `AUDIO_DEVICE_TYPE = "AudioDevice"`, and one `const` per call
  name. Note the two internal close names, per the `tls` precedent:
  `CLOSE = "audio.close"` is the surface name, and `CLOSE_INPUT =
  "audio.closeInput"` / `CLOSE_OUTPUT = "audio.closeOutput"` are the internal
  targets. `is_audio_call` must accept all three.
- `is_builtin_type`, `builtin_type_fields` (`AUDIO_DEVICE_TYPE => Some(&[...])`),
  `call_param_names`, `call_return_type_name`, `resolve_call`,
  `expected_arguments`, `arity`.
- `resource_close_function`: `AUDIO_INPUT_TYPE => Some(CLOSE_INPUT)`,
  `AUDIO_OUTPUT_TYPE => Some(CLOSE_OUTPUT)`. Scope-drop then reaches the right
  body statically. IR lowering rewrites the surface `audio::close(x)` onto
  whichever internal target `x`'s type selects, exactly as `tls` rewrites its
  listener overload (`src/builtins/tls.rs:48`).
- `resolve_call` carries the overloads: `openInput`/`openOutput` on
  `(Integer, Integer, Integer)` and `(AudioDevice, Integer, Integer, Integer)`;
  `read` on `(AudioInput, Integer)` and `(AudioInput, Integer, Integer)` **and
  no `AudioOutput` form**; `write` on `(AudioOutput, List OF Byte)` **and no
  `AudioInput` form**; `poll` on `(AudioInput)`, `(AudioOutput)`,
  `(AudioInput, Integer)`, `(AudioOutput, Integer)`; `available`, `xruns`, and
  `close` on each type.
- `arity`: `openInput`/`openOutput` `(3, 4)`, `read` `(2, 3)`, `poll` `(1, 2)`,
  the rest exact.
- `expected_arguments` for `read` must name `AudioInput` and for `write`
  `AudioOutput`, so the diagnostic on a swapped stream says what the caller did
  wrong rather than "no matching overload".

Register in `src/builtins/mod.rs`: `mod audio;`, the `is_builtin_import` arm,
the `is_builtin_type` aggregate, and every metadata dispatch that enumerates
packages. Register **both** `AudioInput` and `AudioOutput` in
`src/builtins/resource.rs:BUILTIN_RESOURCES` with `sendable: false` and
`close_may_fail: true`.

`sendable: false` is a v1 constraint with a real consequence, stated here so it
is not discovered later: **a program cannot capture and play back on separate
threads**, because neither resource can cross a thread boundary and both `read`
and `write` block. Single-threaded duplex *is* expressible — open both, then
drive them from one loop with `poll` / `available` / timed `read`. That is why
those three calls exist. Making the resources sendable is deferred until
callback/ring ownership is audited against `thread::transfer`.

Note this is orthogonal to §3.1: even a sendable, unified `AudioStream` would
still be two OS handles, because no backend has a duplex handle to give.

## 5. Runtime Helper Spine

- `RuntimeHelper::Audio` in `src/target/shared/runtime/mod.rs`, with
  `name() => "audio"`.
- `helper_for_call`: `builtins::audio::is_audio_call(name) => Some(Audio)`.
- New `src/target/shared/runtime/audio_specs.rs` with one `RuntimeHelperSpec`
  per *symbol*. `spec_for_call` (`catalog.rs:156`) is a linear first-match on
  the `call` string, so **two specs must never share a `call` value** — every
  overload that needs a different body gets its own internal call name, exactly
  as `tls.close` / `tls.closeListener` do (`net_specs.rs:609,620`).

| Internal call | Symbol | Direction |
| --- | --- | --- |
| `audio.devices` | `_mfb_rt_audio_audio_devices` | — |
| `audio.openInput` | `_mfb_rt_audio_audio_openInput` | in |
| `audio.openInputDevice` | `_mfb_rt_audio_audio_openInputDevice` | in |
| `audio.openOutput` | `_mfb_rt_audio_audio_openOutput` | out |
| `audio.openOutputDevice` | `_mfb_rt_audio_audio_openOutputDevice` | out |
| `audio.read` | `_mfb_rt_audio_audio_read` | in |
| `audio.readTimeout` | `_mfb_rt_audio_audio_readTimeout` | in |
| `audio.write` | `_mfb_rt_audio_audio_write` | out |
| `audio.poll` | `_mfb_rt_audio_audio_poll` | either |
| `audio.pollTimeout` | `_mfb_rt_audio_audio_pollTimeout` | either |
| `audio.available` | `_mfb_rt_audio_audio_available` | either |
| `audio.xruns` | `_mfb_rt_audio_audio_xruns` | either |
| `audio.closeInput` | `_mfb_rt_audio_audio_closeInput` | in |
| `audio.closeOutput` | `_mfb_rt_audio_audio_closeOutput` | out |

Fourteen symbols. `read`/`write`/`open*`/`close*` are direction-specific and
perform **no** runtime direction check — overload resolution already guaranteed
it. `poll`, `pollTimeout`, `available`, and `xruns` accept either type, share
one symbol each, and branch on `AudioHandle.kind` internally. No user error is
reachable through that branch, so it costs a compare and buys four fewer
symbols.

`src/target/shared/validate.rs:210` checks helper completeness **per family,
not per spec**: it treats the `Audio` helper as implemented as soon as *at least
one* of its specs has non-empty `params`/`returns`/`clobbers`. So `audio.devices`
may carry `params: &[]` exactly as `os.pid` does (`os_specs.rs:120`), riding on
the open/read/write specs whose ABI rows are complete. No predicate change and no
placeholder params are needed.

### 5.1 The shared native stream record

Defined here because B and C must agree on it byte-for-byte. The MFBASIC handle
is a pointer-sized arena reference to:

```
AudioHandle {                 // arena; identical layout for both resource types
  u64   kind                  // 1 = input, 2 = output; read only by poll/available/xruns
  u64   closed
  u64   sampleRate
  u64   channels
  u64   bytesPerFrame         // channels * 2
  u64   bufferFrames
  void* state                 // mmap'd, page-aligned; see below
}

AudioState {                  // one mmap'd page, NOT arena memory
  u8[128]  mutex              // pthread_mutex_t (64 B macOS, 40 B glibc; 128 reserved)
  u8[128]  cond               // pthread_cond_t  (48 B both; 128 reserved)
  u64      ringCapacity       // bytes
  u64      ringHead
  u64      ringTail
  u64      xruns              // frames lost
  u64      lastError
  void*    osObject           // AudioQueueRef (macOS) / snd_pcm_t* (Linux)
  u8[]     ring               // remainder of the mapping
}
```

`state` is `mmap`'d, not `malloc`'d: this runtime imports no `malloc`/`free`
anywhere, and the arena itself is built on `mmap`
(`src/target/shared/code/entry_and_arena.rs:1139`). `close` `munmap`s it.
Arena memory is unusable here because an OS audio callback thread touches
`AudioState` while the owning thread's arena may be growing or being freed.

The 128-byte reservations for `pthread_mutex_t` / `pthread_cond_t` are
generous by design; both backends must `pthread_mutex_init` / `pthread_cond_init`
them and must assert the reservation at build time against the platform's real
sizes rather than trusting these numbers.

## 6. Concurrency Contract (binding on B and C)

This section is the reason the backend split works, and it is the constraint
that killed the previous draft of this plan.

**This compiler has no atomic instructions.** There are no acquire/release
loads, no load/store-exclusive pairs, no `lock`-prefixed RMW, no `amoswap`, and
no fences in `src/arch/aarch64`, `src/arch/x86_64`, or `src/arch/riscv64`, and
no atomic ops in the MIR layer. Every existing cross-thread synchronization in
the tree is a `pthread_mutex_*` / `pthread_cond_*` C call
(`src/target/shared/code/runtime_helpers_thread.rs:99`).

Therefore:

- **No audio backend may use a lock-free ring buffer.** A plain-load/plain-store
  SPSC ring is incorrect on AArch64 and RISC-V, which are weakly ordered.
- **Every backend must therefore run its OS audio callback on a thread where
  taking a `pthread_mutex` is legal.** This forbids a Core Audio `AudioUnit`
  render callback, which runs on a realtime thread that must never block.
- macOS satisfies this with `AudioQueue`, whose callbacks run on an ordinary
  internal thread (plan-33-B).
- Linux satisfies this trivially: ALSA's blocking `snd_pcm_readi`/`snd_pcm_writei`
  are called directly from the helper thread and there is no callback at all
  (plan-33-C).

If a future backend needs a realtime render callback, it must first land atomic
MIR ops and encoders on all three architectures. That is a separate plan and a
hard prerequisite, not an implementation detail.

## 7. Error Codes

Two new runtime error codes, both in subsystem `7-705` (package helpers /
builtins), placed after the current highest `7-705` row
(`ErrAuthenticationFailed = 7-705-0016`):

| Code | Integer | Name | Meaning |
| --- | --- | --- | --- |
| `7-705-0017` | `77050017` | `ErrAudioUnavailable` | Audio backend library or device is unavailable (no `libasound.so.2`, no audio device, or capture authorization denied). |
| `7-705-0018` | `77050018` | `ErrAudioDevice` | Audio device open, configuration, or stream operation failed. |

Parameter violations (§3.5) do **not** get a dedicated code — they reuse the
existing `ErrInvalidArgument` (`7-705-0002`).

The registry is single-sourced and generated, so two files must land together:

- **`src/docs/spec/diagnostics/02_error-codes.md`** — append the two rows above to
  the **Constant Registry** table. `build.rs:generate_errorcode_table` regenerates
  `ERRORCODE_CONSTANTS` from this table, and the
  `errorcode.rs:table_matches_registry` drift-guard test re-parses it, so adding
  the rows here is what makes `errorCode::ErrAudioUnavailable` /
  `errorCode::ErrAudioDevice` catchable in user programs. No hand-editing of
  `src/builtins/errorcode.rs` is needed or allowed — it is generated.
- **`src/target/shared/code/error_constants.rs`** — add the matching
  `(code, message, symbol)` triples in the "General runtime (7705)" block, in
  ascending-code order after `ERR_FLOAT_OVERFLOW_*`, following the existing
  naming: `ERR_AUDIO_UNAVAILABLE_CODE/MESSAGE/SYMBOL` and
  `ERR_AUDIO_DEVICE_CODE/MESSAGE/SYMBOL` (message text identical to the registry
  "Meaning" column; symbol `_mfb_str_error_audio_unavailable` /
  `_mfb_str_error_audio_device`). These are the constants the plan-33-B/C helper
  bodies raise with, so the triples land **with those backends**, not in this
  sub-plan — this sub-plan adds only the registry rows so the `errorCode::`
  constants resolve. (An unused triple would otherwise sit dead until a backend
  references it.)

Ordering note: because this sub-plan emits no helper bodies, the registry rows in
`02_error-codes.md` are the only Phase-1/2 change here. The `error_constants.rs`
triples are listed above so B and C use identical names and values, not because
they belong to plan-33-A.

## Compatibility / Format Impact

Externally observable additions:

- New import-gated built-in package: `audio`.
- New runtime error codes `ErrAudioUnavailable` (`7-705-0017`) and
  `ErrAudioDevice` (`7-705-0018`); §3.5 violations reuse `ErrInvalidArgument`.
- New resource types `AudioInput` and `AudioOutput`; new record type
  `AudioDevice`.
- New raw PCM contract: interleaved `s16le`, one frame is `channels * 2` bytes.
- New runtime helper family and `_mfb_rt_audio_audio_*` symbols.

Unchanged: every existing package name, the resource ABI, `List OF Byte` layout,
the native calling convention, `io` standard streams, `fs` semantics, and all
object/binary formats. A program that does not `IMPORT audio` gains no audio
symbol, no platform import, and no dynamic library dependency —
`runtime_imports(&self, spec)` is dispatched per helper spec, so this falls out
of the existing design rather than needing new gating.

## Phases

### Phase 1 - Frontend package and metadata

- [ ] Add `src/builtins/audio.rs` with the constants, metadata functions, and
      `resolve_call` overloads from §4, plus unit tests covering every call,
      every overload, both resource types, and the `AudioDevice` field list.
- [ ] Register `audio` in `src/builtins/mod.rs` (`mod`, `is_builtin_import`,
      `is_builtin_type`, and each package-enumerating metadata dispatch).
- [ ] Register `AudioInput` and `AudioOutput` in `BUILTIN_RESOURCES` as
      `sendable: false`, `close_may_fail: true`; unit-test that
      `resource_close_function` returns `audio.closeInput` / `audio.closeOutput`
      respectively, and that `AudioDevice` is *not* a resource.
- [ ] Wire the surface-`audio::close`-to-internal-target rewrite in IR lowering,
      mirroring the `tls::close`/`tls.closeListener` path.
- [ ] Append the two §7 rows (`ErrAudioUnavailable = 7-705-0017`,
      `ErrAudioDevice = 7-705-0018`) to the **Constant Registry** table in
      `src/docs/spec/diagnostics/02_error-codes.md`; confirm
      `cargo test --bin mfb errorcode` (the `table_matches_registry` drift guard)
      passes and that `errorCode::ErrAudioUnavailable` /
      `errorCode::ErrAudioDevice` fold to their integers. Do **not** hand-edit the
      generated `src/builtins/errorcode.rs`. The `error_constants.rs` triples are
      deferred to plan-33-B/C (§7) since no body raises them yet.
- [ ] Tests: `tests/syntax/audio/` invalid coverage for wrong arity, wrong
      argument types, `audio::` use without `IMPORT audio`, constructing an
      `AudioDevice` literal (must be rejected — it is obtained only from
      `devices()`), and — load-bearing for §3.1 —
      `func_audio_write_input_invalid` and `func_audio_read_output_invalid`,
      which assert that a swapped stream is a **compile** error naming the
      expected type.

Acceptance: `cargo test --bin mfb builtins::audio` passes; the swapped-stream
programs fail to compile with a diagnostic naming `AudioOutput` / `AudioInput`;
scope-drop on each type emits its own close target.
Commit: -

### Phase 2 - Runtime helper spec rows

- [ ] Add `RuntimeHelper::Audio`, its `name()` arm, and the `helper_for_call`
      arm.
- [ ] Add `src/target/shared/runtime/audio_specs.rs` with all fourteen symbol
      rows from §5, each with complete `params`/`returns`/`clobbers` (except
      `audio.devices`, which uses `params: &[]` per §5).
- [ ] Wire the module into `catalog.rs:supported_helper_specs`.
- [ ] Tests: assert every `audio.*` call resolves to a spec, every spec's symbol
      is unique tree-wide, and every spec passes the
      `src/target/shared/validate.rs:210` completeness predicate.

Acceptance: `cargo test --bin mfb target::shared::runtime` passes. Building a
program that calls `audio::openOutput` fails with exactly
`native code plan does not emit runtime helper '_mfb_rt_audio_audio_openOutput'`
— proving the spine is wired and the body is honestly absent.
Commit: -

### Phase 3 - Native plan imports (no bodies)

Note the ordering constraint: `runtime_imports` is spec-driven and needs no
emitted code, but the **object plan** is built from emitted `CodeFunction`s and
therefore cannot be tested until B/C land. Only the native plan is in scope here.

- [ ] Confirm `runtime_imports` on all four targets returns an empty import set
      for audio specs (the real imports arrive with each backend), and add a
      regression test that a non-audio program has no audio symbol.
- [ ] Add a native-plan test per target asserting the audio helper symbols
      appear in the plan's `runtime_helpers` for an audio program and are absent
      otherwise.

Acceptance: native plan JSON for an `IMPORT audio` program names the audio
helpers on `macos-aarch64`, `linux-aarch64`, `linux-x86_64`, and
`linux-riscv64`; a program without `IMPORT audio` names none.
Commit: -

## Validation Plan

- Tests: unit tests for metadata/overload/spec parity; syntax-invalid tests for
  every call; native-plan tests for helper presence and absence.
- Runtime proof: **not applicable to this sub-plan** — it deliberately emits no
  code. The only runtime-observable behavior is the `does not emit runtime
  helper` build error, which Phase 2 asserts. Do not declare `audio` working on
  any platform on the strength of this sub-plan.
- Doc sync: deferred to plan-33-D, which owns the spec and man pages once the
  API is real.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- Zero-arg helper ABI - **decided (2026-07-12):** `validate.rs:210` checks
  completeness per helper *family*, not per spec, so `audio.devices` uses
  `params: &[]` just like `os.pid` (`os_specs.rs:120`). No predicate change and
  no placeholder params — see §5.
- `xruns` vs raising - recommended: a monotonic counter, as argued in §3.3;
  alternative: raise `ErrAudioXrun` on the first call after a loss, which makes
  a recoverable hiccup fatal.
- Error codes - **decided (2026-07-12): add two new codes.** Register
  `ErrAudioUnavailable` and `ErrAudioDevice` under subsystem `7-705` (package
  helpers / builtins), taking the next free numbers after `ErrAuthenticationFailed`
  (`7-705-0016`): `7-705-0017` and `7-705-0018`. §3.5 parameter violations reuse
  the existing `ErrInvalidArgument` (`7-705-0002`). See §7 for the exact rows and
  the two files that must stay in lockstep.

## Summary

Two resources, one record, nine calls, `s16le` frames, blocking-with-timeout
read, blocking write. The load-bearing content of this sub-plan is §6: because
the compiler has no atomics, every backend must confine OS audio work to a
thread on which a mutex is legal. That single constraint determines the macOS
backend choice in plan-33-B and makes plan-33-C nearly trivial.
