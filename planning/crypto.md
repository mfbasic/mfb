# crypto: RSA signatures (blocks RS* and PS* in jwt)

## The issue

`jwt` cannot verify or sign RS256, RS384, RS512, PS256, PS384 or PS512 tokens,
or a JWK with `kty: "RSA"`. Each is refused with `ErrUnsupported` (`77050007`).
The cause is below `jwt`: the `crypto` builtin has no RSA primitive.

Evidence, from the tree:

- `src/docs/spec/stdlib/10_crypto.md` lists the public-key surface: Ed25519 and
  Ed448 (RFC 8032) and ECDSA over P-256, P-384 and P-521. No RSA key type, no
  RSA signature, no RSA encryption. `grep -n -i rsa` on that file returns nothing.
- `Certificate` members in that spec are `Ed25519`, `Ed448`, `X25519` and `X448`
  only.
- `packages/jwt/src/key.mfb` (`kty = "RSA"` branch) and the `lib.mfb` alg
  table refuse RSA explicitly.

## Why it matters

Most OpenID Providers sign ID tokens with RS256 by default. The published
`jwt` 1.0.0 therefore cannot verify tokens from those providers. That is a
real interop gap for an OIDC consumer, not a corner case. (Provider defaults
are widely reported; this plan does not measure them.)

The gap is in `crypto`, so no change inside `jwt` can close it. `jwt` already
has the parts that RSA needs: strict Base64url, the JWS signing input, the
header and allowlist policy, and the JWK import/export code. Only the
primitive is missing.

## Constraint: do not write bignum in source

The spec explains why NIST-EC does not use a software core. Generic bignum
arithmetic is roughly 100x slower than Ed25519's special-prime field, and it
is impractical at RSA sizes. NIST-EC therefore binds the platform key API:

- macOS: `SecKey` (Security.framework), non-deprecated create, sign, verify.
- Linux: `EVP_PKEY` in libcrypto, keys exchanged as DER through
  `d2i_AutoPrivateKey` / `d2i_PUBKEY`, `EVP_DigestSign` / `EVP_DigestVerify`.

Both platform APIs already implement RSA. So RSA should follow the same route:
bind the platform's RSA, do not implement modular exponentiation in MFBASIC.
This is a design choice for the plan to confirm, not a settled fact. Windows is
not listed in the spec's binding section, so Windows support needs its own
check.

## What has to exist

Phase 1, the primitive, in `crypto`:

1. `Certificate` gains RSA member(s). Key size must be a parameter (2048 is
   the floor; 3072 and 4096 are the usual choices). Decide whether `generate`
   takes a bit size or a fixed size per member.
2. `crypto::sign` and `crypto::verify` support the RSASSA-PKCS1-v1_5 and
   RSASSA-PSS schemes, with SHA-256, SHA-384 and SHA-512. The two schemes
   are different primitives and need separate names. JWS needs both: RS* is
   PKCS1-v1_5, PS* is PSS with MGF1 and salt length equal to the digest length.
3. Verification rejects a modulus below the floor, a signature longer than the
   modulus, and a signature representative at or above the modulus. These
   are the cases that a bad binding tends to miss.
4. The DER/PEM or JWK-shaped (`n`, `e`, `d`, `p`, `q`, ...) import and export
   needed by `jwt` and by `KeyPair` code. Decide whether `crypto` owns the
   JWK codec or `jwt` keeps it, since `jwt` already has one.

Phase 2, the policy, in `jwt`:

5. Lift the `ErrUnsupported` refusal for the six RS/PS algorithms and the
   `kty: "RSA"` branch. Keep the allowlist requirement unchanged.
6. Keep the minimum RSA key size enforced at key construction, not only at
   verify, matching the existing key-pair consistency check.
7. Oracle: extend `packages/jwt/oracle/` to cross-verify RS and PS tokens
   against panva/jose in both directions, as it does for ES and EdDSA.
8. Docs and CHANGELOG: this is additive, so a minor version (1.1.0), not a
   breaking change. Tokens that verified before still verify. The README's
   "What it does not sign" section and the `lib.mfb` DESC both need rewriting.

## Open decisions

- Key sizes: floor of 2048 only, or also refuse 2048 under a policy knob?
- Schemes: include PSS (PS*) in the first release, or RS* only? PS* is
  preferred by new deployments and is required by some providers.
- Interface: one `Certificate` member per (scheme, hash) pair, or a single RSA
  member plus a scheme enum on `sign`? The scheme must not be inferred from
  the key.
- Windows: is it in scope? If not, `crypto` must refuse RSA on Windows with a
  clear `ErrUnsupported`, not fail silently.

## Acceptance

- `crypto` round-trips RSASSA-PKCS1-v1_5 and RSASSA-PSS against known-answer
  vectors (RFC 8017 test vectors or the NIST SigVer/SigGen set), and rejects
  the malformed cases listed in phase 1, item 3.
- `mfb test packages/jwt` passes with the RS/PS round trips and the refusals
  that remain (unknown `kty`, undersized keys).
- `packages/jwt/oracle/run.sh` reports 0 failures for RS and PS against
  panva/jose.
- A JWT from a real OIDC provider's JWKS verifies. This is the end-to-end test
  that the unit vectors cannot provide.

## Not in scope

- Encryption (RSA-OAEP, JWE). `jwt` does not implement JWE.
- Key generation for RSA in the `jwt` package API beyond what `crypto` exposes.
