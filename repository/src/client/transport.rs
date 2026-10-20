//! plan-176-L: the registry client's transport code. The module map lives in
//! `repository/src/client/mod.rs`.

use super::*;

/// A registry that cannot complete a TCP handshake this quickly is down; failing
/// fast beats waiting out a full request deadline on a dead host.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Deadline for a control-plane JSON call. These payloads are small — keys,
/// signatures, index metadata — so a slow one means a sick registry, not a big
/// transfer. This is reqwest's own default, kept deliberately: it is the right
/// bound for every endpoint except `/blob`.
pub(super) const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline for one `/blob/<hash>` transfer.
///
/// `CONTROL_TIMEOUT` is far too tight here: the registry accepts bodies up to
/// 64 MiB, and moving that inside 30s demands ~18 Mbit/s of *sustained*
/// throughput. A vendored native library on an ordinary connection therefore
/// fails outright with the default — the common case, not an edge case.
///
/// This bound is loose on purpose. Blocking reqwest exposes no read/stall
/// timeout (`ClientBuilder::read_timeout` is async-only in 0.12), and its
/// `timeout` is a *deadline* applied twice — once to connect+headers, then again
/// to the whole body read (`blocking::Response::bytes` re-wraps it). So the only
/// portable choice is between an unbounded hang and a generous deadline; 10
/// minutes carries a full 64 MiB down to ~0.9 Mbit/s while still bounding a
/// wedged socket. A true stall timeout needs the async API or a chunked reader.
pub(super) const BLOB_TIMEOUT: Duration = Duration::from_secs(600);

/// Ceilings on how much of a response body is buffered (bug-276 R3). Control-plane
/// JSON is small by construction; a blob is a vendored native library, and 96 MiB
/// leaves generous slack over the ~64 MiB such a file realistically reaches.
pub(super) const MAX_JSON_BYTES: u64 = 1024 * 1024;

pub(super) const MAX_BLOB_BYTES: u64 = 96 * 1024 * 1024;

/// The one shared HTTP client for every registry call.
///
/// Built once, for two reasons. Each blocking `Client` owns a tokio runtime on a
/// background thread, so constructing one per request spawned a runtime and paid
/// a fresh TLS handshake every time, defeating connection pooling entirely —
/// which matters now that installing a package can mean a run of sequential blob
/// fetches. And it is the only place to set `connect_timeout`, which reqwest
/// leaves unset by default.
pub(super) fn http_client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(CONTROL_TIMEOUT)
                .redirect(redirect_policy())
                .build()
                .map_err(|err| format!("failed to build the repository HTTP client: {err}"))
        })
        .as_ref()
        .map_err(|err| err.clone())
}

/// The client used by every **credential-bearing** request, built with
/// `redirect::Policy::none()` (bug-490).
///
/// The shared [`http_client`] vets each redirect hop's SCHEME and IP-LITERAL
/// class but not its ORIGIN, so an ordinary hostname target like
/// `https://attacker.example/` passes — by design, since a presigned blob URL is
/// exactly that shape. For a 307/308 the method and body are replayed, and
/// reqwest's cross-host stripping covers HEADERS only
/// (`remove_sensitive_headers`: AUTHORIZATION, COOKIE, …). This client's
/// credential is a BODY field (`sessionToken`), so nothing strips it: a
/// control-plane call answered with `307 Location: https://attacker.example/x`
/// re-posts the session token — and for `/publish`, the whole base64 `.mfp`; for
/// `/machines/link`, the sealed ident keypair — to a host the registry chose.
///
/// No control-plane route is documented to redirect, so the fix is simply not to
/// follow one here. Blob GET/HEAD keep the shared client, because a presigned-URL
/// hop is legitimate there and those bytes are content-address-verified
/// afterwards.
///
/// A second `OnceLock` rather than a per-call `Client`: each blocking client owns
/// a tokio runtime on a background thread, which is why [`http_client`] is shared
/// in the first place.
pub(super) fn no_redirect_client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(CONTROL_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|err| {
                    format!("failed to build the repository control-plane HTTP client: {err}")
                })
        })
        .as_ref()
        .map_err(|err| err.clone())
}

pub fn repo_url_from_env() -> String {
    std::env::var("MFB_REPO_URL").unwrap_or_else(|_| DEFAULT_REPO_URL.to_string())
}

/// Reject plaintext `http://` to a non-loopback registry (audit-2 SUP-01 /
/// bug-189). Package signatures remain the authenticity anchor, but cleartext
/// transport leaks which packages a build pulls and — critically — lets an
/// on-path attacker MITM the first-contact server-key pin (SUP-02). `http` stays
/// allowed for loopback so a local dev registry needs no TLS; `https` is required
/// for anything else. Called by every network entry point.
pub(super) fn ensure_transport_security(repo_url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(repo_url)
        .map_err(|err| format!("invalid registry URL '{repo_url}': {err}"))?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = parsed.host_str().unwrap_or("");
            // `host_str` returns an IPv6 literal in brackets (`[::1]`); strip them
            // so it parses as an `IpAddr`.
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            let is_loopback = host == "localhost"
                || bare
                    .parse::<std::net::IpAddr>()
                    .map(|ip| ip.is_loopback())
                    .unwrap_or(false);
            if is_loopback {
                Ok(())
            } else {
                Err(format!(
                    "refusing plaintext http:// to non-loopback registry '{host}': use \
                     https:// (set MFB_REPO_URL). Signatures still gate package \
                     authenticity, but http exposes your dependency set and lets an \
                     on-path attacker tamper with the trust bootstrap."
                ))
            }
        }
        other => Err(format!(
            "unsupported registry URL scheme '{other}://' in '{repo_url}'"
        )),
    }
}

/// A registry redirect chain longer than this is refused (bug-420 item 2). A
/// legitimate blob fetch is at most one hop (the presigned-URL 302); reqwest's
/// default cap is 10, kept here as a generous backstop for a well-behaved CDN.
const MAX_REDIRECTS: usize = 10;

/// Redirect policy for the shared client (bug-420 item 2).
///
/// reqwest's default follows up to 10 redirects to *any* host and scheme, but
/// `ensure_transport_security` only vets the initial URL — never a redirect
/// target. So a hostile or compromised registry could answer `/blob`, `/index`,
/// `/log/*`, or `/root.json` with a 302 to an internal/link-local host (blind
/// SSRF from a dev/CI box at 169.254.169.254, 127.0.0.1, RFC-1918) or to a
/// plaintext `http://` URL (downgrading the pinned https transport, leaking
/// which package is fetched). Authenticity is unaffected — blob bytes stay
/// SHA-256 checked and control-plane bodies signature-checked — but this closes
/// the transport-level SSRF/leak by vetting every hop.
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error(format!(
                "repository redirected more than {MAX_REDIRECTS} times"
            ));
        }
        let target = attempt.url();
        // Two stages, deliberately separate: the static URL check, then the
        // resolution check that closes the hostname hole (bug-585).
        let vetted = ensure_redirect_target(target)
            .and_then(|()| ensure_redirect_host_resolves_public(target, resolve_redirect_host));
        match vetted {
            Ok(()) => attempt.follow(),
            Err(message) => attempt.error(message),
        }
    })
}

/// Vet a single redirect target's URL TEXT: https only (no plaintext
/// downgrade), and never an IP literal in an SSRF-sensitive range (bug-420
/// item 2).
///
/// This is half the guard. It decides everything that can be read off the URL
/// without touching the network, so it stays pure and cheap. A target written
/// as a HOSTNAME is decided by [`ensure_redirect_host_resolves_public`]
/// (bug-585); [`redirect_policy`] is the one place both run.
pub(super) fn ensure_redirect_target(url: &reqwest::Url) -> Result<(), String> {
    if url.scheme() != "https" {
        return Err(format!(
            "refusing to follow a repository redirect to non-https URL '{url}': a redirect \
             must not downgrade the pinned https transport"
        ));
    }
    if let Some(host) = url.host_str() {
        // `host_str` brackets an IPv6 literal (`[::1]`); strip them so it parses.
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
            if is_blocked_redirect_ip(ip) {
                return Err(format!(
                    "refusing to follow a repository redirect to internal address '{host}': a \
                     redirect to a private, loopback, or link-local host is a possible SSRF"
                ));
            }
        }
    }
    Ok(())
}

/// Resolve a redirect target's hostname through the platform resolver — the same
/// `getaddrinfo` reqwest's own connector will call a moment later.
///
/// Split out from [`ensure_redirect_host_resolves_public`] so the policy can be
/// tested against a deterministic answer set: a security filter whose tests
/// depend on live DNS is a filter nobody can prove either way offline.
pub(super) fn resolve_redirect_host(
    host: &str,
    port: u16,
) -> Result<Vec<std::net::IpAddr>, String> {
    use std::net::ToSocketAddrs as _;
    (host, port)
        .to_socket_addrs()
        .map(|addrs| addrs.map(|addr| addr.ip()).collect())
        .map_err(|err| err.to_string())
}

/// The other half of the redirect guard: refuse a target whose HOSTNAME resolves
/// into a blocked range (bug-585).
///
/// `ensure_redirect_target` only ever parsed the host as an `IpAddr`, so a
/// hostile registry could 302 a blob fetch at `https://localhost:<port>/` — or
/// any attacker-controlled name with an `A` record of `127.0.0.1`,
/// `169.254.169.254`, or RFC-1918 space — and reqwest would resolve the name
/// itself and open a real socket to a service reachable only from the developer
/// or CI machine. Measured, not reasoned:
/// `a_hostname_redirect_resolving_to_loopback_is_refused_before_connecting`
/// recorded an accepted connection on the loopback probe before this landed.
///
/// Three properties a reader needs, stated plainly:
///
/// - **It fails CLOSED.** A name that will not resolve, or that resolves to an
///   empty answer, is refused rather than followed. That costs nothing real: an
///   unresolvable hop cannot be connected to either, so the request was going to
///   fail regardless — it just fails with an honest message instead of a
///   connector error. And a filter that treats "I could not check" as "allowed"
///   is not a filter.
/// - **ANY blocked answer refuses the hop**, not just the first. A name with a
///   mixed public/internal answer set is exactly how this gets smuggled past a
///   check that only looks at `addrs[0]`, and reqwest walks the whole list.
/// - **It is NOT airtight against DNS rebinding, and cannot be made so here.**
///   This resolves, then reqwest resolves again and connects: a classic TOCTOU.
///   A name with a ~0 TTL answering public once and `127.0.0.1` the next time
///   wins the race. Closing it needs the connection to be pinned to the exact
///   address this function approved — a custom `reqwest::dns::Resolve` on the
///   shared client — and that is not available *here*, because the resolver is
///   handed a bare hostname with no way to tell an initial URL from a redirect
///   hop, and `http://localhost:<port>` is a SUPPORTED local-dev registry
///   (`ensure_transport_security`). So this raises the attack from "write the
///   address in the `Location` header" to "win a resolver race", and the OS
///   resolver cache plus a realistic TTL make that unreliable for the attacker.
///   Blob bytes remain SHA-256 verified regardless; what is at stake is the
///   transport-level probe, not integrity.
///
/// Cost: one `getaddrinfo` per redirect hop, on a name reqwest is about to
/// resolve anyway, so in practice it is a warm-cache lookup. A legitimate blob
/// fetch is one hop.
pub(super) fn ensure_redirect_host_resolves_public(
    url: &reqwest::Url,
    resolve: impl Fn(&str, u16) -> Result<Vec<std::net::IpAddr>, String>,
) -> Result<(), String> {
    let Some(host) = url.host_str() else {
        return Ok(());
    };
    // An IP literal was already decided by `ensure_redirect_target`; there is no
    // name to look up, and no lookup is performed. (`host_str` brackets an IPv6
    // literal, so strip them before parsing.)
    if host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
        .is_ok()
    {
        return Ok(());
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = resolve(host, port).map_err(|err| {
        format!(
            "refusing to follow a repository redirect to '{host}': its address could not be \
             resolved ({err}). The redirect guard fails closed — a hop that cannot be shown \
             to be public is not followed."
        )
    })?;
    if addresses.is_empty() {
        return Err(format!(
            "refusing to follow a repository redirect to '{host}': it resolved to no \
             addresses. The redirect guard fails closed."
        ));
    }
    if let Some(blocked) = addresses.iter().find(|ip| is_blocked_redirect_ip(**ip)) {
        return Err(format!(
            "refusing to follow a repository redirect to '{host}': it resolves to internal \
             address {blocked}. A redirect to a private, loopback, or link-local host is a \
             possible SSRF, whether the address is written in the URL or answered by DNS."
        ));
    }
    Ok(())
}

/// Addresses a registry redirect must never reach: private, loopback,
/// link-local (including the `169.254.169.254` cloud-metadata endpoint), CGNAT,
/// and the unspecified address.
pub(super) fn is_blocked_redirect_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                // CGNAT shared range 100.64.0.0/10 (RFC 6598), absent from std.
                || (a == 100 && (b & 0xc0) == 0x40)
        }
        IpAddr::V6(v6) => {
            // Map `::ffff:a.b.c.d` back to the IPv4 rules so a mapped loopback or
            // private address cannot slip through as "just an IPv6 host".
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_redirect_ip(IpAddr::V4(v4));
            }
            let seg0 = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || (seg0 & 0xfe00) == 0xfc00 // unique-local fc00::/7
                || (seg0 & 0xffc0) == 0xfe80 // link-local fe80::/10
        }
    }
}

pub(super) fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub(super) fn post_json<T: DeserializeOwned>(
    repo_url: &str,
    path: &str,
    body: &impl serde::Serialize,
) -> Result<T, String> {
    ensure_transport_security(repo_url)?;
    let url = format!("{}{}", repo_url.trim_end_matches('/'), path);
    // bug-490: credential in the body — never follow a redirect. See
    // `no_redirect_client`.
    let response = no_redirect_client()?
        .post(&url)
        .json(body)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    read_json_response(response)
}

pub(super) fn get_json<T: DeserializeOwned>(repo_url: &str, path: &str) -> Result<T, String> {
    ensure_transport_security(repo_url)?;
    let url = format!("{}{}", repo_url.trim_end_matches('/'), path);
    let response = http_client()?
        .get(&url)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    read_json_response(response)
}

/// Read a response body with a hard ceiling (bug-276 R3).
///
/// Every body used to be buffered whole with no length check, and for a blob the
/// SHA-256 check only runs *after* buffering. A hostile registry — or the
/// presigned-URL host that `fetch_blob` silently follows on a 302 from the S3
/// backend — could therefore stream for up to `BLOB_TIMEOUT` (600s) and force a
/// multi-gigabyte allocation before anything got the chance to reject it.
/// Integrity was never at risk; availability was.
///
/// `Content-Length` is honored when present so an oversized body is refused
/// before a byte of it is read, but it is only a hint: the read itself is capped
/// at `limit + 1` so a chunked or lying response is bounded just the same.
pub(super) fn read_body_capped(
    mut response: reqwest::blocking::Response,
    limit: u64,
    what: &str,
) -> Result<Vec<u8>, String> {
    use std::io::Read as _;

    if let Some(length) = response.content_length() {
        if length > limit {
            return Err(format!(
                "{what} is {length} bytes, over the {limit}-byte limit"
            ));
        }
    }
    let mut body = Vec::new();
    response
        .by_ref()
        .take(limit.saturating_add(1))
        .read_to_end(&mut body)
        .map_err(|err| format!("failed to read {what}: {err}"))?;
    if body.len() as u64 > limit {
        return Err(format!("{what} exceeds the {limit}-byte limit"));
    }
    Ok(body)
}

fn read_json_response<T: DeserializeOwned>(
    response: reqwest::blocking::Response,
) -> Result<T, String> {
    let status = response.status();
    let body = match read_body_capped(response, MAX_JSON_BYTES, "repository response") {
        Ok(body) => body,
        Err(err) if status.is_success() => return Err(err),
        // An unreadable or oversized *error* body must not mask the status it
        // arrived with, so fall back to the same generic text as before.
        Err(_) => b"repository request failed".to_vec(),
    };
    if status.is_success() {
        return serde_json::from_slice::<T>(&body)
            .map_err(|err| format!("invalid repository response: {err}"));
    }
    let text = String::from_utf8_lossy(&body);
    if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
        // bug-489: the server authored this string. Sanitize it HERE, at the
        // trust boundary, not at the terminal — see `sanitize_server_text`.
        return Err(sanitize_server_text(&error.error));
    }
    Err(format!(
        "repository request failed with status {status}: {}",
        sanitize_server_text(&text)
    ))
}

/// Sanitize a string the SERVER authored before it becomes an error a caller
/// will show a human (bug-489).
///
/// Done here, at the trust boundary, rather than at the terminal. Two reasons,
/// and the second is what makes the placement load-bearing:
///
/// * **The print sites cannot tell trusted from untrusted.** `mfb`'s
///   `dispatch_command_error` renders every `CommandError`, and 19 of those carry
///   deliberate newlines (multi-line usage hints like `mfb repo …\n\n<hint>`).
///   `terminal_safe::safe` escapes `\n` — correctly, since a server-authored
///   newline forges whole rows — so wrapping the printer would render every one
///   of those hints as a single `\u{000a}`-littered line. Only the source knows
///   which strings are untrusted.
/// * **There are 6 of these and 53 callers.** Every `mfb` site that consumes a
///   client result would otherwise have to remember; a 54th would silently
///   reopen the hole. That is the failure mode this fix exists to close, so the
///   fix must not reproduce it one layer up.
///
/// Errors are display-only by construction, so escaping them costs nothing. Data
/// fields that a caller may COMPARE or store (`version.state`, a package name)
/// are deliberately NOT sanitized here — escaping those would change program
/// logic, so they are sanitized at their print sites instead.
pub(super) fn sanitize_server_text(text: &str) -> String {
    crate::terminal_safe::safe(text).into_owned()
}
