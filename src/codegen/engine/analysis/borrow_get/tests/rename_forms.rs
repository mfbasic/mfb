//! bug-784: the per-binding rename of a re-bound `get` name must walk EVERY statement
//! and value kind a function can contain, or a read behind one it skipped resolves to
//! no binding and the binding it belongs to is classified as if the read were absent.
//!
//! Each function below binds `h` twice (so the renamer runs). The first binding is
//! only read through fields — a plain borrow. The second is handed whole to `f(h)`
//! inside ONE statement or value kind, which makes it a copy (a callee could keep
//! the alias); if the renamer skipped that kind, the read would resolve to nothing
//! and the second binding would wrongly stay a borrow.

use super::*;
use crate::target::shared::nir::{NirFunction, NirModule};
use crate::testutil::{nir_for_src, CodeTarget};

const HEADER: &str = "\
IMPORT collections
IMPORT io
IMPORT fs
IMPORT os

TYPE P
  age AS Integer
  trail AS List OF Float
END TYPE

TYPE Dot
  x AS Integer
END TYPE

TYPE Counter
  n AS Integer
END TYPE

UNION Shape
  Dot
  P
END UNION

FUNC mk() AS List OF P
  RETURN [P[age := 1, trail := [0.0]], P[age := 2, trail := [0.0]]]
END FUNC

FUNC f(p AS P) AS Integer
  RETURN p.age
END FUNC
";

/// `(function name, statements placed after the second `LET h`)`; `f(h)` passes the
/// binding whole.
const CONSTRUCTS: &[(&str, &str)] = &[
    (
        "while_condition",
        "MUT k AS Integer = 0\n    WHILE k < f(h)\n      k = k + 1\n    END WHILE",
    ),
    (
        "do_until_condition",
        "MUT k AS Integer = 0\n    DO\n      k = k + 1\n    LOOP UNTIL k >= f(h)",
    ),
    (
        "for_each_iterable",
        "MUT k AS Integer = 0\n    FOR EACH v IN [f(h), 2]\n      k = k + v\n    NEXT",
    ),
    (
        "for_bounds",
        "MUT k AS Integer = 0\n    FOR i = 0 TO f(h)\n      k = k + i\n    NEXT",
    ),
    (
        "match_guard",
        "MUT k AS Integer = 0\n    MATCH k\n      CASE 1, 2 WHEN k > f(h)\n        k = 0\n      CASE ELSE\n        k = 1\n    END MATCH",
    ),
    (
        "match_scrutinee",
        "MUT k AS Integer = 0\n    MATCH f(h)\n      CASE 1\n        k = 1\n      CASE ELSE\n        k = 2\n    END MATCH",
    ),
    (
        "union_wrap",
        "LET s AS Shape = Dot[f(h)]\n    MUT k AS Integer = 0\n    MATCH s\n      CASE Dot(d)\n        k = d.x\n      CASE P(p)\n        k = p.age\n    END MATCH",
    ),
    (
        "inline_trap",
        "LET parsed AS Integer = toInt(toString(f(h))) TRAP(e)\n      RECOVER 0\n    END TRAP",
    ),
    (
        "closure_capture",
        "LET c AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + f(h)\n    LET r AS Integer = c(1)",
    ),
    (
        "set_literal",
        "LET members AS Set OF Integer = Set OF Integer { f(h), 2 }",
    ),
    (
        "map_literal",
        "LET table AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := f(h) }",
    ),
    (
        "record_constructor",
        "LET q AS P = P[age := f(h), trail := []]",
    ),
    (
        "with_update",
        "LET q AS P = P[age := 1, trail := []]\n    LET q2 AS P = WITH q { age := f(h) }",
    ),
    (
        "fail",
        "IF n > 100 THEN\n      FAIL error(7, toString(f(h)))\n    END IF",
    ),
    (
        "exit_program",
        "IF n > 100 THEN\n      os::exit(f(h))\n    END IF",
    ),
    (
        "operators",
        "LET flag AS Boolean = NOT (f(h) > 1) AND -f(h) < 0",
    ),
];

fn source() -> String {
    let mut text = HEADER.to_string();
    for (name, body) in CONSTRUCTS {
        text.push_str(&format!(
            "\nFUNC c_{name}(n AS Integer) AS Integer\n  MUT xs AS List OF P = mk()\n  MUT total AS Integer = 0\n  \
             FOR j = 0 TO 1\n    LET h AS P = collections::get(xs, j)\n    total = total + h.age\n  NEXT\n  \
             FOR j = 0 TO 1\n    LET h AS P = collections::get(xs, j)\n    {body}\n  NEXT\n  RETURN total\nEND FUNC\n"
        ));
    }
    text.push_str(
        "\nFUNC c_state_assign(c AS fs::File STATE Counter) AS Integer\n  MUT xs AS List OF P = mk()\n  \
         FOR j = 0 TO 1\n    LET h AS P = collections::get(xs, j)\n    c.state.n = c.state.n + h.age\n  NEXT\n  \
         FOR j = 0 TO 1\n    LET h AS P = collections::get(xs, j)\n    c.state.n = f(h)\n  NEXT\n  RETURN 0\nEND FUNC\n\n\
         FUNC main() AS Integer\n  io::print(toString(len(mk())))\n",
    );
    // Each probe is held as a function value so call-graph DCE (Level 1) keeps it:
    // nothing calls them, and a function no root reaches is not emitted.
    for name in CONSTRUCTS
        .iter()
        .map(|(name, _)| *name)
        .chain(["state_assign"])
    {
        text.push_str(&format!("  LET keep_{name} = c_{name}\n"));
    }
    text.push_str("  RETURN 0\nEND FUNC\n");
    text
}

fn module() -> NirModule {
    nir_for_src(
        &source(),
        CodeTarget::MacosAarch64,
        crate::target::NativeBuildMode::Console,
    )
    .unwrap_or_else(|error| panic!("the probe lowers to NIR: {error}"))
}

fn named<'m>(module: &'m NirModule, name: &str) -> &'m NirFunction {
    module
        .functions
        .iter()
        .find(|f| f.name == name || f.name.rsplit(['.', ':']).next() == Some(name))
        .unwrap_or_else(|| panic!("no function `{name}`"))
}

/// The role of each binding of `h` in `function`, in source order.
fn roles_of_h(function: &NirFunction) -> Vec<&'static str> {
    let mut address_taken = HashSet::new();
    crate::codegen::engine::function::function_lowering::collect_address_taken_locals(
        &function.body,
        &mut address_taken,
    );
    let got = collect_borrow_gets(&function.body, &address_taken, &|_, _| false);
    struct Binds<'a> {
        got: &'a BorrowGets,
        out: Vec<&'static str>,
    }
    impl NirVisitor for Binds<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::Bind { name, .. } = op {
                if name == "h" {
                    self.out
                        .push(match self.got.rebound.get(&op_key(op)).map(|r| &r.role) {
                            Some(ReboundRole::Element(_)) => "element",
                            Some(ReboundRole::Borrow) => "borrow",
                            Some(ReboundRole::Copy) => "copy",
                            None => "unswitched",
                        });
                }
            }
            walk_op(self, op);
        }
    }
    let mut binds = Binds {
        got: &got,
        out: Vec::new(),
    };
    binds.visit_ops(&function.body);
    binds.out
}

#[test]
fn a_whole_read_of_a_rebound_name_is_seen_inside_every_construct() {
    let module = module();
    let mut wrong = Vec::new();
    for (name, _) in CONSTRUCTS.iter().chain(&[("state_assign", "")]) {
        let roles = roles_of_h(named(&module, &format!("c_{name}")));
        if roles != ["borrow", "copy"] {
            wrong.push(format!("{name}: {roles:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "the second `h` is passed whole, so it must be a copy beside the first's borrow:\n  {}",
        wrong.join("\n  ")
    );
}
