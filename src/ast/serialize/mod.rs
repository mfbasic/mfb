use super::*;
use crate::json::{join_json, ToJson};

mod body;
mod decls;
mod link;

impl AstProject {
    pub fn to_json(&self) -> String {
        // The compiler-owned prelude is invisible to `-ast` output so golden AST
        // dumps reflect only user source.
        let files = self
            .files
            .iter()
            .filter(|file| {
                file.path != BUILTIN_PRELUDE_PATH
                    && file.path != crate::codegen::builtins::collections::SOURCE_PATH
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

/// JSON fragment appended to a binding/parameter/return for `RES` declarations.
/// Empty for non-resource declarations so ordinary `LET`/`MUT` output (and its
/// goldens) is unchanged.
/// The `"var"` field of a named `EXIT FOR` / `CONTINUE FOR`, written only when present so
/// a bare exit serializes exactly as before.
fn loop_var_json(target: &Option<String>) -> String {
    match target {
        Some(name) => format!(", \"var\": {}", json_string(name)),
        None => String::new(),
    }
}

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
        assert!(
            json.contains("\"kind\": \"bindInField\""),
            "bindInField: {json}"
        );
        assert!(
            json.contains("\"kind\": \"bindState\""),
            "bindState: {json}"
        );
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
