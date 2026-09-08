//! Regression test for bug-557: an executable could not link against a package
//! whose source called `crypto::sign`, `crypto::verify` or `crypto::generate`
//! for one of the software curves.
//!
//! Those three members lower to native code that dispatches on the
//! `crypto::Certificate` at run time and, for the Edwards/Montgomery branches,
//! branches to an MFBASIC helper in the compiler's reserved namespace
//! (`#crypto_generateEd25519`, `#crypto_ed25519Sign`, ...). The lowering is
//! emitted once per unit as a standalone `abi_function` body with no calling
//! function in scope, so the symbol it branches to is the bare
//! `_mfb_ifn_crypto_…` one and cannot be anything else.
//!
//! `prefix_package_symbols` used to rewrite EVERY function a consumed `.mfp`
//! carried into `<identity>.<package>.<name>`, and a package that imports
//! `crypto` carries the registry-injected helpers as functions like any other.
//! The definition therefore moved while the native call site stayed put, and the
//! build died with:
//!
//! ```text
//! error: native code internal relocation target
//!        '_mfb_ifn_crypto_5FgenerateEd25519' is not defined
//! ```
//!
//! The package built fine on its own, and so did an executable that called
//! `crypto` directly — only the composition failed, which is why nothing in the
//! suite caught it. This test builds the composition and runs it.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("mfb_bug557_{}", common::unique_nonce()));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    // An empty per-run key store: a `file:` dependency is permitted unsigned, and
    // this keeps the result independent of whatever registry the developer
    // machine has authed against.
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug557_home"));
    command
}

fn write_project(root: &Path, name: &str, kind: &str, deps: &[&str], entry: bool, source: &str) {
    let dir = root.join(name);
    fs::create_dir_all(dir.join("src")).expect("src dir");
    fs::create_dir_all(dir.join("packages")).expect("packages dir");
    let role = if entry { "main" } else { "package" };
    let packages = deps
        .iter()
        .map(|dep| {
            format!(
                "{{\"name\":\"{dep}\",\"version\":\"=0.1.0\",\"source\":\"file:packages/{dep}.mfp\"}}"
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let entry_field = if entry {
        "\"entry\":\"main\",\"targets\":[\"native\"],"
    } else {
        ""
    };
    let manifest = format!(
        "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"{kind}\",\
         \"description\":\"bug-557 fixture\",\
         \"sources\":[{{\"root\":\"src\",\"role\":\"{role}\",\"include\":[\"**/*.mfb\"]}}],\
         {entry_field}\"packages\":[{packages}]}}\n"
    );
    fs::write(dir.join("project.json"), manifest).expect("write manifest");
    let src_name = if entry { "main.mfb" } else { "lib.mfb" };
    fs::write(dir.join("src").join(src_name), source).expect("write source");
}

fn build(dir: &Path) -> String {
    let output = mfb().arg("build").arg(dir).output().expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "build of {} failed:\n{combined}",
        dir.display()
    );
    combined
}

/// The one runnable artifact of a console build, from the build's own report.
///
/// The path is NOT `<project>/build/<name>.out` everywhere: only macOS emits a
/// single unflavored executable. A Linux console build emits one per libc world
/// — `app-glibc.out` AND `app-musl.out` (`src/os/linux/mod.rs`) — and neither is
/// spelled `app.out`, so hardcoding that name made this test unrunnable on all
/// three Linux rows (`Os { code: 2, kind: NotFound }` out of `Command::new`).
///
/// The rule is `common::build_project`'s — the FIRST reported path, which is the
/// glibc one — and it is deliberately not `cfg!(target_env)`. On the
/// `linux-x86_64-musl` row the test binary is musl but the RUNNER is glibc, and
/// mfb's musl output does not execute there (it dies in the dynamic loader at
/// 127; see the acceptance job's history in `.github/workflows`). Picking by the
/// test binary's own libc would therefore red that row — while the 87 RSS cases
/// in `rt_scope_drop_leaks`, which all go through `build_project`, pass on it
/// today by running the glibc artifact. One rule, one place to be wrong.
fn host_executable(build_output: &str) -> PathBuf {
    let written: Vec<&str> = build_output
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let first = written.first().unwrap_or_else(|| {
        panic!("the build reported no executable:\n{build_output}");
    });
    PathBuf::from(*first)
}

/// A package that reaches all three affected members on both Edwards curves, and
/// an executable that consumes it. Before the fix the executable's build failed
/// with an undefined internal relocation target; after it, the whole round trip
/// runs in-process and reports the signature sizes RFC 8032 fixes.
#[test]
fn an_executable_links_a_package_that_signs_on_a_software_curve() {
    let root = unique_root();

    write_project(
        &root,
        "signer",
        "package",
        &[],
        false,
        r#"IMPORT crypto

DOC
  PACKAGE
  DESC A package that signs and verifies on the software curves.
END DOC

DOC
  FUNC roundTrip
  DESC Generate a key on `curve`, sign a message, verify it, and report the
  DESC signature's length -- or 0 when the signature did not verify.
  ARG curve The certificate type to use.
  RET The signature length, or 0.
END DOC
EXPORT FUNC roundTrip(curve AS crypto::Certificate) AS Integer
  LET pair AS crypto::KeyPair = crypto::generate(curve)
  LET message AS List OF Byte = [1, 2, 3]
  LET signature AS List OF Byte = crypto::sign(curve, pair.privateKey, message)
  IF NOT crypto::verify(curve, pair.publicKey, message, signature) THEN
    RETURN 0
  END IF
  RETURN len(signature)
END FUNC
"#,
    );
    build(&root.join("signer"));

    write_project(
        &root,
        "app",
        "executable",
        &["signer"],
        true,
        r#"IMPORT signer
IMPORT crypto
IMPORT io

FUNC main() AS Integer
  io::print(toString(signer::roundTrip(crypto::Certificate.Ed25519)))
  io::print(toString(signer::roundTrip(crypto::Certificate.Ed448)))
  RETURN 0
END FUNC
"#,
    );
    fs::copy(
        root.join("signer").join("signer.mfp"),
        root.join("app").join("packages").join("signer.mfp"),
    )
    .expect("install signer.mfp");

    let built = build(&root.join("app"));

    let binary = host_executable(&built);
    let run = Command::new(&binary).output().expect("run the executable");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "the executable failed:\n{stdout}{}",
        String::from_utf8_lossy(&run.stderr)
    );
    // 64 and 114 are the fixed RFC 8032 signature sizes; a 0 on either line
    // would mean the call linked but verified nothing.
    assert_eq!(
        stdout.trim().lines().collect::<Vec<_>>(),
        vec!["64", "114"],
        "unexpected signature sizes:\n{stdout}"
    );

    fs::remove_dir_all(&root).ok();
}
