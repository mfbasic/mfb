# bug-269: crypto/TLS residual LOW/NTH cluster — macOS TLS min-version floor, Ed25519 malleable S, constantTimeEqual length leak

Last updated: 2026-07-17
Effort: small (<1h, three small items)
Severity: LOW
Class: Security (hardening)

Status: Fixed (CRY-02/CRY-03 landed; CRY-01 dispositioned below)
Regression Test:
`tests/rt-behavior/crypto/crypto-ed25519-malleability-invalid` (a genuine
signature verifies; the `R || S+L` malleation is rejected — CRY-02; and
constantTimeEqual returns the right verdict for equal, unequal-same-length, and
unequal-length inputs — CRY-03)

## Resolution

- **CRY-02 (Ed25519 malleable S) — FIXED.** `__crypto_ed25519Verify`
  (`crypto_package.mfb`) now rejects a non-canonical `S` before verifying: a new
  `__crypto_scalarBelowL` compares the 32-byte little-endian `S` against the group
  order `L` (`__crypto_edL`) from the most-significant byte down and returns FALSE
  unless `S < L`. A malleated `(R || S+L)` signature no longer verifies against the
  same message, so signature bytes stay a stable identity (safe as a dedup/replay
  key). Genuine signatures (whose `S` is reduced mod `L` at signing) are unaffected.
  The software core is not otherwise switched to strict/ZIP-215 semantics.
- **CRY-03 (constantTimeEqual length branch) — FIXED.** The length comparison is
  folded into the accumulated `diff` (`diff = bxor(na, nb)`, then the byte loop over
  the shared prefix) instead of an early `RETURN FALSE`, so the total time no longer
  branches on length (in)equality. The `constantTimeEqual` man page documents the
  behavior.
- **CRY-01 (macOS TLS min-version floor) — DEFERRED (documented).** Pinning a
  TLS 1.2 floor requires capturing
  `sec_protocol_options_set_min_tls_protocol_version` in the SNI configure block
  and calling it from the block's `invoke`. That means growing the block literal
  (a 5th 8-byte capture), bumping the shared `CFG_DESC` block-descriptor size,
  shifting the stack-frame offsets in both the connect and listen frames, and
  editing the aarch64 invoke trampoline — delicate, macOS-only block-ABI surgery on
  a hardware-validated-working TLS path. Given the LOW severity (it does not bypass
  certificate authentication — chain + hostname are still validated — only weakens
  the negotiated minimum, and recent macOS Network.framework already negotiates
  TLS 1.2+ by default), the regression risk to working TLS outweighs the benefit;
  this is tracked as a hardening item to land with dedicated macOS TLS validation.

Note: the man-page and doc updates keep the embedded spec current with the CRY-02
canonicality rejection and the CRY-03 length-timing behavior.

Three individually-LOW/NTH residual findings on the crypto/TLS surface from
audit-2 that lack their own bug docs. The surface is otherwise well-hardened
(fail-closed verification, no predictable secrets, secrets zeroized); these are
defense-in-depth. Grouped per the repo's low-severity-batch convention.

References:

- `planning/audit-2-crypto-tls.md` (CRY-01, CRY-02, CRY-03).

## Findings

### CRY-01 — macOS TLS client sets no minimum protocol version (downgrade asymmetry vs Linux)
- Location: `src/target/shared/code/tls/macos.rs` connect path (`:507-620`) — only
  `sec_protocol_options_set_tls_server_name` configured; **no**
  `sec_protocol_options_set_min_tls_protocol_version`. Contrast Linux
  `tls/openssl.rs:533-544` (TLS 1.2 floor set + checked).
- Symptom: an active attacker could negotiate down to the OS default minimum on
  macOS. Does **not** bypass cert authentication (chain+hostname still validated)
  — only weakens, not defeats, a correctly-authenticated session. Recent macOS may
  already default to TLS 1.2.
- Fix: call `sec_protocol_options_set_min_tls_protocol_version(options, TLSv12)`
  in the same configure block that sets the server name.

### CRY-02 — Ed25519 software-core verify accepts non-canonical / malleable S
- Location: `src/builtins/crypto_package.mfb:2202-2229`
  (`__crypto_ed25519Verify`).
- Symptom: the TweetNaCl-style verify checks `len==64`, validates the point,
  recomputes `R'`, returns `constantTimeEqual(R,R')` — but does **not** range-check
  `S < L` and uses cofactored verification, so a third party can transform a valid
  signature into a distinct still-verifying one (same message) without the key.
  Matters only if a program treats the signature bytes as a unique/opaque id
  (dedup key, replay cache). Does **not** allow forging over a new message. The
  `.mfp` trust chain uses ed25519-dalek and is unaffected.
- Fix: add an `S < L` canonicality check (compare against the order constant
  already present for `__crypto_modL`); `RETURN FALSE` if not strictly less. Do
  not switch the software core to strict/ZIP-215 wholesale.

### CRY-03 — constantTimeEqual leaks operand length via early return (NTH)
- Location: `src/builtins/crypto_package.mfb:1486-1501`. A length mismatch returns
  immediately (`:1489-1491`); the compare loop then runs `na` iterations, so total
  time reveals length (in)equality. The per-byte compare itself is constant-time
  (`bor`/`bxor` into `diff`).
- Symptom: length is public in almost all MAC schemes → minimal impact; standard
  behavior. No code change strictly required.
- Fix (optional): document the equal-length assumption; or fold the length check
  into the accumulated `diff` so timing does not branch on length.

## Goal

- macOS TLS pins a TLS 1.2 floor (CRY-01); `__crypto_ed25519Verify` rejects
  `S >= L` (CRY-02); the `constantTimeEqual` length behavior is documented or
  made length-timing-independent (CRY-03).

### Non-goals (must NOT change)

- The fail-closed TLS verification posture on either platform.
- The `.mfp` trust chain (ed25519-dalek), which is unaffected.
- Switching the Ed25519 software core to strict/ZIP-215 semantics wholesale.
