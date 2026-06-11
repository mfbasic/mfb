use crate::json_string;
use crate::lexer::{self, Keyword, Token, TokenKind};
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
    Type(TypeDecl),
}

#[derive(Debug)]
pub struct TypeDecl {
    pub kind: TypeDeclKind,
    pub name: String,
    pub line: usize,
}

#[derive(Debug)]
pub enum TypeDeclKind {
    Type,
    Union,
    Enum,
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

#[derive(Clone, Debug)]
pub enum FunctionKind {
    Func,
    Sub,
}

#[derive(Debug)]
pub struct Param {
    pub name: String,
    pub type_name: Option<String>,
    pub default: Option<Expression>,
    pub line: usize,
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
    Binary {
        left: Box<Expression>,
        operator: String,
        right: Box<Expression>,
    },
    Call {
        callee: String,
        arguments: Vec<Expression>,
    },
    Identifier(String),
}

pub fn parse_project(
    project_name: &str,
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Result<AstProject, ()> {
    let mut files = Vec::new();

    for source_root in source_roots(manifest) {
        let root = project_dir.join(&source_root);
        if !root.exists() {
            rules::show_diagnostic(
                "MFB_SOURCE_ROOT_MISSING",
                &format!("Source root `{}` does not exist.", root.display()),
                &root,
                1,
                1,
                1,
            );
            return Err(());
        }

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
        if source_files.is_empty() {
            rules::show_diagnostic(
                "MFB_SOURCE_EMPTY",
                &format!("Source root `{}` contains no .mfb files.", root.display()),
                &root,
                1,
                1,
                1,
            );
            return Err(());
        }

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

    let tokens = lexer::lex(path, &contents)?;
    let ast_file = FileParser::new(path, tokens).parse()?;

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
    tokens: Vec<Token>,
    current: usize,
    had_error: bool,
}

impl<'a> FileParser<'a> {
    fn new(path: &'a Path, tokens: Vec<Token>) -> Self {
        Self {
            path,
            tokens,
            current: 0,
            had_error: false,
        }
    }

    fn parse(&mut self) -> Result<ParsedFile, ()> {
        let mut imports = Vec::new();
        let mut items = Vec::new();
        self.skip_separators();

        while !self.is_at_end() {
            if self.match_keyword(Keyword::Import) {
                let import_token = self.previous().clone();
                let Some(module) = self.parse_qualified_name("Expected package name after IMPORT.")
                else {
                    self.synchronize();
                    self.skip_separators();
                    continue;
                };
                imports.push(Import {
                    module,
                    line: import_token.line,
                });
                self.consume_statement_end("Expected end of statement after IMPORT.");
                self.skip_separators();
                continue;
            }

            if self.check_keyword(Keyword::Sub) || self.check_keyword(Keyword::Func) {
                if let Some(function) = self.parse_function() {
                    items.push(Item::Function(function));
                }
                self.skip_separators();
                continue;
            }

            if self.check_keyword(Keyword::Type)
                || self.check_keyword(Keyword::Union)
                || self.check_keyword(Keyword::Enum)
            {
                if let Some(type_decl) = self.parse_type_decl() {
                    items.push(Item::Type(type_decl));
                }
                self.skip_separators();
                continue;
            }

            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_STATEMENT",
                "Expected an IMPORT, SUB, FUNC, TYPE, UNION, or ENUM declaration at the top level.",
                &token,
            );
            self.synchronize();
            self.skip_separators();
        }

        if self.had_error {
            Err(())
        } else {
            Ok(ParsedFile { imports, items })
        }
    }

    fn parse_function(&mut self) -> Option<Function> {
        let kind_token = self.advance().clone();
        let kind = if matches!(kind_token.kind, TokenKind::Keyword(Keyword::Sub)) {
            FunctionKind::Sub
        } else {
            FunctionKind::Func
        };

        let Some(name) = self.consume_identifier("Function name must be an identifier.") else {
            self.synchronize();
            return None;
        };

        if !self.consume_kind(
            TokenKind::LParen,
            "Function declarations must include a parameter list.",
        ) {
            self.synchronize();
            return None;
        }

        let params = self.parse_params();
        if !self.consume_kind(
            TokenKind::RParen,
            "Function declarations must close the parameter list.",
        ) {
            self.synchronize();
            return None;
        }

        let return_type = if matches!(kind, FunctionKind::Func) && self.match_keyword(Keyword::As) {
            self.parse_type_name()
        } else {
            None
        };

        self.consume_statement_end("Expected end of function header.");
        self.skip_separators();

        let mut body = Vec::new();
        while !self.is_at_end() {
            if self.match_keyword(Keyword::End) {
                let expected = match kind {
                    FunctionKind::Func => Keyword::Func,
                    FunctionKind::Sub => Keyword::Sub,
                };
                if !self.consume_keyword(expected, "END must name the block kind it closes.") {
                    self.synchronize();
                    return None;
                }
                self.consume_statement_end("Expected end of statement after END.");
                return Some(Function {
                    kind,
                    name,
                    params,
                    return_type,
                    body,
                    line: kind_token.line,
                });
            }

            if let Some(statement) = self.parse_statement() {
                body.push(statement);
            } else {
                self.synchronize();
            }
            self.skip_separators();
        }

        self.report(
            "MFB_PARSE_UNTERMINATED_BLOCK",
            "Function block reached end-of-file before its END statement.",
            &kind_token,
        );
        None
    }

    fn parse_type_decl(&mut self) -> Option<TypeDecl> {
        let kind_token = self.advance().clone();
        let (kind, end_keyword) = match kind_token.kind {
            TokenKind::Keyword(Keyword::Type) => (TypeDeclKind::Type, Keyword::Type),
            TokenKind::Keyword(Keyword::Union) => (TypeDeclKind::Union, Keyword::Union),
            TokenKind::Keyword(Keyword::Enum) => (TypeDeclKind::Enum, Keyword::Enum),
            _ => unreachable!(),
        };
        let Some(name) = self.consume_identifier("Type declaration name must be an identifier.")
        else {
            self.synchronize();
            return None;
        };

        self.consume_statement_end("Expected end of type declaration header.");
        self.skip_separators();

        while !self.is_at_end() {
            if self.match_keyword(Keyword::End) {
                if !self
                    .consume_keyword(end_keyword, "END must name the type block kind it closes.")
                {
                    self.synchronize();
                    return None;
                }
                self.consume_statement_end("Expected end of statement after END.");
                return Some(TypeDecl {
                    kind,
                    name,
                    line: kind_token.line,
                });
            }
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNSUPPORTED_TYPE_MEMBER",
                "Type, union, and enum member declarations are not implemented yet.",
                &token,
            );
            self.synchronize();
            self.skip_separators();
        }

        self.report(
            "MFB_PARSE_UNTERMINATED_BLOCK",
            "Type block reached end-of-file before its END statement.",
            &kind_token,
        );
        None
    }

    fn parse_params(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        if self.check_kind(&TokenKind::RParen) {
            return params;
        }

        loop {
            let line = self.peek().line;
            let Some(name) = self.consume_identifier("Parameter name must be an identifier.")
            else {
                self.synchronize();
                return params;
            };
            let type_name = if self.match_keyword(Keyword::As) {
                self.parse_type_name()
            } else {
                None
            };
            let default = if self.match_kind(TokenKind::Equal) {
                self.parse_expression()
            } else {
                None
            };
            params.push(Param {
                name,
                type_name,
                default,
                line,
            });
            if !self.match_kind(TokenKind::Comma) {
                break;
            }
        }

        params
    }

    fn parse_statement(&mut self) -> Option<Statement> {
        if self.check_keyword(Keyword::Let) || self.check_keyword(Keyword::Mut) {
            let keyword = self.advance().clone();
            let mutable = matches!(keyword.kind, TokenKind::Keyword(Keyword::Mut));
            let name = self.consume_identifier("Binding name must be an identifier.")?;
            let type_name = if self.match_keyword(Keyword::As) {
                self.parse_type_name()
            } else {
                None
            };
            let value = if self.match_kind(TokenKind::Equal) {
                self.parse_expression()
            } else {
                None
            };
            self.consume_statement_end("Expected end of statement after binding.");
            return Some(Statement::Let {
                mutable,
                name,
                type_name,
                value,
                line: keyword.line,
            });
        }

        if self.match_keyword(Keyword::Return) {
            let token = self.previous().clone();
            let value = if self.is_statement_end() {
                None
            } else {
                self.parse_expression()
            };
            self.consume_statement_end("Expected end of statement after RETURN.");
            return Some(Statement::Return {
                value,
                line: token.line,
            });
        }

        let token = self.peek().clone();
        let expression = self.parse_expression();
        if expression.is_none() {
            self.report(
                "MFB_PARSE_UNEXPECTED_STATEMENT",
                "Statement is not recognized by the current parser.",
                &token,
            );
            return None;
        }
        self.consume_statement_end("Expected end of statement after expression.");
        Some(Statement::Expression {
            expression: expression.expect("checked expression"),
            line: token.line,
        })
    }

    fn parse_expression(&mut self) -> Option<Expression> {
        self.parse_concat()
    }

    fn parse_concat(&mut self) -> Option<Expression> {
        let mut expression = self.parse_addition()?;
        while self.match_kind(TokenKind::Ampersand) {
            let right = self.parse_addition()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: "&".to_string(),
                right: Box::new(right),
            };
        }
        Some(expression)
    }

    fn parse_addition(&mut self) -> Option<Expression> {
        let mut expression = self.parse_multiplication()?;
        while self.match_any(&[TokenKind::Plus, TokenKind::Minus]) {
            let operator = match self.previous().kind {
                TokenKind::Plus => "+",
                TokenKind::Minus => "-",
                _ => unreachable!(),
            };
            let right = self.parse_multiplication()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: operator.to_string(),
                right: Box::new(right),
            };
        }
        Some(expression)
    }

    fn parse_multiplication(&mut self) -> Option<Expression> {
        let mut expression = self.parse_power()?;
        while self.match_any(&[TokenKind::Star, TokenKind::Slash]) {
            let operator = match self.previous().kind {
                TokenKind::Star => "*",
                TokenKind::Slash => "/",
                _ => unreachable!(),
            };
            let right = self.parse_power()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: operator.to_string(),
                right: Box::new(right),
            };
        }
        Some(expression)
    }

    fn parse_power(&mut self) -> Option<Expression> {
        let mut expression = self.parse_call()?;
        while self.match_kind(TokenKind::Caret) {
            let right = self.parse_call()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: "^".to_string(),
                right: Box::new(right),
            };
        }
        Some(expression)
    }

    fn parse_call(&mut self) -> Option<Expression> {
        let mut expression = self.parse_primary()?;
        loop {
            if self.match_kind(TokenKind::LParen) {
                let callee = match expression {
                    Expression::Identifier(value) => value,
                    _ => {
                        let token = self.previous().clone();
                        self.report(
                            "MFB_PARSE_EXPECTED_EXPRESSION",
                            "Only identifiers can be called by the current parser.",
                            &token,
                        );
                        return None;
                    }
                };
                let mut arguments = Vec::new();
                if !self.check_kind(&TokenKind::RParen) {
                    loop {
                        arguments.push(self.parse_expression()?);
                        if !self.match_kind(TokenKind::Comma) {
                            break;
                        }
                    }
                }
                if !self.consume_kind(TokenKind::RParen, "Expected `)` after call arguments.") {
                    return None;
                }
                expression = Expression::Call { callee, arguments };
            } else {
                break;
            }
        }
        Some(expression)
    }

    fn parse_primary(&mut self) -> Option<Expression> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::String(value) => Some(Expression::String(value)),
            TokenKind::Number(value) => Some(Expression::Number(value)),
            TokenKind::Keyword(Keyword::True) => Some(Expression::Boolean(true)),
            TokenKind::Keyword(Keyword::False) => Some(Expression::Boolean(false)),
            TokenKind::Identifier(value) => {
                let mut name = value;
                while self.match_kind(TokenKind::Dot) {
                    let part = self.consume_identifier("Expected identifier after `.`.")?;
                    name.push('.');
                    name.push_str(&part);
                }
                Some(Expression::Identifier(name))
            }
            TokenKind::LParen => {
                let expression = self.parse_expression();
                self.consume_kind(TokenKind::RParen, "Expected `)` after expression.");
                expression
            }
            _ => {
                self.report(
                    "MFB_PARSE_EXPECTED_EXPRESSION",
                    "Expected an expression.",
                    &token,
                );
                None
            }
        }
    }

    fn parse_qualified_name(&mut self, detail: &str) -> Option<String> {
        let mut name = self.consume_identifier(detail)?;
        while self.match_kind(TokenKind::Dot) {
            let part = self.consume_identifier("Expected identifier after `.`.")?;
            name.push('.');
            name.push_str(&part);
        }
        Some(name)
    }

    fn parse_type_name(&mut self) -> Option<String> {
        self.parse_qualified_name("Expected a type name.")
    }

    fn consume_identifier(&mut self, detail: &str) -> Option<String> {
        if let TokenKind::Identifier(value) = self.peek().kind.clone() {
            self.advance();
            Some(value)
        } else {
            let token = self.peek().clone();
            self.report("MFB_PARSE_INVALID_IDENTIFIER", detail, &token);
            None
        }
    }

    fn consume_keyword(&mut self, keyword: Keyword, detail: &str) -> bool {
        if self.match_keyword(keyword) {
            true
        } else {
            let token = self.peek().clone();
            self.report("MFB_PARSE_UNEXPECTED_TOKEN", detail, &token);
            false
        }
    }

    fn consume_kind(&mut self, kind: TokenKind, detail: &str) -> bool {
        if self.match_kind(kind) {
            true
        } else {
            let token = self.peek().clone();
            self.report("MFB_PARSE_UNEXPECTED_TOKEN", detail, &token);
            false
        }
    }

    fn consume_statement_end(&mut self, detail: &str) -> bool {
        if self.is_statement_end() {
            self.skip_separators();
            true
        } else {
            let token = self.peek().clone();
            self.report("MFB_PARSE_UNEXPECTED_TOKEN", detail, &token);
            false
        }
    }

    fn skip_separators(&mut self) {
        while self.match_any(&[TokenKind::Newline, TokenKind::Colon]) {}
    }

    fn synchronize(&mut self) {
        while !self.is_at_end() && !self.is_statement_end() {
            self.advance();
        }
    }

    fn is_statement_end(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::Newline | TokenKind::Colon | TokenKind::Eof
        )
    }

    fn match_keyword(&mut self, keyword: Keyword) -> bool {
        if self.check_keyword(keyword) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn check_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.peek().kind, TokenKind::Keyword(current) if current == keyword)
    }

    fn match_kind(&mut self, kind: TokenKind) -> bool {
        if self.check_kind(&kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn match_any(&mut self, kinds: &[TokenKind]) -> bool {
        if kinds.iter().any(|kind| self.check_kind(kind)) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn check_kind(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(kind)
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn report(&mut self, rule: &str, detail: &str, token: &Token) {
        self.had_error = true;
        rules::show_diagnostic(rule, detail, self.path, token.line, token.start, token.end);
    }
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
            Item::Type(type_decl) => type_decl.to_json(indent),
        }
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
        format!(
            "\n{}{{ \"kind\": {}, \"name\": {}, \"line\": {} }}",
            pad,
            json_string(kind),
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
            Expression::Binary {
                left,
                operator,
                right,
            } => {
                format!(
                    "{{ \"kind\": \"binary\", \"operator\": {}, \"left\": {}, \"right\": {} }}",
                    json_string(operator),
                    left.to_json(0),
                    right.to_json(0)
                )
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
