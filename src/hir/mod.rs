//! The High-level IR (HIR): a typed, name-resolved tree layer between the AST
//! and the IR (plan-102-C).
//!
//! Every HIR node **mirrors its `crate::ast` counterpart 1:1 in structure** —
//! same fields, same nesting, same source `line`/`column` — with a single,
//! systematic difference: every field that names a *type* in the source
//! language carries a resolved [`crate::types::ParameterType`] instead of the
//! AST's raw `String`/`Option<String>`. So the AST's `type_name: String`
//! becomes `type_: ParameterType`, and `return_type: Option<String>` becomes
//! `returns: ParameterType` (an absent annotation elaborates to
//! [`ParameterType::Unknown`], matching `ir::lower`).
//!
//! The `RES`/`STATE` *ownership axes* stay as sibling fields exactly as the AST
//! separates them — `resource: bool` and `state_type: Option<ParameterType>` —
//! so a HIR type field always carries the **bare** type; the resource/state
//! markers ride alongside it (never baked into the `ParameterType`).
//!
//! Nodes that carry no source-language type strings needing retyping — the
//! native-binding (`RESOURCE`/`FUNC alias`/`LINK`) and documentation/testing
//! blocks — are reused verbatim from `crate::ast` rather than re-mirrored, since
//! [`elaborate`] runs on concrete, post-monomorph code and does not retype them.
//!
//! [`elaborate`] performs a pure structural walk of an [`crate::ast::AstProject`],
//! building the mirrored HIR tree and calling [`ParameterType::parse`] on every
//! type string. The input is concrete (post-monomorph), so `parse` always yields
//! a concrete type — no generic-variable classification happens here (that is a
//! later plan). Name resolution beyond the structural copy is likewise deferred.

use crate::ast::{ExitTarget, FunctionKind, LoopKind, TypeDeclKind, Visibility};
use crate::types::ParameterType;

pub(crate) mod build;

/// A whole project — the elaborated mirror of [`crate::ast::AstProject`].
#[derive(Clone, Debug)]
pub(crate) struct HirProject {
    pub(crate) name: String,
    pub(crate) files: Vec<HirFile>,
}

/// One source file — the elaborated mirror of [`crate::ast::AstFile`].
#[derive(Clone, Debug)]
pub(crate) struct HirFile {
    pub(crate) path: String,
    /// Imports carry no type strings, so the AST node is reused verbatim.
    pub(crate) imports: Vec<crate::ast::Import>,
    pub(crate) items: Vec<HirItem>,
    pub(crate) internal: bool,
}

impl HirFile {
    /// The import binding-name → package-name map for this file, mirroring
    /// [`crate::ast::AstFile::import_bindings`] (the `imports` node is reused
    /// verbatim, so the computation is identical).
    pub(crate) fn import_bindings(&self) -> std::collections::HashMap<String, String> {
        self.imports
            .iter()
            .map(|import| {
                (
                    import.binding_name().to_string(),
                    import.package_name().to_string(),
                )
            })
            .collect()
    }
}

/// A top-level item — the elaborated mirror of [`crate::ast::Item`].
///
/// The native-binding and documentation/testing variants carry no source-language
/// type strings needing a [`ParameterType`], so they reuse the AST struct directly.
#[derive(Clone, Debug)]
pub(crate) enum HirItem {
    Binding(HirTopLevelBinding),
    Function(HirFunction),
    Type(HirTypeDecl),
    Resource(crate::ast::ResourceDecl),
    FuncAlias(crate::ast::FuncAlias),
    Link(crate::ast::LinkBlock),
    Doc(crate::ast::DocBlock),
    Testing(crate::ast::TestingBlock),
}

/// A top-level binding — the elaborated mirror of [`crate::ast::TopLevelBinding`].
#[derive(Clone, Debug)]
pub(crate) struct HirTopLevelBinding {
    pub(crate) visibility: Visibility,
    pub(crate) mutable: bool,
    /// Whether this binding was declared with `RES` (a uniquely-owned resource).
    pub(crate) resource: bool,
    /// The `STATE T` type attached to a `RES` binding, if any (bare, parsed).
    pub(crate) state_type: Option<ParameterType>,
    pub(crate) name: String,
    /// The bare declared type; [`ParameterType::Unknown`] when the source gave no
    /// `AS T` annotation.
    pub(crate) type_: ParameterType,
    /// Whether `type_` came from an explicit `AS T` annotation (`type_name.is_some()`
    /// in the AST), mirroring `IrBinding::explicit_type`.
    pub(crate) explicit_type: bool,
    pub(crate) value: Option<HirExpression>,
    pub(crate) line: usize,
}

/// A type declaration (`TYPE`/`UNION`/`ENUM`) — mirror of [`crate::ast::TypeDecl`].
#[derive(Clone, Debug)]
pub(crate) struct HirTypeDecl {
    pub(crate) kind: TypeDeclKind,
    pub(crate) visibility: Visibility,
    pub(crate) name: String,
    pub(crate) template_params: Vec<String>,
    pub(crate) fields: Vec<HirTypeField>,
    pub(crate) includes: Vec<String>,
    /// Union variants carry only a name, so the AST node is reused verbatim.
    pub(crate) variants: Vec<crate::ast::UnionVariant>,
    /// Enum members carry only a name, so the AST node is reused verbatim.
    pub(crate) members: Vec<crate::ast::EnumMember>,
    pub(crate) line: usize,
}

/// A record field — the elaborated mirror of [`crate::ast::TypeField`].
#[derive(Clone, Debug)]
pub(crate) struct HirTypeField {
    pub(crate) visibility: Option<Visibility>,
    pub(crate) name: String,
    pub(crate) type_: ParameterType,
    pub(crate) line: usize,
}

/// A function or sub — the elaborated mirror of [`crate::ast::Function`].
#[derive(Clone, Debug)]
pub(crate) struct HirFunction {
    pub(crate) kind: FunctionKind,
    pub(crate) visibility: Visibility,
    pub(crate) isolated: bool,
    pub(crate) name: String,
    pub(crate) template_params: Vec<String>,
    pub(crate) params: Vec<HirParam>,
    /// The bare return type; [`ParameterType::Unknown`] when the source gave no
    /// `AS T` annotation.
    pub(crate) returns: ParameterType,
    /// Whether the return type was declared with `RES` (returns a resource).
    pub(crate) return_resource: bool,
    /// The `STATE T` type attached to a `RES` return type, if any (bare, parsed).
    pub(crate) return_state_type: Option<ParameterType>,
    pub(crate) body: Vec<HirStatement>,
    pub(crate) trap: Option<HirTrap>,
    pub(crate) line: usize,
}

/// A function trap handler — the elaborated mirror of [`crate::ast::Trap`].
#[derive(Clone, Debug)]
pub(crate) struct HirTrap {
    pub(crate) name: String,
    pub(crate) body: Vec<HirStatement>,
    pub(crate) line: usize,
}

/// A function parameter — the elaborated mirror of [`crate::ast::Param`].
#[derive(Clone, Debug)]
pub(crate) struct HirParam {
    pub(crate) name: String,
    /// The bare parameter type; [`ParameterType::Unknown`] when unannotated.
    pub(crate) type_: ParameterType,
    /// Whether this parameter was declared with `RES` (a resource pointer).
    pub(crate) resource: bool,
    /// The `STATE T` type attached to a `RES` parameter, if any (bare, parsed).
    pub(crate) state_type: Option<ParameterType>,
    pub(crate) default: Option<HirExpression>,
    pub(crate) line: usize,
}

/// A call argument — the elaborated mirror of [`crate::ast::CallArg`].
#[derive(Clone, Debug)]
pub(crate) enum HirCallArg {
    Positional(HirExpression),
    Named {
        name: String,
        value: HirExpression,
        line: usize,
    },
}

/// A constructor argument — the elaborated mirror of [`crate::ast::ConstructorArg`].
#[derive(Clone, Debug)]
pub(crate) enum HirConstructorArg {
    Positional(HirExpression),
    Named {
        name: String,
        value: HirExpression,
        line: usize,
    },
}

/// A `WITH` record-update field — mirror of [`crate::ast::RecordUpdate`].
#[derive(Clone, Debug)]
pub(crate) struct HirRecordUpdate {
    pub(crate) field: String,
    pub(crate) value: HirExpression,
    pub(crate) line: usize,
}

/// A statement — the elaborated mirror of [`crate::ast::Statement`].
#[derive(Clone, Debug)]
pub(crate) enum HirStatement {
    Let {
        mutable: bool,
        /// Whether this binding was declared with `RES`.
        resource: bool,
        /// The `STATE T` type attached to a `RES` binding, if any (bare, parsed).
        state_type: Option<ParameterType>,
        name: String,
        /// The bare declared type; [`ParameterType::Unknown`] when unannotated.
        type_: ParameterType,
        /// Whether `type_` came from an explicit `AS T` annotation.
        explicit_type: bool,
        value: Option<HirExpression>,
        line: usize,
    },
    Return {
        value: Option<HirExpression>,
        line: usize,
    },
    Exit {
        target: ExitTarget,
        code: Option<HirExpression>,
        line: usize,
    },
    Continue {
        kind: LoopKind,
        line: usize,
    },
    Fail {
        error: HirExpression,
        line: usize,
    },
    Propagate {
        line: usize,
    },
    Recover {
        value: Option<HirExpression>,
        line: usize,
    },
    Assign {
        name: String,
        value: HirExpression,
        line: usize,
    },
    StateAssign {
        resource: String,
        value: HirExpression,
        line: usize,
    },
    Expression {
        expression: HirExpression,
        line: usize,
    },
    If {
        condition: HirExpression,
        then_body: Vec<HirStatement>,
        else_body: Vec<HirStatement>,
        line: usize,
    },
    Match {
        expression: HirExpression,
        cases: Vec<HirMatchCase>,
        line: usize,
    },
    For {
        name: String,
        start: HirExpression,
        end: HirExpression,
        step: Option<HirExpression>,
        body: Vec<HirStatement>,
        line: usize,
    },
    ForEach {
        name: String,
        iterable: HirExpression,
        body: Vec<HirStatement>,
        line: usize,
    },
    While {
        kind: LoopKind,
        condition: HirExpression,
        body: Vec<HirStatement>,
        line: usize,
    },
    DoUntil {
        body: Vec<HirStatement>,
        condition: HirExpression,
        line: usize,
    },
}

/// A `MATCH` case — the elaborated mirror of [`crate::ast::MatchCase`].
#[derive(Clone, Debug)]
pub(crate) struct HirMatchCase {
    pub(crate) pattern: HirMatchPattern,
    pub(crate) guard: Option<HirExpression>,
    pub(crate) body: Vec<HirStatement>,
    pub(crate) line: usize,
}

/// A `MATCH` pattern — the elaborated mirror of [`crate::ast::MatchPattern`].
#[derive(Clone, Debug)]
pub(crate) enum HirMatchPattern {
    Else,
    Literal(HirExpression),
    Union {
        /// The bare union-variant type named by the pattern.
        type_: ParameterType,
        binding: String,
    },
    OneOf(Vec<HirExpression>),
}

/// An expression — the elaborated mirror of [`crate::ast::Expression`].
#[derive(Clone, Debug)]
pub(crate) enum HirExpression {
    String(String),
    Number(String),
    Scalar(u32),
    Boolean(bool),
    Binary {
        left: Box<HirExpression>,
        operator: String,
        right: Box<HirExpression>,
        line: usize,
        column: usize,
    },
    Unary {
        operator: String,
        operand: Box<HirExpression>,
        line: usize,
        column: usize,
    },
    Call {
        callee: String,
        arguments: Vec<HirCallArg>,
        line: usize,
        column: usize,
    },
    Lambda {
        params: Vec<HirParam>,
        body: Box<HirExpression>,
        assign_target: Option<String>,
    },
    Constructor {
        /// The bare record/union type being constructed.
        type_: ParameterType,
        arguments: Vec<HirConstructorArg>,
    },
    WithUpdate {
        target: Box<HirExpression>,
        updates: Vec<HirRecordUpdate>,
    },
    ListLiteral(Vec<HirExpression>),
    SetLiteral {
        /// The bare element type of the set literal.
        element_type: ParameterType,
        elements: Vec<HirExpression>,
    },
    MapLiteral {
        /// The bare key type of the map literal.
        key_type: ParameterType,
        /// The bare value type of the map literal.
        value_type: ParameterType,
        entries: Vec<(HirExpression, HirExpression)>,
    },
    MemberAccess {
        target: Box<HirExpression>,
        member: String,
    },
    Trapped {
        expression: Box<HirExpression>,
        binding: String,
        handler: Vec<HirStatement>,
        line: usize,
    },
    Identifier(String),
}

// --- Elaboration ---------------------------------------------------------------

/// Parse a bare type string into a [`ParameterType`].
/// Parse a type name and classify any generic type variable from the enclosing
/// decl's `type_params` (plan-102-D). On concrete (post-monomorph) input `type_params`
/// is empty, so this is exactly `ParameterType::parse`.
fn parse_type(name: &str, type_params: &[String]) -> ParameterType {
    ParameterType::parse(name).with_vars(type_params)
}

/// Parse an optional type annotation, defaulting an absent one to
/// [`ParameterType::Unknown`] — the same rule `ir::lower` applies (`lower_param`
/// / `lower_binding`). Classifies generic type variables from `type_params`.
fn parse_optional_type(name: &Option<String>, type_params: &[String]) -> ParameterType {
    match name {
        Some(name) => ParameterType::parse(name).with_vars(type_params),
        None => ParameterType::Unknown,
    }
}

/// Parse an optional `STATE T` type, preserving the `Some`/`None` distinction.
fn parse_optional_state(name: &Option<String>, type_params: &[String]) -> Option<ParameterType> {
    name.as_deref()
        .map(|name| ParameterType::parse(name).with_vars(type_params))
}

/// Elaborate a concrete (post-monomorph) [`crate::ast::AstProject`] into a
/// [`HirProject`]: a pure structural walk that attaches a [`ParameterType`] to
/// every type field.
pub(crate) fn elaborate(project: &crate::ast::AstProject) -> HirProject {
    HirProject {
        name: project.name.clone(),
        files: project.files.iter().map(elaborate_file).collect(),
    }
}

pub(crate) fn elaborate_file(file: &crate::ast::AstFile) -> HirFile {
    HirFile {
        path: file.path.clone(),
        imports: file.imports.clone(),
        items: file.items.iter().map(elaborate_item).collect(),
        internal: file.internal,
    }
}

fn elaborate_item(item: &crate::ast::Item) -> HirItem {
    match item {
        crate::ast::Item::Binding(binding) => HirItem::Binding(elaborate_binding(binding)),
        crate::ast::Item::Function(function) => HirItem::Function(elaborate_function(function)),
        crate::ast::Item::Type(decl) => HirItem::Type(elaborate_type_decl(decl)),
        crate::ast::Item::Resource(decl) => HirItem::Resource(decl.clone()),
        crate::ast::Item::FuncAlias(alias) => HirItem::FuncAlias(alias.clone()),
        crate::ast::Item::Link(block) => HirItem::Link(block.clone()),
        crate::ast::Item::Doc(block) => HirItem::Doc(block.clone()),
        crate::ast::Item::Testing(block) => HirItem::Testing(block.clone()),
    }
}

fn elaborate_binding(binding: &crate::ast::TopLevelBinding) -> HirTopLevelBinding {
    // Top-level bindings are never generic, so no type variables are in scope.
    let no_vars: &[String] = &[];
    HirTopLevelBinding {
        visibility: binding.visibility,
        mutable: binding.mutable,
        resource: binding.resource,
        state_type: parse_optional_state(&binding.state_type, no_vars),
        name: binding.name.clone(),
        type_: parse_optional_type(&binding.type_name, no_vars),
        explicit_type: binding.type_name.is_some(),
        value: binding
            .value
            .as_ref()
            .map(|value| elaborate_expression(value, no_vars)),
        line: binding.line,
    }
}

fn elaborate_type_decl(decl: &crate::ast::TypeDecl) -> HirTypeDecl {
    let type_params = &decl.template_params;
    HirTypeDecl {
        kind: decl.kind,
        visibility: decl.visibility,
        name: decl.name.clone(),
        template_params: decl.template_params.clone(),
        fields: decl
            .fields
            .iter()
            .map(|field| elaborate_type_field(field, type_params))
            .collect(),
        includes: decl.includes.clone(),
        variants: decl.variants.clone(),
        members: decl.members.clone(),
        line: decl.line,
    }
}

fn elaborate_type_field(field: &crate::ast::TypeField, type_params: &[String]) -> HirTypeField {
    HirTypeField {
        visibility: field.visibility,
        name: field.name.clone(),
        type_: parse_type(&field.type_name, type_params),
        line: field.line,
    }
}

fn elaborate_function(function: &crate::ast::Function) -> HirFunction {
    let type_params = &function.template_params;
    HirFunction {
        kind: function.kind,
        visibility: function.visibility,
        isolated: function.isolated,
        name: function.name.clone(),
        template_params: function.template_params.clone(),
        params: function
            .params
            .iter()
            .map(|param| elaborate_param(param, type_params))
            .collect(),
        returns: parse_optional_type(&function.return_type, type_params),
        return_resource: function.return_resource,
        return_state_type: parse_optional_state(&function.return_state_type, type_params),
        body: function
            .body
            .iter()
            .map(|statement| elaborate_statement(statement, type_params))
            .collect(),
        trap: function
            .trap
            .as_ref()
            .map(|trap| elaborate_trap(trap, type_params)),
        line: function.line,
    }
}

fn elaborate_trap(trap: &crate::ast::Trap, type_params: &[String]) -> HirTrap {
    HirTrap {
        name: trap.name.clone(),
        body: trap
            .body
            .iter()
            .map(|statement| elaborate_statement(statement, type_params))
            .collect(),
        line: trap.line,
    }
}

fn elaborate_param(param: &crate::ast::Param, type_params: &[String]) -> HirParam {
    HirParam {
        name: param.name.clone(),
        type_: parse_optional_type(&param.type_name, type_params),
        resource: param.resource,
        state_type: parse_optional_state(&param.state_type, type_params),
        default: param
            .default
            .as_ref()
            .map(|value| elaborate_expression(value, type_params)),
        line: param.line,
    }
}

fn elaborate_statement(statement: &crate::ast::Statement, type_params: &[String]) -> HirStatement {
    use crate::ast::Statement;
    match statement {
        Statement::Let {
            mutable,
            resource,
            state_type,
            name,
            type_name,
            value,
            line,
        } => HirStatement::Let {
            mutable: *mutable,
            resource: *resource,
            state_type: parse_optional_state(state_type, type_params),
            name: name.clone(),
            type_: parse_optional_type(type_name, type_params),
            explicit_type: type_name.is_some(),
            value: value
                .as_ref()
                .map(|__v| elaborate_expression(__v, type_params)),
            line: *line,
        },
        Statement::Return { value, line } => HirStatement::Return {
            value: value
                .as_ref()
                .map(|__v| elaborate_expression(__v, type_params)),
            line: *line,
        },
        Statement::Exit { target, code, line } => HirStatement::Exit {
            target: *target,
            code: code
                .as_ref()
                .map(|__v| elaborate_expression(__v, type_params)),
            line: *line,
        },
        Statement::Continue { kind, line } => HirStatement::Continue {
            kind: *kind,
            line: *line,
        },
        Statement::Fail { error, line } => HirStatement::Fail {
            error: elaborate_expression(error, type_params),
            line: *line,
        },
        Statement::Propagate { line } => HirStatement::Propagate { line: *line },
        Statement::Recover { value, line } => HirStatement::Recover {
            value: value
                .as_ref()
                .map(|__v| elaborate_expression(__v, type_params)),
            line: *line,
        },
        Statement::Assign { name, value, line } => HirStatement::Assign {
            name: name.clone(),
            value: elaborate_expression(value, type_params),
            line: *line,
        },
        Statement::StateAssign {
            resource,
            value,
            line,
        } => HirStatement::StateAssign {
            resource: resource.clone(),
            value: elaborate_expression(value, type_params),
            line: *line,
        },
        Statement::Expression { expression, line } => HirStatement::Expression {
            expression: elaborate_expression(expression, type_params),
            line: *line,
        },
        Statement::If {
            condition,
            then_body,
            else_body,
            line,
        } => HirStatement::If {
            condition: elaborate_expression(condition, type_params),
            then_body: then_body
                .iter()
                .map(|__v| elaborate_statement(__v, type_params))
                .collect(),
            else_body: else_body
                .iter()
                .map(|__v| elaborate_statement(__v, type_params))
                .collect(),
            line: *line,
        },
        Statement::Match {
            expression,
            cases,
            line,
        } => HirStatement::Match {
            expression: elaborate_expression(expression, type_params),
            cases: cases
                .iter()
                .map(|__v| elaborate_match_case(__v, type_params))
                .collect(),
            line: *line,
        },
        Statement::For {
            name,
            start,
            end,
            step,
            body,
            line,
        } => HirStatement::For {
            name: name.clone(),
            start: elaborate_expression(start, type_params),
            end: elaborate_expression(end, type_params),
            step: step
                .as_ref()
                .map(|__v| elaborate_expression(__v, type_params)),
            body: body
                .iter()
                .map(|__v| elaborate_statement(__v, type_params))
                .collect(),
            line: *line,
        },
        Statement::ForEach {
            name,
            iterable,
            body,
            line,
        } => HirStatement::ForEach {
            name: name.clone(),
            iterable: elaborate_expression(iterable, type_params),
            body: body
                .iter()
                .map(|__v| elaborate_statement(__v, type_params))
                .collect(),
            line: *line,
        },
        Statement::While {
            kind,
            condition,
            body,
            line,
        } => HirStatement::While {
            kind: *kind,
            condition: elaborate_expression(condition, type_params),
            body: body
                .iter()
                .map(|__v| elaborate_statement(__v, type_params))
                .collect(),
            line: *line,
        },
        Statement::DoUntil {
            body,
            condition,
            line,
        } => HirStatement::DoUntil {
            body: body
                .iter()
                .map(|__v| elaborate_statement(__v, type_params))
                .collect(),
            condition: elaborate_expression(condition, type_params),
            line: *line,
        },
    }
}

fn elaborate_match_case(case: &crate::ast::MatchCase, type_params: &[String]) -> HirMatchCase {
    HirMatchCase {
        pattern: elaborate_match_pattern(&case.pattern, type_params),
        guard: case
            .guard
            .as_ref()
            .map(|__v| elaborate_expression(__v, type_params)),
        body: case
            .body
            .iter()
            .map(|__v| elaborate_statement(__v, type_params))
            .collect(),
        line: case.line,
    }
}

fn elaborate_match_pattern(
    pattern: &crate::ast::MatchPattern,
    type_params: &[String],
) -> HirMatchPattern {
    use crate::ast::MatchPattern;
    match pattern {
        MatchPattern::Else => HirMatchPattern::Else,
        MatchPattern::Literal(expr) => {
            HirMatchPattern::Literal(elaborate_expression(expr, type_params))
        }
        MatchPattern::Union { type_name, binding } => HirMatchPattern::Union {
            type_: parse_type(type_name, type_params),
            binding: binding.clone(),
        },
        MatchPattern::OneOf(exprs) => HirMatchPattern::OneOf(
            exprs
                .iter()
                .map(|__v| elaborate_expression(__v, type_params))
                .collect(),
        ),
    }
}

fn elaborate_call_arg(arg: &crate::ast::CallArg, type_params: &[String]) -> HirCallArg {
    use crate::ast::CallArg;
    match arg {
        CallArg::Positional(expr) => {
            HirCallArg::Positional(elaborate_expression(expr, type_params))
        }
        CallArg::Named { name, value, line } => HirCallArg::Named {
            name: name.clone(),
            value: elaborate_expression(value, type_params),
            line: *line,
        },
    }
}

fn elaborate_constructor_arg(
    arg: &crate::ast::ConstructorArg,
    type_params: &[String],
) -> HirConstructorArg {
    use crate::ast::ConstructorArg;
    match arg {
        ConstructorArg::Positional(expr) => {
            HirConstructorArg::Positional(elaborate_expression(expr, type_params))
        }
        ConstructorArg::Named { name, value, line } => HirConstructorArg::Named {
            name: name.clone(),
            value: elaborate_expression(value, type_params),
            line: *line,
        },
    }
}

fn elaborate_record_update(
    update: &crate::ast::RecordUpdate,
    type_params: &[String],
) -> HirRecordUpdate {
    HirRecordUpdate {
        field: update.field.clone(),
        value: elaborate_expression(&update.value, type_params),
        line: update.line,
    }
}

fn elaborate_expression(
    expression: &crate::ast::Expression,
    type_params: &[String],
) -> HirExpression {
    use crate::ast::Expression;
    match expression {
        Expression::String(value) => HirExpression::String(value.clone()),
        Expression::Number(value) => HirExpression::Number(value.clone()),
        Expression::Scalar(value) => HirExpression::Scalar(*value),
        Expression::Boolean(value) => HirExpression::Boolean(*value),
        Expression::Binary {
            left,
            operator,
            right,
            line,
            column,
        } => HirExpression::Binary {
            left: Box::new(elaborate_expression(left, type_params)),
            operator: operator.clone(),
            right: Box::new(elaborate_expression(right, type_params)),
            line: *line,
            column: *column,
        },
        Expression::Unary {
            operator,
            operand,
            line,
            column,
        } => HirExpression::Unary {
            operator: operator.clone(),
            operand: Box::new(elaborate_expression(operand, type_params)),
            line: *line,
            column: *column,
        },
        Expression::Call {
            callee,
            arguments,
            line,
            column,
        } => HirExpression::Call {
            callee: callee.clone(),
            arguments: arguments
                .iter()
                .map(|__v| elaborate_call_arg(__v, type_params))
                .collect(),
            line: *line,
            column: *column,
        },
        Expression::Lambda {
            params,
            body,
            assign_target,
        } => HirExpression::Lambda {
            params: params
                .iter()
                .map(|__p| elaborate_param(__p, type_params))
                .collect(),
            body: Box::new(elaborate_expression(body, type_params)),
            assign_target: assign_target.clone(),
        },
        Expression::Constructor {
            type_name,
            arguments,
        } => HirExpression::Constructor {
            type_: parse_type(type_name, type_params),
            arguments: arguments
                .iter()
                .map(|__v| elaborate_constructor_arg(__v, type_params))
                .collect(),
        },
        Expression::WithUpdate { target, updates } => HirExpression::WithUpdate {
            target: Box::new(elaborate_expression(target, type_params)),
            updates: updates
                .iter()
                .map(|__v| elaborate_record_update(__v, type_params))
                .collect(),
        },
        Expression::ListLiteral(elements) => HirExpression::ListLiteral(
            elements
                .iter()
                .map(|__v| elaborate_expression(__v, type_params))
                .collect(),
        ),
        Expression::SetLiteral {
            element_type,
            elements,
        } => HirExpression::SetLiteral {
            element_type: parse_type(element_type, type_params),
            elements: elements
                .iter()
                .map(|__v| elaborate_expression(__v, type_params))
                .collect(),
        },
        Expression::MapLiteral {
            key_type,
            value_type,
            entries,
        } => HirExpression::MapLiteral {
            key_type: parse_type(key_type, type_params),
            value_type: parse_type(value_type, type_params),
            entries: entries
                .iter()
                .map(|(key, value)| {
                    (
                        elaborate_expression(key, type_params),
                        elaborate_expression(value, type_params),
                    )
                })
                .collect(),
        },
        Expression::MemberAccess { target, member } => HirExpression::MemberAccess {
            target: Box::new(elaborate_expression(target, type_params)),
            member: member.clone(),
        },
        Expression::Trapped {
            expression,
            binding,
            handler,
            line,
        } => HirExpression::Trapped {
            expression: Box::new(elaborate_expression(expression, type_params)),
            binding: binding.clone(),
            handler: handler
                .iter()
                .map(|__v| elaborate_statement(__v, type_params))
                .collect(),
            line: *line,
        },
        Expression::Identifier(name) => HirExpression::Identifier(name.clone()),
    }
}

// plan-106-D: the de-elaboration block (HIR → AST) lived here — 16 functions
// behind one `deelaborate` entry, rendering the concrete HIR back to an AST for
// the three post-monomorph validators that still consumed `crate::ast`. All
// three (`resolver::resolve_augmented`, `manifest::entry::validate_entry_point`,
// the former source checker's `check_project_collect`) now take `&HirProject`, so the render
// has no callers and is deleted. It was the last backward edge in the compiler,
// and the last thing that depended on `parse`↔`name` being byte-exact.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{parse_source, AstProject};
    use std::path::Path;

    /// Parse a single `.mfb` source string into a one-file [`AstProject`], the
    /// same shape the real loader builds (minus the prelude, which this test
    /// does not need).
    fn project(source: &str) -> AstProject {
        let file = parse_source(Path::new("main.mfb"), "main.mfb", source)
            .expect("test source should parse");
        AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    /// The lone file's items, after elaboration.
    fn elaborated_items(source: &str) -> Vec<HirItem> {
        let ast = project(source);
        let hir = elaborate(&ast);
        assert_eq!(hir.name, "test");
        assert_eq!(hir.files.len(), 1);
        hir.files.into_iter().next().unwrap().items
    }

    #[test]
    fn function_param_and_return_types_carry_bare_spellings() {
        let items = elaborated_items(
            "FUNC add(a AS Integer, b AS List OF Integer) AS Map OF String TO Integer\n\
               RETURN b\n\
             END FUNC\n",
        );
        let function = items
            .iter()
            .find_map(|item| match item {
                HirItem::Function(function) if function.name == "add" => Some(function),
                _ => None,
            })
            .expect("add function present");
        assert_eq!(function.params[0].type_.name(), "Integer");
        assert_eq!(function.params[1].type_.name(), "List OF Integer");
        assert_eq!(function.returns.name(), "Map OF String TO Integer");
        assert!(!function.params[0].resource);
    }

    #[test]
    fn generic_decls_classify_type_variables_as_var() {
        use crate::types::ParameterType;
        // A generic FUNC: its `template_params` name the type variables, so a leaf
        // matching one elaborates to `Var`, while a concrete leaf (`Integer`) does not
        // (plan-102-D). This is the classification `parse` alone cannot do — it has no
        // scope, so it always yields `Named` (plan-102-A) until `with_vars` reclassifies.
        let items = elaborated_items(
            "FUNC first OF T (xs AS List OF T, i AS Integer) AS T\n\
               RETURN i\n\
             END FUNC\n\
             TYPE Box OF E\n  item AS E\n  count AS Integer\nEND TYPE\n",
        );
        let function = items
            .iter()
            .find_map(|item| match item {
                HirItem::Function(function) if function.name == "first" => Some(function),
                _ => None,
            })
            .expect("first function present");
        assert_eq!(function.template_params, vec!["T".to_string()]);
        // `List OF T`: the element `T` is a `Var`, not a `Named`.
        match &function.params[0].type_ {
            ParameterType::ListOf(elem) => {
                assert!(
                    matches!(elem.as_ref(), ParameterType::Var(_)),
                    "T should be Var"
                );
            }
            other => panic!("expected List OF, got {other:?}"),
        }
        // A concrete scalar param stays a scalar (not reclassified).
        assert!(matches!(function.params[1].type_, ParameterType::Integer));
        // The return `T` is a `Var` whose name still round-trips as `T`.
        assert!(matches!(function.returns, ParameterType::Var(_)));
        assert_eq!(function.returns.name(), "T");

        // A generic TYPE: the field typed by the template param is a `Var`; a concrete
        // field is not.
        let type_decl = items
            .iter()
            .find_map(|item| match item {
                HirItem::Type(decl) if decl.name == "Box" => Some(decl),
                _ => None,
            })
            .expect("Box type present");
        assert!(matches!(type_decl.fields[0].type_, ParameterType::Var(_)));
        assert!(matches!(type_decl.fields[1].type_, ParameterType::Integer));
    }

    #[test]
    fn let_binding_annotation_carries_bare_spelling() {
        let items = elaborated_items(
            "FUNC main() AS Integer\n\
               LET n AS Integer = 5\n\
               LET s = \"hi\"\n\
               RETURN n\n\
             END FUNC\n",
        );
        let body = match &items[0] {
            HirItem::Function(function) => &function.body,
            other => panic!("expected function, got {other:?}"),
        };
        // The annotated `LET n AS Integer` keeps its spelling and is explicit; the
        // inferred `LET s` has no annotation, so it elaborates to `Unknown`.
        let mut lets = body.iter().filter_map(|statement| match statement {
            HirStatement::Let {
                name,
                type_,
                explicit_type,
                ..
            } => Some((name.as_str(), type_.name().into_owned(), *explicit_type)),
            _ => None,
        });
        assert_eq!(lets.next(), Some(("n", "Integer".to_string(), true)));
        assert_eq!(lets.next(), Some(("s", "Unknown".to_string(), false)));
    }

    #[test]
    fn type_record_fields_carry_bare_spellings() {
        let items =
            elaborated_items("TYPE Point\n  x AS Integer\n  PRIVATE y AS Float\nEND TYPE\n");
        let decl = match &items[0] {
            HirItem::Type(decl) => decl,
            other => panic!("expected type decl, got {other:?}"),
        };
        assert_eq!(decl.name, "Point");
        assert_eq!(decl.fields[0].name, "x");
        assert_eq!(decl.fields[0].type_.name(), "Integer");
        assert_eq!(decl.fields[1].name, "y");
        assert_eq!(decl.fields[1].type_.name(), "Float");
    }

    #[test]
    fn collection_literals_and_constructor_carry_bare_spellings() {
        let items = elaborated_items(
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
             FUNC main() AS Integer\n\
               LET l = [1, 2, 3]\n\
               LET st = Set OF Integer { 1, 2, 3 }\n\
               LET m = Map OF String TO Integer { \"a\" := 1 }\n\
               LET p = Point[1, 2]\n\
               RETURN 0\n\
             END FUNC\n",
        );
        let body = &items
            .iter()
            .find_map(|item| match item {
                HirItem::Function(function) if function.name == "main" => Some(function),
                _ => None,
            })
            .expect("main present")
            .body;

        // Walk the `LET` initializers and assert each literal/constructor's bare
        // type spellings survived elaboration byte-exact.
        let mut saw_set = false;
        let mut saw_map = false;
        let mut saw_constructor = false;
        for statement in body {
            if let HirStatement::Let {
                value: Some(value), ..
            } = statement
            {
                match value {
                    HirExpression::SetLiteral { element_type, .. } => {
                        assert_eq!(element_type.name(), "Integer");
                        saw_set = true;
                    }
                    HirExpression::MapLiteral {
                        key_type,
                        value_type,
                        ..
                    } => {
                        assert_eq!(key_type.name(), "String");
                        assert_eq!(value_type.name(), "Integer");
                        saw_map = true;
                    }
                    HirExpression::Constructor { type_, .. } => {
                        assert_eq!(type_.name(), "Point");
                        saw_constructor = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(saw_set, "set literal elaborated");
        assert!(saw_map, "map literal elaborated");
        assert!(saw_constructor, "constructor elaborated");
    }

    #[test]
    fn elaborate_does_not_panic_on_mixed_corpus() {
        // A broad corpus exercising most statement/expression shapes at once; the
        // assertion is simply that `elaborate` completes without panicking.
        let items = elaborated_items(
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
             FUNC classify(n AS Integer) AS String\n\
               IF n > 0 THEN\n    RETURN \"pos\"\n  ELSE\n    RETURN \"neg\"\n  END IF\n\
             END FUNC\n\
             FUNC main() AS Integer\n\
               LET nums AS List OF Integer = [1, 2, 3]\n\
               MUT total AS Integer = 0\n\
               FOR EACH v IN nums\n    total = total + v\n  NEXT\n\
               FOR i = 1 TO 3\n    total = total + i\n  NEXT\n\
               LET label = classify(total)\n\
               RETURN total\n\
             END FUNC\n",
        );
        assert!(items
            .iter()
            .any(|item| matches!(item, HirItem::Function(function) if function.name == "main")));
    }
}
