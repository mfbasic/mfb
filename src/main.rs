mod arch;
mod ast;
mod builtins;
mod bytecode;
mod ir;
mod lexer;
mod man;
mod monomorph;
mod numeric;
mod os;
mod resolver;
mod rules;
mod target;
mod typecheck;
mod unicode_backend;
mod unicode_runtime_tables;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use tinyjson::JsonValue;

const USAGE: &str = "Usage: mfb <command> <arguments>\n\nCommands:\n  help                        Show this message\n  init <location>             Create a new MFBASIC executable project\n  init-pkg <location>         Create a new MFBASIC package project\n  pkg add <url>               Add a compiled package to the current project\n  pkg info <package>          Show information about a compiled package\n  pkg verify                  Verify packages declared by project.json\n  build [-ast|-ir|-bc|-nir|-nplan|-nobj|-ncode] [-target os-arch] [location] Validate and build an MFBASIC project\n  man [package] [function]    Show built-in package and function help";

const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];

fn main() {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        Some("help") | None => {
            println!("{USAGE}");
        }
        Some("init") => {
            let Some(location) = args.next() else {
                eprintln!("error: mfb init requires <location>\n\n{USAGE}");
                process::exit(2);
            };

            if args.next().is_some() {
                eprintln!("error: mfb init accepts exactly one <location>\n\n{USAGE}");
                process::exit(2);
            }

            if let Err(err) = init_project(Path::new(&location)) {
                eprintln!("error: {err}");
                process::exit(1);
            }
        }
        Some("init-pkg") => {
            let Some(location) = args.next() else {
                eprintln!("error: mfb init-pkg requires <location>\n\n{USAGE}");
                process::exit(2);
            };

            if args.next().is_some() {
                eprintln!("error: mfb init-pkg accepts exactly one <location>\n\n{USAGE}");
                process::exit(2);
            }

            if let Err(err) = init_package_project(Path::new(&location)) {
                eprintln!("error: {err}");
                process::exit(1);
            }
        }
        Some("build") => {
            let build_options = match parse_build_options(args.collect()) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    process::exit(2);
                }
            };

            if let Err(()) = build_project(&build_options) {
                process::exit(1);
            }
        }
        Some("pkg") => {
            let pkg_args = args.collect::<Vec<_>>();
            if let Err(err) = run_pkg_command(&pkg_args) {
                match err {
                    PkgCommandError::Usage(message) => {
                        eprintln!("error: {message}");
                        process::exit(2);
                    }
                    PkgCommandError::Failed(message) => {
                        eprintln!("error: {message}");
                        process::exit(1);
                    }
                }
            }
        }
        Some("man") => {
            let man_args = args.collect::<Vec<_>>();
            if let Err(err) = show_man(&man_args) {
                eprintln!("error: {err}");
                process::exit(2);
            }
        }
        Some(command) => {
            eprintln!("error: unknown command '{command}'\n\n{USAGE}");
            process::exit(2);
        }
    }
}

struct BuildOptions {
    location: PathBuf,
    output: BuildOutput,
    target: target::BuildTarget,
}

enum BuildOutput {
    Validate,
    Ast,
    Ir,
    Bytecode,
    NativeIr,
    NativePlan,
    NativeObjectPlan,
    NativeCodePlan,
}

fn parse_build_options(args: Vec<String>) -> Result<BuildOptions, String> {
    let mut location = None;
    let mut output = BuildOutput::Validate;
    let mut target = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == "-ast" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::Ast;
        } else if arg == "-ir" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::Ir;
        } else if arg == "-bc" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::Bytecode;
        } else if arg == "-nir" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::NativeIr;
        } else if arg == "-nplan" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::NativePlan;
        } else if arg == "-nobj" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::NativeObjectPlan;
        } else if arg == "-ncode" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::NativeCodePlan;
        } else if arg == "-target" {
            let Some(value) = iter.next() else {
                return Err("mfb build -target requires os-arch".to_string());
            };
            target = Some(target::BuildTarget::parse(&value)?);
        } else if let Some(value) = arg.strip_prefix("-target=") {
            target = Some(target::BuildTarget::parse(value)?);
        } else if arg.starts_with('-') {
            return Err(format!("unknown build option `{arg}`"));
        } else if location.replace(PathBuf::from(&arg)).is_some() {
            return Err("mfb build accepts at most one [location]".to_string());
        }
    }

    Ok(BuildOptions {
        location: location.unwrap_or_else(|| PathBuf::from(".")),
        output,
        target: target.unwrap_or_else(target::BuildTarget::host),
    })
}

fn init_project(location: &Path) -> Result<(), String> {
    let src_dir = location.join("src");
    fs::create_dir_all(&src_dir).map_err(|err| {
        format!(
            "failed to create source directory '{}': {err}",
            src_dir.display()
        )
    })?;

    let project_path = location.join("project.json");
    let main_path = src_dir.join("main.mfb");

    write_new_file(&project_path, project_manifest(location) + "\n")?;
    write_new_file(&main_path, hello_world_source())?;

    println!("Created MFBASIC project at {}", location.display());
    Ok(())
}

fn init_package_project(location: &Path) -> Result<(), String> {
    let src_dir = location.join("src");
    fs::create_dir_all(&src_dir).map_err(|err| {
        format!(
            "failed to create source directory '{}': {err}",
            src_dir.display()
        )
    })?;

    let project_path = location.join("project.json");
    let lib_path = src_dir.join("lib.mfb");

    write_new_file(&project_path, package_project_manifest(location) + "\n")?;
    write_new_file(&lib_path, package_source())?;

    println!("Created MFBASIC package project at {}", location.display());
    Ok(())
}

fn build_project(options: &BuildOptions) -> Result<(), ()> {
    let target = options.target.clone();
    let project_path = options.location.join("project.json");
    let manifest = validate_project_manifest(&project_path)?;
    let project_kind = project_kind(&manifest);

    let project_name = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .expect("validated project name");
    let ast = ast::parse_project(project_name, &options.location, &manifest)?;
    resolver::resolve_project(&options.location, &manifest, &ast)?;
    let concrete_ast = monomorph::monomorphize_project(&options.location, &ast)?;
    resolver::resolve_project(&options.location, &manifest, &concrete_ast)?;
    let entry = validate_entry_point(&options.location, &manifest, &concrete_ast)?;
    typecheck::check_project(&options.location, &concrete_ast)?;

    match options.output {
        BuildOutput::Validate => {
            if project_kind == "executable" {
                let packages =
                    installed_package_files(&options.location, &manifest).map_err(|err| {
                        eprintln!("error: {err}");
                    })?;
                let (external_functions, external_params) =
                    external_package_function_types_from_files(&packages).map_err(|err| {
                        eprintln!("error: {err}");
                    })?;
                let ir = ir::lower_project_with_external_functions(
                    &concrete_ast,
                    entry.clone(),
                    &external_functions,
                    &external_params,
                );
                let executable_paths =
                    target::write_executable(&options.location, &ir, &target, &packages).map_err(
                        |err| {
                            eprintln!("error: {err}");
                        },
                    )?;
                for executable_path in executable_paths {
                    println!("Wrote executable to {}", executable_path.display());
                }
            } else if project_kind == "package" {
                let packages =
                    installed_package_files(&options.location, &manifest).map_err(|err| {
                        eprintln!("error: {err}");
                    })?;
                let (external_functions, external_params) =
                    external_package_function_types_from_files(&packages).map_err(|err| {
                        eprintln!("error: {err}");
                    })?;
                let ir = ir::lower_project_with_external_functions(
                    &concrete_ast,
                    entry.clone(),
                    &external_functions,
                    &external_params,
                );
                let metadata = package_metadata(&manifest);
                let package_path =
                    target::write_package(&options.location, &ir, &metadata, &packages).map_err(
                        |err| {
                            eprintln!("error: {err}");
                        },
                    )?;
                println!("Wrote package to {}", package_path.display());
            } else {
                println!(
                    "Validated MFBASIC project at {}",
                    options.location.display()
                );
            }
        }
        BuildOutput::Ast => {
            let ast_path = ast::write_ast(&options.location, &ast).map_err(|err| {
                eprintln!("error: {err}");
            })?;
            println!("Wrote AST to {}", ast_path.display());
        }
        BuildOutput::Ir => {
            let (external_functions, external_params) =
                external_package_function_types(&options.location, &manifest);
            let ir = ir::lower_project_with_external_functions(
                &concrete_ast,
                entry.clone(),
                &external_functions,
                &external_params,
            );
            let ir_path = ir::write_ir(&options.location, &ir).map_err(|err| {
                eprintln!("error: {err}");
            })?;
            println!("Wrote IR to {}", ir_path.display());
        }
        BuildOutput::Bytecode => {
            let packages =
                installed_package_files(&options.location, &manifest).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let (external_functions, external_params) =
                external_package_function_types_from_files(&packages).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let ir = ir::lower_project_with_external_functions(
                &concrete_ast,
                entry.clone(),
                &external_functions,
                &external_params,
            );
            let version = manifest
                .get("version")
                .and_then(|value| value.get::<String>())
                .expect("validated project version");
            let bytecode_path = if packages.is_empty() {
                bytecode::write_bytecode_hex(&options.location, &ir, version)
            } else {
                bytecode::write_merged_bytecode_hex(&options.location, &ir, version, &packages)
            }
            .map_err(|err| {
                eprintln!("error: {err}");
            })?;
            println!("Wrote bytecode hex to {}", bytecode_path.display());
        }
        BuildOutput::NativeIr => {
            if project_kind == "package" {
                rules::show_general_diagnostic(
                    "PACKAGE_NATIVE_OUTPUT_UNSUPPORTED",
                    "Package projects do not support native IR output; run `mfb build` to write a .mfp package.",
                );
                return Err(());
            }

            let packages =
                installed_package_files(&options.location, &manifest).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let (external_functions, external_params) =
                external_package_function_types_from_files(&packages).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let ir = ir::lower_project_with_external_functions(
                &concrete_ast,
                entry,
                &external_functions,
                &external_params,
            );
            let nir_path = match target::write_nir(&options.location, &ir, &target, &packages) {
                Ok(path) => path,
                Err(err) => {
                    eprintln!("error: {err}");
                    return Err(());
                }
            };
            println!("Wrote native IR to {}", nir_path.display());
        }
        BuildOutput::NativePlan => {
            if project_kind == "package" {
                rules::show_general_diagnostic(
                    "PACKAGE_NATIVE_OUTPUT_UNSUPPORTED",
                    "Package projects do not support native plan output; run `mfb build` to write a .mfp package.",
                );
                return Err(());
            }

            let packages =
                installed_package_files(&options.location, &manifest).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let (external_functions, external_params) =
                external_package_function_types_from_files(&packages).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let ir = ir::lower_project_with_external_functions(
                &concrete_ast,
                entry,
                &external_functions,
                &external_params,
            );
            let plan_path =
                match target::write_native_plan(&options.location, &ir, &target, &packages) {
                    Ok(path) => path,
                    Err(err) => {
                        eprintln!("error: {err}");
                        return Err(());
                    }
                };
            println!("Wrote native plan to {}", plan_path.display());
        }
        BuildOutput::NativeObjectPlan => {
            if project_kind == "package" {
                rules::show_general_diagnostic(
                    "PACKAGE_NATIVE_OUTPUT_UNSUPPORTED",
                    "Package projects do not support native object plan output; run `mfb build` to write a .mfp package.",
                );
                return Err(());
            }

            let packages =
                installed_package_files(&options.location, &manifest).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let (external_functions, external_params) =
                external_package_function_types_from_files(&packages).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let ir = ir::lower_project_with_external_functions(
                &concrete_ast,
                entry,
                &external_functions,
                &external_params,
            );
            let object_path = match target::write_native_object_plan(
                &options.location,
                &ir,
                &target,
                &packages,
            ) {
                Ok(path) => path,
                Err(err) => {
                    eprintln!("error: {err}");
                    return Err(());
                }
            };
            println!("Wrote native object plan to {}", object_path.display());
        }
        BuildOutput::NativeCodePlan => {
            if project_kind == "package" {
                rules::show_general_diagnostic(
                    "PACKAGE_NATIVE_OUTPUT_UNSUPPORTED",
                    "Package projects do not support native code plan output; run `mfb build` to write a .mfp package.",
                );
                return Err(());
            }

            let packages =
                installed_package_files(&options.location, &manifest).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let (external_functions, external_params) =
                external_package_function_types_from_files(&packages).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let ir = ir::lower_project_with_external_functions(
                &concrete_ast,
                entry,
                &external_functions,
                &external_params,
            );
            let code_path =
                match target::write_native_code_plan(&options.location, &ir, &target, &packages) {
                    Ok(path) => path,
                    Err(err) => {
                        eprintln!("error: {err}");
                        return Err(());
                    }
                };
            println!("Wrote native code plan to {}", code_path.display());
        }
    }

    Ok(())
}

enum PkgCommandError {
    Usage(String),
    Failed(String),
}

fn run_pkg_command(args: &[String]) -> Result<(), PkgCommandError> {
    match args {
        [command, url] if command == "add" => {
            add_package(Path::new("."), url).map_err(PkgCommandError::Failed)
        }
        [command, package] if command == "info" => {
            print_package_info(Path::new(package)).map_err(PkgCommandError::Failed)
        }
        [command] if command == "verify" => {
            verify_packages(Path::new(".")).map_err(PkgCommandError::Failed)
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
        [] => Err(PkgCommandError::Usage(format!(
            "mfb pkg requires a subcommand\n\n{USAGE}"
        ))),
        [command, ..] => Err(PkgCommandError::Usage(format!(
            "unknown pkg command `{command}`\n\n{USAGE}"
        ))),
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

fn print_package_info(path: &Path) -> Result<(), String> {
    let package_bytes =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let content_hash = target::package_mfp::package_content_hash(&package_bytes)?;
    let header = read_mfp_header(path)?;
    let info = bytecode::read_package_info(path)?;

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
        "  bytecode version: {}.{}",
        header.bytecode_major, header.bytecode_minor
    );
    println!("  flags: 0x{:08x}", header.flags);
    println!(
        "  signature type: {}",
        signature_type_name(header.signature_type)
    );
    println!("  signature length: {}", header.signature_length);
    println!("  content hash: {}", hex_bytes(&content_hash));
    println!("  bytecode length: {}", header.bytecode_length);
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
    println!("Bytecode:");
    println!("  ABI format version: {}", info.abi_format_version);
    println!("  types: {}", info.type_count);
    println!("  constants: {}", info.const_count);
    println!("  resources: {}", info.resource_count);
    println!("  functions: {}", info.function_count);
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

fn signature_type_name(signature_type: u16) -> String {
    match signature_type {
        0 => "unsigned".to_string(),
        1 => "Ed25519".to_string(),
        other => format!("unknown ({other})"),
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn package_export_kind_name(kind: bytecode::BytecodeExportKind) -> &'static str {
    match kind {
        bytecode::BytecodeExportKind::Func => "FUNC",
        bytecode::BytecodeExportKind::Sub => "SUB",
        bytecode::BytecodeExportKind::Type => "TYPE",
        bytecode::BytecodeExportKind::Union => "UNION",
        bytecode::BytecodeExportKind::Enum => "ENUM",
    }
}

fn empty_marker(value: &str) -> &str {
    if value.is_empty() {
        "<empty>"
    } else {
        value
    }
}

fn project_package_dependency(value: &JsonValue) -> Option<ProjectPackageDependency> {
    let package = value.get::<HashMap<String, JsonValue>>()?;
    let name = package.get("name")?.get::<String>()?.clone();
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
    let source = package
        .get("source")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    let pin = package
        .get("pin")
        .and_then(|value| value.get::<bool>())
        .copied()
        .unwrap_or(false);

    if name.trim().is_empty() {
        return None;
    }

    Some(ProjectPackageDependency {
        name,
        ident,
        version,
        pin,
        source,
    })
}

#[derive(Debug, PartialEq, Eq)]
enum PackageVerifyStatus {
    Ok,
    NeedsUpdate,
    InvalidPackage,
}

#[derive(Debug, PartialEq, Eq)]
struct PackageVerifyResult {
    version: String,
    status: PackageVerifyStatus,
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

fn package_verify_line(
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

fn verify_package_dependency(
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

fn package_dependency_status(
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

fn package_version_matches(expected: &str, actual: &str) -> bool {
    expected.is_empty() || expected == actual
}

fn show_man(args: &[String]) -> Result<(), String> {
    match args {
        [] => {
            print_man_index();
            Ok(())
        }
        [package_name] => {
            let package =
                man::package(package_name).ok_or_else(|| unknown_package_error(package_name))?;
            print_package_man(package);
            Ok(())
        }
        [package_name, function_name] => {
            let package =
                man::package(package_name).ok_or_else(|| unknown_package_error(package_name))?;
            let function = man::function(package, function_name).ok_or_else(|| {
                format!(
                    "unknown function `{function_name}` in package `{package_name}`\n\nRun `mfb man {package_name}` to list available functions."
                )
            })?;
            if let Some(page) = man::function_page(package, function_name) {
                print_man_page(page);
            } else {
                print_function_man(package, function);
            }
            Ok(())
        }
        _ => Err(format!("mfb man accepts at most two arguments\n\n{USAGE}")),
    }
}

fn print_man_index() {
    println!("Usage: mfb man [package] [function]");
    println!();
    println!("Show help for built-in packages and functions.");
    println!();
    println!("Examples:");
    println!("  mfb man");
    println!("  mfb man general");
    println!("  mfb man io print");
    println!();
    println!("Packages:");
    for package in man::packages() {
        println!("  {:<8} {}", package.name, package.summary);
    }
}

fn print_package_man(package: &man::PackageDoc) {
    if let Some(page) = package.page {
        print_man_page(page);
        if !package.functions.is_empty() {
            println!();
            println!("{}", man_entry_heading(package));
            for function in package.functions {
                println!("  {:<18} {}", function.name, function.summary);
            }
            println!();
            println!(
                "Run `mfb man {} <{}>` for details.",
                package.name,
                man_entry_name(package)
            );
        }
        return;
    }

    println!("Package: {}", package.name);
    println!();
    println!("{}", package.summary);
    println!();
    println!("Usage:");
    println!("  {}", package.usage);
    println!();
    println!("{}:", man_entry_heading(package));
    for function in package.functions {
        println!("  {:<18} {}", function.name, function.summary);
    }
    println!();
    println!(
        "Run `mfb man {} <{}>` for details.",
        package.name,
        man_entry_name(package)
    );
}

fn man_entry_heading(package: &man::PackageDoc) -> &'static str {
    if package.name == "types" {
        "TOPICS"
    } else {
        "FUNCTIONS"
    }
}

fn man_entry_name(package: &man::PackageDoc) -> &'static str {
    if package.name == "types" {
        "topic"
    } else {
        "function"
    }
}

fn print_man_page(page: &str) {
    println!("{}", page.trim_end_matches('\n'));
}

fn print_function_man(package: &man::PackageDoc, function: &man::FunctionDoc) {
    println!("{} {}", package.name, function.name);
    println!();
    println!("{}", function.summary);
    println!();
    println!("Signature:");
    println!("  {}", function.signature);
    println!();
    println!("Example:");
    for line in function.example.lines() {
        println!("  {line}");
    }
}

fn unknown_package_error(package_name: &str) -> String {
    let packages = man::packages()
        .iter()
        .map(|package| package.name)
        .collect::<Vec<_>>()
        .join(", ");
    format!("unknown package `{package_name}`\n\nAvailable packages: {packages}")
}

fn validate_entry_point(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    ast: &ast::AstProject,
) -> Result<Option<ir::EntryPoint>, ()> {
    let kind = project_kind(manifest);
    if kind == "package" {
        return Ok(None);
    }

    let entry = entry_point(manifest);
    let mut matches = Vec::new();

    for file in &ast.files {
        for item in &file.items {
            let ast::Item::Function(function) = item else {
                continue;
            };
            if function.name != entry {
                continue;
            }

            let returns = match function.kind {
                ast::FunctionKind::Sub => "Nothing",
                ast::FunctionKind::Func => function.return_type.as_deref().unwrap_or(""),
            };

            if matches!(function.kind, ast::FunctionKind::Func) && returns != "Integer" {
                rules::show_diagnostic(
                    "PROJECT_ENTRY_INVALID",
                    &format!("Executable FUNC entry `{entry}` must return Integer."),
                    &project_dir.join(&file.path),
                    function.line,
                    1,
                    1,
                );
                return Err(());
            }

            let accepts_args = match function.params.as_slice() {
                [] => false,
                [param] if param.type_name.as_deref() == Some("List OF String") => true,
                [param] => {
                    rules::show_diagnostic(
                        "PROJECT_ENTRY_INVALID",
                        &format!(
                            "Executable entry `{entry}` parameter `{}` must have type List OF String.",
                            param.name
                        ),
                        &project_dir.join(&file.path),
                        param.line,
                        1,
                        1,
                    );
                    return Err(());
                }
                _ => {
                    rules::show_diagnostic(
                        "PROJECT_ENTRY_INVALID",
                        &format!(
                            "Executable entry `{entry}` must declare zero parameters or one `args AS List OF String` parameter."
                        ),
                        &project_dir.join(&file.path),
                        function.line,
                        1,
                        1,
                    );
                    return Err(());
                }
            };

            if function.params.len() == 1 && function.params[0].default.is_some() {
                rules::show_diagnostic(
                    "PROJECT_ENTRY_INVALID",
                    &format!("Executable entry `{entry}` args parameter must not declare a default value."),
                    &project_dir.join(&file.path),
                    function.params[0].line,
                    1,
                    1,
                );
                return Err(());
            }

            matches.push((
                file.path.clone(),
                function.line,
                entry.to_string(),
                returns.to_string(),
                accepts_args,
            ));
        }
    }

    if matches.len() > 1 {
        let (path, line, _, _, _) = &matches[1];
        rules::show_diagnostic(
            "PROJECT_ENTRY_INVALID",
            &format!(
                "Executable project must declare exactly one entry point named `{entry}`; found multiple matching declarations."
            ),
            &project_dir.join(path),
            *line,
            1,
            1,
        );
        return Err(());
    }

    if let Some((_, _, name, returns, accepts_args)) = matches.pop() {
        return Ok(Some(ir::EntryPoint {
            name,
            returns,
            accepts_args,
        }));
    }

    rules::show_diagnostic(
        "PROJECT_ENTRY_INVALID",
        &format!("Executable project must declare an entry point named `{entry}`."),
        &project_dir.join("project.json"),
        1,
        1,
        1,
    );
    Err(())
}

fn parse_project_json(
    contents: &str,
    project_path: &Path,
) -> Result<HashMap<String, JsonValue>, String> {
    let manifest: JsonValue = contents.parse().map_err(|err: tinyjson::JsonParseError| {
        format!("failed to parse '{}': {err}", project_path.display())
    })?;
    manifest
        .get::<HashMap<String, JsonValue>>()
        .cloned()
        .ok_or_else(|| format!("'{}' must contain a JSON object", project_path.display()))
}

struct MfpHeader {
    name: String,
    ident: String,
    version: String,
    ident_key: String,
    ident_fingerprint: String,
    signing_fingerprint: String,
    author: String,
    url: String,
    container_major: u16,
    container_minor: u16,
    bytecode_major: u16,
    bytecode_minor: u16,
    flags: u32,
    signature_type: u16,
    signature_length: usize,
    bytecode_length: usize,
}

struct ProjectPackageDependency {
    name: String,
    ident: String,
    version: String,
    pin: bool,
    source: String,
}

fn package_file_url_path(url: &str) -> Result<PathBuf, String> {
    let Some(path) = url.strip_prefix("file://") else {
        return Err("mfb pkg add currently supports only file:// URLs ending in .mfp".to_string());
    };

    if path.is_empty() {
        return Err("file:// URL must include an absolute package path".to_string());
    }
    if path.contains('?') || path.contains('#') {
        return Err("file:// package URLs must not include query strings or fragments".to_string());
    }

    let path = PathBuf::from(percent_decode_path(path)?);
    if !path.is_absolute() {
        return Err("file:// package URL must resolve to an absolute path".to_string());
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("mfp") {
        return Err("file:// package URL must point to a .mfp file".to_string());
    }
    if !path.is_file() {
        return Err(format!("package file '{}' does not exist", path.display()));
    }

    Ok(path)
}

fn percent_decode_path(path: &str) -> Result<String, String> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("file:// URL contains an incomplete percent escape".to_string());
            }
            let high = hex_value(bytes[index + 1])
                .ok_or_else(|| "file:// URL contains an invalid percent escape".to_string())?;
            let low = hex_value(bytes[index + 2])
                .ok_or_else(|| "file:// URL contains an invalid percent escape".to_string())?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(decoded).map_err(|_| "file:// URL path is not valid UTF-8".to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn read_mfp_header(path: &Path) -> Result<MfpHeader, String> {
    let bytes =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    if bytes.len() < 26 {
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
    if container_major != 1 {
        return Err(format!(
            "'{}' uses unsupported MFP container major version {container_major}",
            path.display()
        ));
    }
    let container_minor = read_u16(&bytes, 10)?;
    let bytecode_major = read_u16(&bytes, 12)?;
    let bytecode_minor = read_u16(&bytes, 14)?;
    let flags = read_u32(&bytes, 16)?;

    let signature_type = read_u16(&bytes, 20)?;
    let signature_length = read_u32(&bytes, 22)? as usize;
    match (signature_type, signature_length) {
        (0, 0) | (1, 64) => {}
        (0, _) => return Err("unsigned .mfp package must have zero signature length".to_string()),
        (1, _) => return Err("Ed25519 .mfp package must have a 64 byte signature".to_string()),
        _ => return Err(format!("unsupported .mfp signature type {signature_type}")),
    }

    let mut offset = 26usize
        .checked_add(signature_length)
        .ok_or_else(|| "invalid .mfp signature length".to_string())?;
    if offset > bytes.len() {
        return Err("truncated .mfp signature".to_string());
    }

    let name = read_mfp_string(&bytes, &mut offset, "name", 255, true)?;
    let ident = read_mfp_string(&bytes, &mut offset, "ident", 255, false)?;
    let version = read_mfp_string(&bytes, &mut offset, "version", 64, true)?;
    let ident_key = read_mfp_string(&bytes, &mut offset, "identKey", 255, false)?;
    let ident_fingerprint = read_mfp_string(&bytes, &mut offset, "identFingerprint", 255, false)?;
    let signing_fingerprint =
        read_mfp_string(&bytes, &mut offset, "signingFingerprint", 255, false)?;
    let author = read_mfp_string(&bytes, &mut offset, "author", 512, false)?;
    let url = read_mfp_string(&bytes, &mut offset, "url", 2048, false)?;
    let bytecode_length = read_u64(&bytes, offset)? as usize;
    offset = offset
        .checked_add(8)
        .and_then(|offset| offset.checked_add(bytecode_length))
        .ok_or_else(|| "invalid .mfp bytecode length".to_string())?;
    if offset != bytes.len() {
        return Err("invalid .mfp bytecode length".to_string());
    }

    Ok(MfpHeader {
        name,
        ident,
        version,
        ident_key,
        ident_fingerprint,
        signing_fingerprint,
        author,
        url,
        container_major,
        container_minor,
        bytecode_major,
        bytecode_minor,
        flags,
        signature_type,
        signature_length,
        bytecode_length,
    })
}

fn read_mfp_string(
    bytes: &[u8],
    offset: &mut usize,
    field: &str,
    limit: usize,
    required: bool,
) -> Result<String, String> {
    let length = read_u32(bytes, *offset)? as usize;
    *offset = offset
        .checked_add(4)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;

    if length > limit {
        return Err(format!(".mfp {field} exceeds the {limit} byte limit"));
    }

    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    if end > bytes.len() {
        return Err(format!("truncated .mfp {field}"));
    }

    let value = std::str::from_utf8(&bytes[*offset..end])
        .map_err(|_| format!(".mfp {field} is not valid UTF-8"))?
        .to_string();
    *offset = end;

    if required && value.is_empty() {
        return Err(format!(".mfp {field} must not be empty"));
    }

    Ok(value)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "truncated .mfp header".to_string())?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "truncated .mfp header".to_string())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| "truncated .mfp header".to_string())?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

fn validate_project_manifest(project_path: &Path) -> Result<HashMap<String, JsonValue>, ()> {
    if !project_path.exists() {
        rules::show_diagnostic(
            "PROJECT_JSON_MISSING",
            "Run `mfb init <location>` first or build from a directory that contains project.json.",
            project_path,
            1,
            1,
            1,
        );
        return Err(());
    }

    let contents = fs::read_to_string(project_path).map_err(|err| {
        rules::show_diagnostic(
            "PROJECT_JSON_READ_FAILED",
            &err.to_string(),
            project_path,
            1,
            1,
            1,
        );
    })?;

    let manifest: JsonValue = contents.parse().map_err(|err: tinyjson::JsonParseError| {
        let column = err.column().max(1);
        rules::show_diagnostic(
            "PROJECT_JSON_PARSE_FAILED",
            &err.to_string(),
            project_path,
            err.line(),
            column,
            column + 1,
        );
    })?;

    let Some(manifest) = manifest.get::<HashMap<String, JsonValue>>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_ROOT_TYPE",
            "The top-level JSON value must be an object with project fields.",
            project_path,
            1,
            1,
            1,
        );
        return Err(());
    };

    let mut valid = true;

    for field in ["name", "version", "mfb"] {
        if !validate_required_string(manifest, project_path, &contents, field) {
            valid = false;
        }
    }

    if !validate_sources(manifest, project_path, &contents) {
        valid = false;
    }

    if !validate_optional_string(manifest, project_path, &contents, "entry") {
        valid = false;
    }

    if !validate_optional_string(manifest, project_path, &contents, "author") {
        valid = false;
    }

    if !validate_optional_string(manifest, project_path, &contents, "url") {
        valid = false;
    }

    if !validate_kind(manifest, project_path, &contents) {
        valid = false;
    }

    if valid {
        Ok(manifest.clone())
    } else {
        Err(())
    }
}

fn validate_required_string(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
    field: &str,
) -> bool {
    let Some(value) = manifest.get(field) else {
        let (line, column) = fallback_field_position(contents);
        rules::show_diagnostic(
            "PROJECT_JSON_REQUIRED_FIELD",
            &format!("Required field `{field}` is missing."),
            project_path,
            line,
            column,
            column + 1,
        );
        return false;
    };

    let (line, column) = field_position(contents, field);
    let Some(value) = value.get::<String>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            &format!("Field `{field}` must be a string."),
            project_path,
            line,
            column,
            column + field.len() + 2,
        );
        return false;
    };

    if value.trim().is_empty() {
        rules::show_diagnostic(
            "PROJECT_JSON_EMPTY_FIELD",
            &format!("Field `{field}` must contain a non-empty string."),
            project_path,
            line,
            column,
            column + field.len() + 2,
        );
        return false;
    }

    true
}

fn validate_optional_string(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
    field: &str,
) -> bool {
    let Some(value) = manifest.get(field) else {
        return true;
    };

    if value.get::<String>().is_some() {
        return true;
    }

    let (line, column) = field_position(contents, field);
    rules::show_diagnostic(
        "PROJECT_JSON_FIELD_TYPE",
        &format!("Field `{field}` must be a string when present."),
        project_path,
        line,
        column,
        column + field.len() + 2,
    );
    false
}

fn validate_sources(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let Some(value) = manifest.get("sources") else {
        let (line, column) = fallback_field_position(contents);
        rules::show_diagnostic(
            "PROJECT_JSON_REQUIRED_FIELD",
            "Required field `sources` is missing.",
            project_path,
            line,
            column,
            column + 1,
        );
        return false;
    };

    let (line, column) = field_position(contents, "sources");
    let Some(sources) = value.get::<Vec<JsonValue>>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            "Field `sources` must be an array.",
            project_path,
            line,
            column,
            column + "\"sources\"".len(),
        );
        return false;
    };

    if sources.is_empty() {
        rules::show_diagnostic(
            "PROJECT_JSON_EMPTY_SOURCES",
            "Add at least one source entry, for example `{ \"root\": \"src\" }`.",
            project_path,
            line,
            column,
            column + "\"sources\"".len(),
        );
        return false;
    }

    let mut valid = true;
    for (index, source) in sources.iter().enumerate() {
        let Some(source) = source.get::<HashMap<String, JsonValue>>() else {
            rules::show_diagnostic(
                "PROJECT_JSON_FIELD_TYPE",
                &format!("Source entry #{index} must be an object."),
                project_path,
                line,
                column,
                column + "\"sources\"".len(),
            );
            valid = false;
            continue;
        };

        if !validate_required_string(source, project_path, contents, "root") {
            valid = false;
        }
        if !validate_source_pattern_field(source, project_path, contents, index, "include") {
            valid = false;
        }
        if !validate_source_pattern_field(source, project_path, contents, index, "exclude") {
            valid = false;
        }
    }

    valid
}

fn validate_source_pattern_field(
    source: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
    index: usize,
    field: &str,
) -> bool {
    let Some(value) = source.get(field) else {
        return true;
    };
    let (line, column) = field_position(contents, field);
    let Some(patterns) = value.get::<Vec<JsonValue>>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            &format!("Source entry #{index} field `{field}` must be an array of strings."),
            project_path,
            line,
            column,
            column + field.len() + 2,
        );
        return false;
    };
    if patterns
        .iter()
        .all(|pattern| pattern.get::<String>().is_some())
    {
        return true;
    }
    rules::show_diagnostic(
        "PROJECT_JSON_FIELD_TYPE",
        &format!("Source entry #{index} field `{field}` must be an array of strings."),
        project_path,
        line,
        column,
        column + field.len() + 2,
    );
    false
}

fn validate_kind(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let Some(value) = manifest.get("kind") else {
        return true;
    };

    let (line, column) = field_position(contents, "kind");
    let Some(kind) = value.get::<String>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            "Field `kind` must be a string when present.",
            project_path,
            line,
            column,
            column + "\"kind\"".len(),
        );
        return false;
    };

    if !matches!(kind.as_str(), "executable" | "package") {
        rules::show_diagnostic(
            "PROJECT_JSON_UNKNOWN_KIND",
            "Expected `executable` or `package`; continuing validation.",
            project_path,
            line,
            column,
            column + "\"kind\"".len(),
        );
    }

    true
}

fn project_kind(manifest: &HashMap<String, JsonValue>) -> &str {
    manifest
        .get("kind")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
        .unwrap_or("executable")
}

fn entry_point(manifest: &HashMap<String, JsonValue>) -> &str {
    manifest
        .get("entry")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
        .unwrap_or("main")
}

fn installed_package_files(
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
            let package_file = project_dir
                .join("packages")
                .join(format!("{}.mfp", dependency.name));
            if package_file.is_file() {
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
                    "package `{}` must be installed as '{}' before bytecode merging",
                    dependency.name,
                    package_file.display()
                ))
            }
        })
        .collect()
}

fn external_package_function_types(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> (
    HashMap<String, String>,
    HashMap<String, Vec<ir::ExternalFunctionParam>>,
) {
    let Ok(packages) = installed_package_files(project_dir, manifest) else {
        return (HashMap::new(), HashMap::new());
    };
    external_package_function_types_from_files_lossy(&packages)
}

fn external_package_function_types_from_files(
    packages: &[PathBuf],
) -> Result<
    (
        HashMap<String, String>,
        HashMap<String, Vec<ir::ExternalFunctionParam>>,
    ),
    String,
> {
    let mut functions = HashMap::new();
    let mut params = HashMap::new();
    for package in packages {
        let header = read_mfp_header(package)?;
        for export in bytecode::read_package_exports(package)? {
            let name = format!("{}.{}", header.name, export.name);
            functions.insert(name.clone(), package_export_function_type(&export));
            params.insert(
                name,
                export
                    .params
                    .iter()
                    .map(|param| ir::ExternalFunctionParam {
                        name: param.name.clone(),
                        type_: param.type_.clone(),
                        has_default: param.has_default,
                    })
                    .collect(),
            );
        }
    }
    Ok((functions, params))
}

fn external_package_function_types_from_files_lossy(
    packages: &[PathBuf],
) -> (
    HashMap<String, String>,
    HashMap<String, Vec<ir::ExternalFunctionParam>>,
) {
    let mut functions = HashMap::new();
    let mut params = HashMap::new();
    for package in packages {
        let Ok(header) = read_mfp_header(package) else {
            continue;
        };
        let Ok(exports) = bytecode::read_package_exports(package) else {
            continue;
        };
        for export in exports {
            let name = format!("{}.{}", header.name, export.name);
            functions.insert(name.clone(), package_export_function_type(&export));
            params.insert(
                name,
                export
                    .params
                    .iter()
                    .map(|param| ir::ExternalFunctionParam {
                        name: param.name.clone(),
                        type_: param.type_.clone(),
                        has_default: param.has_default,
                    })
                    .collect(),
            );
        }
    }
    (functions, params)
}

fn package_export_function_type(export: &bytecode::BytecodeExport) -> String {
    let params = export
        .params
        .iter()
        .map(|param| param.type_.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let prefix = if export.isolated { "ISOLATED " } else { "" };
    format!("{prefix}FUNC({params}) AS {}", export.return_type)
}

fn package_metadata(manifest: &HashMap<String, JsonValue>) -> bytecode::BytecodeMetadata {
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
    let mut metadata = bytecode::BytecodeMetadata::new(name, version);
    metadata.ident = manifest
        .get("ident")
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    metadata.ident_key = manifest
        .get("identKey")
        .or_else(|| manifest.get("ident_key"))
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    metadata.ident_fingerprint = manifest
        .get("identFingerprint")
        .or_else(|| manifest.get("ident_fingerprint"))
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
    metadata.signing_fingerprint = manifest
        .get("signingFingerprint")
        .or_else(|| manifest.get("signing_fingerprint"))
        .and_then(|value| value.get::<String>())
        .cloned()
        .unwrap_or_default();
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
    metadata.dependencies = package_dependencies(manifest);
    metadata
}

fn package_dependencies(
    manifest: &HashMap<String, JsonValue>,
) -> Vec<bytecode::BytecodeDependency> {
    manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(|package| package.get::<HashMap<String, JsonValue>>())
        .filter_map(|package| {
            let name = package.get("name")?.get::<String>()?.clone();
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
            Some(bytecode::BytecodeDependency {
                name,
                ident,
                version,
                pin,
                flags: 0,
            })
        })
        .collect()
}

fn validate_packages_array(manifest: &HashMap<String, JsonValue>) -> Result<(), String> {
    if manifest
        .get("packages")
        .is_some_and(|value| value.get::<Vec<JsonValue>>().is_none())
    {
        return Err("project.json field `packages` must be an array when present".to_string());
    }
    Ok(())
}

fn project_json_with_package(
    contents: &str,
    manifest: &HashMap<String, JsonValue>,
    dependency: &ProjectPackageDependency,
) -> Result<String, String> {
    let packages = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>());

    if packages.is_some_and(|packages| {
        packages.iter().any(|package| {
            package
                .get::<HashMap<String, JsonValue>>()
                .and_then(|package| package.get("name"))
                .and_then(|name| name.get::<String>())
                == Some(&dependency.name)
        })
    }) {
        return Err(format!(
            "project.json already declares package `{}`",
            dependency.name
        ));
    }

    let entry = package_dependency_json(dependency, 4);
    if packages.is_some() {
        insert_package_dependency(contents, &entry)
    } else {
        insert_packages_array(contents, &entry)
    }
}

fn package_dependency_json(dependency: &ProjectPackageDependency, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let field_pad = " ".repeat(indent + 2);
    format!(
        "{pad}{{\n{field_pad}\"name\": {},\n{field_pad}\"ident\": {},\n{field_pad}\"version\": {},\n{field_pad}\"pin\": {},\n{field_pad}\"source\": {}\n{pad}}}",
        json_string(&dependency.name),
        json_string(&dependency.ident),
        json_string(&dependency.version),
        dependency.pin,
        json_string(&dependency.source),
        pad = pad,
        field_pad = field_pad,
    )
}

fn insert_package_dependency(contents: &str, entry: &str) -> Result<String, String> {
    let Some((array_start, array_end)) = json_array_bounds(contents, "packages") else {
        return Err("could not locate project.json `packages` array".to_string());
    };
    let inner = &contents[array_start + 1..array_end];
    let has_entries = !inner.trim().is_empty();
    let before_entry = contents[..array_end].trim_end_matches([' ', '\t', '\r', '\n']);
    let closing_indent = &contents[before_entry.len()..array_end];

    let mut updated = String::new();
    updated.push_str(before_entry);
    if has_entries {
        updated.push(',');
    }
    updated.push('\n');
    updated.push_str(entry);
    updated.push_str(closing_indent);
    updated.push_str(&contents[array_end..]);
    Ok(updated)
}

fn insert_packages_array(contents: &str, entry: &str) -> Result<String, String> {
    let Some(root_end) = root_object_end(contents) else {
        return Err("could not locate end of project.json object".to_string());
    };
    let before = contents[..root_end].trim_end_matches([' ', '\t', '\r', '\n']);
    let between = &contents[before.len()..root_end];
    let needs_comma = before.as_bytes().last().is_some_and(|byte| *byte != b'{');

    let mut updated = String::new();
    updated.push_str(before);
    if needs_comma {
        updated.push(',');
    }
    updated.push_str("\n  \"packages\": [\n");
    updated.push_str(entry);
    updated.push_str("\n  ]");
    updated.push_str(between);
    updated.push_str(&contents[root_end..]);
    Ok(updated)
}

fn json_array_bounds(contents: &str, field: &str) -> Option<(usize, usize)> {
    let field_start = json_field_name_position(contents, field)?;
    let colon = find_json_punct(contents, field_start + field.len() + 2, b':')?;
    let array_start = find_json_punct(contents, colon + 1, b'[')?;
    let array_end = matching_json_delimiter(contents, array_start, b'[', b']')?;
    Some((array_start, array_end))
}

fn json_field_name_position(contents: &str, field: &str) -> Option<usize> {
    let needle = format!("\"{field}\"");
    let mut index = 0;

    loop {
        index = next_json_string_start(contents, index)?;
        let end = json_string_end(contents, index)?;
        if &contents[index..end] == needle {
            return Some(index);
        }
        index = end;
    }
}

fn root_object_end(contents: &str) -> Option<usize> {
    let start = find_json_punct(contents, 0, b'{')?;
    matching_json_delimiter(contents, start, b'{', b'}')
}

fn find_json_punct(contents: &str, start: usize, punct: u8) -> Option<usize> {
    let bytes = contents.as_bytes();
    let mut index = start;
    let mut in_string = false;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == punct {
            return Some(index);
        } else if !byte.is_ascii_whitespace() {
            return None;
        }
        index += 1;
    }

    None
}

fn matching_json_delimiter(contents: &str, start: usize, open: u8, close: u8) -> Option<usize> {
    let bytes = contents.as_bytes();
    let mut index = start;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == open {
            depth = depth.checked_add(1)?;
        } else if byte == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }

    None
}

fn next_json_string_start(contents: &str, start: usize) -> Option<usize> {
    contents[start..].find('"').map(|offset| start + offset)
}

fn json_string_end(contents: &str, start: usize) -> Option<usize> {
    let bytes = contents.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }

    let mut index = start + 1;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

fn field_position(contents: &str, field: &str) -> (usize, usize) {
    let needle = format!("\"{field}\"");
    for (index, line) in contents.lines().enumerate() {
        if let Some(column) = line.find(&needle) {
            return (index + 1, column + 1);
        }
    }

    fallback_field_position(contents)
}

fn fallback_field_position(contents: &str) -> (usize, usize) {
    if contents.is_empty() {
        (1, 1)
    } else {
        (contents.lines().count().max(1), 1)
    }
}

fn write_new_file(path: &Path, contents: String) -> Result<(), String> {
    if path.exists() {
        return Err(format!("refusing to overwrite '{}'", path.display()));
    }

    fs::write(path, contents).map_err(|err| format!("failed to write '{}': {err}", path.display()))
}

fn project_manifest(location: &Path) -> String {
    let name = json_string(&project_name(location));

    format!(
        concat!(
            "{{\n",
            "  \"name\": {},\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"sources\": [\n",
            "    {{\n",
            "      \"root\": \"src\",\n",
            "      \"role\": \"main\",\n",
            "      \"include\": [\"**/*.mfb\"]\n",
            "    }}\n",
            "  ],\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"]\n",
            "}}"
        ),
        name
    )
}

fn package_project_manifest(location: &Path) -> String {
    let name = json_string(&project_name(location));

    format!(
        concat!(
            "{{\n",
            "  \"name\": {},\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"package\",\n",
            "  \"sources\": [\n",
            "    {{\n",
            "      \"root\": \"src\",\n",
            "      \"role\": \"package\",\n",
            "      \"include\": [\"**/*.mfb\"]\n",
            "    }}\n",
            "  ]\n",
            "}}"
        ),
        name
    )
}

pub(crate) fn json_string(value: &str) -> String {
    JsonValue::String(value.to_string())
        .stringify()
        .unwrap_or_else(|_| "\"mfb_project\"".to_string())
}

fn project_name(location: &Path) -> String {
    location
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize_project_name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "mfb_project".to_string())
}

fn sanitize_project_name(name: &str) -> String {
    let mut sanitized = String::new();

    for (index, ch) in name.chars().enumerate() {
        let valid = ch.is_ascii_alphanumeric() || ch == '_';
        if valid && (index > 0 || ch.is_ascii_alphabetic() || ch == '_') {
            sanitized.push(ch);
        } else if index > 0 {
            sanitized.push('_');
        }
    }

    if sanitized.is_empty() {
        "mfb_project".to_string()
    } else {
        sanitized
    }
}

fn hello_world_source() -> String {
    "IMPORT io\n\nSUB main()\n  io.print(\"Hello World\")\nEND SUB\n".to_string()
}

fn package_source() -> String {
    "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_add_manifest_insert_creates_packages_array() {
        let contents = concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"sources\": [{ \"root\": \"src\" }]\n",
            "}\n"
        );
        let manifest = parse_project_json(contents, Path::new("project.json")).expect("manifest");
        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.0.0".to_string(),
            pin: true,
            source: "file:///tmp/source/shape.mfp".to_string(),
        };

        let updated =
            project_json_with_package(contents, &manifest, &dependency).expect("updated manifest");

        assert!(updated.contains("\"packages\": ["));
        assert!(updated.contains("\"name\": \"shape\""));
        assert!(updated.contains("\"ident\": \"ada#shape\""));
        assert!(updated.contains("\"version\": \"1.0.0\""));
        assert!(updated.contains("\"pin\": true"));
        assert!(updated.contains("\"source\": \"file:///tmp/source/shape.mfp\""));
        assert!(updated.parse::<JsonValue>().is_ok());
    }

    #[test]
    fn package_add_manifest_append_preserves_json_array_format() {
        let contents = concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"sources\": [{ \"root\": \"src\" }],\n",
            "  \"packages\": [\n",
            "    {\n",
            "      \"name\": \"math\",\n",
            "      \"ident\": \"std#math\",\n",
            "      \"version\": \"1.0.0\",\n",
            "      \"pin\": true,\n",
            "      \"source\": \"file:packages/math.mfp\"\n",
            "    }\n",
            "  ]\n",
            "}\n"
        );
        let manifest = parse_project_json(contents, Path::new("project.json")).expect("manifest");
        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.0.0".to_string(),
            pin: true,
            source: "file:///tmp/source/shape.mfp".to_string(),
        };

        let updated =
            project_json_with_package(contents, &manifest, &dependency).expect("updated manifest");

        assert!(updated.contains("    },\n    {\n      \"name\": \"shape\""));
        assert!(updated.parse::<JsonValue>().is_ok());
    }

    #[test]
    fn package_verify_status_checks_name_and_version() {
        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.2.3".to_string(),
            pin: true,
            source: "registry:mfb".to_string(),
        };
        assert_eq!(
            package_dependency_status(&dependency, "shape", "ada#shape", "1.2.3"),
            PackageVerifyStatus::Ok
        );
        assert_eq!(
            package_dependency_status(&dependency, "shape", "ada#shape", "1.2.4"),
            PackageVerifyStatus::NeedsUpdate
        );
        assert_eq!(
            package_dependency_status(&dependency, "color", "ada#shape", "1.2.3"),
            PackageVerifyStatus::InvalidPackage
        );
        assert_eq!(
            package_dependency_status(&dependency, "shape", "other#shape", "1.2.3"),
            PackageVerifyStatus::InvalidPackage
        );
    }

    #[test]
    fn package_verify_rejects_range_syntax_as_literal_version() {
        assert!(!package_version_matches("^1.2.3", "1.9.0"));
        assert!(!package_version_matches("~1.2.3", "1.2.9"));
        assert!(package_version_matches("1.2.3", "1.2.3"));
    }

    #[test]
    fn package_verify_line_shows_project_and_package_versions() {
        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.2.0".to_string(),
            pin: false,
            source: "registry:mfb".to_string(),
        };

        assert_eq!(
            package_verify_line(
                &dependency,
                &PackageVerifyResult {
                    version: "1.2.3".to_string(),
                    status: PackageVerifyStatus::Ok,
                }
            ),
            "shape @ 1.2.0 : OK (1.2.3)"
        );
        assert_eq!(
            package_verify_line(
                &dependency,
                &PackageVerifyResult {
                    version: String::new(),
                    status: PackageVerifyStatus::InvalidPackage,
                }
            ),
            "shape @ 1.2.0 : Invalid Package"
        );
    }

    #[test]
    fn package_verify_reads_source_package_manifest() {
        let root = test_temp_dir("package_verify_reads_source_package_manifest");
        let package_dir = root.join("packages").join("shape");
        fs::create_dir_all(&package_dir).expect("package dir");
        fs::write(
            package_dir.join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"shape\",\n",
                "  \"ident\": \"ada#shape\",\n",
                "  \"version\": \"1.2.3\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"package\",\n",
                "  \"sources\": [{ \"root\": \"src\" }]\n",
                "}\n"
            ),
        )
        .expect("package manifest");

        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.2.3".to_string(),
            pin: false,
            source: "registry:mfb".to_string(),
        };

        assert_eq!(
            verify_package_dependency(&root, &dependency),
            PackageVerifyResult {
                version: "1.2.3".to_string(),
                status: PackageVerifyStatus::Ok,
            }
        );

        fs::remove_dir_all(root).expect("remove temp dir");
    }

    #[test]
    fn validate_entry_point_rejects_multiple_matching_declarations() {
        let root = test_temp_dir("validate_entry_point_rejects_multiple_matching_declarations");
        let project_dir = root.join("app");
        let src_dir = project_dir.join("src");
        fs::create_dir_all(&src_dir).expect("src dir");
        fs::write(
            project_dir.join("project.json"),
            project_manifest(&project_dir),
        )
        .expect("project manifest");
        fs::write(src_dir.join("main_a.mfb"), "SUB main()\nEND SUB\n").expect("main_a");
        fs::write(
            src_dir.join("main_b.mfb"),
            "FUNC main(args AS List OF String) AS Integer\n  RETURN 0\nEND FUNC\n",
        )
        .expect("main_b");

        let manifest_contents =
            fs::read_to_string(project_dir.join("project.json")).expect("manifest contents");
        let manifest = parse_project_json(&manifest_contents, &project_dir.join("project.json"))
            .expect("manifest");
        let ast = ast::parse_project("app", &project_dir, &manifest).expect("ast");

        assert!(validate_entry_point(&project_dir, &manifest, &ast).is_err());

        fs::remove_dir_all(root).expect("remove temp dir");
    }

    fn test_temp_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "mfb_{name}_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp dir");
        root
    }
}
