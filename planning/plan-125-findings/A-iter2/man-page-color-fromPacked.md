### 1. Parameter boundary behavior omitted
UNIT:      man-page:color/fromPacked
CLAIM:     “The packed colour, `0xAARRGGBB`. Only the low 32 bits are read.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_from_packed.rs:BODY` masks each extracted channel to the low 32 bits. Probe compiled and ran with the specified release binary: `zero: 0 0 0 0`; `negative: 255 255 255 255`; `highbits: 0 0 0 0`.
SUGGESTED: The packed colour, `0xAARRGGBB`. Any `Integer` is accepted and only its low 32 bits are read: zero is fully transparent black, and negative values are interpreted by those low bits (`-1` is opaque white).