# bug-207: ALSA hw-params getters place `params` in the wrong arg register → openOutput/openInput fail

Last updated: 2026-07-14
Effort: small (<1h)
Severity: MEDIUM
Class: correctness (platform: linux ALSA, runtime-gated)

Status: Fixed (2026-07-15) — the snd_pcm_hw_params_get_rate / get_channels getters now load `params` into ARG[0] directly (mirroring hw_params_free) instead of calling the ARG[1]-targeting `params` closure and then clobbering ARG[1], so `params` reaches the getter's first argument and the readback rate/channels are correct.
Regression Test: verified on HW — `audio::openOutput(44100, 2, 1024)` opens successfully on Ubuntu x86_64 (VM 2228); previously the getter read a garbage rate (leftover dlsym fn-ptr in ARG[0]) and the rate==requested verification failed with ErrAudioDevice.
Regression Test: tests/rt-behavior/ (linux audio.openOutput succeeds)

In `emit_configure_hw_params`, the `snd_pcm_hw_params_get_rate` /
`snd_pcm_hw_params_get_channels` getter calls stage `params` via the `params`
closure (which targets `ARG[1]`) and then immediately overwrite `ARG[1]` with
`&rate`/`&chans`, so the `params` pointer never reaches `ARG[0]`. The function is
invoked with the leftover `dlsym` fn-ptr still in `x0`. Unlike the setters, which
correctly place `pcm` in `ARG[0]` and `params` in `ARG[1]`, these getters take
`params` as their *first* argument.

## Failing Reproduction

`audio::openOutput`/`openInput` on Linux. Observed:
`snd_pcm_hw_params_get_rate(<garbage x0>, &rate, &dir)` reads an unintended
object and writes a garbage rate to `RATE_OFF`; the unconditional
`rate == requested` verification then mismatches and open fails with
`ErrAudioDevice`. Expected: the real rate/channel count is read back and open
succeeds.

## Root Cause

`src/target/shared/code/audio/alsa.rs:653-661` — the getters call the
`ARG[1]`-targeting `params` closure, then clobber `ARG[1]`, leaving `ARG[0]`
holding the `dlsym` fn-ptr instead of `params`.

## Non-goals

- Do not change the setter calls (already correct).

## Blast Radius

- The two hw-params getter calls in `emit_configure_hw_params`.

## Fix Design

For the getters, load `params` into `ARG[0]` (return_register) instead of calling
the `ARG[1]`-targeting `params` closure, then put `&rate`/`&dir` in
`ARG[1]`/`ARG[2]`.
