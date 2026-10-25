//! `mfb build --config <name>` / `mfb test --config <name>` (plan-167-C).
//!
//! `os::getConfigOr` folds each call from the named config the build selects:
//! `--config <name>` or `--config=<name>`, or `release` without the flag. These
//! cases drive the real CLI and RUN the program, because the selection is only
//! observable in the values the built program prints. The acceptance harness
//! has no per-fixture build arguments, which is why this is a Rust test rather
//! than a golden.

#[path = "../common/mod.rs"]
mod common;
use common::{mfb_exe, temp_project};
use std::fs;
use std::path::Path;
use std::process::Command;

const SOURCE: &str = r#"IMPORT io
IMPORT os

FUNC main() AS Integer
  io::print(os::getConfigOr("apiUrl", "none"))
  io::print(toString(os::getConfigOr("retries", 0)))
  io::print(toString(os::getConfigOr("timeout", 0.0)))
  io::print(os::getConfigOr("apiKey", "no-key"))
  RETURN 0
END FUNC

TESTING
  TGROUP "config"
    TCASE "the dev config is selected"
      expectEqual(os::getConfigOr("apiUrl", "none"), "http://localhost")
      expectEqual(os::getConfigOr("retries", 0), 9)
    END TCASE
  END TGROUP
END TESTING
"#;

/// A project whose `release` and `dev` configs differ for one key of each type.
fn config_project(name: &str) -> std::path::PathBuf {
    let project = temp_project(name, SOURCE);
    fs::write(
        project.join("project.json"),
        format!(
            r#"{{
  "name": "{name}",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [{{ "root": "src", "role": "main", "include": ["**/*.mfb"] }}],
  "entry": "main",
  "config": {{
    "release": {{ "apiUrl": "https://prod", "retries": 3, "timeout": 2.5 }},
    "dev": {{ "apiUrl": "http://localhost", "retries": 9, "timeout": 0.25 }}
  }}
}}
"#
        ),
    )
    .expect("write project.json");
    project
}

/// Build `project` with `args` before the path; return (success, stdout, stderr).
fn build(project: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(mfb_exe())
        .arg("build")
        .args(args)
        .arg(project)
        .output()
        .expect("run mfb build");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Build with `args` and run the program, returning its stdout lines.
fn build_and_run(project: &Path, args: &[&str]) -> Vec<String> {
    let (ok, stdout, stderr) = build(project, args);
    assert!(ok, "build {args:?} failed:\n{stdout}\n{stderr}");
    let flavored = format!("-{}.out", common::host_libc_flavor());
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let executable = written
        .iter()
        .find(|path| path.ends_with(&flavored))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("no executable in build output:\n{stdout}"));
    let output = Command::new(executable).output().expect("run program");
    assert!(output.status.success(), "program failed");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn no_flag_selects_release() {
    let project = config_project("cfg_release");
    assert_eq!(
        build_and_run(&project, &[]),
        ["https://prod", "3", "2.50", "no-key"]
    );
    let _ = fs::remove_dir_all(&project);
}

#[test]
fn config_flag_selects_dev_in_both_forms() {
    let project = config_project("cfg_dev");
    let dev = ["http://localhost", "9", "0.25", "no-key"];
    assert_eq!(build_and_run(&project, &["--config=dev"]), dev);
    assert_eq!(build_and_run(&project, &["--config", "dev"]), dev);
    let _ = fs::remove_dir_all(&project);
}

#[test]
fn an_undeclared_config_fails_the_build() {
    let project = config_project("cfg_unknown");
    let (ok, stdout, stderr) = build(&project, &["--config=staging"]);
    assert!(!ok, "--config=staging must fail:\n{stdout}");
    let both = format!("{stdout}{stderr}");
    assert!(both.contains("2-200-0025"), "{both}");
    assert!(
        both.contains("No config named `staging`. Declared: dev, release."),
        "{both}"
    );
    assert!(!common::host_executable(&project, "cfg_unknown").exists());
    let _ = fs::remove_dir_all(&project);
}

#[test]
fn mfb_test_honors_config() {
    let project = config_project("cfg_test");
    let output = Command::new(mfb_exe())
        .arg("test")
        .arg("--config=dev")
        .arg(&project)
        .output()
        .expect("run mfb test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "mfb test --config=dev failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Without the flag the same TESTING block sees `release` and fails.
    let release = Command::new(mfb_exe())
        .arg("test")
        .arg(&project)
        .output()
        .expect("run mfb test");
    assert!(
        !release.status.success(),
        "the dev assertions must fail under release"
    );
    let _ = fs::remove_dir_all(&project);
}

/// `project-local.json` overrides `dev.apiUrl` and adds `dev.apiKey`; a
/// release build is untouched by it.
#[test]
fn a_local_file_overrides_and_extends_a_config() {
    let project = config_project("cfg_local");
    fs::write(
        project.join("project-local.json"),
        r#"{ "config": { "dev": { "apiUrl": "http://mine", "apiKey": "secret" } } }"#,
    )
    .expect("write project-local.json");
    assert_eq!(
        build_and_run(&project, &["--config=dev"]),
        ["http://mine", "9", "0.25", "secret"]
    );
    assert_eq!(
        build_and_run(&project, &[]),
        ["https://prod", "3", "2.50", "no-key"]
    );
    let _ = fs::remove_dir_all(&project);
}

/// A config that exists only in `project-local.json` is selectable.
#[test]
fn a_local_only_config_is_selectable() {
    let project = config_project("cfg_mine");
    fs::write(
        project.join("project-local.json"),
        r#"{ "config": { "mine": { "apiUrl": "http://me", "retries": 1 } } }"#,
    )
    .expect("write project-local.json");
    assert_eq!(
        build_and_run(&project, &["--config", "mine"]),
        ["http://me", "1", "0.00", "no-key"]
    );
    let _ = fs::remove_dir_all(&project);
}

/// A broken local file stops the build, naming its own path.
#[test]
fn a_bad_local_file_fails_the_build_with_its_path() {
    let project = config_project("cfg_badlocal");
    fs::write(
        project.join("project-local.json"),
        "{\n  \"name\": \"other\"\n}\n",
    )
    .expect("write project-local.json");
    let (ok, stdout, stderr) = build(&project, &[]);
    assert!(
        !ok,
        "a bad project-local.json must fail the build:\n{stdout}"
    );
    let both = format!("{stdout}{stderr}");
    assert!(both.contains("project-local.json:2"), "{both}");
    assert!(both.contains("2-200-0026"), "{both}");
    let _ = fs::remove_dir_all(&project);
}

/// A local value may replace a declared one only with the same type.
#[test]
fn a_local_type_change_fails_the_build() {
    let project = config_project("cfg_localtype");
    fs::write(
        project.join("project-local.json"),
        "{\n  \"config\": {\n    \"release\": {\n      \"retries\": \"many\"\n    }\n  }\n}\n",
    )
    .expect("write project-local.json");
    let (ok, stdout, stderr) = build(&project, &[]);
    assert!(
        !ok,
        "a type-changing override must fail the build:\n{stdout}"
    );
    let both = format!("{stdout}{stderr}");
    assert!(both.contains("project-local.json:4"), "{both}");
    assert!(both.contains("2-200-0027"), "{both}");
    let _ = fs::remove_dir_all(&project);
}

/// A source dependency's `project-local.json` is never read: a malformed one
/// in the dependency's directory does not stop the consumer's build, and the
/// dependency's call reads the app's config.
#[test]
fn a_dependency_local_file_is_never_read() {
    let project = temp_project(
        "cfg_dep_app",
        "IMPORT io\nIMPORT cfgdep\n\nFUNC main() AS Integer\n  io::print(cfgdep::value())\n  RETURN 0\nEND FUNC\n",
    );
    fs::write(
        project.join("project.json"),
        r#"{
  "name": "cfg_dep_app",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "packages": [
    { "name": "cfgdep", "version": "=0.1.0", "source": "file:packages/cfgdep", "direct": true, "requiredBy": [] }
  ],
  "entry": "main",
  "config": { "release": { "value": "from-app" } }
}
"#,
    )
    .expect("write app project.json");
    let dep = project.join("packages").join("cfgdep");
    fs::create_dir_all(dep.join("src")).expect("dependency dir");
    fs::write(
        dep.join("project.json"),
        r#"{
  "name": "cfgdep",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "package",
  "description": "a dependency with a broken project-local.json",
  "sources": [{ "root": "src", "role": "lib", "include": ["**/*.mfb"] }]
}
"#,
    )
    .expect("write dependency project.json");
    fs::write(
        dep.join("src").join("lib.mfb"),
        "IMPORT os\n\nEXPORT FUNC value() AS String\n  RETURN os::getConfigOr(\"value\", \"dep-default\")\nEND FUNC\n",
    )
    .expect("write dependency source");
    fs::write(dep.join("project-local.json"), "{ not json").expect("write broken local file");
    assert_eq!(build_and_run(&project, &[]), ["from-app"]);
    let _ = fs::remove_dir_all(&project);
}
