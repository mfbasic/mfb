### 1. Tie-breaking rule for "nearest" is unstated
UNIT:      man-page:color/fromLinear
CLAIM:     "it returns the sRGB channel byte whose linear value is nearest to `value`."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_from_linear.rs:BODY` — the search takes the higher channel only when `value > (midLow + midHigh) / 2`, so an exact midpoint goes to the lower (darker) channel. Probe `/tmp/plan-125-scratch/A-recheck-iter2/man-page-color-fromLinear/probe.mfb` (built with `mfb build -q .`, run `./build/mfb_project.out`): `fromLinear(10)` → `0`, `fromLinear(11)` → `1` (channel 0 is linear 0, channel 1 is linear 20, so 10 is equidistant). Over all 65536 inputs the result is always one of the nearest channels (`not-nearest count=0`), so "nearest" is true, but when two channels are equally near the page does not say which wins.
SUGGESTED: "it returns the sRGB channel byte whose linear value is nearest to `value`; when two channels are equally near, the darker one is returned."

### 2. Second example averages channels, not colours
UNIT:      man-page:color/fromLinear
CLAIM:     "Average two colours correctly — in linear light, not in encoded bytes:"
VERDICT:   misleading
EVIDENCE:  The example body declares `a AS Byte` and `b AS Byte` and averages those two bytes; no `Color` value appears. Compiled and run from `/tmp/plan-125-scratch/A-recheck-iter2/man-page-color-fromLinear/ex2/` it prints `linear midpoint 188` / `byte midpoint 127`, which is the per-channel result. Averaging whole colours is what `color::mix` does.
SUGGESTED: "Average two channel values correctly — in linear light, not in encoded bytes (`color::mix` does this for whole colours):"

Everything else on the page checked out against the binary. The first example (ex1) prints `0`, `255`, `200` as the page implies. The probe confirmed clamping on both sides (`-1` and the most negative Integer → `0`; `65536` and `999999` → `255`), a zero-failure `fromLinear(toLinear(c)) = c` sweep over all 256 channels, and that `luminance`, `brighten`, `darken`, `mix` and `grayscale` all call `color::toLinear` or `__color_fromLinear` (`grep -ln` over `src/codegen/builtins/color/func_*.rs`). No banned vocabulary, no compiler internals, and the Errors table is correctly absent (`errors: vec![]`).
