# plan-33-D: Audio Docs, Spec, and Cross-Target Validation

Last updated: 2026-07-12
Effort: medium (1h-2h)
Depends on: plan-33-A, plan-33-B, plan-33-C

This sub-plan finishes `audio` by making the contract auditable in the embedded
spec and man pages, and by proving the macOS AudioQueue and Linux ALSA backends
behave identically for the same program.

References:

- `planning/plan-33-A-audio-surface.md` — the normative API and semantics.
- `planning/plan-33-B-audio-macos.md`, `planning/plan-33-C-audio-alsa.md` — the
  two backends and their platform-specific caveats.
- `.ai/man_template.md`, `.ai/man_type_template.md`,
  `.ai/man_package_template.md` — required man-page structure.
- `scripts/update_man.sh`, `scripts/update_man_package.sh` — the driver scripts,
  which carry the authoring rules the bare templates omit.
- `.ai/specifications.md` — embedded spec rules and `[[path:Symbol]]` citation
  requirements.
- `src/docs/spec/stdlib/spec.md` and `src/docs/spec/language/18_builtin-functions.md`
  — spec integration points.
- `src/docs/man/builtins/net/**` — the closest documentation precedent: a
  resource package with raw byte IO, a `poll` timeout overload, and a plain
  record type.

## 1. Goal

- `./mfb man audio` and `./mfb spec stdlib audio` document the exact contract:
  two resources, one record, nine calls, `s16le` frame layout, blocking and timed
  semantics, device selection, the error model, `xruns`, per-platform backends,
  and the non-goals.
- The full acceptance suite passes.
- One identical MFBASIC program is proven to produce identical observable
  behavior on macOS and on Linux.

### Non-goals (explicit constraints)

- No new API while documenting. No codecs, file helpers, mixers, volume,
  hot-plug notification, or format negotiation sneak in during a docs pass.
- No undocumented skips. A hardware-gated proof that did not run reports the
  exact blocker, and the platform is then **not** declared verified.
- No duplicated text. Per-function reference lives in `mfb man audio`; the
  behavioral model lives in the `stdlib audio` spec topic, which links rather
  than restates.

## 2. Current State

`src/docs/spec/stdlib/` runs `01_regex.md` through `10_crypto.md`, so `audio`
takes `11_audio.md`.

`src/docs/spec/language/18_builtin-functions.md:42` lists the recognized package
set as `bits`, `collections`, `crypto`, `csv`, `datetime`, `encoding`,
`errorCode`, `fs`, `http`, `io`, `json`, `math`, `net`, `os`, `regex`, `strings`,
`term`, `thread`, `tls`, `vector`, now with a
`[[src/builtins/mod.rs:is_builtin_import]]` citation. The earlier
`bits`/`crypto`/`encoding`/`vector` drift has since been repaired, but the list
is **still stale**: `is_builtin_import` also contains `money` (added by plan-29),
which §18 omits. This sub-plan owns fixing that — adding both `audio` and
`money` — not just appending `audio` to a wrong list.

Man pages live under `src/docs/man/builtins/<package>/`. Test coverage is split:
`tests/syntax/<pkg>/` for frontend rejections, `tests/rt-error/<pkg>/` for
runtime errors, `tests/rt-behavior/<pkg>/` for valid runtime behavior.
`.ai/compiler.md` requires valid *and* invalid coverage for every created or
modified function.

## 3. Documentation Design

Man pages under `src/docs/man/builtins/audio/`, each following its template
exactly:

- `package.md` (`.ai/man_package_template.md`) — overview, platform availability,
  the `s16le` contract, the `IMPORT audio` gate, and the explicit statement that
  no file/codec API exists or is planned.
- `types.md` (`.ai/man_type_template.md`) — `AudioInput` and `AudioOutput` (both
  move-only, non-sendable, closed by `audio::close`, drop-cleaned) and
  `AudioDevice` (plain record, six fields, obtained only from
  `audio::devices()`). State that direction is part of the type: `read` takes an
  `AudioInput`, `write` takes an `AudioOutput`, and swapping them does not
  compile.
- One page per function (`.ai/man_template.md`): `devices.md`, `openInput.md`,
  `openOutput.md`, `read.md`, `write.md`, `poll.md`, `available.md`,
  `xruns.md`, `close.md`. Pages for overloaded calls document every overload.

Spec files:

- `src/docs/spec/stdlib/11_audio.md` — the behavioral model: frame layout,
  `read`'s exact-or-timeout rule, `write`'s block-until-queued rule, what
  `available` means in each direction, `xruns` as an event count, the
  non-sendable/no-duplex consequence, the two backends, and the **error model**:
  `ErrAudioUnavailable` (`7-705-0017`), `ErrAudioDevice` (`7-705-0018`), and
  `ErrInvalidArgument` for §3.5 parameter violations — the two audio codes are
  added to the registry by plan-33-A §7 (`02_error-codes.md`), so this topic
  cites them rather than defining them. Cross-check that `mfb spec diagnostics
  error-codes` lists both.
- `src/docs/spec/stdlib/spec.md` — reading-order bullet and see-also entry.
- `src/docs/spec/language/18_builtin-functions.md` — add `audio` **and repair
  the stale list** (the outstanding omission is `money`; `bits`/`crypto`/
  `encoding`/`vector` were already restored), with a §18.2 orientation row.

Implementation claims cite with invisible `[[path:Symbol]]` per
`.ai/specifications.md`, after grep-confirming each symbol exists.

### 3.1 What the docs must not soften

Three facts are load-bearing and easy to lose in a friendly docs voice. Each
gets a plain statement in both the man page and the spec topic:

1. **Neither resource is sendable**, so a program cannot run capture on one
   thread and playback on another. Full-duplex means opening an `AudioInput` and
   an `AudioOutput` and driving both from one loop with `poll` / `available` /
   timed `read`. This is the reason those three calls exist. It is also why
   there is no duplex resource: no OS in scope has a duplex handle to wrap.
2. **`xruns` counts events, not lost frames** — `xruns() > 0` means audio was
   lost, and the amount is unknowable.
3. **`devices()` returns no channel counts or supported rates.** A caller
   discovers a working configuration by attempting to open and handling the
   error. ALSA cannot report either without opening the device, and a field that
   is truthful on macOS and zero on Linux would be worse than no field.

## 4. Validation Design

### 4.1 Deterministic — must pass everywhere, no audio hardware

- `tests/syntax/audio/` — invalid arity, invalid argument types, `audio::` use
  without `IMPORT audio`, an attempt to construct an `AudioDevice` literal, and
  the two **swapped-direction** programs (`audio::write` on an `AudioInput`,
  `audio::read` on an `AudioOutput`). Those two live here, not in `rt-error`:
  the type split makes them compile errors. One `func_audio_<name>_invalid` per
  call.
- `tests/rt-error/audio/` — every plan-33-A §3.5 parameter violation
  (`sampleRate`, `channels`, `bufferFrames`, `frames`, `timeoutMs`,
  non-whole-frame `write` length); `read`/`write`/`poll`/`available`/`xruns`
  after `close`; double `close` (must be a no-op, not an error); and on Linux,
  `func_audio_devices_unavailable` with a poisoned loader path asserting
  `ErrAudioUnavailable`.
- `tests/rt-behavior/audio/` — `close` idempotence; `xruns` is `0` on a
  freshly-opened stream; a `timeoutMs = 0` `read` on a just-opened input stream
  returns an empty, whole-frame-aligned list without blocking.
- Native-plan tests — audio symbols present only for `IMPORT audio` programs; on
  macOS, per-symbol framework minimality; on Linux, **no `DT_NEEDED` for
  `libasound.so.2` in any target or flavor**.

### 4.2 Hardware-gated — the completion gate

The same program, run on both platforms, must produce the same observable
result. Write it once, in `planning/prompts/` or alongside the tests, and run it
on each host:

| Proof | Assertion |
| --- | --- |
| Enumerate | `devices()` is nonempty; exactly one `isDefaultOutput` when a default output exists; every record has a nonempty `id` and `name`. |
| Playback | 200 ms of a 440 Hz `s16le` tone at 48 kHz stereo plays audibly; exit 0. |
| Playback (named) | The same tone opened via an `id` from `devices()` reaches that specific device. |
| Capture | `read(in, 4800)` on 1×48 kHz returns **exactly 9600 bytes**, containing **at least one nonzero sample**. |
| Capture (timed) | `read(in, 4800, 50)` returns a whole-frame-aligned list of at most 9600 bytes. |
| Stability | `xruns()` is 0 after a 5-second continuous playback loop. |
| Cleanup | No leaked OS stream and no leaked state page (`leaks` on macOS, `valgrind` on Linux). |

The capture proof's nonzero-sample assertion is not pedantry. On macOS a
TCC-denied microphone delivers buffers of digital silence rather than an error
(plan-33-B §4.5); an all-zero capture is indistinguishable from success by byte
count alone. Silence is not proof of capture.

### 4.3 Recording a blocker

If a proof cannot run, record the target triple and flavor, which devices were
detected, whether `libasound.so.2` resolved, the exact command, and the exact
failure. Then say that platform is **unverified** — not "working", not "should
work". Per `.ai/compiler.md`, the runtime completion gate is real-device
execution; compiler output and native-plan goldens do not satisfy it.

The known standing blocker is the Alpine/musl riscv64 host, which does not ship
alsa-lib. There, `ErrAudioUnavailable` is the *expected and asserted* result
(plan-33-C Phase 1), and riscv64 playback stays unverified until alsa-lib is
installed on that box.

## Compatibility / Format Impact

None beyond plan-33-A through 33-C. This sub-plan documents and verifies a
contract already introduced.

## Phases

### Phase 1 - Man pages

- [ ] Read the three templates and both driver scripts before writing anything.
- [ ] Add `package.md`, `types.md`, and the nine function pages.
- [ ] State the three §3.1 facts explicitly.
- [ ] Run the man update/check commands the driver scripts require.

Acceptance: `./mfb man audio` and every `./mfb man audio <fn>` render with no
placeholder text, no missing required section, and no broken link.
Commit: -

### Phase 2 - Embedded spec

- [ ] Add `src/docs/spec/stdlib/11_audio.md` with grep-confirmed citations.
- [ ] Update `src/docs/spec/stdlib/spec.md` reading order and see-also.
- [ ] Update `src/docs/spec/language/18_builtin-functions.md`: add `audio` and
      the still-missing `money` (`bits`/`crypto`/`encoding`/`vector` were already
      restored).
- [ ] Add a unit test asserting the §18 package list equals
      `is_builtin_import`'s set, so it cannot drift again. (This test would have
      caught the `money` omission — the four-package drift recurred as a
      one-package drift because no such test exists yet.)
- [ ] `cargo test --bin mfb spec` and render `./mfb spec stdlib --all` to check
      for leaked `[[` citations and broken links.

Acceptance: `./mfb spec stdlib audio` documents the frame layout, the
exact-or-timeout `read` rule, and the error model; the package-list drift test
passes; `cargo test --bin mfb spec` passes.
Commit: -

### Phase 3 - Cross-target validation

- [ ] Run the audio unit tests, all syntax/rt-error/rt-behavior audio tests, and
      the native-plan import tests on every target.
- [ ] Run the §4.2 proof program on macOS with real devices.
- [ ] Run the §4.2 proof program on a glibc Linux host with alsa-lib and real
      devices.
- [ ] Attempt it on the Alpine/musl riscv64 host; record the outcome per §4.3.
- [ ] `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

Acceptance: deterministic tests and acceptance pass on every target; the §4.2
proofs pass on macOS and on glibc Linux; any unrun proof carries a concrete
blocker and its platform is reported as unverified.
Commit: -

## Validation Plan

- Tests: every public `audio::` call has valid and invalid coverage; runtime
  errors cover parameter validation, all after-close misuses, and the Linux
  missing-library path; both wrong-direction misuses are covered as *compile*
  errors in `tests/syntax/audio/`.
- Runtime proof: §4.2, on real hardware, both backends. Native-plan goldens are
  explicitly not sufficient.
- Doc sync: `mfb man audio` and `mfb spec stdlib audio` match the landed API
  exactly, including every overload.
- Acceptance: `cargo build`, `cargo test --bin mfb spec`, and
  `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- Audibility — recommended: the automated proof asserts successful OS writes and
  byte/frame invariants, and a human confirms the tone once per platform;
  alternative: an external loopback capture, which is stronger but needs CI
  device provisioning.
- Skip representation — recommended: hardware-gated proofs live outside the
  deterministic acceptance suite and are reported with exact blockers;
  alternative: encode them as skipped tests, which hides missing validation
  behind a green run.
- Package-list drift test — recommended: assert the §18 list against
  `is_builtin_import` so the two cannot diverge again; alternative: fix the list
  once and rely on review, which is what allowed the original four-package drift
  and then let it recur as the current `money` omission.

## Summary

Documentation and verification held to the same bar as the code. The three facts
in §3.1 are the ones a friendly docs pass tends to lose, and the nonzero-sample
assertion in §4.2 is the one that separates a working microphone from a
TCC-denied one. The feature is done when the same program demonstrably behaves
the same way on both backends — not when it compiles on both.
