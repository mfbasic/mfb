//! Pipeline `|>` placeholder desugaring.
//!
//! `parse_pipeline` (see [`super::expr`]) lowers `left |> right` by substituting
//! `left` for each `_` placeholder in `right`. These helpers do the expression-
//! tree walk: [`contains_placeholder`] decides whether `right` names a
//! placeholder at all, and [`substitute_placeholder`] performs the rewrite. They
//! previously lived in the `-ast` JSON dumper (`serialize.rs`) and were re-imported
//! back out of it through `mod.rs` to reach the parser — a semantic AST rewrite
//! has nothing to do with dumping AST, so it lives here, imported directly by the
//! parser.

use super::*;

/// What `parse_pipeline` must know about a right-hand side before it splices
/// the left operand in for each `_`: how many placeholders there are (none is
/// an error; more than one COPIES the operand) and the depths that decide how
/// deep the spliced result is (bug-501).
pub(super) struct PlaceholderShape {
    /// Number of `_` leaves.
    pub(super) count: usize,
    /// Tree depth of the right-hand side itself (0 for a lone `_`).
    pub(super) depth: usize,
    /// Depth of the deepest `_` leaf (0 when there is none). The operand's root
    /// lands there, so the spliced tree is
    /// `max(depth, placeholder_depth + operand depth)` deep.
    pub(super) placeholder_depth: usize,
}

/// Test-only convenience over [`placeholder_shape`] for the arm-coverage tests
/// below; the parser reads the full shape.
#[cfg(test)]
pub(super) fn contains_placeholder(expression: &Expression) -> bool {
    placeholder_shape(expression).count > 0
}

pub(super) fn placeholder_shape(expression: &Expression) -> PlaceholderShape {
    shape_at(expression, 0)
}

/// Shape of the subtree rooted at `expression`, which sits `at` levels below
/// the right-hand side's root. Recursing here is safe: every subtree reaching
/// this walk was depth-checked as the parser built it.
fn shape_at(expression: &Expression, at: usize) -> PlaceholderShape {
    if let Expression::Identifier(value) = expression {
        let is_placeholder = value == "_";
        return PlaceholderShape {
            count: usize::from(is_placeholder),
            depth: at,
            placeholder_depth: if is_placeholder { at } else { 0 },
        };
    }
    let mut shape = PlaceholderShape {
        count: 0,
        depth: at,
        placeholder_depth: 0,
    };
    let mut visit = |child: &Expression| {
        let inner = shape_at(child, at + 1);
        shape.count += inner.count;
        shape.depth = shape.depth.max(inner.depth);
        shape.placeholder_depth = shape.placeholder_depth.max(inner.placeholder_depth);
    };
    match expression {
        Expression::Binary { left, right, .. } => {
            visit(left);
            visit(right);
        }
        Expression::Unary { operand, .. } => visit(operand),
        Expression::Call { arguments, .. } => {
            for argument in arguments {
                visit(call_arg_value(argument));
            }
        }
        Expression::Constructor { arguments, .. } => {
            for argument in arguments {
                visit(constructor_arg_value(argument));
            }
        }
        Expression::Lambda { body, .. } => visit(body),
        Expression::ListLiteral(values) => {
            for value in values {
                visit(value);
            }
        }
        Expression::SetLiteral { elements, .. } => {
            for element in elements {
                visit(element);
            }
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                visit(key);
                visit(value);
            }
        }
        Expression::MemberAccess { target, .. } => visit(target),
        // The handler holds statements (never the pipeline input); only the
        // trapped subexpression can carry a placeholder (bug-171 finding C).
        Expression::Trapped { expression, .. } => visit(expression),
        Expression::WithUpdate { target, updates } => {
            visit(target);
            for update in updates {
                visit(&update.value);
            }
        }
        Expression::Identifier(_)
        | Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_) => {}
    }
    shape
}

/// Number of expression nodes in `expression`. `parse_pipeline` charges this
/// once per placeholder beyond the first, since each extra `_` receives its own
/// copy of the whole operand (bug-501 B). A `Trapped` handler holds statements,
/// not expressions, and is not counted: a postfix trap wraps a whole statement's
/// expression, so one can never be a pipeline operand.
pub(super) fn node_count(expression: &Expression) -> usize {
    let children: usize = match expression {
        Expression::Binary { left, right, .. } => node_count(left) + node_count(right),
        Expression::Unary { operand, .. } => node_count(operand),
        Expression::Call { arguments, .. } => arguments
            .iter()
            .map(|argument| node_count(call_arg_value(argument)))
            .sum(),
        Expression::Constructor { arguments, .. } => arguments
            .iter()
            .map(|argument| node_count(constructor_arg_value(argument)))
            .sum(),
        Expression::Lambda { body, .. } => node_count(body),
        Expression::ListLiteral(values) => values.iter().map(node_count).sum(),
        Expression::SetLiteral { elements, .. } => elements.iter().map(node_count).sum(),
        Expression::MapLiteral { entries, .. } => entries
            .iter()
            .map(|(key, value)| node_count(key) + node_count(value))
            .sum(),
        Expression::MemberAccess { target, .. } => node_count(target),
        Expression::Trapped { expression, .. } => node_count(expression),
        Expression::WithUpdate { target, updates } => {
            node_count(target)
                + updates
                    .iter()
                    .map(|update| node_count(&update.value))
                    .sum::<usize>()
        }
        Expression::Identifier(_)
        | Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_) => 0,
    };
    children + 1
}

fn constructor_arg_value(argument: &ConstructorArg) -> &Expression {
    match argument {
        ConstructorArg::Positional(value) | ConstructorArg::Named { value, .. } => value,
    }
}

fn call_arg_value(argument: &CallArg) -> &Expression {
    match argument {
        CallArg::Positional(value) | CallArg::Named { value, .. } => value,
    }
}

pub(super) fn substitute_placeholder(expression: Expression, input: &Expression) -> Expression {
    match expression {
        Expression::Identifier(value) if value == "_" => input.clone(),
        Expression::Binary {
            left,
            operator,
            right,
            line,
            column,
        } => Expression::Binary {
            left: Box::new(substitute_placeholder(*left, input)),
            operator,
            right: Box::new(substitute_placeholder(*right, input)),
            line,
            column,
        },
        Expression::Unary {
            operator,
            operand,
            line,
            column,
        } => Expression::Unary {
            operator,
            operand: Box::new(substitute_placeholder(*operand, input)),
            line,
            column,
        },
        Expression::Call {
            callee,
            arguments,
            line,
            column,
        } => Expression::Call {
            callee,
            arguments: arguments
                .into_iter()
                .map(|argument| substitute_placeholder_call_arg(argument, input))
                .collect(),
            line,
            column,
        },
        Expression::Lambda {
            params,
            body,
            assign_target,
        } => Expression::Lambda {
            params,
            body: Box::new(substitute_placeholder(*body, input)),
            assign_target,
        },
        Expression::Constructor {
            type_name,
            arguments,
        } => Expression::Constructor {
            type_name,
            arguments: arguments
                .into_iter()
                .map(|argument| substitute_placeholder_constructor_arg(argument, input))
                .collect(),
        },
        Expression::ListLiteral(values) => Expression::ListLiteral(
            values
                .into_iter()
                .map(|value| substitute_placeholder(value, input))
                .collect(),
        ),
        Expression::SetLiteral {
            element_type,
            elements,
        } => Expression::SetLiteral {
            element_type,
            elements: elements
                .into_iter()
                .map(|value| substitute_placeholder(value, input))
                .collect(),
        },
        Expression::MapLiteral {
            key_type,
            value_type,
            entries,
        } => Expression::MapLiteral {
            key_type,
            value_type,
            entries: entries
                .into_iter()
                .map(|(key, value)| {
                    (
                        substitute_placeholder(key, input),
                        substitute_placeholder(value, input),
                    )
                })
                .collect(),
        },
        Expression::MemberAccess { target, member } => Expression::MemberAccess {
            target: Box::new(substitute_placeholder(*target, input)),
            member,
        },
        // Mirror `contains_placeholder`, which walks a `Trapped`'s inner
        // expression: substitute there too so a `_` inside a trapped subexpression
        // is rewritten rather than silently left behind (bug-171 finding C). The
        // handler body holds statements (not the pipeline input) and is left as-is.
        Expression::Trapped {
            expression,
            binding,
            handler,
            line,
        } => Expression::Trapped {
            expression: Box::new(substitute_placeholder(*expression, input)),
            binding,
            handler,
            line,
        },
        Expression::WithUpdate { target, updates } => Expression::WithUpdate {
            target: Box::new(substitute_placeholder(*target, input)),
            updates: updates
                .into_iter()
                .map(|update| RecordUpdate {
                    field: update.field,
                    value: substitute_placeholder(update.value, input),
                    line: update.line,
                })
                .collect(),
        },
        other => other,
    }
}

fn substitute_placeholder_constructor_arg(
    argument: ConstructorArg,
    input: &Expression,
) -> ConstructorArg {
    match argument {
        ConstructorArg::Positional(value) => {
            ConstructorArg::Positional(substitute_placeholder(value, input))
        }
        ConstructorArg::Named { name, value, line } => ConstructorArg::Named {
            name,
            value: substitute_placeholder(value, input),
            line,
        },
    }
}

fn substitute_placeholder_call_arg(argument: CallArg, input: &Expression) -> CallArg {
    match argument {
        CallArg::Positional(value) => CallArg::Positional(substitute_placeholder(value, input)),
        CallArg::Named { name, value, line } => CallArg::Named {
            name,
            value: substitute_placeholder(value, input),
            line,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
