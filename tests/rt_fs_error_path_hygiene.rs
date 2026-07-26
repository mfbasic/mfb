//! Regression tests for bug-63: three fs error-path resource-hygiene gaps in the
//! shared code lowerings, each an error branch that skipped a cleanup the success
//! path performs.
//!
//!   (1) fd leaked on record-alloc OOM. After `open`/`mkstemps` returns a valid fd,
//!       the subsequent `arena_alloc` of the File record can fail; the OOM branch
//!       previously jumped to the error tail without `close(fd)`, leaking the OS fd
//!       (`fs::openFile`, `fs::createTempFile`, `fs::readBytes`).
//!   (2) atomic-write temp file left on disk. `fs::writeTextAtomic` /
//!       `writeBytesAtomic` created a `<path>.mfb-XXXXXX.tmp` via `mkstemps`; every
//!       write/fsync/close/record-alloc/rename failure closed the fd but never
//!       `unlink`ed the temp, littering the target directory.
//!   (3) double-close after a failed `close`. `fs::close` set `FILE_OFFSET_CLOSED`
//!       only on the success branch; a failed `close` (EINTR/EIO) returned with
//!       CLOSED still 0, so a later `fs::close` — not seeing "already closed" —
//!       drained again and closed the same fd number (which may by then name an
//!       unrelated open file).
//!
//! Deterministic OOM and close-fault injection are impractical on the dev host, so
//! items (1) and (3) are locked as codegen-structure invariants over the emitted
//! `ncode` on all four backends (they fail the instant the cleanup is dropped from
//! any backend). Item (2) additionally has a real runtime proof: an atomic write
//! whose `rename` fails (target path is an existing directory) must leave no
//! `*.mfb-*.tmp` behind — this failed before the fix (the temp lingered) and passes
//! after it.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const TARGETS: &[&str] = &[
    "macos-aarch64",
    "linux-aarch64",
    "linux-x86_64",
    "linux-riscv64",
];

/// Exercises every fs helper touched by bug-63 so all their lowerings are emitted.
const SOURCE: &str = "IMPORT fs\nIMPORT strings\n\nFUNC main AS Integer\n  RES f AS File = fs::openFile(\"/tmp/x\", \"r\")\n  fs::close(f)\n  fs::createTempFile(\"/tmp\")\n  fs::readBytes(\"/tmp/x\")\n  fs::writeTextAtomic(\"/tmp/x\", \"y\")\n  fs::writeBytesAtomic(\"/tmp/x\", strings::toBytes(\"y\"))\n  RETURN 0\nEND FUNC\n";

fn unique_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("mfb_{name}_{nonce}"))
}

fn temp_project(name: &str, source: &str) -> PathBuf {
    let root = unique_root(name);
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

fn build_ncode(project: &Path, target: &str, name: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
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

fn helper<'a>(ncode: &'a Value, target: &str, symbol: &str) -> &'a Value {
    ncode["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("{target}: helper {symbol} not emitted"))
}

fn ops(func: &Value) -> &[Value] {
    func["instructions"].as_array().expect("instructions array")
}

fn op_name(op: &Value) -> &str {
    op.get("op").and_then(Value::as_str).unwrap_or("")
}

fn call_target(op: &Value) -> &str {
    op.get("target").and_then(Value::as_str).unwrap_or("")
}

/// A call (`bl`/`call`) to the libc `close` wrapper (`_close`/`close`), excluding
/// `closedir` and the `_mfb_rt_fs_file_drain` internal helper.
fn is_close_call(op: &Value) -> bool {
    let t = call_target(op);
    t.ends_with("close") && !t.ends_with("closedir")
}

fn is_unlink_call(op: &Value) -> bool {
    call_target(op).ends_with("unlink")
}

fn count(func: &Value, pred: impl Fn(&Value) -> bool) -> usize {
    ops(func).iter().filter(|op| pred(op)).count()
}

/// Item (1): the record-alloc OOM branch after a successful `open`/`mkstemps` must
/// `close(fd)`. `fs::openFile` and `fs::createTempFile` have no `close` at all in
/// their success paths, so the fix is exactly the first `close` call to appear;
/// `fs::readBytes` already closes once on success, so the OOM close makes it two.
fn assert_close_on_oom(ncode: &Value, target: &str) {
    let open_closes = count(
        helper(ncode, target, "_mfb_rt_fs_fs_openFile"),
        is_close_call,
    );
    assert!(
        open_closes >= 1,
        "{target}: fs::openFile OOM branch does not close the fd (bug-63 item 1): \
         expected >=1 close call, found {open_closes}",
    );
    let temp_closes = count(
        helper(ncode, target, "_mfb_rt_fs_fs_createTempFile"),
        is_close_call,
    );
    assert!(
        temp_closes >= 1,
        "{target}: fs::createTempFile OOM branch does not close the fd (bug-63 \
         item 1): expected >=1 close call, found {temp_closes}",
    );
    let read_closes = count(
        helper(ncode, target, "_mfb_rt_fs_fs_readBytes"),
        is_close_call,
    );
    assert!(
        read_closes >= 2,
        "{target}: fs::readBytes OOM branch does not close the fd (bug-63 item 1): \
         expected >=2 close calls (success + OOM), found {read_closes}",
    );
}

/// Item (2): every atomic-write failure after `mkstemps` unlinks the temp file. The
/// helper has three distinct post-`mkstemps` failure tails (ErrOutput close tail,
/// post-open alloc failure, rename failure), so at least three `unlink` calls must
/// be emitted; before the fix there were none.
fn assert_atomic_unlinks(ncode: &Value, target: &str) {
    for sym in [
        "_mfb_rt_fs_fs_writeTextAtomic",
        "_mfb_rt_fs_fs_writeBytesAtomic",
    ] {
        let unlinks = count(helper(ncode, target, sym), is_unlink_call);
        assert!(
            unlinks >= 3,
            "{target}: {sym} does not unlink the temp file on failure (bug-63 item \
             2): expected >=3 unlink calls, found {unlinks}",
        );
    }
}

/// Item (3): `fs::close` marks the File closed regardless of the `close` result.
/// The store of the CLOSED flag (`str_u64` at `FILE_OFFSET_CLOSED == 8`) must sit
/// between the `close` call and the branch to `..._close_error`; before the fix it
/// sat *after* that branch (only on the success fall-through).
fn assert_close_marked_before_branch(ncode: &Value, target: &str) {
    // `FILE_OFFSET_CLOSED == 8`; ncode stores the offset as a string field.
    const FILE_OFFSET_CLOSED: &str = "8";
    let func = helper(ncode, target, "_mfb_rt_fs_fs_close");
    let ins = ops(func);
    let close_idx = ins
        .iter()
        .position(is_close_call)
        .unwrap_or_else(|| panic!("{target}: fs::close emits no close call"));
    // The branch to `..._close_error` — a `b.lt` on aarch64/x86, a fused `rv.br` on
    // riscv64; identify it by its destination target across every backend.
    let branch_idx = ins[close_idx + 1..]
        .iter()
        .position(|op| call_target(op).ends_with("_close_error"))
        .map(|rel| close_idx + 1 + rel)
        .unwrap_or_else(|| panic!("{target}: fs::close has no branch to close_error"));
    // A record-field store (base is the File pointer, not the `sp` spill area) of
    // the CLOSED flag at offset 8, sitting before the close-result branch.
    let marked = ins[close_idx + 1..branch_idx].iter().any(|op| {
        op_name(op) == "str_u64"
            && op.get("offset").and_then(Value::as_str) == Some(FILE_OFFSET_CLOSED)
            && op.get("base").and_then(Value::as_str) != Some("sp")
    });
    assert!(
        marked,
        "{target}: fs::close does not store CLOSED before branching on the close \
         result (bug-63 item 3): a failed close would leave CLOSED=0 and permit a \
         double-close of a since-reused fd",
    );
}

fn assert_target(target: &str) {
    let name = format!("fs_err_hygiene_{}", target.replace('-', "_"));
    let project = temp_project(&name, SOURCE);
    let ncode = build_ncode(&project, target, &name);
    assert_eq!(
        ncode["target"].as_str(),
        Some(target),
        "ncode target mismatch"
    );
    assert_close_on_oom(&ncode, target);
    assert_atomic_unlinks(&ncode, target);
    assert_close_marked_before_branch(&ncode, target);
    let _ = fs::remove_dir_all(&project);
}

#[test]
fn error_path_hygiene_macos_aarch64() {
    assert_target("macos-aarch64");
}

#[test]
fn error_path_hygiene_linux_aarch64() {
    assert_target("linux-aarch64");
}

#[test]
fn error_path_hygiene_linux_x86_64() {
    assert_target("linux-x86_64");
}

#[test]
fn error_path_hygiene_linux_riscv64() {
    assert_target("linux-riscv64");
}

/// Item (2), runtime proof (host target only). An atomic write whose `rename`
/// fails — the target path is an existing directory, so `rename(temp, dir)` fails
/// with EISDIR/ENOTDIR — must not leave its `*.mfb-*.tmp` behind. Before the fix
/// the temp lingered in the parent directory; after it, the directory is clean.
#[test]
fn atomic_write_rename_failure_unlinks_temp() {
    let _ = TARGETS; // documented full backend matrix; runtime proof is host-only.
    let root = unique_root("fs_atomic_unlink_rt");
    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).expect("create project");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"fs_atomic_unlink_rt\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");

    // `collide` is a directory, so the atomic write's final rename onto it fails and
    // the freshly-created `collide.mfb-XXXXXX.tmp` must be unlinked on the way out.
    let work = root.join("work");
    fs::create_dir_all(&work).expect("create work dir");
    let collide = work.join("collide");
    let program = format!(
        "IMPORT fs\n\nFUNC main AS Integer\n  fs::createDirectory(\"{dir}\")\n  fs::writeTextAtomic(\"{dir}\", \"hello\")\n  RETURN 0\nEND FUNC\n",
        dir = collide.display(),
    );
    fs::write(src_dir.join("main.mfb"), program).expect("write source");

    let build = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg(&root)
        .output()
        .expect("run mfb build");
    assert!(
        build.status.success(),
        "mfb build failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );

    // Locate the built executable from the build log rather than assuming a fixed
    // name: macOS emits a single `<name>.out`, but the linux console build emits one
    // `<name>-glibc.out` + `<name>-musl.out` per libc world (no bare `<name>.out`).
    // Any flavor exercises the same lowering, so run the first one reported.
    let build_stdout = String::from_utf8_lossy(&build.stdout);
    let exe = build_stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .map(PathBuf::from)
        .expect("build reported no executable path");
    // The unhandled error Result traps the program (nonzero exit); that is fine —
    // the invariant under test is that no temp file is left behind.
    let _ = Command::new(&exe).output().expect("run generated program");

    let litter: Vec<_> = fs::read_dir(&work)
        .expect("read work dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".mfb-") && n.ends_with(".tmp"))
        .collect();
    assert!(
        litter.is_empty(),
        "atomic-write failure left temp litter (bug-63 item 2): {litter:?}",
    );

    let _ = fs::remove_dir_all(&root);
}
