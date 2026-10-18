//! bug-913: templates overload by signature exactly like plain FUNCs. Two
//! template FUNCs of one name with different parameter shapes, or a template
//! beside a plain FUNC of that name, are distinct overloads; a call picks by its
//! argument types. A plain overload that matches exactly wins over a template
//! instantiation, and of two applicable templates the more specific one wins.
//! Nothing is silently dropped: each body runs for its own calls.

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
fn two_templates_of_one_name_each_run_their_own_body() {
    for (first, second) in [
        (
            "FUNC f OF T(x AS List OF T) AS String\n  RETURN \"list\"\nEND FUNC\n",
            "FUNC f OF T(x AS T) AS String\n  RETURN \"scalar\"\nEND FUNC\n",
        ),
        (
            "FUNC f OF T(x AS T) AS String\n  RETURN \"scalar\"\nEND FUNC\n",
            "FUNC f OF T(x AS List OF T) AS String\n  RETURN \"list\"\nEND FUNC\n",
        ),
    ] {
        let source = format!(
            "IMPORT io\n{first}{second}FUNC main AS Integer\n  io::print(f([1]))\n  io::print(f(5))\n  RETURN 0\nEND FUNC\n"
        );
        assert_eq!(run("tmpl_shapes", &source), "list\nscalar\n");
    }
}

#[test]
fn templates_overload_by_arity_at_the_same_type_arguments() {
    let source = "IMPORT io
FUNC f OF T(x AS T) AS String
  RETURN \"one\"
END FUNC
FUNC f OF T(x AS T, y AS T) AS String
  RETURN \"two\"
END FUNC
FUNC main AS Integer
  io::print(f(1) & f(1, 2) & f(\"a\", \"b\"))
  RETURN 0
END FUNC
";
    assert_eq!(run("tmpl_arity", source), "onetwotwo\n");
}

#[test]
fn a_plain_overload_beside_a_template_is_callable_and_wins_an_exact_match() {
    let source = "IMPORT io
FUNC f OF T(x AS T) AS String
  RETURN \"tmpl\"
END FUNC
FUNC f(x AS Integer, y AS Integer) AS String
  RETURN \"ov2\"
END FUNC
FUNC g OF T(x AS T) AS String
  RETURN \"tmpl\"
END FUNC
FUNC g(x AS Integer) AS String
  RETURN \"plain\"
END FUNC
FUNC main AS Integer
  io::print(f(1, 2) & \" \" & f(\"x\"))
  io::print(g(1) & \" \" & g(\"s\"))
  RETURN 0
END FUNC
";
    assert_eq!(run("tmpl_plain", source), "ov2 tmpl\nplain tmpl\n");
}

#[test]
fn an_identical_template_signature_is_a_duplicate() {
    let source = "FUNC f OF T(x AS T) AS String
  RETURN \"a\"
END FUNC
FUNC f OF T(x AS T) AS String
  RETURN \"b\"
END FUNC
FUNC main AS Integer
  RETURN 0
END FUNC
";
    let out = build_fails("tmpl_dup", source);
    assert!(out.contains("SYMBOL_DUPLICATE_TOP_LEVEL"), "{out}");
}

#[test]
fn template_overload_diagnostics_name_the_source_function() {
    // Neither template is more specific for `f(1, 2)`: an ambiguity, never a pick.
    let source = "FUNC f OF T(x AS T, y AS Integer) AS String
  RETURN \"a\"
END FUNC
FUNC f OF T(x AS Integer, y AS T) AS String
  RETURN \"b\"
END FUNC
FUNC main AS Integer
  LET s = f(1, 2)
  RETURN 0
END FUNC
";
    let out = build_fails("tmpl_ambiguous", source);
    assert!(
        out.contains("main.mfb:8 error[2-203-0101 TYPE_OVERLOAD_AMBIGUOUS]"),
        "{out}"
    );

    // Two templates equal up to the names of their type parameters are one
    // signature declared twice.
    let source = "FUNC f OF T(x AS T) AS String
  RETURN \"a\"
END FUNC
FUNC f OF U(y AS U) AS String
  RETURN \"b\"
END FUNC
FUNC main AS Integer
  RETURN 0
END FUNC
";
    let out = build_fails("tmpl_alpha_dup", source);
    assert!(
        out.contains("main.mfb:4 error[2-201-0010 SYMBOL_DUPLICATE_TOP_LEVEL]"),
        "{out}"
    );
    assert!(
        out.contains("Top-level symbol `f` was already declared"),
        "{out}"
    );

    // A lone template called with too few arguments names `f`, not `f$Integer`.
    let source = "FUNC f OF T(x AS T, y AS T) AS String
  RETURN \"two\"
END FUNC
FUNC main AS Integer
  LET s = f(1)
  RETURN 0
END FUNC
";
    let out = build_fails("tmpl_arity_leak", source);
    assert!(
        out.contains("Call to `f` has 1 argument(s), expected 2 to 2."),
        "{out}"
    );
}
