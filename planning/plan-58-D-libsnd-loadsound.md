# plan-58-D: `libsnd::loadSound` — decoded audio into `audio::write`

Last updated: 2026-07-20
Effort: small (<1h) — **reduced from medium: bug-364 landed and took both of the
draft's "defects to fix in passing" with it (§2.3).**
Depends on: plan-58-A + plan-58-B (`OUT CBuffer`), plan-58-C (the `.mfp` path —
`libsnd` is a binding package, so its wrappers only exist across that boundary).
**bug-364 is no longer a dependency: it has landed.**
Produces: `bindings/libsnd`'s `Sound` type, `loadSound`, `readSamples`. The
feature's deliverable; nothing consumes it.

Adds a streaming read primitive to `bindings/libsnd` and builds `loadSound` on
it, so an MFBASIC program can decode any format libsndfile supports and play it:

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

- `bindings/libsnd/src/lib.mfb` — the binding as it stands **after bug-364**:
  package DOC (`:15-45`), `RESOURCE SoundFile` (`:61`), `TYPE FileInfo` (`:63`,
  `frames` at `:64`), the `LINK` block (`:72-146`) containing `getFormatCount`
  (`:92`), `getFormat` (`:102`), `openFile` (`:115`), `closeFile` (`:124`),
  `errNum` (`:130`), `errMessage` (`:141`); `sndError` (`:148`); `getFormats`
  DOC (`:153-175`) and `getFormats` (`:176`).
- `/Users/justinzaun/local/brew/include/sndfile.h:744` —
  `sf_count_t sf_read_short (SNDFILE *sndfile, short *ptr, sf_count_t items) ;`
  (**items**, not frames — one item is one sample in one channel);
  `:726` is the `sf_readf_short` frame-wise twin; `:368` `sf_count_t = int64_t`;
  `:676` `sf_seek`. **See §2.4 on the provenance risk of these citations.**
- `mfb man audio write` — "interleaved signed 16-bit little-endian (s16le) PCM…
  one frame is `channels * 2` bytes… length must be nonzero and an exact whole
  number of frames"; `mfb man audio openOutput` — `sampleRate` in
  `8000..192000`, `channels` **1 or 2**, and *"channels and sampleRate are not
  resampled"* (`src/docs/spec/stdlib/11_audio.md:137`).
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
  outside `8000..192000`) still loads; `openOutput` is what rejects it, with its
  own diagnostic. `loadSound` does not second-guess the file.
- A file too large for `loadSound` raises a clear error naming the size, rather
  than exhausting the arena.

### Non-goals (explicit constraints)

- **No compiler work.** After plan-58-B every piece of this is ordinary MFBASIC
  plus one `LINK` wrapper. If something here needs a compiler change, that is a
  signal plan-58-A/B/C is incomplete — fix it there, not with a workaround here.
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

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| `LINK` wrappers in the binding | **6** | `rg -n 'SYMBOL ' bindings/libsnd/src/lib.mfb \| wc -l` |
| Distinct libsndfile symbols bound | **5** (`sf_command` ×2, `sf_open`, `sf_close`, `sf_error`, `sf_error_number`) | same, deduped |
| `LINK` block span | `:72-146` | `rg -n '^LINK\|^END LINK' bindings/libsnd/src/lib.mfb` |
| Package version today | **1.2.3** (bug-364 already bumped it) | `rg -n version bindings/libsnd/project.json` |
| Existing libsnd runtime fixtures | 1 (`libsnd-open-file-info-rt`) | `ls tests/rt-behavior/native/` |
| `MAX_LOAD_BYTES` budget at 8 MiB, stereo 48 kHz s16 | **43.7 s** | `8*1024**2 / (2*2*48000)` |
| Same at 64 MiB | **349.5 s ≈ 5.8 min** | `64*1024**2 / (2*2*48000)` |

### 2.2 What the binding does today

`bindings/libsnd` binds **six wrappers over five libsndfile symbols**
(`lib.mfb:72-146`). It can open a file and report its metadata; **it cannot read
a single sample**, because until plan-58-B there is no ABI type able to carry a
buffer (`src/ir/link.rs:16-35`).

`openFile` (`:115`) already produces `RES SoundFile STATE FileInfo`, so a handle
carries its `SF_INFO` and `.state` reads it — the machinery `loadSound` needs is
in place. `FileInfo.frames` (`:64`) is present and correct.

### 2.3 What bug-364 already fixed — do not redo this work

The 2026-07-19 draft listed two defects "to fix in passing" and made bug-364 a
dependency. **All three are done.** Verified 2026-07-20:

| Draft claim | Reality |
|---|---|
| "bug-364 must land first; `frames` does not exist on `FileInfo`" | **Landed.** `FileInfo.frames` at `lib.mfb:64`; `CSTRUCT SfFileInfo.frames CInt64` at `:88` with the bug-364 comment above it; `project.json` already at `1.2.3` |
| "`sndError` prefixes its message with `\"sqlite3: \"`" | **Already fixed.** `lib.mfb:150` reads `error(err, "libsnd: " & …)`. `rg -n 'sqlite3' bindings/libsnd/src/lib.mfb` → 0 matches |
| "`errMessage` binds the wrong symbol `sf_error`" | **Already fixed.** `errMessage` (`:141`) binds `SYMBOL "sf_error_number"` (`:142`), with a comment citing bug-364. `errNum` (`:130`) correctly binds `sf_error` (`:131`) — that one takes a `SNDFILE*` and is right |

This is why this sub-plan is **small**, not medium: Phase 1 shrinks to
`readSamples` plus its DOC and tests. Do not re-file bug-364 and do not "fix" the
`sf_error` binding on `errNum` — it is correct.

### 2.4 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `sf_read_short` counts **items** (samples), not frames | **CONFIRMED** | `sndfile.h:744`, parameter literally named `items`; `:726` is the frame-wise `sf_readf_short` |
| `sf_count_t` is `int64_t` | **CONFIRMED** | `sndfile.h:368` |
| `sf_error_number(int)` returns `const char*` | **CONFIRMED** | `sndfile.h:634`; `sf_error(SNDFILE*)` returns `int` at `:619` |
| Every supported target is little-endian, so s16le needs no swap | **CONFIRMED** | all arches in `project.json` (macos/linux × aarch64/x86_64/riscv64 × glibc/musl) are LE in every configuration this repo builds |
| Building `pcm` with `collections::append` would be O(N²) | **CONFIRMED** | copying a collection value is shrink-to-fit and re-tightens per copy (`05_collections.md:196-200`) — hence one `CBuffer` allocation, not an append loop |
| bug-364 is an open dependency | **FALSE** | §2.3 — landed |
| The two "defects to fix in passing" still exist | **FALSE** | §2.3 — both already fixed |
| **The `sndfile.h` citations are checkable by a reviewer** | **FALSE — provenance risk** | `ls bindings/libsnd/vendor/` holds only `.so`/`.dylib`/`.dll` binaries, **no headers**. The citations resolve against a Homebrew install at `/Users/justinzaun/local/brew/include/`, which is neither vendored nor version-pinned to the bundled `libsndfile.1.0.37`. CI and other machines cannot verify them. See Open Decision 2 |
| `loadSound` is viable for full-length music on a default build | **FALSE** | §2.5 — 43.7 s of stereo 48 kHz, at 344 MB of arena |

### 2.5 The governing constraint — `loadSound` is a sound-effect API today

`kind = 2` is gated off by default (`MFB_KIND2`,
`builder_collection_layout.rs:2191`), so a `List OF Byte` costs 41 bytes per byte
(plan-58-A §Kind-2 gate). That sets `MAX_LOAD_BYTES` and, with it, what `loadSound` is
*for*:

| Build | `MAX_LOAD_BYTES` | Arena cost | Stereo 48 kHz duration |
|---|---|---|---|
| default (`kind = 1`) | **8 MiB** | 344 MB | **43.7 s** |
| `MFB_KIND2=1` | 64 MiB | 64 MB | 349.5 s ≈ **5.8 min** |

Note the 2026-07-19 draft twice described 64 MiB as "≈11 min stereo 48 kHz". That
is the **mono** figure mislabeled — stereo is 5.8 min. The draft's own 8 MiB
figure (43.7 s) was right, and the two were mutually inconsistent by 2×.

**Consequence for the DOC:** `loadSound` must be documented as a
whole-file-into-memory convenience for short audio — effects, stings, loops —
and `readSamples` as the answer for anything longer. That is not a caveat to bury;
it is what the API is.

## 3. Design Overview

Two pieces, one of which is the deliverable:

1. **`readSamples`** — a single `LINK` wrapper over `sf_read_short` using
   `OUT CBuffer`. This is the only new compiler-facing surface, and the only
   thing that can fail in an interesting way.
2. **`loadSound`** — ordinary MFBASIC on top: read `.state.frames` and
   `.channels`, compute the byte count, gate it against `MAX_LOAD_BYTES`, call
   `readSamples` once, wrap in a `Sound`.

**Where design uncertainty concentrates:** in `readSamples`' ABI declaration
being right — specifically that `SIZE` and `LENGTH` are both expressed in
**bytes** while `sf_read_short` speaks **items**. The `* 2` scaling appears in
both clauses and an error in either is silent: too small a `SIZE` truncates,
too large a `LENGTH` walks past what the callee wrote. Phase 1 proves it against
a file whose exact bytes are known.

**Where correctness risk concentrates:** the short-read and error paths.
`sf_read_short` returns `-1` on error and `0` at EOF, and plan-58-B's clamp is
what stops a negative from becoming a huge unsigned `count`. This sub-plan must
exercise both, not assume plan-58-B's unit tests cover them in situ.

**Rejected alternative:** *build `pcm` with `collections::append` in a loop.*
Rejected: copying a collection value is shrink-to-fit and re-tightens per copy
(`05_collections.md:196-200`), so appending N bytes is O(N²). One `CBuffer`
allocation sized from `frames * channels` is the whole point of plan-58-B.

**Rejected alternative:** *use `sf_readf_short` (frame-wise) instead.* Rejected:
it only moves the `* channels` scaling from MFBASIC into the ABI clause without
removing it, and `items` composes more simply with a byte-sized buffer.

**Rejected alternative:** *have `loadSound` resample or downmix to match a
device.* Rejected as a non-goal: `openOutput` already diagnoses unplayable
geometry, and a decoder that silently changes the audio is worse than one that
reports what the file is.

## 4. Detailed Design

### 4.1 `readSamples`

```basic
FUNC readSamples(RES sndfile AS SoundFile, items AS Integer) AS List OF Byte
  SYMBOL "sf_read_short"
  ABI (h CPtr, buf OUT CBuffer, n CInt64) AS got CInt64
  BUFFER buf SIZE items * 2
  SUCCESS_ON got >= 0
  RETURN buf LENGTH got * 2
END FUNC
```

- `SIZE items * 2` — bytes, because one item is one s16 sample.
- `LENGTH got * 2` — `sf_read_short` returns **items read**, so the byte length
  is `got * 2`. plan-58-B clamps it to `[0, SIZE]`.
- `SUCCESS_ON got >= 0` — `>=` is in `IrLinkExpr::Compare`'s operator set
  (`src/ir/link.rs:526-527`), so this is expressible today.
- Both `* 2` scalings need plan-58-B's `IrLinkExpr::Mul`.

### 4.2 `loadSound`

```
info   = openFile(path).state
total  = info.frames * info.channels        ' items
bytes  = total * 2
if bytes > MAX_LOAD_BYTES  -> error naming the size and the cap
pcm    = readSamples(handle, total)
return Sound { samplerate: info.samplerate, channels: info.channels, pcm: pcm }
```

`MAX_LOAD_BYTES` = **8 MiB** while `MFB_KIND2` is off (§2.5). The error must name
both the file's size and the cap — "too large" without numbers is unactionable.

A file that decodes to fewer bytes than `frames * channels * 2` (a truncated or
malformed file) simply produces a shorter `pcm` via the `LENGTH` clamp; that is
not an error condition.

### 4.3 Docs

- `Sound` gets a type DOC per `.ai/man_type_template.md`.
- `loadSound`'s DOC states the `MAX_LOAD_BYTES` cap **in seconds as well as
  bytes** (§2.5) and points at `readSamples` for longer audio.
- `readSamples`' DOC states that `items` counts samples across all channels, and
  that the returned byte count may be shorter than requested at EOF.
- Package DOC (`lib.mfb:15-45`) gains the `loadSound` → `audio::write` example
  from the header of this document.
- `project.json` version 1.2.3 → **1.3.0** (new exports, backward compatible).

## Compatibility / Format Impact

- **Changes:** three new exports (`Sound`, `loadSound`, `readSamples`), a new
  `LINK` wrapper in the block, package version 1.3.0.
- **Blast radius:** adding a wrapper to the `LINK` block changes the emitted
  thunk set, so **`tests/rt-behavior/native/libsnd-open-file-info-rt` will
  churn** — the existing libsnd runtime fixture, which the draft did not mention.
  Its goldens must be re-synced and re-read, not blindly accepted.
- **Unchanged:** `getFormats`, `AudioFormat`, `openFile`, `closeFile`, `errNum`,
  `errMessage`, `sndError`, the `SoundFile` resource and its close op, the
  bundled library set.

## Phases

### Phase 1 — `readSamples` and its byte/item scaling (the uncertain part)

- [ ] Add `readSamples` to the `LINK` block (`lib.mfb:72-146`) exactly as §4.1.
- [ ] Tests: decode a WAV fixture whose exact PCM bytes are known and compare
      byte-for-byte — this is what proves the `* 2` scalings in `SIZE` and
      `LENGTH` are both right.
- [ ] Tests: a short read at EOF (request more items than remain) returns the
      real remaining byte count, not the requested one.
- [ ] Tests: an error path — read from a closed/invalid handle — surfaces through
      `sndError` as an `Error`, exercising `SUCCESS_ON got >= 0` and the clamp on
      a negative `got`.
- [ ] Re-sync `tests/rt-behavior/native/libsnd-open-file-info-rt` goldens and
      confirm the only change is the added thunk.

Acceptance: the WAV fixture decodes byte-identically to a known-good decode; the
EOF short read returns the correct shorter length; the error path produces an
`Error` rather than a crash or a garbage-length list.
Commit: —

### Phase 2 — `Sound`, `loadSound`, the cap, and the docs

- [ ] `EXPORT TYPE Sound` and `EXPORT FUNC loadSound` per §4.2.
- [ ] `MAX_LOAD_BYTES` = 8 MiB, with the over-cap error naming size and cap.
- [ ] DOCs per §4.3; `scripts/update_man.sh`.
- [ ] `project.json` version → 1.3.0.
- [ ] Tests: `loadSound` on WAV **and** FLAC (proving `sf_read_short` converts
      from the native encoding, which is the whole reason to use libsndfile);
      an over-cap file producing the named error; a mono file and a stereo file
      reporting the right geometry.

Acceptance: `loadSound` on a FLAC returns PCM byte-identical to the same audio
decoded from WAV; the over-cap error names both numbers; `mfb man libsnd
loadSound` renders with the duration caveat.
Commit: —

### Phase 3 — end-to-end playback on hardware (largest blast radius last)

- [ ] A program that `loadSound`s a file and plays it through `audio::write`.
- [ ] Run on every supported target per `.ai/remote_systems.md`. Note that file
      lists **11** ssh hosts; enumerate the ones this binding actually supports
      from `bindings/libsnd/project.json` rather than assuming a count.

Acceptance: audible correct playback, and a byte-comparison of the decoded PCM
against a known-good decode, on every target the binding claims to support.
A target that cannot be tested is recorded as untested — not assumed working.
Commit: —

## Validation Plan

- Tests: per phase. The error and short-read paths are mandatory — they are where
  a wrong `LENGTH` becomes an out-of-bounds list rather than a wrong number.
- Coverage check: `tests/rt-behavior/native/` fixtures are golden-backed and in
  the gate's denominator. `tests/acceptance/` has **no** `golden/` dir by design —
  do not place the proof there and assume coverage.
- Runtime proof: Phase 3's playback, plus the byte-comparison against a
  known-good decode. Playback alone is not proof — it is possible to hear
  plausible audio from a subtly wrong buffer.
- Doc sync: package DOC, `Sound` type DOC, `loadSound` and `readSamples` DOCs,
  `scripts/update_man.sh`.
- Acceptance: the project's full suite, with `libsnd-open-file-info-rt` churn
  reviewed rather than accepted.

## Open Decisions

1. **`MAX_LOAD_BYTES` = 8 MiB vs. tracking the `MFB_KIND2` gate.** Recommended:
   fixed 8 MiB, documented with its duration, until kind = 2 is the default. A
   cap that changes with an env var is a support problem. Revisit when the gate
   flips — at which point the DOC's duration figure changes too. (§2.5)
2. **Vendor or pin `sndfile.h`.** The ABI of the bundled `libsndfile.1.0.37` is
   currently asserted from an unrelated local Homebrew header that no reviewer or
   CI job can check (§2.4). Recommended: vendor the matching header under
   `bindings/libsnd/vendor/` so every `sndfile.h:NNN` citation in this plan is
   verifiable. This is how bug-364 happened. (§2.4)
3. **Whether `readSamples` should be exported at all**, or kept internal with
   only `loadSound` public. Recommended: export it — it is the only way to play
   audio longer than the cap, and §2.5 makes that a common case, not an edge one.
   (§1)

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **bug-364 has landed; the draft's entire "two defects to fix in
  passing" section was stale.** `frames` is present (`lib.mfb:64`, `:88`),
  `sndError` already says `"libsnd: "` (`:150`), and `errMessage` already binds
  `sf_error_number` (`:142`). `project.json` is already at 1.2.3. Effort dropped
  medium → small; Phase 1 lost two of its four tasks.
- 2026-07-20 — **The binding has 6 wrappers over 5 symbols, not "four entry
  points".** The `LINK` block is `:72-146`, not `:86-134`.
- 2026-07-20 — **Every `lib.mfb` line citation in the draft was off by 6-14
  lines**, because bug-364's landing shifted them. All re-measured above.
- 2026-07-20 — **"64 MiB ≈ 11 min stereo 48 kHz" was wrong by 2×** (it is the
  mono figure). Actual: 5.8 min stereo. It also contradicted the draft's own
  correct "8 MiB ≈ 43 s". Both figures re-derived in §2.5.
- 2026-07-20 — **`MAX_LOAD_BYTES` must be 8 MiB, not 64 MiB**, because
  `MFB_KIND2` is off by default and the 41× cost is live. This changes what the
  API is for (§2.5), not just a constant.
- 2026-07-20 — **`libsnd-open-file-info-rt` will churn** and was unmentioned in
  the draft's blast radius.
- 2026-07-20 — **`sndfile.h` is not vendored**; all header citations are
  unverifiable off this machine. Raised as Open Decision 2.

## Summary

The engineering risk here is small and concentrated in one declaration: whether
`SIZE items * 2` and `LENGTH got * 2` are both right, given `sf_read_short`
speaks items and `CBuffer` speaks bytes. Phase 1 answers that against a file with
known bytes.

The larger truth about this sub-plan is a product one, not an engineering one:
with `MFB_KIND2` off, `loadSound` holds **43.7 seconds** of stereo 48 kHz audio
and spends 344 MB doing it. It is a sound-effect API until the gate flips, and
the DOC has to say so.

What is left untouched: every existing wrapper, the `SoundFile` resource, the
bundled library set, and the `audio` package — which this binding still does not
depend on.
