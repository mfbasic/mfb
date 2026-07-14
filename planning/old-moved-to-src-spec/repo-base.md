# MFB Repository Base Implementation Plan

Last updated: 2026-06-21

This document scopes the first repository implementation pass. It covers only
account registration and authentication. Publishing, package resolution,
transparency-log inclusion proofs, metadata signing, key rotation, orgs, and
package upload/download APIs remain out of scope for this pass except for the
storage locations reserved here.

It complements:

- `specifications/repository.md`
- `specifications/package_format.md`
- `specifications/project.md`

---

## 1. Goal

Create an independent Rust repository service under `repository/**` with an
executable named `mfb-repo`.

The executable is launched as:

```sh
./mfb-repo --path ./path_to_repo/
```

The repository root passed with `--path` is the complete server-side storage
root. The first implementation must create and use:

```text
<repo_path>/meta.db
<repo_path>/packages/
<repo_path>/packages/<hash>.mfp
```

`meta.db` is SQLite3 and is the durable source of truth for accounts, public
keys, authentication challenges, issued sessions, and future package metadata.
The `packages/` directory is reserved for immutable `.mfp` blobs addressed by
content hash; registration/authentication tests should verify it is created but
should not implement publishing.

---

## 2. User-Facing Commands

The root `mfb` CLI gains repository commands that talk to a running repository
service:

```sh
mfb repo register <owner_name>
mfb repo auth <owner_name>
```

### 2.1 `mfb repo register <owner_name>`

Registers a new owner account.

Required behavior:

- Validate `owner_name` using the owner-handle rules from
  `repository.md`: ASCII identifier-like handle, case-folded for uniqueness,
  original casing preserved for display, and `std` rejected as reserved.
- Generate a new client-side Ed25519 keypair for the owner.
- Store the keypair locally as:

  ```text
  ~/.mfb/keys/<owner_name>.pub
  ~/.mfb/keys/<owner_name>.prv
  ```

- Create `~/.mfb/keys/` with private permissions before writing key files.
- Write the private key with owner-only permissions.
- Submit only the public key and a proof-of-possession signature to the
  repository service.
- Return an error if the case-folded owner name is already registered.
- Leave no partial local key files behind if registration fails after key
  generation.

For this base pass, the single generated keypair is the account auth key. The
database schema should keep key-role columns or tables ready for the broader
model in `repository.md`, where auth, ident, and signing keys become distinct.
The registration API must not require the server to generate, return, escrow,
or recover private keys.

### 2.2 `mfb repo auth <owner_name>`

Authenticates as an existing owner.

Required behavior:

- Load the local private key from `~/.mfb/keys/<owner_name>.prv`.
- Request an auth challenge for `owner_name` from the repository service.
- Sign the challenge locally and submit the signature.
- Receive a server-signed JWT session token.
- Store the authenticated session as:

  ```text
  ~/.mfb/session/<owner_name>.ses
  ```

- Create `~/.mfb/session/` with private permissions before writing session
  files.
- Write session files with owner-only permissions.
- Replace only that owner's previous session; sessions are one-per-owner.
- Report authentication failure if the owner is unknown, the key fingerprint
  does not match, the challenge is expired or reused, or the signature is
  invalid.

JWT requirements:

- Signed by the repository server, not by the client.
- Contains at least `sub`/owner, owner id, auth-key fingerprint, issued-at time,
  expiry time, and a unique token id.
- Expires one hour after issuance.
- Uses a server signing secret persisted in `meta.db` or a server-private file
  under `<repo_path>` with owner-only permissions.

---

## 3. Repository Service Scope

`mfb-repo --path <repo_path>` starts a local HTTP service for the base pass. The
service owns the SQLite database and package blob directory.

Required startup behavior:

- Create `<repo_path>` if missing.
- Create `<repo_path>/packages` if missing.
- Open or create `<repo_path>/meta.db`.
- Run idempotent migrations before accepting requests.
- Refuse to start if `<repo_path>` exists but is not a directory.
- Refuse to start if `meta.db` cannot be opened, migrated, or locked.
- Print the listening address in a stable machine-readable line that acceptance
  tests can parse.

Required endpoints:

- `POST /accounts/register`
- `POST /auth/challenge`
- `POST /auth/login`
- `GET /health`

The JSON shapes should follow `repository.md` §5 where possible. For this base
pass, `POST /accounts/register` may accept one public key field named
`authKey`; the database must record the role as `auth` so future migrations can
add `ident` and `signing` without reinterpreting existing keys.

---

## 4. SQLite Schema

The first migration should create these durable tables.

### 4.1 `owners`

Fields:

- `id` integer primary key
- `owner_display` text not null
- `owner_folded` text not null unique
- `created_at` integer not null
- `status` text not null, initially `active`

### 4.2 `keys`

Fields:

- `id` integer primary key
- `owner_id` integer not null references `owners(id)`
- `role` text not null, initially `auth`
- `public_key` blob not null
- `fingerprint` text not null unique
- `status` text not null, initially `current`
- `created_at` integer not null
- `revoked_at` integer null

### 4.3 `auth_challenges`

Fields:

- `id` text primary key
- `owner_id` integer not null references `owners(id)`
- `key_id` integer not null references `keys(id)`
- `nonce` blob not null
- `created_at` integer not null
- `expires_at` integer not null
- `used_at` integer null

### 4.4 `sessions`

Fields:

- `id` text primary key
- `owner_id` integer not null references `owners(id)`
- `key_id` integer not null references `keys(id)`
- `jwt_id` text not null unique
- `issued_at` integer not null
- `expires_at` integer not null
- `revoked_at` integer null

### 4.5 Reserved Package Tables

The migration may create package metadata tables now, but they must remain
unused by registration/authentication:

- `packages`
- `package_versions`
- `package_blobs`

No default rows or placeholder package records should be inserted.

---

## 5. Validation and Errors

Registration must reject:

- missing owner name;
- invalid owner name;
- reserved owner name `std`, case-insensitive;
- duplicate owner name after case-folding;
- malformed public key;
- invalid proof-of-possession signature.

Authentication must reject:

- missing owner name;
- unknown owner;
- missing local private key;
- malformed local private key;
- mismatched local key fingerprint;
- expired challenge;
- reused challenge;
- invalid signature;
- expired or malformed session token when a stored session is later read.

Errors must be stable enough for acceptance tests to assert on a clear substring
without depending on full debug output.

---

## 6. Unit Test Plan

Rust unit and integration tests under `repository/**` must cover the service,
storage, crypto, and CLI-support code without relying on a shared global home
directory.

Required coverage:

- owner-name validation accepts representative valid names and rejects invalid
  names, reserved names, non-ASCII names, path separators, empty strings, and
  overlong values;
- owner case-folding rejects duplicate registrations with different casing;
- registration persists the owner and auth key in SQLite;
- registration verifies proof-of-possession and rejects bad signatures;
- local key writing creates `~/.mfb/keys/<owner_name>.pub` and
  `~/.mfb/keys/<owner_name>.prv` with restricted private-key permissions;
- failed duplicate registration does not leave newly generated local key files;
- challenge creation stores nonce, expiry, owner id, and key id;
- login accepts a valid challenge signature exactly once;
- login rejects expired, reused, unknown, and bad-signature challenges;
- JWT creation sets a one-hour expiry and includes owner/key/session claims;
- JWT verification rejects expired, malformed, wrong-signature, and unknown
  session tokens;
- session writing creates `~/.mfb/session/<owner_name>.ses` with restricted
  permissions and replaces only that owner session;
- startup creates `<repo_path>/meta.db` and `<repo_path>/packages`;
- migrations are idempotent against an existing database.

Tests that inspect file permissions may use Unix-only assertions when needed,
but the implementation should keep permission-setting code isolated so
platform-specific behavior is explicit.

---

## 7. Acceptance Test Plan

Acceptance tests must run the built executable rather than calling library code
directly. They should create temporary repository and home directories for each
test case.

Required executable-level scenarios:

1. Start `mfb-repo --path <tmp_repo>`, wait for the printed listening address,
   and assert that `<tmp_repo>/meta.db` and `<tmp_repo>/packages` exist.
2. Run `mfb repo register alice` against the running service and assert:
   - command exits successfully;
   - `~/.mfb/keys/alice.pub` exists;
   - `~/.mfb/keys/alice.prv` exists;
   - the owner exists in `meta.db`.
3. Run `mfb repo register Alice` after registering `alice` and assert:
   - command exits non-zero;
   - stderr contains a duplicate-owner message;
   - no second owner row exists.
4. Run `mfb repo auth alice` after registration and assert:
   - command exits successfully;
   - `~/.mfb/session/alice.ses` exists;
   - the stored token decodes as a JWT with `alice` as subject;
   - the expiry is no more than one hour after issue time.
5. Run `mfb repo auth missing_owner` and assert non-zero exit with an
   unknown-owner message.
6. Delete or corrupt `~/.mfb/keys/alice.prv`, run `mfb repo auth alice`, and
   assert non-zero exit with a local-key error.
7. Run two registrations for different owners and authenticate both; assert
   that `~/.mfb/session/<owner>.ses` is one-per-owner and neither command
   overwrites the other owner's session.

The acceptance harness should stop the repository process at the end of each
case and fail if the process exits unexpectedly before the test completes.

---

## 8. Completion Criteria

This pass is complete only when:

- `repository/**` builds as an independent Rust project;
- `./mfb-repo --path ./path_to_repo/` creates and uses the repository storage
  layout specified here;
- `mfb repo register <owner_name>` works end-to-end against the running
  repository service;
- duplicate owner registration fails deterministically;
- `mfb repo auth <owner_name>` writes a server-signed one-hour JWT session file;
- all repository Rust unit tests pass;
- all executable-level acceptance tests pass.

Compiler acceptance for unrelated MFBASIC language features is not a substitute
for these repository acceptance tests.
