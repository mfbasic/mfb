//! plan-176-L: the registry store's package versions — publishing, listing, docs and release state.
//! The `Store` handle and its connection live in `repository/src/store/mod.rs`.

use super::*;

impl Store {
    /// Every published version as `(package_version_id, ident, version, hash)`,
    /// for the `backfill-metadata` sweep (plan-61-A Phase 4).
    ///
    /// This is the one read accessor A ships. It is not the sub-plan's "no read
    /// accessors" rule leaking: that rule is about the `package_metadata` /
    /// `package_targets` shapes plan-61-B was expected to consume and does not.
    /// Backfill is a *write* ceremony that has to know what to rewrite.
    pub fn all_package_versions(&self) -> Result<Vec<(i64, String, String, String)>, String> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT pv.id, p.ident, pv.version, pv.hash
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 ORDER BY pv.id",
            )
            .map_err(|err| format!("failed to enumerate package versions: {err}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|err| format!("failed to enumerate package versions: {err}"))?;
        let mut versions = Vec::new();
        for row in rows {
            versions.push(row.map_err(|err| format!("failed to read package version: {err}"))?);
        }
        Ok(versions)
    }

    /// Rewrite one version's captured metadata: replace its target rows and set
    /// its `author`/`url`, in a single transaction (plan-61-A Phase 4).
    ///
    /// Delete-then-insert is what makes the backfill idempotent — a second run
    /// must not double every target row. Scoped to one version so a failure
    /// mid-sweep leaves earlier versions committed and the rest untouched.
    pub fn replace_version_metadata(
        &self,
        package_version_id: i64,
        metadata: &PublishMetadata,
        vendor_blobs: &[VendorBlobRef],
    ) -> Result<(), String> {
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start backfill transaction: {err}"))?;
        tx.execute(
            "UPDATE package_versions SET author = ?2, url = ?3, description = ?4 \
             WHERE id = ?1",
            params![
                package_version_id,
                metadata.author,
                metadata.url,
                metadata.description
            ],
        )
        .map_err(|err| format!("failed to update version metadata: {err}"))?;
        tx.execute(
            "DELETE FROM package_version_targets WHERE package_version_id = ?1",
            params![package_version_id],
        )
        .map_err(|err| format!("failed to clear version targets: {err}"))?;
        insert_version_targets(&tx, package_version_id, vendor_blobs)?;
        tx.commit()
            .map_err(|err| format!("failed to commit backfill: {err}"))
    }

    /// The stored `(author, url)` for one published version. Test-only, for the
    /// same reason as `target_rows_for_test`: the publish-path tests live in
    /// `server.rs` and plan-61-A ships no real read accessors.
    #[cfg(test)]
    pub(crate) fn version_metadata_for_test(
        &self,
        ident: &str,
        version: &str,
    ) -> (Option<String>, Option<String>) {
        self.conn()
            .query_row(
                "SELECT pv.author, pv.url FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.version = ?2",
                params![ident, version],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    /// The stored `description` for one published version. Test-only.
    #[cfg(test)]
    pub(crate) fn version_description_for_test(
        &self,
        ident: &str,
        version: &str,
    ) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT pv.description FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.version = ?2",
                params![ident, version],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// Every `package_version_targets` row as
    /// `(os, arch, libc, lib_type, source)`, ordered by insertion.
    ///
    /// Test-only. plan-61-A deliberately ships **no** read accessors — plan-61-B
    /// defines its own `package_detail`/`package_audit` queries over these
    /// tables — but the publish-path tests in `server.rs` live outside this
    /// module and cannot reach the private connection.
    #[cfg(test)]
    pub(crate) fn target_rows_for_test(&self) -> Vec<TargetRowForTest> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT os, arch, libc, lib_type, source
                 FROM package_version_targets ORDER BY rowid",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        rows
    }

    /// The published versions of a package, oldest first (plan-10-A `/index`).
    /// Each row carries the version, content hash, publish time, and current
    /// release state; the transparency-log entry is resolved separately.
    pub fn list_package_versions(&self, ident: &str) -> Result<Vec<PackageVersionRow>, String> {
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT pv.version, pv.hash, pv.created_at, pv.state, pv.abi_index,
                        publisher.owner_display, publisher_key.fingerprint, pv.transfer_epoch
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 LEFT JOIN owners publisher ON publisher.id = pv.publisher_owner_id
                 LEFT JOIN keys publisher_key ON publisher_key.id = pv.publisher_ident_key_id
                 WHERE p.ident = ?1
                 ORDER BY pv.created_at ASC, pv.id ASC",
            )
            .map_err(|err| format!("failed to prepare version query: {err}"))?;
        let rows = statement
            .query_map(params![ident], |row| {
                Ok(PackageVersionRow {
                    version: row.get(0)?,
                    hash: row.get(1)?,
                    published_at: row.get(2)?,
                    state: row.get(3)?,
                    abi_index: row.get(4)?,
                    publisher_display: row.get(5)?,
                    publisher_ident_fingerprint: row.get(6)?,
                    transfer_epoch: row.get(7)?,
                })
            })
            .map_err(|err| format!("failed to list package versions: {err}"))?;
        let mut versions = Vec::new();
        for row in rows {
            versions.push(row.map_err(|err| format!("failed to read package version: {err}"))?);
        }
        Ok(versions)
    }

    /// The package's headline "latest" release: newest by `published_at` among
    /// the versions whose state is **active** (plan-126-A).
    ///
    /// Returns `(version, state)`, or `None` when the package has no active
    /// release at all — every published version yanked, blocked or
    /// legal-tombstoned — which is a statement to render, not missing data.
    ///
    /// This is the **only** server-side "latest version" selection. Before it
    /// existed, both the package page and the search results took
    /// `ORDER BY created_at DESC LIMIT 1` with no state predicate, so a package
    /// whose newest release was yanked advertised that release as its headline
    /// version — on the search page with no state badge at all.
    ///
    /// Two properties are load-bearing:
    ///
    /// * **The predicate is an allowlist**, `state IN ('available','deprecated')`,
    ///   matching [`crate::validation::state_is_active`] and through it the
    ///   install client's `state_is_floating_eligible`. Written as
    ///   `state != 'yanked'` it would admit `blocked` and `legal-tombstoned`.
    /// * **Ordering is by publish time**, not semver — `max_by_key(published_at)`
    ///   is how the install client selects, and diverging here would put the two
    ///   surfaces in disagreement about the same package.
    ///
    /// This filters the *selection* only. [`Self::package_detail`] still returns
    /// every version unfiltered; the transparency listing is never narrowed.
    pub fn latest_active_version(&self, ident: &str) -> Result<Option<(String, String)>, String> {
        let conn = self.conn();
        conn.query_row(
            // The two state literals are the SQL spelling of
            // `validation::state_is_active`. Keep them in step: a state added to
            // the active set there must be added here, and the
            // `active_states_are_an_allowlist_matching_the_install_client` test
            // plus this module's yanked/blocked/tombstoned tests are what catch
            // a one-sided edit.
            "SELECT pv.version, pv.state
             FROM package_versions pv
             JOIN packages p ON p.id = pv.package_id
             WHERE p.ident = ?1 AND pv.state IN ('available', 'deprecated')
             ORDER BY pv.created_at DESC, pv.id DESC
             LIMIT 1",
            params![ident],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|err| format!("failed to load latest active version: {err}"))
    }

    /// Record the raw MFPC section-17 bytes for one version (plan-126-E).
    ///
    /// **Raw bytes, not a decoded structure.** `mfb_wire::docs` is the single
    /// decoder; storing decoded rows would create a second representation that
    /// must migrate whenever the wire format is extended. A `BLOB` costs one
    /// decode per page render, on data already in the local database.
    ///
    /// `INSERT OR IGNORE`: a version's documentation is fixed by its signed
    /// payload, so an existing row is never rewritten. That is what keeps the
    /// backfill sweep idempotent -- a second pass over the same blob changes
    /// nothing.
    ///
    /// Returns whether a row was **actually inserted**: `false` when one already
    /// existed. The backfill sweep counts only real inserts, so a second run
    /// reports zero rather than re-claiming the rows the first run wrote.
    pub fn put_version_docs(
        &self,
        package_version_id: i64,
        section: &[u8],
    ) -> Result<bool, String> {
        let conn = self.conn();
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO package_version_docs (package_version_id, doc_section)
                 VALUES (?1, ?2)",
                params![package_version_id, section],
            )
            .map_err(|err| format!("failed to record version documentation: {err}"))?;
        Ok(inserted == 1)
    }

    /// The doc section of a package's **latest active** release (plan-126-E), as
    /// `(version, raw section-17 bytes)`.
    ///
    /// Built on [`Self::latest_active_version`], deliberately -- not on its own
    /// `ORDER BY created_at LIMIT 1`. That is what makes a yanked newest release
    /// fall back to the older active one's documentation, and it keeps one
    /// definition of "latest" in the registry instead of two predicates that can
    /// drift (plan-126-A).
    ///
    /// `None` when the package has no active release, **or** when its latest
    /// active release carries no documentation. It never falls back to an older
    /// release's docs: the Docs tab's contract is "the current version's
    /// documentation", and showing an older version's would contradict it
    /// (plan-126-F Open Decision).
    ///
    /// `latest_active_version` is called before this function takes the
    /// connection lock. The lock is not re-entrant, so holding it across that
    /// call would deadlock.
    pub fn latest_active_version_docs(
        &self,
        ident: &str,
    ) -> Result<Option<(String, Vec<u8>)>, String> {
        let Some((version, _state)) = self.latest_active_version(ident)? else {
            return Ok(None);
        };
        let conn = self.conn();
        let section: Option<Vec<u8>> = conn
            .query_row(
                "SELECT d.doc_section
                 FROM package_version_docs d
                 JOIN package_versions pv ON pv.id = d.package_version_id
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.version = ?2",
                params![ident, version],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load version documentation: {err}"))?;
        Ok(section.map(|section| (version, section)))
    }

    pub fn package_version_exists(&self, ident: &str, version: &str) -> Result<bool, String> {
        let conn = self.conn();
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.version = ?2",
                params![ident, version],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to check package version: {err}"))?;
        Ok(exists.is_some())
    }

    /// Total `package_versions` rows across every package this owner owns — the
    /// quantity bounded by the per-owner publish quota (bug-188 / REPO-13).
    pub fn owner_version_count(&self, owner_id: i64) -> Result<i64, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT COUNT(*) FROM package_versions v
             JOIN packages p ON v.package_id = p.id
             WHERE p.owner_id = ?1",
            params![owner_id],
            |row| row.get(0),
        )
        .map_err(|err| format!("failed to count owner versions: {err}"))
    }

    pub fn publish_package_version(
        &self,
        owner_id: i64,
        ident: &str,
        version: &str,
        hash: &str,
        blob_path: &str,
        abi_index: &str,
        vendor_blobs: &[VendorBlobRef],
        metadata: &PublishMetadata,
    ) -> Result<PublishedVersion, String> {
        // REPO-17: validate the ident's package component and the version against
        // an explicit safe charset/length before either reaches the log payload,
        // the `/index` route, or the REPO-14 log-lookup pattern. The owner half is
        // also re-checked here (validate_ident), the authoritative publish choke
        // point that every publish path funnels through.
        validate_ident(ident)?;
        validate_version(version)?;
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start publish transaction: {err}"))?;
        tx.execute(
            "INSERT OR IGNORE INTO packages (ident, owner_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![ident, owner_id, now],
        )
        .map_err(|err| format!("failed to create package identity: {err}"))?;
        let package_id: i64 = tx
            .query_row(
                "SELECT id FROM packages WHERE ident = ?1 AND owner_id = ?2",
                params![ident, owner_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load package identity: {err}"))?
            .ok_or_else(|| "package identity is owned by another owner".to_string())?;
        tx.execute(
            "INSERT OR IGNORE INTO package_blobs (hash, path, kind, created_at)
             VALUES (?1, ?2, 'package', ?3)",
            params![hash, blob_path, now],
        )
        .map_err(|err| format!("failed to store package blob metadata: {err}"))?;
        // plan-168-A §4.3: record who published the release, under which of
        // their ident keys, and in which ownership epoch. The key is looked up
        // among the publisher's ident keys of any status: a rotation between
        // `/validate` and this write retires the key but does not unmake the
        // signature the header carries.
        let publisher_ident_key_id: Option<i64> = match &metadata.publisher_ident_fingerprint {
            Some(fingerprint) => Some(
                tx.query_row(
                    "SELECT id FROM keys WHERE owner_id = ?1 AND role = 'ident' AND fingerprint = ?2",
                    params![owner_id, fingerprint],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|err| format!("failed to load publisher ident key: {err}"))?
                .ok_or_else(|| {
                    format!("publisher ident key {fingerprint} is not one of the publisher's keys")
                })?,
            ),
            None => None,
        };
        let transfer_epoch: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM transfer_history WHERE package_id = ?1",
                params![package_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("failed to count transfer history: {err}"))?;
        tx.execute(
            "INSERT INTO package_versions
                 (package_id, version, hash, state, abi_index, author, url, description,
                  created_at, publisher_owner_id, publisher_ident_key_id, transfer_epoch)
             VALUES (?1, ?2, ?3, 'available', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                package_id,
                version,
                hash,
                abi_index,
                metadata.author,
                metadata.url,
                metadata.description,
                now,
                owner_id,
                publisher_ident_key_id,
                transfer_epoch
            ],
        )
        .map_err(|err| {
            if is_unique_violation(&err) {
                format!("package version {ident}@{version} is already published")
            } else {
                format!("failed to publish package version: {err}")
            }
        })?;
        // plan-48-A §4.5: record the version→native-blob edges so a future GC
        // (plan-49) has the reachability data. The vendor blobs were already
        // uploaded via PUT /blob and their `package_blobs` rows exist; nothing
        // reads these edges in this plan.
        let package_version_id = tx.last_insert_rowid();
        // plan-126-E: the doc row is written in the SAME transaction as the version
        // row. A crash between two separate writes would leave a version whose docs
        // never appear -- and no backfill run could tell that apart from a package
        // that was simply never documented.
        if let Some(section) = &metadata.docs {
            tx.execute(
                "INSERT OR IGNORE INTO package_version_docs (package_version_id, doc_section)
                 VALUES (?1, ?2)",
                params![package_version_id, section],
            )
            .map_err(|err| format!("failed to record version documentation: {err}"))?;
        }
        for vendor in vendor_blobs {
            tx.execute(
                "INSERT OR IGNORE INTO package_version_blobs (package_version_id, hash)
                 VALUES (?1, ?2)",
                params![package_version_id, vendor.hash],
            )
            .map_err(|err| format!("failed to record version blob edge: {err}"))?;
        }
        // plan-61-A §3: the native target matrix, written **per locator**. The
        // `package_version_blobs` loop above collapses duplicate hashes on
        // purpose — a blob edge is about reachability, and probing one blob
        // twice is waste. Targets are the opposite: two locators sharing a hash
        // (two platforms shipping byte-identical builds under different
        // `source` names) are two supported platforms, and deduping them here
        // would silently under-report what the package runs on.
        insert_version_targets(&tx, package_version_id, vendor_blobs)?;
        let log_entry = append_log_tx(
            &tx,
            "publish",
            &format!(
                "{{\"ident\":{},\"version\":{},\"hash\":{}}}",
                json_value(ident),
                json_value(version),
                json_value(hash),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit publish: {err}"))?;
        Ok(PublishedVersion {
            ident: ident.to_string(),
            version: version.to_string(),
            hash: hash.to_string(),
            published_at: now,
            state: "available".to_string(),
            log_entry,
        })
    }

    /// Set a published version's release state (plan-10-C1). Updates the
    /// current state, records the transition with a timestamp, and appends one
    /// transparency-log entry — all in a single transaction. Ident-signature
    /// authorization is checked by the caller before this runs. Returns the
    /// publish/transition log entry reference.
    ///
    /// `owner_id` must be the package's **current** owner (`packages.owner_id`,
    /// which a transfer rewrites): the row is resolved under that predicate, so
    /// the store refuses a former owner even if a handler forgot to (bug-494),
    /// exactly as `publish_package_version` does.
    pub fn set_release_state(
        &self,
        owner_id: i64,
        ident: &str,
        version: &str,
        state: &str,
    ) -> Result<LogEntryRef, String> {
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start release-state transaction: {err}"))?;
        let package_version_id: Option<i64> = tx
            .query_row(
                "SELECT pv.id
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id
                 WHERE p.ident = ?1 AND pv.version = ?2 AND p.owner_id = ?3",
                params![ident, version, owner_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load package version: {err}"))?;
        let Some(package_version_id) = package_version_id else {
            return Err(format!(
                "package version {ident}@{version} is not published by this account"
            ));
        };
        tx.execute(
            "UPDATE package_versions SET state = ?1 WHERE id = ?2",
            params![state, package_version_id],
        )
        .map_err(|err| format!("failed to update release state: {err}"))?;
        tx.execute(
            "INSERT INTO release_state_changes (package_version_id, state, created_at)
             VALUES (?1, ?2, ?3)",
            params![package_version_id, state, now],
        )
        .map_err(|err| format!("failed to record release-state change: {err}"))?;
        let log_entry = append_log_tx(
            &tx,
            "release-state",
            &format!(
                "{{\"ident\":{},\"version\":{},\"state\":{}}}",
                json_value(ident),
                json_value(version),
                json_value(state),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit release-state change: {err}"))?;
        Ok(log_entry)
    }
}

/// One published version of a package (plan-10-A `/index`).
#[derive(Debug, Clone)]
pub struct PackageVersionRow {
    pub version: String,
    pub hash: String,
    pub published_at: i64,
    pub state: String,
    /// The per-symbol ABI index JSON string (plan-10-B1), `{}` when absent.
    pub abi_index: String,
    /// The publishing owner's display name (plan-168-A). `None` only for a
    /// pre-plan release whose prefix owner no longer resolves.
    pub publisher_display: Option<String>,
    /// The publisher's ident-key fingerprint, `None` for a pre-plan release.
    pub publisher_ident_fingerprint: Option<String>,
    /// The number of accepted transfers before this release was published.
    pub transfer_epoch: i64,
}

/// Write the `package_version_targets` rows for one version, one row per
/// section-10 locator (plan-61-A §3).
///
/// Takes a `&Connection` so it serves both the publish transaction and the
/// backfill's per-version transaction. It only inserts — the backfill clears
/// the version's existing rows first, which is what makes a re-run idempotent
/// rather than duplicating every target.
fn insert_version_targets(
    conn: &Connection,
    package_version_id: i64,
    vendor_blobs: &[VendorBlobRef],
) -> Result<(), String> {
    for vendor in vendor_blobs {
        conn.execute(
            "INSERT INTO package_version_targets
                 (package_version_id, blob_hash, os, arch, libc, lib_type, logical, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                package_version_id,
                vendor.hash,
                vendor.os,
                vendor.arch,
                vendor.libc,
                vendor.lib_type,
                vendor.logical,
                vendor.source,
            ],
        )
        .map_err(|err| format!("failed to record version target: {err}"))?;
    }
    Ok(())
}
