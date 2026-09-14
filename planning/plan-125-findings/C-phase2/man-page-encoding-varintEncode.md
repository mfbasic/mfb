### 1. Parameter omits its accepted range and zero/negative behavior
UNIT:      man-page:encoding/varintEncode
CLAIM:     "The integer to encode."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_varint_encode.rs:__encoding_varintEncode` has no rejection path and its `Integer` parameter accepts the full signed range. The probe at `/tmp/plan-125-scratch/C-phase2/man-page-encoding-varintEncode/src/main.mfb` printed `00`, `01`, `feffffffffffffffff01`, and `ffffffffffffffffff01` for 0, -1, Integer maximum, and Integer minimum respectively; it also round-tripped both extrema.
SUGGESTED: The signed integer to encode. Every `Integer`, including `0` and negative values, is accepted.