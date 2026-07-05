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
    let payload_sha256 = crypto::sha256(&bytes[signature_end..payload_end]);

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
    let proof: serde_json::Value = serde_json::from_str(&package.proof)
        .map_err(|_| "malformed proof JSON".to_string())?;
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
            return Err(format!("attestation {field} does not match the package header"));
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
