# bug-490: a cross-origin 307/308 redirect re-sends the registry request body, leaking the session token

Last updated: 2026-09-03
Effort: small (<1h)
Severity: MEDIUM
Class: security (credential confidentiality / SSRF-adjacent)

Status: Open (found in audit-3, Surface 9 SUP-03; `planning/completed/audit-3-supply-chain.md`)

Regression Test: none yet — add one asserting a credential-bearing request refuses a cross-origin redirect.

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

## Reproduction

Not demonstrated end-to-end: the guard requires an https target, so a loopback
harness cannot drive it without a trusted certificate. The three code facts —
the origin-blind guard (`client.rs:152-175`), the 307/308 body preservation, and
`session_token` in the body — are read directly and are unconditional.

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
