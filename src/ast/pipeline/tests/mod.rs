use super::*;

mod copies;

fn placeholder() -> Expression {
    Expression::Identifier("_".to_string())
}

// `SetLiteral` and `Trapped` are the two placeholder-walk arms not reached by
// the parse-driven `pipeline_placeholder_each_expression_kind_as_rhs` fixture
// (a `Set OF T { }` / trapped subexpression is awkward as a pipeline RHS in
// source). Drive them directly: build a tree whose sole placeholder sits in
// the arm, confirm `contains_placeholder` sees it, then substitute a literal
// and confirm the placeholder is gone.

#[test]
fn set_literal_placeholder_arm() {
    let expr = Expression::SetLiteral {
        element_type: "Integer".to_string(),
        elements: vec![placeholder()],
    };
    assert!(contains_placeholder(&expr));
    let input = Expression::Number("1".to_string());
    let rewritten = substitute_placeholder(expr, &input);
    assert!(!contains_placeholder(&rewritten));
    // The literal replaced the `_` in the element position.
    let Expression::SetLiteral { elements, .. } = rewritten else {
        panic!("expected a SetLiteral");
    };
    assert!(matches!(&elements[0], Expression::Number(n) if n == "1"));
}

#[test]
fn trapped_placeholder_arm() {
    // The handler holds statements (not the pipeline input); only the trapped
    // subexpression carries the placeholder, matching bug-171 finding C.
    let expr = Expression::Trapped {
        expression: Box::new(placeholder()),
        binding: "e".to_string(),
        handler: Vec::new(),
        line: 0,
    };
    assert!(contains_placeholder(&expr));
    let input = Expression::Number("2".to_string());
    let rewritten = substitute_placeholder(expr, &input);
    assert!(!contains_placeholder(&rewritten));
    let Expression::Trapped { expression, .. } = rewritten else {
        panic!("expected a Trapped");
    };
    assert!(matches!(&*expression, Expression::Number(n) if n == "2"));
}
