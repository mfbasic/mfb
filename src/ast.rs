use crate::json_string;
use crate::lexer::{self, Keyword, Token, TokenKind};
use crate::rules;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

#[derive(Clone, Debug)]
pub struct AstProject {
    pub name: String,
    pub files: Vec<AstFile>,
}

#[derive(Clone, Debug)]
pub struct AstFile {
    pub path: String,
    pub imports: Vec<Import>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct Import {
    pub module: String,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub enum Item {
    Function(Function),
    Type(TypeDecl),
}

#[derive(Clone, Debug)]
pub struct TypeDecl {
    pub kind: TypeDeclKind,
    pub visibility: Visibility,
    pub name: String,
    pub template_params: Vec<String>,
    pub fields: Vec<TypeField>,
    pub includes: Vec<String>,
    pub variants: Vec<UnionVariant>,
    pub members: Vec<EnumMember>,
    pub line: usize,
}

#[derive(Clone, Copy, Debug)]
pub enum TypeDeclKind {
    Type,
    Union,
    Enum,
}

#[derive(Clone, Debug)]
pub struct TypeField {
    pub visibility: Option<Visibility>,
    pub name: String,
    pub type_name: String,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct UnionVariant {
    pub name: String,
    pub fields: Vec<TypeField>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct EnumMember {
    pub name: String,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub kind: FunctionKind,
    pub visibility: Visibility,
    pub isolated: bool,
    pub name: String,
    pub template_params: Vec<String>,
    pub params: Vec<Param>,
    pub return_type: Option<String>,
    pub body: Vec<Statement>,
    pub trap: Option<Trap>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Trap {
    pub name: String,
    pub body: Vec<Statement>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub enum FunctionKind {
    Func,
    Sub,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Visibility {
    Private,
    Package,
    Export,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub type_name: Option<String>,
    pub default: Option<Expression>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub enum ConstructorArg {
    Positional(Expression),
    Named {
        name: String,
        value: Expression,
        line: usize,
    },
}

#[derive(Clone, Debug)]
pub struct RecordUpdate {
    pub field: String,
    pub value: Expression,
    pub line: usize,
}

#[derive(Clone, Debug)]
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
    Fail {
        error: Expression,
        line: usize,
    },
    Propagate {
        line: usize,
    },
    Recover {
        value: Expression,
        line: usize,
    },
    Assign {
        name: String,
        value: Expression,
        line: usize,
    },
    Expression {
        expression: Expression,
        line: usize,
    },
    If {
        condition: Expression,
        then_body: Vec<Statement>,
        else_body: Vec<Statement>,
        line: usize,
    },
    Match {
        expression: Expression,
        cases: Vec<MatchCase>,
        line: usize,
    },
    Using {
        name: String,
        value: Expression,
        body: Vec<Statement>,
        line: usize,
    },
}

#[derive(Clone, Debug)]
pub struct MatchCase {
    pub pattern: MatchPattern,
    pub body: Vec<Statement>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub enum MatchPattern {
    Else,
    Expression(Expression),
}

#[derive(Clone, Debug)]
pub enum Expression {
    String(String),
    Number(String),
    Boolean(bool),
    Binary {
        left: Box<Expression>,
        operator: String,
        right: Box<Expression>,
    },
    Unary {
        operator: String,
        operand: Box<Expression>,
    },
    Call {
        callee: String,
        arguments: Vec<Expression>,
    },
    Lambda {
        params: Vec<Param>,
        body: Box<Expression>,
    },
    Constructor {
        type_name: String,
        arguments: Vec<ConstructorArg>,
    },
    WithUpdate {
        target: Box<Expression>,
        updates: Vec<RecordUpdate>,
    },
    ListLiteral(Vec<Expression>),
    MapLiteral {
        key_type: String,
        value_type: String,
        entries: Vec<(Expression, Expression)>,
    },
    MemberAccess {
        target: Box<Expression>,
        member: String,
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

#[derive(Clone, Copy)]
enum BlockTerminator {
    Case,
    Else,
    ElseIf,
    EndIf,
    EndMatch,
    EndUsing,
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

            if self.check_top_level_item_start() {
                let visibility = self.parse_visibility().unwrap_or(Visibility::Private);
                if let Some(function) = self.parse_function() {
                    items.push(Item::Function(Function {
                        visibility,
                        ..function
                    }));
                }
                self.skip_separators();
                continue;
            }

            if self.check_top_level_type_start() {
                let visibility = self.parse_visibility().unwrap_or(Visibility::Private);
                if let Some(type_decl) = self.parse_type_decl() {
                    items.push(Item::Type(TypeDecl {
                        visibility,
                        ..type_decl
                    }));
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
        let isolated = self.match_keyword(Keyword::Isolated);
        let kind_token = self.advance().clone();
        let kind = if matches!(kind_token.kind, TokenKind::Keyword(Keyword::Sub)) {
            FunctionKind::Sub
        } else {
            FunctionKind::Func
        };
        if isolated && !matches!(kind, FunctionKind::Func) {
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "ISOLATED is valid only on FUNC declarations.",
                &kind_token,
            );
        }

        let Some(name) = self.consume_identifier("Function name must be an identifier.") else {
            self.synchronize();
            return None;
        };
        let template_params = self.parse_template_params();

        let params = if self.match_kind(TokenKind::LParen) {
            let params = self.parse_params();
            if !self.consume_kind(
                TokenKind::RParen,
                "Function declarations must close the parameter list.",
            ) {
                self.synchronize();
                return None;
            }
            params
        } else {
            Vec::new()
        };

        let return_type = if matches!(kind, FunctionKind::Func) && self.match_keyword(Keyword::As) {
            self.parse_type_name()
        } else {
            None
        };

        self.consume_statement_end("Expected end of function header.");
        self.skip_separators();

        let mut body = Vec::new();
        let mut trap = None;
        while !self.is_at_end() {
            if self.check_keyword(Keyword::Trap) {
                trap = self.parse_trap();
                self.skip_separators();
                continue;
            }
            if self.check_keyword(Keyword::End) {
                self.advance();
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
                    visibility: Visibility::Private,
                    isolated,
                    name,
                    template_params,
                    params,
                    return_type,
                    body,
                    trap,
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

    fn parse_trap(&mut self) -> Option<Trap> {
        let token = self.advance().clone();
        let Some(name) = self.consume_identifier("TRAP must bind an error identifier.") else {
            self.synchronize();
            return None;
        };
        self.consume_statement_end("Expected end of statement after TRAP header.");
        self.skip_separators();

        let mut body = Vec::new();
        while !self.is_at_end() && !self.is_end_block(Keyword::Trap) {
            if let Some(statement) = self.parse_statement() {
                body.push(statement);
            } else {
                self.synchronize();
            }
            self.skip_separators();
        }
        if !self.consume_end_block(Keyword::Trap, "TRAP block must end with END TRAP.") {
            return None;
        }
        Some(Trap {
            name,
            body,
            line: token.line,
        })
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
        let template_params = if matches!(kind, TypeDeclKind::Enum) {
            Vec::new()
        } else {
            self.parse_template_params()
        };

        let includes =
            if matches!(kind, TypeDeclKind::Union) && self.check_identifier_ci("INCLUDES") {
                self.advance();
                self.parse_union_includes()
            } else {
                Vec::new()
            };

        self.consume_statement_end("Expected end of type declaration header.");
        self.skip_separators();

        let mut fields = Vec::new();
        let mut variants = Vec::new();
        let mut members = Vec::new();
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
                    visibility: Visibility::Private,
                    name,
                    template_params,
                    fields,
                    includes,
                    variants,
                    members,
                    line: kind_token.line,
                });
            }

            match kind {
                TypeDeclKind::Type => {
                    if let Some(field) = self.parse_type_field() {
                        fields.push(field);
                    } else {
                        self.synchronize();
                    }
                }
                TypeDeclKind::Union => {
                    if let Some(variant) = self.parse_union_variant() {
                        variants.push(variant);
                    } else {
                        self.synchronize();
                    }
                }
                TypeDeclKind::Enum => {
                    let parsed = self.parse_enum_members();
                    if parsed.is_empty() {
                        self.synchronize();
                    }
                    members.extend(parsed);
                }
            }
            self.skip_separators();
        }

        self.report(
            "MFB_PARSE_UNTERMINATED_BLOCK",
            "Type block reached end-of-file before its END statement.",
            &kind_token,
        );
        None
    }

    fn parse_union_includes(&mut self) -> Vec<String> {
        let mut includes = Vec::new();
        loop {
            if let Some(name) = self.parse_qualified_name("Expected a union name after INCLUDES.") {
                includes.push(name);
            }
            if !self.match_kind(TokenKind::Comma) {
                break;
            }
        }
        includes
    }

    fn parse_template_params(&mut self) -> Vec<String> {
        if !self.check_identifier_ci("OF") {
            return Vec::new();
        }
        self.advance();
        let mut params = Vec::new();
        loop {
            if let Some(name) =
                self.consume_identifier("Expected template parameter name after OF.")
            {
                params.push(name);
            } else {
                self.synchronize();
                break;
            }
            if !self.match_kind(TokenKind::Comma) {
                break;
            }
        }
        params
    }

    fn parse_type_field(&mut self) -> Option<TypeField> {
        let line = self.peek().line;
        let visibility = self.parse_visibility();
        let name = self.consume_identifier("Field name must be an identifier.")?;
        if !self.consume_keyword(Keyword::As, "Field declarations must include an `AS` type.") {
            return None;
        }
        let type_name = self.parse_type_name()?;
        self.consume_statement_end("Expected end of statement after field declaration.");
        Some(TypeField {
            visibility,
            name,
            type_name,
            line,
        })
    }

    fn parse_union_variant(&mut self) -> Option<UnionVariant> {
        let line = self.peek().line;
        let name = self.consume_identifier("Union variant name must be an identifier.")?;
        let fields = if self.match_kind(TokenKind::LParen) {
            let fields = self.parse_variant_fields();
            if !self.consume_kind(
                TokenKind::RParen,
                "Union variant payload must close with `)`.",
            ) {
                return None;
            }
            fields
        } else {
            Vec::new()
        };
        self.consume_statement_end("Expected end of statement after union variant.");
        Some(UnionVariant { name, fields, line })
    }

    fn parse_variant_fields(&mut self) -> Vec<TypeField> {
        let mut fields = Vec::new();
        if self.check_kind(&TokenKind::RParen) {
            return fields;
        }

        loop {
            let line = self.peek().line;
            let Some(name) = self.consume_identifier("Variant field name must be an identifier.")
            else {
                self.synchronize();
                return fields;
            };
            if !self.consume_keyword(Keyword::As, "Variant fields must include an `AS` type.") {
                self.synchronize();
                return fields;
            }
            let Some(type_name) = self.parse_type_name() else {
                self.synchronize();
                return fields;
            };
            fields.push(TypeField {
                visibility: None,
                name,
                type_name,
                line,
            });
            if !self.match_kind(TokenKind::Comma) {
                break;
            }
        }

        fields
    }

    fn parse_enum_members(&mut self) -> Vec<EnumMember> {
        let mut members = Vec::new();
        loop {
            let line = self.peek().line;
            let Some(name) = self.consume_identifier("Enum member name must be an identifier.")
            else {
                break;
            };
            members.push(EnumMember { name, line });
            if !self.match_kind(TokenKind::Comma) {
                break;
            }
        }
        self.consume_statement_end("Expected end of statement after enum member declaration.");
        members
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
        if self.check_keyword(Keyword::If) {
            return self.parse_if_statement();
        }

        if self.check_keyword(Keyword::Match) {
            return self.parse_match_statement();
        }

        if self.check_keyword(Keyword::Using) {
            return self.parse_using_statement();
        }

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

        if self.match_keyword(Keyword::Fail) {
            let token = self.previous().clone();
            let error = self.parse_expression()?;
            self.consume_statement_end("Expected end of statement after FAIL.");
            return Some(Statement::Fail {
                error,
                line: token.line,
            });
        }

        if self.match_keyword(Keyword::Propagate) {
            let token = self.previous().clone();
            self.consume_statement_end("Expected end of statement after PROPAGATE.");
            return Some(Statement::Propagate { line: token.line });
        }

        if self.match_keyword(Keyword::Recover) {
            let token = self.previous().clone();
            let value = self.parse_expression()?;
            self.consume_statement_end("Expected end of statement after RECOVER.");
            return Some(Statement::Recover {
                value,
                line: token.line,
            });
        }

        if let TokenKind::Identifier(name) = self.peek().kind.clone() {
            if self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Equal))
            {
                let token = self.advance().clone();
                self.advance();
                let value = self.parse_expression()?;
                self.consume_statement_end("Expected end of statement after assignment.");
                return Some(Statement::Assign {
                    name,
                    value,
                    line: token.line,
                });
            }
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

    fn parse_if_statement(&mut self) -> Option<Statement> {
        let token = self.advance().clone();
        let condition = self.parse_expression()?;
        if !self.consume_keyword(Keyword::Then, "IF statements must include THEN.") {
            return None;
        }

        if !self.is_statement_end() {
            let then_body = vec![self.parse_statement()?];
            return Some(Statement::If {
                condition,
                then_body,
                else_body: Vec::new(),
                line: token.line,
            });
        }

        self.consume_statement_end("Expected end of statement after IF header.");
        self.skip_separators();
        let then_body = self.parse_statement_block(&[
            BlockTerminator::Else,
            BlockTerminator::ElseIf,
            BlockTerminator::EndIf,
        ]);
        let else_body = self.parse_if_tail();

        if !self.consume_end_block(Keyword::If, "IF block must end with END IF.") {
            return None;
        }

        Some(Statement::If {
            condition,
            then_body,
            else_body,
            line: token.line,
        })
    }

    fn parse_if_tail(&mut self) -> Vec<Statement> {
        if self.match_keyword(Keyword::Else) {
            self.consume_statement_end("Expected end of statement after ELSE.");
            self.skip_separators();
            return self.parse_statement_block(&[BlockTerminator::EndIf]);
        }

        if self.match_keyword(Keyword::ElseIf) {
            let token = self.previous().clone();
            let Some(condition) = self.parse_expression() else {
                return Vec::new();
            };
            if !self.consume_keyword(Keyword::Then, "ELSEIF clauses must include THEN.") {
                return Vec::new();
            }
            self.consume_statement_end("Expected end of statement after ELSEIF header.");
            self.skip_separators();
            let then_body = self.parse_statement_block(&[
                BlockTerminator::Else,
                BlockTerminator::ElseIf,
                BlockTerminator::EndIf,
            ]);
            let else_body = self.parse_if_tail();
            return vec![Statement::If {
                condition,
                then_body,
                else_body,
                line: token.line,
            }];
        }

        Vec::new()
    }

    fn parse_match_statement(&mut self) -> Option<Statement> {
        let token = self.advance().clone();
        let expression = self.parse_expression()?;
        self.consume_statement_end("Expected end of statement after MATCH expression.");
        self.skip_separators();

        let mut cases = Vec::new();
        while !self.is_at_end() && !self.is_end_block(Keyword::Match) {
            if !self.match_keyword(Keyword::Case) {
                let token = self.peek().clone();
                self.report(
                    "MFB_PARSE_UNEXPECTED_STATEMENT",
                    "MATCH blocks contain CASE clauses.",
                    &token,
                );
                self.synchronize();
                self.skip_separators();
                continue;
            }

            let case_token = self.previous().clone();
            let pattern = if self.match_keyword(Keyword::Else) {
                MatchPattern::Else
            } else {
                MatchPattern::Expression(self.parse_expression()?)
            };
            self.consume_statement_end("Expected end of statement after CASE pattern.");
            self.skip_separators();
            let body =
                self.parse_statement_block(&[BlockTerminator::Case, BlockTerminator::EndMatch]);
            cases.push(MatchCase {
                pattern,
                body,
                line: case_token.line,
            });
        }

        if !self.consume_end_block(Keyword::Match, "MATCH block must end with END MATCH.") {
            return None;
        }

        Some(Statement::Match {
            expression,
            cases,
            line: token.line,
        })
    }

    fn parse_using_statement(&mut self) -> Option<Statement> {
        let token = self.advance().clone();
        let name = self.consume_identifier("USING binding name must be an identifier.")?;
        if !self.consume_kind(TokenKind::Equal, "USING must bind a resource with `=`.") {
            return None;
        }
        let value = self.parse_expression()?;
        self.consume_statement_end("Expected end of statement after USING binding.");
        self.skip_separators();
        let body = self.parse_statement_block(&[BlockTerminator::EndUsing]);
        if !self.consume_end_block(Keyword::Using, "USING block must end with END USING.") {
            return None;
        }
        Some(Statement::Using {
            name,
            value,
            body,
            line: token.line,
        })
    }

    fn parse_statement_block(&mut self, terminators: &[BlockTerminator]) -> Vec<Statement> {
        let mut body = Vec::new();
        while !self.is_at_end() && !self.check_block_terminator(terminators) {
            if let Some(statement) = self.parse_statement() {
                body.push(statement);
            } else {
                self.synchronize();
            }
            self.skip_separators();
        }
        body
    }

    fn parse_expression(&mut self) -> Option<Expression> {
        self.parse_pipeline()
    }

    fn parse_pipeline(&mut self) -> Option<Expression> {
        let mut expression = self.parse_or()?;
        while self.match_kind(TokenKind::PipeGreater) {
            let token = self.previous().clone();
            let right = self.parse_or()?;
            if !contains_placeholder(&right) {
                self.report(
                    "MFB_PARSE_PIPELINE_PLACEHOLDER_MISSING",
                    "Pipeline right-hand side must contain `_` as the input placeholder.",
                    &token,
                );
                return None;
            }
            expression = substitute_placeholder(right, &expression);
        }
        Some(expression)
    }

    fn parse_or(&mut self) -> Option<Expression> {
        let mut expression = self.parse_and()?;
        while self.match_any_keywords(&[Keyword::Or, Keyword::Xor]) {
            let operator = match self.previous().kind {
                TokenKind::Keyword(Keyword::Or) => "OR",
                TokenKind::Keyword(Keyword::Xor) => "XOR",
                _ => unreachable!(),
            };
            let right = self.parse_and()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: operator.to_string(),
                right: Box::new(right),
            };
        }
        Some(expression)
    }

    fn parse_and(&mut self) -> Option<Expression> {
        let mut expression = self.parse_not()?;
        while self.match_keyword(Keyword::And) {
            let right = self.parse_not()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: "AND".to_string(),
                right: Box::new(right),
            };
        }
        Some(expression)
    }

    fn parse_not(&mut self) -> Option<Expression> {
        if self.match_keyword(Keyword::Not) {
            let operand = self.parse_not()?;
            return Some(Expression::Unary {
                operator: "NOT".to_string(),
                operand: Box::new(operand),
            });
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Option<Expression> {
        let mut expression = self.parse_concat()?;
        while self.match_any(&[
            TokenKind::Equal,
            TokenKind::NotEqual,
            TokenKind::Less,
            TokenKind::LessEqual,
            TokenKind::Greater,
            TokenKind::GreaterEqual,
        ]) {
            let operator = match self.previous().kind {
                TokenKind::Equal => "=",
                TokenKind::NotEqual => "<>",
                TokenKind::Less => "<",
                TokenKind::LessEqual => "<=",
                TokenKind::Greater => ">",
                TokenKind::GreaterEqual => ">=",
                _ => unreachable!(),
            };
            let right = self.parse_concat()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: operator.to_string(),
                right: Box::new(right),
            };
        }
        Some(expression)
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
        while self.match_any(&[TokenKind::Star, TokenKind::Slash])
            || self.match_any_keywords(&[Keyword::Mod, Keyword::Div])
        {
            let operator = match self.previous().kind {
                TokenKind::Star => "*",
                TokenKind::Slash => "/",
                TokenKind::Keyword(Keyword::Mod) => "MOD",
                TokenKind::Keyword(Keyword::Div) => "DIV",
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
        let mut expression = self.parse_unary()?;
        if self.match_kind(TokenKind::Caret) {
            let right = self.parse_power()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: "^".to_string(),
                right: Box::new(right),
            };
        }
        Some(expression)
    }

    fn parse_unary(&mut self) -> Option<Expression> {
        if self.match_kind(TokenKind::Minus) {
            let operand = self.parse_unary()?;
            return Some(Expression::Unary {
                operator: "-".to_string(),
                operand: Box::new(operand),
            });
        }
        if self.match_keyword(Keyword::With) {
            return self.parse_with_update();
        }
        self.parse_member_access()
    }

    fn parse_with_update(&mut self) -> Option<Expression> {
        let target = self.parse_member_access()?;
        if !self.consume_kind(TokenKind::LBrace, "Expected `{` after WITH target.") {
            return None;
        }
        let mut updates = Vec::new();
        if !self.check_kind(&TokenKind::RBrace) {
            loop {
                let line = self.peek().line;
                let Some(field) =
                    self.consume_identifier("WITH update field must be an identifier.")
                else {
                    self.synchronize();
                    return None;
                };
                if !self.consume_kind(
                    TokenKind::ColonEqual,
                    "Expected `:=` between WITH update field and value.",
                ) {
                    return None;
                }
                let value = self.parse_expression()?;
                updates.push(RecordUpdate { field, value, line });
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.consume_kind(TokenKind::RBrace, "Expected `}` after WITH updates.") {
            return None;
        }
        Some(Expression::WithUpdate {
            target: Box::new(target),
            updates,
        })
    }

    fn parse_member_access(&mut self) -> Option<Expression> {
        let mut expression = self.parse_call_or_constructor()?;
        while self.match_kind(TokenKind::Dot) {
            let member = self.consume_identifier("Expected identifier after `.`.")?;
            expression = Expression::MemberAccess {
                target: Box::new(expression),
                member,
            };
        }
        Some(expression)
    }

    fn parse_call_or_constructor(&mut self) -> Option<Expression> {
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
                let arguments = self.parse_argument_list(TokenKind::RParen)?;
                expression = Expression::Call { callee, arguments };
            } else if self.match_kind(TokenKind::LBracket) {
                let type_name = match expression {
                    Expression::Identifier(value) => value,
                    _ => {
                        let token = self.previous().clone();
                        self.report(
                            "MFB_PARSE_EXPECTED_EXPRESSION",
                            "Only identifiers can be used as constructors.",
                            &token,
                        );
                        return None;
                    }
                };
                let arguments = self.parse_constructor_argument_list(TokenKind::RBracket)?;
                expression = Expression::Constructor {
                    type_name,
                    arguments,
                };
            } else {
                break;
            }
        }
        Some(expression)
    }

    fn parse_argument_list(&mut self, closing: TokenKind) -> Option<Vec<Expression>> {
        let mut arguments = Vec::new();
        if !self.check_kind(&closing) {
            loop {
                arguments.push(self.parse_expression()?);
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        let detail = match closing {
            TokenKind::RParen => "Expected `)` after call arguments.",
            TokenKind::RBracket => "Expected `]` after constructor arguments.",
            _ => "Expected closing delimiter after arguments.",
        };
        if !self.consume_kind(closing, detail) {
            return None;
        }
        Some(arguments)
    }

    fn parse_constructor_argument_list(
        &mut self,
        closing: TokenKind,
    ) -> Option<Vec<ConstructorArg>> {
        let mut arguments = Vec::new();
        if !self.check_kind(&closing) {
            loop {
                if matches!(self.peek().kind, TokenKind::Identifier(_))
                    && self
                        .peek_next()
                        .is_some_and(|token| matches!(token.kind, TokenKind::ColonEqual))
                {
                    let line = self.peek().line;
                    let name =
                        self.consume_identifier("Constructor field name must be an identifier.")?;
                    self.consume_kind(
                        TokenKind::ColonEqual,
                        "Expected `:=` between constructor field and value.",
                    );
                    let value = self.parse_expression()?;
                    arguments.push(ConstructorArg::Named { name, value, line });
                } else {
                    arguments.push(ConstructorArg::Positional(self.parse_expression()?));
                }
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        let detail = match closing {
            TokenKind::RBracket => "Expected `]` after constructor arguments.",
            _ => "Expected closing delimiter after constructor arguments.",
        };
        if !self.consume_kind(closing, detail) {
            return None;
        }
        Some(arguments)
    }

    fn parse_primary(&mut self) -> Option<Expression> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::String(value) => Some(Expression::String(value)),
            TokenKind::Number(value) => Some(Expression::Number(value)),
            TokenKind::Keyword(Keyword::True) => Some(Expression::Boolean(true)),
            TokenKind::Keyword(Keyword::False) => Some(Expression::Boolean(false)),
            TokenKind::Keyword(Keyword::Nothing) => {
                Some(Expression::Identifier("NOTHING".to_string()))
            }
            TokenKind::Keyword(Keyword::Lambda) => self.parse_lambda(),
            TokenKind::Identifier(value) => {
                if value.eq_ignore_ascii_case("Map") && self.check_identifier_ci("OF") {
                    self.advance();
                    let key_type = self.parse_type_name()?;
                    if !self.check_identifier_ci("TO") {
                        let token = self.peek().clone();
                        self.report(
                            "MFB_PARSE_UNEXPECTED_TOKEN",
                            "Expected `TO` in map literal type.",
                            &token,
                        );
                        return None;
                    }
                    self.advance();
                    let value_type = self.parse_type_name()?;
                    return self.parse_map_literal(key_type, value_type);
                }
                let mut name = value;
                while self.match_kind(TokenKind::DoubleColon) {
                    let part = self.consume_identifier("Expected identifier after `::`.")?;
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
            TokenKind::LBracket => self.parse_list_literal(),
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
        while self.match_kind(TokenKind::DoubleColon) {
            let part = self.consume_identifier("Expected identifier after `::`.")?;
            name.push('.');
            name.push_str(&part);
        }
        Some(name)
    }

    fn parse_type_name(&mut self) -> Option<String> {
        if self.match_keyword(Keyword::Func) {
            return self.parse_function_type_name(false);
        }
        if self.match_keyword(Keyword::Isolated) {
            if self.consume_keyword(Keyword::Func, "ISOLATED type must be followed by FUNC.") {
                return self.parse_function_type_name(true);
            }
            return None;
        }
        let mut name = self.parse_type_base_name("Expected a type name.")?;
        if self.check_identifier_ci("OF") {
            self.advance();
            if name.eq_ignore_ascii_case("Map") || name.eq_ignore_ascii_case("Thread") {
                let first = self.parse_type_name()?;
                if !self.check_identifier_ci("TO") {
                    let token = self.peek().clone();
                    self.report(
                        "MFB_PARSE_UNEXPECTED_TOKEN",
                        if name.eq_ignore_ascii_case("Map") {
                            "Expected `TO` in map type."
                        } else {
                            "Expected `TO` in thread type."
                        },
                        &token,
                    );
                    return None;
                }
                self.advance();
                let second = self.parse_type_name()?;
                name.push_str(" OF ");
                name.push_str(&first);
                name.push_str(" TO ");
                name.push_str(&second);
                return Some(name);
            }

            let mut args = vec![self.parse_type_name()?];
            while self.match_kind(TokenKind::Comma) {
                args.push(self.parse_type_name()?);
            }
            if name.eq_ignore_ascii_case("List") || name.eq_ignore_ascii_case("Result") {
                if args.len() != 1 {
                    let token = self.peek().clone();
                    self.report(
                        "MFB_PARSE_UNEXPECTED_TOKEN",
                        "This built-in template accepts exactly one type argument.",
                        &token,
                    );
                    return None;
                }
            }
            if args.is_empty() {
                let token = self.peek().clone();
                self.report(
                    "MFB_PARSE_UNEXPECTED_TOKEN",
                    "Expected at least one template type argument.",
                    &token,
                );
                return None;
            }
            name.push_str(" OF ");
            name.push_str(&args.join(", "));
        }
        Some(name)
    }

    fn parse_function_type_name(&mut self, isolated: bool) -> Option<String> {
        if !self.consume_kind(TokenKind::LParen, "Function type must include `(`.") {
            return None;
        }
        let mut params = Vec::new();
        if !self.check_kind(&TokenKind::RParen) {
            loop {
                params.push(self.parse_type_name()?);
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.consume_kind(TokenKind::RParen, "Function type must close with `)`.") {
            return None;
        }
        if !self.consume_keyword(Keyword::As, "Function type must include `AS`.") {
            return None;
        }
        let returns = self.parse_type_name()?;
        Some(format!(
            "{}FUNC({}) AS {}",
            if isolated { "ISOLATED " } else { "" },
            params.join(", "),
            returns
        ))
    }

    fn parse_lambda(&mut self) -> Option<Expression> {
        if !self.consume_kind(TokenKind::LParen, "Lambda must include `(` after LAMBDA.") {
            return None;
        }
        let params = self.parse_params();
        if !self.consume_kind(TokenKind::RParen, "Lambda must close the parameter list.") {
            return None;
        }
        if !self.consume_kind(
            TokenKind::Arrow,
            "Lambda must include `->` before its body.",
        ) {
            return None;
        }
        let body = self.parse_expression()?;
        Some(Expression::Lambda {
            params,
            body: Box::new(body),
        })
    }

    fn parse_type_base_name(&mut self, detail: &str) -> Option<String> {
        let mut name = match self.peek().kind.clone() {
            TokenKind::Identifier(value) => {
                self.advance();
                value
            }
            TokenKind::Keyword(Keyword::Nothing) => {
                self.advance();
                "Nothing".to_string()
            }
            _ => {
                let token = self.peek().clone();
                self.report("MFB_PARSE_INVALID_IDENTIFIER", detail, &token);
                return None;
            }
        };
        while self.match_kind(TokenKind::DoubleColon) {
            let part = self.consume_identifier("Expected identifier after `::`.")?;
            name.push('.');
            name.push_str(&part);
        }
        Some(name)
    }

    fn parse_list_literal(&mut self) -> Option<Expression> {
        let mut values = Vec::new();
        if !self.check_kind(&TokenKind::RBracket) {
            loop {
                values.push(self.parse_expression()?);
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume_kind(TokenKind::RBracket, "Expected `]` after list literal.");
        Some(Expression::ListLiteral(values))
    }

    fn parse_map_literal(&mut self, key_type: String, value_type: String) -> Option<Expression> {
        if !self.consume_kind(TokenKind::LBrace, "Expected `{` after map literal type.") {
            return None;
        }
        let mut entries = Vec::new();
        if !self.check_kind(&TokenKind::RBrace) {
            loop {
                let key = self.parse_expression()?;
                if !self.consume_kind(
                    TokenKind::ColonEqual,
                    "Expected `:=` between map key and value.",
                ) {
                    return None;
                }
                let value = self.parse_expression()?;
                entries.push((key, value));
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume_kind(TokenKind::RBrace, "Expected `}` after map literal.");
        Some(Expression::MapLiteral {
            key_type,
            value_type,
            entries,
        })
    }

    fn parse_visibility(&mut self) -> Option<Visibility> {
        if self.match_keyword(Keyword::Private) {
            Some(Visibility::Private)
        } else if self.match_keyword(Keyword::Package) {
            Some(Visibility::Package)
        } else if self.match_keyword(Keyword::Export) {
            Some(Visibility::Export)
        } else {
            None
        }
    }

    fn check_top_level_item_start(&self) -> bool {
        self.check_keyword(Keyword::Sub)
            || self.check_keyword(Keyword::Func)
            || (self.check_keyword(Keyword::Isolated)
                && self
                    .tokens
                    .get(self.current + 1)
                    .is_some_and(|token| matches!(token.kind, TokenKind::Keyword(Keyword::Func))))
            || (self.check_visibility()
                && self.tokens.get(self.current + 1).is_some_and(|token| {
                    matches!(
                        token.kind,
                        TokenKind::Keyword(Keyword::Sub)
                            | TokenKind::Keyword(Keyword::Func)
                            | TokenKind::Keyword(Keyword::Isolated)
                    )
                }))
    }

    fn check_top_level_type_start(&self) -> bool {
        self.check_keyword(Keyword::Type)
            || self.check_keyword(Keyword::Union)
            || self.check_keyword(Keyword::Enum)
            || (self.check_visibility()
                && self.tokens.get(self.current + 1).is_some_and(|token| {
                    matches!(
                        token.kind,
                        TokenKind::Keyword(Keyword::Type)
                            | TokenKind::Keyword(Keyword::Union)
                            | TokenKind::Keyword(Keyword::Enum)
                    )
                }))
    }

    fn check_visibility(&self) -> bool {
        self.check_keyword(Keyword::Private)
            || self.check_keyword(Keyword::Package)
            || self.check_keyword(Keyword::Export)
    }

    fn check_block_terminator(&self, terminators: &[BlockTerminator]) -> bool {
        terminators.iter().any(|terminator| match terminator {
            BlockTerminator::Case => self.check_keyword(Keyword::Case),
            BlockTerminator::Else => self.check_keyword(Keyword::Else),
            BlockTerminator::ElseIf => self.check_keyword(Keyword::ElseIf),
            BlockTerminator::EndIf => self.is_end_block(Keyword::If),
            BlockTerminator::EndMatch => self.is_end_block(Keyword::Match),
            BlockTerminator::EndUsing => self.is_end_block(Keyword::Using),
        })
    }

    fn is_end_block(&self, keyword: Keyword) -> bool {
        self.check_keyword(Keyword::End)
            && self.tokens.get(self.current + 1).is_some_and(
                |token| matches!(token.kind, TokenKind::Keyword(current) if current == keyword),
            )
    }

    fn consume_end_block(&mut self, keyword: Keyword, detail: &str) -> bool {
        if !self.consume_keyword(Keyword::End, detail) {
            return false;
        }
        if !self.consume_keyword(keyword, "END must name the block kind it closes.") {
            return false;
        }
        self.consume_statement_end("Expected end of statement after END.")
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

    fn check_identifier_ci(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Identifier(value) if value.eq_ignore_ascii_case(expected))
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

    fn match_any_keywords(&mut self, keywords: &[Keyword]) -> bool {
        if keywords.iter().any(|keyword| self.check_keyword(*keyword)) {
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

    fn peek_next(&self) -> Option<&Token> {
        self.tokens.get(self.current + 1)
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
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"fields\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            self.line,
            pad,
            join_indented(&self.fields, indent + 2),
            pad,
            pad
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
                "{}  \"name\": {},\n",
                "{}",
                "{}  \"line\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"returnType\": {},\n",
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
            json_string(&self.name),
            template_params,
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
                format!(
                    "\n{}{{ \"kind\": \"recover\", \"value\": {}, \"line\": {} }}",
                    pad,
                    value.to_json(indent),
                    line
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
            Statement::Using {
                name,
                value,
                body,
                line,
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"kind\": \"using\",\n",
                        "{}  \"name\": {},\n",
                        "{}  \"value\": {},\n",
                        "{}  \"line\": {},\n",
                        "{}  \"body\": [{}\n{}  ]\n",
                        "{}}}"
                    ),
                    pad,
                    pad,
                    pad,
                    json_string(name),
                    pad,
                    value.to_json(0),
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
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"pattern\": {},\n",
                "{}  \"line\": {},\n",
                "{}  \"body\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            self.pattern.to_json(indent),
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
            MatchPattern::Expression(expression) => {
                format!(
                    "{{ \"kind\": \"expression\", \"expression\": {} }}",
                    expression.to_json(indent)
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
            Expression::Unary { operator, operand } => {
                format!(
                    "{{ \"kind\": \"unary\", \"operator\": {}, \"operand\": {} }}",
                    json_string(operator),
                    operand.to_json(0)
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
            Expression::Lambda { params, body } => {
                let params = params
                    .iter()
                    .map(|param| param.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"lambda\", \"params\": [{}], \"body\": {} }}",
                    params,
                    body.to_json(0)
                )
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
            Expression::Identifier(value) => {
                format!(
                    "{{ \"kind\": \"identifier\", \"value\": {} }}",
                    json_string(value)
                )
            }
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
        Visibility::Package => "package",
        Visibility::Export => "export",
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

fn contains_placeholder(expression: &Expression) -> bool {
    match expression {
        Expression::Identifier(value) => value == "_",
        Expression::Binary { left, right, .. } => {
            contains_placeholder(left) || contains_placeholder(right)
        }
        Expression::Unary { operand, .. } => contains_placeholder(operand),
        Expression::Call { arguments, .. } => arguments.iter().any(contains_placeholder),
        Expression::Constructor { arguments, .. } => {
            arguments.iter().any(constructor_arg_contains_placeholder)
        }
        Expression::Lambda { body, .. } => contains_placeholder(body),
        Expression::ListLiteral(values) => values.iter().any(contains_placeholder),
        Expression::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(key, value)| contains_placeholder(key) || contains_placeholder(value)),
        Expression::MemberAccess { target, .. } => contains_placeholder(target),
        Expression::WithUpdate { target, updates } => {
            contains_placeholder(target)
                || updates
                    .iter()
                    .any(|update| contains_placeholder(&update.value))
        }
        Expression::String(_) | Expression::Number(_) | Expression::Boolean(_) => false,
    }
}

fn constructor_arg_contains_placeholder(argument: &ConstructorArg) -> bool {
    match argument {
        ConstructorArg::Positional(value) => contains_placeholder(value),
        ConstructorArg::Named { value, .. } => contains_placeholder(value),
    }
}

fn substitute_placeholder(expression: Expression, input: &Expression) -> Expression {
    match expression {
        Expression::Identifier(value) if value == "_" => input.clone(),
        Expression::Binary {
            left,
            operator,
            right,
        } => Expression::Binary {
            left: Box::new(substitute_placeholder(*left, input)),
            operator,
            right: Box::new(substitute_placeholder(*right, input)),
        },
        Expression::Unary { operator, operand } => Expression::Unary {
            operator,
            operand: Box::new(substitute_placeholder(*operand, input)),
        },
        Expression::Call { callee, arguments } => Expression::Call {
            callee,
            arguments: arguments
                .into_iter()
                .map(|argument| substitute_placeholder(argument, input))
                .collect(),
        },
        Expression::Lambda { params, body } => Expression::Lambda {
            params,
            body: Box::new(substitute_placeholder(*body, input)),
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
