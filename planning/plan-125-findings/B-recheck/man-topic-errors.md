Probes are done. The report format is confirmed unconditional in the runtime constants, so I have what I need. Findings follow.

### 1. Inline TRAP placement rule omits RES, which the page's own example uses
UNIT:      man-topic:errors
PAGE:      package-wide (section "Local handling with an inline TRAP")
CATEGORY:  consistency
CLAIM:     "An inline TRAP is legal only as the value of a LET/MUT binding, an assignment, or a bare expression statement"
VERDICT:   inconsistent
EVIDENCE:  The example immediately above that sentence is `RES f = fs::openFile(path) TRAP(e) ... END TRAP`. Probe `/tmp/plan-125-scratch/B-recheck/man-topic-errors/p1` (built with `mfb build -q p1`, run `p1/build/p1.out`) compiles and runs a `RES f = fs::openFile(...) TRAP(e) ... RECOVER fs::openFile(...) END TRAP` and prints `open failed: Filesystem path does not exist. code=77030001`, so RES is a legal position.
SUGGESTED: "An inline TRAP is legal only as the value of a LET, MUT or RES binding, an assignment, or a bare expression statement"

### 2. Uncaught-error report format is wrong
UNIT:      man-topic:errors
PAGE:      package-wide (section "Program entry points")
CATEGORY:  overview-mismatch
CLAIM:     "If an entry point returns an uncaught Err, the program writes Error: and the error code on one line of stderr, the message on the next, and exits with status 255."
VERDICT:   wrong
EVIDENCE:  Probe p2 (`FAIL error(errorCode::ErrNotFound, "boom here")` in main), run `p2/build/p2.out; echo exit=$?` printed four stderr lines: `Thread:   0x00000000`, `Error:    7-705-0004`, `Message:  boom here`, `Location: src/main.mfb:3:8`, then `exit=255`. The prefixes are unconditional constants (`src/codegen/error/constants/error_constants.rs:ENTRY_ERROR_MESSAGE_PREFIX`, `ENTRY_ERROR_LOCATION_PREFIX`). The code is printed in a dashed `7-705-0004` form, not as the Integer. Also "Err" names a type the page says does not exist ("there is no result type to name").
SUGGESTED: "If an entry point fails with an uncaught Error, the program writes a four-line report to stderr — the thread, the error code in its dashed form (`Error:    7-705-0004` for errorCode::ErrNotFound), the message, and the source location — and exits with status 255."

### 3. No answer to "what does Error.source hold when a built-in fails?"
UNIT:      man-topic:errors
PAGE:      package-wide (Description)
CATEGORY:  coverage
CLAIM:     "The returned Error.source records the location of the error(...) call and is not rewritten as the error propagates, so it always points at the original failure site."
VERDICT:   missing
EVIDENCE:  Most errors a developer traps come from built-ins, where no `error(...)` call exists in their program, and the page never says what `source` holds then. Probe p1 trapped `fs::openFile("/nonexistent/zzz")` and printed `src src/main.mfb:19:11`, the position of the failing call in the caller's source. The page shows the ErrorLoc fields but never shows reading them.
SUGGESTED: Add after the quoted sentence: "A built-in that fails records the location of the call in your program; `e.source.filename`, `e.source.line` and `e.source.char` name it."

### 4. errorCode is used in the body but not reachable from See also
UNIT:      man-topic:errors
PAGE:      package-wide (See also)
CATEGORY:  discoverability
CLAIM:     See also lists `mfb man general error`, `mfb man flow match`, `mfb man types`, `mfb spec diagnostics error-codes`.
VERDICT:   missing
EVIDENCE:  `grep -n errorCode src/docs/man/errors/package.md` shows the body relies on `errorCode::ErrNotFound` twice (lines 52, 115) and the `IMPORT errorCode` example, yet the only link to the code names is the contributor-facing `mfb spec` topic. `mfb man general error` already closes the loop with "and mfb man errorCode for the code names"; this topic does not. `mfb man errorCode` renders.
SUGGESTED: Add `mfb man errorCode` to See also, and in the Description sentence "conventionally with a semantic code such as errorCode::ErrNotFound" append "(see mfb man errorCode for the full list)".

What I checked beyond these: the four See-also targets all render; left-to-right argument evaluation with early abort (`eval x` then `trapped: x failed`, `eval y` never printed); bare `TRAP` without a name; `RECOVER` with no operand for an AS Nothing call; a raising operator (`1 / 0`) inside a trapped expression routing to the handler with code 77050002; `error(...)` recording its own call line. All matched the page.
