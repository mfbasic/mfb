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

/// Returns true when `arg` requests command-specific help.
fn is_help_flag(arg: &str) -> bool {
    arg == "--help" || arg == "-h"
}

pub(crate) fn run() {
    let mut args = env::args().skip(1);

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
                return;
            }
            let mut init_args = init_args.into_iter();

            let Some(location) = init_args.next() else {
                eprintln!("error: mfb init requires <location>\n\n{USAGE}");
                process::exit(2);
            };

            if init_args.next().is_some() {
                eprintln!("error: mfb init accepts exactly one <location>\n\n{USAGE}");
                process::exit(2);
            }

            if let Err(err) = init_project(Path::new(&location)) {
                eprintln!("error: {err}");
                process::exit(1);
            }
        }
        Some("init-pkg") => {
            let init_args = args.collect::<Vec<_>>();
            if init_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{INIT_PKG_HELP}");
                return;
            }
            let mut init_args = init_args.into_iter();

            let Some(location) = init_args.next() else {
                eprintln!("error: mfb init-pkg requires <location>\n\n{USAGE}");
                process::exit(2);
            };

            if init_args.next().is_some() {
                eprintln!("error: mfb init-pkg accepts exactly one <location>\n\n{USAGE}");
                process::exit(2);
            }

            if let Err(err) = init_package_project(Path::new(&location)) {
                eprintln!("error: {err}");
                process::exit(1);
            }
        }
        Some("build") => {
            let build_args = args.collect::<Vec<_>>();
            if build_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{BUILD_HELP}");
                return;
            }
            let build_options = match parse_build_options(build_args) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    process::exit(2);
                }
            };

            if let Err(()) = build_project(&build_options) {
                exit_after_diagnostics(1);
            }
        }
        Some("test") => {
            let test_args = args.collect::<Vec<_>>();
            if test_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{TEST_HELP}");
                return;
            }
            let test_options = match parse_test_options(test_args) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    process::exit(2);
                }
            };

            if let Err(()) = build_project(&test_options) {
                exit_after_diagnostics(1);
            }
        }
        Some("pkg") => {
            let pkg_args = args.collect::<Vec<_>>();
            if pkg_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{PKG_HELP}");
                return;
            }
            if let Err(err) = run_pkg_command(&pkg_args) {
                crate::cli::dispatch_command_error(err);
            }
        }
        Some("repo") => {
            let repo_args = args.collect::<Vec<_>>();
            if repo_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return;
            }
            if let Err(err) = run_repo_command(&repo_args) {
                crate::cli::dispatch_command_error(err);
            }
        }
        Some("machine") => {
            let machine_args = args.collect::<Vec<_>>();
            if machine_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return;
            }
            if let Err(err) = crate::cli::repo::run_machine_command(&machine_args) {
                crate::cli::dispatch_command_error(err);
            }
        }
        Some("key") => {
            let key_args = args.collect::<Vec<_>>();
            if key_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return;
            }
            if let Err(err) = crate::cli::repo::run_key_command(&key_args) {
                crate::cli::dispatch_command_error(err);
            }
        }
        Some("org") => {
            let org_args = args.collect::<Vec<_>>();
            if org_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return;
            }
            if let Err(err) = crate::cli::repo::run_org_command(&org_args) {
                crate::cli::dispatch_command_error(err);
            }
        }
        Some("token") => {
            let token_args = args.collect::<Vec<_>>();
            if token_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return;
            }
            if let Err(err) = crate::cli::repo::run_token_command(&token_args) {
                crate::cli::dispatch_command_error(err);
            }
        }
        Some("audit") => {
            let audit_args = args.collect::<Vec<_>>();
            if audit_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{AUDIT_HELP}");
                return;
            }
            let options = match audit::parse_options(audit_args) {
                Ok(options) => options,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    process::exit(2);
                }
            };
            exit_after_diagnostics(audit::run(&options));
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
                return;
            }
            if let Err(err) = show_man(&man_args) {
                eprintln!("error: {err}");
                process::exit(2);
            }
        }
        Some("spec") => {
            let spec_args = args.collect::<Vec<_>>();
            if spec_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{SPEC_HELP}");
                return;
            }
            if let Err(err) = show_spec(&spec_args) {
                eprintln!("error: {err}");
                process::exit(2);
            }
        }
        Some("doc") => {
            let doc_args = args.collect::<Vec<_>>();
            if doc_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{DOC_HELP}");
                return;
            }
            exit_after_diagnostics(run_doc_command(&doc_args));
        }
        Some("fmt") => {
            let fmt_args = args.collect::<Vec<_>>();
            if fmt_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{FMT_HELP}");
                return;
            }
            exit_after_diagnostics(run_fmt_command(&fmt_args));
        }
        Some(command) => {
            eprintln!("error: unknown command '{command}'\n\n{USAGE}");
            process::exit(2);
        }
    }
    // A command that completed normally may still have crossed the rendering
    // cap (warnings render too); close its stream the same way.
    crate::rules::report_suppressed_diagnostics();
}

/// Exit once a command's diagnostic stream is complete: first print how many
/// located diagnostics were withheld past `rules::MAX_RENDERED_DIAGNOSTICS`
/// (bug-505), so the developer knows the rendered set is a prefix, then exit
/// with `code`.
fn exit_after_diagnostics(code: i32) -> ! {
    crate::rules::report_suppressed_diagnostics();
    process::exit(code)
}
