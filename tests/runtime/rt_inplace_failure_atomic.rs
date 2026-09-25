//! plan-142: a self-update that fails leaves its binding unchanged.
//!
//! `x = op(x, …)` is an assignment, and an assignment whose value fails never
//! happens (`mfb spec language memory-semantics` §14): a function-level `TRAP`
//! handler that reads `x` must see the value from before the statement. An
//! in-place arm writes `x`'s own block, so it must detect every failure it can
//! raise — a callback's error, an invalid range — **before** its first write. Each
//! case runs the failing self-update inside a function whose handler prints `x`,
//! and requires the original contents (`tests/runtime/rt_inplace_self_update.rs`
//! separately proves the arm is the path taken).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const HELPERS: &str = "\
IMPORT collections
IMPORT io
IMPORT math

FUNC failOnThree(n AS Integer) AS Boolean
  IF n = 3 THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN n MOD 2 = 0
END FUNC

FUNC failOnThreeStr(s AS String) AS Boolean
  IF s = \"c\" THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN s <> \"a\"
END FUNC

FUNC tenFailOnThree(n AS Integer) AS Integer
  IF n = 3 THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN n * 10
END FUNC

FUNC growFailOnC(s AS String) AS String
  IF s = \"c\" THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN s & \"-grown-well-past-its-old-length\"
END FUNC

FUNC keyFailOnThree(n AS Integer) AS Integer
  IF n = 3 THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN 0 - n
END FUNC

FUNC longFailOnC(v AS String) AS String
  IF v = \"c\" THEN
    FAIL error(77050002, \"three\")
  END IF
  RETURN v & \"-made-longer-than-before\"
END FUNC

FUNC showMap(m AS Map OF String TO String) AS String
  MUT out AS String = \"\"
  FOR EACH k IN collections::keys(m)
    out = out & k & \"=\" & collections::get(m, k) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showInts(xs AS List OF Integer) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showFloats(xs AS List OF Float) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showStrs(xs AS List OF String) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & v & \",\"
  NEXT
  RETURN out
END FUNC
";

/// `(case, declaration of x, the failing self-update, renderer, expected output)`.
const CASES: &[(&str, &str, &str, &str, &str)] = &[
    (
        "filter predicate fails on the 3rd element",
        "MUT x AS List OF Integer = [1, 2, 3, 4, 5]",
        "x = collections::filter(x, failOnThree)",
        "showInts",
        "1,2,3,4,5,",
    ),
    (
        "filter predicate fails, String list",
        "MUT x AS List OF String = [\"a\", \"b\", \"c\", \"d\"]",
        "x = collections::filter(x, failOnThreeStr)",
        "showStrs",
        "a,b,c,d,",
    ),
    (
        "mid with a negative count",
        "MUT x AS List OF Integer = [1, 2, 3, 4, 5]",
        "x = collections::mid(x, 1, -1)",
        "showInts",
        "1,2,3,4,5,",
    ),
    (
        "mid past the end",
        "MUT x AS List OF String = [\"a\", \"b\", \"c\"]",
        "x = collections::mid(x, 2, 5)",
        "showStrs",
        "a,b,c,",
    ),
    (
        "math::sqrt over a negative element (plan-142-C)",
        "MUT x AS List OF Float = [4.0, 9.0, -1.0, 16.0]",
        "x = math::sqrt(x)",
        "showFloats",
        "4.00,9.00,-1.00,16.00,",
    ),
    (
        "math::log over a zero element (plan-142-C)",
        "MUT x AS List OF Float = [1.0, 0.0, 2.0]",
        "x = math::log(x)",
        "showFloats",
        "1.00,0.00,2.00,",
    ),
    (
        "transform callback fails on the 3rd element (plan-142-C)",
        "MUT x AS List OF Integer = [1, 2, 3, 4]",
        "x = collections::transform(x, tenFailOnThree)",
        "showInts",
        "1,2,3,4,",
    ),
    (
        "transform callback fails, String list with growing results (plan-142-C)",
        "MUT x AS List OF String = [\"a\", \"b\", \"c\", \"d\"]",
        "x = collections::transform(x, growFailOnC)",
        "showStrs",
        "a,b,c,d,",
    ),
    (
        "sortBy key function fails (plan-142-C)",
        "MUT x AS List OF Integer = [4, 1, 3, 2]",
        "x = collections::sortBy(x, keyFailOnThree)",
        "showInts",
        "4,1,3,2,",
    ),
    (
        "mapValues callback fails on the 3rd value (plan-142-D)",
        "MUT x AS Map OF String TO String = Map OF String TO String { \"p\" := \"a\", \"q\" := \"b\", \"r\" := \"c\", \"s\" := \"d\" }",
        "x = collections::mapValues(x, longFailOnC)",
        "showMap",
        "p=a,q=b,r=c,s=d,",
    ),
    (
        "mid with a negative start",
        "MUT x AS List OF Integer = [1, 2, 3]",
        "x = collections::mid(x, -1, 1)",
        "showInts",
        "1,2,3,",
    ),
];

fn program(decl: &str, statement: &str, show: &str) -> String {
    format!(
        "{HELPERS}
FUNC run() AS Integer
  {decl}
  {statement}
  io::print(\"not reached \" & {show}(x))
  RETURN 0

  TRAP(e)
    io::print({show}(x))
    RETURN 1
  END TRAP
END FUNC

FUNC main() AS Integer
  io::print(\"run -> \" & toString(run()))
  RETURN 0
END FUNC
"
    )
}

#[test]
fn a_failing_self_update_leaves_the_binding_unchanged() {
    let mut failures = Vec::new();
    for (index, (case, decl, statement, show, want)) in CASES.iter().enumerate() {
        let project = common::temp_project(
            &format!("inplace_atomic_{index}"),
            &program(decl, statement, show),
        );
        let exe = common::build_project(&project);
        let output = Command::new(&exe).output().expect("run program");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let expected = format!("{want}\nrun -> 1\n");
        if !output.status.success() || stdout != expected {
            failures.push(format!(
                "{case}: `{statement}` — want {expected:?}, got {stdout:?} ({}), stderr {:?}",
                common::exit_description(&output.status),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// plan-145-D: the same guarantee at a field. `filter`'s failing predicate, `mid`'s
/// bad range and a `math` domain error, each over a record field and a `STATE`
/// field, leave the field (and its siblings) as they were. `b` and `f` are NOT
/// the record's last inlined field — the arms mutate a middle sub-block where it
/// lies (plan-145-A Open Decision 2). The whole program then frees every byte it
/// allocated: a shrunk sub-block must not leak the record's tail (`live_bytes 0`
/// in the `--debug` report). The `STATE` handle is a `RES` parameter, so the
/// handler may read it (bug-676).
#[test]
fn a_failing_field_self_update_leaves_the_owner_unchanged() {
    // (name, field, the failing update's value with `@` for the owner)
    let cases: &[(&str, &str, &str)] = &[
        ("filter", "b", "collections::filter(@.b, failOnThree)"),
        ("mid", "b", "collections::mid(@.b, 1, -1)"),
        ("sqrt", "f", "math::sqrt(@.f)"),
    ];
    let mut source = format!(
        "{HELPERS}
IMPORT fs

TYPE R
  a AS Integer
  b AS List OF Integer
  f AS List OF Float
  c AS List OF Integer
END TYPE

FUNC fresh() AS R
  RETURN R[a := 1, b := [1, 2, 3, 4, 5], f := [4.0, 9.0, -1.0], c := [7]]
END FUNC

FUNC show(r AS R) AS String
  RETURN toString(r.a) & \"|\" & showInts(r.b) & \"|\" & showFloats(r.f) & \"|\" & showInts(r.c)
END FUNC
"
    );
    let mut calls = String::new();
    let mut want = Vec::new();
    let original = "1|1,2,3,4,5,|4.00,9.00,-1.00,|7,";
    for (name, field, update) in cases {
        let rec = update.replace('@', "r");
        let st = update.replace('@', "h.state");
        source.push_str(&format!(
            "
FUNC rec_{name}() AS Integer
  MUT r AS R = fresh()
  r = WITH r {{ {field} := {rec} }}
  io::print(\"not reached\")
  RETURN 0

  TRAP(e)
    io::print(\"rec {name} \" & show(r))
    RETURN 1
  END TRAP
END FUNC

FUNC st_{name}(RES h AS fs::File STATE R) AS Integer
  h.state = WITH h.state {{ {field} := {st} }}
  io::print(\"not reached\")
  RETURN 0

  TRAP(e)
    io::print(\"state {name} \" & show(h.state))
    RETURN 1
  END TRAP
END FUNC
"
        ));
        calls.push_str(&format!(
            "  io::print(toString(rec_{name}()))\n  \
             h.state = fresh()\n  \
             io::print(toString(st_{name}(h)))\n  \
             io::print(\"after {name} \" & show(h.state))\n"
        ));
        want.push(format!("rec {name} {original}"));
        want.push("1".to_string());
        want.push(format!("state {name} {original}"));
        want.push("1".to_string());
        want.push(format!("after {name} {original}"));
    }
    source.push_str(&format!(
        "
FUNC main() AS Integer
  RES h AS fs::File STATE R = fs::openFile(\"/dev/null\")
{calls}  RETURN 0
END FUNC
"
    ));

    let project = common::temp_project("inplace_atomic_fields", &source);
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
    let run = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    let text = String::from_utf8_lossy(&run.stdout).into_owned();
    let report = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(
        run.status.success(),
        "program failed ({}):\n{text}\n{report}\n--- source ---\n{source}",
        common::exit_description(&run.status)
    );
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    assert_eq!(
        lines, want,
        "a failed field update must leave the owner as it was"
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
    assert_eq!(
        (sum(".alloc_calls "), sum(".live_bytes ")),
        (sum(".free_calls "), 0),
        "the field cases leaked:\n{report}"
    );
}

/// plan-145-E: `union` and `merge` at a last-inlined record field and `STATE`
/// field. Their only possible failures are an operand's (here a callback that
/// fails) and `ErrOutOfMemory` at the reserve, both before the first write, so a
/// failing statement leaves the owner as it was; and the grown record is freed
/// whole (`live_bytes 0`).
#[test]
fn a_failing_reallocating_field_update_leaves_the_owner_unchanged() {
    let source = format!(
        "{HELPERS}
IMPORT fs

TYPE U
  a AS Integer
  s AS Set OF String
END TYPE

TYPE M
  a AS Integer
  m AS Map OF String TO String
END TYPE

FUNC failSet() AS Set OF String
  FAIL error(77050002, \"set\")
END FUNC

FUNC failBool() AS Boolean
  FAIL error(77050002, \"bool\")
END FUNC

FUNC showU(u AS U) AS String
  MUT l AS List OF String = []
  FOR EACH v IN u.s
    l = collections::append(l, v)
  NEXT
  RETURN toString(u.a) & \"|\" & showStrs(collections::sort(l))
END FUNC

FUNC freshU() AS U
  RETURN U[a := 1, s := Set OF String {{ \"x\", \"y\" }}]
END FUNC

FUNC freshM() AS M
  RETURN M[a := 1, m := Map OF String TO String {{ \"k\" := \"v\" }}]
END FUNC

FUNC recUnion() AS Integer
  MUT r AS U = freshU()
  FOR i = 1 TO 40
    r = WITH r {{ s := collections::union(r.s, Set OF String {{ \"grow-\" & toString(i) }}) }}
  NEXT
  r = WITH r {{ s := collections::union(r.s, failSet()) }}
  RETURN 0

  TRAP(e)
    io::print(\"rec union \" & toString(len(r.s)) & \" \" & toString(r.a))
    RETURN 1
  END TRAP
END FUNC

FUNC stMerge(RES h AS fs::File STATE M) AS Integer
  FOR i = 1 TO 40
    h.state = WITH h.state {{ m := collections::merge(h.state.m, Map OF String TO String {{ \"k\" & toString(i) := \"value\" }}, TRUE) }}
  NEXT
  LET n AS Map OF String TO String = Map OF String TO String {{ \"z\" := \"zz\" }}
  h.state = WITH h.state {{ m := collections::merge(h.state.m, n, failBool()) }}
  RETURN 0

  TRAP(e)
    io::print(\"state merge \" & showMap(h.state.m) & \" \" & toString(h.state.a))
    RETURN 1
  END TRAP
END FUNC

FUNC main() AS Integer
  io::print(toString(recUnion()))
  RES h AS fs::File STATE M = fs::openFile(\"/dev/null\")
  h.state = freshM()
  io::print(toString(stMerge(h)))
  io::print(\"after \" & toString(len(h.state.m)))
  RETURN 0
END FUNC
"
    );
    // `showMap` walks the keys in insertion order: the entry key, then the 40 merged.
    let mut merged = String::from("k=v,");
    for i in 1..=40 {
        merged.push_str(&format!("k{i}=value,"));
    }
    let want = vec![
        "rec union 42 1".to_string(),
        "1".to_string(),
        format!("state merge {merged} 1"),
        "1".to_string(),
        "after 41".to_string(),
    ];

    let project = common::temp_project("inplace_atomic_realloc_fields", &source);
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
    let run = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    let text = String::from_utf8_lossy(&run.stdout).into_owned();
    let report = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(
        run.status.success(),
        "program failed ({}):\n{text}\n{report}\n--- source ---\n{source}",
        common::exit_description(&run.status)
    );
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    assert_eq!(
        lines, want,
        "a failed reallocating field update must leave the owner as it was"
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
    assert_eq!(
        (sum(".alloc_calls "), sum(".live_bytes ")),
        (sum(".free_calls "), 0),
        "the reallocating field cases leaked:\n{report}"
    );
}
