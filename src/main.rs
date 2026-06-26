mod arch;
mod ast;
mod audit;
mod binary_repr;
mod builtins;
mod doc;
mod escape;
mod fmt;
mod internal_name;
mod ir;
mod lexer;
mod man;
mod monomorph;
mod numeric;
mod os;
mod resolver;
mod rules;
mod spec;
mod target;
mod typecheck;
mod unicode_backend;
mod unicode_runtime_tables;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process;
use tinyjson::JsonValue;

const USAGE: &str = "Usage: mfb <command> <arguments>\n\nCommands:\n  help                                 Show this message\n  init <location>                      Create a new MFBASIC executable project\n  init-pkg <location>                  Create a new MFBASIC package project\n  repo register <owner_name>           Register a repository owner\n  repo auth <owner_name>               Authenticate as a repository owner\n  pkg add <url>                        Add a compiled package to the current project\n  pkg info <package>                   Show information about a compiled package\n  pkg verify                           Verify packages declared by project.json\n  pkg publish <owner_name> <package>   Publish a signed package project\n  pkg doc <name-or-path> [--out file]  Render HTML docs from a compiled package\n  doc [--out file] [location]          Render HTML docs from package or file source\n  fmt [--check] [--indent N] [location] Format project source (indentation and capitalization)\n  build [--sign owner] [-ast|-ir|-br|-nir|-nplan|-nobj|-ncode] [-target os-arch] [-app] [location] Validate and build an MFBASIC project\n  audit [--format text|json] [--locked] [path] Report audit findings for a project\n  man [package] [function]             Show built-in package and function help
  spec [topic] [subtopic] [--all]      Show the MFBASIC language specification";

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
        Some("repo") => {
            let repo_args = args.collect::<Vec<_>>();
            if let Err(err) = run_repo_command(&repo_args) {
                match err {
                    RepoCommandError::Usage(message) => {
                        eprintln!("error: {message}");
                        process::exit(2);
                    }
                    RepoCommandError::Failed(message) => {
                        eprintln!("error: {message}");
                        process::exit(1);
                    }
                }
            }
        }
        Some("audit") => {
            let options = match audit::parse_options(args.collect()) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    process::exit(2);
                }
            };
            process::exit(audit::run(&options));
        }
        Some("man") => {
            let man_args = args.collect::<Vec<_>>();
            if let Err(err) = show_man(&man_args) {
                eprintln!("error: {err}");
                process::exit(2);
            }
        }
        Some("spec") => {
            let spec_args = args.collect::<Vec<_>>();
            if let Err(err) = show_spec(&spec_args) {
                eprintln!("error: {err}");
                process::exit(2);
            }
        }
        Some("doc") => {
            let doc_args = args.collect::<Vec<_>>();
            process::exit(run_doc_command(&doc_args));
        }
        Some("fmt") => {
            let fmt_args = args.collect::<Vec<_>>();
            process::exit(run_fmt_command(&fmt_args));
        }
        Some(command) => {
            eprintln!("error: unknown command '{command}'\n\n{USAGE}");
            process::exit(2);
        }
    }
}

enum RepoCommandError {
    Usage(String),
    Failed(String),
}

fn run_repo_command(args: &[String]) -> Result<(), RepoCommandError> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(RepoCommandError::Usage(
            "mfb repo requires register or auth".to_string(),
        ));
    };
    if args.len() != 2 {
        return Err(RepoCommandError::Usage(format!(
            "mfb repo {command} requires exactly one <owner_name>"
        )));
    }

    let owner = &args[1];
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = mfb_repository::local::LocalPaths::from_env().map_err(RepoCommandError::Failed)?;

    match command {
        "register" => {
            let response = mfb_repository::client::register(&repo_url, &paths, owner)
                .map_err(RepoCommandError::Failed)?;
            println!(
                "Registered owner {} with auth fingerprint {}",
                response.owner, response.auth_fingerprint
            );
            Ok(())
        }
        "auth" => {
            let response = mfb_repository::client::auth(&repo_url, &paths, owner)
                .map_err(RepoCommandError::Failed)?;
            println!(
                "Authenticated owner {} until {}",
                response.owner, response.expires_at
            );
            Ok(())
        }
        _ => Err(RepoCommandError::Usage(format!(
            "unknown mfb repo command '{command}'"
        ))),
    }
}

struct BuildOptions {
    location: PathBuf,
    output: BuildOutput,
    target: target::BuildTarget,
    sign_owner: Option<String>,
    app_mode: bool,
}

enum BuildOutput {
    Validate,
    Ast,
    Ir,
    BinaryRepr,
    NativeIr,
    NativePlan,
    NativeObjectPlan,
    NativeCodePlan,
}

fn parse_build_options(args: Vec<String>) -> Result<BuildOptions, String> {
    let mut location = None;
    let mut output = BuildOutput::Validate;
    let mut target = None;
    let mut sign_owner = None;
    let mut app_mode = false;
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
        } else if arg == "-br" {
            if !matches!(output, BuildOutput::Validate) {
                return Err("mfb build accepts only one output mode".to_string());
            }
            output = BuildOutput::BinaryRepr;
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
        } else if arg == "--sign" {
            let Some(value) = iter.next() else {
                return Err("mfb build --sign requires <owner_name>".to_string());
            };
            if sign_owner.replace(value).is_some() {
                return Err("mfb build accepts at most one --sign option".to_string());
            }
        } else if let Some(value) = arg.strip_prefix("--sign=") {
            if sign_owner.replace(value.to_string()).is_some() {
                return Err("mfb build accepts at most one --sign option".to_string());
            }
        } else if arg == "-app" {
            if app_mode {
                return Err("mfb build accepts at most one -app option".to_string());
            }
            app_mode = true;
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
        sign_owner,
        app_mode,
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

    // `mfb build -app` (plan-04-macos-app.md §5.1, plan-05-linux-app.md §5.1) is an
    // executable-only build flag supported on app-capable native targets (macOS via
    // AppKit, Linux via GTK4). Reject incompatible combinations up front, before any
    // lowering.
    if options.app_mode {
        if project_kind != "executable" {
            eprintln!("error: mfb build -app requires an executable project");
            return Err(());
        }
        if !target::target_supports_app_mode(&target) {
            eprintln!(
                "error: mfb build -app requires a macOS or Linux target (got {})",
                target.name()
            );
            return Err(());
        }
    }
    // The target OS selects the app toolkit and therefore the build mode. The CLI
    // has already verified the target supports app mode at this point.
    let build_mode = if options.app_mode {
        match target.os.as_str() {
            "linux" => target::NativeBuildMode::LinuxApp,
            _ => target::NativeBuildMode::MacApp,
        }
    } else {
        target::NativeBuildMode::Console
    };

    let project_name = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .expect("validated project name");
    let ast = ast::parse_project(project_name, &options.location, &manifest)?;
    resolver::resolve_project(&options.location, &manifest, &ast)?;
    let concrete_ast = monomorph::monomorphize_project(&options.location, &ast)?;
    // Skip DOC validation on the post-monomorph pass: monomorphization renames
    // overloaded/generic declarations, so their doc headers would falsely appear
    // unresolved. The original-AST pass above already validated them.
    resolver::resolve_project_with(&options.location, &manifest, &concrete_ast, false)?;
    let entry = validate_entry_point(&options.location, &manifest, &concrete_ast)?;
    typecheck::check_project(&options.location, &concrete_ast)?;
    let signing = match &options.sign_owner {
        Some(owner) if matches!(options.output, BuildOutput::Validate) => {
            Some(load_build_signing_info(owner).map_err(|err| {
                eprintln!("error: {err}");
            })?)
        }
        Some(_) => {
            eprintln!(
                "error: mfb build --sign is only supported for package and executable builds"
            );
            return Err(());
        }
        None => None,
    };

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
                let executable_paths = target::write_executable(
                    &options.location,
                    &ir,
                    &target,
                    &packages,
                    signing
                        .as_ref()
                        .map(|signing| signing.executable_metadata.as_slice()),
                    build_mode,
                )
                .map_err(|err| {
                    eprintln!("error: {err}");
                })?;
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
                let mut ir = ir::lower_project_with_external_functions(
                    &concrete_ast,
                    entry.clone(),
                    &external_functions,
                    &external_params,
                );
                // Collect documentation from the pre-monomorphization AST: it keeps
                // the original declaration names (and every overload), which the
                // monomorphized AST renames away, so overloaded/generic exported
                // declarations still get a `.mfp` doc entry (plan-09-doc.md §5).
                ir.docs = ir::collect_project_docs(&ast);
                let mut metadata = package_metadata(&manifest);
                if let Some(signing) = &signing {
                    apply_signing_metadata(&mut metadata, signing);
                }
                let package_path = target::write_package(
                    &options.location,
                    &ir,
                    &metadata,
                    &packages,
                    signing
                        .as_ref()
                        .map(|signing| signing.private_key.as_slice()),
                )
                .map_err(|err| {
                    eprintln!("error: {err}");
                })?;
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
        BuildOutput::BinaryRepr => {
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
            // -br dumps this project's own structured Binary Representation. Imported
            // packages are decoded and merged only in the native consumption
            // path; the hex dump reflects the project's own IR, not a merge.
            let binary_repr_path =
                binary_repr::write_binary_repr_hex(&options.location, &ir, version).map_err(
                    |err| {
                        eprintln!("error: {err}");
                    },
                )?;
            println!(
                "Wrote binary representation hex to {}",
                binary_repr_path.display()
            );
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
            let nir_path =
                match target::write_nir(&options.location, &ir, &target, &packages, build_mode) {
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
            let plan_path = match target::write_native_plan(
                &options.location,
                &ir,
                &target,
                &packages,
                build_mode,
            ) {
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
                build_mode,
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
            let code_path = match target::write_native_code_plan(
                &options.location,
                &ir,
                &target,
                &packages,
                build_mode,
            ) {
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

struct BuildSigningInfo {
    owner: String,
    ident_key: String,
    ident_fingerprint: String,
    signing_fingerprint: String,
    private_key: Vec<u8>,
    executable_metadata: Vec<u8>,
}

fn load_build_signing_info(owner: &str) -> Result<BuildSigningInfo, String> {
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = mfb_repository::local::LocalPaths::from_env()?;
    let signing_info = mfb_repository::client::signing_info(&repo_url, &paths, owner)?;
    let private_key = mfb_repository::local::read_private_key(&paths, owner)?;
    let local_public = mfb_repository::crypto::public_from_private(&private_key)?;
    let server_signing_public =
        mfb_repository::crypto::decode_bytes(&signing_info.signing_key, "signingKey")?;
    if local_public != server_signing_public {
        return Err("local private key does not match repository signing key".to_string());
    }
    let local_fingerprint = mfb_repository::crypto::fingerprint(&local_public);
    if local_fingerprint != signing_info.signing_fingerprint {
        return Err(
            "local private key fingerprint does not match repository signing key".to_string(),
        );
    }

    let ident_key = format!("ed25519:{}", signing_info.ident_key);
    let signing_key = format!("ed25519:{}", signing_info.signing_key);
    let executable_metadata = executable_signing_metadata_json(
        &signing_info.owner,
        &ident_key,
        &signing_info.ident_fingerprint,
        &signing_key,
        &signing_info.signing_fingerprint,
    )
    .into_bytes();

    Ok(BuildSigningInfo {
        owner: signing_info.owner,
        ident_key,
        ident_fingerprint: signing_info.ident_fingerprint,
        signing_fingerprint: signing_info.signing_fingerprint,
        private_key,
        executable_metadata,
    })
}

fn apply_signing_metadata(
    metadata: &mut binary_repr::BinaryReprMetadata,
    signing: &BuildSigningInfo,
) {
    metadata.ident_key = signing.ident_key.clone();
    metadata.ident_fingerprint = signing.ident_fingerprint.clone();
    metadata.signing_fingerprint = signing.signing_fingerprint.clone();
    metadata.author = signing.owner.clone();
}

fn executable_signing_metadata_json(
    owner: &str,
    ident_key: &str,
    ident_fingerprint: &str,
    signing_key: &str,
    signing_fingerprint: &str,
) -> String {
    format!(
        "{{\"format\":\"mfb-signing-v1\",\"owner\":{},\"author\":{},\"identKey\":{},\"identFingerprint\":{},\"signingKey\":{},\"signingFingerprint\":{},\"signatureType\":\"Ed25519\"}}\n",
        json_string(owner),
        json_string(owner),
        json_string(ident_key),
        json_string(ident_fingerprint),
        json_string(signing_key),
        json_string(signing_fingerprint),
    )
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

/// `mfb doc <path> [--out <file>]` — render HTML documentation from source
/// (plan-09-doc.md §6.1). Returns a process exit code.
fn run_doc_command(args: &[String]) -> i32 {
    let mut path: Option<&String> = None;
    let mut out: Option<&String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                let Some(file) = args.get(index) else {
                    eprintln!("error: mfb doc --out requires a file\n\n{USAGE}");
                    return 2;
                };
                out = Some(file);
            }
            flag if flag.starts_with("--") => {
                eprintln!("error: unknown flag `{flag}`\n\n{USAGE}");
                return 2;
            }
            _ => {
                if path.is_some() {
                    eprintln!("error: mfb doc accepts exactly one <path>\n\n{USAGE}");
                    return 2;
                }
                path = Some(&args[index]);
            }
        }
        index += 1;
    }
    // Like `mfb build`, the path defaults to the current directory.
    let path = path.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let out_path = out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("doc.html"));

    match build_source_doc_page(&path) {
        Ok((page, valid)) => {
            let html = doc::render_html(&page);
            if let Err(err) = fs::write(&out_path, html) {
                eprintln!("error: failed to write '{}': {err}", out_path.display());
                return 1;
            }
            println!("Wrote documentation to {}", out_path.display());
            // Diagnostics for invalid blocks were already printed to stderr.
            if valid {
                0
            } else {
                1
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

/// Parse a source path (project directory or single `.mfb` file), validate its
/// `DOC` blocks, and build a renderable page. The bool is `false` when any block
/// failed validation (diagnostics already emitted).
fn build_source_doc_page(path: &Path) -> Result<(doc::DocPage, bool), String> {
    if path.is_dir() {
        let project_path = path.join("project.json");
        let manifest = validate_project_manifest(&project_path)
            .map_err(|_| "project validation failed".to_string())?;
        let name = manifest
            .get("name")
            .and_then(|value| value.get::<String>())
            .cloned()
            .unwrap_or_else(|| "package".to_string());
        let ast = ast::parse_project(&name, path, &manifest)
            .map_err(|_| "failed to parse project source".to_string())?;
        let valid = resolver::resolve_project(path, &manifest, &ast).is_ok();
        Ok((doc::from_source(&ast), valid))
    } else {
        let contents = fs::read_to_string(path)
            .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("doc")
            .to_string();
        let display = path.to_string_lossy().replace('\\', "/");
        let file = ast::parse_source(path, &display, &contents)
            .map_err(|_| "failed to parse source file".to_string())?;
        let project = ast::AstProject {
            name: stem,
            files: vec![file],
        };
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let valid = resolver::validate_project_docs(parent, &project);
        Ok((doc::from_source(&project), valid))
    }
}

/// `mfb fmt [--indent N] [location]` — format MFBASIC source in place. Like
/// `mfb build` and `mfb doc`, the location defaults to the current directory and
/// may be a project directory (formats every selected `.mfb` file) or a single
/// `.mfb` file. Returns a process exit code.
fn run_fmt_command(args: &[String]) -> i32 {
    let mut location: Option<&String> = None;
    let mut indent: usize = 2;
    let mut check = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--check" {
            check = true;
        } else if let Some(value) = arg.strip_prefix("--indent=") {
            match parse_indent(value) {
                Ok(width) => indent = width,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    return 2;
                }
            }
        } else if arg == "--indent" {
            index += 1;
            let Some(value) = args.get(index) else {
                eprintln!("error: mfb fmt --indent requires a value\n\n{USAGE}");
                return 2;
            };
            match parse_indent(value) {
                Ok(width) => indent = width,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    return 2;
                }
            }
        } else if arg.starts_with("--") {
            eprintln!("error: unknown flag `{arg}`\n\n{USAGE}");
            return 2;
        } else if location.replace(arg).is_some() {
            eprintln!("error: mfb fmt accepts exactly one [location]\n\n{USAGE}");
            return 2;
        }
        index += 1;
    }

    let path = location
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    match format_path(&path, indent, check) {
        Ok(true) => 0,
        // `--check` found files that are not formatted (mfbasic.md §22).
        Ok(false) => 1,
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

fn parse_indent(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("mfb fmt --indent requires a non-negative integer (got `{value}`)"))
}

/// Format the source selected by `path`. Without `check`, rewrites files in
/// place and prints one line per file changed. With `check`, writes nothing and
/// reports files that are not formatted. Returns `Ok(false)` only in check mode
/// when at least one file would change (so the caller exits non-zero).
fn format_path(path: &Path, indent: usize, check: bool) -> Result<bool, String> {
    let files = if path.is_file() {
        if path.extension().and_then(|ext| ext.to_str()) != Some("mfb") {
            return Err(format!("`{}` is not a .mfb source file", path.display()));
        }
        vec![path.to_path_buf()]
    } else if path.is_dir() {
        let project_path = path.join("project.json");
        let manifest = validate_project_manifest(&project_path)
            .map_err(|_| "project validation failed".to_string())?;
        ast::selected_source_paths(path, &manifest)
            .map_err(|_| "failed to enumerate project source files".to_string())?
    } else {
        return Err(format!("no such file or directory: `{}`", path.display()));
    };

    let mut changed = 0;
    for file in &files {
        let original = fs::read_to_string(file)
            .map_err(|err| format!("failed to read '{}': {err}", file.display()))?;
        let formatted = fmt::format_source(&original, indent);
        if formatted == original {
            continue;
        }
        changed += 1;
        if check {
            println!("Not formatted: {}", file.display());
        } else {
            fs::write(file, &formatted)
                .map_err(|err| format!("failed to write '{}': {err}", file.display()))?;
            println!("Formatted {}", file.display());
        }
    }

    if check {
        if changed > 0 {
            rules::show_general_diagnostic(
                "FMT_CHECK_FAILED",
                &format!("{changed} file(s) are not formatted; run `mfb fmt` to fix."),
            );
            return Ok(false);
        }
        println!("All {} file(s) already formatted", files.len());
    } else if changed == 0 {
        println!("Already formatted: {} file(s) unchanged", files.len());
    }
    Ok(true)
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

/// `mfb spec [topic] [subtopic] [--width N] [--color|--no-color]`. Renders the
/// embedded Markdown specification to the terminal, reflowing to the terminal
/// width so tables stay readable.
fn show_spec(args: &[String]) -> Result<(), String> {
    let mut width: Option<usize> = None;
    let mut color: Option<bool> = None;
    let mut all = false;
    let mut positional: Vec<&str> = Vec::new();

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--all" => all = true,
            "--no-color" => color = Some(false),
            "--color" => color = Some(true),
            "--width" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "mfb spec --width requires a number".to_string())?;
                width = Some(parse_spec_width(value)?);
            }
            other if other.starts_with("--width=") => {
                width = Some(parse_spec_width(&other["--width=".len()..])?);
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option `{other}`\n\n{USAGE}"));
            }
            other => positional.push(other),
        }
    }

    let style = spec::render::Style {
        width: width.unwrap_or_else(detect_terminal_width),
        color: color.unwrap_or_else(|| std::io::stdout().is_terminal()),
    };

    match positional.as_slice() {
        [] => {
            if all {
                return Err(format!("mfb spec --all requires a topic\n\n{USAGE}"));
            }
            print_spec_index(&style);
            Ok(())
        }
        [package_name] => {
            let package = spec::package(package_name)
                .ok_or_else(|| unknown_spec_package_error(package_name))?;
            if all {
                print_spec_all(package, &style);
            } else {
                print_spec_package(package, &style);
            }
            Ok(())
        }
        [package_name, topic_name] => {
            if all {
                return Err(
                    "mfb spec --all cannot be combined with a subtopic".to_string()
                );
            }
            let package = spec::package(package_name)
                .ok_or_else(|| unknown_spec_package_error(package_name))?;
            let topic = spec::topic(package, topic_name).ok_or_else(|| {
                format!(
                    "unknown topic `{topic_name}` in spec `{package_name}`\n\nRun `mfb spec {package_name}` to list available topics."
                )
            })?;
            println!("{}", spec::render::render(topic.page, &style));
            Ok(())
        }
        _ => Err(format!("mfb spec accepts at most two arguments\n\n{USAGE}")),
    }
}

fn parse_spec_width(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("invalid --width value `{value}`"))
        .map(|width| width.clamp(20, 1000))
}

/// Terminal width for spec rendering. Prefer an explicit `COLUMNS` override,
/// then ask the terminal itself via `TIOCGWINSZ`, then fall back to the classic
/// 80 (also used when stdout is piped/redirected and has no window size).
fn detect_terminal_width() -> usize {
    if let Some(width) = env::var("COLUMNS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
    {
        return width.clamp(20, 1000);
    }
    if let Some(width) = terminal_width_from_ioctl() {
        return width.clamp(20, 1000);
    }
    80
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn terminal_width_from_ioctl() -> Option<usize> {
    use std::os::raw::{c_int, c_ulong};

    #[repr(C)]
    struct Winsize {
        rows: u16,
        cols: u16,
        xpixel: u16,
        ypixel: u16,
    }

    extern "C" {
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    }

    #[cfg(target_os = "macos")]
    const TIOCGWINSZ: c_ulong = 0x4008_7468;
    #[cfg(target_os = "linux")]
    const TIOCGWINSZ: c_ulong = 0x5413;

    let mut ws = Winsize {
        rows: 0,
        cols: 0,
        xpixel: 0,
        ypixel: 0,
    };
    // SAFETY: `ws` is a valid, properly aligned `winsize` that lives across the
    // call; `ioctl` only writes into it. Querying stdout (fd 1) on a non-tty
    // returns a non-zero status, which we treat as "unknown".
    let rc = unsafe { ioctl(1, TIOCGWINSZ, std::ptr::addr_of_mut!(ws)) };
    (rc == 0 && ws.cols > 0).then_some(ws.cols as usize)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn terminal_width_from_ioctl() -> Option<usize> {
    None
}

fn print_spec_index(style: &spec::render::Style) {
    println!("Usage: mfb spec [topic] [subtopic] [--all]");
    println!();
    println!("Show the MFBASIC language specification.");
    println!();
    println!("Examples:");
    println!("  mfb spec");
    println!("  mfb spec architecture");
    println!("  mfb spec architecture native");
    println!("  mfb spec architecture --all");
    println!();
    println!("Topics:");
    println!();
    let entries: Vec<(&str, &str)> = spec::packages()
        .iter()
        .map(|package| (package.name, package.summary.as_str()))
        .collect();
    print_spec_listing("Topic", &entries, style);
}

fn print_spec_package(package: &spec::SpecPackage, style: &spec::render::Style) {
    println!("{}", spec::render::render(package.overview, style));
    if !package.topics.is_empty() {
        println!();
        let entries: Vec<(&str, &str)> = package
            .topics
            .iter()
            .map(|topic| (topic.name, topic.summary.as_str()))
            .collect();
        print_spec_listing("Subtopic", &entries, style);
        println!();
        println!("Run `mfb spec {} <subtopic>` for details.", package.name);
    }
}

/// `mfb spec <topic> --all`: print the overview followed by every subtopic page,
/// each separated by a full-width rule, as one continuous document.
fn print_spec_all(package: &spec::SpecPackage, style: &spec::render::Style) {
    println!("{}", spec::render::render(package.overview, style));
    for topic in &package.topics {
        println!();
        println!("{}", "─".repeat(style.width));
        println!();
        println!("{}", spec::render::render(topic.page, style));
    }
}

/// Render a `(name, summary)` listing as a width-aware table through the spec
/// renderer, so the summary column wraps instead of running off the terminal.
fn print_spec_listing(heading: &str, entries: &[(&str, &str)], style: &spec::render::Style) {
    if entries.is_empty() {
        return;
    }
    let mut markdown = format!("| {heading} | Summary |\n| --- | --- |\n");
    for (name, summary) in entries {
        markdown.push_str(&format!(
            "| {} | {} |\n",
            escape_spec_cell(name),
            escape_spec_cell(summary),
        ));
    }
    println!("{}", spec::render::render(&markdown, style));
}

/// Escape a literal `|` so it stays inside its table cell rather than starting a
/// new column.
fn escape_spec_cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn unknown_spec_package_error(package_name: &str) -> String {
    let packages = spec::packages()
        .iter()
        .map(|package| package.name)
        .collect::<Vec<_>>()
        .join(", ");
    format!("unknown spec topic `{package_name}`\n\nAvailable topics: {packages}")
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
    binary_repr_major: u16,
    binary_repr_minor: u16,
    flags: u32,
    signature_type: u16,
    signature_length: usize,
    binary_repr_length: usize,
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
    let binary_repr_major = read_u16(&bytes, 12)?;
    let binary_repr_minor = read_u16(&bytes, 14)?;
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
    let binary_repr_length = read_u64(&bytes, offset)? as usize;
    offset = offset
        .checked_add(8)
        .and_then(|offset| offset.checked_add(binary_repr_length))
        .ok_or_else(|| "invalid .mfp binary representation length".to_string())?;
    if offset != bytes.len() {
        return Err("invalid .mfp binary representation length".to_string());
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
        binary_repr_major,
        binary_repr_minor,
        flags,
        signature_type,
        signature_length,
        binary_repr_length,
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
        let (line, column) = fallback_field_position(contents);
        rules::show_diagnostic(
            "PROJECT_JSON_REQUIRED_FIELD",
            "Required field `kind` is missing.",
            project_path,
            line,
            column,
            column + 1,
        );
        return false;
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
        .expect("validated project manifests must include a string `kind` field")
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
                    "package `{}` must be installed as '{}' before binary representation merging",
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
        for export in binary_repr::read_package_exports(package)? {
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
        let Ok(exports) = binary_repr::read_package_exports(package) else {
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
                    })
                    .collect(),
            );
        }
    }
    (functions, params)
}

fn package_export_function_type(export: &binary_repr::BinaryReprExport) -> String {
    let params = export
        .params
        .iter()
        .map(|param| param.type_.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let prefix = if export.isolated { "ISOLATED " } else { "" };
    format!("{prefix}FUNC({params}) AS {}", export.return_type)
}

fn package_metadata(manifest: &HashMap<String, JsonValue>) -> binary_repr::BinaryReprMetadata {
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
) -> Vec<binary_repr::BinaryReprDependency> {
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
            Some(binary_repr::BinaryReprDependency {
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
    "IMPORT io\n\nSUB main()\n  io::print(\"Hello World\")\nEND SUB\n".to_string()
}

fn package_source() -> String {
    "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_build_options_defaults_to_console_mode() {
        let options = parse_build_options(vec!["some/project".to_string()]).expect("options");
        assert!(!options.app_mode);
    }

    #[test]
    fn parse_build_options_accepts_app_flag() {
        let options = parse_build_options(vec!["-app".to_string(), "some/project".to_string()])
            .expect("options");
        assert!(options.app_mode);
    }

    #[test]
    fn parse_build_options_rejects_duplicate_app_flag() {
        let result = parse_build_options(vec!["-app".to_string(), "-app".to_string()]);
        match result {
            Err(err) => assert!(err.contains("at most one -app")),
            Ok(_) => panic!("duplicate -app must be rejected"),
        }
    }

    #[test]
    fn parse_build_options_app_flag_composes_with_native_output() {
        let options =
            parse_build_options(vec!["-app".to_string(), "-nir".to_string()]).expect("options");
        assert!(options.app_mode);
        assert!(matches!(options.output, BuildOutput::NativeIr));
    }

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

    #[test]
    fn validate_project_manifest_rejects_missing_kind() {
        let root = test_temp_dir("validate_project_manifest_rejects_missing_kind");
        let project_dir = root.join("app");
        fs::create_dir_all(&project_dir).expect("project dir");
        fs::write(
            project_dir.join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"sources\": [{ \"root\": \"src\" }]\n",
                "}\n"
            ),
        )
        .expect("project manifest");

        assert!(validate_project_manifest(&project_dir.join("project.json")).is_err());

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
