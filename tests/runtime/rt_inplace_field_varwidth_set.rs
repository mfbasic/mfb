//! bug-759: `collections::set` on a variable-width list (`List OF String`) held in a
//! record field, a nested record field or a `STATE` field — the owner's last inlined
//! field — runs in the list's own sub-block. It used to decline and rebuild the
//! owner every statement (O(n) per `set`, 42–156 µs at n = 1000–4000).
//!
//! Each site rewrites every element with a longer literal, then with a longer one
//! still, which overflows the data region and forces the repack to grow the RECORD
//! (`InlineGrow`): the test reads the list and the owner's other fields afterwards.
//! A last case sets an element from another element of the same list — an operand
//! that could point into the owner, which the arm declines — and must stay exact.
//!
//! Cost: `--debug` allocations at N and 2N. A literal item allocates nothing, so in
//! place the difference is only the geometric repacks; the rebuilding path
//! allocates a whole record per `set` (at least 6N more for 3 sites x 2 passes).

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const SOURCE: &str = r#"IMPORT io
IMPORT os
IMPORT fs
IMPORT strings
IMPORT collections

TYPE Rec
  before AS Integer
  xs AS List OF String
END TYPE

TYPE Outer
  tag AS Integer
  inner AS Rec
END TYPE

FUNC strs(n AS Integer) AS List OF String
  MUT out AS List OF String = []
  FOR i = 0 TO n - 1
    out = collections::append(out, "s")
  NEXT
  RETURN out
END FUNC

FUNC summary(xs AS List OF String) AS String
  IF os::getEnv("CHECK") <> "1" THEN
    RETURN toString(len(xs))
  END IF
  MUT total AS Integer = 0
  FOR EACH x IN xs
    total = total + len(x)
  NEXT
  RETURN toString(len(xs)) & "/" & toString(total) & "/" & collections::get(xs, 0) & "/" & collections::get(xs, len(xs) - 1)
END FUNC

FUNC field(n AS Integer) AS String
  MUT rec AS Rec = Rec[7, strs(n)]
  FOR j = 0 TO n - 1
    rec = WITH rec { xs := collections::set(rec.xs, j, "medium-length") }
  NEXT
  FOR j = 0 TO n - 1
    rec = WITH rec { xs := collections::set(rec.xs, j, "a much longer replacement string") }
  NEXT
  RETURN toString(rec.before) & " " & summary(rec.xs)
END FUNC

FUNC nested(n AS Integer) AS String
  MUT o AS Outer = Outer[9, Rec[8, strs(n)]]
  FOR j = 0 TO n - 1
    o = WITH o { inner := WITH o.inner { xs := collections::set(o.inner.xs, j, "medium-length") } }
  NEXT
  FOR j = 0 TO n - 1
    o = WITH o { inner := WITH o.inner { xs := collections::set(o.inner.xs, j, "a much longer replacement string") } }
  NEXT
  RETURN toString(o.tag) & " " & toString(o.inner.before) & " " & summary(o.inner.xs)
END FUNC

FUNC state(RES f AS fs::File STATE Rec, n AS Integer) AS String
  f.state = Rec[6, strs(n)]
  FOR j = 0 TO n - 1
    f.state.xs = collections::set(f.state.xs, j, "medium-length")
  NEXT
  FOR j = 0 TO n - 1
    f.state.xs = collections::set(f.state.xs, j, "a much longer replacement string")
  NEXT
  RETURN toString(f.state.before) & " " & summary(f.state.xs)
END FUNC

FUNC aliasing(n AS Integer) AS String
  MUT rec AS Rec = Rec[5, strs(n)]
  rec = WITH rec { xs := collections::set(rec.xs, 0, "first") }
  FOR j = 1 TO n - 1
    rec = WITH rec { xs := collections::set(rec.xs, j, collections::get(rec.xs, j - 1)) }
  NEXT
  RETURN toString(rec.before) & " " & summary(rec.xs)
END FUNC

FUNC main() AS Nothing
  LET n AS Integer = toInt(os::getEnv("N"))
  io::print(field(n))
  io::print(nested(n))
  RES f AS fs::File STATE Rec = fs::open(os::getEnv("PROBE"), "read")
  io::print(state(f, n))
  io::print(aliasing(50))
END FUNC
"#;

fn counter(stats: &str, key: &str) -> u64 {
    stats
        .lines()
        .find_map(|line| line.strip_prefix(&format!("arena.0.{key} ")))
        .unwrap_or_else(|| panic!("no arena.0.{key} in:\n{stats}"))
        .trim()
        .parse()
        .expect("numeric counter")
}

/// The expected `summary` once every element holds the longest literal.
fn summary(n: usize) -> String {
    let long = "a much longer replacement string";
    format!("{n}/{}/{long}/{long}", n * long.len())
}

#[test]
fn a_field_list_set_runs_in_place_and_keeps_the_owner_exact() {
    let project = common::temp_project("bug759_field_varwidth_set", SOURCE);
    std::fs::write(project.join("probe.txt"), "x").expect("probe file");
    let built = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    assert!(
        built.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let executable = project.join("build").join(format!(
        "bug759_field_varwidth_set{}",
        if cfg!(windows) { ".exe" } else { ".out" }
    ));
    let run = |n: usize, check: bool| {
        let output = Command::new(&executable)
            .env("N", n.to_string())
            .env("CHECK", if check { "1" } else { "0" })
            .env("PROBE", project.join("probe.txt"))
            .output()
            .expect("run the probe");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stats = format!("{stdout}{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "probe failed:\n{stats}");
        (stdout, counter(&stats, "alloc_calls"))
    };
    // Contents and the owner's siblings, after the overflow repacks.
    let (stdout, _) = run(400, true);
    let alias = "first";
    assert_eq!(
        stdout.lines().take(4).collect::<Vec<_>>(),
        vec![
            format!("7 {}", summary(400)),
            format!("9 8 {}", summary(400)),
            format!("6 {}", summary(400)),
            format!("5 50/{}/{alias}/{alias}", 50 * alias.len()),
        ],
        "list contents and sibling fields after the overflow repacks"
    );
    // Cost. Without the content walk, literal items and literal list elements
    // allocate nothing per element and the aliasing case runs at a fixed size, so
    // what grows with N is the in-place sites' geometric repacks and list growth:
    // logarithmic. A rebuild of the three sites adds at least 6 x 400.
    let (_, small) = run(400, false);
    let (_, large) = run(800, false);
    let growth = large - small;
    assert!(
        growth < 400 / 8,
        "allocations grew by {growth} from N=400 to N=800"
    );
}
