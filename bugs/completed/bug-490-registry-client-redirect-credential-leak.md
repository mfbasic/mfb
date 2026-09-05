# bug-490: a cross-origin 307/308 redirect re-sends the registry request body, leaking the session token

Last updated: 2026-09-03
Effort: small (<1h)
Severity: MEDIUM
Class: security (credential confidentiality / SSRF-adjacent)

Status: **FIXED** (2026-09-05, `18f589667`)

Regression Test: `repository/src/client.rs` —
`a_credentialed_post_does_not_follow_a_cross_origin_redirect` (RED before the
fix) and `a_blob_get_still_consults_the_redirect_policy` (the non-goal pin).

## Summary

The registry client's redirect guard vets a hop's *scheme* and *IP-literal
class* but not its *origin*. A hostname target like `https://attacker.example/`
passes. For a 307/308 redirect, HTTP method and body are preserved and replayed;
`reqwest` strips `Authorization`/`Cookie` on a cross-host hop but those are
**headers** — the registry client carries its credential as a **body** field
(`sessionToken`). So a control-plane request answered with `307 Location:
https://attacker.example/x` re-posts the session token (and, for `/publish`, the
entire base64 `.mfp`; for `/machines/link`, the sealed ident keypair) to an
attacker-chosen https host. Triggerable by the configured registry, or by an
open-redirect / subdomain-takeover / CDN misconfig on an otherwise honest one.

## Mechanism

```rust
// repository/src/client.rs:152
fn ensure_redirect_target(url: &reqwest::Url) -> Result<(), String> {
    if url.scheme() != "https" { return Err(...); }           // scheme only
    if let Some(host) = url.host_str() {
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = bare.parse::<std::net::IpAddr>() { ... }  // IP literals only
    }
    Ok(())                                                     // any hostname passes
}
```

The credential is a body field on every control-plane call:

```
$ grep -n 'session_token' repository/src/client.rs
352, 392, 482, 1042, 1061, 1081, 1098, 1109, 1127, 1139, 1156, 1167, 1187, 1200 ...
```

reqwest's cross-host stripping covers headers only
(`reqwest .../redirect.rs` `remove_sensitive_headers` → AUTHORIZATION, COOKIE,
cookie2, PROXY_AUTHORIZATION, WWW_AUTHENTICATE); 307/308 preserve method and
body (`tower-http follow_redirect` `TEMPORARY_REDIRECT | PERMANENT_REDIRECT`
keeps the method, and clones the body), and `RequestBuilder::json` produces a
reusable in-memory body, so `try_clone` succeeds.

Affected credentialed callers: `request_attestation`, `link_start`,
`rotate_ident`, `set_org_member`, `issue_publish_token`, `revoke_publish_token`,
`transfer_offer`, `transfer_accept`, `set_release_state`, `validate_package`,
`publish_package`.

## Reproduction — DEMONSTRATED after all (2026-09-05)

This section said "not demonstrated end-to-end … a loopback harness cannot drive
it without a trusted certificate". That is escapable, and the escape is the
guard's own documented limit: `ensure_redirect_target` blocks IP **literals**,
and its doc says "a hostname that resolves to an internal address is out of scope
for this literal check". So `https://localhost:<port>/` PASSES the guard, and the
hop gets attempted for real.

The target port has nothing listening, deliberately — pointing it at the test
stub would make the client open a TLS handshake against a plain-HTTP socket and
the stub's `read_request` would block on a `\r\n\r\n` that never arrives. A dead
port fails instantly and still proves the hop was taken.

Measured, with the pre-fix shared client, on a credentialed
`post_json("/publish", {"sessionToken": …})` answered
`307 Location: https://localhost:1/steal`:

    failed to connect to repository service: error sending request for url (…/publish)

i.e. the client **followed** the cross-origin hop and failed at the far end.
After the fix the same request reports

    repository request failed with status 307

and never opens a connection. What this shows is that the hop is ATTEMPTED with
the body intact; it does not show a listening attacker receiving the token, which
would need a trusted certificate. That limit is the honest one and is why the
assertion is on the connection attempt.

## Best fix

Reject a redirect hop whose `(scheme, host, port)` differs from the configured
registry origin for every route except `GET /blob/<hash>` (the only route with a
legitimate presigned-URL hop, and the only one whose bytes are
content-address-verified afterwards). Cheapest form: give the credentialed
`post_json` / `put_blob` calls a second client built with
`redirect::Policy::none()` — no control-plane route is documented to redirect —
and keep the shared client for blob GETs. State the invariant in
`.ai/net-tls.md`: a credential-bearing request never follows a cross-origin hop.

## Non-goals

- Do not break the presigned-URL 302 on `GET /blob`.
- Do not reintroduce a per-call `Client` (the `OnceLock` shared client exists to
  avoid a per-request tokio runtime; a second `OnceLock` is fine).
- Do not weaken the existing https-only / IP-literal checks.

## Prior art

bug-420 item 2 added the redirect guard for the SSRF/downgrade half (cited in
the code at `client.rs:129`). The credential-leak half is new; no prior item
(searched redirect / SSRF / token leak / bearer / sessionToken).
