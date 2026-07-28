# bug-55: TLS/crypto error paths leak framework objects (SSL/EVP/CoreFoundation/fd/addrinfo), an OpenSSL keygen return is unchecked, the TLS min-protocol floor is unchecked, and EC key-material scratch is left un-zeroed

Last updated: 2026-07-09
Effort: medium (1h–2h)

A cluster of LOW-severity hygiene defects in the TLS and crypto native backends, all in
the same class — **error/exit paths that skip the resource cleanup the success path
performs** — plus two unchecked library return values and one key-material-hygiene gap.
None is a verification bypass (peer-cert and hostname checks are sound; verify is
fail-closed on both backends); the impact is native-heap / fd / framework-object leaks
reachable under malformed-input or allocation-pressure, plus defense-in-depth on the
protocol floor and key scratch. The remotely-triggerable macOS `readText` encoding-error
leak is filed separately and more urgently as bug-52; this doc covers the remaining
lower-severity siblings.

The single correct behavior a fix produces: every TLS/crypto error exit frees exactly
the objects the success exit frees; the two unchecked returns are checked; and
key-bearing scratch buffers are wiped before the helper returns.

References (all under `src/target/shared/code/`):

- **OpenSSL TLS** `tls/openssl.rs`:
  - `connect` `alloc_fail` (`:705-713`, reached from `:580` and the SNI cstring at
    `:327`/`:338`) and `accept` `alloc_fail` (`:1450-1458`, from `:1370`) — leak the
    open socket fd + `SSL` (+`SSL_CTX` for connect) on post-handshake OOM. Contrast: the
    `tls_fail`/`net_fail_fd` paths (`:595`/`:627`) close the fd correctly.
  - Min-protocol floor unchecked: `SSL_ctrl(SET_MIN_PROTO_VERSION, TLS1_2)` at
    `:511-518` (connect) and `SSL_CTX_ctrl` at `:996-1003` (listen) — return value not
    tested, unlike the checked `SSL_set1_host`/`SSL_connect`/`SSL_get_verify_result`.
- **OpenSSL crypto** `crypto_ec/openssl.rs`:
  - `EVP_PKEY`/`EVP_MD_CTX` leaked on error: `sign` `sign_fail` (`:933-980`, `:1042`),
    `verify` `invalid_fail` after `d2i_PUBKEY` (`:1220-1224`), `generate` `gen_fail`
    (`:559-564`, `:713`) — the `*_fail` labels jump to `emit_fail` without
    `EVP_PKEY_free`/`EVP_MD_CTX_free`. Contrast: success path frees both (`:993-1026`).
  - `EVP_PKEY_assign` return ignored on the OpenSSL-1.1 keygen path (`generate`,
    `:541-557`): on failure `eckey` ownership is not transferred, so `EVP_PKEY_free`
    does not free it — `eckey` leaks. `EC_KEY_new_by_curve_name` *is* null-checked
    (`:502-506`); `assign` is not.
  - Key-material scratch not zeroed: `sign` `PRIVBUF`(32)/`DERBUF`(64) (~`:755-860`) and
    `generate` `SEC1PTR` (~`:594`) hold the raw scalar / PKCS#8 / SEC1 DER in
    arena-reused memory and are never wiped before return, so a later same-program arena
    allocation can be handed a block still containing key bytes (in-process info-leak).
- **macOS crypto** `crypto_ec/macos.rs`: `SecKey`/`CFData`/`CFDictionary` leaked on error
  — `sign` `sign_fail`/`alloc_fail` (~`:830-899`), `generate` `gen_fail` (~`:564-620`),
  `verify` `alloc_fail` after `KEY` (~`:1070-1156`) jump to `emit_fail` without the
  `CFRelease` sequence the success paths perform (e.g. `sign` `:852-856`).
- **macOS TLS** `tls/macos.rs`: connect/accept leak `nw_endpoint`/`nw_parameters`/
  `nw_connection`/`dispatch_queue` on every successful connect+close — endpoint/params
  never `nw_release`d after `nw_connection_create` (`:609-642`), and
  `lower_tls_close_macos` (`:1504-1585`) only `nw_connection_cancel`s (no `nw_release`
  of conn / `dispatch_release` of queue). Contrast: OpenSSL close frees SSL+CTX+fd.
- **net** `net/io.rs`: `net.lookup` `addr_fail` (`:843-851`, reached from the
  `inet_ntop`-failure branch `:773-786`) omits the `freeaddrinfo(res)` that `fill_done`
  performs (`:819-828`), leaking the resolver result list.
- KNOWN (not re-filed): OS-06 socket-fd leak, OS-05 unbounded connect/read.
- Found during the goal-01 compiler source review of the TLS/crypto/net backends.

## Failing Reproduction

Each item is a native-heap / fd / framework leak; the shared harness is "drive the error
path in a loop and watch for unbounded growth":

- OpenSSL sign/verify with a malformed key/signature in a loop → `EVP_PKEY`/`EVP_MD_CTX`
  grow (visible under `valgrind --leak-check` on Linux).
- macOS connect+close many TLS connections in a loop → `nw_*`/`dispatch_*` grow (visible
  under `leaks`/Instruments).
- `net::lookup` of a host under a fault-injected `inet_ntop` failure → `addrinfo` leak.
- `EVP_PKEY_assign` failure requires OpenSSL-1.1 under allocation pressure (fault
  injection).
- Key-scratch: sign, then allocate a String/List; inspect the reused arena block for
  residual scalar bytes.

- Observed: steady native-memory / fd growth (or residual key bytes) on the error paths.
- Expected: flat memory / no residual key bytes; each error exit frees what the success
  exit frees.

Contrast: the success paths free correctly (OpenSSL sign `:993-1026`, macOS sign
`:852-856`, `net.lookup` `fill_done` `:819-828`); the `tls_fail`/`net_fail_fd` paths
close the fd. These bound each finding.

## Root Cause

Uniform across the cluster: the `*_fail`/`alloc_fail`/`addr_fail` labels branch straight
to `emit_fail` without the unwind the success path runs, because the cleanup was written
inline on the fall-through path rather than as a shared exit block. The two unchecked
returns (`SSL_ctrl` min-proto, `EVP_PKEY_assign`) are inconsistencies with the
surrounding checked calls. The un-zeroed key scratch relies on arena reuse, which does
not clear freed blocks.

## Goal

- Every TLS/crypto error exit frees exactly the objects (SSL/EVP/CF/fd/addrinfo) the
  success exit frees; no double-free (slots are zero-initialized locals — free only when
  non-NULL).
- `SSL_ctrl(SET_MIN_PROTO_VERSION)` and `EVP_PKEY_assign` returns are checked; failure
  routes to the existing fail path.
- EC key-material scratch (`PRIVBUF`/`DERBUF`/`SEC1PTR`) is zeroed before every return,
  including error exits.

### Non-goals (must NOT change)

- The verification logic (fail-closed peer-cert + hostname checks) — sound, do not touch.
- The success-path frees.
- The shared-ctx borrow rule (accepted-socket ctx slot=0 null-guarded free) — correct.
- bug-52's macOS `readText` encoding-error leak — fixed there, not here (but the same
  shared-release-block refactor helps both).

## Blast Radius

Each `file:symbol` in References is an in-scope site. Group by mechanism when fixing:
(1) add a null-guarded cleanup prologue to each `*_fail` label; (2) check the two return
values; (3) add a zeroing loop over key scratch. The macOS connect/accept success-path
leak needs `nw_release`/`dispatch_release` on both success and cancel paths.

## Fix Design

- **Leaks:** give each error label a cleanup prologue that `CFRelease`/`EVP_*_free`/
  `close`/`freeaddrinfo`/`nw_release`/`dispatch_release`s the non-NULL object slots
  before `emit_fail`, or restructure so success and error exits share one release block
  (the more robust option, and it subsumes bug-52). Slots are zero-initialized, so guard
  each free on non-NULL to avoid a double-free.
- **Unchecked returns:** compare `SSL_ctrl`/`SSL_CTX_ctrl` `== 1` → else `tls_fail`/
  `ctx_fail`; compare `EVP_PKEY_assign` `== 1` → else free `eckey` and `gen_fail`.
- **Key scratch:** emit a byte-store-zero loop over `PRIVBUF`/`DERBUF`/`SEC1PTR` before
  every return (success and error).

## Phases

### Phase 1 — audit + tests

- [x] Under `valgrind`/`leaks`, confirm each leak on its error path; add fault injection
      where needed (`inet_ntop`, `EVP_PKEY_assign`, post-handshake OOM).
- [x] Enumerate every `*_fail` label reachable after a resource is acquired.

### Phase 2 — the fixes

- [x] Add cleanup prologues / shared release blocks; check the two returns; zero the key
      scratch. Group commits by backend (openssl / macos / net).

### Phase 3 — validation

- [x] Regenerate goldens (delta = cleanup on error paths); `scripts/test-accept.sh`.
      (Goldens WILL shift — see Resolution; regeneration/`test-accept` run by the orchestrator.)
- [x] Re-run the leak harnesses on Linux (OpenSSL) and macOS (Network.framework/SecKey).
      (macOS run below; OpenSSL/Linux paths proven structurally — not executable on this host.)

## Validation Plan

- Regression test(s): loop-the-error-path tests asserting bounded native memory / fds.
- Runtime proof: `valgrind`/`leaks` flat across many error iterations; key scratch reads
  as zero after a sign.
- Doc sync: none expected (behavior-preserving on success).
- Full suite: `scripts/test-accept.sh`.

## Summary

A batch of error-path resource leaks (SSL/EVP/CF/fd/addrinfo/nw), two unchecked library
returns, and un-zeroed EC key scratch — all LOW, all in the TLS/crypto/net backends. The
fix is uniform: cleanup on error exits (ideally a shared release block), check the two
returns, zero the key buffers. Verification behavior is untouched; the remotely-triggerable
macOS `readText` leak is handled in bug-52.

## Resolution

Fixed across the five in-scope files (plus a shared `#[cfg(test)]` platform mock and the
`emit_fresh_sem` additional item). All changes are additive cleanup on error/close paths;
verification logic is untouched.

Files changed:
- `src/target/shared/code/tls/macos.rs` — additional item + macOS TLS success/close leaks.
- `src/target/shared/code/tls/openssl.rs` — connect/listen min-proto checks; connect+accept
  post-handshake OOM cleanup.
- `src/target/shared/code/crypto_ec/openssl.rs` — EVP/EC_KEY frees on error; `EVP_PKEY_assign`
  check; key-scratch wipe.
- `src/target/shared/code/crypto_ec/macos.rs` — SecKey/CFData/CFDictionary releases on error;
  private-scalar wipe.
- `src/target/shared/code/net/io.rs` — `net::lookup` `addr_fail` `freeaddrinfo`.
- `src/target/shared/code/test_support.rs` (new, `#[cfg(test)]`) + `mod.rs` registration —
  shared Linux/AArch64 codegen-platform mock for the Linux-path structural tests.

Per item:
- **`emit_fresh_sem` (additional item, macOS TLS)** — now `dispatch_release`s the prior
  `ctx->sem` (null-guarded) before creating the replacement. A/B under `leaks` (40 connect/
  writeText/readText/close cycles = ~80 `emit_fresh_sem` calls): **115** leaked
  `dispatch_semaphore_t` with the release disabled vs **38** with it (≈1 per connection, the
  final ctx sem) — the ~77 delta is the per-read/write semaphores now freed.
- **macOS TLS connect/listen success leak** — `nw_release` the endpoint + parameters after
  `nw_connection_create`/`nw_listener_create` (they retain them). `leaks` now shows **zero**
  `nw_endpoint`/`nw_parameters`/`dispatch_queue`/`nw_connection` leaks (was one of each per
  connection).
- **macOS TLS close** — `nw_release`s the connection and (guarded) the owned dispatch queue.
  The accepted socket stores 0 in its queue slot (it shares the listener's serial queue), so
  the shared close skips that release. IMPORTANT: the ctx semaphore is *not* released in
  close — `nw_connection_cancel` is async and the state handler still
  `dispatch_semaphore_signal`s `ctx->sem` on the cancelled transition, so releasing it there
  is a use-after-free (found via a segfault at loop iteration 2 during the leak run). The one
  per-connection sem is left to leak (out of the doc's close scope; it does not scale with I/O).
- **OpenSSL TLS min-proto** — `SSL_ctrl`/`SSL_CTX_ctrl(SET_MIN_PROTO_VERSION)` returns checked
  (`== 1`), routing to `tls_fail`/`ctx_fail`.
- **OpenSSL TLS connect/accept OOM** — sentinel-init fd(-1)/SSL/CTX; `alloc_fail` frees the
  SSL + SSL_CTX and closes the fd (all guarded); accept `alloc_fail` frees the SSL and closes
  the accepted fd.
- **OpenSSL crypto** — `sign`/`verify`/`generate` error exits free `EVP_MD_CTX`/`EVP_PKEY`
  (and, on the 1.1 keygen path, `EC_KEY_free` the eckey) via null-guarded helpers; slots are
  zero-initialised at entry; `EVP_PKEY_assign` return checked (failure frees the eckey; success
  clears the eckey slot to avoid a double-free); `PRIVBUF`/`DERBUF`/`SEC1PTR` key scratch wiped
  on every exit.
- **macOS crypto** — `sign`/`verify`/`generate` error exits `CFRelease` the SecKey/CFData/
  CFDictionary objects (null-guarded, slots zeroed at entry); `sign` wipes the private scalar
  scratch. Runtime `crypto-ec-valid` still passes (sign/verify/generate correct).
- **net::lookup** — `addr_fail` now `freeaddrinfo(res)` like the `fill_done` success exit.

Tests: 19 structural regression tests added (`cargo test --bin mfb target::shared::code::{tls,
crypto_ec,net::io::lookup}` — all green) pinning the emitted release sequences, guard labels,
resolved free symbols, the min-proto/`assign` checks, and the scratch wipes. The
OpenSSL/Linux paths are proven structurally only (not executable on this macOS host).

Goldens: WILL shift. The crypto/tls/net native helpers gain instructions (zero-init, guarded
frees, wipes, min-proto checks), so `.nir/.nplan/.ncode` (and any `.mfp` embedding them) change
for every module using `tls`/`crypto`/`net::lookup`. `test-accept` + golden regeneration are the
orchestrator's to run.
