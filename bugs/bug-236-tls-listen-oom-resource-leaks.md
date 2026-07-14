# bug-236: tls::listen leaks the TLS context / CoreFoundation objects on its allocation-failure and success paths

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: memory-safety (leak)

Status: Open

`tls::listen` misses the cleanup its `connect`/`accept` siblings already perform:

- OpenSSL (`src/target/shared/code/tls/openssl.rs:1191`, `alloc_fail_fd`
  `:1289-1310`): on a record-allocation OOM, the already-created server `SSL_CTX`
  is leaked — `alloc_fail_fd` closes the fd but never `SSL_CTX_free`s the context.
  connect (`:751-823`) and accept (`:1576-1605`) got full SSL/CTX/fd cleanup on
  the same OOM class (bug-55); listen was missed. Fix: `SSL_CTX_free(ctx)` before
  closing the fd (route the record alloc to `ctx_fail`).
- macOS (`src/target/shared/code/tls/macos.rs:1914-2024`, `emit_import_pem_item`):
  the transient CoreFoundation import objects — the `CFDataCreate` result (+1) and
  the `SecItemImport` out-array (+1), for both cert and key — are never released,
  even on the success path (`CFRelease` is not among the resolved symbols). The
  `alloc_fail`/`net_fail`/`load_fail` exits additionally leak the
  listener/queue/params/endpoint/identity built so far. Bounded (listen is
  one-shot per server). Fix: resolve `CFRelease`, release the DATA/ITEMS objects
  after extracting the cert/key ref, and add best-effort cleanup to the error
  exits.
