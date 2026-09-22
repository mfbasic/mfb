//! plan-145-C: a mixed `WITH` — one collection field served by a self-update arm
//! beside scalar fields — updates the owner in place, and keeps `WITH`'s rules:
//!
//! * **atomicity** — every update reads the OLD record and a failure leaves the
//!   record unchanged, so a scalar that fails must fail before the arm mutates;
//! * **order of effects** — updates are evaluated in source order;
//! * a later update that reads the arm's field sees the OLD value.
//!
//! "In place" is measured as in `rt_inplace_self_update.rs`: the statement run `N`
//! and `2N` times, `arena.<k>.alloc_calls` summed from the `--debug` report; a
//! rebuild allocates the new record every run. A case that must DECLINE asserts
//! the opposite, and every case checks the printed values against what `WITH`
//! means.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const N: u64 = 2000;

/// Build `source` with `--debug`, run it, and return `(stdout lines, alloc_calls,
/// free_calls, live_bytes)` summed over the arenas. A failing build or run panics
/// with the source.
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

/// `count(2N) - count(N)` for `program(n)`, and the 2N run's output.
fn extra(name: &str, program: impl Fn(u64) -> String) -> (u64, Vec<String>) {
    let (_, once, ..) = run(&format!("{name}_n"), &program(N));
    let (lines, twice, ..) = run(&format!("{name}_2n"), &program(2 * N));
    (twice.saturating_sub(once), lines)
}

const TYPES: &str = "\
IMPORT collections
IMPORT fs
IMPORT io
IMPORT json

TYPE Rec
  hp AS Integer
  n AS Integer
  xs AS List OF Integer
END TYPE

TYPE P
  hp AS Integer
  n AS Integer
  xs AS List OF Integer
END TYPE

FUNC say(v AS Integer) AS Integer
  ' Prints a literal, so the call allocates nothing on its own.
  IF v = 1 THEN
    io::print(\"say 1\")
  ELSE
    io::print(\"say 2\")
  END IF
  RETURN v
END FUNC

FUNC count(xs AS List OF Integer) AS Integer
  RETURN len(xs)
END FUNC
";

/// A scalar before the arm and a local-read scalar after it: both fields updated
/// in place, the record and the payload alike.
#[test]
fn a_mixed_with_updates_the_collection_and_the_scalars_in_place() {
    let (grow, lines) = extra("mixed_rec", |n| {
        format!(
            "{TYPES}\nFUNC main() AS Integer\n  MUT r AS Rec = Rec[hp := 1, n := 0, xs := [1, 2, 3]]\n  \
             LET k AS Integer = 5\n  FOR i = 1 TO {n}\n    \
             r = WITH r {{ hp := r.hp + 1, xs := collections::append(r.xs, k), n := k }}\n  NEXT\n  \
             io::print(toString(r.hp) & \" \" & toString(r.n) & \" \" & toString(len(r.xs)))\n  \
             RETURN 0\nEND FUNC\n"
        )
    });
    assert_eq!(lines, vec![format!("{} 5 {}", 2 * N + 1, 2 * N + 3)]);
    assert!(
        grow < N / 8,
        "record: {N} more runs allocated {grow} more blocks — a rebuild"
    );

    let (grow, lines) = extra("mixed_state", |n| {
        format!(
            "{TYPES}\nFUNC main() AS Integer\n  RES h AS fs::File STATE P = fs::openFile(\"/dev/null\")\n  \
             h.state = P[hp := 1, n := 0, xs := [1, 2, 3]]\n  LET k AS Integer = 5\n  \
             FOR i = 1 TO {n}\n    \
             h.state = WITH h.state {{ hp := h.state.hp + 1, xs := collections::append(h.state.xs, k), n := k }}\n  \
             NEXT\n  \
             io::print(toString(h.state.hp) & \" \" & toString(h.state.n) & \" \" & toString(len(h.state.xs)))\n  \
             RETURN 0\nEND FUNC\n"
        )
    });
    assert_eq!(lines, vec![format!("{} 5 {}", 2 * N + 1, 2 * N + 3)]);
    assert!(
        grow < N / 8,
        "STATE: {N} more runs allocated {grow} more blocks — a rebuild"
    );
}

/// `hp := r.hp + big` overflows. It is written before the arm, so it is evaluated
/// before the arm mutates `xs`: the handler sees the record exactly as it was. The
/// same statement without the overflow is in place (the first test's shape), so
/// this exercises the in-place path, not the rebuild. The `STATE` handle is a `RES`
/// parameter, so the handler may read it (a function's OWN resource is closed
/// before its handler runs, bug-676), and the caller reads it again after.
#[test]
fn a_failing_scalar_leaves_the_whole_owner_unchanged() {
    let source = format!(
        "{TYPES}\nFUNC viaRecord(big AS Integer) AS Integer\n  \
         MUT r AS Rec = Rec[hp := 1, n := 0, xs := [1, 2, 3]]\n  \
         r = WITH r {{ hp := r.hp + big, xs := collections::append(r.xs, 7), n := 9 }}\n  \
         RETURN 0\n\n  TRAP(e)\n    \
         io::print(\"record hp=\" & toString(r.hp) & \" n=\" & toString(r.n) & \" len=\" & toString(len(r.xs)))\n    \
         RETURN 1\n  END TRAP\nEND FUNC\n\n\
         FUNC viaState(RES h AS fs::File STATE P, big AS Integer) AS Integer\n  \
         h.state = WITH h.state {{ hp := h.state.hp + big, xs := collections::append(h.state.xs, 7), n := 9 }}\n  \
         RETURN 0\n\n  TRAP(e)\n    \
         io::print(\"state hp=\" & toString(h.state.hp) & \" n=\" & toString(h.state.n) & \" len=\" & toString(len(h.state.xs)))\n    \
         RETURN 1\n  END TRAP\nEND FUNC\n\n\
         FUNC main() AS Integer\n  LET big AS Integer = 9223372036854775807\n  \
         io::print(toString(viaRecord(big)))\n  \
         RES h AS fs::File STATE P = fs::openFile(\"/dev/null\")\n  \
         h.state = P[hp := 1, n := 0, xs := [1, 2, 3]]\n  \
         io::print(toString(viaState(h, big)))\n  \
         io::print(\"after hp=\" & toString(h.state.hp) & \" n=\" & toString(h.state.n) & \" len=\" & toString(len(h.state.xs)))\n  \
         RETURN 0\nEND FUNC\n"
    );
    let (lines, alloc, free, live) = run("mixed_atomic", &source);
    assert_eq!(
        lines,
        vec![
            "record hp=1 n=0 len=3",
            "1",
            "state hp=1 n=0 len=3",
            "1",
            "after hp=1 n=0 len=3"
        ],
        "a failed update must leave every field as it was"
    );
    assert_eq!((alloc, live), (free, 0), "the failed update leaked");
}

/// Updates are evaluated in source order. A scalar with an effect BEFORE the arm
/// runs first, and the statement stays in place; one AFTER the arm cannot run
/// early without reordering the effects, so the statement declines to the rebuild
/// — which prints in source order too.
#[test]
fn effects_run_in_source_order() {
    let body = |stmt: &str, n: u64| {
        format!(
            "{TYPES}\nFUNC main() AS Integer\n  MUT r AS Rec = Rec[hp := 1, n := 0, xs := [1]]\n  \
             FOR i = 1 TO {n}\n    {stmt}\n  NEXT\n  \
             io::print(toString(r.n) & \" \" & toString(len(r.xs)))\n  RETURN 0\nEND FUNC\n"
        )
    };
    let before = "r = WITH r { n := say(1), xs := collections::append(r.xs, say(2)) }";
    let (lines, ..) = run("mixed_order_before", &body(before, 1));
    assert_eq!(lines, vec!["say 1", "say 2", "1 2"]);
    let (grow, _) = extra("mixed_order_before_grow", |n| body(before, n));
    assert!(
        grow < N / 8,
        "an effect before the arm: {grow} more blocks — a rebuild"
    );

    let after = "r = WITH r { xs := collections::append(r.xs, say(1)), n := say(2) }";
    let (lines, ..) = run("mixed_order_after", &body(after, 1));
    assert_eq!(lines, vec!["say 1", "say 2", "2 2"]);
    let (grow, _) = extra("mixed_order_after_grow", |n| body(after, n));
    assert!(
        grow >= N,
        "an effect after the arm must decline (it would run early): only {grow} more blocks"
    );
}

/// A later update that reads the arm's field reads the OLD value, as `WITH` does.
/// An effect-free read is evaluated before the arm, so it is admitted and still
/// sees the old value; a read through a user function (which could have an
/// effect) declines — and the rebuild sees the old value as well.
#[test]
fn a_later_update_reading_the_arms_field_sees_the_old_value() {
    let body = |stmt: &str, n: u64| {
        format!(
            "{TYPES}\nFUNC main() AS Integer\n  MUT r AS Rec = Rec[hp := 1, n := 0, xs := [1, 2, 3]]\n  \
             FOR i = 1 TO {n}\n    {stmt}\n  NEXT\n  \
             io::print(toString(r.n) & \" \" & toString(len(r.xs)))\n  RETURN 0\nEND FUNC\n"
        )
    };
    let pure = "r = WITH r { xs := collections::append(r.xs, 4), n := len(r.xs) }";
    let (lines, ..) = run("mixed_read_pure", &body(pure, 1));
    assert_eq!(
        lines,
        vec!["3 4"],
        "`n` must read the list before the append"
    );
    let (grow, _) = extra("mixed_read_pure_grow", |n| body(pure, n));
    assert!(
        grow < N / 8,
        "an effect-free read: {grow} more blocks — a rebuild"
    );

    let called = "r = WITH r { xs := collections::append(r.xs, 4), n := count(r.xs) }";
    let (lines, ..) = run("mixed_read_called", &body(called, 1));
    assert_eq!(
        lines,
        vec!["3 4"],
        "`n` must read the list before the append"
    );
    let (grow, _) = extra("mixed_read_called_grow", |n| body(called, n));
    assert!(
        grow >= N,
        "a read through a call must decline: only {grow} more blocks"
    );
}

/// A pointer field and a scalar in one `WITH`: both stored in their slots, the old
/// `json::Json` dropped, nothing leaked.
#[test]
fn a_pointer_and_a_scalar_update_in_place_without_a_leak() {
    let program = |stmt: &str, n: u64| {
        format!(
            "IMPORT io\nIMPORT json\n\nTYPE Doc\n  j AS json::Json\n  n AS Integer\nEND TYPE\n\n\
             FUNC fresh(v AS Integer) AS json::Json\n  RETURN json::JsonNum[toFloat(v)]\nEND FUNC\n\n\
             SUB sink(v AS json::Json)\nEND SUB\n\n\
             FUNC main() AS Integer\n  MUT d AS Doc = Doc[j := json::parse(\"[1,2]\"), n := 0]\n  \
             FOR i = 1 TO {n}\n    {stmt}\n  NEXT\n  \
             io::print(json::stringify(d.j) & \" \" & toString(d.n))\n  RETURN 0\nEND FUNC\n"
        )
    };
    let update = "d = WITH d { j := fresh(i), n := d.n + 1 }";
    let (lines, alloc, free, live) = run("mixed_pointer", &program(update, N));
    assert_eq!(lines, vec![format!("{N} {N}")]);
    assert_eq!((alloc, live), (free, 0), "the replaced json::Json leaked");
    // The value allocates on its own; the control builds the same value and hands
    // it to a no-op SUB. A rebuild adds the record (and the kept fields) on top.
    let (grow, _) = extra("mixed_pointer_grow", |n| program(update, n));
    let (control, _) = extra("mixed_pointer_control", |n| program("sink(fresh(i))", n));
    assert!(
        grow as f64 <= 1.125 * control as f64,
        "{N} more runs allocated {grow} more blocks, the value alone {control} — a rebuild"
    );
}
