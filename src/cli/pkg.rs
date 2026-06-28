use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

use crate::binary_repr;
use crate::doc;
use crate::manifest::package::{
    package_file_url_path, project_json_with_package, project_package_dependency, read_mfp_header,
    ProjectPackageDependency,
};
use crate::manifest::{parse_project_json, project_kind, validate_packages_array, validate_project_manifest};
use crate::target;
use crate::USAGE;

use super::build::{build_project, BuildOptions, BuildOutput};

pub(crate) enum PkgCommandError {
    Usage(String),
    Failed(String),
}

pub(crate) fn run_pkg_command(args: &[String]) -> Result<(), PkgCommandError> {
    match args {
        [command, url] if command == "add" => {
            add_package(Path::new("."), url).map_err(PkgCommandError::Failed)
        }
        [command, package] if command == "info" => {
            print_package_info(Path::new(package)).map_err(PkgCommandError::Failed)
        }
        [command, rest @ ..] if command == "doc" => run_pkg_doc(rest),
        [command] if command == "verify" => {
            verify_packages(Path::new(".")).map_err(PkgCommandError::Failed)
        }
        [command, owner, package] if command == "publish" => {
            publish_package_project(owner, Path::new(package)).map_err(PkgCommandError::Failed)
        }
        [command, ..] if command == "add" => Err(PkgCommandError::Usage(format!(
            "mfb pkg add requires exactly one <url>\n\n{USAGE}"
        ))),
        [command, ..] if command == "info" => Err(PkgCommandError::Usage(format!(
            "mfb pkg info requires exactly one <package>\n\n{USAGE}"
        ))),
        [command, ..] if command == "verify" => Err(PkgCommandError::Usage(format!(
            "mfb pkg verify accepts no arguments\n\n{USAGE}"
        ))),
        [command, ..] if command == "publish" => Err(PkgCommandError::Usage(format!(
            "mfb pkg publish requires <owner_name> <package>\n\n{USAGE}"
        ))),
        [] => Err(PkgCommandError::Usage(format!(
            "mfb pkg requires a subcommand\n\n{USAGE}"
        ))),
        [command, ..] => Err(PkgCommandError::Usage(format!(
            "unknown pkg command `{command}`\n\n{USAGE}"
        ))),
    }
}

fn publish_package_project(owner: &str, project_dir: &Path) -> Result<(), String> {
    let project_path = project_dir.join("project.json");
    let manifest = validate_project_manifest(&project_path)
        .map_err(|_| "package project validation failed".to_string())?;
    if project_kind(&manifest) != "package" {
        return Err("mfb pkg publish requires a package project".to_string());
    }

    build_project(&BuildOptions {
        location: project_dir.to_path_buf(),
        output: BuildOutput::Validate,
        target: target::BuildTarget::host(),
        sign_owner: Some(owner.to_string()),
        app_mode: false,
        regalloc: target::shared::code::regalloc::active_kind(),
    })
    .map_err(|_| "package build failed".to_string())?;

    let package_name = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .expect("validated project name");
    let package_path = project_dir.join(format!("{package_name}.mfp"));
    let artifact = fs::read(&package_path).map_err(|err| {
        format!(
            "failed to read built package '{}': {err}",
            package_path.display()
        )
    })?;
    let package = mfb_repository::package::parse_mfp_package(&artifact).map_err(|err| {
        format!(
            "failed to verify built package '{}': {err}",
            package_path.display()
        )
    })?;
    binary_repr::read_package_info(&package_path)?;

    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = mfb_repository::local::LocalPaths::from_env()?;
    let content_hash = package.content_hash_hex();
    let artifact_request = mfb_repository::client::PackageArtifact {
        ident: &package.ident,
        version: &package.version,
        artifact: &artifact,
        content_hash: &content_hash,
        ident_fingerprint: &package.ident_fingerprint,
        signing_fingerprint: &package.signing_fingerprint,
    };

    let report =
        mfb_repository::client::validate_package(&repo_url, &paths, owner, &artifact_request)?;
    print_publish_verify_report(&report);
    if !report.valid {
        return Err("package validation failed".to_string());
    }

    let response =
        mfb_repository::client::publish_package(&repo_url, &paths, owner, &artifact_request)?;
    println!(
        "Published {}@{} as {}",
        response.ident, response.version, response.hash
    );
    Ok(())
}

fn print_publish_verify_report(report: &mfb_repository::server::ValidatePackageResponse) {
    println!("Package validation report:");
    println!("  valid: {}", report.valid);
    println!("  content hash: {}", empty_marker(&report.content_hash));
    println!("  diagnostics:");
    if report.diagnostics.is_empty() {
        println!("    <none>");
    } else {
        for diagnostic in &report.diagnostics {
            println!("    {diagnostic}");
        }
    }
}

fn add_package(project_dir: &Path, url: &str) -> Result<(), String> {
    let source_path = package_file_url_path(url)?;
    let package = read_mfp_header(&source_path)?;

    let project_path = project_dir.join("project.json");
    let contents = fs::read_to_string(&project_path)
        .map_err(|err| format!("failed to read '{}': {err}", project_path.display()))?;
    let manifest = parse_project_json(&contents, &project_path)?;
    validate_packages_array(&manifest)?;

    let package_filename = format!("{}.mfp", package.name);
    let dependency = ProjectPackageDependency {
        name: package.name.clone(),
        ident: package.ident.clone(),
        version: package.version.clone(),
        pin: true,
        source: url.to_string(),
    };
    let updated = project_json_with_package(&contents, &manifest, &dependency)?;

    let packages_dir = project_dir.join("packages");
    fs::create_dir_all(&packages_dir)
        .map_err(|err| format!("failed to create '{}': {err}", packages_dir.display()))?;

    let destination = packages_dir.join(&package_filename);
    fs::copy(&source_path, &destination).map_err(|err| {
        format!(
            "failed to copy '{}' to '{}': {err}",
            source_path.display(),
            destination.display()
        )
    })?;

    fs::write(&project_path, updated)
        .map_err(|err| format!("failed to write '{}': {err}", project_path.display()))?;

    println!(
        "Added package {} {} to {}",
        package.name,
        package.version,
        project_path.display()
    );
    Ok(())
}

fn verify_packages(project_dir: &Path) -> Result<(), String> {
    let project_path = project_dir.join("project.json");
    let contents = fs::read_to_string(&project_path)
        .map_err(|err| format!("failed to read '{}': {err}", project_path.display()))?;
    let manifest = parse_project_json(&contents, &project_path)?;
    validate_packages_array(&manifest)?;

    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Ok(());
    };

    for package in packages {
        let Some(dependency) = project_package_dependency(package) else {
            println!("<invalid> @ <invalid> : Invalid Package");
            continue;
        };
        let result = verify_package_dependency(project_dir, &dependency);
        println!("{}", package_verify_line(&dependency, &result));
    }

    Ok(())
}

/// `mfb pkg doc <name-or-path> [--out <file>]` — render HTML from a compiled
/// package's doc section (plan-09-doc.md §6.2).
fn run_pkg_doc(args: &[String]) -> Result<(), PkgCommandError> {
    let mut target: Option<String> = None;
    let mut out: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                let file = args.get(index).ok_or_else(|| {
                    PkgCommandError::Usage("mfb pkg doc --out requires a file".to_string())
                })?;
                out = Some(file.clone());
            }
            flag if flag.starts_with("--") => {
                return Err(PkgCommandError::Usage(format!("unknown flag `{flag}`")));
            }
            value => {
                if target.is_some() {
                    return Err(PkgCommandError::Usage(
                        "mfb pkg doc accepts exactly one <name-or-path>".to_string(),
                    ));
                }
                target = Some(value.to_string());
            }
        }
        index += 1;
    }
    let target = target.ok_or_else(|| {
        PkgCommandError::Usage(format!("mfb pkg doc requires <name-or-path>\n\n{USAGE}"))
    })?;
    let out_path = out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("doc.html"));
    write_package_doc(&target, &out_path).map_err(PkgCommandError::Failed)
}

/// Resolve a package name or path to its `.mfp`, decode the doc section, and
/// render it. A package with no doc section yields a minimal page (exit 0).
fn write_package_doc(target: &str, out_path: &Path) -> Result<(), String> {
    let direct = Path::new(target);
    let package_path = if target.ends_with(".mfp") || direct.is_file() {
        direct.to_path_buf()
    } else {
        let candidate = Path::new("packages").join(format!("{target}.mfp"));
        if candidate.is_file() {
            candidate
        } else {
            return Err(format!(
                "no package named `{target}` found (looked for '{}')",
                candidate.display()
            ));
        }
    };

    let header = read_mfp_header(&package_path)?;
    let docs = binary_repr::read_package_docs(&package_path)?;
    let html = if docs.is_empty() {
        doc::render_empty_html(&header.name)
    } else {
        let page = doc::from_package(docs, &header.name);
        doc::render_html(&page)
    };
    fs::write(out_path, html)
        .map_err(|err| format!("failed to write '{}': {err}", out_path.display()))?;
    println!("Wrote documentation to {}", out_path.display());
    Ok(())
}

fn print_package_info(path: &Path) -> Result<(), String> {
    let package_bytes =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let content_hash = target::package_mfp::package_content_hash(&package_bytes)?;
    let header = read_mfp_header(path)?;
    let info = binary_repr::read_package_info(path)?;

    println!("Package: {}", header.name);
    println!("Ident: {}", empty_marker(&header.ident));
    println!("Version: {}", header.version);
    println!("Ident Key: {}", empty_marker(&header.ident_key));
    println!(
        "Ident Fingerprint: {}",
        empty_marker(&header.ident_fingerprint)
    );
    println!(
        "Signing Fingerprint: {}",
        empty_marker(&header.signing_fingerprint)
    );
    println!("Author: {}", empty_marker(&header.author));
    println!("URL: {}", empty_marker(&header.url));
    println!("Path: {}", path.display());
    println!();
    println!("Container:");
    println!("  format: MFP");
    println!(
        "  version: {}.{}",
        header.container_major, header.container_minor
    );
    println!(
        "  binary representation version: {}.{}",
        header.binary_repr_major, header.binary_repr_minor
    );
    println!("  flags: 0x{:08x}", header.flags);
    println!(
        "  signature type: {}",
        signature_type_name(header.signature_type)
    );
    println!("  signature length: {}", header.signature_length);
    println!("  content hash: {}", hex_bytes(&content_hash));
    println!(
        "  binary representation length: {}",
        header.binary_repr_length
    );
    println!();
    println!("Manifest:");
    println!("  name: {}", info.manifest_name);
    println!("  ident: {}", empty_marker(&info.manifest_ident));
    println!("  version: {}", info.manifest_version);
    println!("  ident key: {}", empty_marker(&info.manifest_ident_key));
    println!(
        "  ident fingerprint: {}",
        empty_marker(&info.manifest_ident_fingerprint)
    );
    println!(
        "  signing fingerprint: {}",
        empty_marker(&info.manifest_signing_fingerprint)
    );
    println!("  author: {}", empty_marker(&info.author));
    println!("  url: {}", empty_marker(&info.url));
    println!();
    println!("Binary Representation:");
    println!("  ABI format version: {}", info.abi_format_version);
    println!("  types: {}", info.type_count);
    println!("  constants: {}", info.const_count);
    println!("  resources: {}", info.resource_count);
    println!("  functions: {}", info.function_count);
    println!("  globals: {}", info.global_count);
    println!("  cleanups: {}", info.cleanup_count);
    println!("  imports: {}", info.import_count);
    println!("  exports: {}", info.export_count);
    println!();
    println!("Exports:");
    if info.exports.is_empty() {
        println!("  <none>");
    } else {
        for export in &info.exports {
            println!(
                "  {} {}",
                package_export_kind_name(export.kind),
                export.name
            );
            println!("    sigHash: {}", export.sig_hash);
        }
    }
    println!();
    println!("Package State:");
    if info.globals.is_empty() {
        println!("  <none>");
    } else {
        for global in &info.globals {
            println!(
                "  {} {} AS {}",
                if global.mutable { "MUT" } else { "LET" },
                global.name,
                global.type_
            );
            println!("    visibility: {}", global.visibility);
            if global.mutable && global.visibility == "export" {
                println!("    audit: exported mutable package state");
            }
        }
    }
    println!();
    println!("Resource Cleanups:");
    if info.cleanups.is_empty() {
        println!("  <none>");
    } else {
        for cleanup in &info.cleanups {
            println!("  {} cleanup {}", cleanup.function, cleanup.cleanup_id);
            println!("    pc: {}..{}", cleanup.start_pc, cleanup.end_pc);
            println!("    resource register: {}", cleanup.resource_register);
            println!("    close function id: {}", cleanup.close_function_id);
            if cleanup.records_secondary_close_failure {
                println!("    audit: records secondary close failure");
            }
        }
    }
    println!();
    println!("Imports:");
    if info.imports.is_empty() {
        println!("  <none>");
    } else {
        for import in &info.imports {
            println!("  {}", import.package_name);
            println!("    ident: {}", empty_marker(&import.package_ident));
            println!("    version: {}", empty_marker(&import.version));
            println!("    pin: {}", import.pin);
            println!("    flags: 0x{:08x}", import.flags);
            if import.used_symbols.is_empty() {
                println!("    used symbols: <none>");
            } else {
                println!("    used symbols:");
                for symbol in &import.used_symbols {
                    println!("      {}", symbol.name);
                    println!("        sigHash: {}", symbol.sig_hash);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn signature_type_name(signature_type: u16) -> String {
    match signature_type {
        0 => "unsigned".to_string(),
        1 => "Ed25519".to_string(),
        other => format!("unknown ({other})"),
    }
}

pub(crate) fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn package_export_kind_name(kind: binary_repr::BinaryReprExportKind) -> &'static str {
    match kind {
        binary_repr::BinaryReprExportKind::Func => "FUNC",
        binary_repr::BinaryReprExportKind::Sub => "SUB",
        binary_repr::BinaryReprExportKind::Type => "TYPE",
        binary_repr::BinaryReprExportKind::Union => "UNION",
        binary_repr::BinaryReprExportKind::Enum => "ENUM",
    }
}

fn empty_marker(value: &str) -> &str {
    if value.is_empty() {
        "<empty>"
    } else {
        value
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PackageVerifyStatus {
    Ok,
    NeedsUpdate,
    InvalidPackage,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PackageVerifyResult {
    pub(crate) version: String,
    pub(crate) status: PackageVerifyStatus,
}

impl PackageVerifyStatus {
    fn message(&self) -> &'static str {
        match self {
            PackageVerifyStatus::Ok => "OK",
            PackageVerifyStatus::NeedsUpdate => "Needs Update",
            PackageVerifyStatus::InvalidPackage => "Invalid Package",
        }
    }
}

pub(crate) fn package_verify_line(
    dependency: &ProjectPackageDependency,
    result: &PackageVerifyResult,
) -> String {
    if result.version.is_empty() {
        format!(
            "{} @ {} : {}",
            dependency.name,
            dependency.version,
            result.status.message()
        )
    } else {
        format!(
            "{} @ {} : {} ({})",
            dependency.name,
            dependency.version,
            result.status.message(),
            result.version
        )
    }
}

pub(crate) fn verify_package_dependency(
    project_dir: &Path,
    dependency: &ProjectPackageDependency,
) -> PackageVerifyResult {
    let package_file = project_dir
        .join("packages")
        .join(format!("{}.mfp", dependency.name));
    if package_file.is_file() {
        return match read_mfp_header(&package_file) {
            Ok(header) => PackageVerifyResult {
                version: header.version.clone(),
                status: package_dependency_status(
                    dependency,
                    &header.name,
                    &header.ident,
                    &header.version,
                ),
            },
            Err(_) => PackageVerifyResult {
                version: String::new(),
                status: PackageVerifyStatus::InvalidPackage,
            },
        };
    }

    let package_manifest = project_dir
        .join("packages")
        .join(&dependency.name)
        .join("project.json");
    if package_manifest.is_file() {
        return verify_source_package_manifest(&package_manifest, dependency);
    }

    PackageVerifyResult {
        version: String::new(),
        status: PackageVerifyStatus::InvalidPackage,
    }
}

fn verify_source_package_manifest(
    package_manifest: &Path,
    dependency: &ProjectPackageDependency,
) -> PackageVerifyResult {
    let Ok(contents) = fs::read_to_string(package_manifest) else {
        return PackageVerifyResult {
            version: String::new(),
            status: PackageVerifyStatus::InvalidPackage,
        };
    };
    let Ok(manifest) = parse_project_json(&contents, package_manifest) else {
        return PackageVerifyResult {
            version: String::new(),
            status: PackageVerifyStatus::InvalidPackage,
        };
    };
    let Some(actual_name) = manifest.get("name").and_then(|value| value.get::<String>()) else {
        return PackageVerifyResult {
            version: String::new(),
            status: PackageVerifyStatus::InvalidPackage,
        };
    };
    let Some(actual_version) = manifest
        .get("version")
        .and_then(|value| value.get::<String>())
    else {
        return PackageVerifyResult {
            version: String::new(),
            status: PackageVerifyStatus::InvalidPackage,
        };
    };
    let actual_ident = manifest
        .get("ident")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
        .unwrap_or(actual_name);

    PackageVerifyResult {
        version: actual_version.clone(),
        status: package_dependency_status(dependency, actual_name, actual_ident, actual_version),
    }
}

pub(crate) fn package_dependency_status(
    dependency: &ProjectPackageDependency,
    actual_name: &str,
    actual_ident: &str,
    actual_version: &str,
) -> PackageVerifyStatus {
    if dependency.name != actual_name {
        return PackageVerifyStatus::InvalidPackage;
    }
    if !dependency.ident.is_empty() && !actual_ident.is_empty() && dependency.ident != actual_ident
    {
        return PackageVerifyStatus::InvalidPackage;
    }
    if dependency.pin {
        package_version_status(&dependency.version, actual_version)
    } else {
        package_version_status(&dependency.version, actual_version)
    }
}

fn package_version_status(expected_version: &str, actual_version: &str) -> PackageVerifyStatus {
    if package_version_matches(expected_version, actual_version) {
        PackageVerifyStatus::Ok
    } else {
        PackageVerifyStatus::NeedsUpdate
    }
}

pub(crate) fn package_version_matches(expected: &str, actual: &str) -> bool {
    expected.is_empty() || expected == actual
}
