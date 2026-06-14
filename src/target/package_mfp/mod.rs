use crate::bytecode::{self, BytecodeMetadata};
use crate::ir::IrProject;
use std::fs;
use std::path::{Path, PathBuf};

const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
const CONTAINER_MAJOR: u16 = 1;
const CONTAINER_MINOR: u16 = 0;
const BYTECODE_MAJOR: u16 = 1;
const BYTECODE_MINOR: u16 = 0;
const SIGNATURE_UNSIGNED: u16 = 0;
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
    metadata: &BytecodeMetadata,
) -> Result<PathBuf, String> {
    let bytecode = bytecode::build_bytecode_bytes(ir, metadata)?;
    let package = build_package_bytes(metadata, &bytecode)?;
    let path = project_dir.join(format!("{}.mfp", metadata.name));
    fs::write(&path, package)
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    Ok(path)
}

pub fn build_package_bytes(
    metadata: &BytecodeMetadata,
    package_bytecode: &[u8],
) -> Result<Vec<u8>, String> {
    validate_metadata(metadata)?;
    if !package_bytecode.starts_with(b"MFBC") {
        return Err("package payload must be MFB bytecode with MFBC magic".to_string());
    }

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MFP_MAGIC);
    put_u16(&mut bytes, CONTAINER_MAJOR);
    put_u16(&mut bytes, CONTAINER_MINOR);
    put_u16(&mut bytes, BYTECODE_MAJOR);
    put_u16(&mut bytes, BYTECODE_MINOR);
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
    put_u64(&mut bytes, package_bytecode.len() as u64);
    bytes.extend_from_slice(package_bytecode);
    Ok(bytes)
}

fn validate_metadata(metadata: &BytecodeMetadata) -> Result<(), String> {
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

fn package_ident(metadata: &BytecodeMetadata) -> &str {
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

fn container_flags(metadata: &BytecodeMetadata) -> u32 {
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
        let metadata = BytecodeMetadata {
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
        let payload = b"MFBCpayload";

        let package = build_package_bytes(&metadata, payload).expect("package bytes");

        assert!(package.starts_with(&MFP_MAGIC));
        assert_eq!(&package[8..10], &CONTAINER_MAJOR.to_le_bytes());
        assert_eq!(&package[10..12], &CONTAINER_MINOR.to_le_bytes());
        assert_eq!(&package[12..14], &BYTECODE_MAJOR.to_le_bytes());
        assert_eq!(&package[14..16], &BYTECODE_MINOR.to_le_bytes());
        assert!(package.ends_with(payload));
    }

    #[test]
    fn rejects_non_bytecode_payload() {
        let metadata = BytecodeMetadata {
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
        assert_eq!(err, "package payload must be MFB bytecode with MFBC magic");
    }
}
