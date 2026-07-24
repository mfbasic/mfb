use super::*;

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
