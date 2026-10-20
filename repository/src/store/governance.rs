//! plan-176-L: the registry store's governance — orgs, publish tokens and ownership transfers.
//! The `Store` handle and its connection live in `repository/src/store/mod.rs`.

use super::*;

impl Store {
    /// Resolve an owner name to its row (any account: user or org).
    pub(super) fn owner_record(&self, owner: &str) -> Result<Option<OwnerRecord>, String> {
        let folded = fold_owner(owner);
        let conn = self.conn();
        conn.query_row(
            "SELECT id, owner_display FROM owners WHERE owner_folded = ?1 AND status = 'active'",
            params![folded],
            |row| {
                Ok(OwnerRecord {
                    id: row.get(0)?,
                    owner_display: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("failed to load owner: {err}"))
    }

    // --- Orgs (plan-10-D1) -------------------------------------------------

    /// A member's role in an org (`owner`/`admin`/`publisher`), or None.
    pub fn org_member_role(&self, org: &str, member: &str) -> Result<Option<String>, String> {
        let org_folded = fold_owner(org);
        let member_folded = fold_owner(member);
        let conn = self.conn();
        conn.query_row(
            "SELECT m.role
             FROM org_members m
             JOIN owners o ON o.id = m.org_id
             JOIN owners u ON u.id = m.member_id
             WHERE o.owner_folded = ?1 AND u.owner_folded = ?2",
            params![org_folded, member_folded],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| format!("failed to load org member role: {err}"))
    }

    /// Grant (or update) a member's org role and log it (plan-10-D1). Caller
    /// verifies the granting member's authority before this runs.
    pub fn grant_org_member(&self, org: &str, member: &str, role: &str) -> Result<(), String> {
        if !matches!(role, "owner" | "admin" | "publisher") {
            return Err("role must be owner, admin, or publisher".to_string());
        }
        let Some(org_record) = self.owner_record(org)? else {
            return Err("unknown org".to_string());
        };
        let Some(member_record) = self.owner_record(member)? else {
            return Err("unknown member account".to_string());
        };
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start org transaction: {err}"))?;
        tx.execute(
            "INSERT INTO org_members (org_id, member_id, role, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(org_id, member_id) DO UPDATE SET role = excluded.role",
            params![org_record.id, member_record.id, role, now],
        )
        .map_err(|err| format!("failed to record org member: {err}"))?;
        append_log_tx(
            &tx,
            "org-role",
            &format!(
                "{{\"org\":{},\"member\":{},\"role\":{}}}",
                json_value(&org_record.owner_display),
                json_value(&member_record.owner_display),
                json_value(role),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit org member grant: {err}"))?;
        Ok(())
    }

    /// Remove a member from an org and log it (plan-10-D1). Returns false when
    /// the member had no role.
    pub fn remove_org_member(&self, org: &str, member: &str) -> Result<bool, String> {
        let Some(org_record) = self.owner_record(org)? else {
            return Err("unknown org".to_string());
        };
        let Some(member_record) = self.owner_record(member)? else {
            return Err("unknown member account".to_string());
        };
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start org transaction: {err}"))?;
        let removed = tx
            .execute(
                "DELETE FROM org_members WHERE org_id = ?1 AND member_id = ?2",
                params![org_record.id, member_record.id],
            )
            .map_err(|err| format!("failed to remove org member: {err}"))?;
        if removed == 0 {
            tx.commit().ok();
            return Ok(false);
        }
        append_log_tx(
            &tx,
            "org-role",
            &format!(
                "{{\"org\":{},\"member\":{},\"role\":{}}}",
                json_value(&org_record.owner_display),
                json_value(&member_record.owner_display),
                json_value("removed"),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit org member removal: {err}"))?;
        Ok(true)
    }

    /// The org's members as `(member_display, role)`, oldest first.
    pub fn list_org_members(&self, org: &str) -> Result<Vec<(String, String)>, String> {
        let org_folded = fold_owner(org);
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT u.owner_display, m.role
                 FROM org_members m
                 JOIN owners o ON o.id = m.org_id
                 JOIN owners u ON u.id = m.member_id
                 WHERE o.owner_folded = ?1
                 ORDER BY m.id ASC",
            )
            .map_err(|err| format!("failed to prepare org query: {err}"))?;
        let rows = statement
            .query_map(params![org_folded], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|err| format!("failed to list org members: {err}"))?;
        let mut members = Vec::new();
        for row in rows {
            members.push(row.map_err(|err| format!("failed to read org member: {err}"))?);
        }
        Ok(members)
    }

    // --- Publish tokens (plan-10-D1) --------------------------------------

    /// Issue a scoped, TTL-bounded publish token: register its auth key on the
    /// owner and record the scope/expiry, logged. The token can open sessions
    /// and request attestations only within `scope` and only until `expires_at`
    /// — it never bypasses the ident-proof requirement. Caller verifies the
    /// owner-ident authorization before this runs.
    pub fn issue_publish_token(
        &self,
        owner: &str,
        token_public: &[u8],
        proof: &[u8],
        scope: &str,
        ttl_secs: i64,
    ) -> Result<(OwnerRecord, KeyRecord, i64), String> {
        let Some(owner_record) = self.owner_record(owner)? else {
            return Err("unknown owner".to_string());
        };
        let message = crypto::registration_message(
            crypto::ROLE_AUTH,
            &owner_record.owner_display,
            token_public,
        );
        crypto::verify(token_public, &message, proof)
            .map_err(|_| "invalid token proof-of-possession signature".to_string())?;
        if scope.is_empty() || scope.len() > 255 {
            return Err("token scope must be 1..=255 bytes".to_string());
        }
        if ttl_secs <= 0 || ttl_secs > 365 * 24 * 3600 {
            return Err("token ttl must be 1..=31536000 seconds".to_string());
        }
        let fingerprint = crypto::fingerprint(token_public);
        let now = now_unix();
        let expires_at = now + ttl_secs;
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start token transaction: {err}"))?;
        let key_id = insert_key_tx(
            &tx,
            owner_record.id,
            KEY_ROLE_AUTH,
            token_public,
            &fingerprint,
            now,
            "failed to register token key",
        )?;
        tx.execute(
            "INSERT INTO publish_tokens (owner_id, key_id, scope, expires_at, revoked_at, created_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
            params![owner_record.id, key_id, scope, expires_at, now],
        )
        .map_err(|err| format!("failed to record publish token: {err}"))?;
        append_log_tx(
            &tx,
            "token-issue",
            &format!(
                "{{\"owner\":{},\"tokenFingerprint\":{},\"scope\":{}}}",
                json_value(&owner_record.owner_display),
                json_value(&fingerprint),
                json_value(scope),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit token issue: {err}"))?;
        Ok((
            owner_record,
            KeyRecord {
                id: key_id,
                public_key: token_public.to_vec(),
                fingerprint,
            },
            expires_at,
        ))
    }

    /// The publish-token scope/expiry/revocation for an auth key, if it is a
    /// token. Used at `/signing` to bound what a token session may attest.
    pub fn publish_token_for_key(
        &self,
        key_id: i64,
    ) -> Result<Option<(String, i64, Option<i64>)>, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT scope, expires_at, revoked_at FROM publish_tokens WHERE key_id = ?1",
            params![key_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|err| format!("failed to load publish token: {err}"))
    }

    /// Revoke a publish token (its auth key and any sessions), logged. Returns
    /// false when no active token matches the fingerprint.
    pub fn revoke_publish_token(&self, owner: &str, fingerprint: &str) -> Result<bool, String> {
        let Some(owner_record) = self.owner_record(owner)? else {
            return Err("unknown owner".to_string());
        };
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start token revoke transaction: {err}"))?;
        let key_id: Option<i64> = tx
            .query_row(
                "SELECT k.id
                 FROM publish_tokens t
                 JOIN keys k ON k.id = t.key_id
                 WHERE t.owner_id = ?1 AND k.fingerprint = ?2 AND t.revoked_at IS NULL",
                params![owner_record.id, fingerprint],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| format!("failed to load token: {err}"))?;
        let Some(key_id) = key_id else {
            tx.commit().ok();
            return Ok(false);
        };
        tx.execute(
            "UPDATE publish_tokens SET revoked_at = ?1 WHERE key_id = ?2",
            params![now, key_id],
        )
        .map_err(|err| format!("failed to revoke token: {err}"))?;
        tx.execute(
            "UPDATE keys SET status = 'revoked', revoked_at = ?1 WHERE id = ?2",
            params![now, key_id],
        )
        .map_err(|err| format!("failed to revoke token key: {err}"))?;
        tx.execute(
            "UPDATE sessions SET revoked_at = ?1 WHERE key_id = ?2 AND revoked_at IS NULL",
            params![now, key_id],
        )
        .map_err(|err| format!("failed to close token sessions: {err}"))?;
        append_log_tx(
            &tx,
            "token-revoke",
            &format!(
                "{{\"owner\":{},\"tokenFingerprint\":{}}}",
                json_value(&owner_record.owner_display),
                json_value(fingerprint),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit token revoke: {err}"))?;
        Ok(true)
    }

    // --- Ownership transfer (plan-10-D1) ----------------------------------

    /// The account that currently owns a package (may differ from the ident
    /// string's owner after a transfer).
    pub fn package_owner(&self, ident: &str) -> Result<Option<OwnerRecord>, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT o.id, o.owner_display
             FROM packages p JOIN owners o ON o.id = p.owner_id
             WHERE p.ident = ?1",
            params![ident],
            |row| {
                Ok(OwnerRecord {
                    id: row.get(0)?,
                    owner_display: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("failed to load package owner: {err}"))
    }

    /// The terms the next transfer of `ident` must sign (plan-168-A §4.3): the
    /// number of accepted transfers so far, the stored owner and their current
    /// ident key, and the cumulative release digest. `None` for an unknown
    /// package.
    pub fn transfer_terms(&self, ident: &str) -> Result<Option<TransferTerms>, String> {
        let conn = self.conn();
        Ok(transfer_terms_on(&conn, ident)?.map(|(_, terms)| terms))
    }

    /// Record a signed transfer offer: the current owner offers `ident` to
    /// `to_owner` on the terms `(sequence, release_digest)`, signed by the
    /// owner's current ident over `crypto::transfer_offer_message_v2`.
    ///
    /// The terms are recomputed and the signature verified inside the writing
    /// transaction, so an offer whose terms moved under it (a publish or an
    /// accept in between) is refused rather than recorded with a signature
    /// over terms that no longer hold.
    pub fn create_transfer_offer(
        &self,
        ident: &str,
        from_owner: &str,
        to_owner: &str,
        sequence: i64,
        release_digest: &str,
        offer_signature: &[u8],
    ) -> Result<(), String> {
        let Some(package_owner) = self.package_owner(ident)? else {
            return Err("unknown package".to_string());
        };
        if fold_owner(&package_owner.owner_display) != fold_owner(from_owner) {
            return Err("offering owner does not currently own the package".to_string());
        }
        let Some(to_record) = self.owner_record(to_owner)? else {
            return Err("unknown recipient account".to_string());
        };
        if to_record.id == package_owner.id {
            return Err("cannot transfer a package to its current owner".to_string());
        }
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start transfer transaction: {err}"))?;
        // Re-read the package inside the *writing* transaction and re-verify
        // ownership against it (bug-274). The checks above ran under their own
        // short-lived lock acquisitions, and axum handlers run concurrently over
        // one `Arc<Mutex<Connection>>`, so between them and this commit an accept
        // can land and move the package. The UPSERT below resets `accepted_at` to
        // NULL unconditionally, so without this re-check a still-in-flight offer
        // could overwrite an already-accepted row and re-list itself as pending
        // under the previous owner's now-stale authority.
        //
        // Returning here rolls the transaction back, which is the intended abort.
        let (package_id, current_owner_id): (i64, i64) = tx
            .query_row(
                "SELECT id, owner_id FROM packages WHERE ident = ?1",
                params![ident],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|err| format!("failed to load package: {err}"))?;
        if current_owner_id != package_owner.id {
            return Err("offering owner does not currently own the package".to_string());
        }
        let Some((_, terms)) = transfer_terms_on(&tx, ident)? else {
            return Err("unknown package".to_string());
        };
        if terms.sequence != sequence || terms.release_digest != release_digest {
            return Err(STALE_TRANSFER_TERMS.to_string());
        }
        let message = crypto::transfer_offer_message_v2(crypto::TransferTermsRef {
            ident,
            sequence,
            from_owner: &terms.current_owner.owner_display,
            from_ident_fingerprint: &terms.current_ident.fingerprint,
            to_owner: &to_record.owner_display,
            release_digest,
        });
        crypto::verify(&terms.current_ident.public_key, &message, offer_signature)
            .map_err(|_| "invalid transfer offer ident signature".to_string())?;
        tx.execute(
            "INSERT INTO transfer_offers (package_id, from_owner_id, to_owner_id, created_at,
                 accepted_at, sequence, from_ident_key_id, release_digest, offer_signature)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8)
             ON CONFLICT(package_id) DO UPDATE SET
               from_owner_id = excluded.from_owner_id,
               to_owner_id = excluded.to_owner_id,
               created_at = excluded.created_at,
               accepted_at = NULL,
               sequence = excluded.sequence,
               from_ident_key_id = excluded.from_ident_key_id,
               release_digest = excluded.release_digest,
               offer_signature = excluded.offer_signature",
            params![
                package_id,
                package_owner.id,
                to_record.id,
                now,
                sequence,
                terms.current_ident.id,
                release_digest,
                offer_signature
            ],
        )
        .map_err(|err| format!("failed to record transfer offer: {err}"))?;
        append_log_tx(
            &tx,
            "transfer-offer",
            &format!(
                "{{\"ident\":{},\"from\":{},\"to\":{}}}",
                json_value(ident),
                json_value(&package_owner.owner_display),
                json_value(&to_record.owner_display),
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit transfer offer: {err}"))?;
        Ok(())
    }

    /// Accept a pending signed transfer: verify the recipient's signature over
    /// `crypto::transfer_accept_message_v2` on the package's current terms,
    /// require those terms to be exactly the pending offer's, then append the
    /// `transfer_history` link, re-bind the package to `to_owner`, and log it.
    /// Already-published versions keep verifying against the old ident's
    /// proofs (issued facts); new versions publish under the new owner's ident.
    pub fn accept_transfer(
        &self,
        ident: &str,
        to_owner: &str,
        accept_signature: &[u8],
    ) -> Result<(), String> {
        let Some(to_record) = self.owner_record(to_owner)? else {
            return Err("unknown recipient account".to_string());
        };
        let now = now_unix();
        let mut conn = self.conn();
        let tx = conn
            .transaction()
            .map_err(|err| format!("failed to start transfer transaction: {err}"))?;
        let Some((package_id, terms)) = transfer_terms_on(&tx, ident)? else {
            return Err(NO_PENDING_OFFER.to_string());
        };
        let Some(to_ident) = current_ident_key_on(&tx, to_record.id)? else {
            return Err("recipient has no current ident key".to_string());
        };
        let message = crypto::transfer_accept_message_v2(crypto::TransferTermsRef {
            ident,
            sequence: terms.sequence,
            from_owner: &terms.current_owner.owner_display,
            from_ident_fingerprint: &terms.current_ident.fingerprint,
            to_owner: &to_record.owner_display,
            release_digest: &terms.release_digest,
        });
        crypto::verify(&to_ident.public_key, &message, accept_signature)
            .map_err(|_| "invalid transfer accept ident signature".to_string())?;
        let offer: Option<PendingOfferRow> = tx
            .query_row(
                // `o.from_owner_id = p.owner_id` is the stale-offer guard
                // (bug-274): an offer is only acceptable while the account that
                // made it is still the package's owner. Without it, an offer
                // resurrected after ownership moved — or simply left pending
                // across a transfer — could re-bind the package on the authority
                // of an account that no longer holds it.
                "SELECT o.id, o.sequence, o.from_ident_key_id, o.release_digest,
                        o.offer_signature, o.created_at
                 FROM transfer_offers o
                 JOIN packages p ON p.id = o.package_id
                 WHERE p.ident = ?1 AND o.to_owner_id = ?2 AND o.accepted_at IS NULL
                   AND o.from_owner_id = p.owner_id",
                params![ident, to_record.id],
                |row| {
                    Ok(PendingOfferRow {
                        id: row.get(0)?,
                        sequence: row.get(1)?,
                        from_ident_key_id: row.get(2)?,
                        release_digest: row.get(3)?,
                        offer_signature: row.get(4)?,
                        offered_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(|err| format!("failed to load transfer offer: {err}"))?;
        let Some(offer) = offer else {
            return Err(NO_PENDING_OFFER.to_string());
        };
        let (Some(sequence), Some(from_key_id), Some(digest), Some(offer_signature)) = (
            offer.sequence,
            offer.from_ident_key_id,
            offer.release_digest,
            offer.offer_signature,
        ) else {
            return Err(
                "this offer predates signed transfers; the owner must re-offer".to_string(),
            );
        };
        // The recipient signed the CURRENT terms; the offer must have been made
        // on exactly those, under the offerer's still-current ident key, or the
        // recorded link would pair two signatures over different facts.
        if sequence != terms.sequence
            || digest != terms.release_digest
            || from_key_id != terms.current_ident.id
        {
            return Err(STALE_TRANSFER_TERMS.to_string());
        }
        tx.execute(
            "INSERT INTO transfer_history
                 (package_id, sequence, from_owner_id, from_ident_key_id, to_owner_id,
                  to_ident_key_id, release_digest, offer_signature, accept_signature,
                  offered_at, accepted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                package_id,
                sequence,
                terms.current_owner.id,
                from_key_id,
                to_record.id,
                to_ident.id,
                digest,
                offer_signature,
                accept_signature,
                offer.offered_at,
                now
            ],
        )
        .map_err(|err| format!("failed to record transfer history: {err}"))?;
        tx.execute(
            "UPDATE packages SET owner_id = ?1 WHERE id = ?2",
            params![to_record.id, package_id],
        )
        .map_err(|err| format!("failed to re-bind package owner: {err}"))?;
        tx.execute(
            "UPDATE transfer_offers SET accepted_at = ?1 WHERE id = ?2",
            params![now, offer.id],
        )
        .map_err(|err| format!("failed to close transfer offer: {err}"))?;
        append_log_tx(
            &tx,
            "transfer-accept",
            &format!(
                "{{\"ident\":{},\"to\":{},\"sequence\":{}}}",
                json_value(ident),
                json_value(&to_record.owner_display),
                sequence,
            ),
        )?;
        tx.commit()
            .map_err(|err| format!("failed to commit transfer accept: {err}"))?;
        Ok(())
    }

    /// Whether the package moved through a transfer accepted before signed
    /// transfers existed (plan-168-B): an unsigned legacy history link, which
    /// no client can authenticate.
    pub fn has_legacy_transfer(&self, ident: &str) -> Result<bool, String> {
        let conn = self.conn();
        conn.query_row(
            "SELECT EXISTS (
                 SELECT 1 FROM transfer_history h JOIN packages p ON p.id = h.package_id
                 WHERE p.ident = ?1 AND h.offer_signature IS NULL)",
            params![ident],
            |row| row.get(0),
        )
        .map_err(|err| format!("failed to read transfer history: {err}"))
    }

    /// The package's accepted transfers, oldest first (plan-168-A §4.3). A
    /// legacy link (replayed from the transparency log) has no keys, digest or
    /// signatures.
    pub fn transfer_history(&self, ident: &str) -> Result<Vec<TransferRecord>, String> {
        let conn = self.conn();
        let mut statement = conn
            .prepare(
                "SELECT h.sequence, from_o.owner_display, from_k.public_key,
                        to_o.owner_display, to_k.public_key, h.release_digest,
                        h.offer_signature, h.accept_signature, h.offered_at, h.accepted_at
                 FROM transfer_history h
                 JOIN packages p ON p.id = h.package_id
                 JOIN owners from_o ON from_o.id = h.from_owner_id
                 JOIN owners to_o ON to_o.id = h.to_owner_id
                 LEFT JOIN keys from_k ON from_k.id = h.from_ident_key_id
                 LEFT JOIN keys to_k ON to_k.id = h.to_ident_key_id
                 WHERE p.ident = ?1
                 ORDER BY h.sequence ASC",
            )
            .map_err(|err| format!("failed to prepare transfer history query: {err}"))?;
        let rows = statement
            .query_map(params![ident], |row| {
                Ok(TransferRecord {
                    sequence: row.get(0)?,
                    from_owner: row.get(1)?,
                    from_ident_key: row.get(2)?,
                    to_owner: row.get(3)?,
                    to_ident_key: row.get(4)?,
                    release_digest: row.get(5)?,
                    offer_signature: row.get(6)?,
                    accept_signature: row.get(7)?,
                    offered_at: row.get(8)?,
                    accepted_at: row.get(9)?,
                })
            })
            .map_err(|err| format!("failed to load transfer history: {err}"))?;
        let mut history = Vec::new();
        for row in rows {
            history.push(row.map_err(|err| format!("failed to read transfer history: {err}"))?);
        }
        Ok(history)
    }
}

/// The terms the next transfer of a package signs (plan-168-A §4.3).
#[derive(Debug, Clone)]
pub struct TransferTerms {
    /// The number of accepted transfers so far: the next link's position.
    pub sequence: i64,
    /// The stored (current) owner, who must sign the offer.
    pub current_owner: OwnerRecord,
    /// That owner's current ident key, which signs the offer.
    pub current_ident: KeyRecord,
    /// `crypto::release_digest` over every version of the package.
    pub release_digest: String,
}

/// One accepted transfer, as `/index` serves it (plan-168-A §4.4). Every
/// `Option` is `None` exactly for a legacy link replayed from the log.
#[derive(Debug, Clone)]
pub struct TransferRecord {
    pub sequence: i64,
    pub from_owner: String,
    pub from_ident_key: Option<Vec<u8>>,
    pub to_owner: String,
    pub to_ident_key: Option<Vec<u8>>,
    pub release_digest: Option<String>,
    pub offer_signature: Option<Vec<u8>>,
    pub accept_signature: Option<Vec<u8>>,
    pub offered_at: i64,
    pub accepted_at: i64,
}

/// A pending `transfer_offers` row as `accept_transfer` reads it. The four
/// term columns are NULL on an offer recorded before signed transfers.
struct PendingOfferRow {
    id: i64,
    sequence: Option<i64>,
    from_ident_key_id: Option<i64>,
    release_digest: Option<String>,
    offer_signature: Option<Vec<u8>>,
    offered_at: i64,
}

const STALE_TRANSFER_TERMS: &str =
    "transfer terms are stale (sequence/releases changed); re-run the offer";

const NO_PENDING_OFFER: &str = "no pending transfer offer for this account";

/// [`Store::transfer_terms`] on the caller's connection or transaction, plus
/// the package row id. `None` for an unknown package.
fn transfer_terms_on(
    conn: &Connection,
    ident: &str,
) -> Result<Option<(i64, TransferTerms)>, String> {
    let package: Option<(i64, i64, String)> = conn
        .query_row(
            "SELECT p.id, o.id, o.owner_display
             FROM packages p JOIN owners o ON o.id = p.owner_id
             WHERE p.ident = ?1",
            params![ident],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|err| format!("failed to load package owner: {err}"))?;
    let Some((package_id, owner_id, owner_display)) = package else {
        return Ok(None);
    };
    let sequence: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transfer_history WHERE package_id = ?1",
            params![package_id],
            |row| row.get(0),
        )
        .map_err(|err| format!("failed to count transfer history: {err}"))?;
    let Some(current_ident) = current_ident_key_on(conn, owner_id)? else {
        return Err("package owner has no current ident key".to_string());
    };
    let mut statement = conn
        .prepare("SELECT version, hash FROM package_versions WHERE package_id = ?1")
        .map_err(|err| format!("failed to prepare release digest query: {err}"))?;
    let rows = statement
        .query_map(params![package_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|err| format!("failed to load releases: {err}"))?;
    let mut releases = Vec::new();
    for row in rows {
        releases.push(row.map_err(|err| format!("failed to read release: {err}"))?);
    }
    let release_digest = crypto::release_digest(
        releases
            .iter()
            .map(|(version, hash)| (version.as_str(), hash.as_str())),
    );
    Ok(Some((
        package_id,
        TransferTerms {
            sequence,
            current_owner: OwnerRecord {
                id: owner_id,
                owner_display,
            },
            current_ident,
            release_digest,
        },
    )))
}
