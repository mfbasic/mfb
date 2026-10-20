use crate::crypto;
use crate::local::{self, LocalPaths};
use crate::server::{
    ChallengeRequest, ChallengeResponse, CheckpointResponse, ConsistencyProofResponse,
    ErrorResponse, IdentChainResponse, InclusionProofResponse, IndexResponse, LinkFetchRequest,
    LinkFetchResponse, LinkStartRequest, LinkStartResponse, LogEntry, LoginRequest, LoginResponse,
    OrgMemberRequest, OrgMemberResponse, OwnerBinding, PackageArtifactRequest,
    PublishPackageResponse, RegisterProofs, RegisterRequest, RegisterResponse, ReleaseStateRequest,
    ReleaseStateResponse, RevokeChallengeRequest, RevokeRequest, RevokeResponse, RootResponse,
    RotateRequest, RotateResponse, ServerIdentResponse, SignedMetadataResponse, SigningRequest,
    SigningResponse, TokenIssueRequest, TokenIssueResponse, TokenRevokeRequest,
    TokenRevokeResponse, TransferAcceptRequest, TransferOfferRequest, TransferResponse,
    ValidatePackageResponse,
};
use crate::validation::{fold_owner, validate_owner_name};
use crate::DEFAULT_REPO_URL;
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use std::sync::OnceLock;
use std::time::Duration;

/// A registry that cannot complete a TCP handshake this quickly is down; failing
/// fast beats waiting out a full request deadline on a dead host.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Deadline for a control-plane JSON call. These payloads are small — keys,
/// signatures, index metadata — so a slow one means a sick registry, not a big
/// transfer. This is reqwest's own default, kept deliberately: it is the right
/// bound for every endpoint except `/blob`.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

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
const BLOB_TIMEOUT: Duration = Duration::from_secs(600);

/// Ceilings on how much of a response body is buffered (bug-276 R3). Control-plane
/// JSON is small by construction; a blob is a vendored native library, and 96 MiB
/// leaves generous slack over the ~64 MiB such a file realistically reaches.
const MAX_JSON_BYTES: u64 = 1024 * 1024;
const MAX_BLOB_BYTES: u64 = 96 * 1024 * 1024;

/// The one shared HTTP client for every registry call.
///
/// Built once, for two reasons. Each blocking `Client` owns a tokio runtime on a
/// background thread, so constructing one per request spawned a runtime and paid
/// a fresh TLS handshake every time, defeating connection pooling entirely —
/// which matters now that installing a package can mean a run of sequential blob
/// fetches. And it is the only place to set `connect_timeout`, which reqwest
/// leaves unset by default.
fn http_client() -> Result<&'static Client, String> {
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
fn no_redirect_client() -> Result<&'static Client, String> {
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
fn ensure_transport_security(repo_url: &str) -> Result<(), String> {
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
fn ensure_redirect_target(url: &reqwest::Url) -> Result<(), String> {
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
fn resolve_redirect_host(host: &str, port: u16) -> Result<Vec<std::net::IpAddr>, String> {
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
fn ensure_redirect_host_resolves_public(
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
fn is_blocked_redirect_ip(ip: std::net::IpAddr) -> bool {
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

/// Fetch the registry public key from `GET /ident` and pin it as
/// `server.pub` on first contact; a later mismatch is refused (plan-23 index
/// §10.3). Every online flow calls this before touching other routes.
pub fn ensure_server_key(repo_url: &str, paths: &LocalPaths) -> Result<Vec<u8>, String> {
    let server_key = fetch_server_key(repo_url)?;
    local::pin_server_key(paths, &server_key)?;
    Ok(server_key)
}

/// Fetch the registry public key from `GET /ident` and check it against the
/// fingerprint the response names — with no local state: nothing is pinned or
/// compared against a pin. Authenticated by transport only; a caller that holds
/// a signed claim about the key (an attestation's `repoFingerprint`) must compare
/// against that.
pub fn fetch_server_key(repo_url: &str) -> Result<Vec<u8>, String> {
    let response = get_json::<ServerIdentResponse>(repo_url, "/ident")?;
    let server_key = crypto::decode_bytes(&response.server_key, "serverKey")?;
    if crypto::fingerprint(&server_key) != response.server_fingerprint {
        return Err("repository /ident fingerprint does not match its key".to_string());
    }
    Ok(server_key)
}

pub fn register(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
) -> Result<RegisterResponse, String> {
    validate_owner_name(owner)?;
    // Refuse before touching anything if this machine already holds keys for the
    // owner (bug-272). The keypair writes below truncate unconditionally and the
    // error path calls `remove_owner_keys`, so without this guard a `register`
    // against an owner that already exists — the *guaranteed* server error — first
    // overwrote the existing ident private key and then deleted the replacement.
    // Per plan-23 the ident key is the account authority, so on a machine that is
    // the sole holder that sequence permanently locks the account out.
    //
    // Cleaning up after a failed registration for a genuinely *new* owner is still
    // correct and is left alone; the rule is only that `register` must never delete
    // keys it did not create in this call.
    for path in [
        paths.auth_private_key_path(owner),
        paths.ident_private_key_path(owner),
    ] {
        if path.exists() {
            return Err(format!(
                "keys for '{owner}' already exist locally; \
                 use `mfb repo auth` or `mfb repo link`, or remove them first"
            ));
        }
    }
    ensure_server_key(repo_url, paths)?;
    // Both keypairs are generated locally; only the public halves and their
    // role-separated proofs-of-possession go to the server (plan-23 §3.1).
    let (auth_public, auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    let auth_message = crypto::registration_message(crypto::ROLE_AUTH, owner, &auth_public);
    let auth_proof = crypto::sign(&auth_private, &auth_message)?;
    let ident_message = crypto::registration_message(crypto::ROLE_IDENT, owner, &ident_public);
    let ident_proof = crypto::sign(&ident_private, &ident_message)?;
    let request = RegisterRequest {
        owner: owner.to_string(),
        auth_key: crypto::encode_bytes(&auth_public),
        ident_key: crypto::encode_bytes(&ident_public),
        proofs: RegisterProofs {
            auth: crypto::encode_bytes(&auth_proof),
            ident: crypto::encode_bytes(&ident_proof),
        },
    };
    local::write_auth_keypair(paths, owner, &auth_public, &auth_private)?;
    local::write_ident_keypair(paths, owner, &ident_public, &ident_private)?;
    let response = post_json::<RegisterResponse>(repo_url, "/accounts/register", &request);
    match response {
        Ok(response) => Ok(response),
        Err(err) => {
            local::remove_owner_keys(paths, owner);
            Err(err)
        }
    }
}

pub fn auth(repo_url: &str, paths: &LocalPaths, owner: &str) -> Result<LoginResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let private = match local::read_auth_private_key(paths, owner) {
        Ok(private) => private,
        Err(local_err) => {
            let probe = post_json::<ChallengeResponse>(
                repo_url,
                "/auth/challenge",
                &ChallengeRequest {
                    owner: owner.to_string(),
                    auth_fingerprint: String::new(),
                },
            );
            if let Err(remote_err) = probe {
                if remote_err.contains("unknown owner") {
                    return Err(remote_err);
                }
            }
            return Err(local_err);
        }
    };
    let public = crypto::public_from_private(&private)?;
    if let Ok(stored_public) = local::read_auth_public_key(paths, owner) {
        if stored_public != public {
            return Err("mismatched local key fingerprint".to_string());
        }
    }
    let fingerprint = crypto::fingerprint(&public);
    let challenge = post_json::<ChallengeResponse>(
        repo_url,
        "/auth/challenge",
        &ChallengeRequest {
            owner: owner.to_string(),
            auth_fingerprint: fingerprint,
        },
    )?;
    let nonce = crypto::decode_bytes(&challenge.nonce, "nonce")?;
    let message = crypto::challenge_message(&challenge.challenge_id, &nonce);
    let signature = crypto::sign(&private, &message)?;
    let login = post_json::<LoginResponse>(
        repo_url,
        "/auth/login",
        &LoginRequest {
            challenge_id: challenge.challenge_id,
            signature: crypto::encode_bytes(&signature),
        },
    )?;
    local::write_session(paths, owner, &login.session_token)?;
    // A rotation staged by a run whose response was lost is settled here too
    // (bug-793). An unresolved one is kept for the next rotate or auth; logging in
    // does not depend on it.
    recover_pending_ident(repo_url, paths, owner)?;
    Ok(login)
}

/// `POST /signing` (plan-23 §3.3): pre-register the one-off signing key for
/// one exact package+version and fetch the server-signed attestation. The
/// attestation signature is verified against the pinned server key before it
/// is returned, so a swapped registry can never hand back paperwork the
/// consumer chain would later reject.
pub fn request_attestation(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    ident: &str,
    version: &str,
    signing_fingerprint: &str,
) -> Result<SigningResponse, String> {
    validate_owner_name(owner)?;
    let server_key = ensure_server_key(repo_url, paths)?;
    let session_token = local::read_session(paths, owner)?;
    let response = post_json::<SigningResponse>(
        repo_url,
        "/signing",
        &SigningRequest {
            owner: owner.to_string(),
            ident: ident.to_string(),
            version: version.to_string(),
            signing_fingerprint: signing_fingerprint.to_string(),
            session_token,
        },
    )?;
    let signature = crypto::decode_bytes(&response.attestation_signature, "attestationSignature")?;
    crypto::verify(
        &server_key,
        &crypto::attestation_signing_input(response.attestation.as_bytes()),
        &signature,
    )
    .map_err(|_| "attestation signature does not verify under the pinned server key".to_string())?;
    Ok(response)
}

/// Old-machine side of a machine link (plan-23 §3.2): generate the one-time
/// pairing code, encrypt the local ident keypair under it, and park the blob
/// on the server (single-use, short TTL). Returns `(code, expires_at)` — the
/// code is displayed to the user and never sent anywhere.
pub fn link_start(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
) -> Result<(String, i64), String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, owner)?;
    let ident_public = local::read_ident_public_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;

    let code = crypto::generate_pairing_code();
    let mut plaintext = ident_private.clone();
    plaintext.extend_from_slice(&ident_public);
    let (blob, salt) = crypto::seal_pairing_blob(&code, &plaintext)?;
    // bug-583: publish the PUBLIC half of the code-derived approval keypair as
    // the verifier for the fetch half. The code itself never leaves this
    // machine, so the relay cannot derive the private half and cannot approve
    // an auth key of its own.
    let (approval_public, _approval_private) = crypto::pairing_approval_keypair(&code)?;
    let response = post_json::<LinkStartResponse>(
        repo_url,
        "/machines/link",
        &LinkStartRequest {
            owner: owner.to_string(),
            lookup: crypto::pairing_lookup(&code),
            blob: crypto::encode_bytes(&blob),
            salt: crypto::encode_bytes(&salt),
            approval_key: crypto::encode_bytes(&approval_public),
            session_token,
        },
    )?;
    Ok((code, response.expires_at))
}

/// New-machine side of a machine link: generate this machine's own auth
/// keypair, fetch the relay blob with the typed pairing code, decrypt the
/// ident keypair, and store all four key files. After this the machine is a
/// full equal — `mfb repo auth` opens its session.
pub fn link_fetch(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    code: &str,
) -> Result<RegisterResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let (auth_public, auth_private) = crypto::generate_keypair();
    let message = crypto::registration_message(crypto::ROLE_AUTH, owner, &auth_public);
    let proof = crypto::sign(&auth_private, &message)?;
    // bug-583: prove possession of the typed code by signing THIS auth key
    // with the code-derived approval private key. The lookup only names the
    // pending pairing; it is not authorization.
    let lookup = crypto::pairing_lookup(code.trim());
    let (_approval_public, approval_private) = crypto::pairing_approval_keypair(code.trim())?;
    let approval = crypto::sign(
        &approval_private,
        &crypto::pairing_approval_message(&lookup, &auth_public),
    )?;
    let response = post_json::<LinkFetchResponse>(
        repo_url,
        "/machines/link/fetch",
        &LinkFetchRequest {
            owner: owner.to_string(),
            lookup,
            auth_key: crypto::encode_bytes(&auth_public),
            proof: crypto::encode_bytes(&proof),
            approval: crypto::encode_bytes(&approval),
        },
    )?;
    let blob = crypto::decode_bytes(&response.blob, "blob")?;
    let salt = crypto::decode_bytes(&response.salt, "salt")?;
    let plaintext = crypto::open_pairing_blob(code.trim(), &blob, &salt)?;
    if plaintext.len() != crypto::PRIVATE_KEY_LEN + crypto::PUBLIC_KEY_LEN {
        return Err("pairing blob does not contain an ident keypair".to_string());
    }
    let (ident_private, ident_public) = plaintext.split_at(crypto::PRIVATE_KEY_LEN);
    if crypto::public_from_private(ident_private)? != ident_public {
        return Err("pairing blob ident keypair is inconsistent".to_string());
    }
    local::write_auth_keypair(paths, owner, &auth_public, &auth_private)?;
    local::write_ident_keypair(paths, owner, ident_public, ident_private)?;
    Ok(RegisterResponse {
        owner: response.owner,
        auth_fingerprint: response.auth_fingerprint,
        ident_fingerprint: crypto::fingerprint(ident_public),
    })
}

/// Rotate the account ident (plan-23-B2, `mfb key rotate`): generate a new
/// ident keypair, sign the chain link with the OLD ident, prove possession
/// with the NEW one, and install the new keypair locally on success. Other
/// linked machines still hold the old (now `past`) ident private key and
/// must re-link — the rotation exists because a machine was lost, so the new
/// private key is never distributed automatically.
pub fn rotate_ident(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
) -> Result<RotateResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    // A rotation staged by an earlier run may already be the account authority:
    // its request can have committed with the response lost (bug-793). Settle it
    // before anything here signs with the live key or writes a new staging, since
    // overwriting it would destroy the only copy of the key the registry trusts.
    match recover_pending_ident(repo_url, paths, owner)? {
        PendingIdent::None | PendingIdent::Discarded => {}
        PendingIdent::Promoted(fingerprint) => {
            return Ok(RotateResponse {
                owner: owner.to_string(),
                ident_fingerprint: fingerprint,
            })
        }
        PendingIdent::Unresolved(reason) => {
            return Err(format!(
                "{reason}; a staged ident key was left at '{}' — re-run `mfb key \
                 rotate {owner}` once the registry can confirm whether it committed",
                paths.ident_pending_private_key_path(owner).display()
            ))
        }
    }
    let old_private = local::read_ident_private_key(paths, owner)?;
    let old_public = local::read_ident_public_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;

    let (new_public, new_private) = crypto::generate_keypair();
    let chain_message =
        crypto::ident_rotation_message(owner, &crypto::fingerprint(&old_public), &new_public);
    let chain_signature = crypto::sign(&old_private, &chain_message)?;
    let possession_message = crypto::registration_message(crypto::ROLE_IDENT, owner, &new_public);
    let possession_proof = crypto::sign(&new_private, &possession_message)?;

    // Stage the new keypair *before* asking the server to commit the rotation
    // (bug-276 R1). The server marks the old ident `past` inside its transaction,
    // so if the new private key only existed in memory until after a successful
    // response, then a lost response or a failing disk write would leave this
    // machine holding a key that is no longer the account authority and no copy
    // of the one that is. Rotation exists precisely because other machines are
    // lost or untrusted, so that can brick the account.
    local::write_pending_ident_keypair(paths, owner, &new_public, &new_private)?;
    let response = post_json::<RotateResponse>(
        repo_url,
        "/keys/rotate",
        &RotateRequest {
            owner: owner.to_string(),
            new_ident_key: crypto::encode_bytes(&new_public),
            chain_signature: crypto::encode_bytes(&chain_signature),
            possession_proof: crypto::encode_bytes(&possession_proof),
            session_token,
        },
    );
    match response {
        Ok(response) => {
            local::promote_pending_ident_keypair(paths, owner)?;
            Ok(response)
        }
        Err(err) => {
            // The request failed, but "failed" includes a response lost after the
            // server already committed — the case that actually strands the
            // account. Ask the registry which ident it now holds.
            match recover_pending_ident(repo_url, paths, owner)? {
                PendingIdent::Promoted(fingerprint) => Ok(RotateResponse {
                    owner: owner.to_string(),
                    ident_fingerprint: fingerprint,
                }),
                // Server still on the old ident: the rotation did not happen.
                PendingIdent::None | PendingIdent::Discarded => Err(err),
                // Leave the staged key in place — it is the only copy of a
                // possibly-authoritative key; a later rotate or auth settles it.
                PendingIdent::Unresolved(reason) => Err(format!(
                    "{err}; {reason}; a staged ident key was left at '{}' — re-run \
                     `mfb key rotate {owner}` once the registry can confirm whether it \
                     committed",
                    paths.ident_pending_private_key_path(owner).display()
                )),
            }
        }
    }
}

/// What [`recover_pending_ident`] decided about a staged ident rotation.
#[derive(Debug, PartialEq, Eq)]
enum PendingIdent {
    /// Nothing was staged.
    None,
    /// The staged key is the registry's current ident: the rotation committed,
    /// and the staging is now the live keypair. Carries its fingerprint.
    Promoted(String),
    /// The registry still holds this machine's live ident: the rotation never
    /// committed, so the staging was discarded.
    Discarded,
    /// The registry could not be consulted, or holds a key that is neither the
    /// staged nor the live one. The staging is kept untouched.
    Unresolved(String),
}

/// Settle a staged ident rotation by asking the registry which ident key it
/// currently holds (bug-276 R1, bug-793).
///
/// Only a registry that demonstrably holds the staged key promotes it, and only
/// one that demonstrably still holds the live key discards it. Every other state
/// keeps the staging: deleting a key that might be the account authority is the
/// one outcome worth avoiding. `Err` is reserved for a local file failure.
fn recover_pending_ident(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
) -> Result<PendingIdent, String> {
    let Some(staged_private) = local::read_pending_ident_private_key(paths, owner)? else {
        return Ok(PendingIdent::None);
    };
    let staged_public = crypto::public_from_private(&staged_private)?;
    let current = match fetch_ident_chain(repo_url, owner)
        .and_then(|chain| crypto::decode_bytes(&chain.ident_key, "identKey"))
    {
        Ok(current) => current,
        Err(err) => {
            return Ok(PendingIdent::Unresolved(format!(
                "the registry could not be asked whether a staged ident rotation \
                 committed: {err}"
            )))
        }
    };
    if current == staged_public {
        local::promote_pending_ident_keypair(paths, owner)?;
        return Ok(PendingIdent::Promoted(crypto::fingerprint(&staged_public)));
    }
    if local::read_ident_public_key(paths, owner).ok().as_deref() == Some(current.as_slice()) {
        local::remove_pending_ident_keypair(paths, owner);
        return Ok(PendingIdent::Discarded);
    }
    Ok(PendingIdent::Unresolved(format!(
        "the registry's ident for '{owner}' ({}) is neither the staged key nor this \
         machine's live key",
        crypto::fingerprint(&current)
    )))
}

/// Fetch the owner's current ident binding and the signed rotation chain.
pub fn fetch_ident_chain(repo_url: &str, owner: &str) -> Result<IdentChainResponse, String> {
    validate_owner_name(owner)?;
    get_json::<IdentChainResponse>(repo_url, &format!("/idents/{owner}"))
}

/// The owners' rotation chains as plan-168-C's verifier (`crate::trust`) takes
/// them, read from `GET /idents/<owner>` on `repo_url`. The chain needs no
/// server signature: each link is signed by the key it retires.
pub fn registry_chains(
    repo_url: &str,
) -> impl Fn(&str) -> Result<Vec<crate::server::IdentChainLink>, String> + '_ {
    move |owner: &str| fetch_ident_chain(repo_url, owner).map(|response| response.chain)
}

/// Walk a signed ident chain from `pinned` (plan-23-B2 pin-follow): verify
/// each link's old-key signature over the rotation message and follow
/// old→new until the chain is exhausted. Returns the newest chained
/// successor of `pinned`, or None when `pinned` never appears — the
/// no-chain-link case (a re-anchor), which callers must treat as a hard
/// error, never a silent re-pin.
pub fn follow_ident_chain(
    owner: &str,
    pinned: &[u8],
    chain: &[crate::server::IdentChainLink],
) -> Result<Option<Vec<u8>>, String> {
    let mut path = chain_path(owner, pinned, chain)?;
    Ok(if path.len() > 1 { path.pop() } else { None })
}

/// Every key on `owner`'s signed rotation chain from `start` forward, `start`
/// first (plan-168-C): each successor is accepted only through a link signed by
/// the key it retires, over the owner-bound rotation message. The chain is read
/// once, oldest link first, so the walk is bounded by its length. A key `K` is
/// "forward-reachable" from `start` exactly when it is on this path. A `start`
/// the chain never names yields just `[start]`.
pub fn chain_path(
    owner: &str,
    start: &[u8],
    chain: &[crate::server::IdentChainLink],
) -> Result<Vec<Vec<u8>>, String> {
    let mut path = vec![start.to_vec()];
    for link in chain {
        let old_key = crypto::decode_bytes(&link.old_key, "oldKey")?;
        if Some(&old_key) != path.last() {
            continue;
        }
        let new_key = crypto::decode_bytes(&link.new_key, "newKey")?;
        let signature = crypto::decode_bytes(&link.signature, "signature")?;
        let message =
            crypto::ident_rotation_message(owner, &crypto::fingerprint(&old_key), &new_key);
        crypto::verify(&old_key, &message, &signature)
            .map_err(|_| "invalid ident chain link signature".to_string())?;
        path.push(new_key);
    }
    Ok(path)
}

/// Revoke a (lost) machine's auth key. Authority is the ident key alone: the
/// request signs a server challenge plus the fingerprint being revoked with
/// the local ident private key; no session is required.
pub fn revoke_machine(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    auth_fingerprint: &str,
) -> Result<RevokeResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, owner)?;
    let challenge = post_json::<ChallengeResponse>(
        repo_url,
        "/machines/revoke/challenge",
        &RevokeChallengeRequest {
            owner: owner.to_string(),
        },
    )?;
    let nonce = crypto::decode_bytes(&challenge.nonce, "nonce")?;
    let message = crypto::revocation_message(&challenge.challenge_id, &nonce, auth_fingerprint);
    let signature = crypto::sign(&ident_private, &message)?;
    post_json::<RevokeResponse>(
        repo_url,
        "/machines/revoke",
        &RevokeRequest {
            challenge_id: challenge.challenge_id,
            auth_fingerprint: auth_fingerprint.to_string(),
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// Fetch the signed log checkpoint, verify it under the pinned server key,
/// and enforce append-only growth against the locally pinned checkpoint
/// (plan-23-B3): a shrunken tree, or a different root at the same size, is a
/// hard error — never silently re-pinned.
/// Fetch and verify the signed checkpoint **without** pinning it.
///
/// Splitting the pin out matters for `verify_log_consistency` (bug-276 R2):
/// pinning the new head before the consistency proof is checked means a detected
/// fork has already overwritten the pin it would be detected against, so the
/// evidence self-destructs after a single error and the next call succeeds
/// against the forked history.
fn fetch_checkpoint_unpinned(
    repo_url: &str,
    paths: &LocalPaths,
) -> Result<CheckpointResponse, String> {
    let server_key = ensure_server_key(repo_url, paths)?;
    let checkpoint = get_json::<CheckpointResponse>(repo_url, "/log/checkpoint")?;
    let root = decode_hex32(&checkpoint.root_hash, "rootHash")?;
    let signature = crypto::decode_bytes(&checkpoint.signature, "signature")?;
    crypto::verify(
        &server_key,
        &crate::log::checkpoint_signing_input(checkpoint.size as u64, &root),
        &signature,
    )
    .map_err(|_| "log checkpoint does not verify under the pinned server key".to_string())?;
    if let Some((pinned_size, pinned_root)) = local::read_checkpoint(paths)? {
        if checkpoint.size < pinned_size {
            return Err(format!(
                "registry log ROLLBACK: checkpoint size {} is smaller than the pinned size {pinned_size}",
                checkpoint.size
            ));
        }
        if checkpoint.size == pinned_size && checkpoint.root_hash != pinned_root {
            return Err(
                "registry log FORK: checkpoint root differs from the pinned root at the same size"
                    .to_string(),
            );
        }
    }
    Ok(checkpoint)
}

/// Fetch, verify and pin the current checkpoint.
///
/// This is the **only** primitive that advances the local pin, and every advance
/// past an existing pin is gated on an RFC 6962 consistency proof (bug-582).
/// Signatures and monotonicity alone cannot establish an append-only history: a
/// registry holding its online key can sign any larger tree it likes, so a
/// bigger head is a *candidate*, never evidence. Before the gate moved here,
/// `fetch_checkpoint` wrote any `size > pinned_size` straight over the pin,
/// which erased the root the fork would have been detected against — and
/// `verify_publish_inclusion`, the whole of `pkg install --proof`'s contact with
/// the log, advanced through it with no consistency check anywhere in the flow.
pub fn fetch_checkpoint(repo_url: &str, paths: &LocalPaths) -> Result<CheckpointResponse, String> {
    // Read the pin BEFORE the fetch, so the value proven against is the one the
    // client already accepted and not anything this exchange may have written.
    let pinned = local::read_checkpoint(paths)?;
    // Deliberately unpinned (bug-276 R2): the candidate head must not overwrite
    // the pin until its consistency against that pin has been proven, or a fork
    // erases the very evidence it would be caught by.
    let checkpoint = fetch_checkpoint_unpinned(repo_url, paths)?;
    let Some((pinned_size, pinned_root)) = pinned else {
        // Trust on first use: there is no predecessor to prove an extension of.
        local::write_checkpoint(paths, checkpoint.size, &checkpoint.root_hash)?;
        return Ok(checkpoint);
    };
    if checkpoint.size == pinned_size {
        // `fetch_checkpoint_unpinned` has already refused every other root at
        // this size, so this IS the pinned head. Nothing advances, so there is
        // nothing to prove and no proof to ask the registry for.
        return Ok(checkpoint);
    }
    // Strictly larger — a smaller size was refused above as a ROLLBACK.
    let proof = get_json::<ConsistencyProofResponse>(
        repo_url,
        &format!("/log/consistency?from={pinned_size}&to={}", checkpoint.size),
    )?;
    let old_root = decode_hex32(&pinned_root, "pinned root")?;
    let new_root = decode_hex32(&checkpoint.root_hash, "rootHash")?;
    let mut path = Vec::new();
    for node in &proof.path {
        path.push(decode_hex32(node, "proof node")?);
    }
    crate::log::verify_consistency(
        pinned_size as usize,
        checkpoint.size as usize,
        &old_root,
        &new_root,
        &path,
    )?;
    // Proven an extension of the pinned history — only now advance the pin.
    local::write_checkpoint(paths, checkpoint.size, &checkpoint.root_hash)?;
    Ok(checkpoint)
}

/// The canonical `publish` log payload, byte-for-byte as the registry writes it
/// in `store.rs` (`{"ident":…,"version":…,"hash":…}`, `serde_json`-escaped).
///
/// The client reconstructs this from values it already knows so the leaf can be
/// bound to a specific package; see `verify_publish_inclusion`. It must track
/// `store.rs`'s encoding exactly — a drift in field order or escaping turns every
/// honest inclusion check into a failure.
fn publish_leaf_payload(ident: &str, version: &str, content_hash: &str) -> String {
    let encode = |value: &str| serde_json::to_string(value).expect("JSON string encoding is total");
    format!(
        "{{\"ident\":{},\"version\":{},\"hash\":{}}}",
        encode(ident),
        encode(version),
        encode(content_hash),
    )
}

/// Verify that the publish of `ident@version` with content hash
/// `expected_content_hash` is included in the registry log under the current
/// (verified, rollback-checked) checkpoint. Returns the log entry and the
/// checkpoint it verified against.
pub fn verify_publish_inclusion(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    version: &str,
    expected_content_hash: &str,
) -> Result<(LogEntry, CheckpointResponse), String> {
    let checkpoint = fetch_checkpoint(repo_url, paths)?;
    let entry = get_json::<LogEntry>(
        repo_url,
        &format!(
            "/log/publish?ident={}&version={}",
            percent_encode(ident),
            percent_encode(version)
        ),
    )?;
    let proof = get_json::<InclusionProofResponse>(
        repo_url,
        &format!("/log/proof/{}?size={}", entry.index, checkpoint.size),
    )?;
    if proof.index != entry.index || proof.size != checkpoint.size {
        return Err("inclusion proof does not match the requested entry".to_string());
    }
    let leaf = decode_hex32(&entry.leaf_hash, "leafHash")?;
    if decode_hex32(&proof.leaf_hash, "leafHash")? != leaf {
        return Err("inclusion proof leaf does not match the publish entry".to_string());
    }
    // Bind the leaf to *this* package (bug-273). Proving the served leaf is in the
    // tree says nothing about which entry it is: the client knows ident, version
    // and content hash but previously verified none of them, so a hostile registry
    // could answer with the (index, leafHash) of any genuine entry — its own
    // `register`, an unrelated publish — and the RFC-6962 math would check out,
    // reporting "inclusion verified" for a publish that never happened.
    //
    // The payload is rebuilt from values the client independently holds; hashing a
    // server-supplied payload instead would leave the registry free to choose what
    // the leaf describes.
    let expected_leaf = crate::log::leaf_hash(
        publish_leaf_payload(ident, version, expected_content_hash).as_bytes(),
    );
    if expected_leaf != leaf {
        return Err(format!(
            "publish log leaf does not match {ident}@{version}: the registry served \
             an entry for different contents"
        ));
    }
    let root = decode_hex32(&checkpoint.root_hash, "rootHash")?;
    let mut path = Vec::new();
    for node in &proof.path {
        path.push(decode_hex32(node, "proof node")?);
    }
    crate::log::verify_inclusion(
        entry.index as usize,
        checkpoint.size as usize,
        &leaf,
        &path,
        &root,
    )?;
    Ok((entry, checkpoint))
}

/// Fetch and verify a consistency proof between the pinned checkpoint and
/// the current one.
///
/// Since bug-582 this is exactly `fetch_checkpoint`: the consistency gate lives
/// in the one pin-advance primitive rather than in one of its two callers, so a
/// call site cannot pick the unsafe half by accident. The name is kept because
/// it says at the call site *why* the log is being contacted.
pub fn verify_log_consistency(
    repo_url: &str,
    paths: &LocalPaths,
) -> Result<CheckpointResponse, String> {
    fetch_checkpoint(repo_url, paths)
}

fn decode_hex32(value: &str, field: &str) -> Result<[u8; 32], String> {
    let raw = hex::decode(value).map_err(|_| format!("malformed {field}"))?;
    if raw.len() != 32 {
        return Err(format!("malformed {field}"));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
}

fn percent_encode(value: &str) -> String {
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

/// The root-delegated keys and index state recovered from a verified metadata
/// chain (plan-10-C2).
#[derive(Debug)]
pub struct DelegatedMetadata {
    /// The root-delegated server (attestation) key. A consumer refuses any
    /// attestation not signed by this exact key.
    pub server_key: Vec<u8>,
    /// The verified `root.json` version (bug-584). Pinned locally and never
    /// allowed to go backwards, so a renewal cannot be rolled back.
    pub root_version: i64,
    pub snapshot_version: i64,
    pub index_hash: String,
}

/// Verify the signed-metadata chain (plan-10-C2): root → timestamp → snapshot.
/// Rejects a bad root fingerprint, a registry-id mismatch, expired metadata,
/// an undelegated key, a snapshot/timestamp that disagree, and a version
/// rollback below `min_root_version` or `min_snapshot_version`. `now` is passed
/// in for testability.
///
/// The root version floor is bug-584: root renewal rotates the delegated online
/// keys under the same anchor, so an old-but-unexpired `root.json` — correctly
/// signed by the very key the client pinned — would otherwise resurrect the
/// snapshot/timestamp keys a renewal retired.
pub fn verify_registry_metadata(
    root: &RootResponse,
    timestamp: &SignedMetadataResponse,
    snapshot: &SignedMetadataResponse,
    expected_registry_id: &str,
    pinned_root_fingerprint: &str,
    min_root_version: i64,
    min_snapshot_version: i64,
    now: i64,
) -> Result<DelegatedMetadata, String> {
    // Root: the pinned fingerprint is the sole trust anchor.
    let root_key = crypto::decode_bytes(&root.root_key, "rootKey")?;
    if crypto::fingerprint(&root_key) != pinned_root_fingerprint {
        return Err("registry root key does not match the pinned root fingerprint".to_string());
    }
    let root_signature = crypto::decode_bytes(&root.signature, "root signature")?;
    crypto::verify(
        &root_key,
        &crypto::root_signing_input(root.signed.as_bytes()),
        &root_signature,
    )
    .map_err(|_| "root.json signature does not verify under the root key".to_string())?;
    let root_doc = parse_metadata(&root.signed, "root.json")?;
    check_field(&root_doc, "registryId", expected_registry_id, "root.json")?;
    check_not_expired(&root_doc, now, "root.json")?;
    let root_version = metadata_i64(&root_doc, "version", "root.json")?;
    if root_version < min_root_version {
        return Err(format!(
            "metadata ROLLBACK: root version {root_version} is below the pinned version {min_root_version}"
        ));
    }
    let server_key = decode_delegated_key(&root_doc, "serverKey")?;
    let snapshot_key = decode_delegated_key(&root_doc, "snapshotKey")?;
    let timestamp_key = decode_delegated_key(&root_doc, "timestampKey")?;

    // Timestamp: signed by the delegated timestamp key, fresh, no rollback.
    let timestamp_signature = crypto::decode_bytes(&timestamp.signature, "timestamp signature")?;
    crypto::verify(
        &timestamp_key,
        &crypto::timestamp_signing_input(timestamp.signed.as_bytes()),
        &timestamp_signature,
    )
    .map_err(|_| "timestamp.json signature does not verify under the delegated key".to_string())?;
    let timestamp_doc = parse_metadata(&timestamp.signed, "timestamp.json")?;
    check_field(
        &timestamp_doc,
        "registryId",
        expected_registry_id,
        "timestamp.json",
    )?;
    check_not_expired(&timestamp_doc, now, "timestamp.json")?;
    let snapshot_version = metadata_i64(&timestamp_doc, "snapshotVersion", "timestamp.json")?;
    if snapshot_version < min_snapshot_version {
        return Err(format!(
            "metadata ROLLBACK: snapshot version {snapshot_version} is below the pinned version {min_snapshot_version}"
        ));
    }
    let timestamp_index_hash = metadata_str(&timestamp_doc, "indexHash", "timestamp.json")?;

    // Snapshot: signed by the delegated snapshot key, fresh, and agrees with
    // the timestamp on version + index hash.
    let snapshot_signature = crypto::decode_bytes(&snapshot.signature, "snapshot signature")?;
    crypto::verify(
        &snapshot_key,
        &crypto::snapshot_signing_input(snapshot.signed.as_bytes()),
        &snapshot_signature,
    )
    .map_err(|_| "snapshot.json signature does not verify under the delegated key".to_string())?;
    let snapshot_doc = parse_metadata(&snapshot.signed, "snapshot.json")?;
    check_field(
        &snapshot_doc,
        "registryId",
        expected_registry_id,
        "snapshot.json",
    )?;
    check_not_expired(&snapshot_doc, now, "snapshot.json")?;
    let snapshot_doc_version = metadata_i64(&snapshot_doc, "version", "snapshot.json")?;
    let snapshot_index_hash = metadata_str(&snapshot_doc, "indexHash", "snapshot.json")?;
    if snapshot_doc_version != snapshot_version {
        return Err("timestamp and snapshot disagree on the snapshot version".to_string());
    }
    if snapshot_index_hash != timestamp_index_hash {
        return Err("timestamp and snapshot disagree on the index hash".to_string());
    }

    Ok(DelegatedMetadata {
        server_key,
        root_version,
        snapshot_version,
        index_hash: snapshot_index_hash,
    })
}

fn parse_metadata(signed: &str, what: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(signed).map_err(|_| format!("malformed {what}"))
}

fn check_field(
    doc: &serde_json::Value,
    field: &str,
    expected: &str,
    what: &str,
) -> Result<(), String> {
    if doc.get(field).and_then(|value| value.as_str()) != Some(expected) {
        return Err(format!("{what} {field} does not match the expected value"));
    }
    Ok(())
}

fn check_not_expired(doc: &serde_json::Value, now: i64, what: &str) -> Result<(), String> {
    let expires = doc
        .get("expires")
        .and_then(|value| value.as_i64())
        .ok_or_else(|| format!("{what} is missing an expiry"))?;
    if expires <= now {
        return Err(format!("{what} has expired"));
    }
    Ok(())
}

fn metadata_i64(doc: &serde_json::Value, field: &str, what: &str) -> Result<i64, String> {
    doc.get(field)
        .and_then(|value| value.as_i64())
        .ok_or_else(|| format!("{what} is missing {field}"))
}

fn metadata_str(doc: &serde_json::Value, field: &str, what: &str) -> Result<String, String> {
    doc.get(field)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{what} is missing {field}"))
}

fn decode_delegated_key(doc: &serde_json::Value, field: &str) -> Result<Vec<u8>, String> {
    let encoded = doc
        .get(field)
        .and_then(|value| value.as_str())
        .ok_or_else(|| format!("root.json is missing {field}"))?;
    crypto::decode_bytes(encoded, field)
}

/// `mfb repo trust`: fetch and verify the metadata chain, cross-check that the
/// pinned server key is the root-delegated one, and pin the registry id + root
/// fingerprint. Returns the pinned snapshot version.
pub fn trust_registry(
    repo_url: &str,
    paths: &LocalPaths,
    registry_id: &str,
    root_fingerprint: &str,
) -> Result<i64, String> {
    let server_key = ensure_server_key(repo_url, paths)?;
    let delegated = fetch_and_verify_metadata(repo_url, registry_id, root_fingerprint, 0, 0)?;
    if delegated.server_key != server_key {
        return Err(
            "registry attestation key is not delegated by the pinned root; refusing to trust"
                .to_string(),
        );
    }
    local::write_root_pin(paths, registry_id, root_fingerprint)?;
    local::write_root_version(paths, delegated.root_version)?;
    local::write_snapshot_version(paths, delegated.snapshot_version)?;
    Ok(delegated.snapshot_version)
}

/// Verify the pinned metadata chain before trusting the registry, when (and
/// only when) a root has been pinned via `mfb repo trust` — the signed-metadata
/// layer is opt-in on top of the plan-23 pinned-server-key anchor. Advances the
/// pinned snapshot version on success; rejects a rollback.
pub fn verify_pinned_metadata(repo_url: &str, paths: &LocalPaths) -> Result<(), String> {
    let Some((registry_id, root_fingerprint)) = local::read_root_pin(paths)? else {
        return Ok(());
    };
    let server_key = ensure_server_key(repo_url, paths)?;
    let min_root = local::read_root_version(paths)?.unwrap_or(0);
    let min = local::read_snapshot_version(paths)?.unwrap_or(0);
    let delegated =
        fetch_and_verify_metadata(repo_url, &registry_id, &root_fingerprint, min_root, min)?;
    if delegated.server_key != server_key {
        return Err(
            "registry attestation key is not delegated by the pinned root; refusing to trust"
                .to_string(),
        );
    }
    // A root renewal advances this; a client pinned before the renewal follows
    // it, and the retired root can never be replayed back over it (bug-584).
    local::write_root_version(paths, delegated.root_version)?;
    local::write_snapshot_version(paths, delegated.snapshot_version)?;
    Ok(())
}

fn fetch_and_verify_metadata(
    repo_url: &str,
    registry_id: &str,
    root_fingerprint: &str,
    min_root_version: i64,
    min_snapshot_version: i64,
) -> Result<DelegatedMetadata, String> {
    let root = get_json::<RootResponse>(repo_url, "/root.json")?;
    let timestamp = get_json::<SignedMetadataResponse>(repo_url, "/timestamp.json")?;
    let snapshot = get_json::<SignedMetadataResponse>(repo_url, "/snapshot.json")?;
    verify_registry_metadata(
        &root,
        &timestamp,
        &snapshot,
        registry_id,
        root_fingerprint,
        min_root_version,
        min_snapshot_version,
        crate::store::now_unix(),
    )
}

/// `POST /orgs/members` (plan-10-D1): grant or remove a member's org role,
/// authorized by the grantor's local ident key + session.
pub fn set_org_member(
    repo_url: &str,
    paths: &LocalPaths,
    org: &str,
    grantor: &str,
    member: &str,
    role: &str,
    remove: bool,
) -> Result<OrgMemberResponse, String> {
    validate_owner_name(grantor)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, grantor)?;
    let session_token = local::read_session(paths, grantor)?;
    let role_in_message = if remove { "removed" } else { role };
    let signature = crypto::sign(
        &ident_private,
        &crypto::org_role_message(org, member, role_in_message),
    )?;
    post_json::<OrgMemberResponse>(
        repo_url,
        "/orgs/members",
        &OrgMemberRequest {
            org: org.to_string(),
            grantor: grantor.to_string(),
            member: member.to_string(),
            role: role.to_string(),
            action: if remove { "remove" } else { "grant" }.to_string(),
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// `POST /tokens` (plan-10-D1): issue a scoped publish token. Generates the
/// token keypair locally, registers its public key, and returns the response
/// plus the token private key (base64url) for the operator to deploy to CI.
pub fn issue_publish_token(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    scope: &str,
    ttl_seconds: i64,
) -> Result<(TokenIssueResponse, String), String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;
    let (token_public, token_private) = crypto::generate_keypair();
    let token_fingerprint = crypto::fingerprint(&token_public);
    let proof = crypto::sign(
        &token_private,
        &crypto::registration_message(crypto::ROLE_AUTH, owner, &token_public),
    )?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::token_issue_message(owner, &token_fingerprint, scope),
    )?;
    let response = post_json::<TokenIssueResponse>(
        repo_url,
        "/tokens",
        &TokenIssueRequest {
            owner: owner.to_string(),
            token_key: crypto::encode_bytes(&token_public),
            proof: crypto::encode_bytes(&proof),
            scope: scope.to_string(),
            ttl_seconds,
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )?;
    Ok((response, crypto::encode_bytes(&token_private)))
}

/// `POST /tokens/revoke` (plan-10-D1): revoke a publish token by fingerprint.
pub fn revoke_publish_token(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    token_fingerprint: &str,
) -> Result<TokenRevokeResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::token_revoke_message(owner, token_fingerprint),
    )?;
    post_json::<TokenRevokeResponse>(
        repo_url,
        "/tokens/revoke",
        &TokenRevokeRequest {
            owner: owner.to_string(),
            token_fingerprint: token_fingerprint.to_string(),
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// Verify a server-signed name binding (plan-168-A) under `server_key` and
/// return the bound ident public key: the key must hash to the bound
/// fingerprint and the signature must cover `name_binding_message(owner,
/// fingerprint)`.
pub fn verify_owner_binding(server_key: &[u8], binding: &OwnerBinding) -> Result<Vec<u8>, String> {
    let public = crypto::decode_bytes(
        binding
            .ident_key
            .strip_prefix("ed25519:")
            .unwrap_or(&binding.ident_key),
        "identKey",
    )?;
    if crypto::fingerprint(&public) != binding.ident_fingerprint {
        return Err(format!(
            "registry binding for {} names a key that does not match its fingerprint",
            sanitize_server_text(&binding.owner)
        ));
    }
    let signature = crypto::decode_bytes(&binding.name_binding_signature, "nameBindingSignature")?;
    crypto::verify(
        server_key,
        &crypto::name_binding_message(&binding.owner, &binding.ident_fingerprint),
        &signature,
    )
    .map_err(|_| {
        format!(
            "registry binding for {} does not verify under the registry server key",
            sanitize_server_text(&binding.owner)
        )
    })?;
    Ok(public)
}

/// The package's current owner as a verified `/index` states it (plan-168-A
/// §4.5): the last transfer's `toBinding`, or the index's own binding for a
/// package that never moved. Returns `(owner display, ident fingerprint)`.
fn current_owner_binding(
    server_key: &[u8],
    index: &IndexResponse,
) -> Result<(String, String), String> {
    match index.transfers.last() {
        Some(link) => {
            verify_owner_binding(server_key, &link.to_binding)?;
            Ok((
                link.to_binding.owner.clone(),
                link.to_binding.ident_fingerprint.clone(),
            ))
        }
        None => Ok((index.owner.clone(), index.ident_fingerprint.clone())),
    }
}

/// The v2 transfer terms the next link of an ident signs (plan-168-A §4.5),
/// computed from a verified `/index` exactly as the registry computes them
/// from its tables: the sequence is the number of served links and the digest
/// covers every served version.
struct ClientTransferTerms {
    current_owner: String,
    current_fingerprint: String,
    sequence: i64,
    release_digest: String,
    to_owner: String,
}

fn client_transfer_terms(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    to_owner: &str,
) -> Result<ClientTransferTerms, String> {
    let Some((owner, package)) = ident.split_once('#') else {
        return Err("ident must use <owner>#<package>".to_string());
    };
    validate_owner_name(to_owner)?;
    let server_key = ensure_server_key(repo_url, paths)?;
    verify_pinned_metadata(repo_url, paths)?;
    let index = fetch_index_with_key(repo_url, &server_key, owner, package)?;
    let (current_owner, current_fingerprint) = current_owner_binding(&server_key, &index)?;
    // The signed bytes carry the recipient's REGISTERED display name, which is
    // what the registry records and `/index` later serves.
    let recipient = fetch_ident_chain(repo_url, to_owner)?.owner;
    Ok(ClientTransferTerms {
        current_owner,
        current_fingerprint,
        sequence: i64::try_from(index.transfers.len())
            .map_err(|_| "registry index serves too many transfers".to_string())?,
        release_digest: crypto::release_digest(
            index
                .versions
                .iter()
                .map(|version| (version.version.as_str(), version.hash.as_str())),
        ),
        to_owner: recipient,
    })
}

/// `POST /packages/transfer/offer` (plan-10-D1, v2 since plan-168-A): the
/// package's CURRENT owner — the last transfer's recipient, or the ident
/// prefix owner for a package that never moved — offers it to a recipient,
/// signing the v2 terms read from `/index` with the local ident key.
pub fn transfer_offer(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    to_owner: &str,
) -> Result<TransferResponse, String> {
    let terms = client_transfer_terms(repo_url, paths, ident, to_owner)?;
    validate_owner_name(&terms.current_owner)?;
    let ident_private = local::read_ident_private_key(paths, &terms.current_owner)?;
    if crypto::fingerprint(&crypto::public_from_private(&ident_private)?)
        != terms.current_fingerprint
    {
        return Err(format!(
            "this machine does not hold {}'s current ident",
            terms.current_owner
        ));
    }
    let session_token = local::read_session(paths, &terms.current_owner)?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::transfer_offer_message_v2(crypto::TransferTermsRef {
            ident,
            sequence: terms.sequence,
            from_owner: &terms.current_owner,
            from_ident_fingerprint: &terms.current_fingerprint,
            to_owner: &terms.to_owner,
            release_digest: &terms.release_digest,
        }),
    )?;
    post_json::<TransferResponse>(
        repo_url,
        "/packages/transfer/offer",
        &TransferOfferRequest {
            ident: ident.to_string(),
            from_owner: terms.current_owner,
            to_owner: terms.to_owner,
            sequence: terms.sequence,
            release_digest: terms.release_digest,
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// `POST /packages/transfer/accept` (plan-10-D1, v2 since plan-168-A): the
/// recipient accepts a pending offer by signing the same v2 terms, read from
/// `/index`, with the local ident key.
pub fn transfer_accept(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    to_owner: &str,
) -> Result<TransferResponse, String> {
    let terms = client_transfer_terms(repo_url, paths, ident, to_owner)?;
    let ident_private = local::read_ident_private_key(paths, to_owner)?;
    let session_token = local::read_session(paths, to_owner)?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::transfer_accept_message_v2(crypto::TransferTermsRef {
            ident,
            sequence: terms.sequence,
            from_owner: &terms.current_owner,
            from_ident_fingerprint: &terms.current_fingerprint,
            to_owner: &terms.to_owner,
            release_digest: &terms.release_digest,
        }),
    )?;
    post_json::<TransferResponse>(
        repo_url,
        "/packages/transfer/accept",
        &TransferAcceptRequest {
            ident: ident.to_string(),
            to_owner: to_owner.to_string(),
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// `POST /release-state` (plan-10-C1): a maintainer sets a published version's
/// release state. Signed with the local ident key (authority) and carrying the
/// session token (auth); an auth session alone is refused server-side.
pub fn set_release_state(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    ident: &str,
    version: &str,
    state: &str,
) -> Result<ReleaseStateResponse, String> {
    validate_owner_name(owner)?;
    ensure_server_key(repo_url, paths)?;
    let ident_private = local::read_ident_private_key(paths, owner)?;
    let session_token = local::read_session(paths, owner)?;
    let signature = crypto::sign(
        &ident_private,
        &crypto::release_state_message(ident, version, state),
    )?;
    post_json::<ReleaseStateResponse>(
        repo_url,
        "/release-state",
        &ReleaseStateRequest {
            owner: owner.to_string(),
            ident: ident.to_string(),
            version: version.to_string(),
            state: state.to_string(),
            session_token,
            ident_signature: crypto::encode_bytes(&signature),
        },
    )
}

/// `GET /index/<owner>#<package>` (plan-10-A): fetch the published version
/// list plus the owner's current ident key. The server-signed name binding is
/// verified under the pinned server key, so the returned `identKey` is a
/// registry-authenticated anchor a first `mfb pkg add` can pin.
pub fn fetch_index(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    package: &str,
) -> Result<IndexResponse, String> {
    validate_owner_name(owner)?;
    let server_key = ensure_server_key(repo_url, paths)?;
    // If a signed-metadata root is pinned (plan-10-C2), the chain must verify
    // and delegate this server key before we trust anything the index says.
    verify_pinned_metadata(repo_url, paths)?;
    fetch_index_with_key(repo_url, &server_key, owner, package)
}

/// [`fetch_index`] against a server key the caller already trusts, with no local
/// state: the route binding, the identKey/fingerprint cross-check and the
/// name-binding signature are all checked under `server_key`.
pub fn fetch_index_with_key(
    repo_url: &str,
    server_key: &[u8],
    owner: &str,
    package: &str,
) -> Result<IndexResponse, String> {
    validate_owner_name(owner)?;
    let ident = format!("{owner}#{package}");
    let response =
        get_json::<IndexResponse>(repo_url, &format!("/index/{}", percent_encode(&ident)))?;
    // bug-581: bind the response to the route BEFORE anything in it is trusted.
    //
    // Every signature check below is self-referential: the name binding is
    // verified over `name_binding_message(response.owner, response.ident_fingerprint)`,
    // i.e. over values the response itself supplies. It therefore proves the
    // registry signed *something*, never that it answered the question that was
    // asked. A registry holding its online server key could return a validly
    // signed binding and version list for a different package entirely and the
    // client would return it as the requested index.
    //
    // The ident is compared exactly because the registry echoes the requested
    // ident verbatim (`server.rs:package_index` returns the raw path parameter),
    // so this is a true route binding and not a guess about normalization.
    if response.ident != ident {
        return Err(format!(
            "registry index is for a different package: requested {ident}, served {}",
            sanitize_server_text(&response.ident)
        ));
    }
    // The owner, by contrast, is compared CASE-FOLDED. The registry resolves
    // owners by `owner_folded` and answers with `owner_display`, so `Alice#pkg`
    // is a legitimate request whose honest answer says `alice`. An exact
    // comparison here would refuse valid input — the failure mode this repo has
    // shipped four times. `fold_owner` is the registry's own rule, not a
    // second, drifting copy of it.
    if fold_owner(&response.owner) != fold_owner(owner) {
        return Err(format!(
            "registry index names a different owner: requested {owner}, served {}",
            sanitize_server_text(&response.owner)
        ));
    }
    // The pinned ident is only as trustworthy as the name binding: verify it
    // under the pinned server key and cross-check the fingerprint.
    let ident_public = crypto::decode_bytes(
        response
            .ident_key
            .strip_prefix("ed25519:")
            .unwrap_or(&response.ident_key),
        "identKey",
    )?;
    if crypto::fingerprint(&ident_public) != response.ident_fingerprint {
        return Err("registry index identKey does not match its fingerprint".to_string());
    }
    let signature = crypto::decode_bytes(&response.name_binding_signature, "nameBindingSignature")?;
    crypto::verify(
        server_key,
        &crypto::name_binding_message(&response.owner, &response.ident_fingerprint),
        &signature,
    )
    .map_err(|_| {
        "registry name binding does not verify under the registry server key".to_string()
    })?;
    Ok(response)
}

/// `GET /blob/<hash>` (plan-10-A): download a content-addressed `.mfp` blob and
/// verify its bytes hash to the requested hash before returning them.
pub fn fetch_blob(repo_url: &str, hash: &str) -> Result<Vec<u8>, String> {
    ensure_transport_security(repo_url)?;
    let url = format!("{}/blob/{}", repo_url.trim_end_matches('/'), hash);
    // Blob bodies are orders of magnitude larger than any control-plane payload,
    // so they get their own deadline; the client default would reject a slow but
    // perfectly healthy transfer. The override rides into the `Response`, so it
    // covers the body read below and not just the headers.
    let response = http_client()?
        .get(&url)
        .timeout(BLOB_TIMEOUT)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = read_body_capped(response, MAX_JSON_BYTES, "repository error body")
            .unwrap_or_else(|_| b"repository request failed".to_vec());
        let text = String::from_utf8_lossy(&body);
        if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
            // bug-489: server-authored; sanitize at the trust boundary.
            return Err(sanitize_server_text(&error.error));
        }
        return Err(format!(
            "repository request failed with status {status}: {}",
            sanitize_server_text(&text)
        ));
    }
    let bytes = read_body_capped(response, MAX_BLOB_BYTES, "blob body")?;
    if hex::encode(crypto::sha256(&bytes)) != hash {
        return Err("downloaded blob does not match the requested content hash".to_string());
    }
    Ok(bytes)
}

/// `HEAD /blob/<hash>` (plan-48-B §4.2): probe whether the registry already
/// holds a blob for `hash`. The dedup win on publish — an unchanged vendored
/// library uploads once, ever, across every version that ships it.
pub fn blob_exists(repo_url: &str, hash: &str) -> Result<bool, String> {
    ensure_transport_security(repo_url)?;
    let url = format!("{}/blob/{}", repo_url.trim_end_matches('/'), hash);
    let response = http_client()?
        .head(&url)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    let status = response.status();
    if status.is_success() {
        Ok(true)
    } else if status == reqwest::StatusCode::NOT_FOUND {
        Ok(false)
    } else {
        Err(format!(
            "repository HEAD /blob request failed with status {status}"
        ))
    }
}

/// `PUT /blob/<hash>` (plan-48-B §4.2): upload raw native-library bytes,
/// authenticated with the session token in an `Authorization: Bearer` header —
/// the one header-borne credential on this server (plan-48-A §4.3), because a
/// raw-body request cannot carry the body-field `sessionToken`. The server
/// re-hashes the body before storing, so a corrupt upload is rejected there and
/// the client need not verify a response.
pub fn put_blob(
    repo_url: &str,
    hash: &str,
    bytes: Vec<u8>,
    session_token: &str,
) -> Result<(), String> {
    ensure_transport_security(repo_url)?;
    let url = format!("{}/blob/{}", repo_url.trim_end_matches('/'), hash);
    // Blob uploads share the download's generous deadline — a large library
    // needs far more than the control-plane 30s to move (plan-48-B §4.1).
    // bug-490: no documented redirect on this route. The bearer header WOULD be
    // stripped by reqwest on a cross-host hop, but the body — the raw native
    // library bytes — would be replayed to whatever origin the registry names,
    // so this does not follow one either.
    let response = no_redirect_client()?
        .put(&url)
        .bearer_auth(session_token)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .timeout(BLOB_TIMEOUT)
        .body(bytes)
        .send()
        .map_err(|err| format!("failed to connect to repository service: {err}"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let text = response
        .text()
        .unwrap_or_else(|_| "repository request failed".to_string());
    if let Ok(error) = serde_json::from_str::<ErrorResponse>(&text) {
        // bug-489: the server authored this string. Sanitize it HERE, at the
        // trust boundary, not at the terminal — see `sanitize_server_text`.
        return Err(sanitize_server_text(&error.error));
    }
    Err(format!(
        "repository PUT /blob request failed with status {status}: {}",
        sanitize_server_text(&text)
    ))
}

pub struct PackageArtifact<'a> {
    pub ident: &'a str,
    pub version: &'a str,
    pub artifact: &'a [u8],
    pub content_hash: &'a str,
    pub ident_fingerprint: &'a str,
    pub signing_fingerprint: &'a str,
}

pub fn validate_package(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    package: &PackageArtifact<'_>,
) -> Result<ValidatePackageResponse, String> {
    validate_owner_name(owner)?;
    let session_token = local::read_session(paths, owner)?;
    post_json::<ValidatePackageResponse>(
        repo_url,
        "/validate",
        &package_request(package, session_token),
    )
}

pub fn publish_package(
    repo_url: &str,
    paths: &LocalPaths,
    owner: &str,
    package: &PackageArtifact<'_>,
) -> Result<PublishPackageResponse, String> {
    validate_owner_name(owner)?;
    let session_token = local::read_session(paths, owner)?;
    post_json::<PublishPackageResponse>(
        repo_url,
        "/publish",
        &package_request(package, session_token),
    )
}

fn package_request(package: &PackageArtifact<'_>, session_token: String) -> PackageArtifactRequest {
    PackageArtifactRequest {
        ident: package.ident.to_string(),
        version: package.version.to_string(),
        artifact: crypto::encode_bytes(package.artifact),
        content_hash: package.content_hash.to_string(),
        ident_fingerprint: package.ident_fingerprint.to_string(),
        signing_fingerprint: package.signing_fingerprint.to_string(),
        session_token,
    }
}

fn post_json<T: DeserializeOwned>(
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

fn get_json<T: DeserializeOwned>(repo_url: &str, path: &str) -> Result<T, String> {
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
fn read_body_capped(
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
fn sanitize_server_text(text: &str) -> String {
    crate::terminal_safe::safe(text).into_owned()
}

#[cfg(test)]
mod tests;
