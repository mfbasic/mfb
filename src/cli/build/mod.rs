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
    imported_resource_closers, imported_type_defs, imported_type_defs_from_files,
    installed_package_files, package_metadata,
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
            eprintln!(
                "error: app mode requires a macOS, Linux, or Windows target (got {})",
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
        resolver::augment_project(&ast)?
    };
    // plan-102-D3: elaborate ABOVE monomorph, then monomorphize the generic HIR
    // into concrete HIR, which `ir::lower_augmented_project` consumes directly.
    let generic_hir = {
        let _span = crate::trace::span("elaborate");
        crate::hir::elaborate(&augmented)
    };
    crate::trace::count("HIR functions (generic)", hir_function_count(&generic_hir));
    let concrete_hir = {
        let _span = crate::trace::span("monomorphize");
        monomorph::monomorphize_project(&options.location, &generic_hir)?
    };
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
    let imported_resources = imported_resource_closers(&options.location, &manifest);
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
    let imported_types = imported_type_defs(&options.location, &manifest);
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
                let path = match writer(&options.location, ir, &target, packages, build_mode) {
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

    /// plan-42: `--target`/`--app` are the documented spellings; the
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

        // `--regalloc` was removed with the bump oracle; both spellings now
        // land in the unknown-option arm like any other retired flag.
        assert!(parse_test_options(s(&["--regalloc", "bump"])).is_err());
        assert!(parse_test_options(s(&["-regalloc=bump"])).is_err());

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
        // Repeating the same flag is not a conflict. `-q` is idempotent; `-v`
        // escalates to the compile profiler (see
        // `parse_build_options_repeated_verbose_selects_trace`) — plan-36
        // specified only that repeating must not error, not what a second `-v`
        // means, and the second one now means Trace.
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
            Verbosity::Trace
        );
    }

    /// `-vv`, `-v -v` and `--verbose --verbose` all select the compile
    /// profiler, and a third flag is a no-op (nothing sits above Trace).
    #[test]
    fn parse_build_options_repeated_verbose_selects_trace() {
        for args in [
            &["-vv"][..],
            &["-v", "-v"][..],
            &["--verbose", "--verbose"][..],
            &["-v", "--verbose"][..],
            &["-v", "-v", "-v"][..],
            &["-vv", "-v"][..],
        ] {
            let options = parse_build_options(s(args)).expect("trace options");
            assert_eq!(options.verbosity, Verbosity::Trace, "args {args:?}");
        }
        // Ordered least-to-most verbose, which is what every `>=` gate in
        // `Reporter` relies on to make each level a superset of the one below.
        assert!(Verbosity::Trace > Verbosity::Verbose);
        assert!(Verbosity::Verbose > Verbosity::Normal);
        assert!(Verbosity::Normal > Verbosity::Quiet);
    }

    /// `-q` conflicts with the bundled `-vv` exactly as it does with `-v`.
    #[test]
    fn parse_build_options_quiet_and_trace_conflict() {
        for args in [
            &["-q", "-vv"][..],
            &["-vv", "-q"][..],
            &["-q", "-v", "-v"][..],
        ] {
            let err = build_err(args);
            assert!(
                err.contains("at most one of -q / -v"),
                "unexpected error for {args:?}: {err}"
            );
        }
    }

    #[test]
    fn parse_test_options_is_quiet() {
        // `mfb test` never prints the build summary (it would churn the
        // non-portable `.testrun` goldens); see plan-36.
        let options = parse_test_options(vec![]).expect("test options");
        assert_eq!(options.verbosity, Verbosity::Quiet);
    }

    #[test]
    fn parse_test_options_verbose_both_spellings() {
        for flag in ["-v", "--verbose"] {
            let options = parse_test_options(s(&[flag])).expect("verbose test options");
            assert_eq!(options.verbosity, Verbosity::Verbose, "flag {flag}");
            // The flag changes nothing else about the run.
            assert_eq!(
                options.mode,
                crate::testing::CompileMode::Test { coverage: false }
            );
            assert_eq!(options.location, PathBuf::from("."));
        }
        // Repeating it is not an error, and it composes with the other flags.
        // As on `mfb build`, the second `-v` escalates to the compile profiler.
        let options =
            parse_test_options(s(&["-v", "--verbose", "--coverage", "proj"])).expect("options");
        assert_eq!(options.verbosity, Verbosity::Trace);
        assert_eq!(
            options.mode,
            crate::testing::CompileMode::Test { coverage: true }
        );
        assert_eq!(options.location, PathBuf::from("proj"));
    }

    /// `mfb test -vv` reaches the same profiler `mfb build -vv` does, from a
    /// Quiet baseline rather than a Normal one.
    #[test]
    fn parse_test_options_repeated_verbose_selects_trace() {
        for args in [&["-vv"][..], &["-v", "-v"][..], &["--verbose", "-vv"][..]] {
            let options = parse_test_options(s(args)).expect("trace test options");
            assert_eq!(options.verbosity, Verbosity::Trace, "args {args:?}");
        }
    }

    /// `mfb test` builds quietly by default, so `-q` would be a no-op flag; it
    /// stays unknown rather than becoming silently accepted noise.
    #[test]
    fn parse_test_options_rejects_quiet() {
        for flag in ["-q", "--quiet"] {
            let err = match parse_test_options(s(&[flag])) {
                Ok(_) => panic!("expected `{flag}` to be rejected by mfb test"),
                Err(message) => message,
            };
            assert!(err.contains("unknown test option"), "flag {flag}: {err}");
        }
    }

    #[test]
    fn parse_build_options_regalloc_both_forms_and_bad_value() {
        assert!(parse_build_options(s(&["--regalloc"])).is_err());
        assert!(parse_build_options(s(&["--regalloc", "not-a-strategy"])).is_err());
        assert!(parse_build_options(s(&["--regalloc=not-a-strategy"])).is_err());
    }

    /// plan-100 §2: every spelling of the `-O` dial parses to the same
    /// [`crate::optimizer::OptLevel`], and the flagless build is `-O1` -- the
    /// level at which the shipping Level-1 passes run, i.e. today's codegen.
    #[test]
    fn parse_build_options_opt_level_every_spelling() {
        use crate::optimizer::OptLevel;

        for (args, want) in [
            (s(&["-O0"]), OptLevel(0)),
            (s(&["-O", "0"]), OptLevel(0)),
            (s(&["-O=0"]), OptLevel(0)),
            (s(&["--optimize=0"]), OptLevel(0)),
            (s(&["--optimize", "0"]), OptLevel(0)),
            (s(&["-optimize=0"]), OptLevel(0)),
            (s(&["-optimize", "0"]), OptLevel(0)),
            (s(&["-O1"]), OptLevel(1)),
            (s(&["-O", "1"]), OptLevel(1)),
            (s(&["-O=1"]), OptLevel(1)),
            (s(&["--optimize=1"]), OptLevel(1)),
            (s(&["--optimize", "1"]), OptLevel(1)),
            (s(&["-optimize=1"]), OptLevel(1)),
            (s(&["-optimize", "1"]), OptLevel(1)),
        ] {
            let spelling = format!("{args:?}");
            let parsed = parse_build_options(args).expect(&spelling);
            assert_eq!(parsed.opt, want, "{spelling}");
        }

        // The whole point of the dial: absent the flag the default is `-O1`,
        // not `-O0`, so default codegen is unchanged from before plan-100.
        assert_eq!(
            parse_build_options(vec![]).expect("flagless").opt,
            OptLevel(1)
        );
        assert_eq!(
            parse_test_options(vec![]).expect("flagless test").opt,
            OptLevel(1)
        );

        // `mfb test` takes the dial too, in every spelling `mfb build` does.
        for (args, want) in [
            (s(&["-O0"]), OptLevel(0)),
            (s(&["-O", "0"]), OptLevel(0)),
            (s(&["--optimize=0"]), OptLevel(0)),
            (s(&["-optimize", "1"]), OptLevel(1)),
        ] {
            let spelling = format!("{args:?}");
            let parsed = parse_test_options(args).expect(&spelling);
            assert_eq!(parsed.opt, want, "{spelling}");
        }
    }

    /// Levels the scaffold does not implement yet, and malformed values, error
    /// the same way `--regalloc bogus` does rather than silently defaulting.
    #[test]
    fn parse_build_options_opt_level_rejects_unlanded_and_malformed() {
        // `-O` with nothing after it.
        assert!(parse_build_options(s(&["-O"])).is_err());
        assert!(parse_test_options(s(&["--optimize"])).is_err());

        // Landed levels parse in the attached spelling too (2 and 3 opened
        // with the DCE/ADCE rows).
        use crate::optimizer::OptLevel;
        for (level, want) in [("-O2", OptLevel(2)), ("-O3", OptLevel(3))] {
            let parsed = parse_build_options(s(&[level])).expect(level);
            assert_eq!(parsed.opt, want, "{level}");
        }

        for bogus in ["-O4", "-O6", "-Ox", "-O=9", "--optimize=4", "-optimize=z"] {
            let message = build_err(&[bogus]);
            assert!(
                message.contains("available: 0, 1, 2, 3"),
                "{bogus} -> {message}"
            );
            assert!(parse_test_options(s(&[bogus])).is_err(), "{bogus}");
        }

        // The space form reports the same list.
        let message = build_err(&["-O", "5"]);
        assert!(message.contains("available: 0, 1, 2, 3"), "{message}");
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

    fn copy_test_project(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).expect("copy destination");
        for entry in std::fs::read_dir(from).expect("read fixture directory") {
            let entry = entry.expect("fixture entry");
            let source = entry.path();
            let destination = to.join(entry.file_name());
            if source.is_dir() {
                if entry.file_name() == "golden" || entry.file_name() == "build" {
                    continue;
                }
                copy_test_project(&source, &destination);
            } else {
                std::fs::copy(&source, &destination).expect("copy fixture file");
            }
        }
    }

    /// Keep every compile-only byte-identity corpus in-process so cargo-llvm-cov
    /// credits the clean-room `gen_*` files. The artifact gate runs these same
    /// projects through a child compiler, whose profile cargo-llvm-cov does not
    /// associate with the test target's object list. Targets are derived from the
    /// committed ncodesum names, matching artifact-gate.sh rather than widening a
    /// fixture onto a backend it intentionally does not support.
    #[test]
    fn builtin_codegen_corpora_lower_in_process() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/byte-identity");
        for package in std::fs::read_dir(&root).expect("byte-identity root") {
            let fixture = package.expect("package directory").path();
            if !fixture.join("project.json").is_file() {
                continue;
            }
            let dir = tempfile::tempdir().expect("temp dir");
            copy_test_project(&fixture, dir.path());
            let project = dir.path().to_str().expect("utf8 temp path");
            let golden = fixture.join("golden");
            for target in crate::target::registered_targets() {
                let target_name = target.name();
                let has_golden = std::fs::read_dir(&golden)
                    .expect("fixture goldens")
                    .filter_map(Result::ok)
                    .any(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .ends_with(&format!(".{target_name}.ncodesum"))
                    });
                if !has_golden {
                    continue;
                }
                let options = parse_build_options(s(&["-ncode", "-target", &target_name, project]))
                    .expect("options");
                build_project(&options)
                    .unwrap_or_else(|()| panic!("{} lowers for {target_name}", fixture.display()));
            }
        }
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

    /// bug-503 (audit-3 LNK-12): a `project.json` `name` of `../evil` used to be
    /// `Path::join`ed into `build/../evil.out` and written 0755 — an arbitrary
    /// executable write from merely *building* an untrusted project. The build
    /// must refuse the name and write nothing outside the project.
    #[test]
    fn build_project_rejects_a_traversing_project_name() {
        let root = tempfile::tempdir().expect("temp dir");
        let dir = root.path().join("proj");
        std::fs::create_dir_all(&dir).expect("project dir");
        write_executable_project(&dir);
        let manifest = std::fs::read_to_string(dir.join("project.json")).expect("manifest");
        std::fs::write(
            dir.join("project.json"),
            manifest.replace("\"name\": \"app\"", "\"name\": \"../evil\""),
        )
        .expect("rewrite manifest");
        let options =
            parse_build_options(vec![dir.to_str().unwrap().to_string()]).expect("options");
        assert!(
            build_project(&options).is_err(),
            "a traversing project name must fail the build"
        );
        // `proj/build/../evil.out` is `<root>/evil.out`; `-ast` would land
        // `<root>/evil.ast`. Neither — nor anything else — may appear beside the
        // project.
        let escaped: Vec<_> = std::fs::read_dir(root.path())
            .expect("read root")
            .map(|entry| entry.expect("entry").file_name())
            .filter(|name| name != "proj")
            .collect();
        assert!(escaped.is_empty(), "files escaped the project: {escaped:?}");
        assert!(
            !dir.join(crate::os::BUILD_DIR).exists(),
            "a refused build must not create build/"
        );
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
                // Required for `kind: "package"` since plan-61-F. Without it
                // this helper's manifest fails validation — which would also
                // make `build_project_rejects_native_output_for_a_package`
                // pass for the wrong reason, since it only asserts `is_err()`.
                "  \"description\": \"Test fixture package for the build CLI.\",\n",
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

    fn resolved(unit: &str, source: &str) -> crate::codegen::link::locator::ResolvedLibrary {
        let locator = vendor_locator(source);
        crate::codegen::link::locator::ResolvedLibrary {
            dlopen_name: crate::codegen::link::locator::dlopen_name(&locator, unit),
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
    fn write_vendor_source(
        root: &Path,
        library: &crate::codegen::link::locator::ResolvedLibrary,
        bytes: &[u8],
    ) {
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

    #[test]
    fn parse_build_options_defaults_to_console_mode() {
        let options = parse_build_options(vec!["some/project".to_string()]).expect("options");
        assert!(!options.app_mode);
    }

    #[test]
    fn parse_build_options_accepts_app_flag() {
        let options = parse_build_options(vec!["--app".to_string(), "some/project".to_string()])
            .expect("options");
        assert!(options.app_mode);
    }

    #[test]
    fn parse_build_options_rejects_duplicate_app_flag() {
        let result = parse_build_options(vec!["--app".to_string(), "--app".to_string()]);
        match result {
            Err(err) => assert!(err.contains("at most one -app")),
            Ok(_) => panic!("duplicate --app must be rejected"),
        }
    }

    #[test]
    fn parse_build_options_app_flag_composes_with_native_output() {
        let options =
            parse_build_options(vec!["--app".to_string(), "--nir".to_string()]).expect("options");
        assert!(options.app_mode);
        assert_eq!(options.outputs, vec![BuildOutput::NativeIr]);
    }

    #[test]
    fn parse_build_options_combines_output_flags_in_order() {
        let options = parse_build_options(vec![
            "--ast".to_string(),
            "--ir".to_string(),
            "--ncode".to_string(),
            "--mir".to_string(),
            "some/project".to_string(),
        ])
        .expect("options");
        assert_eq!(
            options.outputs,
            vec![
                BuildOutput::Ast,
                BuildOutput::Ir,
                BuildOutput::NativeCodePlan,
                BuildOutput::Mir,
            ]
        );
    }

    /// plan-42: the single-dash spellings stay working, undocumented aliases —
    /// a mixed-spelling command line parses exactly like the `--` one.
    #[test]
    fn parse_build_options_accepts_single_dash_aliases() {
        let options = parse_build_options(vec![
            "-app".to_string(),
            "-ast".to_string(),
            "--ir".to_string(),
            "-mir".to_string(),
            "some/project".to_string(),
        ])
        .expect("options");
        assert!(options.app_mode);
        assert_eq!(
            options.outputs,
            vec![BuildOutput::Ast, BuildOutput::Ir, BuildOutput::Mir]
        );
    }

    #[test]
    fn parse_build_options_rejects_duplicate_output_flag() {
        let result = parse_build_options(vec!["--ncode".to_string(), "--ncode".to_string()]);
        match result {
            Err(err) => assert!(err.contains("duplicate output flag `--ncode`")),
            Ok(_) => panic!("duplicate output flag must be rejected"),
        }
        // The duplicate check is per-output, not per-spelling: `-ncode --ncode`
        // is the same flag twice.
        let mixed = parse_build_options(vec!["-ncode".to_string(), "--ncode".to_string()]);
        match mixed {
            Err(err) => assert!(err.contains("duplicate output flag `--ncode`")),
            Ok(_) => panic!("mixed-spelling duplicate output flag must be rejected"),
        }
    }

    #[test]
    fn parse_build_options_no_output_flags_means_full_build() {
        let options = parse_build_options(vec!["some/project".to_string()]).expect("options");
        assert!(options.outputs.is_empty());
    }

    // Some CI environments run as root, where a `0o000` permission is ignored
    // (root bypasses the check), so a permission-denied test would spuriously
    // succeed at the very operation it means to fail. Probe once and skip those
    // tests there rather than asserting a failure that cannot happen.
    #[cfg(unix)]
    fn running_as_root() -> bool {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let probe = dir.path().join("probe");
        std::fs::write(&probe, b"x").unwrap();
        let mut perm = std::fs::metadata(&probe).unwrap().permissions();
        perm.set_mode(0o000);
        std::fs::set_permissions(&probe, perm).unwrap();
        // A 0o000 file is still readable by root.
        std::fs::read(&probe).is_ok()
    }

    // ---- test_mode.rs: run_test_binary + generate_coverage_report (plan-68-B B1) ----

    /// `run_test_binary` maps the child's exit status onto `mfb test`'s result:
    /// success passes, a non-zero exit fails, and a failure to even spawn fails.
    #[test]
    #[cfg(unix)]
    fn run_test_binary_maps_every_exit_status() {
        // Exit 0 -> Ok (the success arm).
        assert!(run_test_binary(Path::new("/usr/bin/true"), None).is_ok());
        // Non-zero exit -> Err (the `Ok(_) => Err(())` arm).
        assert!(run_test_binary(Path::new("/usr/bin/false"), None).is_err());
        // Spawn failure on a nonexistent path -> Err (the `Err(err)` arm).
        assert!(run_test_binary(Path::new("/no/such/binary-xyzzy-42"), None).is_err());
    }

    /// An empty project directory has no `coverage.covmap.json`, so `read_covmap`
    /// is `None`: the report warns and returns without writing `coverage.html`.
    #[test]
    fn generate_coverage_report_warns_when_the_covmap_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        generate_coverage_report(dir.path());
        assert!(!dir.path().join(crate::testing::COVERAGE_HTML).exists());
    }

    /// With a real covmap seeded (as a `--coverage` build writes), `read_covmap`
    /// is `Some`, so `generate_html` and the write arm run and produce the HTML.
    #[test]
    fn generate_coverage_report_writes_html_from_a_seeded_covmap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.mfb"), "PRINT 1\n").unwrap();
        crate::testing::coverage::write_covmap(
            &dir.path().join(crate::testing::COVMAP_FILE),
            &[crate::testing::coverage::CovSlot {
                file: "main.mfb".to_string(),
                line: 1,
            }],
        )
        .unwrap();
        generate_coverage_report(dir.path());
        assert!(dir.path().join(crate::testing::COVERAGE_HTML).is_file());
    }

    // ---- resources.rs: copy_resources remaining branches (plan-68-B B3) ----

    /// A root-level glob (`*.png`) has an empty fixed prefix, so `copy_resources`
    /// walks the project root itself and strips no prefix from the destination.
    #[test]
    fn copy_resources_handles_a_root_level_glob() {
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        std::fs::write(root.join("banner.png"), b"img").unwrap();
        std::fs::write(root.join("notes.txt"), b"skip").unwrap();
        let out = tempfile::tempdir().unwrap();
        let entries = vec![crate::manifest::ResourceEntry {
            src: "*.png".to_string(),
            dst: String::new(),
        }];
        copy_resources(root, &entries, out.path()).expect("root glob copies");
        assert!(out.path().join("banner.png").is_file());
        // A non-matching root file is left behind.
        assert!(!out.path().join("notes.txt").exists());
    }

    /// An unreadable resource source directory is a hard scan error, not a silent
    /// skip (the direct `read_dir` failure inside `collect_files_recursive`).
    #[test]
    #[cfg(unix)]
    fn copy_resources_reports_an_unreadable_source_directory() {
        use std::os::unix::fs::PermissionsExt;
        if running_as_root() {
            return;
        }
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("x.ogg"), b"a").unwrap();
        let mut perm = std::fs::metadata(&data).unwrap().permissions();
        perm.set_mode(0o000);
        std::fs::set_permissions(&data, perm).unwrap();
        let out = tempfile::tempdir().unwrap();
        let entries = vec![crate::manifest::ResourceEntry {
            src: "data/*.ogg".to_string(),
            dst: "m/".to_string(),
        }];
        let err = copy_resources(root, &entries, out.path())
            .expect_err("an unreadable source directory must error");
        // Restore so the tempdir can be cleaned up.
        let mut perm = std::fs::metadata(&data).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&data, perm).unwrap();
        assert!(err.contains("failed to scan resources"), "{err}");
    }

    /// An unreadable subdirectory encountered while walking a resource root is a
    /// hard scan error (`collect_files_recursive`'s propagated `read_dir` error).
    #[test]
    #[cfg(unix)]
    fn copy_resources_reports_an_unreadable_subdirectory() {
        use std::os::unix::fs::PermissionsExt;
        if running_as_root() {
            return;
        }
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        let data = root.join("data");
        let sub = data.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(data.join("top.ogg"), b"a").unwrap();
        std::fs::write(sub.join("deep.ogg"), b"b").unwrap();
        let mut perm = std::fs::metadata(&sub).unwrap().permissions();
        perm.set_mode(0o000);
        std::fs::set_permissions(&sub, perm).unwrap();
        let out = tempfile::tempdir().unwrap();
        let entries = vec![crate::manifest::ResourceEntry {
            src: "data/**/*.ogg".to_string(),
            dst: "all/".to_string(),
        }];
        let err = copy_resources(root, &entries, out.path())
            .expect_err("an unreadable subdirectory must error");
        let mut perm = std::fs::metadata(&sub).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&sub, perm).unwrap();
        assert!(err.contains("failed to scan resources"), "{err}");
    }

    /// A destination directory that cannot be created (unwritable resource dir)
    /// is a hard error (the `create_dir_all` arm).
    #[test]
    #[cfg(unix)]
    fn copy_resources_reports_an_uncreatable_destination() {
        use std::os::unix::fs::PermissionsExt;
        if running_as_root() {
            return;
        }
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("x.ogg"), b"a").unwrap();
        let out = tempfile::tempdir().unwrap();
        let mut perm = std::fs::metadata(out.path()).unwrap().permissions();
        perm.set_mode(0o500);
        std::fs::set_permissions(out.path(), perm).unwrap();
        let entries = vec![crate::manifest::ResourceEntry {
            src: "data/*.ogg".to_string(),
            dst: "sub/".to_string(),
        }];
        let err = copy_resources(root, &entries, out.path())
            .expect_err("an uncreatable destination must error");
        let mut perm = std::fs::metadata(out.path()).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(out.path(), perm).unwrap();
        assert!(err.contains("failed to create"), "{err}");
    }

    /// A matched source file that cannot be read is a hard copy error (the
    /// `fs::copy` arm), distinct from the create-directory arm above.
    #[test]
    #[cfg(unix)]
    fn copy_resources_reports_an_unreadable_source_file() {
        use std::os::unix::fs::PermissionsExt;
        if running_as_root() {
            return;
        }
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        let data = root.join("data");
        std::fs::create_dir_all(&data).unwrap();
        let file = data.join("x.ogg");
        std::fs::write(&file, b"a").unwrap();
        let mut perm = std::fs::metadata(&file).unwrap().permissions();
        perm.set_mode(0o000);
        std::fs::set_permissions(&file, perm).unwrap();
        let out = tempfile::tempdir().unwrap();
        let entries = vec![crate::manifest::ResourceEntry {
            src: "data/*.ogg".to_string(),
            dst: "m/".to_string(),
        }];
        let err = copy_resources(root, &entries, out.path())
            .expect_err("an unreadable source file must error");
        let mut perm = std::fs::metadata(&file).unwrap().permissions();
        perm.set_mode(0o644);
        std::fs::set_permissions(&file, perm).unwrap();
        assert!(err.contains("failed to copy resource"), "{err}");
    }

    // ---- native_libs.rs: verify / vendor-source / copy helpers (plan-68-B B4) ----

    /// `verify_vendor_libraries` fails, naming the missing file, when a resolved
    /// vendor blob is absent from disk.
    #[test]
    fn verify_vendor_libraries_reports_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let library = resolved("sqlite3", "libfoo.so");
        // No source written -> sha256_file errs -> NATIVE_LIBRARY_FILE_MISSING.
        assert!(!verify_vendor_libraries(
            std::slice::from_ref(&library),
            dir.path(),
            OWN_UNIT
        ));
    }

    /// A vendor locator that records no hash is a malformed package: the verify
    /// fails on the "records no hash" arm even though the file is present.
    #[test]
    fn verify_vendor_libraries_rejects_a_locator_without_a_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut library = resolved("sqlite3", "libfoo.so");
        library.locator.hash = None;
        write_vendor_source(dir.path(), &library, b"bytes");
        assert!(!verify_vendor_libraries(
            std::slice::from_ref(&library),
            dir.path(),
            OWN_UNIT
        ));
    }

    /// A present file whose bytes hash to something other than the recorded
    /// sha256 is the wrong version of the library and is rejected.
    #[test]
    fn verify_vendor_libraries_rejects_a_hash_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        // `resolved` records hash [1u8; 32]; real bytes hash to something else.
        let library = resolved("sqlite3", "libfoo.so");
        write_vendor_source(dir.path(), &library, b"totally different bytes");
        assert!(!verify_vendor_libraries(
            std::slice::from_ref(&library),
            dir.path(),
            OWN_UNIT
        ));
    }

    /// A file whose bytes hash exactly to the recorded sha256 verifies.
    #[test]
    fn verify_vendor_libraries_accepts_a_matching_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut library = resolved("sqlite3", "libfoo.so");
        write_vendor_source(dir.path(), &library, b"the real bytes");
        let path = vendor_source_path(dir.path(), OWN_UNIT, &library);
        library.locator.hash = Some(crate::manifest::libraries::sha256_file(&path).unwrap());
        assert!(verify_vendor_libraries(
            std::slice::from_ref(&library),
            dir.path(),
            OWN_UNIT
        ));
    }

    /// A locator declared by the project's own unit reads from `<root>/vendor/`;
    /// an imported unit's reads from `<root>/packages/<unit>.vendor/`.
    #[test]
    fn vendor_source_path_distinguishes_own_and_imported_units() {
        let root = Path::new("/proj");
        let own = resolved(OWN_UNIT, "libbar.so");
        assert_eq!(
            vendor_source_path(root, OWN_UNIT, &own),
            crate::manifest::libraries::vendor_path(root, "libbar.so"),
        );
        let imported = resolved("other", "libbar.so");
        assert_eq!(
            vendor_source_path(root, OWN_UNIT, &imported),
            crate::manifest::libraries::imported_vendor_path(root, "other", "libbar.so"),
        );
    }

    /// `copy_vendor_libraries` surfaces the copy error when a resolved vendor
    /// source file does not exist on disk (the `fs::copy` failure arm).
    #[test]
    fn copy_vendor_libraries_errors_when_the_source_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let library = resolved("sqlite3", "libfoo.so");
        // Deliberately do NOT write the source file.
        let out = dir.path().join("out");
        let err = copy_vendor_libraries(
            std::slice::from_ref(&library),
            dir.path(),
            OWN_UNIT,
            std::slice::from_ref(&out),
        )
        .expect_err("a missing source must fail the copy");
        assert!(err.contains("failed to copy vendored library"), "{err}");
    }

    // ---- packages.rs: the §3.5 signed-package chain (plan-68-B B2) ----
    //
    // `classify_installed_package` performs no network I/O: after the header
    // identKey check it verifies the attestation/proof/signature/payload chain
    // purely over the `.mfp` bytes plus a locally *pinned* server key. So the
    // whole chain is unit-coverable with a hermetic, self-consistent signed
    // fixture built here (the ident key signs the proof, a throwaway "server" key
    // signs the attestation, and the one-off signing key signs the container) and
    // that server key pinned under an empty `MFB_HOME`.

    #[derive(Clone, Copy)]
    enum SignedTamper {
        /// A fully valid chain (verifies to `Verified`).
        None,
        /// The header `identKey` is not decodable base64url.
        MalformedHeaderIdent,
        /// The attestation signature does not verify under the server key.
        AttestationSig,
        /// The proof signature does not verify under the ident key.
        ProofSig,
        /// The container is signed by a key other than the advertised signingKey.
        SignatureMismatch,
        /// A payload byte is flipped after the signed prefix (hash weld breaks).
        Payload,
    }

    struct SignedFixture {
        bytes: Vec<u8>,
        /// The ident-key trust anchor (`ed25519:<base64url>`) a project would pin.
        ident_key: String,
        /// The throwaway server public key to pin so the attestation verifies.
        server_public: Vec<u8>,
    }

    fn build_signed_fixture(ident: &str, version: &str, tamper: SignedTamper) -> SignedFixture {
        use mfb_repository::crypto;
        let (ident_public, ident_private) = crypto::generate_keypair();
        let (signing_public, signing_private) = crypto::generate_keypair();
        let (server_public, server_private) = crypto::generate_keypair();
        let ident_fingerprint = crypto::fingerprint(&ident_public);
        let signing_fingerprint = crypto::fingerprint(&signing_public);
        let repo_fingerprint = crypto::fingerprint(&server_public);
        let (owner, name) = ident.split_once('#').expect("ident is <owner>#<name>");

        let proof = format!(
            "{{\"owner\":\"{owner}\",\"ident\":\"{ident}\",\"version\":\"{version}\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}"
        );
        let mut proof_sig = crypto::sign(
            &ident_private,
            &crypto::proof_signing_input(proof.as_bytes()),
        )
        .unwrap();
        let attestation = format!(
            "{{\"repoFingerprint\":\"{repo_fingerprint}\",\"owner\":\"{owner}\",\"ident\":\"{ident}\",\"version\":\"{version}\",\"identFingerprint\":\"{ident_fingerprint}\",\"signingFingerprint\":\"{signing_fingerprint}\"}}"
        );
        let mut attestation_sig = crypto::sign(
            &server_private,
            &crypto::attestation_signing_input(attestation.as_bytes()),
        )
        .unwrap();

        // The header identKey (used as the trust anchor and to derive
        // identFingerprint); malformed only for that specific tamper.
        let header_ident_key = if matches!(tamper, SignedTamper::MalformedHeaderIdent) {
            "ed25519:not-valid-base64url-$$$".to_string()
        } else {
            format!("ed25519:{}", crypto::encode_bytes(&ident_public))
        };
        // The container is normally signed by `signing_private`; for the
        // signature-mismatch tamper it is signed by an UNRELATED key while the
        // advertised signingKey (and thus every fingerprint) stays consistent, so
        // the proof/attestation still verify and only the container signature
        // fails.
        let container_private = if matches!(tamper, SignedTamper::SignatureMismatch) {
            crypto::generate_keypair().1
        } else {
            signing_private
        };
        if matches!(tamper, SignedTamper::ProofSig) {
            proof_sig[0] ^= 0xff;
        }
        if matches!(tamper, SignedTamper::AttestationSig) {
            attestation_sig[0] ^= 0xff;
        }

        let mut metadata =
            binary_repr::BinaryReprMetadata::new(name.to_string(), version.to_string());
        metadata.ident = ident.to_string();
        metadata.author = owner.to_string();

        let signing = target::package_mfp::PackageSigning {
            ident_key: header_ident_key.clone(),
            signing_key: format!("ed25519:{}", crypto::encode_bytes(&signing_public)),
            signing_private: container_private,
            proof,
            proof_sig,
            attestation,
            attestation_sig,
        };
        let payload = b"MFPCsigned-fixture-payload".to_vec();
        let mut bytes =
            target::package_mfp::build_package_bytes(&metadata, &payload, Some(&signing)).unwrap();
        if matches!(tamper, SignedTamper::Payload) {
            // Flip the final payload byte: after the signed prefix, so the
            // container signature still verifies but the payload-hash weld breaks.
            let last = bytes.len() - 1;
            bytes[last] ^= 0xff;
        }
        SignedFixture {
            bytes,
            ident_key: header_ident_key,
            server_public,
        }
    }

    /// Write a fixture's bytes to a `.mfp` under a tempdir and return the path.
    fn write_mfp(dir: &Path, bytes: &[u8]) -> PathBuf {
        let path = dir.join("pkg.mfp");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// A signed package with no pinned trust anchor is untrusted (the
    /// file-embedded key is attacker-controlled).
    #[test]
    fn classify_signed_package_without_a_trust_anchor_is_untrusted() {
        let dir = tempfile::tempdir().unwrap();
        let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::None);
        let path = write_mfp(dir.path(), &fx.bytes);
        let classification = classify_installed_package(&path, None);
        assert_eq!(classification.state, PackageVerification::Tampered);
        assert_eq!(
            classification.refusal.expect("refusal").0,
            "PACKAGE_IDENT_KEY_UNTRUSTED"
        );
    }

    /// A malformed pinned trust anchor is rejected before any file key is read.
    #[test]
    fn classify_signed_package_with_a_malformed_anchor_is_untrusted() {
        let dir = tempfile::tempdir().unwrap();
        let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::None);
        let path = write_mfp(dir.path(), &fx.bytes);
        let classification = classify_installed_package(&path, Some("not-base64!"));
        assert_eq!(classification.state, PackageVerification::Tampered);
        assert_eq!(
            classification.refusal.expect("refusal").0,
            "PACKAGE_IDENT_KEY_UNTRUSTED"
        );
    }

    /// A malformed header identKey (present but not decodable) is untrusted.
    #[test]
    fn classify_signed_package_with_a_malformed_header_ident_is_untrusted() {
        let dir = tempfile::tempdir().unwrap();
        let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::MalformedHeaderIdent);
        let path = write_mfp(dir.path(), &fx.bytes);
        // A well-formed anchor so decode passes; the header key is the malformed one.
        let anchor = format!(
            "ed25519:{}",
            mfb_repository::crypto::encode_bytes(&[7u8; 32])
        );
        let classification = classify_installed_package(&path, Some(&anchor));
        assert_eq!(classification.state, PackageVerification::Tampered);
        assert_eq!(
            classification.refusal.expect("refusal").0,
            "PACKAGE_IDENT_KEY_UNTRUSTED"
        );
    }

    /// A well-formed anchor that is not the package's own header key is untrusted
    /// (the header-identKey-≠-pinned arm).
    #[test]
    fn classify_signed_package_with_a_mismatched_anchor_is_untrusted() {
        let dir = tempfile::tempdir().unwrap();
        let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::None);
        let path = write_mfp(dir.path(), &fx.bytes);
        // A valid but wrong ed25519 key (32 bytes, not the fixture's ident).
        let anchor = format!(
            "ed25519:{}",
            mfb_repository::crypto::encode_bytes(&[9u8; 32])
        );
        let classification = classify_installed_package(&path, Some(&anchor));
        assert_eq!(classification.state, PackageVerification::Tampered);
        let (rule, detail) = classification.refusal.expect("refusal");
        assert_eq!(rule, "PACKAGE_IDENT_KEY_UNTRUSTED");
        assert!(detail.contains("does not match"), "{detail}");
    }

    /// With the correct anchor but NO pinned registry key on the machine, the
    /// attestation cannot be checked — the reachable frontier without a registry.
    #[test]
    fn classify_signed_package_without_a_pinned_server_key_is_untrusted() {
        let _lock = crate::cli::tests::ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _mfb = crate::cli::tests::EnvVarGuard::set("MFB_HOME", home.path().to_str().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::None);
        let path = write_mfp(dir.path(), &fx.bytes);
        let classification = classify_installed_package(&path, Some(&fx.ident_key));
        assert_eq!(classification.state, PackageVerification::Tampered);
        let (rule, detail) = classification.refusal.expect("refusal");
        assert_eq!(rule, "PACKAGE_ATTESTATION_INVALID");
        assert!(detail.contains("no pinned registry key"), "{detail}");
    }

    /// Pin the server key and classify each chain-link tamper on its own arm, then
    /// the fully valid package to `Verified`.
    fn classify_pinned(fx: &SignedFixture) -> PackageClassification {
        let _lock = crate::cli::tests::ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _mfb = crate::cli::tests::EnvVarGuard::set("MFB_HOME", home.path().to_str().unwrap());
        let _fp = crate::cli::tests::EnvVarGuard::unset("MFB_REPO_SERVER_FINGERPRINT");
        let repo_url = mfb_repository::client::repo_url_from_env();
        let paths = crate::cli::local_paths_for_repo(&repo_url).unwrap();
        mfb_repository::local::pin_server_key(&paths, &fx.server_public).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = write_mfp(dir.path(), &fx.bytes);
        classify_installed_package(&path, Some(&fx.ident_key))
    }

    #[test]
    fn classify_signed_package_with_a_bad_attestation_signature() {
        let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::AttestationSig);
        let classification = classify_pinned(&fx);
        assert_eq!(classification.state, PackageVerification::Tampered);
        assert_eq!(
            classification.refusal.expect("refusal").0,
            "PACKAGE_ATTESTATION_INVALID"
        );
    }

    #[test]
    fn classify_signed_package_with_a_bad_proof_signature() {
        let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::ProofSig);
        let classification = classify_pinned(&fx);
        assert_eq!(classification.state, PackageVerification::Tampered);
        assert_eq!(
            classification.refusal.expect("refusal").0,
            "PACKAGE_PROOF_INVALID"
        );
    }

    #[test]
    fn classify_signed_package_with_a_bad_container_signature() {
        let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::SignatureMismatch);
        let classification = classify_pinned(&fx);
        assert_eq!(classification.state, PackageVerification::Tampered);
        assert_eq!(
            classification.refusal.expect("refusal").0,
            "PACKAGE_SIGNATURE_INVALID"
        );
    }

    #[test]
    fn classify_signed_package_with_a_broken_payload_hash() {
        let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::Payload);
        let classification = classify_pinned(&fx);
        assert_eq!(classification.state, PackageVerification::Tampered);
        assert_eq!(
            classification.refusal.expect("refusal").0,
            "PACKAGE_PAYLOAD_HASH_MISMATCH"
        );
    }

    #[test]
    fn classify_fully_signed_package_verifies() {
        let fx = build_signed_fixture("ada#shape", "1.0.0", SignedTamper::None);
        let classification = classify_pinned(&fx);
        assert_eq!(classification.state, PackageVerification::Verified);
        assert!(classification.refusal.is_none());
    }

    // ---- verify_and_report_packages: entry-shape + Verified/Tampered arms ----

    /// A `packages` entry that is not an object, or an object without a `name`,
    /// is silently skipped rather than crashing the report.
    #[test]
    fn verify_and_report_skips_malformed_package_entries() {
        let manifest = crate::manifest::parse_project_json(
            concat!(
                "{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",",
                "\"sources\":[{\"root\":\"src\"}],",
                "\"packages\":[42, {\"version\":\"1.0.0\"}]}"
            ),
            Path::new("project.json"),
        )
        .expect("manifest");
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(verify_and_report_packages(dir.path(), &manifest, false).is_ok());
    }

    /// An installed, fully-verified dependency reports `[Verified]` and does not
    /// block the build.
    #[test]
    fn verify_and_report_accepts_a_verified_dependency() {
        let _lock = crate::cli::tests::ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _mfb = crate::cli::tests::EnvVarGuard::set("MFB_HOME", home.path().to_str().unwrap());
        let _fp = crate::cli::tests::EnvVarGuard::unset("MFB_REPO_SERVER_FINGERPRINT");
        let fx = build_signed_fixture("sec#signed", "0.1.0", SignedTamper::None);
        let repo_url = mfb_repository::client::repo_url_from_env();
        let paths = crate::cli::local_paths_for_repo(&repo_url).unwrap();
        mfb_repository::local::pin_server_key(&paths, &fx.server_public).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).unwrap();
        std::fs::write(packages.join("signed.mfp"), &fx.bytes).unwrap();
        let manifest = crate::manifest::parse_project_json(
            &format!(
                "{{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{{\"root\":\"src\"}}],\"packages\":[{{\"name\":\"signed\",\"ident\":\"sec#signed\",\"version\":\"0.1.0\",\"pin\":true,\"source\":\"sec#signed\",\"identKey\":\"{}\"}}]}}",
                fx.ident_key
            ),
            Path::new("project.json"),
        )
        .expect("manifest");
        assert!(verify_and_report_packages(dir.path(), &manifest, false).is_ok());
    }

    // ---- build_project: reachable validation/error branches (plan-68-B B6) ----

    /// Every `BuildOutput` label is the documented noun (drives the `Wrote …` and
    /// package-unsupported lines).
    #[test]
    fn build_output_label_names_every_variant() {
        assert_eq!(BuildOutput::Ast.label(), "AST");
        assert_eq!(BuildOutput::Ir.label(), "IR");
        assert_eq!(BuildOutput::BinaryRepr.label(), "binary representation");
        assert_eq!(BuildOutput::NativeIr.label(), "native IR");
        assert_eq!(BuildOutput::NativePlan.label(), "native plan");
        assert_eq!(BuildOutput::NativeObjectPlan.label(), "native object plan");
        assert_eq!(BuildOutput::NativeCodePlan.label(), "native code plan");
        assert_eq!(BuildOutput::Mir.label(), "MIR");
    }

    /// A verbose build runs the `phase …` reporter arms (level-gated stderr; no
    /// effect on the emitted bytes).
    #[test]
    fn build_project_verbose_runs_the_phase_reporter() {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        let options =
            parse_build_options(s(&["-v", dir.path().to_str().unwrap()])).expect("options");
        assert_eq!(options.verbosity, Verbosity::Verbose);
        build_project(&options).expect("verbose build succeeds");
    }

    /// A `-vv` build runs the whole compile profiler — every span opened deep in
    /// codegen, the leaderboards, the counters, and the render — and still
    /// produces the same artifact. The span stack is thread-local and each span
    /// records against its depth, so an unbalanced open/close would show up here
    /// as a panic or a hang rather than silently mis-filing later builds.
    #[test]
    fn build_project_trace_runs_the_compile_profiler() {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        let options =
            parse_build_options(s(&["-vv", dir.path().to_str().unwrap()])).expect("options");
        assert_eq!(options.verbosity, Verbosity::Trace);
        build_project(&options).expect("trace build succeeds");
    }

    /// App mode requires an executable project; a package with `--app` is rejected
    /// before any lowering.
    #[test]
    fn build_project_rejects_app_mode_for_a_package() {
        let dir = tempfile::tempdir().unwrap();
        write_package_project(dir.path());
        let options =
            parse_build_options(s(&["-app", dir.path().to_str().unwrap()])).expect("options");
        assert!(build_project(&options).is_err());
    }

    /// Write an app-mode executable manifest whose `icon` points at a file that
    /// does not exist, so the icon existence check fails.
    fn write_app_project_missing_icon(dir: &Path) {
        std::fs::write(
            dir.join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"executable\",\n",
                "  \"entry\": \"main\",\n",
                "  \"mode\": \"app\",\n",
                "  \"icon\": \"assets/missing.png\",\n",
                "  \"targets\": [\"native\"],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .expect("manifest");
        std::fs::create_dir_all(dir.join("src")).expect("src dir");
        std::fs::write(dir.join("src").join("main.mfb"), "SUB main()\nEND SUB\n").expect("source");
    }

    /// A missing `icon` is a hard error in app mode, on each app-capable target's
    /// build-mode arm (macOS host, and the cross Linux/Windows selectors).
    #[test]
    fn build_project_app_mode_missing_icon_is_rejected_per_target() {
        for target in [None, Some("linux-aarch64"), Some("windows-x86_64")] {
            let dir = tempfile::tempdir().unwrap();
            write_app_project_missing_icon(dir.path());
            let mut args = vec!["-app".to_string()];
            if let Some(t) = target {
                args.push("-target".to_string());
                args.push(t.to_string());
            }
            args.push(dir.path().to_str().unwrap().to_string());
            let options = parse_build_options(args).expect("options");
            assert!(
                build_project(&options).is_err(),
                "target {target:?}: a missing icon must be rejected"
            );
        }
    }

    /// The `app` package is importable only in app mode; a console build that
    /// imports it is a compile error.
    #[test]
    fn build_project_rejects_importing_app_without_app_mode() {
        let dir = tempfile::tempdir().unwrap();
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
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "IMPORT app\n\nSUB main()\nEND SUB\n",
        )
        .unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }

    /// An `expect*` assertion outside a `TCASE` body is rejected before lowering.
    #[test]
    fn build_project_rejects_expect_outside_a_test_case() {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "SUB main()\n  expectInteger(1, 1)\nEND SUB\n",
        )
        .unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }

    /// A `mfb test --coverage` host run writes the coverage map, runs the driver,
    /// and folds the counts into a coverage report.
    #[test]
    fn build_project_coverage_test_writes_a_report() {
        let dir = tempfile::tempdir().unwrap();
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
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            concat!(
                "FUNC main AS Integer\n",
                "  RETURN 0\n",
                "END FUNC\n\n",
                "TESTING\n",
                "  TGROUP \"g\"\n",
                "    TCASE \"c\"\n",
                "      expectInteger(1, 1)\n",
                "    END TCASE\n",
                "  END TGROUP\n",
                "END TESTING\n"
            ),
        )
        .unwrap();
        let options =
            parse_test_options(s(&["--coverage", dir.path().to_str().unwrap()])).expect("options");
        build_project(&options).expect("coverage test passes");
        assert!(dir.path().join(crate::testing::COVMAP_FILE).is_file());
        assert!(dir.path().join(crate::testing::COVERAGE_HTML).is_file());
    }

    /// An unknown project `kind` is a warning, not an error: the build validates
    /// and returns Ok having produced no artifact (bug-300 E8).
    #[test]
    fn build_project_unknown_kind_validates_and_builds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"program\",\n",
                "  \"entry\": \"main\",\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "SUB main()\nEND SUB\n",
        )
        .unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        build_project(&options).expect("unknown kind validates");
        assert!(!dir.path().join("app.mfp").exists());
    }

    /// Every native artifact dump writer runs for an executable project.
    #[test]
    fn build_project_writes_every_native_dump() {
        for flag in ["-nir", "-nplan", "-nobj", "-ncode", "-mir"] {
            let dir = tempfile::tempdir().unwrap();
            write_executable_project(dir.path());
            let options =
                parse_build_options(s(&[flag, dir.path().to_str().unwrap()])).expect("options");
            build_project(&options).unwrap_or_else(|_| panic!("{flag} dump should succeed"));
        }
    }

    /// A native dump is unsupported for a package project (each native flavor).
    #[test]
    fn build_project_rejects_native_dumps_for_a_package() {
        for flag in ["-nir", "-mir"] {
            let dir = tempfile::tempdir().unwrap();
            write_package_project(dir.path());
            let options =
                parse_build_options(s(&[flag, dir.path().to_str().unwrap()])).expect("options");
            assert!(
                build_project(&options).is_err(),
                "{flag} must be rejected for a package"
            );
        }
    }

    /// `--sign` combined with an artifact dump flag is rejected (signing is only
    /// for a full package/executable build).
    #[test]
    fn build_project_rejects_sign_with_output_flags() {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        let options =
            parse_build_options(s(&["--sign", "ada", "--ast", dir.path().to_str().unwrap()]))
                .expect("options");
        assert!(build_project(&options).is_err());
    }

    /// A `--sign` build with no local ident key fails fast at signing-info load
    /// (no network), exercising the version/ident extraction and the load call
    /// site (the registry request itself is the signing.rs boundary A excepts).
    #[test]
    fn build_project_sign_without_a_local_ident_key_fails() {
        let _lock = crate::cli::tests::ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _mfb = crate::cli::tests::EnvVarGuard::set("MFB_HOME", home.path().to_str().unwrap());
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        let options = parse_build_options(s(&["--sign", "ada", dir.path().to_str().unwrap()]))
            .expect("options");
        assert!(build_project(&options).is_err());
    }

    /// A `LINK` naming a library with no `libraries` entry is a hard error: the
    /// native-library table cannot be assembled, aborting the executable build
    /// (and driving `assemble_native_library_table`'s error-finding loop).
    #[test]
    fn build_project_rejects_a_link_without_a_libraries_entry() {
        let dir = tempfile::tempdir().unwrap();
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
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            concat!(
                "LINK \"foo\" AS fooLib\n",
                "  FUNC ping() AS Nothing\n",
                "    SYMBOL \"foo_ping\"\n",
                "    ABI () AS status CInt32\n",
                "    SUCCESS_ON status = 0\n",
                "  END FUNC\n",
                "END LINK\n\n",
                "SUB main()\n",
                "  fooLib::ping()\n",
                "END SUB\n"
            ),
        )
        .unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }

    /// A host executable that vendors a native library resolves the vendor
    /// locator, hash-verifies the blob, and copies it beside the binary — driving
    /// `resolved_vendor_libraries`' resolution loop and the vendor copy on a real
    /// build. The `.so` need not be a real library: codegen emits a `dlopen` stub
    /// and only the recorded hash is checked at build time.
    #[test]
    fn build_project_builds_an_executable_that_vendors_a_library() {
        let host = target::BuildTarget::host();
        let dir = tempfile::tempdir().unwrap();
        // A Linux native build emits BOTH libc flavors from one invocation
        // (plan-56-B), and a Linux `vendor` locator must name its exact `arch` and
        // `libc` (§3.2) — a glibc `.so` cannot double as musl, so `libc` may not be
        // omitted. A logical library therefore needs one vendored blob per flavor,
        // each with its own filename (they share the one flat `build/vendor/`), and
        // both are copied. macOS has no libc axis and yields a single target, hence
        // one blob and one copy.
        let (locators, blobs): (String, Vec<&str>) = if host.os == "linux" {
            (
                format!(
                    "[ {{ \"os\": \"linux\", \"arch\": \"{arch}\", \"libc\": \"glibc\", \
                       \"type\": \"vendor\", \"source\": \"libfoo-glibc.so\" }}, \
                       {{ \"os\": \"linux\", \"arch\": \"{arch}\", \"libc\": \"musl\", \
                       \"type\": \"vendor\", \"source\": \"libfoo-musl.so\" }} ]",
                    arch = host.arch,
                ),
                vec!["libfoo-glibc.so", "libfoo-musl.so"],
            )
        } else {
            (
                format!(
                    "[ {{ \"os\": \"{os}\", \"arch\": \"{arch}\", \"type\": \"vendor\", \
                       \"source\": \"libfoo.so\" }} ]",
                    os = host.os,
                    arch = host.arch,
                ),
                vec!["libfoo.so"],
            )
        };
        std::fs::write(
            dir.path().join("project.json"),
            format!(
                concat!(
                    "{{\n",
                    "  \"name\": \"app\",\n",
                    "  \"version\": \"0.1.0\",\n",
                    "  \"mfb\": \"1.0\",\n",
                    "  \"kind\": \"executable\",\n",
                    "  \"entry\": \"main\",\n",
                    "  \"targets\": [\"native\"],\n",
                    "  \"libraries\": {{ \"foo\": {locators} }},\n",
                    "  \"sources\": [{{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }}]\n",
                    "}}\n"
                ),
                locators = locators,
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            concat!(
                "LINK \"foo\" AS fooLib\n",
                "  FUNC ping() AS Nothing\n",
                "    SYMBOL \"foo_ping\"\n",
                "    ABI () AS status CInt32\n",
                "    SUCCESS_ON status = 0\n",
                "  END FUNC\n",
                "END LINK\n\n",
                "SUB main()\n",
                "  fooLib::ping()\n",
                "END SUB\n"
            ),
        )
        .unwrap();
        // The vendored blobs (dummy bytes; the build records each one's sha256).
        // The `.so` need not be a real library, so both flavors may share bytes.
        std::fs::create_dir_all(dir.path().join("vendor")).unwrap();
        for blob in &blobs {
            std::fs::write(
                dir.path().join("vendor").join(blob),
                b"\x7fELF dummy vendor blob",
            )
            .unwrap();
        }
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        build_project(&options).expect("vendored build should succeed");
        // Every vendored blob the host's emitted flavors resolve was copied into
        // build/vendor/ beside the executable (one per libc flavor on Linux, one
        // on macOS).
        let vendor_out = dir
            .path()
            .join(crate::os::BUILD_DIR)
            .join(crate::os::VENDOR_DIR);
        let copied: Vec<PathBuf> = std::fs::read_dir(&vendor_out)
            .expect("build/vendor exists")
            .map(|entry| entry.expect("entry").path())
            .filter(|path| path.is_file())
            .collect();
        assert_eq!(
            copied.len(),
            blobs.len(),
            "every vendored library the host resolves was copied"
        );
    }

    /// A resources entry whose source resolves outside the project (an in-tree
    /// symlink escaping the root) fails the build at the resource-copy step, after
    /// codegen — exercising the `copy_resources` error arm in `build_project`.
    #[test]
    #[cfg(unix)]
    fn build_project_reports_a_resource_copy_failure() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.conf"), b"secret").unwrap();
        let dir = tempfile::tempdir().unwrap();
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
                "  \"resources\": [{ \"src\": \"assets/*.conf\", \"dst\": \"cfg/\" }],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "IMPORT io\n\nSUB main()\n  io::print(\"hi\")\nEND SUB\n",
        )
        .unwrap();
        // An in-tree name that textually looks contained but resolves outside.
        std::os::unix::fs::symlink(outside.path(), dir.path().join("assets")).unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }

    /// The same missing-`libraries` error aborts a package build (the package-path
    /// `assemble_native_libraries` gate).
    #[test]
    fn build_project_rejects_a_package_link_without_a_libraries_entry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"lib\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"package\",\n",
                "  \"description\": \"Test fixture package with an unmatched LINK.\",\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"package\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("lib.mfb"),
            concat!(
                "LINK \"foo\" AS fooLib\n",
                "  FUNC ping() AS Nothing\n",
                "    SYMBOL \"foo_ping\"\n",
                "    ABI () AS status CInt32\n",
                "    SUCCESS_ON status = 0\n",
                "  END FUNC\n",
                "END LINK\n\n",
                "EXPORT FUNC go() AS Nothing\n",
                "  fooLib::ping()\n",
                "END FUNC\n"
            ),
        )
        .unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }

    /// An installed but tampered signed dependency reports `[Tampered]` and is a
    /// hard build gate (the refusal arm).
    #[test]
    fn verify_and_report_refuses_a_tampered_dependency() {
        let _lock = crate::cli::tests::ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _mfb = crate::cli::tests::EnvVarGuard::set("MFB_HOME", home.path().to_str().unwrap());
        // No pinned server key -> the signed package fails attestation -> Tampered.
        let fx = build_signed_fixture("sec#signed", "0.1.0", SignedTamper::None);
        let dir = tempfile::tempdir().unwrap();
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).unwrap();
        std::fs::write(packages.join("signed.mfp"), &fx.bytes).unwrap();
        let manifest = crate::manifest::parse_project_json(
            &format!(
                "{{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"sources\":[{{\"root\":\"src\"}}],\"packages\":[{{\"name\":\"signed\",\"ident\":\"sec#signed\",\"version\":\"0.1.0\",\"pin\":true,\"source\":\"sec#signed\",\"identKey\":\"{}\"}}]}}",
                fx.ident_key
            ),
            Path::new("project.json"),
        )
        .expect("manifest");
        assert!(verify_and_report_packages(dir.path(), &manifest, false).is_err());
    }

    /// Write an app-mode executable manifest whose `icon` points at an existing
    /// file (dummy bytes). The icon existence check resolves (the `Some(resolved)`
    /// arm), then the macOS backend rejects the non-1024×1024 image.
    fn write_app_project_present_icon(dir: &Path) {
        std::fs::write(
            dir.join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"executable\",\n",
                "  \"entry\": \"main\",\n",
                "  \"mode\": \"app\",\n",
                "  \"icon\": \"assets/app.png\",\n",
                "  \"targets\": [\"native\"],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .expect("manifest");
        std::fs::create_dir_all(dir.join("src")).expect("src dir");
        std::fs::write(dir.join("src").join("main.mfb"), "SUB main()\nEND SUB\n").expect("source");
        std::fs::create_dir_all(dir.join("assets")).expect("assets dir");
        // A file that exists (so the existence check passes) but is not a valid
        // 1024×1024 image (so the backend's deep icon validation fails).
        std::fs::write(dir.join("assets").join("app.png"), b"not really a png").expect("icon");
    }

    /// A present `icon` resolves (the `Some(resolved)` icon arm), then the macOS
    /// app backend rejects the dummy image inside `write_executable` — driving the
    /// icon-resolves arm and the `write_executable` error arm on the host.
    #[test]
    fn build_project_app_mode_present_icon_resolves_then_backend_rejects() {
        // Only meaningful when the host is app-capable (macOS): the `write_executable`
        // MacApp arm is what rejects the dummy icon. On a non-app host this build
        // routes through a cross target instead, so guard on the host target.
        if !target::target_supports_app_mode(&target::BuildTarget::host()) {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        write_app_project_present_icon(dir.path());
        let options =
            parse_build_options(s(&["-app", dir.path().to_str().unwrap()])).expect("options");
        assert!(
            build_project(&options).is_err(),
            "a present-but-invalid icon must be rejected by the backend"
        );
    }

    /// App mode with no `icon` field takes the `None => None` icon arm. Routed to a
    /// Windows cross target (mfb's internal PE linker builds it without a host
    /// toolchain), exercising the no-icon arm and the WindowsApp build mode.
    #[test]
    fn build_project_app_mode_without_an_icon_field_takes_the_none_arm() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"executable\",\n",
                "  \"entry\": \"main\",\n",
                "  \"mode\": \"app\",\n",
                "  \"targets\": [\"native\"],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .expect("manifest");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "SUB main()\nEND SUB\n",
        )
        .unwrap();
        let options = parse_build_options(s(&[
            "-app",
            "-target",
            "windows-x86_64",
            dir.path().to_str().unwrap(),
        ]))
        .expect("options");
        build_project(&options).expect("windows app build with no icon succeeds");
    }

    /// `--sign` with a manifest `ident` owned by a different signer fails at
    /// `signing_ident` — before any registry/key access — exercising the signing
    /// error arm.
    #[test]
    fn build_project_sign_rejects_a_foreign_manifest_ident() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"executable\",\n",
                "  \"entry\": \"main\",\n",
                "  \"ident\": \"ada#app\",\n",
                "  \"targets\": [\"native\"],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "IMPORT io\n\nSUB main()\n  io::print(\"hi\")\nEND SUB\n",
        )
        .unwrap();
        // `zed` does not own `ada#app`, so signing_ident rejects it.
        let options = parse_build_options(s(&["--sign", "zed", dir.path().to_str().unwrap()]))
            .expect("options");
        assert!(build_project(&options).is_err());
    }

    /// A top-level `EXPORT` in an executable resolves cleanly but is rejected by
    /// the export-in-executable diagnostic, driving the `had_error` return after
    /// the verify phase (as distinct from a resolver name error).
    #[test]
    fn build_project_export_in_an_executable_is_a_verify_error() {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "EXPORT FUNC leak() AS Integer\n  RETURN 1\nEND FUNC\n\nSUB main()\nEND SUB\n",
        )
        .unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }

    /// A `build/` path that already exists as a regular file (not a directory)
    /// makes the start-of-build clear fail with a non-`NotFound` error, aborting
    /// the executable build.
    #[test]
    fn build_project_errors_when_build_path_is_a_file() {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        // A regular file where the build directory should be: remove_dir_all fails
        // with NotADirectory (not NotFound), which is the fatal arm.
        std::fs::write(dir.path().join(crate::os::BUILD_DIR), b"not a directory").unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }

    /// An executable declaring a local package dependency whose `.mfp` is not
    /// installed passes package verification (a missing local dep is skipped there)
    /// then fails at the strict install check inside the executable build.
    #[test]
    fn build_project_executable_missing_dependency_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
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
                "  \"packages\": [{ \"name\": \"ghost\", \"ident\": \"ada#ghost\", \"version\": \"0.1.0\", \"pin\": true, \"source\": \"file:packages/ghost.mfp\" }],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "IMPORT io\n\nSUB main()\n  io::print(\"hi\")\nEND SUB\n",
        )
        .unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }

    /// The same missing-dependency install check gates a package build (the
    /// package arm's install-check error).
    #[test]
    fn build_project_package_missing_dependency_is_a_hard_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"lib\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"package\",\n",
                "  \"description\": \"Fixture package with a missing dependency.\",\n",
                "  \"packages\": [{ \"name\": \"ghost\", \"ident\": \"ada#ghost\", \"version\": \"0.1.0\", \"pin\": true, \"source\": \"file:packages/ghost.mfp\" }],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"package\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("lib.mfb"),
            "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n",
        )
        .unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(build_project(&options).is_err());
    }

    /// A build that reads an installed dependency's exported function signatures
    /// drives the `returns_imported_resource` filter closure over every external
    /// signature (bug-377), even for the common non-resource return.
    #[test]
    fn build_project_reads_installed_dependency_exports() {
        let dir = tempfile::tempdir().unwrap();
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
                "  \"packages\": [{ \"name\": \"trap_builtin_pkg\", \"ident\": \"tests#trap\", \"version\": \"0.1.0\", \"pin\": false, \"source\": \"file:packages/trap_builtin_pkg.mfp\" }],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "IMPORT io\n\nSUB main()\n  io::print(\"hi\")\nEND SUB\n",
        )
        .unwrap();
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).unwrap();
        std::fs::copy(
            "tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp",
            packages.join("trap_builtin_pkg.mfp"),
        )
        .expect("copy fixture");
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        // The build succeeds; the point is that the external-signature filter ran.
        build_project(&options).expect("build with an installed dependency succeeds");
    }

    /// Write a `kind: "package"` project at `dir` exporting `answer()`.
    fn write_source_package(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir.join("src")).expect("package src dir");
        std::fs::write(
            dir.join("project.json"),
            format!(
                concat!(
                    "{{\n",
                    "  \"name\": \"{name}\",\n",
                    "  \"version\": \"0.1.0\",\n",
                    "  \"mfb\": \"1.0\",\n",
                    "  \"kind\": \"package\",\n",
                    "  \"description\": \"A source-directory dependency.\",\n",
                    "  \"targets\": [\"native\"],\n",
                    "  \"sources\": [{{ \"root\": \"src\", \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }}]\n",
                    "}}\n"
                ),
                name = name
            ),
        )
        .expect("package manifest");
        std::fs::write(
            dir.join("src").join("lib.mfb"),
            "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n",
        )
        .expect("package source");
    }

    /// bug-480 Defect A: a dependency declared by SOURCE DIRECTORY is compiled
    /// into `build/packages/<name>.mfp` and its exported signatures resolve, so
    /// the call types as `Integer` instead of `Unknown`.
    #[test]
    fn build_project_compiles_a_source_directory_dependency() {
        let dir = tempfile::tempdir().unwrap();
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
                "  \"packages\": [{ \"name\": \"tiny\", \"version\": \"=0.1.0\", \"source\": \"file:packages/tiny\" }],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "IMPORT io\nIMPORT tiny\n\nSUB main()\n  io::print(toString(tiny::answer()))\nEND SUB\n",
        )
        .unwrap();
        write_source_package(&dir.path().join("packages").join("tiny"), "tiny");

        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        build_project(&options).expect("source-directory dependency builds");

        // The compiled interface lands in the cache, never beside the sources.
        let cache = crate::manifest::package::source_package_cache_dir(dir.path());
        assert!(
            cache.join("tiny.mfp").is_file(),
            "the dependency's .mfp belongs in build/packages/"
        );
        assert!(
            !dir.path()
                .join("packages")
                .join("tiny")
                .join("tiny.mfp")
                .exists(),
            "nothing is written into the dependency's source tree"
        );
        // The executable-branch build-dir clear must not take the cache with it:
        // its paths are what `write_executable` was handed.
        assert!(
            dir.path()
                .join(crate::os::BUILD_DIR)
                .join("app.out")
                .exists()
                || dir.path().join(crate::os::BUILD_DIR).exists()
        );
    }

    /// A source dependency that depends on itself must be a located, coded
    /// diagnostic — not an infinite recursion.
    #[test]
    fn build_project_rejects_a_source_dependency_cycle() {
        let dir = tempfile::tempdir().unwrap();
        let package_dir = dir.path().join("packages").join("tiny");
        write_source_package(&package_dir, "tiny");
        // Point the package at ITSELF, by absolute path, so the two entries are
        // the same directory rather than two copies of one.
        let absolute = std::fs::canonicalize(&package_dir).expect("canonical package dir");
        std::fs::write(
            package_dir.join("project.json"),
            format!(
                concat!(
                    "{{\n",
                    "  \"name\": \"tiny\",\n",
                    "  \"version\": \"0.1.0\",\n",
                    "  \"mfb\": \"1.0\",\n",
                    "  \"kind\": \"package\",\n",
                    "  \"description\": \"A source package that depends on itself.\",\n",
                    "  \"targets\": [\"native\"],\n",
                    "  \"packages\": [{{ \"name\": \"tiny\", \"version\": \"=0.1.0\", \"source\": \"local://{}\" }}],\n",
                    "  \"sources\": [{{ \"root\": \"src\", \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }}]\n",
                    "}}\n"
                ),
                absolute.display()
            ),
        )
        .unwrap();
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
                "  \"packages\": [{ \"name\": \"tiny\", \"version\": \"=0.1.0\", \"source\": \"file:packages/tiny\" }],\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            "IMPORT io\nIMPORT tiny\n\nSUB main()\n  io::print(toString(tiny::answer()))\nEND SUB\n",
        )
        .unwrap();

        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        assert!(
            build_project(&options).is_err(),
            "a dependency cycle must fail the build"
        );
        // The emitted identity is defined and non-sentinel.
        assert_eq!(
            crate::rules::code_and_name("IMPORT_PACKAGE_MANIFEST_INVALID"),
            ("2-201-0005", "IMPORT_PACKAGE_MANIFEST_INVALID")
        );
    }

    /// A cross-target `mfb test` build cannot run the produced binary on the host,
    /// so it writes the test executable and reports the artifact instead of
    /// executing it — driving the cross-target test-report arm.
    #[test]
    fn build_project_cross_target_test_reports_the_artifact() {
        let dir = tempfile::tempdir().unwrap();
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
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src").join("main.mfb"),
            concat!(
                "FUNC main AS Integer\n",
                "  RETURN 0\n",
                "END FUNC\n\n",
                "TESTING\n",
                "  TGROUP \"g\"\n",
                "    TCASE \"c\"\n",
                "      expectInteger(1, 1)\n",
                "    END TCASE\n",
                "  END TGROUP\n",
                "END TESTING\n"
            ),
        )
        .unwrap();
        let options = parse_test_options(s(&[
            "--target",
            "windows-x86_64",
            dir.path().to_str().unwrap(),
        ]))
        .expect("options");
        // A cross target cannot be run on the host; the driver is written and
        // reported (no execution, no host `build/` clobber).
        build_project(&options).expect("cross-target test build writes an artifact");
    }

    /// A package build whose output location is read-only fails when
    /// `write_package` cannot create the `.mfp`, exercising that error arm.
    #[test]
    #[cfg(unix)]
    fn build_project_package_write_to_a_readonly_location_fails() {
        use std::os::unix::fs::PermissionsExt;
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        write_package_project(dir.path());
        // Read + execute, but not write: sources still read, the .mfp cannot be
        // written.
        let mut perm = std::fs::metadata(dir.path()).unwrap().permissions();
        perm.set_mode(0o555);
        std::fs::set_permissions(dir.path(), perm).unwrap();
        let options =
            parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
        let result = build_project(&options);
        // Restore write so the temp dir can be cleaned up.
        let mut restore = std::fs::metadata(dir.path()).unwrap().permissions();
        restore.set_mode(0o755);
        std::fs::set_permissions(dir.path(), restore).unwrap();
        assert!(
            result.is_err(),
            "write_package into a read-only dir must fail"
        );
    }

    /// Every artifact-dump writer surfaces its write error when the output location
    /// is read-only, driving each dump's error arm (AST/IR/BR and the shared native
    /// dump writer).
    #[test]
    #[cfg(unix)]
    fn build_project_dump_writers_report_a_readonly_location() {
        use std::os::unix::fs::PermissionsExt;
        if running_as_root() {
            return;
        }
        for flag in [
            "-ast", "-ir", "-br", "-nir", "-nplan", "-nobj", "-ncode", "-mir",
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_executable_project(dir.path());
            let mut perm = std::fs::metadata(dir.path()).unwrap().permissions();
            perm.set_mode(0o555);
            std::fs::set_permissions(dir.path(), perm).unwrap();
            let options =
                parse_build_options(s(&[flag, dir.path().to_str().unwrap()])).expect("options");
            let result = build_project(&options);
            let mut restore = std::fs::metadata(dir.path()).unwrap().permissions();
            restore.set_mode(0o755);
            std::fs::set_permissions(dir.path(), restore).unwrap();
            assert!(
                result.is_err(),
                "{flag}: a read-only location must fail the dump"
            );
        }
    }
}
