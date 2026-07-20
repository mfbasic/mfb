use super::*;

impl NirModule {
    pub(crate) fn to_json(&self) -> String {
        let globals = if self.globals.is_empty() {
            String::new()
        } else {
            format!("  \"globals\": [{}\n  ],\n", join_json(&self.globals, 2))
        };
        let link_functions = if self.link_functions.is_empty() {
            String::new()
        } else {
            format!(
                "  \"linkFunctions\": [{}\n  ],\n",
                self.link_functions
                    .iter()
                    .map(|function| link_function_json(function, 2))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-nir\",\n",
                "  \"version\": 1,\n",
                "  \"target\": {},\n",
                "  \"buildMode\": {},\n",
                "  \"project\": {},\n",
                "  \"entry\": {},\n",
                "{}",
                "  \"types\": [{}\n  ],\n",
                "  \"imports\": [{}\n  ],\n",
                "{}",
                "  \"runtimeHelpers\": [{}],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
            json_string(self.build_mode.as_str()),
            json_string(&self.project),
            self.entry
                .as_ref()
                .map(|entry| entry.to_json(2))
                .unwrap_or_else(|| "null".to_string()),
            globals,
            join_json(&self.types, 2),
            join_json(&self.imports, 2),
            link_functions,
            self.runtime_helpers
                .iter()
                .map(|helper| json_string(helper.name()))
                .collect::<Vec<_>>()
                .join(", "),
            join_json(&self.functions, 2)
        )
    }
}

trait ToNirJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToNirJson for NirGlobal {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let value = self
            .value
            .as_ref()
            .map(|value| value.to_json(indent))
            .unwrap_or_else(|| "null".to_string());
        format!(
            concat!(
                "\n{}{{ \"name\": {}, \"symbol\": {}, \"visibility\": {}, ",
                "\"mutable\": {}, \"type\": {}, \"value\": {} }}"
            ),
            pad,
            json_string(&self.name),
            json_string(&self.symbol),
            json_string(&self.visibility),
            self.mutable,
            json_string(&self.type_),
            value
        )
    }
}

impl ToNirJson for NirEntryPoint {
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

impl ToNirJson for NirType {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self.kind.as_str() {
            // `record` and `resource` share the field-carrying shape of `type`
            // (`validate_nir`'s `type_value_names` accepts all three
            // interchangeably), so they render identically. A crafted `.mfp` can
            // carry either kind and must dump without panicking (bug-70).
            "type" | "record" | "resource" => format!(
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
            // `validate_nir` (`type_value_names`) runs before every `to_json`
            // caller and rejects any kind other than
            // type/record/resource/union/enum, so no other kind can reach here.
            other => unreachable!(
                "validate_nir rejects NIR type kind '{other}' before to_json is called"
            ),
        }
    }
}

impl ToNirJson for NirField {
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

impl ToNirJson for NirVariant {
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

impl ToNirJson for NirEnumMember {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!("\n{}{{ \"name\": {} }}", pad, json_string(&self.name))
    }
}

impl ToNirJson for NirImport {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"package\": {},\n",
                "{}  \"name\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"kind\": {},\n",
                "{}  \"isolated\": {},\n",
                "{}  \"params\": [{}],\n",
                "{}  \"returns\": {}\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.package),
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.symbol),
            pad,
            json_string(&self.kind),
            pad,
            self.isolated,
            pad,
            self.params
                .iter()
                .map(|param| param.to_json(0))
                .collect::<Vec<_>>()
                .join(", "),
            pad,
            json_string(&self.returns),
            pad
        )
    }
}

impl ToNirJson for NirImportParam {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"type\": {}, \"hasDefault\": {} }}",
            json_string(&self.type_),
            self.has_default
        )
    }
}

impl ToNirJson for NirFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let file = if self.file.is_empty() {
            String::new()
        } else {
            format!("{pad}  \"file\": {},\n", json_string(&self.file))
        };
        let resource_owners = if self.resource_owners.is_empty() {
            String::new()
        } else {
            // Sort by binding name so the dump is deterministic (the source is a
            // HashMap).
            let mut owners: Vec<(&String, &crate::escape::ResOwner)> =
                self.resource_owners.iter().collect();
            owners.sort_by(|a, b| a.0.cmp(b.0));
            let entries = owners
                .iter()
                .map(|(name, owner)| {
                    format!(
                        "{{ \"name\": {}, \"owner\": {} }}",
                        json_string(name),
                        res_owner_json(owner)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{pad}  \"resourceOwners\": [{entries}],\n")
        };
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"visibility\": {},\n",
                "{}  \"kind\": {},\n",
                "{}  \"isolated\": {},\n",
                "{}",
                "{}",
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
            self.isolated,
            file,
            resource_owners,
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

/// Render a resource-ownership decision for the `-nir` dump (bug-139.4). Dump-only;
/// no parser reads it back.
fn res_owner_json(owner: &crate::escape::ResOwner) -> String {
    match owner {
        crate::escape::ResOwner::Local => "{ \"kind\": \"local\" }".to_string(),
        crate::escape::ResOwner::Float(scope) => {
            format!(
                "{{ \"kind\": \"float\", \"scope\": {} }}",
                json_string(scope)
            )
        }
        crate::escape::ResOwner::FloatBlocked(scope) => {
            format!(
                "{{ \"kind\": \"float-blocked\", \"scope\": {} }}",
                json_string(scope)
            )
        }
    }
}

/// Render a native `LINK` function for the `-nir` dump (bug-139.4). Dump-only; no
/// parser reads it back.
fn link_function_json(function: &crate::ir::IrLinkFunction, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let params = function
        .params
        .iter()
        .map(|(name, type_)| {
            format!(
                "{{ \"name\": {}, \"type\": {} }}",
                json_string(name),
                json_string(type_)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let abi_slots = function
        .abi_slots
        .iter()
        .map(|slot| {
            format!(
                "{{ \"name\": {}, \"ctype\": {}, \"out\": {} }}",
                json_string(&slot.name),
                json_string(&slot.ctype),
                slot.direction.writes_back()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let consts = function
        .consts
        .iter()
        .map(|(slot, value)| {
            format!(
                "{{ \"slot\": {}, \"value\": {} }}",
                json_string(slot),
                value
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let success_on = function
        .success_on
        .as_ref()
        .map(link_expr_json)
        .unwrap_or_else(|| "null".to_string());
    let result = function
        .result
        .as_ref()
        .map(link_expr_json)
        .unwrap_or_else(|| "null".to_string());
    let free = function
        .free
        .as_ref()
        .map(|free| {
            format!(
                "{{ \"slot\": {}, \"symbol\": {} }}",
                json_string(&free.slot),
                json_string(&free.symbol)
            )
        })
        .unwrap_or_else(|| "null".to_string());
    format!(
        concat!(
            "\n{}{{ \"alias\": {}, \"name\": {}, \"library\": {}, \"symbol\": {}, ",
            "\"params\": [{}], \"returnType\": {}, \"returnResource\": {}, ",
            "\"abiSlots\": [{}], \"abiReturnName\": {}, \"abiReturnCtype\": {}, ",
            "\"consts\": [{}], \"successOn\": {}, \"result\": {}, \"free\": {} }}"
        ),
        pad,
        json_string(&function.alias),
        json_string(&function.name),
        json_string(&function.library),
        json_string(&function.symbol),
        params,
        json_string(&function.return_type),
        function.return_resource,
        abi_slots,
        json_string(&function.abi_return_name),
        json_string(&function.abi_return_ctype),
        consts,
        success_on,
        result,
        free
    )
}

fn link_expr_json(expr: &crate::ir::IrLinkExpr) -> String {
    match expr {
        crate::ir::IrLinkExpr::Var(name) => {
            format!(
                "{{ \"kind\": \"var\", \"name\": {} }}",
                crate::json_string(name)
            )
        }
        crate::ir::IrLinkExpr::Int(value) => {
            format!("{{ \"kind\": \"int\", \"value\": {value} }}")
        }
        crate::ir::IrLinkExpr::Compare { op, lhs, rhs } => format!(
            "{{ \"kind\": \"compare\", \"op\": {}, \"lhs\": {}, \"rhs\": {} }}",
            json_string(op),
            link_expr_json(lhs),
            link_expr_json(rhs)
        ),
        crate::ir::IrLinkExpr::And(lhs, rhs) => format!(
            "{{ \"kind\": \"and\", \"lhs\": {}, \"rhs\": {} }}",
            link_expr_json(lhs),
            link_expr_json(rhs)
        ),
        crate::ir::IrLinkExpr::Or(lhs, rhs) => format!(
            "{{ \"kind\": \"or\", \"lhs\": {}, \"rhs\": {} }}",
            link_expr_json(lhs),
            link_expr_json(rhs)
        ),
        crate::ir::IrLinkExpr::Not(operand) => {
            format!(
                "{{ \"kind\": \"not\", \"operand\": {} }}",
                link_expr_json(operand)
            )
        }
    }
}

impl ToNirJson for NirParam {
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

impl ToNirJson for NirOp {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self {
            NirOp::Bind {
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
            NirOp::Assign { name, value } => format!(
                "\n{}{{ \"op\": \"assign\", \"name\": {}, \"value\": {} }}",
                pad,
                json_string(name),
                value.to_json(indent)
            ),
            NirOp::StateAssign { resource, value } => format!(
                "\n{}{{ \"op\": \"stateAssign\", \"resource\": {}, \"value\": {} }}",
                pad,
                json_string(resource),
                value.to_json(indent)
            ),
            NirOp::StoreGlobal { name, type_, value } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!(
                    "\n{}{{ \"op\": \"storeGlobal\", \"name\": {}, \"type\": {}, \"value\": {} }}",
                    pad,
                    json_string(name),
                    json_string(type_),
                    value
                )
            }
            NirOp::Return { value } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!("\n{}{{ \"op\": \"return\", \"value\": {} }}", pad, value)
            }
            NirOp::ExitLoop { kind } => {
                format!(
                    "\n{}{{ \"op\": \"exitLoop\", \"loop\": {} }}",
                    pad,
                    json_string(loop_kind_name(*kind))
                )
            }
            NirOp::ContinueLoop { kind } => {
                format!(
                    "\n{}{{ \"op\": \"continueLoop\", \"loop\": {} }}",
                    pad,
                    json_string(loop_kind_name(*kind))
                )
            }
            NirOp::ExitProgram { code } => {
                format!(
                    "\n{}{{ \"op\": \"exitProgram\", \"code\": {} }}",
                    pad,
                    code.to_json(indent)
                )
            }
            NirOp::Fail { error } => {
                format!(
                    "\n{}{{ \"op\": \"fail\", \"error\": {} }}",
                    pad,
                    error.to_json(indent)
                )
            }
            NirOp::Eval { value } => format!(
                "\n{}{{ \"op\": \"eval\", \"value\": {} }}",
                pad,
                value.to_json(indent)
            ),
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => format!(
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
            ),
            NirOp::Match { value, cases } => format!(
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
            ),
            NirOp::While {
                kind,
                condition,
                body,
            } => format!(
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
            ),
            NirOp::For {
                name,
                type_,
                start,
                end,
                step,
                body,
                ..
            } => format!(
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
            ),
            NirOp::DoUntil { body, condition } => format!(
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
            ),
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => format!(
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
            ),
            NirOp::Trap { name, body } => format!(
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
            ),
        }
    }
}

impl ToNirJson for NirMatchCase {
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

impl ToNirJson for NirMatchPattern {
    fn to_json(&self, indent: usize) -> String {
        match self {
            NirMatchPattern::Else => "{ \"kind\": \"else\" }".to_string(),
            NirMatchPattern::Value(value) => {
                format!(
                    "{{ \"kind\": \"value\", \"value\": {} }}",
                    value.to_json(indent)
                )
            }
            NirMatchPattern::OneOf(values) => format!(
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

impl ToNirJson for NirValue {
    fn to_json(&self, _indent: usize) -> String {
        match self {
            NirValue::Const { type_, value } => format!(
                "{{ \"kind\": \"const\", \"type\": {}, \"value\": {} }}",
                json_string(type_),
                json_string(value)
            ),
            NirValue::Local(name) => {
                format!("{{ \"kind\": \"local\", \"name\": {} }}", json_string(name))
            }
            NirValue::LocalRef { name, type_ } => format!(
                "{{ \"kind\": \"localRef\", \"name\": {}, \"type\": {} }}",
                json_string(name),
                json_string(type_)
            ),
            NirValue::Global { name, type_ } => format!(
                "{{ \"kind\": \"global\", \"name\": {}, \"type\": {} }}",
                json_string(name),
                json_string(type_)
            ),
            NirValue::FunctionRef { name, type_ } => format!(
                "{{ \"kind\": \"functionRef\", \"name\": {}, \"type\": {} }}",
                json_string(name),
                json_string(type_)
            ),
            NirValue::Closure {
                name,
                type_,
                captures,
            } => format!(
                "{{ \"kind\": \"closure\", \"name\": {}, \"type\": {}, \"captures\": [{}] }}",
                json_string(name),
                json_string(type_),
                join_values(captures)
            ),
            NirValue::Capture {
                index,
                type_,
                by_ref,
            } => {
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
            NirValue::Call { target, args, .. } => format!(
                "{{ \"kind\": \"call\", \"target\": {}, \"args\": [{}] }}",
                json_string(target),
                join_values(args)
            ),
            NirValue::CallResult { target, args, .. } => format!(
                "{{ \"kind\": \"callResult\", \"target\": {}, \"args\": [{}] }}",
                json_string(target),
                join_values(args)
            ),
            NirValue::RuntimeCall {
                helper,
                target,
                args,
                ..
            } => format!(
                "{{ \"kind\": \"runtimeCall\", \"helper\": {}, \"target\": {}, \"args\": [{}] }}",
                json_string(helper.name()),
                json_string(target),
                join_values(args)
            ),
            NirValue::Constructor { type_, args } => format!(
                "{{ \"kind\": \"constructor\", \"type\": {}, \"args\": [{}] }}",
                json_string(type_),
                join_values(args)
            ),
            NirValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => format!(
                "{{ \"kind\": \"unionWrap\", \"union\": {}, \"member\": {}, \"value\": {} }}",
                json_string(union_type),
                json_string(member_type),
                value.to_json(0)
            ),
            NirValue::UnionExtract { type_, value } => format!(
                "{{ \"kind\": \"unionExtract\", \"type\": {}, \"value\": {} }}",
                json_string(type_),
                value.to_json(0)
            ),
            NirValue::ResultIsOk { value } => format!(
                "{{ \"kind\": \"resultIsOk\", \"value\": {} }}",
                value.to_json(0)
            ),
            NirValue::ResultValue { value } => format!(
                "{{ \"kind\": \"resultValue\", \"value\": {} }}",
                value.to_json(0)
            ),
            NirValue::ResultError { value } => format!(
                "{{ \"kind\": \"resultError\", \"value\": {} }}",
                value.to_json(0)
            ),
            NirValue::WithUpdate {
                type_,
                target,
                updates,
            } => format!(
                "{{ \"kind\": \"with\", \"type\": {}, \"target\": {}, \"updates\": [{}] }}",
                json_string(type_),
                target.to_json(0),
                updates
                    .iter()
                    .map(|update| update.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            NirValue::ListLiteral { type_, values } => format!(
                "{{ \"kind\": \"list\", \"type\": {}, \"values\": [{}] }}",
                json_string(type_),
                join_values(values)
            ),
            NirValue::MapLiteral { type_, entries } => {
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
            NirValue::MemberAccess { target, member } => format!(
                "{{ \"kind\": \"memberAccess\", \"target\": {}, \"member\": {} }}",
                target.to_json(0),
                json_string(member)
            ),
            NirValue::Binary {
                op, left, right, ..
            } => format!(
                "{{ \"kind\": \"binary\", \"op\": {}, \"left\": {}, \"right\": {} }}",
                json_string(op),
                left.to_json(0),
                right.to_json(0)
            ),
            NirValue::Unary { op, operand, .. } => format!(
                "{{ \"kind\": \"unary\", \"op\": {}, \"operand\": {} }}",
                json_string(op),
                operand.to_json(0)
            ),
        }
    }
}

impl ToNirJson for NirRecordUpdate {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"field\": {}, \"value\": {} }}",
            json_string(&self.field),
            self.value.to_json(0)
        )
    }
}

fn join_json<T: ToNirJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

fn loop_kind_name(kind: LoopKind) -> &'static str {
    match kind {
        LoopKind::For => "for",
        LoopKind::Do => "do",
        LoopKind::While => "while",
    }
}

fn join_values(values: &[NirValue]) -> String {
    values
        .iter()
        .map(|value| value.to_json(0))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_with_kind(kind: &str) -> NirType {
        NirType {
            kind: kind.to_string(),
            visibility: "public".to_string(),
            name: "Widget".to_string(),
            fields: vec![NirField {
                visibility: None,
                name: "id".to_string(),
                type_: "Integer".to_string(),
            }],
            includes: Vec::new(),
            variants: Vec::new(),
            members: Vec::new(),
        }
    }

    // bug-70: a crafted `.mfp` can carry a `record`/`resource` NIR type kind
    // (`validate_nir` whitelists both alongside `type`). The `-nir` dump must
    // render them instead of hitting the old `unreachable!("known NIR type
    // kind")`.
    #[test]
    fn renders_record_and_resource_kinds_without_panicking() {
        for kind in ["record", "resource"] {
            let json = type_with_kind(kind).to_json(2);
            assert!(
                json.contains(&format!("\"kind\": \"{kind}\"")),
                "expected kind '{kind}' in dump, got: {json}"
            );
            assert!(
                json.contains("\"name\": \"Widget\""),
                "expected the type name in dump, got: {json}"
            );
            assert!(
                json.contains("\"fields\""),
                "record/resource render with the field-carrying `type` shape: {json}"
            );
        }
    }

    // The valid-package shape (`type`) is unchanged — record/resource render
    // byte-identically to it apart from the kind string.
    #[test]
    fn record_kind_matches_type_shape() {
        let as_type = type_with_kind("type").to_json(2);
        let as_record = type_with_kind("record").to_json(2);
        assert_eq!(
            as_type.replace("\"kind\": \"type\"", "\"kind\": \"record\""),
            as_record
        );
    }
}
