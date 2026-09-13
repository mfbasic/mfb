# tools/thread-package-sources

Source projects for the worker packages the thread test suite spawns: one directory
per worker (transfer, cancellation, file/stdin sinks, TLS listener handoff, import
and link workers, and so on). The thread specification's validation section
(`src/docs/spec/threading/12_validation.md`) describes the suite that uses them, and
individual tests load a worker's compiled `.mfp` directly (for example
`tests/net/rt_tls_listener_thread_transfer.rs` reads
`xfer_tls_listener_worker/xfer_tls_listener_worker.mfp`).

Rebuild every worker's `.mfp` from these sources with `scripts/sync-package-mfp.sh`
whenever the package binary format changes.
