use crate::binary_repr::{self, BinaryReprMetadata};
use crate::ir::IrProject;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
const CONTAINER_MAJOR: u16 = 1;
const CONTAINER_MINOR: u16 = 0;
const BINARY_REPR_MAJOR: u16 = 1;
const BINARY_REPR_MINOR: u16 = 0;
const SIGNATURE_UNSIGNED: u16 = 0;
const SIGNATURE_ED25519: u16 = 1;
const HEADER_PREFIX_LEN: usize = 26;
const FLAG_PRE_RELEASE: u32 = 1 << 3;

const NAME_LIMIT: usize = 255;
const IDENT_LIMIT: usize = 255;
const VERSION_LIMIT: usize = 64;
const IDENT_KEY_LIMIT: usize = 255;
const IDENT_FINGERPRINT_LIMIT: usize = 255;
const SIGNING_FINGERPRINT_LIMIT: usize = 255;
const AUTHOR_LIMIT: usize = 512;
const URL_LIMIT: usize = 2048;

pub fn write_package(
    project_dir: &Path,
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    let binary_repr = binary_repr::build_package_binary_repr_bytes(ir, metadata, packages)?;
    let package = build_package_bytes(metadata, &binary_repr)?;
    let path = project_dir.join(format!("{}.mfp", metadata.name));
    fs::write(&path, package)
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    Ok(path)
}

pub fn build_package_bytes(
    metadata: &BinaryReprMetadata,
    package_binary_repr: &[u8],
) -> Result<Vec<u8>, String> {
    validate_metadata(metadata)?;
    if !package_binary_repr.starts_with(b"MFPC") {
        return Err("package payload must be the binary representation container".to_string());
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MFP_MAGIC);
    put_u16(&mut bytes, CONTAINER_MAJOR);
    put_u16(&mut bytes, CONTAINER_MINOR);
    put_u16(&mut bytes, BINARY_REPR_MAJOR);
    put_u16(&mut bytes, BINARY_REPR_MINOR);
    put_u32(&mut bytes, container_flags(metadata));
    put_u16(&mut bytes, SIGNATURE_UNSIGNED);
    put_u32(&mut bytes, 0);
    put_bytes(&mut bytes, metadata.name.as_bytes());
    put_bytes(&mut bytes, package_ident(metadata).as_bytes());
    put_bytes(&mut bytes, metadata.version.as_bytes());
    put_bytes(&mut bytes, metadata.ident_key.as_bytes());
    put_bytes(&mut bytes, metadata.ident_fingerprint.as_bytes());
    put_bytes(&mut bytes, metadata.signing_fingerprint.as_bytes());
    put_bytes(&mut bytes, metadata.author.as_bytes());
    put_bytes(&mut bytes, metadata.url.as_bytes());
    put_u64(&mut bytes, package_binary_repr.len() as u64);
    bytes.extend_from_slice(package_binary_repr);
    Ok(bytes)
}

pub fn package_content_hash(bytes: &[u8]) -> Result<[u8; 32], String> {
    if bytes.len() < HEADER_PREFIX_LEN {
        return Err("package is too small to be a valid .mfp package".to_string());
    }
    if bytes[0..8] != MFP_MAGIC {
        return Err("package does not have the MFP package magic".to_string());
    }
    let signature_type = u16::from_le_bytes([bytes[20], bytes[21]]);
    let signature_length =
        u32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]) as usize;
    validate_signature_header(signature_type, signature_length)?;
    let signature_end = HEADER_PREFIX_LEN
        .checked_add(signature_length)
        .ok_or_else(|| "invalid .mfp signature length".to_string())?;
    if signature_end > bytes.len() {
        return Err("truncated .mfp signature".to_string());
    }

    let mut hasher = Sha256::new();
    hasher.update(&bytes[..HEADER_PREFIX_LEN]);
    if signature_length > 0 {
        hasher.update(vec![0; signature_length]);
    }
    hasher.update(&bytes[signature_end..]);
    let digest = hasher.finalize();
    let mut hash = [0; 32];
    hash.copy_from_slice(&digest);
    Ok(hash)
}

fn validate_metadata(metadata: &BinaryReprMetadata) -> Result<(), String> {
    validate_string("name", &metadata.name, NAME_LIMIT, true)?;
    validate_string("ident", package_ident(metadata), IDENT_LIMIT, true)?;
    validate_string("version", &metadata.version, VERSION_LIMIT, true)?;
    validate_string("identKey", &metadata.ident_key, IDENT_KEY_LIMIT, false)?;
    validate_string(
        "identFingerprint",
        &metadata.ident_fingerprint,
        IDENT_FINGERPRINT_LIMIT,
        false,
    )?;
    validate_string(
        "signingFingerprint",
        &metadata.signing_fingerprint,
        SIGNING_FINGERPRINT_LIMIT,
        false,
    )?;
    validate_string("author", &metadata.author, AUTHOR_LIMIT, false)?;
    validate_string("url", &metadata.url, URL_LIMIT, false)?;
    Ok(())
}

fn validate_signature_header(signature_type: u16, signature_length: usize) -> Result<(), String> {
    match (signature_type, signature_length) {
        (SIGNATURE_UNSIGNED, 0) | (SIGNATURE_ED25519, 64) => Ok(()),
        (SIGNATURE_UNSIGNED, _) => {
            Err("unsigned .mfp package must have zero signature length".to_string())
        }
        (SIGNATURE_ED25519, _) => {
            Err("Ed25519 .mfp package must have a 64 byte signature".to_string())
        }
        _ => Err(format!("unsupported .mfp signature type {signature_type}")),
    }
}

fn package_ident(metadata: &BinaryReprMetadata) -> &str {
    if metadata.ident.is_empty() {
        &metadata.name
    } else {
        &metadata.ident
    }
}

fn validate_string(field: &str, value: &str, limit: usize, required: bool) -> Result<(), String> {
    if required && value.is_empty() {
        return Err(format!("package {field} must not be empty"));
    }
    if value.len() > limit {
        return Err(format!(
            "package {field} is {} bytes, exceeding the {limit} byte limit",
            value.len()
        ));
    }
    Ok(())
}

fn container_flags(metadata: &BinaryReprMetadata) -> u32 {
    if metadata.version.contains('-') {
        FLAG_PRE_RELEASE
    } else {
        0
    }
}

fn put_bytes(dst: &mut Vec<u8>, bytes: &[u8]) {
    put_u32(dst, bytes.len() as u32);
    dst.extend_from_slice(bytes);
}

fn put_u16(dst: &mut Vec<u8>, value: u16) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(dst: &mut Vec<u8>, value: u64) {
    dst.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_mfbc_payload_in_unsigned_mfp_container() {
        let metadata = BinaryReprMetadata {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.2.3".to_string(),
            ident_key: "ed25519:abc".to_string(),
            ident_fingerprint: "sha256:ident".to_string(),
            signing_fingerprint: "sha256:signing".to_string(),
            author: "Ada".to_string(),
            url: "https://example.invalid/shape".to_string(),
            dependencies: Vec::new(),
        };
        let payload = b"MFPCpayload";

        let package = build_package_bytes(&metadata, payload).expect("package bytes");

        assert!(package.starts_with(&MFP_MAGIC));
        assert_eq!(&package[8..10], &CONTAINER_MAJOR.to_le_bytes());
        assert_eq!(&package[10..12], &CONTAINER_MINOR.to_le_bytes());
        assert_eq!(&package[12..14], &BINARY_REPR_MAJOR.to_le_bytes());
        assert_eq!(&package[14..16], &BINARY_REPR_MINOR.to_le_bytes());
        assert!(package.ends_with(payload));
        let hash = package_content_hash(&package).expect("content hash");
        assert_ne!(hash, [0; 32]);
    }

    #[test]
    fn content_hash_zeroes_signature_bytes_but_covers_header_fields() {
        let metadata = BinaryReprMetadata {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.2.3".to_string(),
            ident_key: "ed25519:abc".to_string(),
            ident_fingerprint: "sha256:ident".to_string(),
            signing_fingerprint: "sha256:signing".to_string(),
            author: "Ada".to_string(),
            url: "https://example.invalid/shape".to_string(),
            dependencies: Vec::new(),
        };
        let mut package = build_package_bytes(&metadata, b"MFPCpayload").expect("package bytes");
        package[20..22].copy_from_slice(&SIGNATURE_ED25519.to_le_bytes());
        package[22..26].copy_from_slice(&64_u32.to_le_bytes());
        package.splice(HEADER_PREFIX_LEN..HEADER_PREFIX_LEN, [0x7f; 64]);

        let hash = package_content_hash(&package).expect("content hash");
        package[HEADER_PREFIX_LEN..HEADER_PREFIX_LEN + 64].fill(0x42);
        assert_eq!(package_content_hash(&package).expect("content hash"), hash);

        package[16] ^= FLAG_PRE_RELEASE as u8;
        assert_ne!(package_content_hash(&package).expect("content hash"), hash);
    }

    #[test]
    fn rejects_non_binary_repr_payload() {
        let metadata = BinaryReprMetadata {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.2.3".to_string(),
            ident_key: String::new(),
            ident_fingerprint: String::new(),
            signing_fingerprint: String::new(),
            author: String::new(),
            url: String::new(),
            dependencies: Vec::new(),
        };

        let err = build_package_bytes(&metadata, b"nope").expect_err("invalid payload");
        assert_eq!(
            err,
            "package payload must be the binary representation container"
        );
    }
}
