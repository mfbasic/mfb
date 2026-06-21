use crate::crypto;
use sha2::{Digest, Sha256};

const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
const HEADER_PREFIX_LEN: usize = 26;
const SIGNATURE_ED25519: u16 = 1;

#[derive(Debug, Clone)]
pub struct MfpPackage {
    pub name: String,
    pub ident: String,
    pub version: String,
    pub ident_key: String,
    pub ident_fingerprint: String,
    pub signing_fingerprint: String,
    pub author: String,
    pub url: String,
    pub container_major: u16,
    pub container_minor: u16,
    pub binary_repr_major: u16,
    pub binary_repr_minor: u16,
    pub flags: u32,
    pub signature_type: u16,
    pub signature: Vec<u8>,
    pub binary_repr_length: usize,
    pub content_hash: [u8; 32],
}

impl MfpPackage {
    pub fn content_hash_hex(&self) -> String {
        hex::encode(self.content_hash)
    }
}

pub fn parse_mfp_package(bytes: &[u8]) -> Result<MfpPackage, String> {
    if bytes.len() < HEADER_PREFIX_LEN {
        return Err("package is too small to be a valid .mfp package".to_string());
    }
    if bytes[0..8] != MFP_MAGIC {
        return Err("package does not have the MFP package magic".to_string());
    }

    let container_major = read_u16(bytes, 8)?;
    if container_major != 1 {
        return Err(format!(
            "unsupported MFP container major version {container_major}"
        ));
    }
    let container_minor = read_u16(bytes, 10)?;
    let binary_repr_major = read_u16(bytes, 12)?;
    let binary_repr_minor = read_u16(bytes, 14)?;
    let flags = read_u32(bytes, 16)?;
    let signature_type = read_u16(bytes, 20)?;
    let signature_length = read_u32(bytes, 22)? as usize;
    validate_signature_header(signature_type, signature_length)?;
    let signature_start = HEADER_PREFIX_LEN;
    let signature_end = signature_start
        .checked_add(signature_length)
        .ok_or_else(|| "invalid .mfp signature length".to_string())?;
    if signature_end > bytes.len() {
        return Err("truncated .mfp signature".to_string());
    }

    let mut offset = signature_end;
    let name = read_mfp_string(bytes, &mut offset, "name", 255, true)?;
    let ident = read_mfp_string(bytes, &mut offset, "ident", 255, true)?;
    let version = read_mfp_string(bytes, &mut offset, "version", 64, true)?;
    let ident_key = read_mfp_string(bytes, &mut offset, "identKey", 255, false)?;
    let ident_fingerprint = read_mfp_string(bytes, &mut offset, "identFingerprint", 255, false)?;
    let signing_fingerprint =
        read_mfp_string(bytes, &mut offset, "signingFingerprint", 255, false)?;
    let author = read_mfp_string(bytes, &mut offset, "author", 512, false)?;
    let url = read_mfp_string(bytes, &mut offset, "url", 2048, false)?;
    let binary_repr_length = read_u64(bytes, offset)? as usize;
    offset = offset
        .checked_add(8)
        .and_then(|offset| offset.checked_add(binary_repr_length))
        .ok_or_else(|| "invalid .mfp binary representation length".to_string())?;
    if offset != bytes.len() {
        return Err("invalid .mfp binary representation length".to_string());
    }

    Ok(MfpPackage {
        name,
        ident,
        version,
        ident_key,
        ident_fingerprint,
        signing_fingerprint,
        author,
        url,
        container_major,
        container_minor,
        binary_repr_major,
        binary_repr_minor,
        flags,
        signature_type,
        signature: bytes[signature_start..signature_end].to_vec(),
        binary_repr_length,
        content_hash: package_content_hash(bytes)?,
    })
}

pub fn verify_package_signature(package: &MfpPackage, signing_public_key: &[u8]) -> Result<(), String> {
    if package.signature_type != SIGNATURE_ED25519 {
        return Err("registry publishes require an Ed25519 package signature".to_string());
    }
    let message = package_signature_message(
        &package.content_hash,
        package.ident.as_bytes(),
        package.version.as_bytes(),
    );
    crypto::verify(signing_public_key, &message, &package.signature)
}

pub fn package_content_hash(bytes: &[u8]) -> Result<[u8; 32], String> {
    if bytes.len() < HEADER_PREFIX_LEN {
        return Err("package is too small to be a valid .mfp package".to_string());
    }
    let signature_length = read_u32(bytes, 22)? as usize;
    let signature_end = HEADER_PREFIX_LEN
        .checked_add(signature_length)
        .ok_or_else(|| "invalid .mfp signature length".to_string())?;
    if signature_end > bytes.len() {
        return Err("truncated .mfp signature".to_string());
    }

    let mut hasher = Sha256::new();
    hasher.update(&bytes[..HEADER_PREFIX_LEN]);
    hasher.update(vec![0; signature_length]);
    hasher.update(&bytes[signature_end..]);
    let digest = hasher.finalize();
    let mut hash = [0; 32];
    hash.copy_from_slice(&digest);
    Ok(hash)
}

pub fn package_signature_message(content_hash: &[u8; 32], ident: &[u8], version: &[u8]) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(b"MFP-PACKAGE-v1");
    message.extend_from_slice(content_hash);
    message.extend_from_slice(ident);
    message.extend_from_slice(version);
    message
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
    let value = std::str::from_utf8(&bytes[*offset..end])
        .map_err(|_| format!(".mfp {field} is not valid UTF-8"))?
        .to_string();
    *offset = end;
    if required && value.is_empty() {
        return Err(format!(".mfp {field} must not be empty"));
    }
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
