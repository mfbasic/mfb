//! bug-1019: a value passed through a function that returns its own parameter
//! (`idOf(squares(20.0))`, `FUNC idOf OF T(x AS T) AS T / RETURN x`) is freed.
//!
//! Such a function returns a BORROW of its argument (plan-86 K1,
//! `function_returns_param_borrow`): the callee returns the argument's pointer
//! uncopied, the caller copies the result at an owning store and leaves the
//! argument's block to the argument's owner. plan-147's hand-over broke the
//! second half. A fresh temporary, or a local at its last use, was GIVEN to the
//! callee's owned variant (`idOf$own1`), which moved the block straight back
//! out as its result — so the block had no owner left: the caller no longer
//! owned the argument, and it still read the result as a borrow of it. Every
//! such call leaked the argument's block (144 B for the two-element list here,
//! 3200 B over the 50-pass loop).
//!
//! Each program must end with every block it allocated freed exactly once
//! (`alloc_calls == free_calls`, `live_bytes 0`, `double_free_skips 0`) and
//! print the right value. The cases that were already balanced are here as
//! the guard that the fix moved no free it should not have: a parameter the
//! callee does not return is still handed over, and a function that is also
//! used as a callback still returns a fresh block.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const HELPERS: &str = "\
IMPORT io
IMPORT collections

TYPE Square
  side AS Float
END TYPE

TYPE Bag
  items AS List OF Integer
  name AS String
END TYPE

FUNC squares(first AS Float) AS List OF Square
  RETURN [Square[first], Square[first + 1.0]]
END FUNC

FUNC ints(n AS Integer) AS List OF Integer
  RETURN [n, n + 1, n + 2]
END FUNC

FUNC maybe(n AS Integer) AS List OF Integer
  IF n < 0 THEN
    FAIL error(1, \"neg\")
  END IF
  RETURN ints(n)
END FUNC

FUNC idOf OF T(x AS T) AS T
  RETURN x
END FUNC

FUNC idL(x AS List OF Square) AS List OF Square
  RETURN x
END FUNC

FUNC idInts(x AS List OF Integer) AS List OF Integer
  RETURN x
END FUNC

FUNC lenOf OF T(x AS List OF T) AS Integer
  RETURN len(x)
END FUNC

FUNC pick(a AS List OF Integer, b AS List OF Integer, first AS Boolean) AS List OF Integer
  IF first THEN
    RETURN a
  END IF
  RETURN b
END FUNC

FUNC keepFirst(a AS List OF Integer, spare AS List OF Integer) AS List OF Integer
  MUT mine AS List OF Integer = spare
  mine = collections::append(mine, 1)
  io::print(toString(len(mine)))
  RETURN a
END FUNC

FUNC viaCall(n AS Float) AS List OF Square
  RETURN idOf(squares(n))
END FUNC

FUNC side(s AS Square) AS Float
  RETURN s.side
END FUNC

FUNC idCb(s AS Square) AS Square
  RETURN s
END FUNC

FUNC grow(acc AS List OF Integer, n AS Integer) AS List OF Integer
  RETURN collections::append(acc, n)
END FUNC
";

/// `(name, body of main, expected stdout lines joined by '|')`.
const CASES: &[(&str, &str, &str)] = &[
    // The bug document's program: a generic identity over a collection temporary.
    (
        "generic_temp",
        "LET d AS List OF Square = idOf(squares(20.0))\n  io::print(toString(len(d)))",
        "2",
    ),
    // Not a generics bug: the same function written out.
    (
        "nongeneric_temp",
        "LET d AS List OF Square = idL(squares(20.0))\n  io::print(toString(len(d)))",
        "2",
    ),
    // A local at its last use was handed over the same way.
    (
        "local_last_use",
        "LET s AS List OF Square = squares(20.0)\n  LET d AS List OF Square = idOf(s)\n  io::print(toString(len(d)))",
        "2",
    ),
    // A local that is read again is lent, and always was.
    (
        "local_still_live",
        "LET s AS List OF Square = squares(20.0)\n  LET d AS List OF Square = idOf(s)\n  io::print(toString(len(d) + len(s)))",
        "4",
    ),
    // The shape bs18-agent-j reported: the borrow passed on to a builtin.
    (
        "transform",
        "LET d AS List OF Float = collections::transform(idOf(squares(20.0)), side)\n  io::print(toString(len(d)))",
        "2",
    ),
    ("record_temp", "LET q AS Square = idOf(Square[3.0])\n  io::print(toString(q.side))", "3.00"),
    ("string_temp", "LET t AS String = idOf(\"a\" & toString(7))\n  io::print(t)", "a7"),
    (
        "map_temp",
        "LET m AS Map OF String TO Integer = idOf(Map OF String TO Integer { \"a\" := 1 })\n  io::print(toString(len(m)))",
        "1",
    ),
    (
        "set_temp",
        "LET s AS Set OF Integer = idOf(Set OF Integer { 1, 2 })\n  io::print(toString(len(s)))",
        "2",
    ),
    (
        "literal_arg",
        "LET d AS List OF Integer = idOf([1, 2, 3])\n  io::print(toString(len(d)))",
        "3",
    ),
    (
        "record_of_collection",
        "LET b AS Bag = idOf(Bag[ints(1), \"bag\"])\n  io::print(b.name & toString(len(b.items)))",
        "bag3",
    ),
    // A generic that does NOT return its argument only reads it.
    ("generic_no_return", "io::print(toString(lenOf(squares(20.0))))", "2"),
    (
        "nested",
        "LET d AS List OF Square = idOf(idOf(squares(20.0)))\n  io::print(toString(len(d)))",
        "2",
    ),
    ("passed_on", "io::print(toString(len(idInts(idInts(ints(3))))))", "3"),
    // Two returned parameters: whichever comes back, both temporaries are freed.
    (
        "pick_first",
        "LET d AS List OF Integer = pick(ints(1), ints(5), TRUE)\n  io::print(toString(collections::get(d, 0)))",
        "1",
    ),
    (
        "pick_second",
        "LET d AS List OF Integer = pick(ints(1), ints(5), FALSE)\n  io::print(toString(collections::get(d, 0)))",
        "5",
    ),
    // One parameter returned (lent), the other consumed by the callee (still
    // handed over): the two contracts side by side in one call.
    (
        "one_returned_one_consumed",
        "LET d AS List OF Integer = keepFirst(ints(1), ints(5))\n  io::print(toString(len(d)))",
        "4|3",
    ),
    // The borrow returned straight out of another function.
    (
        "return_of_call",
        "LET d AS List OF Square = viaCall(4.0)\n  io::print(toString(len(d)))",
        "2",
    ),
    (
        "self_assign",
        "MUT xs AS List OF Integer = ints(1)\n  xs = idOf(xs)\n  io::print(toString(len(xs)))",
        "3",
    ),
    (
        "loop",
        "MUT total AS Integer = 0\n  FOR i = 1 TO 50\n    LET d AS List OF Integer = idOf(ints(i))\n    total = total + len(d)\n  NEXT\n  io::print(toString(total))",
        "150",
    ),
    ("discarded", "idOf(squares(20.0))\n  io::print(\"x\")", "x"),
    // The argument built by updating a local in place, then passed through.
    (
        "arg_update",
        "LET xs AS List OF Integer = ints(1)\n  LET d AS List OF Integer = idOf(collections::append(xs, 9))\n  io::print(toString(len(d)))",
        "4",
    ),
    // Under an inline TRAP, on the success path and on the failing one.
    (
        "trap_ok",
        "LET d AS List OF Integer = idOf(maybe(3)) TRAP(e)\n    RECOVER [0]\n  END TRAP\n  io::print(toString(len(d)))",
        "3",
    ),
    (
        "trap_fail",
        "LET d AS List OF Integer = idOf(maybe(-1)) TRAP(e)\n    RECOVER [0]\n  END TRAP\n  io::print(toString(len(d)))",
        "1",
    ),
    // A function also used as a callback returns a fresh block, not a borrow.
    (
        "callback_identity",
        "LET q AS List OF Square = collections::transform(squares(1.0), idCb)\n  LET r AS Square = idCb(Square[9.0])\n  io::print(toString(len(q)) & toString(r.side))",
        "29.00",
    ),
    // plan-147's own shape is untouched: a consumed accumulator is handed over.
    (
        "consumed_accumulator",
        "MUT acc AS List OF Integer = []\n  FOR i = 1 TO 40\n    acc = grow(acc, i)\n  NEXT\n  io::print(toString(len(acc)))",
        "40",
    ),
];

#[test]
fn a_value_passed_through_a_parameter_returning_function_is_freed_once() {
    let mut failures = Vec::new();
    for (name, body, want) in CASES {
        let source = format!("{HELPERS}\nFUNC main() AS Integer\n  {body}\n  RETURN 0\nEND FUNC\n");
        let project = common::temp_project(&format!("param_borrow_handover_{name}"), &source);
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
            .find(|path| path.ends_with(common::host_executable_suffix()))
            .or_else(|| {
                stdout
                    .lines()
                    .find_map(|line| line.strip_prefix("Wrote executable to "))
            })
            .expect("an executable");
        let run = Command::new(exe).output().expect("run the program");
        let out = String::from_utf8_lossy(&run.stdout);
        let err = String::from_utf8_lossy(&run.stderr);
        let counter = |key: &str| -> u64 {
            err.lines()
                .find_map(|l| l.strip_prefix(&format!("arena.0.{key} ")))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(u64::MAX)
        };
        let (alloc, free, live, double) = (
            counter("alloc_calls"),
            counter("free_calls"),
            counter("live_bytes"),
            counter("double_free_skips"),
        );
        let printed = out.lines().collect::<Vec<_>>().join("|");
        if !run.status.success() || printed != *want || alloc != free || live != 0 || double != 0 {
            failures.push(format!(
                "{name}: {} printed {printed:?} (want {want:?}), {alloc} allocated / {free} freed / \
                 {live} B live / {double} double-free skips",
                common::exit_description(&run.status),
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
