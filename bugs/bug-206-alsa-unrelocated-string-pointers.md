# bug-206: ALSA audio.devices emits unrelocated string pointers (throwaway relocations Vec)

Last updated: 2026-07-14
Effort: medium (2h–4h)
Severity: HIGH
Class: memory-safety (platform: linux ALSA, runtime-gated)

Status: Open
Regression Test: tests/rt-behavior/ (linux audio.devices returns without crashing)

In `lower_devices`, `emit_data_address` for the `"pcm"`, `"NAME"`, and `"DESC"`
C-strings is called with a throwaway `&mut Vec::new()` for its relocations, so
the emitted `adrp`/`add_pageoff` pair is never recorded as a relocation. The
register therefore holds an unrelocated (code-page) pointer, and libasound
dereferences garbage.

The `stage` closure signature (`impl Fn(&mut Vec<CodeInstruction>)`) gives the
staged emission no way to record relocations, so they are silently dropped —
unlike every other call site, which pushes `DataAddrHi`/`DataAddrLo` into the
function's real relocations vector. This has stayed latent because the ALSA
backend is runtime-gated and not HW-verified.

## Failing Reproduction

`audio::devices()` on Linux. Observed: `snd_device_name_hint(-1, <unrelocated
iface ptr>, &hints)` and both `snd_device_name_get_hint(hint, <unrelocated attr
ptr>)` calls receive a code-page pointer that libasound dereferences → wrong
results or SIGSEGV. Expected: correct enumeration of PCM devices.

## Root Cause

`src/target/shared/code/audio/alsa.rs:1381,1451,1467` — `emit_data_address`
invoked with a discarded `&mut Vec::new()` inside the `emit_alsa_call` `stage`
closures, so the relocations never reach the function's real relocations vector.

## Non-goals

- Do not change the ALSA call ABI otherwise; only fix relocation recording.

## Blast Radius

- `lower_devices` string-address staging in `alsa.rs`. Audit any other
  `stage`-closure `emit_data_address` use for the same throwaway-Vec pattern.

## Fix Design

Thread the real `relocations` vec into the staged data-address emission — widen
the `stage` closure to also take `&mut Vec<CodeRelocation>`, or precompute the
string address before `emit_alsa_call` where `relocations` is in scope — so the
`adrp`/`add` pair is relocated.
