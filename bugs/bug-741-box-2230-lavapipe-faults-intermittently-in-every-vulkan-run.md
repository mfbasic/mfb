# bug-741: box 2230's lavapipe faults in ~1 of 7 Vulkan runs, whatever the scene

Last updated: 2026-09-26
Effort: small (environment), owner decision needed
Severity: LOW (test reliability; no MFB defect found)
Class: Test infrastructure (Vulkan rows on Windows)

Status: Open — found by bug-735 (lane E), filed and reported to the coordinator; not in
bug-735's scope (the fix is the box's driver or the test instrument, not MFB code).

**The single correct behavior a fix produces:** a `scripts/test-canvas-gpu-rows.sh
--target windows-x86_64` row passes or fails on its pixels, never on `gpu
rc=-1073741819` from a driver fault.

## Failing Reproduction

`mfb` at `0cd2db799` (before bug-735). A one-rectangle scene (`canvas::present([
canvas::Rectangle[x := 100.0, y := 100.0, w := 10.0, h := 10.0, paint :=
canvas::fill(color::rgb(200, 0, 0))]])`) built `mfb build --app --debug --target
windows-x86_64`, run 40 times on box 2230 with `MFB_WINAPP_HEADLESS=1 MFB_CANVAS_SYNC=1
MFB_CANVAS_GPU=1` from a `.bat` loop:

- observed: 6 of 40 runs exit `-1073741819` (`0xC0000005`); `tests/canvas/scenes/
  small_polygons_on_black.mfb` 1 of 20. In full row runs: `polylinesblack`, `groupsblack`,
  `rectstepblack`, `hairline`, `smallpolysblack`, `matrix`, `xformblack` each crashed once
  on 2026-09-26 and passed on re-run.
- expected: 0 of 40.
- 2226 (lavapipe, aarch64 Linux) never showed it in the same day's runs (two full row
  sets, ~60 runs).

## Root Cause (evidence; the driver has no symbols)

Seven minidumps (Windows Error Reporting LocalDumps, parsed by hand as
`.ai/remote_systems.md` describes) all fault at `vulkan_lvp.dll+0x1feb331`, a READ at a
page boundary (`0x…4000`, `0x…d0000`, `0x…c9a4000`, …), on a lavapipe worker thread whose
stack holds only `vulkan_lvp.dll` frames (offsets in its LLVM range). The MFB graphics
thread is at the same moment inside one Vulkan call. No MFB frame and no JIT'd shader
code is on the faulting thread. A fixed site, a scene-independent rate and a
page-boundary address are a driver over-read past an allocation that sometimes ends at a
page end. The driver is Mesa 26.2 lavapipe in `C:\mfbvk\mesa\x64` (dated 2026-08-11),
under the box's x86-64 emulation.

Hypotheses not yet eliminated: an MFB buffer handed to the driver too short by a few
bytes (e.g. a mapped region's size) that only faults when it ends a page. Against it:
the one-rectangle scene's buffers are the same fixed sizes as every scene's, and 2226's
lavapipe never faults. Confirm by running under a Mesa built with symbols, or a newer
Mesa release.

## Non-goals

- Do not add a retry to `scripts/test-canvas-gpu-rows.sh` or loosen any row without an
  owner decision: it would also hide a real MFB crash on that path.

## Fix options (owner)

1. Update 2230's Mesa (or install a build with PDBs) and re-measure the 40-run loop.
2. Let the harness retry a run that exits `0xC0000005` inside `vulkan_lvp.dll` once,
   reporting it — a change to the instrument.

Commit: —
