# bug-485: `rt_tls_connect_allow_self_signed` takes hard dependencies on newer-OpenSSL-only CLI options, and misreports the failures

Sub-issues (both in `tests/rt_tls_connect_allow_self_signed.rs`):

- **A** — `write_cert` uses `openssl req -not_before/-not_after` (OpenSSL 3.5+).
  Kills `still_rejects_an_expired_certificate` on CI. *This is what was filed.*
- **B** — `start_peer` uses `openssl s_server -naccept` (absent from LibreSSL),
  and its "child exited" branch blames the port race for what is actually an
  argument rejection. Found while reproducing A; kills the **other three** cases
  on macOS `/usr/bin/openssl`. See "Sub-issue B" below.

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

| Environment | Version | `req -not_before` | `ca -startdate -enddate -selfsign` | `s_server -naccept` |
| --- | --- | --- | --- | --- |
| CI, `ubuntu-24.04` (`.github/workflows/coverage.yml:83`) | OpenSSL 3.0.13 | fails ✗ (the reported failure) | documented ✓ (3.0 manpage) | ✓ (CI's other 3 cases pass) |
| macOS, `/usr/bin/openssl` | LibreSSL 3.3.6 | fails ✗ (reproduced) | works ✓ (measured) | **absent ✗** (sub-issue B) |
| macOS, `~/local/brew/bin/openssl` | OpenSSL 3.6.2 | works ✓ | works ✓ (measured) | ✓ (measured) |
| Linux box 2228, Debian 13 | OpenSSL 3.5.6 | works ✓ | works ✓ (measured) | ✓ (measured) |
| Linux box 2223, Kali | OpenSSL 3.6.3 | works ✓ | works ✓ (measured) | ✓ (measured) |

Note what this matrix says about *reproduction*: every reachable Linux box runs
OpenSSL ≥ 3.5, so none of them can reproduce sub-issue A. LibreSSL 3.3.6 is the
only local implementation that can, and the CI log is the only direct evidence
from 3.0.13 itself.

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
so the harness took a hard dependency on OpenSSL ≥ 3.5. The only other `openssl`
call in the file that is portable back to LibreSSL is `req -days`; the
`s_server` spawn has its own version dependency, which is sub-issue B below.

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

### Sub-issue B — `start_peer`'s `-naccept`, and the wrong cause it names

Found by running the whole file against LibreSSL 3.3.6 to confirm sub-issue A.
Not one test failed as documented — **all four** did, and the other three named a
cause that does not exist:

```
thread 'accepts_a_self_signed_peer' panicked at tests/rt_tls_connect_allow_self_signed.rs:255:5:
openssl s_server lost the bind on ten consecutive ports
```

`start_peer` spawns the peer with `-naccept 4`. Measured on LibreSSL 3.3.6:

```
$ openssl s_server -quiet -accept 18541 -cert c.pem -key k.pem -naccept 4
unknown option -naccept
usage: s_server [-accept port] [-alpn protocols] [-bugs] [-CAfile file]
  → exit 1, immediately
$ openssl s_server -quiet -accept 18542 -cert c.pem -key k.pem
  → still alive after 1s, accepting
```

`-naccept` is absent from LibreSSL's `s_server`
(`openssl s_server -help | grep -c naccept` → `0`).

The mechanism is a **misclassified exit**. `start_peer`'s poll loop reads any
`try_wait() == Some(_)` as "it could not bind, so another case won the port" — a
real hazard the surrounding comment documents at length. But an *unusable
argument* also exits immediately, and it is not a race: retrying cannot fix it.
The loop retries ten times, each on a fresh port, then reports `lost the bind on
ten consecutive ports` — a diagnosis with no relationship to the actual fault.
Both streams are `Stdio::null()`, so `unknown option -naccept` is discarded.

A and B are the same defect class — a hard dependency on a CLI option that is not
universally present, with the diagnostic thrown away — which is why they are
fixed together. Their effects are independent: A is CI-only (OpenSSL 3.0.13 *has*
`s_server -naccept`, hence CI's `3 passed; 1 failed`), B is
macOS-system-openssl-only.

B does **not** affect CI. It is in scope anyway because this bug's Goal requires
the expired case to run on macOS `/usr/bin/openssl`, and B blocks the file from
running there at all — so A cannot be verified on that row of the matrix without
fixing B.

## Goal

- `still_rejects_an_expired_certificate` runs the full handshake and observes
  `result=raised` on every `openssl` that can serve as this file's peer, not just
  on OpenSSL ≥ 3.5.

  > **Corrected during the fix.** This goal originally also named macOS
  > `/usr/bin/openssl` (LibreSSL 3.3.6) as a row that must go green. It cannot,
  > for a reason that has nothing to do with this bug: LibreSSL's `s_server`
  > stops serving after `start_peer`'s bare TCP readiness probe, so no client
  > ever reaches it. Measured — a real client against each peer after one probe
  > connection:
  >
  > | peer | `-naccept` | next connection |
  > | --- | --- | --- |
  > | LibreSSL 3.3.6 | omitted | **wedged** (client still waiting after 12s) |
  > | LibreSSL 3.3.6 | `4` | rejects the option, exits |
  > | OpenSSL 3.6.2 | omitted | served (`Verify return code: 18`) |
  > | OpenSSL 3.6.2 | `4` | served (`Verify return code: 18`) |
  >
  > LibreSSL is therefore not a supported peer, and the corrected goal is that it
  > must fail **loudly and accurately** rather than appear to work. See the
  > vacuous-pass note in Non-goals — this is not a technicality, it is the single
  > most dangerous thing found in this bug.
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
- **Do not let an unusable peer produce a passing run.** Three of the four cases
  assert `result=raised`, so they pass for *any* rejection — including "the
  client never connected at all". An intermediate version of this fix probed for
  `-naccept` and omitted it where unsupported; on LibreSSL that got past
  `start_peer` and produced `3 passed; 1 failed`, where all three passes were
  **vacuous** — the peer was wedged and nothing was verified. That is strictly
  worse than the original four loud failures, and it is the exact hazard the file
  header warns about ("a test proving a self-signed certificate is accepted
  proves nothing about safety"). The probe was reverted for this reason. Any
  future work here must keep an unusable peer *failing*, never passing.
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

### Phase 1 — reproduce + audit (no behavior change)

- [x] Reproduce sub-issue A on a pre-3.5 `openssl`. `/usr/bin/openssl`
      (LibreSSL 3.3.6) with `write_cert`'s exact `Peer::Expired` argv →
      `exit=1`, the `usage: req ...` block, and **no `cert.pem` written** —
      which is what trips the `assert!`.
- [x] Reproduce it through the test:
      `cargo test --test rt_tls_connect_allow_self_signed --no-fail-fast` with
      LibreSSL fronted on `PATH` → `still_rejects_an_expired_certificate`
      panics at `:160` with `openssl failed to generate a self-signed cert`.
- [x] Sub-issue B surfaced by the same run (the other three cases failed with
      `lost the bind on ten consecutive ports`); root-caused to `s_server`
      having no `-naccept` on LibreSSL, and written up above.
- [x] Blast-radius audit re-run at fix time; verdicts below.

Acceptance: met — the reproduction fails for the documented reason, and the
audit has a verdict per site.
Commit: (see Phase 2 — Phase 1 produced no code change)

### Phase 2 — the fix

- [x] `write_cert`: route `Peer::Expired` through a new `write_expired_cert`
      that generates a CSR and self-signs it with
      `openssl ca -batch -selfsign -startdate 190101000000Z -enddate
      200101000000Z`, writing the `[ca]` scaffolding (`index.txt`, `serial`,
      `policy`, `x509_extensions`) into the per-test temp root. The in-date arm
      keeps its `req -x509 ... -days 397` argv unchanged.
- [x] SAN and `extendedKeyUsage=serverAuth` move into the generated config's
      `[ext]` section for the `ca` path only — `ca` ignores `-addext`.
      `Peer::san()` remains the single source of truth, because an `-addext`
      argument and a config line share the `key=value` spelling.
- [x] New `assert_expired_cert_shape`: read the certificate back and require the
      SAN, the `serverAuth` EKU, and `x509 -checkend 0` reporting it expired.
      Without this the case could pass for the wrong reason — see Summary.
- [x] New `run_openssl` helper: capture `.output()` and report the argv plus the
      CLI's stderr on failure, instead of `.status()` with both streams
      discarded.
- [x] `start_peer`: on an early child exit, read the captured stderr and, if the
      CLI rejected an *argument*, fail immediately naming it — rather than
      retrying ten ports and blaming the port race.
- [ ] ~~Probe `s_server` for `-naccept` and omit it where unsupported.~~
      **Reverted.** It did not make LibreSSL usable (the peer wedges after the
      readiness probe regardless) and it converted three loud failures into three
      vacuous passes. See Non-goals.
- [ ] ~~Add an explicit `-addext basicConstraints=critical,CA:TRUE` to the
      in-date certificates.~~ **Reverted.** The hypothesis that LibreSSL's
      missing `basicConstraints` was why `accepts_a_self_signed_peer` failed was
      tested and **disproved** — the case still failed with it present. Keeping
      an unmotivated change would be scope creep.

Acceptance: met on every usable peer — see Phase 3.
Commit: `—` (filled at landing)

### Phase 3 — validation

- [x] **macOS / OpenSSL 3.6.2 (the only axis where all four cases genuinely
      exercise TLS locally):** `test result: ok. 4 passed; 0 failed` in 1.35s,
      including `accepts_a_self_signed_peer` — the one case that cannot pass for
      free — and `still_rejects_an_expired_certificate` through the new path.
- [x] **macOS / LibreSSL 3.3.6:** now fails in **0.14s** (was ~30s of pointless
      retries) with `openssl s_server rejected an argument, so this is not a lost
      bind and retrying will not help` quoting `unknown option -naccept`. No
      vacuous passes. This is the intended outcome for an unusable peer, not a
      regression.
- [x] **Linux / OpenSSL 3.5.6 (box 2228, Debian 13) and 3.6.3 (box 2223,
      Kali):** the replacement recipe run end to end produces
      `notBefore=Jan 1 00:00:00 2019 GMT`, `notAfter=Jan 1 00:00:00 2020 GMT`,
      SAN present, EKU present, `checkend: EXPIRED (correct)` on both.
- [x] Full local `cargo test --no-fail-fast`.
- [x] `cargo fmt` over the repo and the nested `repository/` workspace.
- [ ] **CI (ubuntu-24.04, OpenSSL 3.0.13) — not directly executed.** No box or
      container runtime with a 3.0.x `openssl` is reachable (`docker`/`podman`
      absent on 2228 and 2223). The axis is *bracketed* rather than run: the
      recipe is measured working on LibreSSL 3.3.6 (older, and missing strictly
      more options than 3.0.13) and on 3.5.6 / 3.6.2 / 3.6.3 (newer), and all
      three options are documented in the OpenSSL **3.0** `ca(1)` manpage.
      Confirm on the next CI run.

Acceptance: met, with the CI row explicitly recorded as bracketed, not executed.
Commit: `—` (filled at landing)

## Validation Plan

- Regression test:
  `tests/rt_tls_connect_allow_self_signed.rs::still_rejects_an_expired_certificate`
  — the failing-then-passing test is the existing one; the fix is what lets it
  run at all. `assert_expired_cert_shape` is the new permanent guard against it
  passing for the wrong reason.
- Runtime proof: on OpenSSL 3.6.2 the client prints `result=raised` against an
  `s_server` serving the 2019–2020 certificate, while `accepts_a_self_signed_peer`
  prints `result=connected` in the same run — so the expiry rejection is a real
  TLS outcome and not a failure to connect.
- Doc sync: none. No `mfb man` / `mfb spec` surface changes; no production code
  touched, so no `.ai/*` invariant changes. The durable lesson (OpenSSL CLI
  version skew, and vacuous negatives) is recorded in auto-memory.
- Full suite: `cargo test --no-fail-fast`. `scripts/test-accept.sh` is **not**
  required — no golden fixture, compiler, or generated output is touched.

## Open Decisions

- None outstanding. Both decisions raised when this was filed were resolved by
  measurement: the `[ca]` scratch state lives under the per-test `root` (already
  unique via `nonce()` and cleaned up with the rest), and the stderr capture was
  **not** widened to the sibling `rt_tls_listener_thread_transfer.rs` — it has no
  failing case behind it.

## Summary

The real engineering risk was never the `openssl ca` config itself — it was that
three of the four cases assert only "the handshake raised", so they pass for any
rejection whatsoever, including one where the client never reached the peer.
That is what made both wrong turns in this fix look like progress: the
`-naccept` probe produced `3 passed; 1 failed` on a peer that was wedged, and the
`ca` path could have produced a certificate missing its SAN and been "rejected"
for its name instead of its date. `assert_expired_cert_shape` closes the second
hole permanently; the reverted probe and this document's Non-goals close the
first.

No production code changed — `src/` TLS behavior is untouched, and the only
difference is how the fixture certificate is built. That is also why the Linux
proof did not need a remote rebuild: the client is the same binary, so the
portable question is whether the recipe yields an equivalent certificate there,
which was measured directly on two Linux OpenSSL versions.
