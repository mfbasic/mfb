//! Every arm of the `mfb <command>` dispatcher, by the exit code it answers.
//!
//! `dispatch.rs` was 0 of 201 lines — the largest never-executed file in the
//! tree, and the code every single invocation runs. Not because it is untested
//! (the acceptance harness runs the real binary ~1,400 times) but because it was
//! unreachable from a unit test: it read `env::args()`, which a test cannot
//! inject, and ended each arm in `process::exit`, which takes the test binary
//! down with it. Splitting the `env::args()` read and the `process::exit` into
//! [`super::run`] left [`super::dispatch`] a plain `Vec<String> -> i32`.
//!
//! **The exit code IS the contract.** `0` succeeded, `2` is a usage error and
//! `1` is a command that ran and failed, and a developer's shell — and every
//! `[exit N]` line in the acceptance goldens — reads exactly that. An arm that
//! answered `1` where `2` belongs turns a typo into "your project is broken";
//! one that answered `0` for an unknown command makes `mfb buidl` silently do
//! nothing in a script.
//!
//! Every command name is driven, because a routing table is exactly the kind of
//! thing where one arm is wired to the wrong handler and nothing else notices.
//! Each is given `--help` or a deliberately wrong operand rather than real work:
//! a bare `mfb build` builds the current directory, which is not this suite's
//! business, and the arms above the handler call are what was uncovered anyway.

use super::dispatch;

fn run(args: &[&str]) -> i32 {
    dispatch(args.iter().map(|arg| arg.to_string()).collect())
}

/// The three spellings of help, and a bare `mfb`, all succeed.
///
/// Grouped in one arm in the source, so this is really one claim about four
/// inputs — and the bare-invocation case is the one a user hits by accident.
#[test]
fn help_in_every_spelling_succeeds() {
    for args in [&[][..], &["help"], &["--help"], &["-h"]] {
        assert_eq!(
            run(args),
            0,
            "`mfb {}` prints the usage screen and succeeds",
            args.join(" ")
        );
    }
}

#[test]
fn the_version_flag_succeeds_in_both_spellings() {
    for args in [&["--version"][..], &["-V"]] {
        assert_eq!(run(args), 0, "`mfb {}` prints the version", args.join(" "));
    }
}

/// An unknown command is a USAGE error, not a failure.
///
/// The distinction is the whole point of having two non-zero codes: `mfb buidl`
/// is a typo, and a shell that branches on `$? -eq 1` must not see it as "the
/// build failed".
#[test]
fn an_unknown_command_is_a_usage_error() {
    assert_eq!(run(&["buidl"]), 2);
    assert_eq!(run(&["--nope"]), 2);
}

/// Every command answers `0` to `--help`, and routes to a real screen.
///
/// The list is the dispatcher's own command set. A command added to the routing
/// table without a help arm shows up here as a non-zero code — usually `2`,
/// because the handler is then asked to parse `--help` as an operand.
#[test]
fn every_command_answers_its_help_flag() {
    for command in [
        "init", "init-pkg", "build", "test", "pkg", "repo", "machine", "key", "org", "token",
        "audit", "man", "spec", "doc", "fmt",
    ] {
        assert_eq!(
            run(&[command, "--help"]),
            0,
            "`mfb {command} --help` must print {command}'s help and succeed"
        );
        assert_eq!(
            run(&[command, "-h"]),
            0,
            "`mfb {command} -h` is the same screen: both spellings are what a \
             user reaching for help actually types"
        );
    }
}

/// `init` and `init-pkg` refuse a missing operand and a surplus one, both as
/// usage errors.
///
/// Both directions, because "exactly one" is two checks and a handler that made
/// only the first would silently ignore a second path — creating the project at
/// the first and leaving the developer's second argument unexplained.
#[test]
fn init_requires_exactly_one_location() {
    for command in ["init", "init-pkg"] {
        assert_eq!(
            run(&[command]),
            2,
            "`mfb {command}` with no location is a usage error"
        );
        assert_eq!(
            run(&[command, "one", "two"]),
            2,
            "`mfb {command}` with two locations is a usage error, not a project \
             created at the first"
        );
    }
}

/// A flag the option parser refuses is a usage error, for both build-shaped
/// commands.
///
/// `build` and `test` share `parse_build_options`/`parse_test_options` and each
/// has its own arm, so a refusal wired into one and not the other is invisible
/// without both.
#[test]
fn a_refused_build_flag_is_a_usage_error() {
    assert_eq!(run(&["build", "--not-a-flag"]), 2);
    assert_eq!(run(&["test", "--not-a-flag"]), 2);
}

/// A subcommand-taking command with no subcommand is a usage error.
///
/// These route through `CommandError::Usage`, which is the other half of
/// `dispatch_command_error` — the half that answers `2`. Without them the mapping
/// from `CommandError` to an exit code is asserted in one direction only.
#[test]
fn a_missing_subcommand_is_a_usage_error() {
    for command in ["pkg", "repo", "machine", "key", "org", "token"] {
        assert_eq!(
            run(&[command]),
            2,
            "`mfb {command}` names no subcommand, which is a usage error"
        );
        assert_eq!(
            run(&[command, "not-a-subcommand"]),
            2,
            "`mfb {command} not-a-subcommand` is a usage error too — an unknown \
             subcommand is a typo, not a command that ran and failed"
        );
    }
}

/// A flag `audit` refuses is a usage error.
#[test]
fn a_refused_audit_flag_is_a_usage_error() {
    assert_eq!(run(&["audit", "--not-a-flag"]), 2);
}

/// `man` and `spec` refuse a name they do not have, and succeed on one they do.
///
/// Both halves: a renderer that failed for everything would satisfy the refusal
/// alone, and one that succeeded for everything would satisfy the success alone.
#[test]
fn man_and_spec_answer_for_a_real_name_and_refuse_a_missing_one() {
    assert_eq!(run(&["man", "strings"]), 0, "`strings` is a real package");
    assert_eq!(
        run(&["man", "no_such_package_at_all"]),
        2,
        "a package that does not exist is a usage error"
    );
    assert_eq!(
        run(&["spec"]),
        0,
        "`mfb spec` with no argument lists the spec"
    );
    assert_eq!(
        run(&["spec", "no-such-section-at-all"]),
        2,
        "a spec section that does not exist is a usage error"
    );
}
