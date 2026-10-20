//! plan-176-L: the registry store's accounts — owners, keys, challenges, pairing, rotation, revocation
//! and sessions.
//! The `Store` handle and its connection live in `repository/src/store/mod.rs`.

use super::*;

impl Store {
    /// Register an owner with the two client-held public keys (plan-23 §3.1):
    /// a per-machine `auth` key and the account `ident` key. Each key must
    /// carry a proof-of-possession signed over the role-discriminated
    /// registration message, so an auth proof can never be replayed as an
    /// ident proof (or vice versa). The server never sees a private key.
    pub fn register_owner(
        &self,
        owner: &str,
        auth_key: &[u8],
        auth_proof: &[u8],
        ident_key: &[u8],
        ident_proof: &[u8],
    ) -> Result<(OwnerRecord, KeyRecord, KeyRecord), String> {
        validate_owner_name(owner)?;
        let auth_message = crypto::registration_message(crypto::ROLE_AUTH, owner, auth_key);
        crypto::verify(auth_key, &auth_message, auth_proof)
            .map_err(|_| "invalid auth proof-of-possession signature".to_string())?;
        let ident_message = crypto::registration_message(crypto::ROLE_IDENT, owner, ident_key);
        crypto::verify(ident_key, &ident_message, ident_proof)
            .map_err(|_| "invalid ident proof-of-possession signature".to_string())?;

        let folded = fold_owner(owner);
        let auth_fingerprint = crypto::fingerprint(auth_key);
        let ident_fingerprint = crypto::fingerprint(ident_key);
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start registration transaction: {err}"))?;
        tx.execute(
            "INSERT INTO owners (owner_display, owner_folded, created_at, status)
             VALUES (?1, ?2, ?3, 'active')",
            params![owner, folded, now],
        )
        .map_err(|err| {
            if is_unique_violation(&err) {
                format!("owner name '{owner}' is already in use")
            } else {
                format!("failed to register owner: {err}")
            }
        })?;
        let owner_id = tx.last_insert_rowid();
        let auth_key_id = insert_key_tx(
            &tx,
            owner_id,
            KEY_ROLE_AUTH,
            auth_key,
            &auth_fingerprint,
            now,
            "failed to register auth key",
        )?;
        let ident_key_id = insert_key_tx(
            &tx,
            owner_id,
            KEY_ROLE_IDENT,
            ident_key,
            &ident_fingerprint,
            now,
            "failed to register ident key",
        )?;
        append_log_tx(
            &tx,
            "register",
            &format!(
                "{{\"owner\":{},\"authFingerprint\":{},\"identFingerprint\":{}}}",
                json_value(owner),
                json_value(&auth_fingerprint),
                json_value(&ident_fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit registration: {err}"))?;

        Ok((
            OwnerRecord {
                id: owner_id,
                owner_display: owner.to_string(),
            },
            KeyRecord {
                id: auth_key_id,
                public_key: auth_key.to_vec(),
                fingerprint: auth_fingerprint,
            },
            KeyRecord {
                id: ident_key_id,
                public_key: ident_key.to_vec(),
                fingerprint: ident_fingerprint,
            },
        ))
    }

    pub fn owner_with_auth_key(
        &self,
        owner: &str,
    ) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        self.owner_with_key(owner, "auth")
    }

    pub fn owner_with_ident_key(
        &self,
        owner: &str,
    ) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        self.owner_with_key(owner, "ident")
    }

    /// Look up one of the owner's current auth keys by fingerprint. Machines
    /// are equals (plan-23 §2): an account holds one current auth key per
    /// linked machine, so auth-key resolution is always fingerprint-scoped.
    pub fn owner_auth_key_by_fingerprint(
        &self,
        owner: &str,
        fingerprint: &str,
    ) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn();
        conn.query_row(
            "SELECT o.id, o.owner_display, k.id, k.public_key, k.fingerprint
             FROM owners o
             JOIN keys k ON k.owner_id = o.id
             WHERE o.owner_folded = ?1
               AND o.status = 'active'
               AND k.role = 'auth'
               AND k.status = 'current'
               AND k.fingerprint = ?2",
            params![folded, fingerprint],
            |row| {
                Ok((
                    OwnerRecord {
                        id: row.get(0)?,
                        owner_display: row.get(1)?,
                    },
                    KeyRecord {
                        id: row.get(2)?,
                        public_key: row.get(3)?,
                        fingerprint: row.get(4)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(|err| format!("failed to load owner: {err}"))
    }

    fn owner_with_key(
        &self,
        owner: &str,
        role: &str,
    ) -> Result<Option<(OwnerRecord, KeyRecord)>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn();
        conn.query_row(
            "SELECT o.id, o.owner_display, k.id, k.public_key, k.fingerprint
             FROM owners o
             JOIN keys k ON k.owner_id = o.id
             WHERE o.owner_folded = ?1
               AND o.status = 'active'
               AND k.role = ?2
               AND k.status = 'current'",
            params![folded, role],
            |row| {
                Ok((
                    OwnerRecord {
                        id: row.get(0)?,
                        owner_display: row.get(1)?,
                    },
                    KeyRecord {
                        id: row.get(2)?,
                        public_key: row.get(3)?,
                        fingerprint: row.get(4)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(|err| format!("failed to load owner: {err}"))
    }

    /// Record an attestation issuance (plan-23 §3.3 step 2): every `/signing`
    /// request is logged before the server signs, so a stolen auth session
    /// requesting attestations always leaves a trace.
    pub fn record_signing_request(
        &self,
        owner_id: i64,
        ident: &str,
        version: &str,
        signing_fingerprint: &str,
    ) -> Result<(), String> {
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start signing transaction: {err}"))?;
        tx.execute(
            "INSERT INTO signing_requests (owner_id, ident, version, signing_fingerprint, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![owner_id, ident, version, signing_fingerprint, now_unix()],
        )
        .map_err(|err| format!("failed to record signing request: {err}"))?;
        let owner_display: String = tx
            .query_row(
                "SELECT owner_display FROM owners WHERE id = ?1",
                params![owner_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("failed to load owner: {err}"))?;
        append_log_tx(
            &tx,
            "attestation",
            &format!(
                "{{\"owner\":{},\"ident\":{},\"version\":{},\"signingFingerprint\":{}}}",
                json_value(&owner_display),
                json_value(ident),
                json_value(version),
                json_value(signing_fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit signing request: {err}"))?;
        Ok(())
    }

    /// Legacy single-machine helper kept for tests: challenges the owner's
    /// (sole) current auth key.
    pub fn create_challenge(&self, owner: &str) -> Result<ChallengeRecord, String> {
        validate_owner_name(owner)?;
        let Some((owner, key)) = self.owner_with_auth_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        self.create_challenge_for_key(owner.id, key.id, CHALLENGE_PURPOSE_LOGIN)
    }

    /// Challenge a specific machine's auth key (plan-23-B: an account holds
    /// one current auth key per linked machine).
    pub fn create_auth_challenge(
        &self,
        owner: &str,
        fingerprint: &str,
    ) -> Result<ChallengeRecord, String> {
        validate_owner_name(owner)?;
        let Some((owner, key)) = self.owner_auth_key_by_fingerprint(owner, fingerprint)? else {
            return Err("mismatched local key fingerprint".to_string());
        };
        self.create_challenge_for_key(owner.id, key.id, CHALLENGE_PURPOSE_LOGIN)
    }

    /// Challenge the owner's ident key: proves possession of the account
    /// identity for ident-authorized operations (auth-key revocation). Bound
    /// to the `revoke` purpose, so it can never complete a login (bug-493).
    pub fn create_ident_challenge(&self, owner: &str) -> Result<ChallengeRecord, String> {
        validate_owner_name(owner)?;
        let Some((owner, key)) = self.owner_with_ident_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        self.create_challenge_for_key(owner.id, key.id, CHALLENGE_PURPOSE_REVOKE)
    }

    /// Mint a challenge row for `key_id`, bound to `purpose` (bug-493): the
    /// completer for one purpose refuses a challenge minted for the other.
    fn create_challenge_for_key(
        &self,
        owner_id: i64,
        key_id: i64,
        purpose: &str,
    ) -> Result<ChallengeRecord, String> {
        let id = Uuid::new_v4().to_string();
        let mut nonce = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut nonce);
        let created_at = now_unix();
        let expires_at = created_at + 300;
        let conn = self.conn();
        conn.execute(
            "INSERT INTO auth_challenges
             (id, owner_id, key_id, nonce, purpose, created_at, expires_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![id, owner_id, key_id, nonce, purpose, created_at, expires_at],
        )
        .map_err(|err| format!("failed to create auth challenge: {err}"))?;
        Ok(ChallengeRecord {
            id,
            owner_id,
            key_id,
            nonce,
            expires_at,
            used_at: None,
        })
    }

    /// Store a machine-link relay blob (plan-23 §3.2): a single-use,
    /// short-TTL ciphertext the server cannot read, keyed by the
    /// code-derived lookup. Returns the expiry time.
    pub fn store_pairing_blob(
        &self,
        owner_id: i64,
        lookup: &str,
        blob: &[u8],
        salt: &[u8],
        approval_key: &[u8],
    ) -> Result<i64, String> {
        let now = now_unix();
        let expires_at = now + 600;
        let conn = self.conn();
        // Housekeeping: expired blobs are dead weight; drop them here so no
        // background reaper is needed (full rate-limiting is plan-10-D).
        conn.execute(
            "DELETE FROM pairing_blobs WHERE expires_at <= ?1",
            params![now],
        )
        .map_err(|err| format!("failed to clear expired pairing blobs: {err}"))?;
        conn.execute(
            "INSERT INTO pairing_blobs
                 (owner_id, lookup, blob, salt, approval_key, created_at, expires_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![owner_id, lookup, blob, salt, approval_key, now, expires_at],
        )
        .map_err(|err| {
            if is_unique_violation(&err) {
                "a pairing with this code is already pending; generate a new code".to_string()
            } else {
                format!("failed to store pairing blob: {err}")
            }
        })?;
        Ok(expires_at)
    }

    /// Fetch-and-consume a pairing blob: single use, refused after expiry.
    /// The stored ciphertext is destroyed as it is handed out.
    ///
    /// bug-583: `approve` is handed the row's code-derived approval public key
    /// and decides, INSIDE the transaction, whether this caller proved
    /// possession of the pairing code. A refusal rolls the transaction back,
    /// so a caller holding only the (server-visible) lookup can neither take
    /// the blob nor burn the honest machine's pending pairing. Consumption is
    /// therefore atomic with authorization.
    pub fn take_pairing_blob<F>(
        &self,
        owner: &str,
        lookup: &str,
        approve: F,
    ) -> Result<PairingFetch, String>
    where
        F: FnOnce(&[u8]) -> bool,
    {
        let folded = fold_owner(owner);
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start pairing transaction: {err}"))?;
        let row: Option<(i64, Vec<u8>, Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT p.id, p.blob, p.salt, p.approval_key
                 FROM pairing_blobs p
                 JOIN owners o ON o.id = p.owner_id
                 WHERE p.lookup = ?1
                   AND o.owner_folded = ?2
                   AND o.status = 'active'
                   AND p.used_at IS NULL
                   AND p.expires_at > ?3",
                params![lookup, folded, now],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|err| format!("failed to load pairing blob: {err}"))?;
        let Some((id, blob, salt, approval_key)) = row else {
            tx.commit().ok();
            return Ok(PairingFetch::Missing);
        };
        if !approve(&approval_key) {
            tx.rollback().ok();
            return Ok(PairingFetch::Unapproved);
        }
        tx.execute(
            "UPDATE pairing_blobs SET used_at = ?1, blob = x'' WHERE id = ?2",
            params![now, id],
        )
        .map_err(|err| format!("failed to consume pairing blob: {err}"))?;
        tx.commit()
            .map_err(|err| format!("failed to commit pairing fetch: {err}"))?;
        Ok(PairingFetch::Relayed { blob, salt })
    }

    /// Register an additional machine's auth key on an existing account
    /// (plan-23 §3.2 step 1). The proof must be role-separated exactly like
    /// registration.
    pub fn add_auth_key(
        &self,
        owner: &str,
        public_key: &[u8],
        proof: &[u8],
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        validate_owner_name(owner)?;
        let Some((owner, _ident)) = self.owner_with_ident_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        let message =
            crypto::registration_message(crypto::ROLE_AUTH, &owner.owner_display, public_key);
        crypto::verify(public_key, &message, proof)
            .map_err(|_| "invalid auth proof-of-possession signature".to_string())?;
        let fingerprint = crypto::fingerprint(public_key);
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start link transaction: {err}"))?;
        let key_id = insert_key_tx(
            &tx,
            owner.id,
            KEY_ROLE_AUTH,
            public_key,
            &fingerprint,
            now_unix(),
            "failed to register machine auth key",
        )?;
        append_log_tx(
            &tx,
            "link",
            &format!(
                "{{\"owner\":{},\"authFingerprint\":{}}}",
                json_value(&owner.owner_display),
                json_value(&fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit link: {err}"))?;
        Ok((
            owner,
            KeyRecord {
                id: key_id,
                public_key: public_key.to_vec(),
                fingerprint,
            },
        ))
    }

    /// Rotate the account ident (plan-23-B2): the OLD ident signs the chain
    /// link naming its successor, and the NEW ident proves possession with a
    /// role-separated registration proof. The old key becomes `past` and the
    /// signed link is recorded so consumers can follow the chain.
    pub fn rotate_ident(
        &self,
        owner: &str,
        new_public: &[u8],
        chain_signature: &[u8],
        possession_proof: &[u8],
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        validate_owner_name(owner)?;
        let Some((owner, old_key)) = self.owner_with_ident_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        let chain_message =
            crypto::ident_rotation_message(&owner.owner_display, &old_key.fingerprint, new_public);
        crypto::verify(&old_key.public_key, &chain_message, chain_signature)
            .map_err(|_| "invalid ident chain signature".to_string())?;
        let possession_message =
            crypto::registration_message(crypto::ROLE_IDENT, &owner.owner_display, new_public);
        crypto::verify(new_public, &possession_message, possession_proof)
            .map_err(|_| "invalid ident proof-of-possession signature".to_string())?;

        let fingerprint = crypto::fingerprint(new_public);
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start rotation transaction: {err}"))?;
        tx.execute(
            "UPDATE keys SET status = 'past', revoked_at = ?1 WHERE id = ?2",
            params![now, old_key.id],
        )
        .map_err(|err| format!("failed to retire ident key: {err}"))?;
        let new_key_id = insert_key_tx(
            &tx,
            owner.id,
            KEY_ROLE_IDENT,
            new_public,
            &fingerprint,
            now,
            "failed to register rotated ident key",
        )?;
        tx.execute(
            "INSERT INTO ident_chain (owner_id, old_key_id, new_key_id, signature, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![owner.id, old_key.id, new_key_id, chain_signature, now],
        )
        .map_err(|err| format!("failed to record ident chain link: {err}"))?;
        append_log_tx(
            &tx,
            "rotate",
            &format!(
                "{{\"owner\":{},\"oldIdentFingerprint\":{},\"newIdentFingerprint\":{}}}",
                json_value(&owner.owner_display),
                json_value(&old_key.fingerprint),
                json_value(&fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit ident rotation: {err}"))?;
        Ok((
            owner,
            KeyRecord {
                id: new_key_id,
                public_key: new_public.to_vec(),
                fingerprint,
            },
        ))
    }

    /// Re-anchor ceremony (plan-23 §3.6, total ident loss): a registry
    /// OPERATOR action, deliberately not an HTTP route — it runs against the
    /// database after out-of-band verification. Binds the name to a fresh
    /// ident with **no chain link**, so clients holding the old pin fail
    /// hard instead of silently following.
    pub fn reanchor_ident(&self, owner: &str, new_public: &[u8]) -> Result<KeyRecord, String> {
        validate_owner_name(owner)?;
        let Some((owner, old_key)) = self.owner_with_ident_key(owner)? else {
            return Err("unknown owner".to_string());
        };
        if new_public.len() != crypto::PUBLIC_KEY_LEN {
            return Err("malformed ident public key".to_string());
        }
        let fingerprint = crypto::fingerprint(new_public);
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start re-anchor transaction: {err}"))?;
        tx.execute(
            "UPDATE keys SET status = 'past', revoked_at = ?1 WHERE id = ?2",
            params![now, old_key.id],
        )
        .map_err(|err| format!("failed to retire ident key: {err}"))?;
        let key_id = insert_key_tx(
            &tx,
            owner.id,
            KEY_ROLE_IDENT,
            new_public,
            &fingerprint,
            now,
            "failed to register re-anchored ident key",
        )?;
        append_log_tx(
            &tx,
            "reanchor",
            &format!(
                "{{\"owner\":{},\"oldIdentFingerprint\":{},\"newIdentFingerprint\":{}}}",
                json_value(&owner.owner_display),
                json_value(&old_key.fingerprint),
                json_value(&fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit re-anchor: {err}"))?;
        Ok(KeyRecord {
            id: key_id,
            public_key: new_public.to_vec(),
            fingerprint,
        })
    }

    /// The owner's ident chain, oldest link first: each entry carries the
    /// old/new public keys and the old key's signature over the rotation
    /// message. An empty chain plus a current key that differs from a
    /// consumer's pin means the ident was re-anchored.
    pub fn ident_chain(
        &self,
        owner: &str,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>, Vec<u8>, i64)>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT old_keys.public_key, new_keys.public_key, c.signature, c.created_at
                 FROM ident_chain c
                 JOIN owners o ON o.id = c.owner_id
                 JOIN keys old_keys ON old_keys.id = c.old_key_id
                 JOIN keys new_keys ON new_keys.id = c.new_key_id
                 WHERE o.owner_folded = ?1 AND o.status = 'active'
                 ORDER BY c.id ASC",
            )
            .map_err(|err| format!("failed to prepare chain query: {err}"))?;
        let rows = statement
            .query_map(params![folded], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|err| format!("failed to load ident chain: {err}"))?;
        let mut chain = Vec::new();
        for row in rows {
            chain.push(row.map_err(|err| format!("failed to read ident chain: {err}"))?);
        }
        Ok(chain)
    }

    /// Revoke a machine's auth key and kill every session opened with it
    /// (plan-23 §3.6). Returns false when no current auth key matches.
    pub fn revoke_auth_key(&self, owner_id: i64, fingerprint: &str) -> Result<bool, String> {
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start revocation transaction: {err}"))?;
        let key_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM keys
                 WHERE owner_id = ?1 AND role = 'auth' AND status = 'current' AND fingerprint = ?2",
                params![owner_id, fingerprint],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load auth key: {err}"))?;
        let Some(key_id) = key_id else {
            tx.commit().ok();
            return Ok(false);
        };
        // bug-493: never revoke the account's last current machine key. No
        // ident-authorized route adds an auth key, so an account with none is
        // unrecoverable short of the operator `reanchor` ceremony. Publish
        // tokens are `auth` rows too but do not count as machines — a token
        // cannot enrol one (bug-492) — so an account left with only a token is
        // just as locked out. A token itself may be revoked here freely; the
        // guard is for machine keys. Checked inside the transaction so two
        // concurrent revocations cannot each see the other key as "remaining".
        let is_token: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM publish_tokens WHERE key_id = ?1)",
                params![key_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("failed to classify auth key: {err}"))?;
        if !is_token {
            let remaining: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM keys k
                     WHERE k.owner_id = ?1 AND k.role = 'auth' AND k.status = 'current'
                       AND k.id != ?2
                       AND NOT EXISTS (SELECT 1 FROM publish_tokens t WHERE t.key_id = k.id)",
                    params![owner_id, key_id],
                    |row| row.get(0),
                )
                .map_err(|err| format!("failed to count machine keys: {err}"))?;
            if remaining == 0 {
                return Err(
                    "cannot revoke the account's last auth key; link another machine first"
                        .to_string(),
                );
            }
        }
        tx.execute(
            "UPDATE keys SET status = 'revoked', revoked_at = ?1 WHERE id = ?2",
            params![now, key_id],
        )
        .map_err(|err| format!("failed to revoke auth key: {err}"))?;
        tx.execute(
            "UPDATE sessions SET revoked_at = ?1 WHERE key_id = ?2 AND revoked_at IS NULL",
            params![now, key_id],
        )
        .map_err(|err| format!("failed to revoke sessions: {err}"))?;
        let owner_display: String = tx
            .query_row(
                "SELECT owner_display FROM owners WHERE id = ?1",
                params![owner_id],
                |row| row.get(0),
            )
            .map_err(|err| format!("failed to load owner: {err}"))?;
        append_log_tx(
            &tx,
            "revoke",
            &format!(
                "{{\"owner\":{},\"authFingerprint\":{}}}",
                json_value(&owner_display),
                json_value(fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit revocation: {err}"))?;
        Ok(true)
    }

    /// Complete a login challenge: one minted by `/auth/challenge` for an auth
    /// key. A revocation challenge — the ident key's — is refused here even
    /// with a valid ident signature (bug-493).
    pub fn complete_challenge(
        &self,
        challenge_id: &str,
        signature: &[u8],
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        self.complete_challenge_with(
            challenge_id,
            signature,
            CHALLENGE_PURPOSE_LOGIN,
            KEY_ROLE_AUTH,
            crypto::challenge_message,
        )
    }

    /// Complete an ident challenge whose signature covers the revocation
    /// message (challenge + the fingerprint being revoked). Revocation
    /// authority is the ident key alone (plan-23 §3.6): a login challenge for
    /// an auth key is refused on purpose and role, not merely on signature —
    /// the attacker holding that auth key can sign the revocation message
    /// perfectly well (bug-493 / audit-3 REPO-02).
    pub fn complete_revocation_challenge(
        &self,
        challenge_id: &str,
        signature: &[u8],
        fingerprint: &str,
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        self.complete_challenge_with(
            challenge_id,
            signature,
            CHALLENGE_PURPOSE_REVOKE,
            KEY_ROLE_IDENT,
            |id, nonce| crypto::revocation_message(id, nonce, fingerprint),
        )
    }

    /// The one challenge completer. `purpose` must match what the challenge
    /// was minted with and `role` the challenged key's role; both are checked
    /// before the signature and before the row is burned, so a challenge
    /// presented to the wrong completer stays usable for its own.
    fn complete_challenge_with(
        &self,
        challenge_id: &str,
        signature: &[u8],
        purpose: &str,
        role: &str,
        message: impl Fn(&str, &[u8]) -> Vec<u8>,
    ) -> Result<(OwnerRecord, KeyRecord), String> {
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start login transaction: {err}"))?;
        let loaded = tx
            .query_row(
                "SELECT c.id, c.owner_id, c.key_id, c.nonce, c.expires_at, c.used_at,
                        o.owner_display, k.public_key, k.fingerprint, c.purpose, k.role
                 FROM auth_challenges c
                 JOIN owners o ON o.id = c.owner_id
                 JOIN keys k ON k.id = c.key_id
                 WHERE c.id = ?1",
                params![challenge_id],
                |row| {
                    Ok((
                        ChallengeRecord {
                            id: row.get(0)?,
                            owner_id: row.get(1)?,
                            key_id: row.get(2)?,
                            nonce: row.get(3)?,
                            expires_at: row.get(4)?,
                            used_at: row.get(5)?,
                        },
                        OwnerRecord {
                            id: row.get(1)?,
                            owner_display: row.get(6)?,
                        },
                        KeyRecord {
                            id: row.get(2)?,
                            public_key: row.get(7)?,
                            fingerprint: row.get(8)?,
                        },
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(|err| format!("failed to load auth challenge: {err}"))?;
        let Some((challenge, owner, key, minted_purpose, key_role)) = loaded else {
            return Err("unknown challenge".to_string());
        };
        if minted_purpose != purpose {
            let operation = if purpose == CHALLENGE_PURPOSE_REVOKE {
                "revocation"
            } else {
                purpose
            };
            return Err(format!("challenge was not issued for {operation}"));
        }
        if key_role != role {
            return Err(format!("challenge key is not the account's {role} key"));
        }
        if challenge.used_at.is_some() {
            return Err("reused challenge".to_string());
        }
        if challenge.expires_at <= now_unix() {
            return Err("expired challenge".to_string());
        }
        let message = message(&challenge.id, &challenge.nonce);
        crypto::verify(&key.public_key, &message, signature)?;
        tx.execute(
            "UPDATE auth_challenges SET used_at = ?1 WHERE id = ?2 AND used_at IS NULL",
            params![now_unix(), challenge_id],
        )
        .map_err(|err| format!("failed to mark challenge used: {err}"))?;
        tx.commit()
            .map_err(|err| format!("failed to commit login: {err}"))?;
        Ok((owner, key))
    }

    pub fn insert_session(&self, session: &NewSession) -> Result<String, String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO sessions (id, owner_id, key_id, jwt_id, issued_at, expires_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            params![
                id,
                session.owner_id,
                session.key_id,
                session.jwt_id,
                session.issued_at,
                session.expires_at
            ],
        )
        .map_err(|err| format!("failed to store session: {err}"))?;
        Ok(id)
    }

    pub fn session_exists(&self, jwt_id: &str) -> Result<bool, String> {
        let conn = self.conn();
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM sessions WHERE jwt_id = ?1 AND revoked_at IS NULL",
                params![jwt_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load session: {err}"))?;
        Ok(exists.is_some())
    }

    pub fn server_secret(&self) -> Result<Vec<u8>, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT secret FROM server_secrets WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(|err| format!("failed to load server signing secret: {err}"))
    }

    pub fn count_owners(&self) -> Result<i64, String> {
        let conn = self.conn();
        conn.query_row("SELECT COUNT(*) FROM owners", [], |row| row.get(0))
            .map_err(|err| format!("failed to count owners: {err}"))
    }
}

/// The owner's current ident key, read on the caller's connection or
/// transaction.
pub(super) fn current_ident_key_on(
    conn: &Connection,
    owner_id: i64,
) -> Result<Option<KeyRecord>, String> {
    conn.query_row(
        "SELECT id, public_key, fingerprint FROM keys
         WHERE owner_id = ?1 AND role = 'ident' AND status = 'current'",
        params![owner_id],
        |row| {
            Ok(KeyRecord {
                id: row.get(0)?,
                public_key: row.get(1)?,
                fingerprint: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|err| format!("failed to load ident key: {err}"))
}

/// The ONE place a `keys` row is created (bug-580).
///
/// Role separation is an account-wide invariant, not a per-endpoint check: an
/// account's current ident key and every current auth key must be DIFFERENT
/// keys. Domain-separated proof messages stop a proof being replayed across
/// roles; they do nothing to stop a client deliberately offering the same
/// public key for both, and every insertion path verified its proofs and then
/// inserted without comparing. Theft of a machine auth private key would then
/// also be theft of the identity-signing key, which is the whole separation the
/// design exists to provide.
///
/// Funnelling every insert through here is deliberate. The bug report named two
/// paths; there were five, and the two it missed included `issue_publish_token`
/// — the worst of them, because a publish token is a DELEGATED, exportable
/// credential, so a token equal to the ident key hands the account identity to
/// whatever CI holds the token. A per-site check would have been written from
/// that same incomplete list. `key_insertion_has_exactly_one_writer` keeps it
/// the only writer, so a sixth path cannot be added without meeting this.
///
/// The check runs INSIDE the caller's transaction, so a concurrent rotation
/// cannot slip a colliding key in between the read and the insert. In
/// `register_owner` the auth row is written first, so the ident insert's query
/// sees it and the two-argument case needs no separate comparison.
///
/// Existing accounts that already hold a colliding pair are deliberately left
/// alone: this gates the creation of NEW credentials only, and never revokes
/// live access (the bug's stated non-goal).
pub(super) fn insert_key_tx(
    tx: &rusqlite::Transaction<'_>,
    owner_id: i64,
    role: &str,
    public_key: &[u8],
    fingerprint: &str,
    now: i64,
    what: &str,
) -> Result<i64, String> {
    let opposite = match role {
        KEY_ROLE_AUTH => KEY_ROLE_IDENT,
        KEY_ROLE_IDENT => KEY_ROLE_AUTH,
        other => return Err(format!("unknown key role '{other}'")),
    };
    let collision: Option<i64> = tx
        .query_row(
            "SELECT id FROM keys
             WHERE owner_id = ?1 AND role = ?2 AND status = 'current' AND fingerprint = ?3",
            params![owner_id, opposite, fingerprint],
            |row| row.get(0),
        )
        .optional()
        // Deliberately the caller's own `what` message, not a message about this
        // query. `losing_the_key_tables_errors_revocation_and_chain_reads`
        // (bug-264 / REPO-09) pins that a broken `keys` table degrades to an
        // error naming the OPERATION — "failed to register token key: …" — and
        // that contract must not depend on which statement happens to touch the
        // table first. Adding a read ahead of the insert would otherwise have
        // silently re-worded five operator-facing failures.
        .map_err(|err| format!("{what}: {err}"))?;
    if collision.is_some() {
        return Err(format!(
            "key role separation: this public key is already the account's current {opposite} \
             key. A machine auth key and the account ident key must be different keys, so that \
             losing one does not lose the other."
        ));
    }
    tx.execute(
        "INSERT INTO keys (owner_id, role, public_key, fingerprint, status, created_at, revoked_at)
         VALUES (?1, ?2, ?3, ?4, 'current', ?5, NULL)",
        params![owner_id, role, public_key, fingerprint, now],
    )
    .map_err(|err| format!("{what}: {err}"))?;
    Ok(tx.last_insert_rowid())
}
