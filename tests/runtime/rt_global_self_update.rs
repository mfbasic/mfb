//! plan-142-H: value semantics of a self-update `g = op(g, …)` on a module-level
//! global.
//!
//! After plan-142-H such a statement mutates the global's own block in place
//! instead of building a new one (`StoreGlobal`). That is only sound while
//! nothing else still reads the old block, so each program here pins one reader
//! the in-place write must not disturb — the borrowers plan-142-H §2 lists:
//!
//! * `LET y = g` — a bind of a global is a copy, so `y` keeps the old value;
//! * `f(g)` where `f` self-updates `g` — bug-665: the parameter keeps the
//!   entry-time value;
//! * `FOR EACH v IN g` whose body self-updates `g` — bug-666: the loop visits the
//!   entry-time elements;
//! * a later operand that itself writes `g` (`g = append(g, addHundred())`) — bug-496:
//!   operand 0 is `g` as it was before `addHundred()` ran, and the statement declines
//!   the in-place form (gate G-global-operand);
//! * a self-update that fails inside a `TRAP` — `g` is unchanged;
//! * a global `String` grown in place and then reassigned — the reassignment
//!   resets the capacity the in-place appends kept, so later appends never write
//!   past the new, tight buffer (the stale-shadow overflow of plan-142-G
//!   Correction G1, for a global).
//!
//! The programs held on the copying path before plan-142-H and must hold after.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const SHOW: &str = "\
FUNC showInts(xs AS List OF Integer) AS String
  MUT s AS String = \"\"
  FOR EACH n IN xs
    s = s & toString(n) & \" \"
  NEXT
  RETURN s
END FUNC
";

/// `(name, globals and functions, main body, expected output)`.
const CASES: &[(&str, &str, &str, &str)] = &[
    (
        "let_copy",
        "MUT g AS List OF Integer = [1, 2, 3]

SUB grow()
  g = collections::append(g, 4)
END SUB",
        "LET y AS List OF Integer = g
  grow()
  io::print(showInts(y))
  io::print(showInts(g))",
        "1 2 3 \n1 2 3 4 ",
    ),
    (
        "parameter",
        "MUT g AS List OF Integer = [1, 2, 3]

FUNC grown(xs AS List OF Integer) AS String
  g = collections::append(g, 9)
  g = collections::removeAt(g, 0)
  RETURN showInts(xs)
END FUNC",
        "io::print(grown(g))
  io::print(showInts(g))",
        "1 2 3 \n2 3 9 ",
    ),
    (
        "for_each",
        "MUT g AS List OF Integer = [1, 2, 3]

SUB walk()
  MUT seen AS String = \"\"
  FOR EACH v IN g
    seen = seen & toString(v) & \" \"
    g = collections::append(g, v * 10)
  NEXT
  io::print(seen)
END SUB",
        "walk()
  io::print(showInts(g))",
        "1 2 3 \n1 2 3 10 20 30 ",
    ),
    (
        "later_operand_writes",
        "MUT g AS List OF Integer = [1, 2, 3]

FUNC addHundred() AS Integer
  g = collections::append(g, 100)
  RETURN 5
END FUNC

SUB advance()
  g = collections::append(g, addHundred())
END SUB",
        "advance()
  io::print(showInts(g))",
        "1 2 3 5 ",
    ),
    (
        "trap",
        "MUT g AS List OF Integer = [1, 2, 3]

FUNC failing() AS Integer
  g = collections::removeAt(g, 99)
  RETURN 0
  TRAP(err)
    RETURN -1
  END TRAP
END FUNC",
        "io::print(toString(failing()))
  io::print(showInts(g))",
        "-1\n1 2 3 ",
    ),
    (
        "map_and_set",
        "MUT m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }
MUT gset AS Set OF Integer = Set OF Integer { 1 }

SUB fill()
  FOR i = 1 TO 50
    m = collections::set(m, \"k\" & toString(i), i)
    gset = collections::add(gset, i)
  NEXT
  m = collections::removeKey(m, \"a\")
  gset = collections::remove(gset, 1)
END SUB",
        "LET m0 AS Map OF String TO Integer = m
  LET s0 AS Set OF Integer = gset
  fill()
  io::print(toString(len(m0)) & \" \" & toString(len(s0)))
  io::print(toString(len(m)) & \" \" & toString(len(gset)) & \" \" & toString(collections::get(m, \"k50\")))",
        "1 1\n50 49 50",
    ),
    (
        "string_concat",
        "MUT gs AS String = \"a\"

SUB grow()
  FOR i = 1 TO 100
    gs = gs & \"b\"
  NEXT
END SUB",
        "LET before AS String = gs
  grow()
  LET t AS String = \"T\" & toString(len(gs))
  grow()
  io::print(before & \" \" & t & \" \" & toString(len(gs)))",
        "a T101 201",
    ),
    (
        "string_reassigned",
        "MUT gs AS String = \"a\"

SUB grow(n AS Integer)
  FOR i = 1 TO n
    gs = gs & \"b\"
  NEXT
END SUB",
        "grow(100)
  gs = \"x\" & toString(1)
  LET t1 AS String = \"T\" & toString(len(gs))
  LET t2 AS String = \"U\" & toString(len(gs))
  LET t3 AS String = \"V\" & toString(len(gs))
  grow(60)
  io::print(toString(len(gs)) & \" \" & t1 & t2 & t3)",
        "62 T2U2V2",
    ),
];

#[test]
fn a_global_self_update_keeps_every_reader_s_value() {
    let mut failures = Vec::new();
    for (name, module, body, want) in CASES {
        let source = format!(
            "IMPORT collections\nIMPORT io\n\n{module}\n\n{SHOW}\nFUNC main() AS Integer\n  \
             {body}\n  RETURN 0\nEND FUNC\n"
        );
        let project = common::temp_project(&format!("global_self_update_{name}"), &source);
        let output = Command::new(common::mfb_exe())
            .arg("build")
            .arg(&project)
            .output()
            .expect("run mfb build");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() {
            failures.push(format!(
                "{name}: build failed:\n{stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            ));
            continue;
        }
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
        let printed = String::from_utf8_lossy(&run.stdout);
        let printed = printed.trim_end_matches('\n');
        if !run.status.success() || printed != *want {
            failures.push(format!(
                "{name}: printed {printed:?} (want {want:?}), exit {:?}, stderr {}",
                run.status.code(),
                String::from_utf8_lossy(&run.stderr).trim()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
