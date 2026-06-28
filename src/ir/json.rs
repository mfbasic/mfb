use super::*;

impl IrProject {
    pub(super) fn to_json(&self) -> String {
        let bindings = if self.bindings.is_empty() {
            String::new()
        } else {
            format!("  \"bindings\": [{}\n  ],\n", join_json(&self.bindings, 2))
        };
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-ir\",\n",
                "  \"version\": 1,\n",
                "  \"project\": {},\n",
                "  \"entry\": {},\n",
                "{}",
                "  \"types\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.name),
            self.entry
                .as_ref()
                .map(|entry| entry.to_json(2))
                .unwrap_or_else(|| "null".to_string()),
            bindings,
            join_json(&self.types, 2),
            join_json(&self.functions, 2)
        )
    }
}

impl EntryPoint {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "{{\n",
                "{}  \"name\": {},\n",
                "{}  \"returns\": {},\n",
                "{}  \"accepts_args\": {}\n",
                "{}}}"
            ),
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.returns),
            pad,
            self.accepts_args,
            pad
        )
    }
}

trait ToIrJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToIrJson for IrType {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self.kind.as_str() {
            "type" => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"kind\": {},\n",
                    "{}  \"visibility\": {},\n",
                    "{}  \"name\": {},\n",
                    "{}  \"fields\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                json_string(&self.kind),
                pad,
                json_string(&self.visibility),
                pad,
                json_string(&self.name),
                pad,
                join_json(&self.fields, indent + 2),
                pad,
                pad
            ),
            "union" => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"kind\": {},\n",
                    "{}  \"visibility\": {},\n",
                    "{}  \"name\": {},\n",
                    "{}  \"includes\": [{}],\n",
                    "{}  \"variants\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                json_string(&self.kind),
                pad,
                json_string(&self.visibility),
                pad,
                json_string(&self.name),
                pad,
                self.includes
                    .iter()
                    .map(|value| json_string(value))
                    .collect::<Vec<_>>()
                    .join(", "),
                pad,
                join_json(&self.variants, indent + 2),
                pad,
                pad
            ),
            "enum" => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"kind\": {},\n",
                    "{}  \"visibility\": {},\n",
                    "{}  \"name\": {},\n",
                    "{}  \"members\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                json_string(&self.kind),
                pad,
                json_string(&self.visibility),
                pad,
                json_string(&self.name),
                pad,
                join_json(&self.members, indent + 2),
                pad,
                pad
            ),
            _ => unreachable!("known IR type kind"),
        }
    }
}

impl ToIrJson for IrBinding {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let value = self
            .value
            .as_ref()
            .map(|value| value.to_json(indent))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"name\": {}, \"visibility\": {}, \"mutable\": {}, \"type\": {}, \"value\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.visibility),
            self.mutable,
            json_string(&self.type_),
            value
        )
    }
}

impl ToIrJson for IrField {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let visibility = self
            .visibility
            .as_ref()
            .map(|value| json_string(value))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"visibility\": {}, \"name\": {}, \"type\": {} }}",
            pad,
            visibility,
            json_string(&self.name),
            json_string(&self.type_)
        )
    }
}

impl ToIrJson for IrVariant {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"fields\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            join_json(&self.fields, indent + 2),
            pad,
            pad
        )
    }
}

impl ToIrJson for IrEnumMember {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!("\n{}{{ \"name\": {} }}", pad, json_string(&self.name))
    }
}

impl ToIrJson for IrFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"visibility\": {},\n",
                "{}  \"kind\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"returns\": {},\n",
                "{}  \"body\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.visibility),
            pad,
            json_string(&self.kind),
            pad,
            join_json(&self.params, indent + 2),
            pad,
            pad,
            json_string(&self.returns),
            pad,
            join_json(&self.body, indent + 2),
            pad,
            pad
        )
    }
}

impl ToIrJson for IrParam {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let default = self
            .default
            .as_ref()
            .map(|value| value.to_json(indent))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"name\": {}, \"type\": {}, \"default\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.type_),
            default
        )
    }
}

impl ToIrJson for IrOp {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self {
            IrOp::Bind {
                mutable,
                name,
                type_,
                value,
            } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!(
                    "\n{}{{ \"op\": \"bind\", \"mutable\": {}, \"name\": {}, \"type\": {}, \"value\": {} }}",
                    pad,
                    mutable,
                    json_string(name),
                    json_string(type_),
                    value
                )
            }
            IrOp::AssignGlobal { name, value } => {
                format!(
                    "\n{}{{ \"op\": \"assignGlobal\", \"name\": {}, \"value\": {} }}",
                    pad,
                    json_string(name),
                    value.to_json(indent)
                )
            }
            IrOp::Return { value } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!("\n{}{{ \"op\": \"return\", \"value\": {} }}", pad, value)
            }
            IrOp::ExitLoop { kind } => {
                format!(
                    "\n{}{{ \"op\": \"exitLoop\", \"loop\": {} }}",
                    pad,
                    json_string(loop_kind_name(*kind))
                )
            }
            IrOp::ContinueLoop { kind } => {
                format!(
                    "\n{}{{ \"op\": \"continueLoop\", \"loop\": {} }}",
                    pad,
                    json_string(loop_kind_name(*kind))
                )
            }
            IrOp::ExitProgram { code } => {
                format!(
                    "\n{}{{ \"op\": \"exitProgram\", \"code\": {} }}",
                    pad,
                    code.to_json(indent)
                )
            }
            IrOp::Fail { error } => {
                format!(
                    "\n{}{{ \"op\": \"fail\", \"error\": {} }}",
                    pad,
                    error.to_json(indent)
                )
            }
            IrOp::Assign { name, value } => {
                format!(
                    "\n{}{{ \"op\": \"assign\", \"name\": {}, \"value\": {} }}",
                    pad,
                    json_string(name),
                    value.to_json(indent)
                )
            }
            IrOp::StateAssign { resource, value } => {
                format!(
                    "\n{}{{ \"op\": \"stateAssign\", \"resource\": {}, \"value\": {} }}",
                    pad,
                    json_string(resource),
                    value.to_json(indent)
                )
            }
            IrOp::Eval { value } => {
                format!(
                    "\n{}{{ \"op\": \"eval\", \"value\": {} }}",
                    pad,
                    value.to_json(indent)
                )
            }
            IrOp::If {
                condition,
                then_body,
                else_body,
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"op\": \"if\",\n",
                        "{}  \"condition\": {},\n",
                        "{}  \"then\": [{}\n{}  ],\n",
                        "{}  \"else\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    condition.to_json(indent),
                    pad,
                    join_json(then_body, indent + 2),
                    pad,
                    pad,
                    join_json(else_body, indent + 2),
                    pad,
                    pad
                )
            }
            IrOp::Match { value, cases } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"op\": \"match\",\n",
                        "{}  \"value\": {},\n",
                        "{}  \"cases\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    value.to_json(indent),
                    pad,
                    join_json(cases, indent + 2),
                    pad,
                    pad
                )
            }
            IrOp::While {
                kind,
                condition,
                body,
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"op\": \"while\",\n",
                        "{}  \"loop\": {},\n",
                        "{}  \"condition\": {},\n",
                        "{}  \"body\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    json_string(loop_kind_name(*kind)),
                    pad,
                    condition.to_json(indent),
                    pad,
                    join_json(body, indent + 2),
                    pad,
                    pad
                )
            }
            IrOp::For {
                name,
                type_,
                start,
                end,
                step,
                body,
                ..
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"op\": \"for\",\n",
                        "{}  \"name\": {},\n",
                        "{}  \"type\": {},\n",
                        "{}  \"start\": {},\n",
                        "{}  \"end\": {},\n",
                        "{}  \"step\": {},\n",
                        "{}  \"body\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    json_string(name),
                    pad,
                    json_string(type_),
                    pad,
                    start.to_json(indent),
                    pad,
                    end.to_json(indent),
                    pad,
                    step.to_json(indent),
                    pad,
                    join_json(body, indent + 2),
                    pad,
                    pad
                )
            }
            IrOp::DoUntil { body, condition } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"op\": \"doUntil\",\n",
                        "{}  \"condition\": {},\n",
                        "{}  \"body\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    condition.to_json(indent),
                    pad,
                    join_json(body, indent + 2),
                    pad,
                    pad
                )
            }
            IrOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"op\": \"forEach\",\n",
                        "{}  \"name\": {},\n",
                        "{}  \"type\": {},\n",
                        "{}  \"iterable\": {},\n",
                        "{}  \"body\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    json_string(name),
                    pad,
                    json_string(type_),
                    pad,
                    iterable.to_json(indent),
                    pad,
                    join_json(body, indent + 2),
                    pad,
                    pad
                )
            }
            IrOp::Trap { name, body } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"op\": \"trap\",\n",
                        "{}  \"name\": {},\n",
                        "{}  \"body\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    json_string(name),
                    pad,
                    join_json(body, indent + 2),
                    pad,
                    pad
                )
            }
        }
    }
}

impl ToIrJson for IrMatchCase {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"pattern\": {},\n",
                "{}  \"guard\": {},\n",
                "{}  \"body\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            self.pattern.to_json(indent),
            pad,
            self.guard
                .as_ref()
                .map(|guard| guard.to_json(indent))
                .unwrap_or_else(|| "null".to_string()),
            pad,
            join_json(&self.body, indent + 2),
            pad,
            pad
        )
    }
}

impl ToIrJson for IrMatchPattern {
    fn to_json(&self, indent: usize) -> String {
        match self {
            IrMatchPattern::Else => "{ \"kind\": \"else\" }".to_string(),
            IrMatchPattern::Value(value) => {
                format!(
                    "{{ \"kind\": \"value\", \"value\": {} }}",
                    value.to_json(indent)
                )
            }
            IrMatchPattern::OneOf(values) => format!(
                "{{ \"kind\": \"oneOf\", \"values\": [{}] }}",
                values
                    .iter()
                    .map(|value| value.to_json(indent))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl ToIrJson for IrValue {
    fn to_json(&self, _indent: usize) -> String {
        match self {
            IrValue::Const { type_, value } => {
                format!(
                    "{{ \"kind\": \"const\", \"type\": {}, \"value\": {} }}",
                    json_string(type_),
                    json_string(value)
                )
            }
            IrValue::Local(name) => {
                format!("{{ \"kind\": \"local\", \"name\": {} }}", json_string(name))
            }
            IrValue::Global(name) => {
                format!(
                    "{{ \"kind\": \"global\", \"name\": {} }}",
                    json_string(name)
                )
            }
            IrValue::LocalRef { name, type_ } => {
                format!(
                    "{{ \"kind\": \"localRef\", \"name\": {}, \"type\": {} }}",
                    json_string(name),
                    json_string(type_)
                )
            }
            IrValue::FunctionRef { name, type_ } => {
                format!(
                    "{{ \"kind\": \"functionRef\", \"name\": {}, \"type\": {} }}",
                    json_string(name),
                    json_string(type_)
                )
            }
            IrValue::Closure {
                name,
                type_,
                captures,
            } => {
                let captures = captures
                    .iter()
                    .map(|arg| arg.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"closure\", \"name\": {}, \"type\": {}, \"captures\": [{}] }}",
                    json_string(name),
                    json_string(type_),
                    captures
                )
            }
            IrValue::Capture {
                index,
                type_,
                by_ref,
            } => {
                // Emit `byRef` only for a slot-borrow capture so ordinary by-value
                // captures keep their existing serialization.
                if *by_ref {
                    format!(
                        "{{ \"kind\": \"capture\", \"index\": {}, \"type\": {}, \"byRef\": true }}",
                        index,
                        json_string(type_)
                    )
                } else {
                    format!(
                        "{{ \"kind\": \"capture\", \"index\": {}, \"type\": {} }}",
                        index,
                        json_string(type_)
                    )
                }
            }
            IrValue::Call { target, args, .. } => {
                let args = args
                    .iter()
                    .map(|arg| arg.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"call\", \"target\": {}, \"args\": [{}] }}",
                    json_string(target),
                    args
                )
            }
            IrValue::CallResult { target, args, .. } => {
                let args = args
                    .iter()
                    .map(|arg| arg.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"callResult\", \"target\": {}, \"args\": [{}] }}",
                    json_string(target),
                    args
                )
            }
            IrValue::Constructor { type_, args } => {
                let args = args
                    .iter()
                    .map(|arg| arg.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"constructor\", \"type\": {}, \"args\": [{}] }}",
                    json_string(type_),
                    args
                )
            }
            IrValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => format!(
                "{{ \"kind\": \"unionWrap\", \"union\": {}, \"member\": {}, \"value\": {} }}",
                json_string(union_type),
                json_string(member_type),
                value.to_json(0)
            ),
            IrValue::UnionExtract { type_, value } => format!(
                "{{ \"kind\": \"unionExtract\", \"type\": {}, \"value\": {} }}",
                json_string(type_),
                value.to_json(0)
            ),
            IrValue::ResultIsOk { value } => format!(
                "{{ \"kind\": \"resultIsOk\", \"value\": {} }}",
                value.to_json(0)
            ),
            IrValue::ResultValue { value } => format!(
                "{{ \"kind\": \"resultValue\", \"value\": {} }}",
                value.to_json(0)
            ),
            IrValue::ResultError { value } => format!(
                "{{ \"kind\": \"resultError\", \"value\": {} }}",
                value.to_json(0)
            ),
            IrValue::WithUpdate {
                type_,
                target,
                updates,
            } => {
                let updates = updates
                    .iter()
                    .map(|update| update.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"with\", \"type\": {}, \"target\": {}, \"updates\": [{}] }}",
                    json_string(type_),
                    target.to_json(0),
                    updates
                )
            }
            IrValue::ListLiteral { type_, values } => {
                let values = values
                    .iter()
                    .map(|value| value.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"list\", \"type\": {}, \"values\": [{}] }}",
                    json_string(type_),
                    values
                )
            }
            IrValue::MapLiteral { type_, entries } => {
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
                    "{{ \"kind\": \"map\", \"type\": {}, \"entries\": [{}] }}",
                    json_string(type_),
                    entries
                )
            }
            IrValue::MemberAccess { target, member } => {
                format!(
                    "{{ \"kind\": \"memberAccess\", \"target\": {}, \"member\": {} }}",
                    target.to_json(0),
                    json_string(member)
                )
            }
            IrValue::Binary {
                op, left, right, ..
            } => {
                format!(
                    "{{ \"kind\": \"binary\", \"op\": {}, \"left\": {}, \"right\": {} }}",
                    json_string(op),
                    left.to_json(0),
                    right.to_json(0)
                )
            }
            IrValue::Unary { op, operand, .. } => {
                format!(
                    "{{ \"kind\": \"unary\", \"op\": {}, \"operand\": {} }}",
                    json_string(op),
                    operand.to_json(0)
                )
            }
        }
    }
}

impl ToIrJson for IrRecordUpdate {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"field\": {}, \"value\": {} }}",
            json_string(&self.field),
            self.value.to_json(0)
        )
    }
}

fn join_json<T: ToIrJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn loop_kind_name(kind: LoopKind) -> &'static str {
    match kind {
        LoopKind::For => "for",
        LoopKind::Do => "do",
        LoopKind::While => "while",
    }
}

pub(crate) fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Private => "private",
        Visibility::Package => "package",
        Visibility::Export => "export",
    }
}

// ===========================================================================
// Binary Representation (structured) encode/decode
// ===========================================================================
//
// The package payload is a faithful, versioned binary serialization of the
// compiler's IR (`IrProject` / `IrFunction` / `IrOp` / `IrValue` / `IrType`).
// It is *not* a flat opcode stream: control flow stays nested (regions with
// explicit ends) and expressions stay as trees, exactly as in memory. The
// in-memory IR is free to change behind this format; this encoding is the
// stable contract.
//
// The format is self-contained: strings are inline length-prefixed, integers
// are little-endian. There is no separate interned pool here — the `.mfp`
// container's tables (manifest/ABI/import/export) are kept and derived
// alongside this payload by `binary_repr.rs`, but a function body is faithfully
// reconstructable from this payload alone.

