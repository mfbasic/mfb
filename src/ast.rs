use crate::json_string;
use crate::rules;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

#[derive(Debug)]
pub struct AstProject {
    pub name: String,
    pub files: Vec<AstFile>,
}

#[derive(Debug)]
pub struct AstFile {
    pub path: String,
    pub imports: Vec<Import>,
    pub items: Vec<Item>,
}

#[derive(Debug)]
pub struct Import {
    pub module: String,
    pub line: usize,
}

#[derive(Debug)]
pub enum Item {
    Function(Function),
}

#[derive(Debug)]
pub struct Function {
    pub kind: FunctionKind,
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<String>,
    pub body: Vec<Statement>,
    pub line: usize,
}

#[derive(Debug)]
pub enum FunctionKind {
    Func,
    Sub,
}

#[derive(Debug)]
pub struct Param {
    pub name: String,
    pub type_name: Option<String>,
    pub default: Option<Expression>,
}

#[derive(Debug)]
pub enum Statement {
    Let {
        mutable: bool,
        name: String,
        type_name: Option<String>,
        value: Option<Expression>,
        line: usize,
    },
    Return {
        value: Option<Expression>,
        line: usize,
    },
    Expression {
        expression: Expression,
        line: usize,
    },
}

#[derive(Debug)]
pub enum Expression {
    String(String),
    Number(String),
    Boolean(bool),
    Call {
        callee: String,
        arguments: Vec<Expression>,
    },
    Identifier(String),
    Raw(String),
}

pub fn parse_project(
    project_name: &str,
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Result<AstProject, ()> {
    let mut files = Vec::new();

    for source_root in source_roots(manifest) {
        let root = project_dir.join(&source_root);
        let source_files = collect_mfb_files(&root).map_err(|err| {
            rules::show_diagnostic(
                "MFB_SOURCE_READ_FAILED",
                &format!("Could not read source root `{}`: {err}", root.display()),
                &root,
                1,
                1,
                1,
            );
        })?;

        for source_file in source_files {
            files.push(parse_file(project_dir, &source_file)?);
        }
    }

    Ok(AstProject {
        name: project_name.to_string(),
        files,
    })
}

pub fn write_ast(project_dir: &Path, ast: &AstProject) -> Result<PathBuf, String> {
    let ast_path = project_dir.join(format!("{}.ast", ast.name));
    fs::write(&ast_path, ast.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", ast_path.display()))?;
    Ok(ast_path)
}

fn parse_file(project_dir: &Path, path: &Path) -> Result<AstFile, ()> {
    let contents = fs::read_to_string(path).map_err(|err| {
        rules::show_diagnostic("MFB_SOURCE_READ_FAILED", &err.to_string(), path, 1, 1, 1);
    })?;

    let mut parser = FileParser::new(path, &contents);
    let ast_file = parser.parse();
    if parser.had_error {
        return Err(());
    }

    let relative_path = path
        .strip_prefix(project_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    Ok(AstFile {
        path: relative_path,
        imports: ast_file.imports,
        items: ast_file.items,
    })
}

fn source_roots(manifest: &HashMap<String, JsonValue>) -> Vec<String> {
    manifest
        .get("sources")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(|source| source.get::<HashMap<String, JsonValue>>())
        .filter_map(|source| source.get("root"))
        .filter_map(|root| root.get::<String>())
        .cloned()
        .collect()
}

fn collect_mfb_files(root: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut files = Vec::new();

    if root.is_file() {
        if root.extension().and_then(|ext| ext.to_str()) == Some("mfb") {
            files.push(root.to_path_buf());
        }
        return Ok(files);
    }

    if !root.exists() {
        return Ok(files);
    }

    collect_mfb_files_inner(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_mfb_files_inner(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_mfb_files_inner(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("mfb") {
            files.push(path);
        }
    }

    Ok(())
}

struct ParsedFile {
    imports: Vec<Import>,
    items: Vec<Item>,
}

struct FileParser<'a> {
    path: &'a Path,
    lines: Vec<&'a str>,
    current: usize,
    had_error: bool,
}

impl<'a> FileParser<'a> {
    fn new(path: &'a Path, contents: &'a str) -> Self {
        Self {
            path,
            lines: contents.lines().collect(),
            current: 0,
            had_error: false,
        }
    }

    fn parse(&mut self) -> ParsedFile {
        let mut imports = Vec::new();
        let mut items = Vec::new();

        while self.current < self.lines.len() {
            let line_number = self.current + 1;
            let line = strip_comment(self.lines[self.current]).trim();
            if line.is_empty() {
                self.current += 1;
                continue;
            }

            if let Some(module) = parse_import(line) {
                imports.push(Import {
                    module,
                    line: line_number,
                });
                self.current += 1;
                continue;
            }

            if starts_with_keyword(line, "SUB") || starts_with_keyword(line, "FUNC") {
                if let Some(function) = self.parse_function(line, line_number) {
                    items.push(Item::Function(function));
                }
                continue;
            }

            self.report(
                "MFB_PARSE_UNEXPECTED_STATEMENT",
                "Expected an IMPORT, SUB, or FUNC declaration at the top level.",
                line_number,
                1,
                line.len().max(1),
            );
            self.current += 1;
        }

        ParsedFile { imports, items }
    }

    fn parse_function(&mut self, header: &str, line_number: usize) -> Option<Function> {
        let kind = if starts_with_keyword(header, "SUB") {
            FunctionKind::Sub
        } else {
            FunctionKind::Func
        };

        let header_after_keyword = header
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim())
            .unwrap_or("");

        let Some(open_paren) = header_after_keyword.find('(') else {
            self.report(
                "MFB_PARSE_INVALID_FUNCTION_HEADER",
                "Function declarations must include a parameter list.",
                line_number,
                1,
                header.len().max(1),
            );
            self.current += 1;
            return None;
        };

        let Some(close_paren) = header_after_keyword.rfind(')') else {
            self.report(
                "MFB_PARSE_INVALID_FUNCTION_HEADER",
                "Function declarations must close the parameter list.",
                line_number,
                1,
                header.len().max(1),
            );
            self.current += 1;
            return None;
        };

        let name = header_after_keyword[..open_paren].trim();
        if !is_identifier(name) {
            self.report(
                "MFB_PARSE_INVALID_IDENTIFIER",
                "Function name must be an identifier.",
                line_number,
                1,
                header.len().max(1),
            );
            self.current += 1;
            return None;
        }

        let params = parse_params(&header_after_keyword[open_paren + 1..close_paren]);
        let return_type = parse_return_type(&kind, &header_after_keyword[close_paren + 1..]);
        self.current += 1;

        let mut body = Vec::new();
        while self.current < self.lines.len() {
            let body_line_number = self.current + 1;
            let body_line = strip_comment(self.lines[self.current]).trim();
            if body_line.is_empty() {
                self.current += 1;
                continue;
            }

            let is_end = match kind {
                FunctionKind::Func => keyword_eq(body_line, "END FUNC"),
                FunctionKind::Sub => keyword_eq(body_line, "END SUB"),
            };

            if is_end {
                self.current += 1;
                return Some(Function {
                    kind,
                    name: name.to_string(),
                    params,
                    return_type,
                    body,
                    line: line_number,
                });
            }

            if let Some(statement) = parse_statement(body_line, body_line_number) {
                body.push(statement);
            } else {
                self.report(
                    "MFB_PARSE_UNEXPECTED_STATEMENT",
                    "Statement is not recognized by the current parser.",
                    body_line_number,
                    1,
                    body_line.len().max(1),
                );
            }
            self.current += 1;
        }

        self.report(
            "MFB_PARSE_UNTERMINATED_BLOCK",
            "Function block reached end-of-file before its END statement.",
            line_number,
            1,
            header.len().max(1),
        );
        None
    }

    fn report(&mut self, rule: &str, detail: &str, line: usize, start: usize, end: usize) {
        self.had_error = true;
        rules::show_diagnostic(rule, detail, self.path, line, start, end);
    }
}

fn parse_import(line: &str) -> Option<String> {
    if !starts_with_keyword(line, "IMPORT") {
        return None;
    }

    let module = line["IMPORT".len()..].trim();
    if module.is_empty() {
        None
    } else {
        Some(module.to_string())
    }
}

fn parse_params(input: &str) -> Vec<Param> {
    split_top_level(input, ',')
        .into_iter()
        .filter_map(|param| {
            let param = param.trim();
            if param.is_empty() {
                return None;
            }

            let (before_default, default) = param
                .split_once('=')
                .map(|(left, right)| (left.trim(), Some(parse_expression(right.trim()))))
                .unwrap_or((param, None));

            let (name, type_name) = split_keyword(before_default, "AS")
                .map(|(left, right)| (left.trim().to_string(), Some(right.trim().to_string())))
                .unwrap_or((before_default.trim().to_string(), None));

            Some(Param {
                name,
                type_name,
                default,
            })
        })
        .collect()
}

fn parse_return_type(kind: &FunctionKind, input: &str) -> Option<String> {
    if matches!(kind, FunctionKind::Sub) {
        return None;
    }

    split_keyword(input.trim(), "AS").map(|(_, right)| right.trim().to_string())
}

fn parse_statement(line: &str, line_number: usize) -> Option<Statement> {
    if starts_with_keyword(line, "LET") || starts_with_keyword(line, "MUT") {
        let mutable = starts_with_keyword(line, "MUT");
        let keyword_len = if mutable { "MUT".len() } else { "LET".len() };
        let rest = line[keyword_len..].trim();
        let (binding_part, value) = rest
            .split_once('=')
            .map(|(left, right)| (left.trim(), Some(parse_expression(right.trim()))))
            .unwrap_or((rest, None));
        let (name, type_name) = split_keyword(binding_part, "AS")
            .map(|(left, right)| (left.trim().to_string(), Some(right.trim().to_string())))
            .unwrap_or((binding_part.trim().to_string(), None));

        return Some(Statement::Let {
            mutable,
            name,
            type_name,
            value,
            line: line_number,
        });
    }

    if starts_with_keyword(line, "RETURN") {
        let rest = line["RETURN".len()..].trim();
        return Some(Statement::Return {
            value: (!rest.is_empty()).then(|| parse_expression(rest)),
            line: line_number,
        });
    }

    Some(Statement::Expression {
        expression: parse_expression(line),
        line: line_number,
    })
}

fn parse_expression(input: &str) -> Expression {
    let input = input.trim();

    if input.starts_with('"') && input.ends_with('"') && input.len() >= 2 {
        return Expression::String(unescape_basic_string(&input[1..input.len() - 1]));
    }

    if input.parse::<i64>().is_ok() || input.parse::<f64>().is_ok() {
        return Expression::Number(input.to_string());
    }

    if input.eq_ignore_ascii_case("TRUE") {
        return Expression::Boolean(true);
    }

    if input.eq_ignore_ascii_case("FALSE") {
        return Expression::Boolean(false);
    }

    if let Some((callee, args)) = parse_call(input) {
        return Expression::Call {
            callee,
            arguments: args.into_iter().map(|arg| parse_expression(&arg)).collect(),
        };
    }

    if is_identifier(input) || is_dotted_identifier(input) {
        return Expression::Identifier(input.to_string());
    }

    Expression::Raw(input.to_string())
}

fn parse_call(input: &str) -> Option<(String, Vec<String>)> {
    let open_paren = input.find('(')?;
    if !input.ends_with(')') {
        return None;
    }

    let callee = input[..open_paren].trim();
    if !is_identifier(callee) && !is_dotted_identifier(callee) {
        return None;
    }

    let args = split_top_level(&input[open_paren + 1..input.len() - 1], ',');
    Some((callee.to_string(), args))
}

fn split_top_level(input: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for ch in input.chars() {
        if in_string {
            current.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ch if ch == separator && depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }

    parts
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
        } else if ch == '\'' {
            return &line[..index];
        }
    }

    line
}

fn split_keyword<'a>(input: &'a str, keyword: &str) -> Option<(&'a str, &'a str)> {
    let needle = format!(" {keyword} ");
    let upper = input.to_ascii_uppercase();

    if upper.starts_with(&format!("{keyword} ")) {
        return Some(("", &input[keyword.len() + 1..]));
    }

    upper
        .find(&needle)
        .map(|index| (&input[..index], &input[index + needle.len()..]))
}

fn starts_with_keyword(line: &str, keyword: &str) -> bool {
    line.len() >= keyword.len()
        && line[..keyword.len()].eq_ignore_ascii_case(keyword)
        && line[keyword.len()..]
            .chars()
            .next()
            .map(|ch| ch.is_whitespace())
            .unwrap_or(true)
}

fn keyword_eq(line: &str, keyword: &str) -> bool {
    line.eq_ignore_ascii_case(keyword)
}

fn is_identifier(input: &str) -> bool {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_dotted_identifier(input: &str) -> bool {
    input.split('.').all(is_identifier)
}

fn unescape_basic_string(input: &str) -> String {
    input
        .replace("\\\"", "\"")
        .replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\\\", "\\")
}

impl AstProject {
    pub fn to_json(&self) -> String {
        format!(
            "{{\n  \"project\": {},\n  \"files\": [{}\n  ]\n}}\n",
            json_string(&self.name),
            join_indented(&self.files, 2)
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
        format!(
            "\n{}{{ \"module\": {}, \"line\": {} }}",
            pad,
            json_string(&self.module),
            self.line
        )
    }
}

impl ToAstJson for Item {
    fn to_json(&self, indent: usize) -> String {
        match self {
            Item::Function(function) => function.to_json(indent),
        }
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
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"kind\": {},\n",
                "{}  \"name\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"returnType\": {},\n",
                "{}  \"body\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(match self.kind {
                FunctionKind::Func => "func",
                FunctionKind::Sub => "sub",
            }),
            pad,
            json_string(&self.name),
            pad,
            self.line,
            pad,
            join_indented(&self.params, indent + 2),
            pad,
            pad,
            return_type,
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
            "\n{}{{ \"name\": {}, \"type\": {}, \"default\": {} }}",
            pad,
            json_string(&self.name),
            type_name,
            default
        )
    }
}

impl ToAstJson for Statement {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self {
            Statement::Let {
                mutable,
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
                    "\n{}{{ \"kind\": \"binding\", \"mutable\": {}, \"name\": {}, \"type\": {}, \"value\": {}, \"line\": {} }}",
                    pad,
                    mutable,
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
            Statement::Expression { expression, line } => {
                format!(
                    "\n{}{{ \"kind\": \"expression\", \"expression\": {}, \"line\": {} }}",
                    pad,
                    expression.to_json(indent),
                    line
                )
            }
        }
    }
}

impl ToAstJson for Expression {
    fn to_json(&self, _indent: usize) -> String {
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
            Expression::Boolean(value) => {
                format!("{{ \"kind\": \"boolean\", \"value\": {} }}", value)
            }
            Expression::Call { callee, arguments } => {
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
            Expression::Identifier(value) => {
                format!(
                    "{{ \"kind\": \"identifier\", \"value\": {} }}",
                    json_string(value)
                )
            }
            Expression::Raw(value) => {
                format!("{{ \"kind\": \"raw\", \"value\": {} }}", json_string(value))
            }
        }
    }
}

fn join_indented<T: ToAstJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}
