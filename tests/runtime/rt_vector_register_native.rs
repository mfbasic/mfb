//! The nine `vector::` shapes run register-native, and only they do.
//!
//! bug-783: `vector_field_count` recognised a vector by its bare spelling
//! (`Float3`), but since the built-in package types became package-qualified the
//! builtin's identity is `vector.Float3`. So a `vector::Float3[…]` literal was
//! built as an arena block (one allocation per literal), while the inlined vector
//! ops, which constructed their result under the bare spelling, still matched.
//! And a user's own `TYPE Float3` — a bare spelling is always a user type — was
//! taken for the builtin and crashed the compiler.
//!
//! `a_vector_literal_or_inlined_op_allocates_nothing_per_iteration`: each case
//! runs its loop body `N` and `2N` times under `--debug`; the arena
//! `alloc_calls` must not grow with the iteration count beyond the case's
//! allowance (0 for a literal or an inlined op).
//!
//! The literals becoming register-native exposed a second cost: `RETURN` of a
//! register-native vector (every vector op's FUNC body ends in one) built the
//! block, copied it, and freed the first. `lower_returned_value` now claims the
//! materialized block, so a returned vector is one allocation.
//!
//! `a_user_type_named_like_a_vector_is_an_ordinary_record`: a user `TYPE Float3`
//! and `TYPE Integer2` with non-vector fields build, run, and print the right sum.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const N: u32 = 50;

/// `(name, body, allowed)`: the loop body, which must add a `Float` to `acc`, and
/// the arena allocations it may make per iteration. `i` is the `Integer` loop
/// counter, `f` is `toFloat(i)`; `a`/`b` are `vector::Float3` and `p`/`q` are
/// `vector::Float2` locals bound from literals (so an op over them inlines).
const CASES: &[(&str, &str, u64)] = &[
    (
        "literal_float3",
        "LET c AS vector::Float3 = vector::Float3[1.0, 2.0, f]\n    acc = acc + c.z",
        0,
    ),
    (
        "literal_float2",
        "LET c AS vector::Float2 = vector::Float2[f, 2.0]\n    acc = acc + c.x + c.y",
        0,
    ),
    (
        "literal_float4",
        "LET c AS vector::Float4 = vector::Float4[f, 2.0, 3.0, 4.0]\n    acc = acc + c.w",
        0,
    ),
    (
        "literal_integer3",
        "LET c AS vector::Integer3 = vector::Integer3[i, 2, 3]\n    acc = acc + toFloat(c.x + c.z)",
        0,
    ),
    (
        "literal_fixed2",
        "LET c AS vector::Fixed2 = vector::Fixed2[toFixed(f), toFixed(2.0)]\n    acc = acc + toFloat(c.x)",
        0,
    ),
    (
        "scale",
        "LET c AS vector::Float3 = vector::scale(a, b)\n    acc = acc + c.z",
        0,
    ),
    ("dot", "acc = acc + vector::dot(a, b)", 0),
    (
        "cross",
        "LET c AS vector::Float3 = vector::cross(a, b)\n    acc = acc + c.x + c.y + c.z",
        0,
    ),
    ("length", "acc = acc + vector::length(a)", 0),
    ("distance", "acc = acc + vector::distance(p, q)", 0),
    (
        "normalize",
        "LET c AS vector::Float3 = vector::normalize(a)\n    acc = acc + c.z",
        0,
    ),
    (
        "lerp",
        "LET c AS vector::Float2 = vector::lerp(p, q, 0.5)\n    acc = acc + c.y",
        0,
    ),
    // Literal operands do not inline (a literal is not re-evaluation safe), so the
    // op's FUNC body runs: one block per operand passed plus the returned one.
    (
        "scale_literal_operands",
        "LET c AS vector::Float3 = vector::scale(vector::Float3[1.0, 2.0, f], vector::Float3[0.5, 0.5, 0.5])\n    acc = acc + c.z",
        3,
    ),
    // A user FUNC returning a vector literal hands back one block per call.
    (
        "returned_literal",
        "LET c AS vector::Float3 = mk(f)\n    acc = acc + c.z",
        1,
    ),
];

fn program(body: &str, reps: u32) -> String {
    format!(
        "IMPORT io
IMPORT vector

FUNC mk(f AS Float) AS vector::Float3
  RETURN vector::Float3[1.0, 2.0, f]
END FUNC

FUNC main() AS Integer
  MUT acc AS Float = 0.0
  FOR i = 1 TO {reps}
    LET f AS Float = toFloat(i)
    LET a AS vector::Float3 = vector::Float3[1.0, 2.0, f]
    LET b AS vector::Float3 = vector::Float3[f, 1.0, 0.5]
    LET p AS vector::Float2 = vector::Float2[f, 0.0]
    LET q AS vector::Float2 = vector::Float2[0.0, 1.0]
    {body}
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
"
    )
}

/// Build (with `--debug` when `debug`), run, and return `(stdout, stderr)`.
fn run(name: &str, source: &str, debug: bool) -> (String, String) {
    let project = common::temp_project(name, source);
    let mut build = Command::new(common::mfb_exe());
    build.arg("build");
    if debug {
        build.arg("--debug");
    }
    let output = build.arg(&project).output().expect("run mfb build");
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
    (text, report)
}

/// `alloc_calls` summed over the arenas of a `--debug` report.
fn alloc_calls(report: &str) -> u64 {
    report
        .lines()
        .filter_map(|line| line.strip_prefix("arena."))
        .filter_map(|rest| rest.split_once(".alloc_calls "))
        .filter(|(arena, _)| !arena.contains('.'))
        .map(|(_, n)| n.trim().parse::<u64>().expect("a count"))
        .sum()
}

#[test]
fn a_vector_literal_or_inlined_op_allocates_nothing_per_iteration() {
    let mut failures = Vec::new();
    for (name, body, allowed) in CASES {
        let (_, once) = run(&format!("vecnat_{name}_1"), &program(body, N), true);
        let (_, twice) = run(&format!("vecnat_{name}_2"), &program(body, 2 * N), true);
        let (a, b) = (alloc_calls(&once), alloc_calls(&twice));
        if b > a + allowed * u64::from(N) {
            failures.push(format!(
                "{name}: alloc_calls {a} at {N} iterations, {b} at {} ({} per iteration)",
                2 * N,
                (b as f64 - a as f64) / f64::from(N)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "a vector literal or inlined vector op must stay in registers:\n  {}",
        failures.join("\n  ")
    );
}

#[test]
fn a_user_type_named_like_a_vector_is_an_ordinary_record() {
    let source = "IMPORT io

TYPE Float3
  name AS String
  count AS Integer
  weight AS Float
END TYPE

TYPE Integer2
  a AS String
  b AS String
END TYPE

FUNC main() AS Integer
  MUT acc AS Integer = 0
  FOR i = 1 TO 10
    LET c AS Float3 = Float3[\"n\" & toString(i), i, 0.5]
    LET p AS Integer2 = Integer2[c.name, \"xy\"]
    acc = acc + c.count + len(p.a) + len(p.b)
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
";
    // Σ i = 55, Σ len(\"n\" & i) = 9 × 2 + 3 = 21, Σ len(\"xy\") = 20.
    let (text, _) = run("vecnat_user_float3", source, false);
    assert_eq!(text, "96\n");
}

/// A vector literal handed over to a callee that returns it (`RETURN v`) belongs
/// to the callee: the caller must not also free it. The literal is claimed off the
/// pending-temp list only once it is a block, so the claim runs after the
/// register-native value is materialized (found by the acceptance suite's
/// `vector::clamp_length` within-max case, which read `0.00` from a freed block).
#[test]
fn a_handed_over_vector_literal_is_freed_once() {
    let source = "IMPORT io
IMPORT vector

FUNC pick(v AS vector::Float2, big AS Boolean) AS vector::Float2
  IF big THEN
    RETURN vector::Float2[0.0, 0.0]
  END IF
  RETURN v
END FUNC

FUNC main() AS Integer
  LET w AS vector::Float2 = vector::clamp_length(vector::Float2[1.0, 1.0], 10.0)
  LET p AS vector::Float2 = pick(vector::Float2[3.0, 4.0], FALSE)
  LET q AS vector::Float2 = pick(vector::Float2[3.0, 4.0], TRUE)
  io::print(toString(w.x) & \" \" & toString(w.y) & \" \" & toString(p.x) & \" \" & toString(p.y) & \" \" & toString(q.x))
  RETURN 0
END FUNC
";
    let (text, report) = run("vecnat_handover", source, true);
    assert_eq!(text, "1.00 1.00 3.00 4.00 0.00\n");
    let count = |key: &str| -> u64 {
        report
            .lines()
            .filter_map(|line| line.strip_prefix("arena."))
            .filter_map(|rest| rest.split_once(key))
            .filter(|(arena, _)| !arena.contains('.'))
            .map(|(_, n)| n.trim().parse::<u64>().expect("a count"))
            .sum()
    };
    assert_eq!(count(".double_free_skips "), 0, "{report}");
    assert_eq!(count(".alloc_calls "), count(".free_calls "), "{report}");
}
