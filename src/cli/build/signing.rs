use super::*;

pub(crate) struct BuildSigningInfo {
    pub(crate) owner: String,
    /// The signed package identity (`<owner>#<package>`), stamped into the
    /// header when the manifest declares no ident of its own.
    pub(crate) ident: String,
    pub(crate) ident_fingerprint: String,
    pub(crate) signing_fingerprint: String,
    /// The full signing bundle threaded to the package writer: ident key,
    /// one-off signing keypair, ident-signed proof, server-signed
    /// attestation. The one-off private key exists only here, in memory,
    /// and is discarded when the build ends (plan-23 §3.3).
    pub(crate) package_signing: target::package_mfp::PackageSigning,
    pub(crate) executable_metadata: Vec<u8>,
}

/// The identity a `--sign` build signs for: the manifest ident when declared
/// (which must belong to the signing owner), else `<owner>#<name>`.
pub(super) fn signing_ident(
    owner: &str,
    name: &str,
    manifest_ident: &str,
) -> Result<String, String> {
    if manifest_ident.is_empty() {
        return Ok(format!("{owner}#{name}"));
    }
    let Some((ident_owner, _)) = manifest_ident.split_once('#') else {
        return Err(format!(
            "project ident `{manifest_ident}` must use <owner>#<package> to be signed"
        ));
    };
    if !ident_owner.eq_ignore_ascii_case(owner) {
        return Err(format!(
            "project ident `{manifest_ident}` does not belong to owner `{owner}`"
        ));
    }
    Ok(manifest_ident.to_string())
}

/// Assemble the plan-23 §3.3 signing bundle: generate the one-off signing
/// keypair, fetch the server attestation pre-registering it for this exact
/// package+version, and mint the ident-signed proof locally.
// coverage:off — reaches a live registry (request_attestation) and requires a
// registered ident key on the machine; exercised end-to-end by the tests/
// package-publish integration harness, not a unit test.
pub(super) fn load_build_signing_info(
    owner: &str,
    ident: &str,
    version: &str,
) -> Result<BuildSigningInfo, String> {
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::super::local_paths_for_repo(&repo_url)?;

    // The account ident key must live on this machine (register or link).
    let ident_private = mfb_repository::local::read_ident_private_key(&paths, owner)?;
    let ident_public = mfb_repository::local::read_ident_public_key(&paths, owner)?;
    if mfb_repository::crypto::public_from_private(&ident_private)? != ident_public {
        return Err("local ident key files do not match each other".to_string());
    }
    let ident_fingerprint = mfb_repository::crypto::fingerprint(&ident_public);

    // One-off signing keypair: fresh for this build, discarded with it.
    let (signing_public, signing_private) = mfb_repository::crypto::generate_keypair();
    let signing_fingerprint = mfb_repository::crypto::fingerprint(&signing_public);

    // Fetch the attestation (verified against the pinned server key inside
    // the client) and cross-check that the server's current name↔ident
    // binding is the ident key this machine holds.
    let attestation_response = mfb_repository::client::request_attestation(
        &repo_url,
        &paths,
        owner,
        ident,
        version,
        &signing_fingerprint,
    )?;
    let attestation_fields: tinyjson::JsonValue = attestation_response
        .attestation
        .parse()
        .map_err(|_| "repository returned a malformed attestation".to_string())?;
    let attestation_field =
        |field: &str| -> Option<String> { attestation_fields[field].get::<String>().cloned() };
    if attestation_field("identFingerprint").as_deref() != Some(ident_fingerprint.as_str()) {
        return Err(
            "repository attestation names a different ident key than this machine holds; \
             re-link this machine or rotate the ident"
                .to_string(),
        );
    }
    if attestation_field("ident").as_deref() != Some(ident)
        || attestation_field("version").as_deref() != Some(version)
        || attestation_field("signingFingerprint").as_deref() != Some(signing_fingerprint.as_str())
    {
        return Err("repository attestation does not pin the requested package".to_string());
    }
    let attestation_sig = mfb_repository::crypto::decode_bytes(
        &attestation_response.attestation_signature,
        "attestationSignature",
    )?;

    // Mint the proof (plan-23 §5) and sign it with the ident key.
    let issued = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let proof = format!(
        "{{\"owner\":{},\"ident\":{},\"version\":{},\"identFingerprint\":{},\"signingFingerprint\":{},\"issued\":{}}}",
        json_string(owner),
        json_string(ident),
        json_string(version),
        json_string(&ident_fingerprint),
        json_string(&signing_fingerprint),
        issued,
    );
    let proof_sig = mfb_repository::crypto::sign(
        &ident_private,
        &mfb_repository::crypto::proof_signing_input(proof.as_bytes()),
    )?;

    let ident_key = format!(
        "ed25519:{}",
        mfb_repository::crypto::encode_bytes(&ident_public)
    );
    let signing_key = format!(
        "ed25519:{}",
        mfb_repository::crypto::encode_bytes(&signing_public)
    );
    let executable_metadata = executable_signing_metadata_json(
        owner,
        &ident_key,
        &ident_fingerprint,
        &signing_key,
        &signing_fingerprint,
        &proof,
        &mfb_repository::crypto::encode_bytes(&proof_sig),
        &attestation_response.attestation,
        &attestation_response.attestation_signature,
    )
    .into_bytes();

    Ok(BuildSigningInfo {
        owner: owner.to_string(),
        ident: ident.to_string(),
        ident_fingerprint,
        signing_fingerprint,
        package_signing: target::package_mfp::PackageSigning {
            ident_key,
            signing_key,
            signing_private,
            proof,
            proof_sig,
            attestation: attestation_response.attestation,
            attestation_sig,
        },
        executable_metadata,
    })
}

pub(crate) fn apply_signing_metadata(
    metadata: &mut binary_repr::BinaryReprMetadata,
    signing: &BuildSigningInfo,
) {
    // The embedded manifest repeats the header identity (plan-23 §4): the
    // full ident key plus the fingerprints of the header's identKey and
    // signingKey. The signed ident is stamped too, so a manifest without an
    // ident of its own still matches the header's `<owner>#<name>`.
    metadata.ident = signing.ident.clone();
    metadata.ident_key = signing.package_signing.ident_key.clone();
    metadata.ident_fingerprint = signing.ident_fingerprint.clone();
    metadata.signing_fingerprint = signing.signing_fingerprint.clone();
    metadata.author = signing.owner.clone();
}

#[allow(clippy::too_many_arguments)]
pub(super) fn executable_signing_metadata_json(
    owner: &str,
    ident_key: &str,
    ident_fingerprint: &str,
    signing_key: &str,
    signing_fingerprint: &str,
    proof: &str,
    proof_sig: &str,
    attestation: &str,
    attestation_sig: &str,
) -> String {
    format!(
        "{{\"format\":\"mfb-signing-v1\",\"owner\":{},\"author\":{},\"identKey\":{},\"identFingerprint\":{},\"signingKey\":{},\"signingFingerprint\":{},\"proof\":{},\"proofSignature\":{},\"attestation\":{},\"attestationSignature\":{},\"signatureType\":\"Ed25519\"}}\n",
        json_string(owner),
        json_string(owner),
        json_string(ident_key),
        json_string(ident_fingerprint),
        json_string(signing_key),
        json_string(signing_fingerprint),
        json_string(proof),
        json_string(proof_sig),
        json_string(attestation),
        json_string(attestation_sig),
    )
}
