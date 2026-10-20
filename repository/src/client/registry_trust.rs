//! plan-176-L: the registry client's registry_trust code. The module map lives in
//! `repository/src/client/mod.rs`.

use super::*;

/// Fetch the signed log checkpoint, verify it under the pinned server key,
/// and enforce append-only growth against the locally pinned checkpoint
/// (plan-23-B3): a shrunken tree, or a different root at the same size, is a
/// hard error — never silently re-pinned.
/// Fetch and verify the signed checkpoint **without** pinning it.
///
/// Splitting the pin out matters for `verify_log_consistency` (bug-276 R2):
/// pinning the new head before the consistency proof is checked means a detected
/// fork has already overwritten the pin it would be detected against, so the
/// evidence self-destructs after a single error and the next call succeeds
/// against the forked history.
fn fetch_checkpoint_unpinned(
    repo_url: &str,
    paths: &LocalPaths,
) -> Result<CheckpointResponse, String> {
    let server_key = ensure_server_key(repo_url, paths)?;
    let checkpoint = get_json::<CheckpointResponse>(repo_url, "/log/checkpoint")?;
    let root = decode_hex32(&checkpoint.root_hash, "rootHash")?;
    let signature = crypto::decode_bytes(&checkpoint.signature, "signature")?;
    crypto::verify(
        &server_key,
        &crate::log::checkpoint_signing_input(checkpoint.size as u64, &root),
        &signature,
    )
    .map_err(|_| "log checkpoint does not verify under the pinned server key".to_string())?;
    if let Some((pinned_size, pinned_root)) = local::read_checkpoint(paths)? {
        if checkpoint.size < pinned_size {
            return Err(format!(
                "registry log ROLLBACK: checkpoint size {} is smaller than the pinned size {pinned_size}",
                checkpoint.size
            ));
        }
        if checkpoint.size == pinned_size && checkpoint.root_hash != pinned_root {
            return Err(
                "registry log FORK: checkpoint root differs from the pinned root at the same size"
                    .to_string(),
            );
        }
    }
    Ok(checkpoint)
}

/// Fetch, verify and pin the current checkpoint.
///
/// This is the **only** primitive that advances the local pin, and every advance
/// past an existing pin is gated on an RFC 6962 consistency proof (bug-582).
/// Signatures and monotonicity alone cannot establish an append-only history: a
/// registry holding its online key can sign any larger tree it likes, so a
/// bigger head is a *candidate*, never evidence. Before the gate moved here,
/// `fetch_checkpoint` wrote any `size > pinned_size` straight over the pin,
/// which erased the root the fork would have been detected against — and
/// `verify_publish_inclusion`, the whole of `pkg install --proof`'s contact with
/// the log, advanced through it with no consistency check anywhere in the flow.
pub fn fetch_checkpoint(repo_url: &str, paths: &LocalPaths) -> Result<CheckpointResponse, String> {
    // Read the pin BEFORE the fetch, so the value proven against is the one the
    // client already accepted and not anything this exchange may have written.
    let pinned = local::read_checkpoint(paths)?;
    // Deliberately unpinned (bug-276 R2): the candidate head must not overwrite
    // the pin until its consistency against that pin has been proven, or a fork
    // erases the very evidence it would be caught by.
    let checkpoint = fetch_checkpoint_unpinned(repo_url, paths)?;
    let Some((pinned_size, pinned_root)) = pinned else {
        // Trust on first use: there is no predecessor to prove an extension of.
        local::write_checkpoint(paths, checkpoint.size, &checkpoint.root_hash)?;
        return Ok(checkpoint);
    };
    if checkpoint.size == pinned_size {
        // `fetch_checkpoint_unpinned` has already refused every other root at
        // this size, so this IS the pinned head. Nothing advances, so there is
        // nothing to prove and no proof to ask the registry for.
        return Ok(checkpoint);
    }
    // Strictly larger — a smaller size was refused above as a ROLLBACK.
    let proof = get_json::<ConsistencyProofResponse>(
        repo_url,
        &format!("/log/consistency?from={pinned_size}&to={}", checkpoint.size),
    )?;
    let old_root = decode_hex32(&pinned_root, "pinned root")?;
    let new_root = decode_hex32(&checkpoint.root_hash, "rootHash")?;
    let mut path = Vec::new();
    for node in &proof.path {
        path.push(decode_hex32(node, "proof node")?);
    }
    crate::log::verify_consistency(
        pinned_size as usize,
        checkpoint.size as usize,
        &old_root,
        &new_root,
        &path,
    )?;
    // Proven an extension of the pinned history — only now advance the pin.
    local::write_checkpoint(paths, checkpoint.size, &checkpoint.root_hash)?;
    Ok(checkpoint)
}

/// The canonical `publish` log payload, byte-for-byte as the registry writes it
/// in `store.rs` (`{"ident":…,"version":…,"hash":…}`, `serde_json`-escaped).
///
/// The client reconstructs this from values it already knows so the leaf can be
/// bound to a specific package; see `verify_publish_inclusion`. It must track
/// `store.rs`'s encoding exactly — a drift in field order or escaping turns every
/// honest inclusion check into a failure.
pub(super) fn publish_leaf_payload(ident: &str, version: &str, content_hash: &str) -> String {
    let encode = |value: &str| serde_json::to_string(value).expect("JSON string encoding is total");
    format!(
        "{{\"ident\":{},\"version\":{},\"hash\":{}}}",
        encode(ident),
        encode(version),
        encode(content_hash),
    )
}

/// Verify that the publish of `ident@version` with content hash
/// `expected_content_hash` is included in the registry log under the current
/// (verified, rollback-checked) checkpoint. Returns the log entry and the
/// checkpoint it verified against.
pub fn verify_publish_inclusion(
    repo_url: &str,
    paths: &LocalPaths,
    ident: &str,
    version: &str,
    expected_content_hash: &str,
) -> Result<(LogEntry, CheckpointResponse), String> {
    let checkpoint = fetch_checkpoint(repo_url, paths)?;
    let entry = get_json::<LogEntry>(
        repo_url,
        &format!(
            "/log/publish?ident={}&version={}",
            percent_encode(ident),
            percent_encode(version)
        ),
    )?;
    let proof = get_json::<InclusionProofResponse>(
        repo_url,
        &format!("/log/proof/{}?size={}", entry.index, checkpoint.size),
    )?;
    if proof.index != entry.index || proof.size != checkpoint.size {
        return Err("inclusion proof does not match the requested entry".to_string());
    }
    let leaf = decode_hex32(&entry.leaf_hash, "leafHash")?;
    if decode_hex32(&proof.leaf_hash, "leafHash")? != leaf {
        return Err("inclusion proof leaf does not match the publish entry".to_string());
    }
    // Bind the leaf to *this* package (bug-273). Proving the served leaf is in the
    // tree says nothing about which entry it is: the client knows ident, version
    // and content hash but previously verified none of them, so a hostile registry
    // could answer with the (index, leafHash) of any genuine entry — its own
    // `register`, an unrelated publish — and the RFC-6962 math would check out,
    // reporting "inclusion verified" for a publish that never happened.
    //
    // The payload is rebuilt from values the client independently holds; hashing a
    // server-supplied payload instead would leave the registry free to choose what
    // the leaf describes.
    let expected_leaf = crate::log::leaf_hash(
        publish_leaf_payload(ident, version, expected_content_hash).as_bytes(),
    );
    if expected_leaf != leaf {
        return Err(format!(
            "publish log leaf does not match {ident}@{version}: the registry served \
             an entry for different contents"
        ));
    }
    let root = decode_hex32(&checkpoint.root_hash, "rootHash")?;
    let mut path = Vec::new();
    for node in &proof.path {
        path.push(decode_hex32(node, "proof node")?);
    }
    crate::log::verify_inclusion(
        entry.index as usize,
        checkpoint.size as usize,
        &leaf,
        &path,
        &root,
    )?;
    Ok((entry, checkpoint))
}

/// Fetch and verify a consistency proof between the pinned checkpoint and
/// the current one.
///
/// Since bug-582 this is exactly `fetch_checkpoint`: the consistency gate lives
/// in the one pin-advance primitive rather than in one of its two callers, so a
/// call site cannot pick the unsafe half by accident. The name is kept because
/// it says at the call site *why* the log is being contacted.
pub fn verify_log_consistency(
    repo_url: &str,
    paths: &LocalPaths,
) -> Result<CheckpointResponse, String> {
    fetch_checkpoint(repo_url, paths)
}

pub(super) fn decode_hex32(value: &str, field: &str) -> Result<[u8; 32], String> {
    let raw = hex::decode(value).map_err(|_| format!("malformed {field}"))?;
    if raw.len() != 32 {
        return Err(format!("malformed {field}"));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
}

/// The root-delegated keys and index state recovered from a verified metadata
/// chain (plan-10-C2).
#[derive(Debug)]
pub struct DelegatedMetadata {
    /// The root-delegated server (attestation) key. A consumer refuses any
    /// attestation not signed by this exact key.
    pub server_key: Vec<u8>,
    /// The verified `root.json` version (bug-584). Pinned locally and never
    /// allowed to go backwards, so a renewal cannot be rolled back.
    pub root_version: i64,
    pub snapshot_version: i64,
    pub index_hash: String,
}

/// Verify the signed-metadata chain (plan-10-C2): root → timestamp → snapshot.
/// Rejects a bad root fingerprint, a registry-id mismatch, expired metadata,
/// an undelegated key, a snapshot/timestamp that disagree, and a version
/// rollback below `min_root_version` or `min_snapshot_version`. `now` is passed
/// in for testability.
///
/// The root version floor is bug-584: root renewal rotates the delegated online
/// keys under the same anchor, so an old-but-unexpired `root.json` — correctly
/// signed by the very key the client pinned — would otherwise resurrect the
/// snapshot/timestamp keys a renewal retired.
pub fn verify_registry_metadata(
    root: &RootResponse,
    timestamp: &SignedMetadataResponse,
    snapshot: &SignedMetadataResponse,
    expected_registry_id: &str,
    pinned_root_fingerprint: &str,
    min_root_version: i64,
    min_snapshot_version: i64,
    now: i64,
) -> Result<DelegatedMetadata, String> {
    // Root: the pinned fingerprint is the sole trust anchor.
    let root_key = crypto::decode_bytes(&root.root_key, "rootKey")?;
    if crypto::fingerprint(&root_key) != pinned_root_fingerprint {
        return Err("registry root key does not match the pinned root fingerprint".to_string());
    }
    let root_signature = crypto::decode_bytes(&root.signature, "root signature")?;
    crypto::verify(
        &root_key,
        &crypto::root_signing_input(root.signed.as_bytes()),
        &root_signature,
    )
    .map_err(|_| "root.json signature does not verify under the root key".to_string())?;
    let root_doc = parse_metadata(&root.signed, "root.json")?;
    check_field(&root_doc, "registryId", expected_registry_id, "root.json")?;
    check_not_expired(&root_doc, now, "root.json")?;
    let root_version = metadata_i64(&root_doc, "version", "root.json")?;
    if root_version < min_root_version {
        return Err(format!(
            "metadata ROLLBACK: root version {root_version} is below the pinned version {min_root_version}"
        ));
    }
    let server_key = decode_delegated_key(&root_doc, "serverKey")?;
    let snapshot_key = decode_delegated_key(&root_doc, "snapshotKey")?;
    let timestamp_key = decode_delegated_key(&root_doc, "timestampKey")?;

    // Timestamp: signed by the delegated timestamp key, fresh, no rollback.
    let timestamp_signature = crypto::decode_bytes(&timestamp.signature, "timestamp signature")?;
    crypto::verify(
        &timestamp_key,
        &crypto::timestamp_signing_input(timestamp.signed.as_bytes()),
        &timestamp_signature,
    )
    .map_err(|_| "timestamp.json signature does not verify under the delegated key".to_string())?;
    let timestamp_doc = parse_metadata(&timestamp.signed, "timestamp.json")?;
    check_field(
        &timestamp_doc,
        "registryId",
        expected_registry_id,
        "timestamp.json",
    )?;
    check_not_expired(&timestamp_doc, now, "timestamp.json")?;
    let snapshot_version = metadata_i64(&timestamp_doc, "snapshotVersion", "timestamp.json")?;
    if snapshot_version < min_snapshot_version {
        return Err(format!(
            "metadata ROLLBACK: snapshot version {snapshot_version} is below the pinned version {min_snapshot_version}"
        ));
    }
    let timestamp_index_hash = metadata_str(&timestamp_doc, "indexHash", "timestamp.json")?;

    // Snapshot: signed by the delegated snapshot key, fresh, and agrees with
    // the timestamp on version + index hash.
    let snapshot_signature = crypto::decode_bytes(&snapshot.signature, "snapshot signature")?;
    crypto::verify(
        &snapshot_key,
        &crypto::snapshot_signing_input(snapshot.signed.as_bytes()),
        &snapshot_signature,
    )
    .map_err(|_| "snapshot.json signature does not verify under the delegated key".to_string())?;
    let snapshot_doc = parse_metadata(&snapshot.signed, "snapshot.json")?;
    check_field(
        &snapshot_doc,
        "registryId",
        expected_registry_id,
        "snapshot.json",
    )?;
    check_not_expired(&snapshot_doc, now, "snapshot.json")?;
    let snapshot_doc_version = metadata_i64(&snapshot_doc, "version", "snapshot.json")?;
    let snapshot_index_hash = metadata_str(&snapshot_doc, "indexHash", "snapshot.json")?;
    if snapshot_doc_version != snapshot_version {
        return Err("timestamp and snapshot disagree on the snapshot version".to_string());
    }
    if snapshot_index_hash != timestamp_index_hash {
        return Err("timestamp and snapshot disagree on the index hash".to_string());
    }

    Ok(DelegatedMetadata {
        server_key,
        root_version,
        snapshot_version,
        index_hash: snapshot_index_hash,
    })
}

pub(super) fn parse_metadata(signed: &str, what: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(signed).map_err(|_| format!("malformed {what}"))
}

pub(super) fn check_field(
    doc: &serde_json::Value,
    field: &str,
    expected: &str,
    what: &str,
) -> Result<(), String> {
    if doc.get(field).and_then(|value| value.as_str()) != Some(expected) {
        return Err(format!("{what} {field} does not match the expected value"));
    }
    Ok(())
}

pub(super) fn check_not_expired(
    doc: &serde_json::Value,
    now: i64,
    what: &str,
) -> Result<(), String> {
    let expires = doc
        .get("expires")
        .and_then(|value| value.as_i64())
        .ok_or_else(|| format!("{what} is missing an expiry"))?;
    if expires <= now {
        return Err(format!("{what} has expired"));
    }
    Ok(())
}

pub(super) fn metadata_i64(
    doc: &serde_json::Value,
    field: &str,
    what: &str,
) -> Result<i64, String> {
    doc.get(field)
        .and_then(|value| value.as_i64())
        .ok_or_else(|| format!("{what} is missing {field}"))
}

pub(super) fn metadata_str(
    doc: &serde_json::Value,
    field: &str,
    what: &str,
) -> Result<String, String> {
    doc.get(field)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{what} is missing {field}"))
}

pub(super) fn decode_delegated_key(
    doc: &serde_json::Value,
    field: &str,
) -> Result<Vec<u8>, String> {
    let encoded = doc
        .get(field)
        .and_then(|value| value.as_str())
        .ok_or_else(|| format!("root.json is missing {field}"))?;
    crypto::decode_bytes(encoded, field)
}

/// `mfb repo trust`: fetch and verify the metadata chain, cross-check that the
/// pinned server key is the root-delegated one, and pin the registry id + root
/// fingerprint. Returns the pinned snapshot version.
pub fn trust_registry(
    repo_url: &str,
    paths: &LocalPaths,
    registry_id: &str,
    root_fingerprint: &str,
) -> Result<i64, String> {
    let server_key = ensure_server_key(repo_url, paths)?;
    let delegated = fetch_and_verify_metadata(repo_url, registry_id, root_fingerprint, 0, 0)?;
    if delegated.server_key != server_key {
        return Err(
            "registry attestation key is not delegated by the pinned root; refusing to trust"
                .to_string(),
        );
    }
    local::write_root_pin(paths, registry_id, root_fingerprint)?;
    local::write_root_version(paths, delegated.root_version)?;
    local::write_snapshot_version(paths, delegated.snapshot_version)?;
    Ok(delegated.snapshot_version)
}

/// Verify the pinned metadata chain before trusting the registry, when (and
/// only when) a root has been pinned via `mfb repo trust` — the signed-metadata
/// layer is opt-in on top of the plan-23 pinned-server-key anchor. Advances the
/// pinned snapshot version on success; rejects a rollback.
pub fn verify_pinned_metadata(repo_url: &str, paths: &LocalPaths) -> Result<(), String> {
    let Some((registry_id, root_fingerprint)) = local::read_root_pin(paths)? else {
        return Ok(());
    };
    let server_key = ensure_server_key(repo_url, paths)?;
    let min_root = local::read_root_version(paths)?.unwrap_or(0);
    let min = local::read_snapshot_version(paths)?.unwrap_or(0);
    let delegated =
        fetch_and_verify_metadata(repo_url, &registry_id, &root_fingerprint, min_root, min)?;
    if delegated.server_key != server_key {
        return Err(
            "registry attestation key is not delegated by the pinned root; refusing to trust"
                .to_string(),
        );
    }
    // A root renewal advances this; a client pinned before the renewal follows
    // it, and the retired root can never be replayed back over it (bug-584).
    local::write_root_version(paths, delegated.root_version)?;
    local::write_snapshot_version(paths, delegated.snapshot_version)?;
    Ok(())
}

fn fetch_and_verify_metadata(
    repo_url: &str,
    registry_id: &str,
    root_fingerprint: &str,
    min_root_version: i64,
    min_snapshot_version: i64,
) -> Result<DelegatedMetadata, String> {
    let root = get_json::<RootResponse>(repo_url, "/root.json")?;
    let timestamp = get_json::<SignedMetadataResponse>(repo_url, "/timestamp.json")?;
    let snapshot = get_json::<SignedMetadataResponse>(repo_url, "/snapshot.json")?;
    verify_registry_metadata(
        &root,
        &timestamp,
        &snapshot,
        registry_id,
        root_fingerprint,
        min_root_version,
        min_snapshot_version,
        crate::store::now_unix(),
    )
}
