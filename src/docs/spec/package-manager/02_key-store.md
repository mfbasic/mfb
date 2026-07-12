# Local Key & Session Store

The `mfb` package-manager client keeps a per-machine store of Ed25519 owner
keypairs, the pinned registry public key, and short-lived repository sessions
under a private per-repository root directory. This store is the local half of
the repository protocol: `register` generates and writes the machine's **auth**
keypair and the account's **ident** keypair, `auth` signs a challenge with the
stored auth private key and caches the returned session token, and every
authenticated request (attestation fetch, publish) reads the cached session
back. This topic owns the on-disk layout, the path-resolution rules, the
encoding, and the permission model. [[repository/src/local.rs:LocalPaths]]

## Root Directory

The store base is resolved once, lazily, from the environment, and then scoped
per repository: because key and session files are named only by owner, the CLI
appends the lowercase-hex SHA-256 of the repository URL as a directory
component so that one owner name used against two different repositories never
collides. [[src/cli/mod.rs:local_paths_for_repo]]

| source | precedence | value |
| --- | --- | --- |
| `MFB_HOME` | highest | taken verbatim as the store base |
| `HOME` | fallback | store base is `$HOME/.mfb` |
| neither set | — | hard error: `HOME is not set` |

The per-repository root is `<base>/<repo-hash>/` where `repo-hash =
hex(SHA-256(repo_url))`. `MFB_HOME` is used exactly as given (it is *not*
joined with `.mfb`), which lets tests and sandboxes point the store at a
throwaway directory. [[src/cli/mod.rs:local_paths_for_repo]]

## Layout

The per-repository root holds the pinned registry key and two subdirectories,
one for keys and one for sessions; key files are named after their owner and
role. [[repository/src/local.rs:keys_dir]]

```text
$MFB_HOME (or ~/.mfb)/<repo-hash>/
├── server.pub                  0600   base64url registry public key (pinned)
├── checkpoint                  0600   "<size> <root-hex>" last-seen log head
├── root-pin                    0600   "<registry-id> <root-fingerprint>"
├── snapshot-version            0600   highest snapshot version seen (rollback defense)
├── keys/                       0700
│   ├── <owner>.auth.pub        0600   base64url auth public key (per machine)
│   ├── <owner>.auth.prv        0600   base64url auth private key
│   ├── <owner>.ident.pub       0600   base64url ident public key (per account)
│   └── <owner>.ident.prv       0600   base64url ident private key
└── session/                    0700
    └── <owner>.ses             0600   session JWT (HS256)
```

| path | accessor | contents |
| --- | --- | --- |
| `server.pub` | `server_key_path` | base64url 32-byte registry public key, pinned on first contact |
| `checkpoint` | `checkpoint_path` | last-seen transparency-log head (`<size> <root-hex>`); rollback/fork detection anchor |
| `root-pin` | `root_pin_path` | pinned signed-metadata root: `<registry-id> <root-fingerprint>` |
| `snapshot-version` | `snapshot_version_path` | highest snapshot version seen; metadata rollback defense |
| `keys/<owner>.auth.pub` | `auth_public_key_path` | base64url 32-byte Ed25519 auth public key |
| `keys/<owner>.auth.prv` | `auth_private_key_path` | base64url 32-byte Ed25519 auth private (seed) key |
| `keys/<owner>.ident.pub` | `ident_public_key_path` | base64url 32-byte Ed25519 ident public key |
| `keys/<owner>.ident.prv` | `ident_private_key_path` | base64url 32-byte Ed25519 ident private (seed) key |
| `session/<owner>.ses` | `session_path` | repository session token (a signed JWT), one line |

The **auth** keypair is per machine and only logs into the registry API; the
**ident** keypair *is* the account identity, lives on every linked machine, and
signs build proofs — see `./mfb spec package-manager signing` for the key
model. `mfb repo register` writes both keypairs on the first machine;
`mfb repo link` writes them on every later machine — the auth keypair freshly
generated there, the ident keypair decrypted from the pairing relay blob
(argon2id + ChaCha20-Poly1305 under the one-time pairing code; see
*repository-protocol*). Machines are full equals afterwards.
[[repository/src/client.rs:link_fetch]] All key file names are `format!("{owner}.{role}.ext")`; `owner` must
already be a validated owner name before any path is built (callers run
`validate_owner_name` first). See `./mfb spec package-manager owner-names` for
the grammar. [[repository/src/local.rs:auth_public_key_path]]

## The Pinned Server Key (`server.pub`)

`server.pub` holds the registry's own public key, fetched from `GET /ident` and
written on **first contact** (trust-on-first-use). Every subsequent online flow
re-fetches `/ident` and compares: a key that differs from the pinned one is a
hard error naming the pinned file (`repository server key does not match the
pinned key in '<path>'`), never a silent re-pin. Offline verification reads the
pinned file directly. [[repository/src/local.rs:pin_server_key]][[repository/src/client.rs:ensure_server_key]]

## Encoding

Key material is stored as text, not raw bytes. Both `.pub` and `.prv` files hold
the **base64url, no-padding** encoding of the raw key bytes
(`URL_SAFE_NO_PAD`). On read, the file contents are trimmed of surrounding
whitespace before decoding, so a trailing newline is tolerated; a decode failure
yields a role-qualified message such as `malformed local auth public key` or
`malformed local ident private key`.
[[repository/src/crypto.rs:encode_bytes]]

| key | raw length | on-disk form |
| --- | --- | --- |
| public | 32 bytes (`PUBLIC_KEY_LEN`) | base64url-no-pad string |
| private | 32 bytes (`PRIVATE_KEY_LEN`) | base64url-no-pad string |

Session files are written and read **verbatim** (no base64 wrapping) — the JWT
is already a printable token. Reads trim surrounding whitespace.
[[repository/src/local.rs:write_session]]

## Permissions

The store is private to the invoking user. Directories are created with mode
`0700` and files with mode `0600`; the bits are applied after creation. On
non-Unix targets the permission step is a no-op. [[repository/src/local.rs:set_permissions]]

| target | mode | applied by |
| --- | --- | --- |
| `keys/`, `session/` | `0700` | `create_private_dir` (via `create_dir_all` then `set_permissions`) |
| `<owner>.<role>.pub`, `<owner>.<role>.prv`, `server.pub`, `<owner>.ses` | `0600` | `write_private_file` (via `fs::write` then `set_permissions`) |

`create_private_dir` calls `create_dir_all`, so the root and intermediate
directories are created as needed, but only the named leaf directory's mode is
set explicitly — the root itself keeps the process-default (umask-derived)
mode. [[repository/src/local.rs:create_private_dir]]

## Write & Read Operations

| operation | function | effect |
| --- | --- | --- |
| store auth keypair | `write_auth_keypair` | ensures `keys/` (0700), writes `<owner>.auth.pub` and `<owner>.auth.prv` (0600), each base64url-encoded |
| store ident keypair | `write_ident_keypair` | same, for `<owner>.ident.pub` / `<owner>.ident.prv` |
| discard owner keys | `remove_owner_keys` | best-effort `remove_file` of all four key files; errors ignored (used to roll back a failed `register`) |
| load auth public key | `read_auth_public_key` | reads, trims, base64url-decodes `<owner>.auth.pub` |
| load auth private key | `read_auth_private_key` | reads, trims, base64url-decodes `<owner>.auth.prv`; missing file ⇒ `missing local private key …` |
| load ident public key | `read_ident_public_key` | reads, trims, base64url-decodes `<owner>.ident.pub` |
| load ident private key | `read_ident_private_key` | reads, trims, base64url-decodes `<owner>.ident.prv`; missing file ⇒ `missing local ident private key …` |
| pin server key | `pin_server_key` | writes `server.pub` on first use; errors if a different key is already pinned |
| load pinned server key | `read_pinned_server_key` | reads, trims, base64url-decodes `server.pub` |
| store session | `write_session` | ensures `session/` (0700), writes `<owner>.ses` (0600) verbatim |
| load session | `read_session` | reads `<owner>.ses`, returns it trimmed |

The write helpers are keyed by owner, so writing keypairs for a second owner
adds new files rather than replacing the first owner's.
[[repository/src/local.rs:write_auth_keypair]]

## Owner-Scoped Sessions

Every key and session file is namespaced by owner, so a single machine can hold
credentials for any number of owners simultaneously. Writing a session for one
owner never disturbs another's, and re-writing the same owner's session
overwrites it in place (last write wins). The owner argument threads through
`register`/`auth`/publish in the client; `auth` writes the freshly issued token
to `session/<owner>.ses`, and later authenticated calls read it back by the same
owner key. [[repository/src/local.rs:write_session]]

The cached token is the repository's session JWT (signed `HS256`, with a
server-side expiry of one hour from issue); the client treats it as an opaque
string and simply replays it. When it expires, the next call fails and the user
re-runs `auth`. [[repository/src/client.rs:auth]]

## Relation to Signing

The auth private key in `keys/<owner>.auth.prv` proves machine login: `register`
signs the role-discriminated `mfb-repo-register-v1` message with it and `auth`
signs the `mfb-repo-auth-v1` challenge. The ident private key in
`keys/<owner>.ident.prv` proves account identity: it signs its own registration
proof and the per-build package proof. Package signing and the publish flow
build on the session token cached here plus both keys. See
`./mfb spec package-manager signing`. [[repository/src/crypto.rs:registration_message]]

## See Also

* ./mfb spec package-manager repository-protocol — the register/challenge/login endpoints that read and write this store
* ./mfb spec package-manager signing — how the stored private key signs packages and how fingerprints are derived
* ./mfb spec package-manager owner-names — the owner-name grammar that file names are built from
* ./mfb spec tooling cli-reference — the `mfb` commands that drive register/auth/publish
* ./mfb spec diagnostics error-codes — failure codes surfaced by store I/O and decode errors
