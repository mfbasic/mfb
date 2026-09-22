//! plan-145-G: a module-level record's field self-update (`gR = WITH gR { … }`)
//! runs in place — letter C's stores, F's overwrite and the seam's arms, with the
//! block pointer loaded from the global's slot and stored back after a grow.
//!
//! A global's block is reachable from every function, so the value semantics are
//! the risk. plan-142-H's four cases, applied to a record:
//!
//! * `LET y = gR` taken before the update is unchanged after it;
//! * `f(gR)` where `f` updates `gR` (bug-665's shape) prints the entry value;
//! * `FOR EACH v IN gR.xs` whose body updates `gR.xs` (bug-666's shape) visits
//!   the entry elements;
//! * a failed update inside `TRAP` leaves `gR` unchanged;
//! * an update whose operand itself writes `gR` keeps `WITH`'s snapshot: the
//!   operand's write is discarded (`G-global-operand`).
//!
//! "In place" is measured as in `rt_inplace_self_update.rs`: `N` and `2N` runs,
//! `arena.<k>.alloc_calls` summed from the `--debug` report.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const N: u64 = 2000;

/// Build `source` with `--debug`, run it, and return `(stdout lines, alloc_calls,
/// free_calls, live_bytes)` summed over the arenas.
fn run(name: &str, source: &str) -> (Vec<String>, u64, u64, u64) {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "build failed:\n{stdout}\n{}\n--- source ---\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|p| p.ends_with("-glibc.out"))
        .or_else(|| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
        })
        .expect("an executable")
        .to_string();
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "program failed:\n{text}\n{report}\n--- source ---\n{source}"
    );
    let sum = |key: &str| -> u64 {
        report
            .lines()
            .filter_map(|line| line.strip_prefix("arena."))
            .filter_map(|rest| rest.split_once(key))
            .filter(|(arena, _)| !arena.contains('.'))
            .map(|(_, n)| n.trim().parse::<u64>().expect("a count"))
            .sum()
    };
    (
        text.lines().map(str::to_string).collect(),
        sum(".alloc_calls "),
        sum(".free_calls "),
        sum(".live_bytes "),
    )
}

const TYPES: &str = "\
IMPORT collections
IMPORT io

TYPE G
  n AS Integer
  xs AS List OF Integer
END TYPE

MUT gR AS G

FUNC showL(xs AS List OF Integer) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC
";

/// A scalar store and an `append` (a grow of the global's block) run in place:
/// `N` more runs allocate next to nothing more, and the values are right.
#[test]
fn a_global_record_field_update_runs_in_place() {
    let program = |n: u64| {
        format!(
            "{TYPES}\nFUNC main() AS Integer\n  gR = G[n := 0, xs := [1]]\n  FOR i = 1 TO {n}\n    \
             gR = WITH gR {{ n := gR.n + 1 }}\n    \
             gR = WITH gR {{ xs := collections::append(gR.xs, i) }}\n  NEXT\n  \
             io::print(toString(gR.n) & \" \" & toString(len(gR.xs)))\n  RETURN 0\nEND FUNC\n"
        )
    };
    let (_, once, ..) = run("global_rec_n", &program(N));
    let (lines, twice, freed, live) = run("global_rec_2n", &program(2 * N));
    assert_eq!(lines, vec![format!("{} {}", 2 * N, 2 * N + 1)]);
    // The global's own final block is live until exit (a module-level value has no
    // scope drop; plan-142-H's global tests measure growth for the same reason):
    // exactly one block, and nothing else.
    assert_eq!(twice, freed + 1, "the global record leaked ({live} B live)");
    let grow = twice.saturating_sub(once);
    assert!(
        grow < N / 8,
        "{N} more runs allocated {grow} more blocks — a rebuild"
    );
}

/// plan-142-H's value cases, each against the in-place update.
#[test]
fn a_global_record_keeps_value_semantics() {
    let source = format!(
        "{TYPES}
SUB bump()
  gR = WITH gR {{ xs := collections::append(gR.xs, 99), n := 7 }}
END SUB

FUNC seen(r AS G) AS String
  bump()
  RETURN showL(r.xs) & \" n=\" & toString(r.n)
END FUNC

FUNC touch() AS Integer
  gR = WITH gR {{ xs := collections::append(gR.xs, 5) }}
  RETURN 1
END FUNC

FUNC failing() AS Integer
  FAIL error(77050002, \"no\")
END FUNC

FUNC trapped() AS Integer
  gR = WITH gR {{ xs := collections::append(gR.xs, failing()) }}
  RETURN 0

  TRAP(e)
    io::print(\"trap \" & showL(gR.xs))
    RETURN 1
  END TRAP
END FUNC

FUNC main() AS Integer
  gR = G[n := 0, xs := [1, 2]]
  LET y AS G = gR
  gR = WITH gR {{ xs := collections::append(gR.xs, 3) }}
  gR = WITH gR {{ n := 5 }}
  io::print(\"snapshot \" & showL(y.xs) & \" n=\" & toString(y.n))
  io::print(\"param \" & seen(gR))
  io::print(\"after \" & showL(gR.xs) & \" n=\" & toString(gR.n))
  MUT visited AS String = \"\"
  FOR EACH v IN gR.xs
    visited = visited & toString(v) & \",\"
    gR = WITH gR {{ xs := collections::append(gR.xs, v * 10) }}
  NEXT
  io::print(\"loop \" & visited & \" now \" & toString(len(gR.xs)))
  io::print(toString(trapped()))
  io::print(\"end \" & toString(len(gR.xs)))
  ' `WITH` builds from gR's value before `touch()` ran, so touch's append is
  ' discarded (G-global-operand declines the in-place store that would keep it).
  gR = WITH gR {{ n := touch() }}
  io::print(\"operand \" & toString(len(gR.xs)) & \" n=\" & toString(gR.n))
  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("global_rec_values", &source);
    assert_eq!(
        lines,
        vec![
            "snapshot 1,2, n=0",
            "param 1,2,3, n=5",
            "after 1,2,3,99, n=7",
            "loop 1,2,3,99, now 8",
            "trap 1,2,3,99,10,20,30,990,",
            "1",
            "end 8",
            "operand 8 n=1",
        ],
        "a global record's in-place update must keep value semantics"
    );
    // As above: only the global's final block outlives `main`.
    assert_eq!(alloc, free + 1, "the value cases leaked ({live} B live)");
}
