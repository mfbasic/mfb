### 1. Does not state that input is not mutated
UNIT:      man-page:encoding/hexEncode
CLAIM:     "`encoding::hexEncode` returns the base-16 representation of `data`, emitting two lowercase hexadecimal characters for every input byte with no separators, prefix, or padding."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/encoding/func_hex_encode.rs:BODY` only iterates over `data` and constructs a separate `out` string. The probe at `/tmp/plan-125-scratch/C-phase2/man-page-encoding-hexEncode/src/main.mfb` printed `all=000f10ff` while encoding its input; no operation mutates `data`.
SUGGESTED: "`encoding::hexEncode` returns a new base-16 string for `data` and does not mutate `data`, emitting two lowercase hexadecimal characters for every input byte with no separators or prefix."

### 2. Parameter description omits empty-input behavior
UNIT:      man-page:encoding/hexEncode
CLAIM:     "The bytes to encode."
VERDICT:   incomplete
EVIDENCE:  The rendered parameter row gives no empty-input behavior. The probe `/tmp/plan-125-scratch/C-phase2/man-page-encoding-hexEncode/src/main.mfb`, built and run with the specified release binary, printed `empty=` for `encoding::hexEncode([])`. `src/codegen/builtins/encoding/func_hex_encode.rs:BODY` initializes `out` to `""` and returns it when the loop has no items.
SUGGESTED: "The bytes to encode. An empty list returns the empty string."