# Error Propagation

Every MFBASIC function call returns a `Result` at the IR level. A source call
that is not directly matched is the ordinary `Result` auto-unwrap: a `CallResult`
whose `Ok` value flows on and whose `Error` propagates to the enclosing `TRAP`
(or the function's error result). This is structured IR — a `MATCH`/`PROPAGATE`
over the call — not an opcode pair.

The thread trampoline stores the worker's returned result tag and value/error in
the control block. `thread::waitFor` reads that stored result, materializes any
heap-backed payload into the caller's arena before user code observes it, and
closes the parent `Thread` handle before behaving like a normal fallible call:

- `Ok(value)` returns `value`.
- `Error(error)` propagates as the caller's error.

`thread::waitFor` is the only retrieval path; there is no `t.result` field. After
it retrieves the outcome, later use of the same `Thread` handle fails with
`ErrResourceClosed`.

Because worker and package functions ride the same `IR -> NIR -> native` path as
the executable's own code, this behavior is automatic for calls made inside a
worker. If a worker calls an imported package function and that function returns
`Err`, the worker returns or propagates the error according to the normal
structured `Result`/`TRAP` semantics — there is no separate bridge to keep in
sync.
