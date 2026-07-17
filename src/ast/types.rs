use super::*;

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
    /// True for compiler-injected built-in package source (`json`, `regex`,
    /// `collections`). Such files are lexed in internal mode, rewriting their
    /// `__`-prefixed private names to the untypeable internal sigil. Not emitted
    /// in the `.ast` dump (injected files are excluded from it).
    pub internal: bool,
}

#[derive(Clone, Debug)]
pub struct Import {
    pub module: String,
    pub alias: Option<String>,
    pub line: usize,
}

impl Import {
    pub fn package_name(&self) -> &str {
        self.module
            .split('.')
            .next()
            .unwrap_or(self.module.as_str())
    }

    pub fn binding_name(&self) -> &str {
        self.alias.as_deref().unwrap_or_else(|| self.package_name())
    }
}

impl AstFile {
    pub fn import_bindings(&self) -> HashMap<String, String> {
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

#[derive(Clone, Debug)]
pub enum Item {
    Binding(TopLevelBinding),
    Function(Function),
    Type(TypeDecl),
    /// A package-scope `[vis] RESOURCE Name CLOSE BY pkg::close` declaration
    /// that introduces a native resource type (plan-link-update.md §5).
    Resource(ResourceDecl),
    /// A `[vis] FUNC alias AS qualified::func` transparent re-export of a `LINK`
    /// function — required to re-export a registered close op (plan-link-update.md §5a).
    FuncAlias(FuncAlias),
    /// A `LINK "lib" AS alias … END LINK` native binding block (plan-link-update.md §5b).
    Link(LinkBlock),
    /// A `DOC … END DOC` documentation block (plan-09-doc.md). Free-standing: the
    /// header line alone names the declaration it documents; proximity is not
    /// assumed. Resolution links it to its target and validates its contents.
    Doc(DocBlock),
    /// A `TESTING … END TESTING` block (plan-18-testing.md) holding `TGROUP`/`TCASE`
    /// test cases. Parsed and validated in every mode; dropped before codegen for
    /// `mfb build`, and desugared into a synthesized driver for `mfb test`.
    Testing(TestingBlock),
}

/// A `TESTING … END TESTING` top-level block: a flat list of `TGROUP` groups in
/// declaration order (plan-18-testing.md §4).
#[derive(Clone, Debug)]
pub struct TestingBlock {
    pub groups: Vec<TestGroup>,
    pub line: usize,
}

/// A `TGROUP <string> … END TGROUP` group: a described bundle of members, each
/// either a `TCASE` case or a nested `TGROUP` sub-group, in declaration order.
#[derive(Clone, Debug)]
pub struct TestGroup {
    pub description: String,
    pub members: Vec<TestGroupMember>,
    pub line: usize,
}

/// One member of a `TGROUP`, in declaration order: an ordinary `TCASE` case or a
/// nested `TGROUP` sub-group. Nesting may go arbitrarily deep, and cases and
/// sub-groups may interleave within a single group.
#[derive(Clone, Debug)]
pub enum TestGroupMember {
    Case(TestCase),
    Group(TestGroup),
}

/// A `TCASE <string> … END TCASE` case: a described statement body exercising the
/// assertion builtins (`expectEQ`/`expectNQ`/`expectTrap`/`expectNTrap`).
#[derive(Clone, Debug)]
pub struct TestCase {
    pub description: String,
    pub body: Vec<Statement>,
    pub line: usize,
}

/// The kind named by a `DOC` block's header line.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DocHeaderKind {
    Func,
    Sub,
    Type,
    Union,
    Enum,
    Package,
}

impl DocHeaderKind {
    pub fn keyword(self) -> &'static str {
        match self {
            DocHeaderKind::Func => "FUNC",
            DocHeaderKind::Sub => "SUB",
            DocHeaderKind::Type => "TYPE",
            DocHeaderKind::Union => "UNION",
            DocHeaderKind::Enum => "ENUM",
            DocHeaderKind::Package => "PACKAGE",
        }
    }
}

/// The kind of a prose block in a `DOC` body: an ordinary description paragraph
/// (`DESC`) or one of the callouts (`WARN`/`INFO`/`SEC`). They interleave in
/// source order so a callout can sit between two paragraphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocProseKind {
    Desc,
    Warn,
    Info,
    Sec,
}

impl DocProseKind {
    pub fn from_keyword(keyword: &str) -> Option<DocProseKind> {
        match keyword.to_ascii_uppercase().as_str() {
            "DESC" => Some(DocProseKind::Desc),
            "WARN" => Some(DocProseKind::Warn),
            "INFO" => Some(DocProseKind::Info),
            "SEC" => Some(DocProseKind::Sec),
            _ => None,
        }
    }

    /// Stable on-wire code for the `.mfp` doc section and `-ast` output.
    pub fn code(self) -> u8 {
        match self {
            DocProseKind::Desc => 0,
            DocProseKind::Warn => 1,
            DocProseKind::Info => 2,
            DocProseKind::Sec => 3,
        }
    }

    pub fn from_code(code: u8) -> DocProseKind {
        match code {
            1 => DocProseKind::Warn,
            2 => DocProseKind::Info,
            3 => DocProseKind::Sec,
            _ => DocProseKind::Desc,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DocProseKind::Desc => "desc",
            DocProseKind::Warn => "warn",
            DocProseKind::Info => "info",
            DocProseKind::Sec => "sec",
        }
    }
}

/// One prose paragraph or callout in a `DOC` body.
#[derive(Clone, Debug)]
pub struct DocProse {
    pub kind: DocProseKind,
    pub text: String,
}

/// A parsed (but not yet semantically validated) documentation block. Duplicate
/// and context checks are deferred to the resolver, so repeated fields such as
/// `RET`/`EXAMPLE`/`DEPRECATED`/`GROUP` are kept as lists here.
#[derive(Clone, Debug)]
pub struct DocBlock {
    pub line: usize,
    /// Raw attribute words from the `DOC` keyword line (e.g. `INTERNAL`).
    pub attrs: Vec<String>,
    pub header_kind: DocHeaderKind,
    /// Declaration name; empty for a `PACKAGE` header.
    pub header_name: String,
    /// Parenthesized parameter-type disambiguator from the header (FUNC/SUB), or
    /// `None` when the header named no overload signature.
    pub header_params: Option<Vec<String>>,
    pub header_line: usize,
    /// Description/callout prose blocks in source order (`DESC`/`WARN`/`INFO`/`SEC`).
    pub desc: Vec<DocProse>,
    /// `DEPRECATED` body lines: `(message, line)`. More than one is an error.
    pub deprecated: Vec<(String, usize)>,
    /// `GROUP` body lines: `(name, line)`. FUNC/SUB only; more than one is an error.
    pub groups: Vec<(String, usize)>,
    pub args: Vec<DocNamed>,
    /// `RET` body lines: `(text, line)`. More than one is an error.
    pub rets: Vec<(String, usize)>,
    pub errors: Vec<DocError>,
    pub props: Vec<DocNamed>,
    /// `EXAMPLE` sub-blocks: `(source, line)`. More than one is an error.
    pub examples: Vec<(String, usize)>,
}

/// An `ARG name desc` or `PROP name desc` documentation line.
#[derive(Clone, Debug)]
pub struct DocNamed {
    pub name: String,
    pub desc: String,
    pub line: usize,
}

/// An `ERROR code desc` documentation line.
#[derive(Clone, Debug)]
pub struct DocError {
    pub code: String,
    pub desc: String,
    pub line: usize,
}

/// A package-scope native resource declaration: `[vis] RESOURCE Name CLOSE BY
/// closeFn [THREAD_SENDABLE]` (plan-link-update.md §5). `close_fn` is the
/// (possibly qualified) registered close op; `Name` is an opaque unique native
/// handle whose hidden representation is a `CPtr`.
#[derive(Clone, Debug)]
pub struct ResourceDecl {
    pub visibility: Visibility,
    pub name: String,
    /// The registered close op, qualified as `alias.func` (dotted, like other
    /// qualified names in the AST).
    pub close_fn: String,
    /// Whether the resource opts into thread sendability (plan-link-update.md §8).
    pub thread_sendable: bool,
    pub line: usize,
}

/// A transparent function alias `[vis] FUNC alias AS qualified::func`
/// (plan-link-update.md §5a). The alias names the *same* function — same
/// signature, and, for a close op, the same registered close op.
#[derive(Clone, Debug)]
pub struct FuncAlias {
    pub visibility: Visibility,
    pub name: String,
    /// The aliased target, qualified as `alias.func` (dotted).
    pub target: String,
    pub line: usize,
}

/// A `LINK "library" AS alias` native binding block (plan-link-update.md §5b).
#[derive(Clone, Debug)]
pub struct LinkBlock {
    /// The native library name, e.g. `"sqlite3"`.
    pub library: String,
    /// The local binding name for the block's functions, e.g. `sqliteLink`.
    pub alias: String,
    pub functions: Vec<LinkFunction>,
    /// `CSTRUCT <CName> AS <MfbType>` C-layout declarations (plan-50-B).
    pub cstructs: Vec<CStructDecl>,
    pub line: usize,
}

/// A `CSTRUCT <CName> AS <MfbType> … END CSTRUCT` declaration inside a `LINK`
/// block: the byte layout of a C struct, and the MFBASIC record it presents as
/// (plan-50-B).
///
/// The layout is **computed** from the field ctypes, never declared: there is no
/// offset, size, or padding syntax. `<CName>` is private to the `LINK` block
/// (`NATIVE_CSTRUCT_ESCAPE`); only `<MfbType>` is nameable by ordinary code.
#[derive(Clone, Debug)]
pub struct CStructDecl {
    /// The C-side name, e.g. `SfFormatInfo`. Local to the owning LINK alias.
    pub name: String,
    /// The MFBASIC record type this struct maps to, e.g. `AudioFormat`.
    pub maps_to: String,
    /// Fields in **C declaration order** — the order drives the offsets.
    pub fields: Vec<CStructField>,
    pub line: usize,
}

/// One `CSTRUCT` field: `<name> <ctype>`.
#[derive(Clone, Debug)]
pub struct CStructField {
    pub name: String,
    pub ctype: String,
    pub line: usize,
}

/// A native function declaration inside a `LINK` block: an MFBASIC-facing
/// signature plus its native `SYMBOL`, `ABI` mapping, `CONST` pins, and success
/// gate (plan-link-update.md §5b/§5c).
#[derive(Clone, Debug)]
pub struct LinkFunction {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<String>,
    /// Whether the return type was declared with `RES` (produces a resource).
    pub return_resource: bool,
    /// The native C symbol, e.g. `"sqlite3_open"`.
    pub symbol: String,
    /// The named-slot ABI signature.
    pub abi: AbiSpec,
    /// `CONST slot = value` pins (plan-link-update.md §5c).
    pub consts: Vec<ConstPin>,
    /// `SUCCESS_ON <expr>` gate, if any (the De Morgan complement of `ERROR_ON`).
    pub success_on: Option<Expression>,
    /// `RESULT <expr>` value mapping, if any (plan-link-update.md §5b).
    pub result: Option<Expression>,
    /// `FREE <slot> … END FREE` block releasing a caller-owned native return
    /// after it is copied out (mfbasic.md §17).
    pub free: Option<FreeSpec>,
    pub line: usize,
}

/// A `FREE <slot> SYMBOL "…" ABI (ptr CPtr) AS CVoid END FREE` block: after the
/// wrapper copies the named produced pointer into its owned MFBASIC result, the
/// original native pointer is passed to the named deallocator (mfbasic.md §17).
#[derive(Clone, Debug)]
pub struct FreeSpec {
    /// The produced slot whose pointer is released (`return` or an `OUT` slot).
    pub slot: String,
    /// The deallocator native symbol, e.g. `sqlite3_free`.
    pub symbol: String,
    /// The deallocator's single pointer parameter: name and C type.
    pub param_name: String,
    pub param_ctype: String,
    /// The deallocator's native return C type, e.g. `CVoid`.
    pub return_ctype: String,
    pub line: usize,
}

/// The `ABI (slot, …) AS retname rettype` named-slot signature
/// (plan-link-update.md §5b).
#[derive(Clone, Debug)]
pub struct AbiSpec {
    /// The parenthesized ABI slots, in native C argument order.
    pub slots: Vec<AbiSlot>,
    /// The native return slot: name and C type after `)` `AS`.
    pub return_name: String,
    pub return_ctype: String,
    /// Source line of the `ABI` clause; retained for diagnostics.
    #[allow(dead_code)]
    pub line: usize,
}

/// One `ABI (...)` slot: `name ctype`, `name OUT ctype`, or `return OUT ctype`.
#[derive(Clone, Debug)]
pub struct AbiSlot {
    /// Slot name; the literal `return` marks the wrapper-result OUT slot.
    pub name: String,
    pub ctype: String,
    /// `IN` (the default), `OUT`, or `INOUT` (plan-50-C).
    pub direction: crate::ir::AbiDirection,
    pub line: usize,
}

/// A `CONST slot = value` pin (plan-link-update.md §5c).
#[derive(Clone, Debug)]
pub struct ConstPin {
    pub slot: String,
    pub value: Expression,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct TopLevelBinding {
    pub visibility: Visibility,
    pub mutable: bool,
    /// Whether this binding was declared with `RES` (a uniquely-owned resource).
    pub resource: bool,
    /// The `STATE T` type attached to a `RES` binding, if any.
    pub state_type: Option<String>,
    pub name: String,
    pub type_name: Option<String>,
    pub value: Option<Expression>,
    pub line: usize,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    /// Whether the return type was declared with `RES` (returns a resource).
    pub return_resource: bool,
    /// The `STATE T` type attached to a `RES` return type, if any.
    pub return_state_type: Option<String>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FunctionKind {
    Func,
    Sub,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Visibility {
    Private,
    Public,
    Export,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub type_name: Option<String>,
    /// Whether this parameter was declared with `RES` (a borrowed/owned resource).
    pub resource: bool,
    /// The `STATE T` type attached to a `RES` parameter, if any.
    pub state_type: Option<String>,
    pub default: Option<Expression>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub enum CallArg {
    Positional(Expression),
    Named {
        name: String,
        value: Expression,
        line: usize,
    },
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
        /// Whether this binding was declared with `RES`.
        resource: bool,
        /// The `STATE T` type attached to a `RES` binding, if any.
        state_type: Option<String>,
        name: String,
        type_name: Option<String>,
        value: Option<Expression>,
        line: usize,
    },
    Return {
        value: Option<Expression>,
        line: usize,
    },
    Exit {
        target: ExitTarget,
        code: Option<Expression>,
        line: usize,
    },
    Continue {
        kind: LoopKind,
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
        value: Option<Expression>,
        line: usize,
    },
    Assign {
        name: String,
        value: Expression,
        line: usize,
    },
    /// `resource.state = value` — replace a `RES` binding's `STATE` payload. The
    /// language has no general field assignment; this is the one member-target
    /// assignment, mirroring the functional `WITH` update idiom
    /// (`s.state = WITH s.state { field := ... }`).
    StateAssign {
        resource: String,
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
    For {
        name: String,
        start: Expression,
        end: Expression,
        step: Option<Expression>,
        body: Vec<Statement>,
        line: usize,
    },
    ForEach {
        name: String,
        iterable: Expression,
        body: Vec<Statement>,
        line: usize,
    },
    While {
        kind: LoopKind,
        condition: Expression,
        body: Vec<Statement>,
        line: usize,
    },
    DoUntil {
        body: Vec<Statement>,
        condition: Expression,
        line: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitTarget {
    For,
    Do,
    While,
    Sub,
    Func,
    Program,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoopKind {
    For,
    Do,
    While,
}

#[derive(Clone, Debug)]
pub struct MatchCase {
    pub pattern: MatchPattern,
    pub guard: Option<Expression>,
    pub body: Vec<Statement>,
    pub line: usize,
}

#[derive(Clone, Debug)]
pub enum MatchPattern {
    Else,
    Literal(Expression),
    Union { type_name: String, binding: String },
    OneOf(Vec<Expression>),
}

#[derive(Clone, Debug)]
pub enum Expression {
    String(String),
    Number(String),
    /// A backtick scalar literal `` `x` `` carrying its decoded Unicode scalar
    /// value (plan-41-A). Intrinsically typed `Scalar`.
    Scalar(u32),
    Boolean(bool),
    Binary {
        left: Box<Expression>,
        operator: String,
        right: Box<Expression>,
        // Internal source location of the operator; not serialized to AST JSON.
        line: usize,
        column: usize,
    },
    Unary {
        operator: String,
        operand: Box<Expression>,
        // Internal source location of the operator; not serialized to AST JSON.
        line: usize,
        column: usize,
    },
    Call {
        callee: String,
        arguments: Vec<CallArg>,
        // Internal source location of the call expression; not serialized to AST JSON.
        line: usize,
        column: usize,
    },
    Lambda {
        params: Vec<Param>,
        body: Box<Expression>,
        /// Set when the lambda body is an assignment `name = <body>` rather than a
        /// plain expression. Such a lambda evaluates `body`, assigns it to the
        /// outer (or parameter) binding `name`, and yields `Nothing`. This is the
        /// shape that lets a non-escaping callback mutate a captured `MUT`
        /// binding, e.g. `LAMBDA(x) -> total = total + x`.
        assign_target: Option<String>,
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
    Trapped {
        expression: Box<Expression>,
        binding: String,
        handler: Vec<Statement>,
        line: usize,
    },
    Identifier(String),
}
