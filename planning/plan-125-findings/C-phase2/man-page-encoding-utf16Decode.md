### 1. Missing raised-error contract
UNIT:      man-page:encoding/utf16Decode
CLAIM:     "Every element must lie in `0..65535`; a value outside that range is rejected."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_utf16_decode.rs:BODY` raises `error(77050003, ...)` for out-of-range units and all unpaired-surrogate forms, but its `RegistryFunction` declares `errors: vec![]`, so the rendered page has no Errors table. Probe output: `high-last=77050003`, `high-not-low=77050003`, `low-alone=77050003`, `negative=77050003`, `over-max=77050003`.
SUGGESTED: Add an `ErrInvalidFormat` Errors row: “An element is outside `0..65535`, or the sequence contains an unpaired surrogate.”