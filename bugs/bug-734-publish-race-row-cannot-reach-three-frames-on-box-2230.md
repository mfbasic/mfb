# bug-734: the publish-race row cannot render the 3 frames it needs on box 2230, so its gate there cannot pass

Last updated: 2026-09-25
Effort: medium (1h–2h)
Severity: LOW
Class: Other (test gate that cannot pass on its designated box)

Status: Open
Regression Test: `scripts/test-canvas-gpu-rows.sh --target windows-x86_64 --rows race`; `tests/canvas/rt_canvas_geo_native.rs::a_frame_never_draws_one_scenes_geometry_for_anothers_items`

The `race` row of `scripts/test-canvas-gpu-rows.sh` runs
`tests/canvas/scenes/canvas_geo_native_publish_race.mfb`. It fails on box 2230
(Win11 x86_64, emulated) every time with `FAIL: race/sw: the race did not happen (2
frames, geoResolvedChecked=12000)`. `geoVerifyMismatches` is 0. It fails the same way
at `main` HEAD (`4df37a4fe`), so it is **not** caused by bug-687, which found it.

**The single correct behavior a fix produces:** on every box the script targets, the
race row renders at least 3 frames that overlap the worker's publishes. It then passes
or fails on `geoVerifyMismatches` alone, with the "≥ 3 frames" requirement unchanged.

References:

- bug-686 (completed): `a_frame_never_draws_one_scenes_geometry_for_anothers_items`
  and the scene. The first two frames only fill the cache for scenes A and B; a
  cache-HIT race needs a third frame that overlaps a publish. That makes "≥ 3 frames" a
  real requirement, not a proxy that can be lowered.
- bug-688: `df4f589a7` added the fall-back to a software race when the GPU run renders
  fewer than 3 frames. It measured 3 software frames on 2230 then, on 2026-09-24, before
  the box's 14:53 reboot.
- Found by bug-687 (`bugs/bug-687-metal-coverage-flips-past-gpu-tolerance-near-black.md`,
  Phase 2 results).

## Failing Reproduction

```
cargo build --release
scripts/test-canvas-gpu-rows.sh target/release/mfb --target windows-x86_64 --rows race
```

- Observed: `the GPU run rendered only 2 frames; racing on the software renderer`, then
  `FAIL: race/sw: the race did not happen (2 frames, geoResolvedChecked=12000)`.
- Expected: `ok: race/<tag>: N frames rendered during 60 publishes` with N ≥ 3.

| build | box | runs | frames |
| --- | --- | --- | --- |
| `main` `4df37a4fe` (detached worktree `/tmp/b687-main`) | 2230, alone | 3 | GPU 2, software 2 — FAIL ×3 |
| bug-687 before its shader work, `8158a9524` | 2230 | 1 | GPU 2, software 2 — FAIL |
| bug-687 final | 2230 | 2 | GPU 2, software 2 — FAIL ×2 |
| bug-688 (`df4f589a7`, 2026-09-24, before the reboot) | 2230 | 1 | software 3 — pass |
| bug-687 V5 builds | 2226 (aarch64, native) | 2 | GPU 3 and 10 — pass |

Timing probe: the same scene plus a `datetime::monotonicNanos` stamp around the 60
presents (`/tmp/b687-race/race2`), in software with `MFB_CANVAS_GEO_VERIFY=1`:

| host | 60 publishes span | frames | render + offsets time |
| --- | --- | --- | --- |
| 2230 (3 runs) | 19.1 s, 15.0 s, 16.1 s | 2, 2, 2 | ~13-18 s per frame |
| macOS local (2 runs) | 1.8 s, 1.8 s | 6, 9 | ~0.2-0.3 s per frame |

Without `MFB_CANVAS_GEO_VERIFY`, 2230 still renders 2 frames (3 runs).

## Root Cause

This is timing, not a defect in the scene, the runtime or the Windows software path.

- The worker publishes 60 scenes with `os::sleep(20)` between them
  (`canvas_geo_native_publish_race.mfb`, `main`). Its publish window is fixed by the
  worker's own speed.
- Frames are rendered by the graphics thread, one at a time, each of the latest
  published scene. So the number of frames that can overlap the window is roughly the
  window divided by one frame's time, plus the final frame.
- On 2230 a 6,000-item frame (the MFBASIC rasteriser plus the offsets phase, x86-64
  under emulation) takes about as long as the whole window: 13-18 s against 15-19 s. So
  one frame overlaps the publishes and one more renders the last scene: 2.
- bug-688's 3 was a margin case. The box has since rebooted, and
  `TextInputHost.exe` (PID 6388) has burned about 0.8 of a core continuously since
  boot: measured over 10 s, and 66,935 CPU-seconds since 2026-09-24 14:55. That is 20%
  of the box's 4 logical CPUs. It is not ours, so it was left running.

The other rows are unaffected because they use `MFB_CANVAS_SYNC=1`: every present waits
for its frame, so they do not depend on overlap.

## Goal

- On 2230, 2226 and macOS, the race row and the Rust test render ≥ 3 frames that
  overlap the publishes, with `geoResolvedChecked` ≥ 6,000 and `geoVerifyMismatches=0`.

### Non-goals (must NOT change)

- The "≥ 3 frames" requirement in the script and in the Rust test. Lowering it to 2 is
  the tempting wrong fix: two frames only populate the caches, so the cache-hit race the
  row exists for is never exercised.
- `geoVerifyMismatches` as the verdict.
- The shared scene's item set (A/B kinds per index), which is what makes a
  wrong-scene hit visible.

## Blast Radius

Found with `grep -rn "publish_race" tests scripts`:

- `scripts/test-canvas-gpu-rows.sh` `race` row: fails on 2230. Fixed by this bug.
- `tests/canvas/rt_canvas_geo_native.rs::a_frame_never_draws_one_scenes_geometry_for_anothers_items`:
  runs on macOS, 6-9 frames. Unaffected today; the same fix keeps it robust on a slow CI
  runner.
- Other `test-canvas-gpu-rows.sh` rows run with `MFB_CANVAS_SYNC=1`. Unaffected.

## Fix Design (to decide)

Make the publish window depend on rendered frames rather than on wall-clock time. The
worker keeps alternating A and B until at least 3 frames have been rendered after both
scenes are cached. It stops at 60 publishes or later, and never at fewer than 60.

This needs something the program can read that counts rendered frames, and the scene
changes, so it requires AGENTS.md's four-question proof for the shared scene. The
alternative is a harness-only change: the script runs the software race with a longer
window, for example through an environment variable the scene reads. That still changes
the scene. Either way the assertions stay as they are.

## Phases

### Phase 1: failing reproduction

- [x] Reproduced at `main` HEAD on 2230 (table above).

### Phase 2: the fix

- [ ] Choose between a frame-count-driven publish loop and a harness knob; prove the
      scene change with the four questions; the race row passes on 2230, 2226 and macOS
      with the 3-frame requirement untouched.

Commit: —
