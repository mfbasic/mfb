//! `ir::lower::concat_fold`: a `collections::reduce` whose reducer is
//! `RETURN acc & <rhs>` is rewritten into a loop that appends to one String,
//! wherever in a statement the fold is the first effectful node — and declined
//! (left as the call, which is slow and known good) whenever any condition is
//! not exactly right.
//!
//! Each case lowers a program and asserts which of the two it got: a rewritten
//! fold leaves no `collections.reduce` call behind and binds a `$g_foldN` temp; a
//! declined one keeps the call.

use super::*;

const REDUCERS: &str = "\
IMPORT collections
IMPORT io
TYPE Named
  name AS String
END TYPE
FUNC withNeg(acc AS String, x AS Integer) AS String
  RETURN acc & toString(-x)
END FUNC
FUNC withNamed(acc AS String, x AS Integer) AS String
  RETURN acc & toString(value := x)
END FUNC
FUNC withLiterals(acc AS String, x AS Integer) AS String
  RETURN acc & (\",\" & toString(x + 1) & toString(TRUE))
END FUNC
FUNC withMember(acc AS String, x AS Named) AS String
  RETURN acc & x.name
END FUNC
FUNC make() AS List OF Integer
  RETURN [1, 2]
END FUNC
";

/// `(collections.reduce calls left, `$g_foldN` temps bound)` in `function`.
fn fold_shape(ir: &IrProject, function_name: &str) -> (usize, usize) {
    let mut calls = 0;
    let mut temps = 0;
    walk_ops(&function(ir, function_name).body, &mut |op| {
        if let IrOp::Bind { name, .. } = op {
            let fold = name.strip_prefix("$g_fold").unwrap_or("");
            if fold.starts_with(|c: char| c.is_ascii_digit()) {
                temps += 1;
            }
        }
        let values: Vec<&IrValue> = match op {
            IrOp::Bind {
                value: Some(value), ..
            }
            | IrOp::Return {
                value: Some(value), ..
            } => vec![value],
            IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::Eval { value, .. } => vec![value],
            _ => vec![],
        };
        for value in values {
            crate::ir::value::visit_value(value, &mut |node| {
                if matches!(node, IrValue::Call { target, .. } if target == "collections.reduce") {
                    calls += 1;
                }
            });
        }
    });
    (calls, temps)
}

fn lowered(body: &str) -> IrProject {
    let src = format!("{REDUCERS}{body}");
    lower_src_concrete(&src, None)
}

/// The fold is rewritten in every statement position, and through the
/// operands evaluated before it.
#[test]
fn a_concat_fold_is_rewritten_in_every_statement_position() {
    let ir = lowered(
        "\
MUT total AS String = \"\"
LET shared AS List OF Integer = [1, 2]
FUNC asReturn(xs AS List OF Integer) AS String
  RETURN collections::reduce(xs, \"\", withNeg)
END FUNC
FUNC main() AS Integer
  LET xs = [1, 2, 3]
  LET bound = collections::reduce(xs, \"\", withNamed)
  MUT assigned AS String = \"\"
  assigned = collections::reduce(xs, \"\", withLiterals)
  total = collections::reduce(xs, \"\", withNeg)
  io::print(collections::reduce(xs, \"\", withNeg))
  LET prefixed = \"x\" & collections::reduce(xs, \"\", withNeg)
  LET negated = NOT (collections::reduce(xs, \"\", withNeg) = \"\")
  LET fromGlobal = collections::reduce(shared, \"\", withNeg)
  RETURN len(bound & assigned & prefixed & fromGlobal & asReturn(xs))
END FUNC
",
    );
    let mut binds = Vec::new();
    walk_ops(&function(&ir, "main").body, &mut |op| {
        if let IrOp::Bind { name, value, .. } = op {
            let kind = match value {
                Some(IrValue::Call { target, .. }) => format!("call {target}"),
                Some(IrValue::Local(source)) => format!("local {source}"),
                Some(_) => "other".to_string(),
                None => "none".to_string(),
            };
            binds.push(format!("{name}={kind}"));
        }
    });
    assert_eq!(
        fold_shape(&ir, "main"),
        (0, 7),
        "all seven folds in main, the global list's included: {binds:?}"
    );
    assert_eq!(fold_shape(&ir, "asReturn"), (0, 1), "the RETURN fold");
}

/// Every decline keeps the call.
#[test]
fn a_fold_that_is_not_exactly_right_keeps_the_call() {
    let ir = lowered(
        "\
FUNC main() AS Integer
  LET xs = [1, 2, 3]
  LET seed = \"\"
  LET fromLocalSeed = collections::reduce(xs, seed, withNeg)
  LET fromLambda = collections::reduce(xs, \"\", LAMBDA(acc AS String, x AS Integer) -> acc & toString(x))
  LET fromCall = collections::reduce(make(), \"\", withNeg)
  LET fromEmpty AS String = collections::reduce([], \"\", withNeg)
  LET items = [Named[\"a\"]]
  LET fromMember = collections::reduce(items, \"\", withMember)
  LET fromLiteral = collections::reduce([4, 5], \"\", withNeg)
  RETURN 0
END FUNC
",
    );
    assert_eq!(
        fold_shape(&ir, "main"),
        (6, 0),
        "an unannotated seed, a closure reducer, an effectful list, an untyped \
         list, a reducer whose right side is not enumerated and a list literal \
         (not effect-free) all decline"
    );
}
