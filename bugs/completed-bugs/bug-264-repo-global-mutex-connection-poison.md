# bug-264: registry serializes all DB access on one `Arc<Mutex<Connection>>` that permanently poisons on a panic

Last updated: 2026-07-17
Effort: large (3h–1d)
Severity: MEDIUM
Class: Availability / robustness

Status: Fixed
Regression Test: `repository/src/store.rs` —
`poisoned_connection_lock_recovers_and_keeps_serving` (poisons the connection
mutex via a panic-while-held, then proves reads and writes still succeed)

## Resolution

Chose Fix Design option (b): a non-poisoning acquisition. A new
`Store::conn(&self)` helper acquires the connection guard with
`self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())`, recovering
the inner guard from a `PoisonError` instead of returning "database lock poisoned"
on every subsequent request. All 47 `self.conn.lock().map_err(...)?` call sites
were replaced with the infallible `self.conn()`. The SQLite connection stays
usable across a Rust panic (rusqlite statements are transactional; an in-flight
transaction rolls back on guard drop), so recovery is correct. The rate-limiter
mutex in `server.rs` got the same poison-recovery treatment.

The single-connection serialization (the read-concurrency half of the goal) is
left in place: it is the sanctioned trade-off of keeping one connection under
option (b), and a connection-pool refactor would compose with REPO-16's
read-amplification memoization (tracked in bug-271) rather than land in isolation.
The severity driver — a single reachable panic permanently DoS-ing the whole
registry — is removed.

`mfb-repo` holds a single `Arc<Mutex<Connection>>` and takes the lock for every
database access, reads included. This serialises the whole service on one
connection (no read concurrency), and — more seriously — a panic while the lock
is held **poisons** the mutex permanently: every subsequent request fails with
"database lock poisoned" until the process is restarted. A single reachable panic
in a critical section is a full-service DoS. The single correct behavior a fix
produces: a panic in one request cannot wedge the entire registry, and reads do
not contend on a single global lock.

References:

- `planning/audit-2-repository.md` (REPO-09).
- `planning/old-plans/audit-1-*` (original REPO-09).
- `repository/src/store.rs:13` — `Arc<Mutex<Connection>>` guarding all DB access.

## Failing Reproduction

Any code path that panics while holding the connection lock (e.g. an
`unwrap`/`expect`/slice-index panic inside a `store` method between lock
acquisition and release). Observed: after that request, `Mutex::lock()` returns
`PoisonError` and every following DB operation returns "database lock poisoned"
— the registry is down until restart. Expected: the failed request errors; other
requests continue to serve.

Note: rusqlite returns `Result` for most misuse, so a reachable panic is *hard*
to trigger today (practical severity ~LOW–MEDIUM), but the architecture makes any
future panic catastrophic rather than local.

## Root Cause

One `Mutex<Connection>` (`store.rs:13`) is both the concurrency primitive and the
single connection. `std::sync::Mutex` poisons on panic-while-held, so a panic
anywhere in a store critical section makes the lock permanently unusable; and
because reads take the same lock, there is no read parallelism even in the happy
path.

## Goal

- A panic in one DB critical section does not permanently disable the service
  (either the poison is recovered, or panics cannot occur in the critical
  section), and read queries do not serialise behind a single global lock.

### Non-goals (must NOT change)

- The on-disk SQLite schema / data.
- The transactional semantics of existing mutating routes.

## Fix Design

Options, in rough preference order: (a) move to an `r2d2`/`deadpool`-style
connection **pool** (per-request connection; SQLite WAL mode gives concurrent
readers) — removes both the single-lock serialization and the shared-poison blast
radius; (b) if keeping one connection, use a non-poisoning lock (`parking_lot::
Mutex`) or recover from `PoisonError` explicitly and ensure critical sections are
panic-free (return `Result`, no `unwrap`). Recommend (a): it addresses REPO-16's
read-amplification pressure too.
