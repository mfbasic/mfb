# Local Key & Session Store

The `mfb` package-manager client keeps a per-machine store of Ed25519 owner
keypairs and short-lived repository sessions under a single private root
directory. This store is the local half of the repository protocol: `register`
generates and writes a keypair, `auth` signs a challenge with the stored private
key and caches the returned session token, and every authenticated request
(`signing-info`, publish) reads the cached session back. This topic owns the
on-disk layout, the path-resolution rules, the encoding, and the permission
model. [[repository/src/local.rs:LocalPaths]]

## Root Directory

The store root is resolved once, lazily, from the environment. [[repository/src/local.rs:LocalPaths]]

| source | precedence | value |
| --- | --- | --- |
| `MFB_HOME` | highest | taken verbatim as the store root |
| `HOME` | fallback | store root is `$HOME/.mfb` |
| neither set | — | hard error: `HOME is not set` |

`MFB_HOME` is used exactly as given (it is *not* joined with `.mfb`), which lets
tests and sandboxes point the store at a throwaway directory. The default root
is `~/.mfb`. [[repository/src/local.rs:from_env]]

## Layout

The root holds two subdirectories, one for keys and one for sessions; every file
is named after its owner. [[repository/src/local.rs:keys_dir]]

```text
$MFB_HOME (or ~/.mfb)/          (default mode)
├── keys/                       0700
│   ├── <owner>.pub             0600   base64url public key
│   └── <owner>.prv             0600   base64url private key
└── session/                    0700
    └── <owner>.ses             0600   session JWT (HS256)
```

| path | accessor | contents |
| --- | --- | --- |
| `keys/<owner>.pub` | `public_key_path` | base64url-encoded 32-byte Ed25519 public key |
| `keys/<owner>.prv` | `private_key_path` | base64url-encoded 32-byte Ed25519 private (seed) key |
| `session/<owner>.ses` | `session_path` | repository session token (a signed JWT), one line |

All file names are `format!("{owner}.ext")`; `owner` must already be a validated
owner name before any path is built (callers run `validate_owner_name` first).
See `./mfb spec package-manager owner-names` for the grammar.
[[repository/src/local.rs:public_key_path]]

## Encoding

Key material is stored as text, not raw bytes. Both `.pub` and `.prv` files hold
the **base64url, no-padding** encoding of the raw key bytes
(`URL_SAFE_NO_PAD`). On read, the file contents are trimmed of surrounding
whitespace before decoding, so a trailing newline is tolerated; a decode failure
yields `malformed local public key` / `malformed local private key`.
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
| `<owner>.pub`, `<owner>.prv`, `<owner>.ses` | `0600` | `write_private_file` (via `fs::write` then `set_permissions`) |

`create_private_dir` calls `create_dir_all`, so the root and intermediate
directories are created as needed, but only the named leaf directory's mode is
set explicitly — the root itself keeps the process-default (umask-derived)
mode. [[repository/src/local.rs:create_private_dir]]

## Write & Read Operations

| operation | function | effect |
| --- | --- | --- |
| store keypair | `write_keypair` | ensures `keys/` (0700), writes `<owner>.pub` and `<owner>.prv` (0600), each base64url-encoded |
| discard keypair | `remove_keypair` | best-effort `remove_file` of both key files; errors ignored (used to roll back a failed `register`) |
| load public key | `read_public_key` | reads, trims, base64url-decodes `<owner>.pub` |
| load private key | `read_private_key` | reads, trims, base64url-decodes `<owner>.prv`; missing file ⇒ `missing local private key …` |
| store session | `write_session` | ensures `session/` (0700), writes `<owner>.ses` (0600) verbatim |
| load session | `read_session` | reads `<owner>.ses`, returns it trimmed |

`write_keypair` is keyed by owner, so writing a keypair for a second owner adds a
new pair of files rather than replacing the first. [[repository/src/local.rs:write_keypair]]

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

The private key in `keys/<owner>.prv` is the same key used to prove ownership in
the repository protocol: `register` signs the `mfb-repo-register-v1` domain
message and `auth` signs the `mfb-repo-auth-v1` challenge with it. Package
signing fingerprints and the publish signing flow build on the session token
cached here. See `./mfb spec package-manager signing`. [[repository/src/crypto.rs:registration_message]]

## See Also

* ./mfb spec package-manager repository-protocol — the register/challenge/login endpoints that read and write this store
* ./mfb spec package-manager signing — how the stored private key signs packages and how fingerprints are derived
* ./mfb spec package-manager owner-names — the owner-name grammar that file names are built from
* ./mfb spec tooling cli-reference — the `mfb` commands that drive register/auth/publish
* ./mfb spec diagnostics error-codes — failure codes surfaced by store I/O and decode errors
