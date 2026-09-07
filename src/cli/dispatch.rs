//! The top-level `mfb <command>` dispatcher. `run` is the entry point that
//! `fn main` delegates to; it parses the leading subcommand and routes to the
//! matching CLI handler.

use std::env;
use std::path::Path;
use std::process;

use crate::audit;
use crate::cli::build::{build_project, parse_build_options, parse_test_options};
use crate::cli::doc::run_doc_command;
use crate::cli::fmt::run_fmt_command;
use crate::cli::help::{
    AUDIT_HELP, BUILD_HELP, DOC_HELP, FMT_HELP, INIT_HELP, INIT_PKG_HELP, PKG_HELP, REPO_HELP,
    SPEC_HELP, TEST_HELP, USAGE,
};
use crate::cli::init::{init_package_project, init_project};
use crate::cli::man::show_man;
use crate::cli::pkg::run_pkg_command;
use crate::cli::repo::run_repo_command;
use crate::cli::spec::show_spec;

#[cfg(test)]
#[path = "dispatch/tests.rs"]
mod tests;

/// Returns true when `arg` requests command-specific help.
fn is_help_flag(arg: &str) -> bool {
    arg == "--help" || arg == "-h"
}

/// The process entry: dispatch this invocation's arguments, then exit with the
/// code it produced.
///
/// Nothing but the `env::args()` read and the `process::exit` lives here, and
/// that is the point. [`dispatch`] used to be this function, and being this
/// function is what made it untestable: a unit test cannot inject `env::args()`,
/// and a `process::exit` inside a test takes the whole test binary down. It was
/// the largest never-executed file in the tree — 0 of 201 lines — while being
/// the code EVERY invocation runs.
pub(crate) fn run() {
    let code = dispatch(env::args().skip(1).collect::<Vec<_>>());
    // A zero exit falls through rather than calling `process::exit(0)`, so the
    // compiler thread ends and `main` returns exactly as it did before.
    if code != 0 {
        process::exit(code);
    }
}

/// Route one invocation's arguments and answer its exit code.
///
/// `0` is success; `2` is a usage error (an unknown command, a missing or
/// surplus operand, a flag the parser refused) and `1` is a command that ran and
/// failed. Those are the codes the acceptance goldens record as `[exit N]`, so
/// the mapping is pinned by ~1,400 fixtures as well as by this module's tests.
pub(crate) fn dispatch(args: Vec<String>) -> i32 {
    let mut args = args.into_iter();

    match args.next().as_deref() {
        // `help`, `--help`/`-h`, and a bare `mfb` all reach the same screen; the
        // flag spellings are what a user reaching for help actually types
        // (plan-42 §4.4).
        Some("help") | Some("--help") | Some("-h") | None => {
            println!("{USAGE}");
        }
        Some("--version") | Some("-V") => {
            crate::cli::version::print_version();
        }
        Some("init") => {
            let init_args = args.collect::<Vec<_>>();
            if init_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{INIT_HELP}");
                return 0;
            }
            let mut init_args = init_args.into_iter();

            let Some(location) = init_args.next() else {
                eprintln!("error: mfb init requires <location>\n\n{USAGE}");
                return 2;
            };

            if init_args.next().is_some() {
                eprintln!("error: mfb init accepts exactly one <location>\n\n{USAGE}");
                return 2;
            }

            if let Err(err) = init_project(Path::new(&location)) {
                eprintln!("error: {err}");
                return 1;
            }
        }
        Some("init-pkg") => {
            let init_args = args.collect::<Vec<_>>();
            if init_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{INIT_PKG_HELP}");
                return 0;
            }
            let mut init_args = init_args.into_iter();

            let Some(location) = init_args.next() else {
                eprintln!("error: mfb init-pkg requires <location>\n\n{USAGE}");
                return 2;
            };

            if init_args.next().is_some() {
                eprintln!("error: mfb init-pkg accepts exactly one <location>\n\n{USAGE}");
                return 2;
            }

            if let Err(err) = init_package_project(Path::new(&location)) {
                eprintln!("error: {err}");
                return 1;
            }
        }
        Some("build") => {
            let build_args = args.collect::<Vec<_>>();
            if build_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{BUILD_HELP}");
                return 0;
            }
            let build_options = match parse_build_options(build_args) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    return 2;
                }
            };

            if let Err(()) = build_project(&build_options) {
                return close_diagnostics(1);
            }
        }
        Some("test") => {
            let test_args = args.collect::<Vec<_>>();
            if test_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{TEST_HELP}");
                return 0;
            }
            let test_options = match parse_test_options(test_args) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    return 2;
                }
            };

            if let Err(()) = build_project(&test_options) {
                return close_diagnostics(1);
            }
        }
        Some("pkg") => {
            let pkg_args = args.collect::<Vec<_>>();
            if pkg_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{PKG_HELP}");
                return 0;
            }
            if let Err(err) = run_pkg_command(&pkg_args) {
                return crate::cli::dispatch_command_error(err);
            }
        }
        Some("repo") => {
            let repo_args = args.collect::<Vec<_>>();
            if repo_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return 0;
            }
            if let Err(err) = run_repo_command(&repo_args) {
                return crate::cli::dispatch_command_error(err);
            }
        }
        Some("machine") => {
            let machine_args = args.collect::<Vec<_>>();
            if machine_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return 0;
            }
            if let Err(err) = crate::cli::repo::run_machine_command(&machine_args) {
                return crate::cli::dispatch_command_error(err);
            }
        }
        Some("key") => {
            let key_args = args.collect::<Vec<_>>();
            if key_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return 0;
            }
            if let Err(err) = crate::cli::repo::run_key_command(&key_args) {
                return crate::cli::dispatch_command_error(err);
            }
        }
        Some("org") => {
            let org_args = args.collect::<Vec<_>>();
            if org_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return 0;
            }
            if let Err(err) = crate::cli::repo::run_org_command(&org_args) {
                return crate::cli::dispatch_command_error(err);
            }
        }
        Some("token") => {
            let token_args = args.collect::<Vec<_>>();
            if token_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return 0;
            }
            if let Err(err) = crate::cli::repo::run_token_command(&token_args) {
                return crate::cli::dispatch_command_error(err);
            }
        }
        Some("audit") => {
            let audit_args = args.collect::<Vec<_>>();
            if audit_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{AUDIT_HELP}");
                return 0;
            }
            let options = match audit::parse_options(audit_args) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    return 2;
                }
            };
            return close_diagnostics(audit::run(&options));
        }
        Some("man") => {
            // Registry-driven man page: renders any package/function from its
            // descriptor metadata (intro/desc/example, params, return, errors).
            let man_args = args.collect::<Vec<_>>();
            if man_args.iter().any(|arg| is_help_flag(arg)) {
                println!("Usage: mfb man <package> [function]");
                println!();
                println!(
                    "Render a builtin package or function's man page from the descriptor registry."
                );
                return 0;
            }
            if let Err(err) = show_man(&man_args) {
                eprintln!("error: {err}");
                return 2;
            }
        }
        Some("spec") => {
            let spec_args = args.collect::<Vec<_>>();
            if spec_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{SPEC_HELP}");
                return 0;
            }
            if let Err(err) = show_spec(&spec_args) {
                eprintln!("error: {err}");
                return 2;
            }
        }
        Some("doc") => {
            let doc_args = args.collect::<Vec<_>>();
            if doc_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{DOC_HELP}");
                return 0;
            }
            return close_diagnostics(run_doc_command(&doc_args));
        }
        Some("fmt") => {
            let fmt_args = args.collect::<Vec<_>>();
            if fmt_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{FMT_HELP}");
                return 0;
            }
            return close_diagnostics(run_fmt_command(&fmt_args));
        }
        Some(command) => {
            eprintln!("error: unknown command '{command}'\n\n{USAGE}");
            return 2;
        }
    }
    // A command that completed normally may still have crossed the rendering
    // cap (warnings render too); close its stream the same way.
    close_diagnostics(0)
}

/// Close a command's diagnostic stream and answer `code`: first print how many
/// located diagnostics were withheld past `rules::MAX_RENDERED_DIAGNOSTICS`
/// (bug-505), so the developer knows the rendered set is a prefix.
fn close_diagnostics(code: i32) -> i32 {
    crate::rules::report_suppressed_diagnostics();
    code
}
