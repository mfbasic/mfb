use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tinyjson::JsonValue;

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
use crate::syntaxcheck;

pub(crate) struct BuildOptions {
    pub(crate) location: PathBuf,
    /// Requested artifact dumps, in flag order. Empty means a full
    /// validate/build (the flagless `mfb build`). Any combination of the
    /// output flags may be given in one invocation; each artifact is written
    /// from a single shared front-end pass.
    pub(crate) outputs: Vec<BuildOutput>,
    pub(crate) target: target::BuildTarget,
    pub(crate) sign_owner: Option<String>,
    pub(crate) app_mode: bool,
    /// Register-allocation strategy selected by `-regalloc <name>` (plan-03
    /// §4.2). Defaults to the backend default.
    pub(crate) regalloc: target::shared::code::regalloc::RegallocKind,
    /// `--unsigned`: opt into building against unsigned dependencies whose
    /// source is not local (audit-1 PKG-01). Unsigned *local* (`file:`/`local:`)
    /// dependencies are always permitted; this flag additionally allows unsigned
    /// dependencies pulled from a remote/registry source.
    pub(crate) allow_unsigned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuildOutput {
    Ast,
    Ir,
    BinaryRepr,
    NativeIr,
    NativePlan,
    NativeObjectPlan,
    NativeCodePlan,
    /// Target-neutral MIR dump (`-mir`, plan-00-A §12a): the neutral counterpart
    /// to `-ncode`.
    Mir,
}

impl BuildOutput {
    fn from_flag(flag: &str) -> Option<BuildOutput> {
        match flag {
            "-ast" => Some(BuildOutput::Ast),
            "-ir" => Some(BuildOutput::Ir),
            "-br" => Some(BuildOutput::BinaryRepr),
            "-nir" => Some(BuildOutput::NativeIr),
            "-nplan" => Some(BuildOutput::NativePlan),
            "-nobj" => Some(BuildOutput::NativeObjectPlan),
            "-ncode" => Some(BuildOutput::NativeCodePlan),
            "-mir" => Some(BuildOutput::Mir),
            _ => None,
        }
    }
}

pub(crate) fn parse_build_options(args: Vec<String>) -> Result<BuildOptions, String> {
    let mut location = None;
    let mut outputs: Vec<BuildOutput> = Vec::new();
    let mut target = None;
    let mut sign_owner = None;
    let mut app_mode = false;
    let mut allow_unsigned = false;
    let mut regalloc = target::shared::code::regalloc::active_kind();
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if let Some(output) = BuildOutput::from_flag(&arg) {
            if outputs.contains(&output) {
                return Err(format!("mfb build got duplicate output flag `{arg}`"));
            }
            outputs.push(output);
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
        } else if arg == "--unsigned" {
            allow_unsigned = true;
        } else if arg == "-regalloc" {
            let Some(value) = iter.next() else {
                return Err("mfb build -regalloc requires a strategy name".to_string());
            };
            regalloc = target::shared::code::regalloc::parse_kind(&value)?;
        } else if let Some(value) = arg.strip_prefix("-regalloc=") {
            regalloc = target::shared::code::regalloc::parse_kind(value)?;
        } else if arg.starts_with('-') {
            return Err(format!("unknown build option `{arg}`"));
        } else if location.replace(PathBuf::from(&arg)).is_some() {
            return Err("mfb build accepts at most one [location]".to_string());
        }
    }

    Ok(BuildOptions {
        location: location.unwrap_or_else(|| PathBuf::from(".")),
        outputs,
        target: target.unwrap_or_else(target::BuildTarget::host),
        sign_owner,
        app_mode,
        regalloc,
        allow_unsigned,
    })
}

pub(crate) fn build_project(options: &BuildOptions) -> Result<(), ()> {
    // Record the register-allocation strategy for the native backend to read
    // during lowering (plan-03 §4.2).
    target::shared::code::regalloc::set_strategy(options.regalloc);
    let target = options.target.clone();
    let project_path = options.location.join("project.json");
    let manifest = validate_project_manifest(&project_path)?;
    let project_kind = project_kind(&manifest);

    // audit-1 PKG-01: verify every declared dependency's signature against a
    // project-pinned trust anchor before it is decoded, merged, or lowered, and
    // print a per-package verification report. A tampered signed dependency (or a
    // disallowed unsigned one) hard-fails the build with a non-zero exit.
    verify_and_report_packages(&options.location, &manifest, options.allow_unsigned)?;

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
    // plan-20-Z cutover: the semantic rules are split across two passes that
    // both run to completion (neither short-circuits the other) so a program
    // with errors of both kinds reports all of them:
    //   - `syntaxcheck` rejects the source-syntax rules — constructs total
    //     lowering erases (named arguments, EXIT flavors, inline-trap
    //     boundaries), which therefore cannot exist in IR or packages;
    //   - `ir::verify` runs on the source-lowered IR and is the sole rejecter
    //     for every rule ported off `syntaxcheck` — the same implementation that
    //     guards decoded package IR, so source and package are checked once.
    // Lowering is total (plan-20-D), so it is safe to run even when syntaxcheck
    // found errors. External package signatures are resolved on the package
    // path, so an empty external map suffices for the source functions here.
    // Both checkers collect (rather than print) so their diagnostics can be
    // merged and rendered in a single line-ordered pass; otherwise every
    // relocated `ir::verify` rule would print after all of syntaxcheck's,
    // scrambling the source-order sequence the goldens record (plan-20-Z).
    let syntaxcheck_diagnostics = syntaxcheck::check_project_collect(&options.location, &concrete_ast);
    let source_ir = ir::lower_project_with_external_functions(
        &concrete_ast,
        entry.clone(),
        &HashMap::new(),
        &HashMap::new(),
    );
    let verify_diagnostics = ir::verify_source_diagnostics(&source_ir, &options.location);
    let Ok(mut diagnostics) = syntaxcheck_diagnostics else {
        return Err(());
    };
    diagnostics.extend(verify_diagnostics);
    let had_error = !diagnostics.is_empty();
    crate::rules::render_pending(diagnostics);
    if had_error {
        return Err(());
    }
    let signing = match &options.sign_owner {
        Some(owner) if options.outputs.is_empty() => {
            // The proof and attestation pin the exact package identity, so the
            // signed ident/version are fixed here from the validated manifest
            // (plan-23 §3.3). A manifest without an ident gets the canonical
            // `<owner>#<name>` (stamped into the header by
            // apply_signing_metadata so header and proof agree).
            let version = manifest
                .get("version")
                .and_then(|value| value.get::<String>())
                .expect("validated project version");
            let manifest_ident = manifest
                .get("ident")
                .and_then(|value| value.get::<String>())
                .cloned()
                .unwrap_or_default();
            let ident = signing_ident(owner, project_name, &manifest_ident).map_err(|err| {
                eprintln!("error: {err}");
            })?;
            Some(load_build_signing_info(owner, &ident, version).map_err(|err| {
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

    if options.outputs.is_empty() {
        if project_kind == "executable" {
            let packages = installed_package_files(&options.location, &manifest).map_err(|err| {
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
            let packages = installed_package_files(&options.location, &manifest).map_err(|err| {
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
                signing.as_ref().map(|signing| &signing.package_signing),
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
        return Ok(());
    }

    // Artifact dumps. Any combination of output flags shares this one
    // front-end pass; `packages` and the merged IR are computed at most once
    // and each artifact writer then runs its own (unchanged) backend path.
    // Artifacts are written in flag order; the first failure stops the run.
    let mut packages_cache: Option<Vec<PathBuf>> = None;
    let mut ir_cache: Option<ir::IrProject> = None;
    for output in &options.outputs {
        // The -ast and -ir dumps work for every project kind; the native
        // dumps require an executable project.
        match output {
            BuildOutput::Ast => {
                let ast_path = ast::write_ast(&options.location, &ast).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
                println!("Wrote AST to {}", ast_path.display());
                continue;
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
                continue;
            }
            BuildOutput::BinaryRepr => {}
            BuildOutput::NativeIr
            | BuildOutput::NativePlan
            | BuildOutput::NativeObjectPlan
            | BuildOutput::NativeCodePlan
            | BuildOutput::Mir => {
                if project_kind == "package" {
                    let what = match output {
                        BuildOutput::NativeIr => "native IR",
                        BuildOutput::NativePlan => "native plan",
                        BuildOutput::NativeObjectPlan => "native object plan",
                        BuildOutput::NativeCodePlan => "native code plan",
                        _ => "MIR",
                    };
                    rules::show_general_diagnostic(
                        "PACKAGE_NATIVE_OUTPUT_UNSUPPORTED",
                        &format!("Package projects do not support {what} output; run `mfb build` to write a .mfp package."),
                    );
                    return Err(());
                }
            }
        }

        if packages_cache.is_none() {
            packages_cache = Some(
                installed_package_files(&options.location, &manifest).map_err(|err| {
                    eprintln!("error: {err}");
                })?,
            );
        }
        let packages = packages_cache.as_ref().expect("cached packages");
        if ir_cache.is_none() {
            let (external_functions, external_params) =
                external_package_function_types_from_files(packages).map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            ir_cache = Some(ir::lower_project_with_external_functions(
                &concrete_ast,
                entry.clone(),
                &external_functions,
                &external_params,
            ));
        }
        let ir = ir_cache.as_ref().expect("cached IR");

        match output {
            BuildOutput::BinaryRepr => {
                let version = manifest
                    .get("version")
                    .and_then(|value| value.get::<String>())
                    .expect("validated project version");
                // -br dumps this project's own structured Binary Representation. Imported
                // packages are decoded and merged only in the native consumption
                // path; the hex dump reflects the project's own IR, not a merge.
                let binary_repr_path =
                    binary_repr::write_binary_repr_hex(&options.location, ir, version).map_err(
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
                let nir_path =
                    match target::write_nir(&options.location, ir, &target, packages, build_mode) {
                        Ok(path) => path,
                        Err(err) => {
                            eprintln!("error: {err}");
                            return Err(());
                        }
                    };
                println!("Wrote native IR to {}", nir_path.display());
            }
            BuildOutput::NativePlan => {
                let plan_path = match target::write_native_plan(
                    &options.location,
                    ir,
                    &target,
                    packages,
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
                let object_path = match target::write_native_object_plan(
                    &options.location,
                    ir,
                    &target,
                    packages,
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
                let code_path = match target::write_native_code_plan(
                    &options.location,
                    ir,
                    &target,
                    packages,
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
            BuildOutput::Mir => {
                let mir_path =
                    match target::write_mir(&options.location, ir, &target, packages, build_mode) {
                        Ok(path) => path,
                        Err(err) => {
                            eprintln!("error: {err}");
                            return Err(());
                        }
                    };
                println!("Wrote MIR to {}", mir_path.display());
            }
            BuildOutput::Ast | BuildOutput::Ir => unreachable!("handled above"),
        }
    }

    Ok(())
}

pub(crate) struct BuildSigningInfo {
    pub(crate) owner: String,
    /// The signed package identity (`<owner>#<package>`), stamped into the
    /// header when the manifest declares no ident of its own.
    pub(crate) ident: String,
    pub(crate) ident_fingerprint: String,
    pub(crate) signing_fingerprint: String,
    /// The full signing bundle threaded to the package writer: ident key,
    /// one-off signing keypair, ident-signed proof, server-signed
    /// attestation. The one-off private key exists only here, in memory,
    /// and is discarded when the build ends (plan-23 §3.3).
    pub(crate) package_signing: target::package_mfp::PackageSigning,
    pub(crate) executable_metadata: Vec<u8>,
}

/// The identity a `--sign` build signs for: the manifest ident when declared
/// (which must belong to the signing owner), else `<owner>#<name>`.
fn signing_ident(owner: &str, name: &str, manifest_ident: &str) -> Result<String, String> {
    if manifest_ident.is_empty() {
        return Ok(format!("{owner}#{name}"));
    }
    let Some((ident_owner, _)) = manifest_ident.split_once('#') else {
        return Err(format!(
            "project ident `{manifest_ident}` must use <owner>#<package> to be signed"
        ));
    };
    if !ident_owner.eq_ignore_ascii_case(owner) {
        return Err(format!(
            "project ident `{manifest_ident}` does not belong to owner `{owner}`"
        ));
    }
    Ok(manifest_ident.to_string())
}

/// Assemble the plan-23 §3.3 signing bundle: generate the one-off signing
/// keypair, fetch the server attestation pre-registering it for this exact
/// package+version, and mint the ident-signed proof locally.
fn load_build_signing_info(
    owner: &str,
    ident: &str,
    version: &str,
) -> Result<BuildSigningInfo, String> {
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = super::local_paths_for_repo(&repo_url)?;

    // The account ident key must live on this machine (register or link).
    let ident_private = mfb_repository::local::read_ident_private_key(&paths, owner)?;
    let ident_public = mfb_repository::local::read_ident_public_key(&paths, owner)?;
    if mfb_repository::crypto::public_from_private(&ident_private)? != ident_public {
        return Err("local ident key files do not match each other".to_string());
    }
    let ident_fingerprint = mfb_repository::crypto::fingerprint(&ident_public);

    // One-off signing keypair: fresh for this build, discarded with it.
    let (signing_public, signing_private) = mfb_repository::crypto::generate_keypair();
    let signing_fingerprint = mfb_repository::crypto::fingerprint(&signing_public);

    // Fetch the attestation (verified against the pinned server key inside
    // the client) and cross-check that the server's current name↔ident
    // binding is the ident key this machine holds.
    let attestation_response = mfb_repository::client::request_attestation(
        &repo_url,
        &paths,
        owner,
        ident,
        version,
        &signing_fingerprint,
    )?;
    let attestation_fields: tinyjson::JsonValue = attestation_response
        .attestation
        .parse()
        .map_err(|_| "repository returned a malformed attestation".to_string())?;
    let attestation_field = |field: &str| -> Option<String> {
        attestation_fields[field].get::<String>().cloned()
    };
    if attestation_field("identFingerprint").as_deref() != Some(ident_fingerprint.as_str()) {
        return Err(
            "repository attestation names a different ident key than this machine holds; \
             re-link this machine or rotate the ident"
                .to_string(),
        );
    }
    if attestation_field("ident").as_deref() != Some(ident)
        || attestation_field("version").as_deref() != Some(version)
        || attestation_field("signingFingerprint").as_deref() != Some(signing_fingerprint.as_str())
    {
        return Err("repository attestation does not pin the requested package".to_string());
    }
    let attestation_sig = mfb_repository::crypto::decode_bytes(
        &attestation_response.attestation_signature,
        "attestationSignature",
    )?;

    // Mint the proof (plan-23 §5) and sign it with the ident key.
    let issued = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let proof = format!(
        "{{\"owner\":{},\"ident\":{},\"version\":{},\"identFingerprint\":{},\"signingFingerprint\":{},\"issued\":{}}}",
        json_string(owner),
        json_string(ident),
        json_string(version),
        json_string(&ident_fingerprint),
        json_string(&signing_fingerprint),
        issued,
    );
    let proof_sig = mfb_repository::crypto::sign(
        &ident_private,
        &mfb_repository::crypto::proof_signing_input(proof.as_bytes()),
    )?;

    let ident_key = format!(
        "ed25519:{}",
        mfb_repository::crypto::encode_bytes(&ident_public)
    );
    let signing_key = format!(
        "ed25519:{}",
        mfb_repository::crypto::encode_bytes(&signing_public)
    );
    let executable_metadata = executable_signing_metadata_json(
        owner,
        &ident_key,
        &ident_fingerprint,
        &signing_key,
        &signing_fingerprint,
        &proof,
        &mfb_repository::crypto::encode_bytes(&proof_sig),
        &attestation_response.attestation,
        &attestation_response.attestation_signature,
    )
    .into_bytes();

    Ok(BuildSigningInfo {
        owner: owner.to_string(),
        ident: ident.to_string(),
        ident_fingerprint,
        signing_fingerprint,
        package_signing: target::package_mfp::PackageSigning {
            ident_key,
            signing_key,
            signing_private,
            proof,
            proof_sig,
            attestation: attestation_response.attestation,
            attestation_sig,
        },
        executable_metadata,
    })
}

/// Result of verifying one installed dependency (audit-1 PKG-01).
#[derive(Clone, Copy, PartialEq, Eq)]
enum PackageVerification {
    /// `signature_type == 1`, the recomputed content hash matches, and the
    /// Ed25519 signature verifies against the project-pinned trust anchor.
    Verified,
    /// `signature_type == 0` — no signature present.
    Unsigned,
    /// A signed package that fails to verify (bad/absent trust anchor, hash
    /// mismatch, bad signature) or is otherwise malformed. Always fatal.
    Tampered,
}

impl PackageVerification {
    fn label(self) -> &'static str {
        match self {
            PackageVerification::Verified => "Verified",
            PackageVerification::Unsigned => "Unsigned",
            PackageVerification::Tampered => "Tampered",
        }
    }
}

/// Verify every declared dependency and print `uses <name> - [<state>]` for each
/// (audit-1 PKG-01). Verification is a hard build gate: all packages are checked
/// and reported first, then the build aborts with a non-zero exit if any package
/// is Tampered, or if an Unsigned package is not permitted by policy.
///
/// The trust anchor is the `identKey` pinned in the importing project's
/// `project.json` dependency entry — never the key embedded in the untrusted
/// file. Unsigned dependencies from a local source (`file:`/`local:`, or no
/// source) are permitted; unsigned dependencies from a remote source require the
/// `--unsigned` opt-in.
pub(crate) fn verify_and_report_packages(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    allow_unsigned: bool,
) -> Result<(), ()> {
    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Ok(());
    };

    let mut errors: Vec<String> = Vec::new();
    for entry in packages {
        let Some(object) = entry.get::<HashMap<String, JsonValue>>() else {
            continue;
        };
        let Some(name) = object.get("name").and_then(|value| value.get::<String>()) else {
            continue;
        };
        let source = object
            .get("source")
            .and_then(|value| value.get::<String>())
            .map(String::as_str)
            .unwrap_or_default();
        let trust_anchor = object
            .get("identKey")
            .or_else(|| object.get("ident_key"))
            .and_then(|value| value.get::<String>())
            .map(String::as_str);

        let package_file = project_dir
            .join("packages")
            .join(format!("{name}.mfp"));
        if !package_file.is_file() {
            // A missing dependency is reported by the later install check with a
            // more actionable message; do not emit a verification line for it.
            continue;
        }

        let state = classify_installed_package(&package_file, trust_anchor);
        println!("uses {name} - [{}]", state.label());
        match state {
            PackageVerification::Verified => {}
            PackageVerification::Unsigned => {
                if !source_is_local(source) && !allow_unsigned {
                    errors.push(format!(
                        "package `{name}` is unsigned but its source is not local; pass --unsigned to allow it"
                    ));
                }
            }
            PackageVerification::Tampered => {
                errors.push(format!(
                    "package `{name}` failed signature verification (tampered or untrusted); refusing to build"
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        for error in &errors {
            eprintln!("error: {error}");
        }
        Err(())
    }
}

/// A dependency `source` that resolves to a file on disk the project controls,
/// rather than a remote/registry fetch. Unsigned local dependencies are the
/// common local-development case and are permitted without `--unsigned`.
fn source_is_local(source: &str) -> bool {
    source.is_empty() || source.starts_with("file:") || source.starts_with("local:")
}

/// Classify an installed `.mfp` (audit-1 PKG-01) by the plan-23 §3.5 chain.
/// Any parse error is treated as Tampered — a malformed container on the
/// trusted import path is never benign.
///
/// Anchors: the `identKey` pinned in the importing project's `project.json`
/// (never the file-embedded key) and the registry key pinned as `server.pub`.
/// The chain walks pinned server key → attestation → pinned ident → proof →
/// one-off signing key → bytes; any swapped byte or key breaks a link.
fn classify_installed_package(path: &Path, trust_anchor: Option<&str>) -> PackageVerification {
    let Ok(bytes) = std::fs::read(path) else {
        return PackageVerification::Tampered;
    };
    let Ok(package) = mfb_repository::package::parse_mfp_package(&bytes) else {
        return PackageVerification::Tampered;
    };
    if package.signature_type == 0 {
        return PackageVerification::Unsigned;
    }
    // §3.5 step 1 — the header identKey must be the pinned ident key. A
    // signed package with no pinned anchor cannot be trusted (the
    // file-embedded key is attacker-controlled).
    let Some(trust_anchor) = trust_anchor else {
        return PackageVerification::Tampered;
    };
    let Ok(pinned_ident) = decode_trust_anchor(trust_anchor) else {
        return PackageVerification::Tampered;
    };
    let Ok(header_ident) =
        mfb_repository::package::decode_metadata_key(&package.ident_key, "identKey")
    else {
        return PackageVerification::Tampered;
    };
    if header_ident != pinned_ident {
        return PackageVerification::Tampered;
    }
    // §3.5 step 2 — the attestation verifies under the pinned registry key
    // and pins this exact package.
    let repo_url = mfb_repository::client::repo_url_from_env();
    let Ok(paths) = super::local_paths_for_repo(&repo_url) else {
        return PackageVerification::Tampered;
    };
    let Ok(server_key) = mfb_repository::local::read_pinned_server_key(&paths) else {
        // Verifying a registry-signed package requires the pinned registry
        // key; run any `mfb repo` command against the registry to pin it.
        return PackageVerification::Tampered;
    };
    let repo_fingerprint = mfb_repository::crypto::fingerprint(&server_key);
    if mfb_repository::package::verify_attestation(&package, &server_key, &repo_fingerprint)
        .is_err()
    {
        return PackageVerification::Tampered;
    }
    // §3.5 step 3 — the proof verifies under the (pinned) ident key.
    if mfb_repository::package::verify_proof(&package, &pinned_ident).is_err() {
        return PackageVerification::Tampered;
    }
    // §3.5 steps 4–5 — the package signature verifies under the one-off
    // signing key over the signed prefix, and the payload hash weld holds.
    if mfb_repository::package::verify_package_signature(&package).is_err() {
        return PackageVerification::Tampered;
    }
    if mfb_repository::package::verify_payload_hash(&package).is_err() {
        return PackageVerification::Tampered;
    }
    PackageVerification::Verified
}

/// Decode a pinned trust-anchor public key. Accepts the header key format
/// (`ed25519:<base64url>`) as well as a bare base64url key.
fn decode_trust_anchor(value: &str) -> Result<Vec<u8>, String> {
    mfb_repository::package::decode_metadata_key(value, "identKey")
}

pub(crate) fn apply_signing_metadata(
    metadata: &mut binary_repr::BinaryReprMetadata,
    signing: &BuildSigningInfo,
) {
    // The embedded manifest repeats the header identity (plan-23 §4): the
    // full ident key plus the fingerprints of the header's identKey and
    // signingKey. The signed ident is stamped too, so a manifest without an
    // ident of its own still matches the header's `<owner>#<name>`.
    metadata.ident = signing.ident.clone();
    metadata.ident_key = signing.package_signing.ident_key.clone();
    metadata.ident_fingerprint = signing.ident_fingerprint.clone();
    metadata.signing_fingerprint = signing.signing_fingerprint.clone();
    metadata.author = signing.owner.clone();
}

#[allow(clippy::too_many_arguments)]
fn executable_signing_metadata_json(
    owner: &str,
    ident_key: &str,
    ident_fingerprint: &str,
    signing_key: &str,
    signing_fingerprint: &str,
    proof: &str,
    proof_sig: &str,
    attestation: &str,
    attestation_sig: &str,
) -> String {
    format!(
        "{{\"format\":\"mfb-signing-v1\",\"owner\":{},\"author\":{},\"identKey\":{},\"identFingerprint\":{},\"signingKey\":{},\"signingFingerprint\":{},\"proof\":{},\"proofSignature\":{},\"attestation\":{},\"attestationSignature\":{},\"signatureType\":\"Ed25519\"}}\n",
        json_string(owner),
        json_string(owner),
        json_string(ident_key),
        json_string(ident_fingerprint),
        json_string(signing_key),
        json_string(signing_fingerprint),
        json_string(proof),
        json_string(proof_sig),
        json_string(attestation),
        json_string(attestation_sig),
    )
}
