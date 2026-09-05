//! bug-535: a `RES` bind of a built-in resource received off a thread channel,
//! with no other call into that resource's package, failed the build with
//!
//! ```text
//! error: NIR declares unused runtime helper 'tcp'
//! ```
//!
//! — no code, no location, and the name of an internal data structure.
//!
//! `validate_nir` requires the module's declared runtime helpers and its *used*
//! ones to agree in both directions. A plain resource bind drops through a
//! close op codegen emits at scope exit, not through an NIR call, so nothing
//! populated the used set for it. Until `thread::accept` existed, every program
//! holding a resource also called into its package and was counted through that
//! call, which is why the hole survived: adding any `tcp::` call anywhere makes
//! the same program build.
//!
//! The shape is exactly what the `thread` package intro recommends — "a server
//! may accept on one thread and hand each connection to a worker" — reduced to
//! the worker side.
//!
//! These are build-only and host-targeted (the program is built for this
//! machine and, in one case, run). `tests/rt_thread_accept_res_drop_closes.rs`
//! carries the runtime half: that the accepted handle is live and that its
//! scope drop really closes it.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"mfb_project\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\
         \"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

/// Build `source` for the host and return `Ok(executable)` or the compiler's
/// combined output.
fn build(name: &str, source: &str) -> Result<PathBuf, String> {
    let project = temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!("stdout:\n{stdout}\nstderr:\n{stderr}"));
    }
    Ok(PathBuf::from(
        stdout
            .lines()
            .find_map(|line| line.strip_prefix("Wrote executable to "))
            .unwrap_or_else(|| panic!("no executable path in build output:\n{stdout}")),
    ))
}

/// The reproduction: a worker that receives a `RES <type_>` off its channel and
/// lets the binding drop, with no other call into `package` anywhere.
fn worker_program(package: &str, type_: &str, bind_type: &str) -> String {
    format!(
        "IMPORT io\n\
         IMPORT thread\n\
         IMPORT {package}\n\n\
         TYPE Progress\n\
        \x20 pos AS Integer\n\
         END TYPE\n\n\
         ISOLATED FUNC worker(t AS ThreadWorker OF RES {type_} TO Integer, n AS Integer) AS Integer\n\
        \x20 RES s AS {bind_type} = thread::accept(t, 1000)\n\
        \x20 RETURN 1\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
        \x20 LET a AS Thread OF RES {type_} TO Integer = thread::start(worker, 0)\n\
        \x20 io::print(\"started\")\n\
        \x20 RETURN 0\n\
         END FUNC\n"
    )
}

/// Every built-in resource `thread::accept` can deliver. The two confirmed in
/// the bug report were `tcp` and `tls` only because those are what the reporter
/// tried; the defect is per-package and all six failed identically.
const SENDABLE_RESOURCES: &[(&str, &str)] = &[
    ("tcp", "tcp::Socket"),
    ("tcp", "tcp::Listener"),
    ("tls", "tls::Socket"),
    ("tls", "tls::Listener"),
    ("udp", "udp::Socket"),
    ("fs", "fs::File"),
];

#[test]
fn a_worker_binding_an_accepted_resource_builds_for_every_sendable_resource() {
    for (package, type_) in SENDABLE_RESOURCES {
        let name = type_.replace("::", "_").to_lowercase();
        let source = worker_program(package, type_, type_);
        if let Err(err) = build(&format!("b535_{name}"), &source) {
            panic!(
                "a worker whose only `{package}::` reference is a `RES {type_}` bind \
                 off thread::accept must build (bug-535), got:\n{err}"
            );
        }
    }
}

#[test]
fn a_stateful_accepted_resource_bind_builds() {
    // plan-74's requirement, on the plain-resource path: a `STATE T` suffix
    // names the same resource as the bare form and must resolve to the same
    // close helper. A textual match on the declared type would miss it and
    // reintroduce bug-535 in its stateful form.
    let source = worker_program("fs", "fs::File", "fs::File STATE Progress");
    if let Err(err) = build("b535_fs_file_state", &source) {
        panic!("a `RES f AS fs::File STATE Progress` bind off thread::accept must build:\n{err}");
    }
}

#[test]
fn a_bind_that_only_aliases_a_live_resource_still_builds() {
    // The over-count pin, and the reason the fix carries the declarer's aliasing
    // gate. `RES g AS fs::File = f` closes nothing (bug-375, §15.6), so
    // `required_helpers` declares no `fs` helper for it. Counting one as *used*
    // would trip the opposite arm — "NIR runtime call requires undeclared
    // helper 'fs'" — turning bug-535 into its mirror image on a program that
    // built before the fix. This program builds on both sides of it.
    let source = "IMPORT io\n\
                  IMPORT fs\n\n\
                  FUNC take(RES f AS fs::File) AS Integer\n\
                 \x20 RES g AS fs::File = f\n\
                 \x20 RETURN 1\n\
                  END FUNC\n\n\
                  FUNC main AS Integer\n\
                 \x20 io::print(\"started\")\n\
                 \x20 RETURN 0\n\
                  END FUNC\n";
    if let Err(err) = build("b535_alias_only", source) {
        panic!("an alias-only resource bind must keep building:\n{err}");
    }

    // The second aliasing shape the declarer recognizes: reading a resource
    // element out of a collection yields a pointer to the one resource, never a
    // transfer (§15.6), so the collection's owning scope closes it.
    let from_get = "IMPORT io\n\
                    IMPORT fs\n\
                    IMPORT collections\n\n\
                    FUNC pick(files AS List OF RES fs::File) AS Integer\n\
                   \x20 RES f AS fs::File = collections::get(files, 0)\n\
                   \x20 RETURN 1\n\
                    END FUNC\n\n\
                    FUNC main AS Integer\n\
                   \x20 io::print(\"started\")\n\
                   \x20 RETURN 0\n\
                    END FUNC\n";
    if let Err(err) = build("b535_alias_get", from_get) {
        panic!("a collection-element resource bind must keep building:\n{err}");
    }
}

#[test]
fn the_repaired_program_runs() {
    // A build that stops erroring is not the fix on its own: the module the
    // validator now admits must be a working program. This is the bug's own
    // reproduction, executed.
    let exe = build(
        "b535_runs",
        &worker_program("tcp", "tcp::Socket", "tcp::Socket"),
    )
    .expect("the bug-535 reproduction must build");
    let output = Command::new(&exe).output().expect("run built program");
    assert!(
        output.status.success(),
        "the reproduction must run cleanly, got {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "started",
        "unexpected program output"
    );
}
