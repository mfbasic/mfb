# Error Propagation

Every MFBASIC function call returns a `Result` at the IR level, auto-unwrapped via
the structured `MATCH`/`PROPAGATE` desugaring owned by
`./mfb spec language error-model`. This topic covers only how a worker's result —
and its source location — survive the thread boundary.

The thread trampoline stores the worker's returned result tag and value/error in
the control block, and also captures the error's `ErrorLoc` origin pointer into
the dedicated `result error source` field (control-block offset 96). This
preserves the worker's terminal-error source location (file, line, char) across
the thread boundary. [[src/target/shared/code/mod.rs:THREAD_OFFSET_RESULT_SOURCE]] `thread::waitFor` reads the stored result, materializes any
heap-backed payload into the caller's arena before user code observes it, and
closes the parent `Thread` handle before behaving like a normal fallible call:

- `Ok(value)` returns `value`.
- `Error(error)` propagates as the caller's error, with its source location
  recovered from the captured origin pointer.

`thread::waitFor` is the only retrieval path; there is no `t.result` field. After
it retrieves the outcome, later use of the same `Thread` handle fails with
`ErrResourceClosed`.

Because worker and package functions ride the same `IR -> NIR -> native` path as
the executable's own code, this behavior is automatic for calls made inside a
worker. If a worker calls an imported package function and that function returns
`Err`, the worker returns or propagates the error according to the normal
structured `Result`/`TRAP` semantics — there is no separate bridge to keep in
sync.

## See Also

* ./mfb spec language error-model — the `Result` auto-unwrap and `TRAP` semantics
* ./mfb spec threading control-block — the `result error source` field at offset 96
