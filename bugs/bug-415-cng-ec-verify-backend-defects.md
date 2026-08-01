# bug-415: Windows CNG EC verify backend — malformed key returns FALSE/ErrUnknown instead of ErrInvalidArgument, unbounded DER parse, and a hash-provider handle leak

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (spec/cross-platform divergence) + Memory-safety (LOW) +
Resource-safety (LOW)

Status: Open
Regression Test: tests/ — a Windows `crypto::p256Verify` with a wrong-length /
off-curve public key must raise `ErrInvalidArgument` (matching Linux/macOS); a
short/malformed DER signature must not read past the signature buffer.

Three defects in `src/target/shared/code/crypto_ec/cng_sign_verify.rs` (the Windows
CNG ECDSA backend, new since goal-06), batched (same file/subsystem):

### (1) `:424` — malformed public key gives the wrong result (MED, security-adjacent)
The `crypto p256Verify` man mandates: "a publicKey that is not a well-formed 65-byte
SEC1 point (wrong length, or bytes that do not decode to a valid curve point) raises
`ErrInvalidArgument` rather than returning a verdict." CNG `verify()` (:377-511) has
**no** `ErrInvalidArgument` path (`ERR_INVALID_ARGUMENT` appears only in `sign()`,
:362). A wrong-length pubkey routes `branch_ne(&bad_sig)` (:424) → Boolean FALSE
(:493-499); a right-length but off-curve key makes `BCryptImportKeyPair` fail →
`&fail` → ErrUnknown (:435). Both cases diverge from macOS (`macos.rs:1021-1025` →
invalid_fail) and OpenSSL (`openssl.rs:1321` `emit_len_check` → invalid_fail), which
raise `ErrInvalidArgument`. It fails **safe** (never accepts a bad signature), hence
MED — but breaks the documented wire-compatible cross-platform contract.
- Fix: add a SEC1-length + import-failure → `ErrInvalidArgument` path in CNG
  `verify`, mirroring the macOS/OpenSSL backends.

### (2) `:441` — unbounded DER parse of the untrusted signature (LOW, OOB read)
CNG `verify` is the only backend that hand-parses the untrusted DER signature
(macOS/OpenSSL delegate to `SecKeyVerifySignature`/`EVP_DigestVerify`). It never
bounds against `SIGLEN`: `:442-456` read `sig[0]`/`sig[1]` with no `SIGLEN >= 2`
guard, and `der_decode_int` (:53-104) reads a caller-declared INTEGER length and
`copy_bytes` that many bytes from `body+2` with only a `len <= field` cap (:84-85),
never checking `body+2+len <= SIGBUF+SIGLEN`. A crafted short signature reads up to
`field` (32/48/66) bytes past the `max(count,1)`-byte arena buffer; the SEQUENCE-body
advance can walk ~255 bytes past probing for the next tag. Bounded into adjacent
arena memory (no SIGSEGV, cannot forge TRUE — BCryptVerifySignature does the real
crypto), hence LOW.
- Fix: bounds-check every DER length against `SIGLEN` (mirror bug-136.3's OpenSSL
  SEC1/SPKI guards).

### (3) `:207` — BCrypt hash provider handle leak on error (LOW)
`hash_message` opens a hash algorithm provider into `HASHALG` (:188-195) and closes
it only on success (:209-213). If `BCryptHash` returns NTSTATUS < 0 (:206-207), it
`branch_lt(fail)` before the close, leaking the provider handle; the shared
`emit_cleanup` doesn't know `HASHALG`. Affects sign and verify. Per-call kernel
handle leak (process exit reclaims); rare (catastrophic CNG error).
- Fix: close `HASHALG` on the failure path (or register it with `emit_cleanup`).

References: `src/target/shared/code/crypto_ec/cng_sign_verify.rs:424`/`:441`/`:207`;
contrast `macos.rs:1021-1025`, `openssl.rs:1321`/`:1834`, bug-136.3, bug-317 T4.
Found during goal-07.

## Failing Reproduction

Windows-only (CNG backend); not reproducible on the macOS host. Confirmed
statically: no `ERR_INVALID_ARGUMENT` emit in CNG `verify`; no `SIGLEN`-vs-copy
bound in `der_decode_int`; `BCryptHash` failure branches before the `HASHALG` close.

- Observed: malformed pubkey → FALSE/ErrUnknown; short DER → OOB read of adjacent
  arena; BCryptHash failure → leaked provider handle.
- Expected: `ErrInvalidArgument` for a malformed key; bounded DER parse; handle
  closed on every path.

## Root Cause

The CNG verify backend omits the argument validation, DER bounds, and error-path
cleanup that the macOS/OpenSSL backends apply.

## Goal

- CNG EC verify matches the documented contract: `ErrInvalidArgument` on a malformed
  key, bounded untrusted-DER parsing, and no handle leak on error.

### Non-goals

- The real signature verification (BCryptVerifySignature) is correct and must be
  preserved. The macOS/OpenSSL backends are already correct.

## Blast Radius

- `cng_sign_verify.rs:424` (verify verdict), `:53-104`/`:441` (DER bounds), `:207`
  (handle leak). All Windows-CNG-only; other backends unaffected.
