//! bug-689: reading a record element out of a `List`/`Map` must not copy it when
//! the read is only a field read, and `get` → `WITH` → `set` on one element must
//! update it inside the container's own block.
//!
//! * **Bounded** (`the_*_is_bounded`): every program runs its loop `REPS` times and
//!   then `2 * REPS` times; the allocation count must not change. Before the fix
//!   each `get` of a record element allocated a copy of the whole element, so the
//!   count grew with the iteration count.
//! * **Value semantics** (`a_*`): a read that escapes, or outlives a write to its
//!   container, is still an independent value. Each of these prints what the
//!   copying lowering printed.
//! * Every program ends with `alloc_calls = free_calls` and `live_bytes 0` —
//!   except the one block a program that declares the module-level `gps` still
//!   holds in it at exit (a global lives until the process ends, `mfb spec memory
//!   program-startup`); those programs end with `gps = []`, one 48-byte block.

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
IMPORT io

TYPE Dot
  lon AS Float
  age AS Integer
END TYPE

TYPE P
  lon AS Float
  age AS Integer
  trail AS List OF Float
END TYPE

FUNC n AS Integer
  RETURN 64
END FUNC

FUNC mk AS List OF P
  MUT ps AS List OF P = []
  FOR i = 1 TO n()
    ps = collections::append(ps, P[lon := 0.5, age := i, trail := [0.0, 0.0, 0.0, 0.0]])
  NEXT
  RETURN ps
END FUNC

FUNC twice(v AS Integer) AS Integer
  RETURN v * 2
END FUNC

FUNC trailSum(ps AS List OF P) AS Float
  MUT s AS Float = 0.0
  FOR EACH p IN ps
    FOR EACH t IN p.trail
      s = s + t
    NEXT
  NEXT
  RETURN s
END FUNC

FUNC ageSum(ps AS List OF P) AS Integer
  MUT s AS Integer = 0
  FOR i = 0 TO len(ps) - 1
    LET p AS P = collections::get(ps, i)
    s = s + p.age
  NEXT
  RETURN s
END FUNC
";

/// The module-level list the global-container programs use, and the statement
/// that leaves it holding its one empty block at exit.
const GLOBAL: &str = "MUT gps AS List OF P = []\n";
const GLOBAL_EXIT: &str = "  gps = []\n";

/// `n()` is 64: the ages are `1..=64`.
const AGES: i64 = 64 * 65 / 2;

/// Nothing leaked: every block freed, or — for a program with the global — every
/// block but the global's own empty list.
fn assert_no_leak(name: &str, (alloc, free, live): (u64, u64, u64), global: bool) {
    let (blocks, bytes) = if global { (1, 48) } else { (0, 0) };
    assert_eq!(
        (alloc - free, live),
        (blocks, bytes),
        "{name}: leaked — {alloc} allocated, {free} freed, {live} B live"
    );
}

/// Run `body` (the statements of `main`, with `REPS` standing for the repeat
/// count) at `reps` and at `2 * reps`; return the first run's output after
/// asserting the allocation count did not grow, and neither run leaked.
fn bounded(name: &str, body: &str, reps: i64, global: bool) -> Vec<String> {
    let program = |r: i64| {
        format!(
            "{TYPES}{}\nFUNC main() AS Integer\n{}\n{}  RETURN 0\nEND FUNC\n",
            if global { GLOBAL } else { "" },
            body.replace("REPS", &r.to_string()),
            if global { GLOBAL_EXIT } else { "" },
        )
    };
    let (lines, alloc, free, live) = run(&format!("{name}_1"), &program(reps));
    let (_, alloc2, free2, live2) = run(&format!("{name}_2"), &program(2 * reps));
    assert_no_leak(
        &format!("{name} at {reps} reps"),
        (alloc, free, live),
        global,
    );
    assert_no_leak(
        &format!("{name} at {} reps", 2 * reps),
        (alloc2, free2, live2),
        global,
    );
    assert_eq!(
        alloc,
        alloc2,
        "{name}: allocations grew with the iteration count ({alloc} at {reps} reps, \
         {alloc2} at {} reps) — a record element is copied per read",
        2 * reps
    );
    lines
}

/// `sum + get(ps, i).age` on a `MUT` local list: the field read needs no copy.
#[test]
fn a_direct_field_read_is_bounded() {
    let lines = bounded(
        "direct_field",
        "  MUT ps AS List OF P = mk()\n  MUT sum AS Integer = 0\n  FOR r = 1 TO REPS\n    \
         FOR i = 0 TO n() - 1\n      sum = sum + collections::get(ps, i).age\n    NEXT\n  NEXT\n  \
         io::print(toString(sum))",
        40,
        false,
    );
    assert_eq!(lines, vec![(40 * AGES).to_string()]);
}

/// The same read on a scalar-only record (`Dot`).
#[test]
fn a_direct_field_read_of_a_scalar_record_is_bounded() {
    let lines = bounded(
        "direct_scalar",
        "  MUT ds AS List OF Dot = []\n  FOR i = 1 TO n()\n    \
         ds = collections::append(ds, Dot[lon := 1.0, age := i])\n  NEXT\n  \
         MUT sum AS Integer = 0\n  FOR r = 1 TO REPS\n    FOR i = 0 TO n() - 1\n      \
         sum = sum + collections::get(ds, i).age\n    NEXT\n  NEXT\n  io::print(toString(sum))",
        40,
        false,
    );
    assert_eq!(lines, vec![(40 * AGES).to_string()]);
}

/// A nested read `get(ps, i).trail` handed to `len` — the element is still only
/// read.
#[test]
fn a_binding_read_only_through_fields_of_an_immutable_list_is_bounded() {
    let lines = bounded(
        "binding_let",
        "  LET qs AS List OF P = mk()\n  MUT sum AS Integer = 0\n  FOR r = 1 TO REPS\n    \
         FOR i = 0 TO n() - 1\n      LET p AS P = collections::get(qs, i)\n      \
         sum = sum + p.age + len(p.trail)\n    NEXT\n  NEXT\n  io::print(toString(sum))",
        40,
        false,
    );
    assert_eq!(lines, vec![(40 * (AGES + 4 * 64)).to_string()]);
}

/// The container is a `MUT` that is reassigned elsewhere in the function, but
/// not between the `get` and the binding's last read.
#[test]
fn a_binding_read_only_through_fields_of_a_mut_list_is_bounded() {
    let lines = bounded(
        "binding_mut",
        "  MUT ps AS List OF P = []\n  ps = mk()\n  MUT sum AS Integer = 0\n  \
         FOR r = 1 TO REPS\n    FOR i = 0 TO n() - 1\n      \
         LET p AS P = collections::get(ps, i)\n      sum = sum + twice(p.age)\n    NEXT\n  NEXT\n  \
         io::print(toString(sum))",
        40,
        false,
    );
    assert_eq!(lines, vec![(80 * AGES).to_string()]);
}

/// A module-level container, read through a binding and directly; the user call
/// in between (`twice`) cannot store to it.
#[test]
fn a_read_of_a_module_level_list_is_bounded() {
    let lines = bounded(
        "global_read",
        "  gps = mk()\n  MUT sum AS Integer = 0\n  FOR r = 1 TO REPS\n    \
         FOR i = 0 TO n() - 1\n      LET p AS P = collections::get(gps, i)\n      \
         sum = sum + twice(p.age) + collections::get(gps, i).age\n    NEXT\n  NEXT\n  \
         io::print(toString(sum))",
        40,
        true,
    );
    assert_eq!(lines, vec![(120 * AGES).to_string()]);
}

/// A `Map` value read the same two ways.
#[test]
fn a_read_of_a_map_value_is_bounded() {
    let lines = bounded(
        "map_read",
        "  LET ps AS List OF P = mk()\n  MUT m AS Map OF Integer TO P = Map OF Integer TO P {}\n  \
         FOR i = 0 TO n() - 1\n    m = collections::set(m, i, collections::get(ps, i))\n  NEXT\n  \
         MUT sum AS Integer = 0\n  FOR r = 1 TO REPS\n    FOR k = 0 TO n() - 1\n      \
         LET p AS P = collections::get(m, k)\n      \
         sum = sum + p.age + collections::get(m, k).age\n    NEXT\n  NEXT\n  \
         io::print(toString(sum))",
        40,
        false,
    );
    assert_eq!(lines, vec![(80 * AGES).to_string()]);
}

/// Probe C: `LET p = get(ps, i)` then `ps = set(ps, i, p)` — the element is put
/// back unchanged.
#[test]
fn a_get_then_set_back_is_bounded() {
    let lines = bounded(
        "get_set_back",
        "  MUT ps AS List OF P = mk()\n  FOR r = 1 TO REPS\n    FOR i = 0 TO n() - 1\n      \
         LET p AS P = collections::get(ps, i)\n      ps = collections::set(ps, i, p)\n    NEXT\n  \
         NEXT\n  io::print(toString(ageSum(ps)))",
        40,
        false,
    );
    assert_eq!(lines, vec![AGES.to_string()]);
}

/// Probe B: `MUT p = get(ps, i)`, `p = WITH p { … }`, `ps = set(ps, i, p)` —
/// a scalar, a computed scalar and a fixed-width `set` on the element's trail.
#[test]
fn a_get_with_set_element_update_is_bounded() {
    let lines = bounded(
        "get_with_set",
        "  MUT ps AS List OF P = mk()\n  MUT head AS Integer = 0\n  FOR r = 1 TO REPS\n    \
         head = (head + 1) MOD 4\n    FOR i = 0 TO n() - 1\n      \
         MUT p AS P = collections::get(ps, i)\n      LET x AS Float = p.lon + 0.5\n      \
         p = WITH p { lon := x, age := p.age + 1, trail := collections::set(p.trail, head, x) }\n      \
         ps = collections::set(ps, i, p)\n    NEXT\n  NEXT\n  \
         io::print(toString(ageSum(ps)))\n  io::print(toString(trailSum(ps)))",
        40,
        false,
    );
    // Each element's lon climbs 0.5 per rep from 0.5; the last four reps wrote
    // the trail slots with lon values 20.5 - 1.5 .. 20.5 (reps 37..40).
    assert_eq!(
        lines,
        vec![
            (AGES + 40 * 64).to_string(),
            format!("{:.2}", 64.0 * (19.0 + 19.5 + 20.0 + 20.5)),
        ]
    );
}

/// The same update over a module-level list.
#[test]
fn a_get_with_set_element_update_of_a_module_level_list_is_bounded() {
    let lines = bounded(
        "global_get_with_set",
        "  gps = mk()\n  FOR r = 1 TO REPS\n    FOR i = 0 TO n() - 1\n      \
         MUT p AS P = collections::get(gps, i)\n      \
         p = WITH p { age := p.age + twice(1), trail := collections::set(p.trail, 1, p.lon) }\n      \
         gps = collections::set(gps, i, p)\n    NEXT\n  NEXT\n  \
         io::print(toString(ageSum(gps)))\n  io::print(toString(trailSum(gps)))",
        40,
        true,
    );
    assert_eq!(
        lines,
        vec![(AGES + 80 * 64).to_string(), format!("{:.2}", 64.0 * 0.5)]
    );
}

/// The single-expression form `ps = set(ps, i, WITH get(ps, i) { … })`.
#[test]
fn a_single_expression_element_update_is_bounded() {
    let lines = bounded(
        "single_expression",
        "  MUT ps AS List OF P = mk()\n  FOR r = 1 TO REPS\n    FOR i = 0 TO n() - 1\n      \
         ps = collections::set(ps, i, WITH collections::get(ps, i) { age := collections::get(ps, i).age + 1 })\n    \
         NEXT\n  NEXT\n  io::print(toString(ageSum(ps)))",
        40,
        false,
    );
    assert_eq!(lines, vec![(AGES + 40 * 64).to_string()]);
}

/// A returned element is an independent value: the container's later update does
/// not reach it.
#[test]
fn a_returned_element_is_a_copy() {
    let source = format!(
        "{TYPES}
FUNC pick(ps AS List OF P, i AS Integer) AS P
  LET p AS P = collections::get(ps, i)
  RETURN p
END FUNC

FUNC main() AS Integer
  MUT ps AS List OF P = mk()
  LET kept AS P = pick(ps, 0)
  ps = collections::set(ps, 0, P[lon := 9.0, age := 99, trail := []])
  io::print(toString(kept.age) & \" \" & toString(len(kept.trail)) & \" \" & toString(collections::get(ps, 0).age))
  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("returned", &source);
    assert_eq!(lines, vec!["1 4 99"]);
    assert_eq!((alloc, live), (free, 0));
}

/// An element stored into another list, then the source list updated in place.
#[test]
fn an_element_stored_into_another_list_is_a_copy() {
    let source = format!(
        "{TYPES}
FUNC main() AS Integer
  MUT ps AS List OF P = mk()
  MUT qs AS List OF P = []
  FOR i = 0 TO 3
    LET p AS P = collections::get(ps, i)
    qs = collections::append(qs, p)
    ps = collections::set(ps, i, WITH collections::get(ps, i) {{ age := 0 }})
  NEXT
  io::print(toString(ageSum(qs)) & \" \" & toString(ageSum(ps)))
  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("stored", &source);
    assert_eq!(lines, vec![format!("10 {}", AGES - 10)]);
    assert_eq!((alloc, live), (free, 0));
}

/// The container is overwritten, then grown, while a bound element is still
/// read; interleaved allocations reuse the freed memory.
#[test]
fn a_binding_that_outlives_a_container_write_is_a_copy() {
    let source = format!(
        "{TYPES}
FUNC main() AS Integer
  MUT ps AS List OF P = mk()
  LET a AS P = collections::get(ps, 0)
  ps = collections::set(ps, 0, P[lon := 9.0, age := 99, trail := [1.0]])
  LET b AS P = collections::get(ps, 1)
  FOR k = 1 TO 500
    ps = collections::append(ps, P[lon := 1.0, age := 7, trail := [2.0, 2.0, 2.0, 2.0, 2.0, 2.0]])
  NEXT
  MUT junk AS List OF List OF Integer = []
  FOR k = 1 TO 200
    junk = collections::append(junk, [k, k, k, k, k, k, k, k])
  NEXT
  io::print(toString(a.age) & \" \" & toString(len(a.trail)) & \" \" & toString(b.age) & \" \" & toString(len(b.trail)))
  io::print(toString(len(ps)) & \" \" & toString(len(junk)))
  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("outlives_write", &source);
    assert_eq!(lines, vec!["1 4 2 4", "564 200"]);
    assert_eq!((alloc, live), (free, 0));
}

/// A call that replaces the module-level container runs between the `get` and
/// the read.
#[test]
fn a_binding_across_a_call_that_writes_the_module_level_list_is_a_copy() {
    let source = format!(
        "{TYPES}{GLOBAL}
SUB clobber()
  gps = [P[lon := 3.0, age := 42, trail := [5.0]]]
  MUT junk AS List OF List OF Integer = []
  FOR k = 1 TO 100
    junk = collections::append(junk, [k, k, k, k, k, k, k, k])
  NEXT
END SUB

FUNC main() AS Integer
  gps = mk()
  LET p AS P = collections::get(gps, 2)
  clobber()
  io::print(toString(p.age) & \" \" & toString(len(p.trail)) & \" \" & toString(collections::get(gps, 0).age))
{GLOBAL_EXIT}  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("call_writes_global", &source);
    assert_eq!(lines, vec!["3 4 42"]);
    assert_no_leak("call_writes_global", (alloc, free, live), true);
}

/// The `get`'s own index operand replaces the module-level list: the read is of
/// the list as it was before the call (bug-496 snapshots it into a statement
/// temporary), and the binding must not point into that temporary.
#[test]
fn a_binding_whose_index_call_writes_the_module_level_list_is_a_copy() {
    let source = format!(
        "{TYPES}{GLOBAL}
FUNC clobber() AS Integer
  gps = [P[lon := 3.0, age := 42, trail := [5.0]]]
  RETURN 2
END FUNC

FUNC main() AS Integer
  gps = mk()
  LET p AS P = collections::get(gps, clobber())
  MUT junk AS List OF List OF Integer = []
  FOR k = 1 TO 100
    junk = collections::append(junk, [k, k, k, k, k, k, k, k])
  NEXT
  io::print(toString(p.age) & \" \" & toString(len(p.trail)) & \" \" & toString(len(gps)))
{GLOBAL_EXIT}  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("index_writes_global", &source);
    assert_eq!(lines, vec!["3 4 1"]);
    assert_no_leak("index_writes_global", (alloc, free, live), true);
}

/// A `MATCH` over `getOr(xs, k, <fresh default>)`: on a miss the binding holds
/// the default, which is a statement temporary. The plan-86 E borrow bound the
/// default's block and the statement freed it; the next allocation reused it.
#[test]
fn a_match_over_a_get_or_miss_reads_the_default() {
    let source = "\
IMPORT io
IMPORT collections

TYPE Circle
  r AS Integer
  tag AS List OF Integer
END TYPE

TYPE Square
  s AS Integer
END TYPE

UNION Shape
  Circle
  Square
END UNION

FUNC mk(n AS Integer) AS Shape
  RETURN Circle[r := n, tag := [n, n, n]]
END FUNC

FUNC main() AS Integer
  LET xs AS List OF Shape = [mk(1), mk(2)]
  MUT total AS Integer = 0
  FOR i = 0 TO 5
    LET e AS Shape = collections::getOr(xs, i + 10, mk(i + 100))
    LET junk AS List OF Integer = [7, 7, 7, 7, 7, 7]
    MATCH e
      CASE Circle(c)
        total = total + c.r + collections::get(c.tag, 2)
      CASE Square(s)
        total = total + s.s
    END MATCH
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
";
    let (lines, alloc, free, live) = run("get_or_default", source);
    assert_eq!(lines, vec![(2 * (600 + 15)).to_string()]);
    assert_eq!((alloc, live), (free, 0));
}

/// The bound element is read after the `set` that writes it back, and the list
/// is read between the `WITH` and the `set`: each read sees its own value.
#[test]
fn an_element_read_around_its_write_back_sees_each_value() {
    let source = format!(
        "{TYPES}
FUNC main() AS Integer
  MUT ps AS List OF P = mk()
  MUT p AS P = collections::get(ps, 0)
  p = WITH p {{ age := p.age + 10 }}
  io::print(toString(collections::get(ps, 0).age))
  ps = collections::set(ps, 0, p)
  io::print(toString(collections::get(ps, 0).age))
  MUT q AS P = collections::get(ps, 1)
  q = WITH q {{ age := q.age + 20 }}
  ps = collections::set(ps, 1, q)
  q = WITH q {{ age := q.age + 1 }}
  io::print(toString(q.age) & \" \" & toString(collections::get(ps, 1).age))
  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("around_write_back", &source);
    assert_eq!(lines, vec!["1", "11", "23 22"]);
    assert_eq!((alloc, live), (free, 0));
}

/// An element update that GROWS a field (an `append` to the trail) changes the
/// element's size; every element keeps its own values.
#[test]
fn an_element_update_that_grows_a_field_keeps_every_element() {
    let source = format!(
        "{TYPES}
FUNC main() AS Integer
  MUT ps AS List OF P = mk()
  FOR r = 1 TO 30
    FOR i = 0 TO n() - 1
      MUT p AS P = collections::get(ps, i)
      p = WITH p {{ age := p.age + 1, trail := collections::append(p.trail, toFloat(i)) }}
      ps = collections::set(ps, i, p)
    NEXT
  NEXT
  MUT lens AS Integer = 0
  FOR EACH p IN ps
    lens = lens + len(p.trail)
  NEXT
  io::print(toString(ageSum(ps)) & \" \" & toString(lens) & \" \" & toString(trailSum(ps)))
  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("grows_field", &source);
    // 30 appends of `i` to element i's trail: sum over i of 30 * i.
    assert_eq!(
        lines,
        vec![format!(
            "{} {} {:.2}",
            AGES + 30 * 64,
            64 * (4 + 30),
            30.0 * (63.0 * 64.0 / 2.0)
        )]
    );
    assert_eq!((alloc, live), (free, 0));
}

/// A failing element update leaves the module-level list exactly as it was:
/// a bad index at the `get`, a bad index inside a field operand, and a failing
/// scalar operand — in both spellings.
#[test]
fn a_failing_element_update_leaves_the_list_unchanged() {
    let source = format!(
        "{TYPES}{GLOBAL}
SUB bumpAt(i AS Integer, slot AS Integer, text AS String)
  MUT p AS P = collections::get(gps, i)
  p = WITH p {{ age := p.age + toInt(text), trail := collections::set(p.trail, slot, 1.0) }}
  gps = collections::set(gps, i, p)
END SUB

SUB bumpInline(i AS Integer, slot AS Integer, text AS String)
  gps = collections::set(gps, i, WITH collections::get(gps, i) {{ age := collections::get(gps, i).age + toInt(text), trail := collections::set(collections::get(gps, i).trail, slot, 1.0) }})
END SUB

FUNC main() AS Integer
  gps = mk()
  bumpAt(999, 0, \"1\") TRAP(e)
    RECOVER
  END TRAP
  bumpAt(0, 9, \"1\") TRAP(e)
    RECOVER
  END TRAP
  bumpAt(0, 0, \"x\") TRAP(e)
    RECOVER
  END TRAP
  bumpInline(999, 0, \"1\") TRAP(e)
    RECOVER
  END TRAP
  bumpInline(1, 9, \"1\") TRAP(e)
    RECOVER
  END TRAP
  bumpInline(1, 0, \"x\") TRAP(e)
    RECOVER
  END TRAP
  io::print(toString(ageSum(gps)) & \" \" & toString(trailSum(gps)))
  bumpAt(0, 0, \"5\")
  bumpInline(1, 2, \"7\")
  io::print(toString(ageSum(gps)) & \" \" & toString(trailSum(gps)))
{GLOBAL_EXIT}  RETURN 0
END FUNC
"
    );
    let (lines, alloc, free, live) = run("failing_update", &source);
    assert_eq!(
        lines,
        vec![format!("{AGES} 0.00"), format!("{} 2.00", AGES + 12)]
    );
    assert_no_leak("failing_update", (alloc, free, live), true);
}
