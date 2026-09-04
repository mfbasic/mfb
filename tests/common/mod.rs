#![allow(dead_code)]

pub mod canvas_image;

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
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
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let path = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output executable path");
    PathBuf::from(path)
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
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg(APP_BUILD_FLAG)
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

/// Spawn `executable`, capture stdout, and wait up to `timeout`. On timeout the
/// child is killed and the test panics — `hang_context` names the specific hang
/// each caller guards against (e.g. a reintroduced linear `^` loop). Poll
/// interval is 25 ms; it is immaterial against these multi-second timeouts.
///
/// The child runs with its own directory as cwd. Without this it inherited the
/// test process's cwd — the crate root — so any fixture doing relative file I/O
/// (`fs::writeBytes("payload.bin", …)`) wrote **into the repository** and left
/// the file behind when the test failed early. Anchoring cwd to the executable's
/// directory keeps that I/O inside the temp project the caller already deletes.
pub fn run_bounded(
    executable: &Path,
    timeout: Duration,
    hang_context: &str,
) -> (ExitStatus, String) {
    let mut command = Command::new(executable);
    if let Some(dir) = executable.parent() {
        command.current_dir(dir);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn executable");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            let mut stdout = String::new();
            if let Some(mut pipe) = child.stdout.take() {
                pipe.read_to_string(&mut stdout).ok();
            }
            return (status, stdout);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "executable {} did not finish within {timeout:?} — {hang_context}",
                executable.display(),
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub fn run_capture_with_env(executable: &Path, envs: &[(&str, String)]) -> (i32, String, String) {
    let mut command = Command::new(executable);
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("run executable");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        String::from_utf8(output.stderr).expect("utf8 stderr"),
    )
}

/// Build a `close()` interposer that fails the close of a chosen fd, so a test
/// can observe *that* the runtime closed a resource (and how it reports a close
/// failure). Unix-only by construction: the shim is a `SYS_close` raw-syscall
/// shared object injected with `LD_PRELOAD` / `DYLD_INSERT_LIBRARIES`, and
/// Windows has neither the syscall interface nor a loader-level symbol-preload
/// mechanism to hang it on. Callers are `#[cfg(unix)]` for the same reason.
#[cfg(unix)]
pub fn build_close_interposer(root: &Path) -> PathBuf {
    let source = root.join("fail_close.c");
    fs::write(
        &source,
        r#"
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>
#if !defined(__APPLE__)
#include <stdio.h>
#endif

static void mfb_marker_text(const char *text) {
  const char *marker = getenv("MFB_INTERPOSER_MARKER");
  if (marker && marker[0]) {
    int fd = open(marker, O_WRONLY | O_CREAT | O_APPEND, 0600);
    if (fd >= 0) {
      write(fd, text, strlen(text));
      syscall(SYS_close, fd);
    }
  }
}

__attribute__((constructor)) static void mfb_marker(void) {
  const char *marker = getenv("MFB_INTERPOSER_MARKER");
  if (marker && marker[0]) {
    int fd = open(marker, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd >= 0) {
      syscall(SYS_close, fd);
    }
  }
  mfb_marker_text("loaded\n");
}

static int should_fail_close(int fd) {
  const char *target = getenv("MFB_FAIL_CLOSE_PATH");
  if (target && target[0]) {
    if (strcmp(target, "*") == 0 && fd > 2) {
      mfb_marker_text("fail\n");
      errno = EIO;
      return 1;
    }
    char path[4096];
#if defined(__APPLE__)
    if (fcntl(fd, F_GETPATH, path) == 0 && strcmp(path, target) == 0) {
      errno = EIO;
      return 1;
    }
#else
    char link_path[64];
    snprintf(link_path, sizeof(link_path), "/proc/self/fd/%d", fd);
    ssize_t len = readlink(link_path, path, sizeof(path) - 1);
    if (len >= 0) {
      path[len] = '\0';
      if (strcmp(path, target) == 0) {
        errno = EIO;
        return 1;
      }
    }
#endif
  }
  return 0;
}

static long mfb_close(int fd) {
  if (should_fail_close(fd)) {
    return -1L;
  }
  return syscall(SYS_close, fd);
}

#if defined(__APPLE__)
typedef struct {
  const void *replacement;
  const void *replacee;
} interpose_t;
__attribute__((used)) static const interpose_t interposers[] __attribute__((section("__DATA,__interpose"))) = {
  { (const void *)mfb_close, (const void *)close }
};
#else
int close(int fd) {
  return (int)mfb_close(fd);
}
#endif
"#,
    )
    .expect("write close interposer source");

    let library = if cfg!(target_os = "macos") {
        root.join("libfail_close.dylib")
    } else {
        root.join("libfail_close.so")
    };
    let mut command = Command::new("cc");
    if cfg!(target_os = "macos") {
        command.args(["-dynamiclib", "-o"]);
    } else {
        command.args(["-shared", "-fPIC", "-o"]);
    }
    command.arg(&library).arg(&source);
    if !cfg!(target_os = "macos") {
        command.arg("-ldl");
    }
    let output = command.output().expect("compile close interposer");
    assert!(
        output.status.success(),
        "interposer build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    library
}

pub fn run_with_stdin(executable: &Path, stdin: &[u8]) -> String {
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Pipe stderr too: otherwise the child inherits the test harness's fd 2,
        // which is a tty when `cargo test` runs in an interactive terminal, and
        // `io::isErrorTerminal()` then reports TRUE instead of the expected FALSE.
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn executable");
    let mut child_stdin = child.stdin.take().expect("stdin pipe");
    child_stdin.write_all(stdin).expect("write stdin");
    drop(child_stdin);
    let output = child.wait_with_output().expect("wait for executable");
    assert!(
        output.status.success(),
        "program failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
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

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// The Python 3 interpreter that drives the helper scripts below. `python3` is
/// the canonical name on Unix and is also what GitHub's Windows images put on
/// PATH, but a stock python.org install on Windows ships only `python.exe` — so
/// probe rather than fail with a bare "program not found". The Microsoft Store
/// `python.exe` *alias stub* answers the probe with a non-zero exit, so checking
/// the status (not just that the spawn succeeded) rejects it.
pub fn python_exe() -> &'static str {
    static NAME: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        for candidate in ["python3", "python"] {
            let ok = Command::new(candidate)
                .arg("--version")
                .output()
                .is_ok_and(|out| out.status.success());
            if ok {
                return candidate;
            }
        }
        "python3"
    })
}

pub fn decode_hex(value: &str) -> Vec<u8> {
    fn nibble(byte: u8) -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => panic!("invalid hex byte {byte}"),
        }
    }

    let bytes = value.as_bytes();
    assert_eq!(bytes.len() % 2, 0, "hex output must have even length");
    bytes
        .chunks_exact(2)
        .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
        .collect()
}

/// Run `executable` with stdout pointed at a **read-only** descriptor (the null
/// device opened `O_RDONLY`). Any real write to fd 1 then fails deterministically
/// on every platform — unlike a *closed* fd, a valid-but-read-only descriptor
/// cannot be silently reopened or replaced by the runtime/loader, so this is the
/// portable way to exercise the stdout-write failure path (bug-04: `io::flush` /
/// `io::input` detect failures via `write`, not `fsync`).
///
/// The read-only descriptor is handed to `Popen` as the child's stdout directly
/// rather than dup'd over fd 1 from a `preexec_fn`: `preexec_fn` is a fork-only
/// hook that raises `ValueError` on Windows, and passing the descriptor is
/// exactly equivalent (the POSIX form built a stdout pipe only to immediately
/// replace it, so the child's stdout was never observable either way — hence the
/// empty stdout line below).
pub fn run_with_readonly_stdout(executable: &Path, stdin: &[u8]) -> (i32, String, String) {
    let output = Command::new(python_exe())
        .arg("-c")
        .arg(
            r#"import binascii, os, subprocess, sys
stdin_data = bytes.fromhex(sys.argv[2])

readonly_stdout = os.open(os.devnull, os.O_RDONLY)
try:
    proc = subprocess.Popen(
        [sys.argv[1]],
        stdin=subprocess.PIPE,
        stdout=readonly_stdout,
        stderr=subprocess.PIPE,
    )
finally:
    os.close(readonly_stdout)
_, err = proc.communicate(stdin_data)
sys.stdout.write(str(proc.returncode) + "\n")
sys.stdout.write("\n")
sys.stdout.write(binascii.hexlify(err).decode("ascii") + "\n")"#,
        )
        .arg(executable)
        .arg(hex(stdin))
        .output()
        .expect("run readonly-stdout helper");

    assert!(
        output.status.success(),
        "readonly-stdout helper failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf8 helper output");
    let mut lines = stdout.lines();
    let status = lines
        .next()
        .expect("status line")
        .parse::<i32>()
        .expect("status code");
    let child_stdout =
        String::from_utf8(decode_hex(lines.next().expect("stdout line"))).expect("utf8 stdout");
    let child_stderr =
        String::from_utf8(decode_hex(lines.next().expect("stderr line"))).expect("utf8 stderr");
    (status, child_stdout, child_stderr)
}

pub fn run_under_pty(executable: &Path) -> String {
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"import fcntl, os, pty, select, struct, subprocess, sys, termios, time
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 100, 0, 0))
# Hold a spare slave handle in the parent for the whole drain. Without it the
# only slave references are the child's, so the pty is torn down the instant the
# child exits -- and on macOS/BSD a master read after the last slave closes
# returns EIO *and discards whatever the tty still had queued*. This child writes
# three short lines and exits immediately, so on a loaded machine it routinely
# beat the parent's first read and the helper returned zero bytes ("expected tty
# output, got []"). Keeping a slave open means the tty survives the child, so the
# queued bytes stay readable and the loop below ends on child exit rather than on
# an EOF that races it.
keep = os.dup(slave)
proc = subprocess.Popen([sys.argv[1]], stdin=slave, stdout=slave, stderr=slave, close_fds=True)
os.close(slave)
chunks = []
deadline = time.time() + 30.0
while True:
    ready, _, _ = select.select([master], [], [], 0.2)
    if ready:
        try:
            data = os.read(master, 4096)
        except OSError:
            data = b""
        if data:
            chunks.append(data)
            continue
    if proc.poll() is not None:
        # The child is gone and cannot write again; sweep the tty buffer dry.
        while True:
            ready, _, _ = select.select([master], [], [], 0.0)
            if not ready:
                break
            try:
                data = os.read(master, 4096)
            except OSError:
                data = b""
            if not data:
                break
            chunks.append(data)
        break
    if time.time() > deadline:
        proc.kill()
        sys.stderr.write("timed out waiting for the pty child to exit\n")
        sys.exit(124)
os.close(keep)
os.close(master)
sys.stdout.buffer.write(b"".join(chunks))
sys.exit(proc.wait())"#,
        )
        .arg(executable)
        .output()
        .expect("run pty helper");

    assert!(
        output.status.success(),
        "pty run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

pub fn run_pty_prompt_interaction(executable: &Path, prompt: &str, input: &[u8]) -> String {
    run_pty_prompt_interaction_inner(executable, prompt, input, false)
}

/// Like `run_pty_prompt_interaction`, but for the echo-suppressing reads
/// (`readChar`/`readByte`/`readLine`): after the prompt appears, wait until the
/// child has actually cleared the terminal `ECHO` flag before injecting input.
///
/// The child writes its prompt with echo still on (the prompt is a *separate*
/// `io::write` statement — the runtime can't know an echo-suppressed read
/// follows), then enters raw mode inside the read. Injecting the reply the
/// instant the prompt is visible races that `tcsetattr`: if the byte lands while
/// echo is still on, the kernel line discipline echoes it and the assertion
/// flakes (deterministically under `cargo llvm-cov`, which perturbs scheduling).
/// Gating the write on `ECHO` being cleared — the child is then blocked in
/// `read()` — closes the window without a timing hack.
pub fn run_pty_prompt_interaction_echo_off(
    executable: &Path,
    prompt: &str,
    input: &[u8],
) -> String {
    run_pty_prompt_interaction_inner(executable, prompt, input, true)
}

pub fn run_pty_prompt_interaction_inner(
    executable: &Path,
    prompt: &str,
    input: &[u8],
    wait_echo_off: bool,
) -> String {
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"import fcntl, os, pty, select, subprocess, sys, termios, time
prompt = sys.argv[2].encode()
reply = bytes.fromhex(sys.argv[3])
wait_echo_off = sys.argv[4] == "1"
master, slave = pty.openpty()
# Keep a spare handle to the slave: it lets us read the child's termios after it
# closes its own copies, and it keeps the pty alive across the child's exit so
# the drain below cannot lose queued output (see that loop). Held until the very
# end -- the drain therefore ends on child exit, not on an EOF it would race.
echo_probe = os.dup(slave)
proc = subprocess.Popen([sys.argv[1]], stdin=slave, stdout=slave, stderr=slave, close_fds=True)
os.close(slave)
chunks = []
seen = b""
deadline = time.time() + 5.0
while prompt not in seen:
    remaining = deadline - time.time()
    if remaining <= 0:
        proc.kill()
        sys.stderr.write("timed out waiting for prompt; saw %r\n" % seen)
        sys.exit(124)
    ready, _, _ = select.select([master], [], [], remaining)
    if not ready:
        continue
    data = os.read(master, 4096)
    if not data:
        break
    chunks.append(data)
    seen += data
if wait_echo_off:
    echo_deadline = time.time() + 5.0
    while True:
        try:
            lflag = termios.tcgetattr(echo_probe)[3]
        except OSError:
            break
        if not (lflag & termios.ECHO):
            break
        if time.time() > echo_deadline:
            proc.kill()
            sys.stderr.write("timed out waiting for child to disable echo\n")
            sys.exit(124)
        time.sleep(0.001)
os.write(master, reply)
# `echo_probe` stays open through this drain (it is the spare slave handle), for
# the same reason `run_under_pty` holds one: once the child's are the only slave
# references left, its exit tears the pty down and a macOS/BSD master read then
# returns EIO *and drops whatever was still queued*. Every caller here asserts on
# output the child prints immediately before exiting, so that tail is exactly
# what would be lost. Ending the loop on child exit instead of on EOF removes the
# race; the sweep afterwards takes whatever the tty still holds.
deadline = time.time() + 30.0
while True:
    ready, _, _ = select.select([master], [], [], 0.2)
    if ready:
        try:
            data = os.read(master, 4096)
        except OSError:
            data = b""
        if data:
            chunks.append(data)
            continue
    if proc.poll() is not None:
        while True:
            ready, _, _ = select.select([master], [], [], 0.0)
            if not ready:
                break
            try:
                data = os.read(master, 4096)
            except OSError:
                data = b""
            if not data:
                break
            chunks.append(data)
        break
    if time.time() > deadline:
        proc.kill()
        sys.stderr.write("timed out waiting for process exit\n")
        sys.exit(124)
os.close(echo_probe)
os.close(master)
sys.stdout.buffer.write(b"".join(chunks))
sys.exit(proc.wait())"#,
        )
        .arg(executable)
        .arg(prompt)
        .arg(hex(input))
        .arg(if wait_echo_off { "1" } else { "0" })
        .output()
        .expect("run pty prompt helper");

    assert!(
        output.status.success(),
        "pty prompt run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 pty prompt output")
}

/// Build a `write()` interposer that caps every call to 4096 bytes, forcing the
/// short *positive* returns that bug-51's output loops must survive. macOS routes
/// `write` through libSystem, so a `__DATA,__interpose` shim reaches the mfb
/// binary; linux-x86_64 issues a raw `write` syscall that no libc interposer can
/// hook, so this validation is macOS-only.
#[cfg(target_os = "macos")]
pub fn build_short_write_interposer(root: &Path) -> PathBuf {
    let source = root.join("short_write.c");
    fs::write(
        &source,
        r#"
#include <stdlib.h>
#include <sys/syscall.h>
#include <unistd.h>

static long mfb_short_write(int fd, const void *buf, unsigned long n) {
  unsigned long cap = n > 4096 ? 4096 : n;
  return syscall(SYS_write, fd, buf, (size_t)cap);
}

typedef struct {
  const void *replacement;
  const void *replacee;
} interpose_t;
__attribute__((used)) static const interpose_t interposers[] __attribute__((section("__DATA,__interpose"))) = {
  { (const void *)mfb_short_write, (const void *)write }
};
"#,
    )
    .expect("write short-write interposer source");
    let library = root.join("libshort_write.dylib");
    let output = Command::new("cc")
        .args(["-dynamiclib", "-o"])
        .arg(&library)
        .arg(&source)
        .output()
        .expect("compile short-write interposer");
    assert!(
        output.status.success(),
        "interposer build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    library
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

/// `run_bounded`, plus the child's **peak resident set size** in bytes.
///
/// bug-510: the decoder-bound tests assert that a program's memory stays a small
/// multiple of its input, and `Command::output` cannot say what a child's peak
/// was. `wait4(2)` can — `ru_maxrss` is filled for the *specific* child reaped —
/// which is also why `getrusage(RUSAGE_CHILDREN)` is the wrong tool here: it
/// folds every terminated child in, and the `mfb build` that produced the
/// executable is a child of this process too. The child is reaped with
/// `WNOHANG` polls under the same deadline discipline as `run_bounded`, because a
/// bomb's failure mode is "still running" and a blocking wait would sit there for
/// as long as the bomb lasts. `ru_maxrss` is bytes on macOS and kibibytes on
/// Linux; the result is normalised to bytes. Non-Unix hosts get `None` for the
/// size and the caller skips the memory assertion.
pub fn run_bounded_with_rss(
    executable: &Path,
    timeout: Duration,
    hang_context: &str,
) -> (ExitStatus, String, Option<u64>) {
    let mut command = Command::new(executable);
    if let Some(dir) = executable.parent() {
        command.current_dir(dir);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn executable");
    // Drain stdout on its own thread: a child that fills the pipe while we only
    // poll for exit would block forever, which reads as a hang.
    let mut pipe = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut out = String::new();
        pipe.read_to_string(&mut out).ok();
        out
    });
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let pid = child.id() as libc::pid_t;
        let start = Instant::now();
        loop {
            let mut status: libc::c_int = 0;
            let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
            let reaped = unsafe { libc::wait4(pid, &mut status, libc::WNOHANG, &mut usage) };
            if reaped == pid {
                let stdout = reader.join().expect("stdout reader");
                let raw = usage.ru_maxrss as u64;
                let bytes = if cfg!(target_os = "macos") { raw } else { raw * 1024 };
                return (ExitStatus::from_raw(status), stdout, Some(bytes));
            }
            if reaped < 0 {
                panic!("wait4({pid}) failed: {}", std::io::Error::last_os_error());
            }
            if start.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "executable {} did not finish within {timeout:?} — {hang_context}",
                    executable.display(),
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    #[cfg(not(unix))]
    {
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().expect("try_wait") {
                let stdout = reader.join().expect("stdout reader");
                return (status, stdout, None);
            }
            if start.elapsed() > timeout {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "executable {} did not finish within {timeout:?} — {hang_context}",
                    executable.display(),
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
