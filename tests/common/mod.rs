#![allow(dead_code)]

pub mod canvas_image;
pub mod debug_report;
pub mod ncode_emu;
mod process;
#[allow(unused_imports)] // each includer uses a subset of these helpers
pub use process::*;

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// A token unique across this process's threads, and across concurrent runs.
///
/// **Not a timestamp.** Every scratch path in this suite used to be named
/// `<prefix>_<SystemTime::now().as_nanos()>`, on the assumption that a
/// nanosecond stamp cannot repeat. It repeats constantly: the clock is far
/// coarser than its unit, and libtest starts a binary's cases on every core at
/// once. Measured on this macOS host — four threads, 200 000 samples each —
/// **698 577 of the 800 000 stamps were duplicates.**
///
/// Two cases that collide get the SAME scratch directory, and then one's
/// `remove_dir_all` deletes the other's freshly built executable, so the spawn
/// fails `NotFound`. That is what took the macOS CI row down in
/// `rt_tls_connect_allow_self_signed`:
///
/// ```text
/// spawn the mfb tls client: Os { code: 2, kind: NotFound }
/// ```
///
/// The quiet half is worse: colliding cases also overwrite each other's fixture
/// files, and nothing reports it.
///
/// A process-wide counter cannot repeat within the process, and the pid
/// separates concurrent runs on one machine — which the clock never did either.
pub fn unique_nonce() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// Render `path` as the CONTENTS of an MFB string literal.
///
/// **Never interpolate a host path into MFB source raw.** MFB reads `\` in a
/// string literal as an escape introducer, and a Windows path is nothing but
/// backslashes, so the literal
///
/// ```text
/// "C:\Users\test\AppData\Local\Temp\f.txt"
/// ```
///
/// compiles to `C:Users<TAB>estAppDataLocalTempf.txt` — `\t` is a TAB and every
/// other backslash is swallowed. Nothing errors: the program simply writes to a
/// mangled RELATIVE path while the test waits on the absolute one it asked for.
/// That is how three `http` suites reported "mfb http server never published its
/// port" on the Windows CI row while the same programs worked by hand.
///
/// Unix never noticed because its paths contain no backslashes.
pub fn mfb_path_literal(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

pub fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

pub fn build_project(project: &Path) -> PathBuf {
    let mut command = Command::new(mfb_exe());
    command.arg("build").arg(project);
    built_executable(command)
}

/// [`build_project`] at an explicit optimization level (`-O <level>`), for a
/// test that proves a program runs the same under the optimizer's higher rows.
pub fn build_project_opt(project: &Path, level: &str) -> PathBuf {
    let mut command = Command::new(mfb_exe());
    command.arg("build").arg("-O").arg(level).arg(project);
    built_executable(command)
}

/// Run a prepared `mfb build` command and return the host-runnable executable
/// it wrote.
fn built_executable(mut command: Command) -> PathBuf {
    let output = command.output().expect("run mfb build");
    assert!(
        output.status.success(),
        "build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    // A Linux build writes one executable per libc world (`-glibc.out`, then
    // `-musl.out`), and only the host's one can run: taking the first line
    // handed a musl box (Alpine) a glibc binary whose loader does not exist, so
    // every caller failed to spawn it with `NotFound`.
    let path = written
        .iter()
        .find(|path| path.ends_with(host_executable_suffix()))
        .or_else(|| written.first())
        .expect("build output executable path");
    PathBuf::from(path)
}

/// The host-runnable console executable a plain `mfb build` of project `name`
/// wrote under `project`, for a test that runs the build command itself (to
/// pass its own flags and keep its own failure message) instead of going
/// through [`build_project`].
///
/// macOS writes `build/<name>.out` and Windows `build/<name>.exe`, but a Linux
/// build writes one executable per libc world, `build/<name>-glibc.out` and
/// `build/<name>-musl.out` (`write_executable` in `src/os/linux/link/mod.rs`),
/// and no `build/<name>.out` at all: hard-coding the macOS name made every such
/// test fail on Linux with `NotFound`. Only the host's flavor can run.
pub fn host_executable(project: &Path, name: &str) -> PathBuf {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else if cfg!(target_os = "linux") {
        format!("{name}{}", host_executable_suffix())
    } else {
        format!("{name}.out")
    };
    project.join("build").join(file)
}

/// The file-name ending of the Linux console executable this host can run:
/// `-glibc.out` or `-musl.out`, after [`host_libc_flavor`]. For picking the
/// runnable one out of a build's `Wrote executable to` lines; a test that
/// matched `-glibc.out` literally ran the glibc build on a musl box, whose
/// loader does not exist there, and failed to spawn it.
pub fn host_executable_suffix() -> &'static str {
    if host_libc_flavor() == "musl" {
        "-musl.out"
    } else {
        "-glibc.out"
    }
}

/// The `mfb build` flag that puts the build in app mode for the host.
///
/// Everywhere but Linux this is plain `--app`. Linux `--app` *seals* its output
/// into one `build/<name>-<libc>.AppImage` per libc world (plan-56-B §4.4) and
/// deletes the AppDir it built them from (plan-51-C §3.3), leaving an artifact
/// that needs `/dev/fuse` and a suid `fusermount3` to launch — neither of which
/// a CI container has. `--app-debug` implies `--app` and additionally keeps the
/// AppDir (plan-51-C §4.7); the only difference in the emitted program is that
/// the intermediate survives, so a test can run the same ELF the AppImage
/// carries without FUSE.
pub const APP_BUILD_FLAG: &str = if cfg!(target_os = "linux") {
    "--app-debug"
} else {
    "-app"
};

/// Build `project` in app mode and return the executable to run.
///
/// Every headless canvas suite wants exactly this: `--app` (or its Linux
/// equivalent above), then the one runnable artifact it produced.
pub fn build_app(project: &Path, name: &str) -> PathBuf {
    build_app_with(project, name, &[])
}

/// [`build_app`] with `--debug` (plan-130-E): the build a suite needs when its run sets
/// any variable but `MFB_HEADLESS`, `MFB_CANVAS_GPU`, `MFB_CANVAS_SYNC` and
/// `MFB_MOUSE_INJECT` — `MFB_CANVAS_STATS`, `MFB_CANVAS_DUMP` and the rest have readers
/// only in a `--debug` build. A normal build ignores them.
pub fn build_app_debug(project: &Path, name: &str) -> PathBuf {
    build_app_with(project, name, &["--debug"])
}

fn build_app_with(project: &Path, name: &str, extra: &[&str]) -> PathBuf {
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg(APP_BUILD_FLAG)
        .args(extra)
        .arg(project)
        .output()
        .unwrap_or_else(|err| panic!("run mfb build {APP_BUILD_FLAG}: {err}"));
    assert!(
        output.status.success(),
        "mfb build {APP_BUILD_FLAG} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    app_binary(project, name)
}

/// The executable an app-mode build of `name` left under `project`.
///
/// macOS puts it inside an `.app` bundle, Linux inside the AppDir `--app-debug`
/// kept, and Windows writes it beside `build/`.
pub fn app_binary(project: &Path, name: &str) -> PathBuf {
    let bundled = project
        .join("build")
        .join(format!("{name}.app"))
        .join("Contents")
        .join("MacOS")
        .join(name);
    if bundled.exists() {
        return bundled;
    }
    if cfg!(target_os = "linux") {
        return project
            .join("build")
            .join(format!("{name}-{}.AppDir", host_libc_flavor()))
            .join("usr")
            .join("bin")
            .join(name);
    }
    let plain = project.join("build").join(name);
    if plain.exists() {
        return plain;
    }
    project
        .join("build")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// Which of the two Linux libc worlds this host can actually load.
///
/// A build always emits both flavors and only one of them runs here, so the
/// question is which loader — and which system libraries — this box has.
///
/// **glibc wins whenever it is present, and a musl loader alone does not settle
/// it.** Installing `musl-tools` to cross-compile puts `/lib/ld-musl-x86_64.so.1`
/// on an otherwise ordinary glibc box, which is exactly what CI's musl matrix row
/// does — it builds the compiler for musl and runs it on a glibc Ubuntu runner.
/// Reading that loader as "this host is musl" picks the musl AppDir, whose
/// `libgtk-4.so.1` and `libgio-2.0.so.0` are the distribution's glibc builds and
/// do not resolve:
///
///   Error loading shared library libgtk-4.so.1: No such file or directory
///
/// So ask for glibc first and fall back to musl, which is the honest reading of
/// an Alpine box: it has the musl loader and no `ld-linux*` at all, and the test
/// boxes deliberately carry no `gcompat` shim that would blur the two.
pub fn host_libc_flavor() -> &'static str {
    let has = |dir: &str, prefix: &str| {
        std::fs::read_dir(dir).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
        })
    };
    // The glibc loader is `ld-linux-<arch>.so.N`, in /lib or /lib64 depending on
    // the distribution's multiarch layout.
    if has("/lib", "ld-linux") || has("/lib64", "ld-linux") {
        return "glibc";
    }
    if has("/lib", "ld-musl-") {
        return "musl";
    }
    "glibc"
}

/// How a child process ended, phrased for a failure message.
///
/// `ExitStatus::code()` answers `None` for a child killed by a signal, so a
/// message built from it alone reports `exited None` for a SIGSEGV, a SIGABRT
/// and an OOM-killer SIGKILL alike — three different bugs with one indication.
/// On Unix the signal number is available and is the whole diagnosis, so say it.
pub fn exit_description(status: &ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exit {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            let name = match signal {
                2 => " (SIGINT)",
                4 => " (SIGILL)",
                6 => " (SIGABRT)",
                8 => " (SIGFPE)",
                9 => " (SIGKILL — the OOM killer or an outside kill, not a crash)",
                10 => " (SIGBUS)",
                11 => " (SIGSEGV)",
                13 => " (SIGPIPE)",
                15 => " (SIGTERM)",
                _ => "",
            };
            return format!("killed by signal {signal}{name}");
        }
    }
    format!("ended with no exit code ({status:?})")
}

/// Build `project` with `-ncode -target <target>` and return the parsed
/// `<name>.ncode` dump as JSON.
pub fn build_ncode(project: &Path, target: &str, name: &str) -> serde_json::Value {
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg("-ncode")
        .arg("-target")
        .arg(target)
        .arg(project)
        .output()
        .expect("run mfb build -ncode");
    assert!(
        output.status.success(),
        "mfb build -ncode -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let path = project.join(format!("{name}.ncode"));
    let text = fs::read_to_string(&path).expect("read ncode dump");
    serde_json::from_str(&text).expect("parse ncode json")
}

/// [`build_ncode`] at an explicit optimization level (`-O <level>`), for a test
/// that compares what one row changes between two levels of the same source.
pub fn build_ncode_opt(project: &Path, target: &str, name: &str, level: &str) -> serde_json::Value {
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg("-ncode")
        .arg("-O")
        .arg(level)
        .arg("-target")
        .arg(target)
        .arg(project)
        .output()
        .expect("run mfb build -ncode -O");
    assert!(
        output.status.success(),
        "mfb build -ncode -O {level} -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let path = project.join(format!("{name}.ncode"));
    let text = fs::read_to_string(&path).expect("read ncode dump");
    serde_json::from_str(&text).expect("parse ncode json")
}

/// [`build_ncode`] for an `--app` build: the dump then carries the target's app
/// bootstrap (window procedures, key handlers), which a console build has none of.
pub fn build_ncode_app(project: &Path, target: &str, name: &str) -> serde_json::Value {
    build_ncode_app_with(project, target, name, &[])
}

/// [`build_ncode_app`] with extra build flags — `--debug`, for the plan of a build
/// that carries the test hooks.
pub fn build_ncode_app_with(
    project: &Path,
    target: &str,
    name: &str,
    extra: &[&str],
) -> serde_json::Value {
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg("-q")
        .arg("-ncode")
        .arg("--app")
        .args(extra)
        .arg("-target")
        .arg(target)
        .arg(project)
        .output()
        .expect("run mfb build -ncode --app");
    assert!(
        output.status.success(),
        "mfb build -ncode --app -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let path = project.join(format!("{name}.ncode"));
    let text = fs::read_to_string(&path).expect("read ncode dump");
    serde_json::from_str(&text).expect("parse ncode json")
}

/// Build `project` for a Linux `target` (`-q`) and return the bytes of the
/// produced console ELF. Console builds emit one flavored executable per libc
/// world; either is fine for a header check (they share the ELF layout).
/// plan-46-D §4.1: the build emits into the project's `build/` directory.
pub fn build_linux_elf(project: &Path, target: &str, name: &str) -> Vec<u8> {
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg("-q")
        .arg("-target")
        .arg(target)
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "mfb build -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let out_dir = project.join("build");
    let glibc = out_dir.join(format!("{name}-glibc.out"));
    let musl = out_dir.join(format!("{name}-musl.out"));
    let path = if glibc.exists() { glibc } else { musl };
    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

/// bug-452 codegen oracle. A raw indirect `blr` is an unstaged external C call:
/// its result lives in the platform C-return bank (`rax`/`c_return(0)`), NOT the
/// aligned MFB-return bank (`aligned_reg` — `rdi` on SysV, `rcx` on Win64) that a
/// direct `bl` stages into. Reading the result from `aligned_reg` after a raw
/// `blr` picks up the stale first argument — the bug-452 defect (identical to the
/// bug-450 crypto instance). This walks every function whose symbol contains
/// `sym_needle`, and for each `blr` scans the window until the aligned register is
/// next *written* (arg-setup for the next call, or a `sxtw reg,c_return` funnel),
/// asserting no instruction in that window *reads* `aligned_reg` (as a `src`/`lhs`
/// operand). A correct fix reads results from `c_return(0)`, so on aarch64 (where
/// both banks are `x0`) the count is trivially zero and on x86-64 it is zero only
/// once every result read is repointed. `min_blr_sites` guards the fixture from
/// silently ceasing to exercise the external-call path (an inert guard).
pub fn assert_no_aligned_bank_result_reads(
    ncode: &serde_json::Value,
    aligned_reg: &str,
    sym_needle: &str,
    min_blr_sites: usize,
) {
    let functions = ncode["functions"]
        .as_array()
        .expect("ncode has a functions array");
    let mut blr_sites = 0usize;
    let mut inspected_fns = 0usize;
    for func in functions {
        let sym = func["symbol"].as_str().unwrap_or("");
        if !sym.contains(sym_needle) {
            continue;
        }
        let insts = func["instructions"]
            .as_array()
            .expect("function has an instructions array");
        inspected_fns += 1;
        let mut i = 0usize;
        while i < insts.len() {
            if insts[i]["op"].as_str() != Some("blr") {
                i += 1;
                continue;
            }
            blr_sites += 1;
            let mut j = i + 1;
            while j < insts.len() {
                let op = insts[j]["op"].as_str().unwrap_or("");
                if op == "bl" || op == "blr" {
                    break;
                }
                for field in ["src", "lhs"] {
                    let reads = insts[j].get(field).and_then(|v| v.as_str()) == Some(aligned_reg);
                    assert!(
                        !reads,
                        "{sym}: an external `blr` call result is read from `{aligned_reg}` \
                         (the aligned MFB-return bank) instead of the C-return bank — \
                         bug-452 regression. Consumer: {}",
                        insts[j]
                    );
                }
                // The result is live in `aligned_reg` only until it is overwritten:
                // a fresh arg-load, or a `sxtw aligned_reg, c_return` funnel that
                // re-homes the correct result there. After that write, later reads
                // are of a new value, not the stale call result.
                if insts[j].get("dst").and_then(|v| v.as_str()) == Some(aligned_reg) {
                    break;
                }
                j += 1;
            }
            i = if j > i { j } else { i + 1 };
        }
    }
    assert!(
        inspected_fns > 0,
        "no function symbol contained `{sym_needle}` — the oracle inspected nothing"
    );
    assert!(
        blr_sites >= min_blr_sites,
        "expected at least {min_blr_sites} external `blr` call sites under `{sym_needle}`, \
         found {blr_sites} — the fixture no longer exercises the external-call path, \
         so the guard is inert"
    );
}

/// bug-452, Win64 audio (WASAPI) variant. Unlike SysV, Win64 external calls are
/// NOT staged into the aligned bank (`win_x86_64::code::emit_external_call` passes
/// the Windows target so the shared emitter skips the `%retC`→aligned `mov`), so
/// BOTH the direct IAT calls (`ole_call`, a `bl`) and the COM vtable calls
/// (`com_call`, a `blr`) leave their HRESULT/DWORD result in `rax` (`c_return(0)`),
/// not the aligned bank (`rcx`). Each such call sign-extends its result in place —
/// `sxtw rcx, rcx` — which sign-extends the STALE first argument / `this` pointer,
/// not the real status; the fix reads the result from `c_return(0)`
/// (`sxtw rcx, rax`). This asserts no `sxtw` sign-extends the aligned bank into
/// itself (the exact bug shape); `min_sxtw_sites` guards against the fixture no
/// longer exercising the sign-extended external-call path.
pub fn assert_no_inplace_sxtw_of_aligned_bank(
    ncode: &serde_json::Value,
    aligned_reg: &str,
    sym_needle: &str,
    min_sxtw_sites: usize,
) {
    let functions = ncode["functions"]
        .as_array()
        .expect("ncode has a functions array");
    let mut sxtw_sites = 0usize;
    let mut inspected_fns = 0usize;
    for func in functions {
        let sym = func["symbol"].as_str().unwrap_or("");
        if !sym.contains(sym_needle) {
            continue;
        }
        inspected_fns += 1;
        let insts = func["instructions"]
            .as_array()
            .expect("function has an instructions array");
        for inst in insts {
            if inst["op"].as_str() != Some("sxtw") {
                continue;
            }
            sxtw_sites += 1;
            let src = inst.get("src").and_then(|v| v.as_str());
            let dst = inst.get("dst").and_then(|v| v.as_str());
            assert!(
                !(src == Some(aligned_reg) && dst == Some(aligned_reg)),
                "{sym}: an external call result is sign-extended in place in the \
                 aligned bank (`sxtw {aligned_reg}, {aligned_reg}`) — the stale \
                 argument, not the C-return bank result — bug-452 regression. \
                 Instruction: {inst}"
            );
        }
    }
    assert!(
        inspected_fns > 0,
        "no function symbol contained `{sym_needle}` — the oracle inspected nothing"
    );
    assert!(
        sxtw_sites >= min_sxtw_sites,
        "expected at least {min_sxtw_sites} `sxtw` sites under `{sym_needle}`, \
         found {sxtw_sites} — the fixture no longer exercises the sign-extended \
         external-call path, so the guard is inert"
    );
}

// ---- repo_acceptance shared helpers (bug-327 T1-6) ----
use mfb_repository::crypto;
use mfb_repository::store::Store;
use std::io::{BufRead, BufReader};
use std::process::Child;
use std::sync::Once;

static BUILD_REPO: Once = Once::new();
static BUILD_RELEASE_MFB: Once = Once::new();

pub struct RepoProcess {
    pub child: Child,
    pub url: String,
}

impl Drop for RepoProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The compiler binary the golden/acceptance path drives. plan-67-A: this MUST
/// resolve a **release** build. plan-67 (B–F) injects debug-gated runtime perf
/// tracking into every compiled program whenever the *compiler* is built with
/// `debug_assertions` on; a debug compiler would then make every acceptance /
/// `rt_*` program print a perf table at exit (and every `.ncode` dump gain the
/// injected calls), turning the goldens to noise. Release has `debug_assertions`
/// off, so the injection is inert. Pure-Rust unit tests (`src/**` `#[cfg(test)]`)
/// never call this and stay on the default (debug) profile, so the compiler's
/// internal `debug_assertions` invariant checks still run under `cargo test`.
///
/// Resolution order: an explicit `MFB_TEST_EXE` override (so CI / a golden-regen
/// command can hand in the release binary it already built) → the `release`
/// sibling of `CARGO_BIN_EXE_mfb` if it already exists → a nested
/// `cargo build --release --bin mfb` performed once (mirrors `repo_exe()`'s
/// on-demand build, bug-347). The target directory is derived from
/// `CARGO_BIN_EXE_mfb` so a custom `CARGO_TARGET_DIR` is honored.
pub fn mfb_exe() -> String {
    if let Ok(exe) = std::env::var("MFB_TEST_EXE") {
        return exe;
    }
    let debug =
        std::env::var("CARGO_BIN_EXE_mfb").unwrap_or_else(|_| "target/debug/mfb".to_string());
    let debug_path = PathBuf::from(&debug);
    // `.../target/<profile>/mfb` → `.../target/release/mfb`.
    let profile_dir = debug_path
        .parent()
        .expect("mfb binary has a parent directory");
    let target_dir = profile_dir.parent().expect("profile dir sits under target");
    let release = target_dir
        .join("release")
        .join(format!("mfb{}", std::env::consts::EXE_SUFFIX));

    // Always delegate the up-to-date decision to Cargo — never skip on mere
    // existence. A pre-existing `target/release/mfb` is NOT proof it matches the
    // current source: a binary left over from an earlier checkout silently makes
    // every release-driven test build with stale codegen. That produces false
    // failures (a fix that landed after the binary was built looks un-applied)
    // and, far worse, false passes (a regression in current source is masked by
    // an older-but-correct binary). `cargo build --release` is a fast no-op when
    // the artifact is already current and rebuilds it when it is not, so it is
    // the only sound staleness authority here. `call_once` keeps it to one
    // invocation per test process; Cargo's own build lock serializes any race
    // between concurrently-running test binaries.
    BUILD_RELEASE_MFB.call_once(|| {
        let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cmd.args(["build", "--release", "--bin", "mfb"])
            .arg("--target-dir")
            .arg(target_dir);
        let status = cmd.status().expect("build release mfb");
        assert!(status.success(), "release mfb build failed");
    });
    assert!(
        release.exists(),
        "release mfb missing at {}",
        release.display()
    );
    release.to_string_lossy().into_owned()
}

// bug-347: `repository` is a workspace member, so `cargo test` builds `mfb-repo`
// into the *shared* target dir alongside this test's own binaries and we just
// use it. `CARGO_BIN_EXE_mfb-repo` is not usable here: Cargo defines that only
// for integration tests of the package that declares the bin, and this test
// belongs to `mfb`. Deriving the directory from `mfb`'s own bin path instead
// stays correct under `--release` and a custom `CARGO_TARGET_DIR`.
//
// The fallback covers `cargo test --test repo_acceptance`, which selects only
// `mfb` and so does not build another member's bin. Unlike the pre-bug-347
// version, this build shares the workspace target dir and profile, so it cannot
// disagree with the binary the rest of the suite uses.
pub fn repo_exe() -> String {
    let mfb = std::path::PathBuf::from(mfb_exe());
    let bin_dir = mfb
        .parent()
        .expect("mfb binary has a parent directory")
        .to_path_buf();
    let exe = bin_dir.join(format!("mfb-repo{}", std::env::consts::EXE_SUFFIX));

    BUILD_REPO.call_once(|| {
        if exe.exists() {
            return;
        }
        let target_dir = bin_dir.parent().expect("target dir above the profile dir");
        let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cmd.args(["build", "-p", "mfb_repository", "--bin", "mfb-repo"])
            .arg("--target-dir")
            .arg(target_dir);
        if bin_dir.file_name().is_some_and(|n| n == "release") {
            cmd.arg("--release");
        }
        let status = cmd.status().expect("build mfb-repo");
        assert!(status.success(), "mfb-repo build failed");
    });

    assert!(exe.exists(), "mfb-repo missing at {}", exe.display());
    exe.to_string_lossy().into_owned()
}

pub fn start_repo(repo_dir: &std::path::Path) -> RepoProcess {
    let mut child = Command::new(repo_exe())
        .args([
            "--dbpath",
            repo_dir.join("meta.db").to_str().unwrap(),
            "--datapath",
            repo_dir.join("packages").to_str().unwrap(),
            "--listen",
            "127.0.0.1:0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start mfb-repo");

    let stdout = child.stdout.take().expect("repo stdout");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("repo listen line");
    let address = line
        .trim()
        .strip_prefix("MFB_REPO_LISTEN=")
        .expect("repo listen prefix");
    RepoProcess {
        child,
        url: format!("http://{address}"),
    }
}

pub fn open_store(repo_dir: &std::path::Path) -> mfb_repository::store::OpenedRepository {
    Store::open_repository(&repo_dir.join("meta.db"), &repo_dir.join("packages"))
        .expect("open repository store")
}

/// The local key/session store the CLI uses for this repository: MFB_HOME
/// scoped by the SHA-256 of the repository URL (`~/.mfb/<repo-hash>/`).
pub fn mfb_repo_home(repo: &RepoProcess, home: &std::path::Path) -> std::path::PathBuf {
    home.join(".mfb")
        .join(crypto::fingerprint(repo.url.as_bytes()))
}

pub fn run_mfb(repo: &RepoProcess, home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(mfb_exe())
        .args(args)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.join(".mfb"))
        .output()
        .expect("run mfb")
}

pub fn run_mfb_plain(args: &[&str]) -> std::process::Output {
    Command::new(mfb_exe())
        .args(args)
        .output()
        .expect("run mfb")
}

/// `run_mfb`, but from a chosen working directory — needed to exercise the
/// commands whose path argument defaults to `.` (plan-60-A §4.2).
pub fn run_mfb_in(
    repo: &RepoProcess,
    home: &std::path::Path,
    cwd: &std::path::Path,
    args: &[&str],
) -> std::process::Output {
    Command::new(mfb_exe())
        .args(args)
        .current_dir(cwd)
        .env("MFB_REPO_URL", &repo.url)
        .env("MFB_HOME", home.join(".mfb"))
        .output()
        .expect("run mfb")
}

/// Whether an emitted instruction's base register is the STACK pointer, on either
/// architecture.
///
/// Codegen-inspection tests read store or load ORDER out of an `.ncode` plan, and
/// traffic to the frame is not traffic to the structure under test — a spill has
/// nothing to do with the ordering being pinned. Excluding it by register name is
/// right; excluding it by **one** name is not. The stack pointer prints as `sp` on
/// AArch64 and as `rsp` on x86-64, so a filter written against `sp` alone is blind on
/// half the targets these tests iterate.
///
/// Both directions of that blindness have been observed in this repo, and the second is
/// the dangerous one:
///
/// * a **false failure** — `rt_canvas_present_deep_copy`'s `removeGroup` pin saw
///   `+176` as its first "store" on Linux x86-64, an offset that cannot be a group-slot
///   word at all, and failed for a reason unrelated to the emitter;
/// * a **false pass** — a predicate of the shape *"some `str_u64` to offset N whose
///   base is not `sp`"* is satisfied on x86-64 by any spill that happens to use that
///   offset, so the assertion succeeds without the store it is looking for ever
///   existing.
///
/// Use this rather than comparing against a spelling.
pub fn is_stack_base(instruction: &serde_json::Value) -> bool {
    matches!(instruction["base"].as_str(), Some("sp") | Some("rsp"))
}

/// The same test, for a base register already extracted as a string.
pub fn is_stack_register(name: &str) -> bool {
    name == "sp" || name == "rsp"
}

/// The instructions of one function's code plan that lie OFF its inline-raise
/// error exits — for an owner count that means "one owner on the normal path".
///
/// bug-695 made an inline raise (a checked `+`, a failing conversion, an
/// allocation failure) leave the function the way a failing call does: it frees
/// the statement's pending temps at the raise site and branches to a landing pad
/// that drops every live cleanup. Those drops run INSTEAD of the normal-path ones,
/// never besides them, so counting them as extra owners is wrong. Two regions are
/// skipped:
///
/// * a raise site's exit: from the `_mfb_rt_park_error` call that directly follows
///   `_mfb_make_error_result` up to and including the first `ret` or unconditional
///   `b` (stopping at the FIRST one can only keep too much, so a count errs high
///   and fails, never low and passes);
/// * the landing pads, all placed after the function body: everything from the
///   first `error_return_pad*` label on.
pub fn outside_inline_error_exits(instructions: &[serde_json::Value]) -> Vec<&serde_json::Value> {
    let mut kept = Vec::new();
    let mut in_exit = false;
    let mut previous_target: Option<&str> = None;
    for instruction in instructions {
        let op = instruction["op"].as_str();
        let target = instruction["target"].as_str();
        if op == Some("label")
            && instruction["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("error_return_pad"))
        {
            break;
        }
        if in_exit {
            if matches!(op, Some("ret") | Some("b")) {
                in_exit = false;
            }
        } else if target == Some("_mfb_rt_park_error")
            && previous_target == Some("_mfb_make_error_result")
        {
            in_exit = true;
        } else {
            kept.push(instruction);
        }
        previous_target = if op == Some("bl") { target } else { None };
    }
    kept
}

/// A minimal TrueType face, for tests that must construct a `canvas::Text`.
///
/// Shared rather than duplicated because plan-116-I made a font *resource* the thing a
/// `Text` item holds: a fixture can no longer fabricate `FontRef[id := 0]` and must
/// load a real face, so more than one test file needs these bytes.
///
/// It is a stub — its `A` and `B` glyphs are solid boxes — which is exactly what a
/// coverage assertion wants: a glyph that either renders as a filled rectangle or does
/// not render at all, with nothing in between to argue about.
/// The twelve-glyph fixture font, as bytes.
pub fn fixture_truetype() -> Vec<u8> {
    const B64: &str = concat!(
        "AAEAAAAGAAAAAAAAY21hcAAAAAAAAABsAAAANGdseWYAAAAAAAAAoAAAACJoZWFkAAAAAAAAAMIA",
        "AAA2aGhlYQAAAAAAAAD4AAAAJGhtdHgAAAAAAAABHAAAAAxsb2NhAAAAAAAAASgAAAAIAAAAAQAD",
        "AAoAAAAMAAwAAAAAACgAAAAAAAAAAgAAAEEAAABBAAAAAQAAAEIAAABCAAAAAgABAGQAAAGQASwA",
        "AwAAAQEBAQBkASwAAP7UAAAAAAEsAAAAAAAAAAAAAAAAAAAAAAAAAAAD6AAAAAAAAAAAAAAAAAAA",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAyD/OABkAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAMB",
        "9AAAAPoAAAEsAAAAAAAAABEAEQ==",
    );
    let table: Vec<u8> =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".to_vec();
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for ch in B64.bytes() {
        if ch == b'=' {
            break;
        }
        let v = table
            .iter()
            .position(|&c| c == ch)
            .expect("base64 alphabet") as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

// --- Cross-process port gate (bug-488) --------------------------------------

/// Machine-wide gate for the "bind an ephemeral port, release it, hand the
/// number to a child that binds it" window.
///
/// That window is unavoidable when the child is an external program that takes a
/// port on its command line (`openssl s_server`), and it is a race: between the
/// release and the child's `bind(2)`, anything on the machine may take the port.
///
/// The gate this replaces was a `static OnceLock<Mutex<()>>` — **per-process**.
/// It serialized the cases inside one test binary and nothing else, so it could
/// not serialize against another test binary in the same `cargo test`, nor
/// against a second `cargo test` on the same machine. The latter is routine
/// here: work runs in several `.claude/worktrees/*` sessions at once, and
/// `rt_tls_connect_allow_self_signed` produced six false failures that way
/// during one work pass — in BOTH directions (accepting a peer it should have
/// refused, and refusing one it should have accepted), which is the signature of
/// two runs sharing state rather than one stale value leaking.
///
/// `flock(2)` fixes exactly that: the lock lives on a file in the temp dir, so
/// every process on the machine contends for the same one. It also covers
/// threads within a process — each `acquire` opens its own file description, and
/// `flock` locks per description, so two threads contend as two processes would.
///
/// Held for the whole bind→release→child-listening window; released on drop.
/// Unix-only: `flock(2)` has no Windows equivalent here, and none is needed — the
/// one consumer, `rt_tls_connect_allow_self_signed`, is itself
/// `#![cfg(any(target_os = "macos", target_os = "linux"))]` (its peer is the
/// `openssl` CLI). Left ungated, this module is compiled into EVERY test binary,
/// so `libc::flock`/`AsRawFd` took the whole `windows-x86_64` row down at COMPILE
/// time — 8 errors, 0 of 123 binaries run.
#[cfg(unix)]
pub struct PortGate(fs::File);

#[cfg(unix)]
impl PortGate {
    /// Blocks until every other holder on this machine releases.
    pub fn acquire() -> Self {
        use std::os::unix::io::AsRawFd;
        let path = std::env::temp_dir().join("mfb-test-port-gate.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .unwrap_or_else(|e| panic!("open the port gate at {}: {e}", path.display()));
        // EINTR is the only retryable error here; anything else means the gate
        // is unusable and serializing silently-not-at-all is how this bug
        // survived six sightings, so fail loudly instead.
        loop {
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if rc == 0 {
                break;
            }
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                panic!("flock the port gate: {err}");
            }
        }
        Self(file)
    }
}

#[cfg(unix)]
impl Drop for PortGate {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}
