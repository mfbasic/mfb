//! bug-945: `Result` and `Ok` mean nothing to a user. The runtime's fallible-call
//! representation is internal (`#Result` / `#Ok`, names no identifier can spell),
//! so a user `TYPE Result` / `TYPE Ok` is an ordinary type: it declares, constructs,
//! reads fields, joins a union, is matched by `CASE`, instantiates as a template,
//! and `typeName` reports the user's name. A fallible call in the same program
//! still traps and unwraps through the internal representation.

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

#[test]
fn user_record_named_result_is_an_ordinary_type() {
    let source = "IMPORT io
TYPE Result
  code AS Integer
  note AS String
END TYPE
FUNC half(n AS Integer) AS Integer
  IF n < 0 THEN FAIL error(901, \"negative\")
  RETURN n / 2
END FUNC
FUNC wrap(n AS Integer) AS Result
  LET h = half(n) TRAP(e)
    RETURN Result[e.code, \"trapped\"]
  END TRAP
  RETURN Result[h, \"ok\"]
END FUNC
FUNC main() AS Nothing
  LET r = Result[1, \"hi\"]
  io::print(r.note)
  io::print(typeName(r))
  LET a = wrap(8)
  io::print(toString(a.code) & \" \" & a.note)
  LET b = wrap(-1)
  io::print(toString(b.code) & \" \" & b.note)
END FUNC
";
    assert_eq!(
        run("user_type_result", source),
        "hi\nResult\n4 ok\n901 trapped\n"
    );
}

#[test]
fn user_record_named_ok_is_an_ordinary_type() {
    let source = "IMPORT io
TYPE Ok
  v AS Integer
END TYPE
FUNC bump(o AS Ok) AS Ok
  RETURN Ok[o.v + 1]
END FUNC
FUNC main() AS Nothing
  LET r = Ok[1]
  io::print(toString(r.v))
  io::print(toString(bump(r).v))
  io::print(typeName(r))
END FUNC
";
    assert_eq!(run("user_type_ok", source), "1\n2\nOk\n");
}

#[test]
fn user_union_member_named_ok_is_matched() {
    let source = "IMPORT io
TYPE Ok
  v AS Integer
END TYPE
TYPE Bad
  why AS String
END TYPE
UNION Outcome
  Ok
  Bad
END UNION
FUNC show(o AS Outcome) AS String
  MATCH o
    CASE Ok(x)
      RETURN \"ok \" & toString(x.v)
    CASE Bad(b)
      RETURN \"bad \" & b.why
  END MATCH
END FUNC
FUNC main() AS Nothing
  io::print(show(Ok[3]))
  io::print(show(Bad[\"no\"]))
END FUNC
";
    assert_eq!(run("user_union_ok", source), "ok 3\nbad no\n");
}

#[test]
fn user_template_named_result_instantiates() {
    let source = "IMPORT io
TYPE Result OF T
  value AS T
  fine AS Boolean
END TYPE
FUNC unwrapOr OF T(fallback AS T, r AS Result OF T) AS T
  IF r.fine THEN RETURN r.value
  RETURN fallback
END FUNC
FUNC main() AS Nothing
  LET a AS Result OF Integer = Result[7, TRUE]
  LET b AS Result OF String = Result[\"x\", FALSE]
  io::print(toString(unwrapOr(0, a)))
  io::print(unwrapOr(\"fallback\", b))
END FUNC
";
    assert_eq!(run("user_template_result", source), "7\nfallback\n");
}
