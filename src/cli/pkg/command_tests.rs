//! The `mfb pkg` subcommands that read a path, and every usage refusal.
//!
//! `cli/pkg.rs` is 2,274 lines and was at 62.09% — the largest single gap left
//! under `src/`. Its command table is a slice `match` where each subcommand has
//! a working arm and a usage arm right beneath it, and only the working arms of
//! the network-free commands were ever run (by the acceptance harness, through
//! the built binary; nothing reached them in process).
//!
//! What is driven here is everything that does not need a network or a
//! current-directory project: the two path-taking readers (`info`, `validate`)
//! and the usage refusals, which are the arms that answer `2`.
//!
//! **A usage arm is a real contract, not a formality.** The table is ordered, and
//! each usage arm sits after its working ones, so `[command, ..]` catches
//! whatever the working arms did not. Move one up and it shadows the command it
//! was meant to explain — `mfb pkg info x.mfp` would answer "requires exactly one
//! <package>" while holding exactly one. That is invisible to a type checker and
//! to every test that only calls the working form.

use super::{run_pkg_command, PkgCommandError};

fn run(args: &[&str]) -> Result<(), PkgCommandError> {
    run_pkg_command(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
}

fn usage(args: &[&str]) -> String {
    match run(args) {
        Err(PkgCommandError::Usage(message)) => message,
        Err(PkgCommandError::Failed(message)) => panic!(
            "`mfb pkg {}` must be a USAGE error -- the command line is malformed, \
             not a command that ran and failed; got: {message}",
            args.join(" ")
        ),
        Ok(()) => panic!(
            "`mfb pkg {}` must be refused rather than run",
            args.join(" ")
        ),
    }
}

/// A committed `.mfp` in the tree, used as a package that really parses.
const REAL_PACKAGE: &str =
    "tests/rt-behavior/native/native-link-import-sqlite-rt/packages/sqlite3.mfp";

/// `pkg info` reads a real package and refuses one that is not there.
///
/// Both halves: a reader that failed for everything satisfies the refusal alone,
/// and one that succeeded for everything satisfies the read alone.
#[test]
fn pkg_info_reads_a_package_and_refuses_a_missing_one() {
    assert!(
        run(&["info", REAL_PACKAGE]).is_ok(),
        "`{REAL_PACKAGE}` is a committed package and must read"
    );
    assert!(
        matches!(
            run(&["info", "no/such/package.mfp"]),
            Err(PkgCommandError::Failed(_))
        ),
        "a package file that is not there is a FAILURE -- the command line was \
         well formed and the command could not do its job"
    );
}

/// Every subcommand's usage arm answers, and names the subcommand it is about.
///
/// The message matters as much as the code: the arms are adjacent in one `match`
/// and take the same shape, so a copied arm that kept the previous
/// subcommand's text tells the developer to fix the wrong command line. Checked
/// by NAME rather than by full text, so rewording the help does not break this.
#[test]
fn each_subcommand_refuses_its_own_wrong_arity_and_says_which() {
    for (args, name) in [
        (&["info"][..], "info"),
        (&["info", "a", "b"], "info"),
        (&["validate"], "validate"),
        (&["validate", "a", "b"], "validate"),
        (&["add"], "add"),
        (&["install", "a", "b"], "install"),
    ] {
        let message = usage(args);
        assert!(
            message.contains(name),
            "`mfb pkg {}` must be refused with a message naming `{name}`, so the \
             developer knows which command line to fix; got: {message}",
            args.join(" ")
        );
    }
}

/// An unknown subcommand, and none at all, are usage errors.
///
/// The table's last arm. Without it a typo would fall through the whole match
/// and the compiler would decide what happens, which for a slice `match` means
/// a non-exhaustive-pattern error at build time rather than a message at run
/// time -- so this arm exists precisely to turn a would-be crash into help.
#[test]
fn an_unknown_or_absent_subcommand_is_a_usage_error() {
    assert!(
        !usage(&[]).is_empty(),
        "`mfb pkg` alone must explain itself"
    );
    let message = usage(&["not-a-subcommand"]);
    assert!(
        message.contains("not-a-subcommand"),
        "an unknown subcommand must be quoted back, so a typo is visible; got: \
         {message}"
    );
}
