//! bug-842: a resource produced inline in a non-statement expression is closed at
//! the end of its statement.
//!
//! A producer call used as a call argument, as the target of a member read, or as
//! an element of a list or record literal that is itself passed on, used to yield a
//! record that no binding claimed and no cleanup was registered for: the handle was
//! never closed — not at the statement, not at scope exit, not at program end. With
//! `fs::File` that is a file-descriptor leak, so a loop of such calls dies with
//! `7-702-0001 Read operation failed` once the process runs out of descriptors,
//! while the bound-then-passed form runs clean.
//!
//! Each case loops far past a 64-descriptor limit; a leak of one descriptor per
//! iteration exhausts it within the first hundred.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_restemp_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

/// Scaffold an executable project for `source`, build it, and return the
/// executable; panics with the build output when it does not compile.
fn build(root: &Path, source: &str) -> PathBuf {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"restemp\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write manifest");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    let out = Command::new(common::mfb_exe())
        .arg("build")
        .arg(root)
        .output()
        .expect("run mfb build");
    assert!(
        out.status.success(),
        "build failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    root.join("build/restemp.out")
}

/// Run `exe` with at most 64 open descriptors.
fn run_limited(exe: &Path) -> (bool, String, String) {
    let out = Command::new("sh")
        .arg("-c")
        .arg("ulimit -n 64 && exec \"$0\"")
        .arg(exe)
        .output()
        .expect("run the program under sh");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn inline_resource_temporaries_do_not_leak_descriptors() {
    let root = unique_root("loop");
    let data = root.join("in.txt");
    fs::write(&data, "data").expect("write input");
    let source = format!(
        "IMPORT io\n\
         IMPORT fs\n\
         TYPE Cur\n\
         \x20 pos AS Integer\n\
         END TYPE\n\
         TYPE Holder\n\
         \x20 handle AS RES fs::File\n\
         END TYPE\n\
         FUNC useIt(f AS fs::File) AS Integer\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC useList(xs AS List OF RES fs::File) AS Integer\n\
         \x20 RETURN len(xs)\n\
         END FUNC\n\
         FUNC useH(h AS Holder) AS Integer\n\
         \x20 RETURN 1\n\
         END FUNC\n\
         FUNC openS(p AS String) AS RES fs::File STATE Cur\n\
         \x20 RES f AS fs::File STATE Cur = fs::openFile(p, \"r\")\n\
         \x20 f.state.pos = 1\n\
         \x20 RETURN f\n\
         END FUNC\n\
         FUNC main() AS Integer\n\
         \x20 LET p = \"{path}\"\n\
         \x20 MUT n AS Integer = 0\n\
         \x20 FOR i = 1 TO 500\n\
         \x20   n = n + useIt(fs::openFile(p, \"r\"))\n\
         \x20   n = n + useList([fs::openFile(p, \"r\")])\n\
         \x20   n = n + useH(Holder[fs::openFile(p, \"r\")])\n\
         \x20   n = n + openS(p).state.pos\n\
         \x20 NEXT\n\
         \x20 io::print(\"n=\" & toString(n))\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        path = data.display()
    );
    let exe = build(&root, &source);
    let (ok, stdout, stderr) = run_limited(&exe);
    assert!(
        ok && stdout == "n=2000\n" && stderr.is_empty(),
        "every inline temporary must close at its statement's end (bug-842); \
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
