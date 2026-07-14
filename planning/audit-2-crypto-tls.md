# Audit 2 — Surface 5: Crypto / TLS / verification

Last updated: 2026-07-14
Untrusted party: a remote TLS peer, or the author of a signed `.mfp`. Must not:
bypass certificate/signature verification, exploit a weak/misused primitive or
predictable secret, or leak key material. This surface had **no** prior audit-1
coverage (bug-96 noted the gap); audited fresh.

Scope read: `src/target/shared/code/crypto_ec.rs`,
`src/target/shared/code/crypto.rs`, `src/target/shared/runtime/crypto_specs.rs`,
`src/builtins/crypto.rs`, `src/builtins/crypto_package.mfb`, `src/builtins/tls.rs`,
`src/target/shared/code/tls/{openssl,macos,mod}.rs`, `repository/src/crypto.rs`,
`repository/src/package.rs`, `src/audit/collect/source.rs`.
Dependency usage noted where relevant: ed25519-dalek 2.2.0, p256 0.13.2,
rustls 0.23.41, security-framework 3.7.0.

## Positive confirmations (the point of the audit)

- **TLS certificate verification is enforced on both platforms by default; no
  reachable skip-verify path.** Linux/OpenSSL: `SSL_set_verify(SSL_VERIFY_PEER)`
  → `SSL_set1_host` (result checked `==1`, `tls/openssl.rs:511-512`) → `SSL_connect`
  checked `==1` (`:562-563`) → `SSL_get_verify_result` checked `==X509_V_OK`
  (`:581-582`); TLS 1.2 floor set and checked (`:533-544`). No `SSL_VERIFY_NONE`,
  no user knob. macOS Network.framework installs only
  `sec_protocol_options_set_tls_server_name` (`tls/macos.rs:606`) — **no**
  `sec_protocol_verify_block`/trust-override — so default chain+hostname
  validation applies. The SNI/validation name is `serverName` if non-empty else
  `host` (`openssl.rs:337-364`, `macos.rs:536`) — never empty, so hostname
  verification always runs.
- **ECDSA verify return codes checked correctly** (no "any nonzero = success"):
  OpenSSL `EVP_DigestVerify` compared `==1` (`openssl.rs:1577-1586`); macOS
  `SecKeyVerifySignature` `Boolean` masked to bit 0 (`macos.rs:1231-1235`).
- **Invalid-curve/malformed keys rejected by the library**, not mistrusted:
  OpenSSL splices the caller point into a fixed SPKI + `d2i_PUBKEY`
  (on-curve validation inside `EC_POINT_oct2point`, `openssl.rs:1473-1494`); macOS
  `SecKeyCreateWithData(kSecAttrKeyClassPublic)` validates the point
  (`macos.rs:1172-1193`). Untrusted DER signatures are never hand-parsed.
- **No predictable secrets; secrets zeroized.** `crypto::randomBytes` uses
  `getentropy` in ≤256-byte chunks with a checked return and wipes the scratch
  (`crypto.rs:99-108,180-198`, bug-177 D); never the per-arena PCG64. `randomInt`
  is unbiased rejection sampling (`crypto_package.mfb:1520-1539`). Ed25519 signing
  uses the deterministic RFC-8032 nonce (`:2170-2172`). EC key scratch wiped on
  success and every error exit (`openssl.rs:839-843,1235-1237,883-886`;
  `macos.rs:949-950,968`). `.mfp` trust crypto uses ed25519-dalek + OsRng,
  argon2id KDF, ChaCha20-Poly1305 with OsRng nonce, domain-separated inputs
  (`repository/src/crypto.rs:72-234`); package verify is fail-closed
  (`repository/src/package.rs:231-239`).
- **bug-96 confirmed fixed:** the audit collector classifies tls/http as network
  and the crypto builtins as randomness/fallible surfaces
  (`src/audit/collect/source.rs:517-606,637`).

## Findings (all LOW / NTH)

### CRY-01 — LOW — macOS TLS client sets no minimum protocol version (downgrade asymmetry vs Linux)
- Location: `src/target/shared/code/tls/macos.rs` connect path (`:507-620`; only
  `set_tls_server_name` configured — no `sec_protocol_options_set_min_tls_protocol_version`).
  Contrast `tls/openssl.rs:533-544` (TLS 1.2 floor set + checked).
- Threat/impact: an active attacker could negotiate an older TLS version down to
  the OS default minimum on macOS (legacy-protocol weakness). Does **not** bypass
  cert authentication — chain+hostname still validated — so only weakens, not
  defeats, a correctly-authenticated session.
- Best fix (internal): call
  `sec_protocol_options_set_min_tls_protocol_version(options, TLSv12)` in the same
  configure block that sets the server name.
- Not demonstrated on-host (needs a macOS box + a MITM forcing TLS 1.0). LOW —
  recent macOS may already default to TLS 1.2.

### CRY-02 — LOW (NTH) — Ed25519 software-core verify accepts non-canonical / malleable S
- Location: `src/builtins/crypto_package.mfb:2202-2229` (`__crypto_ed25519Verify`).
- Threat/impact: the TweetNaCl-style verify checks `len==64`, validates the point,
  recomputes `R'`, returns `constantTimeEqual(R,R')` — but does **not** range-check
  `S < L` and uses cofactored verification. A third party can transform a valid
  signature into a distinct still-verifying one (same message) without the key.
  Matters only if a program treats the signature bytes as a unique/opaque id
  (dedup key, replay cache). Does **not** allow forging over a new message. Scope:
  only the MFBASIC-visible `crypto::ed25519Verify`; the `.mfp` trust chain uses
  ed25519-dalek (`repository/src/crypto.rs:35-48`), unaffected.
- Best fix: add an `S < L` canonicality check (compare against the order constant
  already present for `__crypto_modL`); `RETURN FALSE` if not strictly less.
- Non-goals: switching the software core to strict/ZIP-215 wholesale.

### CRY-03 — NTH — `constantTimeEqual` leaks operand length via early return
- Location: `src/builtins/crypto_package.mfb:1486-1501`. Length mismatch returns
  immediately (`:1489-1491`); the compare loop runs `na` iterations, so total time
  reveals length (in)equality. The per-byte compare itself is constant-time
  (`bor`/`bxor` into `diff`). Length is public in almost all MAC schemes → minimal
  impact; standard behavior. No code change strictly needed; document the
  equal-length assumption if desired.

## Verdict

Surface 5 is **unusually well-hardened** (largely goal-03 fixes: bug-55/136/177).
No verification bypass, no predictable secret, no key-material leak; verification
is fail-closed on both platforms. Only LOW/NTH items (CRY-01/02/03). No bug docs.
bug-96 confirmed fixed.

Adjacency note: the *server-side enforcement* of `.mfp` trust (whether
`verify_packages`/`classify_installed_package` is always on the install path)
belongs to Surfaces 1/8 and is covered there (PKG-01 fixed; SUP-04 confirmed
install is not blind).
