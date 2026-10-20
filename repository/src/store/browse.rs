//! plan-176-L: the registry store's browsing — package detail, audit, search and typosquat checks.
//! The `Store` handle and its connection live in `repository/src/store/mod.rs`.

use super::*;

impl Store {
    /// Everything `GET /packages/:ident` renders: the package's owner, its
    /// recorded metadata, and **every** version with its native target rows
    /// (plan-61-B §3).
    ///
    /// Returns `None` when no package carries this ident.
    ///
    /// No `state` filter, deliberately and permanently. A yanked or superseded
    /// version is still part of the published history, and a view that silently
    /// omits non-current versions reproduces exactly the truncation the open
    /// SUP-03 downgrade attack performs (`plan-61-repo-web.md` §4). `state` is a
    /// field for the caller to render, never a filter this query applies.
    pub fn package_detail(&self, ident: &str) -> Result<Option<PackageDetailRow>, String> {
        let conn = self.conn();
        let owner: Option<String> = conn
            .query_row(
                "SELECT o.owner_display FROM packages p
                 JOIN owners o ON o.id = p.owner_id
                 WHERE p.ident = ?1",
                params![ident],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load package: {err}"))?;
        let Some(owner) = owner else {
            return Ok(None);
        };

        let mut statement = conn
            .prepare(
                "SELECT pv.id, pv.version, pv.hash, pv.created_at, pv.state,
                        pv.abi_index, pv.author, pv.url, pv.description
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1
                 ORDER BY pv.created_at DESC, pv.id DESC",
            )
            .map_err(|err| format!("failed to prepare detail query: {err}"))?;
        let rows = statement
            .query_map(params![ident], |row| {
                Ok(PackageDetailVersion {
                    id: row.get(0)?,
                    version: row.get(1)?,
                    hash: row.get(2)?,
                    published_at: row.get(3)?,
                    state: row.get(4)?,
                    abi_index: row.get(5)?,
                    author: row.get(6)?,
                    url: row.get(7)?,
                    description: row.get(8)?,
                    targets: Vec::new(),
                })
            })
            .map_err(|err| format!("failed to load package detail: {err}"))?;
        let mut versions = Vec::new();
        for row in rows {
            versions.push(row.map_err(|err| format!("failed to read package detail: {err}"))?);
        }

        let mut target_statement = conn
            .prepare(
                "SELECT os, arch, libc, lib_type, logical, source, blob_hash
                 FROM package_version_targets
                 WHERE package_version_id = ?1
                 ORDER BY rowid",
            )
            .map_err(|err| format!("failed to prepare target query: {err}"))?;
        for version in &mut versions {
            let rows = target_statement
                .query_map(params![version.id], |row| {
                    Ok(PackageTargetRow {
                        os: row.get(0)?,
                        arch: row.get(1)?,
                        libc: row.get(2)?,
                        lib_type: row.get(3)?,
                        logical: row.get(4)?,
                        source: row.get(5)?,
                        blob_hash: row.get(6)?,
                    })
                })
                .map_err(|err| format!("failed to load version targets: {err}"))?;
            for row in rows {
                version
                    .targets
                    .push(row.map_err(|err| format!("failed to read version target: {err}"))?);
            }
        }

        Ok(Some(PackageDetailRow { owner, versions }))
    }

    /// The transparency record for one package: its release-state history and
    /// its owner's ident-key rotations (plan-61-B §4). The publish log entries
    /// and inclusion proofs are assembled by the handler from the existing log
    /// API, which already knows how to build them.
    ///
    /// Returns `None` for an unknown ident, so the handler can 404 without a
    /// second existence query.
    pub fn package_audit(&self, ident: &str) -> Result<Option<PackageAuditRow>, String> {
        let conn = self.conn();
        let owner: Option<String> = conn
            .query_row(
                "SELECT o.owner_display FROM packages p
                 JOIN owners o ON o.id = p.owner_id
                 WHERE p.ident = ?1",
                params![ident],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load package: {err}"))?;
        let Some(owner) = owner else {
            return Ok(None);
        };

        let mut statement = conn
            .prepare(
                "SELECT pv.version, rsc.state, rsc.created_at
                 FROM release_state_changes rsc
                 JOIN package_versions pv ON pv.id = rsc.package_version_id
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1
                 ORDER BY rsc.id ASC",
            )
            .map_err(|err| format!("failed to prepare state-change query: {err}"))?;
        let rows = statement
            .query_map(params![ident], |row| {
                Ok(ReleaseStateChangeRow {
                    version: row.get(0)?,
                    state: row.get(1)?,
                    at: row.get(2)?,
                })
            })
            .map_err(|err| format!("failed to load state changes: {err}"))?;
        let mut state_changes = Vec::new();
        for row in rows {
            state_changes.push(row.map_err(|err| format!("failed to read state change: {err}"))?);
        }

        // Query the versions on the connection already held. Calling
        // `list_package_versions` here would re-enter `self.conn()` while this
        // function still owns the guard, and the mutex is not reentrant — that
        // deadlocks the handler rather than failing it.
        let mut version_statement = conn
            .prepare(
                "SELECT pv.version FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1
                 ORDER BY pv.created_at ASC, pv.id ASC",
            )
            .map_err(|err| format!("failed to prepare version query: {err}"))?;
        let rows = version_statement
            .query_map(params![ident], |row| row.get::<_, String>(0))
            .map_err(|err| format!("failed to list package versions: {err}"))?;
        let mut versions = Vec::new();
        for row in rows {
            versions.push(row.map_err(|err| format!("failed to read package version: {err}"))?);
        }
        drop(version_statement);
        drop(statement);
        drop(conn);

        Ok(Some(PackageAuditRow {
            owner,
            versions,
            state_changes,
        }))
    }

    /// Ranked package search for `GET /search` (plan-61-B §3).
    ///
    /// Ranking is exact ident, then ident prefix, then ident substring, then
    /// owner-name match, then an edit-distance-1 fuzzy tail. Ties break on
    /// ident so results are stable between requests.
    ///
    /// **The fuzzy tail runs only when the first four find nothing.** Phase 2's
    /// measurement task: `typosquat_candidates` is `SELECT ident FROM packages`
    /// — an unindexed full table scan with per-row Levenshtein in Rust
    /// (`store.rs:2438-2454`). It was written for one call per publish, not for
    /// a per-keystroke anonymous search route, so it is gated rather than
    /// wired into the common path.
    ///
    /// `query` is matched with `LIKE ... ESCAPE '\'` and the user's `%`, `_`
    /// and `\` are escaped, so a query of `%` matches a literal percent sign
    /// rather than every package.
    pub fn search_packages(
        &self,
        query: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SearchResultRow>, String> {
        let query = query.trim();
        // An empty query returns nothing rather than the whole table: this is
        // an anonymous route, and "no filter" must not mean "enumerate
        // everything".
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let folded = query.to_lowercase();
        let pattern = escape_like(&folded);
        let conn = self.conn();

        // Rank against the **package component** as well as the whole ident.
        // A user searching `sql` means the package named `sql`, not the literal
        // string `alice#sql` — ranking only on the full ident put every result
        // in the substring bucket and reduced the order to alphabetical.
        // `substr(ident, instr(ident, '#') + 1)` is the package half.
        let mut statement = conn
            .prepare(
                "SELECT p.ident, o.owner_display,
                        CASE
                            WHEN lower(p.ident) = ?1
                              OR lower(substr(p.ident, instr(p.ident, '#') + 1)) = ?1 THEN 0
                            WHEN lower(p.ident) LIKE ?2 ESCAPE '\\'
                              OR lower(substr(p.ident, instr(p.ident, '#') + 1))
                                 LIKE ?2 ESCAPE '\\' THEN 1
                            WHEN lower(p.ident) LIKE ?3 ESCAPE '\\' THEN 2
                            WHEN lower(o.owner_display) LIKE ?3 ESCAPE '\\' THEN 3
                            ELSE 4
                        END AS rank
                 FROM packages p
                 JOIN owners o ON o.id = p.owner_id
                 WHERE lower(p.ident) = ?1
                    OR lower(p.ident) LIKE ?3 ESCAPE '\\'
                    OR lower(o.owner_display) LIKE ?3 ESCAPE '\\'
                    -- plan-61-E: description text, ranked *below* ident and
                    -- owner. Scoped to the newest version so an old version's
                    -- description cannot keep a package findable by a word its
                    -- current release no longer contains.
                    OR EXISTS (
                        SELECT 1 FROM package_versions pv
                        WHERE pv.package_id = p.id
                          AND pv.id = (
                              SELECT id FROM package_versions
                              WHERE package_id = p.id
                              ORDER BY created_at DESC, id DESC LIMIT 1
                          )
                          AND lower(pv.description) LIKE ?3 ESCAPE '\\'
                    )
                 ORDER BY rank ASC, p.ident ASC
                 LIMIT ?4 OFFSET ?5",
            )
            .map_err(|err| format!("failed to prepare search query: {err}"))?;
        let rows = statement
            .query_map(
                params![
                    folded,
                    format!("{pattern}%"),
                    format!("%{pattern}%"),
                    limit,
                    offset
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|err| format!("failed to search packages: {err}"))?;
        let mut idents = Vec::new();
        for row in rows {
            idents.push(row.map_err(|err| format!("failed to read search result: {err}"))?);
        }
        drop(statement);

        // Fuzzy tail: only reached when nothing above matched, and only on the
        // first page — an offset past the end of an empty result set is still
        // empty.
        if idents.is_empty() && offset == 0 {
            let mut fuzzy = Vec::new();
            for (ident, owner) in fuzzy_search_candidates(&conn, folded.chars().count())? {
                if within_edit_distance_one(&ident.to_lowercase(), &folded) {
                    fuzzy.push((ident, owner));
                }
            }
            fuzzy.sort();
            fuzzy.truncate(limit.max(0) as usize);
            idents = fuzzy;
        }

        // Attach each package's newest **active** release. Done per package
        // rather than in the ranking query so the rank expression stays
        // readable; the page is already capped by `limit`.
        //
        // plan-126-A: the state predicate is the same allowlist
        // `Store::latest_active_version` applies, and `pv.state` is selected
        // alongside so the result chip can be badged. Before this, the search
        // page showed a yanked release as a package's current version with no
        // state marker of any kind — `SearchResultRow` carried no state field,
        // so there was nothing a renderer could have marked it with. That made
        // this the one surface where the missing filter was not merely cosmetic.
        let mut latest_statement = conn
            .prepare(
                "SELECT pv.version, pv.created_at, pv.description, pv.state
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.state IN ('available', 'deprecated')
                 ORDER BY pv.created_at DESC, pv.id DESC
                 LIMIT 1",
            )
            .map_err(|err| format!("failed to prepare latest-version query: {err}"))?;
        let mut results = Vec::new();
        for (ident, owner) in idents {
            let latest = latest_statement
                .query_row(params![ident], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .optional()
                .map_err(|err| format!("failed to read latest version: {err}"))?;
            // A match with no active release stays a match: the package exists
            // and the query found it. Only its headline version is absent, and
            // the page states that rather than dropping the row — a search that
            // silently hid a fully-yanked package would be the same truncation
            // the Overview's complete version table exists to prevent.
            let (latest_version, published_at, description, latest_state) = match latest {
                Some((version, at, description, release_state)) => {
                    (Some(version), Some(at), description, Some(release_state))
                }
                None => (None, None, None, None),
            };
            results.push(SearchResultRow {
                ident,
                owner,
                latest_version,
                latest_state,
                published_at,
                description,
            });
        }
        Ok(results)
    }

    /// Existing package idents within edit distance 1 of `ident` (excluding an
    /// exact match) — the warn-only typosquat check at publish (plan-10-D2).
    pub fn typosquat_candidates(&self, ident: &str) -> Result<Vec<String>, String> {
        let conn = self.conn();
        let mut statement = conn
            .prepare("SELECT ident FROM packages")
            .map_err(|err| format!("failed to prepare typosquat query: {err}"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|err| format!("failed to scan packages: {err}"))?;
        let mut candidates = Vec::new();
        for row in rows {
            let existing = row.map_err(|err| format!("failed to read package ident: {err}"))?;
            if existing != ident && within_edit_distance_one(&existing, ident) {
                candidates.push(existing);
            }
        }
        Ok(candidates)
    }
}

/// One `GET /search` hit (plan-61-B §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResultRow {
    pub ident: String,
    pub owner: String,
    /// The newest **active** release (plan-126-A). `None` for a package
    /// identity with no published version yet — and also for one whose every
    /// published version is yanked, blocked or legal-tombstoned.
    pub latest_version: Option<String>,
    /// The release state of `latest_version`, so a `deprecated` headline is
    /// badged rather than reading as current. `None` exactly when
    /// `latest_version` is.
    pub latest_state: Option<String>,
    pub published_at: Option<i64>,
    /// NULL until plan-61-E.
    pub description: Option<String>,
}

/// Escape a user string for use inside a `LIKE` pattern with `ESCAPE '\'`.
///
/// Without this, a query of `%` matches every package and `_` matches any
/// single character — a caller could enumerate the whole registry through a
/// route whose entire point is that an empty query returns nothing. The
/// backslash must be escaped first, or it would double-escape the escapes this
/// function itself inserts.
fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// One package as `GET /packages/:ident` sees it (plan-61-B).
#[derive(Debug, Clone)]
pub struct PackageDetailRow {
    pub owner: String,
    /// Every version, newest first, regardless of release state.
    pub versions: Vec<PackageDetailVersion>,
}

#[derive(Debug, Clone)]
pub struct PackageDetailVersion {
    /// `package_versions.id` — used to attach targets, not rendered.
    pub id: i64,
    pub version: String,
    pub hash: String,
    pub published_at: i64,
    pub state: String,
    pub abi_index: String,
    pub author: Option<String>,
    pub url: Option<String>,
    /// NULL until plan-61-E populates it.
    pub description: Option<String>,
    pub targets: Vec<PackageTargetRow>,
}

/// One row of the native target matrix. `arch = None` is the any-arch
/// wildcard, not missing data (plan-61-A §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTargetRow {
    pub os: String,
    pub arch: Option<String>,
    pub libc: Option<String>,
    pub lib_type: String,
    pub logical: String,
    pub source: String,
    pub blob_hash: Option<String>,
}

/// The transparency record for one package (plan-61-B §4).
#[derive(Debug, Clone)]
pub struct PackageAuditRow {
    pub owner: String,
    pub versions: Vec<String>,
    pub state_changes: Vec<ReleaseStateChangeRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseStateChangeRow {
    pub version: String,
    pub state: String,
    pub at: i64,
}

/// Whether two strings are within Levenshtein edit distance 1 (a single
/// insert, delete, or substitution), used for the typosquat warning.
/// Length-bounded candidate set for the fuzzy search tail (bug-420 item 3).
///
/// `within_edit_distance_one` can only return true when the two strings' lengths
/// differ by at most one, so the fuzzy tail never needs to look at a package
/// whose ident length is outside `[query_len - 1, query_len + 1]`. Pushing that
/// bound into SQL keeps the anonymous no-match search path from pulling every
/// `(ident, owner)` row into Rust and paying a per-row `Vec<char>` allocation +
/// Levenshtein — the O(n) full-scan amplification bug-420 flagged. It is a
/// *candidate* filter only: the exact edit-distance check still runs in Rust, so
/// results are identical to the old unbounded scan. (Idents are validated ASCII,
/// so SQLite's `length()` matches the Rust `chars().count()` the distance check
/// uses.)
pub(super) fn fuzzy_search_candidates(
    conn: &Connection,
    query_len: usize,
) -> Result<Vec<(String, String)>, String> {
    let low = query_len.saturating_sub(1) as i64;
    let high = query_len.saturating_add(1) as i64;
    let mut statement = conn
        .prepare(
            "SELECT p.ident, o.owner_display FROM packages p
             JOIN owners o ON o.id = p.owner_id
             WHERE length(p.ident) BETWEEN ?1 AND ?2",
        )
        .map_err(|err| format!("failed to prepare fuzzy query: {err}"))?;
    let rows = statement
        .query_map(params![low, high], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|err| format!("failed to scan packages: {err}"))?;
    let mut candidates = Vec::new();
    for row in rows {
        candidates.push(row.map_err(|err| format!("failed to read package: {err}"))?);
    }
    Ok(candidates)
}

pub(super) fn within_edit_distance_one(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (la, lb) = (a.len(), b.len());
    if la.abs_diff(lb) > 1 {
        return false;
    }
    if la == lb {
        // At most one substitution.
        return a.iter().zip(&b).filter(|(x, y)| x != y).count() <= 1;
    }
    // Lengths differ by exactly one: check for a single insertion/deletion.
    let (short, long) = if la < lb { (&a, &b) } else { (&b, &a) };
    let mut i = 0usize;
    let mut j = 0usize;
    let mut edits = 0usize;
    while i < short.len() && j < long.len() {
        if short[i] == long[j] {
            i += 1;
            j += 1;
        } else {
            edits += 1;
            if edits > 1 {
                return false;
            }
            j += 1; // skip a char in the longer string
        }
    }
    true
}
