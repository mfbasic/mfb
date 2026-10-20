//! plan-176-L: the registry store's blob bookkeeping — kinds, native blobs and reachability.
//! The `Store` handle and its connection live in `repository/src/store/mod.rs`.

use super::*;

/// One `package_blobs` row, as the garbage collector sees it (plan-49).
///
/// `path` is the `blob_ref` recorded when the row was written — informational
/// only, and a stale one if the datapath moved. The collector addresses the
/// backing object by `hash` + `kind`, exactly the way `GET /blob/<hash>` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobRow {
    pub hash: String,
    pub path: String,
    pub kind: String,
    pub created_at: i64,
}

impl BlobRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(BlobRow {
            hash: row.get(0)?,
            path: row.get(1)?,
            kind: row.get(2)?,
            created_at: row.get(3)?,
        })
    }
}

impl Store {
    /// The stored `kind` of a blob (`"package"` or `"native"`), or `None` when
    /// no blob row exists for `hash` (plan-48-A §4.1). Lets `GET /blob/<hash>`
    /// 404 an unknown hash from SQLite and select the right backend name/suffix
    /// without an S3 round trip.
    pub fn blob_kind(&self, hash: &str) -> Result<Option<String>, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT kind FROM package_blobs WHERE hash = ?1",
            params![hash],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| format!("failed to load blob kind: {err}"))
    }

    /// Record a freshly uploaded native-library blob's metadata row
    /// (plan-48-A §4.3). Idempotent: re-uploading an existing blob leaves the
    /// original row untouched.
    /// Returns whether the caller should go on to promote a `<hash>.bin`.
    ///
    /// `package_blobs` is keyed by hash alone and `GET /blob/<hash>` picks the
    /// backend file from that row's `kind`. So when a package blob already exists
    /// with these exact bytes, the `INSERT OR IGNORE` is ignored and the row keeps
    /// `kind='package'` — at which point writing a `.bin` too produced a second
    /// copy that nothing ever reads or collects (bug-276 R5).
    ///
    /// Reporting `false` there is safe precisely because the blob is
    /// content-addressed: the existing object holds byte-identical content and
    /// `GET` already serves it under the recorded kind, so the upload is a
    /// genuine no-op rather than a dropped write.
    pub fn record_native_blob(&self, hash: &str, blob_path: &str) -> Result<bool, String> {
        let now = now_unix();
        let conn = self.conn();
        let existing: Option<String> = conn
            .query_row(
                "SELECT kind FROM package_blobs WHERE hash = ?1",
                params![hash],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|err| format!("failed to load native blob metadata: {err}"))?;
        if let Some(kind) = existing {
            return Ok(kind == "native");
        }
        conn.execute(
            "INSERT INTO package_blobs (hash, path, kind, created_at)
             VALUES (?1, ?2, 'native', ?3)",
            params![hash, blob_path, now],
        )
        .map_err(|err| format!("failed to store native blob metadata: {err}"))?;
        Ok(true)
    }

    // --- Blob garbage collection (plan-49) ---------------------------------

    /// Every blob row **no live package version references**, older than
    /// `grace_seconds` at `now` (plan-49 §4.1/§4.2).
    ///
    /// The reachable set is `package_versions.hash` **∪**
    /// `package_version_blobs.hash`, and *both* halves are load-bearing:
    /// `package_versions.hash` is the `.mfp` artifact itself, which is not a
    /// vendor blob and therefore never appears in `package_version_blobs`.
    /// Dropping the first half would report every published package as garbage.
    ///
    /// Reachability is recomputed from these two tables on every call rather
    /// than tracked in a refcount (plan-49 §3.4): a refcount drifts permanently
    /// on any crash between its two writes, and a drifted one either leaks
    /// forever or deletes a live blob. This is self-correcting instead.
    ///
    /// A **yanked** version is still a live row and so still reachable
    /// (plan-49 §3.2): yanking says "do not resolve this by default", not
    /// "delete it", and lockfiles pinning the hash must keep installing. Only a
    /// version row that no longer exists releases its blobs.
    ///
    /// `now` is a parameter rather than read from the clock so a caller — and a
    /// test — can ask what the candidate set looks like at a given instant.
    pub fn unreachable_blobs(&self, now: i64, grace_seconds: i64) -> Result<Vec<BlobRow>, String> {
        if grace_seconds < 0 {
            return Err("grace period must not be negative".to_string());
        }
        // A grace period large enough to underflow would wrap into the future
        // and make every blob a candidate — refuse it rather than sweep the
        // whole registry.
        let cutoff = now
            .checked_sub(grace_seconds)
            .ok_or_else(|| format!("grace period of {grace_seconds}s overflows the clock"))?;
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT hash, path, kind, created_at FROM package_blobs
                 WHERE hash NOT IN (SELECT hash FROM package_versions
                                    UNION
                                    SELECT hash FROM package_version_blobs)
                   AND created_at < ?1
                 ORDER BY created_at, hash",
            )
            .map_err(|err| format!("failed to prepare blob reachability query: {err}"))?;
        let rows = statement
            .query_map(params![cutoff], BlobRow::from_row)
            .map_err(|err| format!("failed to scan blobs: {err}"))?;
        let mut blobs = Vec::new();
        for row in rows {
            blobs.push(row.map_err(|err| format!("failed to read blob row: {err}"))?);
        }
        Ok(blobs)
    }

    /// Every blob row a live package version *does* reference — the complement
    /// of [`Store::unreachable_blobs`] with no grace-period filter, so a `gc`
    /// report can state what fraction of the store is garbage.
    pub fn reachable_blobs(&self) -> Result<Vec<BlobRow>, String> {
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT hash, path, kind, created_at FROM package_blobs
                 WHERE hash IN (SELECT hash FROM package_versions
                                UNION
                                SELECT hash FROM package_version_blobs)
                 ORDER BY created_at, hash",
            )
            .map_err(|err| format!("failed to prepare reachable blob query: {err}"))?;
        let rows = statement
            .query_map([], BlobRow::from_row)
            .map_err(|err| format!("failed to scan reachable blobs: {err}"))?;
        let mut blobs = Vec::new();
        for row in rows {
            blobs.push(row.map_err(|err| format!("failed to read blob row: {err}"))?);
        }
        Ok(blobs)
    }

    /// Whether one hash is referenced by a live version right now.
    ///
    /// The sweep re-asks this immediately before deleting each object, rather
    /// than trusting the scan it started with. The grace period makes the scan
    /// safe against an *in-flight* publish (§3.1), but a publisher who uploads a
    /// blob and publishes more than a grace period later is not in flight — they
    /// are slow, and their blob is a legitimate candidate right up until the
    /// publish commits. Re-checking narrows that window from the length of the
    /// whole sweep to the gap between this query and the delete.
    ///
    /// It does not *close* the window; nothing short of a lock would, and the
    /// design deliberately has none. `forget_blob`'s foreign key is the backstop
    /// for the remainder.
    pub fn blob_is_reachable(&self, hash: &str) -> Result<bool, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM package_versions WHERE hash = ?1
                           UNION
                           SELECT 1 FROM package_version_blobs WHERE hash = ?1)",
            params![hash],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists != 0)
        .map_err(|err| format!("failed to re-check blob reachability: {err}"))
    }

    /// Drop a `package_blobs` row after its backing object has been deleted
    /// (plan-49 §4.4). Reports whether a row was actually removed, so a second
    /// `gc --delete` over the same hash is a visible no-op rather than a
    /// silent one.
    ///
    /// `package_version_blobs.hash` is a foreign key onto this row and
    /// `foreign_keys` is `ON`, so if a publish landed an edge between the scan
    /// and this delete, SQLite refuses it and the error surfaces instead of
    /// orphaning a live version's vendor blob.
    pub fn forget_blob(&self, hash: &str) -> Result<bool, String> {
        let conn = self.conn();
        let removed = conn
            .execute("DELETE FROM package_blobs WHERE hash = ?1", params![hash])
            .map_err(|err| format!("failed to delete blob metadata for {hash}: {err}"))?;
        Ok(removed > 0)
    }
}
