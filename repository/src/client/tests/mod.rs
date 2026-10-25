use super::*;
use crate::server::IdentChainLink;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::thread;

mod accounts;
mod governance_faults;
mod identity_faults;
mod transport;
mod transport_faults;
mod verification;

struct MetadataFixture {
    root_fingerprint: String,
    server_public: Vec<u8>,
    root: RootResponse,
    timestamp: SignedMetadataResponse,
    snapshot: SignedMetadataResponse,
}

fn temp_paths() -> (tempfile::TempDir, LocalPaths) {
    let temp = tempfile::tempdir().unwrap();
    let paths = LocalPaths::new(temp.path().join(".mfb"));
    (temp, paths)
}

// =====================================================================
// A loopback HTTP stub, shared by the flow tests below.
//
// Every registry call in this file goes through *blocking* reqwest, so the
// stub is an ordinary blocking TCP server: read one request, record it,
// answer from a small route table. Replies always carry `Connection:
// close`, so each call lands on its own accept and route dispatch stays a
// pure function of the request.
// =====================================================================

fn method_of(request: &str) -> &str {
    request.split_whitespace().next().unwrap_or("")
}

/// The request target (`/log/proof/0?size=2`) from the request line.
fn target_of(request: &str) -> &str {
    request.split_whitespace().nth(1).unwrap_or("")
}

/// The target with any query string removed.
fn path_of(request: &str) -> &str {
    target_of(request).split('?').next().unwrap_or("")
}

fn body_of(request: &str) -> &str {
    request.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")
}

fn json_body(request: &str) -> serde_json::Value {
    serde_json::from_str(body_of(request))
        .unwrap_or_else(|err| panic!("request body is not JSON ({err}): {}", body_of(request)))
}

/// A required string field of a recorded request body.
fn field(request: &str, name: &str) -> String {
    json_body(request)
        .get(name)
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| {
            panic!(
                "request body has no string field '{name}': {}",
                body_of(request)
            )
        })
        .to_string()
}

/// Read exactly one HTTP request: headers, then `Content-Length` bytes.
fn read_request(sock: &mut TcpStream) -> String {
    let mut raw: Vec<u8> = Vec::new();
    let mut scratch = [0u8; 8192];
    loop {
        let read = match sock.read(&mut scratch) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        raw.extend_from_slice(&scratch[..read]);
        let Some(head_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&raw[..head_end]).to_ascii_lowercase();
        let length = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if raw.len() >= head_end + 4 + length {
            break;
        }
    }
    String::from_utf8_lossy(&raw).to_string()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// A complete HTTP/1.1 reply. A `HEAD` gets the headers only, as the
/// registry's own `HEAD /blob` does.
fn http_reply(method: &str, status: u16, body: &str) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    )
    .into_bytes();
    if method != "HEAD" {
        out.extend_from_slice(body.as_bytes());
    }
    out
}

struct Stub {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Stub {
    /// The recorded request whose path is exactly `path`, or — only when
    /// there is no exact match — the first one that starts with it. The
    /// exact match must win: `/machines/revoke/challenge` is a prefix
    /// neighbour of `/machines/revoke`.
    fn request_to(&self, path: &str) -> String {
        let requests = self.requests.lock().unwrap();
        requests
            .iter()
            .find(|request| path_of(request) == path)
            .or_else(|| {
                requests
                    .iter()
                    .find(|request| path_of(request).starts_with(path))
            })
            .unwrap_or_else(|| {
                let seen: Vec<&str> = requests.iter().map(|r| path_of(r)).collect();
                panic!("no request to {path}; saw {seen:?}")
            })
            .clone()
    }
}

/// Spawn a stub that answers each request with raw bytes from `handler`.
fn spawn_raw<F>(handler: F) -> Stub
where
    F: Fn(&str) -> Vec<u8> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut sock) = connection else { return };
            let request = read_request(&mut sock);
            if request.is_empty() {
                continue;
            }
            let reply = handler(&request);
            recorded.lock().unwrap().push(request);
            let _ = sock.write_all(&reply);
            let _ = sock.flush();
        }
    });
    Stub {
        url: format!("http://127.0.0.1:{port}"),
        requests,
    }
}

/// A path → reply table. A target ending in `/` matches by prefix.
struct Routes(Vec<(String, u16, String)>);

impl Routes {
    fn new() -> Self {
        Routes(Vec::new())
    }

    fn ok(self, target: &str, body: impl Into<String>) -> Self {
        self.reply(target, 200, body)
    }

    fn reply(mut self, target: &str, status: u16, body: impl Into<String>) -> Self {
        self.0.push((target.to_string(), status, body.into()));
        self
    }

    fn serve(self) -> Stub {
        spawn_raw(move |request| {
            let path = path_of(request);
            let matched = self.0.iter().find(|(target, _, _)| {
                target.as_str() == path
                    || (target.ends_with('/') && path.starts_with(target.as_str()))
            });
            let (status, body) = match matched {
                Some((_, status, body)) => (*status, body.clone()),
                None => (
                    404,
                    format!("{{\"error\":\"stub has no route for {path}\"}}"),
                ),
            };
            http_reply(method_of(request), status, &body)
        })
    }
}

/// The registry's own signing identity, as `GET /ident` publishes it.
struct Registry {
    public: Vec<u8>,
    private: Vec<u8>,
}

impl Registry {
    fn new() -> Registry {
        let (public, private) = crypto::generate_keypair();
        Registry { public, private }
    }

    fn ident_body(&self) -> String {
        serde_json::json!({
            "serverKey": crypto::encode_bytes(&self.public),
            "serverFingerprint": crypto::fingerprint(&self.public),
        })
        .to_string()
    }

    /// A route table that already answers the `GET /ident` every online
    /// flow makes first.
    fn routes(&self) -> Routes {
        Routes::new().ok("/ident", self.ident_body())
    }

    fn checkpoint_body(&self, size: i64, root: &[u8; 32]) -> String {
        let signature = crypto::sign(
            &self.private,
            &crate::log::checkpoint_signing_input(size as u64, root),
        )
        .unwrap();
        serde_json::json!({
            "size": size,
            "rootHash": hex::encode(root),
            "signature": crypto::encode_bytes(&signature),
        })
        .to_string()
    }
}

/// Provision `owner` on this machine the way a completed `register` +
/// `auth` would: an ident keypair and a live session. Returns the ident
/// public key, which the assertions verify signatures against.
fn seed_ident(paths: &LocalPaths, owner: &str) -> Vec<u8> {
    let (ident_public, ident_private) = crypto::generate_keypair();
    local::write_ident_keypair(paths, owner, &ident_public, &ident_private).unwrap();
    local::write_session(paths, owner, "session-jwt").unwrap();
    ident_public
}

/// Assert that `request`'s base64url `field` is a valid signature by
/// `public` over `message`.
fn assert_signed(request: &str, name: &str, public: &[u8], message: &[u8]) {
    let signature = crypto::decode_bytes(&field(request, name), name).unwrap();
    crypto::verify(public, message, &signature)
        .unwrap_or_else(|err| panic!("'{name}' does not verify under the expected key: {err}"));
}

// --- signed metadata (plan-10-C2) -------------------------------------

/// A stable metadata root, so successive chains from the same authority
/// share a root fingerprint — the precondition for exercising a *version*
/// rollback rather than tripping the root-fingerprint check first.
struct MetadataAuthority {
    root_public: Vec<u8>,
    root_private: Vec<u8>,
    snapshot_public: Vec<u8>,
    snapshot_private: Vec<u8>,
    timestamp_public: Vec<u8>,
    timestamp_private: Vec<u8>,
}

impl MetadataAuthority {
    fn new() -> MetadataAuthority {
        let (root_public, root_private) = crypto::generate_keypair();
        let (snapshot_public, snapshot_private) = crypto::generate_keypair();
        let (timestamp_public, timestamp_private) = crypto::generate_keypair();
        MetadataAuthority {
            root_public,
            root_private,
            snapshot_public,
            snapshot_private,
            timestamp_public,
            timestamp_private,
        }
    }

    fn fingerprint(&self) -> String {
        crypto::fingerprint(&self.root_public)
    }
}

/// A valid metadata chain that delegates `server_public` as the
/// attestation key, so `trust_registry`'s cross-check can succeed.
fn build_metadata_delegating(
    authority: &MetadataAuthority,
    registry_id: &str,
    version: i64,
    expires: i64,
    server_public: &[u8],
) -> MetadataFixture {
    build_metadata_delegating_at_root_version(
        authority,
        registry_id,
        1,
        version,
        expires,
        server_public,
    )
}

/// `build_metadata_delegating` with an explicit `root.json` version — what a
/// root RENEWAL moves (bug-584).
fn build_metadata_delegating_at_root_version(
    authority: &MetadataAuthority,
    registry_id: &str,
    root_version: i64,
    version: i64,
    expires: i64,
    server_public: &[u8],
) -> MetadataFixture {
    let MetadataAuthority {
        root_public,
        root_private,
        snapshot_public,
        snapshot_private,
        timestamp_public,
        timestamp_private,
    } = authority;
    let root_signed = format!(
        "{{\"type\":\"root\",\"registryId\":\"{registry_id}\",\"version\":{root_version},\"expires\":{expires},\"serverKey\":\"{}\",\"snapshotKey\":\"{}\",\"timestampKey\":\"{}\"}}",
        crypto::encode_bytes(server_public),
        crypto::encode_bytes(snapshot_public),
        crypto::encode_bytes(timestamp_public),
    );
    let root_signature = crypto::sign(
        root_private,
        &crypto::root_signing_input(root_signed.as_bytes()),
    )
    .unwrap();
    let timestamp_signed = format!(
        "{{\"type\":\"timestamp\",\"registryId\":\"{registry_id}\",\"version\":{version},\"expires\":{expires},\"snapshotVersion\":{version},\"indexHash\":\"idx\"}}",
    );
    let timestamp_signature = crypto::sign(
        timestamp_private,
        &crypto::timestamp_signing_input(timestamp_signed.as_bytes()),
    )
    .unwrap();
    let snapshot_signed = format!(
        "{{\"type\":\"snapshot\",\"registryId\":\"{registry_id}\",\"version\":{version},\"expires\":{expires},\"indexHash\":\"idx\"}}",
    );
    let snapshot_signature = crypto::sign(
        snapshot_private,
        &crypto::snapshot_signing_input(snapshot_signed.as_bytes()),
    )
    .unwrap();
    MetadataFixture {
        root_fingerprint: crypto::fingerprint(root_public),
        server_public: server_public.to_vec(),
        root: RootResponse {
            signed: root_signed,
            signature: crypto::encode_bytes(&root_signature),
            root_key: crypto::encode_bytes(root_public),
            root_fingerprint: crypto::fingerprint(root_public),
        },
        timestamp: SignedMetadataResponse {
            signed: timestamp_signed,
            signature: crypto::encode_bytes(&timestamp_signature),
        },
        snapshot: SignedMetadataResponse {
            signed: snapshot_signed,
            signature: crypto::encode_bytes(&snapshot_signature),
        },
    }
}

/// Add the three metadata routes for `fixture` to a route table.
fn with_metadata_routes(routes: Routes, fixture: &MetadataFixture) -> Routes {
    routes
        .ok("/root.json", serde_json::to_string(&fixture.root).unwrap())
        .ok(
            "/timestamp.json",
            serde_json::to_string(&fixture.timestamp).unwrap(),
        )
        .ok(
            "/snapshot.json",
            serde_json::to_string(&fixture.snapshot).unwrap(),
        )
}

// --- index ------------------------------------------------------------

fn index_body(registry: &Registry, ident_key: &str, ident_fingerprint: &str) -> String {
    index_body_for(registry, "alice", "alice#pkg", ident_key, ident_fingerprint)
}

/// The same body with the route-identifying fields under the caller's
/// control, so a test can serve an index that is *validly signed* and yet
/// answers a different question than the one asked (bug-581). The signature
/// is always over the `owner` the body carries — that is precisely the
/// self-referential property that made the binding useless on its own.
fn index_body_for(
    registry: &Registry,
    owner: &str,
    ident: &str,
    ident_key: &str,
    ident_fingerprint: &str,
) -> String {
    let signature = crypto::sign(
        &registry.private,
        &crypto::name_binding_message(owner, ident_fingerprint),
    )
    .unwrap();
    serde_json::json!({
        "ident": ident,
        "owner": owner,
        "identKey": ident_key,
        "identFingerprint": ident_fingerprint,
        "nameBindingSignature": crypto::encode_bytes(&signature),
        "serverFingerprint": crypto::fingerprint(&registry.public),
        "versions": [{
            "version": "1.0.0",
            "hash": "h",
            "publishedAt": 1,
            "state": "available",
            "abiIndex": {},
            "logEntry": null,
        }],
    })
    .to_string()
}
