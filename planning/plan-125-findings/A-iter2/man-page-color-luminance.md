### 1. Green-to-blue weighting is misstated
UNIT:      man-page:color/luminance
CLAIM:     "The weights are not equal because the eye is not equally sensitive: green carries roughly seven times the perceived brightness of blue at the same channel value."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/color/func_luminance.rs:BODY` weights linear green by `7152` and blue by `722`, a ratio of 9.91. The rendered example compiled and ran with `mfb build /tmp/plan-125-scratch/A-iter2/man-page-color-luminance/probe`; it printed green `0.72` and blue `0.07`, consistent with about tenfold weighting.
SUGGESTED: The weights are not equal because the eye is not equally sensitive: green is weighted about ten times as much as blue at the same linear-light channel value.