use super::*;

/// Fold the coverage sidecars (`coverage.covmap.json` written by the build, plus
/// `coverage.covdata`/`coverage.covfail` written by the run) into `coverage.html`
/// (plan-18-C). Best-effort: a missing sidecar warns rather than fails.
pub(super) fn generate_coverage_report(project_dir: &Path) {
    let covmap = project_dir.join(crate::testing::COVMAP_FILE);
    let Some(slots) = crate::testing::coverage::read_covmap(&covmap) else {
        eprintln!("warning: coverage map missing; skipping coverage report");
        return;
    };
    let counts =
        crate::testing::coverage::read_counts(&project_dir.join(crate::testing::COVDATA_FILE));
    let failed =
        crate::testing::coverage::read_failed(&project_dir.join(crate::testing::COVFAIL_FILE));
    let html = crate::testing::coverage::generate_html(project_dir, &slots, &counts, &failed);
    let out = project_dir.join(crate::testing::COVERAGE_HTML);
    match std::fs::write(&out, html) {
        Ok(()) => println!("Wrote coverage report to {}", out.display()),
        Err(err) => eprintln!("warning: failed to write coverage report: {err}"),
    }
}

/// A unique temporary directory for a `mfb test` executable, so the linked
/// binary never lands in the project directory. Named by process id + a
/// high-resolution timestamp; created *exclusively* (like `write_new_file`'s
/// `create_new`/`O_EXCL`) so a pre-existing or symlinked directory an attacker
/// planted at the predictable path cannot redirect where the executable is
/// written and run. Retries with a fresh suffix on `AlreadyExists`.
pub(super) fn make_temp_output_dir() -> Result<PathBuf, ()> {
    let base = std::env::temp_dir();
    let pid = std::process::id();
    for _ in 0..64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let dir = base.join(format!("mfb-test-{pid}-{nanos}"));
        // `create_dir` (not `create_dir_all`) fails atomically on an existing
        // path, so we never adopt a directory we did not create ourselves.
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                eprintln!("error: failed to create temporary test directory: {err}");
                return Err(());
            }
        }
    }
    eprintln!("error: failed to create a unique temporary test directory");
    Err(())
}

/// Pick the Linux artifact matching the libc this compiler itself targets.
/// Linux builds emit both flavors, but only the host flavor is runnable.
pub(super) fn host_test_executable(paths: &[PathBuf]) -> Option<&Path> {
    #[cfg(target_os = "linux")]
    {
        let suffix = if cfg!(target_env = "musl") {
            "-musl.out"
        } else {
            "-glibc.out"
        };
        paths
            .iter()
            .find(|path| path.to_string_lossy().ends_with(suffix))
            .map(PathBuf::as_path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        paths.first().map(PathBuf::as_path)
    }
}

/// Run the freshly built test executable, inheriting its stdio, and map its exit
/// status to `mfb test`'s result: success (all cases passed) or failure.
pub(super) fn run_test_binary(path: &Path) -> Result<(), ()> {
    let mut command = match std::env::var("MFB_TEST_RUNNER") {
        Ok(runner) => {
            let mut parts = runner.split_ascii_whitespace();
            let Some(program) = parts.next() else {
                eprintln!("error: MFB_TEST_RUNNER is empty");
                return Err(());
            };
            let mut command = std::process::Command::new(program);
            command.args(parts).arg(path);
            command
        }
        Err(_) => std::process::Command::new(path),
    };
    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(_) => Err(()),
        Err(err) => {
            eprintln!("error: failed to run test executable: {err}");
            Err(())
        }
    }
}
