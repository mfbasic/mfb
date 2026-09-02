# bug-485: `still_rejects_an_expired_certificate` hard-fails on any OpenSSL older than 3.5 (`openssl req -not_before`)

Last updated: 2026-09-02
Effort: small (<1h)
Severity: MEDIUM
Class: Other (test infrastructure — a security-negative guard that cannot run)

Status: Open
Regression Test: `tests/rt_tls_connect_allow_self_signed.rs`

`tests/rt_tls_connect_allow_self_signed.rs:write_cert` builds the expired peer
certificate with `openssl req -x509 ... -not_before 20190101000000Z -not_after
20200101000000Z`. Those two options do not exist before OpenSSL **3.5**. On
ubuntu-24.04 (OpenSSL 3.0.13, the CI runner) and on macOS `/usr/bin/openssl`
(LibreSSL 3.3.6) the CLI prints its usage block and exits 1, so
`still_rejects_an_expired_certificate` panics at line 160 with `openssl failed
to generate a self-signed cert` before a single byte of TLS is exchanged. The
other three cases in the file use `-days 397` and pass, which is exactly the
observed CI shape (`3 passed; 1 failed`).

The correct behavior a fix produces: **`write_cert` generates the expired
self-signed identity — `CN=localhost`, `subjectAltName=DNS:localhost,IP:127.0.0.1`,
`extendedKeyUsage=serverAuth`, notBefore 2019-01-01, notAfter 2020-01-01 — using
only `openssl` options that exist in OpenSSL 3.0 and LibreSSL, so
`still_rejects_an_expired_certificate` actually runs the handshake and observes
`result=raised` on every supported environment.**

This is not a cosmetic CI failure. The case is one of the three *negative*
guards that give the `allowSelfSigned := TRUE` flag its meaning: without it,
nothing in the suite proves the flag relaxes the trust anchor check *only* and
leaves the date check intact. Today that proof runs on exactly one class of
machine — a developer box with a Homebrew OpenSSL ≥ 3.5 first on `PATH`.

References:

- `bugs/completed/` — bug-477 introduced the file (`511bdf31c bug-477 Phase 1/2a:
  the allowSelfSigned parameter, its pad, and the RED tests`); the header comment
  in the test file is the contract this bug protects.
- `.ai/net-tls.md` — networking / TLS readiness and the client-trust surface.
- `.ai/testing-gates.md` — the CI axes; see also the memory note
  "CI = linux + DEBUG; local gates = mac + RELEASE": a green local gate proves
  neither axis, which is precisely how this shipped.
- Sibling: `tests/rt_tls_listener_thread_transfer.rs:write_cert` — same
  generate-the-identity-at-run-time approach, `-days` only.

## Failing Reproduction

Run the exact `openssl req` invocation `write_cert` builds for `Peer::Expired`,
against an `openssl` that predates 3.5:

```
/usr/bin/openssl req -x509 -newkey rsa:2048 \
  -keyout /tmp/ossltest/key.pem -out /tmp/ossltest/cert.pem -nodes \
  -subj /CN=localhost \
  -addext subjectAltName=DNS:localhost,IP:127.0.0.1 \
  -addext extendedKeyUsage=serverAuth \
  -not_before 20190101000000Z -not_after 20200101000000Z
```

- Observed: the `usage: req [-addext ext] [-asn1-kludge] ...` block on stderr and
  `exit=1`. In the test the streams are `Stdio::null()`, so all that survives is
  the assertion:
  ```
  thread 'still_rejects_an_expired_certificate' panicked at
  tests/rt_tls_connect_allow_self_signed.rs:160:5:
  openssl failed to generate a self-signed cert
  test result: FAILED. 3 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
  ```
- Expected: `exit=0`, a `cert.pem` whose `notBefore=Jan 1 00:00:00 2019 GMT` /
  `notAfter=Jan 1 00:00:00 2020 GMT`, and the test proceeding to the handshake
  and asserting `result=raised`.

Contrast cases that work today and bound the bug:

- The same command with `-days 397` instead of the two date options succeeds on
  every binary tested — this is why `accepts_a_self_signed_peer`,
  `still_rejects_a_name_mismatch` and `defaults_to_rejecting_a_self_signed_peer`
  pass on CI.
- `openssl ca -startdate/-enddate` is documented in the **OpenSSL 3.0** manpage
  (`https://docs.openssl.org/3.0/man1/openssl-ca/`, "This allows the start date
  to be explicitly set. The format of the date is YYMMDDHHMMSSZ ... or
  YYYYMMDDHHMMSSZ"), and `-selfsign` with it; `openssl req -not_before` is
  **absent** from the 3.0 `req` manpage
  (`https://docs.openssl.org/3.0/man1/openssl-req/`) and present in master. In
  OpenSSL 3.6.2 `openssl ca -help` prints `-not_before val  An alias for
  -startdate` — i.e. the newer spelling is a synonym for an option that has been
  there all along.

Observed matrix:

| Environment | Version | `req -not_before` | `ca -startdate -enddate -selfsign` |
| --- | --- | --- | --- |
| CI, `ubuntu-24.04` (`.github/workflows/coverage.yml:83`) | OpenSSL 3.0.13 | fails ✗ (the reported failure) | documented ✓ (3.0 manpage) |
| macOS, `/usr/bin/openssl` | LibreSSL 3.3.6 | fails ✗ (reproduced above) | works ✓ (measured) |
| macOS, `~/local/brew/bin/openssl` | OpenSSL 3.6.2 | works ✓ | works ✓ (measured) |

The measured `ca` runs produced, on LibreSSL 3.3.6:

```
notBefore=Jan  1 00:00:00 2019 GMT
notAfter=Jan  1 00:00:00 2020 GMT
X509v3 Subject Alternative Name:  DNS:localhost, IP Address:127.0.0.1
X509v3 Extended Key Usage:        TLS Web Server Authentication
```

## Root Cause

`tests/rt_tls_connect_allow_self_signed.rs:write_cert` (lines 143–151) branches
on `Peer::Expired` and appends `-not_before` / `-not_after`:

```rust
match peer {
    Peer::Expired => args.extend([
        "-not_before".to_string(), "20190101000000Z".to_string(),
        "-not_after".to_string(),  "20200101000000Z".to_string(),
    ]),
    _ => args.extend(["-days".to_string(), "397".to_string()]),
}
```

The doc comment above it explains *why* a fixed 2019–2020 window is required —
"so the certificate is unambiguously expired no matter when the suite runs" —
and that reasoning is correct and must survive the fix. What went wrong is the
*spelling*: `-not_before`/`-not_after` are a recent addition to `openssl req`,
so the harness took a hard dependency on OpenSSL ≥ 3.5 while every other
`openssl` call in the file (`req -days`, `s_server -quiet -accept -cert -key
-naccept`) is portable back to LibreSSL.

The failure is silent about its cause because both streams are `Stdio::null()`
(line 155–156): the usage dump that names the unknown option is discarded, and
all that reaches the log is the generic `openssl failed to generate a
self-signed cert` — the same message `write_cert` would produce for a missing
key file, a bad subject, or a broken install.

The three passing cases are immune for one reason only: they take the `_ =>`
arm and pass `-days`, which has existed since OpenSSL 0.9.x.

`have_openssl()` (line 74) does not help here — it runs `openssl version`, which
succeeds on 3.0.13 and LibreSSL alike. The gate detects *absence* of the CLI,
not absence of an option.

## Goal

- `still_rejects_an_expired_certificate` runs the full handshake and observes
  `result=raised` on ubuntu-24.04 (OpenSSL 3.0.13) and on macOS with
  `/usr/bin/openssl` (LibreSSL 3.3.6), not just on OpenSSL ≥ 3.5.
- The expired identity keeps every property the current one has: `CN=localhost`,
  `subjectAltName=DNS:localhost,IP:127.0.0.1`, `extendedKeyUsage=serverAuth`,
  and a *fixed* 2019-01-01 → 2020-01-01 window (not a window relative to now).
- The `-days 397` + `serverAuth` shape of the two in-date certificates is
  untouched, so the macOS Apple-policy constraint documented on `write_cert`
  still holds.
- A future option-availability failure names the option: `write_cert` must
  capture and report `openssl`'s stderr instead of discarding it.

### Non-goals (must NOT change)

- **Do not delete, `#[ignore]`, or skip-guard the expired case.** Extending
  `have_openssl()` into a "does this openssl support `-not_before`" probe and
  early-returning is the tempting wrong fix: it turns a red CI into a green one
  while the negative guard still never runs on the axis that matters. Skipping
  is explicitly forbidden.
- Do not switch the expired window to a relative/negative `-days`. Measured:
  `/usr/bin/openssl req -x509 ... -days -400` exits 0 and emits
  `notBefore=Sep 2 2026 / notAfter=Oct 2 2026` — a *valid* certificate. That
  would silently invert the assertion.
- Do not commit a certificate or key to the tree. The file header states why
  the identity is generated at run time; that stays.
- Do not change `openssl s_server` as the peer, or replace it with `tls::listen`
  — the file header's asymmetry argument is the point of the test.
- No production code changes. `src/` TLS behavior is correct; this is a harness
  portability bug only.

## Blast Radius

Searched the tree for the option (`grep -rn -- "-not_before" --include='*.rs'
--include='*.sh' --include='*.yml' --include='*.md' .`, excluding `/target/`) —
exactly one hit.

- `tests/rt_tls_connect_allow_self_signed.rs:147` (`write_cert`, `Peer::Expired`
  arm) — **fixed by this bug**. The only `-not_before` in the repository.
- `tests/rt_tls_listener_thread_transfer.rs:write_cert` (line 75) — **unaffected**.
  It passes `-days 2` and no date options; verified by reading the full
  invocation (lines 78–96).
- `tests/rt_tls_connect_allow_self_signed.rs:start_peer` (`openssl s_server
  -quiet -accept -cert -key -naccept`) — **unaffected**; every flag predates
  OpenSSL 3.0 and the three passing cases exercise it on CI today.
- All other `openssl` uses in the tree are `s_client`/`s_server`/`version`
  peers — **unaffected**, and proven so by the fact that they pass on the CI
  runner.
- `Stdio::null()` on `openssl` invocations is a shared *diagnostic* weakness at
  both `write_cert` sites and `start_peer` — **latent, out of scope beyond the
  one site this bug touches**: fixing the message where the failure actually
  occurs is what this bug needs, and widening it to every `openssl` spawn in the
  TLS tests is churn without a failing case behind it.

## Fix Design

Replace the `req -x509` one-shot for `Peer::Expired` with the two-step
CSR-then-`ca -selfsign` path, which reaches the same certificate through
options that exist in OpenSSL 3.0 and LibreSSL:

1. `openssl req -new -newkey rsa:2048 -nodes -keyout key.pem -out csr.pem -subj
   /CN=localhost -config <cnf>`
2. `openssl ca -batch -selfsign -config <cnf> -keyfile key.pem -in csr.pem -out
   cert.pem -startdate 190101000000Z -enddate 200101000000Z -notext`

The generated `<cnf>` carries the `[ca]`/`[CA_default]` scaffolding (`database`
→ an empty `index.txt`, `serial` → `01`, `new_certs_dir`, `default_md = sha256`,
`policy` with `commonName = supplied`, `unique_subject = no`) and an
`x509_extensions` section supplying `subjectAltName` and
`extendedKeyUsage = serverAuth` — `ca` takes extensions from the config, not
from `-addext`, so the SAN/EKU move there for this path only.

This whole pipeline was **run end to end against LibreSSL 3.3.6** and produced
the notBefore/notAfter/SAN/EKU quoted in the reproduction section above, so the
design is measured rather than proposed.

The two in-date peers keep the existing `req -x509 -addext ... -days 397` path
verbatim — a single `match peer` fork inside `write_cert`, with the in-date arm
byte-identical to today. That confines the correctness risk to the case that is
already failing everywhere.

Alongside it, swap the two `openssl` spawns in `write_cert` from `.status()`
with `Stdio::null()` to `.output()`, and put the captured stderr into the
assertion message, so the next unsupported-option failure says which option.

Rejected alternatives:

- **Negative `-days`** — measured to produce a *valid* certificate on LibreSSL
  (see Non-goals). Silently inverts the test.
- **Skip when `-not_before` is unsupported** — forbidden above; it is the
  failure mode this bug exists to prevent.
- **Commit a pre-generated expired cert + key** — contradicts the file header's
  stated reason for run-time generation and puts a private key in the tree.
- **`faketime`/`libfaketime`** — not present on the CI runner or on macOS;
  trades one unportable dependency for a worse one.
- **Build the X.509 in Rust (rcgen/openssl crate)** — a new dev-dependency, and
  it weakens the test's premise: the peer must be an implementation with no idea
  this flag exists.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Confirm the reproduction on a non-3.5 `openssl` and record the exit code
      and usage dump (done above with `/usr/bin/openssl`, LibreSSL 3.3.6).
- [ ] Complete the blast-radius audit and write each verdict into this file
      (done above; re-confirm the `grep` at fix time in case a peer session has
      added a site).

Acceptance: `cargo test -p mfb --test rt_tls_connect_allow_self_signed
--no-fail-fast` with `/usr/bin/openssl` first on `PATH` fails
`still_rejects_an_expired_certificate` with `openssl failed to generate a
self-signed cert`, and the other three pass.
Commit: —

### Phase 2 — the fix

- [ ] In `tests/rt_tls_connect_allow_self_signed.rs:write_cert`, split the
      `Peer::Expired` arm onto the `req -new` + `ca -selfsign -startdate
      -enddate` pipeline, generating the `[ca]` config and the `index.txt` /
      `serial` scratch files under `root`. Keep the in-date arm unchanged.
- [ ] Move `subjectAltName` / `extendedKeyUsage=serverAuth` into the config's
      `x509_extensions` section for the `ca` path, keeping `Peer::san()` and
      `Peer::subject()` as the single source of truth for both paths.
- [ ] Capture `openssl` stderr (`.output()` instead of `.status()` +
      `Stdio::null()`) and include it in the assertion message.
- [ ] Update the `write_cert` doc comment: state that the expired window is set
      through `ca -startdate/-enddate` *because* `req -not_before` requires
      OpenSSL ≥ 3.5 and CI runs 3.0.x — so this does not get "simplified" back.

Acceptance: all four tests pass with `/usr/bin/openssl` (LibreSSL 3.3.6) first
on `PATH` **and** with the Homebrew OpenSSL 3.6.2 first on `PATH`; the generated
expired cert reports `notBefore=Jan 1 00:00:00 2019 GMT` /
`notAfter=Jan 1 00:00:00 2020 GMT` with the SAN and `serverAuth` EKU present;
nothing under `src/` changed.
Commit: —

### Phase 3 — validation

- [ ] `cargo test -p mfb --test rt_tls_connect_allow_self_signed --no-fail-fast`
      under both `openssl` binaries (the two-axis run above).
- [ ] `cargo test -p mfb --test rt_tls_listener_thread_transfer --no-fail-fast`
      — the sibling audited as unaffected; prove it.
- [ ] Full `cargo test --no-fail-fast`.
- [ ] `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0
      cargo fmt)`.
- [ ] Re-run the reproduction on the Linux box (see `.ai/remote_systems.md`) to
      close the CI axis, since a green macOS run proves neither the OS nor the
      OpenSSL-3.0 axis.

Acceptance: full suite green; the expired case passes on every row of the matrix
where it previously failed; no golden or `.ncode` delta (this touches no
production code, so any delta is a bug-hunt trigger).
Commit: —

## Validation Plan

- Regression test: `tests/rt_tls_connect_allow_self_signed.rs::still_rejects_an_expired_certificate`
  — the failing-then-passing test is the existing one; the fix is what lets it
  run at all.
- Runtime proof: with `/usr/bin/openssl` on `PATH`, the client program prints
  `result=raised` against an `s_server` serving the 2019–2020 certificate —
  i.e. `allowSelfSigned := TRUE` still enforces the date check. Confirm by
  inspecting the generated `cert.pem` with `openssl x509 -noout -dates -text`
  before asserting.
- Doc sync: none expected — this is test infrastructure; no `mfb man` or `mfb
  spec` surface changes. `.ai/net-tls.md` gains no invariant (the durable lesson
  is a harness one and belongs in memory, not the spec).
- Full suite: `cargo test --no-fail-fast`, plus `scripts/test-accept.sh` is
  **not** required — no golden fixture, compiler, or generated output is touched.

## Open Decisions

- Where the `[ca]` scratch state lives — under the per-test `root` tempdir
  (recommended: it is already unique per test via `nonce()` and cleaned up with
  the rest) vs. a separate tempdir. Recommend `root`.
- Whether to widen the stderr-capture change to `start_peer` and the sibling
  test's `write_cert` in the same commit. Recommend **no** — audited as latent
  and out of scope; keep the commit itemized.

## Summary

The engineering risk is entirely in Phase 2's `openssl ca` config: `ca` is
fussier than `req -x509` (it needs `index.txt`, `serial`, a `policy` section,
and takes extensions from the config rather than `-addext`), and a
misconfiguration produces a certificate that is expired but missing its SAN —
which would flip the test from "rejected for the date" to "rejected for the
name" and pass for the wrong reason. That is why Phase 2's acceptance criterion
inspects the generated certificate's dates *and* extensions rather than trusting
a green test. Everything else is untouched: no production code, no goldens, and
the three currently-passing cases keep their exact `req -x509 -days 397`
invocation.
