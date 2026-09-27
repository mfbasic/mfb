### 1. Unterminated CR is discarded
UNIT:      man-page:io/readLine
CLAIM:     "Input that ends mid-line is not lost: those bytes come back as the final, unterminated line, and the *next* call raises `ErrEndOfFile`."
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase1/man-page-io-readLine/eof/src/main.mfb`, built with the specified binary and fed `tail\r`, printed `first=[tail]` then `second-error=77020003`; the terminal CR did not come back. `src/codegen/builtins/io/gen_read_line_family.rs:trim_cr` strips a trailing CR after either LF or EOF.
SUGGESTED: Input ending after other bytes returns a final unterminated line, then the next call raises `ErrEndOfFile`. A trailing CR is removed even when input ends without an LF.

### 2. Raisable errors are absent from the rendered Errors table
UNIT:      man-page:io/readLine
CLAIM:     "Bytes are decoded as UTF-8 as they arrive, with the full validity check; an ill-formed sequence fails rather than yielding a replacement character."
VERDICT:   incomplete
EVIDENCE:  The rendered `mfb man io readLine` page has no Errors section because `src/codegen/builtins/io/func_read_line.rs:register` declares `errors: vec![]`. Yet `lower_read_line_family` raises `ErrWriteFailed`, `ErrEndOfFile`, `ErrInputFailed`, `ErrEncoding`, `ErrOutOfMemory`, and `ErrInvalidContext` at `src/codegen/builtins/io/gen_read_line_family.rs:423-453`. The invalid-UTF-8 probe fed `\xFF\n` printed `error=77020004`, confirming `ErrEncoding`.
SUGGESTED: Declare and render all six errors with their conditions; in prose, say “Malformed UTF-8 raises `ErrEncoding`.”

### 3. Broadcast-log implementation detail leaks into developer documentation
UNIT:      man-page:io/readLine
CLAIM:     "Standard input is a per-thread broadcast log;"
VERDICT:   out-of-scope
EVIDENCE:  This describes the implementation mechanism rather than what a terminal user needs to do. The user-visible requirement is independently stated in the following sentence and enforced by `src/codegen/builtins/io/gen_read_line_family.rs:453`, which raises `ErrInvalidContext` for an unsubscribed reader.
SUGGESTED: In a worker thread, call `thread::openStdIn` before reading standard input; otherwise `io::readLine` raises `ErrInvalidContext`.