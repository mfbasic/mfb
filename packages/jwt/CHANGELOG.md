# Changelog

All notable changes to the `jwt` package. The package follows
[Semantic Versioning](https://semver.org/): from 1.0.0 on, a breaking change to
the exported API, or to which tokens verify and which error a refused token
raises, requires a new major version.

## 1.0.0

The first stable release. It supersedes the unreleased 0.1.0 working version. The
one exported name removed is `MAX_SEGMENT_BYTES`, which the segment-size change
below retired. The decisions below were made in the course of hardening the
package for release.

### Changed

- `jwt::verify` checks its `Options` on every call, not only when
  `jwt::options` builds them. A record update can set any field to anything, so
  an allowlist that is empty, names `none` or an algorithm this package does
  not implement, or a negative `leewaySeconds` or `maxAgeSeconds` is refused
  with `77050002`, `93130004` or `77050007`.
- An unsecured token (`alg: "none"`, with an empty third segment) is refused
  with `93130004` (`ErrorAlgorithm`), as the `verify` doc has always said. It was
  `77050003` (`ErrorFormat`). The signature segment is now checked after the
  algorithm, so the algorithm decides the error.
- `jwt::importJwkSet` skips a key that is not a signature key by its own `use` or
  `key_ops` (`use: "enc"`, or `key_ops` naming neither `sign` nor `verify`), as
  its doc says. It failed the whole set before. Malformed keys still fail.
- Error messages that echo token or JWK text (`alg`, `kid`, `typ`, claim values,
  `kty`, `crv`) show printable ASCII only, at most 64 characters, through the
  new internal `shown`.
- The exported `MAX_SEGMENT_BYTES` (262144) is removed. A segment is no longer
  limited on its own. The 1048576-byte limit on the
  whole token is the only size limit, and it bounds every segment. The old
  per-segment limit refused payloads of about 192 KB, well inside the whole-token
  limit.
- `jwt::signWithHeader` refuses a `typ` or `kid` that is not a string.
- `jwt::unverifiedHeader` and `jwt::unverifiedClaims` read a token with an empty
  signature segment, as they always read the other segments.

### Added

- The key constructors (`jwt::keyFromPair`, `jwt::importJwk`, `jwt::generate`)
  refuse a key whose private and public halves do not match. A mismatched JWK used
  to sign tokens that no published key verified.
- `check-doc-examples.sh` in the package directory builds and runs every
  `EXAMPLE` in the `DOC` blocks.
- Tests for each rule above, including the ECDSA high-S twin (which verifies),
  an Ed25519 `S + L` twin (which does not), an off-curve NIST point, a whole
  token just past and just under the size limit, and the JWK Set skips.

### Documented

- ECDSA signatures are malleable: `(r, s)` and `(r, n−s)` both verify, as they do
  in `crypto` and in the mainstream JOSE libraries. The package does not
  normalise them. Key replay caches and revocation lists on `jti` or the claims,
  not on token bytes. Ed25519 and Ed448 have one signature text.
- `issuer`, `audience` and `subject` are unchecked by default, and a key with no
  `kid` is tried for every token, as the mainstream libraries do.
- RS256, RS384, RS512, PS256, PS384 and PS512 are not supported. `crypto` has no
  RSA primitive.
