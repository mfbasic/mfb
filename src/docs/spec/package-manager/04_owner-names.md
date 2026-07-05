# Owner Names

An *owner name* is the namespace under which packages are published: it is the
left-hand side of a package ident (`<owner>#<package>`). Owner names are
registered once per repository, bound to an authentication keypair, and reused
on every subsequent publish. Because the name appears in idents, on-disk paths,
and signed messages, its grammar is deliberately narrow and is validated
identically on the client (before any network call) and on the server (before
any database write). This topic owns that grammar, the case-folding rule, and
the exact rejection conditions. [[repository/src/validation.rs:validate_owner_name]]

## Grammar

An owner name is a non-empty ASCII string of at most `OWNER_LIMIT` bytes whose
first character is an ASCII letter or underscore and whose remaining characters
are ASCII letters, digits, or underscores. `OWNER_LIMIT` is **255**.
[[repository/src/validation.rs:OWNER_LIMIT]]

```text
owner   = head tail
head    = ALPHA / "_"
tail    = *( ALPHA / DIGIT / "_" )
ALPHA   = %x41-5A / %x61-7A          ; A-Z a-z
DIGIT   = %x30-39                    ; 0-9
```

| rule | value |
| --- | --- |
| character set | ASCII only (`str::is_ascii`) |
| max length | `OWNER_LIMIT` = 255 bytes |
| min length | 1 (non-empty) |
| first char | `[A-Za-z_]` |
| subsequent chars | `[A-Za-z0-9_]` |
| reserved | `std` (case-insensitive) |

Length is measured in **bytes**, not Unicode scalar values; since the string is
required to be ASCII, the two counts coincide, but the byte-length check runs
before the ASCII check. [[repository/src/validation.rs:validate_owner_name]]

## Rejection Conditions

Validation returns the first matching error, in this order. Each row is a hard
rejection; there are no warnings. [[repository/src/validation.rs:validate_owner_name]]

| order | condition | error message |
| --- | --- | --- |
| 1 | empty string | `missing owner name` |
| 2 | length > 255 bytes | `invalid owner name: owner name is too long` |
| 3 | contains a non-ASCII byte | `invalid owner name: owner name must be ASCII` |
| 4 | equals `std` case-insensitively | `reserved owner name: std` |
| 5 | first char not `[A-Za-z_]` | `invalid owner name: must start with a letter or underscore` |
| 6 | any later char not `[A-Za-z0-9_]` | `invalid owner name: only ASCII letters, digits, and underscores are allowed` |

The ordering is observable: a too-long non-ASCII name reports the length error
(2), and a name beginning with a digit reports the leading-character error (5)
rather than the trailing-character error (6). The reserved-name check (4) runs
before the character-grammar checks, so `std` is rejected as reserved rather
than as a (valid) grammar match. [[repository/src/validation.rs:validate_owner_name]]

### Accepted / rejected examples

```text
accepted:  alice  Alice  _owner  owner_1  A123
rejected:  ""  std  STD  1alice  alice-bob  alice/bob  alice.bob  éclair
```

`éclair` is rejected at condition 3 (non-ASCII); `alice-bob`, `alice/bob`, and
`alice.bob` at condition 6 (illegal separator). [[repository/src/validation.rs:validate_owner_name]]

## Case Folding

Owner names are compared **case-insensitively** by lowercasing all ASCII
letters. `fold_owner(owner)` returns `owner.to_ascii_lowercase()`; only ASCII
case is folded, which is sufficient because validation already guarantees the
input is ASCII. [[repository/src/validation.rs:fold_owner]]

The display form (the bytes the user registered, preserving case) and the folded
form are stored separately. Registration inserts both `owner_display` and
`owner_folded`; all later lookups key on the folded form, and uniqueness is
enforced on the folded column. The practical consequence: `Alice` and `alice`
are the *same* owner — whichever is registered first wins, and a later attempt to
register the other case fails with `owner name '<owner>' is already in use`.
[[repository/src/store.rs:register_owner]] [[repository/src/store.rs:owner_with_auth_key]]

At publish time the owner segment parsed out of the package ident
(`<owner>#<package>`) is folded and compared against the folded session subject;
a mismatch yields `session owner does not match package ident owner`.
[[repository/src/server.rs:validate_package_request]]

One endpoint is an exception: `/signing` compares the requested owner against
the session subject (the registered *display* form) **without** folding, so
`mfb build --sign` must be given the owner in exactly the case it was
registered with (the ident's owner segment, by contrast, is folded).
[[repository/src/server.rs:signing]]

## Where Validation Runs

`validate_owner_name` is the single source of truth and is called on both sides
of every owner-scoped operation, so a malformed name is rejected locally before
a request is sent and again on the server before it touches the store.

| call site | operation |
| --- | --- |
| `client::register` | `POST /accounts/register` — claim a new owner |
| `client::auth` | `POST /auth/challenge` + `/auth/login` — open a session |
| `client::request_attestation` | `POST /signing` — fetch a build attestation |
| `client::validate_package` | `POST /validate` — dry-run a publish |
| `client::publish_package` | `POST /publish` — publish an artifact |
| `store::register_owner` | server-side insert of the owner + auth key |
| `store::create_challenge` | server-side challenge issuance |

[[repository/src/client.rs:register]] [[repository/src/client.rs:auth]] [[repository/src/client.rs:signing_info]] [[repository/src/client.rs:validate_package]] [[repository/src/client.rs:publish_package]] [[repository/src/store.rs:register_owner]] [[repository/src/store.rs:create_challenge]]

The owner name is also baked into the proof-of-possession message signed at
registration, so the name a client validated is cryptographically bound to the
key it registers; the server recomputes the same message before accepting the
proof. [[repository/src/crypto.rs:registration_message]] [[repository/src/store.rs:register_owner]]

## See Also

* ./mfb spec package-manager repository-protocol — the account, auth, and publish endpoints that carry an owner name
* ./mfb spec package-manager signing — the proof-of-possession message and how the owner is bound to its key
* ./mfb spec package-manager key-store — local storage of an owner's keypair and session token
* ./mfb spec tooling project-manifest — the `ident` field where an owner name appears as `<owner>#<package>`
* ./mfb spec package container-format — package idents embedded in metadata
