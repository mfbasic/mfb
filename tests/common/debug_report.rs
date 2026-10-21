//! Build, run and read the `--debug` report (plan-130): shared by `rt_debug_arena.rs`
//! and `rt_debug_soak.rs` (plan-133-A).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Build `project` with `--debug` and return the executable (the host libc's on Linux).
pub fn build_debug_project(name: &str, project: &Path) -> PathBuf {
    build_debug_project_with(name, project, &[])
}

/// [`build_debug_project`] with extra `mfb build` arguments (an `-O <level>`, say)
/// placed between `--debug` and the project path.
pub fn build_debug_project_with(name: &str, project: &Path, extra: &[&str]) -> PathBuf {
    let output = Command::new(super::mfb_exe())
        .arg("build")
        .arg("--debug")
        .args(extra)
        .arg(project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed to build:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    // A Linux build writes `-glibc.out` and `-musl.out`; only the host's one runs.
    let flavored = format!("-{}.out", super::host_libc_flavor());
    let chosen = written
        .iter()
        .find(|path| path.ends_with(&flavored))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    PathBuf::from(chosen)
}

/// Build `source` as a one-file project with `--debug`.
pub fn build_debug(name: &str, source: &str) -> PathBuf {
    let project = super::temp_project(name, source);
    build_debug_project(name, &project)
}

/// Run `exe`, require success, and return `(stdout, stderr)`.
pub fn run_ok(name: &str, exe: &Path) -> (String, String) {
    let output = Command::new(exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{name} failed:\n{stdout}\n{stderr}"
    );
    (stdout, stderr)
}

/// The `arena.` lines of the (last) report block on `stderr`.
pub fn arena_lines(case: &str, stderr: &str) -> Vec<String> {
    let at = stderr
        .rfind("mfb.debug.begin ")
        .unwrap_or_else(|| panic!("{case}: no report block on stderr:\n{stderr}"));
    stderr[at..]
        .lines()
        .take_while(|line| *line != "mfb.debug.end 1")
        .filter(|line| line.starts_with("arena."))
        .map(str::to_string)
        .collect()
}

/// `arena.<index>.<counter>` of `lines`.
pub fn counter(case: &str, lines: &[String], index: usize, name: &str) -> u64 {
    let key = format!("arena.{index}.{name} ");
    lines
        .iter()
        .find_map(|line| line.strip_prefix(&key))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{case}: no `{key}<n>` in {lines:?}"))
}
