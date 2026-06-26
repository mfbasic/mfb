# Validation

Thread support is not validated by compiler output alone. A complete
implementation must include runtime tests that execute generated native
programs.

Required coverage includes:

- Starting a thread and printing from the worker.
- Sending parent-to-worker messages.
- Emitting worker-to-parent messages.
- Running two threads and cancelling them cooperatively.
- Using standard packages such as `strings`, `fs`, and `io` inside a worker.
- Passing data from main to a worker and returning structured data.
- Calling an imported package from inside a worker.
- Verifying the same runtime behavior on supported OS targets.

Acceptance tests should include `.run` goldens when behavior is observable
through stdout/stderr or exit code.
