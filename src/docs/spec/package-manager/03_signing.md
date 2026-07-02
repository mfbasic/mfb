# Signing and Trust

How publisher identity, package signatures, and the local-vs-repository key match are established. All asymmetric crypto is Ed25519 over a SHA-256 fingerprint scheme; the wire and metadata encodings live in `mfb_repository::crypto`. [[repository/src/crypto.rs:sign]]

## Keys and fingerprints

The repository models two logical roles, the **ident key** and the **signing key**, even though the current implementation derives both from a single per-owner Ed25519 keypair. The ident key names the publisher (publisher identity); the signing key is the key whose private half signs `.mfp` packages.

| Concept | Meaning | Encoding (metadata form) |
| --- | --- | --- |
| ident key | Publisher identity public key | `ed25519:` + URL-safe base64 (no pad) of 32-byte public key |
| signing key | Key verifying a package signature | `ed25519:` + URL-safe base64 of 32-byte public key |
| fingerprint | Key identifier | lowercase hex of `SHA-256(public_key)` |

The `ed25519:`-prefixed form is the *metadata* encoding (package header,
manifest, executable signing blob); the `/keys/signing` wire response carries
the bare base64 key and the client prepends the prefix. [[src/cli/build.rs:load_build_signing_info]]

A keypair is 32-byte public + 32-byte private; a signature is 64 bytes. [[repository/src/crypto.rs:PUBLIC_KEY_LEN]] Keys are generated from OS entropy. [[repository/src/crypto.rs:generate_keypair]] The public key can always be re-derived from the private key, which is how the build path checks a local key against the repository. [[repository/src/crypto.rs:public_from_private]]

The fingerprint is `hex(SHA-256(public_key))` — the public key bytes hashed directly, no domain prefix. [[repository/src/crypto.rs:fingerprint]]

Raw key/signature/nonce bytes are carried on the wire as URL-safe base64 without padding. [[repository/src/crypto.rs:encode_bytes]]

In the current server, `/keys/signing` returns the owner's single auth public key for **both** `identKey`/`identFingerprint` and `signingKey`/`signingFingerprint`; the two roles are reported as equal. The split exists so signing can later be delegated to a distinct key without a protocol change. [[repository/src/server.rs:signing_info]]

## Signing-domain byte strings

Two off-package proofs use length-free, NUL-delimited domain-separated messages. The literals are exact (verified against source); `\0` is a single NUL byte (`0x00`).

Registration message — proves control of a freshly generated keypair when claiming an owner name:

```text
"mfb-repo-register-v1\0" || owner || "\0" || publicKey
```

[[repository/src/crypto.rs:registration_message]]

Challenge message — proves control of the private key when authenticating against a server-issued challenge:

```text
"mfb-repo-auth-v1\0" || challengeId || "\0" || nonce
```

[[repository/src/crypto.rs:challenge_message]]

| Field | Element | Type |
| --- | --- | --- |
| register | domain | ASCII `mfb-repo-register-v1` + 1 NUL |
| register | `owner` | UTF-8 owner name bytes |
| register | separator | 1 NUL |
| register | `publicKey` | 32 raw bytes |
| challenge | domain | ASCII `mfb-repo-auth-v1` + 1 NUL |
| challenge | `challengeId` | UTF-8 challenge id bytes |
| challenge | separator | 1 NUL |
| challenge | `nonce` | server-issued raw nonce bytes |

The domain prefix plus the embedded role-specific separator prevent a signature minted for one purpose from being replayed as the other, and prevent either from being replayed as a package signature (the package signature uses its own `"MFP-PACKAGE-v1"` domain — see container-format).

### Registration flow

`register` generates a keypair, builds `registration_message(owner, public)`, signs it, and `POST`s `{owner, authKey, proof}` to `/accounts/register`; the keypair is written locally first and removed again if the server rejects the request. [[repository/src/client.rs:register]] The server decodes `authKey`/`proof` and verifies the proof before recording the owner. [[repository/src/server.rs:register]]

### Challenge flow

`auth` reads the local private key, derives the public key and its fingerprint, requests a challenge from `/auth/challenge`, signs `challenge_message(challengeId, nonce)`, and posts the signature to `/auth/login` to obtain a session token. [[repository/src/client.rs:auth]]

## `build --sign`: local-key vs repository-key match

`mfb build --sign <owner>` resolves signing material through `load_build_signing_info`, which is only honored for package and executable builds (validate output); other outputs error. [[src/cli/build.rs:load_build_signing_info]]

The match is two-stage and both checks must pass:

1. Fetch the repository signing info for `owner` (`signing_info` → `/keys/signing`, session-authenticated). [[repository/src/client.rs:signing_info]]
2. Read the local private key, derive its public key, and decode the repository `signingKey`. If `localPublic != serverSigningPublic`, fail with `local private key does not match repository signing key`. [[src/cli/build.rs:load_build_signing_info]]
3. Compute `fingerprint(localPublic)`; if it differs from `signingFingerprint`, fail with `local private key fingerprint does not match repository signing key`.

On success it composes the `ed25519:`-prefixed `identKey`/`signingKey` strings, builds the executable-signing JSON, and returns a `BuildSigningInfo { owner, ident_key, ident_fingerprint, signing_fingerprint, private_key, executable_metadata }`. [[src/cli/build.rs:load_build_signing_info]]

For **package** builds the identity fields are stamped into the binary-representation metadata via `apply_signing_metadata` (sets `ident_key`, `ident_fingerprint`, `signing_fingerprint`, `author = owner`), and the loaded private key is passed to `write_package`, which produces the `.mfp` Ed25519 signature (`signatureType = 1`; see container-format). [[src/cli/build.rs:apply_signing_metadata]] For **executable** builds the JSON blob below is embedded instead.

## Executable signing metadata (`mfb-signing-v1`)

Executable builds embed a single-line JSON object describing the signer. Field order and the trailing newline are fixed by the formatter; string values are JSON-escaped. [[src/cli/build.rs:executable_signing_metadata_json]]

```json
{"format":"mfb-signing-v1","owner":"<owner>","author":"<owner>","identKey":"ed25519:<base64>","identFingerprint":"<hex>","signingKey":"ed25519:<base64>","signingFingerprint":"<hex>","signatureType":"Ed25519"}
```

| Field | Value |
| --- | --- |
| `format` | constant `mfb-signing-v1` |
| `owner` | owner name returned by `/keys/signing` |
| `author` | same as `owner` |
| `identKey` | `ed25519:` + base64 ident public key |
| `identFingerprint` | hex SHA-256 fingerprint of the ident key |
| `signingKey` | `ed25519:` + base64 signing public key |
| `signingFingerprint` | hex SHA-256 fingerprint of the signing key |
| `signatureType` | constant `Ed25519` |

The blob is UTF-8 bytes (`.into_bytes()`) and threaded to `target::write_executable` as the executable signing metadata. [[src/cli/build.rs:load_build_signing_info]]

## Trust boundary

The binary-representation reader does **not** verify the cryptographic signature at import time — it only checks header/manifest identity agreement and signature-length sanity. Signature verification against a trusted key is the package manager's responsibility at install/resolve time, using `mfb_repository::crypto::verify`. [[repository/src/crypto.rs:verify]] The `.mfp` header `signingFingerprint` names the key expected to verify the package signature; the embedded signed manifest must carry the same identity (see container-format).

## See Also

* ./mfb spec package container-format — the `.mfp` Ed25519 signature header, `"MFP-PACKAGE-v1"` signature input, and content-hash coverage
* ./mfb spec package-manager repository-protocol — the `/accounts/register`, `/auth/challenge`, `/auth/login`, and `/keys/signing` endpoints
* ./mfb spec package-manager key-store — where the local keypair and session token are stored on disk
* ./mfb spec package-manager owner-names — owner-name validation rules used before signing
* ./mfb spec tooling project-manifest — `identKey`, `identFingerprint`, and `signingFingerprint` fields in the manifest
* ./mfb spec tooling cli-reference — `mfb build --sign`, `mfb repo register`, and `mfb repo auth`
