# bug-1101: a macOS `--app` run with `MFB_MACAPP_HEADLESS=1` draws on the GPU even when `MFB_HEADLESS` is set

Last updated: 2026-10-17
Effort: small (<1h)
Severity: LOW
Class: Correctness (renderer selection)

Status: Open
Regression Test: none yet

Found by bs24-agent-a while working bug-1088, reproduced by the bug-staging-24
coordinator, and placed in a later phase.

`mfb spec app canvas` (renderer selection): "a real window asks for the GPU ...; a
headless run (`MFB_HEADLESS` set) draws in software. `MFB_CANVAS_GPU` overrides both
ways". `MFB_MACAPP_HEADLESS` is not in the spec at all.

**The single correct behavior a fix produces:** with `MFB_HEADLESS` set and
`MFB_CANVAS_GPU` unset, a macOS app draws in software whatever `MFB_MACAPP_HEADLESS` is;
and the spec says what `MFB_MACAPP_HEADLESS` alone selects.

## Reproduction (main `7781cd71b`)

Any one-frame canvas program, `mfb build -q --app --debug .`, then
`MFB_CANVAS_SYNC=1 MFB_CANVAS_STATS=<file> ./build/<name>.app/Contents/MacOS/<name>`
with, in turn:

| environment | `gpuSelected` | `gpuFrames` |
|---|---|---|
| `MFB_HEADLESS=1` | FALSE | 0 |
| `MFB_MACAPP_HEADLESS=1` | TRUE | 1 |
| `MFB_MACAPP_HEADLESS=1 MFB_HEADLESS=1` | **TRUE** | 1 |

The last row contradicts the spec. Agent and coordinator runs that use
`MFB_MACAPP_HEADLESS=1` for "software" frames without `MFB_CANVAS_GPU=0` are
therefore GPU frames; the `rt_*` tests use `MFB_HEADLESS` and are not affected.
