use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tinyjson::JsonValue;

use crate::ast;
use crate::binary_repr;
use crate::ir;
use crate::json::json_string;
use crate::manifest::entry::validate_entry_point;
use crate::manifest::libraries::Libc;
use crate::manifest::package::{
    external_package_function_types, external_package_function_types_from_files,
    imported_global_defs, imported_global_defs_from_files, imported_resource_closers,
    imported_type_defs, imported_type_defs_from_files, installed_package_files, package_metadata,
};
use crate::manifest::project_kind;
use crate::manifest::validate_project_manifest;
use crate::manifest::{build_mode_is_app, icon_path};
use crate::monomorph;
use crate::resolver;
use crate::rules;
use crate::target;

/// How much human-facing progress `mfb build` prints (plan-36). Never reaches
/// codegen — only the CLI's own `println!`/`eprintln!` lines are gated on it, so
/// the emitted artifact bytes are identical across all four levels.
///
/// Ordered least-to-most verbose, and compared with `>=` rather than `==` at
/// every gate: a level that prints must print everything the level below it
/// does, so `-vv` is `-v` plus more rather than a different report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum Verbosity {
    /// `-q`/`--quiet`: today's minimal output — only the `Wrote … to` artifact
    /// line(s) and any diagnostics.
    Quiet,
    /// Default: the `Building …` summary line plus the artifact line.
    #[default]
    Normal,
    /// `-v`/`--verbose`: additionally a `phase <name> <N>ms` line per front-end
    /// stage and a `<catalog row>: <count>` optimizer-pass fire-count line per
    /// landed dial row. Doubles as a lightweight build profiler.
    Verbose,
    /// `-vv` / `-v -v` / `--verbose --verbose`: everything `-v` prints, plus the
    /// [`crate::trace`] compile profiler — a `codegen: <stage> <N>ms` completion
    /// time for each live codegen sub-stage, and, once the build ends, the
    /// nested span tree, the slowest-unit leaderboards, and the size counters.
    ///
    /// `-v` tells you *which phase* is slow; this tells you which pass, which
    /// function, and over how much input. It is a diagnostic mode, not a louder
    /// default: the report is tens of lines long.
    Trace,
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
    /// The codegen sub-stage currently streaming, and when it started. Only
    /// `Trace` fills it: `-v` prints a bare stage name on entry, and printing a
    /// *duration* means waiting for the stage to end, which is one stage later.
    /// `RefCell` because the backend receives `progress` as a `&dyn Fn(&str)` —
    /// a shared closure — so the reporter cannot take `&mut self` there.
    stage: std::cell::RefCell<Option<(String, std::time::Instant)>>,
}

impl Reporter {
    pub(crate) fn new(level: Verbosity) -> Self {
        // The compile profiler is a process-global sink (spans open deep inside
        // codegen, which has no channel back to the CLI), so `-vv` arms it here,
        // before the pipeline runs, rather than threading a handle down.
        if level >= Verbosity::Trace {
            crate::trace::enable();
        }
        Self {
            level,
            stage: std::cell::RefCell::new(None),
        }
    }

    /// The `Building …` context line — printed at Normal and above, suppressed
    /// at Quiet.
    fn summary(&self, line: &str) {
        if self.level != Verbosity::Quiet {
            eprintln!("{line}");
        }
    }

    /// One `phase <name> <N>ms` profiler line — printed at Verbose and above.
    /// The caller always computes the elapsed time (so `-v` and the default take
    /// an identical path into codegen); only the print is level-gated.
    fn phase(&self, name: &str, dt: Duration) {
        if self.level >= Verbosity::Verbose {
            eprintln!("phase {name} {}ms", dt.as_millis());
        }
    }

    /// One live `codegen: <stage>` line, printed as a `write_executable`
    /// sub-stage is entered — Verbose and above, on stderr (bug-393). Unlike
    /// [`Reporter::phase`], which is a post-hoc total printed once the whole
    /// stage completes, these stream *during* codegen so a minute-plus build is
    /// visibly progressing and the slow sub-stage is named. The backend calls it
    /// unconditionally through a closure; the level gate lives here, so codegen
    /// bytes never depend on verbosity.
    ///
    /// At `Trace` each stage additionally reports how long it took, printed when
    /// the *next* stage opens (or by [`Reporter::finish_stage`] for the last
    /// one). These are the only per-stage numbers covering the platform-specific
    /// tail — object writing, linking, bundle sealing — which lives in each
    /// backend's `write_executable` rather than in the shared lowering the span
    /// tree instruments.
    fn progress(&self, stage: &str) {
        if self.level < Verbosity::Verbose {
            return;
        }
        self.finish_stage();
        eprintln!("codegen: {stage}");
        if self.level >= Verbosity::Trace {
            *self.stage.borrow_mut() = Some((stage.to_string(), std::time::Instant::now()));
            // Mirror the stage into the span tree, so the deep spans the shared
            // lowering opens land under the stage that contains them and each
            // backend's platform-specific tail gets a node of its own.
            crate::trace::stage(stage);
        }
    }

    /// Close out the streaming codegen stage, printing its elapsed time. Called
    /// when the next stage opens and once more once codegen returns, so the
    /// final stage (linking) is timed like every other — and so the stage span
    /// closes *inside* the enclosing `codegen+link` span rather than outliving
    /// it.
    fn finish_stage(&self) {
        crate::trace::end_stage();
        if let Some((name, start)) = self.stage.borrow_mut().take() {
            eprintln!("codegen: {name} {}ms", start.elapsed().as_millis());
        }
    }

    /// One `<catalog row>: <count>` line per landed dial row — Verbose and
    /// above, on stderr, printed once codegen has run. The counts accumulate in
    /// `optimizer::stats` as the gated passes fire (the passes have no channel
    /// back to the CLI); a row is printed only when the active dial actually
    /// ran it, so `-O0` prints nothing and `-O1` omits the L2/L3 rows.
    fn opt_stats(&self) {
        if self.level < Verbosity::Verbose {
            return;
        }
        for row in crate::optimizer::catalog::rows() {
            if crate::optimizer::level_enabled(row.level) {
                eprintln!("{}: {}", row.name, row.fired());
            }
        }
    }

    /// The `-vv` compile-profiler report: the span tree, the slowest-unit
    /// leaderboards, and the size counters. Renders nothing at any lower level
    /// (no span was ever recorded).
    fn trace_report(&self) {
        self.finish_stage();
        crate::trace::render();
    }
}

/// Top-level function declarations across an elaborated project, for the `-vv`
/// size counters.
///
/// Counted on the HIR rather than the IR because the interesting comparison is
/// *across* monomorphization — the same shape on both sides of it — and only
/// the HIR exists on the generic side.
fn hir_function_count(project: &crate::hir::HirProject) -> u64 {
    project
        .files
        .iter()
        .flat_map(|file| &file.items)
        .filter(|item| matches!(item, crate::hir::HirItem::Function(_)))
        .count() as u64
}

pub(crate) struct BuildOptions {
    pub(crate) location: PathBuf,
    /// Requested artifact dumps, in flag order. Empty means a full
    /// validate/build (the flagless `mfb build`). Any combination of the
    /// output flags may be given in one invocation; each artifact is written
    /// from a single shared front-end pass.
    pub(crate) outputs: Vec<BuildOutput>,
    /// Where a `kind: "package"` build writes its `.mfp`. `None` means beside
    /// the sources (`<project_dir>/<name>.mfp`), which is what `mfb build` on a
    /// package project does.
    ///
    /// Set only when the compiler is building a dependency declared by SOURCE
    /// DIRECTORY on an importer's behalf (bug-480): the compiled interface is an
    /// intermediate of the IMPORTER's build and belongs in its
    /// `build/packages/` cache, never in the dependency's source tree.
    pub(crate) package_output_dir: Option<PathBuf>,
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
    /// `--debug` (plan-130): build a program that writes a measurement report to
    /// stderr as the last act of `_mfb_shutdown`. Reaches codegen as
    /// `NirModule::debug`; a build without it is byte-identical to one that
    /// predates the flag. It also writes the program's function and data names
    /// into the executable's native symbol table, for debuggers and profilers
    /// (plan-166; the linker reads `EncodedImage::debug_symbols`).
    pub(crate) debug: bool,
    /// `--config <name>` (plan-167-C): the named config in `project.json` (merged
    /// with `project-local.json`) that `os::getConfigOr` calls fold from. `None`
    /// selects `release`. Read only for the invoked project; a source
    /// dependency's build never sets it.
    pub(crate) config: Option<String>,
    /// Optimization scale level selected by `-O<N>` / `--optimize <N>`
    /// (plan-100). Defaults to `-O1`, at which the shipping Level-1 passes run
    /// -- so the flagless build is today's exact codegen. `-O0` turns the dial
    /// passes off and legitimately emits different (unoptimized) code.
    pub(crate) opt: crate::optimizer::OptLevel,
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

    /// The human-readable noun for this output, used both in the `Wrote <noun> to
    /// …` success line and in the package-unsupported diagnostic. Kept in one
    /// place so the two sites cannot drift (bug-340 B4).
    fn label(self) -> &'static str {
        match self {
            BuildOutput::Ast => "AST",
            BuildOutput::Ir => "IR",
            BuildOutput::BinaryRepr => "binary representation",
            BuildOutput::NativeIr => "native IR",
            BuildOutput::NativePlan => "native plan",
            BuildOutput::NativeObjectPlan => "native object plan",
            BuildOutput::NativeCodePlan => "native code plan",
            BuildOutput::Mir => "MIR",
        }
    }
}

pub(crate) fn build_project(options: &BuildOptions) -> Result<(), ()> {
    // Record the optimization level for the gated passes to read at their
    // seams (plan-100 §2). Default `-O1` runs the Level-1 rows, as today.
    crate::optimizer::set_opt_level(options.opt);
    let reporter = Reporter::new(options.verbosity);
    let target = options.target.clone();
    let project_path = options.location.join("project.json");
    let manifest = validate_project_manifest(&project_path)?;
    // plan-167-C: select the named config on every path of the invoked
    // project's build, so an undeclared `--config` name fails a package build
    // too. A source dependency's build (`package_output_dir` set) selects
    // nothing: it emits a `.mfp`, which never folds.
    let selected_config = if options.package_output_dir.is_none() {
        select_build_config(options, &project_path)?
    } else {
        crate::manifest::config::SelectedConfig::default()
    };
    let project_kind = project_kind(&manifest);

    // bug-480 Defect A: a dependency declared by SOURCE DIRECTORY has no
    // installed `.mfp`, so compile it into this build's package cache first.
    // Everything downstream — the verification report just below, the resolver,
    // the shape pass, monomorph's overload table, the signature/type/closer
    // readers and `merge_packages` — then resolves it through the one artifact
    // format they already understand.
    build_source_dependencies(options, &manifest)?;

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
            // Named from the registered backends, so the list cannot drift: "Linux"
            // alone read as including linux-riscv64, which has no app backend.
            let app_targets: Vec<String> = target::registered_targets()
                .iter()
                .filter(|candidate| target::target_supports_app_mode(candidate))
                .map(|candidate| candidate.name())
                .collect();
            eprintln!(
                "error: app mode is not supported on {} (app-mode targets: {})",
                target.name(),
                app_targets.join(", ")
            );
            return Err(());
        }
    }
    // The target OS selects the app toolkit and therefore the build mode. The CLI
    // has already verified the target supports app mode at this point.
    let build_mode = if app_mode {
        match target.os.as_str() {
            "linux" => target::NativeBuildMode::LinuxApp,
            // plan-66-I: an explicit Windows arm — the `_ => MacApp` fallthrough
            // otherwise misroutes a Windows `-app` build into the macOS toolkit.
            "windows" => target::NativeBuildMode::WindowsApp,
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
    // The `-vv` span tree mirrors the `-v` phase lines at the top level, then
    // keeps going: every `reporter.phase(name, …)` below has a `trace::span`
    // with the same name wrapped around it, so a reader can descend from a slow
    // `phase` line straight into the sub-steps that account for it. The spans
    // are inert unless `-vv` armed the tracer (`crate::trace`).
    let parse_span = crate::trace::span("parse");
    let mut ast = {
        let _span = crate::trace::span("parse_project");
        ast::parse_project(project_name, &options.location, &manifest)?
    };
    // plan-62-A §3.3 / plan-98-B: the `app::` and `canvas::` packages are importable
    // ONLY in `--app` builds. `is_builtin_import` makes `IMPORT app` legal at the
    // name gate (so the resolver does not reject it as an unknown package); the CLI
    // is the sole place that sees the full `app_mode` decision (the additive `-app`
    // flag over the manifest `"mode":"app"`), so the app-mode requirement is
    // enforced here, before any lowering — the same shape as the
    // `target_supports_app_mode` reject above. A console build that imports either
    // is a compile error: `app` controls a window's presentation surface and
    // `canvas` draws on one, and a console binary has neither.
    if !app_mode {
        let app_only = ast.files.iter().find_map(|file| {
            file.imports
                .iter()
                .map(|import| import.package_name())
                .find(|name| *name == "app" || *name == "canvas")
        });
        if let Some(package) = app_only {
            eprintln!(
                "error: the `{package}` package requires app mode (build with -app or set \"mode\": \"app\" in project.json)"
            );
            return Err(());
        }
    }
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
    let scope_diagnostics = {
        let _span = crate::trace::span("scope_privates");
        crate::ast::scope_privates::scope_privates(&mut ast)
    };
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
    let test_lowering = {
        let _span = crate::trace::span("lower_testing_blocks");
        crate::testing::lower_testing_blocks(&mut ast, options.mode, &project_abs)
    };
    if options.mode.coverage() {
        let covmap = project_abs.join(crate::testing::COVMAP_FILE);
        if let Err(err) = crate::testing::coverage::write_covmap(&covmap, &test_lowering.cov_slots)
        {
            eprintln!("warning: failed to write coverage map: {err}");
        }
    }
    drop(parse_span);
    reporter.phase("parse", parse_start.elapsed());
    crate::trace::count("source files", ast.files.len() as u64);
    let resolve_start = std::time::Instant::now();
    let resolve_span = crate::trace::span("resolve");
    {
        let _span = crate::trace::span("resolve_project");
        resolver::resolve_project(&options.location, &manifest, &ast)?;
    }
    // Inject the builtin package sources BEFORE monomorphization so the
    // monomorphizer sees them (in particular so a builtin's native overload set is
    // mangled to private symbols like a user overload, not collided at codegen).
    let augmented = {
        let _span = crate::trace::span("augment_project");
        resolver::augment_project(&ast, options.debug)?
    };
    // plan-102-D3: elaborate ABOVE monomorph, then monomorphize the generic HIR
    // into concrete HIR, which `ir::lower_augmented_project` consumes directly.
    let generic_hir = {
        let _span = crate::trace::span("elaborate");
        crate::hir::elaborate(&augmented)
    };
    crate::trace::count("HIR functions (generic)", hir_function_count(&generic_hir));
    let mut concrete_hir = {
        let _span = crate::trace::span("monomorphize");
        monomorph::monomorphize_project(&options.location, &generic_hir)?
    };
    // bug-377: the imported packages' resources and types. Read here
    // because bug-976 needs them before anything consumes the concrete HIR.
    let imported_resources = imported_resource_closers(&options.location, &manifest);
    let imported_types = imported_type_defs(&options.location, &manifest);
    // bug-976: a parameter is a resource because its type is one. `elaborate`
    // derived the flag from what the sources declare; derive it again on the
    // concrete HIR, so an instantiated generic parameter is answered from its
    // concrete type and an imported package's resource counts as one.
    {
        let types = crate::hir::resource_params::ResourceTypes::from_project(&concrete_hir)
            .with_imported(&imported_resources, &imported_types);
        crate::hir::resource_params::derive_resource_params(&mut concrete_hir, &types);
    }
    let concrete_hir = concrete_hir;
    // The pair of counters straddling monomorphization is the one that explains
    // a slow *back* end: everything below here is per-function work, so a 10x
    // jump across this line means codegen was handed 10x the input, which is a
    // different problem from codegen being slow per function.
    crate::trace::count(
        "HIR functions (concrete)",
        hir_function_count(&concrete_hir),
    );
    // plan-106-D: every pass from here down consumes `&concrete_hir` directly.
    // This is where the compile path used to turn around — `deelaborate` rendered
    // the concrete HIR back to an AST for `resolve_augmented`, entry validation
    // and the former source checker, and that render was the last backward edge in the
    // compiler (and the last thing depending on `parse`↔`name` byte-exactness).
    //
    // Skip DOC validation on the post-monomorph pass: monomorphization renames
    // overloaded/generic declarations, so their doc headers would falsely appear
    // unresolved. The original-AST pass above already validated them. The AST is
    // already augmented, so resolve without re-injecting the package sources.
    {
        let _span = crate::trace::span("resolve_augmented");
        resolver::resolve_augmented(&options.location, &manifest, &concrete_hir, false)?;
    }
    drop(resolve_span);
    reporter.phase("resolve", resolve_start.elapsed());
    let verify_start = std::time::Instant::now();
    let verify_span = crate::trace::span("verify");
    // In test mode the synthesized driver is the entry point (it replaces the
    // manifest `main`), so bypass entry validation and point at the driver.
    let entry = match &test_lowering.entry {
        Some(name) => Some(ir::EntryPoint {
            name: name.clone(),
            returns: crate::types::ParameterType::Integer,
            accepts_args: false,
        }),
        None => validate_entry_point(&options.location, &manifest, &concrete_hir)?,
    };
    // The semantic rules are split across two passes that both run to
    // completion (neither short-circuits the other) so a program with errors of
    // both kinds reports all of them (plan-107):
    //   - `ir::shape` walks the concrete HIR for the rules whose evidence
    //     lowering ERASES — named arguments, EXIT flavors, inline-trap
    //     boundaries, literal spellings, the TESTING assertions, native CONST
    //     pins and FREE signatures — each with a justification line naming
    //     the erased fact; its stream renders first;
    //   - `ir::verify` runs on the source-lowered IR and is the sole rejecter
    //     for every other rule — the same implementation that guards decoded
    //     package IR, so source and package are checked once.
    // Lowering is total (plan-20-D), so it is safe to run even when the shape
    // pass found errors.
    //
    // Empty external maps for everything EXCEPT imported resource producers
    // (bug-377). An inferred binding takes its type from the initializer, so
    // `LET music = libsnd::openSound(...)` lowers to a bind of unknown type when
    // the imported signature is missing, and the `RES` ownership axis — which
    // keys on the bound type — can never fire.
    //
    // Handing over *every* imported signature is the wrong cure: it tells verify
    // an imported name's type without also giving it that type's definition, and
    // a half-informed checker is worse than an uninformed one. `LET result =
    // thread::waitFor(t)` then resolves to the imported union `ReturnChoice`
    // whose variants are still absent, so `check_match_exhaustive` reads it as an
    // *open* type and demands a `CASE ELSE` from an exhaustive match
    // (`rt-behavior/threads/thread-return-union`). Same trap bug-258 documents a
    // few hundred lines into `verify`: only a POSITIVELY known type may reject.
    //
    // So restrict the map to functions returning an imported RESOURCE type —
    // exactly the names `imported_resource_closers` also gives verify the
    // registry rows to interpret. Signature and definition arrive together, and
    // no other inference shifts.
    //
    // Both passes collect (rather than print) so their diagnostics can be
    // merged and rendered in a single line-ordered pass, in the stream order
    // the goldens record (plan-20-Z).
    // bug-377: the source IR names an imported resource's type but carries no
    // record that it *is* a resource, so verify's resource rules need the
    // imported packages' `RESOURCE_TABLE` rows handed to them explicitly.
    let imported_resource_types: std::collections::HashSet<&str> = imported_resources
        .iter()
        .map(|resource| resource.type_name.as_str())
        .collect();
    let all_external_signatures = external_package_function_types(&options.location, &manifest);
    // plan-105-A: the signature arrives TYPED, so the return type is a field —
    // not the tail after the last ` AS ` of a string the driver formatted itself.
    // A stateful resource return (`SoundFile STATE FileInfo`) carries its STATE
    // clause inside the nominal leaf; plan-111-G strips it structurally, so the
    // driver no longer reaches into codegen's `&str` name helpers at all.
    // `imported_resource_types` is a NAME set, so the base renders for that
    // lookup only.
    let returns_imported_resource = |signature: &ir::ExternalSignature| {
        imported_resource_types.contains(signature.returns.without_state().name().as_ref())
    };
    let source_external_signatures: HashMap<String, ir::ExternalSignature> =
        all_external_signatures
            .iter()
            .filter(|(_, signature)| returns_imported_resource(signature))
            .map(|(name, signature)| (name.clone(), signature.clone()))
            .collect();
    // bug-551: the imported packages' exported `EXPORT LET`/`EXPORT MUT`, typed.
    // The shape pass, lowering and verify each need them for the same reason
    // they need `imported_types` — the source-path project tables hold only the
    // importer's own declarations.
    let imported_globals = imported_global_defs(&options.location, &manifest);
    // plan-107-E: the pre-lowering shape pass — the source rules whose evidence
    // lowering erases — runs over the same HIR, with the same signature and
    // type inputs, that lowering is about to consume. Its stream comes first.
    let imported_resource_type_names: Vec<String> = imported_resource_types
        .iter()
        .map(|name| name.to_string())
        .collect();
    let shape_diagnostics = {
        let _span = crate::trace::span("shape");
        ir::shape::collect_diagnostics(
            &options.location,
            &concrete_hir,
            &imported_types,
            &imported_globals,
            &all_external_signatures,
            &imported_resource_type_names,
        )
    };
    let source_ir = {
        let _span = crate::trace::span("lower to IR");
        ir::lower_augmented_project(
            &concrete_hir,
            entry.clone(),
            &source_external_signatures,
            &imported_types,
            &imported_globals,
        )
    };
    crate::trace::count("IR functions", source_ir.functions.len() as u64);
    // plan-107-C: the LINK declarations' source spans, so verify's native-ABI
    // rules report at the slot/parameter/field lines the former source checker did.
    let link_spans = ir::link_spans(&concrete_hir);
    let verify_diagnostics = {
        let _span = crate::trace::span("verify rules");
        ir::verify_source_diagnostics(
            &source_ir,
            &options.location,
            &imported_resources,
            &imported_types,
            &imported_globals,
            &link_spans,
        )
    };
    let mut diagnostics = shape_diagnostics;
    diagnostics.extend(verify_diagnostics);
    // EXPORT is only valid in a package project (it is the `.mfp` export flag);
    // in an executable a top-level EXPORT is an error. Checked here because the
    // manifest `kind` is known at the build boundary (see
    // `ir::shape::export_in_executable_diagnostics`).
    let is_package = crate::manifest::project_kind(&manifest) == "package";
    diagnostics.extend(ir::shape::export_in_executable_diagnostics(
        is_package, &ast,
    ));
    // bug-624 B: a package's EXPORT surface may not name a type it does not
    // export — the `.mfp` type table would not carry it, and every importer
    // would reject the package.
    diagnostics.extend(ir::shape::export_names_non_exported_type_diagnostics(
        is_package,
        &source_ir,
        &options.location,
    ));
    diagnostics.extend(scope_diagnostics);
    drop(verify_span);
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
            let external_signatures = external_package_function_types_from_files(&packages)
                .map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let mut ir = ir::lower_augmented_project(
                &concrete_hir,
                entry.clone(),
                &external_signatures,
                &imported_type_defs_from_files(&packages),
                &imported_global_defs_from_files(&packages),
            );
            // plan-46-B §4.3: an executable that declares its *own* `LINK` block
            // needs its own locators too — an imported binding's come from that
            // binding's `.mfp` section 10 instead. Runs the same missing-entry /
            // vendor-hash / coverage checks as a package build.
            if !assemble_native_libraries_for_ir(&mut ir, &manifest, &options.location) {
                return Err(());
            }
            // plan-58-C: the `OUT CBuffer` allocation ceiling. Read here rather
            // than encoded into a package, because LINK thunks are emitted when
            // an EXECUTABLE links — so the ceiling that applies is this project's,
            // and an imported binding cannot raise it on the app's behalf.
            ir.max_buffer_bytes = crate::manifest::max_buffer_bytes(&manifest);
            // plan-172-K: how long a `canvas::AutoPaint` flash lasts — this project's
            // setting, baked into the executable's graphics state.
            ir.canvas_auto_paint_ms = crate::manifest::canvas_auto_paint_ms(&manifest);
            // plan-179: the arena memory cap — this project's setting, applied to the
            // whole process.
            ir.arena_memory = crate::manifest::arena_memory_limits(&manifest);
            // plan-167-B: the config `os::getConfigOr` calls fold from. Like the
            // ceiling above it is this project's, and applies to package code.
            ir.selected_config = selected_config.clone();
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
                if let Err(err) = clear_build_dir(&build_dir) {
                    eprintln!("error: failed to clear '{}': {err}", build_dir.display());
                    return Err(());
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
            let codegen_span = crate::trace::span("codegen+link");
            // bug-393: stream live codegen sub-stage lines during the otherwise
            // opaque `write_executable` block. The closure gates on verbosity via
            // the reporter, so backends call `progress(...)` unconditionally and
            // Normal/Quiet stay silent.
            let progress = |stage: &str| reporter.progress(stage);
            let executable_paths = target::write_executable(
                output_dir,
                &ir,
                &target,
                &packages,
                signing.as_ref().map(|signing| &signing.executable_signing),
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
                // Bake `options.stdinLogCap` (or the default) into the executable.
                crate::manifest::stdin_log_cap(&manifest),
                // plan-130: the `--debug` report switch.
                crate::codegen::debug::DebugOptions {
                    enabled: options.debug,
                },
                &progress,
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
            let vendor_copies: Vec<(
                Vec<crate::codegen::link::locator::ResolvedLibrary>,
                Vec<PathBuf>,
            )> = if build_mode == target::NativeBuildMode::LinuxApp {
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
            // resource directory in `--app` mode), where `os::appResourcePath`
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
            // Close the last streaming codegen stage before the enclosing span,
            // so the stage nests inside `codegen+link` instead of outliving it.
            reporter.finish_stage();
            drop(codegen_span);
            reporter.phase("codegen+link", codegen_start.elapsed());
            reporter.opt_stats();
            // Before the test binary runs: its own output would otherwise be
            // interleaved with the profile, and the profile is about the
            // *compile*.
            reporter.trace_report();
            // `mfb test` compiles the driver, then runs it and adopts its exit
            // status (non-zero iff any case failed).
            if options.mode.is_test() {
                if let Some(dir) = test_output_dir {
                    // Host run: execute the freshly linked binary, then remove
                    // the whole temp directory regardless of outcome.
                    let status = match host_test_executable(&executable_paths) {
                        Some(path) => {
                            let runner = std::env::var("MFB_TEST_RUNNER").ok();
                            run_test_binary(path, runner.as_deref())
                        }
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
            let external_signatures = external_package_function_types_from_files(&packages)
                .map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let mut ir = ir::lower_augmented_project(
                &concrete_hir,
                entry.clone(),
                &external_signatures,
                &imported_type_defs_from_files(&packages),
                &imported_global_defs_from_files(&packages),
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
            let package_path = {
                let _span = crate::trace::span("codegen+link");
                target::write_package(
                    // bug-480: a source-directory dependency's `.mfp` is an
                    // intermediate of the IMPORTER's build, so it is written
                    // into that build's package cache instead of beside these
                    // sources.
                    options
                        .package_output_dir
                        .as_deref()
                        .unwrap_or(&options.location),
                    &ir,
                    &metadata,
                    &packages,
                    signing.as_ref().map(|signing| &signing.package_signing),
                )
                .map_err(|err| {
                    eprintln!("error: {err}");
                })?
            };
            reporter.phase("codegen+link", codegen_start.elapsed());
            reporter.trace_report();
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
                let external_signatures =
                    external_package_function_types(&options.location, &manifest);
                let ir = ir::lower_augmented_project(
                    &concrete_hir,
                    entry.clone(),
                    &external_signatures,
                    &imported_type_defs(&options.location, &manifest),
                    &imported_global_defs(&options.location, &manifest),
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
                    let what = output.label();
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
            let external_signatures = external_package_function_types_from_files(packages)
                .map_err(|err| {
                    eprintln!("error: {err}");
                })?;
            let mut lowered = ir::lower_augmented_project(
                &concrete_hir,
                entry.clone(),
                &external_signatures,
                &imported_type_defs_from_files(packages),
                &imported_global_defs_from_files(packages),
            );
            // The debug emitters below run the same NIR/plan/code pipeline as a
            // real executable build, so they need the same `LINK` locator table
            // the real path assembles above — without it every `--nir`/`--nplan`/
            // `--nobj`/`--ncode` dump of a project declaring its own LINK block
            // failed with NATIVE_LIBRARY_NO_MATCH, which is exactly the case
            // where the dump is wanted.
            if !assemble_native_libraries_for_ir(&mut lowered, &manifest, &options.location) {
                return Err(());
            }
            // The `OUT CBuffer` ceiling the executable path sets (plan-58-C).
            // Without it a dump baked the 64 MiB default whatever `options.maxBuffer`
            // said, so `--ncode` disagreed with the program it claims to show.
            lowered.max_buffer_bytes = crate::manifest::max_buffer_bytes(&manifest);
            lowered.canvas_auto_paint_ms = crate::manifest::canvas_auto_paint_ms(&manifest);
            lowered.arena_memory = crate::manifest::arena_memory_limits(&manifest);
            // plan-167-B: fold `os::getConfigOr` exactly as the executable does,
            // so a native dump shows the values the program will carry.
            lowered.selected_config = selected_config.clone();
            ir_cache = Some(lowered);
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
            BuildOutput::NativeIr
            | BuildOutput::NativePlan
            | BuildOutput::NativeObjectPlan
            | BuildOutput::NativeCodePlan
            | BuildOutput::Mir => {
                // All five native dumps share the same writer signature
                // (`-> Result<PathBuf, String>`) and error/success handling;
                // only the writer function and the noun differ (bug-340 B4).
                let writer = match output {
                    BuildOutput::NativeIr => target::write_nir,
                    BuildOutput::NativePlan => target::write_native_plan,
                    BuildOutput::NativeObjectPlan => target::write_native_object_plan,
                    BuildOutput::NativeCodePlan => target::write_native_code_plan,
                    _ => target::write_mir,
                };
                let path = match writer(
                    &options.location,
                    ir,
                    &target,
                    packages,
                    build_mode,
                    crate::codegen::debug::DebugOptions {
                        enabled: options.debug,
                    },
                ) {
                    Ok(path) => path,
                    Err(err) => {
                        eprintln!("error: {err}");
                        return Err(());
                    }
                };
                println!("Wrote {} to {}", output.label(), path.display());
            }
            BuildOutput::Ast | BuildOutput::Ir => unreachable!("handled above"),
        }
    }

    // The native debug emitters above run the same gated NIR/machine passes as
    // a real build, so `-v` reports their fire counts here too. Front-end-only
    // dumps (`--ast`/`--ir`/`--br`) never enter the native pipeline — no line.
    if options.outputs.iter().any(|output| {
        matches!(
            output,
            BuildOutput::NativeIr
                | BuildOutput::NativePlan
                | BuildOutput::NativeObjectPlan
                | BuildOutput::NativeCodePlan
                | BuildOutput::Mir
        )
    }) {
        reporter.opt_stats();
    }
    // The dump paths are worth profiling too — `--ncode` runs the whole native
    // pipeline — and the front-end spans are recorded regardless of which
    // output was asked for.
    reporter.trace_report();

    Ok(())
}

/// The named config this build folds `os::getConfigOr` calls from: the one
/// `--config` names, or `release` (plan-167-C), after `project-local.json` is
/// merged over `project.json`'s `config`. Every diagnostic is rendered against
/// the file it belongs to. An undeclared name is `PROJECT_JSON_CONFIG_UNKNOWN`
/// at the `config` line.
fn select_build_config(
    options: &BuildOptions,
    project_path: &Path,
) -> Result<crate::manifest::config::SelectedConfig, ()> {
    use crate::manifest::config;
    let contents = std::fs::read_to_string(project_path).map_err(|err| {
        rules::show_diagnostic(
            "PROJECT_JSON_READ_FAILED",
            &err.to_string(),
            project_path,
            1,
            1,
            1,
        );
    })?;
    let project_config =
        config::load_project_config(&options.location, &contents).map_err(|diagnostics| {
            for diagnostic in &diagnostics {
                let (path, text) = match diagnostic.file {
                    config::ConfigFile::Project => (project_path.to_path_buf(), contents.clone()),
                    config::ConfigFile::Local => {
                        let path = options.location.join(config::PROJECT_LOCAL_JSON);
                        let text = std::fs::read_to_string(&path).unwrap_or_default();
                        (path, text)
                    }
                };
                crate::manifest::render_config_diagnostic(diagnostic, &path, &text);
            }
        })?;
    let name = options.config.as_deref().unwrap_or(config::RELEASE);
    project_config
        .select(name, options.location.clone())
        .ok_or_else(|| {
            crate::manifest::render_config_diagnostic(
                &project_config.unknown_name(name),
                project_path,
                &contents,
            );
        })
}

/// The manifest's `libraries` section as an `IrProject::native_libraries` table.
///
/// `testutil::try_code_for_fixture_project` lowers a fixture from its project
/// directory the way this module does, and a fixture whose `LINK` block names a
/// vendored library needs the locators the manifest carries — without them
/// lowering stops at `NATIVE_LIBRARY_NO_MATCH`, which is the build refusing
/// correctly and the harness looking broken.
///
/// A thin re-export rather than widening `native_libs`: the harness needs this
/// one function, and the rest of the module's surface is the build's own.
#[cfg(test)]
pub(crate) fn native_libraries_for_test(
    ir: &mut ir::IrProject,
    manifest: &HashMap<String, JsonValue>,
    project_root: &Path,
) -> bool {
    native_libs::assemble_native_libraries_for_ir(ir, manifest, project_root)
}

/// Compile the dependencies a project declares by SOURCE DIRECTORY, the way the
/// front end above does before anything resolves a package, for a caller that
/// reads only their *interfaces*.
///
/// Every path that resolves a project must run this first. The resolver reads
/// each dependency's `.mfp` from `build/packages/`, and that cache is only as
/// fresh as the last build. A stale one (left by an older compiler) fails
/// resolution with `IMPORT_PACKAGE_INVALID`, while `mfb build` on the same project
/// succeeds because it rebuilds the cache first. Callers: `mfb doc`, and
/// `testutil::fixture_project` (a fixture whose dependency has no committed
/// `.mfp` otherwise resolved only when some earlier build left one behind).
///
/// Host target, default level, quiet, unsigned allowed: the resulting `.mfp` is
/// read for its interface (`external_package_function_types_from_files`,
/// `imported_type_defs_from_files`, the resolver's import check), never handed
/// to a backend.
pub(crate) fn build_source_dependency_interfaces(
    location: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Result<(), ()> {
    source_packages::build_source_dependencies(
        &BuildOptions {
            location: location.to_path_buf(),
            outputs: Vec::new(),
            package_output_dir: None,
            target: target::BuildTarget::host(),
            sign_owner: None,
            app_mode: false,
            app_debug: false,
            debug: false,
            // Only the dependencies' interfaces are read; no config folds.
            config: None,
            opt: crate::optimizer::OptLevel::default(),
            allow_unsigned: true,
            mode: crate::testing::CompileMode::Build,
            verbosity: Verbosity::Quiet,
        },
        manifest,
    )
}

mod native_libs;
mod options;
mod packages;
mod resources;
mod signing;
mod source_packages;
mod test_mode;

use native_libs::*;
use packages::*;
use resources::*;
use signing::*;
use source_packages::*;
use test_mode::*;

pub(crate) use options::{parse_build_options, parse_test_options};
pub(crate) use packages::{classify_installed_package, PackageVerification};

#[cfg(test)]
mod tests;
