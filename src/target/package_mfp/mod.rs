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
mod tests {
    use super::*;
    use crate::binary_repr::NativeLibraryTable;
    use mfb_repository::crypto;

    fn test_metadata() -> BinaryReprMetadata {
        BinaryReprMetadata {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.2.3".to_string(),
            ident_key: String::new(),
            ident_fingerprint: String::new(),
            signing_fingerprint: String::new(),
            author: "Ada".to_string(),
            url: "https://example.invalid/shape".to_string(),
            dependencies: Vec::new(),
            native_libraries: NativeLibraryTable::default(),
        }
    }

    fn test_signing() -> (PackageSigning, Vec<u8>, Vec<u8>) {
        let (ident_public, ident_private) = crypto::generate_keypair();
        let (signing_public, signing_private) = crypto::generate_keypair();
        let (server_public, server_private) = crypto::generate_keypair();
        let proof = format!(
            "{{\"owner\":\"ada\",\"ident\":\"ada#shape\",\"version\":\"1.2.3\",\"identFingerprint\":\"{}\",\"signingFingerprint\":\"{}\",\"issued\":1}}",
            crypto::fingerprint(&ident_public),
            crypto::fingerprint(&signing_public),
        );
        let proof_sig = crypto::sign(
            &ident_private,
            &crypto::proof_signing_input(proof.as_bytes()),
        )
        .unwrap();
        let attestation = format!(
            "{{\"repoFingerprint\":\"{}\",\"owner\":\"ada\",\"ident\":\"ada#shape\",\"version\":\"1.2.3\",\"identFingerprint\":\"{}\",\"signingFingerprint\":\"{}\",\"issued\":1}}",
            crypto::fingerprint(&server_public),
            crypto::fingerprint(&ident_public),
            crypto::fingerprint(&signing_public),
        );
        let attestation_sig = crypto::sign(
            &server_private,
            &crypto::attestation_signing_input(attestation.as_bytes()),
        )
        .unwrap();
        (
            PackageSigning {
                ident_key: format!("ed25519:{}", crypto::encode_bytes(&ident_public)),
                signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
                signing_private,
                proof,
                proof_sig,
                attestation,
                attestation_sig,
            },
            ident_public,
            server_public,
        )
    }

    #[test]
    fn wraps_mfbc_payload_in_unsigned_mfp_container() {
        let package =
            build_package_bytes(&test_metadata(), b"MFPCpayload", None).expect("package bytes");

        assert!(package.starts_with(&MFP_MAGIC));
        assert_eq!(&package[8..10], &CONTAINER_MAJOR.to_le_bytes());
        assert_eq!(&package[10..12], &CONTAINER_MINOR.to_le_bytes());
        assert!(package.ends_with(b"MFPCpayload"));

        let parsed = mfb_repository::package::parse_mfp_package(&package).expect("parse");
        assert_eq!(parsed.name, "shape");
        assert_eq!(parsed.ident, "ada#shape");
        assert_eq!(parsed.version, "1.2.3");
        assert_eq!(parsed.signature_type, 0);
        assert!(parsed.ident_key.is_empty());
        assert!(parsed.signing_key.is_empty());
        // The payload weld holds for unsigned packages too.
        mfb_repository::package::verify_payload_hash(&parsed).expect("payload hash");
    }

    #[test]
    fn signed_package_round_trips_and_verifies() {
        let (signing, ident_public, server_public) = test_signing();
        let package = build_package_bytes(&test_metadata(), b"MFPCpayload", Some(&signing))
            .expect("signed package");

        let parsed = mfb_repository::package::parse_mfp_package(&package).expect("parse");
        assert_eq!(parsed.container_major, 1);
        assert_eq!(parsed.container_minor, 0);
        assert_eq!(parsed.signature_type, 1);
        assert_eq!(parsed.ident_key, signing.ident_key);
        assert_eq!(parsed.signing_key, signing.signing_key);
        assert_eq!(parsed.proof, signing.proof);
        assert_eq!(parsed.attestation, signing.attestation);

        mfb_repository::package::verify_payload_hash(&parsed).expect("payload hash");
        mfb_repository::package::verify_package_signature(&parsed).expect("package signature");
        mfb_repository::package::verify_proof(&parsed, &ident_public).expect("proof");
        mfb_repository::package::verify_attestation(
            &parsed,
            &server_public,
            &crypto::fingerprint(&server_public),
        )
        .expect("attestation");
    }

    #[test]
    fn tampering_with_any_header_byte_breaks_the_signature() {
        let (signing, _ident_public, _server_public) = test_signing();
        let package = build_package_bytes(&test_metadata(), b"MFPCpayload", Some(&signing))
            .expect("signed package");
        let parsed = mfb_repository::package::parse_mfp_package(&package).expect("parse");
        let prefix_len = parsed.signed_prefix.len();

        // Flip one byte in every signed-prefix position that keeps the
        // structure parseable (skip length fields would break parsing — a
        // parse failure is an equally hard refusal, so only assert on the
        // ones that still parse).
        let mut verified_flips = 0;
        for index in 20..prefix_len {
            let mut tampered = package.clone();
            tampered[index] ^= 0x01;
            let Ok(reparsed) = mfb_repository::package::parse_mfp_package(&tampered) else {
                continue;
            };
            assert!(
                mfb_repository::package::verify_package_signature(&reparsed).is_err()
                    || mfb_repository::package::verify_payload_hash(&reparsed).is_err(),
                "flipping signed-prefix byte {index} must break verification"
            );
            verified_flips += 1;
        }
        assert!(verified_flips > 0);

        // Tampering with the payload breaks the packageBinaryHash weld.
        let mut tampered = package.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        let reparsed = mfb_repository::package::parse_mfp_package(&tampered).expect("parse");
        assert!(mfb_repository::package::verify_payload_hash(&reparsed).is_err());
    }

    #[test]
    fn refuses_to_build_a_package_whose_name_is_not_a_path_component() {
        let mut metadata = test_metadata();
        metadata.name = "../evil".to_string();
        let err = build_package_bytes(&metadata, b"", None).expect_err("traversing name");
        assert!(err.contains("not a valid path component"), "{err}");
    }

    /// An empty IR project, enough to type-check a `write_package` call. The
    /// name-traversal guard fires before the IR is lowered, so it is never used.
    fn empty_ir_project() -> IrProject {
        IrProject {
            name: "shape".to_string(),
            entry: None,
            bindings: Vec::new(),
            types: Vec::new(),
            functions: Vec::new(),
            native_resources: Vec::new(),
            link_functions: Vec::new(),
            link_cstructs: Vec::new(),
            link_aliases: Vec::new(),
            docs: crate::ir::ProjectDocs::default(),
            native_libraries: Default::default(),
            max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        }
    }

    #[test]
    fn write_package_rejects_a_traversing_name_and_writes_nothing_outside_the_dir() {
        // bug-58: `metadata.name` flows into `project_dir.join("<name>.mfp")`. A
        // name of `../../evil` would escape the project directory; the write must
        // be refused before any path is built.
        // Nest the project dir two levels down so the naive sink
        // `project_dir/../../evil.mfp` resolves back inside the tempdir (which is
        // auto-cleaned) rather than polluting the shared temp root.
        let root = tempfile::tempdir().expect("tempdir");
        let escape_target = root.path().join("evil.mfp");
        let project_dir = root.path().join("nested").join("project");
        fs::create_dir_all(&project_dir).expect("project dir");

        let mut metadata = test_metadata();
        metadata.name = "../../evil".to_string();
        let ir = empty_ir_project();

        let err = write_package(&project_dir, &ir, &metadata, &[], None)
            .expect_err("traversing name must be rejected");
        assert!(err.contains("not a valid path component"), "{err}");

        // Nothing escaped the project directory to the resolved traversal target.
        assert!(
            !escape_target.exists(),
            "no package may be written outside the project directory"
        );
        // And nothing was written inside the project directory either.
        let wrote_anything = fs::read_dir(&project_dir)
            .expect("read project dir")
            .next()
            .is_some();
        assert!(
            !wrote_anything,
            "no package file should be written on rejection"
        );
    }

    #[test]
    fn streamed_content_hash_matches_whole_file_hash() {
        // Spans several read chunks so the prefix check and the digest both see
        // more than one buffer.
        let mut bytes = MFP_MAGIC.to_vec();
        bytes.extend((0..200_000u32).map(|index| index as u8));
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("big.mfp");
        fs::write(&path, &bytes).expect("write");
        assert_eq!(
            package_content_hash_file(&path).expect("streamed hash"),
            package_content_hash(&bytes).expect("whole-file hash")
        );

        // A short file and a wrong magic are rejected exactly as in-memory.
        let short = dir.path().join("short.mfp");
        fs::write(&short, MFP_MAGIC).expect("write");
        assert!(package_content_hash_file(&short).is_err());
        let wrong = dir.path().join("wrong.mfp");
        fs::write(&wrong, vec![0u8; 64]).expect("write");
        assert!(package_content_hash_file(&wrong).is_err());
        assert!(package_content_hash_file(&dir.path().join("missing.mfp")).is_err());
    }

    #[test]
    fn rejects_non_binary_repr_payload() {
        let err =
            build_package_bytes(&test_metadata(), b"nope", None).expect_err("invalid payload");
        assert_eq!(
            err,
            "package payload must be the binary representation container"
        );
    }
}
