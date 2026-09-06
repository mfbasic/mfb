//! Tests for the backend registry and the per-OS dispatchers.
//!
//! A sibling file rather than an inline `#[cfg(test)] mod`, because an inline
//! one puts its own lines in `src/target.rs`'s coverage denominator: the
//! `cargo llvm-cov` profile merges the test binary with the never-executed
//! plain `mfb` binary, and once a `#[cfg(test)]` module appears the two copies
//! inline differently (plan tests.md, C3). The `panic!` arms of an assertion
//! helper are also never taken on a green run, so they count against the file
//! they live in forever. `scripts/coverage-exceptions.txt` already recommends
//! this move for `src/os/linux/appimage/mod.rs`.

use super::*;

mod cross_executables;

/// The app-mode-capable targets, as a `(name, supports_app_mode)` table.
/// Kept explicit rather than derived so that registering a backend, or
/// flipping one's `supports_app_mode`, fails this table loudly instead of
/// silently agreeing with itself.
const APP_MODE_MATRIX: &[(&str, bool)] = &[
    ("macos-aarch64", true),
    ("linux-aarch64", true),
    ("linux-x86_64", true),
    // rv64 is console-only: the GTK4 toolkit (`target::linux_gtk`) has not
    // been ported, so `-app` is rejected at the CLI (plan-99).
    ("linux-riscv64", false),
    // windows app mode is the Win32 transcript window (plan-66-I/J): a
    // GUI-subsystem PE hosting the program's console I/O.
    ("windows-x86_64", true),
];

#[test]
fn native_build_mode_as_str() {
    assert_eq!(NativeBuildMode::Console.as_str(), "console");
    assert_eq!(NativeBuildMode::MacApp.as_str(), "macos-app");
    assert_eq!(NativeBuildMode::LinuxApp.as_str(), "linux-app");
    assert_eq!(NativeBuildMode::WindowsApp.as_str(), "windows-app");
}

#[test]
fn native_build_mode_is_app() {
    assert!(!NativeBuildMode::Console.is_app());
    assert!(NativeBuildMode::MacApp.is_app());
    assert!(NativeBuildMode::LinuxApp.is_app());
    assert!(NativeBuildMode::WindowsApp.is_app());
}

#[test]
fn build_target_host_is_nonempty() {
    let host = BuildTarget::host();
    assert!(!host.os.is_empty());
    assert!(!host.arch.is_empty());
}

#[test]
fn build_target_name_joins_os_and_arch() {
    let target = BuildTarget {
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
    };
    assert_eq!(target.name(), "linux-x86_64");
}

#[test]
fn is_host_matches_the_running_machine() {
    assert!(BuildTarget::host().is_host());
    let other = BuildTarget {
        os: "plan9".to_string(),
        arch: "sparc".to_string(),
    };
    assert!(!other.is_host());
}

#[test]
fn parse_accepts_every_registered_target() {
    for target in registered_targets() {
        let name = target.name();
        assert_eq!(BuildTarget::parse(&name), Ok(target), "parsing {name}");
    }
}

#[test]
fn parse_splits_os_and_arch() {
    assert_eq!(
        BuildTarget::parse("macos-aarch64"),
        Ok(BuildTarget {
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
        })
    );
}

#[test]
fn parse_rejects_missing_dash() {
    let err = BuildTarget::parse("macos").unwrap_err();
    assert!(err.contains("os-arch format"), "unexpected message: {err}");
}

#[test]
fn parse_rejects_empty_os() {
    assert!(BuildTarget::parse("-aarch64").is_err());
}

#[test]
fn parse_rejects_empty_arch() {
    assert!(BuildTarget::parse("macos-").is_err());
}

#[test]
fn parse_rejects_triple_component() {
    // A third `-v7` component fails the `arch.contains('-')` guard.
    assert!(BuildTarget::parse("linux-arm-v7").is_err());
}

#[test]
fn parse_round_trips_name() {
    for target in registered_targets() {
        let name = target.name();
        let parsed = BuildTarget::parse(&name).expect("parse");
        assert_eq!(parsed.name(), name);
    }
}

#[test]
fn backend_for_resolves_every_registered_target() {
    for target in registered_targets() {
        match backend_for(&target) {
            Ok(backend) => assert_eq!(backend.target(), target),
            Err(err) => panic!("expected a backend for {}: {err}", target.name()),
        }
    }
}

/// plan-47-D..J: `windows-x86_64` is a registered, resolvable target whose
/// machine floor is live — it takes the executable/native-plan path (a
/// `RETURN 42` program compiles to a PE32+ that exits 42 on the Win11 box),
/// not the non-executable stub of 47-B Phase 2. With 47-E–J landed it now
/// advertises the full Console runtime surface (io/fs/term/thread/net/crypto/
/// tls); app mode stays off (Console-only, master §Non-goals).
#[test]
fn windows_x86_64_resolves_and_is_executable() {
    let target = BuildTarget::parse("windows-x86_64").expect("windows-x86_64 parses");
    let backend = backend_for(&target).expect("windows-x86_64 resolves (not unknown-target)");
    assert_eq!(backend.target(), target);
    let caps = backend.capabilities();
    assert!(caps.executable);
    assert!(caps.native_ir && caps.native_plan);
    assert!(caps.native_object_plan && caps.native_code_plan);
    // 47-E–J filled the runtime surface: it is no longer the empty stub wall.
    assert!(!caps.runtime_calls.is_empty());
    for family in [
        "io.print",
        "fs.readText",
        "thread.start",
        "tcp.connect",
        "crypto.randomBytes",
        "tls.connect",
    ] {
        assert!(
            caps.runtime_calls.contains(&family),
            "windows-x86_64 must advertise {family}"
        );
    }
    // plan-66-I: windows-x86_64 now supports app mode (the Win32 transcript
    // window), so `-app` is accepted at the CLI.
    assert!(target_supports_app_mode(&target));
}

#[test]
fn backend_for_unknown_target_errors() {
    let target = BuildTarget {
        os: "plan9".to_string(),
        arch: "sparc".to_string(),
    };
    match backend_for(&target) {
        Ok(_) => panic!("unexpected backend for plan9-sparc"),
        Err(err) => assert!(err.contains("plan9-sparc"), "unexpected message: {err}"),
    }
}

#[test]
fn app_mode_support_matches_the_documented_matrix() {
    for (name, expected) in APP_MODE_MATRIX {
        let target = BuildTarget::parse(name).expect("parse");
        assert_eq!(
            target_supports_app_mode(&target),
            *expected,
            "{name} app-mode support",
        );
    }
}

/// The matrix above must stay in step with the registry: a newly registered
/// backend has to be given a row rather than defaulting silently.
#[test]
fn app_mode_matrix_covers_every_registered_target() {
    for target in registered_targets() {
        let name = target.name();
        assert!(
            APP_MODE_MATRIX.iter().any(|(n, _)| *n == name),
            "{name} is registered but missing from APP_MODE_MATRIX",
        );
    }
    assert_eq!(APP_MODE_MATRIX.len(), registered_targets().len());
}

#[test]
fn unknown_target_does_not_support_app_mode() {
    let target = BuildTarget {
        os: "plan9".to_string(),
        arch: "sparc".to_string(),
    };
    assert!(!target_supports_app_mode(&target));
}

#[test]
fn registered_oses_and_arches_are_deduplicated() {
    let oses = registered_target_oses();
    let arches = registered_target_arches();
    assert!(!oses.is_empty() && !arches.is_empty());
    for (index, os) in oses.iter().enumerate() {
        assert!(!oses[..index].contains(os), "duplicate os {os}");
    }
    for (index, arch) in arches.iter().enumerate() {
        assert!(!arches[..index].contains(arch), "duplicate arch {arch}");
    }
    // Every registered target's tokens appear in the two vocabularies.
    for target in registered_targets() {
        assert!(oses.contains(&target.os), "missing os {}", target.os);
        assert!(
            arches.contains(&target.arch),
            "missing arch {}",
            target.arch
        );
    }
}

#[test]
fn registered_targets_are_unique() {
    let targets = registered_targets();
    for (index, target) in targets.iter().enumerate() {
        assert!(
            !targets[..index].contains(target),
            "duplicate registered target {}",
            target.name(),
        );
    }
}
