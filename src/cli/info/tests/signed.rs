//! plan-176-K: `cli::info::tests` — the signed-image tests and their signing-chain fixture\n//! (`Chain`). The shared image builders and `FakeRegistry` live in `info/tests/mod.rs`.

use super::*;

/// Every truncation and every single-byte corruption of a real header is read
/// without a panic: the file is untrusted, and each offset it names is checked.
#[test]
fn truncated_and_corrupted_images_never_panic() {
    let dir = tempfile::tempdir().unwrap();
    let chain = Chain::new();
    for (_, _, path) in linked_images(dir.path(), Some(&chain.executable_signing())) {
        let bytes = std::fs::read(&path).unwrap();
        for length in 0..bytes.len() {
            render(&path, &bytes[..length], &FakeRegistry::offline());
        }
        for at in 0..bytes.len().min(1_024) {
            for value in [0x00, 0x7f, 0xff] {
                let mut corrupted = bytes.clone();
                corrupted[at] = value;
                render(&path, &corrupted, &FakeRegistry::offline());
            }
        }
    }
}

/// A genuine chain, with every field a test may want to alter before signing.
struct Chain {
    ident_public: Vec<u8>,
    ident_private: Vec<u8>,
    signing_private: Vec<u8>,
    server_private: Vec<u8>,
    server_public: Vec<u8>,
    owner: String,
    author: String,
    registry: Option<String>,
    ident_key: String,
    ident_fingerprint: String,
    signing_key: String,
    signing_fingerprint: String,
    proof: String,
    attestation: String,
}

impl Chain {
    fn new() -> Self {
        Chain::signed_by("alice", "alice#probe")
    }

    /// A chain `owner` signed for `ident` — the ident's prefix may name another
    /// owner, the shape of an ident that reached `owner` by transfer.
    fn signed_by(owner: &str, ident: &str) -> Self {
        let (ident_public, ident_private) = crypto::generate_keypair();
        let (signing_public, signing_private) = crypto::generate_keypair();
        let (server_public, server_private) = crypto::generate_keypair();
        let ident_fingerprint = crypto::fingerprint(&ident_public);
        let signing_fingerprint = crypto::fingerprint(&signing_public);
        let proof = serde_json::json!({
            "owner": owner,
            "ident": ident,
            "version": "1.2.3",
            "identFingerprint": ident_fingerprint,
            "signingFingerprint": signing_fingerprint,
            "issued": 1_000_000_000,
        })
        .to_string();
        let attestation = serde_json::json!({
            "repoFingerprint": crypto::fingerprint(&server_public),
            "owner": owner,
            "ident": ident,
            "version": "1.2.3",
            "identFingerprint": ident_fingerprint,
            "signingFingerprint": signing_fingerprint,
            "issued": 1_000_000_000,
        })
        .to_string();
        Chain {
            ident_key: format!("ed25519:{}", crypto::encode_bytes(&ident_public)),
            ident_public,
            ident_private,
            signing_private,
            server_private,
            server_public,
            owner: owner.to_string(),
            author: owner.to_string(),
            registry: Some(REGISTRY.to_string()),
            ident_fingerprint,
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
            signing_fingerprint,
            proof,
            attestation,
        }
    }

    /// The registry that attested this chain, still holding its key, with the
    /// signing ident as the owner's current key.
    fn registry(&self) -> FakeRegistry {
        FakeRegistry {
            server_key: Ok(self.server_public.clone()),
            current_ident: Ok(self.ident_public.clone()),
            chain: Vec::new(),
            indexes: Vec::new(),
        }
    }

    fn proof_signature(&self) -> Vec<u8> {
        crypto::sign(
            &self.ident_private,
            &crypto::proof_signing_input(self.proof.as_bytes()),
        )
        .unwrap()
    }

    fn attestation_signature(&self) -> Vec<u8> {
        crypto::sign(
            &self.server_private,
            &crypto::attestation_signing_input(self.attestation.as_bytes()),
        )
        .unwrap()
    }

    /// A blob whose content signature covers [`DIGEST`], for the file-less tests.
    fn blob(&self) -> Vec<u8> {
        let content = crypto::sign(
            &self.signing_private,
            &crypto::executable_signing_input(&DIGEST),
        )
        .unwrap();
        self.blob_with(
            &self.proof_signature(),
            &self.attestation_signature(),
            Some(&crypto::encode_bytes(&content)),
        )
    }

    /// What a `--sign` build hands the linker: the blob with the placeholder, and
    /// the one-off key that fills it.
    fn executable_signing(&self) -> ExecutableSigning {
        ExecutableSigning {
            metadata: self.blob_with(
                &self.proof_signature(),
                &self.attestation_signature(),
                Some(CONTENT_SIGNATURE_PLACEHOLDER),
            ),
            signing_private: Some(self.signing_private.clone()),
        }
    }

    fn blob_with(
        &self,
        proof_signature: &[u8],
        attestation_signature: &[u8],
        content_signature: Option<&str>,
    ) -> Vec<u8> {
        // Field order and trailing newline of `executable_signing_metadata_json`;
        // an absent optional field is a blob from before that field existed.
        let registry = self
            .registry
            .as_deref()
            .map(|value| format!("\"registry\":{},", serde_json::Value::from(value)))
            .unwrap_or_default();
        let content = content_signature
            .map(|value| format!("\"contentSignature\":{},", serde_json::Value::from(value)))
            .unwrap_or_default();
        format!(
            "{{\"format\":\"mfb-signing-v1\",\"owner\":{},\"author\":{},{registry}\"identKey\":{},\"identFingerprint\":{},\"signingKey\":{},\"signingFingerprint\":{},\"proof\":{},\"proofSignature\":{},\"attestation\":{},\"attestationSignature\":{},{content}\"signatureType\":\"Ed25519\"}}\n",
            serde_json::Value::from(self.owner.as_str()),
            serde_json::Value::from(self.author.as_str()),
            serde_json::Value::from(self.ident_key.as_str()),
            serde_json::Value::from(self.ident_fingerprint.as_str()),
            serde_json::Value::from(self.signing_key.as_str()),
            serde_json::Value::from(self.signing_fingerprint.as_str()),
            serde_json::Value::from(self.proof.as_str()),
            serde_json::Value::from(crypto::encode_bytes(proof_signature)),
            serde_json::Value::from(self.attestation.as_str()),
            serde_json::Value::from(crypto::encode_bytes(attestation_signature)),
        )
        .into_bytes()
    }
}

/// The value of the `<prefix>: ` line under `Signed: yes`.
fn signed_line(lines: &[String], prefix: &str) -> String {
    let key = format!("{prefix}: ");
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&key))
        .unwrap_or_else(|| panic!("no `{prefix}` line in {lines:?}"))
        .to_string()
}

fn verdicts(
    blob: &[u8],
    digest: Option<[u8; 32]>,
    registry: &dyn RegistryLookup,
) -> (String, String, String) {
    let lines = signing_lines(blob, digest, registry);
    (
        signed_line(&lines, "contents"),
        signed_line(&lines, "ident key"),
        signed_line(&lines, "trust chain"),
    )
}

fn trust_chain(blob: &[u8], registry: &dyn RegistryLookup) -> String {
    verdicts(blob, Some(DIGEST), registry).2
}

/// A signed image in every format reports what the blob records, intact
/// contents, a current ident key, and a verified chain naming the registry key —
/// the linker sealed a content signature this reader accepts.
#[test]
fn a_signed_image_reports_its_signer_and_verdicts() {
    let chain = Chain::new();
    let dir = tempfile::tempdir().unwrap();
    for (format, _, path) in linked_images(dir.path(), Some(&chain.executable_signing())) {
        let out = report(&path, &chain.registry());
        let signed = out.split_once("Signed: yes\n").map(|(_, rest)| rest);
        assert_eq!(
            signed,
            Some(
                format!(
                    "  owner: alice\n  author: alice\n  registry: {REGISTRY}\n  ident: alice#probe\n  \
                     version: 1.2.3\n  issued: 2001-09-09 01:46:40 UTC\n  ident fingerprint: {}\n  \
                     signing fingerprint: {}\n  contents: intact\n  ident key: current\n  \
                     trust chain: verified (repoFingerprint {})\n\n{SIGNED_NOTE}\n",
                    chain.ident_fingerprint,
                    chain.signing_fingerprint,
                    crypto::fingerprint(&chain.server_public)
                )
                .as_str()
            ),
            "{format}:\n{out}"
        );
        assert!(
            !std::fs::read(&path)
                .unwrap()
                .windows(CONTENT_SIGNATURE_PLACEHOLDER.len())
                .any(|window| window == CONTENT_SIGNATURE_PLACEHOLDER.as_bytes()),
            "{format}: the linker filled the placeholder"
        );
    }
}

fn report_line<'a>(out: &'a str, prefix: &str) -> &'a str {
    out.lines()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` line in:\n{out}"))
}

/// Any byte of the image outside the `.mfbsign` blob — header, code, or the last
/// covered byte — is covered: changing one reports the contents modified, while
/// the identity chain, which does not depend on the file's bytes, still verifies.
#[test]
fn a_changed_byte_anywhere_outside_the_blob_reports_modified_contents() {
    let chain = Chain::new();
    let dir = tempfile::tempdir().unwrap();
    for (format, _, path) in linked_images(dir.path(), Some(&chain.executable_signing())) {
        let bytes = std::fs::read(&path).unwrap();
        let binary = inspect(&bytes).expect("a sealed image reads back");
        let blob = binary.signing.clone().expect("signed");
        let code_at = bytes
            .windows(2)
            .position(|window| window == [0xc3, 0] || window == [0xd6, 0])
            .expect("code bytes");
        for at in [24, code_at, blob.start - 1, binary.covered_end - 1] {
            let mut changed = bytes.clone();
            changed[at] ^= 0x01;
            let out = render(&path, &changed, &chain.registry());
            if !out.contains("Signed: yes") {
                // A header byte that unmakes the format is a refusal, not a pass.
                assert_eq!(out, format!("{NOT_MFBASIC}\n"), "{format} @{at}");
                continue;
            }
            assert_eq!(
                report_line(&out, "  contents: "),
                "  contents: MODIFIED (content signature does not match the file)",
                "{format} @{at}"
            );
            assert!(
                report_line(&out, "  trust chain: ").starts_with("  trust chain: verified"),
                "{format} @{at}:\n{out}"
            );
        }
    }
}

/// A valid blob moved into a different image does not match there: the content
/// signature names the image it was sealed over, not the signer alone.
#[test]
fn a_blob_copied_into_another_image_reports_modified_contents() {
    let chain = Chain::new();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let signing = chain.executable_signing();
    let originals = linked_images_with(first.path(), Some(&signing), 1);
    let others = linked_images_with(second.path(), Some(&signing), 2);
    for ((format, _, original), (_, _, other)) in originals.into_iter().zip(others) {
        let source = std::fs::read(&original).unwrap();
        let mut target = std::fs::read(&other).unwrap();
        let from = inspect(&source).unwrap().signing.unwrap();
        let to = inspect(&target).unwrap().signing.unwrap();
        assert_eq!(from.len(), to.len(), "{format}: same blob, same length");
        assert_ne!(
            source[from.clone()],
            target[to.clone()],
            "{format}: sealed apart"
        );
        target[to].copy_from_slice(&source[from]);
        assert_eq!(
            report_line(&render(&other, &target, &chain.registry()), "  contents: "),
            "  contents: MODIFIED (content signature does not match the file)",
            "{format}"
        );
    }
}

#[test]
fn contents_that_carry_no_usable_signature_say_so() {
    let chain = Chain::new();
    let legacy = chain.blob_with(
        &chain.proof_signature(),
        &chain.attestation_signature(),
        None,
    );
    assert_eq!(
        verdicts(&legacy, Some(DIGEST), &chain.registry()).0,
        "not signed (no contentSignature)"
    );
    // A placeholder the linker never filled is no signature over these bytes.
    let unsealed = chain.blob_with(
        &chain.proof_signature(),
        &chain.attestation_signature(),
        Some(CONTENT_SIGNATURE_PLACEHOLDER),
    );
    assert_eq!(
        verdicts(&unsealed, Some(DIGEST), &chain.registry()).0,
        "MODIFIED (content signature does not match the file)"
    );
    assert_eq!(
        verdicts(&chain.blob(), None, &chain.registry()).0,
        "unverified (the signing section lies outside the signed range)"
    );
    // A content signature made by some other key.
    let (_, stranger) = crypto::generate_keypair();
    let foreign = crypto::sign(&stranger, &crypto::executable_signing_input(&DIGEST)).unwrap();
    let blob = chain.blob_with(
        &chain.proof_signature(),
        &chain.attestation_signature(),
        Some(&crypto::encode_bytes(&foreign)),
    );
    assert_eq!(
        verdicts(&blob, Some(DIGEST), &chain.registry()).0,
        "MODIFIED (content signature does not match the file)"
    );
    // The contents verdict stands apart from the chain.
    assert_eq!(
        verdicts(&chain.blob(), Some(DIGEST), &FakeRegistry::offline()).0,
        "intact"
    );
}

/// The registry is taken from the blob, and every way it can fail to vouch for
/// the attestation is refused: absent, unreachable, or holding another key.
#[test]
fn the_chain_is_checked_against_the_registry_the_blob_names() {
    let chain = Chain::new();
    assert_eq!(
        verdicts(&chain.blob(), Some(DIGEST), &FakeRegistry::offline()),
        (
            "intact".to_string(),
            "not checked".to_string(),
            format!("NOT VERIFIED (registry {REGISTRY} is unreachable: connection refused)")
        )
    );

    let (other_key, _) = crypto::generate_keypair();
    let moved = FakeRegistry {
        server_key: Ok(other_key.clone()),
        ..chain.registry()
    };
    assert_eq!(
        trust_chain(&chain.blob(), &moved),
        format!(
            "NOT VERIFIED (registry {REGISTRY} holds a key other than the one that signed the \
             attestation (repoFingerprint {}))",
            crypto::fingerprint(&other_key)
        )
    );

    let mut unnamed = Chain::new();
    unnamed.registry = None;
    let lines = signing_lines(&unnamed.blob(), Some(DIGEST), &FakeRegistry::offline());
    assert_eq!(signed_line(&lines, "registry"), "<none>");
    assert_eq!(
        signed_line(&lines, "trust chain"),
        "NOT VERIFIED (no registry in the signing metadata)"
    );
}

/// The owner's ident key as the registry holds it today: still the signer's, a
/// signed rotation away from it, or replaced with no link — which no longer
/// vouches for the build.
#[test]
fn the_ident_key_is_checked_against_the_owner_s_current_key() {
    let chain = Chain::new();
    let (rotated_public, _) = crypto::generate_keypair();
    let rotation_signature = crypto::sign(
        &chain.ident_private,
        &crypto::ident_rotation_message(
            "alice",
            &crypto::fingerprint(&chain.ident_public),
            &rotated_public,
        ),
    )
    .unwrap();

    let rotated = FakeRegistry {
        current_ident: Ok(rotated_public.clone()),
        chain: vec![(
            chain.ident_public.clone(),
            rotated_public.clone(),
            rotation_signature.clone(),
        )],
        ..chain.registry()
    };
    let (_, ident_key, trust) = verdicts(&chain.blob(), Some(DIGEST), &rotated);
    assert_eq!(
        ident_key,
        format!(
            "rotated since signing (current identFingerprint {})",
            crypto::fingerprint(&rotated_public)
        )
    );
    assert!(trust.starts_with("verified (repoFingerprint "), "{trust}");

    let replaced = FakeRegistry {
        current_ident: Ok(rotated_public.clone()),
        chain: Vec::new(),
        ..chain.registry()
    };
    let (_, ident_key, trust) = verdicts(&chain.blob(), Some(DIGEST), &replaced);
    assert_eq!(
        ident_key,
        format!(
            "REPLACED without a rotation link (current identFingerprint {})",
            crypto::fingerprint(&rotated_public)
        )
    );
    assert!(
        trust.starts_with(
            "NOT VERIFIED (the owner's ident key was replaced without a rotation link;"
        ),
        "{trust}"
    );

    // A link whose signature is not the retiring key's is no rotation.
    let forged = FakeRegistry {
        current_ident: Ok(rotated_public.clone()),
        chain: vec![(chain.ident_public.clone(), rotated_public, vec![0; 64])],
        ..chain.registry()
    };
    assert_eq!(
        verdicts(&chain.blob(), Some(DIGEST), &forged).1,
        "unknown (invalid ident chain link signature)"
    );

    let silent = FakeRegistry {
        current_ident: Err("unknown owner".to_string()),
        ..chain.registry()
    };
    let (_, ident_key, trust) = verdicts(&chain.blob(), Some(DIGEST), &silent);
    assert_eq!(ident_key, "unknown (unknown owner)");
    assert!(trust.starts_with("verified (repoFingerprint "), "{trust}");
}

/// The `/index` of `alice#probe` as the registry serves it, with alice's own
/// key and, when `transfer_to` is given, a signed alice -> recipient transfer
/// (plan-168-A) accepted under the recipient's ident.
fn probe_index(chain: &Chain, transfer_to: Option<(&str, &[u8], &[u8])>) -> IndexResponse {
    let binding = |owner: &str, public: &[u8]| {
        let fingerprint = crypto::fingerprint(public);
        let signature = crypto::sign(
            &chain.server_private,
            &crypto::name_binding_message(owner, &fingerprint),
        )
        .unwrap();
        OwnerBinding {
            owner: owner.to_string(),
            ident_key: format!("ed25519:{}", crypto::encode_bytes(public)),
            ident_fingerprint: fingerprint,
            name_binding_signature: crypto::encode_bytes(&signature),
        }
    };
    let (alice_public, alice_private) = crypto::generate_keypair();
    let origin = binding("alice", &alice_public);
    let transfers = transfer_to
        .map(|(to, to_public, to_private)| {
            let digest = crypto::release_digest(std::iter::empty());
            let alice_fingerprint = crypto::fingerprint(&alice_public);
            let terms = crypto::TransferTermsRef {
                ident: "alice#probe",
                sequence: 0,
                from_owner: "alice",
                from_ident_fingerprint: &alice_fingerprint,
                to_owner: to,
                release_digest: &digest,
            };
            vec![TransferLink {
                sequence: 0,
                from_owner: "alice".to_string(),
                from_ident_key: Some(origin.ident_key.clone()),
                to_owner: to.to_string(),
                to_ident_key: Some(format!("ed25519:{}", crypto::encode_bytes(to_public))),
                release_digest: Some(digest.clone()),
                offer_signature: Some(crypto::encode_bytes(
                    &crypto::sign(&alice_private, &crypto::transfer_offer_message_v2(terms))
                        .unwrap(),
                )),
                accept_signature: Some(crypto::encode_bytes(
                    &crypto::sign(to_private, &crypto::transfer_accept_message_v2(terms)).unwrap(),
                )),
                offered_at: 0,
                accepted_at: 0,
                to_binding: binding(to, to_public),
            }]
        })
        .unwrap_or_default();
    IndexResponse {
        ident: "alice#probe".to_string(),
        owner: "alice".to_string(),
        ident_key: origin.ident_key,
        ident_fingerprint: origin.ident_fingerprint,
        name_binding_signature: origin.name_binding_signature,
        server_fingerprint: crypto::fingerprint(&chain.server_public),
        versions: Vec::new(),
        transfers,
    }
}

/// plan-168-C: an executable bob signed for `alice#probe` after alice
/// transferred it to him verifies — the registry's signed transfer chain shows
/// bob's signing ident in his ownership — and reports bob as its owner.
#[test]
fn a_transferred_executable_reports_the_new_owner() {
    let chain = Chain::signed_by("bob", "alice#probe");
    let registry = FakeRegistry {
        indexes: vec![probe_index(
            &chain,
            Some(("bob", &chain.ident_public, &chain.ident_private)),
        )],
        ..chain.registry()
    };
    let lines = signing_lines(&chain.blob(), Some(DIGEST), &registry);
    assert_eq!(signed_line(&lines, "owner"), "bob");
    assert_eq!(signed_line(&lines, "ident"), "alice#probe");
    assert_eq!(signed_line(&lines, "ident key"), "current");
    let trust = signed_line(&lines, "trust chain");
    assert!(trust.starts_with("verified (repoFingerprint "), "{trust}");
}

/// plan-168-C: without a signed transfer to the signer, an ident whose prefix
/// names another owner is not the signer's — whatever the registry attested.
#[test]
fn a_foreign_ident_without_a_chain_is_refused() {
    let chain = Chain::signed_by("bob", "alice#probe");
    let untransferred = FakeRegistry {
        indexes: vec![probe_index(&chain, None)],
        ..chain.registry()
    };
    assert_eq!(
        trust_chain(&chain.blob(), &untransferred),
        "NOT VERIFIED (proof ident does not belong to the signing owner)"
    );
    // A transfer accepted by some OTHER key of "bob" does not vouch for this one.
    let (other_public, other_private) = crypto::generate_keypair();
    let elsewhere = FakeRegistry {
        indexes: vec![probe_index(
            &chain,
            Some(("bob", &other_public, &other_private)),
        )],
        ..chain.registry()
    };
    assert_eq!(
        trust_chain(&chain.blob(), &elsewhere),
        "NOT VERIFIED (proof ident does not belong to the signing owner)"
    );
}

#[test]
fn a_chain_attested_by_another_registry_key_is_not_verified() {
    let chain = Chain::new();
    let mut other = Chain::new();
    // Same attestation text, signed by a key the registry does not hold.
    other.attestation = chain.attestation.clone();
    let blob = chain.blob_with(
        &chain.proof_signature(),
        &other.attestation_signature(),
        None,
    );
    assert_eq!(
        trust_chain(&blob, &chain.registry()),
        "NOT VERIFIED (invalid attestation signature)"
    );
}

/// Each link of the chain refuses a blob altered after signing, or signed over
/// inconsistent claims.
#[test]
fn every_link_of_the_chain_refuses_an_altered_blob() {
    let verdict = |chain: &Chain, blob: Vec<u8>| trust_chain(&blob, &chain.registry());

    // The proof rewritten after it was signed.
    let mut chain = Chain::new();
    let signed = chain.blob();
    let original = chain.proof.clone();
    chain.proof = original.replace("1.2.3", "9.9.9");
    let forged = String::from_utf8(signed).unwrap().replace(
        &serde_json::Value::from(original.as_str()).to_string(),
        &serde_json::Value::from(chain.proof.as_str()).to_string(),
    );
    assert_eq!(
        verdict(&chain, forged.into_bytes()),
        "NOT VERIFIED (invalid proof signature)"
    );

    // The attestation rewritten after it was signed.
    let chain = Chain::new();
    let forged = String::from_utf8(chain.blob())
        .unwrap()
        .replacen("alice#probe", "alice#other", 2);
    assert!(verdict(&chain, forged.into_bytes()).starts_with("NOT VERIFIED (invalid "));

    // An author the signatures never covered.
    let mut chain = Chain::new();
    chain.author = "mallory".to_string();
    assert_eq!(
        verdict(&chain, chain.blob()),
        "NOT VERIFIED (author does not match the signing owner)"
    );

    // A fingerprint that is not the key's.
    let mut chain = Chain::new();
    chain.ident_fingerprint = crypto::fingerprint(b"another key");
    assert_eq!(
        verdict(&chain, chain.blob()),
        "NOT VERIFIED (identFingerprint does not match identKey)"
    );
    let mut chain = Chain::new();
    chain.signing_fingerprint = crypto::fingerprint(b"another key");
    assert_eq!(
        verdict(&chain, chain.blob()),
        "NOT VERIFIED (signingFingerprint does not match signingKey)"
    );

    // A validly signed proof and attestation that name a different version.
    let mut chain = Chain::new();
    chain.attestation = chain.attestation.replace("1.2.3", "1.2.4");
    assert_eq!(
        verdict(&chain, chain.blob()),
        "NOT VERIFIED (attestation version does not match the signing metadata)"
    );

    // A validly signed proof for an ident another owner holds.
    let mut chain = Chain::new();
    chain.proof = chain.proof.replace("alice#probe", "bob#probe");
    assert_eq!(
        verdict(&chain, chain.blob()),
        "NOT VERIFIED (proof ident does not belong to the signing owner)"
    );

    // Signatures of the wrong length.
    let chain = Chain::new();
    assert_eq!(
        verdict(&chain, chain.blob_with(&[0; 3], &[0; 64], None)),
        "NOT VERIFIED (invalid proof signature)"
    );
}

/// Strings from the blob — the registry URL among them — reach the terminal
/// escaped, including when they come back inside a verdict.
#[test]
fn signer_strings_are_sanitized() {
    let mut chain = Chain::new();
    chain.owner = "al\u{1b}[2Jice".to_string();
    chain.author = chain.owner.clone();
    chain.registry = Some("https://evil\u{1b}[2J.test".to_string());
    let lines = signing_lines(&chain.blob(), Some(DIGEST), &FakeRegistry::offline());
    assert!(
        lines.iter().all(|line| !line.contains('\u{1b}')),
        "{lines:?}"
    );
}

/// plan-166: a `--debug` image's symbol table is inside the range the content
/// signature covers, so a signed `--debug` build still reports intact contents.
#[test]
fn a_signed_debug_image_reports_intact_contents() {
    let chain = Chain::new();
    let dir = tempfile::tempdir().unwrap();
    let debug_image = |text: Vec<u8>, entry: &str| EncodedImage {
        debug_symbols: true,
        ..image(text, entry, Some(chain.executable_signing()))
    };
    let arm_ret = vec![0xc0, 0x03, 0x5f, 0xd6];
    let images = vec![
        (
            "Mach-O",
            crate::os::macos::write_linked_executable(
                &dir.path().join("macho"),
                "probe",
                &debug_image(arm_ret.clone(), "_main"),
            )
            .expect("link Mach-O"),
        ),
        (
            "ELF static",
            crate::os::linux::write_linked_executable(
                &dir.path().join("elf-static"),
                "probe",
                "aarch64",
                LinuxFlavor::Glibc,
                &debug_image(arm_ret.clone(), "_main"),
            )
            .expect("link static ELF"),
        ),
        (
            "ELF dynamic",
            crate::os::linux::write_linked_executable(
                &dir.path().join("elf-dynamic"),
                "probe",
                "aarch64",
                LinuxFlavor::Glibc,
                &EncodedImage {
                    imports: vec![import("libc.so.6", "_exit")],
                    ..debug_image(arm_ret, "_main")
                },
            )
            .expect("link dynamic ELF"),
        ),
        (
            "PE",
            crate::os::windows::write_linked_executable(
                &dir.path().join("pe"),
                "probe",
                &debug_image(vec![0xc3], "_main"),
                false,
                None,
                None,
            )
            .expect("link PE"),
        ),
    ];
    for (format, path) in images {
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.windows(6).any(|window| window == b"_main\0"),
            "{format}: the symbol table names the entry"
        );
        let out = report(&path, &chain.registry());
        assert!(out.contains("Signed: yes"), "{format}:\n{out}");
        assert_eq!(
            report_line(&out, "  contents: "),
            "  contents: intact",
            "{format}"
        );
    }
}

/// The proof's `issued` is shown only when it is a whole number of seconds.
#[test]
fn an_issued_time_that_is_not_whole_seconds_is_shown_as_none() {
    let mut chain = Chain::new();
    chain.proof = chain
        .proof
        .replace("\"issued\":1000000000", "\"issued\":1.5");
    assert!(chain.proof.contains("\"issued\":1.5"), "{}", chain.proof);
    let lines = signing_lines(&chain.blob(), Some(DIGEST), &chain.registry());
    assert_eq!(signed_line(&lines, "issued"), "<none>");
}

/// A blob sealed with anything but Ed25519 is malformed signing metadata.
#[test]
fn a_signature_type_other_than_ed25519_is_malformed() {
    let chain = Chain::new();
    let blob = String::from_utf8(chain.blob())
        .unwrap()
        .replace("\"signatureType\":\"Ed25519\"", "\"signatureType\":\"RSA\"");
    let lines = signing_lines(blob.as_bytes(), Some(DIGEST), &chain.registry());
    assert_eq!(
        lines,
        vec!["trust chain: NOT VERIFIED (malformed signing metadata: unknown signatureType `RSA`)"]
    );
}

/// A signing key that does not decode leaves the contents unverified, and the
/// chain refuses it too.
#[test]
fn an_undecodable_signing_key_leaves_the_contents_unverified() {
    let mut chain = Chain::new();
    chain.signing_key = "ed25519:not base64!".to_string();
    let (contents, _, trust) = verdicts(&chain.blob(), Some(DIGEST), &chain.registry());
    assert!(contents.starts_with("unverified ("), "{contents}");
    assert!(trust.starts_with("NOT VERIFIED ("), "{trust}");
}

/// A validly signed proof that names another signing key, and one whose ident
/// has no owner prefix, are refused.
#[test]
fn a_proof_naming_another_signing_key_or_an_unowned_ident_is_refused() {
    let mut chain = Chain::new();
    let other = crypto::fingerprint(b"another key");
    chain.proof = chain.proof.replace(&chain.signing_fingerprint, &other);
    assert_eq!(
        trust_chain(&chain.blob(), &chain.registry()),
        "NOT VERIFIED (proof signingFingerprint does not match the signing metadata)"
    );
    let mut chain = Chain::new();
    chain.proof = chain.proof.replace("alice#probe", "aliceprobe");
    assert_eq!(
        trust_chain(&chain.blob(), &chain.registry()),
        "NOT VERIFIED (proof ident does not belong to the signing owner)"
    );
}
