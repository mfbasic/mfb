use crate::blobstore::{BlobFetch, BlobKind, BlobStore};
use crate::store::{now_unix, NewSession, PairingFetch, Store};
use crate::{crypto, package};
use axum::extract::{ConnectInfo, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use uuid::Uuid;

mod accounts;
mod auth;
mod blobs;
mod browse;
mod governance;
mod limits;
mod log_proofs;
mod metadata;
mod publish;
mod wire;
pub use accounts::*;
pub use auth::*;
use blobs::*;
use browse::*;
use governance::*;
use limits::*;
use log_proofs::*;
use metadata::*;
use publish::*;
pub use wire::*;

#[derive(Clone)]
pub struct AppState {
    store: Store,
    blob_store: BlobStore,
    rate_limiter: RateLimiter,
}

// coverage:off — serve() binds a real TCP listener, spawns the background
// reaper task, and runs the axum accept loop; none of that is reachable under a
// unit test. The individual route handlers it wires up are tested directly by
// constructing AppState and calling them, which is where the request/response
// logic lives.
pub async fn serve(
    store: Store,
    blob_store: BlobStore,
    listen: SocketAddr,
) -> Result<SocketAddr, String> {
    let state = AppState {
        store,
        blob_store,
        rate_limiter: RateLimiter::new(),
    };
    // Background reaper (plan-10-D2): sweep expired challenges/sessions/pairing
    // blobs and prune the rate-limiter map so nothing accumulates unbounded.
    {
        let reaper_state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                let _ = reaper_state.store.reap_expired();
                reaper_state.rate_limiter.prune(3600);
            }
        });
    }
    let app = build_router(state);
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|err| format!("failed to bind {listen}: {err}"))?;
    let actual = listener
        .local_addr()
        .map_err(|err| format!("failed to read listening address: {err}"))?;
    println!("MFB_REPO_LISTEN={actual}");
    // `into_make_service_with_connect_info` exposes each connection's peer
    // `SocketAddr` to handlers via `ConnectInfo`, so register/login can throttle
    // per client IP instead of one shared global bucket (bug-188 / REPO-12).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|err| format!("repository server failed: {err}"))?;
    Ok(actual)
}
// coverage:on

/// Assemble the route table.
///
/// Split out of `serve` so it can be constructed in a test without binding a
/// listener. That matters for more than tidiness: `Router::route` **panics on a
/// route conflict at construction time**, so a bad table is a dead server at
/// startup rather than a failing handler test. `GET /packages/:ident` puts a
/// parameter segment beside the static `/packages/transfer/*` routes, and
/// `router_has_no_route_conflicts` is what proves matchit resolves that the way
/// this table assumes (static wins over param).
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ident", get(server_ident))
        .route("/accounts/register", post(register))
        .route("/auth/challenge", post(challenge))
        .route("/auth/login", post(login))
        .route("/signing", post(signing))
        .route("/log/checkpoint", get(log_checkpoint))
        .route("/log/proof/:index", get(log_inclusion_proof))
        .route("/log/consistency", get(log_consistency_proof))
        .route("/log/publish", get(log_publish_entry))
        .route("/keys/rotate", post(rotate_ident))
        .route("/idents/:owner", get(ident_chain))
        .route("/machines/link", post(link_start))
        .route("/machines/link/fetch", post(link_fetch))
        .route("/machines/revoke/challenge", post(revoke_challenge))
        .route("/machines/revoke", post(revoke_machine))
        // plan-61-C: the HTML surface. All GET, all anonymous, all carrying
        // the CSP from `web::html_response`.
        .route("/", get(landing_page))
        .route("/style.css", get(stylesheet))
        .route("/search.html", get(search_html))
        .route("/p/:ident", get(package_page_html))
        .route("/p/:ident/audit", get(package_audit_html))
        .route("/p/:ident/docs", get(package_docs_html))
        .route("/search", get(search))
        .route("/index/:ident", get(package_index))
        // plan-61-B: anonymous read surface. These read no credential of any
        // kind — no session token, no bearer header, no cookie — and are `GET`
        // only. `:ident` sits beside the static `/packages/transfer/*` routes
        // below; matchit resolves static before param.
        .route("/packages/:ident", get(package_detail))
        .route("/packages/:ident/audit", get(package_audit))
        .route("/packages/:ident/docs", get(package_docs))
        .route(
            "/blob/:hash",
            get(package_blob).head(head_blob).put(put_blob),
        )
        .route("/release-state", post(release_state))
        .route("/orgs/members", post(org_members))
        .route("/tokens", post(issue_token))
        .route("/tokens/revoke", post(revoke_token))
        .route("/packages/transfer/offer", post(transfer_offer))
        .route("/packages/transfer/accept", post(transfer_accept))
        .route("/root.json", get(root_metadata))
        .route("/snapshot.json", get(snapshot_metadata))
        .route("/timestamp.json", get(timestamp_metadata))
        .route("/validate", post(validate_package))
        .route("/publish", post(publish_package))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

fn bad_request(message: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: message }),
    )
}

fn conflict_or_bad_request(message: String) -> (StatusCode, Json<ErrorResponse>) {
    if message.contains("already in use") || message.contains("reused challenge") {
        (StatusCode::CONFLICT, Json(ErrorResponse { error: message }))
    } else {
        bad_request(message)
    }
}

/// 404 with the standard error shape. Used by the plan-61-B read routes, which
/// answer "no such package" for an unknown owner and an unknown package alike
/// rather than letting a caller tell the two apart.
fn not_found(message: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse { error: message }),
    )
}

/// 500 with the standard error shape. Every internal failure is also written to
/// stderr: the response goes only to the caller, so without this line the
/// operator's log has no record of the failure at all.
fn internal(message: String) -> (StatusCode, Json<ErrorResponse>) {
    eprintln!("internal error: {message}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: message }),
    )
}

fn unauthorized(message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            error: message.to_string(),
        }),
    )
}

#[cfg(test)]
mod tests;
