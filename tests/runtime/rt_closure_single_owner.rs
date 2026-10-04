//! bug-781: a capturing closure is owned by exactly one holder and freed with it.
//!
//! A function value used to be a shared pointer with no owner (the retired
//! arena-lifetime closure rule): a capturing `LAMBDA` stored in a record field,
//! a collection, a union or `Result` payload, or returned from a function was
//! never freed before arena teardown, so every evaluation leaked its closure
//! object and environment (2 blocks, 32 B for one captured `Float`). Output was
//! right, so only the arena counters show it.
//!
//! Under single-owner closures (`mfb spec memory closures`, *Ownership*) copying
//! a function value copies the closure object and its environment, and dropping
//! a holder frees both. Each case builds its closures ten times in a loop; a
//! `--debug` run must print the expected value and end with
//! `alloc_calls = free_calls`, `live_bytes 0` and no double-free skips.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// `(name, program, expected stdout)`.
const CASES: &[(&str, &str, &str)] = &[
    (
        "keep_field_read",
        "\
IMPORT io

TYPE Holder
  f AS FUNC(Float) AS Float
END TYPE

FUNC keep(f AS FUNC(Float) AS Float) AS Holder
  RETURN Holder[f := f]
END FUNC

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET factor AS Float = toFloat(k)
    LET h AS Holder = keep(LAMBDA(v AS Float) -> v + factor)
    LET g AS FUNC(Float) AS Float = h.f
    n = n + toInt(g(1.0))
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "65\n",
    ),
    (
        "direct_construction",
        "\
IMPORT io

TYPE Holder
  f AS FUNC(Float) AS Float
END TYPE

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET factor AS Float = toFloat(k)
    LET h AS Holder = Holder[f := LAMBDA(v AS Float) -> v + factor]
    LET g AS FUNC(Float) AS Float = h.f
    n = n + toInt(g(1.0))
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "65\n",
    ),
    (
        "record_in_list",
        "\
IMPORT io
IMPORT collections

TYPE Holder
  f AS FUNC(Float) AS Float
END TYPE

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET factor AS Float = toFloat(k)
    LET hs AS List OF Holder = [Holder[f := LAMBDA(v AS Float) -> v + factor], Holder[f := LAMBDA(v AS Float) -> v * factor]]
    FOR EACH h IN hs
      LET g AS FUNC(Float) AS Float = h.f
      n = n + toInt(g(1.0))
    NEXT
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "120\n",
    ),
    (
        "record_reassigned",
        "\
IMPORT io

TYPE Holder
  f AS FUNC(Float) AS Float
END TYPE

FUNC main() AS Integer
  MUT n AS Integer = 0
  MUT h AS Holder = Holder[f := LAMBDA(v AS Float) -> v]
  FOR k = 1 TO 10
    LET factor AS Float = toFloat(k)
    h = Holder[f := LAMBDA(v AS Float) -> v + factor]
    LET g AS FUNC(Float) AS Float = h.f
    n = n + toInt(g(1.0))
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "65\n",
    ),
    (
        "closure_in_map",
        "\
IMPORT io
IMPORT collections

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET factor AS Float = toFloat(k)
    MUT m AS Map OF String TO FUNC(Float) AS Float = Map OF String TO FUNC(Float) AS Float {}
    m = collections::set(m, \"add\", LAMBDA(v AS Float) -> v + factor)
    m = collections::set(m, \"mul\", LAMBDA(v AS Float) -> v * factor)
    LET g AS FUNC(Float) AS Float = collections::get(m, \"add\")
    n = n + toInt(g(1.0))
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "65\n",
    ),
    (
        "union_payload",
        "\
IMPORT io

TYPE Adder
  f AS FUNC(Float) AS Float
END TYPE
TYPE Nop
  z AS Integer
END TYPE
UNION Op
  Adder
  Nop
END UNION

FUNC run(o AS Op) AS Float
  MATCH o
    CASE Adder(a)
      LET g AS FUNC(Float) AS Float = a.f
      RETURN g(1.0)
    CASE Nop(x)
      RETURN 0.0
  END MATCH
END FUNC

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET factor AS Float = toFloat(k)
    LET o AS Op = Adder[LAMBDA(v AS Float) -> v + factor]
    n = n + toInt(run(o))
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "65\n",
    ),
    (
        "result_payload_function_trap",
        "\
IMPORT io

FUNC mk(k AS Integer) AS FUNC(Float) AS Float
  IF k > 100 THEN
    FAIL error(7, \"too big\")
  END IF
  LET factor AS Float = toFloat(k)
  RETURN LAMBDA(v AS Float) -> v + factor
END FUNC

FUNC use(k AS Integer) AS Integer
  LET g AS FUNC(Float) AS Float = mk(k)
  RETURN toInt(g(1.0))
  TRAP(e)
    RETURN -1
  END TRAP
END FUNC

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    n = n + use(k)
  NEXT
  n = n + use(1000)
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "64\n",
    ),
    (
        "result_payload_inline_trap",
        "\
IMPORT io

FUNC mk(k AS Integer) AS FUNC(Float) AS Float
  IF k > 100 THEN
    FAIL error(7, \"too big\")
  END IF
  LET factor AS Float = toFloat(k)
  RETURN LAMBDA(v AS Float) -> v + factor
END FUNC

FUNC ident(v AS Float) AS Float
  RETURN v
END FUNC

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET g AS FUNC(Float) AS Float = mk(k) TRAP(e)
      RECOVER ident
    END TRAP
    n = n + toInt(g(1.0))
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "65\n",
    ),
    (
        "returned_and_copied",
        "\
IMPORT io

FUNC mk(k AS Integer) AS FUNC(Float) AS Float
  LET factor AS Float = toFloat(k)
  RETURN LAMBDA(v AS Float) -> v + factor
END FUNC

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET g AS FUNC(Float) AS Float = mk(k)
    LET g2 AS FUNC(Float) AS Float = g
    n = n + toInt(g(1.0)) + toInt(g2(0.0))
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "120\n",
    ),
    (
        "list_of_func",
        "\
IMPORT io
IMPORT collections

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET factor AS Float = toFloat(k)
    MUT fs AS List OF FUNC(Float) AS Float = []
    fs = collections::append(fs, LAMBDA(v AS Float) -> v + factor)
    fs = collections::append(fs, LAMBDA(v AS Float) -> v * factor)
    LET second AS FUNC(Float) AS Float = collections::get(fs, 1)
    n = n + toInt(second(2.0))
    FOR EACH f IN fs
      n = n + toInt(f(1.0))
    NEXT
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "230\n",
    ),
    (
        "nested_record_copied",
        "\
IMPORT io

TYPE Holder
  f AS FUNC(Float) AS Float
  label AS String
END TYPE
TYPE Outer
  inner AS Holder
  count AS Integer
END TYPE

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET factor AS Float = toFloat(k)
    LET o AS Outer = Outer[Holder[LAMBDA(v AS Float) -> v + factor, \"x\" & toString(k)], k]
    LET o2 AS Outer = o
    LET g AS FUNC(Float) AS Float = o2.inner.f
    n = n + toInt(g(1.0)) + len(o.inner.label)
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "86\n",
    ),
    (
        "string_capture_record_copied",
        "\
IMPORT io

TYPE Greeter
  greet AS FUNC(String) AS String
END TYPE

FUNC make(prefix AS String) AS Greeter
  LET p AS String = prefix & \", \"
  RETURN Greeter[LAMBDA(name AS String) -> p & name]
END FUNC

FUNC main() AS Integer
  MUT total AS Integer = 0
  FOR k = 1 TO 10
    LET g AS Greeter = make(\"Hello number \" & toString(k))
    LET copy AS Greeter = g
    LET f AS FUNC(String) AS String = copy.greet
    total = total + len(f(\"world\"))
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
",
        "211\n",
    ),
    (
        "closure_capturing_closure",
        "\
IMPORT io

FUNC adder(k AS Integer) AS FUNC(Integer) AS Integer
  RETURN LAMBDA(v AS Integer) -> v + k
END FUNC

FUNC twice(k AS Integer) AS FUNC(Integer) AS Integer
  LET inner AS FUNC(Integer) AS Integer = adder(k)
  RETURN LAMBDA(v AS Integer) -> inner(inner(v))
END FUNC

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET t AS FUNC(Integer) AS Integer = twice(k)
    n = n + t(0)
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "110\n",
    ),
    (
        "callback_and_byref_capture",
        "\
IMPORT io
IMPORT collections

FUNC applyAll(xs AS List OF Integer, f AS FUNC(Integer) AS Integer) AS List OF Integer
  RETURN collections::transform(xs, f)
END FUNC

FUNC main() AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  MUT n AS Integer = 0
  FOR k = 1 TO 10
    LET ys AS List OF Integer = applyAll(xs, LAMBDA(v AS Integer) -> v * k)
    MUT acc AS Integer = 0
    collections::forEach(ys, LAMBDA(v AS Integer) -> acc = acc + v)
    n = n + acc
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "330\n",
    ),
    (
        "nested_list_get_and_flatten",
        "\
IMPORT io
IMPORT collections

TYPE Holder
  f AS FUNC(Integer) AS Integer
  id AS Integer
END TYPE

FUNC sumAll(hs AS List OF Holder) AS Integer
  MUT t AS Integer = 0
  FOR EACH h IN hs
    LET g AS FUNC(Integer) AS Integer = h.f
    t = t + g(1)
  NEXT
  RETURN t
END FUNC

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 5
    LET hs AS List OF Holder = [Holder[LAMBDA(v AS Integer) -> v + k, 1], Holder[LAMBDA(v AS Integer) -> v * k, 2]]
    LET ch AS List OF List OF Holder = collections::chunks(hs, 1)
    LET second AS List OF Holder = collections::get(ch, 1)
    LET all AS List OF Holder = collections::flatten([hs, second])
    n = n + sumAll(second) + sumAll(all)
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "65\n",
    ),
    (
        "transform_owns_callback_results",
        "\
IMPORT io
IMPORT collections

TYPE Holder
  kids AS List OF Holder
  id AS Integer
END TYPE

FUNC mul(i AS Integer) AS FUNC(Integer) AS Integer
  RETURN LAMBDA(v AS Integer) -> v * i
END FUNC

FUNC main() AS Integer
  MUT n AS Integer = 0
  LET xs AS List OF Integer = [1, 2, 3]
  FOR k = 1 TO 5
    LET hs AS List OF Holder = [Holder[[], 1], Holder[[Holder[[], 7]], 2]]
    LET fresh AS List OF Holder = collections::transform(hs, LAMBDA(h AS Holder) -> Holder[[], h.id + k])
    LET same AS List OF Holder = collections::transform(hs, LAMBDA(h AS Holder) -> h)
    LET fs AS List OF FUNC(Integer) AS Integer = collections::transform(xs, LAMBDA(i AS Integer) -> mul(i + k))
    LET g AS Map OF Integer TO List OF Holder = collections::groupBy(hs, LAMBDA(h AS Holder) -> h.id MOD 2, LAMBDA(h AS Holder) -> Holder[[], h.id])
    FOR EACH f IN fs
      n = n + f(2)
    NEXT
    FOR EACH h IN fresh
      n = n + h.id
    NEXT
    FOR EACH h IN same
      n = n + h.id + len(h.kids)
    NEXT
    n = n + len(g)
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "225\n",
    ),
    (
        "transform_failing_callback_frees_partial_graphs",
        "\
IMPORT io
IMPORT collections

TYPE Holder
  kids AS List OF Holder
  f AS FUNC(Integer) AS Integer
  id AS Integer
END TYPE

FUNC mk(h AS Holder) AS Holder
  IF h.id = 3 THEN
    FAIL error(7, \"boom\")
  END IF
  RETURN Holder[[Holder[[], LAMBDA(v AS Integer) -> v, 0]], LAMBDA(v AS Integer) -> v + h.id, h.id]
END FUNC

FUNC once() AS Integer
  LET hs AS List OF Holder = [Holder[[], LAMBDA(v AS Integer) -> v, 1], Holder[[], LAMBDA(v AS Integer) -> v, 2], Holder[[], LAMBDA(v AS Integer) -> v, 3]]
  LET out AS List OF Holder = collections::transform(hs, mk) TRAP(e)
    RETURN -1
  END TRAP
  RETURN len(out)
END FUNC

FUNC main() AS Integer
  MUT n AS Integer = 0
  FOR k = 1 TO 5
    n = n + once()
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
",
        "-5\n",
    ),
];

/// Build with `--debug`, run, and return `(stdout, alloc_calls, free_calls,
/// live_bytes, double_free_skips)` summed over the arenas.
fn run(name: &str, source: &str) -> (String, u64, u64, u64, u64) {
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
        "{name}: build failed:\n{stdout}\n{}",
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
        sum(".double_free_skips "),
    )
}

#[test]
fn every_holder_of_a_capturing_closure_frees_it_exactly_once() {
    let mut failures = Vec::new();
    for (case, source, want) in CASES {
        let name = format!("b781_{case}");
        let (text, alloc, free, live, double_frees) = run(&name, source);
        if text != *want {
            failures.push(format!("{case}: printed {text:?}, want {want:?}"));
        } else if double_frees != 0 {
            failures.push(format!("{case}: {double_frees} double free(s)"));
        } else if alloc != free || live != 0 {
            failures.push(format!(
                "{case}: leaked {} block(s), {live} B live",
                alloc as i64 - free as i64
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "bug-781: a capturing closure must be freed by its one owner:\n  {}",
        failures.join("\n  ")
    );
}
