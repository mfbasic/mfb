//! plan-176-L: the registry store's transparency log and registry root — the root ceremony, checkpoints
//! and log entries.
//! The `Store` handle and its connection live in `repository/src/store/mod.rs`.

use super::*;

impl Store {
    /// Operator root ceremony (plan-10-C2): generate the offline root key and
    /// the online snapshot/timestamp keys, sign a `root.json` that delegates
    /// the server (attestation), snapshot, and timestamp keys, and persist
    /// everything **except the root private key**, which is returned for the
    /// operator to store offline. The root private key never touches the
    /// serving host's database.
    ///
    /// **One-time** (bug-584): this refuses to run against a configured
    /// registry. It used to mint a fresh offline root key and overwrite the
    /// sole config row on every run, which silently replaced the anchor a
    /// pinned client verifies against — a change indistinguishable, from the
    /// client's side, from registry takeover. Renewing the delegated online
    /// keys and the root expiry is `renew_registry_root`, authenticated by the
    /// offline root key and keeping the anchor; deliberately replacing a lost
    /// anchor is `reanchor_registry_root`.
    pub fn init_registry_root(
        &self,
        registry_id: &str,
        expires_at: i64,
    ) -> Result<Vec<u8>, String> {
        let (root_public, root_private) = crypto::generate_keypair();
        self.write_registry_root(
            registry_id,
            expires_at,
            &root_public,
            &root_private,
            RootCeremony::Init,
        )?;
        Ok(root_private)
    }

    /// Root renewal (bug-584): re-sign `root.json` with a bumped version, a
    /// fresh expiry, and freshly generated online snapshot/timestamp keys —
    /// under the **same** offline root key, which the operator supplies from
    /// offline storage and which is never persisted here.
    ///
    /// This is the authenticated old-to-new transition the pinned anchor
    /// needs: possession of the offline root key is the authentication, the
    /// anchor (`root_public`, hence its fingerprint) is unchanged, so a client
    /// that pinned the registry before the renewal verifies the renewed chain
    /// with no out-of-band step. A key that is not the configured root — and a
    /// registry id that is not the configured one — is refused without
    /// touching any stored field. Returns the new root version.
    pub fn renew_registry_root(
        &self,
        registry_id: &str,
        expires_at: i64,
        root_private: &[u8],
    ) -> Result<i64, String> {
        let root_public = crypto::public_from_private(root_private)
            .map_err(|_| "offline root key is malformed".to_string())?;
        self.write_registry_root(
            registry_id,
            expires_at,
            &root_public,
            root_private,
            RootCeremony::Renew,
        )
    }

    /// Deliberate root re-anchor (bug-584): mint a **new** offline root key and
    /// replace the anchor, for the one case renewal cannot cover — the offline
    /// root key is lost. Mirrors the ident `reanchor` ceremony (plan-23 §3.6):
    /// every client that pinned the old fingerprint fails hard until it
    /// re-pins the new one out of band, which is why it is a separate,
    /// explicitly selected, transparency-logged mode and never something a
    /// repeated `init-root` does by accident. Returns the new root private key
    /// for the operator to store offline.
    pub fn reanchor_registry_root(
        &self,
        registry_id: &str,
        expires_at: i64,
    ) -> Result<Vec<u8>, String> {
        let (root_public, root_private) = crypto::generate_keypair();
        self.write_registry_root(
            registry_id,
            expires_at,
            &root_public,
            &root_private,
            RootCeremony::Reanchor,
        )?;
        Ok(root_private)
    }

    /// The one writer of `registry_config`'s root columns. `ceremony` decides
    /// what it may do to an already-configured root — the whole security
    /// question of bug-584 — and every mode appends a transparency-log entry so
    /// a root change is auditable after the fact.
    fn write_registry_root(
        &self,
        registry_id: &str,
        expires_at: i64,
        root_public: &[u8],
        root_private: &[u8],
        ceremony: RootCeremony,
    ) -> Result<i64, String> {
        if registry_id.is_empty() || registry_id.len() > 255 {
            return Err("registry id must be 1..=255 bytes".to_string());
        }
        let (server_public, _server_private) = self.server_keypair()?;
        let (snapshot_public, snapshot_private) = crypto::generate_keypair();
        let (timestamp_public, timestamp_private) = crypto::generate_keypair();
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start root ceremony transaction: {err}"))?;
        let existing: Option<(i64, String, Vec<u8>)> = tx
            .query_row(
                "SELECT root_version, registry_id, root_public FROM registry_config WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|err| format!("failed to read registry config: {err}"))?;
        let previous_version = match (ceremony, &existing) {
            (RootCeremony::Init, Some(_)) => {
                return Err(
                    "registry root of trust is already initialized; renew it with `mfb-repo \
                     renew-root` (same offline root key, pinned clients keep verifying) or, only \
                     if the offline root key is lost, re-anchor it with `mfb-repo reanchor-root` \
                     (every pinned client must re-pin out of band)"
                        .to_string(),
                )
            }
            (RootCeremony::Init, None) => 0,
            (RootCeremony::Renew | RootCeremony::Reanchor, None) => {
                return Err(
                    "registry root of trust is not initialized; run `mfb-repo init-root` first"
                        .to_string(),
                )
            }
            (RootCeremony::Renew, Some((version, existing_id, existing_public))) => {
                if existing_public.as_slice() != root_public {
                    return Err(
                        "the supplied key is not this registry's offline root key; refusing to \
                         replace the pinned anchor"
                            .to_string(),
                    );
                }
                if existing_id != registry_id {
                    return Err(format!(
                        "registry id `{registry_id}` does not match the configured registry id \
                         `{existing_id}`"
                    ));
                }
                *version
            }
            (RootCeremony::Reanchor, Some((version, _, _))) => *version,
        };
        let version = previous_version + 1;
        let root_json = format!(
            "{{\"type\":\"root\",\"registryId\":{},\"version\":{},\"expires\":{},\"serverKey\":{},\"snapshotKey\":{},\"timestampKey\":{}}}",
            json_value(registry_id),
            version,
            expires_at,
            json_value(&crypto::encode_bytes(&server_public)),
            json_value(&crypto::encode_bytes(&snapshot_public)),
            json_value(&crypto::encode_bytes(&timestamp_public)),
        );
        let root_signature = crypto::sign(
            root_private,
            &crypto::root_signing_input(root_json.as_bytes()),
        )?;
        tx.execute(
            "INSERT INTO registry_config
               (id, registry_id, root_version, root_public, root_json, root_signature,
                snapshot_public, snapshot_private, timestamp_public, timestamp_private, created_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
               registry_id = excluded.registry_id,
               root_version = excluded.root_version,
               root_public = excluded.root_public,
               root_json = excluded.root_json,
               root_signature = excluded.root_signature,
               snapshot_public = excluded.snapshot_public,
               snapshot_private = excluded.snapshot_private,
               timestamp_public = excluded.timestamp_public,
               timestamp_private = excluded.timestamp_private",
            params![
                registry_id,
                version,
                root_public,
                root_json,
                root_signature,
                snapshot_public,
                snapshot_private,
                timestamp_public,
                timestamp_private,
                now,
            ],
        )
        .map_err(|err| format!("failed to store registry root: {err}"))?;
        append_log_tx(
            &tx,
            ceremony.log_kind(),
            &format!(
                "{{\"registryId\":{},\"version\":{},\"rootFingerprint\":{}}}",
                json_value(registry_id),
                version,
                json_value(&crypto::fingerprint(root_public)),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit root ceremony: {err}"))?;
        Ok(version)
    }

    /// The signed `root.json` and its delegated online keypairs, if the root
    /// ceremony has been run.
    pub fn registry_config(&self) -> Result<Option<RegistryConfig>, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT registry_id, root_public, root_json, root_signature,
                    snapshot_public, snapshot_private, timestamp_public, timestamp_private
             FROM registry_config WHERE id = 1",
            [],
            |row| {
                Ok(RegistryConfig {
                    registry_id: row.get(0)?,
                    root_public: row.get(1)?,
                    root_json: row.get(2)?,
                    root_signature: row.get(3)?,
                    snapshot_public: row.get(4)?,
                    snapshot_private: row.get(5)?,
                    timestamp_public: row.get(6)?,
                    timestamp_private: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("failed to load registry config: {err}"))
    }

    /// A canonical hash of the whole served index — every `(ident, version,
    /// hash, state)` tuple, sorted — so a snapshot can attest to the exact
    /// index state and a mirror serving a stale or partial index is detected.
    pub fn index_canonical_hash(&self) -> Result<String, String> {
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT p.ident, pv.version, pv.hash, pv.state
                 FROM package_versions pv
                 JOIN packages p ON p.id = pv.package_id",
            )
            .map_err(|err| format!("failed to prepare index query: {err}"))?;
        let rows = statement
            .query_map([], |row| {
                let ident: String = row.get(0)?;
                let version: String = row.get(1)?;
                let hash: String = row.get(2)?;
                let state: String = row.get(3)?;
                Ok(format!("{ident}\u{0}{version}\u{0}{hash}\u{0}{state}\n"))
            })
            .map_err(|err| format!("failed to read index: {err}"))?;
        let mut lines = Vec::new();
        for row in rows {
            lines.push(row.map_err(|err| format!("failed to read index row: {err}"))?);
        }
        lines.sort();
        let mut bytes = Vec::new();
        for line in lines {
            bytes.extend_from_slice(line.as_bytes());
        }
        Ok(hex::encode(crypto::sha256(&bytes)))
    }

    /// The number of transparency-log entries (the tree size).
    pub fn log_size(&self) -> Result<i64, String> {
        let conn = self.conn();
        conn.query_row("SELECT COUNT(*) FROM log_entries", [], |row| row.get(0))
            .map_err(|err| format!("failed to size the log: {err}"))
    }

    /// The current signed tree head: `(size, root, signature)`, memoised by
    /// log size (bug-579).
    ///
    /// `/log/checkpoint`, `/packages/:ident/audit` and `/snapshot.json` are all
    /// anonymous and each rebuilt this independently: every request loaded every
    /// leaf, recomputed the whole Merkle root and produced a fresh signature. The
    /// cost grew with the log and nothing bounded the repetition, so public
    /// traffic could pin the CPU and serialize unrelated work behind `Store`'s
    /// single connection mutex.
    ///
    /// The memo is deliberately NOT inside `log_leaf_hashes`. That reader has a
    /// documented contract about surfacing a malformed leaf as an error, and a
    /// test corrupts leaves in place — leaving the row COUNT unchanged — to prove
    /// it. A cache keyed on size inside that function would serve the pre-corruption
    /// leaves and silently defeat the check. Here the key is sound because
    /// production only ever appends.
    ///
    /// The signature is recomputed only when the size moves, so a cache hit is
    /// also one fewer Ed25519 signing operation.
    pub fn signed_checkpoint(&self) -> Result<(i64, [u8; 32], Vec<u8>), String> {
        let size = self.log_size()?;
        {
            let cached = self
                .head_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(head) = cached.as_ref() {
                if head.size == size {
                    return Ok((head.size, head.root, head.signature.clone()));
                }
            }
        }
        let leaves = self.log_leaf_hashes(None)?;
        // Re-read rather than trusting the count taken above: an append between
        // the two reads would otherwise cache a root under the wrong size.
        let size = leaves.len() as i64;
        let root = crate::log::root(&leaves);
        let (_public, private) = self.server_keypair()?;
        let signature = crypto::sign(
            &private,
            &crate::log::checkpoint_signing_input(size as u64, &root),
        )?;
        let mut cached = self
            .head_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *cached = Some(CachedLogHead {
            size,
            root,
            signature: signature.clone(),
        });
        Ok((size, root, signature))
    }

    /// The ordered leaf hashes of the first `size` log entries (the whole
    /// log when `size` is None).
    pub fn log_leaf_hashes(&self, size: Option<i64>) -> Result<Vec<[u8; 32]>, String> {
        // A negative size silently selected zero leaves (`WHERE idx < -1`), and
        // `log_consistency_proof` would then compute `to = 0` and hand back a
        // structurally "valid" empty-range proof for a log that is not empty
        // (bug-276 R6). The callers take an unvalidated `Option<i64>` straight off
        // a query string, so a rewriting proxy could feed a client that view and
        // weaken an integrity signal it is trusting. Reject rather than clamp: a
        // negative size is never a meaningful request.
        if let Some(size) = size {
            if size < 0 {
                return Err("log size must not be negative".to_string());
            }
        }
        let conn = self.conn();
        let limit = size.unwrap_or(i64::MAX);
        let mut statement = conn
            .prepare("SELECT leaf_hash FROM log_entries WHERE idx < ?1 ORDER BY idx ASC")
            .map_err(|err| format!("failed to prepare log query: {err}"))?;
        let rows = statement
            .query_map(params![limit], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|err| format!("failed to load log leaves: {err}"))?;
        let mut leaves = Vec::new();
        for row in rows {
            let raw = row.map_err(|err| format!("failed to read log leaf: {err}"))?;
            let mut leaf = [0u8; 32];
            if raw.len() != 32 {
                return Err("malformed log leaf hash".to_string());
            }
            leaf.copy_from_slice(&raw);
            leaves.push(leaf);
        }
        Ok(leaves)
    }

    /// Look up the publish log entry for `ident@version`.
    pub fn publish_log_entry(
        &self,
        ident: &str,
        version: &str,
    ) -> Result<Option<LogEntryRef>, String> {
        let payload_ident = json_value(ident);
        let payload_version = json_value(version);
        let conn = self.conn();
        // publish payloads are canonical (`{"ident":...,"version":...,"hash":...}`),
        // so a prefix match on the two identity fields is exact.
        let prefix = format!("{{\"ident\":{payload_ident},\"version\":{payload_version},");
        // REPO-14: `_` and `%` in the ident/version are SQL `LIKE` wildcards. Owner
        // names admit `_` and package/version are otherwise unconstrained, so an
        // un-escaped prefix could match a *different* package's log entry —
        // corrupting the inclusion-proof mapping a client verifies. Escape the
        // metacharacters (`\` first, then `%`/`_`) and match with an explicit
        // `ESCAPE`, appending the single intended trailing wildcard.
        let escaped = prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let pattern = format!("{escaped}%");
        conn.query_row(
            "SELECT idx, leaf_hash FROM log_entries
             WHERE kind = 'publish' AND payload LIKE ?1 ESCAPE '\\'
             ORDER BY idx ASC LIMIT 1",
            params![pattern],
            |row| {
                let index: i64 = row.get(0)?;
                let raw: Vec<u8> = row.get(1)?;
                Ok((index, raw))
            },
        )
        .optional()
        .map_err(|err| format!("failed to load publish log entry: {err}"))?
        .map(|(index, raw)| {
            let mut leaf_hash = [0u8; 32];
            if raw.len() != 32 {
                return Err("malformed log leaf hash".to_string());
            }
            leaf_hash.copy_from_slice(&raw);
            Ok(LogEntryRef { index, leaf_hash })
        })
        .transpose()
    }
}

/// A reference to one transparency-log entry (plan-23-B3), returned by every
/// state-changing operation and surfaced on the wire as `logEntry`.
#[derive(Debug, Clone)]
pub struct LogEntryRef {
    pub index: i64,
    pub leaf_hash: [u8; 32],
}

/// Which root ceremony `Store::write_registry_root` is performing. The three
/// differ only in what they may do to an already-configured root, which is
/// exactly the trust-continuity question bug-584 settles.
#[derive(Clone, Copy)]
enum RootCeremony {
    /// First-time initialization; refuses if a root is already configured.
    Init,
    /// Renewal under the same offline root key; the anchor is unchanged.
    Renew,
    /// Deliberate replacement of a lost anchor; pinned clients must re-pin.
    Reanchor,
}

impl RootCeremony {
    /// The transparency-log kind recorded for this ceremony.
    fn log_kind(self) -> &'static str {
        match self {
            RootCeremony::Init => "root-init",
            RootCeremony::Renew => "root-renew",
            RootCeremony::Reanchor => "root-reanchor",
        }
    }
}

/// The signed-metadata root of trust (plan-10-C2): the root-signed `root.json`
/// plus the online snapshot/timestamp keypairs the server signs metadata with.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub registry_id: String,
    pub root_public: Vec<u8>,
    pub root_json: String,
    pub root_signature: Vec<u8>,
    pub snapshot_public: Vec<u8>,
    pub snapshot_private: Vec<u8>,
    pub timestamp_public: Vec<u8>,
    pub timestamp_private: Vec<u8>,
}

/// Append one entry to the transparency log inside an existing transaction.
/// The index is dense and monotonic; the leaf hash is the RFC 6962 leaf hash
/// of the payload bytes.
pub(super) fn append_log_tx(
    tx: &rusqlite::Transaction<'_>,
    kind: &str,
    payload: &str,
) -> Result<LogEntryRef, String> {
    let index: i64 = tx
        .query_row("SELECT COUNT(*) FROM log_entries", [], |row| row.get(0))
        .map_err(|err| format!("failed to size the log: {err}"))?;
    let leaf_hash = crate::log::leaf_hash(payload.as_bytes());
    tx.execute(
        "INSERT INTO log_entries (idx, kind, payload, leaf_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![index, kind, payload, leaf_hash.to_vec(), now_unix()],
    )
    .map_err(|err| format!("failed to append log entry: {err}"))?;
    Ok(LogEntryRef { index, leaf_hash })
}
