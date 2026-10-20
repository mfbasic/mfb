//! plan-176-M: AST JSON serialization of statements, match cases and patterns, and
//! expressions. The project/file entry points and the shared helpers live in
//! `ast/serialize/mod.rs`.

use super::*;

impl ToJson for Statement {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self {
            Statement::Let {
                mutable,
                resource,
                state_type,
                name,
                type_name,
                value,
                line,
            } => {
                let type_name = type_name
                    .as_ref()
                    .map(|value| json_string(value))
                    .unwrap_or_else(|| "null".to_string());
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!(
                    "\n{}{{ \"kind\": \"binding\", \"mutable\": {}{}, \"name\": {}, \"type\": {}, \"value\": {}, \"line\": {} }}",
                    pad,
                    mutable,
                    resource_json_suffix(*resource, state_type),
                    json_string(name),
                    type_name,
                    value,
                    line
                )
            }
            Statement::Return { value, line } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!(
                    "\n{}{{ \"kind\": \"return\", \"value\": {}, \"line\": {} }}",
                    pad, value, line
                )
            }
            Statement::Exit { kind, line } => {
                format!(
                    "\n{}{{ \"kind\": \"exit\", \"target\": {}, \"line\": {} }}",
                    pad,
                    json_string(kind.name()),
                    line
                )
            }
            Statement::Continue { kind, line } => {
                format!(
                    "\n{}{{ \"kind\": \"continue\", \"loop\": {}, \"line\": {} }}",
                    pad,
                    json_string(kind.name()),
                    line
                )
            }
            Statement::Fail { error, line } => {
                format!(
                    "\n{}{{ \"kind\": \"fail\", \"error\": {}, \"line\": {} }}",
                    pad,
                    error.to_json(indent),
                    line
                )
            }
            Statement::Propagate { line } => {
                format!("\n{}{{ \"kind\": \"propagate\", \"line\": {} }}", pad, line)
            }
            Statement::Recover { value, line } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!(
                    "\n{}{{ \"kind\": \"recover\", \"value\": {}, \"line\": {} }}",
                    pad, value, line
                )
            }
            Statement::Assign { name, value, line } => {
                format!(
                    "\n{}{{ \"kind\": \"assignment\", \"name\": {}, \"value\": {}, \"line\": {} }}",
                    pad,
                    json_string(name),
                    value.to_json(indent),
                    line
                )
            }
            Statement::StateAssign {
                resource,
                value,
                line,
            } => {
                format!(
                    "\n{}{{ \"kind\": \"stateAssignment\", \"resource\": {}, \"value\": {}, \"line\": {} }}",
                    pad,
                    json_string(resource),
                    value.to_json(indent),
                    line
                )
            }
            Statement::Expression { expression, line } => {
                format!(
                    "\n{}{{ \"kind\": \"expression\", \"expression\": {}, \"line\": {} }}",
                    pad,
                    expression.to_json(indent),
                    line
                )
            }
            Statement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"kind\": \"if\",\n",
                        "{}  \"condition\": {},\n",
                        "{}  \"line\": {},\n",
                        "{}  \"then\": [{}\n{}  ],\n",
                        "{}  \"else\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    condition.to_json(0),
                    pad,
                    line,
                    pad,
                    join_json(then_body, indent + 2),
                    pad,
                    pad,
                    join_json(else_body, indent + 2),
                    pad,
                    pad
                )
            }
            Statement::Match {
                expression,
                cases,
                line,
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"kind\": \"match\",\n",
                        "{}  \"expression\": {},\n",
                        "{}  \"line\": {},\n",
                        "{}  \"cases\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    expression.to_json(0),
                    pad,
                    line,
                    pad,
                    join_json(cases, indent + 2),
                    pad,
                    pad
                )
            }
            Statement::For {
                name,
                start,
                end,
                step,
                body,
                line,
            } => {
                let step = step
                    .as_ref()
                    .map(|value| value.to_json(0))
                    .unwrap_or_else(|| "null".to_string());
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"kind\": \"for\",\n",
                        "{}  \"name\": {},\n",
                        "{}  \"start\": {},\n",
                        "{}  \"end\": {},\n",
                        "{}  \"step\": {},\n",
                        "{}  \"line\": {},\n",
                        "{}  \"body\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    json_string(name),
                    pad,
                    start.to_json(0),
                    pad,
                    end.to_json(0),
                    pad,
                    step,
                    pad,
                    line,
                    pad,
                    join_json(body, indent + 2),
                    pad,
                    pad
                )
            }
            Statement::While {
                kind,
                condition,
                body,
                line,
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"kind\": \"while\",\n",
                        "{}  \"loop\": {},\n",
                        "{}  \"condition\": {},\n",
                        "{}  \"line\": {},\n",
                        "{}  \"body\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    json_string(kind.name()),
                    pad,
                    condition.to_json(0),
                    pad,
                    line,
                    pad,
                    join_json(body, indent + 2),
                    pad,
                    pad
                )
            }
            Statement::DoUntil {
                body,
                condition,
                line,
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"kind\": \"doUntil\",\n",
                        "{}  \"condition\": {},\n",
                        "{}  \"line\": {},\n",
                        "{}  \"body\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    condition.to_json(0),
                    pad,
                    line,
                    pad,
                    join_json(body, indent + 2),
                    pad,
                    pad
                )
            }
            Statement::ForEach {
                name,
                iterable,
                body,
                line,
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"kind\": \"forEach\",\n",
                        "{}  \"name\": {},\n",
                        "{}  \"iterable\": {},\n",
                        "{}  \"line\": {},\n",
                        "{}  \"body\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    json_string(name),
                    pad,
                    iterable.to_json(0),
                    pad,
                    line,
                    pad,
                    join_json(body, indent + 2),
                    pad,
                    pad
                )
            }
        }
    }
}

impl ToJson for MatchCase {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let guard = self
            .guard
            .as_ref()
            .map(|guard| guard.to_json(indent))
            .unwrap_or_else(|| "null".to_string());
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"pattern\": {},\n",
                "{}  \"guard\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"body\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            self.pattern.to_json(indent),
            pad,
            guard,
            pad,
            self.line,
            pad,
            join_json(&self.body, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for MatchPattern {
    fn to_json(&self, indent: usize) -> String {
        match self {
            MatchPattern::Else => "{ \"kind\": \"else\" }".to_string(),
            MatchPattern::Literal(expression) => {
                format!(
                    "{{ \"kind\": \"literal\", \"expression\": {} }}",
                    expression.to_json(indent)
                )
            }
            MatchPattern::Union { type_name, binding } => format!(
                "{{ \"kind\": \"union\", \"type\": {}, \"binding\": {} }}",
                json_string(type_name),
                json_string(binding)
            ),
            MatchPattern::OneOf(expressions) => format!(
                "{{ \"kind\": \"oneOf\", \"patterns\": [{}] }}",
                expressions
                    .iter()
                    .map(|expression| expression.to_json(indent))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl ToJson for Expression {
    fn to_json(&self, indent: usize) -> String {
        match self {
            Expression::String(value) => {
                format!(
                    "{{ \"kind\": \"string\", \"value\": {} }}",
                    json_string(value)
                )
            }
            Expression::Number(value) => {
                format!(
                    "{{ \"kind\": \"number\", \"value\": {} }}",
                    json_string(value)
                )
            }
            Expression::Scalar(code_point) => {
                format!("{{ \"kind\": \"scalar\", \"value\": {} }}", code_point)
            }
            Expression::Boolean(value) => {
                format!("{{ \"kind\": \"boolean\", \"value\": {} }}", value)
            }
            Expression::Binary {
                left,
                operator,
                right,
                ..
            } => {
                format!(
                    "{{ \"kind\": \"binary\", \"operator\": {}, \"left\": {}, \"right\": {} }}",
                    json_string(operator.name()),
                    left.to_json(0),
                    right.to_json(0)
                )
            }
            Expression::Unary {
                operator, operand, ..
            } => {
                format!(
                    "{{ \"kind\": \"unary\", \"operator\": {}, \"operand\": {} }}",
                    json_string(operator.name()),
                    operand.to_json(0)
                )
            }
            Expression::Call {
                callee, arguments, ..
            } => {
                let args = arguments
                    .iter()
                    .map(|arg| arg.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"call\", \"callee\": {}, \"arguments\": [{}] }}",
                    json_string(callee),
                    args
                )
            }
            Expression::Lambda {
                params,
                body,
                assign_target,
            } => {
                let params = params
                    .iter()
                    .map(|param| param.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                match assign_target {
                    Some(target) => format!(
                        "{{ \"kind\": \"lambda\", \"params\": [{}], \"assignTarget\": {}, \"body\": {} }}",
                        params,
                        json_string(target),
                        body.to_json(0)
                    ),
                    None => format!(
                        "{{ \"kind\": \"lambda\", \"params\": [{}], \"body\": {} }}",
                        params,
                        body.to_json(0)
                    ),
                }
            }
            Expression::Constructor {
                type_name,
                arguments,
            } => {
                let args = arguments
                    .iter()
                    .map(|arg| arg.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"constructor\", \"type\": {}, \"arguments\": [{}] }}",
                    json_string(type_name),
                    args
                )
            }
            Expression::WithUpdate { target, updates } => {
                let updates = updates
                    .iter()
                    .map(|update| update.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"with\", \"target\": {}, \"updates\": [{}] }}",
                    target.to_json(0),
                    updates
                )
            }
            Expression::ListLiteral(values) => {
                let values = values
                    .iter()
                    .map(|value| value.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{ \"kind\": \"list\", \"values\": [{}] }}", values)
            }
            Expression::SetLiteral {
                element_type,
                elements,
            } => {
                let values = elements
                    .iter()
                    .map(|value| value.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"set\", \"elementType\": {}, \"values\": [{}] }}",
                    json_string(element_type),
                    values
                )
            }
            Expression::MapLiteral {
                key_type,
                value_type,
                entries,
            } => {
                let entries = entries
                    .iter()
                    .map(|(key, value)| {
                        format!(
                            "{{ \"key\": {}, \"value\": {} }}",
                            key.to_json(0),
                            value.to_json(0)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"map\", \"keyType\": {}, \"valueType\": {}, \"entries\": [{}] }}",
                    json_string(key_type),
                    json_string(value_type),
                    entries
                )
            }
            Expression::MemberAccess { target, member } => {
                format!(
                    "{{ \"kind\": \"memberAccess\", \"target\": {}, \"member\": {} }}",
                    target.to_json(0),
                    json_string(member)
                )
            }
            Expression::Trapped {
                expression,
                binding,
                handler,
                line,
            } => {
                let pad = " ".repeat(indent);
                format!(
                    concat!(
                        "{{\n",
                        "{}  \"kind\": \"trapped\",\n",
                        "{}  \"binding\": {},\n",
                        "{}  \"line\": {},\n",
                        "{}  \"expression\": {},\n",
                        "{}  \"handler\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    json_string(binding),
                    pad,
                    line,
                    pad,
                    expression.to_json(0),
                    pad,
                    join_json(handler, indent + 2),
                    pad,
                    pad
                )
            }
            Expression::Identifier(value) => {
                format!(
                    "{{ \"kind\": \"identifier\", \"value\": {} }}",
                    json_string(value)
                )
            }
        }
    }
}

impl CallArg {
    fn to_json(&self, _indent: usize) -> String {
        match self {
            CallArg::Positional(value) => value.to_json(0),
            CallArg::Named { name, value, .. } => format!(
                "{{ \"kind\": \"named\", \"name\": {}, \"value\": {} }}",
                json_string(name),
                value.to_json(0)
            ),
        }
    }
}

impl ConstructorArg {
    fn to_json(&self, _indent: usize) -> String {
        match self {
            ConstructorArg::Positional(value) => value.to_json(0),
            ConstructorArg::Named { name, value, .. } => format!(
                "{{ \"kind\": \"named\", \"name\": {}, \"value\": {} }}",
                json_string(name),
                value.to_json(0)
            ),
        }
    }
}

impl RecordUpdate {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"field\": {}, \"value\": {} }}",
            json_string(&self.field),
            self.value.to_json(0)
        )
    }
}
