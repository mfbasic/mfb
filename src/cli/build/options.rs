use super::*;

/// Parse the `--target`/`-O` options shared verbatim by `mfb build` and
/// `mfb test`. Returns `Ok(true)` when `arg` was one of them (updating
/// `target` or `opt`, consuming the value from `iter` for the
/// space-separated form), `Ok(false)` when `arg` is not a shared option, or
/// `Err` on a malformed value. `cmd` is the subcommand word (`"build"` /
/// `"test"`) for the error text.
fn parse_common_option(
    arg: &str,
    iter: &mut impl Iterator<Item = String>,
    cmd: &str,
    target: &mut Option<target::BuildTarget>,
    opt: &mut crate::optimizer::OptLevel,
) -> Result<bool, String> {
    if arg == "--target" || arg == "-target" {
        let Some(value) = iter.next() else {
            return Err(format!("mfb {cmd} -target requires os-arch"));
        };
        *target = Some(target::BuildTarget::parse(&value)?);
    } else if let Some(value) = arg
        .strip_prefix("--target=")
        .or_else(|| arg.strip_prefix("-target="))
    {
        *target = Some(target::BuildTarget::parse(value)?);
    } else if arg == "-O" || arg == "--optimize" || arg == "-optimize" {
        let Some(value) = iter.next() else {
            return Err(format!("mfb {cmd} -O requires an optimization level"));
        };
        *opt = crate::optimizer::parse_level(&value)?;
    } else if let Some(value) = arg
        .strip_prefix("--optimize=")
        .or_else(|| arg.strip_prefix("-optimize="))
        .or_else(|| arg.strip_prefix("-O="))
        // The attached `-O0`/`-O1` spelling every C compiler uses. Checked last
        // so the bare `-O`/`-O=` forms above win their exact match first.
        .or_else(|| arg.strip_prefix("-O"))
    {
        *opt = crate::optimizer::parse_level(value)?;
    } else {
        return Ok(false);
    }
    Ok(true)
}

/// Whether `arg` is any spelling of the verbosity flag: the single `-v` /
/// `--verbose`, or the bundled `-vv` that asks for the compile profiler in one
/// word.
fn is_verbose_flag(arg: &str) -> bool {
    matches!(arg, "-v" | "--verbose" | "-vv" | "--vv")
}

/// A verbosity that is not `Quiet` — i.e. one that conflicts with `-q`.
fn is_verbose(level: Verbosity) -> bool {
    level >= Verbosity::Verbose
}

/// Apply one verbosity flag, escalating rather than replacing.
///
/// Repeating the flag is how every compiler spells "more of this", so `-v -v`
/// (and `--verbose --verbose`) reaches [`Verbosity::Trace`] exactly as the
/// bundled `-vv` does; a third `-v` is a no-op because there is nothing above
/// Trace to reach. Escalation, not replacement, is what lets a wrapper script
/// pass a baseline `-v` and a user add another on the command line.
fn raise_verbosity(arg: &str, level: &mut Option<Verbosity>, cmd: &str) -> Result<(), String> {
    let bundled = arg == "-vv" || arg == "--vv";
    *level = Some(match *level {
        Some(Verbosity::Quiet) => {
            return Err(format!("mfb {cmd} accepts at most one of -q / -v"));
        }
        // Already verbose (or already tracing): another flag means Trace.
        Some(Verbosity::Verbose) | Some(Verbosity::Trace) => Verbosity::Trace,
        Some(Verbosity::Normal) | None if bundled => Verbosity::Trace,
        Some(Verbosity::Normal) | None => Verbosity::Verbose,
    });
    Ok(())
}

pub(crate) fn parse_build_options(args: Vec<String>) -> Result<BuildOptions, String> {
    let mut location = None;
    let mut outputs: Vec<BuildOutput> = Vec::new();
    let mut target = None;
    let mut sign_owner = None;
    let mut app_mode = false;
    let mut app_debug = false;
    let mut allow_unsigned = false;
    let mut opt = crate::optimizer::active_opt_level();
    let mut verbosity: Option<Verbosity> = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if let Some(output) = BuildOutput::from_flag(&arg) {
            if outputs.contains(&output) {
                return Err(format!("mfb build got duplicate output flag `{arg}`"));
            }
            outputs.push(output);
        } else if parse_common_option(&arg, &mut iter, "build", &mut target, &mut opt)? {
            // handled by the shared --target/-O parser
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
        } else if arg == "-q" || arg == "--quiet" {
            if verbosity.replace(Verbosity::Quiet).is_some_and(is_verbose) {
                return Err("mfb build accepts at most one of -q / -v".to_string());
            }
        } else if is_verbose_flag(&arg) {
            raise_verbosity(&arg, &mut verbosity, "build")?;
        } else if arg.starts_with('-') {
            return Err(format!("unknown build option `{arg}`"));
        } else if location.replace(PathBuf::from(&arg)).is_some() {
            return Err("mfb build accepts at most one [location]".to_string());
        }
    }

    Ok(BuildOptions {
        location: location.unwrap_or_else(|| PathBuf::from(".")),
        outputs,
        package_output_dir: None,
        target: target.unwrap_or_else(target::BuildTarget::host),
        sign_owner,
        // plan-51-C §4.7: `--app-debug` implies `--app`. `--app --app-debug` is
        // the same thing said twice and is accepted; requiring both would be a
        // papercut with no upside.
        app_mode: app_mode || app_debug,
        app_debug,
        opt,
        allow_unsigned,
        mode: crate::testing::CompileMode::Build,
        verbosity: verbosity.unwrap_or_default(),
    })
}

/// Parse `mfb test [location] [--coverage] [--target …] [-O …] [-v]`. The build
/// pipeline is shared with `mfb build`; only the compile mode and the always-run
/// behavior differ (plan-18).
pub(crate) fn parse_test_options(args: Vec<String>) -> Result<BuildOptions, String> {
    let mut location = None;
    let mut target = None;
    let mut opt = crate::optimizer::active_opt_level();
    let mut coverage = false;
    let mut verbose: Option<Verbosity> = None;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == "--coverage" {
            coverage = true;
        } else if is_verbose_flag(&arg) {
            raise_verbosity(&arg, &mut verbose, "test")?;
        } else if parse_common_option(&arg, &mut iter, "test", &mut target, &mut opt)? {
            // handled by the shared --target/-O parser
        } else if arg.starts_with('-') {
            return Err(format!("unknown test option `{arg}`"));
        } else if location.replace(PathBuf::from(&arg)).is_some() {
            return Err("mfb test accepts at most one [location]".to_string());
        }
    }

    Ok(BuildOptions {
        location: location.unwrap_or_else(|| PathBuf::from(".")),
        outputs: Vec::new(),
        package_output_dir: None,
        target: target.unwrap_or_else(target::BuildTarget::host),
        sign_owner: None,
        app_mode: false,
        // `mfb test` never runs a test binary out of a sealed AppImage, so it
        // takes neither `--app` nor `--app-debug`; both land in the
        // `unknown test option` arm above (plan-51-C §4.7).
        app_debug: false,
        opt,
        allow_unsigned: false,
        mode: crate::testing::CompileMode::Test { coverage },
        // `mfb test`'s user-facing output is the pass/fail tree; the build
        // summary would be noise and (via `target.name()`) non-portable across
        // machines, churning `.testrun` goldens. Stay quiet by default
        // (plan-36); `-v` opts into the build summary, per-phase timings, live
        // `codegen:` lines, and optimizer fire counts, and `-vv` adds the
        // compile-profiler report — all on stderr, so the pass/fail tree on
        // stdout is unchanged.
        verbosity: verbose.unwrap_or(Verbosity::Quiet),
    })
}
