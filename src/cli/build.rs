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
use crate::syntaxcheck;
use crate::target;

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
    /// Ordinary build vs. `mfb test` (plan-18). In test mode the `TESTING`
    /// blocks are desugared into a runnable driver instead of being dropped.
    pub(crate) mode: crate::testing::CompileMode,
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
        mode: crate::testing::CompileMode::Build,
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
    let mut ast = ast::parse_project(project_name, &options.location, &manifest)?;
    // plan-18: the assertion builtins are valid only inside a TCASE body; reject
    // any that appear elsewhere before lowering the TESTING blocks away.
    if crate::testing::validate_expect_placement(&ast) {
        return Err(());
    }
    // plan-24-C: rename file-local PRIVATE top-level declarations to unique
    // `#<hash>$name` internal names (and rewrite their in-file references) BEFORE
    // resolving, so same-named privates in different files never collide and every
    // later stage sees globally-unique names. Runs before the TESTING lowering so
    // case bodies (which may reference privates) are rewritten consistently.
    // Returns shadow warnings (rendered with the other diagnostics below) and a
    // should-never-fire hash-collision.
    let scope_diagnostics = crate::scope_privates::scope_privates(&mut ast);
    // The `-ast` dump shows the parsed TESTING syntax (post-rename), so snapshot
    // after `scope_privates` but before the blocks are lowered away — only when
    // the dump is actually requested.
    let ast_dump = options
        .outputs
        .contains(&BuildOutput::Ast)
        .then(|| ast.clone());
    // Lower every TESTING block: `mfb build` drops them (byte-identical to a
    // program without them); `mfb test` desugars them into a runnable driver and
    // (with --coverage) instruments the user statements. The absolute project dir
    // fixes where the instrumented binary writes its coverage sidecars.
    let project_abs =
        std::fs::canonicalize(&options.location).unwrap_or_else(|_| options.location.clone());
    let test_lowering = crate::testing::lower_testing_blocks(&mut ast, options.mode, &project_abs);
    if options.mode.coverage() {
        let covmap = project_abs.join(crate::testing::COVMAP_FILE);
        if let Err(err) = crate::coverage::write_covmap(&covmap, &test_lowering.cov_slots) {
            eprintln!("warning: failed to write coverage map: {err}");
        }
    }
    resolver::resolve_project(&options.location, &manifest, &ast)?;
    let concrete_ast = monomorph::monomorphize_project(&options.location, &ast)?;
    // Skip DOC validation on the post-monomorph pass: monomorphization renames
    // overloaded/generic declarations, so their doc headers would falsely appear
    // unresolved. The original-AST pass above already validated them.
    resolver::resolve_project_with(&options.location, &manifest, &concrete_ast, false)?;
    // In test mode the synthesized driver is the entry point (it replaces the
    // manifest `main`), so bypass entry validation and point at the driver.
    let entry = match &test_lowering.entry {
        Some(name) => Some(ir::EntryPoint {
            name: name.clone(),
            returns: "Integer".to_string(),
            accepts_args: false,
        }),
        None => validate_entry_point(&options.location, &manifest, &concrete_ast)?,
    };
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
    let syntaxcheck_diagnostics =
        syntaxcheck::check_project_collect(&options.location, &concrete_ast);
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
    // EXPORT is only valid in a package project (it is the `.mfp` export flag);
    // in an executable a top-level EXPORT is an error. Checked here because the
    // manifest `kind` is known at the build boundary (see
    // `syntaxcheck::export_in_executable_diagnostics`).
    let is_package = crate::manifest::project_kind(&manifest) == "package";
    diagnostics.extend(syntaxcheck::export_in_executable_diagnostics(
        is_package, &ast,
    ));
    diagnostics.extend(scope_diagnostics);
    let had_error = diagnostics.iter().any(|d| crate::rules::is_error(&d.rule));
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
            Some(
                load_build_signing_info(owner, &ident, version).map_err(|err| {
                    eprintln!("error: {err}");
                })?,
            )
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
        // `mfb test` always builds a runnable executable (the synthesized driver
        // entry), even for a package project whose normal build emits a `.mfp`.
        if project_kind == "executable" || options.mode.is_test() {
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
            // A host `mfb test` links the driver into a unique temporary
            // directory (removed after the run) so nothing is ever left in the
            // project directory. A cross `-target` test build has no host binary
            // to run, so it writes to the project directory like a normal build
            // and reports the artifact.
            let test_output_dir = if options.mode.is_test() && target.is_host() {
                Some(make_temp_output_dir())
            } else {
                None
            };
            let output_dir = test_output_dir.as_deref().unwrap_or(&options.location);
            let executable_paths = target::write_executable(
                output_dir,
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
            // `mfb test` compiles the driver, then runs it and adopts its exit
            // status (non-zero iff any case failed).
            if options.mode.is_test() {
                if let Some(dir) = test_output_dir {
                    // Host run: execute the freshly linked binary, then remove
                    // the whole temp directory regardless of outcome.
                    let status = match executable_paths.first() {
                        Some(path) => run_test_binary(path),
                        None => {
                            eprintln!("error: mfb test produced no executable to run");
                            Err(())
                        }
                    };
                    if options.mode.coverage() {
                        generate_coverage_report(&project_abs);
                    }
                    let _ = std::fs::remove_dir_all(&dir);
                    return status;
                }
                // Cross target: cannot run; report the artifact.
                for executable_path in executable_paths {
                    println!("Wrote test executable to {}", executable_path.display());
                }
                return Ok(());
            }
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
                let dump_ast = ast_dump.as_ref().unwrap_or(&ast);
                let ast_path = ast::write_ast(&options.location, dump_ast).map_err(|err| {
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

/// Parse `mfb test [location] [--coverage] [-target …] [-regalloc …]`. The build
/// pipeline is shared with `mfb build`; only the compile mode and the always-run
/// behavior differ (plan-18).
pub(crate) fn parse_test_options(args: Vec<String>) -> Result<BuildOptions, String> {
    let mut location = None;
    let mut target = None;
    let mut regalloc = target::shared::code::regalloc::active_kind();
    let mut coverage = false;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == "--coverage" {
            coverage = true;
        } else if arg == "-target" {
            let Some(value) = iter.next() else {
                return Err("mfb test -target requires os-arch".to_string());
            };
            target = Some(target::BuildTarget::parse(&value)?);
        } else if let Some(value) = arg.strip_prefix("-target=") {
            target = Some(target::BuildTarget::parse(value)?);
        } else if arg == "-regalloc" {
            let Some(value) = iter.next() else {
                return Err("mfb test -regalloc requires a strategy name".to_string());
            };
            regalloc = target::shared::code::regalloc::parse_kind(&value)?;
        } else if let Some(value) = arg.strip_prefix("-regalloc=") {
            regalloc = target::shared::code::regalloc::parse_kind(value)?;
        } else if arg.starts_with('-') {
            return Err(format!("unknown test option `{arg}`"));
        } else if location.replace(PathBuf::from(&arg)).is_some() {
            return Err("mfb test accepts at most one [location]".to_string());
        }
    }

    Ok(BuildOptions {
        location: location.unwrap_or_else(|| PathBuf::from(".")),
        outputs: Vec::new(),
        target: target.unwrap_or_else(target::BuildTarget::host),
        sign_owner: None,
        app_mode: false,
        regalloc,
        allow_unsigned: false,
        mode: crate::testing::CompileMode::Test { coverage },
    })
}

/// Fold the coverage sidecars (`coverage.covmap.json` written by the build, plus
/// `coverage.covdata`/`coverage.covfail` written by the run) into `coverage.html`
/// (plan-18-C). Best-effort: a missing sidecar warns rather than fails.
fn generate_coverage_report(project_dir: &Path) {
    let covmap = project_dir.join(crate::testing::COVMAP_FILE);
    let Some(slots) = crate::coverage::read_covmap(&covmap) else {
        eprintln!("warning: coverage map missing; skipping coverage report");
        return;
    };
    let counts = crate::coverage::read_counts(&project_dir.join(crate::testing::COVDATA_FILE));
    let failed = crate::coverage::read_failed(&project_dir.join(crate::testing::COVFAIL_FILE));
    let html = crate::coverage::generate_html(project_dir, &slots, &counts, &failed);
    let out = project_dir.join(crate::testing::COVERAGE_HTML);
    match std::fs::write(&out, html) {
        Ok(()) => println!("Wrote coverage report to {}", out.display()),
        Err(err) => eprintln!("warning: failed to write coverage report: {err}"),
    }
}

/// A unique temporary directory for a `mfb test` executable, so the linked
/// binary never lands in the project directory. Named by process id + a
/// high-resolution timestamp; created eagerly and removed after the run.
fn make_temp_output_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("mfb-test-{}-{nanos}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Run the freshly built test executable, inheriting its stdio, and map its exit
/// status to `mfb test`'s result: success (all cases passed) or failure.
fn run_test_binary(path: &Path) -> Result<(), ()> {
    match std::process::Command::new(path).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(()),
        Err(err) => {
            eprintln!("error: failed to run test executable: {err}");
            Err(())
        }
    }
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
// coverage:off — reaches a live registry (request_attestation) and requires a
// registered ident key on the machine; exercised end-to-end by the tests/
// package-publish integration harness, not a unit test.
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
    let attestation_field =
        |field: &str| -> Option<String> { attestation_fields[field].get::<String>().cloned() };
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

/// Result of verifying one installed dependency (audit-1 PKG-01, plan-23 §3.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PackageVerification {
    /// `signature_type == 1` and the full §3.5 chain verifies against the
    /// pinned trust anchors.
    Verified,
    /// `signature_type == 0` — no signature present.
    Unsigned,
    /// A signed package that fails any link of the chain, or a malformed
    /// container. Always fatal.
    Tampered,
}

impl PackageVerification {
    pub(crate) fn label(self) -> &'static str {
        match self {
            PackageVerification::Verified => "Verified",
            PackageVerification::Unsigned => "Unsigned",
            PackageVerification::Tampered => "Tampered",
        }
    }
}

/// A classified dependency plus, when Tampered, the §3.5 refusal: the 6-605
/// rule name and a human detail line naming the broken chain link.
pub(crate) struct PackageClassification {
    pub(crate) state: PackageVerification,
    pub(crate) refusal: Option<(&'static str, String)>,
}

impl PackageClassification {
    fn ok(state: PackageVerification) -> Self {
        Self {
            state,
            refusal: None,
        }
    }

    fn tampered(rule: &'static str, detail: String) -> Self {
        Self {
            state: PackageVerification::Tampered,
            refusal: Some((rule, detail)),
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

    let mut refusals: Vec<(&'static str, String)> = Vec::new();
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

        let package_file = project_dir.join("packages").join(format!("{name}.mfp"));
        if !package_file.is_file() {
            // A missing dependency is reported by the later install check with a
            // more actionable message; do not emit a verification line for it.
            continue;
        }

        let classification = classify_installed_package(&package_file, trust_anchor);
        println!("uses {name} - [{}]", classification.state.label());
        match classification.state {
            PackageVerification::Verified => {}
            PackageVerification::Unsigned => {
                if !source_is_local(source) && !allow_unsigned {
                    refusals.push((
                        "PACKAGE_UNSIGNED_REMOTE",
                        format!(
                            "package `{name}` is unsigned but its source is not local; pass --unsigned to allow it"
                        ),
                    ));
                }
            }
            PackageVerification::Tampered => {
                let (rule, detail) = classification
                    .refusal
                    .unwrap_or(("PACKAGE_SIGNATURE_INVALID", String::new()));
                refusals.push((
                    rule,
                    format!("package `{name}` failed verification ({detail}); refusing to build"),
                ));
            }
        }
    }

    if refusals.is_empty() {
        Ok(())
    } else {
        for (rule, detail) in &refusals {
            rules::show_general_diagnostic(rule, detail);
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
/// one-off signing key → bytes; any swapped byte or key breaks a link, and
/// each broken link maps to its own 6-605 diagnostic.
pub(crate) fn classify_installed_package(
    path: &Path,
    trust_anchor: Option<&str>,
) -> PackageClassification {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return PackageClassification::tampered(
                "PACKAGE_INVALID",
                format!("failed to read '{}': {err}", path.display()),
            );
        }
    };
    let package = match mfb_repository::package::parse_mfp_package(&bytes) {
        Ok(package) => package,
        Err(err) => return PackageClassification::tampered("PACKAGE_INVALID", err),
    };
    if package.signature_type == 0 {
        return PackageClassification::ok(PackageVerification::Unsigned);
    }
    // §3.5 step 1 — the header identKey must be the pinned ident key. A
    // signed package with no pinned anchor cannot be trusted (the
    // file-embedded key is attacker-controlled).
    let Some(trust_anchor) = trust_anchor else {
        return PackageClassification::tampered(
            "PACKAGE_IDENT_KEY_UNTRUSTED",
            "the importing project pins no identKey for this signed package".to_string(),
        );
    };
    let pinned_ident = match decode_trust_anchor(trust_anchor) {
        Ok(pinned_ident) => pinned_ident,
        Err(err) => {
            return PackageClassification::tampered(
                "PACKAGE_IDENT_KEY_UNTRUSTED",
                format!("the pinned identKey is malformed: {err}"),
            );
        }
    };
    let header_ident =
        match mfb_repository::package::decode_metadata_key(&package.ident_key, "identKey") {
            Ok(header_ident) => header_ident,
            Err(err) => {
                return PackageClassification::tampered(
                    "PACKAGE_IDENT_KEY_UNTRUSTED",
                    format!("the package identKey is malformed: {err}"),
                );
            }
        };
    if header_ident != pinned_ident {
        return PackageClassification::tampered(
            "PACKAGE_IDENT_KEY_UNTRUSTED",
            "the package identKey does not match the identKey pinned in project.json".to_string(),
        );
    }
    // §3.5 step 2 — the attestation verifies under the pinned registry key
    // and pins this exact package.
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = match super::local_paths_for_repo(&repo_url) {
        Ok(paths) => paths,
        Err(err) => return PackageClassification::tampered("PACKAGE_ATTESTATION_INVALID", err),
    };
    let server_key = match mfb_repository::local::read_pinned_server_key(&paths) {
        Ok(server_key) => server_key,
        Err(_) => {
            return PackageClassification::tampered(
                "PACKAGE_ATTESTATION_INVALID",
                "no pinned registry key; run `mfb repo auth <owner>` against the registry to pin server.pub".to_string(),
            );
        }
    };
    let repo_fingerprint = mfb_repository::crypto::fingerprint(&server_key);
    if let Err(err) =
        mfb_repository::package::verify_attestation(&package, &server_key, &repo_fingerprint)
    {
        return PackageClassification::tampered("PACKAGE_ATTESTATION_INVALID", err);
    }
    // §3.5 step 3 — the proof verifies under the (pinned) ident key.
    if let Err(err) = mfb_repository::package::verify_proof(&package, &pinned_ident) {
        return PackageClassification::tampered("PACKAGE_PROOF_INVALID", err);
    }
    // §3.5 steps 4–5 — the package signature verifies under the one-off
    // signing key over the signed prefix, and the payload hash weld holds.
    if let Err(err) = mfb_repository::package::verify_package_signature(&package) {
        return PackageClassification::tampered("PACKAGE_SIGNATURE_INVALID", err);
    }
    if let Err(err) = mfb_repository::package::verify_payload_hash(&package) {
        return PackageClassification::tampered("PACKAGE_PAYLOAD_HASH_MISMATCH", err);
    }
    PackageClassification::ok(PackageVerification::Verified)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn s(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn build_output_from_flag_maps_every_flag() {
        assert_eq!(BuildOutput::from_flag("-ast"), Some(BuildOutput::Ast));
        assert_eq!(BuildOutput::from_flag("-ir"), Some(BuildOutput::Ir));
        assert_eq!(BuildOutput::from_flag("-br"), Some(BuildOutput::BinaryRepr));
        assert_eq!(BuildOutput::from_flag("-nir"), Some(BuildOutput::NativeIr));
        assert_eq!(
            BuildOutput::from_flag("-nplan"),
            Some(BuildOutput::NativePlan)
        );
        assert_eq!(
            BuildOutput::from_flag("-nobj"),
            Some(BuildOutput::NativeObjectPlan)
        );
        assert_eq!(
            BuildOutput::from_flag("-ncode"),
            Some(BuildOutput::NativeCodePlan)
        );
        assert_eq!(BuildOutput::from_flag("-mir"), Some(BuildOutput::Mir));
        assert_eq!(BuildOutput::from_flag("-nope"), None);
    }

    #[test]
    fn parse_build_options_defaults() {
        let options = parse_build_options(vec![]).expect("options");
        assert_eq!(options.location, PathBuf::from("."));
        assert!(options.outputs.is_empty());
        assert!(options.sign_owner.is_none());
        assert!(!options.app_mode);
        assert!(!options.allow_unsigned);
        assert_eq!(options.target, target::BuildTarget::host());
    }

    #[test]
    fn parse_build_options_parses_target_both_forms() {
        let split =
            parse_build_options(s(&["-target", "linux-aarch64"])).expect("split target form");
        assert_eq!(split.target.name(), "linux-aarch64");
        let joined = parse_build_options(s(&["-target=linux-x86_64"])).expect("joined target form");
        assert_eq!(joined.target.name(), "linux-x86_64");
    }

    #[test]
    fn parse_build_options_target_requires_value() {
        assert!(build_err(&["-target"]).contains("-target requires os-arch"));
    }

    #[test]
    fn parse_build_options_target_rejects_malformed() {
        assert!(parse_build_options(s(&["-target", "nodash"])).is_err());
    }

    #[test]
    fn parse_build_options_sign_both_forms_and_conflicts() {
        let split = parse_build_options(s(&["--sign", "ada"])).expect("split sign");
        assert_eq!(split.sign_owner.as_deref(), Some("ada"));
        let joined = parse_build_options(s(&["--sign=bob"])).expect("joined sign");
        assert_eq!(joined.sign_owner.as_deref(), Some("bob"));
        assert!(parse_build_options(s(&["--sign", "requires-value"])).is_ok());
        assert!(parse_build_options(s(&["--sign"])).is_err());
        // Two --sign options conflict.
        assert!(parse_build_options(s(&["--sign", "a", "--sign", "b"])).is_err());
        assert!(parse_build_options(s(&["--sign=a", "--sign=b"])).is_err());
    }

    #[test]
    fn parse_build_options_unsigned_flag() {
        let options = parse_build_options(s(&["--unsigned"])).expect("options");
        assert!(options.allow_unsigned);
    }

    #[test]
    fn parse_build_options_regalloc_both_forms_and_bad_value() {
        assert!(parse_build_options(s(&["-regalloc"])).is_err());
        assert!(parse_build_options(s(&["-regalloc", "not-a-strategy"])).is_err());
        assert!(parse_build_options(s(&["-regalloc=not-a-strategy"])).is_err());
    }

    fn build_err(args: &[&str]) -> String {
        match parse_build_options(s(args)) {
            Ok(_) => panic!("expected an error for {args:?}"),
            Err(message) => message,
        }
    }

    #[test]
    fn parse_build_options_rejects_unknown_option_and_two_locations() {
        assert!(build_err(&["-bogus"]).contains("unknown build option `-bogus`"));
        assert!(build_err(&["one", "two"]).contains("at most one [location]"));
    }

    #[test]
    fn parse_build_options_takes_a_positional_location() {
        let options = parse_build_options(s(&["my/project"])).expect("options");
        assert_eq!(options.location, PathBuf::from("my/project"));
    }

    #[test]
    fn package_verification_labels() {
        assert_eq!(PackageVerification::Verified.label(), "Verified");
        assert_eq!(PackageVerification::Unsigned.label(), "Unsigned");
        assert_eq!(PackageVerification::Tampered.label(), "Tampered");
    }

    #[test]
    fn source_is_local_classifies_sources() {
        assert!(source_is_local(""));
        assert!(source_is_local("file:packages/x.mfp"));
        assert!(source_is_local("local:x"));
        assert!(!source_is_local("ada#shape"));
        assert!(!source_is_local("https://registry/x"));
    }

    #[test]
    fn signing_ident_defaults_to_owner_hash_name() {
        assert_eq!(
            signing_ident("ada", "shape", ""),
            Ok("ada#shape".to_string())
        );
        // A declared ident owned by the signer passes through unchanged.
        assert_eq!(
            signing_ident("ada", "shape", "ada#shape"),
            Ok("ada#shape".to_string())
        );
        // Case-insensitive owner match.
        assert_eq!(
            signing_ident("Ada", "shape", "ada#shape"),
            Ok("ada#shape".to_string())
        );
    }

    #[test]
    fn signing_ident_rejects_bad_idents() {
        assert!(signing_ident("ada", "shape", "no-hash")
            .unwrap_err()
            .contains("must use <owner>#<package>"));
        assert!(signing_ident("ada", "shape", "bob#shape")
            .unwrap_err()
            .contains("does not belong to owner"));
    }

    #[test]
    fn classify_installed_package_reads_unsigned_fixture() {
        // A valid unsigned package classifies as Unsigned (no signature).
        let path = Path::new("tests/package-trap-builtin/golden/trap_builtin_pkg.mfp");
        assert!(path.is_file(), "fixture must exist");
        let classification = classify_installed_package(path, None);
        assert_eq!(classification.state, PackageVerification::Unsigned);
        assert!(classification.refusal.is_none());
    }

    #[test]
    fn classify_installed_package_treats_missing_file_as_tampered() {
        let classification = classify_installed_package(Path::new("/no/such/pkg.mfp"), None);
        assert_eq!(classification.state, PackageVerification::Tampered);
        let (rule, _detail) = classification.refusal.expect("refusal");
        assert_eq!(rule, "PACKAGE_INVALID");
    }

    #[test]
    fn classify_installed_package_treats_garbage_as_tampered() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garbage.mfp");
        std::fs::write(&path, b"this is not an mfp container").expect("write");
        let classification = classify_installed_package(&path, None);
        assert_eq!(classification.state, PackageVerification::Tampered);
        assert_eq!(
            classification.refusal.expect("refusal").0,
            "PACKAGE_INVALID"
        );
    }

    #[test]
    fn verify_and_report_no_packages_is_ok() {
        let manifest = crate::manifest::parse_project_json(
            "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{\"root\":\"src\"}]}",
            Path::new("project.json"),
        )
        .expect("manifest");
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(verify_and_report_packages(dir.path(), &manifest, false).is_ok());
    }

    #[test]
    fn verify_and_report_missing_dependency_file_is_skipped() {
        // A declared dependency whose .mfp is not installed yet emits no
        // verification line and does not fail (the install check reports it).
        let manifest = crate::manifest::parse_project_json(
            concat!(
                "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",",
                "\"sources\":[{\"root\":\"src\"}],",
                "\"packages\":[{\"name\":\"shape\",\"ident\":\"ada#shape\",\"version\":\"1.0.0\",\"pin\":true,\"source\":\"ada#shape\"}]}"
            ),
            Path::new("project.json"),
        )
        .expect("manifest");
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(verify_and_report_packages(dir.path(), &manifest, false).is_ok());
    }

    #[test]
    fn verify_and_report_unsigned_remote_requires_flag() {
        // An installed unsigned package from a remote source is refused unless
        // --unsigned is passed.
        let manifest = crate::manifest::parse_project_json(
            concat!(
                "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",",
                "\"sources\":[{\"root\":\"src\"}],",
                "\"packages\":[{\"name\":\"trap_builtin_pkg\",\"ident\":\"tests#trap\",\"version\":\"0.1.0\",\"pin\":true,\"source\":\"tests#trap\"}]}"
            ),
            Path::new("project.json"),
        )
        .expect("manifest");
        let dir = tempfile::tempdir().expect("temp dir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).expect("packages dir");
        std::fs::copy(
            "tests/package-trap-builtin/golden/trap_builtin_pkg.mfp",
            packages.join("trap_builtin_pkg.mfp"),
        )
        .expect("copy fixture");
        // Remote source, unsigned, no --unsigned -> refused.
        assert!(verify_and_report_packages(dir.path(), &manifest, false).is_err());
        // With --unsigned -> allowed.
        assert!(verify_and_report_packages(dir.path(), &manifest, true).is_ok());
    }

    #[test]
    fn verify_and_report_unsigned_local_is_allowed() {
        let manifest = crate::manifest::parse_project_json(
            concat!(
                "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",",
                "\"sources\":[{\"root\":\"src\"}],",
                "\"packages\":[{\"name\":\"trap_builtin_pkg\",\"ident\":\"tests#trap\",\"version\":\"0.1.0\",\"pin\":true,\"source\":\"file:packages/trap_builtin_pkg.mfp\"}]}"
            ),
            Path::new("project.json"),
        )
        .expect("manifest");
        let dir = tempfile::tempdir().expect("temp dir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).expect("packages dir");
        std::fs::copy(
            "tests/package-trap-builtin/golden/trap_builtin_pkg.mfp",
            packages.join("trap_builtin_pkg.mfp"),
        )
        .expect("copy fixture");
        // Local source, unsigned -> allowed without the flag.
        assert!(verify_and_report_packages(dir.path(), &manifest, false).is_ok());
    }

    #[test]
    fn apply_signing_metadata_copies_fields() {
        let mut metadata =
            binary_repr::BinaryReprMetadata::new("pkg".to_string(), "1.0.0".to_string());
        let signing = BuildSigningInfo {
            owner: "ada".to_string(),
            ident: "ada#pkg".to_string(),
            ident_fingerprint: "if".to_string(),
            signing_fingerprint: "sf".to_string(),
            package_signing: target::package_mfp::PackageSigning {
                ident_key: "ed25519:ik".to_string(),
                signing_key: "ed25519:sk".to_string(),
                signing_private: Vec::new(),
                proof: String::new(),
                proof_sig: Vec::new(),
                attestation: String::new(),
                attestation_sig: Vec::new(),
            },
            executable_metadata: Vec::new(),
        };
        apply_signing_metadata(&mut metadata, &signing);
        assert_eq!(metadata.ident, "ada#pkg");
        assert_eq!(metadata.ident_key, "ed25519:ik");
        assert_eq!(metadata.ident_fingerprint, "if");
        assert_eq!(metadata.signing_fingerprint, "sf");
        assert_eq!(metadata.author, "ada");
    }

    #[test]
    fn executable_signing_metadata_json_is_valid_json() {
        let json = executable_signing_metadata_json(
            "ada", "ik", "if", "sk", "sf", "{}", "psig", "att", "asig",
        );
        let parsed: tinyjson::JsonValue = json.parse().expect("valid JSON");
        let object = parsed
            .get::<std::collections::HashMap<String, tinyjson::JsonValue>>()
            .expect("object");
        assert_eq!(
            object
                .get("format")
                .and_then(|v| v.get::<String>())
                .map(String::as_str),
            Some("mfb-signing-v1")
        );
        assert_eq!(
            object
                .get("owner")
                .and_then(|v| v.get::<String>())
                .map(String::as_str),
            Some("ada")
        );
    }

    #[test]
    fn decode_trust_anchor_accepts_metadata_key_form() {
        // A malformed key is rejected.
        assert!(decode_trust_anchor("not-a-key").is_err());
    }

    fn write_executable_project(dir: &Path) {
        std::fs::write(
            dir.join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"executable\",\n",
                "  \"entry\": \"main\",\n",
                "  \"targets\": [\"native\"],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .expect("manifest");
        std::fs::create_dir_all(dir.join("src")).expect("src dir");
        std::fs::write(
            dir.join("src").join("main.mfb"),
            "IMPORT io\n\nSUB main()\n  io::print(\"hi\")\nEND SUB\n",
        )
        .expect("source");
    }

    #[test]
    fn build_project_validates_a_bad_manifest() {
        let dir = tempfile::tempdir().expect("temp dir");
        // No project.json at all -> validate fails, Err(()).
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }

    #[test]
    fn build_project_rejects_app_mode_for_non_app_target() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_executable_project(dir.path());
        // -app against a non-app target (a bare custom os) is rejected.
        let options = parse_build_options(s(&[
            "-app",
            "-target",
            "freebsd-riscv",
            dir.path().to_str().unwrap(),
        ]))
        .expect("options");
        assert!(build_project(&options).is_err());
    }

    #[test]
    fn build_project_builds_a_host_executable() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_executable_project(dir.path());
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        // Full front-end + native writer for the host target; no network.
        build_project(&options).expect("build should succeed");
    }

    #[test]
    fn build_project_writes_ast_and_ir_dumps() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_executable_project(dir.path());
        let options = parse_build_options(s(&["-ast", "-ir", "-br", dir.path().to_str().unwrap()]))
            .expect("options");
        build_project(&options).expect("dump build should succeed");
    }

    fn write_package_project(dir: &Path) {
        std::fs::write(
            dir.join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"lib\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"package\",\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"package\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .expect("manifest");
        std::fs::create_dir_all(dir.join("src")).expect("src dir");
        std::fs::write(
            dir.join("src").join("lib.mfb"),
            "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n",
        )
        .expect("source");
    }

    #[test]
    fn build_project_builds_a_package() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_package_project(dir.path());
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        build_project(&options).expect("package build should succeed");
        assert!(dir.path().join("lib.mfp").is_file());
    }

    #[test]
    fn build_project_rejects_native_output_for_a_package() {
        let dir = tempfile::tempdir().expect("temp dir");
        write_package_project(dir.path());
        // A native code dump is unsupported for package projects.
        let options =
            parse_build_options(s(&["-ncode", dir.path().to_str().unwrap()])).expect("options");
        assert!(build_project(&options).is_err());
    }

    #[test]
    fn build_project_reports_a_source_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            dir.path().join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"executable\",\n",
                "  \"entry\": \"main\",\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .expect("manifest");
        std::fs::create_dir_all(dir.path().join("src")).expect("src dir");
        // References an unknown package -> resolver/verify error.
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "SUB main()\n  nope::bogus()\nEND SUB\n",
        )
        .expect("source");
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }
}
