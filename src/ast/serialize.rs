use super::*;
use crate::json::{join_json, ToJson};

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
            join_json(&self.imports, indent + 2),
            pad,
            pad,
            join_json(&self.items, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for AstFile {
    fn to_json(&self, indent: usize) -> String {
        self.to_json(indent)
    }
}

impl ToJson for Import {
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

impl ToJson for Item {
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

impl ToJson for TestingBlock {
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
            join_json(&self.groups, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for TestGroup {
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
            join_json(&self.members, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for TestGroupMember {
    fn to_json(&self, indent: usize) -> String {
        match self {
            TestGroupMember::Case(case) => case.to_json(indent),
            TestGroupMember::Group(group) => group.to_json(indent),
        }
    }
}

impl ToJson for TestCase {
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
            join_json(&self.body, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for DocBlock {
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

impl ToJson for ResourceDecl {
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

impl ToJson for FuncAlias {
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

impl ToJson for LinkBlock {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"link\",\n",
                "{}  \"library\": {},\n",
                "{}  \"alias\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"cstructs\": [{}\n{}  ],\n",
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
            join_json(&self.cstructs, indent + 2),
            pad,
            pad,
            join_json(&self.functions, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for CStructDecl {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"cstruct\",\n",
                "{}  \"name\": {},\n",
                "{}  \"mapsTo\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"fields\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.maps_to),
            pad,
            self.line,
            pad,
            join_json(&self.fields, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for CStructField {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"ctype\": {},\n",
                "{}  \"line\": {}\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.ctype),
            pad,
            self.line,
            pad
        )
    }
}

/// bug-300 E3: `BIND IN` blocks and their fields were absent from the `-ast` dump
/// entirely, so a native FUNC carrying one dumped identically to one without.
impl ToJson for BindIn {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"bindIn\",\n",
                "{}  \"slot\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"fields\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            json_string(&self.slot),
            pad,
            self.line,
            pad,
            join_json(&self.fields, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for BindInField {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": \"bindInField\",\n",
                "{}  \"name\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"value\": {}\n",
                "{}}}"
            ),
            pad,
            pad,
            pad,
            json_string(&self.name),
            pad,
            self.line,
            pad,
            self.value.to_json(indent + 2),
            pad
        )
    }
}

impl BindState {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "{{\n",
                "{}  \"kind\": \"bindState\",\n",
                "{}  \"resourceSlot\": {},\n",
                "{}  \"structSlot\": {}\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.resource_slot),
            pad,
            json_string(&self.struct_slot),
            pad
        )
    }
}

impl ToJson for LinkFunction {
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
        // bug-300 E3: these three were omitted, so a native
        // `FUNC … AS RES SoundFile STATE FileInfo` with `BIND IN`/`BIND STATE`
        // dumped identically to one without them. `Function::to_json` already
        // emits `returnState`, so the LINK side was the asymmetric one.
        let return_state = self
            .return_state_type
            .as_ref()
            .map(|value| json_string(value))
            .unwrap_or_else(|| "null".to_string());
        let bind_state = self
            .bind_state
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
                "{}  \"returnState\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"abi\": {},\n",
                "{}  \"consts\": [{}\n{}  ],\n",
                "{}  \"bindIn\": [{}\n{}  ],\n",
                "{}  \"bindState\": {},\n",
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
            return_state,
            pad,
            self.line,
            pad,
            join_json(&self.params, indent + 2),
            pad,
            pad,
            self.abi.to_json(indent + 2),
            pad,
            join_json(&self.consts, indent + 2),
            pad,
            pad,
            join_json(&self.bind_in, indent + 2),
            pad,
            pad,
            bind_state,
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
            join_json(&self.slots, indent + 2),
            pad,
            pad,
            json_string(&self.return_name),
            pad,
            json_string(&self.return_ctype),
            pad
        )
    }
}

impl ToJson for AbiSlot {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"ctype\": {}, \"out\": {}, \"line\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.ctype),
            self.direction.writes_back(),
            self.line
        )
    }
}

impl ToJson for ConstPin {
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

impl ToJson for TopLevelBinding {
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

impl ToJson for TypeDecl {
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
                join_json(&self.fields, indent + 2),
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
                join_json(&self.variants, indent + 2),
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
                join_json(&self.members, indent + 2),
                pad,
                pad
            ),
        }
    }
}

impl ToJson for TypeField {
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

impl ToJson for UnionVariant {
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

impl ToJson for EnumMember {
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

impl ToJson for Function {
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
            join_json(&self.params, indent + 2),
            pad,
            pad,
            return_type,
            return_suffix,
            pad,
            join_json(&self.body, indent + 2),
            pad,
            trap,
            pad
        )
    }
}

impl ToJson for Trap {
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
            join_json(&self.body, indent + 2),
            pad,
            pad
        )
    }
}

impl ToJson for Param {
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

impl ResourceDecl {
    /// Render the declaration's source-form header line for documentation output,
    /// e.g. `EXPORT RESOURCE SoundFile CLOSE BY sndLink.closeFile`.
    ///
    /// The close op is part of the signature because it is the observable
    /// contract of the handle: it names what running the automatic drop actually
    /// calls, and a reader comparing two resources wants to see it.
    pub fn signature_line(&self) -> String {
        format!(
            "{}RESOURCE {} CLOSE BY {}",
            visibility_prefix(self.visibility),
            self.name,
            self.close_fn
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

#[cfg(test)]
mod tests {
    use crate::json::ToJson;

    fn project_json(src: &str) -> String {
        crate::testutil::project_from_src(src).to_json()
    }

    #[test]
    fn astfile_tojson_trait_delegates_to_inherent() {
        // The `impl ToJson for AstFile` (46-48) forwards to the inherent dumper; no
        // production caller reaches it (AstProject calls the inherent method), so
        // invoke the trait method explicitly.
        let file = crate::testutil::parse_file("FUNC f() AS Integer\n  RETURN 1\nEND FUNC\n");
        let json = ToJson::to_json(&file, 0);
        assert!(json.contains("\"path\""));
    }

    #[test]
    fn link_cstruct_bind_in_and_bind_state_serialize() {
        // A LINK with a CSTRUCT (+field) and a native FUNC carrying BIND IN (+field)
        // and BIND STATE drives the CStructDecl/CStructField/BindIn/BindInField/
        // BindState serializers (328-450), which are emitted only when present.
        let json = project_json(
            "LINK \"x\" AS l\n\
             \x20 CSTRUCT Foo AS Rec\n    a CInt32\n  END CSTRUCT\n\
             \x20 FUNC f() AS Integer\n\
             \x20   SYMBOL \"s\"\n\
             \x20   BIND IN slot\n      fld = 1\n    END BIND\n\
             \x20   BIND STATE handle = outbuf\n\
             \x20   ABI (slot CPtr) AS r CInt32\n\
             \x20 END FUNC\n\
             END LINK\n",
        );
        assert!(json.contains("\"kind\": \"cstruct\""), "cstruct: {json}");
        assert!(json.contains("\"ctype\""), "cstruct field: {json}");
        assert!(json.contains("\"kind\": \"bindIn\""), "bindIn: {json}");
        assert!(json.contains("\"kind\": \"bindInField\""), "bindInField: {json}");
        assert!(json.contains("\"kind\": \"bindState\""), "bindState: {json}");
    }

    #[test]
    fn scalar_literal_serializes() {
        // A backtick scalar literal drives the `Expression::Scalar` arm (1289-1290).
        let json = project_json("FUNC f() AS Integer\n  LET x = `A`\n  RETURN 0\nEND FUNC\n");
        assert!(json.contains("\"kind\": \"scalar\""), "{json}");
    }

    #[test]
    fn set_literal_serializes() {
        // A non-empty set literal drives the `Expression::SetLiteral` arm and its
        // element loop (1391-1401).
        let json = project_json(
            "FUNC f() AS Integer\n  LET s AS Set OF Integer = Set OF Integer { 1 }\n  RETURN 0\nEND FUNC\n",
        );
        assert!(json.contains("\"kind\": \"set\""), "{json}");
        assert!(json.contains("\"elementType\""), "{json}");
    }
}
