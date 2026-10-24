use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

use crate::binary_repr;
use crate::ir;

pub(crate) use crate::manifest::json_edit::{
    project_json_with_package, project_json_with_updated_ident_key,
    project_json_with_updated_version, project_json_without_packages,
};
pub(crate) use crate::manifest::url::package_file_url_path;

// plan-126-B: the container magic, the fixed-prefix field readers and the
// signature-header rule live in `mfb_wire::mfp`, the one crate the compiler and
// the registry may both depend on. This reader keeps its own *policy* — every
// `limit`, the `required` flag on each field, and `validate_package_name` below
// are stated at the call sites in `read_mfp_header` and are what bug-340 B8
// protects; only the byte-level reading is shared.
use mfb_wire::mfp::{
    read_mfp_bytes, read_mfp_string, read_u16, read_u32, read_u64, validate_signature_header,
    MFP_MAGIC,
};

/// Parsed container v1.0 `.mfp` header (plan-23 §4). The reader is hard
/// v1.0: `containerMajor.containerMinor` must be exactly `1.0`.
pub(crate) struct MfpHeader {
    pub(crate) name: String,
    pub(crate) ident: String,
    pub(crate) version: String,
    pub(crate) author: String,
    pub(crate) url: String,
    pub(crate) ident_key: String,
    pub(crate) signing_key: String,
    pub(crate) proof: String,
    pub(crate) attestation: String,
    pub(crate) package_binary_hash: [u8; 32],
    pub(crate) container_major: u16,
    pub(crate) container_minor: u16,
    pub(crate) binary_repr_major: u16,
    pub(crate) binary_repr_minor: u16,
    pub(crate) flags: u32,
    pub(crate) signature_type: u16,
    pub(crate) signature_length: usize,
    pub(crate) binary_repr_length: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectPackageDependency {
    pub(crate) name: String,
    pub(crate) ident: String,
    pub(crate) version: String,
    pub(crate) pin: bool,
    pub(crate) source: String,
    /// The pinned owner identKey (plan-23 §3.5 trust anchor), captured on
    /// first `pkg add` of a signed package (trust-on-first-use). Empty for
    /// unsigned dependencies.
    pub(crate) ident_key: String,
    /// bug-628: whether the user added this dependency themselves, as opposed
    /// to `mfb pkg` adding it because another declared package imports it.
    /// `None` when the field is absent or not a boolean.
    pub(crate) direct: Option<bool>,
    /// bug-628: the idents of the declared packages whose import tables name
    /// this one. `None` when the field is absent or not an array of strings.
    pub(crate) required_by: Option<Vec<String>>,
}

/// Rejects a package name that cannot be used as a single path component.
///
/// The implementation lives in `mfb_wire::validation` as
/// `validate_path_component_name` (plan-126-C), beside the registry's sibling
/// policy, so the two can be read together and neither can drift unnoticed.
/// Re-exported under the local name so the 16 call sites in this crate are
/// unchanged.
///
/// **This is the path-component policy, not the registry's.** It has no length
/// cap; the registry's adds `PACKAGE_LIMIT`. That difference is the whole of the
/// divergence, and `mfb_wire::validation`'s
/// `the_two_package_name_policies_differ_only_by_a_length_cap` states it
/// executably.
pub(crate) use mfb_wire::validation::validate_path_component_name as validate_package_name;

pub(crate) fn read_mfp_header(path: &Path) -> Result<MfpHeader, String> {
    let bytes =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    if bytes.len() < 20 {
        return Err(format!(
            "'{}' is too small to be a valid .mfp package",
            path.display()
        ));
    }
    if bytes[0..8] != MFP_MAGIC {
        return Err(format!(
            "'{}' does not have the MFP package magic",
            path.display()
        ));
    }

    let container_major = read_u16(&bytes, 8)?;
    let container_minor = read_u16(&bytes, 10)?;
    if container_major != 1 || container_minor != 0 {
        return Err(format!(
            "'{}' uses unsupported MFP container version {container_major}.{container_minor} (expected 1.0)",
            path.display()
        ));
    }
    let binary_repr_major = read_u16(&bytes, 12)?;
    let binary_repr_minor = read_u16(&bytes, 14)?;
    let flags = read_u32(&bytes, 16)?;

    let mut offset = 20usize;
    let name = read_mfp_string(&bytes, &mut offset, "name", 255, true)?;
    validate_package_name(&name)?;
    let ident = read_mfp_string(&bytes, &mut offset, "ident", 255, false)?;
    let version = read_mfp_string(&bytes, &mut offset, "version", 64, true)?;
    let author = read_mfp_string(&bytes, &mut offset, "author", 512, false)?;
    let url = read_mfp_string(&bytes, &mut offset, "url", 2048, false)?;
    let ident_key = read_mfp_string(&bytes, &mut offset, "identKey", 255, false)?;
    let signing_key = read_mfp_string(&bytes, &mut offset, "signingKey", 255, false)?;
    let proof = read_mfp_string(&bytes, &mut offset, "proof", 4096, false)?;
    let _proof_sig = read_mfp_bytes(&bytes, &mut offset, "proofSig", 64)?;
    let attestation = read_mfp_string(&bytes, &mut offset, "attestation", 4096, false)?;
    let _attestation_sig = read_mfp_bytes(&bytes, &mut offset, "attestationSig", 64)?;

    // From here every step adds a small fixed width to `offset`, which never
    // exceeds `bytes.len()` (each field read above bounds-checks it) — and a
    // `Vec` holds at most `isize::MAX` bytes — so none of these additions can
    // overflow, on any target.
    let hash_end = offset + 32;
    if hash_end > bytes.len() {
        return Err("truncated .mfp packageBinaryHash".to_string());
    }
    let mut package_binary_hash = [0u8; 32];
    package_binary_hash.copy_from_slice(&bytes[offset..hash_end]);
    offset = hash_end;

    let binary_repr_length = read_u64(&bytes, offset)?;
    offset += 8;

    let signature_type = read_u16(&bytes, offset)?;
    let signature_length = read_u32(&bytes, offset + 2)? as usize;
    // The signature-type/length rule is the wire format's, owned by `mfb_wire`
    // (bug-340 B8, plan-126-B); this reader shares it rather than re-inlining the
    // same match. It admits a length of 0 or 64 only.
    validate_signature_header(signature_type, signature_length)?;
    offset += 6 + signature_length;
    // The payload is exactly the rest of the file. The declared length is
    // compared in `u64`, its wire width, so a hostile 64-bit value is never
    // narrowed to `usize` (which would truncate it on a 32-bit target and let
    // the truncated value pass): a length no in-memory file can match is
    // simply unequal.
    if offset > bytes.len() || binary_repr_length != (bytes.len() - offset) as u64 {
        return Err("invalid .mfp binary representation length".to_string());
    }

    Ok(MfpHeader {
        name,
        ident,
        version,
        author,
        url,
        ident_key,
        signing_key,
        proof,
        attestation,
        package_binary_hash,
        container_major,
        container_minor,
        binary_repr_major,
        binary_repr_minor,
        flags,
        signature_type,
        signature_length,
        // Equal to the declared length, checked above.
        binary_repr_length: bytes.len() - offset,
    })
}

/// Where a build keeps the compiled interface of every dependency declared by
/// SOURCE DIRECTORY (bug-480 Defect A).
///
/// A source dependency has no installed `packages/<name>.mfp`; the compiler
/// builds it and drops the resulting `.mfp` here, so every consumer of an
/// imported interface — signatures, type defs, resource closers, monomorph
/// overloads, the shape pass and `merge_packages` — reads the one artifact
/// format it already understands. The directory lives under `build/` because it
/// is a build INPUT the compiler derives, not something the user installs, and
/// because `build/` is already ignored by version control.
pub(crate) fn source_package_cache_dir(project_dir: &Path) -> PathBuf {
    project_dir
        .join(crate::os::BUILD_DIR)
        .join(SOURCE_PACKAGE_CACHE_DIR)
}

/// The name of [`source_package_cache_dir`] inside `build/`. Named separately so
/// the build-directory clear can recognize (and keep) it without rebuilding the
/// path.
pub(crate) const SOURCE_PACKAGE_CACHE_DIR: &str = "packages";

/// The `.mfp` carrying package `name`'s exported interface for a build rooted at
/// `project_dir`, whichever dependency form declared it: the installed
/// `packages/<name>.mfp` first, then the source-dependency cache.
///
/// This is the single resolution point every reader must use. Hand-building
/// `project_dir/packages/<name>.mfp` at a read site is what made a source
/// dependency's exports resolve to nothing (bug-480 Defect A). The INSTALL side
/// (`mfb pkg`, `mfb pkg install`, `mfb audit`) is deliberately not a caller: it
/// manages what lives in `packages/`, which is a different question from what
/// this build resolves an import against.
pub(crate) fn resolved_package_file(project_dir: &Path, name: &str) -> Option<PathBuf> {
    // `name` reaches here from a manifest entry and from an `IMPORT` line, and
    // is interpolated into a file name; refuse a traversing spelling before the
    // join, exactly as the install side does.
    if validate_package_name(name).is_err() {
        return None;
    }
    let installed = project_dir.join("packages").join(format!("{name}.mfp"));
    if installed.is_file() {
        return Some(installed);
    }
    let cached = source_package_cache_dir(project_dir).join(format!("{name}.mfp"));
    if cached.is_file() {
        return Some(cached);
    }
    None
}

/// How a `packages` manifest entry names the dependency's contents.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SourceDependency {
    /// Declared by source directory: this is the directory holding its
    /// `project.json` (bug-480). The compiler compiles it into
    /// [`source_package_cache_dir`] before resolving any import against it.
    Directory(PathBuf),
    /// Declared as a compiled `.mfp`, or fetched from a registry. Nothing to
    /// build: the artifact is installed into `packages/` by `mfb pkg install`.
    Compiled,
    /// A `local://` source whose path is not absolute — `IMPORT_LOCAL_PATH_INVALID`.
    LocalPathNotAbsolute,
}

/// Classify one declared dependency's `source` (`13_modules-and-packages.md`,
/// `07_cli-reference.md`).
///
/// - `local:///abs` — the absolute directory holding the package sources;
/// - `file:<rel>` — project-relative; a `<rel>` ending in `.mfp` is the compiled
///   form, anything else is a source directory;
/// - `file://…` — an absolute URL to a `.mfp` (what `mfb pkg add` writes);
/// - anything else, including an absent `source` — the conventional
///   `packages/<name>/` source directory, which is also where an installed
///   `packages/<name>.mfp` would sit. Both are probed: the compiled form wins.
pub(crate) fn source_dependency(
    project_dir: &Path,
    name: &str,
    source: Option<&str>,
) -> SourceDependency {
    let Some(source) = source.filter(|source| !source.is_empty()) else {
        return SourceDependency::Directory(project_dir.join("packages").join(name));
    };
    if let Some(path) = source.strip_prefix("local://") {
        let path = PathBuf::from(path);
        // Rooted, not `is_absolute`. The two agree everywhere except Windows,
        // where `is_absolute` additionally demands a drive prefix — so the
        // spelling the spec documents, `local:///absolute/path` stripping to
        // `/absolute/path` (13_modules-and-packages.md §3), was rejected there
        // as IMPORT_LOCAL_PATH_INVALID and `local://` was unusable in its
        // documented form. `/abs/dep` is a real rooted path on Windows (it
        // resolves against the current drive), and `has_root` still refuses the
        // thing this check exists to refuse: `local://relative/path`, and the
        // drive-relative `C:dep`.
        return if path.has_root() {
            SourceDependency::Directory(path)
        } else {
            SourceDependency::LocalPathNotAbsolute
        };
    }
    if source.starts_with("file://") {
        // An absolute `file://` URL always names a `.mfp` (`package_file_url_path`).
        return SourceDependency::Compiled;
    }
    if let Some(rel) = source.strip_prefix("file:") {
        if rel.ends_with(".mfp") {
            return SourceDependency::Compiled;
        }
        return SourceDependency::Directory(project_dir.join(rel));
    }
    SourceDependency::Compiled
}

pub(crate) fn installed_package_files(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Result<Vec<PathBuf>, String> {
    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Ok(Vec::new());
    };

    packages
        .iter()
        .filter_map(project_package_dependency)
        .map(|dependency| {
            // bug-480: a source-directory dependency has been compiled into the
            // build's package cache by this point, so it resolves here exactly
            // like an installed `.mfp` and needs no second code path below.
            if let Some(package_file) = resolved_package_file(project_dir, &dependency.name) {
                let header = read_mfp_header(&package_file)?;
                if dependency.pin && header.version != dependency.version {
                    return Err(format!(
                        "package `{}` is pinned to version {}, but installed package is version {}",
                        dependency.name, dependency.version, header.version
                    ));
                }
                Ok(package_file)
            } else {
                Err(format!(
                    "package `{}` must be installed as '{}' before binary representation merging",
                    dependency.name,
                    project_dir
                        .join("packages")
                        .join(format!("{}.mfp", dependency.name))
                        .display()
                ))
            }
        })
        .collect()
}

/// The `(resource type, close op)` pairs an importer must register so
/// `ir::verify`'s resource rules recognize an imported package's resources
/// (bug-377).
///
/// A decoded package carries no `native_resources` (`ir/binary/` drops them by
/// contract), so `IrProject::native_resources` holds the *current* project's
/// declarations only. Without this, `close_op_for` returns `None` for an
/// imported type, `is_resource_or_resource_union` is false, and every resource
/// rule — the `RES` ownership axis, double-close, use-after-close — silently
/// skips it.
///
/// The close op is spelled the way the **source** IR calls it, `<binding>.<alias>`
/// with no identity prefix, because verify runs before `merge_packages` applies
/// the content-addressed prefix. `code::validation` registers the *prefixed*
/// spelling of the same op for exactly the opposite reason — it runs after.
///
/// Lossy like [`external_package_function_types`]: a package that cannot be read
/// contributes nothing rather than failing the build, because the package-level
/// errors are reported by `verify_and_report_packages` well before this point.
pub(crate) fn imported_resource_closers(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Vec<ir::ImportedResource> {
    let Ok(packages) = installed_package_files(project_dir, manifest) else {
        return Vec::new();
    };
    resource_closers_from_files(&packages)
}

fn resource_closers_from_files(packages: &[PathBuf]) -> Vec<ir::ImportedResource> {
    let mut closers = Vec::new();
    for package in packages {
        let Ok(decode) = binary_repr::BinaryReprPackageDecode::read(package) else {
            continue;
        };
        let (Ok(package_name), Ok(resources)) = (decode.name(), decode.resources()) else {
            continue;
        };
        for resource in resources {
            // A built-in is authoritative and already seeded; a package's table
            // merely references it (the former source checker's `collect_package_resources`). A
            // referenced builtin may be recorded by its bare base name (`File`) even
            // though the builtin's own identity is package-qualified (`fs.File`,
            // plan-97), so recognize it by bare name.
            if crate::codegen::resource::is_builtin_backed_resource(
                &crate::types::ParameterType::declared(&resource.type_name),
            ) {
                continue;
            }
            let Some(close_function) = resource.close_function else {
                // Unclosable as far as the importer can tell — registering it
                // would claim a close op that does not resolve.
                continue;
            };
            // A native resource serializes its close op as the bare exported
            // alias (plan-link-update.md §5a); the importer calls it
            // `<binding>.<alias>`. Built-in close names are already dotted.
            let close_function = if resource.native && !close_function.contains('.') {
                format!("{package_name}.{close_function}")
            } else {
                close_function
            };
            // bug-632: registered ONLY under its package-qualified identity. Source
            // writes `binding::Type`, which the parser canonicalizes to
            // `<package>.Type`; spec §13 refuses the bare spelling, and the bare
            // row this used to add as well is what let `RES h AS Db` resolve. The
            // sendable bit rides along so verify's thread-boundary rules see an
            // imported resource exactly as the exporting package declared it.
            closers.push(ir::ImportedResource {
                type_name: format!("{package_name}.{}", resource.type_name),
                close_function,
                sendable: resource.sendable,
            });
        }
    }
    closers
}

pub(crate) fn external_package_function_types(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> HashMap<String, ir::ExternalSignature> {
    let Ok(packages) = installed_package_files(project_dir, manifest) else {
        return HashMap::new();
    };
    external_package_function_types_from_files_lossy(&packages)
}

/// bug-390: reject a dependency set whose intermediary packages disagree on the
/// ABI of a shared foreign type. Each package records, per re-exported foreign
/// type, the owning package's ABI hash it was built against. If two importers
/// carry different hashes for the same `owner::type` — or an importer's hash
/// disagrees with the owner the consumer actually resolves — passing a value of
/// that type between them would be a silent type confusion, so the build fails.
pub(crate) fn verify_foreign_type_abi_consistency(packages: &[PathBuf]) -> Result<(), String> {
    let mut seen: HashMap<(String, String), [u8; 32]> = HashMap::new();
    for package in packages {
        let owner_dir = package.parent();
        for fref in binary_repr::read_package_foreign_type_refs(package)? {
            let key = (fref.owner.clone(), fref.name.clone());
            match seen.get(&key) {
                Some(prev) if *prev != fref.abi_hash => {
                    return Err(format!(
                        "imported packages disagree on the ABI of type `{}` from `{}`: they were built against incompatible versions of `{}`",
                        fref.name, fref.owner, fref.owner
                    ));
                }
                Some(_) => {}
                None => {
                    seen.insert(key, fref.abi_hash);
                }
            }
            // When the owner is installed alongside, its current ABI must match
            // what the importer was built against.
            if let Some(dir) = owner_dir {
                // bug-395: `fref.owner` is decoded verbatim from an untrusted
                // dependency `.mfp` and here becomes a filename joined onto that
                // package's directory. A hostile owner (`../../etc/foo`, or an
                // absolute path) would escape the directory — an existence oracle
                // plus an attacker-triggered read of an out-of-tree `.mfp`.
                // Validate it as a bare package name before the join, the same
                // guard `packages/<name>.mfp` names already carry.
                validate_package_name(&fref.owner)?;
                let owner_path = dir.join(format!("{}.mfp", fref.owner));
                if owner_path.is_file() {
                    let owner_hashes = binary_repr::read_package_type_export_hashes(&owner_path)?;
                    if let Some(owner_hash) = owner_hashes.get(&fref.name) {
                        if *owner_hash != fref.abi_hash {
                            return Err(format!(
                                "type `{}` re-exported by an imported package was built against an ABI-incompatible version of `{}`",
                                fref.name, fref.owner
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn external_package_function_types_from_files(
    packages: &[PathBuf],
) -> Result<HashMap<String, ir::ExternalSignature>, String> {
    verify_foreign_type_abi_consistency(packages)?;
    let mut signatures = HashMap::new();
    for package in packages {
        let decode = binary_repr::BinaryReprPackageDecode::read(package)?;
        let package_name = decode.name()?;
        let owners = package_type_owners(&decode, &package_name);
        for export in decode.exports()? {
            signatures.insert(
                format!("{package_name}.{}", export.name),
                package_export_signature(&package_name, &owners, &export),
            );
        }
    }
    Ok(signatures)
}

/// bug-632: the type names a package OWNS — every record, union and enum in its
/// type-export table, each union variant, and each resource type. A package
/// compiles these as local names, so its `.mfp` spells them bare; an importer
/// must see them package-qualified (spec §13), which [`qualify_package_type`]
/// supplies at every read. Lossy like its callers: an unreadable section
/// contributes no names.
/// Which package DECLARES each type name a `.mfp` spells bare, keyed by that
/// name.
///
/// Usually the package being read, but not always: a `.mfp` records a
/// re-exported FOREIGN type by its bare name plus its owner (`foreign_owner`,
/// bug-390), and `dom.Node` re-exported through `worker` is still `dom`'s. A
/// flat "names this package owns" set cannot tell those apart, and qualifying
/// them all with the reading package minted `worker.Node` — a type nothing
/// declares.
///
/// Excluded: a BUILT-IN resource the package merely references (`tls.Listener`,
/// or a bare `File` for `fs.File` — plan-97/bug-441) and any already-qualified
/// spelling (`crypto.Certificate`), neither of which this package declares.
/// Lossy like its callers: an unreadable section contributes no names.
pub(crate) type PackageTypeOwners = HashMap<String, String>;

pub(crate) fn package_type_owners(
    decode: &binary_repr::BinaryReprPackageDecode,
    package: &str,
) -> PackageTypeOwners {
    let mut owners: PackageTypeOwners = HashMap::new();
    if let Ok(exports) = decode.type_exports() {
        for export in exports {
            if !matches!(
                export.kind,
                binary_repr::BinaryReprExportKind::Type
                    | binary_repr::BinaryReprExportKind::Union
                    | binary_repr::BinaryReprExportKind::Enum
            ) {
                continue;
            }
            let owner = export
                .foreign_owner
                .clone()
                .unwrap_or_else(|| package.to_string());
            owners.insert(export.name, owner.clone());
            for variant in export.variants {
                owners.insert(variant.name, owner.clone());
            }
        }
    }
    if let Ok(resources) = decode.resources() {
        for resource in resources {
            if crate::codegen::resource::is_builtin_backed_resource(
                &crate::types::ParameterType::declared(&resource.type_name),
            ) {
                continue;
            }
            owners
                .entry(resource.type_name)
                .or_insert_with(|| package.to_string());
        }
    }
    owners.retain(|name, _| !name.contains('.'));
    owners
}

/// bug-632: `type_` as an importer names it — each nominal becomes
/// `<owner>.<Name>`, at every depth. A name no package in `owners` declares (a
/// scalar, a built-in package's already-qualified type, a type variable) is
/// unchanged. This gives an imported user type the same package-qualified
/// identity bug-480 Phase 4b gave a built-in one.
pub(crate) fn qualify_package_type(
    type_: &crate::types::ParameterType,
    owners: &PackageTypeOwners,
) -> crate::types::ParameterType {
    type_.map_nominals(&|name| owners.get(name).map(|owner| format!("{owner}.{name}")))
}

/// The record/union/enum layouts every imported (non-builtin) package exports,
/// so IR lowering can type accesses to their fields. Reads the installed `.mfp`s
/// named by the manifest; a package whose metadata cannot be read is skipped
/// (a lossy read, like `external_package_function_types`) — the build's own
/// the former source checker pass reports an unreadable dependency. Built-in packages contribute
/// nothing here: their types reach `TypeIndex` through the AST.
pub(crate) fn imported_type_defs(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Vec<ir::ImportedTypeDef> {
    let Ok(packages) = installed_package_files(project_dir, manifest) else {
        return Vec::new();
    };
    imported_type_defs_from_files(&packages)
}

/// The TYPE names each imported (non-builtin) package exports, keyed by package
/// name: every exported record/union/enum, every union variant, and every
/// resource type. Read from the same installed `.mfp`s as
/// [`imported_type_defs`], and lossy in the same way — a package whose metadata
/// cannot be read contributes no names, which leaves a qualified spelling under
/// it alone rather than guessing.
///
/// The parser consumes this to decide whether `pkg::Leaf` in a TYPE position is
/// really one of `pkg`'s types (`FileParser::normalize_qualified_type_name`).
/// Without it the parser de-qualified every `pkg::Leaf` it saw in a type
/// position, so `pkg::NoSuchType` reached name resolution as the bare
/// `NoSuchType` and was reported as an unknown project type — losing the
/// package attribution bug-480 exists to give it.
pub(crate) fn imported_type_names(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> HashMap<String, std::collections::HashSet<String>> {
    let Ok(packages) = installed_package_files(project_dir, manifest) else {
        return HashMap::new();
    };
    let mut names: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for package in &packages {
        let Ok(decode) = binary_repr::BinaryReprPackageDecode::read(package) else {
            continue;
        };
        let Ok(package_name) = decode.name() else {
            continue;
        };
        let entry = names.entry(package_name).or_default();
        if let Ok(exports) = decode.type_exports() {
            for export in exports {
                match export.kind {
                    binary_repr::BinaryReprExportKind::Type
                    | binary_repr::BinaryReprExportKind::Union
                    | binary_repr::BinaryReprExportKind::Enum => {
                        entry.insert(export.name);
                        for variant in export.variants {
                            entry.insert(variant.name);
                        }
                    }
                    // A function is not a type; de-qualifying one would rename
                    // the call target, which is the over-correction the
                    // type-position normalizer exists to avoid.
                    binary_repr::BinaryReprExportKind::Func => {}
                }
            }
        }
        // A resource type is not in the export table (it has its own section),
        // and `RES h AS pkg::Handle` is a type position like any other.
        if let Ok(resources) = decode.resources() {
            for resource in resources {
                entry.insert(resource.type_name);
            }
        }
    }
    names
}

/// The exported top-level `LET`/`MUT` bindings of every imported (non-builtin)
/// package, keyed by the `package.Name` spelling a consumer reads them by
/// (bug-551). Read from the same installed `.mfp`s as [`imported_type_defs`],
/// and lossy in the same way — a package whose metadata cannot be read
/// contributes nothing, which leaves its names untyped rather than guessing.
pub(crate) fn imported_global_defs(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Vec<ir::ImportedGlobal> {
    let Ok(packages) = installed_package_files(project_dir, manifest) else {
        return Vec::new();
    };
    imported_global_defs_from_files(&packages)
}

pub(crate) fn imported_global_defs_from_files(packages: &[PathBuf]) -> Vec<ir::ImportedGlobal> {
    let mut defs = Vec::new();
    for package in packages {
        let Ok(info) = binary_repr::read_package_info(package) else {
            continue;
        };
        let owners = binary_repr::BinaryReprPackageDecode::read(package)
            .map(|decode| package_type_owners(&decode, &info.manifest_name))
            .unwrap_or_default();
        for global in info.globals {
            // Only `EXPORT` crosses a package boundary. `PRIVATE`/`PUBLIC`
            // bindings are in the same table (the writer records visibility in
            // the entry flags rather than omitting the row), and a `PRIVATE`
            // one additionally reaches here under its mangled
            // `#<identity>$Name` spelling — so filtering on the recorded
            // visibility is the rule, not the mangling.
            if global.visibility != "export" {
                continue;
            }
            defs.push(ir::ImportedGlobal {
                name: format!("{}.{}", info.manifest_name, global.name),
                // The `.mfp` GLOBAL table renders the declared type as text;
                // this is where it stops being one, the same boundary
                // `imported_type_field` crosses for a record field.
                type_: qualify_package_type(
                    &crate::types::ParameterType::parse(&global.type_),
                    &owners,
                ),
                mutable: global.mutable,
            });
        }
    }
    defs
}

pub(crate) fn imported_type_defs_from_files(packages: &[PathBuf]) -> Vec<ir::ImportedTypeDef> {
    let mut defs = Vec::new();
    for package in packages {
        let Ok(decode) = binary_repr::BinaryReprPackageDecode::read(package) else {
            continue;
        };
        let (Ok(package_name), Ok(exports)) = (decode.name(), decode.type_exports()) else {
            continue;
        };
        let owners = package_type_owners(&decode, &package_name);
        for export in exports {
            if let Some(def) = imported_type_def(export, &package_name, &owners) {
                defs.push(def);
            }
        }
    }
    defs
}

fn imported_type_field(
    field: binary_repr::BinaryReprTypeField,
    owners: &PackageTypeOwners,
) -> ir::ImportedTypeField {
    ir::ImportedTypeField {
        name: field.name,
        // plan-111-B: boundary #5 — the `.mfp` package entry's type table is
        // text on disk, and this is where it stops being one. bug-632: and
        // where the package's own names gain their package qualifier.
        type_: qualify_package_type(&crate::types::ParameterType::parse(&field.type_), owners),
    }
}

/// One `.mfp` type export as an importer sees it. bug-632: the declared name,
/// each variant's name and every field type are package-qualified, so the
/// consumer's tables (`ir::lower::TypeIndex`, `ir::verify`, the monomorphizer)
/// key `<package>.<Name>` and never collide with a local type of the same bare
/// name.
fn imported_type_def(
    export: binary_repr::BinaryReprTypeExport,
    package: &str,
    owners: &PackageTypeOwners,
) -> Option<ir::ImportedTypeDef> {
    // A re-exported FOREIGN type keeps its OWNER's identity (bug-390): an
    // importer of the intermediary and an importer of the owner must name one
    // type, not two.
    let owner = export
        .foreign_owner
        .clone()
        .unwrap_or_else(|| package.to_string());
    // …and a name that is ALREADY qualified carries its identity itself. A
    // BUILT-IN value type a package re-exports (`regex.Group`, `http.Response`)
    // has no `.mfp` of its own for the foreign-owner resolution to read, so the
    // qualifier is all there is. Prefixing it again minted `regex.regex.Group`,
    // and every consumer of such a package was refused with `PACKAGE_INVALID …
    // exported type 'regex.Group' references unknown type 'regex.Group'`.
    let qualified = |name: &str| {
        if name.contains('.') {
            name.to_string()
        } else {
            format!("{owner}.{name}")
        }
    };
    let kind = match export.kind {
        binary_repr::BinaryReprExportKind::Type => ir::ImportedTypeKind::Record,
        binary_repr::BinaryReprExportKind::Union => ir::ImportedTypeKind::Union,
        binary_repr::BinaryReprExportKind::Enum => ir::ImportedTypeKind::Enum,
        binary_repr::BinaryReprExportKind::Func => {
            return None;
        }
    };
    Some(ir::ImportedTypeDef {
        name: qualified(&export.name),
        kind,
        fields: export
            .fields
            .into_iter()
            .map(|field| imported_type_field(field, owners))
            .collect(),
        variants: export
            .variants
            .into_iter()
            .map(|variant| ir::ImportedTypeVariant {
                name: qualified(&variant.name),
                fields: variant
                    .fields
                    .into_iter()
                    .map(|field| imported_type_field(field, owners))
                    .collect(),
            })
            .collect(),
        members: export.members,
    })
}

fn external_package_function_types_from_files_lossy(
    packages: &[PathBuf],
) -> HashMap<String, ir::ExternalSignature> {
    let mut signatures = HashMap::new();
    for package in packages {
        let Ok(decode) = binary_repr::BinaryReprPackageDecode::read(package) else {
            continue;
        };
        let (Ok(package_name), Ok(exports)) = (decode.name(), decode.exports()) else {
            continue;
        };
        let owners = package_type_owners(&decode, &package_name);
        for export in exports {
            signatures.insert(
                format!("{package_name}.{}", export.name),
                package_export_signature(&package_name, &owners, &export),
            );
        }
    }
    signatures
}

/// Decode one `.mfp` export into a typed [`ir::ExternalSignature`], parsing each
/// stored type string exactly ONCE (plan-105-A).
///
/// This is the single parse-in boundary for imported signatures: the `.mfp` wire
/// format still stores types as strings, and everything downstream — the driver's
/// resource-import filter, `ir::lower`'s `function_returns`/`function_params` — now
/// reads the structure instead of re-splitting a formatted `FUNC(…) AS R` blob.
///
/// bug-632: every type the package owns is package-qualified
/// ([`qualify_package_type`]), so an importer's `ov::A` argument and the
/// signature's parameter name the same type.
fn package_export_signature(
    package: &str,
    owners: &PackageTypeOwners,
    export: &binary_repr::BinaryReprExport,
) -> ir::ExternalSignature {
    let qualify = |type_: &crate::types::ParameterType| qualify_package_type(type_, owners);
    ir::ExternalSignature {
        params: export
            .params
            .iter()
            .map(|param| ir::ExternalFunctionParam {
                name: param.name.clone(),
                type_: qualify(&param.type_),
                default: match &param.default {
                    binary_repr::BinaryReprExportDefault::None => ir::ExternalDefault::None,
                    binary_repr::BinaryReprExportDefault::Literal { type_, value } => {
                        ir::ExternalDefault::Literal {
                            type_: qualify(type_),
                            value: value.clone(),
                        }
                    }
                    // plan-136-B: an importer names a package symbol `package.name`,
                    // which `ir::package::apply_package_identity` rewrites to the
                    // merged, identity-prefixed definition — a private hidden
                    // default function included.
                    binary_repr::BinaryReprExportDefault::Function(name) => {
                        ir::ExternalDefault::Function(format!("{package}.{name}"))
                    }
                },
            })
            .collect(),
        returns: qualify(&export.return_type),
        isolated: export.isolated,
    }
}

pub(crate) fn package_metadata(
    manifest: &HashMap<String, JsonValue>,
) -> binary_repr::BinaryReprMetadata {
    let name = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .expect("validated project name")
        .clone();
    let version = manifest
        .get("version")
        .and_then(|value| value.get::<String>())
        .expect("validated project version")
        .clone();
    let mut metadata = binary_repr::BinaryReprMetadata::new(name, version);
    metadata.ident = manifest
        .get("ident")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    // Identity-chain fields (identKey and the key fingerprints) are outputs
    // of `--sign` (plan-23 §3.3), stamped by apply_signing_metadata — never
    // read from the manifest. An unsigned package carries no identity chain,
    // and the file-embedded key is never a trust root.
    metadata.author = manifest
        .get("author")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    metadata.url = manifest
        .get("url")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    metadata.description = manifest
        .get("description")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    metadata.dependencies = package_dependencies(manifest);
    metadata
}

/// The dependency fields shared by the `.mfp` metadata builder
/// ([`package_dependencies`]) and the project-manifest reader
/// ([`project_package_dependency`]), with the bug-195 name guard applied once.
///
/// Returns `None` for a missing, blank, or path-traversing `name` so both callers
/// drop such an entry identically (bug-340 B9). The two callers differ only in the
/// extra fields they read (`source`/`identKey`) and the struct they build.
struct CommonDependencyFields {
    name: String,
    ident: String,
    version: String,
    pin: bool,
}

fn common_dependency_fields(
    package: &HashMap<String, JsonValue>,
) -> Option<CommonDependencyFields> {
    let name = package.get("name")?.get::<String>()?.clone();
    // The dependency `name` is interpolated into `packages/<name>.mfp`: it both
    // feeds `.mfp` package metadata and is read/merged by `mfb audit`/`build`/`pkg`.
    // Reject a blank name and validate it as a single path component (bug-195) so a
    // `../…` or absolute name cannot escape `packages/` or land unvalidated in
    // `.mfp` metadata. Previously only `project_package_dependency` was guarded;
    // the copy that reaches the wire format was not (bug-340 B9).
    if name.trim().is_empty() || validate_package_name(&name).is_err() {
        return None;
    }
    let ident = package
        .get("ident")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_else(|| name.clone());
    let version = package
        .get("version")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    let pin = package
        .get("pin")
        .and_then(|value| value.get::<bool>())
        .copied()
        .unwrap_or(false);
    Some(CommonDependencyFields {
        name,
        ident,
        version,
        pin,
    })
}

fn package_dependencies(
    manifest: &HashMap<String, JsonValue>,
) -> Vec<binary_repr::BinaryReprDependency> {
    manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(|package| package.get::<HashMap<String, JsonValue>>())
        .filter_map(|package| {
            let common = common_dependency_fields(package)?;
            Some(binary_repr::BinaryReprDependency {
                name: common.name,
                ident: common.ident,
                version: common.version,
                pin: common.pin,
                flags: 0,
            })
        })
        .collect()
}

pub(crate) fn project_package_dependency(value: &JsonValue) -> Option<ProjectPackageDependency> {
    let package = value.get::<HashMap<String, JsonValue>>()?;
    // The shared name/ident/version/pin extraction, including the bug-195 name
    // guard, lives in `common_dependency_fields`; only `source`/`identKey` and the
    // struct built are specific to a project dependency (bug-340 B9).
    let common = common_dependency_fields(package)?;
    let source = package
        .get("source")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    let ident_key = package
        .get("identKey")
        .or_else(|| package.get("ident_key"))
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    let direct = package
        .get("direct")
        .and_then(|value| value.get::<bool>())
        .copied();
    let required_by = package
        .get("requiredBy")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .and_then(|values| {
            values
                .iter()
                .map(|value| value.get::<String>().cloned())
                .collect::<Option<Vec<String>>>()
        });

    Some(ProjectPackageDependency {
        name: common.name,
        ident: common.ident,
        version: common.version,
        pin: common.pin,
        source,
        ident_key,
        direct,
        required_by,
    })
}

#[cfg(test)]
mod tests;
