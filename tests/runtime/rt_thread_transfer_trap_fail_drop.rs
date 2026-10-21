//! bug-899: a `thread::transfer` that fails under an inline `TRAP` leaves the
//! resource the sender's, so the sender's scope drop still closes it.
//!
//! The inline-`TRAP` lowering retired the sender's cleanup unconditionally, on the
//! failure path too, so every failed transfer leaked the file descriptor: the
//! report's loop died with `7-702-0001 Read operation failed` (EMFILE) at the
//! 253rd iteration under `ulimit -n 256`. Under a 64-descriptor limit 300 failed
//! transfers exhaust it several times over.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_trapxfer_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

/// Scaffold an executable project for `source`, build it, and return the
/// executable; panics with the build output when it does not compile.
fn build(root: &Path, source: &str) -> PathBuf {
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"trapxfer\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write manifest");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    common::build_project(root)
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
fn a_failed_inline_trapped_transfer_keeps_the_senders_close() {
    let root = unique_root("loop");
    let data = root.join("in.txt");
    fs::write(&data, "data").expect("write input");
    let source = format!(
        "IMPORT io\n\
         IMPORT fs\n\
         IMPORT thread\n\
         ISOLATED FUNC ignorer(w AS ThreadWorker OF Nothing RES fs::File TO Integer, n AS Integer) AS Integer\n\
         \x20 RETURN n\n\
         END FUNC\n\
         FUNC once(p AS String) AS Integer\n\
         \x20 RES f AS fs::File = fs::openFile(p, \"r\")\n\
         \x20 RES t AS Thread OF Nothing RES fs::File TO Integer = thread::start(ignorer, 1)\n\
         \x20 LET r AS Integer = thread::waitFor(t)\n\
         \x20 thread::transfer(t, f) TRAP(e)\n\
         \x20   RECOVER\n\
         \x20 END TRAP\n\
         \x20 RETURN 0\n\
         END FUNC\n\
         FUNC main() AS Integer\n\
         \x20 MUT i AS Integer = 0\n\
         \x20 WHILE i < 300\n\
         \x20   LET x AS Integer = once(\"{path}\")\n\
         \x20   i = i + 1\n\
         \x20 END WHILE\n\
         \x20 io::print(\"ok \" & toString(i))\n\
         \x20 RETURN 0\n\
         END FUNC\n",
        path = data.display()
    );
    let exe = build(&root, &source);
    let (ok, stdout, stderr) = run_limited(&exe);
    assert!(
        ok && stdout == "ok 300\n" && stderr.is_empty(),
        "a failed inline-TRAP transfer must leave the sender's close in place (bug-899); \
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
