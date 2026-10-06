# bug-952: `/signing` signs and permanently logs attestations for idents/versions that the registry's REPO-17 charset forbids

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Security

Status: Open
Coordinator note: measured by the review agent against a local `mfb-repo` on 127.0.0.1 only; the coordinator did not re-run it.
Regression Test: (proposed) repository/src/server.rs `signing_rejects_an_ident_publish_would_reject`

`validate_registry_package_name` / `validate_version` (REPO-17) exist to "keep control
characters, quotes, `#`, `/`, whitespace, and the SQL `LIKE` wildcard `%` out of the
[transparency-log] payload entirely". `/publish` enforces them (`Store::publish_package_version`
→ `validate_ident`/`validate_version`). `/signing` checks only for a `#`, a non-empty package
part, ≤255 bytes, and a 1–64-byte version, exactly as 01_repository-protocol.md documents. It then
records the request and appends an `attestation` log entry **before** signing. Any authenticated
(self-registered) owner can therefore put arbitrary bytes into the append-only public log
(newlines, U+202E, NUL, `../`, `a#b`), and the server signs attestations for idents that can
never be published. The correct behavior: `/signing` applies `validate_ident` + `validate_version`
(after the owner check) and refuses before logging, and the spec line is updated to match.

References:

- wire/src/validation.rs module doc and `validate_registry_package_name` ("keeps control characters … out of the payload entirely").
- src/docs/spec/package-manager/01_repository-protocol.md "Attestation Issuance": "`ident` contains `#`, has a nonempty package part and is at most 255 bytes … `version` is 1–64 bytes".

## Failing Reproduction

Python harness /tmp/hardening/B16/py/h.py (register + login + `signing()`):

```
signing('erin', tok, 'erin#../../‮\n x', '1.0')   -> 200
signing('erin', tok, 'erin#a#b', '1.0')                -> 200
signing('erin', tok, 'erin#x', '\u0000')               -> 200
sqlite3 db.sqlite "select idx,payload from log_entries where kind='attestation'"
```

- Observed: all three return 200 with a server-signed attestation. The log holds
  `{"owner":"erin","ident":"erin#../../<U+202E>\n x",…}`, `{"ident":"erin#a#b",…}` and `{"version":"\u0000",…}`.
- Expected: 400 (`invalid package name: …` / `invalid version: …`), and no log entry.

## Root Cause

`repository/src/server.rs:signing` validates the ident shape ad hoc instead of calling
`crate::validation::validate_ident`/`validate_version`, and `Store::record_signing_request` appends the log entry unconditionally.

## Goal

- `/signing` refuses any ident/version `/publish` would refuse, before `record_signing_request`.

### Non-goals (must NOT change)

- Attestation JSON format; the record-before-sign ordering.

## Blast Radius

- `server.rs:signing` — fixed here.
- Other log writers (`register`, `rotate`, `release-state`, `transfer`) take owners/idents that are validated or looked up first; not observed to log raw input.
