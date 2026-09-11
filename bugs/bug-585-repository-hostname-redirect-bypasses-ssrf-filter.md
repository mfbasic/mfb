# bug-585: hostname redirects bypass the repository client's SSRF address filter

Last updated: 2026-09-11
Effort: medium (1–3h)
Severity: MEDIUM
Class: Security / client-side SSRF

Status: Open
Regression Test: a redirect-policy test with a controlled resolver/TLS listener,
covering a hostname which resolves to loopback, link-local, and RFC-1918 space.

The repository client rejects a redirect only when its target is written as an
internal IP literal. A hostile registry can instead redirect a blob request to
an attacker-controlled HTTPS hostname whose DNS answer is an internal address.
The shared `reqwest` client resolves and connects to that address after the
literal-only check, so the registry can make a developer or CI machine send a
GET to services reachable only from that machine.

Correct behavior: before following a redirect, resolve every target hostname
with the same resolver/connection policy that will be used for the request and
refuse any answer in a blocked range. Re-check the connected address so DNS
rebinding cannot turn a previously public answer internal.

References:

- Security review, 2026-09-11 (file-by-file redirect trace)
- `repository/src/client.rs:redirect_policy` and `ensure_redirect_target`

## Failing Reproduction

Arrange a TLS-capable hostname controlled by the test to resolve to `127.0.0.1`
(and separately `169.254.169.254` or RFC-1918). Have a registry endpoint return
`302 Location: https://that-host/blob`. Then fetch a blob through the repository
client:

```
cargo test -p mfb_repository client::tests::hostname_redirect_to_internal_address_is_refused
```

- Observed today: `ensure_redirect_target` sees a hostname, does not parse an
  `IpAddr`, returns `Ok(())`, and `reqwest` connects to its DNS answer.
- Expected: the redirect is rejected before any connection to the internal
  listener.

## Root Cause

`client.rs:ensure_redirect_target` calls `url.host_str()` and applies
`is_blocked_redirect_ip` only inside `if let Ok(ip) = bare.parse::<IpAddr>()`.
The adjacent comment explicitly limits the check to IP literals. The shared
`http_client` follows redirects for blob GET/HEAD so presigned URLs work; those
requests therefore retain this reachable hostname path. `no_redirect_client`
protects credential-bearing control-plane POSTs but does not protect blob
fetches from SSRF.

## Goal

- Reject hostname redirects resolving to loopback, link-local, private, CGNAT,
  unspecified, and other non-public ranges.
- Prevent DNS rebinding between policy evaluation and connection.
- Preserve legitimate HTTPS presigned blob URLs that resolve to public addresses.

### Non-goals (must NOT change)

- Do not disable valid public HTTPS presigned-URL redirects.
- Do not weaken content-hash verification for downloaded blobs.
- Do not re-enable redirects for credential-bearing control-plane requests.

## Blast Radius

- `repository/src/client.rs:{http_client,redirect_policy,ensure_redirect_target}`
  — policy and transport implementation.
- Client redirect tests — controlled DNS/connection coverage is required.

## Fix Design

Use a redirect-capable client whose resolver and connector expose the selected
socket address to the policy, or use an explicit one-hop blob redirect flow:
parse the `Location`, resolve it, reject every blocked answer, connect only to
an approved address while preserving TLS hostname verification, and revalidate
on retries/rebinding. Treat resolution failure or mixed public/internal answers
as a refusal.

## Phases

### Phase 1 — failing test + transport design (no behavior change)

- [ ] Add deterministic hostname-to-internal-address redirect tests.
- [ ] Cover DNS rebinding/mixed-answer behavior and public presigned URLs.

Acceptance: tests prove no socket is opened to an internal address named by a
hostname redirect.
Commit: —

### Phase 2 — the fix

- [ ] Bind redirect validation to DNS resolution and connection selection.
- [ ] Keep TLS certificate validation for the URL hostname.

Acceptance: hostname and IP-literal redirects have equivalent internal-address
protection.
Commit: —

### Phase 3 — full validation

- [ ] Run `cargo test -p mfb_repository`.
- [ ] Exercise public HTTPS blob redirects, all blocked address families, and
  rebinding/multiple-answer cases.

Acceptance: full suite green; the client cannot be driven to an internal
network target by a repository redirect.
Commit: —
