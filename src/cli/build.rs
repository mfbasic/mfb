use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tinyjson::JsonValue;

use crate::ast;
use crate::binary_repr;
use crate::ir;
use crate::json_string;
use crate::manifest::entry::validate_entry_point;
use crate::manifest::libraries::Libc;
use crate::manifest::package::{
    external_package_function_types, external_package_function_types_from_files,
    installed_package_files, package_metadata,
};
use crate::manifest::project_kind;
use crate::manifest::validate_project_manifest;
use crate::manifest::{build_mode_is_app, icon_path};
use crate::monomorph;
use crate::resolver;
use crate::rules;
use crate::syntaxcheck;
use crate::target;
use crate::target::shared::code::link_locator;

/// How much human-facing progress `mfb build` prints (plan-36). Never reaches
/// codegen — only the CLI's own `println!`/`eprintln!` lines are gated on it, so
/// the emitted artifact bytes are identical across all three levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Verbosity {
    /// `-q`/`--quiet`: today's minimal output — only the `Wrote … to` artifact
    /// line(s) and any diagnostics.
    Quiet,
    /// Default: the `Building …` summary line plus the artifact line.
    #[default]
    Normal,
    /// `-v`/`--verbose`: additionally a `phase <name> <N>ms` line per front-end
    /// stage. Doubles as a lightweight build profiler.
    Verbose,
}

/// The single place that knows the verbosity level. All human progress lines go
/// through here; the `Wrote … to` artifact line is printed directly by the
/// pipeline (always, on stdout) and never touches the reporter.
///
/// Summary and phase lines go to **stderr** (progress is diagnostics); the
/// artifact line stays on **stdout** (the machine-consumable channel that
/// integration tests `strip_prefix`).
pub(crate) struct Reporter {
    level: Verbosity,
}

impl Reporter {
    pub(crate) fn new(level: Verbosity) -> Self {
        Self { level }
    }

    /// The `Building …` context line — printed at Normal and Verbose, suppressed
    /// at Quiet.
    fn summary(&self, line: &str) {
        if self.level != Verbosity::Quiet {
            eprintln!("{line}");
        }
    }

    /// One `phase <name> <N>ms` profiler line — printed only at Verbose. The
    /// caller always computes the elapsed time (so `-v` and the default take an
    /// identical path into codegen); only the print is level-gated.
    fn phase(&self, name: &str, dt: Duration) {
        if self.level == Verbosity::Verbose {
            eprintln!("phase {name} {}ms", dt.as_millis());
        }
    }
}

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
    /// `--app-debug` (plan-51-C §4.7): app mode, but keep the intermediate
    /// `build/<name>.AppDir` beside the sealed `build/<name>.AppImage` so the
    /// payload the seal consumed can be inspected. Implies `app_mode`.
    ///
    /// Linux-only in effect but not in acceptance: on macOS `finalize_app_bundle`
    /// returns `None` and the flag does nothing, because there is no intermediate
    /// to keep. Erroring on `--app-debug -target macos-aarch64` would mean a flag
    /// that changes a build's *validity* by target, which is worse than one that
    /// changes nothing.
    pub(crate) app_debug: bool,
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
    /// How much human progress to print (plan-36). `-q`/`--quiet` restores the
    /// minimal artifact-line-only output; `-v`/`--verbose` adds per-phase
    /// timings. Never reaches codegen.
    pub(crate) verbosity: Verbosity,
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
    /// Target-neutral MIR dump (`--mir`, plan-00-A §12a): the neutral counterpart
    /// to `--ncode`.
    Mir,
}

impl BuildOutput {
    /// `--x` is the documented spelling; the single-dash `-x` form predates
    /// plan-42 and stays a working — but undocumented — alias.
    fn from_flag(flag: &str) -> Option<BuildOutput> {
        match flag {
            "--ast" | "-ast" => Some(BuildOutput::Ast),
            "--ir" | "-ir" => Some(BuildOutput::Ir),
            "--br" | "-br" => Some(BuildOutput::BinaryRepr),
            "--nir" | "-nir" => Some(BuildOutput::NativeIr),
            "--nplan" | "-nplan" => Some(BuildOutput::NativePlan),
            "--nobj" | "-nobj" => Some(BuildOutput::NativeObjectPlan),
            "--ncode" | "-ncode" => Some(BuildOutput::NativeCodePlan),
            "--mir" | "-mir" => Some(BuildOutput::Mir),
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
    let mut app_debug = false;
    let mut allow_unsigned = false;
    let mut regalloc = target::shared::code::regalloc::active_kind();
    let mut verbosity: Option<Verbosity> = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if let Some(output) = BuildOutput::from_flag(&arg) {
            if outputs.contains(&output) {
                return Err(format!("mfb build got duplicate output flag `{arg}`"));
            }
            outputs.push(output);
        } else if arg == "--target" || arg == "-target" {
            let Some(value) = iter.next() else {
                return Err("mfb build -target requires os-arch".to_string());
            };
            target = Some(target::BuildTarget::parse(&value)?);
        } else if let Some(value) = arg
            .strip_prefix("--target=")
            .or_else(|| arg.strip_prefix("-target="))
        {
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
        } else if arg == "--app" || arg == "-app" {
            if app_mode {
                return Err("mfb build accepts at most one -app option".to_string());
            }
            app_mode = true;
        } else if arg == "--app-debug" {
            if app_debug {
                return Err("mfb build accepts at most one --app-debug option".to_string());
            }
            app_debug = true;
        } else if arg == "--unsigned" {
            allow_unsigned = true;
        } else if arg == "--regalloc" || arg == "-regalloc" {
            let Some(value) = iter.next() else {
                return Err("mfb build -regalloc requires a strategy name".to_string());
            };
            regalloc = target::shared::code::regalloc::parse_kind(&value)?;
        } else if let Some(value) = arg
            .strip_prefix("--regalloc=")
            .or_else(|| arg.strip_prefix("-regalloc="))
        {
            regalloc = target::shared::code::regalloc::parse_kind(value)?;
        } else if arg == "-q" || arg == "--quiet" {
            if verbosity.replace(Verbosity::Quiet) == Some(Verbosity::Verbose) {
                return Err("mfb build accepts at most one of -q / -v".to_string());
            }
        } else if arg == "-v" || arg == "--verbose" {
            if verbosity.replace(Verbosity::Verbose) == Some(Verbosity::Quiet) {
                return Err("mfb build accepts at most one of -q / -v".to_string());
            }
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
        // plan-51-C §4.7: `--app-debug` implies `--app`. `--app --app-debug` is
        // the same thing said twice and is accepted; requiring both would be a
        // papercut with no upside.
        app_mode: app_mode || app_debug,
        app_debug,
        regalloc,
        allow_unsigned,
        mode: crate::testing::CompileMode::Build,
        verbosity: verbosity.unwrap_or_default(),
    })
}

pub(crate) fn build_project(options: &BuildOptions) -> Result<(), ()> {
    // Record the register-allocation strategy for the native backend to read
    // during lowering (plan-03 §4.2).
    target::shared::code::regalloc::set_strategy(options.regalloc);
    let reporter = Reporter::new(options.verbosity);
    let target = options.target.clone();
    let project_path = options.location.join("project.json");
    let manifest = validate_project_manifest(&project_path)?;
    let project_kind = project_kind(&manifest);

    // audit-1 PKG-01: verify every declared dependency's signature against a
    // project-pinned trust anchor before it is decoded, merged, or lowered, and
    // print a per-package verification report. A tampered signed dependency (or a
    // disallowed unsigned one) hard-fails the build with a non-zero exit.
    verify_and_report_packages(&options.location, &manifest, options.allow_unsigned)?;

    // App mode is requested by either the `-app` CLI flag or `"mode": "app"` in
    // the manifest (plan-22-A §4.2); `-app` is additive, never subtractive, so the
    // two compose without double-erroring.
    let app_mode = options.app_mode || build_mode_is_app(&manifest);

    // `mfb build -app` (plan-04-macos-app.md §5.1, plan-05-linux-app.md §5.1) is an
    // executable-only build flag supported on app-capable native targets (macOS via
    // AppKit, Linux via GTK4). Reject incompatible combinations up front, before any
    // lowering. The `"mode": "app"` manifest field is gated identically.
    if app_mode {
        if project_kind != "executable" {
            eprintln!("error: app mode requires an executable project");
            return Err(());
        }
        if !target::target_supports_app_mode(&target) {
            eprintln!(
                "error: app mode requires a macOS or Linux target (got {})",
                target.name()
            );
            return Err(());
        }
    }
    // The target OS selects the app toolkit and therefore the build mode. The CLI
    // has already verified the target supports app mode at this point.
    let build_mode = if app_mode {
        match target.os.as_str() {
            "linux" => target::NativeBuildMode::LinuxApp,
            _ => target::NativeBuildMode::MacApp,
        }
    } else {
        target::NativeBuildMode::Console
    };

    // The `icon` field (plan-22-A §4.3) is a project-relative source image
    // consumed by the macOS backend (plan-22-B renders it into `AppIcon.icns`).
    // Resolve and existence-check it only when app mode is active; a typo path
    // fails fast here without pulling in an image decoder. Deep validation
    // (decodable, exactly 1024×1024) happens in the backend.
    let app_icon: Option<PathBuf> = if app_mode {
        match icon_path(&manifest) {
            Some(rel) => {
                let resolved = options.location.join(rel);
                if !resolved.is_file() {
                    let contents = std::fs::read_to_string(&project_path).unwrap_or_default();
                    let (line, column) = crate::manifest::field_position(&contents, "icon");
                    rules::show_diagnostic(
                        "PROJECT_JSON_ICON_MISSING",
                        &format!("icon `{rel}` does not resolve to a readable file."),
                        &project_path,
                        line,
                        column,
                        column + "\"icon\"".len(),
                    );
                    return Err(());
                }
                Some(resolved)
            }
            None => None,
        }
    } else {
        None
    };

    let project_name = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .expect("validated project name");
    // plan-36: one concise, deterministic context line before the pipeline runs.
    // Suppressed by `-q`; safe if a golden ever captures it (no timings, no
    // color). Everything from here to the artifact line is instrumented for `-v`.
    reporter.summary(&format!(
        "Building {project_name} ({project_kind}) for {}",
        target.name()
    ));
    let parse_start = std::time::Instant::now();
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
    reporter.phase("parse", parse_start.elapsed());
    let resolve_start = std::time::Instant::now();
    resolver::resolve_project(&options.location, &manifest, &ast)?;
    let concrete_ast = monomorph::monomorphize_project(&options.location, &ast)?;
    // Skip DOC validation on the post-monomorph pass: monomorphization renames
    // overloaded/generic declarations, so their doc headers would falsely appear
    // unresolved. The original-AST pass above already validated them.
    resolver::resolve_project_with(&options.location, &manifest, &concrete_ast, false)?;
    reporter.phase("resolve", resolve_start.elapsed());
    let verify_start = std::time::Instant::now();
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
    reporter.phase("verify", verify_start.elapsed());
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
            let mut ir = ir::lower_project_with_external_functions(
                &concrete_ast,
                entry.clone(),
                &external_functions,
                &external_params,
            );
            // plan-46-B §4.3: an executable that declares its *own* `LINK` block
            // needs its own locators too — an imported binding's come from that
            // binding's `.mfp` section 10 instead. Runs the same missing-entry /
            // vendor-hash / coverage checks as a package build.
            if !assemble_native_libraries_for_ir(&mut ir, &manifest, &options.location) {
                return Err(());
            }
            // A host `mfb test` links the driver into a unique temporary
            // directory (removed after the run) so nothing is ever left in the
            // project directory. A cross `-target` test build has no host binary
            // to run, so it writes to the project directory like a normal build
            // and reports the artifact.
            let test_output_dir = if options.mode.is_test() && target.is_host() {
                Some(make_temp_output_dir()?)
            } else {
                None
            };
            let output_dir = test_output_dir.as_deref().unwrap_or(&options.location);
            // plan-55-A §4.2: clear `build/` at the start of every real build so a
            // file a previous build left there — a stale resource whose source was
            // removed, a stale vendored library, or a prior-mode output (a console
            // binary before an `--app` build) — never survives. Skipped only on the
            // `mfb test` host path, which links into a private temp dir
            // (`test_output_dir`) and must not touch the project's `build/`; a
            // cross-`-target` test build has `test_output_dir == None` and clears
            // like a normal build. Runs once per invocation, so the two Linux libc
            // flavors written in one build survive each other.
            if test_output_dir.is_none() {
                let build_dir = output_dir.join(crate::os::BUILD_DIR);
                if let Err(err) = std::fs::remove_dir_all(&build_dir) {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        eprintln!("error: failed to clear '{}': {err}", build_dir.display());
                        return Err(());
                    }
                }
            }
            // plan-46-C §4.4: hash-verify every `vendor` library this build resolves
            // to, against the sha256 the declaring binding recorded. Runs before
            // codegen so a wrong-version or missing blob fails the build rather
            // than producing a binary that dies at `dlopen`.
            let vendored = match resolved_vendor_libraries(&ir, &packages, &target, build_mode) {
                Ok(vendored) => vendored,
                Err(err) => {
                    eprintln!("error: {err}");
                    return Err(());
                }
            };
            if !verify_vendor_libraries(&vendored, &options.location, &ir.name) {
                return Err(());
            }
            let codegen_start = std::time::Instant::now();
            let executable_paths = target::write_executable(
                output_dir,
                &ir,
                &target,
                &packages,
                signing
                    .as_ref()
                    .map(|signing| signing.executable_metadata.as_slice()),
                build_mode,
                app_icon.as_deref(),
                // bug-248: the macOS `.app` bundle publishes the manifest `version`
                // as CFBundleShortVersionString/CFBundleVersion; App Store upload
                // validation rejects a bundle missing either key.
                crate::manifest::project_version(&manifest),
                // plan-46-D §4.2/§4.3: emit an RPATH only when this build actually
                // resolved a `vendor` locator; the backend picks the string for its
                // output shape.
                !vendored.is_empty(),
                // plan-15 D3: bake the manifest `"config".stdinLogCap` (or the default).
                crate::manifest::stdin_log_cap(&manifest),
            )
            .map_err(|err| {
                eprintln!("error: {err}");
            })?;
            // plan-46-D §4.5: copy the resolved vendor libraries into the directory
            // the executable's RPATH points at, so `dlopen` of the bare filename
            // resolves from any working directory and survives moving `build/`.
            // plan-56-B §4.3: `copy_vendor_libraries` copies EVERY library into
            // EVERY directory it is given, so a Linux app build — which now
            // resolves both libc worlds — must be routed per flavor. Handing it
            // both AppDirs at once would put the glibc blob inside the musl
            // image and vice versa: harmless at runtime (each binary `dlopen`s
            // its own filename) but it doubles the payload and ships a library
            // that can never load there.
            let vendor_copies: Vec<(Vec<link_locator::ResolvedLibrary>, Vec<PathBuf>)> =
                if build_mode == target::NativeBuildMode::LinuxApp {
                    crate::os::linux::flavor::LinuxFlavor::ALL
                        .iter()
                        .map(|flavor| {
                            let libc = flavor.libc();
                            let for_flavor = vendored
                                .iter()
                                .filter(|library| {
                                    // `libc: None` means the locator applies to
                                    // every libc world, so it belongs in both.
                                    library.locator.libc.is_none_or(|l| l == libc)
                                })
                                .cloned()
                                .collect();
                            let dir = output_dir
                                .join(crate::os::BUILD_DIR)
                                .join(crate::os::linux::appdir::appdir_name(
                                    &ir.name,
                                    flavor.suffix(),
                                ))
                                .join("usr")
                                .join("lib");
                            (for_flavor, vec![dir])
                        })
                        .collect()
                } else {
                    vec![(
                        vendored.clone(),
                        vendor_output_dirs(output_dir, &ir.name, build_mode),
                    )]
                };
            for (libraries, dirs) in &vendor_copies {
                if let Err(err) =
                    copy_vendor_libraries(libraries, &options.location, &ir.name, dirs)
                {
                    eprintln!("error: {err}");
                    return Err(());
                }
            }
            // plan-55-A §4.3: copy manifest-declared `resources` into the build
            // output tree (beside the executable in console mode, into the bundle's
            // resource directory in `--app` mode), where `os::resourcePath`
            // (plan-55-B) resolves them at runtime.
            for resource_dir in resource_output_dirs(output_dir, &ir.name, build_mode) {
                if let Err(err) = copy_resources(
                    &options.location,
                    &crate::manifest::resource_entries(&manifest),
                    &resource_dir,
                ) {
                    eprintln!("error: {err}");
                    return Err(());
                }
            }
            // plan-51-C §3.2: seal the Linux AppDir into `build/<name>.AppImage`.
            // Must run *after* vendoring and the resource copy — an AppImage is a
            // sealed file, and everything that belongs inside it has to be there
            // before it closes. macOS returns `None` (its `.app` is a directory
            // and is already complete), as does every console build.
            let executable_paths = match target::finalize_app_bundle(
                output_dir,
                &ir.name,
                &target,
                build_mode,
                options.app_debug,
            ) {
                Ok(sealed) if !sealed.is_empty() => sealed,
                Ok(_) => executable_paths,
                Err(err) => {
                    eprintln!("error: {err}");
                    return Err(());
                }
            };
            reporter.phase("codegen+link", codegen_start.elapsed());
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
            // plan-46-B §4.3: assemble the native library table from the manifest's
            // `libraries` section and the IR's distinct `LINK` names. Aborts the
            // build on a `LINK` with no entry, or a `vendor` file that cannot be
            // hashed; warns per uncovered target and per unused entry.
            if !assemble_native_libraries(&mut metadata, &manifest, &ir, &options.location) {
                return Err(());
            }
            if let Some(signing) = &signing {
                apply_signing_metadata(&mut metadata, signing);
            }
            let codegen_start = std::time::Instant::now();
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
            reporter.phase("codegen+link", codegen_start.elapsed());
            println!("Wrote package to {}", package_path.display());
        } else {
            // bug-300 E8 reported this arm as unreachable ("validate_project_manifest
            // restricts kind to exactly executable|package") and proposed replacing
            // it with `unreachable!()`. That is wrong, and doing so would have turned
            // a live path into a panic: an unrecognized `kind` is a WARNING
            // (`PROJECT_JSON_UNKNOWN_KIND`, "continuing validation"), not an error, so
            // a project with e.g. `"kind": "program"` reaches here, builds nothing,
            // and exits 0. Verified by building one.
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

/// Parse `mfb test [location] [--coverage] [--target …] [--regalloc …]`. The build
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
        } else if arg == "--target" || arg == "-target" {
            let Some(value) = iter.next() else {
                return Err("mfb test -target requires os-arch".to_string());
            };
            target = Some(target::BuildTarget::parse(&value)?);
        } else if let Some(value) = arg
            .strip_prefix("--target=")
            .or_else(|| arg.strip_prefix("-target="))
        {
            target = Some(target::BuildTarget::parse(value)?);
        } else if arg == "--regalloc" || arg == "-regalloc" {
            let Some(value) = iter.next() else {
                return Err("mfb test -regalloc requires a strategy name".to_string());
            };
            regalloc = target::shared::code::regalloc::parse_kind(&value)?;
        } else if let Some(value) = arg
            .strip_prefix("--regalloc=")
            .or_else(|| arg.strip_prefix("-regalloc="))
        {
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
        // `mfb test` never runs a test binary out of a sealed AppImage, so it
        // takes neither `--app` nor `--app-debug`; both land in the
        // `unknown test option` arm above (plan-51-C §4.7).
        app_debug: false,
        regalloc,
        allow_unsigned: false,
        mode: crate::testing::CompileMode::Test { coverage },
        // `mfb test`'s user-facing output is the pass/fail tree; the build
        // summary would be noise and (via `target.name()`) non-portable across
        // machines, churning `.testrun` goldens. Stay quiet (plan-36).
        verbosity: Verbosity::Quiet,
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
/// high-resolution timestamp; created *exclusively* (like `write_new_file`'s
/// `create_new`/`O_EXCL`) so a pre-existing or symlinked directory an attacker
/// planted at the predictable path cannot redirect where the executable is
/// written and run. Retries with a fresh suffix on `AlreadyExists`.
fn make_temp_output_dir() -> Result<PathBuf, ()> {
    let base = std::env::temp_dir();
    let pid = std::process::id();
    for _ in 0..64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let dir = base.join(format!("mfb-test-{pid}-{nanos}"));
        // `create_dir` (not `create_dir_all`) fails atomically on an existing
        // path, so we never adopt a directory we did not create ourselves.
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                eprintln!("error: failed to create temporary test directory: {err}");
                return Err(());
            }
        }
    }
    eprintln!("error: failed to create a unique temporary test directory");
    Err(())
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

/// Assemble the package's `NATIVE_LIBRARY_TABLE` into `metadata` and render every
/// finding (plan-46-B §4.3). Returns whether the build may continue — false when
/// any finding is `Error` severity (a `LINK` with no `libraries` entry, or an
/// unreadable `vendor` file).
///
/// A package with no `LINK` block produces an empty table, emits no section 10,
/// and leaves container flag bit 0 clear — its `.mfp` stays byte-identical.
fn assemble_native_library_table(
    manifest: &HashMap<String, JsonValue>,
    ir: &ir::IrProject,
    project_root: &Path,
) -> Option<binary_repr::NativeLibraryTable> {
    let linked = ir.link_library_names();
    let manifest_path = project_root.join("project.json");
    let (table, findings) =
        crate::manifest::libraries::build_native_library_table(manifest, &linked, project_root);

    let mut ok = true;
    for finding in &findings {
        rules::show_diagnostic(finding.rule, &finding.message, &manifest_path, 1, 1, 1);
        if rules::is_error(finding.rule) {
            ok = false;
        }
    }
    ok.then_some(table)
}

/// Every `(os, arch, libc)` this build will actually emit a binary for.
///
/// A Linux **console** `mfb build` emits both libc flavors from one invocation,
/// each its own codegen pass, so a locator that differs by libc must be resolved
/// (and its vendor file verified) once per flavor.
///
/// A Linux **app-mode** build emits a single glibc binary (plan-05-linux-app.md
/// §5.2), so it must be checked for glibc only: demanding a musl locator — and a
/// musl blob in `vendor/` — for a flavor the build never emits would fail a
/// correct project. macOS has no libc axis and yields one target either way.
///
/// This must stay in lockstep with what the backends actually emit; it is the
/// caller-side mirror of their per-flavor loop.
fn emitted_link_targets(
    target: &target::BuildTarget,
    _build_mode: target::NativeBuildMode,
) -> Vec<link_locator::LinkTarget> {
    if target.os == "linux" {
        // plan-56-B §4.1: every Linux build — console AND app — emits both libc
        // worlds, so vendor resolution must cover both. Leaving app mode on
        // glibc-only here would put the glibc blob inside the musl AppImage.
        let flavors: &[Libc] = &[Libc::Glibc, Libc::Musl];
        flavors
            .iter()
            .map(|libc| link_locator::LinkTarget {
                os: target.os.clone(),
                arch: target.arch.clone(),
                libc: Some(*libc),
            })
            .collect()
    } else {
        vec![link_locator::LinkTarget {
            os: target.os.clone(),
            arch: target.arch.clone(),
            libc: None,
        }]
    }
}

/// Resolve every `vendor` library this build emits, in a deterministic order.
///
/// Shared by the hash verify (plan-46-C §4.4) and the output copy (plan-46-D
/// §4.5) so both act on exactly the same resolved set. Deduplicated by the
/// emitted `dlopen_name`, since both Linux flavors commonly resolve to the same
/// `system` locator and may share a `vendor` one.
fn resolved_vendor_libraries(
    ir: &ir::IrProject,
    packages: &[PathBuf],
    target: &target::BuildTarget,
    build_mode: target::NativeBuildMode,
) -> Result<Vec<link_locator::ResolvedLibrary>, String> {
    let tables =
        link_locator::LibraryTables::collect(packages, &ir.name, ir.native_libraries.clone())?;
    // Derived from the tables, not `ir.link_library_names()`: `ir` here is the
    // project's own IR, not yet merged with its imported packages, so its
    // `link_functions` would miss every library an imported binding links — which
    // is the whole case this verify exists for.
    let linked = tables.logical_names();
    if linked.is_empty() {
        return Ok(Vec::new());
    }

    let mut vendored: Vec<link_locator::ResolvedLibrary> = Vec::new();
    for link_target in emitted_link_targets(target, build_mode) {
        let resolved = link_locator::LinkLibraries::resolve_all(&tables, &linked, &link_target)?;
        for library in resolved.vendored() {
            if !vendored
                .iter()
                .any(|existing| existing.dlopen_name == library.dlopen_name)
            {
                vendored.push(library.clone());
            }
        }
    }
    vendored.sort_by(|a, b| a.dlopen_name.cmp(&b.dlopen_name));
    Ok(vendored)
}

/// The on-disk source of a resolved vendor library (plan-48-B §4.3): the
/// consumer's own `libraries` locators read from `<project>/vendor/`, an imported
/// binding's from `<project>/packages/<declaring-unit>.vendor/` — where `pkg add`
/// placed the downloaded blob. `own_unit` is the project's own name (`ir.name`),
/// which `declaring_unit` equals exactly for a locator from the project's own
/// `libraries` section.
fn vendor_source_path(
    project_root: &Path,
    own_unit: &str,
    library: &link_locator::ResolvedLibrary,
) -> PathBuf {
    if library.declaring_unit == own_unit {
        crate::manifest::libraries::vendor_path(project_root, &library.locator.source)
    } else {
        crate::manifest::libraries::imported_vendor_path(
            project_root,
            &library.declaring_unit,
            &library.locator.source,
        )
    }
}

/// Hash-verify each resolved `vendor` library against the sha256 the declaring
/// binding recorded in its section-10 table (plan-46-C §4.4, plan-48-B §4.3).
///
/// The consumer's own `libraries` locators read the file the author placed at
/// `<project>/vendor/<source>` by hand; an imported binding's read the blob
/// `pkg add` downloaded to `<project>/packages/<declaring-unit>.vendor/<source>`.
/// Either way the `.mfp` carries the hash, never the blob.
fn verify_vendor_libraries(
    vendored: &[link_locator::ResolvedLibrary],
    project_root: &Path,
    own_unit: &str,
) -> bool {
    let mut ok = true;
    for library in vendored {
        let path = vendor_source_path(project_root, own_unit, library);
        let actual = match crate::manifest::libraries::sha256_file(&path) {
            Ok(hash) => hash,
            Err(reason) => {
                rules::show_general_diagnostic(
                    "NATIVE_LIBRARY_FILE_MISSING",
                    &format!(
                        "`{}` vendors native library \"{}\", but {} could not be read: {reason}. \
                         Place that file there — the package carries its hash, not its bytes.",
                        library.declaring_unit,
                        library.locator.source,
                        path.display()
                    ),
                );
                ok = false;
                continue;
            }
        };
        // A vendor locator always carries a hash (the encoder enforces
        // `hash` present iff vendor, and decode re-checks it).
        let Some(expected) = library.locator.hash else {
            rules::show_general_diagnostic(
                "NATIVE_LIBRARY_HASH_MISMATCH",
                &format!(
                    "`{}` vendors native library \"{}\" but records no hash for it; the package \
                     is malformed and must be rebuilt.",
                    library.declaring_unit, library.locator.source
                ),
            );
            ok = false;
            continue;
        };
        if actual != expected {
            rules::show_general_diagnostic(
                "NATIVE_LIBRARY_HASH_MISMATCH",
                &format!(
                    "{} does not match the sha256 `{}` recorded for \"{}\" — this is the wrong \
                     version of the library.\n               expected {}\n               actual   {}",
                    path.display(),
                    library.declaring_unit,
                    library.locator.source,
                    hex(&expected),
                    hex(&actual),
                ),
            );
            ok = false;
        }
    }
    ok
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Where this build's output shape wants its vendored native libraries — the
/// directory (or directories) the emitted RPATH resolves to (plan-46-D §4.4).
///
/// Must stay in lockstep with the RPATH each backend emits: the loader looks
/// exactly here and nowhere else.
///
/// | build | rpath | vendor files |
/// | --- | --- | --- |
/// | linux console | `$ORIGIN/vendor` | `build/vendor/` |
/// | linux `--app` | `$ORIGIN/../lib` | `build/<name>.AppDir/usr/lib/` |
/// | macos console | `@loader_path/vendor` | `build/vendor/` |
/// | macos `--app` | `@executable_path/../Frameworks` | `build/<name>.app/Contents/Frameworks/` |
fn vendor_output_dirs(
    output_dir: &Path,
    project_name: &str,
    build_mode: target::NativeBuildMode,
) -> Vec<PathBuf> {
    let build_dir = output_dir.join(crate::os::BUILD_DIR);
    match build_mode {
        // The `.app` bundle puts its dylibs in the platform-standard
        // `Contents/Frameworks/`, where Apple specifies private shared libraries
        // live and where bundle-inspecting tools expect them. Created only when the
        // build vendors something — an empty `Frameworks/` in every bundle would be
        // noise.
        target::NativeBuildMode::MacApp => vec![build_dir
            .join(format!("{project_name}.app"))
            .join("Contents")
            .join(crate::os::MACOS_APP_FRAMEWORKS_DIR)],
        // The AppDir puts its libraries at `usr/lib/`, one directory up from the
        // executable at `usr/bin/<name>` — the layout every AppDir-consuming tool
        // expects, and what `ELF_APPDIR_VENDOR_RPATH` (`$ORIGIN/../lib`) resolves
        // to. Created only when the build vendors something, so a non-vendoring
        // AppDir carries no empty `usr/lib/` (plan-51-A §4.4).
        // One `usr/lib` per flavor's AppDir (plan-56-B §4.3). Both are returned
        // so the caller can route each flavor's blob into its own image; see
        // `vendor_output_dirs_for_flavor` for the per-flavor selection.
        target::NativeBuildMode::LinuxApp => crate::os::linux::flavor::LinuxFlavor::ALL
            .iter()
            .map(|flavor| {
                build_dir
                    .join(crate::os::linux::appdir::appdir_name(
                        project_name,
                        flavor.suffix(),
                    ))
                    .join("usr")
                    .join("lib")
            })
            .collect(),
        // Both Linux libc flavors live in the one `build/` directory and share the
        // one `vendor/`. That is sound only because vendor `source` filenames are
        // unique project-wide (plan-46-A §4.3), so a glibc blob and a musl blob
        // never collide.
        target::NativeBuildMode::Console => vec![build_dir.join(crate::os::VENDOR_DIR)],
    }
}

/// The directory declared resources are copied into for a given build shape
/// (plan-55-A §4.3). Each entry's `<dst>` is joined *under* this directory.
///
/// Kept in lockstep with plan-55-B's `os::resourcePath` base offset
/// (`resource_base_offset`): the runtime locator resolves to exactly this
/// directory, so a change here without the matching change there makes resources
/// unfindable at runtime.
///
/// | build            | resource dir                                   |
/// | ---              | ---                                            |
/// | console          | `build/`                                       |
/// | macos `--app`    | `build/<name>.app/Contents/Resources/`         |
/// | linux `--app`    | `build/<name>.AppDir/usr/share/<name>/`        |
///
/// The `LinuxApp` arm depends on plan-51-A's AppDir existing; until then a Linux
/// `--app` build never reaches this path (Linux app mode is unimplemented
/// pre-51). It is written now so 51-A needs no change here.
fn resource_output_dirs(
    output_dir: &Path,
    project_name: &str,
    build_mode: target::NativeBuildMode,
) -> Vec<PathBuf> {
    let build_dir = output_dir.join(crate::os::BUILD_DIR);
    match build_mode {
        target::NativeBuildMode::MacApp => vec![build_dir
            .join(format!("{project_name}.app"))
            .join("Contents")
            .join(crate::os::MACOS_APP_RESOURCES_DIR)],
        // Resources are flavor-independent, so BOTH AppDirs get the same copy
        // (plan-56-B §4.3). Broadcasting is correct here and wrong for vendored
        // libraries, which is why the two do not share a helper.
        target::NativeBuildMode::LinuxApp => crate::os::linux::flavor::LinuxFlavor::ALL
            .iter()
            .map(|flavor| {
                build_dir
                    .join(crate::os::linux::appdir::appdir_name(
                        project_name,
                        flavor.suffix(),
                    ))
                    .join("usr")
                    .join("share")
                    .join(project_name)
            })
            .collect(),
        target::NativeBuildMode::Console => vec![build_dir],
    }
}

/// Copy each resolved `vendor` library into the output directory the executable's
/// RPATH points at (plan-46-D §4.5), preserving the executable bit.
///
/// Runs **after** the hash verify on the same files, so the bytes landing in the
/// output are the bytes that were verified — and they are not re-hashed.
///
/// Only *resolved* locators are copied, never the whole `vendor/` directory: a
/// project vendoring blobs for six targets ships one per build.
///
/// The destination filename is `dlopen_name` — the same helper plan-46-C emits the
/// `dlopen` cstring from, never a second copy of the format string. If the file
/// written here and the string emitted there ever disagreed, the `dlopen` would
/// miss at runtime and nothing at build time would notice.
fn copy_vendor_libraries(
    vendored: &[link_locator::ResolvedLibrary],
    project_root: &Path,
    own_unit: &str,
    output_dirs: &[PathBuf],
) -> Result<(), String> {
    if vendored.is_empty() {
        return Ok(());
    }

    // §4.5.2 residual check: the `<declaring-unit>-` prefix makes a collision
    // essentially unrepresentable, but not provably so — a project named `sqlite3`
    // that also imports a package named `sqlite3` could reach the same output name.
    // Absurd, but the cost is a silent wrong-library load, so assert it. Identical
    // hashes are fine: the same bytes, legitimately shared, and the copy is
    // idempotent. This check should never fire; it is the guard rail that lets the
    // prefix be trusted, not the mechanism.
    for (index, library) in vendored.iter().enumerate() {
        for other in &vendored[index + 1..] {
            if library.dlopen_name == other.dlopen_name
                && library.locator.hash != other.locator.hash
            {
                rules::show_general_diagnostic(
                    "NATIVE_LIBRARY_VENDOR_COLLISION",
                    &format!(
                        "`{}` and `{}` both vendor a native library that copies to \"{}\", with \
                         different contents. One would silently overwrite the other and both \
                         bindings would load whichever won. Rename one of the vendored files.",
                        library.declaring_unit, other.declaring_unit, library.dlopen_name
                    ),
                );
                return Err(format!(
                    "vendored native library name collision on \"{}\"",
                    library.dlopen_name
                ));
            }
        }
    }

    for output_dir in output_dirs {
        std::fs::create_dir_all(output_dir)
            .map_err(|err| format!("failed to create '{}': {err}", output_dir.display()))?;
        for library in vendored {
            let from = vendor_source_path(project_root, own_unit, library);
            let to = output_dir.join(&library.dlopen_name);
            std::fs::copy(&from, &to).map_err(|err| {
                format!(
                    "failed to copy vendored library '{}' to '{}': {err}",
                    from.display(),
                    to.display()
                )
            })?;
            // Preserve the executable bit: a shared object is loadable without it
            // on Linux, but macOS `dlopen` of a non-executable dylib can be
            // refused, and `fs::copy` already carries the source mode on Unix.
            // Set it explicitly so a source blob checked out without +x still works.
            let mut permissions = std::fs::metadata(&to)
                .map_err(|err| format!("failed to read '{}': {err}", to.display()))?
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&to, permissions)
                .map_err(|err| format!("failed to mark '{}' executable: {err}", to.display()))?;
        }
    }
    Ok(())
}

/// The fixed (glob-free) leading directory of a resource `src` glob (plan-55-A
/// §4.3): the longest run of leading path components that contain no glob
/// metacharacter (`*`, `?`, `[`, `]`), excluding the final component (which is the
/// file or pattern to match). The result is the directory `copy_resources` walks
/// and the prefix it strips to form each match's destination-relative path.
///
/// | `src` | fixed prefix |
/// | --- | --- |
/// | `data/**/*.ogg` | `data` |
/// | `data/*.ogg` | `data` |
/// | `assets/logo.png` | `assets` |
/// | `*.ogg` | `` (project root) |
fn resource_src_fixed_prefix(src: &str) -> String {
    let normalized = src.replace('\\', "/");
    let components: Vec<&str> = normalized.split('/').collect();
    let has_meta = |component: &str| component.contains(['*', '?', '[', ']']);
    let mut prefix: Vec<&str> = Vec::new();
    for component in &components {
        if has_meta(component) {
            break;
        }
        prefix.push(component);
    }
    // Every component was literal: the last is the file itself, so the walked
    // directory is everything before it.
    if prefix.len() == components.len() {
        prefix.pop();
    }
    prefix.join("/")
}

/// Recursively collect every regular file under `dir` into `out`.
fn collect_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files_recursive(&entry.path(), out)?;
        } else if file_type.is_file() {
            out.push(entry.path());
        }
    }
    Ok(())
}

/// Copy every file matching each resource entry's `src` glob into
/// `<resource_dir>/<dst>/…` (plan-55-A §4.3), preserving structure below the
/// glob's fixed prefix. Runs after `write_executable`, next to the vendor copy.
///
/// For each entry the fixed-prefix directory (`resource_src_fixed_prefix`) is
/// walked; every regular file whose project-relative path matches the `src` glob
/// is copied to `resource_dir/<dst>/<path-with-prefix-stripped>`. An empty match
/// set — or a `src` whose fixed-prefix directory does not exist — is a silent
/// no-op, not an error (a glob may legitimately match nothing on a checkout).
fn copy_resources(
    project_root: &Path,
    entries: &[crate::manifest::ResourceEntry],
    resource_dir: &Path,
) -> Result<(), String> {
    for entry in entries {
        let prefix = resource_src_fixed_prefix(&entry.src);
        let walk_root = if prefix.is_empty() {
            project_root.to_path_buf()
        } else {
            project_root.join(&prefix)
        };
        // A glob whose fixed-prefix directory is absent copies nothing (§4.3).
        if !walk_root.is_dir() {
            continue;
        }
        // bug-298 defense in depth: manifest validation rejects an escaping `src`,
        // but that check is textual and this is the step that actually reads
        // files. Canonicalize and require containment, so a symlink inside the
        // project that points outside it cannot widen the read set either.
        let canonical_root = project_root.canonicalize().map_err(|err| {
            format!(
                "failed to resolve project root '{}': {err}",
                project_root.display()
            )
        })?;
        let canonical_walk = walk_root.canonicalize().map_err(|err| {
            format!(
                "failed to resolve resource source '{}': {err}",
                walk_root.display()
            )
        })?;
        if !canonical_walk.starts_with(&canonical_root) {
            return Err(format!(
                "resource source '{}' resolves to '{}', which is outside the project root '{}'",
                entry.src,
                canonical_walk.display(),
                canonical_root.display()
            ));
        }
        let mut files = Vec::new();
        collect_files_recursive(&walk_root, &mut files).map_err(|err| {
            format!(
                "failed to scan resources under '{}': {err}",
                walk_root.display()
            )
        })?;
        for file in files {
            let rel = file
                .strip_prefix(project_root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            if !crate::ast::manifest::glob_matches(&entry.src, &rel) {
                continue;
            }
            // Destination-relative path: the match minus the fixed prefix (§4.3).
            let dest_relative = if prefix.is_empty() {
                rel.as_str()
            } else {
                rel.strip_prefix(&prefix)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .unwrap_or(rel.as_str())
            };
            let to = resource_dir.join(&entry.dst).join(dest_relative);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create '{}': {err}", parent.display()))?;
            }
            std::fs::copy(&file, &to).map_err(|err| {
                format!(
                    "failed to copy resource '{}' to '{}': {err}",
                    file.display(),
                    to.display()
                )
            })?;
        }
    }
    Ok(())
}

/// Package build: the table becomes `.mfp` section 10 and sets container flag
/// bit 0, so it rides on the metadata the package writer encodes.
fn assemble_native_libraries(
    metadata: &mut binary_repr::BinaryReprMetadata,
    manifest: &HashMap<String, JsonValue>,
    ir: &ir::IrProject,
    project_root: &Path,
) -> bool {
    match assemble_native_library_table(manifest, ir, project_root) {
        Some(table) => {
            metadata.native_libraries = table;
            true
        }
        None => false,
    }
}

/// Executable build: nothing is encoded, but a project declaring its own `LINK`
/// block must still resolve against its own locators at codegen, so the table
/// rides on the IR into the NIR module (plan-46-C).
fn assemble_native_libraries_for_ir(
    ir: &mut ir::IrProject,
    manifest: &HashMap<String, JsonValue>,
    project_root: &Path,
) -> bool {
    match assemble_native_library_table(manifest, ir, project_root) {
        Some(table) => {
            ir.native_libraries = table;
            true
        }
        None => false,
    }
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
        assert_eq!(BuildOutput::from_flag("--ast"), Some(BuildOutput::Ast));
        assert_eq!(BuildOutput::from_flag("--ir"), Some(BuildOutput::Ir));
        assert_eq!(
            BuildOutput::from_flag("--br"),
            Some(BuildOutput::BinaryRepr)
        );
        assert_eq!(BuildOutput::from_flag("--nir"), Some(BuildOutput::NativeIr));
        assert_eq!(
            BuildOutput::from_flag("--nplan"),
            Some(BuildOutput::NativePlan)
        );
        assert_eq!(
            BuildOutput::from_flag("--nobj"),
            Some(BuildOutput::NativeObjectPlan)
        );
        assert_eq!(
            BuildOutput::from_flag("--ncode"),
            Some(BuildOutput::NativeCodePlan)
        );
        assert_eq!(BuildOutput::from_flag("--mir"), Some(BuildOutput::Mir));
        assert_eq!(BuildOutput::from_flag("--nope"), None);
        assert_eq!(BuildOutput::from_flag("-nope"), None);
    }

    /// plan-42: every emit flag's single-dash spelling stays a working alias of
    /// the documented `--` form.
    #[test]
    fn build_output_from_flag_single_dash_aliases_double_dash() {
        for name in ["ast", "ir", "br", "nir", "nplan", "nobj", "ncode", "mir"] {
            let long = BuildOutput::from_flag(&format!("--{name}"));
            let short = BuildOutput::from_flag(&format!("-{name}"));
            assert!(long.is_some(), "--{name} must parse");
            assert_eq!(long, short, "--{name} and -{name} must map identically");
        }
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
            parse_build_options(s(&["--target", "linux-aarch64"])).expect("split target form");
        assert_eq!(split.target.name(), "linux-aarch64");
        let joined =
            parse_build_options(s(&["--target=linux-x86_64"])).expect("joined target form");
        assert_eq!(joined.target.name(), "linux-x86_64");
    }

    /// plan-42: `--target`/`--regalloc`/`--app` are the documented spellings; the
    /// single-dash forms (space and `=`) stay working aliases that parse to the
    /// same options.
    #[test]
    fn parse_build_options_single_dash_aliases_double_dash() {
        for (long, short) in [
            (
                s(&["--target", "linux-aarch64"]),
                s(&["-target", "linux-aarch64"]),
            ),
            (s(&["--target=linux-x86_64"]), s(&["-target=linux-x86_64"])),
        ] {
            let long = parse_build_options(long).expect("--target form");
            let short = parse_build_options(short).expect("-target form");
            assert_eq!(long.target.name(), short.target.name());
        }

        for (long, short) in [
            (s(&["--regalloc", "bump"]), s(&["-regalloc", "bump"])),
            (s(&["--regalloc=bump"]), s(&["-regalloc=bump"])),
        ] {
            let long = parse_build_options(long).expect("--regalloc form");
            let short = parse_build_options(short).expect("-regalloc form");
            assert_eq!(long.regalloc, short.regalloc);
            assert_eq!(
                long.regalloc,
                target::shared::code::regalloc::parse_kind("bump").expect("bump")
            );
        }

        assert!(
            parse_build_options(s(&["--app"]))
                .expect("--app form")
                .app_mode
        );
        assert!(
            parse_build_options(s(&["-app"]))
                .expect("-app form")
                .app_mode
        );
        // The duplicate guard spans both spellings — they are one flag.
        assert!(parse_build_options(s(&["--app", "-app"])).is_err());
    }

    /// plan-51-C §4.7: `--app-debug` is app mode with the intermediate AppDir
    /// kept, so it implies `--app` rather than requiring it alongside.
    #[test]
    fn parse_build_options_app_debug_implies_app_mode() {
        let options = parse_build_options(s(&["--app-debug"])).expect("--app-debug");
        assert!(options.app_debug);
        assert!(options.app_mode, "--app-debug implies --app");

        // Saying it twice over is the same thing said twice, and is accepted.
        let both = parse_build_options(s(&["--app", "--app-debug"])).expect("--app --app-debug");
        assert!(both.app_mode && both.app_debug);

        // A plain `--app` keeps the AppDir-deleting default.
        let plain = parse_build_options(s(&["--app"])).expect("--app");
        assert!(plain.app_mode && !plain.app_debug);

        // Duplicates are rejected, matching `--app`.
        assert!(parse_build_options(s(&["--app-debug", "--app-debug"])).is_err());

        // There is no single-dash alias: `--app-debug` postdates plan-42.
        assert!(parse_build_options(s(&["-app-debug"])).is_err());
    }

    /// `mfb test` never runs a test binary out of a sealed AppImage, so it takes
    /// `--app-debug` no more than it takes `--app`.
    #[test]
    fn parse_test_options_rejects_app_debug() {
        let err = match parse_test_options(s(&["--app-debug"])) {
            Err(err) => err,
            Ok(_) => panic!("mfb test must reject --app-debug"),
        };
        assert!(err.contains("unknown test option"), "{err}");
    }

    /// plan-42: `mfb test` accepts both spellings of its two behavioral flags —
    /// and still refuses `--app`/`-app`, which it never took.
    #[test]
    fn parse_test_options_single_dash_aliases_double_dash() {
        for (long, short) in [
            (
                s(&["--target", "linux-aarch64"]),
                s(&["-target", "linux-aarch64"]),
            ),
            (s(&["--target=linux-x86_64"]), s(&["-target=linux-x86_64"])),
        ] {
            let long = parse_test_options(long).expect("--target form");
            let short = parse_test_options(short).expect("-target form");
            assert_eq!(long.target.name(), short.target.name());
        }

        for (long, short) in [
            (s(&["--regalloc", "bump"]), s(&["-regalloc", "bump"])),
            (s(&["--regalloc=bump"]), s(&["-regalloc=bump"])),
        ] {
            let long = parse_test_options(long).expect("--regalloc form");
            let short = parse_test_options(short).expect("-regalloc form");
            assert_eq!(long.regalloc, short.regalloc);
        }

        assert!(parse_test_options(s(&["--app"])).is_err());
        assert!(parse_test_options(s(&["-app"])).is_err());
    }

    #[test]
    fn parse_build_options_target_requires_value() {
        assert!(build_err(&["--target"]).contains("-target requires os-arch"));
    }

    #[test]
    fn parse_build_options_target_rejects_malformed() {
        assert!(parse_build_options(s(&["--target", "nodash"])).is_err());
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
    fn parse_build_options_verbosity_defaults_to_normal() {
        let options = parse_build_options(vec![]).expect("options");
        assert_eq!(options.verbosity, Verbosity::Normal);
        // The default is also what the derive produces.
        assert_eq!(Verbosity::default(), Verbosity::Normal);
    }

    #[test]
    fn parse_build_options_quiet_both_spellings() {
        for flag in ["-q", "--quiet"] {
            let options = parse_build_options(s(&[flag])).expect("quiet options");
            assert_eq!(options.verbosity, Verbosity::Quiet, "flag {flag}");
        }
    }

    #[test]
    fn parse_build_options_verbose_both_spellings() {
        for flag in ["-v", "--verbose"] {
            let options = parse_build_options(s(&[flag])).expect("verbose options");
            assert_eq!(options.verbosity, Verbosity::Verbose, "flag {flag}");
        }
    }

    #[test]
    fn parse_build_options_quiet_and_verbose_conflict() {
        for args in [
            &["-q", "-v"][..],
            &["-v", "-q"][..],
            &["--quiet", "--verbose"][..],
            &["--verbose", "--quiet"][..],
        ] {
            let err = build_err(args);
            assert!(
                err.contains("at most one of -q / -v"),
                "unexpected error for {args:?}: {err}"
            );
        }
        // Repeating the same flag is not a conflict.
        assert_eq!(
            parse_build_options(s(&["-q", "-q"]))
                .expect("repeat quiet")
                .verbosity,
            Verbosity::Quiet
        );
        assert_eq!(
            parse_build_options(s(&["-v", "-v"]))
                .expect("repeat verbose")
                .verbosity,
            Verbosity::Verbose
        );
    }

    #[test]
    fn parse_test_options_is_quiet() {
        // `mfb test` never prints the build summary (it would churn the
        // non-portable `.testrun` goldens); see plan-36.
        let options = parse_test_options(vec![]).expect("test options");
        assert_eq!(options.verbosity, Verbosity::Quiet);
    }

    #[test]
    fn parse_build_options_regalloc_both_forms_and_bad_value() {
        assert!(parse_build_options(s(&["--regalloc"])).is_err());
        assert!(parse_build_options(s(&["--regalloc", "not-a-strategy"])).is_err());
        assert!(parse_build_options(s(&["--regalloc=not-a-strategy"])).is_err());
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
        let path =
            Path::new("tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp");
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
            "tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp",
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
            "tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp",
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
    fn build_project_clears_stale_build_dir() {
        // plan-55-A §4.2: a real build removes `build/` at the start, so a file a
        // previous build left there is gone afterward while the freshly written
        // executable exists.
        let dir = tempfile::tempdir().expect("temp dir");
        write_executable_project(dir.path());
        let build_dir = dir.path().join(crate::os::BUILD_DIR);
        std::fs::create_dir_all(&build_dir).expect("build dir");
        std::fs::write(build_dir.join("stale.txt"), b"stale").expect("stale");
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        build_project(&options).expect("build should succeed");
        assert!(
            !build_dir.join("stale.txt").exists(),
            "stale file must be cleared by the build"
        );
        assert!(build_dir.exists(), "build dir is recreated by the writer");
    }

    #[test]
    fn mfb_test_host_run_leaves_project_build_dir_untouched() {
        // plan-55-A §4.2: a `mfb test` host run links into a private temp dir and
        // must never clear the project's own `build/`. Seed one, run the tests, and
        // confirm the seeded file survives.
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
                "  \"targets\": [\"native\"],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .expect("manifest");
        std::fs::create_dir_all(dir.path().join("src")).expect("src dir");
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            concat!(
                "FUNC main AS Integer\n",
                "  RETURN 0\n",
                "END FUNC\n",
                "\n",
                "TESTING\n",
                "  TGROUP \"g\"\n",
                "    TCASE \"c\"\n",
                "      expectInteger(1, 1)\n",
                "    END TCASE\n",
                "  END TGROUP\n",
                "END TESTING\n"
            ),
        )
        .expect("source");
        let build_dir = dir.path().join(crate::os::BUILD_DIR);
        std::fs::create_dir_all(&build_dir).expect("build dir");
        std::fs::write(build_dir.join("keep.txt"), b"keep").expect("keep");
        let options =
            parse_test_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        build_project(&options).expect("mfb test should pass");
        assert!(
            build_dir.join("keep.txt").exists(),
            "mfb test host run must not clear the project build/"
        );
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

    // ---- vendored native library copy (plan-46-D §4.5) ----

    use crate::binary_repr::NativeLibraryLocator;
    use crate::manifest::libraries::{LibType, Libc};

    fn vendor_locator(source: &str) -> NativeLibraryLocator {
        NativeLibraryLocator {
            os: "linux".to_string(),
            arch: Some("x86_64".to_string()),
            libc: Some(Libc::Glibc),
            lib_type: LibType::Vendor,
            source: source.to_string(),
            hash: Some([1u8; 32]),
        }
    }

    fn resolved(unit: &str, source: &str) -> link_locator::ResolvedLibrary {
        let locator = vendor_locator(source);
        link_locator::ResolvedLibrary {
            dlopen_name: link_locator::dlopen_name(&locator, unit),
            declaring_unit: unit.to_string(),
            locator,
        }
    }

    /// The consumer project's own name in these tests. Every `resolved(unit, …)`
    /// below uses a `unit` different from this, so the library reads from the
    /// imported-package location `packages/<unit>.vendor/` (plan-48-B §4.3).
    const OWN_UNIT: &str = "app";

    /// Write a resolved library's source bytes where `vendor_source_path` will
    /// look for them given `OWN_UNIT` — the imported `packages/<unit>.vendor/`
    /// directory for a unit other than `OWN_UNIT`.
    fn write_vendor_source(root: &Path, library: &link_locator::ResolvedLibrary, bytes: &[u8]) {
        let path = vendor_source_path(root, OWN_UNIT, library);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
    }

    /// The file written and the string emitted must be the SAME string: a
    /// divergence is a `dlopen` miss at runtime and invisible at build time. Both
    /// sides build it through `dlopen_name`, so pin that they agree.
    #[test]
    fn the_copied_filename_is_the_emitted_dlopen_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let library = resolved("sqlite3", "libfoo.so");
        write_vendor_source(root, &library, b"bytes");
        let out = root.join("out");
        copy_vendor_libraries(
            std::slice::from_ref(&library),
            root,
            OWN_UNIT,
            std::slice::from_ref(&out),
        )
        .expect("copy succeeds");

        // The file on disk is named exactly what plan-46-C emits into the binary.
        assert!(out.join(&library.dlopen_name).is_file());
        assert_eq!(library.dlopen_name, "sqlite3-libfoo.so");
    }

    /// An imported binding's vendor file is read from its per-package
    /// `packages/<unit>.vendor/` directory, never the consumer's own `vendor/`
    /// (plan-48-B §4.3).
    #[test]
    fn imported_vendor_file_is_read_from_the_per_package_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let library = resolved("sqlite3", "libfoo.so");
        // Placed in the imported location; a same-named file in the consumer's own
        // `vendor/` must NOT be picked up in its place.
        std::fs::create_dir_all(root.join("vendor")).unwrap();
        std::fs::write(
            root.join("vendor").join("libfoo.so"),
            b"WRONG own-vendor bytes",
        )
        .unwrap();
        write_vendor_source(root, &library, b"right imported bytes");

        let out = root.join("out");
        copy_vendor_libraries(
            std::slice::from_ref(&library),
            root,
            OWN_UNIT,
            std::slice::from_ref(&out),
        )
        .expect("copy succeeds");
        assert_eq!(
            std::fs::read(out.join(&library.dlopen_name)).unwrap(),
            b"right imported bytes"
        );
    }

    /// The collision this prefix exists to prevent: two packages each vendoring a
    /// `libfoo.so`. Both must land as distinct files — without the prefix one
    /// would silently overwrite the other and both bindings would `dlopen`
    /// whichever won.
    #[test]
    fn two_packages_vendoring_the_same_filename_land_as_two_distinct_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let a = resolved("sqlite3", "libfoo.so");
        let b = resolved("imaging", "libfoo.so");
        write_vendor_source(root, &a, b"bytes");
        write_vendor_source(root, &b, b"bytes");
        assert_ne!(a.dlopen_name, b.dlopen_name);

        let out = root.join("out");
        copy_vendor_libraries(
            &[a.clone(), b.clone()],
            root,
            OWN_UNIT,
            std::slice::from_ref(&out),
        )
        .expect("copy");
        assert!(out.join("sqlite3-libfoo.so").is_file());
        assert!(out.join("imaging-libfoo.so").is_file());
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 2);
    }

    /// §4.5.2 residual check: two declaring units mapping to the same output name
    /// with *different* bytes. This should never fire — it is the guard rail that
    /// lets the prefix be trusted, not the mechanism.
    #[test]
    fn colliding_output_names_with_differing_hashes_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let mut a = resolved("same", "libfoo.so");
        let mut b = resolved("same", "libfoo.so");
        a.locator.hash = Some([1u8; 32]);
        b.locator.hash = Some([2u8; 32]);
        write_vendor_source(root, &a, b"bytes");
        let error = copy_vendor_libraries(&[a, b], root, OWN_UNIT, &[root.join("out")])
            .expect_err("differing hashes on one output name must be rejected");
        assert!(error.contains("collision"), "error: {error}");
    }

    /// Identical hashes are fine: the same bytes, legitimately shared, and the
    /// copy is idempotent.
    #[test]
    fn colliding_output_names_with_identical_hashes_are_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let a = resolved("same", "libfoo.so");
        let b = resolved("same", "libfoo.so");
        write_vendor_source(root, &a, b"bytes");
        copy_vendor_libraries(&[a, b], root, OWN_UNIT, &[root.join("out")])
            .expect("identical bytes may share an output name");
    }

    /// A build with no vendor locators writes no vendor directory at all.
    #[test]
    fn no_vendor_locators_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        copy_vendor_libraries(&[], dir.path(), OWN_UNIT, std::slice::from_ref(&out))
            .expect("no-op");
        assert!(
            !out.exists(),
            "an empty vendor set must not create the directory"
        );
    }

    /// The RPATH each backend emits and the directory the copy targets must agree
    /// — the loader looks exactly there and nowhere else.
    #[test]
    fn vendor_output_dirs_match_the_emitted_rpath_per_shape() {
        let root = Path::new("/proj");
        assert_eq!(
            vendor_output_dirs(root, "app", target::NativeBuildMode::Console),
            vec![PathBuf::from("/proj/build/vendor")],
            "console: $ORIGIN/vendor | @loader_path/vendor -> build/vendor"
        );
        assert_eq!(
            vendor_output_dirs(root, "app", target::NativeBuildMode::LinuxApp),
            vec![
                PathBuf::from("/proj/build/app-glibc.AppDir/usr/lib"),
                PathBuf::from("/proj/build/app-musl.AppDir/usr/lib"),
            ],
            "linux --app: $ORIGIN/../lib -> each flavor's AppDir usr/lib"
        );
        // The two Linux shapes must differ: the console `.out` and the AppDir's
        // `usr/bin/<name>` sit at different depths, so a shared directory would
        // mean one of the two RUNPATHs points at nothing.
        assert_ne!(
            vendor_output_dirs(root, "app", target::NativeBuildMode::Console),
            vendor_output_dirs(root, "app", target::NativeBuildMode::LinuxApp),
        );
        assert_eq!(
            vendor_output_dirs(root, "app", target::NativeBuildMode::MacApp),
            vec![PathBuf::from("/proj/build/app.app/Contents/Frameworks")],
            "macos -app: @executable_path/../Frameworks -> the bundle's Frameworks"
        );
    }

    /// plan-55-A §4.3: the resource directory each build shape writes into, kept
    /// in lockstep with plan-55-B's `resource_base_offset`.
    #[test]
    fn resource_output_dir_per_build_shape() {
        let root = Path::new("/proj");
        assert_eq!(
            resource_output_dirs(root, "app", target::NativeBuildMode::Console),
            vec![PathBuf::from("/proj/build")],
            "console: resources beside the executable in build/"
        );
        assert_eq!(
            resource_output_dirs(root, "app", target::NativeBuildMode::MacApp),
            vec![PathBuf::from("/proj/build/app.app/Contents/Resources")],
            "macos -app: the bundle's Contents/Resources"
        );
        assert_eq!(
            resource_output_dirs(root, "app", target::NativeBuildMode::LinuxApp),
            vec![
                PathBuf::from("/proj/build/app-glibc.AppDir/usr/share/app"),
                PathBuf::from("/proj/build/app-musl.AppDir/usr/share/app"),
            ],
            "linux --app: usr/share/<name> inside BOTH flavors' AppDirs"
        );
    }

    #[test]
    fn resource_src_fixed_prefix_splits_at_first_glob() {
        assert_eq!(resource_src_fixed_prefix("data/**/*.ogg"), "data");
        assert_eq!(resource_src_fixed_prefix("data/*.ogg"), "data");
        assert_eq!(resource_src_fixed_prefix("assets/logo.png"), "assets");
        assert_eq!(resource_src_fixed_prefix("*.ogg"), "");
        assert_eq!(resource_src_fixed_prefix("logo.png"), "");
        assert_eq!(resource_src_fixed_prefix("a/b/c/*.txt"), "a/b/c");
    }

    /// bug-298 defense in depth: manifest validation rejects an escaping `src`
    /// textually, but `copy_resources` is the step that actually reads files, and
    /// a symlink *inside* the project pointing outside it passes every textual
    /// check. Canonicalized containment is what catches that.
    #[test]
    #[cfg(unix)]
    fn copy_resources_refuses_a_source_that_resolves_outside_the_project() {
        let project = tempfile::tempdir().expect("project dir");
        let outside = tempfile::tempdir().expect("outside dir");
        std::fs::write(outside.path().join("secret.conf"), b"secret").unwrap();
        // An in-tree name that textually looks contained, but resolves out.
        std::os::unix::fs::symlink(outside.path(), project.path().join("assets")).unwrap();

        let out = project.path().join("build");
        std::fs::create_dir_all(&out).unwrap();
        let entries = vec![crate::manifest::ResourceEntry {
            src: "assets/*.conf".to_string(),
            dst: "cfg/".to_string(),
        }];
        let err = copy_resources(project.path(), &entries, &out)
            .expect_err("a source resolving outside the project must be refused");
        assert!(
            err.contains("outside the project root"),
            "unexpected error: {err}"
        );
        // Nothing was copied.
        assert!(!out.join("cfg/secret.conf").exists());
    }

    /// plan-55-A §4.3: the three worked examples — flat glob, `**` subtree
    /// preservation, and a single literal file — plus the empty-match no-op.
    #[test]
    fn copy_resources_maps_the_worked_examples() {
        let project = tempfile::tempdir().expect("project dir");
        let root = project.path();
        // data/Mozart1.ogg, data/loops/kick.ogg, assets/logo.png.
        std::fs::create_dir_all(root.join("data/loops")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("data/Mozart1.ogg"), b"a").unwrap();
        std::fs::write(root.join("data/loops/kick.ogg"), b"b").unwrap();
        std::fs::write(root.join("assets/logo.png"), b"c").unwrap();

        let out = tempfile::tempdir().expect("out dir");
        let resource_dir = out.path();
        let entries = vec![
            crate::manifest::ResourceEntry {
                src: "data/*.ogg".to_string(),
                dst: "music/".to_string(),
            },
            crate::manifest::ResourceEntry {
                src: "data/**/*.ogg".to_string(),
                dst: "all/".to_string(),
            },
            crate::manifest::ResourceEntry {
                src: "assets/logo.png".to_string(),
                dst: "img/".to_string(),
            },
            // Matches nothing — must be a silent no-op.
            crate::manifest::ResourceEntry {
                src: "nowhere/*.dat".to_string(),
                dst: "x/".to_string(),
            },
        ];
        copy_resources(root, &entries, resource_dir).expect("copy");

        // data/*.ogg -> music/ : only the top-level file, not the subtree one.
        assert!(resource_dir.join("music/Mozart1.ogg").is_file());
        assert!(!resource_dir.join("music/loops").exists());
        // data/**/*.ogg -> all/ : subtree structure preserved below the prefix.
        assert!(resource_dir.join("all/Mozart1.ogg").is_file());
        assert!(resource_dir.join("all/loops/kick.ogg").is_file());
        // assets/logo.png -> img/logo.png.
        assert!(resource_dir.join("img/logo.png").is_file());
        assert_eq!(
            std::fs::read(resource_dir.join("img/logo.png")).unwrap(),
            b"c"
        );
        // The empty-match entry created nothing.
        assert!(!resource_dir.join("x").exists());
    }

    /// A Linux **console** build emits both libc flavors, so both must be checked;
    /// a Linux **app** build emits a single glibc binary, so demanding a musl
    /// locator (and a musl blob in `vendor/`) for a flavor it never emits would
    /// fail a correct project.
    #[test]
    fn emitted_link_targets_track_what_each_build_mode_actually_emits() {
        let linux = target::BuildTarget {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        };
        let console: Vec<String> = emitted_link_targets(&linux, target::NativeBuildMode::Console)
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(console, vec!["linux/x86_64/glibc", "linux/x86_64/musl"]);

        // plan-56-B §4.1: app mode is no longer glibc-only — it emits one
        // AppImage per libc, so vendor resolution must cover both. Resolving
        // only glibc here would put the glibc blob inside the musl image.
        let app: Vec<String> = emitted_link_targets(&linux, target::NativeBuildMode::LinuxApp)
            .iter()
            .map(|t| t.to_string())
            .collect();
        assert_eq!(
            app, console,
            "app mode resolves the same libc set as console"
        );

        // macOS has no libc axis in either mode.
        let macos = target::BuildTarget {
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
        };
        for mode in [
            target::NativeBuildMode::Console,
            target::NativeBuildMode::MacApp,
        ] {
            let slots = emitted_link_targets(&macos, mode);
            assert_eq!(slots.len(), 1);
            assert_eq!(slots[0].libc, None);
            assert_eq!(slots[0].to_string(), "macos/aarch64");
        }
    }
}
