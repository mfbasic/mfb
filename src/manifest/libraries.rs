//! The `libraries` project.json section: per-platform native-library locators
//! (plan-46-A).
//!
//! A binding package maps each `LINK` **logical library name** (e.g. `sqlite3`)
//! to an ordered list of locators saying which concrete shared object to load for
//! a given `os` / `arch` / `libc`, and whether it is a **system** library (found
//! by the dynamic loader) or a **vendor** library (a file the author ships in
//! `<project root>/vendor/`).
//!
//! ```json
//! "libraries": {
//!   "sqlite3": [
//!     { "os": "macos", "type": "system", "source": "libsqlite3.dylib" },
//!     { "os": "linux", "type": "system", "source": "libsqlite3.so.0" },
//!     { "os": "linux", "arch": "riscv64", "libc": "musl", "source": "libsqlite3-riscv64-musl.so" }
//!   ]
//! }
//! ```
//!
//! This module is the **parse + in-memory model** half. It parses leniently,
//! assuming [`crate::manifest::validate_libraries`] has already run and rejected
//! malformed input; that split keeps the strict schema walk (which needs a source
//! path and diagnostics) out of the accessor.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use tinyjson::JsonValue;

use crate::binary_repr::{NativeLibraryEntry, NativeLibraryLocator, NativeLibraryTable};

/// The libc axis of a Linux locator. macOS has no libc axis at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Libc {
    Glibc,
    Musl,
}

impl Libc {
    /// The manifest token for this flavor.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Glibc => "glibc",
            Self::Musl => "musl",
        }
    }

    /// Parse a manifest `libc` token. `None` for any unrecognized token —
    /// validation rejects those before the accessor ever sees them.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "glibc" => Some(Self::Glibc),
            "musl" => Some(Self::Musl),
            _ => None,
        }
    }
}

/// Where a located library comes from.
///
/// Defaults to [`LibType::Vendor`] because that **fails closed** (plan-46-A
/// §3.1): a missing or typo'd `type` resolves to `<root>/vendor/<source>` and
/// hard-errors at build time when the file is absent. Under a `system` default
/// the same mistake would silently hand `source` to the dynamic loader, which
/// would load whatever it found under that name — a wrong-library load visible
/// only at runtime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LibType {
    /// Ask the dynamic loader for this soname.
    System,
    /// Load this exact file, shipped by the author in `<project root>/vendor/`.
    #[default]
    Vendor,
}

impl LibType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Vendor => "vendor",
        }
    }

    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "system" => Some(Self::System),
            "vendor" => Some(Self::Vendor),
            _ => None,
        }
    }
}

/// One platform locator for a logical library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryLocator {
    /// Canonical `BuildTarget.os` token: `macos` | `linux`.
    pub os: String,
    /// `None` = any arch; else a canonical `BuildTarget.arch` token.
    pub arch: Option<String>,
    /// `None` = any libc. Linux only — macOS has no libc axis.
    pub libc: Option<Libc>,
    /// The JSON key is `type` (a Rust keyword, hence the field rename).
    pub lib_type: LibType,
    /// A bare filename, never a path (plan-46-A §4.2). For a `system` locator
    /// this is the exact soname handed to the loader; for a `vendor` locator the
    /// file lives at `<project root>/vendor/<source>`.
    pub source: String,
}

/// Whether `source` is a bare filename (plan-46-A §4.2): it names a file, never a
/// location. `Err` carries the specific reason, phrased to complete the sentence
/// "field `source` ...".
///
/// This is the **single owner** of the rule. The manifest validator applies it to
/// author input, and the `.mfp` section-10 decoder re-applies it to what it reads
/// back — the compiled package is an untrusted input on the consumer side, and
/// `source` feeds both a `dlopen` C string and a `vendor/` path join. Two copies
/// of this rule would drift, and a decoder that trusted the producer would be the
/// hole.
pub fn source_is_bare(source: &str) -> Result<(), String> {
    if source.is_empty() {
        return Err("must not be blank.".to_string());
    }
    if source.contains('\0') {
        // `source` is emitted verbatim as a C string into the binary by
        // plan-46-C, so an interior NUL would silently truncate the dlopen
        // argument.
        return Err(
            "contains a NUL byte. It is emitted as a C string into the binary, so a NUL would \
             silently truncate the library name."
                .to_string(),
        );
    }
    if let Some(separator) = source.chars().find(|c| *c == '/' || *c == '\\') {
        return Err(format!(
            "contains a path separator (`{separator}`) — it must be a bare filename. A `vendor` \
             locator's file is always resolved at `<project root>/vendor/<source>`, and a \
             `system` locator's `source` is the soname handed to the dynamic loader."
        ));
    }
    if source == "." || source == ".." {
        return Err(format!(
            "is `{source}`, which is a directory reference, not a filename."
        ));
    }
    // Reject a Windows drive prefix now so plan-47 does not inherit a hole.
    let bytes = source.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(format!(
            "carries a drive prefix (`{}:`) — it must be a bare filename, not a path.",
            bytes[0] as char
        ));
    }
    Ok(())
}

/// Parse the `libraries` section into its locators, keyed by logical library
/// name.
///
/// Returns a [`BTreeMap`] so the key order is deterministic — plan-46-B encodes
/// this table into the `.mfp`, and the repo holds a byte-identical self-diff
/// gate. Absent key → empty map (the section is optional).
///
/// Lenient by construction, mirroring `package_dependencies`: a malformed entry
/// is skipped rather than reported, because
/// [`crate::manifest::validate_libraries`] has already rejected the build for it.
pub fn project_libraries(
    manifest: &HashMap<String, JsonValue>,
) -> BTreeMap<String, Vec<LibraryLocator>> {
    let mut libraries = BTreeMap::new();

    let Some(section) = manifest
        .get("libraries")
        .and_then(|value| value.get::<HashMap<String, JsonValue>>())
    else {
        return libraries;
    };

    for (logical, value) in section {
        let locators: Vec<LibraryLocator> = value
            .get::<Vec<JsonValue>>()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get::<HashMap<String, JsonValue>>())
            .filter_map(parse_locator)
            .collect();
        if !locators.is_empty() {
            libraries.insert(logical.clone(), locators);
        }
    }

    libraries
}

/// Parse one locator object. `None` when a required field is missing or
/// mistyped — validation has already rejected that case.
fn parse_locator(entry: &HashMap<String, JsonValue>) -> Option<LibraryLocator> {
    let os = entry.get("os")?.get::<String>()?.trim().to_string();
    let source = entry.get("source")?.get::<String>()?.trim().to_string();
    if os.is_empty() || source.is_empty() {
        return None;
    }

    let arch = entry
        .get("arch")
        .and_then(|value| value.get::<String>())
        .map(|arch| arch.trim().to_string());

    let libc = entry
        .get("libc")
        .and_then(|value| value.get::<String>())
        .and_then(|token| Libc::from_token(token.trim()));

    // An absent `type` takes the `Vendor` default (§3.1).
    let lib_type = match entry.get("type").and_then(|value| value.get::<String>()) {
        Some(token) => LibType::from_token(token.trim())?,
        None => LibType::default(),
    };

    Some(LibraryLocator {
        os,
        arch,
        libc,
        lib_type,
        source,
    })
}

/// One supported target slot `(os, arch, libc)` the coverage check tests a
/// library's locators against (plan-46-B §4.2).
///
/// `libc` is `None` on macOS, which has no libc axis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetSlot {
    pub os: String,
    pub arch: String,
    pub libc: Option<Libc>,
}

impl std::fmt::Display for TargetSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.libc {
            Some(libc) => write!(f, "{}/{}/{}", self.os, self.arch, libc.as_str()),
            None => write!(f, "{}/{}", self.os, self.arch),
        }
    }
}

/// Every `(os, arch, libc)` the compiler can build for (plan-46-B §4.2).
///
/// Derived from the backend registry crossed with the libc axis (linux only), not
/// hardcoded, so registering a backend widens the matrix for free.
pub fn supported_target_slots() -> Vec<TargetSlot> {
    let mut slots = Vec::new();
    for target in crate::target::registered_targets() {
        if target.os == "linux" {
            for libc in [Libc::Glibc, Libc::Musl] {
                slots.push(TargetSlot {
                    os: target.os.clone(),
                    arch: target.arch.clone(),
                    libc: Some(libc),
                });
            }
        } else {
            slots.push(TargetSlot {
                os: target.os.clone(),
                arch: target.arch.clone(),
                libc: None,
            });
        }
    }
    slots
}

/// Whether `locator` covers `slot`.
///
/// `arch: None` covers every arch of its `os` and `libc: None` covers both
/// flavors — the two axes are symmetric wildcards (plan-46-A §4.1), so one
/// `{ "os": "linux", "type": "system", … }` entry covers all six Linux slots.
pub fn locator_covers(locator: &LibraryLocator, slot: &TargetSlot) -> bool {
    if locator.os != slot.os {
        return false;
    }
    if let Some(arch) = &locator.arch {
        if arch != &slot.arch {
            return false;
        }
    }
    if let Some(libc) = locator.libc {
        // On macOS the libc axis does not exist; validation rejects `libc` there,
        // so a Some(libc) locator can only be Linux.
        if slot.libc != Some(libc) {
            return false;
        }
    }
    true
}

/// A problem found while assembling the native library table (plan-46-B §4.3).
///
/// Carries the rule name and a message; the caller renders them. As with
/// [`crate::manifest::check_libraries`], keeping assembly pure makes the checks
/// testable by message rather than by scraping stderr.
#[derive(Debug, PartialEq, Eq)]
pub struct NativeLibraryFinding {
    pub rule: &'static str,
    pub message: String,
}

/// Assemble the `NATIVE_LIBRARY_TABLE` for a binding package (plan-46-B §4.3).
///
/// `linked` is the set of distinct `LINK "<name>"` logical names in the project
/// IR. The table carries **only** those names: an unused `libraries` entry is
/// warned about but never encoded, so the section stays minimal and never carries
/// a locator nothing can reach.
///
/// Runs, in order: the missing-entry error, the vendor sha256 (hard error when
/// the file is missing or unreadable), the per-slot coverage warning, and the
/// unused-entry warning. Returns the table alongside every finding; the caller
/// decides whether the findings are fatal (`rules::is_error`).
pub fn build_native_library_table(
    manifest: &HashMap<String, JsonValue>,
    linked: &[String],
    project_root: &Path,
) -> (NativeLibraryTable, Vec<NativeLibraryFinding>) {
    let libraries = project_libraries(manifest);
    let mut findings = Vec::new();
    let mut entries = Vec::new();

    // Distinct linked names, in a deterministic order — the encoded table is
    // sorted by logical name and the repo holds a byte-identical self-diff gate.
    let mut names: Vec<&String> = linked.iter().collect();
    names.sort();
    names.dedup();

    for logical in &names {
        let Some(locators) = libraries.get(*logical) else {
            // The "error if `LINK <logical_name>` is not listed in libraries"
            // requirement.
            findings.push(NativeLibraryFinding {
                rule: "NATIVE_LIBRARY_MISSING",
                message: format!(
                    "`LINK \"{logical}\"` has no `libraries` entry in project.json. Add one \
                     naming the library to load per platform, for example: \
                     \"libraries\": {{ \"{logical}\": [ {{ \"os\": \"linux\", \"type\": \
                     \"system\", \"source\": \"lib{logical}.so.0\" }} ] }}."
                ),
            });
            continue;
        };

        let mut encoded = Vec::new();
        for locator in locators {
            let hash = match locator.lib_type {
                LibType::System => None,
                LibType::Vendor => {
                    let path = vendor_path(project_root, &locator.source);
                    match sha256_file(&path) {
                        Ok(hash) => Some(hash),
                        Err(reason) => {
                            findings.push(NativeLibraryFinding {
                                rule: "NATIVE_LIBRARY_SOURCE_UNREADABLE",
                                message: format!(
                                    "`libraries.{logical}` declares vendor library \
                                     \"{}\", but {} could not be read to hash it: {reason}. Place \
                                     the file there, or declare the locator as \
                                     `\"type\": \"system\"` if the loader should find it.",
                                    locator.source,
                                    path.display()
                                ),
                            });
                            continue;
                        }
                    }
                }
            };
            encoded.push(NativeLibraryLocator {
                os: locator.os.clone(),
                arch: locator.arch.clone(),
                libc: locator.libc,
                lib_type: locator.lib_type,
                source: locator.source.clone(),
                hash,
            });
        }

        // Coverage: one warning per supported slot no locator reaches.
        for slot in supported_target_slots() {
            if !locators
                .iter()
                .any(|locator| locator_covers(locator, &slot))
            {
                findings.push(NativeLibraryFinding {
                    rule: "NATIVE_LIBRARY_TARGET_UNCOVERED",
                    message: format!(
                        "`libraries.{logical}` has no locator covering {slot}. An executable \
                         built for that target will fail to resolve the library."
                    ),
                });
            }
        }

        entries.push(NativeLibraryEntry {
            logical: (*logical).clone(),
            locators: encoded,
        });
    }

    // Dead config that looks authoritative is worth a line of output: a renamed
    // `LINK`, a removed binding, or a typo in the `libraries` key.
    for logical in libraries.keys() {
        if !names.contains(&logical) {
            findings.push(NativeLibraryFinding {
                rule: "NATIVE_LIBRARY_UNUSED",
                message: format!(
                    "`libraries.{logical}` has no matching `LINK \"{logical}\"` in this package's \
                     code, so it is ignored. Remove it, or correct the name to match the LINK \
                     block."
                ),
            });
        }
    }

    (NativeLibraryTable { entries }, findings)
}

/// The on-disk location of a `vendor` locator's file: `<root>/vendor/<source>`,
/// flat, with no subdirectories. The path is never spelled in the manifest — it
/// is always derived, so it cannot disagree with the rule.
///
/// Safe to join because `source` is validated as a bare filename (§4.2) on the
/// author side and re-validated on decode, so it can never escape `vendor/`.
pub fn vendor_path(project_root: &Path, source: &str) -> std::path::PathBuf {
    project_root.join("vendor").join(source)
}

/// The per-package directory a downloaded imported-binding vendor file lands in
/// (plan-48-B §4.3): `<project>/packages/<declaring-unit>.vendor/`, a sibling of
/// `packages/<name>.mfp`. Deliberately not `<project>/vendor/` (which belongs to
/// the consumer's own `libraries` section — an imported blob must never overwrite
/// a file the user placed there) and per-package rather than flat (two packages
/// may each vendor a same-named file with different bytes; §5).
pub fn imported_vendor_dir(project_root: &Path, declaring_unit: &str) -> std::path::PathBuf {
    project_root
        .join("packages")
        .join(format!("{declaring_unit}.vendor"))
}

/// The full path of one imported-binding vendor file (plan-48-B §4.3). `source`
/// is a validated bare filename, so it cannot escape the `.vendor` directory.
pub fn imported_vendor_path(
    project_root: &Path,
    declaring_unit: &str,
    source: &str,
) -> std::path::PathBuf {
    imported_vendor_dir(project_root, declaring_unit).join(source)
}

/// sha256 a file, streamed in bounded chunks — a vendored `.so` can be tens of
/// megabytes, so it must not be read whole into memory.
pub fn sha256_file(path: &Path) -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    const CHUNK: usize = 64 * 1024;

    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&hasher.finalize());
    Ok(hash)
}

#[cfg(test)]
mod tests;
