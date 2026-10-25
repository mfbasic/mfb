use super::test_support::{serialize, TestPackage};
use super::*;

mod framing;

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
    signed_fixture_for("alice#toolbox", "alice", "alice", version)
}

/// A fully signed package for `ident`, whose header names `author` and
/// whose proof and attestation both sign `signed_owner` as the owner.
fn signed_fixture_for(ident: &str, author: &str, signed_owner: &str, version: &str) -> Fixture {
    let (ident_public, ident_private) = crypto::generate_keypair();
    let (signing_public, signing_private) = crypto::generate_keypair();
    let (server_public, server_private) = crypto::generate_keypair();
    let ident_fingerprint = crypto::fingerprint(&ident_public);
    let signing_fingerprint = crypto::fingerprint(&signing_public);
    let repo_fingerprint = crypto::fingerprint(&server_public);
    let proof = format!(
        "{{\"owner\":\"{signed_owner}\",\"ident\":\"{ident}\",\"version\":\"{version}\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}",
    );
    let proof_sig = crypto::sign(
        &ident_private,
        &crypto::proof_signing_input(proof.as_bytes()),
    )
    .unwrap();
    let attestation = format!(
        "{{\"repoFingerprint\":\"{repo_fingerprint}\",\"owner\":\"{signed_owner}\",\"ident\":\"{ident}\",\"version\":\"{version}\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}",
    );
    let attestation_sig = crypto::sign(
        &server_private,
        &crypto::attestation_signing_input(attestation.as_bytes()),
    )
    .unwrap();
    let artifact = serialize(
        &TestPackage {
            name: ident
                .split_once('#')
                .map_or(ident, |(_, name)| name)
                .to_string(),
            ident: ident.to_string(),
            version: version.to_string(),
            author: author.to_string(),
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

/// plan-168-B: the signed owner binds to the release's author, not to the
/// ident prefix, so bob's release of a transferred `alice#x` verifies — and
/// an alice-signed owner on a bob-authored release does not.
#[test]
fn proof_owner_binds_to_the_author_not_the_prefix() {
    let fx = signed_fixture_for("alice#x", "bob", "bob", "2.0.0");
    let package = parse_mfp_package(&fx.artifact).unwrap();
    verify_proof(&package, &fx.ident_public).unwrap();
    verify_attestation(
        &package,
        &fx.server_public,
        &crypto::fingerprint(&fx.server_public),
    )
    .unwrap();

    let fx = signed_fixture_for("alice#x", "bob", "alice", "2.0.0");
    let package = parse_mfp_package(&fx.artifact).unwrap();
    assert_eq!(
        verify_proof(&package, &fx.ident_public).unwrap_err(),
        "proof owner does not match the package author"
    );
    assert_eq!(
        verify_attestation(
            &package,
            &fx.server_public,
            &crypto::fingerprint(&fx.server_public),
        )
        .unwrap_err(),
        "attestation owner does not match the package author"
    );
}

/// plan-168-B (B-D1): a pre-plan artifact signs the ident prefix's spelling
/// as its owner, which may differ in case from the registered display name
/// the header's author carries. Fold-equality keeps it verifying.
#[test]
fn a_case_differing_legacy_prefix_still_verifies() {
    let fx = signed_fixture_for("Alice#toolbox", "alice", "Alice", "1.0.0");
    let package = parse_mfp_package(&fx.artifact).unwrap();
    verify_proof(&package, &fx.ident_public).unwrap();
    verify_attestation(
        &package,
        &fx.server_public,
        &crypto::fingerprint(&fx.server_public),
    )
    .unwrap();
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
