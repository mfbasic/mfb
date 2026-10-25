//! Running a test's program on another machine (plan-183).
//!
//! A runtime test builds an MFBASIC program and runs it. Set `MFB_TEST_REMOTE` and the
//! same test cross-builds the program for another target, ships it over ssh and runs it
//! there, while the assertions stay on this host. That is how one test proves a
//! runtime property on the remote boxes (`.ai/remote_systems.md`), including the
//! Windows box and the emulated x86_64 one, without a Rust toolchain or a multi-hour
//! build on the box.
//!
//! `MFB_TEST_REMOTE=<target>,<ssh destination>[,<port>[,<libc>]]`, for example
//! `linux-x86_64,test@127.0.0.1,2227,musl` or `windows-x86_64,test@192.168.1.206`.
//! `<libc>` picks the Linux build flavor (default `glibc`). Unset, everything runs on
//! this host as before.
//!
//! A destination `docker:<image>` runs the program in a throwaway container of that
//! image for the target's architecture instead (`docker run --platform linux/amd64`,
//! say), under the container host's emulation when the architecture is not its own —
//! for a Linux target whose box is down. The executable is copied under
//! `~/.cache/mfb-rt`, which the container host must share (Docker Desktop and the
//! Podman machine share `/Users`).
//!
//! Only the helpers that route through here honor it: [`build_args`] for the build and
//! [`output`] for the run.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// One remote run target, parsed from `MFB_TEST_REMOTE`.
pub struct Remote {
    pub target: String,
    pub destination: String,
    pub port: String,
    pub libc: String,
}

impl Remote {
    fn windows(&self) -> bool {
        self.target.starts_with("windows")
    }

    fn docker_image(&self) -> Option<&str> {
        self.destination.strip_prefix("docker:")
    }

    /// `docker run` of `exe args…` with `envs`, in a container of `image` for the
    /// target's architecture.
    fn docker_output(
        &self,
        image: &str,
        exe: &Path,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> Output {
        let platform = match self.target.as_str() {
            "linux-x86_64" => "linux/amd64",
            "linux-aarch64" => "linux/arm64",
            "linux-riscv64" => "linux/riscv64",
            other => panic!("MFB_TEST_REMOTE: no container platform for {other}"),
        };
        let dir = PathBuf::from(std::env::var("HOME").expect("HOME"))
            .join(".cache/mfb-rt")
            .join(super::unique_nonce());
        std::fs::create_dir_all(&dir).expect("create the container share");
        let name = exe.file_name().expect("executable name");
        std::fs::copy(exe, dir.join(name)).expect("copy the executable to the share");
        let mut command = Command::new("docker");
        command.args(["run", "--rm", "--platform", platform, "-v"]);
        command.arg(format!("{}:/mfb-rt", dir.display()));
        for (key, value) in envs {
            command.arg("-e").arg(format!("{key}={value}"));
        }
        command
            .arg(image)
            .arg(format!("/mfb-rt/{}", name.to_string_lossy()))
            .args(args);
        let output = command
            .output()
            .unwrap_or_else(|e| panic!("docker run {}: {e}", exe.display()));
        let _ = std::fs::remove_dir_all(&dir);
        output
    }

    fn ssh(&self) -> Command {
        let mut command = Command::new("ssh");
        command.args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-p",
            &self.port,
            &self.destination,
        ]);
        command
    }

    /// Copy `exe` to the box once and return its path there.
    fn ship(&self, exe: &Path) -> String {
        let name = format!(
            "{}_{}",
            super::unique_nonce(),
            exe.file_name().expect("executable name").to_string_lossy()
        );
        let (dir, remote) = if self.windows() {
            ("mfb-rt".to_string(), format!("mfb-rt/{name}"))
        } else {
            ("/tmp/mfb-rt".to_string(), format!("/tmp/mfb-rt/{name}"))
        };
        let mkdir = if self.windows() {
            format!("if not exist {dir} mkdir {dir}")
        } else {
            format!("mkdir -p {dir}")
        };
        // Not checked: concurrent cases race to create it, and a missing directory
        // fails the copy below anyway.
        let _ = self.ssh().arg(mkdir).output().expect("ssh mkdir");
        let status = Command::new("scp")
            .args(["-o", "BatchMode=yes", "-P", &self.port])
            .arg(exe)
            .arg(format!("{}:{remote}", self.destination))
            .status()
            .expect("scp the executable");
        assert!(
            status.success(),
            "scp {} to {}",
            exe.display(),
            self.destination
        );
        remote
    }
}

/// The remote run target, if `MFB_TEST_REMOTE` names one.
pub fn remote() -> Option<Remote> {
    let spec = std::env::var("MFB_TEST_REMOTE").ok()?;
    let parts: Vec<&str> = spec.split(',').collect();
    assert!(
        parts.len() >= 2,
        "MFB_TEST_REMOTE={spec}: expected <target>,<ssh destination>[,<port>[,<libc>]]"
    );
    Some(Remote {
        target: parts[0].to_string(),
        destination: parts[1].to_string(),
        port: parts.get(2).copied().unwrap_or("22").to_string(),
        libc: parts.get(3).copied().unwrap_or("glibc").to_string(),
    })
}

/// Extra `mfb build` arguments: `--target <target>` for a remote run, else none.
pub fn build_args() -> Vec<String> {
    match remote() {
        Some(remote) => vec!["--target".to_string(), remote.target],
        None => Vec::new(),
    }
}

/// The libc flavor whose executable a Linux build's run should take.
pub fn libc_flavor() -> String {
    match remote() {
        Some(remote) => remote.libc,
        None => super::host_libc_flavor().to_string(),
    }
}

/// Run `exe args…` with `envs` set, here or on the remote box, and return its output.
/// Arguments and values are single tokens with no shell metacharacters.
pub fn output(exe: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let Some(remote) = remote() else {
        return Command::new(exe)
            .args(args)
            .envs(envs.iter().copied())
            .output()
            .unwrap_or_else(|e| panic!("run {}: {e}", exe.display()));
    };
    if let Some(image) = remote.docker_image() {
        return remote.docker_output(image, exe, args, envs);
    }
    let path = remote.ship(exe);
    let line = if remote.windows() {
        // PowerShell: `$env:` sets the child's environment and `$LASTEXITCODE` carries
        // its exit status out (cmd's `%ERRORLEVEL%` expands before the line runs).
        let sets: String = envs
            .iter()
            .map(|(k, v)| format!("$env:{k}='{v}'; "))
            .collect();
        format!(
            "powershell -NoProfile -Command \"{sets}& '.\\{}' {}; exit $LASTEXITCODE\"",
            path.replace('/', "\\"),
            args.join(" ")
        )
    } else {
        let sets: String = envs.iter().map(|(k, v)| format!("{k}={v} ")).collect();
        format!("chmod +x {path} && {sets}{path} {}", args.join(" "))
    };
    remote
        .ssh()
        .arg(line)
        .output()
        .unwrap_or_else(|e| panic!("ssh run {}: {e}", exe.display()))
}

/// The target the test's programs run on: `MFB_TEST_REMOTE`'s, else this host's
/// (`macos-aarch64`, `linux-x86_64`, …).
pub fn run_target() -> String {
    if let Some(remote) = remote() {
        return remote.target;
    }
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

/// Run `exe` under a POSIX shell with `shell_prefix` (`ulimit -v 1024 &&`, say) in
/// front, here or on the remote box. POSIX targets only; a container run is refused
/// (its limits are the container's own).
pub fn output_in_shell(exe: &Path, shell_prefix: &str) -> Output {
    match remote() {
        None => Command::new("sh")
            .arg("-c")
            .arg(format!("{shell_prefix} exec {}", exe.display()))
            .output()
            .unwrap_or_else(|e| panic!("sh -c {}: {e}", exe.display())),
        Some(remote) => {
            assert!(
                !remote.windows() && remote.docker_image().is_none(),
                "output_in_shell runs on a POSIX box"
            );
            let path = remote.ship(exe);
            remote
                .ssh()
                .arg(format!("chmod +x {path} && {shell_prefix} {path}"))
                .output()
                .unwrap_or_else(|e| panic!("ssh run {}: {e}", exe.display()))
        }
    }
}

/// The executable a build wrote, chosen for where it will run.
pub fn pick_executable(written: &[&str]) -> Option<PathBuf> {
    let flavored = format!("-{}.out", libc_flavor());
    written
        .iter()
        .find(|path| path.ends_with(&flavored))
        .or_else(|| written.first())
        .map(PathBuf::from)
}
