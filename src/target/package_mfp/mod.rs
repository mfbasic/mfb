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
/// "Package contains native LINK metadata" (plan-46-B §4.4): set when the package
/// carries a `NATIVE_LIBRARY_TABLE` (section id 10). Optional — section 10 is the
/// source of truth, so a reader that ignores this bit must not reject the package.
const FLAG_NATIVE_LINK_METADATA: u32 = 1 << 0;
const FLAG_PRE_RELEASE: u32 = 1 << 3;

const NAME_LIMIT: usize = 255;
const IDENT_LIMIT: usize = 255;
const VERSION_LIMIT: usize = 64;
const AUTHOR_LIMIT: usize = 512;
const URL_LIMIT: usize = 2048;
const KEY_LIMIT: usize = 255;
const BLOB_LIMIT: usize = 4096;

/// The signing material threaded into a signed package build (plan-23 §3.3):
/// the account ident key, the one-off per-package signing keypair, the
/// ident-signed proof, and the server-signed attestation. The one-off private
/// key lives only in this struct for the duration of the build and is
/// discarded with it.
pub struct PackageSigning {
    /// Ident public key, metadata form (`ed25519:<base64url>`).
    pub ident_key: String,
    /// One-off signing public key, metadata form.
    pub signing_key: String,
    /// One-off signing private key (never written to disk).
    pub signing_private: Vec<u8>,
    /// Proof JSON (plan-23 §5), signed by the ident key.
    pub proof: String,
    /// 64-byte ident signature over `"MFP-PROOF-v1\0" || proof`.
    pub proof_sig: Vec<u8>,
    /// Attestation JSON (plan-23 §5), signed by the server key.
    pub attestation: String,
    /// 64-byte server signature over `"MFP-ATTEST-v1\0" || attestation`.
    pub attestation_sig: Vec<u8>,
}

pub fn write_package(
    project_dir: &Path,
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    packages: &[PathBuf],
    signing: Option<&PackageSigning>,
) -> Result<PathBuf, String> {
    // The output path is `project_dir/<name>.mfp`, so the name is validated
    // (single safe path component — see `validate_metadata`) BEFORE it is ever
    // interpolated into a filesystem path. `build_package_bytes` re-validates,
    // but doing it here first guarantees a traversing name like `../../evil` is
    // rejected before any lowering work is done or any path is built (bug-58,
    // same class as bug-27's pre-verify package write).
    validate_metadata(metadata)?;
    let binary_repr = binary_repr::build_package_binary_repr_bytes(ir, metadata, packages)?;
    let package = build_package_bytes(metadata, &binary_repr, signing)?;
    let path = project_dir.join(format!("{}.mfp", metadata.name));
    fs::write(&path, package)
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    Ok(path)
}

/// Serialize the container v1.0 header + payload (plan-23 §4).
///
/// Layout: magic, containerMajor=1, containerMinor=0, binaryReprMajor/Minor,
/// flags, then length-prefixed name/ident/version/author/url/identKey/
/// signingKey/proof/proofSig/attestation/attestationSig, the raw 32-byte
/// `packageBinaryHash` (SHA-256 of the payload), `binaryReprLength` (u64),
/// `signatureType` (u16), `signatureLength` (u32), the signature bytes, and
/// the payload. The signature is made by the one-off signing key over
/// `"MFP-PACKAGE-v2\0" || SHA-256(bytes[0 .. signature offset))`, so every
/// header byte is covered directly and the payload transitively through
/// `packageBinaryHash`.
pub fn build_package_bytes(
    metadata: &BinaryReprMetadata,
    package_binary_repr: &[u8],
    signing: Option<&PackageSigning>,
) -> Result<Vec<u8>, String> {
    validate_metadata(metadata)?;
    if !package_binary_repr.starts_with(b"MFPC") {
        return Err("package payload must be the binary representation container".to_string());
    }
    if let Some(signing) = signing {
        validate_string("identKey", &signing.ident_key, KEY_LIMIT, true)?;
        validate_string("signingKey", &signing.signing_key, KEY_LIMIT, true)?;
        validate_string("proof", &signing.proof, BLOB_LIMIT, true)?;
        validate_string("attestation", &signing.attestation, BLOB_LIMIT, true)?;
        if signing.proof_sig.len() != 64 || signing.attestation_sig.len() != 64 {
            return Err("proof and attestation signatures must be 64 bytes".to_string());
        }
    }

    let payload_hash = sha256(package_binary_repr);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(&MFP_MAGIC);
    put_u16(&mut bytes, CONTAINER_MAJOR);
    put_u16(&mut bytes, CONTAINER_MINOR);
    put_u16(&mut bytes, BINARY_REPR_MAJOR);
    put_u16(&mut bytes, BINARY_REPR_MINOR);
    put_u32(&mut bytes, container_flags(metadata));
    put_bytes(&mut bytes, metadata.name.as_bytes());
    put_bytes(&mut bytes, package_ident(metadata).as_bytes());
    put_bytes(&mut bytes, metadata.version.as_bytes());
    put_bytes(&mut bytes, metadata.author.as_bytes());
    put_bytes(&mut bytes, metadata.url.as_bytes());
    match signing {
        Some(signing) => {
            put_bytes(&mut bytes, signing.ident_key.as_bytes());
            put_bytes(&mut bytes, signing.signing_key.as_bytes());
            put_bytes(&mut bytes, signing.proof.as_bytes());
            put_bytes(&mut bytes, &signing.proof_sig);
            put_bytes(&mut bytes, signing.attestation.as_bytes());
            put_bytes(&mut bytes, &signing.attestation_sig);
        }
        None => {
            // Unsigned local packages carry none of the trust chain.
            for _ in 0..6 {
                put_u32(&mut bytes, 0);
            }
        }
    }
    bytes.extend_from_slice(&payload_hash);
    put_u64(&mut bytes, package_binary_repr.len() as u64);
    match signing {
        Some(signing) => {
            put_u16(&mut bytes, SIGNATURE_ED25519);
            put_u32(&mut bytes, 64);
            // The signed prefix ends exactly here, before the signature bytes.
            let message = mfb_repository::crypto::package_signing_input(&bytes);
            let signature = mfb_repository::crypto::sign(&signing.signing_private, &message)?;
            if signature.len() != 64 {
                return Err("Ed25519 package signature must be 64 bytes".to_string());
            }
            bytes.extend_from_slice(&signature);
        }
        None => {
            put_u16(&mut bytes, SIGNATURE_UNSIGNED);
            put_u32(&mut bytes, 0);
        }
    }
    bytes.extend_from_slice(package_binary_repr);
    Ok(bytes)
}

/// SHA-256 of the whole artifact: the blob/dedup identity used by the
/// publish flow. With the prefix signature covering the header and
/// `packageBinaryHash` welding the payload, the file is immutable after
/// signing, so no signature-zeroing is needed.
pub fn package_content_hash(bytes: &[u8]) -> Result<[u8; 32], String> {
    if bytes.len() < 20 || bytes[0..8] != MFP_MAGIC {
        return Err("package is not a valid .mfp package".to_string());
    }
    Ok(sha256(bytes))
}

/// The same digest as [`package_content_hash`], streamed from disk in bounded
/// chunks. A `packages/*.mfp` is untrusted input of arbitrary size, so a caller
/// that only needs its content hash must not read it whole into memory.
pub fn package_content_hash_file(path: &Path) -> Result<[u8; 32], String> {
    use std::io::Read;

    const CHUNK: usize = 64 * 1024;

    let mut file =
        fs::File::open(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    let mut prefix: Vec<u8> = Vec::with_capacity(20);
    let mut length: u64 = 0;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        if prefix.len() < 20 {
            let wanted = (20 - prefix.len()).min(read);
            prefix.extend_from_slice(&buffer[..wanted]);
        }
        hasher.update(&buffer[..read]);
        length += read as u64;
    }
    if length < 20 || prefix[0..8] != MFP_MAGIC {
        return Err("package is not a valid .mfp package".to_string());
    }
    let digest = hasher.finalize();
    let mut hash = [0; 32];
    hash.copy_from_slice(&digest);
    Ok(hash)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hash = [0; 32];
    hash.copy_from_slice(&digest);
    hash
}

fn validate_metadata(metadata: &BinaryReprMetadata) -> Result<(), String> {
    validate_string("name", &metadata.name, NAME_LIMIT, true)?;
    // A consumer installs this package as `packages/<name>.mfp`, so the name must
    // be a single safe path component. Refuse to *produce* one that is not.
    crate::manifest::package::validate_package_name(&metadata.name)?;
    validate_string("ident", package_ident(metadata), IDENT_LIMIT, true)?;
    validate_string("version", &metadata.version, VERSION_LIMIT, true)?;
    validate_string("author", &metadata.author, AUTHOR_LIMIT, false)?;
    validate_string("url", &metadata.url, URL_LIMIT, false)?;
    // bug-353 B: each dependency name is interned into the `.mfp` import table
    // and interpolated as `packages/<name>.mfp` by a consumer, so it must be a
    // single safe path component — the same guard the package's own name and the
    // header reader (`read_mfp_header`) apply. The manifest reader
    // (`package_dependencies`) does not validate these, unlike its sibling
    // `project_package_dependency`, so a `../../etc/passwd` dependency reached
    // the container verbatim. Refuse to *produce* such a container; this is the
    // authoritative build-time gate at the boundary the container-format spec
    // (`01_container-format.md:258`) claims enforces it.
    for dependency in &metadata.dependencies {
        validate_string("dependency name", &dependency.name, NAME_LIMIT, true)?;
        crate::manifest::package::validate_package_name(&dependency.name)?;
    }
    Ok(())
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
    let mut flags = 0;
    if metadata.version.contains('-') {
        flags |= FLAG_PRE_RELEASE;
    }
    // plan-46-B §4.4: a binding package carrying a section-10 locator table sets
    // the "contains native LINK metadata" bit the format reserved for it. It stays
    // an *optional* flag — section 10 is the source of truth, and a reader that
    // ignores the bit must not reject the package.
    if !metadata.native_libraries.is_empty() {
        flags |= FLAG_NATIVE_LINK_METADATA;
    }
    flags
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
mod tests;
