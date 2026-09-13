### 1. IO pages use nonexistent error names

UNIT:      man-pkg:io  
PAGE:      package-wide  
CATEGORY:  consistency  
CLAIM:     “the following call raises `ErrEof`” and “a zero-byte or failing write raises `ErrOutput`”  
VERDICT:   inconsistent  
EVIDENCE:  `src/docs/spec/diagnostics/02_error-codes.md` lists `ErrEndOfFile` (`7-702-0003`) and `ErrWriteFailed` (`7-702-0002`), not `ErrEof` or `ErrOutput`. `src/codegen/builtins/io/gen_read_line_family.rs` raises `ErrEndOfFile`; `gen_write_family.rs` raises `ErrWriteFailed`. Probe: `/tmp/plan-125-scratch/B-phase3/man-pkg-io/eof-probe/build/eof_probe.out < /dev/null` printed `Error: 7-702-0003` and “Read operation reached end of file where a value was required.”  
SUGGESTED: Replace every `ErrEof` with `ErrEndOfFile` and every `ErrOutput` with `ErrWriteFailed` across the IO pages (including `input`, `readLine`, `pollInput`, `print`, `write`, error-output functions, and `flush`).