# Signing and Trust

How publisher identity, package signatures, and the local-vs-repository key match are established. All asymmetric crypto is Ed25519 over a SHA-256 fingerprint scheme; the wire and metadata encodings live in `mfb_repository::crypto`. [[repository/src/crypto.rs:sign]]

## The four keys

The trust model holds exactly four Ed25519 keypairs, distinguished by where the
private half lives (plan-23). Whoever holds a private key can forge everything
that key vouches for, so each key's storage *is* its authority boundary.

| Key | How many | Private key lives | Job |
| --- | --- | --- | --- |
| **server key** | 1 per registry | on the server (the only private key it holds) | signs attestations |
| **ident** | 1 per account | on every linked machine (copied at link time) | *is* the user's identity; signs proofs |
| **auth** | 1 per machine | that machine | logs into the registry API; nothing more |
| **signing** | 1 per package, one-off | for the duration of one build, then discarded | signs the `.mfp` |

What each key must **not** be able to do:

* **auth** cannot sign proofs or packages — a stolen login can request
  attestations (logged) but can never produce a package that verifies.
* **server key** signs attestations only — it can never produce a proof, so it
  can never impersonate a user to a consumer who has pinned that user.
* **ident** never signs packages directly — only proofs; the per-package key
  does the byte-signing so the ident signature surface stays tiny.
* **signing (one-off)** has no standing power — it dies at the end of the build.

Forging a package therefore requires **two independent credentials**: the ident
private key (to sign the proof) *and* a live authenticated session (to obtain
the attestation). Either alone is useless. The server holds no user private
keys: a full server compromise yields zero user keys.
[[repository/src/store.rs:register_owner]][[repository/src/store.rs:server_keypair]]

## Fingerprints and encodings

| Concept | Meaning | Encoding (metadata form) |
| --- | --- | --- |
| ident key | Publisher identity public key | `ed25519:` + URL-safe base64 (no pad) of 32-byte public key |
| signing key | Key verifying a package signature | `ed25519:` + URL-safe base64 of 32-byte public key |
| fingerprint | Key identifier | lowercase hex of `SHA-256(public_key)` |

A keypair is 32-byte public + 32-byte private; a signature is 64 bytes. [[repository/src/crypto.rs:PUBLIC_KEY_LEN]] Keys are generated from OS entropy. [[repository/src/crypto.rs:generate_keypair]] The public key can always be re-derived from the private key, which is how the build path checks a local key against the repository. [[repository/src/crypto.rs:public_from_private]]

The fingerprint is `hex(SHA-256(public_key))` — the public key bytes hashed directly, no domain prefix. [[repository/src/crypto.rs:fingerprint]]

Raw key/signature/nonce bytes are carried on the wire as URL-safe base64 without padding. [[repository/src/crypto.rs:encode_bytes]]

## Signing-domain byte strings

Two off-package proofs use length-free, NUL-delimited domain-separated messages. The literals are exact (verified against source); `\0` is a single NUL byte (`0x00`).

Registration message — proves control of a freshly generated keypair when
claiming an owner name. The key's **role** (`auth` or `ident`) is inside the
signed bytes, so a proof minted for one role can never be replayed as the
other:

```text
"mfb-repo-register-v1\0" || role || "\0" || owner || "\0" || publicKey
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
| register | `role` | ASCII `auth` or `ident` |
| register | separator | 1 NUL |
| register | `owner` | UTF-8 owner name bytes |
| register | separator | 1 NUL |
| register | `publicKey` | 32 raw bytes |
| challenge | domain | ASCII `mfb-repo-auth-v1` + 1 NUL |
| challenge | `challengeId` | UTF-8 challenge id bytes |
| challenge | separator | 1 NUL |
| challenge | `nonce` | server-issued raw nonce bytes |

The domain prefix plus the embedded role-specific separator prevent a signature minted for one purpose from being replayed as the other, and prevent either from being replayed as any other signature in the system. The full set of signing domains:

| Domain (ASCII, `\0` = NUL) | Signer | Signs |
| --- | --- | --- |
| `mfb-repo-register-v1\0` | auth / ident key | registration proof-of-possession (role inside the bytes) |
| `mfb-repo-auth-v1\0` | auth key | login challenge |
| `mfb-repo-revoke-v1\0` | ident key | auth-key revocation (challenge + fingerprint) |
| `mfb-repo-ident-rotate-v1\0` | OLD ident key | the rotation chain link naming the successor |
| `mfb-repo-name-binding-v1\0` | server key | the `/index` name→ident binding (owner + ident fingerprint) |
| `MFP-PROOF-v1\0` | ident key | the build proof JSON |
| `MFP-ATTEST-v1\0` | server key | the attestation JSON |
| `MFP-PACKAGE-v2\0` | one-off signing key | `SHA-256(header signed prefix)` — see container-format |

[[repository/src/crypto.rs:proof_signing_input]]

## The proof and the attestation

Every signed build carries two JSON statements (plan-23 §5), both pinning the
**exact** package (`ident` + `version`) and the exact one-off signing key, so a
leaked one-off key plus its paperwork is worth exactly one already-published
package — nothing. Neither carries an expiry: they are notarized statements of
fact at `issued`, true forever; freshness is enforced live at publish time,
never by a clock inside a shipped file.

```text
Proof (ident-signed, minted locally at build time):
{ "owner": "alice",
  "ident": "alice#toolbox",
  "version": "1.2.3",
  "identFingerprint": "<hex sha256 of identKey>",
  "signingFingerprint": "<hex sha256 of signingKey>",
  "issued": <UTC unix seconds> }

Attestation (server-signed, fetched per build via POST /signing):
{ "repoFingerprint": "<hex sha256 of server public key>",
  "owner": "alice",
  "ident": "alice#toolbox",
  "version": "1.2.3",
  "identFingerprint": "<hex sha256 of identKey>",
  "signingFingerprint": "<hex sha256 of signingKey>",
  "issued": <UTC unix seconds> }
```

The signed bytes are the exact JSON strings as produced (fixed field order, no
re-serialization); verifiers compare fields after parsing but verify the
signature over the raw stored bytes.
[[src/cli/build.rs:load_build_signing_info]][[repository/src/server.rs:attestation_json]]

### Registration flow

`register` generates the auth and ident keypairs locally, builds one
role-discriminated `registration_message` per key, signs each with its own
private key, and `POST`s `{owner, authKey, identKey, proofs:{auth,ident}}` to
`/accounts/register`; the keypairs are written locally first and removed again
if the server rejects the request. [[repository/src/client.rs:register]] The
server decodes both keys and proofs and verifies each proof against its own
role's message before recording the owner. [[repository/src/server.rs:register]]

### Challenge flow

`auth` reads the local **auth** private key, derives the public key and its fingerprint, requests a challenge from `/auth/challenge`, signs `challenge_message(challengeId, nonce)`, and posts the signature to `/auth/login` to obtain a session token. [[repository/src/client.rs:auth]]

## `build --sign`: the per-build signing flow

`mfb build --sign <owner>` assembles the plan-23 §3.3 signing bundle through
`load_build_signing_info`, which is only honored for package and executable
builds (validate output); other outputs error. The server must be reachable —
every signed build fetches a fresh attestation. [[src/cli/build.rs:load_build_signing_info]]

1. Fix the signed identity: the manifest `ident` when declared (it must be
   `<owner>#<package>` and belong to the signing owner), else the canonical
   `<owner>#<name>`; the version comes from the validated manifest.
   [[src/cli/build.rs:signing_ident]]
2. Read the local **ident** keypair (`<owner>.ident.{prv,pub}`; register or
   link this machine first) and cross-check the pair.
3. Generate the **one-off signing keypair** — fresh for this build.
4. `POST /signing` with `{owner, ident, version, signingFingerprint}` (see
   repository-protocol). The client verifies the returned attestation
   signature against the pinned `server.pub` and cross-checks that the
   attestation pins the requested ident/version/signing fingerprint and that
   its `identFingerprint` names the ident key this machine holds.
   [[repository/src/client.rs:request_attestation]]
5. Mint the **proof** locally (see above) and sign it with the ident key
   (`MFP-PROOF-v1` domain).
6. Thread the bundle (`PackageSigning { ident_key, signing_key,
   signing_private, proof, proof_sig, attestation, attestation_sig }`) to the
   package writer, which emits the container v1.0 header and makes the prefix
   signature with the one-off key. The one-off private key exists only in
   memory for the duration of the build and is **discarded** with it — it is
   never written to disk. [[src/target/package_mfp/mod.rs:PackageSigning]]

For **package** builds the identity fields are stamped into the
binary-representation metadata via `apply_signing_metadata` (sets `ident`,
`ident_key`, `ident_fingerprint`, `signing_fingerprint`, `author = owner`) so
the embedded manifest repeats the header identity. [[src/cli/build.rs:apply_signing_metadata]]
For **executable** builds the JSON blob below is embedded instead.

## Executable signing metadata (`mfb-signing-v1`)

Executable builds embed a single-line JSON object describing the signer,
including the full proof and attestation so the embedded claim is verifiable.
Field order and the trailing newline are fixed by the formatter; string values
are JSON-escaped. [[src/cli/build.rs:executable_signing_metadata_json]]

```json
{"format":"mfb-signing-v1","owner":"<owner>","author":"<owner>","identKey":"ed25519:<base64>","identFingerprint":"<hex>","signingKey":"ed25519:<base64>","signingFingerprint":"<hex>","proof":"<proof JSON>","proofSignature":"<base64url>","attestation":"<attestation JSON>","attestationSignature":"<base64url>","signatureType":"Ed25519"}
```

| Field | Value |
| --- | --- |
| `format` | constant `mfb-signing-v1` |
| `owner` | the signing owner name |
| `author` | same as `owner` |
| `identKey` | `ed25519:` + base64 ident public key |
| `identFingerprint` | hex SHA-256 fingerprint of the ident key |
| `signingKey` | `ed25519:` + base64 one-off signing public key |
| `signingFingerprint` | hex SHA-256 fingerprint of the one-off signing key |
| `proof` | the ident-signed proof JSON |
| `proofSignature` | base64url 64-byte ident signature over the proof |
| `attestation` | the server-signed attestation JSON |
| `attestationSignature` | base64url 64-byte server signature over the attestation |
| `signatureType` | constant `Ed25519` |

The blob is UTF-8 bytes (`.into_bytes()`) and threaded to `target::write_executable` as the executable signing metadata. [[src/cli/build.rs:load_build_signing_info]]

## Trust boundary

The binary-representation reader does **not** verify the trust chain at import
time — it only checks header/manifest identity agreement and structural
sanity. Chain verification (pinned server key → attestation → pinned ident →
proof → one-off key → bytes) is the package manager's responsibility at
build/verify time; see `./mfb spec package verifier-rules`.
[[src/cli/build.rs:classify_installed_package]]

## See Also

* ./mfb spec package container-format — the container v1.0 header, `"MFP-PACKAGE-v2\0"` prefix signature, and `packageBinaryHash` weld
* ./mfb spec package verifier-rules — the build-time verification chain
* ./mfb spec package-manager repository-protocol — the `/accounts/register`, `/auth/challenge`, `/auth/login`, and `/signing` endpoints
* ./mfb spec package-manager key-store — where the keypairs, pinned server key, and session token are stored on disk
* ./mfb spec package-manager owner-names — owner-name validation rules used before signing
* ./mfb spec tooling cli-reference — `mfb build --sign`, `mfb repo register`, and `mfb repo auth`
