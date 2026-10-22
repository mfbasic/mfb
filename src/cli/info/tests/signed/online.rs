//! `mfb info`'s real registry client (`OnlineRegistry`) against a loopback
//! registry stub that speaks the repository wire format.
//!
//! The other signed tests answer through [`FakeRegistry`], so they prove the
//! verdicts but not that the command asks the right registry the right
//! questions. Here every answer travels over HTTP from `127.0.0.1` (the one
//! host the repository client accepts plain http for): `GET /ident`, `GET
//! /index/<owner>%23<package>` and `GET /idents/<owner>`, each served from the
//! test chain's own keys. The stub records every request, so the tests also pin
//! which routes a verdict needed.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use mfb_repository::server::{IdentChainResponse, ServerIdentResponse};

use super::*;

/// A registry on a loopback port, answering `GET <path>` from a fixed table and
/// 404 for anything else. Each reply closes its connection, so every request is
/// its own accept.
struct LoopbackRegistry {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl LoopbackRegistry {
    fn serve(routes: Vec<(String, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let path = request_path(&mut stream);
                let (status, body) = routes
                    .iter()
                    .find(|(route, _)| *route == path)
                    .map_or((404, r#"{"error":"not found"}"#), |(_, body)| {
                        (200, body.as_str())
                    });
                seen.lock().unwrap().push(path);
                let reply = format!(
                    "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    if status == 200 { "OK" } else { "Not Found" },
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        LoopbackRegistry { url, requests }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

/// The request target of one GET (the client sends no body).
fn request_path(stream: &mut TcpStream) -> String {
    let mut raw = Vec::new();
    let mut scratch = [0u8; 4096];
    while !raw.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut scratch) {
            Ok(0) | Err(_) => break,
            Ok(read) => raw.extend_from_slice(&scratch[..read]),
        }
    }
    String::from_utf8_lossy(&raw)
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string()
}

/// `GET /ident` as the registry holding `chain`'s server key answers it.
fn ident_route(chain: &Chain) -> (String, String) {
    let body = serde_json::to_string(&ServerIdentResponse {
        server_key: crypto::encode_bytes(&chain.server_public),
        server_fingerprint: crypto::fingerprint(&chain.server_public),
    })
    .unwrap();
    ("/ident".to_string(), body)
}

fn index_route(index: &IndexResponse) -> (String, String) {
    (
        "/index/alice%23probe".to_string(),
        serde_json::to_string(index).unwrap(),
    )
}

fn idents_route(owner: &str, current: &[u8], links: Vec<IdentChainLink>) -> (String, String) {
    let body = serde_json::to_string(&IdentChainResponse {
        owner: owner.to_string(),
        ident_key: format!("ed25519:{}", crypto::encode_bytes(current)),
        ident_fingerprint: crypto::fingerprint(current),
        chain: links,
    })
    .unwrap();
    (format!("/idents/{owner}"), body)
}

/// A non-transferred build whose owner has since rotated their ident key: the
/// online lookup reads the registry key, the owner's current key from the signed
/// `/index` binding, and the rotation link from `/idents`, and reports the build
/// verified with the key rotated — the verdict `FakeRegistry` gives for the same
/// chain, now reached over the wire.
#[test]
fn the_online_registry_reports_a_rotated_owner_key() {
    let mut chain = Chain::new();
    let (rotated_public, _) = crypto::generate_keypair();
    let rotation_signature = crypto::sign(
        &chain.ident_private,
        &crypto::ident_rotation_message(
            "alice",
            &crypto::fingerprint(&chain.ident_public),
            &rotated_public,
        ),
    )
    .unwrap();
    // `/index/alice#probe` names the rotated key as alice's current one.
    let mut index = probe_index(&chain, None);
    let rotated_fingerprint = crypto::fingerprint(&rotated_public);
    index.ident_key = format!("ed25519:{}", crypto::encode_bytes(&rotated_public));
    index.ident_fingerprint = rotated_fingerprint.clone();
    index.name_binding_signature = crypto::encode_bytes(
        &crypto::sign(
            &chain.server_private,
            &crypto::name_binding_message("alice", &rotated_fingerprint),
        )
        .unwrap(),
    );
    let link = IdentChainLink {
        old_key: crypto::encode_bytes(&chain.ident_public),
        new_key: crypto::encode_bytes(&rotated_public),
        signature: crypto::encode_bytes(&rotation_signature),
        issued: 0,
    };
    let registry = LoopbackRegistry::serve(vec![
        ident_route(&chain),
        index_route(&index),
        idents_route("alice", &rotated_public, vec![link]),
    ]);
    chain.registry = Some(registry.url.clone());

    let lines = signing_lines(&chain.blob(), Some(DIGEST), &OnlineRegistry);
    assert_eq!(signed_line(&lines, "registry"), registry.url);
    assert_eq!(
        signed_line(&lines, "ident key"),
        format!("rotated since signing (current identFingerprint {rotated_fingerprint})")
    );
    let trust = signed_line(&lines, "trust chain");
    assert_eq!(
        trust,
        format!(
            "verified (repoFingerprint {})",
            crypto::fingerprint(&chain.server_public)
        )
    );
    assert_eq!(
        registry.requests(),
        ["/ident", "/index/alice%23probe", "/idents/alice"]
    );
}

/// plan-168-C over the wire: bob's build of `alice#probe` verifies through the
/// signed transfer the registry's `/index` serves, with the chains of both
/// owners read from `/idents`.
#[test]
fn the_online_registry_verifies_a_transferred_ident() {
    let mut chain = Chain::signed_by("bob", "alice#probe");
    let index = probe_index(
        &chain,
        Some(("bob", &chain.ident_public, &chain.ident_private)),
    );
    let alice_public =
        mfb_repository::package::decode_metadata_key(&index.ident_key, "identKey").unwrap();
    let registry = LoopbackRegistry::serve(vec![
        ident_route(&chain),
        index_route(&index),
        idents_route("alice", &alice_public, Vec::new()),
        idents_route("bob", &chain.ident_public, Vec::new()),
    ]);
    chain.registry = Some(registry.url.clone());

    let lines = signing_lines(&chain.blob(), Some(DIGEST), &OnlineRegistry);
    assert_eq!(signed_line(&lines, "owner"), "bob");
    assert_eq!(signed_line(&lines, "ident key"), "current");
    let trust = signed_line(&lines, "trust chain");
    assert!(trust.starts_with("verified (repoFingerprint "), "{trust}");
    let requests = registry.requests();
    assert_eq!(requests[..2], ["/ident", "/index/alice%23probe"]);
    assert!(
        requests[2..]
            .iter()
            .all(|path| path.starts_with("/idents/")),
        "{requests:?}"
    );
}

/// A registry that answers nothing the client asks reports the chain as not
/// verified, naming the URL the binary carries — never a panic, never a pass.
#[test]
fn an_online_registry_without_the_routes_is_reported_unreachable() {
    let mut chain = Chain::new();
    let registry = LoopbackRegistry::serve(Vec::new());
    chain.registry = Some(registry.url.clone());
    let trust = trust_chain(&chain.blob(), &OnlineRegistry);
    assert!(
        trust.starts_with(&format!(
            "NOT VERIFIED (registry {} is unreachable: ",
            registry.url
        )),
        "{trust}"
    );
    assert_eq!(registry.requests(), ["/ident"]);
}
