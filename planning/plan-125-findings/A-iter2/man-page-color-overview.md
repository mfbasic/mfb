### 1. Direct record construction does not clamp
UNIT:      man-page:color/overview
CLAIM:     “Components clamp rather than fail.”
VERDICT:   misleading
EVIDENCE:  `/tmp/plan-125-scratch/A-iter2/man-page-color-overview/project/src/main.mfb` with `color::rgba(300, -20, 128, 255)` printed `#ff0080ff`; the same program using `color::Color[300, 0, 0, 255]` built but printed `Error: 7-705-0001`. `src/codegen/builtins/color/func_rgba.rs:__color_rgba` clamps its Integer inputs; `color::Color` fields are Byte.
SUGGESTED: `color::rgb` and `color::rgba` clamp components rather than fail; direct `color::Color[...]` construction requires valid Byte channel values.

### 2. The named set is not CSS’s sixteen basic colours
UNIT:      man-page:color/overview
CLAIM:     “These are the CSS basic colours, so color::green is #008000 — a dark green.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/color/constants.rs:BASIC` exposes `orange`, `cyan`, and `magenta`, but not `lime`, `aqua`, or `fuchsia`. CSS Color 4 §6.1 identifies the original sixteen as including aqua, fuchsia, and lime—not orange.
SUGGESTED: `These sixteen constants use CSS named-colour values, so color::green is #008000 — a dark green.`