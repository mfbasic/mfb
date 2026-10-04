# Signing and Trust

How publisher identity, package signatures, and the local-vs-repository key match are established. All asymmetric crypto is Ed25519 over a SHA-256 fingerprint scheme; the wire and metadata encodings live in `mfb_repository::crypto`. [[repository/src/crypto.rs:sign]]

## Key roles

Publisher signing uses these Ed25519 roles:

| Key | Scope | Private key storage | Signed statements |
| --- | --- | --- | --- |
| server | registry | registry database | attestations, name bindings, log checkpoints |
| ident | account | linked machines | build proofs, account mutations, rotation/revocation authority |
| auth | machine/account credential | machine | registration and login challenges |
| signing | build | build memory | package or executable content signature |

The server verifies that auth and ident public keys remain distinct when
registering, linking, or rotating. It stores publisher public keys, not their
plaintext private keys; the machine-link relay holds encrypted ident material.
[[repository/src/store.rs:register_owner]][[repository/src/store.rs:take_pairing_blob]][[repository/src/store.rs:rotate_ident]]

The optional signed-metadata layer adds an offline root key plus online snapshot
and timestamp keypairs. The operator receives the root private key during the
ceremony; the database persists the online private keys and signed root
metadata, not the root private key.
[[repository/src/store.rs:init_registry_root]][[repository/src/store.rs:renew_registry_root]]

A machine auth credential can obtain attestations but cannot make an ident
proof. The ident key can make proofs but does not itself grant an authenticated
registry session. Package verification requires the proof, attestation, and
content signature to agree. Their reusable scope is the exact ident, version,
and signing key: possession of that signing private key and valid statements
can sign different bytes for that same identity/version. The statements do not
contain a content hash; publication separately rejects an already-published
version. [[repository/src/server.rs:attestation_json]][[src/cli/build/signing.rs:load_build_signing_info]][[repository/src/server.rs:validate_package_request]]

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

The domain prefix plus the embedded role-specific separator prevent a signature minted for one purpose from being replayed as the other, and prevent either from being replayed as any other signature in the system. Signing domains include:

| Domain (ASCII, `\0` = NUL) | Signer | Signs |
| --- | --- | --- |
| `mfb-repo-register-v1\0` | auth / ident key | registration proof-of-possession (role inside the bytes) |
| `mfb-repo-auth-v1\0` | auth key | login challenge |
| `mfb-repo-revoke-v1\0` | ident key | auth-key revocation (challenge + fingerprint) |
| `mfb-repo-ident-rotate-v1\0` | OLD ident key | the rotation chain link naming the successor |
| `mfb-repo-name-binding-v1\0` | server key | the `/index` name→ident binding (owner + ident fingerprint) |
| `mfb-repo-release-state-v1\0` | ident key | a `/release-state` change (ident + version + new state) |
| `mfb-repo-root-v1\0` | offline root key | `root.json` delegating the server/snapshot/timestamp keys |
| `mfb-repo-snapshot-v1\0` | snapshot key | `snapshot.json` (the index state) |
| `mfb-repo-timestamp-v1\0` | timestamp key | `timestamp.json` (the current snapshot pointer) |
| `mfb-repo-org-role-v1\0` | member ident key | an org role grant/removal (org + member + role) |
| `mfb-repo-token-issue-v1\0` | ident key | a publish-token issuance (owner + token fingerprint + scope) |
| `mfb-repo-token-revoke-v1\0` | ident key | a publish-token revocation (owner + token fingerprint) |
| `mfb-repo-transfer-offer-v2\0` | current-owner ident | an ownership-transfer offer (ident + sequence + from owner + from ident fingerprint + to owner + release digest) |
| `mfb-repo-transfer-accept-v2\0` | recipient ident | an ownership-transfer acceptance (the same six terms as the offer) |
| `MFP-PROOF-v1\0` | ident key | the build proof JSON |
| `MFP-ATTEST-v1\0` | server key | the attestation JSON |
| `MFP-PACKAGE-v2\0` | one-off signing key | `SHA-256(header signed prefix)` — see container-format |
| `MFB-EXECUTABLE-v1\0` | one-off signing key | `SHA-256(executable image with the .mfbsign body zeroed)` — see the content signature below |

[[repository/src/crypto.rs:proof_signing_input]]

## The proof and the attestation

Every signed build carries two JSON statements, both pinning the
**exact** package identity (`ident` + `version`) and the exact one-off signing
key. They do not independently bind the artifact bytes. Neither carries an expiry: they are notarized statements of
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
[[src/cli/build/signing.rs:load_build_signing_info]][[repository/src/server.rs:attestation_json]]

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

`mfb build --sign <owner>` assembles the signing bundle through
`load_build_signing_info`, which is only honored for package and executable
builds with no intermediate-output selectors; other outputs error.
[[src/cli/build/mod.rs:build_project]] The server must be reachable —
every signed build fetches a fresh attestation. [[src/cli/build/signing.rs:load_build_signing_info]]

1. Fix the signed identity: the manifest `ident` when declared (it must be
   `<owner>#<package>`, and is used as-is — the registry issues the attestation
   only to the package's stored owner or, for a new ident, its prefix owner, so
   a transfer's recipient signs the transferred ident), else the canonical
   `<owner>#<name>`; the version comes from the validated manifest.
   [[src/cli/build/signing.rs:signing_ident]]
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
the embedded manifest repeats the header identity. [[src/cli/build/signing.rs:apply_signing_metadata]]
For **executable** builds the JSON blob below is embedded instead.

## Executable signing metadata (`mfb-signing-v1`)

Executable builds embed a single-line JSON object describing the signer,
including the full proof and attestation so the embedded claim is verifiable.
Field order and the trailing newline are fixed by the formatter; string values
are JSON-escaped. [[src/cli/build/signing.rs:executable_signing_metadata_json]]

```json
{"format":"mfb-signing-v1","owner":"<owner>","author":"<owner>","registry":"<registry URL>","identKey":"ed25519:<base64>","identFingerprint":"<hex>","signingKey":"ed25519:<base64>","signingFingerprint":"<hex>","proof":"<proof JSON>","proofSignature":"<base64url>","attestation":"<attestation JSON>","attestationSignature":"<base64url>","contentSignature":"<base64url>","signatureType":"Ed25519"}
```

| Field | Value |
| --- | --- |
| `format` | constant `mfb-signing-v1` |
| `owner` | the signing owner name |
| `author` | same as `owner` |
| `registry` | the URL of the registry that issued the attestation (the build's `MFB_REPO_URL`, else the default registry) |
| `identKey` | `ed25519:` + base64 ident public key |
| `identFingerprint` | hex SHA-256 fingerprint of the ident key |
| `signingKey` | `ed25519:` + base64 one-off signing public key |
| `signingFingerprint` | hex SHA-256 fingerprint of the one-off signing key |
| `proof` | the ident-signed proof JSON |
| `proofSignature` | base64url 64-byte ident signature over the proof |
| `attestation` | the server-signed attestation JSON |
| `attestationSignature` | base64url 64-byte server signature over the attestation |
| `contentSignature` | base64url 64-byte one-off signing key signature over the executable's contents (below) |
| `signatureType` | constant `Ed25519` |

The blob is UTF-8 bytes (`.into_bytes()`) and threaded to `target::write_executable`, together with the one-off signing private key, as the executable signing metadata. [[src/cli/build/signing.rs:load_build_signing_info]][[src/arch/image.rs:ExecutableSigning]]

### The content signature

`contentSignature` binds the blob to the file that carries it. The one-off
signing key signs

```text
"MFB-EXECUTABLE-v1\0" || SHA-256(covered bytes)
```

where the covered bytes are the executable image from offset 0 up to the covered
end, with the `.mfbsign` section body read as zeros. The covered end is the file
length on ELF and PE; on Mach-O it is the `LC_CODE_SIGNATURE` data offset, since
the ad-hoc code signature is computed after the seal. Everything else — headers,
entry point, load commands, interpreter, import tables, code, data, the
provenance note, and padding — is covered. [[repository/src/crypto.rs:executable_signing_input]][[src/os/content_signature.rs:content_digest]]

The blob is written with a placeholder in the signature's place: the base64url
encoding of 64 zero bytes (86 `A`s), exactly a real signature's length, so filling
it moves no byte of the layout. Every linker — ELF (console and AppDir), Mach-O
(console and `.app`), and PE (console and GUI) — seals its finished image by
locating the `.mfbsign` section, computing the digest, and overwriting the one
unescaped `"contentSignature":"<placeholder>"` field; on Mach-O this runs before
the ad-hoc code signature hashes the pages, so `codesign -v` still passes. The
section is located by the same reader `mfb info` verifies with.
[[src/os/content_signature.rs:seal]][[src/os/inspect.rs:inspect]]

The chain is then: the registry key signs the attestation naming the ident key and
the `signingFingerprint`; the ident key signs the proof naming the same
fingerprint; the one-off key signs the file's contents. The one-off private key
is never written anywhere.

`mfb info <binary>` reads the blob back and checks it with no local state — no
pinned `server.pub`, no `~/.mfb`. The content signature is checked under
`signingKey` over the file it reads. The chain is checked online against the
blob's `registry`: the key fingerprints, the proof under `identKey`, the key the
registry serves at `GET /ident` (which must be the key the attestation's signed
`repoFingerprint` names — the URL itself is unsigned, so this is what binds it),
the attestation under that key, and then the owner's current ident from the
registry's signed name binding (`GET /index/<owner>#<package>` for the proof's
ident) and rotation chain (`GET /idents/<owner>`): still the signing ident,
rotated away from it through signed links, or replaced with no link. When the
proof's ident names another owner, the signer holds it only through a signed
ownership transfer: the index's transfer chain must verify from its name
binding and place the signing ident in the signer's ownership, and the current
ident is then the signer's server-bound one (`toBinding`); otherwise the chain
is reported as `proof ident does not belong to the signing owner`
(`./mfb spec package-manager repository-protocol`). The registry is reached over https
(plain http only for loopback). It keeps no revocation for signing keys or
attestations, and release state (yanked/deprecated) exists only for published
packages, so neither is reported for an executable (`./mfb spec tooling
cli-reference`). [[src/cli/info.rs:verify_chain]][[repository/src/client.rs:fetch_server_key]][[repository/src/client.rs:fetch_index_with_key]][[repository/src/trust.rs:verify_chain]]

## Trust boundary

The binary-representation reader does **not** verify the trust chain at import
time — it only checks header/manifest identity agreement and structural
sanity. Chain verification (pinned server key → attestation → pinned ident →
proof → one-off key → bytes) is the package manager's responsibility at
build/verify time; see `./mfb spec package verifier-rules`.
[[src/cli/build/packages.rs:classify_installed_package]]

## See Also

* ./mfb spec package container-format — the container v1.0 header, `"MFP-PACKAGE-v2\0"` prefix signature, and `packageBinaryHash` weld
* ./mfb spec package verifier-rules — the build-time verification chain
* ./mfb spec package-manager repository-protocol — the `/accounts/register`, `/auth/challenge`, `/auth/login`, and `/signing` endpoints
* ./mfb spec package-manager key-store — where the keypairs, pinned server key, and session token are stored on disk
* ./mfb spec package-manager owner-names — owner-name validation rules used before signing
* ./mfb spec tooling cli-reference — `mfb build --sign`, `mfb repo register`, and `mfb repo auth`
