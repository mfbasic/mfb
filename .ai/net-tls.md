# Networking, TLS & repository-client invariants

Invariants for `tls::connect` timeout semantics, TLS read readiness, and the repository client's transport-security enforcement.

## tls::connect timeoutMs=0 is one immediate attempt

`tls::connect(host, port, timeoutMs, serverName)` follows the language timeout convention: **`timeoutMs = 0` is one immediate, non-blocking attempt** — it succeeds only if connect+handshake complete synchronously, else raises `ErrTimeout` (`7-705-0008`, "Operation did not complete before its deadline") **instantly** (macOS uses `DISPATCH_TIME_NOW`). The *omitted* 3-arg form is the unbounded one, NOT `0`. A negative value raises `ErrInvalidArgument`.

So a real TLS 1.3 handshake to a remote host can essentially never pass with `timeoutMs=0` — it fails in ~4ms with an error that *looks* like a slow-network deadline but isn't (the network can be perfectly healthy; `openssl s_client` to the same host succeeds). Diagnostic tell: a "deadline" error that returns in single-digit ms is a `0`-timeout immediate attempt, not a network wait.

A live-network fixture must therefore pass a positive `timeoutMs`. Example: `tests/rt-behavior/tls/tls-connect-google-rt` had `tls::connect("8.8.8.8", 443, 0, "dns.google")` while its golden expected `connected=TRUE` — internally contradictory. The fix is the fixture: use a positive `timeoutMs` (e.g. 5000) so the live connect reliably completes when the network is up. Such a failure is easy to mislabel as a "network flake" when it is actually a `timeoutMs=0` fixture bug. The acceptance golden harness's `test-gate.sh` skip hook covers the genuinely-offline case.

## TLS readiness must check the decrypted-buffer, not just the fd

A TLS socket's "is a read ready?" is **not** an fd `poll(2)`. One TLS record decrypts to many application bytes; a single `SSL_read`/Network.framework receive drains a record and **buffers the remainder**, so the TLS layer can hold already-decrypted app bytes while the raw fd is idle. An fd-only poll then reports "not ready" while a byte is available — a correctness bug. Readiness = `(TLS-buffered app bytes > 0) OR (raw layer readable)`, and the buffered half is backend-specific:

- **openssl** (`src/target/shared/code/tls/openssl.rs`): `SSL_pending(ssl)` for buffered count, else `poll(fd, POLLIN)`.
- **schannel** (Windows, `tls/schannel_io.rs` / `schannel_read_close.rs`): a decrypted-record carry-over buffer, else `WSAPoll(POLLRDNORM)` (`schannel_impl.rs` `WSAPOLLFD`).
- **macOS** (`tls/macos/`): **Network.framework — there is NO raw fd.** Decrypted data lands in a single-producer/single-consumer ring of pending retained buffers (`macos/mod.rs:106`), drained on the owning thread; readiness = ring non-empty OR terminal state, and any bounded wait uses the read path's `dispatch_semaphore` (which already has a per-read release/leak hazard, `macos/mod.rs:418` — a poll waiting on it must not steal the read's signal or leak a semaphore).

As of this writing `tls` has **no** `poll` and **no** `setReadTimeout` at all (`src/builtins/tls.rs` `TLS_FUNCTIONS` has connect/listen/accept/read/write/close only); `net` does have both. Related: the resources ownership model, and `net::poll` fd scaffolding at `src/target/shared/code/net/poll.rs:17`.

## Repository client transport security is per-URL, not per-hop

In `repository/src/client.rs`, `ensure_transport_security(repo_url)` validates ONLY the initial URL (https, or http-loopback for local dev). It is called at each network entry point (`get_json`/`post_json`/`fetch_blob`/…) but does NOT follow redirects, so a 302 target is never re-checked by it.

The one shared `reqwest::blocking::Client` is built once in `http_client()` (a `OnceLock`) — that is the ONLY place `connect_timeout` AND the redirect policy can be set. Per-hop transport enforcement therefore lives in `redirect_policy()` / `ensure_redirect_target()`, not in `ensure_transport_security`. The redirect guard is https-only and blocks private/loopback/link-local/CGNAT/unspecified IP literals (incl. IPv4-mapped IPv6) — otherwise a hostile registry 302 drives blind SSRF (169.254.169.254, 127.0.0.1, RFC-1918) or an https→http downgrade leak. Blob bytes stay SHA-256 checked and control-plane bodies signature-checked regardless; the redirect guard only closes the transport-level leak.

Takeaway: if you add a new registry route or loosen networking, remember the initial-URL check and the redirect check are SEPARATE — enforce both. `reqwest::redirect::Policy::custom` closures track depth via `attempt.previous().len()` and reject a hop with `attempt.error(String)`.
