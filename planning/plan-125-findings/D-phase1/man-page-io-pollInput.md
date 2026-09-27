### 1. Missing raised errors
UNIT:      man-page:io/pollInput
CLAIM:     "A negative timeoutMs is rejected with ErrInvalidArgument."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/io/func_poll_input.rs:register` declares `errors: vec![]`, so the rendered page has no Errors table. Its `lower_poll_input` raises both `ErrInvalidArgument` for a negative non-omitted timeout and `ErrInputFailed` when the OS readiness query fails. Probe `printf '' | .../probe_project.out` printed `zero=TRUE` and `negative=77050002`.
SUGGESTED: Add Errors rows: “ErrInvalidArgument — `timeoutMs` is negative.” and “ErrInputFailed — standard input could not be queried for readiness.”

### 2. Timeout is not an absolute upper bound after a signal
UNIT:      man-page:io/pollInput
CLAIM:     "A positive value waits up to that long."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/io/func_poll_input.rs:lower_poll_input` stores the normalized timeout at `TIMEOUT_OFFSET`; its EINTR path retries at `os_poll`, which reloads that original timeout for each retry. A handled signal therefore restarts the full requested wait rather than preserving the remaining time.
SUGGESTED: “A positive value waits up to that long when uninterrupted. A handled signal restarts the wait, so signals can extend the total elapsed time.”

### 3. First example fails at end of input
UNIT:      man-page:io/pollInput
CLAIM:     "Read a line only when one is already pending (pass 0 for the immediate check — omitting the timeout would instead block until input is ready):"
VERDICT:   example-broken
EVIDENCE:  The example compiled with the prescribed release binary. Running `printf '' | .../example_zero_project/build/example_zero_project.out` printed `Error: 7-702-0003` and `Read operation reached end of file where a value was required.` EOF makes `pollInput(0)` return `TRUE`, but no line is pending.
SUGGESTED: “Read a line when input is ready, handling end of input as well:” followed by an example that traps `ErrEndOfFile`.

### 4. Second example does not time out at end of input
UNIT:      man-page:io/pollInput
CLAIM:     "Wait up to a second for a keypress:"
VERDICT:   example-broken
EVIDENCE:  The example compiled with the prescribed release binary. Running `printf '' | .../example_second_project/build/example_second_project.out` printed `Error: 7-702-0003` and `Read operation reached end of file where a value was required.`, rather than `timeout`; EOF is immediately ready.
SUGGESTED: “Wait up to a second for input, handling end of input:” followed by an example that traps `ErrEndOfFile`.