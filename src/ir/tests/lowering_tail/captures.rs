//! `ir::lower::captures`: which outer locals a lambda's environment carries.
//!
//! A lambda of the assignment form (`LAMBDA(x) -> total = x`) writes `total`
//! even when it never reads it, and a nested lambda's free variables — its
//! assignment target included — are free in the lambda around it, whose
//! environment the nested one is built from. A field read (`p.count`) and a
//! record update (`WITH p { … }`) read `p`. Each must be captured, or the lifted
//! function names a local that does not resolve.

use super::*;

/// The names every closure built in `function` captures, in order.
fn captured(ir: &IrProject, function_name: &str) -> Vec<Vec<String>> {
    let mut closures = Vec::new();
    walk_ops(&function(ir, function_name).body, &mut |op| {
        let values: Vec<&IrValue> = match op {
            IrOp::Bind {
                value: Some(value), ..
            } => vec![value],
            IrOp::Eval { value, .. } | IrOp::Assign { value, .. } => vec![value],
            _ => vec![],
        };
        for value in values {
            crate::ir::value::visit_value(value, &mut |node| {
                if let IrValue::Closure { captures, .. } = node {
                    closures.push(
                        captures
                            .iter()
                            .map(|capture| match capture {
                                IrValue::Local(name) | IrValue::LocalRef { name, .. } => {
                                    name.clone()
                                }
                                _ => "?".to_string(),
                            })
                            .collect(),
                    );
                }
            });
        }
    });
    closures
}

#[test]
fn an_assignment_target_is_captured_even_when_it_is_not_read() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  MUT total AS Integer = 0
  LET xs = [1, 2, 3]
  collections::forEach(xs, LAMBDA(x AS Integer) -> total = x)
  RETURN total
END FUNC
";
    let ir = lower_src_concrete(src, None);
    assert_eq!(captured(&ir, "main"), [vec!["total".to_string()]]);
}

#[test]
fn a_nested_lambdas_assignment_target_is_captured_by_the_outer_one() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  MUT total AS Integer = 0
  LET rows = [[1, 2], [3]]
  collections::forEach(rows, LAMBDA(row AS List OF Integer) -> collections::forEach(row, LAMBDA(x AS Integer) -> total = x))
  RETURN total
END FUNC
";
    let ir = lower_src_concrete(src, None);
    let outer = captured(&ir, "main");
    assert_eq!(
        outer,
        [vec!["total".to_string()]],
        "the outer lambda carries `total`"
    );
}

/// The nested target is captured once even when the outer lambda already read
/// it ahead of the nested one.
#[test]
fn a_nested_target_the_outer_lambda_already_reads_is_captured_once() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  MUT total AS Integer = 0
  LET rows = [[1, 2], [3]]
  collections::forEach(rows, LAMBDA(row AS List OF Integer) -> collections::forEach(collections::append(row, total), LAMBDA(x AS Integer) -> total = x))
  RETURN total
END FUNC
";
    let ir = lower_src_concrete(src, None);
    assert_eq!(captured(&ir, "main"), [vec!["total".to_string()]]);
}

/// A nested lambda assigning a global, or its own parameter, captures nothing
/// from the function: a global is no local, and a parameter is its own.
#[test]
fn a_nested_target_that_is_a_global_or_a_parameter_is_not_captured() {
    let src = "\
IMPORT collections
MUT last AS Integer = 0
FUNC main() AS Integer
  LET rows = [[1, 2], [3]]
  collections::forEach(rows, LAMBDA(row AS List OF Integer) -> collections::forEach(row, LAMBDA(x AS Integer) -> last = x))
  collections::forEach(rows, LAMBDA(row AS List OF Integer) -> collections::forEach(row, LAMBDA(x AS Integer) -> x = 1))
  RETURN last
END FUNC
";
    let ir = lower_src_concrete(src, None);
    assert!(
        captured(&ir, "main").iter().all(Vec::is_empty),
        "{:?}",
        captured(&ir, "main")
    );
}

/// A closure stored into a collection is an element of it (bug-843), so the
/// resource-escape analysis asks which names it captures — the assignment
/// target included — and lowering still builds the environment from them.
#[test]
fn a_stored_assignment_closure_captures_its_target() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  MUT total AS Integer = 0
  MUT setters AS List OF FUNC(Integer) AS Nothing = []
  setters = collections::append(setters, LAMBDA(x AS Integer) -> total = x)
  RETURN len(setters)
END FUNC
";
    let ir = lower_src_concrete(src, None);
    let closures = captured(&ir, "main");
    assert!(
        closures.contains(&vec!["total".to_string()]),
        "the stored closure carries `total`: {closures:?}"
    );
}

#[test]
fn a_field_read_and_a_record_update_capture_the_record() {
    let src = "\
IMPORT collections
TYPE Counter
  count AS Integer
END TYPE
FUNC main() AS Integer
  LET p = Counter[5]
  LET q = Counter[7]
  LET xs = [1, 2, 3]
  LET reads = collections::transform(xs, LAMBDA(n AS Integer) -> p.count + n)
  LET updates = collections::transform(xs, LAMBDA(n AS Integer) -> WITH q { count := n })
  RETURN len(reads) + len(updates)
END FUNC
";
    let ir = lower_src_concrete(src, None);
    assert_eq!(
        captured(&ir, "main"),
        [vec!["p".to_string()], vec!["q".to_string()]]
    );
}

/// The escape analysis asks for a lambda body's captures over any HIR
/// expression; a trapped one captures what its inner expression reads.
#[test]
fn a_trapped_body_captures_what_its_inner_expression_reads() {
    use crate::hir::HirExpression;
    let body = HirExpression::Trapped {
        expression: Box::new(HirExpression::Identifier("total".to_string())),
        binding: "e".to_string(),
        handler: vec![],
        line: 1,
    };
    let outer = ["total".to_string(), "unused".to_string()]
        .into_iter()
        .collect();
    assert_eq!(
        crate::ir::lower::lambda_captured_names(&[], &body, None, &outer),
        ["total".to_string()]
    );
}

/// A list literal with no expected element type — an inline `len` argument —
/// takes its element type from its first element: a Money literal makes a
/// `List OF Money`.
#[test]
fn an_inline_money_list_literal_is_a_list_of_money() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  RETURN len([1.25m, 2.50m])
END FUNC
";
    let ir = lower_src_concrete(src, None);
    let mut list_types = Vec::new();
    walk_ops(&function(&ir, "main").body, &mut |op| {
        if let IrOp::Return {
            value: Some(value), ..
        } = op
        {
            crate::ir::value::visit_value(value, &mut |node| {
                if let IrValue::ListLiteral { type_, .. } = node {
                    list_types.push(type_.clone());
                }
            });
        }
    });
    assert_eq!(list_types, [ParameterType::list_of(ParameterType::Money)]);
}
