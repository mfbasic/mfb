### 1. Negative input error is omitted

UNIT:      man-page:encoding/uleb128Encode  
CLAIM:     "The non-negative integer to encode."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/encoding/func_uleb128_encode.rs:__encoding_uleb128Encode` executes `FAIL error(77050003, "negative value")` when `value < 0`, while its descriptor’s `errors: vec![]` causes the rendered page to have no Errors table. Probe compiled with the specified release binary:

```basic
IMPORT encoding

SUB main()
  LET bytes AS List OF Byte = encoding::uleb128Encode(-1)
END SUB
```

Output:

```text
Error: 7-705-0003
negative value
exit=255
```

Both published examples compiled and ran, printing `624485` and `1`, `1`, `2`, respectively.  
SUGGESTED: `The integer to encode. It must be zero or positive; a negative value raises ErrInvalidFormat.` Also declare `ErrInvalidFormat` in the descriptor so the derived Errors table is rendered.