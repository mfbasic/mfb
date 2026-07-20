# plan-58-D: `libsnd::loadSound` — decoded audio into `audio::write`

Last updated: 2026-07-19
Effort: medium (1h–2h)
Depends on: bug-364 (the `SF_INFO` fix — `frames` is what sizes the buffer),
plan-58-A + plan-58-B (`OUT CBuffer`), plan-58-C (the `.mfp` path — `libsnd` is a
binding package, so its wrappers only exist across that boundary)

The deliverable. Adds a streaming read primitive to `bindings/libsnd` and builds
`loadSound` on it, so an MFBASIC program can decode any format libsndfile
supports and play it:

```basic
IMPORT audio
IMPORT libsnd

SUB main()
  LET s = libsnd::loadSound("chime.flac")
  RES out AS AudioOutput = audio::openOutput(s.samplerate, s.channels, 512)
  audio::write(out, s.pcm)
END SUB
```

The single behavioral outcome: `loadSound` returns a `Sound` whose `pcm` is
interleaved s16le PCM that `audio::write` plays back as the original recording,
at the rate and channel count `loadSound` reports — verified by ear and by
byte-comparison against a known-good decode, on every supported target.

References (read first):

- `bindings/libsnd/src/lib.mfb` — the binding as it stands: `SoundFile`
  (`:61`), `FileInfo` (`:63`), the `LINK` block (`:71-135`), `openFile` (`:109`),
  `sndError` (`:137`), `getFormats` (`:165`).
- `bugs/bug-364-libsnd-sf-info-missing-frames-field.md` — **read first.** `frames` does not
  exist on `FileInfo` until it lands, and the values `openFile` reports today are
  wrong.
- `/Users/justinzaun/local/brew/include/sndfile.h:744` —
  `sf_count_t sf_read_short (SNDFILE *sndfile, short *ptr, sf_count_t items) ;`
  (**items**, not frames — one item is one sample in one channel);
  `:726` is the `sf_readf_short` frame-wise twin; `:368` `sf_count_t = int64_t`;
  `:676` `sf_seek`.
- `mfb man audio write` — "interleaved signed 16-bit little-endian (s16le) PCM…
  one frame is `channels * 2` bytes… length must be nonzero and an exact whole
  number of frames"; `mfb man audio openOutput` — `sampleRate` in `8000..=192000`,
  `channels` **1 or 2**, and *"channels and sampleRate are not resampled"*.
- `planning/old-plans/plan-50-G-libsnd-binding-getformats.md` — the precedent for
  a libsnd deliverable, including its all-target hardware-validation obligation.
- `.ai/man_template.md`, `.ai/man_type_template.md`, `scripts/update_man.sh` —
  DOC/man authoring rules.
- `.ai/compiler.md` (Hard Completion Gate), `.ai/remote_systems.md` (the boxes).

## 1. Goal

- `bindings/libsnd` exports:
  ```basic
  EXPORT TYPE Sound
    samplerate AS Integer
    channels   AS Integer
    pcm        AS List OF Byte
  END TYPE

  EXPORT FUNC loadSound(path AS String) AS Sound
  EXPORT FUNC readSamples(RES sndfile AS SoundFile, items AS Integer) AS List OF Byte
  ```
- `loadSound` decodes the **whole** file to interleaved s16le PCM in one
  allocation, sized exactly from `frames * channels`.
- `readSamples` is the streaming primitive: it reads at most `items` samples and
  returns however many bytes libsndfile produced, so a caller can play a file
  larger than `loadSound`'s cap by looping.
- Every format the bundled libsndfile supports works — WAV, FLAC, Ogg/Vorbis,
  Opus, AIFF — because `sf_read_short` converts from the file's native encoding.
- A file whose geometry `audio` cannot play (more than 2 channels, or a rate
  outside `8000..=192000`) still loads; `openOutput` is what rejects it, with its
  own diagnostic. `loadSound` does not second-guess the file.
- A file too large for `loadSound` raises a clear error naming the size, rather
  than exhausting the arena.

### Non-goals (explicit constraints)

- **No compiler work.** After plan-58-B every piece of this is ordinary MFBASIC
  plus one `LINK` wrapper. If something here needs a compiler change, that is a
  signal plan-58-A/C is incomplete — fix it there, not with a workaround here.
- **No resampling, no channel mixing, no format conversion beyond what
  `sf_read_short` does.** `loadSound` reports the file's real geometry; matching
  it to a device is the caller's job.
- Do not change `getFormats`, `AudioFormat`, `openFile`, `closeFile`, the
  `SoundFile` resource, or its close op.
- Do not add a dependency on the `audio` package from `libsnd`. They are
  independent: `libsnd` produces bytes, `audio` consumes them, and coupling them
  would force every `libsnd` importer to link `audio`.
- No new bundled library, no `project.json` `libraries` change.

## 2. Current State

`bindings/libsnd` binds four libsndfile entry points (`lib.mfb:86-134`):
`sf_command` twice (the format table), `sf_open`, `sf_close`, `sf_error`. It can
open a file and report its metadata; **it cannot read a single sample**, because
until plan-58-B there was no ABI type able to carry a buffer
(`src/ir/link.rs:16-35`).

`openFile` (`:109`) already produces `RES SoundFile STATE FileInfo`, so a handle
carries its `SF_INFO` and `.state` reads it — the machinery `loadSound` needs is
in place. After bug-364 that state includes `frames`.

Two defects in the existing source to fix in passing:

- `sndError` (`:137-140`) prefixes its message with **`"sqlite3: "`** — copied
  from `bindings/sqlite3` and never corrected. It should read `"libsnd: "`.
- `errMessage` (`:130-134`) binds the **wrong symbol**, confirmed against the
  header:
  ```c
  int         sf_error        (SNDFILE *sndfile) ;   /* sndfile.h:619 */
  const char* sf_error_number (int errnum) ;         /* sndfile.h:634 */
  ```
  The binding declares `errMessage(errNum AS Integer) AS String` with
  `SYMBOL "sf_error"` and `ABI (errNum CInt32) AS message CPtr`. So it passes an
  **integer where `sf_error` expects a `SNDFILE *`**, and then marshals the
  returned **`int`** as a `CPtr` to be copied out as a String. That is two wild
  dereferences per call. It is reached by `sndError` (`:138`) on every error
  path, which is precisely when a program is already in trouble. File with
  bug-364.

Neither is in `loadSound`'s happy path, but `loadSound` is the first function that
will *report* an error through `sndError`, so both are on its blast radius.

## 3. Design Overview

Two layers, and the split is the whole design.

**`readSamples` — the LINK wrapper (bytes in, bytes out).**

```basic
FUNC readSamples(RES sndfile AS SoundFile, items AS Integer) AS List OF Byte
  SYMBOL "sf_read_short"
  ABI (sndfile CPtr, ptr OUT CBuffer, items CInt64) AS got CInt64
  BUFFER ptr SIZE items * 2          ' one short per item
  RETURN ptr LENGTH got * 2          ' got is items; the list is bytes
  SUCCESS_ON got >= 0
END FUNC
```

`sf_read_short` is chosen over `sf_readf_short` deliberately: it counts **items**
(samples), so the byte scale is a fixed `× 2` independent of channel count, and
both the `SIZE` and `LENGTH` expressions stay single multiplications. The
frame-wise variant would need `× 2 × channels`, and `channels` is not an ABI slot.

**`loadSound` — ordinary MFBASIC.**

```basic
LET f = openFile(path)                 ' RES, carries SF_INFO as .state
LET items = f.state.frames * f.state.channels
<size gate>
LET pcm = readSamples(f, items)
RETURN Sound[ samplerate := f.state.samplerate, channels := f.state.channels, pcm := pcm ]
```

One `sf_read_short` call, one allocation, exact size — because bug-364 made
`frames` available and correct. This is why A is a hard dependency and not a
nicety: without it there is no way to size the buffer, and the loop-and-append
alternative would copy the whole PCM buffer on every growth.

**Endianness.** `sf_read_short` writes host-native `short`. `audio::write` wants
**little-endian** s16. Every supported target (aarch64, x86_64, riscv64 on macOS
and Linux) is little-endian, so these coincide and no byte-swap is needed.
State this in a source comment rather than leaving it as a silent assumption —
it is the kind of thing that is invisible until a big-endian target appears.

**Where the correctness risk concentrates:** in the geometry handoff, not in the
size. With plan-57 landed a `List OF Byte` costs `40 + N` bytes rather than
`40 + 41N`, so `loadSound` on a 3-minute stereo track asks for 34.6 MB rather
than 1.4 GB — the API is viable for real music, not just sound effects. A cap is
still wanted for a pathological file, but it is a guard rail rather than the
central constraint it was.

**Rejected alternative:** *`loadSound` loops `readSamples` in chunks and appends.*
Rejected: `collections::append` on a value-semantic list copies, so decoding an
N-byte file would be O(N²). The single sized read is both simpler and correct.
Chunked reading is still available to callers via `readSamples`, which is why it
is exported rather than kept private.

**Rejected alternative:** *have `loadSound` return `List OF Byte`, as originally
requested.* Rejected on the evidence: `audio::openOutput` requires the sample rate
and channel count, and a bare byte list carries neither. A caller who guesses gets
playback at the wrong pitch and speed with no error. The `Sound` record costs one
field access at the call site and makes the correct call possible.

## 4. Detailed Design

### 4.1 The size gate

```basic
LET items = f.state.frames * f.state.channels
LET bytes = items * 2
IF bytes > MAX_LOAD_BYTES THEN
  FAIL error(<code>, "libsnd: " & path & " decodes to " & toString(bytes) &
                     " bytes of PCM, over loadSound's " &
                     toString(MAX_LOAD_BYTES) & "-byte limit; use readSamples to stream it")
END IF
```

`MAX_LOAD_BYTES` should be **at or below** plan-58-B's `CBUFFER_MAX_BYTES` so the
error names `loadSound` and its remedy rather than surfacing from the marshaler.
With plan-58-B's recommended 64 MiB that is ≈11 minutes of stereo 48 kHz — well
past any reasonable in-memory decode, so the gate fires only on a file that would
have exhausted the arena anyway, and `readSamples` remains the answer for
streaming.

Guard `frames` and `channels` for sanity before multiplying: a file libsndfile
could not fully parse can report `frames = 0` (legal — an empty file) or, for a
stream of unknown length, `-1`. Reject a negative product explicitly rather than
letting it reach the `CBuffer` size gate as a huge unsigned value.

### 4.2 Error mapping

Fix `sndError`'s `"sqlite3: "` prefix to `"libsnd: "`, and repoint `errMessage`
at `sf_error_number` (§2, confirmed against `sndfile.h:634`):

```basic
  FUNC errMessage(errNum AS Integer) AS String
    SYMBOL "sf_error_number"
    ABI (errNum CInt32) AS message CPtr
    RETURN message
  END FUNC
```

The returned pointer is a static string owned by libsndfile, so copy-and-leave is
correct and **no `FREE` block** is wanted (`17_native-libraries.md`: a `FREE` on a
library-owned pointer is a wild free).

### 4.3 DOC and man

`Sound`, `loadSound` and `readSamples` all need DOC blocks. Per
`.ai/man_type_template.md` and `.ai/man_template.md`, and following the shape of
the existing `getFormats` DOC (`lib.mfb:142-164`). The `loadSound` DOC must state,
at minimum:

- `pcm` is interleaved s16le, one frame = `channels * 2` bytes — the exact
  contract `audio::write` requires.
- The whole file is decoded into memory, the `MAX_LOAD_BYTES` limit, and that
  `readSamples` is the remedy for longer material.
- `samplerate` and `channels` are the **file's**, not a device's; `openOutput`
  may reject them (more than 2 channels, or a rate outside `8000..=192000`) and
  that is not a `loadSound` failure.
- Which formats work is a property of how the bundled libsndfile was built —
  the same caveat `getFormats`' DOC already carries (`:147-150`).

## Compatibility / Format Impact

- **Changes:** `bindings/libsnd` exports two new functions and one new type; the
  package `version` bumps again (bug-364 took it to `1.2.3`, so `1.3.0` — a
  feature addition).
- **Changes:** `sndError`'s message prefix, and possibly `errMessage`'s symbol
  (§4.2). Both are bug fixes; no correct program depended on either.
- **Unchanged:** `getFormats`, `AudioFormat`, `openFile`, `closeFile`,
  `SoundFile`, the bundled libraries, and the `libraries` manifest table.

## Phases

### Phase 1 — `readSamples` and the error-path fixes

The streaming primitive, landable and useful before `loadSound` exists.

- [ ] Add `readSamples` to the `LINK` block (`bindings/libsnd/src/lib.mfb`), §3.
- [ ] Fix `sndError`'s `"sqlite3: "` → `"libsnd: "` (`:139`).
- [ ] Repoint `errMessage` at `sf_error_number` (§4.2) and record the defect with
      bug-364. Add a runtime case that forces an error (open a nonexistent file)
      and asserts the message is libsndfile's real text — today that path
      dereferences an integer.
- [ ] DOC block for `readSamples` per `.ai/man_template.md`.
- [ ] Tests: `tests/rt-behavior/native/libsnd-read-samples-rt/` — generate a WAV
      of known contents with `fs::writeAllBytes`, open it, `readSamples` it in
      **two** chunks, and assert the concatenation equals the known PCM. Two
      chunks is the point: it proves the handle advances and that a partial read
      truncates correctly.
- [ ] Tests: reading past EOF returns an empty list, and the next call still
      returns empty rather than failing.

Acceptance: `libsnd-read-samples-rt` reconstructs the exact known PCM from two
chunked reads, on macOS/aarch64 and Linux/{aarch64,x86_64,riscv64} ×
{glibc,musl}.
Commit: —

### Phase 2 — `Sound`, `loadSound`, and playback proof

- [ ] Add `EXPORT TYPE Sound` and `EXPORT FUNC loadSound` (§3, §4.1), including
      the size gate and the negative-`frames` guard.
- [ ] Add the endianness comment (§3) at `readSamples`.
- [ ] DOC blocks for `Sound` (`.ai/man_type_template.md`) and `loadSound`
      (`.ai/man_template.md`), covering every point in §4.3. Update the package
      DOC (`lib.mfb:15-45`) so `mfb man libsnd` lists the new API.
- [ ] Bump `bindings/libsnd/project.json` `version` to `1.3.0`.
- [ ] Tests: `tests/rt-behavior/native/libsnd-load-sound-rt/` — generate a WAV of
      known PCM, `loadSound` it, assert `samplerate`, `channels`, and that `pcm`
      is byte-identical to what was written. A generated WAV is the only fixture
      whose expected bytes are known exactly.
- [ ] Tests: the size gate — a file over `MAX_LOAD_BYTES` fails with the specific
      message, and does **not** allocate. Generate a large-but-cheap file (a long
      WAV of silence) rather than committing one.
- [ ] Tests: a **compressed** format round-trip (FLAC is lossless, so the decoded
      PCM is exactly comparable; Ogg/Opus are lossy and cannot be byte-compared).
      Gate it on the format appearing in `getFormats()`, since the bundled build's
      codec set varies per platform — the caveat `getFormats`' own DOC makes.
- [ ] Tests: function-test directories for both new functions per `.ai/compiler.md`
      — valid and invalid, covering wrong argument count and type. Follow the
      current layout (`tests/syntax/<package>/<name>_invalid/`), not the older
      `tests/func_<package>_<func>_*` spelling in that document.

Acceptance: `libsnd-load-sound-rt` reports the generated file's exact geometry and
byte-identical PCM; the FLAC case decodes to the same PCM as the WAV case where
FLAC is available; the oversize case fails with the `loadSound` message and no
allocation. All on macOS/aarch64 and Linux/{aarch64,x86_64,riscv64} ×
{glibc,musl} per `.ai/remote_systems.md`.
Commit: —

### Phase 3 — end-to-end playback (the real proof)

- [ ] Write an example program under the repo's example location: `loadSound` →
      `audio::openOutput(s.samplerate, s.channels, 512)` → `audio::write(out, s.pcm)`.
- [ ] Run it on hardware with audible output and confirm the recording plays at
      the correct pitch, speed, and channel balance.
- [ ] Add the same program as `loadSound`'s DOC `EXAMPLE` block.

Acceptance: **a real sound file plays back correctly through speakers.** A
stereo file with distinct left and right content is the right probe — it catches
channel swaps and interleaving errors that a mono or silent file cannot. Wrong
sample rate is audible as pitch shift; a frame-alignment error as static.
Commit: —

## Validation Plan

- Tests: `tests/rt-behavior/native/libsnd-read-samples-rt/` (chunked reads, EOF);
  `libsnd-load-sound-rt/` (geometry, byte-identity, size gate, FLAC); the
  valid/invalid function-test directories for both new functions.
- Runtime proof: **Phase 3 is the Hard Completion Gate.** Byte-identity against a
  generated WAV proves the decode; only playback proves the contract with
  `audio::write` — the s16le interleaving, the frame alignment, and the geometry
  handoff to `openOutput`. Do not report plan-58 complete on byte-identity alone.
- Hardware coverage: all seven (os, arch, libc) combinations per
  `.ai/remote_systems.md`, as plan-50-G required for `getFormats`. Note that
  audible playback is only possible where there is an audio device — the
  byte-identity tests run everywhere, Phase 3 runs where hardware allows, and any
  target that cannot be proven audibly must be named explicitly rather than
  quietly skipped.
- Doc sync: `bindings/libsnd` DOC blocks only. No spec change — plan-58-A and -C
  own the language-surface documentation.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- **Should `loadSound` also expose `frames`?** It is derivable
  (`bytes / (channels * 2)`), but a caller computing playback duration needs it
  and getting the arithmetic wrong is easy. Recommend adding `frames AS Integer`
  to `Sound` — it is free, since `loadSound` already reads it to size the buffer.
- **Should `readSamples` take frames instead of items?** Items keeps the LINK
  expression to one multiplication (§3) and matches `sf_read_short`. But callers
  think in frames, and `items = frames * channels` is exactly the mistake a
  caller will make. Recommend keeping the wrapper item-based (it mirrors the C
  API it binds) and adding a thin MFBASIC `readFrames(f, frames)` that multiplies
  by `f.state.channels` — the binding layer is the right place for that
  convenience, and it cannot be expressed in the ABI line.
- **`MAX_LOAD_BYTES` at 64 MiB (≈11 min stereo 48 kHz)** follows plan-58-B's
  `CBUFFER_MAX_BYTES`. Recommend keeping them equal and defined in one place, so
  a future change to the cap cannot leave `loadSound` reporting a limit the
  marshaler does not enforce. If plan-57 has not landed, both drop to 8 MiB
  (≈43 s at 41× amplification) — and in that case `loadSound` is a sound-effect
  API, which should be said in its DOC block rather than discovered.

## Summary

The engineering risk is almost entirely in the geometry handoff, not in the
decode or the size: libsndfile does the format work, plan-58-B does the
marshaling, and plan-57 removed the memory cliff that used to dominate this
design. What can still go wrong is a `Sound` whose `samplerate`/`channels` do not
match its `pcm` interleaving — silent in every byte-comparison test, and obvious
the moment it is played. Hence Phase 3.

Untouched: the compiler, `getFormats`, the `SoundFile` resource, the bundled
libraries, and the `audio` package.
