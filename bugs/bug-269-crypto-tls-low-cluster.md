# bug-269: crypto/TLS residual LOW/NTH cluster — macOS TLS min-version floor, Ed25519 malleable S, constantTimeEqual length leak

Last updated: 2026-07-17
Effort: small (<1h, three small items)
Severity: LOW
Class: Security (hardening)

Status: Open
Regression Test: (none yet)

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
