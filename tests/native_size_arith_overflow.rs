//! Regression tests for bug-60: unchecked allocation-size arithmetic before
//! `_mfb_arena_alloc` in `strings::replace` / `strings::join` / list-`replace`
//! and in the thread-queue allocator.
//!
//! The string cases are latent — a 64-bit wrap needs an impossibly large input
//! (a ~2^63-byte string), so they are proven at the emitted-instruction level:
//! the `-ncode` dump must show every size term routed through the checked
//! helpers (`umulh`/`cmp` overflow guards branching to an overflow label) exactly
//! as the audited siblings (`strings::repeat`/`pad`) do. Their success path is
//! also built and run to prove realistic inputs are unchanged.
//!
//! The thread-queue case IS reachable with a realistic (user-supplied) limit:
//! `thread::start` accepts an `inLimit`/`outLimit` and previously only checked
//! `>= 1`. A limit past `u64::MAX / 8` would wrap the `capacity * 8` value-array
//! size to a tiny block while the huge capacity is stored, so a later enqueue
//! would index out of the allocation. It is now rejected as an invalid argument,
//! which the runtime tests below execute and observe.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

fn temp_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project src");
    root
}

fn write_project(root: &Path, name: &str, packages_json: &str, source: &str) {
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],{packages_json}\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
}

fn build_ncode(project: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg("-ncode")
        .arg(project)
        .output()
        .expect("run mfb build -ncode");
    assert!(
        output.status.success(),
        "build -ncode failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let path = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote native code plan to "))
        .expect("ncode output path");
    fs::read_to_string(path).expect("read ncode plan")
}

fn build_executable(project: &Path) -> Option<PathBuf> {
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg(project)
        .output()
        .expect("run mfb build");
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .map(PathBuf::from)
}

struct RunResult {
    stdout: String,
    /// stdout + stderr combined (uncaught runtime errors print to stderr).
    output: String,
    success: bool,
}

fn run_allow_failure(executable: &Path) -> RunResult {
    let output = Command::new(executable).output().expect("run executable");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    RunResult {
        output: format!("{stdout}{stderr}"),
        stdout,
        success: output.status.success(),
    }
}

/// The three string builders route every arena-size term through the checked
/// helpers, so the dump must contain a dedicated overflow label per builder plus
/// the wrap guards that branch to it. This is the only proof available for the
/// string cases: the wrap is not constructible with a real input.
#[test]
fn string_size_arith_has_overflow_guards() {
    let root = temp_root("bug60_string_ncode");
    write_project(
        &root,
        "bug60_string_ncode",
        "",
        "IMPORT io\n\
         IMPORT strings\n\
         IMPORT collections\n\
         \n\
         FUNC main AS Integer\n\
         \x20 LET a AS String = strings::replace(\"hello world hello\", \"hello\", \"hi\")\n\
         \x20 io::print(a)\n\
         \x20 LET parts AS List OF String = [\"a\", \"bb\", \"ccc\"]\n\
         \x20 io::print(strings::join(parts, \"-\"))\n\
         \x20 LET lst AS List OF String = [\"x\", \"y\", \"x\", \"z\"]\n\
         \x20 LET r AS List OF String = collections::replace(lst, \"x\", \"QQ\")\n\
         \x20 io::print(strings::join(r, \",\"))\n\
         \x20 RETURN 0\n\
         END FUNC\n",
    );
    let ncode = build_ncode(&root);

    // A dedicated overflow label exists for each builder, and each is the target
    // of at least one wrap-guard branch (`b.lo` for adds, `b.ne` after `umulh`
    // for the list-replace count*ENTRY_SIZE multiply).
    for label in [
        "replace_overflow",
        "strings_join_overflow",
        "replace_list_overflow",
    ] {
        assert!(
            ncode.contains(&format!("\"name\": \"{label}")),
            "missing overflow label `{label}` in ncode:\n{ncode}"
        );
        assert!(
            ncode.contains(&format!("\"target\": \"{label}")),
            "no wrap guard branches to `{label}` in ncode"
        );
    }

    // The list-replace `count * ENTRY_SIZE` multiply is guarded by a high-half
    // check (`umulh` then a branch to the overflow label).
    let has_umulh_guard = ncode.lines().collect::<Vec<_>>().windows(3).any(|w| {
        w[0].contains("\"op\": \"umulh\"")
            && w.iter().any(|l| l.contains("replace_list_overflow"))
    });
    assert!(
        has_umulh_guard,
        "list-replace multiply is not guarded by a umulh high-half check"
    );

    fs::remove_dir_all(&root).ok();
}

/// The checked helpers must not perturb the success path: realistic inputs
/// produce byte-for-byte the same output as before the guards were added.
#[test]
fn string_size_arith_success_path_unchanged() {
    let root = temp_root("bug60_string_run");
    write_project(
        &root,
        "bug60_string_run",
        "",
        "IMPORT io\n\
         IMPORT strings\n\
         IMPORT collections\n\
         \n\
         FUNC main AS Integer\n\
         \x20 io::print(strings::replace(\"hello world hello\", \"hello\", \"hi\"))\n\
         \x20 LET parts AS List OF String = [\"a\", \"bb\", \"ccc\"]\n\
         \x20 io::print(strings::join(parts, \"-\"))\n\
         \x20 LET lst AS List OF String = [\"x\", \"y\", \"x\", \"z\"]\n\
         \x20 io::print(strings::join(collections::replace(lst, \"x\", \"QQ\"), \",\"))\n\
         \x20 RETURN 0\n\
         END FUNC\n",
    );
    let exe = build_executable(&root).expect("build executable");
    let result = run_allow_failure(&exe);
    assert!(result.success, "program failed:\n{}", result.stdout);
    assert_eq!(result.stdout, "hi world hi\na-bb-ccc\nQQ,y,QQ,z\n");
    fs::remove_dir_all(&root).ok();
}

const THREAD_WORKERS_MFP: &str =
    "tests/rt-behavior/threads/thread-bounded-queues/packages/thread_runtime_workers.mfp";

fn thread_project(name: &str, in_limit: &str, out_limit: &str) -> PathBuf {
    let root = temp_root(name);
    fs::create_dir_all(root.join("packages")).expect("create packages dir");
    let src_mfp = Path::new(env!("CARGO_MANIFEST_DIR")).join(THREAD_WORKERS_MFP);
    fs::copy(&src_mfp, root.join("packages/thread_runtime_workers.mfp"))
        .expect("copy worker package");
    write_project(
        &root,
        name,
        "\"packages\":[{\"name\":\"thread_runtime_workers\",\"version\":\"=0.1.0\",\"source\":\"file:packages/thread_runtime_workers.mfp\"}],",
        &format!(
            "IMPORT io\n\
             IMPORT thread\n\
             IMPORT thread_runtime_workers\n\
             \n\
             FUNC main AS Integer\n\
             \x20 LET outbound AS Thread OF String TO Integer = thread::start(thread_runtime_workers::emitThreeBuffered, \"seed\", {in_limit}, {out_limit})\n\
             \x20 LET first AS String = thread::receive(outbound, 1000)\n\
             \x20 io::print(first)\n\
             \x20 RETURN 0\n\
             END FUNC\n"
        ),
    );
    root
}

/// A queue limit past `u64::MAX / 8` (the largest whose `* 8` byte size still
/// fits in 64 bits) is rejected as an invalid argument instead of wrapping the
/// value-array size and under-allocating.
#[test]
fn thread_queue_limit_out_of_range_rejected() {
    // u64::MAX / 8 + 1.
    let root = thread_project("bug60_thread_reject", "2305843009213693952", "3");
    let exe = build_executable(&root).expect("build executable");
    let result = run_allow_failure(&exe);
    assert!(
        !result.success,
        "out-of-range limit was not rejected:\n{}",
        result.output
    );
    assert!(
        result
            .output
            .contains("Argument value is not valid for the requested operation."),
        "expected an invalid-argument error, got:\n{}",
        result.output
    );
    fs::remove_dir_all(&root).ok();
}

/// A realistic in-range limit is accepted and the worker runs normally — the new
/// upper bound does not reject valid queue sizes.
#[test]
fn thread_queue_limit_in_range_accepted() {
    let root = thread_project("bug60_thread_accept", "1", "3");
    let exe = build_executable(&root).expect("build executable");
    let result = run_allow_failure(&exe);
    assert!(result.success, "in-range limit was rejected:\n{}", result.stdout);
    assert_eq!(result.stdout, "one\n");
    fs::remove_dir_all(&root).ok();
}
