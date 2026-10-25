//! plan-176-L: the registry server's limits code. `AppState`, `serve`, the router and the
//! error responses live in `repository/src/server/mod.rs`.

use super::*;

/// A minimal in-memory sliding-window rate limiter (plan-10-D2). Keyed per
/// endpoint (and, where meaningful, per owner), it caps abusive bursts on the
/// cheap, loggable operations (register/challenge/login/signing) without a
/// dependency. Approximate and process-local — a real deployment fronts this
/// with a proxy — but enough to keep the transparency log spam-free.
#[derive(Clone)]
pub(super) struct RateLimiter {
    /// `pub(super)` so `server::tests` can poison the lock to pin the bug-264
    /// recovery below; nothing outside `limits` touches the map otherwise.
    pub(super) hits: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<i64>>>>,
}

impl RateLimiter {
    pub(super) fn new() -> Self {
        Self {
            hits: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Record a hit for `key`; return false if it exceeded `max` hits within
    /// the last `window_secs`.
    pub(super) fn allow(&self, key: &str, max: usize, window_secs: i64) -> bool {
        let now = now_unix();
        // Recover a poisoned lock rather than panicking every subsequent call: a
        // panic while this lock was held must not wedge rate limiting for the
        // whole process (bug-264 / REPO-09). The hit map is plain data with no
        // cross-field invariant a mid-panic leaves broken.
        let mut hits = self
            .hits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = hits.entry(key.to_string()).or_default();
        entry.retain(|timestamp| now - *timestamp < window_secs);
        if entry.len() >= max {
            return false;
        }
        entry.push(now);
        true
    }

    /// Drop keys whose windows have fully elapsed, so the map stays bounded.
    pub(super) fn prune(&self, window_secs: i64) {
        let now = now_unix();
        // Recover a poisoned lock rather than panicking every subsequent call: a
        // panic while this lock was held must not wedge rate limiting for the
        // whole process (bug-264 / REPO-09). The hit map is plain data with no
        // cross-field invariant a mid-panic leaves broken.
        let mut hits = self
            .hits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        hits.retain(|_key, times| {
            times.retain(|timestamp| now - *timestamp < window_secs);
            !times.is_empty()
        });
    }
}

pub(super) fn too_many_requests() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(ErrorResponse {
            error: "rate limit exceeded; slow down".to_string(),
        }),
    )
}

/// Maximum inline request body (plan-10-D2): caps the base64 artifact carried
/// by `/validate` and `/publish` so a single upload cannot exhaust memory.
pub(super) const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Anonymous transparency-log routes, shared per-IP budget (bug-579).
///
/// Sized from the WORST LEGITIMATE CLIENT, not from a round number. A single
/// `verify_publish_inclusion` makes three log requests (`/log/checkpoint`,
/// `/log/publish`, `/log/proof/:index`) and `mfb pkg install --proof` calls it
/// once PER DEPENDENCY, so a 200-dependency install is a ~600-request burst in
/// seconds — and every developer behind one NAT or one CI egress IP shares this
/// key. A tight budget here would not harden the registry, it would break
/// installs. 1200/minute leaves 2x headroom over that worst case while still
/// converting "unbounded" into "bounded".
///
/// Note this grew with bug-582: a client now fetches a consistency proof on
/// every pin advance, so the log is contacted more than it used to be. Any
/// future tightening has to be re-derived from the client, not guessed.
pub(super) const LOG_PER_IP_MAX: usize = 1200;

/// Per-client (peer-IP) rate caps on the anonymous auth endpoints, replacing the
/// old global-string buckets that let one client lock the whole user base out of
/// registration/login (audit-2 REPO-12 / bug-188). A generous global ceiling is
/// kept as a secondary backstop so extreme aggregate abuse is still bounded, but
/// it is high enough never to trip on one abuser's traffic.
pub(super) const REGISTER_PER_IP_MAX: usize = 20;

pub(super) const LOGIN_PER_IP_MAX: usize = 30;

/// Per-IP caps on `/auth/challenge` and `/signing` (bug-419). Both were
/// previously keyed on the **unauthenticated** `request.owner`, so an anonymous
/// attacker could exhaust a targeted victim's bucket — locking that account out
/// of login (a challenge is the mandatory login prerequisite) or of publish
/// attestations. Re-keyed to the peer IP, following `REGISTER_PER_IP_MAX` /
/// `LOGIN_PER_IP_MAX`, with `AUTH_GLOBAL_CEILING` as the secondary backstop.
/// Same numeric budgets the owner-keyed buckets used, now spent per client.
pub(super) const CHALLENGE_PER_IP_MAX: usize = 20;

pub(super) const SIGNING_PER_IP_MAX: usize = 60;

pub(super) const AUTH_GLOBAL_CEILING: usize = 2000;

/// Per-IP sliding-window cap on `GET /search` (plan-61-B Phase 2).
///
/// Keyed on the **peer IP**, following `REGISTER_PER_IP_MAX` /
/// `LOGIN_PER_IP_MAX`, because `/search` is anonymous and has no `claims.sub`
/// to key on the way `BLOB_UPLOAD_PER_OWNER_MAX` does. Higher than the auth
/// caps because a search box legitimately issues many requests, and search is
/// the only route in this sub-plan that does real query work per call.
pub(super) const SEARCH_PER_IP_MAX: usize = 120;

/// Server-side ceiling on `?limit`. An uncapped limit on an anonymous
/// enumerate route is a trivial resource-exhaustion lever, so an over-cap
/// request is **clamped**, never honoured and never rejected.
pub(super) const SEARCH_LIMIT_MAX: i64 = 50;

pub(super) const SEARCH_LIMIT_DEFAULT: i64 = 20;

/// How much of a description a search result carries (plan-61-E §Open
/// Decisions).
///
/// Clamped on a **character** boundary, not a byte one: descriptions are UTF-8
/// and a byte clamp can split a multi-byte character. Without a clamp, a page
/// of 50 results could carry 50 × 4096 bytes of description on an anonymous
/// route — the same resource lever the `limit` cap exists to close.
pub(super) const SEARCH_DESCRIPTION_PREVIEW_CHARS: usize = 200;

/// Per-owner sliding-window caps on the authenticated package endpoints, whose
/// only prior protection was the shared 64 MiB body cap — a registered (near
/// anonymous) client could hammer `/validate` (5 Ed25519 verifies/call) for CPU
/// or `/publish` for permanent disk (audit-2 REPO-13 / bug-188).
pub(super) const VALIDATE_PER_OWNER_MAX: usize = 60;

pub(super) const PUBLISH_PER_OWNER_MAX: usize = 30;

/// Per-owner sliding-window cap on native-blob uploads (plan-48-A §4.3).
/// Non-optional: without it an authenticated publisher can fill the datapath
/// with 64 MiB objects that nothing can ever reclaim (the registry has no GC).
/// A 7-slot binding is 7 PUTs, so this must comfortably clear one binding's
/// full target set while still bounding a flood.
pub(super) const BLOB_UPLOAD_PER_OWNER_MAX: usize = 120;

/// Per-owner published-version quota: the total number of `package_versions`
/// rows an owner may accumulate. Bounds permanent blob/DB growth from an
/// authenticated flood without constraining any realistic publisher.
pub(super) const MAX_VERSIONS_PER_OWNER: i64 = 10_000;
