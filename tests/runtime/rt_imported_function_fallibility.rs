//! plan-165-B: a function imported from a package is never proven unable to fail.
//!
//! A package's `.mfp` carries no fallibility metadata, so the importer has no
//! proof that even a trivially total exported function cannot fail. Missing
//! information counts as "can fail" (owner decision 2026-10-04): an inline
//! `TRAP` on the call keeps a live handler (no `TYPE_INLINE_TRAP_DEAD_HANDLER`),
//! and the call is a failure edge into a function-level handler, so the handler
//! may not read a local declared after it (bug-797, `TYPE_TRAP_LOCAL_UNINITIALIZED`).
//! The same function declared in the importer itself is proven total, which is
//! the control.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const LIB: &str = "EXPORT FUNC five() AS Integer\n  RETURN 5\nEND FUNC\n";

/// Write the `totallib` package and an executable importing it with `main` as
/// its source, then build the executable.
fn build_importer(test: &str, main: &str) -> Output {
    let root = std::env::temp_dir().join(format!("mfb_p165b_{test}_{}", common::unique_nonce()));
    let lib = root.join("totallib");
    fs::create_dir_all(lib.join("src")).expect("create package dir");
    fs::write(
        lib.join("project.json"),
        "{\"name\":\"totallib\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"package\",\
         \"description\":\"a package\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"package\",\"include\":[\"**/*.mfb\"]}],\
         \"packages\":[]}\n",
    )
    .expect("write package manifest");
    fs::write(lib.join("src/lib.mfb"), LIB).expect("write package source");
    let app = root.join("app");
    fs::create_dir_all(app.join("src")).expect("create app dir");
    fs::write(
        app.join("project.json"),
        "{\"name\":\"importer\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"description\":\"an importer\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"packages\":[{\"name\":\"totallib\",\"version\":\"=0.1.0\",\"source\":\"file:../totallib\",\
         \"direct\":true,\"requiredBy\":[]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write app manifest");
    fs::write(app.join("src/main.mfb"), main).expect("write app source");
    build(&app)
}

fn build(project: &Path) -> Output {
    Command::new(common::mfb_exe())
        .arg("build")
        .arg(project)
        .output()
        .expect("run mfb build")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// An inline `TRAP` on an imported call keeps its handler: no dead-handler
/// warning, and the program builds and runs with the call's value.
#[test]
fn an_inline_trap_on_an_imported_function_keeps_a_live_handler() {
    let output = build_importer(
        "inline",
        "IMPORT totallib\nIMPORT io\n\nFUNC main() AS Integer\n  \
         LET v = totallib::five() TRAP(e)\n    RECOVER -1\n  END TRAP\n  \
         io::print(toString(v))\n  RETURN 0\nEND FUNC\n",
    );
    let text = combined(&output);
    assert!(output.status.success(), "the importer must build:\n{text}");
    assert!(
        !text.contains("TYPE_INLINE_TRAP_DEAD_HANDLER"),
        "an imported function is unproved, so its handler is live:\n{text}"
    );
    let exe = text
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names an executable");
    let run = Command::new(exe.trim()).output().expect("run the importer");
    assert!(run.status.success(), "{}", combined(&run));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "5\n");
}

/// The imported call is a failure edge: a function handler may not read a local
/// declared after it. The same shape over a function declared in the importer
/// (proven total by the fixpoint) is accepted.
#[test]
fn an_imported_call_is_a_failure_edge_for_the_function_handler() {
    let shape = |call: &str| {
        format!(
            "IMPORT totallib\nIMPORT io\n\nFUNC localFive() AS Integer\n  RETURN 5\nEND FUNC\n\n\
             FUNC f() AS Integer\n  LET n = {call}()\n  LET marker = 7\n  \
             RETURN n + marker\n  TRAP(e)\n    RETURN marker\n  END TRAP\nEND FUNC\n\n\
             FUNC main() AS Integer\n  io::print(toString(f()))\n  RETURN 0\nEND FUNC\n"
        )
    };
    let imported = build_importer("edge_imported", &shape("totallib::five"));
    let text = combined(&imported);
    assert!(
        !imported.status.success() && text.contains("TYPE_TRAP_LOCAL_UNINITIALIZED"),
        "the imported call must be a failure edge:\n{text}"
    );
    let local = build_importer("edge_local", &shape("localFive"));
    let text = combined(&local);
    assert!(
        local.status.success() && !text.contains("TYPE_TRAP_LOCAL_UNINITIALIZED"),
        "a proven-total project function is not a failure edge:\n{text}"
    );
}
