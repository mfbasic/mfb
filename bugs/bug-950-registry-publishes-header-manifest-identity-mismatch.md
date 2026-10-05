# bug-950: the registry publishes a package whose header identity disagrees with its embedded manifest, and every client then refuses to install it

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: measured by the review agent against a local `mfb-repo` on 127.0.0.1 only; the coordinator did not re-run it.
Regression Test: (proposed) repository/src/server.rs `a_header_manifest_identity_mismatch_is_refused`

`/validate` and `/publish` compare the plaintext header with the signed MANIFEST
section only for `author` and `url` (`package header and signed manifest disagree`).
They do not compare `name`, `ident`, `version`, `identKey` or the key fingerprints. Every
client's binary-representation reader rejects that mismatch at install time
(`MFP header identity does not match binary representation manifest identity`). The
registry therefore accepts, logs and serves an immutable version that can never be
installed. If that version is the newest `available` release, a floating
`mfb pkg add <owner>#<pkg>` fails for every consumer until the owner yanks it. The harm
is limited to the publisher's own package, because the signatures still hold. The correct
behavior: `/validate` reports, and `/publish` refuses, the same header↔manifest identity
mismatch the client enforces.

References:

- src/docs/spec/package/01_container-format.md: the binary-representation reader rejects "the container header identity does not match the embedded binary representation manifest identity … covers `name`, `ident`, `version`, `identKey`, and the fingerprints" ([[src/binary_repr/reader.rs:validate_container_manifest_identity]]).
- src/docs/spec/package-manager/01_repository-protocol.md "/validate" (checks 0–7) and "/publish" ("validates header author/URL agreement with the payload MANIFEST").

## Failing Reproduction

Python harness /tmp/hardening/B16/py/{h.py,mfp.py,t_v3.py}. It re-serializes a real
`mfb build --sign alice` toolbox.mfp with a fresh one-off key, a real `/signing` attestation and an ident-key
proof. The **header** `name` is changed to `evilname`, and the payload (MANIFEST name `toolbox`) is left untouched:

```
python3 t_v3.py            # POST /publish for alice#toolbox@0.7.0
mfb init app3 && cd app3 && mfb pkg add 'alice#toolbox@0.7.0'
mfb pkg add 'alice#toolbox'        # floating, 0.7.0 is newest available
```

- Observed: `/validate` → `valid: true, diagnostics: []`. `/publish` → `200 … "version":"0.7.0" … "blobStored":true`.
  Both `pkg add` calls → `error: failed to read resolver blob: MFP header identity does not match binary representation manifest identity` (exit 1).
- Expected: `/validate` diagnostic + `/publish` `400`, as already happens for an author/url mismatch
  (`package header and signed manifest disagree: header author "alice" url "https://evil.example/", manifest author "alice" url ""`).

## Root Cause

`repository/src/server.rs:publish_package` compares only `signed.author`/`signed.url`
(`abi::parse_manifest_metadata`). `validate_package_request` performs no header↔manifest comparison at all.
A payload without a parseable MANIFEST (`signed == None`) skips even the author/url check.

## Goal

- Registry-side identity agreement check on the same fields `validate_container_manifest_identity` compares, run in `/validate` (so the dry run predicts `/publish`).

### Non-goals (must NOT change)

- Accepting legitimately signed packages; ABI/description/docs extraction.

## Blast Radius

- `server.rs:validate_package_request` / `publish_package` — fixed here.
- `backfill.rs` re-parses author/url only; unaffected.
