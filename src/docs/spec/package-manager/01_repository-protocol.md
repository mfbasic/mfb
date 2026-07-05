# Repository Protocol

The wire protocol between the `mfb` client and a repository (registry) service.
It backs four commands: `mfb repo register`, `mfb repo auth`, `mfb pkg publish`,
and `mfb build --sign`. (`mfb pkg add` does not use this protocol: it accepts
only `file://` URLs and copies the `.mfp` into `packages/`
locally.[[src/cli/pkg.rs:add_package]][[src/manifest/package.rs:package_file_url_path]])
The client side is the `mfb_repository` crate's `client`
module; the reference server is the same crate's `server` module. This topic
owns the HTTP surface (endpoints, methods, JSON bodies), the challenge-response
authentication flow, the JWT session token, and the validate-then-publish
sequence. Key generation and signing domains are owned by *signing*; on-disk key
and session storage is owned by *key-store*.

All mutating requests are `POST` with a JSON body and expect a JSON response
(`GET` serves `/health` and `/ident`). Binary fields (public keys, proofs,
nonces, signatures, package artifacts) are **base64url, no padding**. Hashes
and key fingerprints are lowercase **hex**.

## Transport and Endpoint Base

The base URL comes from the `MFB_REPO_URL` environment variable, falling back to
`DEFAULT_REPO_URL` = `http://127.0.0.1:7777`.[[repository/src/client.rs:repo_url_from_env]][[repository/src/lib.rs:DEFAULT_REPO_URL]]
Every request URL is `format!("{base}{path}")` with a single trailing slash
trimmed from the base, so `MFB_REPO_URL=https://repo.example/` and the path
`/publish` produce `https://repo.example/publish`.[[repository/src/client.rs:post_json]]

`post_json` is the single transport helper. On a 2xx it deserializes the typed
response. On a non-success status it reads the body, and if the body parses as
`{"error": "..."}` it returns that string verbatim; otherwise it returns
`repository request failed with status <status>: <body>`. A connection failure
returns `failed to connect to repository service: <err>`.[[repository/src/client.rs:post_json]][[repository/src/server.rs:ErrorResponse]]

The reference server ships as the `mfb-repo` binary: `mfb-repo --dbpath
<db_path> --datapath <data_path> [--listen <addr:port>]`. It listens on
`127.0.0.1:7777` by default, prints `MFB_REPO_LISTEN=<actual addr>` once bound,
and keeps its state as a SQLite database at `<db_path>` plus a blob directory
at `<data_path>`. On first run it generates its own Ed25519 **server keypair**
— the only private key the server ever holds; the private half never appears
in any response.[[repository/src/main.rs:parse_args]][[repository/src/server.rs:serve]][[repository/src/store.rs:open_repository]]

### Encoding

| Field class | Encoding | Helper |
| --- | --- | --- |
| public/private keys, proofs, nonces, signatures, artifact bytes | base64url **no pad** | `crypto::encode_bytes` / `crypto::decode_bytes` (`URL_SAFE_NO_PAD`) |
| content hash, key fingerprint | lowercase hex | `crypto::fingerprint` (SHA-256) / `content_hash_hex` |
| session token | compact JWS (three base64url segments) | `jsonwebtoken` |

[[repository/src/crypto.rs:encode_bytes]][[repository/src/crypto.rs:fingerprint]]

## Endpoint Summary

| Path | Method | Auth | Request | Response | Backs |
| --- | --- | --- | --- | --- | --- |
| `/health` | GET | none | — | `{"ok": true}` | (liveness) |
| `/ident` | GET | none | — | `ServerIdentResponse` | server-key pinning |
| `/accounts/register` | POST | proof signatures | `RegisterRequest` | `RegisterResponse` | `repo register` |
| `/auth/challenge` | POST | fingerprint match | `ChallengeRequest` | `ChallengeResponse` | `repo auth` (step 1) |
| `/auth/login` | POST | challenge signature | `LoginRequest` | `LoginResponse` | `repo auth` (step 2) |
| `/signing` | POST | session token | `SigningRequest` | `SigningResponse` | `build --sign` |
| `/validate` | POST | session token | `PackageArtifactRequest` | `ValidatePackageResponse` | `pkg publish` (step 1) |
| `/publish` | POST | session token | `PackageArtifactRequest` | `PublishPackageResponse` | `pkg publish` (step 2) |

[[repository/src/server.rs:serve]]

JSON field names use camelCase on the wire (set by `#[serde(rename)]`), even
though the Rust fields are snake_case. The tables below give the wire names.

## Server Identity — `GET /ident`

Returns the registry's own public key so clients can pin it as `server.pub` on
first contact (see *key-store*). The fingerprint is the same value attestations
later name as `repoFingerprint`.[[repository/src/server.rs:server_ident]]

```json
{
  "serverKey": "<base64url server public key>",
  "serverFingerprint": "<hex sha256 of server public key>"
}
```

The client verifies that `serverFingerprint` matches the key it decodes, pins
the key on first use, and hard-fails if a later fetch disagrees with the pinned
key.[[repository/src/client.rs:ensure_server_key]][[repository/src/local.rs:pin_server_key]]

## Owner Registration — `/accounts/register`

Backs `mfb repo register <owner_name>`. The client validates the owner name,
generates **two** fresh Ed25519 keypairs locally — the machine's `auth` key and
the account's `ident` key — and builds one proof-of-possession per key over the
role-discriminated registration message (signing domain `mfb-repo-register-v1`;
the role string is inside the signed bytes, so an auth proof can never be
replayed as an ident proof). Both keypairs are **written to local storage
first**, then the request is posted. If the POST fails the client **removes the
just-written keypairs** (rollback), so a failed registration leaves no local
keys. Private keys never leave the machine.[[repository/src/client.rs:register]][[repository/src/crypto.rs:registration_message]]

Request `RegisterRequest`:[[repository/src/server.rs:RegisterRequest]]

```json
{
  "owner": "alice",
  "authKey": "<base64url auth public key>",
  "identKey": "<base64url ident public key>",
  "proofs": {
    "auth": "<base64url signature over registration_message(auth)>",
    "ident": "<base64url signature over registration_message(ident)>"
  }
}
```

Response `RegisterResponse`:[[repository/src/server.rs:RegisterResponse]]

```json
{
  "owner": "alice",
  "authFingerprint": "<hex sha256 of auth public key>",
  "identFingerprint": "<hex sha256 of ident public key>"
}
```

The server decodes both keys and proofs, verifies each proof against its own
role's registration message, and records the owner with both public keys
(roles `auth` and `ident`, both `status='current'`). A duplicate owner name
yields a `409 Conflict` (message contains `already in use`); malformed input or
an invalid/role-swapped proof yields
`400`.[[repository/src/server.rs:register]][[repository/src/server.rs:conflict_or_bad_request]]
The CLI prints `Registered owner <owner> with auth fingerprint <fp> and ident
fingerprint <fp>`.[[src/cli/repo.rs:run_repo_command]]

## Challenge-Response Authentication

Backs `mfb repo auth <owner_name>`. Two round trips: obtain a nonce, then prove
possession of the private key by signing it.

### Step 1 — `/auth/challenge`

The client reads the local private key, derives the public key, optionally
cross-checks it against the stored public key (mismatch ⇒ `mismatched local key
fingerprint`), computes the SHA-256 fingerprint, and requests a
challenge.[[repository/src/client.rs:auth]]

If the local private key is **missing**, the client still probes
`/auth/challenge` with an empty fingerprint: if the server replies `unknown
owner` that error is surfaced; otherwise the original local-read error is
returned. This distinguishes "you never registered" from "your key is gone".[[repository/src/client.rs:auth]]

Request `ChallengeRequest`:[[repository/src/server.rs:ChallengeRequest]]

```json
{ "owner": "alice", "authFingerprint": "<hex fingerprint>" }
```

Response `ChallengeResponse`:[[repository/src/server.rs:ChallengeResponse]]

```json
{
  "challengeId": "<opaque id>",
  "nonce": "<base64url random bytes>",
  "expiresAt": 1700000000
}
```

The server looks up the owner's auth key; an unknown owner (`unknown owner`) or
a fingerprint that does not match the registered key (`mismatched local key
fingerprint`) yields `400`.[[repository/src/server.rs:challenge]] The nonce is
32 random bytes and the challenge expires **300 seconds** after
issue.[[repository/src/store.rs:create_challenge]]

### Step 2 — `/auth/login`

The client decodes the nonce, builds the challenge message (signing domain
`mfb-repo-auth-v1`, binding `challengeId` and `nonce`), signs it with the private
key, and posts the login. On success it **writes the returned session token to
local storage** keyed by owner.[[repository/src/client.rs:auth]][[repository/src/crypto.rs:challenge_message]]

Request `LoginRequest`:[[repository/src/server.rs:LoginRequest]]

```json
{ "challengeId": "<from challenge>", "signature": "<base64url signature>" }
```

Response `LoginResponse`:[[repository/src/server.rs:LoginResponse]]

```json
{
  "sessionToken": "<JWT>",
  "owner": "alice",
  "expiresAt": 1700003600
}
```

The server completes the challenge (verifying the signature over the challenge
message). A replayed/already-consumed challenge yields `409` (message contains
`reused challenge`); an unknown challenge id yields `400 unknown challenge` and
a lapsed one `400 expired challenge`. Completion is single-use: the challenge
row is marked used in the same transaction that accepts the
signature.[[repository/src/server.rs:login]][[repository/src/server.rs:conflict_or_bad_request]][[repository/src/store.rs:complete_challenge]]
The CLI prints `Authenticated owner <owner> until <expiresAt>`.[[src/cli/repo.rs:run_repo_command]]

### Session Token (JWT)

`sessionToken` is an HS256 JWT signed with the server secret. Lifetime is **3600
seconds** (`exp = iat + 3600`). Claims:[[repository/src/server.rs:SessionClaims]][[repository/src/server.rs:login]]

| Claim | Meaning |
| --- | --- |
| `sub` | owner display name |
| `owner_id` | internal owner row id |
| `auth_fingerprint` | hex fingerprint of the auth key at issue time |
| `iat` / `exp` | issued-at / expiry (unix seconds; `exp - iat == 3600`) |
| `jti` | random UUID; also recorded server-side as a session row |

Server verification (`verify_session_token`) requires three things: a valid
HS256 signature under the server secret, `exp` not in the past
(`validate_exp`), and a `jti` that still exists in the session table. Failure of
signature or expiry returns `expired or malformed session token`; an unknown
`jti` returns `unknown session token`.[[repository/src/server.rs:verify_session_token]]

## Attestation Issuance — `POST /signing`

Used by `build --sign` (plan-23 §3.3): an authenticated build pre-registers
its **one-off signing key** for one exact package+version and receives the
server-signed **attestation** naming it. Requires a session
token.[[repository/src/client.rs:request_attestation]]

Request `SigningRequest`:[[repository/src/server.rs:SigningRequest]]

```json
{
  "owner": "alice",
  "ident": "alice#toolbox",
  "version": "1.2.3",
  "signingFingerprint": "<hex sha256 of the one-off public key>",
  "sessionToken": "<JWT>"
}
```

Response `SigningResponse`:[[repository/src/server.rs:SigningResponse]]

```json
{
  "owner": "alice",
  "attestation": "<the exact attestation JSON the server signed>",
  "attestationSignature": "<base64url 64-byte Ed25519 signature>"
}
```

Server checks, each a `400` refusal: the session verifies and its `sub` equals
the requested owner (an **exact, case-sensitive** string compare against the
registered display form, unlike the case-folded ident-owner check at publish
time); the session's `owner_id`/`auth_fingerprint` still match the owner's
current auth key; `ident` is `<owner>#<package>` whose owner part case-folds
to the session owner; `version` is 1–64 bytes; `signingFingerprint` is 64
lowercase hex characters; the owner has a current ident key. The request is
**recorded before the server signs** (`signing_requests`), so a stolen auth
session requesting attestations always leaves a trace; the transparency log
(plan-23-B) builds on this. The attestation JSON and signature domain are
specified in *signing*. The signature is made with the server's own keypair —
the key served by `GET /ident`.[[repository/src/server.rs:signing]][[repository/src/store.rs:record_signing_request]]

The client verifies the returned signature against its pinned `server.pub`
before using the attestation, and refuses an attestation that does not pin the
requested package or that names a different ident key than the machine
holds.[[repository/src/client.rs:request_attestation]][[src/cli/build.rs:load_build_signing_info]]

## Package Artifact Requests

`/validate` and `/publish` share one request body, `PackageArtifactRequest`,
built from a `PackageArtifact` plus the session token. The `artifact` is the
whole `.mfp` file base64url-encoded.[[repository/src/client.rs:package_request]][[repository/src/server.rs:PackageArtifactRequest]]

```json
{
  "ident": "alice#mypkg",
  "version": "1.2.3",
  "artifact": "<base64url .mfp bytes>",
  "contentHash": "<hex>",
  "identFingerprint": "<hex>",
  "signingFingerprint": "<hex>",
  "sessionToken": "<JWT>"
}
```

### `/validate`

Returns `ValidatePackageResponse`:[[repository/src/server.rs:ValidatePackageResponse]]

```json
{
  "valid": true,
  "contentHash": "<hex>",
  "abiIndex": {},
  "diagnostics": []
}
```

The server verifies the session token, base64url-decodes and parses the `.mfp`,
recomputes the content hash, and accumulates `diagnostics`. `valid` is true iff
`diagnostics` is empty. An artifact that fails `.mfp` parsing is reported the
same way — `200` with `valid: false`, an **empty** `contentHash`, and the parse
error as the sole diagnostic — not as an HTTP error. Checks include: request
`contentHash`/`ident`/`version`/
`identFingerprint`/`signingFingerprint` matching the parsed package; the ident
shaped as `<owner>#<package>`; the session owner (case-folded) matching the
ident owner; the session's `owner_id`/`auth_fingerprint` still matching the
owner's current key; the package `identKey` equal to `ed25519:<current public
key>`; the
package fingerprints and `author` matching the registered owner; the embedded
package signature verifying against the owner's public key; and the
`ident@version` not already published.[[repository/src/server.rs:validate_package_request]]

### `/publish`

Returns `PublishPackageResponse`:[[repository/src/server.rs:PublishPackageResponse]]

```json
{
  "ident": "alice#mypkg",
  "version": "1.2.3",
  "hash": "<hex>",
  "publishedAt": 1700000000,
  "state": "available",
  "blobStored": true,
  "logEntry": "publish:<uuid>"
}
```

`state` is the constant `available` — the state recorded on the new
`package_versions` row.[[repository/src/store.rs:publish_package_version]]

`/publish` re-runs the **full validation** internally first; if `valid` is false
it returns `400 package validation failed: <diagnostics joined by "; ">`. On
success it writes the artifact blob to `<hash>.mfp` (only if not already
present — `blobStored` reflects whether a new write occurred) and records the
package version.[[repository/src/server.rs:publish_package]]

## `pkg publish`: Validate-then-Publish

`mfb pkg publish <owner_name> <package>` is a build-then-two-call sequence:[[src/cli/pkg.rs:publish_package_project]]

1. Validate the package `project.json`; require kind `package`.
2. `build --sign <owner>` in `Validate` mode to produce the signed `<name>.mfp`.
3. Read and re-parse the `.mfp`; compute the content hash and assemble a
   `PackageArtifact` from the package metadata.
4. Call **`/validate`** (`client::validate_package`); print the report; abort
   with `package validation failed` if `valid` is false.
5. Call **`/publish`** (`client::publish_package`); print `Published
   <ident>@<version> as <hash>`.

The client never publishes a package that did not validate; the server enforces
the same invariant independently by re-validating inside `/publish`.[[repository/src/client.rs:validate_package]][[repository/src/client.rs:publish_package]]

## See Also

* ./mfb spec package-manager signing — keypair generation, signing domains, and the package signature this protocol verifies
* ./mfb spec package-manager key-store — on-disk keypair and session-token storage (the rollback target and session source)
* ./mfb spec package-manager owner-names — owner-name validation and case folding applied before every request
* ./mfb spec package container-format — the `.mfp` bytes carried in `artifact` and parsed during validation
* ./mfb spec tooling cli-reference — `repo register`/`repo auth`/`pkg publish` command surface and exit codes
* ./mfb spec tooling project-manifest — the `project.json` that `pkg publish` reads before building
