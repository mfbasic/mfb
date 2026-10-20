//! plan-176-I: `client::tests` — the transport tests. The shared metadata fixtures and the\n//! in-process HTTP stub live in `client/tests/mod.rs`.

use super::*;

/// bug-490: a credential-bearing request must not follow a redirect at all.
///
/// This document was filed "not demonstrated end to end", on the grounds that
/// the redirect guard requires an https target so a loopback harness cannot
/// drive it. That is escapable: at the time, `ensure_redirect_target` blocked
/// IP LITERALS only, so `https://localhost:<port>/` passed the guard. The hop
/// therefore gets attempted for real, and the attempt is observable in the
/// error.
///
/// bug-585 has since closed that hole in the SHARED client's policy, but this
/// test is unaffected and still measures what it always did: a
/// credential-bearing POST runs on `no_redirect_client`, which consults no
/// policy at all. That independence is the point — the guarantee here is
/// "never follows a redirect", not "follows only safe ones".
///
/// The target port has nothing listening, deliberately: pointing it at a stub
/// would make the client open a TLS handshake against a plain-HTTP socket and
/// the stub's `read_request` would block waiting for a `\r\n\r\n` that never
/// comes. A dead port fails immediately and still proves the hop was tried.
///
/// BEFORE the fix `post_json` followed the 307 and the error came from
/// connecting to the attacker origin. AFTER, the 307 is surfaced as an
/// ordinary status and no connection is made — the session token in the body
/// never leaves the configured registry.
#[test]
fn a_credentialed_post_does_not_follow_a_cross_origin_redirect() {
    let registry = spawn_raw(|_request| {
        // Port 1 is reserved and never listening; `localhost` is a HOSTNAME,
        // so it is not caught by the IP-literal guard.
        b"HTTP/1.1 307 Temporary Redirect\r\nLocation: https://localhost:1/steal\r\n\
              Content-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec()
    });

    let err = post_json::<serde_json::Value>(
        &registry.url,
        "/publish",
        &serde_json::json!({ "sessionToken": "s3cret" }),
    )
    .expect_err("a 307 is not a successful control-plane response");

    assert!(
        err.contains("307"),
        "the redirect must be surfaced as a status, not followed. \
             A message about connecting to localhost means the hop was taken and \
             the body — session token and all — went with it. Got: {err}"
    );
    assert!(
        !err.contains("localhost"),
        "the client must never have contacted the redirect target: {err}"
    );
}

/// The other half, and the bug's explicit non-goal: `GET /blob` still follows
/// a presigned-URL hop, so the fix must not have moved it onto the
/// no-redirect client.
///
/// Asserted by observing that the blob path still CONSULTS the redirect
/// policy — a plaintext hop is refused by `ensure_redirect_target` with its
/// own message. A no-redirect client would instead surface the raw 302.
#[test]
fn a_blob_get_still_consults_the_redirect_policy() {
    let internal = spawn_raw(|_request| http_reply("GET", 200, "{}"));
    let internal_url = internal.url.clone();
    let registry = spawn_raw(move |_request| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {internal_url}/blob\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    });

    let err = fetch_blob(&registry.url, &"0".repeat(64))
        .expect_err("a plaintext redirect target must be refused");
    assert!(
        err.contains("redirect"),
        "the blob path must still run the redirect policy (a presigned hop is \
             legitimate there); got: {err}"
    );
    assert!(
        internal.requests.lock().unwrap().is_empty(),
        "the refused target must never be contacted"
    );
}

/// bug-489: a registry-authored error string reaches the operator's terminal,
/// so it is sanitized where it is created.
///
/// The attack this blocks, measured end to end before the fix: a registry
/// answering `GET /ident` with
/// `{"error":"\x1b[2K\rok: uses toolbox - [Verified]  \u{202e}EVIL"}` printed
/// as `error: ^[[2K^Mok: uses toolbox - [Verified] …` — ESC-`[2K` erases the
/// line and CR returns to column 0, wiping the `error: ` prefix, so the
/// operator sees a forged `[Verified]` success. U+202E then reorders the
/// tail. Every one of those is escaped now.
#[test]
fn a_server_authored_error_is_escaped_at_the_boundary() {
    let hostile = "\u{1b}[2K\rok: uses toolbox - [Verified]  \u{202e}EVIL";
    let rendered = sanitize_server_text(hostile);
    assert_eq!(
        rendered,
        "\\u{001b}[2K\\u{000d}ok: uses toolbox - [Verified]  \\u{202e}EVIL"
    );
    // The three primitives the forgery needs are each inert.
    for needle in ["\u{1b}", "\r", "\u{202e}"] {
        assert!(
            !rendered.contains(needle),
            "{needle:?} still reaches the terminal verbatim"
        );
    }
}

/// A newline is escaped too, and deliberately: a server-authored `\n` forges
/// whole rows, which is the same forgery one line down. This is pinned
/// because it is exactly what makes sanitizing at the PRINT site wrong — see
/// `sanitize_server_text`'s note and
/// `tests/cli/cli_untrusted_registry_text_is_escaped.rs`'s
/// `a_multiline_usage_error_keeps_its_real_newlines`.
#[test]
fn a_server_authored_newline_is_escaped() {
    assert_eq!(
        sanitize_server_text("first\nerror: forged second"),
        "first\\u{000a}error: forged second"
    );
}

/// The positive pin. An ordinary server error must survive verbatim —
/// including non-ASCII text, which is not dangerous and must not be mangled.
/// A sanitizer that escapes valid input is its own defect (and this one is
/// on the path of every failed registry call).
#[test]
fn an_ordinary_server_error_is_unchanged() {
    for ordinary in [
        "package 'alice#toolbox' not found",
        "version 2.1.0 is already published",
        "insufficient scope: publisher required",
        "paquete no encontrado: café-naïve 日本語",
    ] {
        assert_eq!(
            sanitize_server_text(ordinary),
            ordinary,
            "an ordinary error must not be escaped"
        );
    }
}

/// `register` for an owner whose keys are already on this machine must refuse
/// without touching them (bug-272).
///
/// The old order was: generate → write (truncating) → POST → on any error,
/// `remove_owner_keys`. Registering an owner that already exists on the server
/// is *guaranteed* to hit that error path, so the original ident private key
/// was overwritten and then the replacement deleted. plan-23 makes the ident
/// key the account authority, so on a sole-holder machine that is a permanent
/// account lockout.
///
/// The stub has to let `GET /ident` succeed and fail only `/accounts/register`:
/// if the server-key fetch failed instead, `ensure_server_key`'s `?` would
/// return before the writes and the keys would survive by accident, testing
/// nothing. Serving `Connection: close` keeps the two requests on separate
/// accepts.
#[test]
fn register_refuses_and_preserves_keys_when_owner_keys_already_exist() {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::new(temp.path().join(".mfb"));

    // Pre-existing keys, as a prior `register` or `mfb repo link` would leave.
    let (auth_public, auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    local::write_auth_keypair(&paths, "alice", &auth_public, &auth_private).unwrap();
    local::write_ident_keypair(&paths, "alice", &ident_public, &ident_private).unwrap();

    let key_paths = [
        paths.auth_public_key_path("alice"),
        paths.auth_private_key_path("alice"),
        paths.ident_public_key_path("alice"),
        paths.ident_private_key_path("alice"),
    ];
    let before: Vec<Vec<u8>> = key_paths
        .iter()
        .map(|p| std::fs::read(p).expect("key file exists up front"))
        .collect();

    let (server_public, _) = crypto::generate_keypair();
    let ident_body = format!(
        "{{\"serverKey\":\"{}\",\"serverFingerprint\":\"{}\"}}",
        crypto::encode_bytes(&server_public),
        crypto::fingerprint(&server_public)
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        // Request 1: GET /ident succeeds. Request 2: /accounts/register fails
        // exactly as a real registry does for an owner that already exists.
        let replies = [
            (200, "OK", ident_body),
            (
                409,
                "Conflict",
                "{\"error\":\"owner already exists\"}".to_string(),
            ),
        ];
        for (code, reason, body) in replies {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut scratch = [0u8; 4096];
            let _ = sock.read(&mut scratch);
            let _ = write!(
                sock,
                "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.flush();
        }
    });

    let result = register(&format!("http://127.0.0.1:{port}"), &paths, "alice");

    let err = result.expect_err("register must refuse when keys already exist");
    assert!(
        err.contains("already exist"),
        "error should point at the existing keys, got: {err}"
    );

    for (path, original) in key_paths.iter().zip(&before) {
        let now = std::fs::read(path)
            .unwrap_or_else(|e| panic!("key file {} was deleted: {e}", path.display()));
        assert_eq!(&now, original, "key file {} was modified", path.display());
    }

    drop(server);
}

/// The one client is shared, so blob fetches reuse connections instead of
/// standing up a fresh tokio runtime and TLS handshake per request.
#[test]
fn http_client_is_built_once_and_shared() {
    let first = http_client().expect("client builds");
    let second = http_client().expect("client builds");
    assert!(std::ptr::eq(first, second));
}

/// A blob whose body arrives after the control-plane deadline must still
/// download.
///
/// reqwest's blocking `timeout` is a *deadline*, not a stall timeout, and it
/// is applied twice: once to connect+headers, then again — fresh — to the
/// whole body read, because `blocking::Response::bytes` re-wraps it. With the
/// 30s default and no `Client::builder()` anywhere, any blob slow enough to
/// take half a minute failed outright. A vendored native library on an
/// ordinary connection routinely is that slow (64 MiB inside 30s needs
/// ~18 Mbit/s sustained), so this was the common case, not an edge case.
///
/// The server here holds the body back past `CONTROL_TIMEOUT` and then sends
/// it: that delay is exactly what the old code rejected and the per-request
/// `BLOB_TIMEOUT` override now tolerates. Necessarily slow — the 30s bound it
/// guards is the thing under test.
#[test]
fn fetch_blob_survives_a_body_slower_than_the_control_timeout() {
    let body = b"vendored-native-library-payload".to_vec();
    let hash = hex::encode(crypto::sha256(&body));

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();

    let served = body.clone();
    let server = thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        // Drain the request head; its contents do not matter here.
        let mut scratch = [0u8; 1024];
        let _ = sock.read(&mut scratch);
        write!(
            sock,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            served.len()
        )
        .expect("write head");
        sock.flush().expect("flush head");
        // Headers land immediately, so the request phase succeeds either way;
        // the body is what outlives the old deadline.
        thread::sleep(CONTROL_TIMEOUT + Duration::from_secs(2));
        sock.write_all(&served).expect("write body");
        sock.flush().expect("flush body");
    });

    let fetched = fetch_blob(&format!("http://127.0.0.1:{port}"), &hash);
    server.join().expect("server thread");
    assert_eq!(fetched.expect("slow blob still downloads"), body);
}

#[test]
fn register_duplicate_failure_leaves_no_local_keys() {
    let temp_home = tempfile::tempdir().unwrap();
    let paths = LocalPaths::new(temp_home.path().join(".mfb"));

    let (auth_public, auth_private) = crypto::generate_keypair();
    let (ident_public, ident_private) = crypto::generate_keypair();
    local::write_auth_keypair(&paths, "alice", &auth_public, &auth_private).unwrap();
    local::write_ident_keypair(&paths, "alice", &ident_public, &ident_private).unwrap();

    local::remove_owner_keys(&paths, "alice");
    assert!(!paths.auth_private_key_path("alice").exists());
    assert!(!paths.ident_private_key_path("alice").exists());
}

/// A URL that will refuse every connection: port 1 on loopback is never
/// listening, so any HTTP flow fails fast at the connect step. This lets
/// us drive the network-bound client functions through their argument
/// validation and local-key handling up to the first request.
const DEAD_URL: &str = "http://127.0.0.1:1";

#[test]
fn repo_url_env_defaults_and_overrides() {
    // The default is returned when the env var is unset; an explicit value
    // overrides it. (Serialized guard: env is process-global.)
    std::env::remove_var("MFB_REPO_URL");
    assert_eq!(repo_url_from_env(), DEFAULT_REPO_URL);
    std::env::set_var("MFB_REPO_URL", "http://example.test:9999");
    assert_eq!(repo_url_from_env(), "http://example.test:9999");
    std::env::remove_var("MFB_REPO_URL");
}

#[test]
fn decode_hex32_accepts_only_32_byte_hex() {
    let ok = "aa".repeat(32);
    assert_eq!(decode_hex32(&ok, "x").unwrap(), [0xaa; 32]);
    assert!(decode_hex32("zz", "x").unwrap_err().contains("malformed x"));
    assert!(decode_hex32(&"aa".repeat(31), "x").is_err()); // wrong length
}

#[test]
fn percent_encode_preserves_unreserved_and_escapes_the_rest() {
    assert_eq!(percent_encode("azAZ09-_.~"), "azAZ09-_.~");
    assert_eq!(percent_encode("alice#toolbox"), "alice%23toolbox");
    assert_eq!(percent_encode("a b/c"), "a%20b%2Fc");
}

#[test]
fn metadata_helpers_read_fields_and_report_missing() {
    let doc: serde_json::Value =
        serde_json::from_str(r#"{"registryId":"reg","version":7,"indexHash":"h","expires":100}"#)
            .unwrap();
    check_field(&doc, "registryId", "reg", "d").unwrap();
    assert!(check_field(&doc, "registryId", "other", "d").is_err());
    check_not_expired(&doc, 50, "d").unwrap();
    assert!(check_not_expired(&doc, 100, "d")
        .unwrap_err()
        .contains("expired"));
    assert_eq!(metadata_i64(&doc, "version", "d").unwrap(), 7);
    assert!(metadata_i64(&doc, "missing", "d").is_err());
    assert_eq!(metadata_str(&doc, "indexHash", "d").unwrap(), "h");
    assert!(metadata_str(&doc, "missing", "d").is_err());
    // A doc with no expiry field is rejected by check_not_expired.
    let no_exp: serde_json::Value = serde_json::from_str(r#"{"a":1}"#).unwrap();
    assert!(check_not_expired(&no_exp, 0, "d")
        .unwrap_err()
        .contains("missing an expiry"));
    // parse_metadata rejects non-JSON.
    assert!(parse_metadata("not json", "d").is_err());
}

#[test]
fn decode_delegated_key_reads_or_errors() {
    let (public, _private) = crypto::generate_keypair();
    let doc: serde_json::Value = serde_json::from_str(&format!(
        r#"{{"serverKey":"{}"}}"#,
        crypto::encode_bytes(&public)
    ))
    .unwrap();
    assert_eq!(decode_delegated_key(&doc, "serverKey").unwrap(), public);
    assert!(decode_delegated_key(&doc, "missing")
        .unwrap_err()
        .contains("missing"));
}

#[test]
fn follow_ident_chain_walks_signed_links_and_detects_reanchor() {
    // Build a two-hop signed chain k0 -> k1 -> k2.
    let (k0_pub, k0_prv) = crypto::generate_keypair();
    let (k1_pub, k1_prv) = crypto::generate_keypair();
    let (k2_pub, _k2_prv) = crypto::generate_keypair();
    let link = |old_pub: &[u8], old_prv: &[u8], new_pub: &[u8]| {
        let message =
            crypto::ident_rotation_message("alice", &crypto::fingerprint(old_pub), new_pub);
        IdentChainLink {
            old_key: crypto::encode_bytes(old_pub),
            new_key: crypto::encode_bytes(new_pub),
            signature: crypto::encode_bytes(&crypto::sign(old_prv, &message).unwrap()),
            issued: 0,
        }
    };
    let chain = vec![
        link(&k0_pub, &k0_prv, &k1_pub),
        link(&k1_pub, &k1_prv, &k2_pub),
    ];
    // Following from the oldest pin reaches the newest key.
    let followed = follow_ident_chain("alice", &k0_pub, &chain)
        .unwrap()
        .unwrap();
    assert_eq!(followed, k2_pub);
    // Following from a key that never appears (re-anchor) yields None.
    let (stranger, _) = crypto::generate_keypair();
    assert!(follow_ident_chain("alice", &stranger, &chain)
        .unwrap()
        .is_none());
    // A tampered link signature is a hard error.
    let bad = vec![IdentChainLink {
        old_key: crypto::encode_bytes(&k0_pub),
        new_key: crypto::encode_bytes(&k1_pub),
        signature: crypto::encode_bytes(&[0u8; 64]),
        issued: 0,
    }];
    assert!(follow_ident_chain("alice", &k0_pub, &bad).is_err());
}

#[test]
fn abi_map_reads_object_and_defaults_empty() {
    use crate::server::IndexVersion;
    let with_abi = IndexVersion {
        version: "1.0.0".to_string(),
        hash: "h".to_string(),
        published_at: 0,
        state: "available".to_string(),
        abi_index: serde_json::json!({"greet": "aa", "bad": 3}),
        log_entry: None,
        publisher: None,
        epoch: 0,
    };
    let map = with_abi.abi_map();
    assert_eq!(map.get("greet").unwrap(), "aa");
    assert!(!map.contains_key("bad")); // non-string values are skipped
    let without = IndexVersion {
        abi_index: serde_json::json!("not an object"),
        ..with_abi
    };
    assert!(without.abi_map().is_empty());
}

#[test]
fn package_request_carries_all_fields() {
    let artifact = b"artifact-bytes";
    let package = PackageArtifact {
        ident: "alice#toolbox",
        version: "1.0.0",
        artifact,
        content_hash: "hash",
        ident_fingerprint: "identfp",
        signing_fingerprint: "signfp",
    };
    let request = package_request(&package, "session".to_string());
    assert_eq!(request.ident, "alice#toolbox");
    assert_eq!(request.version, "1.0.0");
    assert_eq!(request.artifact, crypto::encode_bytes(artifact));
    assert_eq!(request.content_hash, "hash");
    assert_eq!(request.ident_fingerprint, "identfp");
    assert_eq!(request.signing_fingerprint, "signfp");
    assert_eq!(request.session_token, "session");
}

// --- Network-bound flows: driven against a dead endpoint so the connect
// step fails deterministically. This exercises argument validation, local
// key handling, and the request/response error plumbing without a bind.

#[test]
fn network_flows_reject_invalid_owner_names_before_any_request() {
    let (_temp, paths) = temp_paths();
    assert!(register(DEAD_URL, &paths, "std").is_err());
    assert!(auth(DEAD_URL, &paths, "1bad").is_err());
    assert!(link_start(DEAD_URL, &paths, "").is_err());
    assert!(link_fetch(DEAD_URL, &paths, "bad-name", "code").is_err());
    assert!(rotate_ident(DEAD_URL, &paths, "std").is_err());
    assert!(fetch_ident_chain(DEAD_URL, "std").is_err());
    assert!(revoke_machine(DEAD_URL, &paths, "std", "fp").is_err());
    assert!(set_org_member(DEAD_URL, &paths, "acme", "std", "m", "admin", false).is_err());
    assert!(issue_publish_token(DEAD_URL, &paths, "std", "scope", 60).is_err());
    assert!(revoke_publish_token(DEAD_URL, &paths, "std", "fp").is_err());
    assert!(transfer_offer(DEAD_URL, &paths, "a#p", "std").is_err());
    assert!(transfer_accept(DEAD_URL, &paths, "a#p", "std").is_err());
    assert!(set_release_state(DEAD_URL, &paths, "std", "a#p", "1.0.0", "yanked").is_err());
    assert!(fetch_index(DEAD_URL, &paths, "std", "pkg").is_err());
    assert!(request_attestation(DEAD_URL, &paths, "std", "a#p", "1.0.0", "fp").is_err());
    assert!(validate_package(
        DEAD_URL,
        &paths,
        "std",
        &PackageArtifact {
            ident: "a#p",
            version: "1",
            artifact: b"x",
            content_hash: "h",
            ident_fingerprint: "i",
            signing_fingerprint: "s",
        }
    )
    .is_err());
    assert!(publish_package(
        DEAD_URL,
        &paths,
        "std",
        &PackageArtifact {
            ident: "a#p",
            version: "1",
            artifact: b"x",
            content_hash: "h",
            ident_fingerprint: "i",
            signing_fingerprint: "s",
        }
    )
    .is_err());
}

#[test]
fn network_flows_report_connection_failure_against_a_dead_endpoint() {
    let (_temp, paths) = temp_paths();
    // ensure_server_key / get_json / post_json all fail at connect.
    let err = ensure_server_key(DEAD_URL, &paths).unwrap_err();
    assert!(err.contains("failed to connect"), "{err}");
    assert!(fetch_blob(DEAD_URL, "hash")
        .unwrap_err()
        .contains("failed to connect"));
    assert!(fetch_ident_chain(DEAD_URL, "alice")
        .unwrap_err()
        .contains("failed to connect"));
    assert!(fetch_checkpoint(DEAD_URL, &paths)
        .unwrap_err()
        .contains("failed to connect"));
    assert!(verify_log_consistency(DEAD_URL, &paths)
        .unwrap_err()
        .contains("failed to connect"));
    assert!(
        verify_publish_inclusion(DEAD_URL, &paths, "a#p", "1.0.0", "deadbeef")
            .unwrap_err()
            .contains("failed to connect")
    );
    // register with a valid name reaches the network step and fails there;
    // the failure path must clean up the locally written keypair.
    assert!(register(DEAD_URL, &paths, "alice")
        .unwrap_err()
        .contains("failed to connect"));
    assert!(!paths.auth_private_key_path("alice").exists());
}

#[test]
fn auth_without_local_key_probes_owner_and_surfaces_the_local_error() {
    let (_temp, paths) = temp_paths();
    // No local key on disk and the server is unreachable: the probe fails
    // to connect (not "unknown owner"), so the local read error surfaces.
    let err = auth(DEAD_URL, &paths, "alice").unwrap_err();
    assert!(
        err.contains("failed to connect") || err.contains("missing local private key"),
        "{err}"
    );
}

#[test]
fn flows_needing_local_ident_key_fail_when_it_is_absent() {
    let (_temp, paths) = temp_paths();
    // These reach past ensure_server_key only if the server is live; with a
    // dead endpoint they fail at connect, which is still an Err. Confirm
    // the Err regardless (both the missing-key and connect paths are Err).
    assert!(link_start(DEAD_URL, &paths, "alice").is_err());
    assert!(rotate_ident(DEAD_URL, &paths, "alice").is_err());
    assert!(revoke_machine(DEAD_URL, &paths, "alice", "fp").is_err());
}

#[test]
fn verify_pinned_metadata_is_a_noop_without_a_pin() {
    let (_temp, paths) = temp_paths();
    // No root pin written: the function returns Ok(()) without any request.
    verify_pinned_metadata(DEAD_URL, &paths).unwrap();
}

#[test]
fn trust_registry_fails_to_connect_against_a_dead_endpoint() {
    let (_temp, paths) = temp_paths();
    assert!(trust_registry(DEAD_URL, &paths, "reg-1", "deadbeef")
        .unwrap_err()
        .contains("failed to connect"));
}

/// plan-168-A: an index from a registry that predates the transfer wire
/// has no `transfers`, `publisher` or `epoch`. It still parses, and reads
/// as "never transferred, published by the origin, epoch 0" — exactly what
/// such a registry's data means.
#[test]
fn an_older_index_without_transfer_fields_deserializes() {
    let registry = Registry::new();
    let (ident_public, _) = crypto::generate_keypair();
    let body = index_body(
        &registry,
        &format!("ed25519:{}", crypto::encode_bytes(&ident_public)),
        &crypto::fingerprint(&ident_public),
    );
    assert!(!body.contains("transfers") && !body.contains("epoch"));
    let index: IndexResponse = serde_json::from_str(&body).expect("older shape parses");
    assert!(index.transfers.is_empty());
    assert_eq!(index.versions.len(), 1);
    assert!(index.versions[0].publisher.is_none());
    assert_eq!(index.versions[0].epoch, 0);
}

// --- blobs ------------------------------------------------------------

/// A blob is only returned when its bytes hash to the hash that was asked
/// for; error statuses surface the registry's own message when it sent one.
#[test]
fn fetch_blob_checks_the_content_hash_and_surfaces_error_bodies() {
    let body = b"native-library-bytes".to_vec();
    let hash = hex::encode(crypto::sha256(&body));

    let served = String::from_utf8(body.clone()).unwrap();
    let good = Routes::new().ok("/blob/", served.clone()).serve();
    assert_eq!(fetch_blob(&good.url, &hash).expect("hash matches"), body);

    // The same bytes served under a different hash must be rejected.
    let wrong = Routes::new().ok("/blob/", served).serve();
    let err = fetch_blob(&wrong.url, &"a".repeat(64)).unwrap_err();
    assert_eq!(
        err,
        "downloaded blob does not match the requested content hash"
    );

    // A JSON error body is unwrapped to its message.
    let missing = Routes::new()
        .reply("/blob/", 404, "{\"error\":\"no such blob\"}")
        .serve();
    assert_eq!(fetch_blob(&missing.url, &hash).unwrap_err(), "no such blob");

    // A non-JSON error body is reported with its status.
    let broken = Routes::new()
        .reply("/blob/", 500, "upstream storage exploded")
        .serve();
    let err = fetch_blob(&broken.url, &hash).unwrap_err();
    assert!(err.contains("500"), "{err}");
    assert!(err.contains("upstream storage exploded"), "{err}");
}

/// bug-420 item 2: the shared client must not follow a registry redirect
/// that downgrades transport or points at an internal host. A hostile
/// registry that 302s `/index` (or `/blob`, `/root.json`, …) to a
/// loopback/plaintext target would otherwise drive a blind SSRF and a
/// cleartext leak from the client machine. `ensure_transport_security` only
/// vets the *initial* URL, so the guard has to sit in the redirect policy.
#[test]
fn a_registry_redirect_to_an_internal_plaintext_host_is_refused() {
    // The "internal" service the redirect tries to reach.
    let internal = spawn_raw(|_request| http_reply("GET", 200, "{\"secret\":\"internal\"}"));
    let internal_url = internal.url.clone();
    // The registry answers with a 302 to the internal host.
    let registry = spawn_raw(move |_request| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {internal_url}/index\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    });

    let err = get_json::<serde_json::Value>(&registry.url, "/index")
        .expect_err("the client must refuse the redirect, not follow it");
    assert!(
        err.contains("redirect"),
        "error should explain the redirect refusal, got: {err}"
    );
    // The strongest check: the internal service was never contacted.
    assert!(
        internal.requests.lock().unwrap().is_empty(),
        "the client must not connect to the internal redirect target"
    );
}

/// A loopback listener that reports only THAT a connection arrived.
///
/// This is the right instrument for an SSRF test: the attack succeeds the
/// moment a socket is opened to the internal service, whatever is spoken
/// over it afterwards. The accepted socket is dropped immediately so a TLS
/// client fails its handshake at once instead of waiting out `BLOB_TIMEOUT`.
fn spawn_connection_probe() -> (u16, std::sync::mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        for connection in listener.incoming() {
            if connection.is_err() || tx.send(()).is_err() {
                return;
            }
        }
    });
    (port, rx)
}

/// bug-585: a redirect to a HOSTNAME whose DNS answer is internal must be
/// refused before any socket is opened.
///
/// `localhost` is the deterministic case of the general attack — a name the
/// resolver answers with `127.0.0.1` — and it needs no network. Before the
/// fix `ensure_redirect_target` parsed the host as an `IpAddr`, failed, and
/// returned `Ok(())`; reqwest then resolved the name itself and connected.
/// Measured on the pre-fix code: the probe below received a connection.
#[test]
fn a_hostname_redirect_resolving_to_loopback_is_refused_before_connecting() {
    let (port, connected) = spawn_connection_probe();
    let registry = spawn_raw(move |_request| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: https://localhost:{port}/blob\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .into_bytes()
    });

    let err = fetch_blob(&registry.url, &"0".repeat(64))
        .expect_err("a redirect to a loopback-resolving hostname must be refused");
    assert!(
        connected.recv_timeout(Duration::from_millis(500)).is_err(),
        "the client opened a socket to the loopback-only service named by the \
             redirect: that is the SSRF"
    );
    assert!(
        err.contains("redirect"),
        "the refusal must come from the redirect policy, not from a failed \
             handshake against the internal service; got: {err}"
    );
}

/// bug-585 POSITIVE PIN, and the one that matters most: an ordinary public
/// https hop — the S3/Tigris presigned-URL 302 that `GET /blob` exists to
/// follow — must still be followed after the guard learned to resolve names.
///
/// An SSRF filter's standing failure mode is refusing VALID input, and a
/// DNS-based filter that breaks blob downloads is worse than the bug it
/// closes. The resolver is injected so this is decided by the POLICY and not
/// by whether the machine running the test has a network: a security check
/// nobody can prove offline is a check nobody re-verifies.
#[test]
fn a_presigned_blob_redirect_to_a_public_host_is_still_allowed() {
    let url = |raw: &str| raw.parse::<reqwest::Url>().unwrap();
    let public = |_: &str, _: u16| {
        Ok(vec![
            "52.219.128.1".parse::<std::net::IpAddr>().unwrap(),
            "2600:1f18:1::1".parse::<std::net::IpAddr>().unwrap(),
        ])
    };

    for target in [
        "https://bucket.s3.us-east-1.amazonaws.com/blobs/abc?X-Amz-Signature=deadbeef",
        "https://fly.storage.tigris.dev/bucket/blobs/abc?X-Amz-Expires=900",
        "https://packages.example.com/blob/abc",
        // A non-default port on a public host is still a legitimate hop.
        "https://cdn.example.com:8443/blob/abc",
    ] {
        assert_eq!(
            ensure_redirect_host_resolves_public(&url(target), public),
            Ok(()),
            "{target} is an ordinary public presigned hop and must be followed"
        );
    }

    // An IP literal is decided by `ensure_redirect_target` alone: the
    // resolution stage must not look it up at all. Pinned by a resolver that
    // panics if it is ever called — so the literal path pays no DNS cost and
    // cannot be failed closed by a resolver outage.
    let never = |host: &str, _: u16| -> Result<Vec<std::net::IpAddr>, String> {
        panic!("resolved the IP literal '{host}'")
    };
    assert_eq!(
        ensure_redirect_host_resolves_public(&url("https://93.184.216.34/blob"), never),
        Ok(())
    );
    assert_eq!(
        ensure_redirect_host_resolves_public(&url("https://[2606:2800:220::1]/blob"), never),
        Ok(())
    );
}

/// bug-585: every blocked family reached by NAME is refused, and a mixed
/// answer set is refused on the blocked member rather than allowed on the
/// public one — reqwest walks the whole list, so checking `addrs[0]` would
/// be a hole an attacker controls the ordering of.
#[test]
fn a_redirect_host_resolving_into_a_blocked_range_is_refused() {
    let url = |raw: &str| raw.parse::<reqwest::Url>().unwrap();
    let target = url("https://blob.attacker.example/blob");
    let ip = |raw: &str| raw.parse::<std::net::IpAddr>().unwrap();

    for answer in [
        "127.0.0.1",        // loopback
        "169.254.169.254",  // cloud metadata (link-local)
        "10.0.0.5",         // RFC 1918
        "192.168.1.1",      //
        "172.16.0.1",       //
        "100.64.0.1",       // CGNAT
        "0.0.0.0",          // unspecified
        "::1",              // IPv6 loopback
        "fe80::1",          // IPv6 link-local
        "fc00::1",          // IPv6 unique-local
        "::ffff:127.0.0.1", // IPv4-mapped loopback
    ] {
        let err = ensure_redirect_host_resolves_public(&target, |_, _| Ok(vec![ip(answer)]))
            .expect_err("a blocked answer must refuse the hop");
        assert!(err.contains("redirect"), "{answer}: {err}");
        assert!(err.contains("blob.attacker.example"), "{answer}: {err}");
    }

    // Mixed answers: public FIRST, internal second. The blocked member wins.
    let mixed = |_: &str, _: u16| Ok(vec![ip("93.184.216.34"), ip("169.254.169.254")]);
    let err = ensure_redirect_host_resolves_public(&target, mixed)
        .expect_err("a mixed answer set must be refused on its blocked member");
    assert!(err.contains("169.254.169.254"), "{err}");
}

/// bug-585: the guard FAILS CLOSED. A hop whose name will not resolve, or
/// that resolves to nothing, is refused rather than handed to the connector
/// unchecked — "I could not check" must never mean "allowed".
///
/// This costs nothing real: an unresolvable hop could not have been
/// connected to either, so the request fails either way; it just fails with
/// a message that names the reason.
#[test]
fn an_unresolvable_redirect_host_fails_closed() {
    let url = |raw: &str| raw.parse::<reqwest::Url>().unwrap();
    let target = url("https://blob.attacker.example/blob");

    let err = ensure_redirect_host_resolves_public(&target, |_, _| {
        Err("nodename nor servname provided".to_string())
    })
    .expect_err("a resolution failure must refuse the hop");
    assert!(err.contains("fails closed"), "{err}");

    let err = ensure_redirect_host_resolves_public(&target, |_, _| Ok(Vec::new()))
        .expect_err("an empty answer set must refuse the hop");
    assert!(err.contains("no addresses"), "{err}");
}

/// bug-585: the injected resolver above proves the POLICY; this proves the
/// policy is wired to the REAL resolver, which is the half an injected
/// double can never show.
///
/// Both cases are deterministic with no network: `localhost` is answered
/// from the hosts file, and `.invalid` is guaranteed never to resolve
/// (RFC 2606) whether the machine is online or not.
#[test]
fn the_redirect_guard_uses_the_platform_resolver() {
    let answers = resolve_redirect_host("localhost", 443).expect("localhost resolves");
    assert!(!answers.is_empty(), "localhost resolved to nothing");
    assert!(
        answers.iter().all(|ip| ip.is_loopback()),
        "localhost answered something other than loopback: {answers:?}"
    );
    assert!(
        answers.iter().all(|ip| is_blocked_redirect_ip(*ip)),
        "every localhost answer must be a blocked redirect target: {answers:?}"
    );

    assert!(
        resolve_redirect_host("mfb-585-does-not-exist.invalid", 443).is_err(),
        "an RFC 2606 .invalid name must not resolve"
    );
}

/// bug-420 item 2: the redirect-target guard blocks a plaintext downgrade
/// and every SSRF-sensitive IP range, while still allowing a normal public
/// https target (e.g. a presigned S3 URL).
#[test]
fn redirect_targets_block_internal_and_downgrade_but_allow_public_https() {
    let url = |raw: &str| raw.parse::<reqwest::Url>().unwrap();

    // A downgrade to plaintext http is refused outright.
    assert!(ensure_redirect_target(&url("http://packages.example.com/blob")).is_err());

    // Internal / link-local / loopback / CGNAT targets over https are refused.
    for target in [
        "https://169.254.169.254/latest/meta-data/", // cloud metadata (link-local)
        "https://127.0.0.1/blob",
        "https://10.0.0.5/blob",
        "https://192.168.1.1/blob",
        "https://172.16.0.1/blob",
        "https://100.64.0.1/blob", // CGNAT
        "https://[::1]/blob",
        "https://[fe80::1]/blob",
        "https://[fc00::1]/blob",
        "https://[::ffff:127.0.0.1]/blob", // IPv4-mapped loopback
    ] {
        assert!(
            ensure_redirect_target(&url(target)).is_err(),
            "{target} should be refused"
        );
    }

    // A public https host — including a presigned-URL host — is allowed.
    assert!(ensure_redirect_target(&url("https://s3.amazonaws.com/bucket/key")).is_ok());
    assert!(ensure_redirect_target(&url("https://packages.example.com/blob")).is_ok());
    assert!(ensure_redirect_target(&url("https://93.184.216.34/blob")).is_ok());
}

/// `HEAD /blob` is a presence probe: 404 is "absent", not an error, and
/// anything else is.
#[test]
fn blob_exists_maps_status_to_presence() {
    let present = Routes::new().ok("/blob/", "").serve();
    assert!(blob_exists(&present.url, "h").unwrap());

    let absent = Routes::new().reply("/blob/", 404, "").serve();
    assert!(!blob_exists(&absent.url, "h").unwrap());

    let broken = Routes::new().reply("/blob/", 500, "").serve();
    let err = blob_exists(&broken.url, "h").unwrap_err();
    assert!(err.contains("HEAD /blob"), "{err}");
    assert!(err.contains("500"), "{err}");
}

/// The upload is the one call that authenticates with a header, because a
/// raw-body request cannot carry the body-field session token.
#[test]
fn put_blob_authenticates_with_a_bearer_header_and_reports_failures() {
    let stub = Routes::new().ok("/blob/", "{}").serve();
    put_blob(&stub.url, "abc", b"payload".to_vec(), "session-jwt").expect("upload");
    let request = stub.request_to("/blob/abc");
    let lowered = request.to_ascii_lowercase();
    assert!(
        lowered.contains("authorization: bearer session-jwt"),
        "upload must carry the session as a bearer token: {request}"
    );
    assert!(
        lowered.contains("content-type: application/octet-stream"),
        "{request}"
    );
    assert!(
        request.ends_with("payload"),
        "the raw bytes are the body: {request}"
    );

    // A JSON error is unwrapped; anything else is reported with its status.
    let denied = Routes::new()
        .reply(
            "/blob/",
            403,
            "{\"error\":\"token is not scoped for this package\"}",
        )
        .serve();
    assert_eq!(
        put_blob(&denied.url, "abc", b"x".to_vec(), "t").unwrap_err(),
        "token is not scoped for this package"
    );
    let broken = Routes::new()
        .reply("/blob/", 500, "gateway timeout")
        .serve();
    let err = put_blob(&broken.url, "abc", b"x".to_vec(), "t").unwrap_err();
    assert!(err.contains("PUT /blob"), "{err}");
    assert!(err.contains("gateway timeout"), "{err}");
}

// --- validate / publish ----------------------------------------------

/// Both artifact calls post the same request shape, carrying the session
/// token read from disk.
#[test]
fn validate_and_publish_post_the_artifact_with_the_local_session() {
    let (_temp, paths) = temp_paths();
    local::write_session(&paths, "alice", "session-jwt").unwrap();
    let package = PackageArtifact {
        ident: "alice#pkg",
        version: "1.0.0",
        artifact: b"artifact",
        content_hash: "ch",
        ident_fingerprint: "ifp",
        signing_fingerprint: "sfp",
    };

    let stub = Routes::new()
        .ok(
            "/validate",
            serde_json::json!({
                "valid": true,
                "contentHash": "ch",
                "abiIndex": {"greet": "aa"},
                "diagnostics": [],
            })
            .to_string(),
        )
        .ok(
            "/publish",
            serde_json::json!({
                "ident": "alice#pkg",
                "version": "1.0.0",
                "hash": "ch",
                "publishedAt": 7,
                "state": "available",
                "blobStored": true,
                "logEntry": {"index": 2, "leafHash": "bb"},
            })
            .to_string(),
        )
        .serve();

    let validated = validate_package(&stub.url, &paths, "alice", &package).expect("validate");
    assert!(validated.valid);
    assert_eq!(validated.content_hash, "ch");
    assert_eq!(
        field(&stub.request_to("/validate"), "sessionToken"),
        "session-jwt"
    );

    let published = publish_package(&stub.url, &paths, "alice", &package).expect("publish");
    assert_eq!(published.log_entry.index, 2);
    assert!(published.blob_stored);
    assert!(
        published.warnings.is_empty(),
        "an omitted warnings field defaults to empty"
    );
    assert_eq!(
        field(&stub.request_to("/publish"), "artifact"),
        crypto::encode_bytes(b"artifact")
    );
}

// --- response plumbing ------------------------------------------------

/// The body ceiling (bug-276 R3) is enforced from the `Content-Length` hint
/// *and* from the read itself, and an unreadable error body never masks the
/// status it arrived with.
#[test]
fn response_bodies_are_capped_and_errors_keep_their_status() {
    // A success whose body is not JSON at all.
    let garbage = spawn_raw(|request| http_reply(method_of(request), 200, "<html>nope</html>"));
    let err = fetch_ident_chain(&garbage.url, "alice").unwrap_err();
    assert!(err.contains("invalid repository response"), "{err}");

    // An error status with a non-JSON body keeps both status and text.
    let plain = spawn_raw(|request| http_reply(method_of(request), 500, "backend on fire"));
    let err = fetch_ident_chain(&plain.url, "alice").unwrap_err();
    assert!(err.contains("500"), "{err}");
    assert!(err.contains("backend on fire"), "{err}");

    // A success declaring more than MAX_JSON_BYTES is refused on the hint
    // alone, before a byte of it is read.
    let huge = spawn_raw(|_| {
        b"HTTP/1.1 200 OK\r\nContent-Length: 99999999\r\nConnection: close\r\n\r\n".to_vec()
    });
    let err = fetch_ident_chain(&huge.url, "alice").unwrap_err();
    assert!(err.contains("99999999 bytes"), "{err}");
    assert!(
        err.contains(&format!("{MAX_JSON_BYTES}-byte limit")),
        "{err}"
    );

    // The same oversized body on an *error* status must not swallow the
    // status: the generic text stands in for the unreadable body.
    let huge_error = spawn_raw(|_| {
        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 99999999\r\n\
              Connection: close\r\n\r\n"
            .to_vec()
    });
    let err = fetch_ident_chain(&huge_error.url, "alice").unwrap_err();
    assert!(err.contains("503"), "{err}");
    assert!(err.contains("repository request failed"), "{err}");

    // A body with no Content-Length at all is still bounded by the read.
    let lying = spawn_raw(|_| {
        let mut out = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
        out.extend(std::iter::repeat_n(b'x', MAX_JSON_BYTES as usize + 1));
        out
    });
    let err = fetch_ident_chain(&lying.url, "alice").unwrap_err();
    assert!(
        err.contains(&format!("exceeds the {MAX_JSON_BYTES}-byte limit")),
        "{err}"
    );
}

/// A registry URL that is not a URL at all is rejected by name, before any
/// socket is opened.
#[test]
fn transport_security_rejects_an_unparseable_url() {
    let err = ensure_transport_security("packages.example.com/no-scheme").unwrap_err();
    assert!(err.contains("invalid registry URL"), "{err}");
    assert!(err.contains("packages.example.com/no-scheme"), "{err}");
}

#[test]
fn transport_security_requires_https_for_non_loopback() {
    // SUP-01 / bug-189: http is allowed only for loopback; anything else must
    // use https.
    assert!(ensure_transport_security("http://127.0.0.1:7777").is_ok());
    assert!(ensure_transport_security("http://localhost:7777").is_ok());
    assert!(ensure_transport_security("http://[::1]:7777").is_ok());
    assert!(ensure_transport_security("https://packages.example.com").is_ok());
    // Plaintext to a non-loopback host is rejected, naming https.
    let err = ensure_transport_security("http://packages.example.com").unwrap_err();
    assert!(err.contains("https"), "error should steer to https: {err}");
    // An unsupported scheme is rejected too.
    assert!(ensure_transport_security("ftp://packages.example.com").is_err());
}
