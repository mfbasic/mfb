# audit-3 — Surface 6: crypto / TLS / verification

Part of `planning/goal-08-platform-security-review.md`. Finding prefix `CRY-`
(crypto package: CRY-01..08; tls package + trust: CRY-50..54). Untrusted party: a
remote TLS peer; the author of a signed `.mfp`; whoever supplies ciphertext /
passwords / signatures / certs to a compiled program's `crypto::`/`tls::` calls.

**Verdict: 1 HIGH · 4 MEDIUM · rest LOW/NTH.** The TLS *trust* core is sound — no
`SSL_VERIFY_NONE` reachable, chain + hostname verified by default on all three
backends, no env/manifest/config trust knob (only the call-site `allowSelfSigned`,
which `http::` pins false). The one HIGH is a Windows-backend layout confusion in
`tls::write` (a memory-safety mirror of the Surface-4 CRITICAL). The remaining
findings are a timing side-channel, a documented-but-unenforced protocol floor, an
audit blind spot, and a Windows key-container lifetime nit.

## HIGH

### CRY-50 — Schannel `tls::write(sock, List OF Byte)` uses the String layout → OOB read, remote crash of every MFBASIC HTTPS server on Windows → **bug-508**

`lower_tls_write` does `let _ = text;` (discards the payload-type selector) and
reads length from `arg+0` / data from `arg+8` — the String layout — for a byte-list
payload whose collection block puts `count` at `+8` and data ~40 bytes in
(`gen_schannel_io.rs:334`, lead-verified). The block-header word becomes the write
length (~16.4 MiB) → OOB read → `c0000005`. Agent-demonstrated on box 2230; the
Schannel mirror of the Surface-4 CRITICAL bug-497.

## MEDIUM

- **CRY-01** — X25519 ladder conditionally swaps under `IF r = 1` on a
  private-scalar bit (X448/Ed25519 use masked selects) → timing side-channel leaking
  the scalar; reachable via `crypto::exchange`/`decrypt`
  (`helper_x25519.rs:37-70`, lead-verified). → **bug-511.**
- **CRY-51** — the documented "TLS 1.2 floor" is enforced on OpenSSL only; Schannel
  leaves `grbitEnabledProtocols=0` and the macOS client sets no min version; the
  code comment claiming `SCH_USE_STRONG_CRYPTO` supplies the floor is wrong
  (`gen_schannel_impl.rs:399-425`). Re-open of audit-2 CRY-01 (macOS half) plus a
  new Windows half.
- **CRY-52** — `mfb audit`'s `AUDIT-TLS-RELAXED-TRUST` misses `allowSelfSigned`
  passed **positionally** (identical IR, clean audit); demonstrated with `mfb audit`
  (`src/audit/collect/source.rs:679-689`).
- **CRY-53** — Windows server private key imported into a *persisted* CryptoAPI key
  container deleted only on the clean-close path; container name = hex of a heap
  pointer (`gen_schannel_server.rs:505-543`).

## LOW / NTH

- **CRY-02** (LOW) — AES-GCM core is S-box-table-driven; `xtime` and GHASH's
  bit-serial multiply branch on secret-derived values (`helper_aes_sub.rs:11`).
- **CRY-03** (LOW) — Win64 CNG ECDSA verify never checks the DER SEQUENCE length or
  trailing bytes → accepts signatures macOS/Linux reject (malleability;
  platform-divergent verdict); memory-safe (`func_verify.rs:1211`).
- **CRY-04** (LOW) — `crypto::pbkdf2` iteration count/length and `shake256` length
  caller-controlled with no ceiling / interruption point (`helper_pbkdf2.rs:20`).
- **CRY-05** (LOW) — AEAD 2^32-block limit unenforced; GCM/ChaCha counter wraps back
  into J0 (keystream reuse) (`helper_gcm_inc32.rs:14`).
- **CRY-06** (LOW) — MFB software cores never wipe key material (native seams do)
  (`helper_hmac.rs:25`).
- **CRY-07** (NTH) — Ed25519 verify accepts a non-canonical public key (`y ≥ p`),
  no small-order check → divergence from ed25519-dalek used for `.mfp` trust
  (`helper_unpack25519.rs:20`).
- **CRY-08** (NTH) — Poly1305 final conditional subtraction branches on the
  accumulator (`helper_poly_finish.rs:50`).
- **CRY-54** (LOW) — macOS listener→accept SPSC ring uses plain loads/stores, no
  barriers, and its fast path bypasses the semaphore (`macos_aarch64/tls.rs:437`).

## Positives (re-verified)

AEAD tag verified **before** any plaintext exists and the compare is constant-time;
entropy is `getentropy`/`BCryptGenRandom` with a checked return and wiped scratch;
audit-2 CRY-02 (`S ≥ L`) and CRY-03 (`constantTimeEqual` length) both confirmed
fixed (the latter in emitted code); `gen_cert.rs` issues no certificate (no
serial/validity/CA question); no release-only `debug_assert` bounds hole. TLS: no
`SSL_VERIFY_NONE` reachable, chain+hostname verified by default on all three
backends, no env/manifest/config trust knob (only call-site `allowSelfSigned`,
which `http::` pins false), no session resumption/early data, all three accept
paths bound the handshake when `timeoutMs` is given. **Revocation checking is
absent on every backend and not documented as absent** — noted, not filed.

## Bug docs filed

bug-508 (CRY-50, HIGH), bug-511 (CRY-01, MEDIUM). CRY-51/52/53 recorded for
follow-up.

## Coverage

Read: `builtins/crypto/**` (AEAD, hash/HMAC/HKDF/PBKDF2, sign/verify, exchange,
random, constant-time equal, gen_cert, the X25519/AES/GHASH/Poly cores),
`builtins/tls/**` (schannel + macOS + openssl backends, the write/connect/accept
paths, trust decisions), `src/audit/collect/source.rs` (the TLS-relaxed-trust rule).

Gaps: no constant *table* (SHA K/IV, AES S-box, Keccak RC, Ed448/GF) was verified
against the standards (relied on the KAT fixtures); the ~70 field-arithmetic/
schedule helpers were not audited for a hidden branch-on-secret of the CRY-01/02
class; `gen_macos/server.rs`'s `tls::listen` construction, the three close/shutdown
tails, and `gen_macos/address.rs` skimmed. No Windows/Linux execution (CRY-03/50
code-read / box-2230-agent).
