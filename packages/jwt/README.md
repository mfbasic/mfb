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

MUT claims AS Map OF String TO json::Json = Map OF String TO json::Json {}
claims = collections::set(claims, "iss", json::JsonStr["https://issuer.example"])
claims = collections::set(claims, "aud", json::JsonStr["https://api.example"])
claims = collections::set(claims, "sub", json::JsonStr["user-1"])
claims = collections::set(claims, "exp", jwt::numericDate(datetime::now().seconds + 3600))
LET token AS String = jwt::sign(key, json::JsonObj[claims])

LET choices AS jwt::Options = WITH jwt::options(["EdDSA"]) { issuer := "https://issuer.example", audience := "https://api.example", requiredClaims := ["exp"], leewaySeconds := 30 }
LET checked AS jwt::Token = jwt::verify(token, [jwt::publicKeyOf(key)], choices)
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
`errorCode::ErrUnsupported` rather than being misread. Most OpenID Providers
still sign with RSA, so this package cannot verify their tokens yet; the missing
piece is RSA in `crypto`, not here.

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

* **The encodings have one text.** Base64url must be canonical and unpadded
  (RFC 7515 §2): `=` padding, the standard `+`/`/` alphabet, and a final group
  whose leftover bits are not zero are all refused. A duplicate member name in a
  header or claims set is refused, because two readers need not keep the same
  one.
* **A signature may have more than one text.** ECDSA signatures are malleable:
  for one key and message, `(r, s)` and `(r, n−s)` both verify. `crypto` does not
  normalise to low-S, RFC 7518 does not require it, and mainstream JOSE libraries
  accept both, so `jwt::verify` does too. Ed25519 and Ed448 do not have this:
  `crypto` rejects a non-canonical scalar. Do not use a token's bytes, or its
  signature, as an identifier. Key a replay cache, revocation list or audit
  record on the `jti` claim (RFC 7519 §4.1.7), or on the claims themselves.
* **`crit` is refused.** RFC 7515 §4.1.11 requires a verifier to understand every
  parameter listed there or reject the token, and this package implements no
  extension.
* **An unsecured token is refused by its algorithm.** `alg: "none"` is
  `ErrorAlgorithm` whatever its signature segment holds. An empty signature on an
  algorithm you do allow is a format error.
* **An ECDSA signature is parsed strictly.** Every DER length and INTEGER is
  checked for minimal encoding, and `r`/`s` are range-checked against the curve
  order, because that parser reads bytes an attacker wrote.
* **The payload's JSON is parsed only after the signature verifies**, so a
  document nobody signed is never walked.
* **An HMAC is compared with `crypto::constantTimeEqual`**, never with `=`.
* **Messages are safe to log.** Token-controlled text in an error (an `alg`, a
  `kid`, a claim value) is printable ASCII and at most 64 characters.
* **A key is what it says it is.** A JWK or key pair whose private and public
  halves do not match is refused when the key is built, rather than producing
  tokens no published key verifies.

Two defaults are deliberately permissive, as they are in the mainstream
libraries, and a production deployment should set them:

* `issuer`, `audience` and `subject` are unchecked until you name them.
* A key with no `kid` is tried for every token, whatever `kid` the token names.
  Give every key a `kid` once you rotate keys.

`jwt::unverifiedHeader` and `jwt::unverifiedClaims` exist for a debugger, a log
formatter, or picking which key to fetch. Their names say what they are.

## Time

`exp`, `nbf` and `iat` are compared against `datetime::now()`, or against an
instant you pin:

```mfb
LET fixed AS jwt::Options = WITH jwt::options(["HS256"]) { fixedTime := TRUE, timeSeconds := 1700000000 }
```

which is what makes a test of an expiry rule reproducible. `leewaySeconds`
forgives clock skew, must be zero or more, and defaults to none.

A NumericDate is a whole number of seconds (RFC 7519 §2). Because
`json::JsonNum` holds a `Float`, a time claim carrying a fraction — or a
magnitude past the range binary64 represents exactly — is rejected rather than
rounded: a value that cannot be compared exactly against a clock cannot decide
whether a token has expired.

## Limits and performance

A token larger than 1 MiB (`MAX_TOKEN_BYTES`, 1048576 bytes, all three segments
together) is refused before it is parsed. Measured on the development machine
with a 933,428-byte token: `jwt::sign` about 120 ms and `jwt::verify` about 40 ms,
from `datetime::now()` around each call.

## Testing

```sh
mfb test packages/jwt                 # 115 tests: round trips, rejections, claims, DER strictness
packages/jwt/check-doc-examples.sh    # every EXAMPLE in the DOC blocks builds and runs
packages/jwt/oracle/run.sh            # the same questions, asked of panva/jose
```

The package's own tests pin what this package decided. They cannot catch a
misreading of RFC 7515/7518/7519, because the same misreading would be in the
code and the test — so `oracle/` asks an independent implementation the same
questions, cross-verifies every signature in both directions, and compares
deterministic tokens byte for byte. See `oracle/README.md`.

## Versioning

This is 1.0.0. The public surface is everything `jwt.mfp` exports. Behaviour
changes that make a token that verified stop verifying, or that change which
error a refused token raises, are breaking and wait for 2.0.0. See `CHANGELOG.md`.
