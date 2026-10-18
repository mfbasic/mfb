//! bug-833: a template with more than one type argument is written with a
//! parenthesized list everywhere (`TYPE Pairing OF (A, B)`, `Pair OF (Integer,
//! String)`), so a bare `Name OF T` has exactly one argument and a following `,`
//! belongs to the enclosing list. A user-template parameter can then sit anywhere
//! in a FUNC, LAMBDA or function-type parameter list, and a nested multi-argument
//! template is unambiguous. The old bare multi-argument spelling is a located parse
//! error that names the parenthesized form.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

fn run(name: &str, source: &str) -> String {
    let project = common::temp_project(name, source);
    let exe = common::build_project(&project);
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    assert!(out.status.success(), "program failed");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Build `source`, expecting failure; return the build's combined output.
fn build_fails(name: &str, source: &str) -> String {
    let project = common::temp_project(name, source);
    let out = Command::new(common::mfb_exe())
        .arg("build")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let _ = std::fs::remove_dir_all(&project);
    assert!(!out.status.success(), "build must fail");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn spec_push_example_with_template_parameter_first() {
    // `./mfb spec language templates` §3, verbatim.
    let source = "IMPORT io
IMPORT collections
TYPE Stack OF T
  items AS List OF T
END TYPE
FUNC push OF T(s AS Stack OF T, value AS T) AS Stack OF T
  RETURN WITH s { items := collections::append(s.items, value) }
END FUNC
FUNC main() AS Nothing
  MUT s AS Stack OF Integer
  s = push(s, 1)
  s = push(s, 2)
  io::print(toString(len(s.items)))
END FUNC
";
    assert_eq!(run("push_first", source), "2\n");
}

#[test]
fn multi_argument_templates_are_parenthesized_in_every_position() {
    let source = "IMPORT io
IMPORT collections
TYPE Pairing OF (A, B)
  left AS A
  right AS B
END TYPE
TYPE Box OF T
  value AS T
END TYPE
TYPE Holder OF T
  held AS T
END TYPE
FUNC flip OF (A, B)(p AS Pairing OF (A, B), tag AS String) AS Pairing OF (B, A)
  io::print(tag)
  RETURN Pairing[p.right, p.left]
END FUNC
FUNC describe(p AS Pair OF (Integer, String), b AS Integer) AS String
  RETURN toString(p.first + b) & p.second
END FUNC
FUNC apply(f AS FUNC(Box OF Integer, Integer) AS Integer, b AS Box OF Integer) AS Integer
  RETURN f(b, 10)
END FUNC
FUNC main() AS Nothing
  LET p AS Pairing OF (Integer, String) = Pairing[7, \"seven\"]
  LET q AS Pairing OF (String, Integer) = flip(p, \"flipped\")
  io::print(q.left & \" \" & toString(q.right))
  io::print(describe(Pair[1, \"x\"], 2))
  LET add = LAMBDA(b AS Box OF Integer, x AS Integer) -> b.value + x
  io::print(toString(apply(add, Box[5])))
  LET h AS Holder OF Pairing OF (Integer, String) = Holder[Pairing[3, \"three\"]]
  io::print(toString(h.held.left) & h.held.right)
  MUT pairs AS List OF Pair OF (Integer, String) = [Pair[4, \"four\"]]
  io::print(toString(len(pairs)) & collections::get(pairs, 0).second)
END FUNC
";
    assert_eq!(
        run("multi_paren", source),
        "flipped\nseven 7\n3x\n15\n3three\n1four\n"
    );
}

#[test]
fn bare_multi_argument_template_is_a_located_parse_error() {
    let source = "FUNC main() AS Nothing
  LET p AS Pair OF Integer, String = Pair[1, \"x\"]
END FUNC
";
    let out = build_fails("bare_use", source);
    assert!(
        out.contains("main.mfb:2 error[1-102-0005 MFB_PARSE_UNEXPECTED_TOKEN]"),
        "{out}"
    );
    assert!(out.contains("`Pair OF (Integer, String)`"), "{out}");

    let source = "TYPE Pairing OF A, B
  left AS A
  right AS B
END TYPE
FUNC main() AS Nothing
END FUNC
";
    let out = build_fails("bare_decl", source);
    assert!(
        out.contains("main.mfb:1 error[1-102-0005 MFB_PARSE_UNEXPECTED_TOKEN]"),
        "{out}"
    );
    assert!(out.contains("`OF (A, B)`"), "{out}");
}
