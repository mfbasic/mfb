//! plan-176-M: AST JSON serialization of the declarations — bindings, types, functions,
//! traps and parameters — and their `signature_line`s. The project/file entry points and
//! the shared helpers live in `ast/serialize/mod.rs`.

use super::*;

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
            json_string("func"),
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
            param_state_json_suffix(&self.state_type),
            default,
            self.line
        )
    }
}

/// A parameter's `STATE` clause, when it has one. There is no `resource` key: a
/// parameter carries no `RES` keyword (bug-976), and whether its type is a
/// resource is decided after the parse, so the AST has nothing to print for it.
fn param_state_json_suffix(state_type: &Option<String>) -> String {
    state_type
        .as_ref()
        .map(|value| format!(", \"state\": {}", json_string(value)))
        .unwrap_or_default()
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
        out.push_str("FUNC ");
        out.push_str(&self.name);
        out.push('(');
        let params = self
            .params
            .iter()
            .map(|param| {
                let mut text = String::new();
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
        let ret = self.return_type.as_deref().unwrap_or("Nothing");
        out.push_str(" AS ");
        out.push_str(ret);
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
