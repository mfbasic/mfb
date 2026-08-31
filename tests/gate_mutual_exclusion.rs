//! bug-470: `artifact-gate.sh` and `test-accept.sh` must exclude each other
//! WITHIN one tree, and must NOT exclude a run in a DIFFERENT tree.
//!
//! Both properties are load-bearing and they pull in opposite directions, which
//! is why the original guard got both wrong at once:
//!
//! * **Same tree, either script** — the two write and delete the same fixture
//!   dump files, so a concurrent pair corrupts each other's artifacts. The
//!   original guards each matched only their OWN script name, so an
//!   `artifact-gate` and a `test-accept` in one tree proceeded without either
//!   noticing. That is the filed bug.
//!
//! * **Different trees** — each worktree owns its own `tests/`, so two runs in
//!   two trees cannot corrupt each other. The original guard matched on the
//!   script's *name* (`*/artifact-gate.sh`) with no notion of which tree it
//!   belonged to, so a run in `.claude/worktrees/467` refused a run in
//!   `.claude/worktrees/474` — pure lost throughput, and the reason a
//!   machine-wide `flock` would have been the wrong fix.
//!
//! The refusal exit code stays **98**, never 1: `tests/golden.rs` branches on it
//! to say "nothing was checked" rather than reporting a golden regression.
//!
//! These tests drive the scripts' lock helper directly rather than running a
//! real gate — a real one costs minutes and needs a built `mfb`. What is under
//! test is the mutual-exclusion decision, which is entirely in the helper.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Repo root, from this test binary's own location, never `$PWD` — the whole
/// point of the fix is that the tree is identified by where the script lives.
fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // CARGO_MANIFEST_DIR is the crate root, which is the repo root here.
    if !p.join("scripts/gate-lock.sh").exists() {
        p = p.join("..");
    }
    p
}

/// Run a shell snippet with the lock helper sourced, in `tree`, reporting the
/// exit status. `holder` is what the caller claims to be (`artifact-gate.sh` or
/// `test-accept.sh`), mirroring how each script identifies itself.
fn run_with_lock(tree: &Path, holder: &str, body: &str) -> Output {
    let helper = tree.join("scripts/gate-lock.sh");
    let script = format!(
        r#"set -u
GATE_LOCK_HOLDER={holder}
GATE_LOCK_TREE="{tree}"
. "{helper}"
{body}
"#,
        holder = holder,
        tree = tree.display(),
        helper = helper.display(),
        body = body,
    );
    Command::new("bash")
        .arg("-c")
        .arg(script)
        .output()
        .expect("run bash")
}

/// A second tree, so the cross-tree case is a real directory pair rather than a
/// simulated one. Only `scripts/` and `tests/` need to exist.
fn make_sibling_tree(name: &str) -> PathBuf {
    let root = repo_root();
    let dst = std::env::temp_dir().join(format!("mfb-bug470-{name}"));
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(dst.join("scripts")).expect("mkdir scripts");
    std::fs::create_dir_all(dst.join("tests")).expect("mkdir tests");
    std::fs::copy(
        root.join("scripts/gate-lock.sh"),
        dst.join("scripts/gate-lock.sh"),
    )
    .expect("copy helper");
    dst
}

#[test]
fn a_second_run_in_the_same_tree_is_refused_with_98() {
    let tree = make_sibling_tree("same-a");
    // Acquire, then attempt a nested acquire as the OTHER script. The nested
    // attempt is the one under test.
    let out = run_with_lock(
        &tree,
        "artifact-gate.sh",
        r#"gate_lock_acquire
GATE_LOCK_HOLDER=test-accept.sh gate_lock_acquire && echo "ACQUIRED-TWICE"
echo "nested-exit=$?"
"#,
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stdout.contains("ACQUIRED-TWICE"),
        "test-accept acquired a lock artifact-gate already holds in the SAME tree\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("nested-exit=98"),
        "a same-tree refusal must exit 98 (not 1 — golden.rs branches on it)\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("artifact-gate.sh"),
        "the refusal must NAME the competing script so the reader is not guessing\nstderr:\n{stderr}"
    );
}

#[test]
fn the_same_pairing_is_refused_in_the_other_direction_too() {
    // The original guards were symmetric in their blindness; assert both
    // orders so a fix that only teaches ONE script about the other regresses.
    let tree = make_sibling_tree("same-b");
    let out = run_with_lock(
        &tree,
        "test-accept.sh",
        r#"gate_lock_acquire
GATE_LOCK_HOLDER=artifact-gate.sh gate_lock_acquire && echo "ACQUIRED-TWICE"
echo "nested-exit=$?"
"#,
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("ACQUIRED-TWICE") && stdout.contains("nested-exit=98"),
        "artifact-gate must be refused while test-accept holds the same tree\nstdout:\n{stdout}"
    );
}

#[test]
fn a_run_in_a_different_tree_is_not_refused() {
    // The half this bug's document did not ask for, and the half a machine-wide
    // lock would break: separate trees own separate `tests/`, so they must run
    // concurrently.
    let tree_a = make_sibling_tree("cross-a");
    let tree_b = make_sibling_tree("cross-b");

    // The holder must stay ALIVE while tree B contends. An earlier version of
    // this test backgrounded a `sleep` and let the acquiring shell exit — which
    // fired the EXIT trap and RELEASED the lock, so tree B never contended at
    // all and the test passed even against a machine-wide lock. It was
    // incapable of failing. The holder is now the long-lived process itself.
    let helper_a = tree_a.join("scripts/gate-lock.sh");
    let mut holder = Command::new("bash")
        .arg("-c")
        .arg(format!(
            r#"set -u
GATE_LOCK_HOLDER=artifact-gate.sh
GATE_LOCK_TREE="{tree}"
. "{helper}"
gate_lock_acquire || exit 9
# Signal readiness HERE, not by probing for the lock file: the readiness probe
# must not assume where the lock lives, or it silently reports "never acquired"
# for any implementation that puts the lock somewhere else — masking the very
# difference this test exists to detect.
touch "$GATE_LOCK_TREE/ready"
sleep 30
"#,
            tree = tree_a.display(),
            helper = helper_a.display(),
        ))
        .spawn()
        .expect("spawn holder");

    // Wait for the holder to say it holds the lock, so the test is not racing
    // the holder's own acquire.
    let ready = tree_a.join("ready");
    let mut waited = 0;
    while !ready.exists() && waited < 5000 {
        std::thread::sleep(std::time::Duration::from_millis(25));
        waited += 25;
    }
    assert!(
        ready.exists(),
        "tree A never acquired its lock; nothing was being contended"
    );

    let out = run_with_lock(
        &tree_b,
        "artifact-gate.sh",
        r#"gate_lock_acquire && echo "ACQUIRED-B"
echo "exit=$?"
"#,
    );
    let stdout = String::from_utf8_lossy(&out.stdout);

    let _ = holder.kill();
    let _ = holder.wait();

    assert!(
        stdout.contains("ACQUIRED-B"),
        "a run in a DIFFERENT tree was refused — the guard is serializing \
         worktrees that cannot corrupt each other (bug-470 second defect)\nstdout:\n{stdout}"
    );
}

#[test]
fn a_lock_whose_holder_is_gone_is_reclaimed() {
    // A killed run must not wedge the tree forever. The lock records its
    // holder's pid; a lock whose pid no longer exists is stale and reclaimable.
    let tree = make_sibling_tree("stale");
    let out = run_with_lock(
        &tree,
        "artifact-gate.sh",
        r#"gate_lock_acquire
# Forge a holder pid that cannot be running: rewrite the record, then release
# our EXIT trap's claim on it by clearing the variable it checks.
echo "artifact-gate.sh 999999 0" > "$GATE_LOCK_DIR/owner"
GATE_LOCK_HELD=
GATE_LOCK_HOLDER=test-accept.sh gate_lock_acquire && echo "RECLAIMED"
echo "exit=$?"
"#,
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("RECLAIMED"),
        "a lock whose holder pid is gone must be reclaimable, or a killed run \
         wedges the tree permanently\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
