//! bug-470: every script that takes the tree's gate lock must RELEASE it on a
//! normal, successful exit — not only when it is interrupted.
//!
//! Separate from `gate_mutual_exclusion.rs` on purpose. That file tests the lock
//! helper's decision logic by driving the helper directly, which is the right
//! level for "who refuses whom". It cannot see this defect at all, because the
//! defect is not in the helper: it is in how a CALLER installs the helper's trap.
//!
//! The defect, measured on the unlanded fix before this test existed:
//! `test-accept.sh` acquires the lock at the top (which installs
//! `trap gate_lock_release EXIT INT TERM`) and then, forty lines later, runs
//! `trap 'rm -rf "$MFB_HOME"' EXIT` for its own scratch `$MFB_HOME`. **`trap`
//! replaces; it does not chain.** So the EXIT handler was gone, and a successful
//! run exited 0, printed `acceptance tests passed`, and left `tests/.gate.lock`
//! behind with its own now-dead pid in `owner`.
//!
//! Why it deserves a test and not a comment. It very nearly hides: INT and TERM
//! were NOT clobbered, so an interrupted run released correctly and only the
//! SUCCESS path leaked — the opposite of where anyone looks. And it self-heals
//! *most* of the time, because the next acquire finds a lock whose holder pid is
//! gone and reclaims it. "Most" is the problem: pids get reused, so once an
//! unrelated live process inherits that number `kill -0` succeeds, the stale lock
//! reads as HELD, and the tree wedges behind a refusal with no live rival to
//! point at — the "one ^C wedges the tree" failure the reclaim path exists to
//! prevent, reached from the other side.
//!
//! Each test runs against its OWN throwaway tree, so it neither contends with a
//! real gate run nor with its sibling test, and can never wedge the real tree.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A private tree holding just the lock helper: enough for `gate_lock_acquire`,
/// and isolated from the real `tests/.gate.lock`.
fn private_tree(name: &str) -> PathBuf {
    let dst = std::env::temp_dir().join(format!("mfb-bug470-release-{name}"));
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(dst.join("scripts")).expect("mkdir scripts");
    std::fs::create_dir_all(dst.join("tests")).expect("mkdir tests");
    std::fs::copy(
        repo_root().join("scripts/gate-lock.sh"),
        dst.join("scripts/gate-lock.sh"),
    )
    .expect("copy helper");
    dst
}

/// Every `trap ... EXIT` line `script` installs, verbatim and in file order,
/// minus the acquire's own (which lives in `gate-lock.sh`).
///
/// Reading them out of the script is what keeps this honest: the test exercises
/// the real handler chain, so a future `trap ... EXIT` that forgets the release
/// fails here without anyone remembering to update the test. An empty result is
/// legitimate — `artifact-gate.sh` installs none of its own, which is exactly
/// why it never had the bug — and is still worth asserting, because adding one
/// later is the way it would acquire it.
fn exit_traps_of(script: &Path) -> String {
    let text = std::fs::read_to_string(script)
        .unwrap_or_else(|e| panic!("read {}: {e}", script.display()));
    text.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("trap ") && l.contains("EXIT"))
        .filter(|l| !l.contains("gate_lock_release EXIT"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Acquire the lock exactly as `script` does, install `script`'s own EXIT traps
/// on top, exit 0 — and report whether the lock outlived the shell.
fn lock_survives_a_normal_exit(script: &str) -> bool {
    let tree = private_tree(script);
    let traps = exit_traps_of(&repo_root().join("scripts").join(script));
    // `$MFB_HOME` is what test-accept.sh's trap removes. Bind it to a real temp
    // dir: under `set -u` an unset name aborts the handler before it reaches the
    // release, which would fail this test for a reason the product does not have.
    let body = format!(
        r#"
set -u
MFB_HOME=$(mktemp -d)
export MFB_HOME
GATE_LOCK_HOLDER="{script}"
GATE_LOCK_TREE="{tree}"
. "{tree}/scripts/gate-lock.sh"
gate_lock_acquire || exit $?
[ -d "{tree}/tests/.gate.lock" ] || {{ echo "acquire did not create the lock" >&2; exit 1; }}
{traps}
exit 0
"#,
        script = script,
        tree = tree.display(),
        traps = traps,
    );
    let out = Command::new("bash").arg("-c").arg(&body).output().expect("bash");
    assert!(
        out.status.success(),
        "harness snippet failed ({}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    tree.join("tests/.gate.lock").exists()
}

#[test]
fn test_accept_releases_the_lock_on_a_successful_exit() {
    assert!(
        !lock_survives_a_normal_exit("test-accept.sh"),
        "test-accept.sh left tests/.gate.lock behind after exiting 0 — its own \
         `trap ... EXIT` for $MFB_HOME replaced the lock's EXIT trap instead of \
         chaining to it, so only INT/TERM released and the SUCCESS path leaked"
    );
}

#[test]
fn artifact_gate_releases_the_lock_on_a_successful_exit() {
    assert!(
        !lock_survives_a_normal_exit("artifact-gate.sh"),
        "artifact-gate.sh left tests/.gate.lock behind after exiting 0"
    );
}

#[test]
fn sync_goldens_releases_the_lock_on_a_successful_exit() {
    assert!(
        !lock_survives_a_normal_exit("sync-goldens.sh"),
        "sync-goldens.sh left tests/.gate.lock behind after exiting 0"
    );
}
