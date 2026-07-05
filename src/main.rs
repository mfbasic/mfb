mod arch;
mod ast;
mod audit;
mod binary_repr;
mod builtins;
mod cli;
mod doc;
mod docs;
mod escape;
mod fmt;
mod internal_name;
mod ir;
mod lexer;
mod manifest;
mod monomorph;
mod numeric;
mod os;
mod resolver;
mod rules;
mod target;
mod syntaxcheck;
mod unicode_backend;
mod unicode_runtime_tables;

use std::env;
use std::path::Path;
use std::process;
use tinyjson::JsonValue;

use cli::build::{build_project, parse_build_options};
use cli::doc::run_doc_command;
use cli::fmt::run_fmt_command;
use cli::init::{init_package_project, init_project};
use cli::man::show_man;
use cli::pkg::{run_pkg_command, PkgCommandError};
use cli::repo::{run_repo_command, RepoCommandError};
use cli::spec::show_spec;

pub(crate) const USAGE: &str = "\
Usage: mfb <command> [arguments]

Project Setup:
  init <path>             Create a new MFBASIC executable project
  init-pkg <path>         Create a new MFBASIC package project

Package Management:
  pkg add <url>           Add a compiled package to the current project
  pkg info <pkg>          Show information about a compiled package
  pkg verify              Verify packages declared in project.json
  pkg publish <own> <pkg> Publish a signed package project

Repository & Auth:
  repo register <owner>   Register a repository owner
  repo auth <owner>       Authenticate as a repository owner

Build & Development:
  build [options] [path]  Validate and build an MFBASIC project
  fmt [options] [path]    Format project source (indentation/capitalization)
  audit [options] [path]  Report security and code audit findings

Documentation & Reference:
  doc [options] [path]    Render HTML docs from package or file source
  pkg doc <pkg> [options] Render HTML docs from a compiled package
  man [pkg] [func]        Show built-in package and function help
  spec [topic] [sub]      Show the MFBASIC language specification
  help                    Show this message

Run 'mfb <command> --help' for more information on a specific command.";

pub(crate) const INIT_HELP: &str = "\
Usage: mfb init <path>

Create a new MFBASIC executable project at the specified path.

Arguments:
  <path>      The directory where the project will be initialized.";

pub(crate) const INIT_PKG_HELP: &str = "\
Usage: mfb init-pkg <path>

Create a new MFBASIC package project (library) at the specified path.

Arguments:
  <path>      The directory where the project will be initialized.";

pub(crate) const PKG_HELP: &str = "\
Usage: mfb pkg <command> [arguments]

Commands:
  add <url>           Add a compiled package as a dependency
  info <pkg>          Show metadata and dependencies of a compiled package
  verify              Check project.json dependencies against local cache
  publish <own> <pkg> Sign and publish the current project to a repository
  doc <pkg>           Generate HTML documentation from a compiled package

Options for 'publish':
  --owner <name>      Explicitly set the owner name for signing";

pub(crate) const REPO_HELP: &str = "\
Usage: mfb repo <command> <owner>

Commands:
  register            Register a new repository owner identity
  auth                Log in to an existing owner account

Arguments:
  <owner>             The unique handle for the repository owner";

pub(crate) const BUILD_HELP: &str = "\
Usage: mfb build [options] [path]

Validate and compile an MFBASIC project.

Arguments:
  [path]              Path to the project (default: current directory)

Options:
  --sign <owner>      Sign the resulting binary with the specified owner
  --target <os-arch>  Cross-compile to a specific target (e.g., linux-x64)
  --app               Build as a standalone application instead of a library
  --unsigned          Allow unsigned dependencies from a non-local source

Debug/Inspection (Emits intermediate output):
  -ast                Outputs Abstract Syntax Tree
  -ir                 Outputs Intermediate Representation
  -br                 Outputs MFPC binary representation
  -mir                Outputs Mid-level IR
  -nir                Outputs native IR
  -nplan              Outputs the execution plan
  -ncode              Outputs native code output";

pub(crate) const FMT_HELP: &str = "\
Usage: mfb fmt [options] [path]

Format MFBASIC source files for consistent indentation and capitalization.

Options:
  --check             Check if files are formatted without writing changes
  --indent <N>        Set the number of spaces for indentation (default: 2)

Arguments:
  [path]              File or directory to format (default: current directory)";

pub(crate) const AUDIT_HELP: &str = "\
Usage: mfb audit [options] [path]

Scan the project for security vulnerabilities and code smells.

Options:
  --format <type>     Output format: text, json (default: text)
  --locked            Only audit packages defined in project.lock";

pub(crate) const DOC_HELP: &str = "\
Usage: mfb doc [options] [path]

Render HTML documentation from source files or a project directory.

Options:
  --out <file>        Path to the generated HTML file (default: index.html)

Arguments:
  [path]              Source file or project folder to document";

pub(crate) const MAN_HELP: &str = "\
Usage: mfb man [package] [function]

Display the built-in manual for packages and specific functions.

Example:
  mfb man standard print";

pub(crate) const SPEC_HELP: &str = "\
Usage: mfb spec [topic] [subtopic] [options]

Display the formal MFBASIC language specification.

Options:
  --all               Print the entire specification to the console

Example:
  mfb spec types integer";

/// Returns true when `arg` requests command-specific help.
pub(crate) fn is_help_flag(arg: &str) -> bool {
    arg == "--help" || arg == "-h"
}

fn main() {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        Some("help") | None => {
            println!("{USAGE}");
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
                process::exit(1);
            }
        }
        Some("pkg") => {
            let pkg_args = args.collect::<Vec<_>>();
            if pkg_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{PKG_HELP}");
                return;
            }
            if let Err(err) = run_pkg_command(&pkg_args) {
                match err {
                    PkgCommandError::Usage(message) => {
                        eprintln!("error: {message}");
                        process::exit(2);
                    }
                    PkgCommandError::Failed(message) => {
                        eprintln!("error: {message}");
                        process::exit(1);
                    }
                }
            }
        }
        Some("repo") => {
            let repo_args = args.collect::<Vec<_>>();
            if repo_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{REPO_HELP}");
                return;
            }
            if let Err(err) = run_repo_command(&repo_args) {
                match err {
                    RepoCommandError::Usage(message) => {
                        eprintln!("error: {message}");
                        process::exit(2);
                    }
                    RepoCommandError::Failed(message) => {
                        eprintln!("error: {message}");
                        process::exit(1);
                    }
                }
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
            process::exit(audit::run(&options));
        }
        Some("man") => {
            let man_args = args.collect::<Vec<_>>();
            if man_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{MAN_HELP}");
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
            process::exit(run_doc_command(&doc_args));
        }
        Some("fmt") => {
            let fmt_args = args.collect::<Vec<_>>();
            if fmt_args.iter().any(|arg| is_help_flag(arg)) {
                println!("{FMT_HELP}");
                return;
            }
            process::exit(run_fmt_command(&fmt_args));
        }
        Some(command) => {
            eprintln!("error: unknown command '{command}'\n\n{USAGE}");
            process::exit(2);
        }
    }
}

pub(crate) fn json_string(value: &str) -> String {
    JsonValue::String(value.to_string())
        .stringify()
        .unwrap_or_else(|_| "\"mfb_project\"".to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tinyjson::JsonValue;

    use crate::ast;
    use crate::cli::build::{parse_build_options, BuildOutput};
    use crate::cli::init::project_manifest;
    use crate::cli::pkg::{
        package_dependency_status, package_verify_line, package_version_matches,
        verify_package_dependency, PackageVerifyResult, PackageVerifyStatus,
    };
    use crate::manifest::entry::validate_entry_point;
    use crate::manifest::package::{project_json_with_package, ProjectPackageDependency};
    use crate::manifest::{parse_project_json, validate_project_manifest};

    #[test]
    fn parse_build_options_defaults_to_console_mode() {
        let options = parse_build_options(vec!["some/project".to_string()]).expect("options");
        assert!(!options.app_mode);
    }

    #[test]
    fn parse_build_options_accepts_app_flag() {
        let options = parse_build_options(vec!["-app".to_string(), "some/project".to_string()])
            .expect("options");
        assert!(options.app_mode);
    }

    #[test]
    fn parse_build_options_rejects_duplicate_app_flag() {
        let result = parse_build_options(vec!["-app".to_string(), "-app".to_string()]);
        match result {
            Err(err) => assert!(err.contains("at most one -app")),
            Ok(_) => panic!("duplicate -app must be rejected"),
        }
    }

    #[test]
    fn parse_build_options_app_flag_composes_with_native_output() {
        let options =
            parse_build_options(vec!["-app".to_string(), "-nir".to_string()]).expect("options");
        assert!(options.app_mode);
        assert_eq!(options.outputs, vec![BuildOutput::NativeIr]);
    }

    #[test]
    fn parse_build_options_combines_output_flags_in_order() {
        let options = parse_build_options(vec![
            "-ast".to_string(),
            "-ir".to_string(),
            "-ncode".to_string(),
            "-mir".to_string(),
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

    #[test]
    fn parse_build_options_rejects_duplicate_output_flag() {
        let result = parse_build_options(vec!["-ncode".to_string(), "-ncode".to_string()]);
        match result {
            Err(err) => assert!(err.contains("duplicate output flag `-ncode`")),
            Ok(_) => panic!("duplicate output flag must be rejected"),
        }
    }

    #[test]
    fn parse_build_options_no_output_flags_means_full_build() {
        let options = parse_build_options(vec!["some/project".to_string()]).expect("options");
        assert!(options.outputs.is_empty());
    }

    #[test]
    fn package_add_manifest_insert_creates_packages_array() {
        let contents = concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"sources\": [{ \"root\": \"src\" }]\n",
            "}\n"
        );
        let manifest = parse_project_json(contents, Path::new("project.json")).expect("manifest");
        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.0.0".to_string(),
            pin: true,
            source: "file:///tmp/source/shape.mfp".to_string(),
        };

        let updated =
            project_json_with_package(contents, &manifest, &dependency).expect("updated manifest");

        assert!(updated.contains("\"packages\": ["));
        assert!(updated.contains("\"name\": \"shape\""));
        assert!(updated.contains("\"ident\": \"ada#shape\""));
        assert!(updated.contains("\"version\": \"1.0.0\""));
        assert!(updated.contains("\"pin\": true"));
        assert!(updated.contains("\"source\": \"file:///tmp/source/shape.mfp\""));
        assert!(updated.parse::<JsonValue>().is_ok());
    }

    #[test]
    fn package_add_manifest_append_preserves_json_array_format() {
        let contents = concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"sources\": [{ \"root\": \"src\" }],\n",
            "  \"packages\": [\n",
            "    {\n",
            "      \"name\": \"math\",\n",
            "      \"ident\": \"std#math\",\n",
            "      \"version\": \"1.0.0\",\n",
            "      \"pin\": true,\n",
            "      \"source\": \"file:packages/math.mfp\"\n",
            "    }\n",
            "  ]\n",
            "}\n"
        );
        let manifest = parse_project_json(contents, Path::new("project.json")).expect("manifest");
        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.0.0".to_string(),
            pin: true,
            source: "file:///tmp/source/shape.mfp".to_string(),
        };

        let updated =
            project_json_with_package(contents, &manifest, &dependency).expect("updated manifest");

        assert!(updated.contains("    },\n    {\n      \"name\": \"shape\""));
        assert!(updated.parse::<JsonValue>().is_ok());
    }

    #[test]
    fn package_verify_status_checks_name_and_version() {
        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.2.3".to_string(),
            pin: true,
            source: "registry:mfb".to_string(),
        };
        assert_eq!(
            package_dependency_status(&dependency, "shape", "ada#shape", "1.2.3"),
            PackageVerifyStatus::Ok
        );
        assert_eq!(
            package_dependency_status(&dependency, "shape", "ada#shape", "1.2.4"),
            PackageVerifyStatus::NeedsUpdate
        );
        assert_eq!(
            package_dependency_status(&dependency, "color", "ada#shape", "1.2.3"),
            PackageVerifyStatus::InvalidPackage
        );
        assert_eq!(
            package_dependency_status(&dependency, "shape", "other#shape", "1.2.3"),
            PackageVerifyStatus::InvalidPackage
        );
    }

    #[test]
    fn package_verify_rejects_range_syntax_as_literal_version() {
        assert!(!package_version_matches("^1.2.3", "1.9.0"));
        assert!(!package_version_matches("~1.2.3", "1.2.9"));
        assert!(package_version_matches("1.2.3", "1.2.3"));
    }

    #[test]
    fn package_verify_line_shows_project_and_package_versions() {
        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.2.0".to_string(),
            pin: false,
            source: "registry:mfb".to_string(),
        };

        assert_eq!(
            package_verify_line(
                &dependency,
                &PackageVerifyResult {
                    version: "1.2.3".to_string(),
                    status: PackageVerifyStatus::Ok,
                }
            ),
            "shape @ 1.2.0 : OK (1.2.3)"
        );
        assert_eq!(
            package_verify_line(
                &dependency,
                &PackageVerifyResult {
                    version: String::new(),
                    status: PackageVerifyStatus::InvalidPackage,
                }
            ),
            "shape @ 1.2.0 : Invalid Package"
        );
    }

    #[test]
    fn package_verify_reads_source_package_manifest() {
        let root = test_temp_dir("package_verify_reads_source_package_manifest");
        let package_dir = root.join("packages").join("shape");
        fs::create_dir_all(&package_dir).expect("package dir");
        fs::write(
            package_dir.join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"shape\",\n",
                "  \"ident\": \"ada#shape\",\n",
                "  \"version\": \"1.2.3\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"package\",\n",
                "  \"sources\": [{ \"root\": \"src\" }]\n",
                "}\n"
            ),
        )
        .expect("package manifest");

        let dependency = ProjectPackageDependency {
            name: "shape".to_string(),
            ident: "ada#shape".to_string(),
            version: "1.2.3".to_string(),
            pin: false,
            source: "registry:mfb".to_string(),
        };

        assert_eq!(
            verify_package_dependency(&root, &dependency),
            PackageVerifyResult {
                version: "1.2.3".to_string(),
                status: PackageVerifyStatus::Ok,
            }
        );

        fs::remove_dir_all(root).expect("remove temp dir");
    }

    #[test]
    fn validate_entry_point_rejects_multiple_matching_declarations() {
        let root = test_temp_dir("validate_entry_point_rejects_multiple_matching_declarations");
        let project_dir = root.join("app");
        let src_dir = project_dir.join("src");
        fs::create_dir_all(&src_dir).expect("src dir");
        fs::write(
            project_dir.join("project.json"),
            project_manifest(&project_dir),
        )
        .expect("project manifest");
        fs::write(src_dir.join("main_a.mfb"), "SUB main()\nEND SUB\n").expect("main_a");
        fs::write(
            src_dir.join("main_b.mfb"),
            "FUNC main(args AS List OF String) AS Integer\n  RETURN 0\nEND FUNC\n",
        )
        .expect("main_b");

        let manifest_contents =
            fs::read_to_string(project_dir.join("project.json")).expect("manifest contents");
        let manifest = parse_project_json(&manifest_contents, &project_dir.join("project.json"))
            .expect("manifest");
        let ast = ast::parse_project("app", &project_dir, &manifest).expect("ast");

        assert!(validate_entry_point(&project_dir, &manifest, &ast).is_err());

        fs::remove_dir_all(root).expect("remove temp dir");
    }

    #[test]
    fn validate_project_manifest_rejects_missing_kind() {
        let root = test_temp_dir("validate_project_manifest_rejects_missing_kind");
        let project_dir = root.join("app");
        fs::create_dir_all(&project_dir).expect("project dir");
        fs::write(
            project_dir.join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"sources\": [{ \"root\": \"src\" }]\n",
                "}\n"
            ),
        )
        .expect("project manifest");

        assert!(validate_project_manifest(&project_dir.join("project.json")).is_err());

        fs::remove_dir_all(root).expect("remove temp dir");
    }

    fn test_temp_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "mfb_{name}_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("temp dir");
        root
    }
}
