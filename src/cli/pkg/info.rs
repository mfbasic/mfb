//! plan-176-L: `mfb pkg` — the info commands. The dispatcher `run_pkg_command` and the
//! shared `use` header live in `cli/pkg/mod.rs`.

use super::*;

/// `mfb pkg doc <name-or-path> [--out <file>]` — render HTML from a compiled
/// package's doc section (plan-09-doc.md §6.2).
pub(super) fn run_pkg_doc(args: &[String]) -> Result<(), PkgCommandError> {
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
        PkgCommandError::Usage(format!("mfb pkg doc requires <name-or-path>\n\n{PKG_HELP}"))
    })?;
    let out_path = out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("doc.html"));
    write_package_doc(&target, &out_path).map_err(PkgCommandError::Failed)
}

/// Resolve a package name or path to its `.mfp`, decode the doc section, and
/// render it. A package with no doc section yields a minimal page (exit 0).
pub(super) fn write_package_doc(target: &str, out_path: &Path) -> Result<(), String> {
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

pub(super) fn print_package_info(path: &Path) -> Result<(), String> {
    let package_bytes =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let content_hash = target::package_mfp::package_content_hash(&package_bytes)?;
    let header = read_mfp_header(path)?;
    let info = binary_repr::read_package_info(path)?;

    println!("Package: {}", crate::terminal_safe::safe(&header.name));
    println!("Ident: {}", empty_marker(&header.ident));
    println!("Version: {}", crate::terminal_safe::safe(&header.version));
    println!("Ident Key: {}", empty_marker(&header.ident_key));
    println!("Signing Key: {}", empty_marker(&header.signing_key));
    println!(
        "Proof: {}",
        if header.proof.is_empty() {
            std::borrow::Cow::Borrowed("<none>")
        } else {
            crate::terminal_safe::safe(&header.proof)
        }
    );
    println!(
        "Attestation: {}",
        if header.attestation.is_empty() {
            std::borrow::Cow::Borrowed("<none>")
        } else {
            crate::terminal_safe::safe(&header.attestation)
        }
    );
    println!("Author: {}", empty_marker(&header.author));
    println!("URL: {}", empty_marker(&header.url));
    // Best-effort registry release state (plan-10-C1): only shown when the
    // package has a registry ident and the configured registry answers. Silent
    // otherwise, so offline `pkg info` is unchanged.
    if let Some((owner, package_name)) = header.ident.split_once('#') {
        let repo_url = mfb_repository::client::repo_url_from_env();
        if let Ok(paths) = super::local_paths_for_repo(&repo_url) {
            if let Ok(index) =
                mfb_repository::client::fetch_index(&repo_url, &paths, owner, package_name)
            {
                if let Some(version) = index
                    .versions
                    .iter()
                    .find(|version| version.version == header.version)
                {
                    // bug-489: registry-authored. Sanitized HERE, not at the
                    // source, because `state` is a data field a caller may
                    // compare — escaping it upstream would change logic, not
                    // just display.
                    println!(
                        "Release State: {}",
                        crate::terminal_safe::safe(&version.state)
                    );
                }
            }
        }
    }
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
    println!(
        "  package binary hash: {}",
        hex_bytes(&header.package_binary_hash)
    );
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

pub(super) fn package_export_kind_name(kind: binary_repr::BinaryReprExportKind) -> &'static str {
    match kind {
        binary_repr::BinaryReprExportKind::Func => "FUNC",
        binary_repr::BinaryReprExportKind::Type => "TYPE",
        binary_repr::BinaryReprExportKind::Union => "UNION",
        binary_repr::BinaryReprExportKind::Enum => "ENUM",
    }
}

/// `<empty>` for a blank field, otherwise the value with terminal-unsafe code
/// points escaped. Header fields come from an untrusted `.mfp`, so an embedded
/// ESC/newline or bidi override must not reach the operator's terminal verbatim
/// (bug-210; `read_mfp_string` only enforces valid UTF-8).
pub(super) fn empty_marker(value: &str) -> std::borrow::Cow<'_, str> {
    if value.is_empty() {
        std::borrow::Cow::Borrowed("<empty>")
    } else {
        crate::terminal_safe::safe(value)
    }
}
