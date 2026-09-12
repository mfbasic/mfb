# bug-585: hostname redirects bypass the repository client's SSRF address filter

Last updated: 2026-09-12
Effort: medium (1–3h)
Severity: MEDIUM
Class: Security / client-side SSRF

Status: Fixed
Regression Test: `client::tests::a_hostname_redirect_resolving_to_loopback_is_refused_before_connecting`
(end-to-end, loopback connection probe), plus
`a_presigned_blob_redirect_to_a_public_host_is_still_allowed` (positive pin),
`a_redirect_host_resolving_into_a_blocked_range_is_refused`,
`an_unresolvable_redirect_host_fails_closed`, and
`the_redirect_guard_uses_the_platform_resolver`.

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

### Measured, 2026-09-12 (the doc's root cause held, exactly as written)

The reproduction needs no controlled resolver and no TLS listener: `localhost`
IS a hostname whose DNS answer is `127.0.0.1`, so it drives the whole bug from
the hosts file with no network. The RED test
`a_hostname_redirect_resolving_to_loopback_is_refused_before_connecting` spawns
a bare loopback `TcpListener` that reports only THAT a connection arrived — the
correct instrument, since an SSRF succeeds the moment the socket opens — and has
the registry answer `GET /blob` with `302 Location: https://localhost:<port>/blob`.

Pre-fix, both assertions failed in order:

```
the client opened a socket to the loopback-only service named by the redirect:
that is the SSRF
```

and then, with the connection assertion removed, the error surfaced was a TLS
handshake failure against the probe rather than a redirect refusal. So the
socket was really opened; this is not a paper finding.

Two clarifications the original text did not make:

- The reach is not `/blob` alone. `http_client()` is shared by `fetch_blob`,
  `blob_exists` and `get_json`, so `/index`, `/root.json` and `/log/*` carry the
  same hop. Only `post_json`/`put_blob` are safe, and only because bug-490 moved
  them to `no_redirect_client`.
- `MAX_REDIRECTS = 10` does not bound the attack in any useful sense: it bounds
  the CHAIN, and every hop in the chain is a fresh request to a host the registry
  chose. One blob fetch therefore buys up to ten internal probes.

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

- [x] Add deterministic hostname-to-internal-address redirect tests.
- [x] Cover DNS rebinding/mixed-answer behavior and public presigned URLs.

Acceptance: tests prove no socket is opened to an internal address named by a
hostname redirect.
Commit: c7e7f1fec (landed with the fix — the RED test and the code it proves are
one change; the pre-fix failure is recorded above and reproducible from a
detached worktree at `beaa789c4`)

### Phase 2 — the fix

- [x] Bind redirect validation to DNS resolution and connection selection.
- [x] Keep TLS certificate validation for the URL hostname.

Acceptance: hostname and IP-literal redirects have equivalent internal-address
protection.
Commit: c7e7f1fec

### Phase 3 — full validation

- [x] Run `cargo test -p mfb_repository`.
- [x] Exercise public HTTPS blob redirects, all blocked address families, and
  rebinding/multiple-answer cases.

Acceptance: full suite green; the client cannot be driven to an internal
network target by a repository redirect.
Commit: c7e7f1fec

## What landed, and what it does NOT close

`redirect_policy` now runs two stages. `ensure_redirect_target` is unchanged and
still decides everything readable off the URL text (https-only, blocked IP
literal). The new `ensure_redirect_host_resolves_public` decides a target written
as a NAME: it resolves through `resolve_redirect_host` — a thin
`ToSocketAddrs` wrapper, i.e. the same `getaddrinfo` reqwest's connector is about
to call — and refuses if ANY answer is in a blocked range. The resolver is a
parameter so the policy is testable offline; a security check nobody can prove
without a network is a check nobody re-verifies.

Three properties, stated rather than implied:

- **Fails closed.** An unresolvable name, or an empty answer set, is refused. The
  cost is nil — a name that will not resolve could not have been connected to
  either — and a filter that reads "I could not check" as "allowed" is not a
  filter.
- **Any blocked answer refuses**, not `addrs[0]`. reqwest walks the whole list,
  and the answer ORDER is attacker-controlled, so a mixed public/internal set is
  exactly the smuggling shape. Pinned by
  `a_redirect_host_resolving_into_a_blocked_range_is_refused`.
- **NOT airtight against DNS rebinding.** This resolves, then reqwest resolves
  again and connects: resolve-then-connect is TOCTOU by construction. A name with
  a ~0 TTL that answers public once and `127.0.0.1` the next time wins the race.
  What the fix buys is raising the attack from "write the address in the
  `Location` header" — free, deterministic, one line of server config — to
  "win a resolver race against the OS cache". It does not eliminate the class.

Closing the rebinding half needs the connection pinned to the exact address the
policy approved, i.e. a custom `reqwest::dns::Resolve` on the shared client. That
was considered and rejected HERE, for a concrete reason rather than effort: a
`Resolve` impl is handed a bare hostname with no way to distinguish an initial
registry URL from a redirect hop, and `http://localhost:<port>` is a SUPPORTED
local-dev registry (`ensure_transport_security` allows plaintext to loopback by
design). A blanket filtering resolver would therefore break local development and
every loopback-bound test in the crate. Doing it properly means a connector that
carries per-request context, which is a transport rewrite, not this bug.

Cost: one `getaddrinfo` per redirect HOP, on a name reqwest resolves moments
later anyway, so in practice a warm-cache lookup. A legitimate blob fetch is one
hop. The IP-literal path performs no lookup at all — pinned by a resolver double
that panics if it is called.

### Follow-up, not landed here (out of this bug's scope)

`.ai/net-tls.md` still says the redirect guard is IP-literal-only and that
"`https://localhost:1/` passes". That was true and is now false. The prose needs
a one-paragraph correction; it was left alone because this fix was scoped to
`repository/src/client.rs`. The in-file doc comments on `ensure_redirect_target`
and the bug-490 tests were corrected as part of the fix.
