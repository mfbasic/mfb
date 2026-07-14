use super::*;

impl AstProject {
    pub fn to_json(&self) -> String {
        // The compiler-owned prelude is invisible to `-ast` output so golden AST
        // dumps reflect only user source.
        let files = self
            .files
            .iter()
            .filter(|file| {
                file.path != BUILTIN_PRELUDE_PATH
                    && file.path != crate::builtins::collections::SOURCE_PATH
            })
            .map(|file| file.to_json(2))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\n  \"project\": {},\n  \"files\": [{}\n  ]\n}}\n",
            json_string(&self.name),
            files
        )
    }
}

impl AstFile {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{\n{}  \"path\": {},\n{}  \"imports\": [{}\n{}  ],\n{}  \"items\": [{}\n{}  ]\n{}}}",
            pad,
            pad,
            json_string(&self.path),
            pad,
            join_indented(&self.imports, indent + 2),
            pad,
            pad,
            join_indented(&self.items, indent + 2),
            pad,
            pad
        )
    }
}

trait ToAstJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToAstJson for AstFile {
    fn to_json(&self, indent: usize) -> String {
        self.to_json(indent)
    }
}

impl ToAstJson for Import {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match &self.alias {
            Some(alias) => format!(
                "\n{}{{ \"module\": {}, \"alias\": {}, \"line\": {} }}",
                pad,
                json_string(&self.module),
                json_string(alias),
                self.line
            ),
            None => format!(
                "\n{}{{ \"module\": {}, \"line\": {} }}",
                pad,
                json_string(&self.module),
                self.line
            ),
        }
    }
}

impl ToAstJson for Item {
    fn to_json(&self, indent: usize) -> String {
        match self {
            Item::Binding(binding) => binding.to_json(indent),
            Item::Function(function) => function.to_json(indent),
            Item::Type(type_decl) => type_decl.to_json(indent),
            Item::Resource(resource) => resource.to_json(indent),
            Item::FuncAlias(alias) => alias.to_json(indent),
            Item::Link(link) => link.to_json(indent),
            Item::Doc(doc) => doc.to_json(indent),
            Item::Testing(testing) => testing.to_json(indent),
        }
    }
}

impl ToAstJson for TestingBlock {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"testing\",\n",
                "{}  \"line\": {},\n",
                "{}  \"groups\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            self.line,
            pad,
            join_indented(&self.groups, indent + 2),
            pad,
            pad
        )
    }
}

impl ToAstJson for TestGroup {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"description\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"members\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.description),
            pad,
            self.line,
            pad,
            join_indented(&self.members, indent + 2),
            pad,
            pad
        )
    }
}

impl ToAstJson for TestGroupMember {
    fn to_json(&self, indent: usize) -> String {
        match self {
            TestGroupMember::Case(case) => case.to_json(indent),
            TestGroupMember::Group(group) => group.to_json(indent),
        }
    }
}

impl ToAstJson for TestCase {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"description\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"body\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.description),
            pad,
            self.line,
            pad,
            join_indented(&self.body, indent + 2),
            pad,
            pad
        )
    }
}

impl ToAstJson for DocBlock {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let string_list = |values: &[String]| -> String {
            let inner = values
                .iter()
                .map(|value| json_string(value))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        };
        let named_list = |values: &[DocNamed]| -> String {
            let inner = values
                .iter()
                .map(|value| {
                    format!(
                        "{{ \"name\": {}, \"desc\": {} }}",
                        json_string(&value.name),
                        json_string(&value.desc)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        };
        let error_list = {
            let inner = self
                .errors
                .iter()
                .map(|value| {
                    format!(
                        "{{ \"code\": {}, \"desc\": {} }}",
                        json_string(&value.code),
                        json_string(&value.desc)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        };
        let prose_list = {
            let inner = self
                .desc
                .iter()
                .map(|prose| {
                    format!(
                        "{{ \"kind\": {}, \"text\": {} }}",
                        json_string(prose.kind.label()),
                        json_string(&prose.text)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        };
        let deprecated = self
            .deprecated
            .iter()
            .map(|(message, _)| message.clone())
            .collect::<Vec<_>>();
        let groups = self
            .groups
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        let rets = self
            .rets
            .iter()
            .map(|(text, _)| text.clone())
            .collect::<Vec<_>>();
        let examples = self
            .examples
            .iter()
            .map(|(text, _)| text.clone())
            .collect::<Vec<_>>();
        let signature = match &self.header_params {
            Some(params) => string_list(params),
            None => "null".to_string(),
        };
        format!(
            "\n{pad}{{ \"kind\": \"doc\", \"header\": {}, \"name\": {}, \"signature\": {}, \"attrs\": {}, \"desc\": {}, \"deprecated\": {}, \"group\": {}, \"args\": {}, \"ret\": {}, \"errors\": {}, \"props\": {}, \"example\": {}, \"line\": {} }}",
            json_string(self.header_kind.keyword()),
            json_string(&self.header_name),
            signature,
            string_list(&self.attrs),
            prose_list,
            string_list(&deprecated),
            string_list(&groups),
            named_list(&self.args),
            string_list(&rets),
            error_list,
            named_list(&self.props),
            string_list(&examples),
            self.line
        )
    }
}

impl ToAstJson for ResourceDecl {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"kind\": \"resource\", \"visibility\": {}, \"name\": {}, \"closeFn\": {}, \"threadSendable\": {}, \"line\": {} }}",
            pad,
            json_string(visibility_name(self.visibility)),
            json_string(&self.name),
            json_string(&self.close_fn),
            self.thread_sendable,
            self.line
        )
    }
}

impl ToAstJson for FuncAlias {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"kind\": \"funcAlias\", \"visibility\": {}, \"name\": {}, \"target\": {}, \"line\": {} }}",
            pad,
            json_string(visibility_name(self.visibility)),
            json_string(&self.name),
            json_string(&self.target),
            self.line
        )
    }
}

impl ToAstJson for LinkBlock {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"link\",\n",
                "{}  \"library\": {},\n",
                "{}  \"alias\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"functions\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            json_string(&self.library),
            pad,
            json_string(&self.alias),
            pad,
            self.line,
            pad,
            join_indented(&self.functions, indent + 2),
            pad,
            pad
        )
    }
}

impl ToAstJson for LinkFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let return_type = self
            .return_type
            .as_ref()
            .map(|value| json_string(value))
            .unwrap_or_else(|| "null".to_string());
        let success_on = self
            .success_on
            .as_ref()
            .map(|value| value.to_json(indent + 2))
            .unwrap_or_else(|| "null".to_string());
        let result = self
            .result
            .as_ref()
            .map(|value| value.to_json(indent + 2))
            .unwrap_or_else(|| "null".to_string());
        let free = self
            .free
            .as_ref()
            .map(|value| value.to_json(indent + 2))
            .unwrap_or_else(|| "null".to_string());
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"linkFunc\",\n",
                "{}  \"name\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"returnResource\": {},\n",
                "{}  \"returnType\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"abi\": {},\n",
                "{}  \"consts\": [{}\n{}  ],\n",
                "{}  \"successOn\": {},\n",
                "{}  \"result\": {},\n",
                "{}  \"free\": {}\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.symbol),
            pad,
            self.return_resource,
            pad,
            return_type,
            pad,
            self.line,
            pad,
            join_indented(&self.params, indent + 2),
            pad,
            pad,
            self.abi.to_json(indent + 2),
            pad,
            join_indented(&self.consts, indent + 2),
            pad,
            pad,
            success_on,
            pad,
            result,
            pad,
            free,
            pad
        )
    }
}

impl FreeSpec {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "{{\n",
                "{}  \"slot\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"paramName\": {},\n",
                "{}  \"paramCType\": {},\n",
                "{}  \"returnCType\": {}\n",
                "{}}}"
            ),
            pad,
            json_string(&self.slot),
            pad,
            json_string(&self.symbol),
            pad,
            json_string(&self.param_name),
            pad,
            json_string(&self.param_ctype),
            pad,
            json_string(&self.return_ctype),
            pad
        )
    }
}

impl AbiSpec {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "{{\n",
                "{}  \"slots\": [{}\n{}  ],\n",
                "{}  \"returnName\": {},\n",
                "{}  \"returnCType\": {}\n",
                "{}}}"
            ),
            pad,
            join_indented(&self.slots, indent + 2),
            pad,
            pad,
            json_string(&self.return_name),
            pad,
            json_string(&self.return_ctype),
            pad
        )
    }
}

impl ToAstJson for AbiSlot {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"ctype\": {}, \"out\": {}, \"line\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.ctype),
            self.is_out,
            self.line
        )
    }
}

impl ToAstJson for ConstPin {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"slot\": {}, \"value\": {}, \"line\": {} }}",
            pad,
            json_string(&self.slot),
            self.value.to_json(indent),
            self.line
        )
    }
}

impl ToAstJson for TopLevelBinding {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let type_name = self
            .type_name
            .as_ref()
            .map(|value| json_string(value))
            .unwrap_or_else(|| "null".to_string());
        let value = self
            .value
            .as_ref()
            .map(|value| value.to_json(indent))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"kind\": \"binding\", \"visibility\": {}, \"mutable\": {}{}, \"name\": {}, \"type\": {}, \"value\": {}, \"line\": {} }}",
            pad,
            json_string(visibility_name(self.visibility)),
            self.mutable,
            resource_json_suffix(self.resource, &self.state_type),
            json_string(&self.name),
            type_name,
            value,
            self.line
        )
    }
}

impl ToAstJson for TypeDecl {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let kind = match self.kind {
            TypeDeclKind::Type => "type",
            TypeDeclKind::Union => "union",
            TypeDeclKind::Enum => "enum",
        };
        let template_params = template_params_json(&self.template_params, indent);
        match self.kind {
            TypeDeclKind::Type => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"kind\": {},\n",
                    "{}  \"visibility\": {},\n",
                    "{}  \"name\": {},\n",
                    "{}",
                    "{}  \"line\": {},\n",
                    "{}  \"fields\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                json_string(kind),
                pad,
                json_string(visibility_name(self.visibility)),
                pad,
                json_string(&self.name),
                template_params,
                pad,
                self.line,
                pad,
                join_indented(&self.fields, indent + 2),
                pad,
                pad
            ),
            TypeDeclKind::Union => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"kind\": {},\n",
                    "{}  \"visibility\": {},\n",
                    "{}  \"name\": {},\n",
                    "{}",
                    "{}  \"line\": {},\n",
                    "{}  \"includes\": [{}],\n",
                    "{}  \"variants\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                json_string(kind),
                pad,
                json_string(visibility_name(self.visibility)),
                pad,
                json_string(&self.name),
                template_params,
                pad,
                self.line,
                pad,
                self.includes
                    .iter()
                    .map(|value| json_string(value))
                    .collect::<Vec<_>>()
                    .join(", "),
                pad,
                join_indented(&self.variants, indent + 2),
                pad,
                pad
            ),
            TypeDeclKind::Enum => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"kind\": {},\n",
                    "{}  \"visibility\": {},\n",
                    "{}  \"name\": {},\n",
                    "{}",
                    "{}  \"line\": {},\n",
                    "{}  \"members\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                json_string(kind),
                pad,
                json_string(visibility_name(self.visibility)),
                pad,
                json_string(&self.name),
                template_params,
                pad,
                self.line,
                pad,
                join_indented(&self.members, indent + 2),
                pad,
                pad
            ),
        }
    }
}

impl ToAstJson for TypeField {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let visibility = self
            .visibility
            .map(visibility_name)
            .map(json_string)
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"visibility\": {}, \"name\": {}, \"type\": {}, \"line\": {} }}",
            pad,
            visibility,
            json_string(&self.name),
            json_string(&self.type_name),
            self.line
        )
    }
}

impl ToAstJson for UnionVariant {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"line\": {} }}",
            pad,
            json_string(&self.name),
            self.line
        )
    }
}

impl ToAstJson for EnumMember {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"line\": {} }}",
            pad,
            json_string(&self.name),
            self.line
        )
    }
}

impl ToAstJson for Function {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let return_type = self
            .return_type
            .as_ref()
            .map(|value| json_string(value))
            .unwrap_or_else(|| "null".to_string());
        let return_suffix = if self.return_resource {
            let state = self
                .return_state_type
                .as_ref()
                .map(|value| json_string(value))
                .unwrap_or_else(|| "null".to_string());
            format!(", \"returnResource\": true, \"returnState\": {state}")
        } else {
            String::new()
        };
        let trap = self
            .trap
            .as_ref()
            .map(|trap| format!(",\n{}  \"trap\": {}", pad, trap.to_json(indent)))
            .unwrap_or_default();
        let template_params = template_params_json(&self.template_params, indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": {},\n",
                "{}  \"visibility\": {},\n",
                "{}  \"isolated\": {},\n",
                "{}  \"name\": {},\n",
                "{}",
                "{}  \"line\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"returnType\": {}{},\n",
                "{}  \"body\": [{}\n{}  ]{}",
                "\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(match self.kind {
                FunctionKind::Func => "func",
                FunctionKind::Sub => "sub",
            }),
            pad,
            json_string(visibility_name(self.visibility)),
            pad,
            self.isolated,
            pad,
            json_string(&self.name),
            template_params,
            pad,
            self.line,
            pad,
            join_indented(&self.params, indent + 2),
            pad,
            pad,
            return_type,
            return_suffix,
            pad,
            join_indented(&self.body, indent + 2),
            pad,
            trap,
            pad
        )
    }
}

impl ToAstJson for Trap {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "{{\n",
                "{}  \"name\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"body\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            json_string(&self.name),
            pad,
            self.line,
            pad,
            join_indented(&self.body, indent + 2),
            pad,
            pad
        )
    }
}

impl ToAstJson for Param {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let type_name = self
            .type_name
            .as_ref()
            .map(|value| json_string(value))
            .unwrap_or_else(|| "null".to_string());
        let default = self
            .default
            .as_ref()
            .map(|value| value.to_json(indent))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"name\": {}, \"type\": {}{}, \"default\": {}, \"line\": {} }}",
            pad,
            json_string(&self.name),
            type_name,
            resource_json_suffix(self.resource, &self.state_type),
            default,
            self.line
        )
    }
}

impl ToAstJson for Statement {
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
            Statement::Exit { target, code, line } => {
                let code = code
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!(
                    "\n{}{{ \"kind\": \"exit\", \"target\": {}, \"code\": {}, \"line\": {} }}",
                    pad,
                    json_string(exit_target_name(*target)),
                    code,
                    line
                )
            }
            Statement::Continue { kind, line } => {
                format!(
                    "\n{}{{ \"kind\": \"continue\", \"loop\": {}, \"line\": {} }}",
                    pad,
                    json_string(loop_kind_name(*kind)),
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
                    join_indented(then_body, indent + 2),
                    pad,
                    pad,
                    join_indented(else_body, indent + 2),
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
                    join_indented(cases, indent + 2),
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
                    join_indented(body, indent + 2),
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
                    json_string(loop_kind_name(*kind)),
                    pad,
                    condition.to_json(0),
                    pad,
                    line,
                    pad,
                    join_indented(body, indent + 2),
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
                    join_indented(body, indent + 2),
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
                    join_indented(body, indent + 2),
                    pad,
                    pad
                )
            }
        }
    }
}

impl ToAstJson for MatchCase {
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
            join_indented(&self.body, indent + 2),
            pad,
            pad
        )
    }
}

impl ToAstJson for MatchPattern {
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

impl ToAstJson for Expression {
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
                    json_string(operator),
                    left.to_json(0),
                    right.to_json(0)
                )
            }
            Expression::Unary {
                operator, operand, ..
            } => {
                format!(
                    "{{ \"kind\": \"unary\", \"operator\": {}, \"operand\": {} }}",
                    json_string(operator),
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
                    join_indented(handler, indent + 2),
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

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Private => "private",
        Visibility::Public => "public",
        Visibility::Export => "export",
    }
}

/// Source-form visibility keyword prefix (with trailing space) used when
/// rendering a declaration signature for documentation.
fn visibility_prefix(visibility: Visibility) -> &'static str {
    match visibility {
        // `PUBLIC` is the default visibility, so it is omitted from rendered
        // source; the explicit non-default modifiers (`PRIVATE`, `EXPORT`) render.
        Visibility::Public => "",
        Visibility::Private => "PRIVATE ",
        Visibility::Export => "EXPORT ",
    }
}

impl Function {
    /// Render the declaration's source-form signature line for documentation
    /// output, e.g. `EXPORT FUNC f(a AS Integer) AS Nothing`.
    pub fn signature_line(&self) -> String {
        let mut out = String::new();
        out.push_str(visibility_prefix(self.visibility));
        if self.isolated {
            out.push_str("ISOLATED ");
        }
        out.push_str(match self.kind {
            FunctionKind::Func => "FUNC ",
            FunctionKind::Sub => "SUB ",
        });
        out.push_str(&self.name);
        out.push('(');
        let params = self
            .params
            .iter()
            .map(|param| {
                let mut text = String::new();
                if param.resource {
                    text.push_str("RES ");
                }
                text.push_str(&param.name);
                if let Some(type_name) = &param.type_name {
                    text.push_str(" AS ");
                    text.push_str(type_name);
                }
                text
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&params);
        out.push(')');
        if let FunctionKind::Func = self.kind {
            let ret = self.return_type.as_deref().unwrap_or("Nothing");
            out.push_str(" AS ");
            out.push_str(ret);
        }
        out
    }
}

impl TypeDecl {
    /// Render the declaration's source-form header line for documentation output,
    /// e.g. `EXPORT TYPE Column`.
    pub fn signature_line(&self) -> String {
        let keyword = match self.kind {
            TypeDeclKind::Type => "TYPE",
            TypeDeclKind::Union => "UNION",
            TypeDeclKind::Enum => "ENUM",
        };
        format!(
            "{}{keyword} {}",
            visibility_prefix(self.visibility),
            self.name
        )
    }
}

/// JSON fragment appended to a binding/parameter/return for `RES` declarations.
/// Empty for non-resource declarations so ordinary `LET`/`MUT` output (and its
/// goldens) is unchanged.
fn resource_json_suffix(resource: bool, state_type: &Option<String>) -> String {
    if !resource {
        return String::new();
    }
    let state = state_type
        .as_ref()
        .map(|value| json_string(value))
        .unwrap_or_else(|| "null".to_string());
    format!(", \"resource\": true, \"state\": {state}")
}

fn exit_target_name(target: ExitTarget) -> &'static str {
    match target {
        ExitTarget::For => "for",
        ExitTarget::Do => "do",
        ExitTarget::While => "while",
        ExitTarget::Sub => "sub",
        ExitTarget::Func => "func",
        ExitTarget::Program => "program",
    }
}

fn loop_kind_name(kind: LoopKind) -> &'static str {
    match kind {
        LoopKind::For => "for",
        LoopKind::Do => "do",
        LoopKind::While => "while",
    }
}

fn join_indented<T: ToAstJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

fn template_params_json(params: &[String], indent: usize) -> String {
    if params.is_empty() {
        return String::new();
    }
    let pad = " ".repeat(indent);
    format!(
        "{}  \"templateParams\": [{}],\n",
        pad,
        params
            .iter()
            .map(|param| json_string(param))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(super) fn contains_placeholder(expression: &Expression) -> bool {
    match expression {
        Expression::Identifier(value) => value == "_",
        Expression::Binary { left, right, .. } => {
            contains_placeholder(left) || contains_placeholder(right)
        }
        Expression::Unary { operand, .. } => contains_placeholder(operand),
        Expression::Call { arguments, .. } => arguments.iter().any(call_arg_contains_placeholder),
        Expression::Constructor { arguments, .. } => {
            arguments.iter().any(constructor_arg_contains_placeholder)
        }
        Expression::Lambda { body, .. } => contains_placeholder(body),
        Expression::ListLiteral(values) => values.iter().any(contains_placeholder),
        Expression::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(key, value)| contains_placeholder(key) || contains_placeholder(value)),
        Expression::MemberAccess { target, .. } => contains_placeholder(target),
        Expression::Trapped { expression, .. } => contains_placeholder(expression),
        Expression::WithUpdate { target, updates } => {
            contains_placeholder(target)
                || updates
                    .iter()
                    .any(|update| contains_placeholder(&update.value))
        }
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_) => false,
    }
}

fn constructor_arg_contains_placeholder(argument: &ConstructorArg) -> bool {
    match argument {
        ConstructorArg::Positional(value) => contains_placeholder(value),
        ConstructorArg::Named { value, .. } => contains_placeholder(value),
    }
}

fn call_arg_contains_placeholder(argument: &CallArg) -> bool {
    match argument {
        CallArg::Positional(value) => contains_placeholder(value),
        CallArg::Named { value, .. } => contains_placeholder(value),
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
