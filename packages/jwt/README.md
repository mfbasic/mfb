# jwt — sign and verify JSON Web Tokens

`jwt::sign` turns a claims set into a token; `jwt::verify` turns a token back
into its claims, after checking the signature against a key you supplied and an
algorithm you named.

```mfb
IMPORT jwt
IMPORT json
IMPORT collections
IMPORT datetime

LET key AS jwt::Key = jwt::generate("EdDSA", "Ed25519")

LET subject AS json::Json = json::JsonStr["user-1"]
MUT claims AS Map OF String TO json::Json = Map OF String TO json::Json {}
claims = collections::set(claims, "sub", subject)
claims = collections::set(claims, "exp", jwt::numericDate(datetime::now().seconds + 3600))
LET token AS String = jwt::sign(key, json::JsonObj[claims])

LET checked AS jwt::Token = jwt::verify(token, [key], jwt::options(["EdDSA"]))
```

Each value is widened to `json::Json` on the way into the map — the six member
types are records and the map holds the union over them — and
`jwt::numericDate` turns whole seconds into the JSON number a time claim holds.
A fixed claims set is shorter written as JSON: `json::parse("{\"sub\":\"a\"}")`.

Claims are an ordinary `json::Json` value, so a payload is the same tree
`json::parse` produces and `json::stringify` serializes, and anything that
already consumes JSON consumes a verified payload unchanged. Because the model
is `json::Json`, an importer writes `IMPORT json` alongside `IMPORT jwt` to name
the type. (Imports are not transitive.)

Full API and prose: `mfb pkg doc packages/jwt/jwt.mfp`, or `mfb doc packages/jwt`
for the internals too.

## What it signs

| `alg` | Primitive | Key |
| --- | --- | --- |
| `HS256`, `HS384`, `HS512` | HMAC-SHA-2 (RFC 2104) | a shared secret, at least the digest length |
| `EdDSA` | RFC 8032 PureEdDSA | Ed25519 **or** Ed448 |
| `ES256`, `ES384`, `ES512` | FIPS 186-4 ECDSA | P-256, P-384, P-521 |

These are the algorithms `crypto` has primitives for. `EdDSA` names two curves
rather than one, and RFC 8037 puts the difference in the **key's** `crv` rather
than in the token's header — so a `jwt::Key` always carries its own curve, and
nothing here reads a curve off a token.

## What it does not sign

`RS256`, `RS384`, `RS512`, `PS256`, `PS384` and `PS512` need RSA, and `crypto`
has no RSA primitive. A token using one, or a JWK with `kty: "RSA"`, fails with
`errorCode::ErrUnsupported` rather than being misread. If you need to verify
tokens from an issuer that signs with RSA — which most OpenID Providers still do
— this package cannot do it yet, and the missing piece is in `crypto` rather than
here.

`alg: "none"` is never signed and never verified. An unsecured JWS carries no
signature, so there is nothing about it to check.

## Why this is a package and not a built-in

`crypto` owns primitives: a hash, a MAC, a signature over bytes. JWT is a
**policy** over those primitives — which algorithms to accept, what a duplicate
member name means, whether a non-canonical Base64url segment is a token, how much
clock skew to forgive. Every one of those is a decision that a deployment may
need to revisit, and a package can evolve them on its own schedule instead of
tying them to a compiler release.

The three seams that had to be written rather than called are the argument for
the split:

* **Strict Base64url.** `encoding::base64UrlDecode` accepts `=` padding and
  discards a final group's leftover bits — right for a general codec, wrong for a
  JWS, where the signature covers the segment *text*.
* **DER ↔ the fixed-width `R ‖ S`.** `crypto::sign` returns an ASN.1
  `Ecdsa-Sig-Value` and RFC 7518 §3.4 wants two fixed-width coordinates.
* **JWK import and export.** `crypto` has no key import, but its SEC1 form is
  exactly a JWK's `x`, `y` and `d` concatenated.

None of the three belongs in a primitive package, and all three are the kind of
thing a JWT implementation must get exactly right.

## The security posture

The verifier takes its algorithm **from you**, never from the token.
`jwt::options` requires an allowlist and has no default:

```mfb
LET choices AS jwt::Options = WITH jwt::options(["ES256"]) { issuer := "https://issuer.example", requiredClaims := ["exp"] }
```

That one requirement closes the two best-known JWT vulnerabilities. A token
naming `none` verifies with no key at all; a token naming `HS256` where the
deployment expects `ES256` can be verified as an HMAC keyed by the public key it
was supposed to be checked *against*. Naming what you accept, once, refuses both
— and so does the fact that a `jwt::Key` carries its own algorithm, so an
`ES256` public key is never tried as an `HS256` secret.

On top of that:

* **A token has exactly one text.** Base64url must be canonical and unpadded
  (RFC 7515 §2): `=` padding, the standard `+`/`/` alphabet, and a final group
  whose leftover bits are not zero are all refused. Where a byte string has
  several legal spellings, anything keyed on the text — a replay cache, a
  revocation list, an audit log — can be desynchronised from anything keyed on
  the claims.
* **A duplicate member name is refused.** JSON does not forbid
  `{"alg":"HS256","alg":"none"}`, and a parser that keeps one member cannot
  report the other — but two readers need not keep the same one.
* **`crit` is refused.** RFC 7515 §4.1.11 requires a verifier to understand every
  parameter listed there or reject the token, and this package implements no
  extension, so the honest answer is always to reject.
* **An ECDSA signature is parsed strictly.** Every DER length and INTEGER is
  checked for minimal encoding, and `r`/`s` are range-checked against the curve
  order, because that parser reads bytes an attacker wrote.
* **The payload's JSON is parsed only after the signature verifies**, so a
  document nobody signed is never walked.
* **An HMAC is compared with `crypto::constantTimeEqual`**, never with `=`.

`jwt::unverifiedHeader` and `jwt::unverifiedClaims` exist for a debugger, a log
formatter, or picking which key to fetch. Their names say what they are.

## Time

`exp`, `nbf` and `iat` are compared against `datetime::now()`, or against an
instant you pin:

```mfb
LET fixed AS jwt::Options = WITH jwt::options(["HS256"]) { fixedTime := TRUE, timeSeconds := 1700000000 }
```

which is what makes a test of an expiry rule reproducible. `leewaySeconds`
forgives clock skew and defaults to none.

A NumericDate is a whole number of seconds (RFC 7519 §2). Because
`json::JsonNum` holds a `Float`, a time claim carrying a fraction — or a
magnitude past the range binary64 represents exactly — is rejected rather than
rounded: a value that cannot be compared exactly against a clock cannot decide
whether a token has expired.

## Testing

```sh
mfb test packages/jwt          # 102 tests: round trips, rejections, claims, DER strictness
packages/jwt/oracle/run.sh     # the same questions, asked of panva/jose
```

The package's own tests pin what this package decided. They cannot catch a
misreading of RFC 7515/7518/7519, because the same misreading would be in the
code and the test — so `oracle/` asks an independent implementation the same
questions, cross-verifies every signature in both directions, and compares
deterministic tokens byte for byte. See `oracle/README.md`.
