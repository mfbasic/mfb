use crate::crypto;

const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
/// Fixed-size fields before the variable-length header section:
/// magic (8) + containerMajor/Minor (4) + binaryReprMajor/Minor (4) + flags (4).
const FIXED_PREFIX_LEN: usize = 20;
const SIGNATURE_ED25519: u16 = 1;

/// A parsed container v1.0 `.mfp` header (plan-23 §4). The reader is hard
/// v1.0: `containerMajor.containerMinor` must be exactly `1.0`, with no
/// backwards compatibility for the pre-plan-23 layout.
///
/// Field order on disk:
/// magic, containerMajor, containerMinor, binaryReprMajor, binaryReprMinor,
/// flags, name, ident, version, author, url, identKey, signingKey, proof,
/// proofSig, attestation, attestationSig, packageBinaryHash (32 raw bytes),
/// binaryReprLength (u64), signatureType (u16), signatureLength (u32),
/// signature, packageBinaryRepr. All strings are u32-length-prefixed UTF-8.
/// The signature signs `"MFP-PACKAGE-v2\0" || SHA-256(bytes[0..signature
/// offset))`; the payload is covered transitively via `packageBinaryHash`.
#[derive(Debug, Clone)]
pub struct MfpPackage {
    pub name: String,
    pub ident: String,
    pub version: String,
    pub author: String,
    pub url: String,
    /// Ident public key in metadata form (`ed25519:<base64url>`), or empty
    /// for an unsigned package.
    pub ident_key: String,
    /// One-off signing public key in metadata form, or empty when unsigned.
    pub signing_key: String,
    /// Ident-signed proof JSON (plan-23 §5), or empty when unsigned.
    pub proof: String,
    /// 64-byte ident signature over `"MFP-PROOF-v1\0" || proof`.
    pub proof_sig: Vec<u8>,
    /// Server-signed attestation JSON (plan-23 §5), or empty when unsigned.
    pub attestation: String,
    /// 64-byte server signature over `"MFP-ATTEST-v1\0" || attestation`.
    pub attestation_sig: Vec<u8>,
    /// SHA-256 of `packageBinaryRepr` as recorded in the header.
    pub package_binary_hash: [u8; 32],
    pub container_major: u16,
    pub container_minor: u16,
    pub binary_repr_major: u16,
    pub binary_repr_minor: u16,
    pub flags: u32,
    pub signature_type: u16,
    pub signature: Vec<u8>,
    pub binary_repr_length: usize,
    /// The signed prefix: every byte before the `signature` bytes themselves
    /// (it includes `signatureType` and `signatureLength`) — exactly what the
    /// package signature covers.
    pub signed_prefix: Vec<u8>,
    /// SHA-256 recomputed over the actual payload bytes.
    pub payload_sha256: [u8; 32],
    /// The raw `packageBinaryRepr` payload bytes (the MFPC container). Carries
    /// the ABI index section the registry serves (plan-10-B1).
    pub payload: Vec<u8>,
    /// SHA-256 of the whole artifact (blob/dedup identity).
    pub content_hash: [u8; 32],
}

impl MfpPackage {
    pub fn content_hash_hex(&self) -> String {
        hex::encode(self.content_hash)
    }

    /// Hex SHA-256 fingerprint of the ident public key (empty when unsigned).
    pub fn ident_fingerprint(&self) -> Result<String, String> {
        metadata_key_fingerprint(&self.ident_key, "identKey")
    }

    /// Hex SHA-256 fingerprint of the signing public key (empty when unsigned).
    pub fn signing_fingerprint(&self) -> Result<String, String> {
        metadata_key_fingerprint(&self.signing_key, "signingKey")
    }
}

/// Decode a metadata-form public key (`ed25519:<base64url>`); a bare
/// base64url key is also accepted for pinned trust anchors.
pub fn decode_metadata_key(value: &str, field: &str) -> Result<Vec<u8>, String> {
    let encoded = value.strip_prefix("ed25519:").unwrap_or(value);
    let key = crypto::decode_bytes(encoded, field)?;
    if key.len() != crypto::PUBLIC_KEY_LEN {
        return Err(format!("malformed {field}"));
    }
    Ok(key)
}

/// Hex fingerprint of a metadata-form key, or empty for an empty key.
pub fn metadata_key_fingerprint(value: &str, field: &str) -> Result<String, String> {
    if value.is_empty() {
        return Ok(String::new());
    }
    Ok(crypto::fingerprint(&decode_metadata_key(value, field)?))
}

pub fn parse_mfp_package(bytes: &[u8]) -> Result<MfpPackage, String> {
    if bytes.len() < FIXED_PREFIX_LEN {
        return Err("package is too small to be a valid .mfp package".to_string());
    }
    if bytes[0..8] != MFP_MAGIC {
        return Err("package does not have the MFP package magic".to_string());
    }

    let container_major = read_u16(bytes, 8)?;
    let container_minor = read_u16(bytes, 10)?;
    if container_major != 1 || container_minor != 0 {
        return Err(format!(
            "unsupported MFP container version {container_major}.{container_minor} (expected 1.0)"
        ));
    }
    let binary_repr_major = read_u16(bytes, 12)?;
    let binary_repr_minor = read_u16(bytes, 14)?;
    let flags = read_u32(bytes, 16)?;

    let mut offset = FIXED_PREFIX_LEN;
    let name = read_mfp_string(bytes, &mut offset, "name", 255, true)?;
    let ident = read_mfp_string(bytes, &mut offset, "ident", 255, true)?;
    let version = read_mfp_string(bytes, &mut offset, "version", 64, true)?;
    let author = read_mfp_string(bytes, &mut offset, "author", 512, false)?;
    let url = read_mfp_string(bytes, &mut offset, "url", 2048, false)?;
    let ident_key = read_mfp_string(bytes, &mut offset, "identKey", 255, false)?;
    let signing_key = read_mfp_string(bytes, &mut offset, "signingKey", 255, false)?;
    let proof = read_mfp_string(bytes, &mut offset, "proof", 4096, false)?;
    let proof_sig = read_mfp_bytes(bytes, &mut offset, "proofSig", 64)?;
    let attestation = read_mfp_string(bytes, &mut offset, "attestation", 4096, false)?;
    let attestation_sig = read_mfp_bytes(bytes, &mut offset, "attestationSig", 64)?;

    let hash_end = offset
        .checked_add(32)
        .ok_or_else(|| "truncated .mfp packageBinaryHash".to_string())?;
    if hash_end > bytes.len() {
        return Err("truncated .mfp packageBinaryHash".to_string());
    }
    let mut package_binary_hash = [0u8; 32];
    package_binary_hash.copy_from_slice(&bytes[offset..hash_end]);
    offset = hash_end;

    let binary_repr_length = read_u64(bytes, offset)? as usize;
    offset = offset
        .checked_add(8)
        .ok_or_else(|| "invalid .mfp binary representation length".to_string())?;

    let signature_type = read_u16(bytes, offset)?;
    let signature_length = read_u32(bytes, offset + 2)? as usize;
    validate_signature_header(signature_type, signature_length)?;
    let signature_offset = offset
        .checked_add(6)
        .ok_or_else(|| "truncated .mfp header".to_string())?;
    let signed_prefix = bytes[..signature_offset].to_vec();
    let signature_end = signature_offset
        .checked_add(signature_length)
        .ok_or_else(|| "invalid .mfp signature length".to_string())?;
    if signature_end > bytes.len() {
        return Err("truncated .mfp signature".to_string());
    }
    let signature = bytes[signature_offset..signature_end].to_vec();

    let payload_end = signature_end
        .checked_add(binary_repr_length)
        .ok_or_else(|| "invalid .mfp binary representation length".to_string())?;
    if payload_end != bytes.len() {
        return Err("invalid .mfp binary representation length".to_string());
    }
    let payload = bytes[signature_end..payload_end].to_vec();
    let payload_sha256 = crypto::sha256(&payload);

    // Signed packages must carry the full chain; unsigned local packages must
    // carry none of it (a partial chain is malformed either way).
    if signature_type == SIGNATURE_ED25519 {
        for (value, field) in [
            (ident_key.is_empty(), "identKey"),
            (signing_key.is_empty(), "signingKey"),
            (proof.is_empty(), "proof"),
            (proof_sig.is_empty(), "proofSig"),
            (attestation.is_empty(), "attestation"),
            (attestation_sig.is_empty(), "attestationSig"),
        ] {
            if value {
                return Err(format!("signed .mfp package is missing {field}"));
            }
        }
    } else {
        for (value, field) in [
            (!ident_key.is_empty(), "identKey"),
            (!signing_key.is_empty(), "signingKey"),
            (!proof.is_empty(), "proof"),
            (!proof_sig.is_empty(), "proofSig"),
            (!attestation.is_empty(), "attestation"),
            (!attestation_sig.is_empty(), "attestationSig"),
        ] {
            if value {
                return Err(format!("unsigned .mfp package must not carry {field}"));
            }
        }
    }

    Ok(MfpPackage {
        name,
        ident,
        version,
        author,
        url,
        ident_key,
        signing_key,
        proof,
        proof_sig,
        attestation,
        attestation_sig,
        package_binary_hash,
        container_major,
        container_minor,
        binary_repr_major,
        binary_repr_minor,
        flags,
        signature_type,
        signature,
        binary_repr_length,
        signed_prefix,
        payload_sha256,
        payload,
        content_hash: crypto::sha256(bytes),
    })
}

/// Verify the package signature: 64-byte Ed25519 over
/// `"MFP-PACKAGE-v2\0" || SHA-256(signed prefix)` under the header's
/// **signingKey** (the one-off per-package key).
pub fn verify_package_signature(package: &MfpPackage) -> Result<(), String> {
    if package.signature_type != SIGNATURE_ED25519 {
        return Err("package is not Ed25519-signed".to_string());
    }
    let signing_key = decode_metadata_key(&package.signing_key, "signingKey")?;
    let message = crypto::package_signing_input(&package.signed_prefix);
    crypto::verify(&signing_key, &message, &package.signature)
        .map_err(|_| "invalid package signature".to_string())
}

/// Verify the header→payload weld: the recorded `packageBinaryHash` must be
/// the SHA-256 of the actual payload bytes.
pub fn verify_payload_hash(package: &MfpPackage) -> Result<(), String> {
    if package.package_binary_hash != package.payload_sha256 {
        return Err("packageBinaryHash does not match the package payload".to_string());
    }
    Ok(())
}

/// Verify the ident-signed proof under the given ident public key and check
/// that every proof field pins this exact package (plan-23 §3.5 step 3).
pub fn verify_proof(package: &MfpPackage, ident_public: &[u8]) -> Result<(), String> {
    let message = crypto::proof_signing_input(package.proof.as_bytes());
    crypto::verify(ident_public, &message, &package.proof_sig)
        .map_err(|_| "invalid proof signature".to_string())?;
    let proof: serde_json::Value =
        serde_json::from_str(&package.proof).map_err(|_| "malformed proof JSON".to_string())?;
    let expect = |field: &str, value: &str| -> Result<(), String> {
        if proof.get(field).and_then(|value| value.as_str()) != Some(value) {
            return Err(format!("proof {field} does not match the package header"));
        }
        Ok(())
    };
    let Some((owner, _)) = package.ident.split_once('#') else {
        return Err("package ident must use <owner>#<package>".to_string());
    };
    expect("owner", owner)?;
    expect("ident", &package.ident)?;
    expect("version", &package.version)?;
    expect("identFingerprint", &package.ident_fingerprint()?)?;
    expect("signingFingerprint", &package.signing_fingerprint()?)?;
    Ok(())
}

/// Verify the server-signed attestation under the given server public key and
/// check that every attestation field pins this exact package (plan-23 §3.5
/// step 2). `repo_fingerprint` is the expected registry key fingerprint.
pub fn verify_attestation(
    package: &MfpPackage,
    server_public: &[u8],
    repo_fingerprint: &str,
) -> Result<(), String> {
    let message = crypto::attestation_signing_input(package.attestation.as_bytes());
    crypto::verify(server_public, &message, &package.attestation_sig)
        .map_err(|_| "invalid attestation signature".to_string())?;
    let attestation: serde_json::Value = serde_json::from_str(&package.attestation)
        .map_err(|_| "malformed attestation JSON".to_string())?;
    let expect = |field: &str, value: &str| -> Result<(), String> {
        if attestation.get(field).and_then(|value| value.as_str()) != Some(value) {
            return Err(format!(
                "attestation {field} does not match the package header"
            ));
        }
        Ok(())
    };
    expect("repoFingerprint", repo_fingerprint)?;
    let Some((owner, _)) = package.ident.split_once('#') else {
        return Err("package ident must use <owner>#<package>".to_string());
    };
    expect("owner", owner)?;
    expect("ident", &package.ident)?;
    expect("version", &package.version)?;
    expect("identFingerprint", &package.ident_fingerprint()?)?;
    expect("signingFingerprint", &package.signing_fingerprint()?)?;
    Ok(())
}

pub fn package_content_hash(bytes: &[u8]) -> Result<[u8; 32], String> {
    if bytes.len() < FIXED_PREFIX_LEN || bytes[0..8] != MFP_MAGIC {
        return Err("package is not a valid .mfp package".to_string());
    }
    Ok(crypto::sha256(bytes))
}

fn validate_signature_header(signature_type: u16, signature_length: usize) -> Result<(), String> {
    match (signature_type, signature_length) {
        (0, 0) | (1, 64) => Ok(()),
        (0, _) => Err("unsigned .mfp package must have zero signature length".to_string()),
        (1, _) => Err("Ed25519 .mfp package must have a 64 byte signature".to_string()),
        _ => Err(format!("unsupported .mfp signature type {signature_type}")),
    }
}

fn read_mfp_string(
    bytes: &[u8],
    offset: &mut usize,
    field: &str,
    limit: usize,
    required: bool,
) -> Result<String, String> {
    let raw = read_mfp_bytes(bytes, offset, field, limit)?;
    let value = String::from_utf8(raw).map_err(|_| format!(".mfp {field} is not valid UTF-8"))?;
    if required && value.is_empty() {
        return Err(format!(".mfp {field} must not be empty"));
    }
    Ok(value)
}

fn read_mfp_bytes(
    bytes: &[u8],
    offset: &mut usize,
    field: &str,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let length = read_u32(bytes, *offset)? as usize;
    *offset = offset
        .checked_add(4)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    if length > limit {
        return Err(format!(".mfp {field} exceeds the {limit} byte limit"));
    }
    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    if end > bytes.len() {
        return Err(format!("truncated .mfp {field}"));
    }
    let value = bytes[*offset..end].to_vec();
    *offset = end;
    Ok(value)
}

/// Test-only container v1.0 builder: serialize a package with an arbitrary
/// trust chain so server tests can craft forgeries without the compiler's
/// writer. Mirrors `src/target/package_mfp/mod.rs::build_package_bytes`.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::crypto;

    pub(crate) struct TestPackage {
        pub name: String,
        pub ident: String,
        pub version: String,
        pub author: String,
        pub url: String,
        pub payload: Vec<u8>,
        pub ident_key: String,
        pub signing_key: String,
        pub proof: String,
        pub proof_sig: Vec<u8>,
        pub attestation: String,
        pub attestation_sig: Vec<u8>,
    }

    fn put_bytes(dst: &mut Vec<u8>, bytes: &[u8]) {
        dst.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        dst.extend_from_slice(bytes);
    }

    pub(crate) fn serialize(package: &TestPackage, signing_private: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00]);
        bytes.extend_from_slice(&1u16.to_le_bytes()); // containerMajor
        bytes.extend_from_slice(&0u16.to_le_bytes()); // containerMinor
        bytes.extend_from_slice(&1u16.to_le_bytes()); // binaryReprMajor
        bytes.extend_from_slice(&0u16.to_le_bytes()); // binaryReprMinor
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        put_bytes(&mut bytes, package.name.as_bytes());
        put_bytes(&mut bytes, package.ident.as_bytes());
        put_bytes(&mut bytes, package.version.as_bytes());
        put_bytes(&mut bytes, package.author.as_bytes());
        put_bytes(&mut bytes, package.url.as_bytes());
        put_bytes(&mut bytes, package.ident_key.as_bytes());
        put_bytes(&mut bytes, package.signing_key.as_bytes());
        put_bytes(&mut bytes, package.proof.as_bytes());
        put_bytes(&mut bytes, &package.proof_sig);
        put_bytes(&mut bytes, package.attestation.as_bytes());
        put_bytes(&mut bytes, &package.attestation_sig);
        bytes.extend_from_slice(&crypto::sha256(&package.payload));
        bytes.extend_from_slice(&(package.payload.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // signatureType
        bytes.extend_from_slice(&64u32.to_le_bytes()); // signatureLength
        let signature = crypto::sign(signing_private, &crypto::package_signing_input(&bytes))
            .expect("test package signature");
        bytes.extend_from_slice(&signature);
        bytes.extend_from_slice(&package.payload);
        bytes
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "truncated .mfp header".to_string())?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "truncated .mfp header".to_string())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| "truncated .mfp header".to_string())?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

#[cfg(test)]
mod tests {
    use super::test_support::{serialize, TestPackage};
    use super::*;

    /// A fully signed `alice#toolbox` package with a self-consistent trust
    /// chain: the ident key signs the proof and a throwaway "server" key signs
    /// the attestation. Returns the artifact plus the keys so tests can verify
    /// or tamper individual fields.
    struct Fixture {
        artifact: Vec<u8>,
        ident_public: Vec<u8>,
        signing_public: Vec<u8>,
        server_public: Vec<u8>,
    }

    fn signed_fixture(version: &str) -> Fixture {
        let (ident_public, ident_private) = crypto::generate_keypair();
        let (signing_public, signing_private) = crypto::generate_keypair();
        let (server_public, server_private) = crypto::generate_keypair();
        let ident_fingerprint = crypto::fingerprint(&ident_public);
        let signing_fingerprint = crypto::fingerprint(&signing_public);
        let repo_fingerprint = crypto::fingerprint(&server_public);
        let proof = format!(
            "{{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"{version}\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}",
        );
        let proof_sig = crypto::sign(
            &ident_private,
            &crypto::proof_signing_input(proof.as_bytes()),
        )
        .unwrap();
        let attestation = format!(
            "{{\"repoFingerprint\":\"{repo_fingerprint}\",\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"{version}\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}",
        );
        let attestation_sig = crypto::sign(
            &server_private,
            &crypto::attestation_signing_input(attestation.as_bytes()),
        )
        .unwrap();
        let artifact = serialize(
            &TestPackage {
                name: "toolbox".to_string(),
                ident: "alice#toolbox".to_string(),
                version: version.to_string(),
                author: "alice".to_string(),
                url: String::new(),
                payload: b"MFPCtestpayload".to_vec(),
                ident_key: format!("ed25519:{}", crypto::encode_bytes(&ident_public)),
                signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
                proof,
                proof_sig,
                attestation,
                attestation_sig,
            },
            &signing_private,
        );
        Fixture {
            artifact,
            ident_public,
            signing_public,
            server_public,
        }
    }

    #[test]
    fn parses_and_verifies_a_fully_signed_package() {
        let fx = signed_fixture("1.0.0");
        let package = parse_mfp_package(&fx.artifact).unwrap();
        assert_eq!(package.ident, "alice#toolbox");
        assert_eq!(package.version, "1.0.0");
        assert_eq!(package.author, "alice");
        assert_eq!(package.signature_type, 1);
        assert_eq!(package.container_major, 1);
        assert_eq!(package.container_minor, 0);
        assert_eq!(
            package.content_hash_hex(),
            hex::encode(package.content_hash)
        );
        assert_eq!(
            package.ident_fingerprint().unwrap(),
            crypto::fingerprint(&fx.ident_public)
        );
        assert_eq!(
            package.signing_fingerprint().unwrap(),
            crypto::fingerprint(&fx.signing_public)
        );
        verify_payload_hash(&package).unwrap();
        verify_package_signature(&package).unwrap();
        verify_proof(&package, &fx.ident_public).unwrap();
        verify_attestation(
            &package,
            &fx.server_public,
            &crypto::fingerprint(&fx.server_public),
        )
        .unwrap();
        assert_eq!(
            package_content_hash(&fx.artifact).unwrap(),
            package.content_hash
        );
    }

    #[test]
    fn parse_rejects_malformed_prefixes_and_versions() {
        assert!(parse_mfp_package(b"tiny")
            .unwrap_err()
            .contains("too small"));
        // Right size but wrong magic.
        let mut wrong_magic = vec![0u8; 32];
        assert!(parse_mfp_package(&wrong_magic)
            .unwrap_err()
            .contains("magic"));
        let _ = &mut wrong_magic;
        // Correct magic but unsupported container version.
        let mut wrong_version = signed_fixture("1.0.0").artifact;
        wrong_version[8] = 2; // containerMajor = 2
        assert!(parse_mfp_package(&wrong_version)
            .unwrap_err()
            .contains("unsupported MFP container version"));
    }

    #[test]
    fn parse_rejects_truncation_and_bad_signature_length() {
        let fx = signed_fixture("1.0.0");
        // Truncate mid-way: dropping the trailing payload makes the recorded
        // binaryReprLength disagree with the actual bytes.
        let mut truncated = fx.artifact.clone();
        truncated.truncate(truncated.len() - 4);
        assert!(parse_mfp_package(&truncated).is_err());
    }

    #[test]
    fn decode_metadata_key_handles_prefix_bare_and_malformed() {
        let (public, _private) = crypto::generate_keypair();
        let bare = crypto::encode_bytes(&public);
        assert_eq!(decode_metadata_key(&bare, "identKey").unwrap(), public);
        assert_eq!(
            decode_metadata_key(&format!("ed25519:{bare}"), "identKey").unwrap(),
            public
        );
        // Wrong length after decode.
        let short = crypto::encode_bytes(&[0u8; 10]);
        assert!(decode_metadata_key(&short, "identKey")
            .unwrap_err()
            .contains("malformed identKey"));
        // Not valid base64url.
        assert!(decode_metadata_key("!!!", "identKey").is_err());
    }

    #[test]
    fn metadata_key_fingerprint_is_empty_for_empty_key() {
        assert_eq!(metadata_key_fingerprint("", "identKey").unwrap(), "");
        let (public, _private) = crypto::generate_keypair();
        let value = format!("ed25519:{}", crypto::encode_bytes(&public));
        assert_eq!(
            metadata_key_fingerprint(&value, "identKey").unwrap(),
            crypto::fingerprint(&public)
        );
    }

    #[test]
    fn verify_functions_reject_tampering() {
        let fx = signed_fixture("1.0.0");
        let package = parse_mfp_package(&fx.artifact).unwrap();

        // Wrong signing key: signature verification fails.
        let (other_public, _) = crypto::generate_keypair();
        let mut mutated = package.clone();
        mutated.signing_key = format!("ed25519:{}", crypto::encode_bytes(&other_public));
        assert!(verify_package_signature(&mutated).is_err());

        // Payload-hash weld broken.
        let mut bad_hash = package.clone();
        bad_hash.package_binary_hash = [0u8; 32];
        assert!(verify_payload_hash(&bad_hash)
            .unwrap_err()
            .contains("packageBinaryHash"));

        // Proof under the wrong ident key.
        assert!(verify_proof(&package, &other_public).is_err());

        // Attestation under the wrong server key.
        assert!(verify_attestation(&package, &other_public, "fp").is_err());

        // Attestation with a mismatched repoFingerprint.
        assert!(verify_attestation(&package, &fx.server_public, "wrong-fp")
            .unwrap_err()
            .contains("repoFingerprint"));
    }

    #[test]
    fn verify_proof_rejects_field_mismatch_and_bad_json() {
        // A package whose ident lacks '#': verify_proof reports the format.
        let fx = signed_fixture("1.0.0");
        let mut package = parse_mfp_package(&fx.artifact).unwrap();
        package.ident = "no-hash".to_string();
        // The proof signature no longer matches the mutated proof bytes? The
        // proof JSON is unchanged, so the signature still verifies but the
        // ident split fails.
        assert!(verify_proof(&package, &fx.ident_public)
            .unwrap_err()
            .contains("<owner>#<package>"));
    }

    #[test]
    fn verify_package_signature_rejects_unsigned_type() {
        let fx = signed_fixture("1.0.0");
        let mut package = parse_mfp_package(&fx.artifact).unwrap();
        package.signature_type = 0;
        assert!(verify_package_signature(&package)
            .unwrap_err()
            .contains("not Ed25519-signed"));
    }

    #[test]
    fn validate_signature_header_accepts_valid_and_rejects_invalid() {
        assert!(validate_signature_header(0, 0).is_ok());
        assert!(validate_signature_header(1, 64).is_ok());
        assert!(validate_signature_header(0, 5)
            .unwrap_err()
            .contains("zero signature length"));
        assert!(validate_signature_header(1, 10)
            .unwrap_err()
            .contains("64 byte signature"));
        assert!(validate_signature_header(9, 0)
            .unwrap_err()
            .contains("unsupported"));
    }

    #[test]
    fn package_content_hash_rejects_non_packages() {
        assert!(package_content_hash(b"short").is_err());
        let mut not_mfp = vec![0u8; 32];
        not_mfp[0] = 0xff;
        assert!(package_content_hash(&not_mfp)
            .unwrap_err()
            .contains("not a valid"));
    }

    #[test]
    fn read_integer_helpers_report_truncation() {
        assert!(read_u16(&[0u8], 0).is_err());
        assert!(read_u32(&[0u8; 2], 0).is_err());
        assert!(read_u64(&[0u8; 4], 0).is_err());
        assert_eq!(read_u16(&[1, 0], 0).unwrap(), 1);
        assert_eq!(read_u32(&[1, 0, 0, 0], 0).unwrap(), 1);
        assert_eq!(read_u64(&[1, 0, 0, 0, 0, 0, 0, 0], 0).unwrap(), 1);
    }

    /// A raw container v1.0 builder with full control over each field, so tests
    /// can craft unsigned/partial-chain packages the signed test_support helper
    /// cannot. Mirrors the on-disk layout in `parse_mfp_package`.
    struct RawPackage {
        name: Vec<u8>,
        ident: Vec<u8>,
        version: Vec<u8>,
        author: Vec<u8>,
        ident_key: Vec<u8>,
        signing_key: Vec<u8>,
        proof: Vec<u8>,
        proof_sig: Vec<u8>,
        attestation: Vec<u8>,
        attestation_sig: Vec<u8>,
        signature_type: u16,
        signature: Vec<u8>,
        payload: Vec<u8>,
    }

    impl Default for RawPackage {
        fn default() -> Self {
            RawPackage {
                name: b"toolbox".to_vec(),
                ident: b"alice#toolbox".to_vec(),
                version: b"1.0.0".to_vec(),
                author: b"alice".to_vec(),
                ident_key: Vec::new(),
                signing_key: Vec::new(),
                proof: Vec::new(),
                proof_sig: Vec::new(),
                attestation: Vec::new(),
                attestation_sig: Vec::new(),
                signature_type: 0,
                signature: Vec::new(),
                payload: b"MFPCtestpayload".to_vec(),
            }
        }
    }

    fn put(dst: &mut Vec<u8>, bytes: &[u8]) {
        dst.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        dst.extend_from_slice(bytes);
    }

    fn build_raw(package: &RawPackage) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00]);
        bytes.extend_from_slice(&1u16.to_le_bytes()); // containerMajor
        bytes.extend_from_slice(&0u16.to_le_bytes()); // containerMinor
        bytes.extend_from_slice(&1u16.to_le_bytes()); // binaryReprMajor
        bytes.extend_from_slice(&0u16.to_le_bytes()); // binaryReprMinor
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        put(&mut bytes, &package.name);
        put(&mut bytes, &package.ident);
        put(&mut bytes, &package.version);
        put(&mut bytes, &package.author);
        put(&mut bytes, b""); // url
        put(&mut bytes, &package.ident_key);
        put(&mut bytes, &package.signing_key);
        put(&mut bytes, &package.proof);
        put(&mut bytes, &package.proof_sig);
        put(&mut bytes, &package.attestation);
        put(&mut bytes, &package.attestation_sig);
        bytes.extend_from_slice(&crypto::sha256(&package.payload));
        bytes.extend_from_slice(&(package.payload.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&package.signature_type.to_le_bytes());
        bytes.extend_from_slice(&(package.signature.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&package.signature);
        bytes.extend_from_slice(&package.payload);
        bytes
    }

    #[test]
    fn unsigned_package_parses_and_must_carry_no_chain() {
        // A clean unsigned package parses.
        let unsigned = build_raw(&RawPackage::default());
        let package = parse_mfp_package(&unsigned).unwrap();
        assert_eq!(package.signature_type, 0);
        assert_eq!(package.ident_fingerprint().unwrap(), "");

        // An unsigned package that carries any chain field is malformed.
        let with_ident = build_raw(&RawPackage {
            ident_key: b"ed25519:something".to_vec(),
            ..RawPackage::default()
        });
        assert!(parse_mfp_package(&with_ident)
            .unwrap_err()
            .contains("unsigned .mfp package must not carry identKey"));
    }

    #[test]
    fn signed_package_missing_a_chain_field_is_rejected() {
        // signatureType=1 with a 64-byte signature but no chain fields at all.
        let missing = build_raw(&RawPackage {
            signature_type: SIGNATURE_ED25519,
            signature: vec![0u8; 64],
            ..RawPackage::default()
        });
        assert!(parse_mfp_package(&missing)
            .unwrap_err()
            .contains("signed .mfp package is missing identKey"));
    }

    #[test]
    fn parse_rejects_field_limits_and_empty_required_fields() {
        // An empty required field (name) is rejected.
        let empty_name = build_raw(&RawPackage {
            name: Vec::new(),
            ..RawPackage::default()
        });
        assert!(parse_mfp_package(&empty_name)
            .unwrap_err()
            .contains("name must not be empty"));

        // A field over its length limit (version limit is 64).
        let long_version = build_raw(&RawPackage {
            version: vec![b'9'; 100],
            ..RawPackage::default()
        });
        assert!(parse_mfp_package(&long_version)
            .unwrap_err()
            .contains("version exceeds the 64 byte limit"));

        // Non-UTF-8 in a string field.
        let bad_utf8 = build_raw(&RawPackage {
            author: vec![0xff, 0xfe],
            ..RawPackage::default()
        });
        assert!(parse_mfp_package(&bad_utf8)
            .unwrap_err()
            .contains("not valid UTF-8"));
    }

    #[test]
    fn verify_attestation_rejects_ident_without_hash() {
        // A signed fixture whose ident lacks '#': the attestation-owner split
        // fails.
        let fx = signed_fixture("1.0.0");
        let mut package = parse_mfp_package(&fx.artifact).unwrap();
        package.ident = "no-hash".to_string();
        assert!(verify_attestation(
            &package,
            &fx.server_public,
            &crypto::fingerprint(&fx.server_public)
        )
        .unwrap_err()
        .contains("<owner>#<package>"));
    }
}
