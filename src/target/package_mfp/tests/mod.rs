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
        description: String::new(),
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

/// bug-353 B: a dependency name is interned into the import table and
/// interpolated as `packages/<name>.mfp` by a consumer, so it must be a
/// single safe path component. The manifest reader `package_dependencies`
/// omits the `validate_package_name` guard its sibling applies, so a `../…`
/// name reached the container verbatim. `validate_metadata` is the
/// authoritative build-time gate and must refuse to produce such a container.
#[test]
fn rejects_a_traversal_dependency_name() {
    let mut metadata = test_metadata();
    metadata.dependencies = vec![binary_repr::BinaryReprDependency {
        name: "../../etc/passwd".to_string(),
        ident: "evil".to_string(),
        version: "9.9.9".to_string(),
        pin: false,
        flags: 0,
    }];
    let err = build_package_bytes(&metadata, b"MFPCpayload", None)
        .expect_err("a traversal dependency name must be rejected");
    assert!(err.contains("not a valid path component"), "{err}");

    // A blank dependency name is likewise rejected (required, non-empty).
    let mut blank = test_metadata();
    blank.dependencies = vec![binary_repr::BinaryReprDependency {
        name: "   ".to_string(),
        ident: "x".to_string(),
        version: "1.0.0".to_string(),
        pin: false,
        flags: 0,
    }];
    assert!(build_package_bytes(&blank, b"MFPCpayload", None).is_err());

    // A well-formed dependency name still produces a container.
    let mut ok = test_metadata();
    ok.dependencies = vec![binary_repr::BinaryReprDependency {
        name: "greet".to_string(),
        ident: "greet".to_string(),
        version: "1.0.0".to_string(),
        pin: false,
        flags: 0,
    }];
    assert!(build_package_bytes(&ok, b"MFPCpayload", None).is_ok());
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
        canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS,
        arena_memory: crate::ir::ArenaMemoryLimits::default(),
        selected_config: Default::default(),
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
    let err = build_package_bytes(&test_metadata(), b"nope", None).expect_err("invalid payload");
    assert_eq!(
        err,
        "package payload must be the binary representation container"
    );
}

/// A signed package's trust chain is validated before any byte is written: every
/// field the container carries must be present, within its limit, and the two
/// signatures exactly 64 bytes.
#[test]
fn a_malformed_signing_chain_is_refused_before_building() {
    let payload = b"MFPC-payload";
    let build = |edit: &dyn Fn(&mut PackageSigning)| {
        let (mut signing, _, _) = test_signing();
        edit(&mut signing);
        build_package_bytes(&test_metadata(), payload, Some(&signing))
            .expect_err("the malformed chain is refused")
    };
    assert_eq!(
        build(&|signing| signing.ident_key = String::new()),
        "package identKey must not be empty"
    );
    assert_eq!(
        build(&|signing| signing.proof = "x".repeat(BLOB_LIMIT + 1)),
        format!(
            "package proof is {} bytes, exceeding the {BLOB_LIMIT} byte limit",
            BLOB_LIMIT + 1
        )
    );
    assert_eq!(
        build(&|signing| {
            signing.proof_sig.pop();
        }),
        "proof and attestation signatures must be 64 bytes"
    );
    assert_eq!(
        build(&|signing| signing.attestation_sig.push(0)),
        "proof and attestation signatures must be 64 bytes"
    );
}

/// The whole-artifact hash is only defined for something that is an `.mfp`.
#[test]
fn the_content_hash_of_bytes_that_are_not_a_package_is_refused() {
    assert_eq!(
        package_content_hash(b"short").err().as_deref(),
        Some("package is not a valid .mfp package")
    );
    let mut wrong_magic = vec![0u8; 64];
    wrong_magic[0] = b'X';
    assert!(package_content_hash(&wrong_magic).is_err());
}
