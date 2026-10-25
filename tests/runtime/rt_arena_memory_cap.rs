//! plan-179-C: the arena memory cap is enforced on every arena block a process maps.
//!
//! One process-wide counter charges each block before it is mapped and gives the
//! bytes back when a mapping fails or a worker's arena is unmapped. A charge that
//! would pass the cap is `ErrOutOfMemory`, which the program can trap.

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;

/// A temp project whose `project.json` carries `options` (a JSON object body).
fn project(name: &str, source: &str, options: &str) -> PathBuf {
    let root = common::temp_project(name, source);
    let manifest = root.join("project.json");
    let text = std::fs::read_to_string(&manifest).expect("read project.json");
    let text = text.replacen('{', &format!("{{\"options\":{{{options}}},"), 1);
    std::fs::write(&manifest, text).expect("write project.json");
    root
}

fn run(exe: &PathBuf) -> (bool, String, String) {
    let output = Command::new(exe).output().expect("run the program");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Each recursion level holds one live 1 MiB string, up to 64 levels, and the dive
/// is trapped as a whole, so an `ErrOutOfMemory` at any level ends it cleanly.
const DIVE: &str = r#"IMPORT errorCode
IMPORT io
IMPORT strings

FUNC dive(depth AS Integer) AS Integer
  LET s AS String = strings::repeat("x", 1048576)
  io::print("held " & toString(depth))
  IF depth >= 64 THEN
    RETURN depth
  END IF
  RETURN dive(depth + 1) + len(s) - 1048576
END FUNC

FUNC main AS Integer
  LET reached AS Integer = dive(1) TRAP(e)
    IF e.code = errorCode::ErrOutOfMemory THEN RECOVER -1
    FAIL e
  END TRAP
  IF reached < 0 THEN
    io::print("out of memory")
  ELSE
    io::print("done " & toString(reached))
  END IF
  RETURN 0
END FUNC
"#;

/// The deepest `held N` line the dive printed.
fn deepest(stdout: &str) -> u64 {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("held "))
        .map(|n| n.parse::<u64>().expect("a depth"))
        .max()
        .unwrap_or(0)
}

/// A 16 MiB cap stops the dive with a trappable `ErrOutOfMemory` once about 16 MiB
/// is live — not at the first block (a counter that refused every map would pass a
/// test that only looked for the error) and not past the cap.
#[test]
fn the_cap_is_enforced_and_trappable() {
    let root = project("arena_cap_enforced", DIVE, "\"maxArenaMemoryMiB\": 16");
    let exe = common::build_project(&root);
    let (ok, stdout, stderr) = run(&exe);
    let _ = std::fs::remove_dir_all(&root);
    assert!(
        ok,
        "the program must exit 0 after trapping\n{stdout}\n{stderr}"
    );
    assert!(stdout.ends_with("out of memory\n"), "{stdout}");
    let held = deepest(&stdout);
    assert!(
        (8..16).contains(&held),
        "held {held} MiB under a 16 MiB cap\n{stdout}"
    );
}

/// The same program with the cap disabled runs all 64 levels: a disabled build
/// emits no check.
#[test]
fn a_disabled_cap_is_not_enforced() {
    let root = project(
        "arena_cap_disabled",
        DIVE,
        "\"maxArenaMemoryDisable\": true, \"maxArenaMemoryMiB\": 16",
    );
    let exe = common::build_project(&root);
    let (ok, stdout, stderr) = run(&exe);
    let _ = std::fs::remove_dir_all(&root);
    assert!(ok, "{stdout}\n{stderr}");
    assert!(stdout.ends_with("done 64\n"), "{stdout}");
}

/// 50 workers one after another, each mapping an 8 MiB string in its own arena:
/// 400 MiB mapped in total, never more than one worker's live at once. Under a
/// 64 MiB cap this completes only if each finished worker's unmapped blocks are
/// given back to the counter.
#[test]
fn finished_workers_give_their_bytes_back() {
    const CHURN: &str = r#"IMPORT io
IMPORT strings
IMPORT thread

ISOLATED FUNC churn(w AS ThreadWorker OF String TO Integer, seed AS Integer) AS Integer
  LET big AS String = strings::repeat("x", 8388608)
  RETURN len(big) MOD 1000 + seed
END FUNC

FUNC main AS Integer
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 50
    RES t AS Thread OF String TO Integer = thread::start(churn, 1)
    total = total + thread::waitFor(t)
    i = i + 1
  END WHILE
  io::print("workers " & toString(i) & " total " & toString(total))
  RETURN 0
END FUNC
"#;
    let root = project("arena_cap_churn", CHURN, "\"maxArenaMemoryMiB\": 64");
    let exe = common::build_project(&root);
    let (ok, stdout, stderr) = run(&exe);
    let _ = std::fs::remove_dir_all(&root);
    assert!(ok, "{stdout}\n{stderr}");
    assert_eq!(stdout, "workers 50 total 30450\n");
}

/// A block whose map fails gives its charge back. The cap is at most the machine's
/// RAM, so an oversized request is refused by the cap before the map is tried; a
/// real map failure needs an address-space limit, which only Linux enforces
/// (`RLIMIT_AS`). Under `ulimit -v 512 MiB` and a 700 MiB cap, two 600 MiB requests
/// pass the cap and fail to map; the 256 MiB request after them fits the cap only
/// if both charges were returned.
#[cfg(target_os = "linux")]
#[test]
fn a_failed_map_gives_its_charge_back() {
    const REFUND: &str = r#"IMPORT errorCode
IMPORT io
IMPORT strings

FUNC take(n AS Integer) AS Integer
  LET s AS String = strings::repeat("x", n)
  RETURN len(s)
END FUNC

FUNC try(n AS Integer) AS Integer
  LET got AS Integer = take(n) TRAP(e)
    IF e.code = errorCode::ErrOutOfMemory THEN RECOVER -1
    FAIL e
  END TRAP
  RETURN got
END FUNC

FUNC main AS Integer
  io::print(toString(try(629145600)))
  io::print(toString(try(629145600)))
  io::print(toString(try(268435456)))
  RETURN 0
END FUNC
"#;
    let root = project(
        "arena_cap_refund",
        REFUND,
        "\"maxArenaMemoryMiB\": 700, \"maxArenaMemoryPercent\": 100",
    );
    let exe = common::build_project(&root);
    let output = Command::new("sh")
        .arg("-c")
        .arg("ulimit -v 524288 && exec \"$0\"")
        .arg(&exe)
        .output()
        .expect("run the program under ulimit -v");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&root);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert_eq!(stdout, "-1\n-1\n268435456\n", "{stderr}");
}
