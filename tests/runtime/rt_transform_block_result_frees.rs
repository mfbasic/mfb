//! bug-677: `collections::transform` frees a callback's block result.
//!
//! The `FunctionRef` ABI owns the callback's result (bug-569), and the append
//! byte-copies it into the output list, so the returned block is dead after the
//! append. bug-569 freed it only for a `String` result: a record, a data union or
//! a collection result leaked one block per element per call — `transform` of
//! three elements through `FUNC(Integer) AS P` left 96 bytes live, and the
//! in-place self-update `xs = transform(xs, f)` over a `List OF P` leaked the same
//! from its parked results, as did `m = mapValues(m, f)` over a record-valued map,
//! and both on a failing callback's unwind. Found by plan-145-D's differential probe. Each program
//! must end with every block it allocated freed, and print the right result —
//! including a callback that hands its (borrowed) argument back.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const HELPERS: &str = "\
IMPORT collections
IMPORT io

TYPE P
  k AS Integer
  s AS String
END TYPE

TYPE Q
  a AS Integer
  b AS Integer
END TYPE

TYPE Circle
  r AS Integer
END TYPE

TYPE Sq
  w AS Integer
END TYPE

UNION Shape
  Circle
  Sq
END UNION

FUNC mkP(n AS Integer) AS P
  RETURN P[k := n, s := \"made\"]
END FUNC

FUNC mkQ(n AS Integer) AS Q
  RETURN Q[a := n, b := n]
END FUNC

FUNC mkL(n AS Integer) AS List OF Integer
  RETURN [n, n, n]
END FUNC

FUNC mkU(n AS Integer) AS Shape
  RETURN Circle[r := n]
END FUNC

FUNC upP(p AS P) AS P
  RETURN P[k := p.k + 1, s := p.s & \"!\"]
END FUNC

FUNC idP(p AS P) AS P
  RETURN p
END FUNC

FUNC upFailOnOne(p AS P) AS P
  IF p.k = 1 THEN
    FAIL error(77050002, \"one\")
  END IF
  RETURN P[k := p.k + 1, s := p.s]
END FUNC

FUNC failingSelf(xs AS List OF P) AS Integer
  MUT ys AS List OF P = xs
  ys = collections::transform(ys, upFailOnOne)
  RETURN 0

  TRAP(e)
    RETURN len(ys)
  END TRAP
END FUNC

FUNC failingMapSelf() AS Integer
  MUT m AS Map OF String TO P = Map OF String TO P { \"a\" := P[k := 3, s := \"x\"], \"b\" := P[k := 1, s := \"y\"] }
  m = collections::mapValues(m, upFailOnOne)
  RETURN 0

  TRAP(e)
    RETURN len(m)
  END TRAP
END FUNC

FUNC showP(xs AS List OF P) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v.k) & v.s & \",\"
  NEXT
  RETURN out
END FUNC
";

/// `(name, statements printing a result, expected output)`. `ns` is `[1, 2, 3]`
/// and `ps` three `P` records.
const CASES: &[(&str, &str, &str)] = &[
    (
        "record",
        "LET r AS List OF P = collections::transform(ns, mkP)\n  io::print(showP(r))",
        "1made,2made,3made,",
    ),
    (
        "fixed_record",
        "LET r AS List OF Q = collections::transform(ns, mkQ)\n  io::print(toString(collections::get(r, 2).b))",
        "3",
    ),
    (
        "list",
        "LET r AS List OF List OF Integer = collections::transform(ns, mkL)\n  io::print(toString(len(collections::get(r, 1))))",
        "3",
    ),
    (
        "union",
        "LET r AS List OF Shape = collections::transform(ns, mkU)\n  io::print(toString(len(r)))",
        "3",
    ),
    (
        "record_to_record",
        "LET r AS List OF P = collections::transform(ps, upP)\n  io::print(showP(r))",
        "4x!,3yy!,2z!,",
    ),
    (
        "self_update",
        "ps = collections::transform(ps, upP)\n  ps = collections::transform(ps, upP)\n  io::print(showP(ps))",
        "5x!!,4yy!!,3z!!,",
    ),
    (
        "identity",
        "LET r AS List OF P = collections::transform(ps, idP)\n  \
         LET t AS List OF P = collections::transform(ps, LAMBDA(q AS P) -> q)\n  \
         ps = collections::transform(ps, idP)\n  io::print(showP(r) & showP(t) & showP(ps))",
        "3x,2yy,1z,3x,2yy,1z,3x,2yy,1z,",
    ),
    (
        "map_values_self_update",
        "MUT m AS Map OF String TO P = Map OF String TO P { \"a\" := P[k := 1, s := \"x\"], \"b\" := P[k := 2, s := \"yy\"] }\n  \
         m = collections::mapValues(m, upP)\n  m = collections::mapValues(m, upP)\n  \
         io::print(collections::get(m, \"b\").s)",
        "yy!!",
    ),
    (
        "failing_callback_self_update",
        "io::print(toString(failingSelf(ps)) & toString(failingMapSelf()))",
        "32",
    ),
];

#[test]
fn a_block_result_is_freed_after_the_append() {
    let mut failures = Vec::new();
    for (name, statement, want) in CASES {
        let source = format!(
            "{HELPERS}\nFUNC main() AS Integer\n  LET ns AS List OF Integer = [1, 2, 3]\n  \
             MUT ps AS List OF P = [P[k := 3, s := \"x\"], P[k := 2, s := \"yy\"], P[k := 1, s := \"z\"]]\n  \
             {statement}\n  RETURN 0\nEND FUNC\n"
        );
        let project = common::temp_project(&format!("transform_block_{name}"), &source);
        let output = Command::new(common::mfb_exe())
            .arg("build")
            .arg("--debug")
            .arg(&project)
            .output()
            .expect("run mfb build --debug");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(output.status.success(), "{name}: build failed:\n{stdout}");
        let exe = stdout
            .lines()
            .filter_map(|line| line.strip_prefix("Wrote executable to "))
            .find(|path| path.ends_with("-glibc.out"))
            .or_else(|| {
                stdout
                    .lines()
                    .find_map(|line| line.strip_prefix("Wrote executable to "))
            })
            .expect("an executable");
        let run = Command::new(exe).output().expect("run the program");
        let _ = std::fs::remove_dir_all(&project);
        let out = String::from_utf8_lossy(&run.stdout);
        let err = String::from_utf8_lossy(&run.stderr);
        let counter = |key: &str| -> u64 {
            err.lines()
                .find_map(|l| l.strip_prefix(&format!("arena.0.{key} ")))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(u64::MAX)
        };
        let (alloc, free, live) = (
            counter("alloc_calls"),
            counter("free_calls"),
            counter("live_bytes"),
        );
        if !run.status.success() || out.lines().next() != Some(*want) || alloc != free || live != 0
        {
            failures.push(format!(
                "{name}: printed {:?} (want {want}), {alloc} allocated / {free} freed / {live} B live",
                out.trim()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
