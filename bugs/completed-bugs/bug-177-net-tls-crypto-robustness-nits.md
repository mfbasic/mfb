# bug-177 — net/tls/crypto robustness nits (datagram EINTR, error-path fd leak, TLS-by-IP, entropy not zeroized, ignored library return codes)

Last updated: 2026-07-12
Severity: LOW (batch).
Class: Correctness / Memory-safety / Security (all LOW).
Status: FIXED (2026-07-13; goal: resolve bugs 170-180; full acceptance suite green)

## Findings

**A. `recvfrom`/`sendto` do not retry EINTR.**
`src/target/shared/code/net/io.rs:1398-1401` (recvfrom), `:1644-1652` (sendto).
`recv_fail`/`send_fail` test EAGAIN (and EMSGSIZE) but never `EINTR`, so a signal
interrupting a blocking datagram op before any byte moves is misreported as
`ERR_NETWORK_FAILED`. bug-115 added EINTR retry to accept/read/write/poll/connect
but missed these two. Fix: add an EINTR comparison that re-issues the syscall.
The same one-shot path-based fs write/read loops treat EINTR as a hard error
(`src/target/shared/code/fs_helpers_atomic.rs:521-526, 863-868, 1362-1367` and the
`read_text_path` loop) — they branch directly instead of using
`emit_transfer_loop_tail`/`emit_single_op_eintr_guard` (the File-based helpers in
`fs_helpers_io.rs` retry EINTR per bug-62). Partial transfers are handled
correctly (not a truncation bug), but an EINTR before any byte moves becomes a
spurious ErrOutput/ErrRead. Same fix.

**B. `tls::connect` `load_fail` path leaks the connected socket fd (OpenSSL).**
`src/target/shared/code/tls/openssl.rs:639-647`. If libssl fails to dlopen (or a
core symbol is absent) *after* the TCP socket is connected, `load_fail` calls only
`emit_fail` and returns — unlike `net_fail_fd`/`tls_fail`/`connect_timeout`/
`alloc_fail` it never `close`s the fd at `FD_OFFSET`. One leaked fd in a
near-fatal (OpenSSL-missing) environment. Fix: `close(fd)` (guarded fd≥0) before
`emit_fail`.

**C. TLS hostname verification is DNS-name only; connecting by IP literal is not
verified-by-IP (fails closed).** `src/target/shared/code/tls/openssl.rs:489-508`.
`SSL_set1_host(ssl, sniCstr)` matches DNS-name SANs/CN only, not `iPAddress` SANs
(that needs `X509_VERIFY_PARAM_set1_ip`), so an IP-literal connection fails
verification rather than validating against an iPAddress SAN. This fails *closed*
(over-strict) — not an auth bypass — hence LOW. (Full chain trust + DNS-name
verification ARE correctly wired on both backends; no verification bypass exists.)
Fix (optional): detect a numeric host and use `SSL_set1_ip`, or document TLS-by-IP
as unsupported.

**D. `crypto::randomBytes` entropy scratch buffer left un-zeroized in the arena.**
`src/target/shared/code/crypto.rs:141-160`. Unlike the EC helpers
(`zero_scratch_guarded`), the getentropy scratch block is neither freed nor wiped,
so a later same-program arena allocation can be handed a block still holding the
generated random bytes. Muted (the bytes are the returned value) but if used to
derive key material the residue outlives its intended lifetime. Also `:113-118`:
`randomBytes(n)` computes `count*ENTRY + HEADER + count` with no overflow/upper
bound (only the incidental earlier `arena_alloc(count)` guards it). Fix: zero the
scratch after the copy; reject `count` above a sane maximum.

**E. OpenSSL EC keygen/encode return codes ignored.**
`src/target/shared/code/crypto_ec/openssl.rs:596-612` (`EC_KEY_generate_key`
result discarded on the 1.1 fallback path — mitigated because `i2d_PrivateKey` on
a keyless EVP_PKEY returns ≤0 → gen_fail, and the SEC1 bounds guard rejects a
short buffer), `:698, :734` (second-call `i2d_PrivateKey`/`i2d_PUBKEY` write
returns discarded → silent partial/undefined buffer content, no OOB), `:720-757`
(SPKI point slice not length-checked against `i2d_PUBKEY` output, unlike the
bug-136.3-guarded SEC1 scalar read at `:765-769`; in-bounds in practice since the
SPKI is self-produced with default uncompressed form). No signature-verification
bypass or weak-key emission — these are robustness/asymmetry gaps. Fix: check each
library return `> 0`/`== 1` and route to `gen_fail`; add the SPKI length guard.
