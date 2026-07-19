# bug-319: ALSA audio open error paths leak the mmap'd state page (and hw-params object) on unavailable/misconfigured devices

Last updated: 2026-07-19
Effort: small (<1h)
Severity: MEDIUM
Class: Memory-safety (resource leak)

Status: Fixed (2026-07-19, aa5adc2d1) — A1 and A2 both closed by one shared
`emit_open_cleanup` used by the `unavailable` and `dev_fail` exits, releasing
hw-params, PCM handle and mmap page, each guarded on its own slot. A dlsym miss
inside the cleanup skips only that disposal and continues, so it can never
branch back to an error exit and loop.

Found and fixed a hazard this document did not anticipate: `lower_open`
continues to prepare/start after the success-path `hw_params_free`, and those
can still reach `dev_fail` — so adding the cleanup without also zeroing
`PARAMS_OFF` would have introduced a double free.

The ALSA `lower_open` mmaps a 16 KiB `STATE_PAGE` and stores it *before* calling
`emit_dlopen`/`emit_dlsym`. Every dlopen/dlsym failure branches to the `unavailable`
label, which calls `emit_fail` with no `munmap` and no `snd_pcm_close` — so on any
host without `libasound.so.2` (the documented riscv64/musl / Alpine boxes that
"degrade cleanly to ErrAudioUnavailable") each `openOutput`/`openInput` leaks 16 KiB.
Separately, `emit_configure_hw_params` allocates a `snd_pcm_hw_params_t` freed only on
the success path, so a device that can't honor the exact requested rate/channel count
leaks one small heap block per failed open. The sibling `dev_fail` path has the
proper bug-180 close+munmap cleanup; the `unavailable` path and the hw-params free
were never given it.

The single correct behavior a fix produces: every ALSA open error exit releases the
resources it allocated (the mmap'd state page, any open PCM handle, the hw-params
object) — so repeated failed opens do not leak.

References:

- `bugs/completed-bugs/bug-180-*` (scoped open-error cleanup to `dev_fail` only),
  bug-207 (hw-params getter args + close/munmap).
- Found during goal-06 review of `src/target/shared/code/audio/alsa.rs`.

## Items

### A1 — `unavailable` failure path leaks the mmap'd state page (and possibly an open PCM handle), MEDIUM
- `src/target/shared/code/audio/alsa.rs:619` (`lower_open`, `unavailable` label);
  root cause is call order — `mmap` (~line 500, stored to `STATE_OFF`) runs before
  `emit_dlopen` (520) and all `emit_dlsym`s.
- `unavailable` calls `emit_fail` with no `munmap`/`snd_pcm_close`; `dev_fail`
  (628-672) has the correct bug-180 cleanup. Any host lacking libasound leaks 16 KiB
  per open; a partial/wrong-ABI libasound where `snd_pcm_open` resolves but a later
  dlsym is missing additionally leaks the open PCM handle.
- Fix: move `emit_dlopen`/symbol resolution before the mmap, or give `unavailable`
  the same guarded close+munmap block `dev_fail` uses (`STATE_OFF` is zeroed at entry,
  so the null-guard is safe).

### A2 — `emit_configure_hw_params` leaks the `snd_pcm_hw_params_t` on any hw-config/verify failure, LOW
- `src/target/shared/code/audio/alsa.rs:726` (malloc) vs `:986` (free); error exits
  via `check(instructions, dev_fail)` (e.g. `:979`/`:983`).
- `snd_pcm_hw_params_malloc` is freed only on the success path; every intermediate
  `check(...)` to `dev_fail` (including the rate/channel verify mismatch) skips the
  free, and `dev_fail` closes the PCM + munmaps but never frees `params`.
- Fix: free `params` before branching to `dev_fail` (a small `hw_fail` trampoline),
  or free it in `dev_fail` guarded by non-null `PARAMS_OFF`.

## Goal

- ALSA open error exits release the mmap page, PCM handle, and hw-params object; no
  leak on unavailable or misconfigured devices.

### Non-goals (must NOT change)

- The success path and `dev_fail`'s existing cleanup (correct).
- macOS AudioQueue backend (reviewed clean).

## Blast Radius

- `lower_open` `unavailable` path and `emit_configure_hw_params` — cited sites (Linux
  ALSA only; runtime-gated).

## Fix Design

Either reorder dlopen-before-mmap (so no page exists when dlopen fails) or extend the
error labels with the guarded cleanup `dev_fail` already uses. Reordering is cleaner
for A1; a guarded free is minimal for A2. Rejected: leaving the leaks — they grow per
call on affected hosts.

## Phases

### Phase 1 — failing test
- [ ] A test on a no-libasound host (or a stubbed dlopen) asserting bounded RSS across
      repeated `audio::openOutput`; a device-config-mismatch test for A2.
### Phase 2 — the fixes
- [ ] Add the releases / reorder dlopen.
### Phase 3 — validation
- [ ] Full suite green; success path unchanged; no double-free/double-munmap.

## Validation Plan

- Regression: repeated-failed-open RSS test (A1); config-mismatch leak test (A2).
- Runtime proof: no growth on the riscv64/musl/Alpine no-audio boxes.
- Doc sync: none.

## Summary

Two ALSA open error-path leaks: the `unavailable` exit leaks the 16 KiB state page
(MEDIUM, hits every no-audio host), and hw-config failures leak the hw-params object
(LOW). Both are the cleanup `dev_fail` already does, just missing on these exits.
