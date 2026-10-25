//! The client transport (`client/transport.rs`) and package calls
//! (`client/packages.rs`) on a registry that drops the connection, truncates a
//! body, or serves an index whose identKey does not decode; and the redirect
//! host guard driven through ONE deterministic resolver for every branch.

use super::transport::DEAD_URL;
use super::*;

/// A deterministic resolver answering by name.
fn table_resolver(host: &str, _port: u16) -> Result<Vec<std::net::IpAddr>, String> {
    let ip = |raw: &str| raw.parse::<std::net::IpAddr>().unwrap();
    match host {
        "public.example" => Ok(vec![ip("93.184.216.34"), ip("2606:2800:220::1")]),
        "internal.example" => Ok(vec![ip("93.184.216.34"), ip("10.0.0.7")]),
        "empty.example" => Ok(Vec::new()),
        "ip-literal-was-resolved" => panic!("an IP literal must not be looked up"),
        _ => Err("NXDOMAIN".to_string()),
    }
}

/// bug-585: every branch of the host guard under one resolver — a public answer
/// passes, any internal answer refuses, an empty or failed lookup fails closed,
/// and an IP literal or a URL with no host is decided without a lookup.
#[test]
fn the_redirect_host_guard_decides_every_answer_shape() {
    let url = |raw: &str| raw.parse::<reqwest::Url>().unwrap();
    let guard = |raw: &str| ensure_redirect_host_resolves_public(&url(raw), table_resolver);

    assert_eq!(guard("https://public.example/blob"), Ok(()));
    assert_eq!(guard("https://93.184.216.34/blob"), Ok(()));
    assert_eq!(guard("https://[2606:2800:220::1]/blob"), Ok(()));
    assert_eq!(guard("file:///tmp/blob"), Ok(()));
    assert_eq!(
        guard("https://internal.example/blob").unwrap_err(),
        "refusing to follow a repository redirect to 'internal.example': it resolves to \
         internal address 10.0.0.7. A redirect to a private, loopback, or link-local host is a \
         possible SSRF, whether the address is written in the URL or answered by DNS."
    );
    assert_eq!(
        guard("https://empty.example/blob").unwrap_err(),
        "refusing to follow a repository redirect to 'empty.example': it resolved to no \
         addresses. The redirect guard fails closed."
    );
    assert_eq!(
        guard("https://unknown.example/blob").unwrap_err(),
        "refusing to follow a repository redirect to 'unknown.example': its address could not \
         be resolved (NXDOMAIN). The redirect guard fails closed — a hop that cannot be shown \
         to be public is not followed."
    );
}

/// A reply that promises more body than it sends, then hangs up.
fn truncated(status: &str, promised: usize, sent: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {promised}\r\n\
         Connection: close\r\n\r\n{sent}"
    )
    .into_bytes()
}

#[test]
fn a_truncated_json_body_is_a_read_error() {
    let stub = spawn_raw(|_| truncated("200 OK", 200, "{\"owner\":"));
    let err = fetch_ident_chain(&stub.url, "alice").unwrap_err();
    assert!(
        err.starts_with("failed to read repository response: "),
        "{err}"
    );
}

#[test]
fn a_truncated_error_body_still_reports_the_status() {
    let stub = spawn_raw(|_| truncated("500 Internal Server Error", 200, "{\"err"));
    assert_eq!(
        fetch_blob(&stub.url, &"a".repeat(64)).unwrap_err(),
        "repository request failed with status 500 Internal Server Error: repository request \
         failed"
    );
    assert_eq!(
        put_blob(&stub.url, &"a".repeat(64), b"bytes".to_vec(), "jwt").unwrap_err(),
        "repository PUT /blob request failed with status 500 Internal Server Error: repository \
         request failed"
    );
}

#[test]
fn blob_and_control_calls_report_a_dead_endpoint() {
    for err in [
        blob_exists(DEAD_URL, &"a".repeat(64)).unwrap_err(),
        put_blob(DEAD_URL, &"a".repeat(64), b"bytes".to_vec(), "jwt").unwrap_err(),
        post_json::<serde_json::Value>(DEAD_URL, "/accounts/register", &serde_json::json!({}))
            .unwrap_err(),
    ] {
        assert!(
            err.starts_with("failed to connect to repository service: "),
            "{err}"
        );
    }
}

#[test]
fn an_index_whose_ident_key_does_not_decode_is_refused() {
    let registry = Registry::new();
    let (_temp, paths) = temp_paths();
    let stub = registry
        .routes()
        .ok(
            "/index/",
            index_body(&registry, "ed25519:not base64!", "fingerprint"),
        )
        .serve();
    assert_eq!(
        fetch_index(&stub.url, &paths, "alice", "pkg").unwrap_err(),
        "malformed identKey"
    );
}

/// bug-420 item 2: the redirect decision as the policy makes it — a public
/// https hop is followed until the cap, then refused whatever its target; a
/// downgrade or an internal literal is refused at any depth. (IP literals and
/// an `.invalid` name keep this offline.)
#[test]
fn the_redirect_decision_caps_hops_and_vets_every_target() {
    let url = |raw: &str| raw.parse::<reqwest::Url>().unwrap();
    let public = url("https://93.184.216.34/blob");
    assert_eq!(vet_redirect(0, &public), Ok(()));
    assert_eq!(vet_redirect(9, &public), Ok(()));
    assert_eq!(
        vet_redirect(10, &public).unwrap_err(),
        "repository redirected more than 10 times"
    );
    assert!(vet_redirect(0, &url("http://93.184.216.34/blob"))
        .unwrap_err()
        .contains("non-https"));
    assert_eq!(
        vet_redirect(0, &url("https://[::1]/blob")).unwrap_err(),
        "refusing to follow a repository redirect to internal address '[::1]': a redirect to \
         a private, loopback, or link-local host is a possible SSRF"
    );
    assert!(vet_redirect(0, &url("https://mfb-redirect.invalid/blob"))
        .unwrap_err()
        .contains("fails closed"));
}
