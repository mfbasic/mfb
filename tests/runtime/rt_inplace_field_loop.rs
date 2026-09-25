//! plan-145-H: a field self-update inside `FOR EACH` over that field (sites S7,
//! T7), and on a record captured by reference in a `collections::forEach` lambda
//! (S9).
//!
//! * The loop walks the field's ENTRY-TIME elements: it never sees its own writes
//!   (the loop walks a copy made once at entry, when its body writes the owner).
//! * No leak: before plan-145-H the rebuild kept every displaced record (or
//!   `STATE` payload) alive for the loop's sake and never freed it — one block per
//!   iteration. Every program here must end with `alloc_calls = free_calls` and
//!   `live_bytes 0`.
//! * S9: the lambda's updates reach the parent's record, grown or not.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

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
IMPORT fs
IMPORT io

TYPE R
  n AS Integer
  xs AS List OF Integer
END TYPE

FUNC odd(v AS Integer) AS Boolean
  RETURN v MOD 2 = 1
END FUNC

FUNC showL(xs AS List OF Integer) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC
";

/// Four ops inside a loop over the field they update, on a record and on a
/// `STATE` payload: each visits exactly the entry-time elements `3,1,2,`.
#[test]
fn a_loop_over_a_field_it_updates_visits_the_entry_elements() {
    let ops = [
        ("append", "collections::append(@.xs, v)", "3,1,2,3,1,2,"),
        ("removeAt", "collections::removeAt(@.xs, 0)", ""),
        ("sort", "collections::sort(@.xs)", "1,2,3,"),
        ("filter", "collections::filter(@.xs, odd)", "3,1,"),
    ];
    let mut body = String::new();
    let mut want = Vec::new();
    for (name, op, after) in ops {
        let rec = op.replace('@', "r");
        let st = op.replace('@', "h.state");
        body.push_str(&format!(
            "  r = R[n := 0, xs := [3, 1, 2]]\n  MUT seen_{name} AS String = \"\"\n  \
             FOR EACH v IN r.xs\n    seen_{name} = seen_{name} & toString(v) & \",\"\n    \
             r = WITH r {{ xs := {rec} }}\n  NEXT\n  \
             io::print(\"rec {name} \" & seen_{name} & \" -> \" & showL(r.xs))\n  \
             h.state = R[n := 0, xs := [3, 1, 2]]\n  MUT sseen_{name} AS String = \"\"\n  \
             FOR EACH v IN h.state.xs\n    sseen_{name} = sseen_{name} & toString(v) & \",\"\n    \
             h.state = WITH h.state {{ xs := {st} }}\n  NEXT\n  \
             io::print(\"state {name} \" & sseen_{name} & \" -> \" & showL(h.state.xs))\n"
        ));
        want.push(format!("rec {name} 3,1,2, -> {after}"));
        want.push(format!("state {name} 3,1,2, -> {after}"));
    }
    let source = format!(
        "{TYPES}\nFUNC main() AS Integer\n  MUT r AS R\n  \
         RES h AS fs::File STATE R = fs::openFile(\"/dev/null\")\n{body}  RETURN 0\nEND FUNC\n"
    );
    let (lines, alloc, free, live) = run("field_loop_ops", &source);
    assert_eq!(
        lines, want,
        "the loop must walk the field's entry-time elements"
    );
    assert_eq!((alloc, live), (free, 0), "the field loops leaked");
}

/// `EXIT FOR` and `RETURN` from inside the loop free its copy, and a 1,000
/// iteration loop over a field it grows ends with nothing live — before
/// plan-145-H it leaked one record (or payload) per iteration.
#[test]
fn a_loop_over_a_field_it_updates_frees_on_every_exit() {
    let source = format!(
        "{TYPES}
FUNC early(r0 AS R) AS Integer
  MUT r AS R = r0
  FOR EACH v IN r.xs
    r = WITH r {{ xs := collections::append(r.xs, v) }}
    IF v = 2 THEN
      RETURN len(r.xs)
    END IF
  NEXT
  RETURN 0
END FUNC

FUNC main() AS Integer
  MUT r AS R = R[n := 0, xs := [1, 2, 3]]
  FOR EACH v IN r.xs
    r = WITH r {{ xs := collections::append(r.xs, v * 10) }}
    IF v = 2 THEN
      EXIT FOR
    END IF
  NEXT
  io::print(\"exit \" & showL(r.xs))
  io::print(\"return \" & toString(early(R[n := 0, xs := [1, 2, 3]])))
  MUT big AS R = R[n := 0, xs := []]
  FOR i = 1 TO 1000
    big = WITH big {{ xs := collections::append(big.xs, i) }}
  NEXT
  FOR EACH v IN big.xs
    big = WITH big {{ n := big.n + 1, xs := collections::append(big.xs, v) }}
  NEXT
  io::print(\"big \" & toString(big.n) & \" \" & toString(len(big.xs)))
  RES h AS fs::File STATE R = fs::openFile(\"/dev/null\")
  h.state = R[n := 0, xs := [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]]
  FOR i = 1 TO 100
    FOR EACH v IN h.state.xs
      h.state = WITH h.state {{ xs := collections::append(h.state.xs, v) }}
      h.state = WITH h.state {{ xs := collections::removeAt(h.state.xs, 0) }}
    NEXT
  NEXT
  io::print(\"state \" & toString(len(h.state.xs)))
  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("field_loop_exits", &source);
    assert_eq!(
        lines,
        vec!["exit 1,2,3,10,20,", "return 5", "big 1000 2000", "state 10",]
    );
    assert_eq!(
        (alloc, live),
        (free, 0),
        "a field loop leaked: {alloc} allocated, {free} freed, {live} B live"
    );
}

/// S9: a record captured by reference in a `forEach` lambda, updated in it —
/// a scalar, an `append` (a grow of the parent's block) and a `filter` — prints
/// what the copying path would, and the parent sees every update.
#[test]
fn a_record_captured_by_reference_is_updated_in_place() {
    let source = format!(
        "{TYPES}
FUNC main() AS Integer
  MUT r AS R = R[n := 0, xs := [1]]
  LET items AS List OF Integer = [1, 2, 3, 4, 5, 6]
  FOR i = 1 TO 50
    collections::forEach(items, LAMBDA(v AS Integer) -> r = WITH r {{ n := r.n + v }})
    collections::forEach(items, LAMBDA(v AS Integer) -> r = WITH r {{ xs := collections::append(r.xs, v) }})
  NEXT
  collections::forEach([0], LAMBDA(v AS Integer) -> r = WITH r {{ xs := collections::filter(r.xs, odd) }})
  io::print(toString(r.n) & \" \" & toString(len(r.xs)))
  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("field_capture", &source);
    // n: 50 * (1+..+6); xs: [1] + 50 * [1..6], filtered to odds: 1 + 50 * 3.
    assert_eq!(lines, vec![format!("{} {}", 50 * 21, 1 + 50 * 3)]);
    assert_eq!((alloc, live), (free, 0), "the captured record leaked");
}
