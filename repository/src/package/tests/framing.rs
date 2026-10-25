//! `.mfp` framing and trust-chain refusals the main suite leaves out: a file
//! cut inside the payload hash or the signature, a binary-representation
//! length that overflows, and a proof or attestation that is validly signed but
//! is not the JSON (or not the package) it must be.

use super::*;

const PAYLOAD: &[u8] = b"MFPCtestpayload";

/// Byte offsets of the fixed tail of a serialized package:
/// `hash(32) | binaryReprLength(8) | signatureType(2) | signatureLength(4) |
/// signature(64) | payload`.
fn hash_offset(artifact: &[u8]) -> usize {
    artifact.len() - PAYLOAD.len() - 64 - 6 - 8 - 32
}

#[test]
fn a_file_cut_inside_the_payload_hash_is_truncated() {
    let fx = signed_fixture("1.0.0");
    let mut cut = fx.artifact.clone();
    cut.truncate(hash_offset(&fx.artifact) + 10);
    assert_eq!(
        parse_mfp_package(&cut).unwrap_err(),
        "truncated .mfp packageBinaryHash"
    );
}

#[test]
fn a_file_cut_inside_the_signature_is_truncated() {
    let fx = signed_fixture("1.0.0");
    let mut cut = fx.artifact.clone();
    cut.truncate(hash_offset(&fx.artifact) + 32 + 8 + 6 + 10);
    assert_eq!(
        parse_mfp_package(&cut).unwrap_err(),
        "truncated .mfp signature"
    );
}

#[test]
fn a_binary_representation_length_that_overflows_is_refused() {
    let fx = signed_fixture("1.0.0");
    let mut forged = fx.artifact.clone();
    let at = hash_offset(&fx.artifact) + 32;
    forged[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(
        parse_mfp_package(&forged).unwrap_err(),
        "invalid .mfp binary representation length"
    );
}

/// A package whose proof and attestation texts are chosen by the test but
/// validly signed by its ident key and a server key.
struct Chosen {
    package: MfpPackage,
    ident_public: Vec<u8>,
    server_public: Vec<u8>,
}

fn with_chain(proof: &str, attestation: &str) -> Chosen {
    let (ident_public, ident_private) = crypto::generate_keypair();
    let (signing_public, signing_private) = crypto::generate_keypair();
    let (server_public, server_private) = crypto::generate_keypair();
    let artifact = serialize(
        &TestPackage {
            name: "toolbox".to_string(),
            ident: "alice#toolbox".to_string(),
            version: "1.0.0".to_string(),
            author: "alice".to_string(),
            url: String::new(),
            payload: PAYLOAD.to_vec(),
            ident_key: format!("ed25519:{}", crypto::encode_bytes(&ident_public)),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
            proof: proof.to_string(),
            proof_sig: crypto::sign(
                &ident_private,
                &crypto::proof_signing_input(proof.as_bytes()),
            )
            .unwrap(),
            attestation: attestation.to_string(),
            attestation_sig: crypto::sign(
                &server_private,
                &crypto::attestation_signing_input(attestation.as_bytes()),
            )
            .unwrap(),
        },
        &signing_private,
    );
    Chosen {
        package: parse_mfp_package(&artifact).unwrap(),
        ident_public,
        server_public,
    }
}

#[test]
fn a_signed_proof_that_is_not_json_is_refused() {
    let chosen = with_chain("not json", "{}");
    assert_eq!(
        verify_proof(&chosen.package, &chosen.ident_public).unwrap_err(),
        "malformed proof JSON"
    );
}

#[test]
fn a_signed_proof_for_another_version_is_refused() {
    let chosen = with_chain(
        "{\"owner\":\"alice\",\"ident\":\"alice#toolbox\",\"version\":\"9.9.9\"}",
        "{}",
    );
    assert_eq!(
        verify_proof(&chosen.package, &chosen.ident_public).unwrap_err(),
        "proof version does not match the package header"
    );
}

#[test]
fn a_signed_attestation_that_is_not_json_is_refused() {
    let chosen = with_chain("{}", "not json");
    assert_eq!(
        verify_attestation(
            &chosen.package,
            &chosen.server_public,
            &crypto::fingerprint(&chosen.server_public),
        )
        .unwrap_err(),
        "malformed attestation JSON"
    );
}
