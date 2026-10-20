//! plan-176-M: the HIR node definitions. The AST→HIR elaboration lives in `hir/mod.rs`.

use super::*;

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
    ///
    /// This is the file's **resolution scope**, not necessarily its own
    /// declaration list — see [`own_imports`](Self::own_imports).
    pub(crate) imports: Vec<crate::ast::Import>,
    /// The packages this file's own source `IMPORT`ed, in source order.
    ///
    /// Held separately from [`imports`](Self::imports) because monomorphization
    /// widens that list: `monomorph::lower` emits a generated instantiation into
    /// the file that declares its template (the FIRST file for a template the
    /// toolchain declares) and unions every other file's imports into each file
    /// that hosts one, so those bodies' package-qualified names still resolve;
    /// the hosting file's own bindings win. After that pass
    /// `main.mfb`'s `imports` reads `{tcp, udp, net, strings, collections}` for a
    /// program that wrote only `IMPORT tcp` and `IMPORT udp` — `net`, `strings`
    /// and `collections` arrive from the injected `udp` companion source.
    ///
    /// The widening is correct for resolution and lowering, and invisible to
    /// name resolution (which ran on the original AST, and still rejects a
    /// `net::…` call in a file that did not import `net`). It is wrong for any
    /// rule asking what the *author* imported — `ir::shape`'s foreign-record
    /// field rule (bug-466), whose whole point is that an unrelated `IMPORT udp`
    /// must not change the verdict. Monomorph copies this field verbatim.
    pub(crate) own_imports: Vec<String>,
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
/// The documentation/testing variants carry no source-language type strings
/// needing a [`ParameterType`], so they reuse the AST struct directly.
#[derive(Clone, Debug)]
pub(crate) enum HirItem {
    Binding(HirTopLevelBinding),
    Function(HirFunction),
    Type(HirTypeDecl),
    Resource(crate::ast::ResourceDecl),
    FuncAlias(crate::ast::FuncAlias),
    Link(HirLinkBlock),
    Doc(crate::ast::DocBlock),
    Testing(crate::ast::TestingBlock),
}

/// A `LINK` block, elaborated: every MFBASIC type spelling it declares has
/// become a [`ParameterType`].
///
/// plan-111-B: this variant used to carry the raw `crate::ast::LinkBlock` — the
/// one item kind HIR did not elaborate — so `resolver`, `ir::lower`,
/// `ir::verify::link` and `ir::shape` each read its spellings and re-derived
/// structure from them, one of them with a third private copy of the `STATE`
/// grammar. `ir::lower`'s own comment named the gap: "elaborating `LinkBlock`
/// properly is recorded as a task in plan-106-E". This is that task. The
/// AST→type boundary now sits here, where every other item's already is.
///
/// Everything a `LINK` declaration says that is NOT an MFBASIC type — the
/// native symbol, the ABI slot map, `CONST` pins, `BIND`/`BUFFER`/`SUCCESS_ON`/
/// `RETURN`/`FREE` clauses — stays as its AST node, because none of it is
/// type-domain and re-mirroring it would buy nothing.
#[derive(Clone, Debug)]
pub(crate) struct HirLinkBlock {
    /// The native library name, e.g. `"sqlite3"`.
    pub(crate) library: String,
    /// The local binding name for the block's functions, e.g. `sqliteLink`.
    pub(crate) alias: String,
    pub(crate) functions: Vec<HirLinkFunction>,
    /// `CSTRUCT <CName> AS <MfbType>` C-layout declarations (plan-50-B).
    pub(crate) cstructs: Vec<HirCStructDecl>,
    pub(crate) line: usize,
}

/// A native function declaration inside a [`HirLinkBlock`].
#[derive(Clone, Debug)]
pub(crate) struct HirLinkFunction {
    pub(crate) name: String,
    pub(crate) params: Vec<HirLinkParam>,
    /// The declared return type; `None` when the declaration omitted one.
    pub(crate) return_type: Option<ParameterType>,
    /// Whether the return type was declared with `RES` (produces a resource).
    pub(crate) return_resource: bool,
    /// The `STATE T` clause on a `RES` return, if any (plan-53-A).
    pub(crate) return_state_type: Option<ParameterType>,
    /// The native C symbol, e.g. `"sqlite3_open"`.
    pub(crate) symbol: String,
    pub(crate) abi: HirAbiSpec,
    pub(crate) consts: Vec<crate::ast::ConstPin>,
    pub(crate) bind_in: Vec<crate::ast::BindIn>,
    pub(crate) bind_state: Option<crate::ast::BindState>,
    pub(crate) buffers: Vec<crate::ast::BufferSpec>,
    pub(crate) result_length: Option<crate::ast::Expression>,
    pub(crate) success_on: Option<crate::ast::Expression>,
    pub(crate) result: Option<crate::ast::Expression>,
    pub(crate) free: Option<HirFreeSpec>,
    pub(crate) line: usize,
}

/// The elaborated mirror of [`crate::ast::AbiSpec`] — the `ABI (slots…) AS name
/// ctype` native call shape, with every C type parsed (plan-113).
#[derive(Clone, Debug)]
pub(crate) struct HirAbiSpec {
    pub(crate) slots: Vec<HirAbiSlot>,
    pub(crate) return_name: String,
    pub(crate) return_ctype: ParameterType,
    pub(crate) line: usize,
}

/// The elaborated mirror of [`crate::ast::AbiSlot`].
#[derive(Clone, Debug)]
pub(crate) struct HirAbiSlot {
    pub(crate) name: String,
    pub(crate) ctype: ParameterType,
    pub(crate) direction: crate::ir::AbiDirection,
    pub(crate) line: usize,
}

/// The elaborated mirror of [`crate::ast::FreeSpec`] — the deallocator block's
/// parameter and return C types.
#[derive(Clone, Debug)]
pub(crate) struct HirFreeSpec {
    pub(crate) slot: String,
    pub(crate) symbol: String,
    pub(crate) param_name: String,
    pub(crate) param_ctype: ParameterType,
    pub(crate) return_ctype: ParameterType,
    pub(crate) line: usize,
}

/// The elaborated mirror of [`crate::ast::CStructField`] — one `<name> <ctype>`
/// field of a `CSTRUCT`.
#[derive(Clone, Debug)]
pub(crate) struct HirCStructField {
    pub(crate) name: String,
    pub(crate) ctype: ParameterType,
    pub(crate) line: usize,
}

/// One parameter of a [`HirLinkFunction`].
#[derive(Clone, Debug)]
pub(crate) struct HirLinkParam {
    pub(crate) name: String,
    /// The declared type; `None` when the declaration omitted one.
    pub(crate) type_: Option<ParameterType>,
    /// Whether this parameter is a resource: its type is one, or it carries
    /// `STATE`. Derived from the type, never written (bug-976,
    /// [`resource_params`]).
    pub(crate) resource: bool,
    /// The `STATE T` type attached to the parameter, if any.
    pub(crate) state_type: Option<ParameterType>,
    /// plan-136-A: elaborated like a FUNC default, so it can be resolved and
    /// lowered at the declaration.
    pub(crate) default: Option<HirExpression>,
    pub(crate) line: usize,
}

/// A `CSTRUCT <CName> AS <MfbType>` declaration, with its MFBASIC record type
/// and its C-side field list both elaborated.
///
/// plan-113: the field list used to be the verbatim AST struct, on the reasoning
/// that *"a `ctype` is a C ABI slot spelling, not an MFBASIC type"*. It is now a
/// [`ParameterType`] like every other type this stage carries — a
/// [`ParameterType::C`] for one of the 16 primitives, a
/// [`Named`](ParameterType::Named) for a name outside them (which the CSTRUCT
/// rules then reject). This is the boundary where that conversion belongs: the
/// AST is the name world, and `elaborate_link_block` is where its spellings
/// become types.
#[derive(Clone, Debug)]
pub(crate) struct HirCStructDecl {
    /// The C-side name, e.g. `SfFormatInfo`. Local to the owning LINK alias.
    pub(crate) name: String,
    /// The MFBASIC record type this struct maps to, e.g. `AudioFormat`.
    pub(crate) maps_to: ParameterType,
    /// Fields in **C declaration order** — the order drives the offsets.
    pub(crate) fields: Vec<HirCStructField>,
    pub(crate) line: usize,
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
    /// `INCLUDES`d type names, elaborated. plan-111-B: the AST spells these as
    /// bare or `pkg.`-qualified identifiers, but `resolver::resolve_type` treats
    /// a scalar name (`Integer`) differently from a nominal one, so which of the
    /// two an include is has to be decided by the grammar — here, at the AST
    /// boundary, rather than by a `parse` in the resolver.
    pub(crate) includes: Vec<ParameterType>,
    /// Union variants name a member type — possibly a template instantiation
    /// over this declaration's parameters (bug-680); elaborated for the same
    /// reason as [`includes`](Self::includes).
    pub(crate) variants: Vec<HirUnionVariant>,
    /// Enum members carry only a name, so the AST node is reused verbatim.
    pub(crate) members: Vec<crate::ast::EnumMember>,
    pub(crate) line: usize,
}

/// One `UNION` variant — the elaborated mirror of
/// [`crate::ast::UnionVariant`].
#[derive(Clone, Debug)]
pub(crate) struct HirUnionVariant {
    pub(crate) type_: ParameterType,
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

/// A function — the elaborated mirror of [`crate::ast::Function`].
#[derive(Clone, Debug)]
pub(crate) struct HirFunction {
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
    /// A return's `STATE T` clause on a type that is not a resource (bug-998),
    /// moved here exactly as [`HirParam::stray_state`] is.
    pub(crate) return_stray_state: Option<ParameterType>,
    pub(crate) body: Vec<HirStatement>,
    pub(crate) trap: Option<HirTrap>,
    pub(crate) line: usize,
    /// Set by the monomorphizer on an instantiation of a template declared in an
    /// internal (built-in package) file. The instantiation is emitted into the
    /// user's first file (a user template's goes into its declaring file), but it
    /// is still the package's code: the resolver exempts it from the rules an
    /// internal file is exempt from.
    pub(crate) internal_origin: bool,
    /// bug-642: for an `internal_origin` instantiation, every user call that
    /// reaches it, in the order monomorphization met them (one instantiation
    /// serves every call with the same type arguments). The instantiated body's own
    /// lines are the package source's, not the file it is emitted into, so a
    /// diagnostic raised while checking the body is reported at each of these
    /// instead. A nested instantiation (a helper the package body calls) inherits
    /// the outermost user call. Empty for everything else.
    pub(crate) instantiated_at: Vec<InstantiationSite>,
}

/// bug-642: where a built-in package's generic body was instantiated from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InstantiationSite {
    /// The project-relative file of the user's call.
    pub(crate) file: String,
    /// The 1-based line of the user's call.
    pub(crate) line: usize,
    /// The member as the user spells it (`collections::sort`).
    pub(crate) callee: String,
}

impl InstantiationSite {
    /// A diagnostic's detail, reworded for the user's call: it names the member
    /// whose body the rule failed in, since the reported line is the call's.
    pub(crate) fn describe(&self, detail: &str) -> String {
        format!(
            "In the body of `{}`, instantiated by this call: {detail}",
            self.callee
        )
    }
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
    /// Whether this parameter is a resource: its type is one, or it carries
    /// `STATE`. Derived from the type, never written (bug-976,
    /// [`resource_params`]).
    pub(crate) resource: bool,
    /// The `STATE T` type attached to the parameter, if any (bare, parsed).
    pub(crate) state_type: Option<ParameterType>,
    /// A `STATE T` clause the source wrote on a parameter whose type is not a
    /// resource (bug-998). [`resource_params`] moves the clause here on the
    /// build path, so the parameter types as the plain value it is and
    /// `ir::shape` reports the clause once (`TYPE_STATE_REQUIRES_RESOURCE`)
    /// instead of every use of the parameter failing to type.
    pub(crate) stray_state: Option<ParameterType>,
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
        operator: BinaryOp,
        right: Box<HirExpression>,
        line: usize,
        column: usize,
    },
    Unary {
        operator: UnaryOp,
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
