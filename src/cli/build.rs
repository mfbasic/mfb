use std::path::PathBuf;

use crate::ast;
use crate::binary_repr;
use crate::ir;
use crate::json_string;
use crate::manifest::entry::validate_entry_point;
use crate::manifest::package::{
    external_package_function_types, external_package_function_types_from_files,
    installed_package_files, package_metadata,
};
use crate::manifest::project_kind;
use crate::manifest::validate_project_manifest;
use crate::monomorph;
use crate::resolver;
use crate::rules;
use crate::target;
use crate::typecheck;

pub(crate) struct BuildOptions {
    pub(crate) location: PathBuf,
    pub(crate) output: BuildOutput,
    pub(crate) target: target::BuildTarget,
    pub(crate) sign_owner: Option<String>,
    pub(crate) app_mode: bool,
}

pub(crate) enum BuildOutput {
    Validate,
    Ast,
    Ir,
    BinaryRepr,
    NativeIr,
    NativePlan,
    NativeObjectPlan,
    NativeCodePlan,
}

pub(crate) fn parse_build_options(args: Vec<String>) -> Result<BuildOptions, String> {
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

pub(crate) fn build_project(options: &BuildOptions) -> Result<(), ()> {
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

pub(crate) struct BuildSigningInfo {
    pub(crate) owner: String,
    pub(crate) ident_key: String,
    pub(crate) ident_fingerprint: String,
    pub(crate) signing_fingerprint: String,
    pub(crate) private_key: Vec<u8>,
    pub(crate) executable_metadata: Vec<u8>,
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

pub(crate) fn apply_signing_metadata(
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
