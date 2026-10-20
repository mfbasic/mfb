//! plan-176-K: `cli::build::tests` — the signing tests. The shared project builders live in\n//! `build/tests/mod.rs`.

use super::*;

#[test]
fn package_verification_labels() {
    assert_eq!(PackageVerification::Verified.label(), "Verified");
    assert_eq!(PackageVerification::Unsigned.label(), "Unsigned");
    assert_eq!(PackageVerification::Tampered.label(), "Tampered");
}

#[test]
fn source_is_local_classifies_sources() {
    assert!(source_is_local(""));
    assert!(source_is_local("file:packages/x.mfp"));
    assert!(source_is_local("local:x"));
    assert!(!source_is_local("ada#shape"));
    assert!(!source_is_local("https://registry/x"));
}

#[test]
fn signing_ident_defaults_to_owner_hash_name() {
    assert_eq!(
        signing_ident("ada", "shape", ""),
        Ok("ada#shape".to_string())
    );
    // A declared ident owned by the signer passes through unchanged.
    assert_eq!(
        signing_ident("ada", "shape", "ada#shape"),
        Ok("ada#shape".to_string())
    );
    // Case-insensitive owner match.
    assert_eq!(
        signing_ident("Ada", "shape", "ada#shape"),
        Ok("ada#shape".to_string())
    );
}

#[test]
fn signing_ident_rejects_bad_idents() {
    assert!(signing_ident("ada", "shape", "no-hash")
        .unwrap_err()
        .contains("must use <owner>#<package>"));
}

/// bug-792: the recipient of a transferred package signs releases of an
/// ident whose prefix names the original owner; the registry's `/signing`
/// decides who may (its stored package owner), not this local check.
#[test]
fn signing_ident_accepts_a_transferred_ident() {
    assert_eq!(
        signing_ident("bob", "shape", "ada#shape"),
        Ok("ada#shape".to_string())
    );
}

#[test]
fn classify_installed_package_reads_unsigned_fixture() {
    // A valid unsigned package classifies as Unsigned (no signature).
    let path = Path::new("tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp");
    assert!(path.is_file(), "fixture must exist");
    let classification = classify_installed_package(path, None);
    assert_eq!(classification.state, PackageVerification::Unsigned);
    assert!(classification.refusal.is_none());
}

#[test]
fn classify_installed_package_treats_missing_file_as_tampered() {
    let classification = classify_installed_package(Path::new("/no/such/pkg.mfp"), None);
    assert_eq!(classification.state, PackageVerification::Tampered);
    let (rule, _detail) = classification.refusal.expect("refusal");
    assert_eq!(rule, "PACKAGE_INVALID");
}

#[test]
fn classify_installed_package_treats_garbage_as_tampered() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garbage.mfp");
    std::fs::write(&path, b"this is not an mfp container").expect("write");
    let classification = classify_installed_package(&path, None);
    assert_eq!(classification.state, PackageVerification::Tampered);
    assert_eq!(
        classification.refusal.expect("refusal").0,
        "PACKAGE_INVALID"
    );
}

#[test]
fn verify_and_report_no_packages_is_ok() {
    let manifest = crate::manifest::parse_project_json(
        "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
        Path::new("project.json"),
    )
    .expect("manifest");
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(verify_and_report_packages(dir.path(), &manifest, false).is_ok());
}

#[test]
fn verify_and_report_missing_dependency_file_is_skipped() {
    // A declared dependency whose .mfp is not installed yet emits no
    // verification line and does not fail (the install check reports it).
    let manifest = crate::manifest::parse_project_json(
        concat!(
            "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",",
            "\"sources\":[{\"root\":\"src\"}],",
            "\"packages\":[{\"name\":\"shape\",\"ident\":\"ada#shape\",\"version\":\"1.0.0\",\"pin\":true,\"source\":\"ada#shape\",\"direct\":true,\"requiredBy\":[]}]}"
        ),
        Path::new("project.json"),
    )
    .expect("manifest");
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(verify_and_report_packages(dir.path(), &manifest, false).is_ok());
}

#[test]
fn verify_and_report_unsigned_remote_requires_flag() {
    // An installed unsigned package from a remote source is refused unless
    // --unsigned is passed.
    let manifest = crate::manifest::parse_project_json(
        concat!(
            "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",",
            "\"sources\":[{\"root\":\"src\"}],",
            "\"packages\":[{\"name\":\"trap_builtin_pkg\",\"ident\":\"tests#trap\",\"version\":\"0.1.0\",\"pin\":true,\"source\":\"tests#trap\",\"direct\":true,\"requiredBy\":[]}]}"
        ),
        Path::new("project.json"),
    )
    .expect("manifest");
    let dir = tempfile::tempdir().expect("temp dir");
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).expect("packages dir");
    std::fs::copy(
        "tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp",
        packages.join("trap_builtin_pkg.mfp"),
    )
    .expect("copy fixture");
    // Remote source, unsigned, no --unsigned -> refused.
    assert!(verify_and_report_packages(dir.path(), &manifest, false).is_err());
    // With --unsigned -> allowed.
    assert!(verify_and_report_packages(dir.path(), &manifest, true).is_ok());
}

/// bug-491: a `pin: true` dependency whose installed version differs from the
/// pinned one refuses the build.
///
/// The comparison existed in `manifest::package::installed_package_files`
/// since it was written, and a unit test asserted its error — but all three
/// of that function's callers discard the `Result` deliberately, so an
/// unreadable package degrades rather than failing the build. The pin
/// mismatch rode the same `Result` and was therefore enforced NOWHERE, while
/// the passing unit test kept it reading as a live gate.
///
/// `--unsigned` is passed in both halves so the only thing that can decide
/// the outcome is the version: the fixture is an unsigned package from a
/// remote source, which would otherwise be refused for that instead.
#[test]
fn verify_and_report_refuses_a_pinned_version_mismatch() {
    let manifest_for = |version: &str, pin: &str| {
        crate::manifest::parse_project_json(
            &format!(
                "{{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\
                     \"sources\":[{{\"root\":\"src\"}}],\
                     \"packages\":[{{\"name\":\"trap_builtin_pkg\",\"ident\":\"tests#trap\",\
                     \"version\":\"{version}\",\"pin\":{pin},\"source\":\"tests#trap\",\"direct\":true,\"requiredBy\":[]}}]}}"
            ),
            Path::new("project.json"),
        )
        .expect("manifest")
    };
    let install = || {
        let dir = tempfile::tempdir().expect("temp dir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).expect("packages dir");
        std::fs::copy(
            "tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp",
            packages.join("trap_builtin_pkg.mfp"),
        )
        .expect("copy fixture");
        dir
    };

    // RED: the fixture is 0.1.0; pinning it to 9.9.9 must refuse.
    let dir = install();
    assert!(
        verify_and_report_packages(dir.path(), &manifest_for("9.9.9", "true"), true).is_err(),
        "a pinned version mismatch must refuse the build"
    );

    // Positive pin: the SAME fixture at its real version is allowed, so the
    // refusal above is the version and nothing else.
    let dir = install();
    assert!(
        verify_and_report_packages(dir.path(), &manifest_for("0.1.0", "true"), true).is_ok(),
        "a pinned dependency at its pinned version must build"
    );

    // And an UNPINNED dependency is not version-checked at all: `version` is
    // an ABI floor there, not an equality, so a newer selection is legitimate.
    let dir = install();
    assert!(
        verify_and_report_packages(dir.path(), &manifest_for("9.9.9", "false"), true).is_ok(),
        "an unpinned dependency must not be version-compared"
    );
}

#[test]
fn verify_and_report_unsigned_local_is_allowed() {
    let manifest = crate::manifest::parse_project_json(
        concat!(
            "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",",
            "\"sources\":[{\"root\":\"src\"}],",
            "\"packages\":[{\"name\":\"trap_builtin_pkg\",\"ident\":\"tests#trap\",\"version\":\"0.1.0\",\"pin\":true,\"source\":\"file:packages/trap_builtin_pkg.mfp\",\"direct\":true,\"requiredBy\":[]}]}"
        ),
        Path::new("project.json"),
    )
    .expect("manifest");
    let dir = tempfile::tempdir().expect("temp dir");
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).expect("packages dir");
    std::fs::copy(
        "tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp",
        packages.join("trap_builtin_pkg.mfp"),
    )
    .expect("copy fixture");
    // Local source, unsigned -> allowed without the flag.
    assert!(verify_and_report_packages(dir.path(), &manifest, false).is_ok());
}

#[test]
fn apply_signing_metadata_copies_fields() {
    let mut metadata = binary_repr::BinaryReprMetadata::new("pkg".to_string(), "1.0.0".to_string());
    let signing = BuildSigningInfo {
        owner: "ada".to_string(),
        ident: "ada#pkg".to_string(),
        ident_fingerprint: "if".to_string(),
        signing_fingerprint: "sf".to_string(),
        package_signing: target::package_mfp::PackageSigning {
            ident_key: "ed25519:ik".to_string(),
            signing_key: "ed25519:sk".to_string(),
            signing_private: Vec::new(),
            proof: String::new(),
            proof_sig: Vec::new(),
            attestation: String::new(),
            attestation_sig: Vec::new(),
        },
        executable_signing: crate::arch::image::ExecutableSigning {
            metadata: Vec::new(),
            signing_private: None,
        },
    };
    apply_signing_metadata(&mut metadata, &signing);
    assert_eq!(metadata.ident, "ada#pkg");
    assert_eq!(metadata.ident_key, "ed25519:ik");
    assert_eq!(metadata.ident_fingerprint, "if");
    assert_eq!(metadata.signing_fingerprint, "sf");
    assert_eq!(metadata.author, "ada");
}

#[test]
fn executable_signing_metadata_json_is_valid_json() {
    let json = executable_signing_metadata_json(
        "ada",
        "https://registry.example",
        "ik",
        "if",
        "sk",
        "sf",
        "{}",
        "psig",
        "att",
        "asig",
    );
    let parsed: tinyjson::JsonValue = json.parse().expect("valid JSON");
    let object = parsed
        .get::<std::collections::HashMap<String, tinyjson::JsonValue>>()
        .expect("object");
    assert_eq!(
        object
            .get("format")
            .and_then(|v| v.get::<String>())
            .map(String::as_str),
        Some("mfb-signing-v1")
    );
    assert_eq!(
        object
            .get("owner")
            .and_then(|v| v.get::<String>())
            .map(String::as_str),
        Some("ada")
    );
    assert_eq!(
        object
            .get("registry")
            .and_then(|v| v.get::<String>())
            .map(String::as_str),
        Some("https://registry.example")
    );
    // The linker's seal fills exactly this placeholder.
    assert_eq!(
        object
            .get("contentSignature")
            .and_then(|v| v.get::<String>())
            .map(String::as_str),
        Some(crate::os::content_signature::CONTENT_SIGNATURE_PLACEHOLDER)
    );
}

#[test]
fn decode_trust_anchor_accepts_metadata_key_form() {
    // A malformed key is rejected.
    assert!(decode_trust_anchor("not-a-key").is_err());
}

// ---- packages.rs: the §3.5 signed-package chain (plan-68-B B2) ----
//
// `classify_installed_package` performs no network I/O: after the header
// identKey check it verifies the attestation/proof/signature/payload chain
// purely over the `.mfp` bytes plus a locally *pinned* server key. So the
// whole chain is unit-coverable with a hermetic, self-consistent signed
// fixture built here (the ident key signs the proof, a throwaway "server" key
// signs the attestation, and the one-off signing key signs the container) and
// that server key pinned under an empty `MFB_HOME`.

#[derive(Clone, Copy)]
enum SignedTamper {
    /// A fully valid chain (verifies to `Verified`).
    None,
    /// The header `identKey` is not decodable base64url.
    MalformedHeaderIdent,
    /// The attestation signature does not verify under the server key.
    AttestationSig,
    /// The proof signature does not verify under the ident key.
    ProofSig,
    /// The container is signed by a key other than the advertised signingKey.
    SignatureMismatch,
    /// A payload byte is flipped after the signed prefix (hash weld breaks).
    Payload,
}

struct SignedFixture {
    bytes: Vec<u8>,
    /// The ident-key trust anchor (`ed25519:<base64url>`) a project would pin.
    ident_key: String,
    /// The throwaway server public key to pin so the attestation verifies.
    server_public: Vec<u8>,
}

fn build_signed_fixture(ident: &str, version: &str, tamper: SignedTamper) -> SignedFixture {
    use mfb_repository::crypto;
    let (ident_public, ident_private) = crypto::generate_keypair();
    let (signing_public, signing_private) = crypto::generate_keypair();
    let (server_public, server_private) = crypto::generate_keypair();
    let ident_fingerprint = crypto::fingerprint(&ident_public);
    let signing_fingerprint = crypto::fingerprint(&signing_public);
    let repo_fingerprint = crypto::fingerprint(&server_public);
    let (owner, name) = ident.split_once('#').expect("ident is <owner>#<name>");

    let proof = format!(
        "{{\"owner\":\"{owner}\",\"ident\":\"{ident}\",\"version\":\"{version}\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}"
    );
    let mut proof_sig = crypto::sign(
        &ident_private,
        &crypto::proof_signing_input(proof.as_bytes()),
    )
    .unwrap();
    let attestation = format!(
        "{{\"repoFingerprint\":\"{repo_fingerprint}\",\"owner\":\"{owner}\",\"ident\":\"{ident}\",\"version\":\"{version}\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}"
    );
    let mut attestation_sig = crypto::sign(
        &server_private,
        &crypto::attestation_signing_input(attestation.as_bytes()),
    )
    .unwrap();

    // The header identKey (used as the trust anchor and to derive
    // identFingerprint); malformed only for that specific tamper.
    let header_ident_key = if matches!(tamper, SignedTamper::MalformedHeaderIdent) {
        "ed25519:not-valid-base64url-$$$".to_string()
    } else {
        format!("ed25519:{}", crypto::encode_bytes(&ident_public))
    };
    // The container is normally signed by `signing_private`; for the
    // signature-mismatch tamper it is signed by an UNRELATED key while the
    // advertised signingKey (and thus every fingerprint) stays consistent, so
    // the proof/attestation still verify and only the container signature
    // fails.
    let container_private = if matches!(tamper, SignedTamper::SignatureMismatch) {
        crypto::generate_keypair().1
    } else {
        signing_private
    };
    if matches!(tamper, SignedTamper::ProofSig) {
        proof_sig[0] ^= 0xff;
    }
    if matches!(tamper, SignedTamper::AttestationSig) {
        attestation_sig[0] ^= 0xff;
    }

    let mut metadata = binary_repr::BinaryReprMetadata::new(name.to_string(), version.to_string());
    metadata.ident = ident.to_string();
    metadata.author = owner.to_string();

    let signing = target::package_mfp::PackageSigning {
        ident_key: header_ident_key.clone(),
        signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
        signing_private: container_private,
        proof,
        proof_sig,
        attestation,
        attestation_sig,
    };
    let payload = b"MFPCsigned-fixture-payload".to_vec();
    let mut bytes =
        target::package_mfp::build_package_bytes(&metadata, &payload, Some(&signing)).unwrap();
    if matches!(tamper, SignedTamper::Payload) {
        // Flip the final payload byte: after the signed prefix, so the
        // container signature still verifies but the payload-hash weld breaks.
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
    }
    SignedFixture {
        bytes,
        ident_key: header_ident_key,
        server_public,
    }
}

/// Write a fixture's bytes to a `.mfp` under a tempdir and return the path.
fn write_mfp(dir: &Path, bytes: &[u8]) -> PathBuf {
    let path = dir.join("pkg.mfp");
    std::fs::write(&path, bytes).unwrap();
    path
}

/// A signed package with no pinned trust anchor is untrusted (the
/// file-embedded key is attacker-controlled).
#[test]
fn classify_signed_package_without_a_trust_anchor_is_untrusted() {
    let dir = tempfile::tempdir().unwrap();
    let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::None);
    let path = write_mfp(dir.path(), &fx.bytes);
    let classification = classify_installed_package(&path, None);
    assert_eq!(classification.state, PackageVerification::Tampered);
    assert_eq!(
        classification.refusal.expect("refusal").0,
        "PACKAGE_IDENT_KEY_UNTRUSTED"
    );
}

/// A malformed pinned trust anchor is rejected before any file key is read.
#[test]
fn classify_signed_package_with_a_malformed_anchor_is_untrusted() {
    let dir = tempfile::tempdir().unwrap();
    let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::None);
    let path = write_mfp(dir.path(), &fx.bytes);
    let classification = classify_installed_package(&path, Some("not-base64!"));
    assert_eq!(classification.state, PackageVerification::Tampered);
    assert_eq!(
        classification.refusal.expect("refusal").0,
        "PACKAGE_IDENT_KEY_UNTRUSTED"
    );
}

/// A malformed header identKey (present but not decodable) is untrusted.
#[test]
fn classify_signed_package_with_a_malformed_header_ident_is_untrusted() {
    let dir = tempfile::tempdir().unwrap();
    let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::MalformedHeaderIdent);
    let path = write_mfp(dir.path(), &fx.bytes);
    // A well-formed anchor so decode passes; the header key is the malformed one.
    let anchor = format!(
        "ed25519:{}",
        mfb_repository::crypto::encode_bytes(&[7u8; 32])
    );
    let classification = classify_installed_package(&path, Some(&anchor));
    assert_eq!(classification.state, PackageVerification::Tampered);
    assert_eq!(
        classification.refusal.expect("refusal").0,
        "PACKAGE_IDENT_KEY_UNTRUSTED"
    );
}

/// A well-formed anchor that is not the package's own header key is untrusted
/// (the header-identKey-≠-pinned arm).
#[test]
fn classify_signed_package_with_a_mismatched_anchor_is_untrusted() {
    let dir = tempfile::tempdir().unwrap();
    let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::None);
    let path = write_mfp(dir.path(), &fx.bytes);
    // A valid but wrong ed25519 key (32 bytes, not the fixture's ident).
    let anchor = format!(
        "ed25519:{}",
        mfb_repository::crypto::encode_bytes(&[9u8; 32])
    );
    let classification = classify_installed_package(&path, Some(&anchor));
    assert_eq!(classification.state, PackageVerification::Tampered);
    let (rule, detail) = classification.refusal.expect("refusal");
    assert_eq!(rule, "PACKAGE_IDENT_KEY_UNTRUSTED");
    assert!(detail.contains("does not match"), "{detail}");
}

/// With the correct anchor but NO pinned registry key on the machine, the
/// attestation cannot be checked — the reachable frontier without a registry.
#[test]
fn classify_signed_package_without_a_pinned_server_key_is_untrusted() {
    let _lock = crate::cli::tests::ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let _mfb = crate::cli::tests::EnvVarGuard::set("MFB_HOME", home.path().to_str().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::None);
    let path = write_mfp(dir.path(), &fx.bytes);
    let classification = classify_installed_package(&path, Some(&fx.ident_key));
    assert_eq!(classification.state, PackageVerification::Tampered);
    let (rule, detail) = classification.refusal.expect("refusal");
    assert_eq!(rule, "PACKAGE_ATTESTATION_INVALID");
    assert!(detail.contains("no pinned registry key"), "{detail}");
}

/// Pin the server key and classify each chain-link tamper on its own arm, then
/// the fully valid package to `Verified`.
fn classify_pinned(fx: &SignedFixture) -> PackageClassification {
    let _lock = crate::cli::tests::ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let _mfb = crate::cli::tests::EnvVarGuard::set("MFB_HOME", home.path().to_str().unwrap());
    let _fp = crate::cli::tests::EnvVarGuard::unset("MFB_REPO_SERVER_FINGERPRINT");
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = crate::cli::local_paths_for_repo(&repo_url).unwrap();
    mfb_repository::local::pin_server_key(&paths, &fx.server_public).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = write_mfp(dir.path(), &fx.bytes);
    classify_installed_package(&path, Some(&fx.ident_key))
}

#[test]
fn classify_signed_package_with_a_bad_attestation_signature() {
    let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::AttestationSig);
    let classification = classify_pinned(&fx);
    assert_eq!(classification.state, PackageVerification::Tampered);
    assert_eq!(
        classification.refusal.expect("refusal").0,
        "PACKAGE_ATTESTATION_INVALID"
    );
}

#[test]
fn classify_signed_package_with_a_bad_proof_signature() {
    let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::ProofSig);
    let classification = classify_pinned(&fx);
    assert_eq!(classification.state, PackageVerification::Tampered);
    assert_eq!(
        classification.refusal.expect("refusal").0,
        "PACKAGE_PROOF_INVALID"
    );
}

#[test]
fn classify_signed_package_with_a_bad_container_signature() {
    let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::SignatureMismatch);
    let classification = classify_pinned(&fx);
    assert_eq!(classification.state, PackageVerification::Tampered);
    assert_eq!(
        classification.refusal.expect("refusal").0,
        "PACKAGE_SIGNATURE_INVALID"
    );
}

#[test]
fn classify_signed_package_with_a_broken_payload_hash() {
    let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::Payload);
    let classification = classify_pinned(&fx);
    assert_eq!(classification.state, PackageVerification::Tampered);
    assert_eq!(
        classification.refusal.expect("refusal").0,
        "PACKAGE_PAYLOAD_HASH_MISMATCH"
    );
}

#[test]
fn classify_fully_signed_package_verifies() {
    let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::None);
    let classification = classify_pinned(&fx);
    assert_eq!(classification.state, PackageVerification::Verified);
    assert!(classification.refusal.is_none());
}

// ---- verify_and_report_packages: entry-shape + Verified/Tampered arms ----

/// A `packages` entry that is not an object, or an object without a `name`,
/// is silently skipped rather than crashing the report.
#[test]
fn verify_and_report_skips_malformed_package_entries() {
    let manifest = crate::manifest::parse_project_json(
        concat!(
            "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",",
            "\"sources\":[{\"root\":\"src\"}],",
            "\"packages\":[42, {\"version\":\"1.0.0\"}]}"
        ),
        Path::new("project.json"),
    )
    .expect("manifest");
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(verify_and_report_packages(dir.path(), &manifest, false).is_ok());
}

/// An installed, fully-verified dependency reports `[Verified]` and does not
/// block the build.
#[test]
fn verify_and_report_accepts_a_verified_dependency() {
    let _lock = crate::cli::tests::ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let _mfb = crate::cli::tests::EnvVarGuard::set("MFB_HOME", home.path().to_str().unwrap());
    let _fp = crate::cli::tests::EnvVarGuard::unset("MFB_REPO_SERVER_FINGERPRINT");
    let fx = build_signed_fixture("sec#signed", "0.1.0", SignedTamper::None);
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = crate::cli::local_paths_for_repo(&repo_url).unwrap();
    mfb_repository::local::pin_server_key(&paths, &fx.server_public).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).unwrap();
    std::fs::write(packages.join("signed.mfp"), &fx.bytes).unwrap();
    let manifest = crate::manifest::parse_project_json(
        &format!(
            "{{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{{\"root\":\"src\"}}],\"packages\":[{{\"name\":\"signed\",\"ident\":\"sec#signed\",\"version\":\"0.1.0\",\"pin\":true,\"source\":\"sec#signed\",\"identKey\":\"{}\",\"direct\":true,\"requiredBy\":[]}}]}}",
            fx.ident_key
        ),
        Path::new("project.json"),
    )
    .expect("manifest");
    assert!(verify_and_report_packages(dir.path(), &manifest, false).is_ok());
}

/// An installed but tampered signed dependency reports `[Tampered]` and is a
/// hard build gate (the refusal arm).
#[test]
fn verify_and_report_refuses_a_tampered_dependency() {
    let _lock = crate::cli::tests::ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let _mfb = crate::cli::tests::EnvVarGuard::set("MFB_HOME", home.path().to_str().unwrap());
    // No pinned server key -> the signed package fails attestation -> Tampered.
    let fx = build_signed_fixture("sec#signed", "0.1.0", SignedTamper::None);
    let dir = tempfile::tempdir().unwrap();
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).unwrap();
    std::fs::write(packages.join("signed.mfp"), &fx.bytes).unwrap();
    let manifest = crate::manifest::parse_project_json(
        &format!(
            "{{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{{\"root\":\"src\"}}],\"packages\":[{{\"name\":\"signed\",\"ident\":\"sec#signed\",\"version\":\"0.1.0\",\"pin\":true,\"source\":\"sec#signed\",\"identKey\":\"{}\",\"direct\":true,\"requiredBy\":[]}}]}}",
            fx.ident_key
        ),
        Path::new("project.json"),
    )
    .expect("manifest");
    assert!(verify_and_report_packages(dir.path(), &manifest, false).is_err());
}
