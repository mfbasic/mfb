# bug-236: tls::listen leaks the TLS context / CoreFoundation objects on its allocation-failure and success paths

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: memory-safety (leak)

Status: Partially fixed (2026-07-15).

DONE — OpenSSL: the TlsListener record allocation ran its OOM through
`alloc_fail_fd`, which closes the fd but leaked the already-created server
`SSL_CTX`. `alloc_fail_fd` cannot simply gain an `SSL_CTX_free` — the pre-ctx
cstring allocations share it and the CTX slot is not live for them (the bug-201
class) — and routing to `ctx_fail` would misreport the OOM as ErrTlsFailed. So a
dedicated `alloc_fail_ctx_fd` exit now frees the context and falls through into the
shared fd-close + ErrOutOfMemory report. Verified: 15 tls acceptance tests pass.

REMAINING — macOS CFRelease: NOT done, and the fix as written in this bug is
unsafe. `emit_import_pem_item` extracts the cert/key via `CFArrayGetValueAtIndex`,
which returns an **unretained** reference under the CoreFoundation *Get* rule — the
array owns it. "Release the DATA/ITEMS objects after extracting the cert/key ref"
would therefore deallocate the array and leave the extracted ref dangling (a
use-after-free, strictly worse than the current bounded leak). A correct fix must
either `CFRetain` the ref before releasing ITEMS (and release it once
`sec_identity_create`/`SecIdentityCreate` has consumed it) or defer the ITEMS/DATA
release until after that consumption; it also needs `CFRelease`/`CFRetain` added to
`SERVER_SYMBOLS`. That ownership change cannot be runtime-verified here (it needs a
macOS TLS server with a PEM identity and a real client), so it is deliberately left
open rather than guessed. The leak is bounded — `tls::listen` is one-shot per
server.

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
