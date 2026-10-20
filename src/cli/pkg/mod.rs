//! `mfb pkg` consumer-side package commands, plus the *implementations* of the
//! publisher-side commands.
//!
//! Note the deliberate split (plan-60-A §4.1): the five publisher-side commands
//! — `publish`, `check-abi`, `release-state`, `transfer`, `transfer-accept` —
//! are **dispatched** from `mfb repo` in `super::repo`, but their
//! implementations stay here, next to the `pkg.rs`-private helpers they use
//! (`install_vendor_blobs`, `hex_bytes`). Only the command surface moved; file
//! organization is a separate concern from which word invokes a command.

use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

use crate::binary_repr;
use crate::cli::help::PKG_HELP;
use crate::doc;
use crate::manifest::package::{
    package_file_url_path, project_json_with_package, project_json_with_updated_version,
    project_json_without_packages, project_package_dependency, read_mfp_header, MfpHeader,
    ProjectPackageDependency,
};
use crate::manifest::{
    parse_project_json, project_kind, validate_packages_array, validate_project_manifest,
};
use crate::target;

use super::repo::REPO_HELP_HINT;

use super::build::{build_project, BuildOptions, Verbosity};

/// The `mfb pkg` command's terminal error. An alias of the shared
/// [`crate::cli::CommandError`] (bug-340 B2): one enum, two domain-facing names.
pub(crate) use super::CommandError as PkgCommandError;
// From a part, `super` is `cli::pkg`; part bodies spell `super::build`, `super::resolve`,
// `super::local_paths_for_repo` and the like, which live in `cli`.
use super::{
    build, commit_staged_package, confirm, install_vendor_file, local_paths_for_repo, resolve,
    stage_package_blob,
};

mod add;
mod info;
mod publish;
mod remove;
mod update;
mod verify;
pub(crate) use add::*;
pub(crate) use info::*;
pub(crate) use publish::*;
pub(crate) use remove::*;
use update::*;
pub(crate) use verify::*;

#[cfg(test)]
mod command_tests;

pub(crate) fn run_pkg_command(args: &[String]) -> Result<(), PkgCommandError> {
    match args {
        [command, rest @ ..] if command == "add" => {
            let options = parse_add_options(rest)?;
            add_package(Path::new("."), &options).map_err(PkgCommandError::Failed)
        }
        [command, package] if command == "info" => {
            print_package_info(Path::new(package)).map_err(PkgCommandError::Failed)
        }
        [command, rest @ ..] if command == "doc" => run_pkg_doc(rest),
        [command] if command == "verify" => {
            verify_packages(Path::new("."), false).map_err(PkgCommandError::Failed)
        }
        [command, flag] if command == "verify" && flag == "--proof" => {
            verify_packages(Path::new("."), true).map_err(PkgCommandError::Failed)
        }
        [command, package] if command == "validate" => {
            validate_package_file(Path::new("."), package).map_err(PkgCommandError::Failed)
        }
        [command] if command == "install" => {
            super::resolve::install(Path::new(".")).map_err(PkgCommandError::Failed)
        }
        [command, location] if command == "install" => {
            super::resolve::install(Path::new(location)).map_err(PkgCommandError::Failed)
        }
        [command, ..] if command == "install" => Err(PkgCommandError::Usage(format!(
            "mfb pkg install accepts at most one [location]\n\n{PKG_HELP}"
        ))),
        [command, rest @ ..] if command == "remove" => {
            let options = parse_remove_options(rest)?;
            run_remove(Path::new("."), &options.0, options.1).map_err(PkgCommandError::Failed)
        }
        [command, rest @ ..] if command == "update" => {
            let options = parse_update_options(rest)?;
            run_update(Path::new("."), &options).map_err(PkgCommandError::Failed)
        }
        [command, ..] if command == "validate" => Err(PkgCommandError::Usage(format!(
            "mfb pkg validate requires exactly one <package>\n\n{PKG_HELP}"
        ))),
        [command, ..] if command == "add" => Err(PkgCommandError::Usage(format!(
            "mfb pkg add requires exactly one <url>\n\n{PKG_HELP}"
        ))),
        [command, ..] if command == "info" => Err(PkgCommandError::Usage(format!(
            "mfb pkg info requires exactly one <package>\n\n{PKG_HELP}"
        ))),
        [command, ..] if command == "verify" => Err(PkgCommandError::Usage(format!(
            "mfb pkg verify accepts only the optional --proof flag\n\n{PKG_HELP}"
        ))),
        [] => Err(PkgCommandError::Usage(format!(
            "mfb pkg requires a subcommand\n\n{PKG_HELP}"
        ))),
        // The five publisher-side commands moved to `mfb repo` (plan-60-A).
        // Name the new location rather than falling through to a bare "unknown
        // pkg command": this is not an alias — it still exits 2 and does
        // nothing — but it is the difference between a five-second fix and a
        // grep through the help text.
        [command, ..]
            if matches!(
                command.as_str(),
                "publish" | "check-abi" | "release-state" | "transfer" | "transfer-accept"
            ) =>
        {
            Err(PkgCommandError::Usage(format!(
                "mfb pkg {command} has moved to mfb repo {command}\n\n{REPO_HELP_HINT}"
            )))
        }
        [command, ..] => Err(PkgCommandError::Usage(format!(
            "unknown pkg command `{command}`\n\n{PKG_HELP}"
        ))),
    }
}

#[cfg(test)]
mod tests;
