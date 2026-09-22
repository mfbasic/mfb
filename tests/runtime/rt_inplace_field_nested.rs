//! plan-145-F: a nested field path — `o = WITH o { inner := WITH o.inner { b :=
//! op(o.inner.b, …) } }`, and `h.state.inner = WITH h.state.inner { … }` — is a
//! field site of depth 2 that every field-capable arm serves.
//!
//! "In place" is measured as in `rt_inplace_self_update.rs`: the statement run `N`
//! and `2N` times, `arena.<k>.alloc_calls` summed from the `--debug` report; a
//! rebuild allocates the new records every run. A reallocating arm needs every
//! level last-inlined: an `inner` followed by another inlined field declines, and
//! a sibling of `inner` inside the outer record survives the outer grow. Every
//! case checks the values, and every program frees what it allocated.

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

/// `count(2N) - count(N)` for `program(n)`, the 2N run's output, and whether the
/// 2N run freed everything it allocated.
fn extra(name: &str, program: impl Fn(u64) -> String) -> (u64, Vec<String>, bool) {
    let (_, once, ..) = run(&format!("{name}_n"), &program(N));
    let (lines, twice, freed, live) = run(&format!("{name}_2n"), &program(2 * N));
    (
        twice.saturating_sub(once),
        lines,
        twice == freed && live == 0,
    )
}

const TYPES: &str = "\
IMPORT collections
IMPORT fs
IMPORT io

TYPE Rec
  a AS List OF Integer
  b AS List OF Integer
END TYPE

TYPE Out
  n AS Integer
  inner AS Rec
END TYPE

TYPE Side
  inner AS Rec
  tail AS List OF Integer
END TYPE

FUNC keep(v AS Integer) AS Boolean
  RETURN v MOD 3 <> 0
END FUNC

FUNC total(xs AS List OF Integer) AS Integer
  MUT t AS Integer = 0
  FOR EACH v IN xs
    t = t + v
  NEXT
  RETURN t
END FUNC
";

/// Depth-2 `append` (a grow of the OUTER block through the path) and `filter`
/// (the sub-block where it lies), in a record and in a `STATE` payload. `a`, the
/// sibling before `b`, and `n`, the outer scalar, survive every grow.
#[test]
fn a_depth_two_append_and_filter_run_in_place() {
    let (grow, lines, freed) = extra("nested_rec", |n| {
        format!(
            "{TYPES}\nFUNC main() AS Integer\n  MUT o AS Out = Out[n := 7, inner := Rec[a := [4, 5], b := [1]]]\n  \
             FOR i = 1 TO {n}\n    \
             o = WITH o {{ inner := WITH o.inner {{ b := collections::append(o.inner.b, i) }} }}\n    \
             o = WITH o {{ inner := WITH o.inner {{ b := collections::filter(o.inner.b, keep) }} }}\n  NEXT\n  \
             io::print(toString(o.n) & \" \" & toString(total(o.inner.a)) & \" \" & toString(len(o.inner.b)))\n  \
             RETURN 0\nEND FUNC\n"
        )
    });
    // `b` keeps every value not divisible by 3: 1, then 1..2n filtered.
    let kept = (1..=2 * N).filter(|v| v % 3 != 0).count() as u64 + 1;
    assert_eq!(lines, vec![format!("7 9 {kept}")]);
    assert!(freed, "the nested record leaked");
    assert!(
        grow < N / 8,
        "record: {N} more runs allocated {grow} more blocks — a rebuild"
    );

    let (grow, lines, freed) = extra("nested_state", |n| {
        format!(
            "{TYPES}\nFUNC main() AS Integer\n  RES h AS fs::File STATE Out = fs::openFile(\"/dev/null\")\n  \
             h.state = Out[n := 7, inner := Rec[a := [4, 5], b := [1]]]\n  FOR i = 1 TO {n}\n    \
             h.state.inner = WITH h.state.inner {{ b := collections::append(h.state.inner.b, i) }}\n    \
             h.state.inner = WITH h.state.inner {{ b := collections::filter(h.state.inner.b, keep) }}\n  NEXT\n  \
             io::print(toString(h.state.n) & \" \" & toString(total(h.state.inner.a)) & \" \" & toString(len(h.state.inner.b)))\n  \
             RETURN 0\nEND FUNC\n"
        )
    });
    assert_eq!(lines, vec![format!("7 9 {kept}")]);
    assert!(freed, "the nested STATE payload leaked");
    assert!(
        grow < N / 8,
        "STATE: {N} more runs allocated {grow} more blocks — a rebuild"
    );
}

/// `Side.inner` is followed by the inlined `tail`: growing `inner.b` would shift
/// `tail`, so the append declines to the rebuild (it allocates every run) and
/// `tail` reads back intact. The non-reallocating `filter` still runs in place
/// there.
#[test]
fn a_not_last_inner_level_declines_a_grow() {
    let (grow, lines, freed) = extra("nested_not_last", |n| {
        format!(
            "{TYPES}\nFUNC main() AS Integer\n  MUT s AS Side = Side[inner := Rec[a := [4], b := [1]], tail := [9, 9]]\n  \
             FOR i = 1 TO {n}\n    \
             s = WITH s {{ inner := WITH s.inner {{ b := collections::append(s.inner.b, i) }} }}\n  NEXT\n  \
             io::print(toString(len(s.inner.b)) & \" \" & toString(total(s.tail)))\n  RETURN 0\nEND FUNC\n"
        )
    });
    assert_eq!(lines, vec![format!("{} 18", 2 * N + 1)]);
    assert!(freed, "the declined nested update leaked");
    assert!(
        grow >= N,
        "a grow at a not-last inner level must rebuild; {N} more runs allocated only {grow}"
    );

    let (grow, lines, freed) = extra("nested_not_last_filter", |n| {
        format!(
            "{TYPES}\nFUNC main() AS Integer\n  MUT s AS Side = Side[inner := Rec[a := [4], b := [1, 2, 3, 4, 5, 6]], tail := [9, 9]]\n  \
             FOR i = 1 TO {n}\n    \
             s = WITH s {{ inner := WITH s.inner {{ b := collections::filter(s.inner.b, keep) }} }}\n  NEXT\n  \
             io::print(toString(len(s.inner.b)) & \" \" & toString(total(s.tail)))\n  RETURN 0\nEND FUNC\n"
        )
    });
    assert_eq!(lines, vec!["4 18".to_string()]);
    assert!(freed, "the nested filter leaked");
    assert!(
        grow < N / 8,
        "a non-reallocating arm at a not-last inner level runs in place; {N} more runs \
         allocated {grow} more blocks"
    );
}
