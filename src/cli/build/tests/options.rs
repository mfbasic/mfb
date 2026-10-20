//! plan-176-K: `cli::build::tests` — the options tests. The shared project builders live in\n//! `build/tests/mod.rs`.

use super::*;

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
    assert!(!options.debug);
    assert!(!options.allow_unsigned);
    assert_eq!(options.target, target::BuildTarget::host());
}

#[test]
fn parse_build_options_parses_target_both_forms() {
    let split = parse_build_options(s(&["--target", "linux-aarch64"])).expect("split target form");
    assert_eq!(split.target.name(), "linux-aarch64");
    let joined = parse_build_options(s(&["--target=linux-x86_64"])).expect("joined target form");
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

/// plan-130-A: `--debug` turns on the stderr debug report.
#[test]
fn parse_build_options_debug_sets_flag() {
    let options = parse_build_options(s(&["--debug"])).expect("--debug");
    assert!(options.debug);
    // `--debug` is its own axis: it is not app mode and not `--app-debug`.
    assert!(!options.app_mode && !options.app_debug);
    let both = parse_build_options(s(&["--app", "--debug"])).expect("--app --debug");
    assert!(both.app_mode && both.debug);
}

#[test]
fn parse_build_options_rejects_repeated_debug() {
    let err = match parse_build_options(s(&["--debug", "--debug"])) {
        Err(err) => err,
        Ok(_) => panic!("mfb build must reject a repeated --debug"),
    };
    assert_eq!(err, "mfb build accepts at most one --debug option");
}

#[test]
fn parse_test_options_debug_sets_flag() {
    let options = parse_test_options(s(&["--debug"])).expect("--debug");
    assert!(options.debug);
    assert!(!parse_test_options(vec![]).expect("defaults").debug);
    let err = match parse_test_options(s(&["--debug", "--debug"])) {
        Err(err) => err,
        Ok(_) => panic!("mfb test must reject a repeated --debug"),
    };
    assert_eq!(err, "mfb test accepts at most one --debug option");
}

/// plan-167-C: `--config <name>` and `--config=<name>` select a named config;
/// without the flag nothing is selected here and the build uses `release`.
#[test]
fn parse_build_options_parses_config_both_forms() {
    assert_eq!(parse_build_options(vec![]).expect("defaults").config, None);
    let spaced = parse_build_options(s(&["--config", "dev"])).expect("--config dev");
    assert_eq!(spaced.config.as_deref(), Some("dev"));
    let joined = parse_build_options(s(&["--config=dev", "proj"])).expect("--config=dev");
    assert_eq!(joined.config.as_deref(), Some("dev"));
    assert_eq!(joined.location, PathBuf::from("proj"));
}

#[test]
fn parse_build_options_rejects_repeated_config() {
    for args in [
        ["--config", "a", "--config", "b"].as_slice(),
        &["--config=a", "--config=a"],
    ] {
        let err = match parse_build_options(s(args)) {
            Err(err) => err,
            Ok(_) => panic!("mfb build must reject a repeated --config: {args:?}"),
        };
        assert_eq!(err, "mfb build accepts at most one --config option");
    }
}

#[test]
fn parse_build_options_rejects_empty_config() {
    for args in [["--config"].as_slice(), &["--config="]] {
        let err = match parse_build_options(s(args)) {
            Err(err) => err,
            Ok(_) => panic!("mfb build must reject a missing config name: {args:?}"),
        };
        assert_eq!(err, "mfb build --config requires a config name");
    }
}

#[test]
fn parse_test_options_parses_config_both_forms() {
    assert_eq!(parse_test_options(vec![]).expect("defaults").config, None);
    let spaced = parse_test_options(s(&["--config", "dev"])).expect("--config dev");
    assert_eq!(spaced.config.as_deref(), Some("dev"));
    let joined = parse_test_options(s(&["--config=dev"])).expect("--config=dev");
    assert_eq!(joined.config.as_deref(), Some("dev"));
}

#[test]
fn parse_test_options_rejects_repeated_config() {
    let err = match parse_test_options(s(&["--config=a", "--config", "b"])) {
        Err(err) => err,
        Ok(_) => panic!("mfb test must reject a repeated --config"),
    };
    assert_eq!(err, "mfb test accepts at most one --config option");
}

#[test]
fn parse_test_options_rejects_empty_config() {
    for args in [["--config"].as_slice(), &["--config="]] {
        let err = match parse_test_options(s(args)) {
            Err(err) => err,
            Ok(_) => panic!("mfb test must reject a missing config name: {args:?}"),
        };
        assert_eq!(err, "mfb test --config requires a config name");
    }
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
