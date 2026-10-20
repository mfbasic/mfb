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
use mfb_repository::server::{IdentChainLink, IndexResponse, OwnerBinding, TransferLink};

use super::*;
use crate::arch::image::{
    EncodedImage, EncodedImport, EncodedSection, EncodedSymbol, ExecutableSigning, ImportKind,
};
use crate::os::content_signature::CONTENT_SIGNATURE_PLACEHOLDER;
use crate::os::inspect::{compiler, MH_MAGIC_64};
use crate::os::linux::flavor::LinuxFlavor;

mod signed;

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
        debug_symbols: false,
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
    /// The `/index` answers a transferred ident's check reads (plan-168-C),
    /// by `owner#package`; anything else is an unknown package.
    indexes: Vec<IndexResponse>,
}

impl FakeRegistry {
    /// A registry that cannot be reached.
    fn offline() -> Self {
        FakeRegistry {
            server_key: Err("connection refused".to_string()),
            current_ident: Err("connection refused".to_string()),
            chain: Vec::new(),
            indexes: Vec::new(),
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
        assert_eq!(registry, REGISTRY);
        if owner != "alice" {
            // Only the transfer tests ask for another owner's chain, and none
            // of their owners rotated.
            assert!(
                !self.indexes.is_empty(),
                "a chain lookup for `{owner}` outside a transfer test"
            );
            return Ok(Vec::new());
        }
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

    fn index(
        &self,
        registry: &str,
        server_key: &[u8],
        owner: &str,
        package: &str,
    ) -> Result<IndexResponse, String> {
        self.index_for(registry, server_key, owner, package)
    }
}

impl FakeRegistry {
    fn index_for(
        &self,
        registry: &str,
        server_key: &[u8],
        owner: &str,
        package: &str,
    ) -> Result<IndexResponse, String> {
        assert_eq!(registry, REGISTRY);
        assert_eq!(Ok(server_key), self.server_key.as_deref().map_err(|_| ()));
        let ident = format!("{owner}#{package}");
        self.indexes
            .iter()
            .find(|index| index.ident == ident)
            .cloned()
            .ok_or_else(|| "unknown package".to_string())
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

const SIGNED_PLACEHOLDER: &[u8] = b"{\"format\":\"mfb-signing-v1\"}\n";

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
