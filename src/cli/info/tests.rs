//! `mfb info` against images the in-tree linkers actually write.
//!
//! Every positive case links a real image — ELF, PE and Mach-O — through the
//! same `write_linked_executable` a build uses, so a linker that moves the note,
//! the `.mfbsign` section, the import tables, or the range the content signature
//! covers breaks these tests rather than the command. The signed cases build a
//! genuine plan-23 chain from freshly generated keys: an ident key signs the
//! proof, a stand-in registry key signs the attestation, and the one-off signing
//! key — handed to the linker — seals the content signature. The registry is a
//! [`FakeRegistry`] answering from those same keys.

use std::path::{Path, PathBuf};

use mfb_repository::crypto;
use mfb_repository::server::IdentChainLink;

use super::*;
use crate::arch::image::{
    EncodedImage, EncodedImport, EncodedSection, EncodedSymbol, ExecutableSigning, ImportKind,
};
use crate::os::content_signature::CONTENT_SIGNATURE_PLACEHOLDER;
use crate::os::inspect::{compiler, MH_MAGIC_64};
use crate::os::linux::flavor::LinuxFlavor;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The content digest the file-less chain tests sign and check against.
const DIGEST: [u8; 32] = [7; 32];

/// The registry URL every test chain names.
const REGISTRY: &str = "https://registry.test";

fn image(text: Vec<u8>, entry: &str, signing_metadata: Option<ExecutableSigning>) -> EncodedImage {
    EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: entry.to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: entry.to_string(),
        initializers: Vec::new(),
        signing_metadata,
        rpaths: Vec::new(),
    }
}

fn import(library: &str, symbol: &str) -> EncodedImport {
    EncodedImport {
        library: library.to_string(),
        symbol: symbol.to_string(),
        kind: ImportKind::Function,
        version: None,
    }
}

/// One linked image per format, as `(format line, architecture line, path)`.
/// `text_tag` is appended to every image's code, so two calls can produce images
/// that differ only in their contents.
fn linked_images_with(
    dir: &Path,
    signing: Option<&ExecutableSigning>,
    text_tag: u8,
) -> Vec<(&'static str, &'static str, PathBuf)> {
    let signing = || signing.cloned();
    let x86_ret = vec![0xc3, text_tag];
    let arm_ret = vec![0xc0, 0x03, 0x5f, 0xd6, text_tag, 0, 0, 0];
    vec![
        (
            "Format: ELF",
            "Architecture: x86-64",
            crate::os::linux::write_linked_executable(
                &dir.join("elf-x86"),
                "probe",
                "x86_64",
                LinuxFlavor::Glibc,
                &image(x86_ret.clone(), "_start", signing()),
            )
            .expect("link x86-64 ELF"),
        ),
        (
            "Format: ELF",
            "Architecture: aarch64",
            crate::os::linux::write_linked_executable(
                &dir.join("elf-arm"),
                "probe",
                "aarch64",
                LinuxFlavor::Musl,
                &image(arm_ret.clone(), "_start", signing()),
            )
            .expect("link aarch64 ELF"),
        ),
        (
            "Format: PE",
            "Architecture: x86-64",
            crate::os::windows::write_linked_executable(
                &dir.join("pe"),
                "probe",
                &image(x86_ret, "_start", signing()),
                false,
                None,
                None,
            )
            .expect("link PE"),
        ),
        (
            "Format: Mach-O",
            "Architecture: aarch64",
            crate::os::macos::write_linked_executable(
                &dir.join("macho"),
                "probe",
                &image(arm_ret, "_main", signing()),
            )
            .expect("link Mach-O"),
        ),
    ]
}

fn linked_images(
    dir: &Path,
    signing: Option<&ExecutableSigning>,
) -> Vec<(&'static str, &'static str, PathBuf)> {
    linked_images_with(dir, signing, 0)
}

/// A registry answering from fixed results. It records nothing about local
/// state because there is none to record: every answer is keyed by the URL the
/// binary names, which the fake checks.
struct FakeRegistry {
    server_key: Result<Vec<u8>, String>,
    current_ident: Result<Vec<u8>, String>,
    chain: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)>,
}

impl FakeRegistry {
    /// A registry that cannot be reached.
    fn offline() -> Self {
        FakeRegistry {
            server_key: Err("connection refused".to_string()),
            current_ident: Err("connection refused".to_string()),
            chain: Vec::new(),
        }
    }
}

impl RegistryLookup for FakeRegistry {
    fn server_key(&self, registry: &str) -> Result<Vec<u8>, String> {
        assert_eq!(
            registry, REGISTRY,
            "the lookup uses the URL the binary names"
        );
        self.server_key.clone()
    }

    fn current_ident(
        &self,
        registry: &str,
        server_key: &[u8],
        owner: &str,
        package: &str,
    ) -> Result<Vec<u8>, String> {
        assert_eq!(registry, REGISTRY);
        assert_eq!(Ok(server_key), self.server_key.as_deref().map_err(|_| ()));
        assert_eq!((owner, package), ("alice", "probe"));
        self.current_ident.clone()
    }

    fn ident_chain(&self, registry: &str, owner: &str) -> Result<Vec<IdentChainLink>, String> {
        assert_eq!((registry, owner), (REGISTRY, "alice"));
        Ok(self
            .chain
            .iter()
            .map(|(old, new, signature)| IdentChainLink {
                old_key: crypto::encode_bytes(old),
                new_key: crypto::encode_bytes(new),
                signature: crypto::encode_bytes(signature),
                issued: 0,
            })
            .collect())
    }
}

fn report(path: &Path, registry: &dyn RegistryLookup) -> String {
    render(path, &std::fs::read(path).expect("read image"), registry)
}

#[test]
fn an_unsigned_image_reports_format_arch_libraries_compiler_and_unsigned() {
    let dir = tempfile::tempdir().unwrap();
    for (format, arch, path) in linked_images(dir.path(), None) {
        let out = report(&path, &FakeRegistry::offline());
        let mut expected = vec![
            format!("File: {}", path.display()),
            format.to_string(),
            arch.to_string(),
        ];
        if format == "Format: ELF" {
            expected.push("Linking: static".to_string());
        }
        // These images import nothing, so they load no library.
        expected.push("Libraries: none".to_string());
        expected.push(format!("Compiler: mfb {VERSION}"));
        expected.push("Signed: no".to_string());
        assert_eq!(out, expected.join("\n") + "\n", "{format}");
    }
}

/// A file with no provenance note is refused in one line, however close it comes:
/// empty, text, a truncated ELF magic, and a real image whose note owner is one
/// byte off.
#[test]
fn a_file_without_the_note_is_not_a_mfbasic_binary() {
    let expected = format!("{NOT_MFBASIC}\n");
    let mach_o_magic = MH_MAGIC_64.to_le_bytes();
    for bytes in [
        &b""[..],
        &b"hello"[..],
        &b"\x7fELF"[..],
        &b"MZ"[..],
        &mach_o_magic[..],
    ] {
        assert_eq!(
            render(Path::new("x"), bytes, &FakeRegistry::offline()),
            expected
        );
    }
    let dir = tempfile::tempdir().unwrap();
    for (format, _, path) in linked_images(dir.path(), None) {
        let mut bytes = std::fs::read(&path).unwrap();
        let mut renamed = 0;
        for at in 0..bytes.len() - 7 {
            if &bytes[at..at + 7] == b"MFBasic" {
                bytes[at + 6] = b'X';
                renamed += 1;
            }
        }
        assert!(renamed > 0, "{format}: the owner string is in the image");
        assert_eq!(
            render(&path, &bytes, &FakeRegistry::offline()),
            expected,
            "{format}"
        );
    }
}

/// A dynamic ELF names its interpreter.
#[test]
fn a_dynamic_elf_reports_its_interpreter() {
    let dir = tempfile::tempdir().unwrap();
    let mut linked = image(vec![0xc3], "_start", None);
    linked.imports = vec![import("libc.so.6", "exit")];
    let path = crate::os::linux::write_linked_executable(
        dir.path(),
        "dynamic",
        "x86_64",
        LinuxFlavor::Musl,
        &linked,
    )
    .expect("link dynamic ELF");
    let out = report(&path, &FakeRegistry::offline());
    assert!(
        out.contains("Linking: dynamic (interpreter /lib/ld-musl-x86_64.so.1)\n"),
        "{out}"
    );
}

/// A dynamic ELF lists its `DT_NEEDED` libraries and its `DT_RUNPATH`, read
/// through `DT_STRTAB` the way the loader resolves them.
#[test]
fn a_dynamic_elf_reports_its_libraries_and_search_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut linked = image(vec![0xc3], "_start", None);
    linked.imports = vec![import("libc.so.6", "exit"), import("libm.so.6", "sqrt")];
    linked.rpaths = vec!["$ORIGIN/vendor".to_string()];
    let path = crate::os::linux::write_linked_executable(
        dir.path(),
        "needs",
        "x86_64",
        LinuxFlavor::Glibc,
        &linked,
    )
    .expect("link dynamic ELF");
    let out = report(&path, &FakeRegistry::offline());
    assert!(
        out.contains(
            "Libraries:\n  libc.so.6\n  libm.so.6\nSearch paths:\n  $ORIGIN/vendor\nCompiler: "
        ),
        "{out}"
    );
}

/// A Mach-O lists the install names of its `LC_LOAD_DYLIB` commands and its
/// `LC_RPATH` search paths.
#[test]
fn a_mach_o_reports_its_dylibs_and_search_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut linked = image(vec![0xc0, 0x03, 0x5f, 0xd6], "_main", None);
    linked.imports = vec![import("libSystem", "_exit"), import("libz", "_crc32")];
    linked.rpaths = vec!["@loader_path/vendor".to_string()];
    let path = crate::os::macos::write_linked_executable(dir.path(), "dylibs", &linked)
        .expect("link Mach-O");
    let out = report(&path, &FakeRegistry::offline());
    assert!(
        out.contains(
            "Architecture: aarch64\nLibraries:\n  /usr/lib/libSystem.B.dylib\n  /usr/lib/libz.1.dylib\n\
             Search paths:\n  @loader_path/vendor\nCompiler: "
        ),
        "{out}"
    );
}

/// A PE lists the DLLs its import directory names, in import order.
#[test]
fn a_pe_reports_its_import_dlls() {
    let dir = tempfile::tempdir().unwrap();
    let mut linked = image(vec![0xc3], "_start", None);
    linked.imports = vec![
        import("kernel32.dll", "ExitProcess"),
        import("ws2_32.dll", "WSAStartup"),
        import("kernel32.dll", "GetStdHandle"),
    ];
    let path = crate::os::windows::write_linked_executable(
        dir.path(),
        "imports",
        &linked,
        false,
        None,
        None,
    )
    .expect("link PE");
    let out = report(&path, &FakeRegistry::offline());
    assert!(
        out.contains("Architecture: x86-64\nLibraries:\n  kernel32.dll\n  ws2_32.dll\nCompiler: "),
        "{out}"
    );
}

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

const SIGNED_PLACEHOLDER: &[u8] = b"{\"format\":\"mfb-signing-v1\"}\n";

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
        let (ident_public, ident_private) = crypto::generate_keypair();
        let (signing_public, signing_private) = crypto::generate_keypair();
        let (server_public, server_private) = crypto::generate_keypair();
        let ident_fingerprint = crypto::fingerprint(&ident_public);
        let signing_fingerprint = crypto::fingerprint(&signing_public);
        let proof = serde_json::json!({
            "owner": "alice",
            "ident": "alice#probe",
            "version": "1.2.3",
            "identFingerprint": ident_fingerprint,
            "signingFingerprint": signing_fingerprint,
            "issued": 1_000_000_000,
        })
        .to_string();
        let attestation = serde_json::json!({
            "repoFingerprint": crypto::fingerprint(&server_public),
            "owner": "alice",
            "ident": "alice#probe",
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
            owner: "alice".to_string(),
            author: "alice".to_string(),
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

#[test]
fn malformed_signing_metadata_is_reported_as_such() {
    for (blob, reason) in [
        (&b"\xff\xfe"[..], "not UTF-8"),
        (&b"[]"[..], "signing metadata is not a JSON object"),
        (
            &b"{\"format\":\"mfb-signing-v2\"}"[..],
            "unknown format `mfb-signing-v2`",
        ),
        (SIGNED_PLACEHOLDER, "missing `signatureType`"),
    ] {
        assert_eq!(
            signing_lines(blob, None, &FakeRegistry::offline()),
            vec![format!(
                "trust chain: NOT VERIFIED (malformed signing metadata: {reason})"
            )]
        );
    }
}

/// Nesting deep enough to overflow a recursive parser is refused before parsing
/// (bug-398): the blob and the proof inside it are both untrusted.
#[test]
fn pathologically_nested_json_is_refused_without_recursing() {
    let nested = "[".repeat(120_000) + &"]".repeat(120_000);
    assert_eq!(
        signing_lines(nested.as_bytes(), None, &FakeRegistry::offline()),
        vec!["trust chain: NOT VERIFIED (malformed signing metadata: signing metadata is not a JSON object)".to_string()]
    );
    let blob = format!(
        "{{\"format\":\"mfb-signing-v1\",\"signatureType\":\"Ed25519\",\"owner\":\"a\",\"author\":\"a\",\
         \"identKey\":\"\",\"identFingerprint\":\"\",\"signingKey\":\"\",\"signingFingerprint\":\"\",\
         \"proof\":{},\"proofSignature\":\"\",\"attestation\":\"\",\"attestationSignature\":\"\"}}",
        serde_json::Value::from(nested.as_str())
    );
    assert_eq!(
        signing_lines(blob.as_bytes(), None, &FakeRegistry::offline()),
        vec![
            "trust chain: NOT VERIFIED (malformed signing metadata: proof is not a JSON object)"
                .to_string()
        ]
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

#[test]
fn a_descriptor_of_an_unknown_version_names_no_compiler() {
    let mut descriptor = crate::os::note::mfb_note_descriptor();
    assert_eq!(compiler(&descriptor), Some(format!("mfb {VERSION}")));
    descriptor[4] = 2;
    assert_eq!(
        compiler(&descriptor),
        Some("unknown (marker descriptor version 2)".to_string())
    );
    descriptor[0] = b'X';
    assert_eq!(compiler(&descriptor), None);
    assert_eq!(compiler(b"MFB1"), None);
}

#[test]
fn format_utc_renders_civil_dates() {
    assert_eq!(format_utc(0), "1970-01-01 00:00:00 UTC");
    assert_eq!(format_utc(-1), "1969-12-31 23:59:59 UTC");
    assert_eq!(format_utc(951_782_400), "2000-02-29 00:00:00 UTC");
    assert_eq!(format_utc(1_000_000_000), "2001-09-09 01:46:40 UTC");
    // The extremes an untrusted `issued` can carry do not overflow.
    format_utc(i64::MIN);
    format_utc(i64::MAX);
}
