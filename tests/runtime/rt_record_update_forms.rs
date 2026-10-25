//! bug-766: a record update through a `List`/`Map` element, a helper function, or
//! a `transform` callback costs no per-update allocation — as the MUT spelling
//! already does.
//!
//! Each case runs its update loop `REPS` and `2 × REPS` times under `--debug`:
//! an update allocates nothing per rep (a `transform` exactly one per rep: its
//! result is a new list by value semantics), nothing may leak,
//! and the case must print what its **contrast** — the same update in a spelling
//! the compiler already serves without allocating (the `MUT p = get` / `p = WITH p`
//! / `set(xs, i, p)` form, or an inline `WITH`) — prints. Before the fix every
//! non-contrast case allocated one or two blocks per update (A: an owned copy of
//! the element and a fresh `WITH` block; B: an arena block per element read and
//! per `WITH` item when two elements live across a write; C: a fresh block per
//! helper call returning `WITH p { … }`; D: a rebuild for a same-size `String`
//! field; E: a callback result block per element).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const REPS: u32 = 50;

const PRELUDE: &str = "\
IMPORT io
IMPORT collections
IMPORT vector

TYPE Item
  id AS Integer
  name AS String
  score AS Float
END TYPE

TYPE Body
  x AS Float
  vx AS Float
  mass AS Float
END TYPE

TYPE VBody
  pos AS vector::Float3
  vel AS vector::Float3
  mass AS Float
END TYPE

TYPE Wide
  count AS Integer
  total AS Integer
  tag AS String
END TYPE

FUNC items(n AS Integer) AS List OF Item
  MUT xs AS List OF Item = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, Item[id := i, name := \"item\" & toString(i), score := 0.5])
  NEXT
  RETURN xs
END FUNC

FUNC bodies(n AS Integer) AS List OF Body
  MUT xs AS List OF Body = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, Body[x := toFloat(i), vx := 1.0, mass := 2.0 + toFloat(i)])
  NEXT
  RETURN xs
END FUNC

FUNC vbodies(n AS Integer) AS List OF VBody
  MUT xs AS List OF VBody = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, VBody[pos := vector::Float3[toFloat(i), 0.0, 0.0], vel := vector::Float3[1.0, 0.5, 0.25], mass := 2.0 + toFloat(i)])
  NEXT
  RETURN xs
END FUNC

FUNC stepNoTag(w AS Wide, i AS Integer) AS Wide
  RETURN WITH w { count := w.count + 1, total := w.total + i MOD 7 }
END FUNC

FUNC sumItems(xs AS List OF Item) AS Integer
  MUT h AS Integer = 0
  FOR EACH it IN xs
    h = (h * 31 + it.id + toInt(it.score * 4.0)) MOD 1000000007
  NEXT
  RETURN h
END FUNC

FUNC sumBodies(xs AS List OF Body) AS Integer
  MUT h AS Integer = 0
  FOR EACH b IN xs
    h = (h * 31 + toInt(b.x * 1000.0) + toInt(b.vx * 1000.0)) MOD 1000000007
  NEXT
  RETURN h
END FUNC

FUNC sumVBodies(xs AS List OF VBody) AS Integer
  MUT h AS Integer = 0
  FOR EACH b IN xs
    h = (h * 31 + toInt(b.pos.x * 1000.0) + toInt(b.vel.x * 1000.0)) MOD 1000000007
  NEXT
  RETURN h
END FUNC
";

/// `(name, setup, body, contrast body, result expression)`. `body` runs `REPS`
/// times; `contrast` is the same work in an already-allocation-free spelling.
const CASES: &[(&str, &str, &str, &str, &str)] = &[
    (
        "A1_with_as_set_item",
        "  MUT xs AS List OF Item = items(200)",
        "    FOR j = 0 TO 199\n      LET it AS Item = collections::get(xs, j)\n      xs = collections::set(xs, j, WITH it { id := it.id + 1, score := it.score + 0.5 })\n    NEXT",
        "    FOR j = 0 TO 199\n      MUT it AS Item = collections::get(xs, j)\n      it = WITH it { id := it.id + 1, score := it.score + 0.5 }\n      xs = collections::set(xs, j, it)\n    NEXT",
        "sumItems(xs)",
    ),
    (
        "A2_let_bound_with",
        "  MUT xs AS List OF Item = items(200)",
        "    FOR j = 0 TO 199\n      LET it AS Item = collections::get(xs, j)\n      LET u AS Item = WITH it { id := it.id + 1 }\n      xs = collections::set(xs, j, u)\n    NEXT",
        "    FOR j = 0 TO 199\n      MUT it AS Item = collections::get(xs, j)\n      it = WITH it { id := it.id + 1 }\n      xs = collections::set(xs, j, it)\n    NEXT",
        "sumItems(xs)",
    ),
    (
        "A3_flat_record",
        "  MUT xs AS List OF Body = bodies(200)",
        "    FOR j = 0 TO 199\n      LET b AS Body = collections::get(xs, j)\n      xs = collections::set(xs, j, WITH b { x := b.x + b.vx })\n    NEXT",
        "    FOR j = 0 TO 199\n      MUT b AS Body = collections::get(xs, j)\n      b = WITH b { x := b.x + b.vx }\n      xs = collections::set(xs, j, b)\n    NEXT",
        "sumBodies(xs)",
    ),
    (
        "A4_write_back_in_if_arms",
        "  MUT xs AS List OF Body = bodies(200)",
        "    FOR j = 0 TO 199\n      LET b AS Body = collections::get(xs, j)\n      IF j MOD 2 = 0 THEN\n        xs = collections::set(xs, j, WITH b { x := b.x + b.vx })\n      ELSE\n        xs = collections::set(xs, j, WITH b { x := b.x - b.vx })\n      END IF\n    NEXT",
        "    FOR j = 0 TO 199\n      MUT b AS Body = collections::get(xs, j)\n      IF j MOD 2 = 0 THEN\n        b = WITH b { x := b.x + b.vx }\n      ELSE\n        b = WITH b { x := b.x - b.vx }\n      END IF\n      xs = collections::set(xs, j, b)\n    NEXT",
        "sumBodies(xs)",
    ),
    (
        "A5_map_value",
        "  MUT mp AS Map OF Integer TO Body = Map OF Integer TO Body { }\n  FOR q = 0 TO 199\n    mp = collections::set(mp, q, Body[x := toFloat(q), vx := 1.0, mass := 2.0])\n  NEXT",
        "    FOR j = 0 TO 199\n      LET b AS Body = collections::get(mp, j)\n      mp = collections::set(mp, j, WITH b { x := b.x + b.vx })\n    NEXT",
        "    FOR j = 0 TO 199\n      MUT b AS Body = collections::get(mp, j)\n      b = WITH b { x := b.x + b.vx }\n      mp = collections::set(mp, j, b)\n    NEXT",
        "sumBodies(collections::values(mp))",
    ),
    (
        "B_two_elements_across_a_write",
        "  MUT xs AS List OF Body = bodies(5)",
        "    FOR i = 0 TO 3\n      FOR j = i + 1 TO 4\n        LET bi AS Body = collections::get(xs, i)\n        LET bj AS Body = collections::get(xs, j)\n        LET dx AS Float = bi.x - bj.x\n        xs = collections::set(xs, i, WITH bi { vx := bi.vx - dx * bj.mass * 0.001 })\n        xs = collections::set(xs, j, WITH bj { vx := bj.vx + dx * bi.mass * 0.001 })\n      NEXT\n    NEXT\n    FOR i = 0 TO 4\n      LET b AS Body = collections::get(xs, i)\n      xs = collections::set(xs, i, WITH b { x := b.x + b.vx * 0.01 })\n    NEXT",
        "    FOR i = 0 TO 3\n      FOR j = i + 1 TO 4\n        MUT bi AS Body = collections::get(xs, i)\n        LET bjx AS Float = collections::get(xs, j).x\n        LET bjm AS Float = collections::get(xs, j).mass\n        LET bim AS Float = bi.mass\n        LET dx AS Float = bi.x - bjx\n        bi = WITH bi { vx := bi.vx - dx * bjm * 0.001 }\n        xs = collections::set(xs, i, bi)\n        MUT bj AS Body = collections::get(xs, j)\n        bj = WITH bj { vx := bj.vx + dx * bim * 0.001 }\n        xs = collections::set(xs, j, bj)\n      NEXT\n    NEXT\n    FOR i = 0 TO 4\n      MUT b AS Body = collections::get(xs, i)\n      b = WITH b { x := b.x + b.vx * 0.01 }\n      xs = collections::set(xs, i, b)\n    NEXT",
        "sumBodies(xs)",
    ),
    (
        "B_vector_fields",
        "  MUT xs AS List OF VBody = vbodies(5)",
        "    FOR i = 0 TO 3\n      FOR j = i + 1 TO 4\n        LET bi AS VBody = collections::get(xs, i)\n        LET bj AS VBody = collections::get(xs, j)\n        LET dx AS Float = bi.pos.x - bj.pos.x\n        xs = collections::set(xs, i, WITH bi { vel := vector::Float3[bi.vel.x - dx * bj.mass * 0.001, bi.vel.y, bi.vel.z] })\n        xs = collections::set(xs, j, WITH bj { vel := vector::Float3[bj.vel.x + dx * bi.mass * 0.001, bj.vel.y, bj.vel.z] })\n      NEXT\n    NEXT",
        "    FOR i = 0 TO 3\n      FOR j = i + 1 TO 4\n        MUT bi AS VBody = collections::get(xs, i)\n        LET bjx AS Float = collections::get(xs, j).pos.x\n        LET bjm AS Float = collections::get(xs, j).mass\n        LET bim AS Float = bi.mass\n        LET dx AS Float = bi.pos.x - bjx\n        bi = WITH bi { vel := vector::Float3[bi.vel.x - dx * bjm * 0.001, bi.vel.y, bi.vel.z] }\n        xs = collections::set(xs, i, bi)\n        MUT bj AS VBody = collections::get(xs, j)\n        bj = WITH bj { vel := vector::Float3[bj.vel.x + dx * bim * 0.001, bj.vel.y, bj.vel.z] }\n        xs = collections::set(xs, j, bj)\n      NEXT\n    NEXT",
        "sumVBodies(xs)",
    ),
    (
        "C_helper_returning_with",
        "  MUT w AS Wide = Wide[count := 0, total := 0, tag := \"t\"]",
        "    FOR i = 0 TO 199\n      w = stepNoTag(w, i)\n    NEXT",
        "    FOR i = 0 TO 199\n      w = WITH w { count := w.count + 1, total := w.total + i MOD 7 }\n    NEXT",
        "w.count * 1000 + w.total",
    ),
    (
        "D_same_size_string_field",
        "  MUT w AS Wide = Wide[count := 0, total := 0, tag := \"odd\"]\n  LET tagOdd AS String = \"odd\"",
        "    FOR i = 0 TO 199\n      w = WITH w { count := w.count + 1, tag := tagOdd }\n    NEXT",
        "    FOR i = 0 TO 199\n      w = WITH w { count := w.count + 1 }\n    NEXT",
        "w.count * 1000 + len(w.tag)",
    ),
    (
        "E_transform_record_callback",
        "  LET xs AS List OF Item = items(200)\n  MUT acc AS Integer = 0",
        "    LET ys AS List OF Item = collections::transform(xs, LAMBDA(it AS Item) -> WITH it { score := it.score * 2.0 })\n    acc = (acc + sumItems(ys)) MOD 1000000007",
        "    MUT ys AS List OF Item = xs\n    FOR j = 0 TO len(ys) - 1\n      MUT it AS Item = collections::get(ys, j)\n      it = WITH it { score := it.score * 2.0 }\n      ys = collections::set(ys, j, it)\n    NEXT\n    acc = (acc + sumItems(ys)) MOD 1000000007",
        "acc",
    ),
];

fn program(setup: &str, body: &str, result: &str, reps: u32) -> String {
    format!(
        "{PRELUDE}
FUNC main() AS Integer
{setup}
  FOR rep = 1 TO {reps}
{body}
  NEXT
  io::print(toString({result}))
  RETURN 0
END FUNC
"
    )
}

/// Build with `--debug`, run, and return `(stdout, alloc_calls, free_calls,
/// live_bytes)` summed over the arenas.
fn run(name: &str, source: &str) -> (String, u64, u64, u64) {
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
        "{name}: build failed:\n{stdout}\n{}\n--- source ---\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|p| p.ends_with(common::host_executable_suffix()))
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
        "{name}: program failed:\n{text}\n{report}"
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
        text,
        sum(".alloc_calls "),
        sum(".free_calls "),
        sum(".live_bytes "),
    )
}

#[test]
fn a_record_update_allocates_nothing_per_update() {
    let mut failures = Vec::new();
    for (name, setup, body, contrast, result) in CASES {
        let (want, _, _, _) = run(
            &format!("b766_{name}_contrast"),
            &program(setup, contrast, result, REPS),
        );
        let (got, alloc, free, live) = run(
            &format!("b766_{name}_1"),
            &program(setup, body, result, REPS),
        );
        let (_, alloc2, free2, live2) = run(
            &format!("b766_{name}_2"),
            &program(setup, body, result, 2 * REPS),
        );
        if got != want {
            failures.push(format!(
                "{name}: printed {got:?}, the contrast spelling {want:?}"
            ));
            continue;
        }
        if alloc != free || live != 0 || alloc2 != free2 || live2 != 0 {
            failures.push(format!(
                "{name}: leaked ({alloc}/{free}/{live} B, {alloc2}/{free2}/{live2} B)"
            ));
            continue;
        }
        // An update allocates nothing per rep. A `transform` returns a new list
        // (value semantics), so its rep owes exactly that one block and nothing per
        // element.
        let allowed = if name.starts_with("E_") { 1 } else { 0 };
        let growth = alloc2.saturating_sub(alloc);
        if growth > allowed * u64::from(REPS) {
            failures.push(format!(
                "{name}: {alloc} allocations at {REPS} reps, {alloc2} at {} — {} per rep \
                 ({allowed} allowed)",
                2 * REPS,
                growth / u64::from(REPS)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "bug-766: a record update must allocate nothing per update:\n  {}",
        failures.join("\n  ")
    );
}
