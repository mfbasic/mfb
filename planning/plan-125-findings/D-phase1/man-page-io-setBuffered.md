### 1. Disabling after a failed drain can strand output
UNIT:      man-page:io/setBuffered
CLAIM:     “Passing `FALSE` **drains any pending bytes first** and then clears the flag, so switching buffering off never strands output.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/io/func_set_buffered.rs:lower_set_buffered` calls `STDOUT_DRAIN_SYMBOL` then unconditionally clears `ARENA_OUT_ENABLED_OFFSET`; `src/codegen/io/stdout/io_stdout.rs:lower_stdout_drain` preserves undrained bytes on failure. Probe `disable_failure/src/main.mfb` buffered `"pending"`, called `setBuffered(FALSE)`, then `io::flush()`, with stdout closed (`>&-`); it printed `exit=23`, proving the failed drain was ignored and the subsequent flush did not report it.
SUGGESTED: Passing `FALSE` attempts to drain pending output and then disables buffering. If that drain fails, pending output remains held until buffering is enabled again and drained.

### 2. A subsequent flush does not surface a failed disable drain
UNIT:      man-page:io/setBuffered
CLAIM:     “That drain is best-effort: this call returns `Nothing` and does not report a write failure, which instead surfaces from the next `io::flush` or buffered write.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/io/func_set_buffered.rs:lower_set_buffered` clears the enabled flag even when its drain fails. `src/codegen/io/stdout/io_stdout.rs:lower_stdout_drain` immediately succeeds when that flag is clear. The closed-stdout probe completed `io::flush()` and returned `23` (`exit=23`), rather than raising a write error.
SUGGESTED: That drain is best-effort: this call returns `Nothing` and does not report a write failure. Because it disables buffering even after a failed drain, enable buffering again before `io::flush` can retry the pending output.