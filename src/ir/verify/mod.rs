//! IR-level semantic verification (plan-19-ir-semantic-verification.md).
//!
//! A compiled package (`.mfp`) carries hand-serializable IR that a consumer
//! decodes and lowers to native code. Only the source front end runs the AST
//! type checker (`src/typecheck/`); the decoded package IR is otherwise trusted
//! to be well typed. A crafted `.mfp` can therefore ship type-confused IR — a
//! `MemberAccess` on an `Integer`, a `Capture` index past the closure's slots, a
//! call with the wrong argument count — that codegen turns into memory-unsafe
//! native code in the victim's binary (audit-1 finding **PKG-02**).
//!
//! This pass is the IR-level semantic checker. It reconstructs a type
//! environment from the merged `IrProject` (types, function signatures, globals,
//! closure shapes) and enforces the semantic invariants that the AST type
//! checker guarantees for source but that nothing re-checks on decoded IR:
//!
//! - **Member access** targets a record that actually declares the member; a
//!   member access on a primitive is rejected — including on a *computed*
//!   primitive result (a call/operator/extract), since the typed IR (format v3,
//!   plan-20-B) annotates every node with its result type.
//! - **Closure captures** address a slot within the enclosing closure's
//!   captured-slot count.
//! - **Call / constructor arity** matches the callee signature / record shape.
//! - **Union wraps** name a real variant of the union.
//! - **Match** statements carry at least one case.
//!
//! Soundness rule: the checker must accept *exactly* the IR the front end emits
//! today (the byte-identical golden suite is the oracle). Every rule therefore
//! only rejects when it can *prove* a violation; whenever a type cannot be
//! resolved with certainty (the node carries the explicit `"Unknown"` marker,
//! or a name is unresolved) the node is skipped rather than rejected. Incomplete
//! type reconstruction weakens the check, it never produces a false rejection.
//!
//! Because the decoded package IR is now fully typed, the member-confusion class
//! is checked completely on the package path (plan-20-C): the checker no longer
//! has to give up when a member access targets a computed value whose type it
//! could not previously reconstruct. The remaining type-relational rules
//! (operand/argument/return compatibility) land with the census port
//! (plan-20-E..I), which relocates the front end's exact compatibility algebra
//! rather than approximating it here.
//!
//! `check` runs on the fully merged project (`merge_packages`) before it is
//! lowered, so every path that produces IR — the source front end and the
//! package decoder — is verified before any native code is emitted.

use super::{IrField, IrOp, IrProject, IrType, IrValue};
use crate::builtins;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// One semantic-verification diagnostic: the rule id, the human-readable detail,
/// the project-relative source file it originated in, and the 1-based line. The
/// checker accumulates these (plan-20-E..I) so it can reproduce the AST type
/// checker's full diagnostic sequence for a program, not just its first error.
#[derive(Clone)]
pub(crate) struct Diagnostic {
    pub(crate) rule: String,
    pub(crate) detail: String,
    pub(crate) file: String,
    pub(crate) line: u32,
}

/// Rules for which `ir::verify` is the sole rejecter (plan-20-Z). On the
/// **source** path `ir::verify` emits ONLY these (typecheck still owns every
/// other rule, so emitting a non-relocated rule here would duplicate it); on
/// the **package** path there is no typecheck, so `ir::verify` emits all of its
/// checks regardless. `typecheck::report` skips this same set. A rule appears
/// here only once `ir::verify` reproduces it completely (verified against every
/// `*-invalid` fixture).
pub const RELOCATED_TO_IR_VERIFY: &[&str] = &[
    "TYPE_BINARY_OPERATOR_MISMATCH",
    "TYPE_UNARY_OPERATOR_MISMATCH",
    "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
    "TYPE_UNKNOWN_FIELD",
    "TYPE_RETURN_MISMATCH",
    "TYPE_LIST_ELEMENT_MISMATCH",
    "TYPE_MAP_KEY_MISMATCH",
    "TYPE_MAP_VALUE_MISMATCH",
    "TYPE_RESOURCE_FIELD_FORBIDDEN",
    "TYPE_MIXED_RESOURCE_UNION",
    "TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION",
    "TYPE_BYTE_LITERAL_OVERFLOW",
    "TYPE_BYTE_LITERAL_UNDERFLOW",
    "TYPE_INTEGER_LITERAL_OVERFLOW",
    "TYPE_FLOAT_LITERAL_OVERFLOW",
    "TYPE_FLOAT_LITERAL_UNDERFLOW",
    "TYPE_FIXED_LITERAL_OVERFLOW",
    "TYPE_FIXED_LITERAL_UNDERFLOW",
    "TYPE_UNARY_OPERATOR_UNKNOWN",
    "TYPE_UNION_INCLUDE_REQUIRES_UNION",
    "TYPE_UNION_MEMBER_REQUIRES_TYPE",
    "TYPE_ENUM_REQUIRES_MEMBER",
    "TYPE_DUPLICATE_VARIANT",
    "TYPE_BINDING_MISMATCH",
    "TYPE_ASSIGN_REQUIRES_MUT",
    "TYPE_ASSIGNMENT_MISMATCH",
    "TYPE_FOR_STEP_ZERO",
    "TYPE_CONDITION_REQUIRES_BOOLEAN",
    "TYPE_FOR_REQUIRES_NUMERIC",
    "TYPE_FOR_EACH_REQUIRES_COLLECTION",
    "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
    "TYPE_CONSTRUCTOR_ARITY_MISMATCH",
    "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
    "TYPE_DEFAULT_VALUE_MISMATCH",
    "TYPE_READ_ONLY_RECORD_UPDATE",
    "TYPE_MATCH_PATTERN_MISMATCH",
    "TYPE_REQUIRES_COMPARABLE",
    "TYPE_MATCH_NOT_EXHAUSTIVE",
];

/// Diagnostic prefix shared with the structural `verify_package` checks so a
/// rejection surfaces as a `PACKAGE_BINARY_REPRESENTATION_*` diagnostic.
const VERIFY_TYPE: &str = "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE";
const VERIFY_MATCH: &str = "PACKAGE_BINARY_REPRESENTATION_VERIFY_MATCH";

/// Scalar types a value can never be member-accessed through. A `MemberAccess`
/// whose target provably has one of these types is a type confusion.
const PRIMITIVE_TYPES: &[&str] = &[
    "Integer", "Float", "String", "Boolean", "Byte", "Fixed", "Nothing",
];

/// Collect every semantic-verification diagnostic for a merged `IrProject`, in
/// the traversal order the AST type checker uses (functions in declaration
/// order; each body's ops in order; each op's sub-values innermost-first). The
/// checker never short-circuits, so a program with several violations yields
/// them all.
pub(crate) fn collect_diagnostics(project: &IrProject) -> Vec<Diagnostic> {
    let env = TypeEnv::build(project);
    for function in &project.functions {
        env.current_file.replace(function.file.clone());
        env.current_return.replace(function.returns.clone());
        env.current_kind.replace(function.kind.clone());
        let mut locals: HashMap<String, String> = HashMap::new();
        let mut muts: HashMap<String, bool> = HashMap::new();
        for param in &function.params {
            env.current_line.set(param.loc.line);
            locals.insert(param.name.clone(), param.type_.clone());
            env.check_map_key_comparable(&param.type_);
            // Parameters are immutable (typecheck registers them
            // `mutable: false`), so assigning one is TYPE_ASSIGN_REQUIRES_MUT.
            muts.insert(param.name.clone(), false);
            if let Some(default) = &param.default {
                env.check_value(default, &locals);
                // A parameter default must match the declared parameter type —
                // typecheck's TYPE_DEFAULT_VALUE_MISMATCH (skip-if-unknown).
                let expected = resource_base_type(&param.type_);
                if !expected.is_empty() && expected != "Unknown" && expected != "Nothing" {
                    if let Some(actual) = env.infer_type(default, &locals) {
                        if !env.expression_compatible(expected, &actual, default) {
                            env.emit(
                                "TYPE_DEFAULT_VALUE_MISMATCH",
                                format!(
                                    "Default value for `{}` has type {actual}, expected {expected}.",
                                    param.name
                                ),
                            );
                        }
                    }
                }
            }
        }
        let closure_slots = env.closure_slot_count(&function.name);
        env.check_ops(
            &function.body,
            &mut locals.clone(),
            &mut muts,
            closure_slots,
            0,
        );
        // Resource use-after-move is a separate straight-line dataflow pass.
        env.check_resource_moves(&function.body, &mut locals, &mut HashSet::new());
    }
    // Global initializers are lowered into a synthetic function later; verify
    // their initializer expressions here with an empty local scope.
    for binding in &project.bindings {
        env.current_file.replace(binding.file.clone());
        env.current_line.set(binding.loc.line);
        if binding.explicit_type {
            env.check_map_key_comparable(&binding.type_);
        }
        if let Some(value) = &binding.value {
            env.check_value(value, &HashMap::new());
            let before = env.diags.borrow().len();
            env.check_literal_range(resource_base_type(&binding.type_), value);
            let range_errored = env.diags.borrow().len() > before;
            if !range_errored && binding.explicit_type {
                env.check_binding_type(&binding.name, &binding.type_, value, &HashMap::new());
            }
        }
    }
    env.check_type_declarations(project);
    env.diags.take()
}

/// Verify the merged `IrProject` on the **package path** (`merge_packages`).
/// Returns `Ok(())` when the IR is well formed, or the first violation as an
/// error string. Package-path diagnostics carry no source context (the decoded
/// `.mfp` has no source file), so first-error is sufficient here.
pub fn check(project: &IrProject) -> Result<(), String> {
    match collect_diagnostics(project).into_iter().next() {
        Some(d) => Err(format!("{}: {}", d.rule, d.detail)),
        None => Ok(()),
    }
}

/// Verify the freshly elaborated **source-path** IR, emitting every diagnostic
/// through the shared diagnostics machinery (so the rule id, span, and source
/// context match what the AST type checker prints). Returns `Err(())` when any
/// diagnostic was emitted. `project_dir` resolves each `Diagnostic::file` to an
/// absolute path for the source-context display.
pub fn check_and_emit(project: &IrProject, project_dir: &Path) -> Result<(), ()> {
    let pending = collect_source_diagnostics(project, project_dir);
    let had_error = !pending.is_empty();
    crate::rules::render_pending(pending);
    if had_error {
        Err(())
    } else {
        Ok(())
    }
}

/// The relocated source-path diagnostics as unrendered `PendingDiagnostic`s, so
/// `build` can merge them with `typecheck`'s stream and render both in one
/// line-ordered pass (plan-20-Z). Only rules in `RELOCATED_TO_IR_VERIFY` are
/// ir::verify's to emit on the source path; the rest are still typecheck's.
pub fn collect_source_diagnostics(
    project: &IrProject,
    project_dir: &Path,
) -> Vec<crate::rules::PendingDiagnostic> {
    collect_diagnostics(project)
        .into_iter()
        .filter(|d| RELOCATED_TO_IR_VERIFY.contains(&d.rule.as_str()))
        .map(|d| crate::rules::PendingDiagnostic {
            rule: d.rule,
            detail: d.detail,
            path: if d.file.is_empty() {
                project_dir.join("<generated>")
            } else {
                project_dir.join(&d.file)
            },
            line: d.line as usize,
        })
        .collect()
}

/// Depth cap mirroring the decoder (`MAX_DECODE_DEPTH`). `check` may run on
/// merged IR that did not flow through the depth-bounded decoder (the project's
/// own functions), so it re-imposes the bound independently.
const MAX_DEPTH: usize = 256;

struct RecordInfo {
    fields: Vec<String>,
    includes: Vec<String>,
}

struct UnionInfo {
    variants: HashSet<String>,
    /// The direct variants in declaration order, for diagnostics that list
    /// missing members in source order (exhaustiveness).
    variant_order: Vec<String>,
    includes: Vec<String>,
}

struct FnSig {
    total: usize,
    optional: usize,
    /// Declared parameter types, positional (for argument-type checking).
    params: Vec<String>,
}

/// The reconstructed typing context: everything the semantic rules need to
/// resolve a name or a type, assembled from the merged project's tables.
struct TypeEnv {
    /// Record-shaped types (`kind` = `type`/`record`) and every union variant
    /// (each variant is itself a record) → its declared field names + includes.
    records: HashMap<String, RecordInfo>,
    /// Union types → their variant names + included unions.
    unions: HashMap<String, UnionInfo>,
    /// Internal (project + merged-package) function signatures, for arity.
    functions: HashMap<String, FnSig>,
    /// Global binding name → declared type.
    globals: HashMap<String, String>,
    /// Global binding name → whether it was declared `MUT` (assignable).
    global_muts: HashMap<String, bool>,
    /// Function name → the distinct captured-slot counts observed at the
    /// `Closure` sites that target it. A single count means the closure shape is
    /// known; zero or multiple distinct counts leaves it ambiguous (skip).
    closure_counts: HashMap<String, HashSet<usize>>,
    /// Record type name → (member name → declared member type), for chained
    /// member-access type inference.
    field_types: HashMap<String, HashMap<String, String>>,
    /// Record type name → its direct fields as ordered (name, type) pairs, for
    /// positional constructor checking (mirrors typecheck's `TypeInfo.fields`,
    /// which is declaration-ordered and not include-expanded).
    record_field_lists: HashMap<String, Vec<(String, String)>>,
    /// Enum type name → its complete member-name set, for MATCH exhaustiveness.
    enums: HashMap<String, HashSet<String>>,
    /// Accumulated diagnostics (plan-20-E..I); the checker pushes here instead
    /// of short-circuiting, so it reproduces the full diagnostic sequence.
    diags: RefCell<Vec<Diagnostic>>,
    /// Source line of the op/declaration currently being checked — the line a
    /// diagnostic emitted from a nested value is attributed to (matching the AST
    /// checker, which reports at the enclosing statement line).
    current_line: Cell<u32>,
    /// Project-relative file of the function currently being checked.
    current_file: RefCell<String>,
    /// Declared return type of the function currently being checked (for
    /// RETURN-type rules).
    current_return: RefCell<String>,
    /// `kind` (`func`/`sub`) of the function currently being checked.
    current_kind: RefCell<String>,
}

impl TypeEnv {
    fn build(project: &IrProject) -> Self {
        let mut records = HashMap::new();
        let mut unions = HashMap::new();
        let mut enums: HashMap<String, HashSet<String>> = HashMap::new();
        let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut record_field_lists: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for ty in &project.types {
            match ty.kind.as_str() {
                "enum" => {
                    enums.insert(
                        ty.name.clone(),
                        ty.members.iter().map(|m| m.name.clone()).collect(),
                    );
                }
                "type" | "record" => {
                    records.insert(
                        ty.name.clone(),
                        RecordInfo {
                            fields: ty.fields.iter().map(|f| f.name.clone()).collect(),
                            includes: ty.includes.clone(),
                        },
                    );
                    field_types.insert(ty.name.clone(), field_type_map(&ty.fields));
                    record_field_lists.insert(
                        ty.name.clone(),
                        ty.fields
                            .iter()
                            .map(|f| (f.name.clone(), f.type_.clone()))
                            .collect(),
                    );
                }
                "union" => {
                    unions.insert(
                        ty.name.clone(),
                        UnionInfo {
                            variants: ty.variants.iter().map(|v| v.name.clone()).collect(),
                            variant_order: ty.variants.iter().map(|v| v.name.clone()).collect(),
                            includes: ty.includes.clone(),
                        },
                    );
                    // Each variant is a record type in its own right; register
                    // its payload fields so `variant.field` accesses resolve.
                    for variant in &ty.variants {
                        records
                            .entry(variant.name.clone())
                            .or_insert_with(|| RecordInfo {
                                fields: variant.fields.iter().map(|f| f.name.clone()).collect(),
                                includes: Vec::new(),
                            });
                        field_types
                            .entry(variant.name.clone())
                            .or_insert_with(|| field_type_map(&variant.fields));
                        record_field_lists
                            .entry(variant.name.clone())
                            .or_insert_with(|| {
                                variant
                                    .fields
                                    .iter()
                                    .map(|f| (f.name.clone(), f.type_.clone()))
                                    .collect()
                            });
                    }
                }
                _ => {}
            }
        }

        let mut functions = HashMap::new();
        for function in &project.functions {
            functions.insert(
                function.name.clone(),
                FnSig {
                    total: function.params.len(),
                    optional: function
                        .params
                        .iter()
                        .filter(|p| p.default.is_some())
                        .count(),
                    params: function.params.iter().map(|p| p.type_.clone()).collect(),
                },
            );
        }

        let globals = project
            .bindings
            .iter()
            .map(|b| (b.name.clone(), b.type_.clone()))
            .collect();
        let global_muts = project
            .bindings
            .iter()
            .map(|b| (b.name.clone(), b.mutable))
            .collect();

        let mut closure_counts: HashMap<String, HashSet<usize>> = HashMap::new();
        for function in &project.functions {
            for param in &function.params {
                if let Some(default) = &param.default {
                    collect_closures(default, &mut closure_counts);
                }
            }
            collect_closures_ops(&function.body, &mut closure_counts);
        }
        for binding in &project.bindings {
            if let Some(value) = &binding.value {
                collect_closures(value, &mut closure_counts);
            }
        }

        TypeEnv {
            records,
            unions,
            functions,
            globals,
            global_muts,
            closure_counts,
            field_types,
            record_field_lists,
            enums,
            diags: RefCell::new(Vec::new()),
            current_line: Cell::new(0),
            current_file: RefCell::new(String::new()),
            current_return: RefCell::new(String::new()),
            current_kind: RefCell::new(String::new()),
        }
    }

    /// Record one diagnostic at the current line/file.
    fn emit(&self, rule: &str, detail: String) {
        self.diags.borrow_mut().push(Diagnostic {
            rule: rule.to_string(),
            detail,
            file: self.current_file.borrow().clone(),
            line: self.current_line.get(),
        });
    }

    /// The unique captured-slot count for a closure-body function, or `None`
    /// when it is never used as a closure or its shape is ambiguous.
    fn closure_slot_count(&self, function: &str) -> Option<usize> {
        let counts = self.closure_counts.get(function)?;
        if counts.len() == 1 {
            counts.iter().next().copied()
        } else {
            None
        }
    }

    fn check_ops(
        &self,
        ops: &[IrOp],
        locals: &mut HashMap<String, String>,
        muts: &mut HashMap<String, bool>,
        closure_slots: Option<usize>,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            self.emit(
                VERIFY_TYPE,
                format!("statement nesting exceeds the {MAX_DEPTH} level limit"),
            );
            return;
        }
        // `$`-temp name → its numeric-literal bind value, for rules that read a
        // literal through a synthesized temp (a FOR loop's STEP is always bound
        // to a `$for` temp immediately before its For op in the same op list).
        let mut temp_consts: HashMap<&str, &IrValue> = HashMap::new();
        for op in ops {
            let line = op.loc().line;
            self.current_line.set(line);
            match op {
                IrOp::Bind {
                    mutable,
                    name,
                    type_,
                    value,
                    explicit_type,
                    ..
                } => {
                    if let Some(value) = value {
                        self.check_value_captures(value, closure_slots);
                        self.check_value(value, locals);
                        let before = self.diags.borrow().len();
                        self.check_literal_range(resource_base_type(type_), value);
                        let range_errored = self.diags.borrow().len() > before;
                        // Only an explicit `AS T` annotation can disagree with
                        // the initializer; an inferred type is the initializer's
                        // type by construction (matches typecheck).
                        if !range_errored && *explicit_type {
                            self.check_binding_type(name, type_, value, locals);
                        }
                    }
                    // A declared map type's key must be comparable; the
                    // inferred case is covered at its MapLiteral (checking it
                    // here too would double-report).
                    if *explicit_type {
                        self.check_map_key_comparable(type_);
                    }
                    locals.insert(name.clone(), type_.clone());
                    // A capture bind's `mutable` reflects the by-ref/non-escaping
                    // proof, not the outer binding's MUTness — typecheck judges
                    // assignments to captures at the lambda site (as
                    // TYPE_LAMBDA_CAPTURE_UNSUPPORTED when escaping), so leave
                    // the capture's mutability unknown here.
                    if !matches!(value, Some(IrValue::Capture { .. })) {
                        muts.insert(name.clone(), *mutable);
                    }
                    if name.starts_with('$') {
                        if let Some(value) = value {
                            temp_consts.insert(name.as_str(), value);
                        }
                    }
                }
                IrOp::Assign { name, value, .. } => {
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                    // Skip synthesized `$`-temp targets (user identifiers cannot
                    // start with `$`): e.g. the RECOVER slot, whose value/type
                    // agreement is TYPE_RECOVER_TYPE_MISMATCH's rule, not this
                    // one. An undeclared target (a lambda writing its captured
                    // outer binding) is skipped the same way — no local info.
                    if name.starts_with('$') {
                        continue;
                    }
                    if muts.get(name) == Some(&false) {
                        self.emit(
                            "TYPE_ASSIGN_REQUIRES_MUT",
                            format!("Binding `{name}` is immutable and cannot be assigned."),
                        );
                    }
                    if let Some(t) = locals.get(name).cloned() {
                        let before = self.diags.borrow().len();
                        self.check_literal_range(resource_base_type(&t), value);
                        let range_errored = self.diags.borrow().len() > before;
                        if !range_errored {
                            self.check_assignment_type(name, &t, value, locals);
                        }
                    }
                }
                IrOp::AssignGlobal { name, value, .. } => {
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                    if self.global_muts.get(name) == Some(&false) {
                        self.emit(
                            "TYPE_ASSIGN_REQUIRES_MUT",
                            format!("Binding `{name}` is immutable and cannot be assigned."),
                        );
                    }
                    if let Some(t) = self.globals.get(name).cloned() {
                        let before = self.diags.borrow().len();
                        self.check_literal_range(resource_base_type(&t), value);
                        let range_errored = self.diags.borrow().len() > before;
                        if !range_errored {
                            self.check_assignment_type(name, &t, value, locals);
                        }
                    }
                }
                IrOp::StateAssign {
                    resource, value, ..
                } => {
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                    // `res.state = value` must match the declared `STATE T` type,
                    // carried in the local's type string (`File STATE T`).
                    if let Some(t) = locals.get(resource) {
                        if let Some(idx) = t.find(" STATE ") {
                            let state_type = t[idx + " STATE ".len()..].to_string();
                            if let Some(actual) = self.infer_type(value, locals) {
                                if !self.expression_compatible(&state_type, &actual, value) {
                                    self.emit(
                                        "TYPE_ASSIGNMENT_MISMATCH",
                                        format!(
                                            "State assignment to `{resource}.state` has type {actual}, expected {state_type}."
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
                IrOp::Eval { value, .. }
                | IrOp::ExitProgram { code: value, .. }
                | IrOp::Fail { error: value, .. } => {
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                }
                IrOp::Return { value, .. } => {
                    if let Some(value) = value {
                        self.check_value_captures(value, closure_slots);
                        self.check_value(value, locals);
                        self.check_return_type(value, locals);
                        let ret = self.current_return.borrow().clone();
                        self.check_literal_range(&ret, value);
                    }
                }
                IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
                IrOp::If {
                    condition,
                    then_body,
                    else_body,
                    ..
                } => {
                    self.check_value_captures(condition, closure_slots);
                    self.check_value(condition, locals);
                    self.check_condition_boolean("IF condition", condition, locals);
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    self.check_ops(
                        then_body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    self.check_ops(
                        else_body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                }
                IrOp::Match { value, cases, .. } => {
                    if cases.is_empty() {
                        self.emit(VERIFY_MATCH, "MATCH has no cases (not exhaustive)".to_string());
                    }
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                    self.check_match_exhaustive(value, cases, locals);
                    self.check_match_patterns(value, cases, locals);
                    self.current_line.set(line);
                    for case in cases {
                        match &case.pattern {
                            super::IrMatchPattern::Else => {}
                            super::IrMatchPattern::Value(v) => {
                                self.check_value(v, locals);
                            }
                            super::IrMatchPattern::OneOf(vs) => {
                                for v in vs {
                                    self.check_value(v, locals);
                                }
                            }
                        }
                        let mut case_locals = locals.clone();
                        let mut case_muts = muts.clone();
                        if let Some(guard) = &case.guard {
                            // A guard may reference the leading union-extract
                            // binds; register those first (mirrors validate.rs).
                            for op in &case.body {
                                let IrOp::Bind { name, type_, .. } = op else {
                                    break;
                                };
                                case_locals.insert(name.clone(), type_.clone());
                            }
                            self.check_value(guard, &case_locals);
                            self.current_line.set(case.loc.line);
                            self.check_condition_boolean("WHEN guard", guard, &case_locals);
                            self.current_line.set(line);
                            case_locals = locals.clone();
                        }
                        self.check_ops(
                            &case.body,
                            &mut case_locals,
                            &mut case_muts,
                            closure_slots,
                            depth + 1,
                        );
                        self.current_line.set(line);
                    }
                }
                IrOp::While {
                    condition, body, ..
                } => {
                    self.check_value_captures(condition, closure_slots);
                    self.check_value(condition, locals);
                    self.check_condition_boolean("WHILE condition", condition, locals);
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    self.check_ops(
                        body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                }
                IrOp::For {
                    name,
                    type_,
                    start,
                    end,
                    step,
                    body,
                    ..
                } => {
                    for value in [start, end, step] {
                        self.check_value_captures(value, closure_slots);
                        self.check_value(value, locals);
                    }
                    // The end/step values are bound to `$for` temps just before
                    // this op (the temp's own type is the promoted loop type,
                    // not the original expression's), so resolve each bound
                    // through `temp_consts` to judge the user's expression.
                    fn resolve<'v>(
                        v: &'v IrValue,
                        temp_consts: &HashMap<&str, &'v IrValue>,
                    ) -> Option<&'v IrValue> {
                        match v {
                            IrValue::Local(n) if n.starts_with('$') => {
                                temp_consts.get(n.as_str()).copied()
                            }
                            other => Some(other),
                        }
                    }
                    // A provably non-numeric bound cannot drive the counter.
                    for (label, bound) in [("start", start), ("end", end), ("step", step)] {
                        let Some(bound) = resolve(bound, &temp_consts) else {
                            continue;
                        };
                        let Some(actual) = self.infer_type(bound, locals) else {
                            continue;
                        };
                        // A local the lowering could not type carries the
                        // literal "Unknown" through the locals map — skip it
                        // like any other unreconstructable type.
                        if actual.is_empty() || actual == "Unknown" {
                            continue;
                        }
                        if !matches!(actual.as_str(), "Integer" | "Float" | "Byte" | "Fixed") {
                            self.emit(
                                "TYPE_FOR_REQUIRES_NUMERIC",
                                format!(
                                    "FOR loop {label} value has type {actual}, expected numeric."
                                ),
                            );
                        }
                    }
                    // A literal STEP of zero never advances the counter (a
                    // non-literal step is left alone, matching typecheck).
                    if resolve(step, &temp_consts).is_some_and(numeric_literal_is_zero) {
                        self.emit(
                            "TYPE_FOR_STEP_ZERO",
                            "FOR loop STEP must not be zero.".to_string(),
                        );
                    }
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    branch.insert(name.clone(), type_.clone());
                    // The loop counter is immutable inside the body (typecheck
                    // registers it `mutable: false`).
                    branch_muts.insert(name.clone(), false);
                    self.check_ops(
                        body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                }
                IrOp::DoUntil {
                    body, condition, ..
                } => {
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    self.check_ops(
                        body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                    // The trailing condition is reported at the loop's own line.
                    self.current_line.set(line);
                    self.check_value_captures(condition, closure_slots);
                    self.check_value(condition, locals);
                    self.check_condition_boolean("LOOP UNTIL condition", condition, locals);
                }
                IrOp::ForEach {
                    name,
                    type_,
                    iterable,
                    body,
                    ..
                } => {
                    self.check_value_captures(iterable, closure_slots);
                    self.check_value(iterable, locals);
                    // Only a List or Map can be iterated. (`MapEntry OF …` does
                    // not match the `Map OF ` prefix.)
                    if let Some(actual) = self.infer_type(iterable, locals) {
                        let base = resource_base_type(&actual);
                        // A local the lowering could not type carries the
                        // literal "Unknown" through the locals map — skip it.
                        if !base.is_empty()
                            && base != "Unknown"
                            && !base.starts_with("List OF ")
                            && !base.starts_with("Map OF ")
                        {
                            self.emit(
                                "TYPE_FOR_EACH_REQUIRES_COLLECTION",
                                format!("FOR EACH source has type {actual}, expected List or Map."),
                            );
                        }
                    }
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    branch.insert(name.clone(), type_.clone());
                    // The element binding is an immutable (borrowed) view.
                    branch_muts.insert(name.clone(), false);
                    self.check_ops(
                        body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                }
                IrOp::Trap { name, body, .. } => {
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    branch.insert(name.clone(), "Error".to_string());
                    branch_muts.insert(name.clone(), false);
                    self.check_ops(
                        body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                }
            }
        }
    }

    /// Enforce the semantic rules on a value expression and recurse into its
    /// sub-values. Argument and sub-expression checks run before the node's own
    /// rule so the innermost violation surfaces first.
    fn check_value(&self, value: &IrValue, locals: &HashMap<String, String>) {
        match value {
            IrValue::MemberAccess { target, member, .. } => {
                self.check_value(target, locals);
                self.check_member_access(target, member, locals);
            }
            IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
                for arg in args {
                    self.check_value(arg, locals);
                }
                self.check_call_arity(target, args.len(), locals);
                self.check_call_argument_types(target, args, locals);
                self.check_builtin_call_args(target, args, locals);
            }
            IrValue::Constructor { type_, args } => {
                for arg in args {
                    self.check_value(arg, locals);
                }
                self.check_constructor(type_, args, locals);
            }
            IrValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => {
                self.check_value(value, locals);
                self.check_union_wrap(union_type, member_type);
            }
            IrValue::Closure { captures, .. } => {
                for capture in captures {
                    self.check_value(capture, locals);
                }
            }
            IrValue::UnionExtract { value, .. }
            | IrValue::ResultIsOk { value }
            | IrValue::ResultValue { value, .. }
            | IrValue::ResultError { value } => {
                self.check_value(value, locals);
            }
            IrValue::Unary { op, operand, .. } => {
                self.check_value(operand, locals);
                self.check_unary_operand(op, operand, locals);
            }
            IrValue::Binary {
                op, left, right, ..
            } => {
                self.check_value(left, locals);
                self.check_value(right, locals);
                self.check_binary_operands(op, left, right, locals);
            }
            IrValue::WithUpdate {
                type_,
                target,
                updates,
            } => {
                self.check_value(target, locals);
                // Compiler/runtime-owned records may never be updated —
                // typecheck's TYPE_READ_ONLY_RECORD_UPDATE (message differs for
                // the Error pair vs the compiler-owned handle records). When
                // lowering could not stamp the update's type (e.g. the target
                // is a member access it didn't resolve), infer the target here.
                let inferred;
                let mut base = resource_base_type(type_);
                if base.is_empty() || base == "Unknown" {
                    inferred = self.infer_type(target, locals);
                    if let Some(t) = &inferred {
                        base = resource_base_type(t);
                    }
                }
                if matches!(base, "Error" | "ErrorLoc") {
                    self.emit(
                        "TYPE_READ_ONLY_RECORD_UPDATE",
                        format!("`{base}` is a read-only built-in record and cannot be updated."),
                    );
                } else if read_only_record_type(base) {
                    self.emit(
                        "TYPE_READ_ONLY_RECORD_UPDATE",
                        format!("TYPE `{base}` is read-only and cannot be updated."),
                    );
                }
                // Each WITH update must match its field's declared type —
                // typecheck's WITH arm of TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH.
                let fields = self.field_types.get(resource_base_type(type_));
                for update in updates {
                    self.check_value(&update.value, locals);
                    let Some(expected) = fields.and_then(|f| f.get(&update.field)) else {
                        continue;
                    };
                    let Some(actual) = self.infer_type(&update.value, locals) else {
                        continue;
                    };
                    if !self.expression_compatible(expected, &actual, &update.value) {
                        self.emit(
                            "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
                            format!(
                                "WITH update for `{}` has type {actual}, expected {expected}.",
                                update.field
                            ),
                        );
                    }
                }
            }
            IrValue::ListLiteral { type_, values } => {
                for v in values {
                    self.check_value(v, locals);
                }
                // A crafted list whose elements do not match its element type is
                // a type confusion: codegen lays out and reads elements
                // uniformly by the declared element type.
                if let Some(element) = type_.strip_prefix("List OF ") {
                    for v in values {
                        self.check_literal_range(element, v);
                        if let Some(actual) = self.infer_type(v, locals) {
                            if !self.expression_compatible(element, &actual, v) {
                                self.emit(
                                    "TYPE_LIST_ELEMENT_MISMATCH",
                                    format!("List element has type {actual}, expected {element}."),
                                );
                            }
                        }
                    }
                }
            }
            IrValue::MapLiteral { type_, entries } => {
                for (k, v) in entries {
                    self.check_value(k, locals);
                    self.check_value(v, locals);
                }
                self.check_map_key_comparable(type_);
                if let Some((key_type, value_type)) = parse_map(type_) {
                    for (k, v) in entries {
                        self.check_literal_range(key_type, k);
                        self.check_literal_range(value_type, v);
                        if let Some(actual) = self.infer_type(k, locals) {
                            if !self.expression_compatible(key_type, &actual, k) {
                                self.emit(
                                    "TYPE_MAP_KEY_MISMATCH",
                                    format!("Map key has type {actual}, expected {key_type}."),
                                );
                            }
                        }
                        if let Some(actual) = self.infer_type(v, locals) {
                            if !self.expression_compatible(value_type, &actual, v) {
                                self.emit(
                                    "TYPE_MAP_VALUE_MISMATCH",
                                    format!("Map value has type {actual}, expected {value_type}."),
                                );
                            }
                        }
                    }
                }
            }
            IrValue::Const { .. }
            | IrValue::Local(_)
            | IrValue::Global(_)
            | IrValue::LocalRef { .. }
            | IrValue::FunctionRef { .. }
            | IrValue::Capture { .. } => {}
        }
    }

    /// Check a numeric literal in a position that expects `expected` against
    /// that type's range (`typecheck`'s TYPE_*_LITERAL_OVERFLOW/UNDERFLOW).
    /// The check is contextual — keyed on the *expected* type, not the literal
    /// node's own type — because lowering does not push the expected type
    /// through a `-` negation (`-1` into `Byte` lowers to `Unary("-",
    /// Const{Integer,"1"})`, with `Byte` only on the enclosing bind). Matches
    /// the AST checker, which validates the literal against the expected type.
    fn check_literal_range(&self, expected: &str, value: &IrValue) {
        // Only a *numeric* literal can overflow a numeric range; a non-numeric
        // Const in a numeric position (e.g. a String arg where Integer is
        // expected) is an argument/assignment mismatch, not a literal overflow.
        let numeric = |t: &str| matches!(t, "Integer" | "Byte" | "Float" | "Fixed");
        match value {
            IrValue::Const { type_, value } if numeric(type_) => {
                self.check_const_literal(expected, value)
            }
            IrValue::Unary { op, operand, .. } if op == "-" => {
                if let IrValue::Const { type_, value } = operand.as_ref() {
                    if numeric(type_) {
                        self.check_negated_const_literal(expected, value);
                    }
                }
            }
            _ => {}
        }
    }

    /// The positive/overflow direction of the literal-range check.
    fn check_const_literal(&self, type_: &str, value: &str) {
        match type_ {
            "Byte" if !value.contains('.') => {
                if value.parse::<u16>().map_or(true, |n| n > u8::MAX as u16) {
                    self.emit(
                        "TYPE_BYTE_LITERAL_OVERFLOW",
                        format!("Integer literal `{value}` is outside the Byte range 0..255."),
                    );
                }
            }
            "Integer" if !value.contains('.') => {
                if value.parse::<i64>().is_err() {
                    self.emit(
                        "TYPE_INTEGER_LITERAL_OVERFLOW",
                        format!("Integer literal `{value}` is outside the Integer range."),
                    );
                }
            }
            "Float" => {
                if let Ok(f) = value.parse::<f64>() {
                    if !f.is_finite() {
                        self.emit(
                            "TYPE_FLOAT_LITERAL_OVERFLOW",
                            format!("Numeric literal `{value}` is outside the Float range."),
                        );
                    }
                }
            }
            "Fixed" => {
                if let Ok(f) = value.parse::<f64>() {
                    if f >= 2147483648.0 {
                        self.emit(
                            "TYPE_FIXED_LITERAL_OVERFLOW",
                            format!("Numeric literal `{value}` is outside the Fixed range."),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// The underflow direction of the literal-range check for a `-<literal>`.
    fn check_negated_const_literal(&self, type_: &str, value: &str) {
        match type_ {
            "Byte" if !value.contains('.') && value != "0" => {
                self.emit(
                    "TYPE_BYTE_LITERAL_UNDERFLOW",
                    format!("Integer literal `-{value}` is outside the Byte range 0..255."),
                );
            }
            "Integer" if !value.contains('.') => {
                if format!("-{value}").parse::<i64>().is_err() {
                    self.emit(
                        "TYPE_INTEGER_LITERAL_OVERFLOW",
                        format!("Integer literal `-{value}` is outside the Integer range."),
                    );
                }
            }
            "Fixed" => {
                if let Ok(f) = value.parse::<f64>() {
                    if -f < -2147483648.0 {
                        self.emit(
                            "TYPE_FIXED_LITERAL_UNDERFLOW",
                            format!("Numeric literal `-{value}` is outside the Fixed range."),
                        );
                    }
                }
            }
            "Float" => {
                if let Ok(f) = value.parse::<f64>() {
                    if !(-f).is_finite() {
                        self.emit(
                            "TYPE_FLOAT_LITERAL_UNDERFLOW",
                            format!("Numeric literal `-{value}` is outside the Float range."),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// Reject a `MemberAccess` whose target provably cannot carry the member: a
    /// primitive-typed target, or a known record that does not declare it.
    fn check_member_access(
        &self,
        target: &IrValue,
        member: &str,
        locals: &HashMap<String, String>,
    ) {
        let Some(type_name) = self.infer_type(target, locals) else {
            return;
        };
        if PRIMITIVE_TYPES.contains(&type_name.as_str()) {
            self.emit(
                "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
                format!("field access requires a record value, got `{type_name}`."),
            );
            return;
        }
        // Only a record can be member-accessed. When the target resolves to a
        // record whose complete field set is known, the member must be present;
        // otherwise (collections, unions, unresolved includes, unknown types)
        // the access is left unchecked.
        if let Some(fields) = self.record_fields(&type_name) {
            if !fields.contains(member) {
                self.emit(
                    "TYPE_UNKNOWN_FIELD",
                    format!("record `{type_name}` has no member `{member}`."),
                );
            }
        }
    }

    /// Reject a binary operator applied to operands whose types it cannot
    /// accept — the IR-level counterpart of `typecheck`'s `infer_binary`
    /// operand rule (`TYPE_BINARY_OPERATOR_MISMATCH` / `TYPE_REQUIRES_COMPARABLE`).
    /// On decoded package IR this is a memory-safety gate: codegen selects the
    /// machine instruction from the operand *types*, so a crafted `String - Integer`
    /// would emit an integer subtract over a string pointer (pointer arithmetic
    /// on attacker data). Only rejects when both operand types are known and
    /// provably incompatible; `Unknown` is treated as any type (matching
    /// `is_numeric(Unknown) == true`), so no valid program is ever rejected.
    fn check_binary_operands(
        &self,
        op: &str,
        left: &IrValue,
        right: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        let (Some(lt), Some(rt)) = (self.infer_type(left, locals), self.infer_type(right, locals))
        else {
            return; // an operand type is unknown → skip (no false reject)
        };
        let numeric = |t: &str| matches!(t, "Integer" | "Byte" | "Float" | "Fixed" | "Unknown");
        let string = |t: &str| matches!(t, "String" | "Unknown");
        let boolean = |t: &str| matches!(t, "Boolean" | "Unknown");
        let ok = match op {
            "AND" | "OR" | "XOR" => boolean(&lt) && boolean(&rt),
            "&" => string(&lt) && string(&rt),
            "<" | ">" | "<=" | ">=" => {
                (numeric(&lt) && numeric(&rt)) || (string(&lt) && string(&rt))
            }
            // Equality (`=`/`<>`): numeric pairs compare, otherwise both
            // operands must be compatible AND comparable. A crafted comparison
            // of non-comparable values (collections, functions, resources,
            // unions) would mislead codegen's comparison lowering.
            "=" | "<>" => {
                if numeric(&lt) && numeric(&rt) {
                    true
                } else if self.compatible(&lt, &rt) || self.compatible(&rt, &lt) {
                    self.is_comparable(&lt) && self.is_comparable(&rt)
                } else {
                    // Incompatible operands: an operator mismatch, not a
                    // comparability failure — reported below with the right id.
                    false
                }
            }
            // Everything else is arithmetic / bitwise: numeric operands only.
            _ => numeric(&lt) && numeric(&rt),
        };
        if !ok {
            if matches!(op, "=" | "<>") {
                // Compatible-but-not-comparable is a comparability failure;
                // incompatible operands are an operator mismatch.
                let rule = if self.compatible(&lt, &rt) || self.compatible(&rt, &lt) {
                    "TYPE_REQUIRES_COMPARABLE"
                } else {
                    "TYPE_BINARY_OPERATOR_MISMATCH"
                };
                self.emit(
                    rule,
                    format!(
                        "Operator `{op}` requires compatible comparable operands, got {lt} and {rt}."
                    ),
                );
                return;
            }
            let requirement = match op {
                "AND" | "OR" | "XOR" => "Boolean operands",
                "&" => "String operands",
                "<" | ">" | "<=" | ">=" => "numeric or String operands",
                _ => "numeric operands",
            };
            self.emit(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                format!("Operator `{op}` requires {requirement}, got {lt} and {rt}."),
            );
        }
    }

    /// Whether a value of type `type_` can be compared for equality
    /// (`typecheck::is_comparable`): primitives/enums yes; collections,
    /// functions, results, resources, and unions no; a record only if every
    /// field is comparable. `Unknown` is comparable (never a false rejection).
    fn is_comparable(&self, type_: &str) -> bool {
        self.is_comparable_seen(resource_base_type(type_), &mut HashSet::new())
    }

    /// Every `Map OF K TO V` nested anywhere in `type_` must have a comparable
    /// key — `typecheck`'s map-key arm of `TYPE_REQUIRES_COMPARABLE` (an
    /// incomparable key breaks the map's hash/equality contract at runtime).
    fn check_map_key_comparable(&self, type_: &str) {
        let t = resource_base_type(type_);
        if let Some(inner) = t.strip_prefix("List OF ") {
            self.check_map_key_comparable(inner);
            return;
        }
        if let Some((key, value)) = parse_map(t) {
            if !key.is_empty() && key != "Unknown" && !self.is_comparable(key) {
                self.emit(
                    "TYPE_REQUIRES_COMPARABLE",
                    format!("Map key type requires a comparable type, got `{key}`."),
                );
            }
            self.check_map_key_comparable(key);
            self.check_map_key_comparable(value);
        }
    }

    fn is_comparable_seen(&self, type_: &str, seen: &mut HashSet<String>) -> bool {
        match type_ {
            "Boolean" | "Byte" | "Error" | "ErrorLoc" | "Fixed" | "Float" | "Integer"
            | "Nothing" | "String" | "Unknown" => return true,
            _ => {}
        }
        if type_.starts_with("List OF ")
            || type_.starts_with("Map OF ")
            || type_.starts_with("Result OF ")
            || type_.starts_with("FUNC(")
            || type_.starts_with("Thread ")
            || type_.starts_with("ThreadWorker ")
        {
            return false;
        }
        if is_resource_name(type_) {
            return false;
        }
        if self.unions.contains_key(type_) {
            return false;
        }
        if self.enums.contains_key(type_) {
            return true;
        }
        if !seen.insert(type_.to_string()) {
            return false; // a cycle → not a base case
        }
        if let Some(fields) = self.field_types.get(type_) {
            let all = fields
                .values()
                .all(|ft| self.is_comparable_seen(resource_base_type(ft), seen));
            seen.remove(type_);
            return all;
        }
        // Unknown user type — permissive (no false rejection).
        true
    }

    /// Structural well-formedness of the type table (`typecheck`'s
    /// `check_type_decl`), checkable directly on the IR. On decoded package IR
    /// these guard codegen's layout and drop assumptions: a record that owns a
    /// resource field, a union mixing data and resource variants (tag-dependent
    /// copyability / drop dispatch), or a record with no base case (infinite
    /// size) would all mislead the layout/drop lowering. Reported at the type
    /// declaration line; the file is unset (a decoded package has no source).
    fn check_type_declarations(&self, project: &IrProject) {
        for ty in &project.types {
            self.current_file.replace(ty.file.clone());
            self.current_line.set(ty.loc.line);
            match ty.kind.as_str() {
                "type" | "record" => {
                    for field in &ty.fields {
                        self.current_line.set(field.loc.line);
                        self.check_map_key_comparable(&field.type_);
                        self.current_line.set(ty.loc.line);
                        if is_resource_name(resource_base_type(&field.type_)) {
                            self.current_line.set(field.loc.line);
                            self.emit(
                                "TYPE_RESOURCE_FIELD_FORBIDDEN",
                                format!(
                                    "Record `{}` field `{}` is resource `{}`; records cannot own resources.",
                                    ty.name, field.name, field.type_
                                ),
                            );
                            self.current_line.set(ty.loc.line);
                        }
                    }
                    if self.record_field_cycle(&ty.name, &ty.name, &mut HashSet::new()) {
                        self.emit(
                            "TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION",
                            format!(
                                "Record `{}` refers to itself without passing through a List, Map, or UNION; such a record has no base case and cannot be constructed.",
                                ty.name
                            ),
                        );
                    }
                }
                "union" => {
                    // `INCLUDES` may only name other unions. A name that is a
                    // known non-union type (record or enum) is a malformed
                    // include. (Undeclared names are a different, resolve-time
                    // rule, so only reject names the IR positively knows.)
                    for include in &ty.includes {
                        if !self.unions.contains_key(include)
                            && (self.records.contains_key(include)
                                || self.enums.contains_key(include))
                        {
                            self.emit(
                                "TYPE_UNION_INCLUDE_REQUIRES_UNION",
                                format!(
                                    "UNION `{}` includes `{}`, but `{}` is not a UNION.",
                                    ty.name, include, include
                                ),
                            );
                        }
                    }
                    // Each named member must be a concrete TYPE (record). A
                    // member that is itself a union or an enum is not a concrete
                    // type. (Records-registered variant names are fine; only a
                    // name that is *also* a declared union/enum is rejected.)
                    for variant in &ty.variants {
                        if self.unions.contains_key(&variant.name)
                            || self.enums.contains_key(&variant.name)
                        {
                            self.current_line.set(variant.loc.line);
                            self.emit(
                                "TYPE_UNION_MEMBER_REQUIRES_TYPE",
                                format!(
                                    "UNION `{}` member `{}` must be a concrete TYPE.",
                                    ty.name, variant.name
                                ),
                            );
                            self.current_line.set(ty.loc.line);
                        }
                    }
                    self.check_union_include_conflicts(ty);
                    self.current_line.set(ty.loc.line);
                    let resource_variants = ty
                        .variants
                        .iter()
                        .filter(|v| is_resource_name(&v.name))
                        .count();
                    if resource_variants > 0 && resource_variants < ty.variants.len() {
                        self.emit(
                            "TYPE_MIXED_RESOURCE_UNION",
                            format!(
                                "UNION `{}` mixes data and resource variants; a union must be all-data or all-resource.",
                                ty.name
                            ),
                        );
                    }
                }
                "enum" => {
                    if ty.members.is_empty() {
                        self.emit(
                            "TYPE_ENUM_REQUIRES_MEMBER",
                            format!("ENUM `{}` must declare at least one member.", ty.name),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// The full member-name set of `union_name`, expanding every `INCLUDES`d
    /// union transitively (cycle-guarded). Mirrors `typecheck`'s
    /// `expanded_union_variants`, but names only — dup detection needs no fields.
    fn expanded_union_variant_names(
        &self,
        union_name: &str,
        visiting: &mut HashSet<String>,
    ) -> Vec<String> {
        if !visiting.insert(union_name.to_string()) {
            return Vec::new();
        }
        let mut names = Vec::new();
        if let Some(info) = self.unions.get(union_name) {
            for include in &info.includes {
                names.extend(self.expanded_union_variant_names(include, visiting));
            }
            names.extend(info.variants.iter().cloned());
        }
        visiting.remove(union_name);
        names
    }

    /// `typecheck::report_expanded_union_member_conflicts` on the IR: a union
    /// member may not be provided by two different includes, nor by both an
    /// include and a local declaration. On decoded package IR a duplicated
    /// variant is an ambiguous tag → mis-dispatch, so this must run here too.
    fn check_union_include_conflicts(&self, ty: &IrType) {
        let Some(info) = self.unions.get(&ty.name) else {
            return;
        };
        // A member provided by two distinct includes.
        let mut included_members: HashMap<String, String> = HashMap::new();
        for include in &info.includes {
            let mut visiting = HashSet::new();
            for name in self.expanded_union_variant_names(include, &mut visiting) {
                if let Some(previous) = included_members.insert(name.clone(), include.clone()) {
                    self.current_line.set(ty.loc.line);
                    self.emit(
                        "TYPE_DUPLICATE_VARIANT",
                        format!(
                            "Member type `{}` in UNION `{}` is provided by both included UNION `{}` and included UNION `{}`.",
                            name, ty.name, previous, include
                        ),
                    );
                }
            }
        }
        // A local variant that collides with an included member.
        for variant in &ty.variants {
            if let Some(include) = included_members.get(&variant.name) {
                self.current_line.set(variant.loc.line);
                self.emit(
                    "TYPE_DUPLICATE_VARIANT",
                    format!(
                        "Member type `{}` in UNION `{}` conflicts with a member included from UNION `{}`.",
                        variant.name, ty.name, include
                    ),
                );
            }
        }
    }

    /// Whether `record` reaches `target` through a chain of direct record-typed
    /// fields (no List/Map/Union indirection) — i.e. an infinitely-sized record.
    fn record_field_cycle(&self, record: &str, target: &str, seen: &mut HashSet<String>) -> bool {
        if !seen.insert(record.to_string()) {
            return false;
        }
        let Some(fields) = self.field_types.get(record) else {
            return false;
        };
        for field_type in fields.values() {
            // Only *direct* record fields propagate the cycle; a List/Map/Union
            // field is a legitimate base-case indirection.
            let base = resource_base_type(field_type);
            if base == target {
                return true;
            }
            if self.records.contains_key(base) && self.record_field_cycle(base, target, seen) {
                return true;
            }
        }
        false
    }

    /// Reject a read of a resource binding after it was moved (closed, returned)
    /// — `typecheck`'s `TYPE_USE_AFTER_MOVE`. On decoded package IR a
    /// use-after-move is a use-after-free / double-free: the resource's backing
    /// handle is released by the move, so a later read hands codegen a dangling
    /// handle. Conservative straight-line dataflow: a move is only tracked
    /// within a linear op sequence (nested blocks get a fresh copy that does not
    /// leak moves back out), so no valid program is ever rejected; it catches
    /// the common close-then-use and double-close. Consumption = a call to the
    /// resource type's registered close op with the binding as its first
    /// argument, or `RETURN <resource>`.
    fn check_resource_moves(
        &self,
        ops: &[IrOp],
        locals: &mut HashMap<String, String>,
        moved: &mut HashSet<String>,
    ) {
        for op in ops {
            self.current_line.set(op.loc().line);
            // A read of an already-moved binding is a use-after-move. The
            // consuming op reads the binding too, but at that point it is not
            // yet in `moved` (we insert below), so the consume itself is fine
            // and a *second* consume (double close) is correctly flagged.
            let mut reads = Vec::new();
            collect_local_reads_op(op, &mut reads);
            for name in &reads {
                if moved.contains(name) {
                    self.emit(
                        "TYPE_USE_AFTER_MOVE",
                        format!("Binding `{name}` was moved and cannot be used again."),
                    );
                }
            }
            if let Some(consumed) = self.consumed_resource(op, locals) {
                moved.insert(consumed);
            }
            match op {
                IrOp::Bind {
                    name, type_, value, ..
                } => {
                    // A rebind of a resource name reopens ownership.
                    if value.is_some() {
                        moved.remove(name);
                    }
                    locals.insert(name.clone(), type_.clone());
                }
                IrOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    self.check_resource_moves(then_body, &mut locals.clone(), &mut moved.clone());
                    self.check_resource_moves(else_body, &mut locals.clone(), &mut moved.clone());
                }
                IrOp::Match { cases, .. } => {
                    for case in cases {
                        self.check_resource_moves(
                            &case.body,
                            &mut locals.clone(),
                            &mut moved.clone(),
                        );
                    }
                }
                IrOp::While { body, .. }
                | IrOp::For { body, .. }
                | IrOp::DoUntil { body, .. }
                | IrOp::ForEach { body, .. }
                | IrOp::Trap { body, .. } => {
                    self.check_resource_moves(body, &mut locals.clone(), &mut moved.clone());
                }
                _ => {}
            }
        }
    }

    /// The resource binding consumed by an op, if any: a call to the binding's
    /// registered close op with it as the first argument, or `RETURN <binding>`.
    fn consumed_resource(&self, op: &IrOp, locals: &HashMap<String, String>) -> Option<String> {
        let close_consumes = |value: &IrValue| -> Option<String> {
            let (target, args) = match value {
                IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
                    (target, args)
                }
                _ => return None,
            };
            // NOTE: thread::transfer is intentionally NOT treated as a move
            // here. On the failure path of `transfer(...) TRAP(e)` ownership
            // returns to the sender so the handler may close the resource — a
            // straight-line detector cannot see that and would false-reject the
            // valid recover pattern. typecheck models the restore explicitly;
            // the IR checker stays conservative and only tracks close/return.
            // A registered close op consumes the resource at arg 0.
            let IrValue::Local(name) = args.first()? else {
                return None;
            };
            let type_ = locals.get(name)?;
            let base = resource_base_type(type_);
            if builtins::resource::builtin_resource_close_function(base) == Some(target.as_str()) {
                Some(name.clone())
            } else {
                None
            }
        };
        match op {
            IrOp::Eval { value, .. } => close_consumes(value),
            IrOp::Bind {
                value: Some(value), ..
            } => close_consumes(value),
            IrOp::Assign { value, .. } => close_consumes(value),
            IrOp::Return {
                value: Some(IrValue::Local(name)),
                ..
            } => {
                let type_ = locals.get(name)?;
                if builtins::resource::builtin_resource_close_function(resource_base_type(type_))
                    .is_some()
                {
                    Some(name.clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Reject a `MATCH` on an enum or union that neither covers every
    /// member/variant nor has an unguarded catch-all (`typecheck`'s
    /// `TYPE_MATCH_NOT_EXHAUSTIVE`). On decoded package IR this is a
    /// memory-safety gate: a non-exhaustive match falls through with no arm
    /// selected, leaving a typed value uninitialized. Only checked when the
    /// scrutinee resolves to a known enum/union with a complete member set
    /// (Result-matches lower to a Boolean flag and are skipped); guarded cases
    /// do not count toward coverage, matching the source rule.
    fn check_match_exhaustive(
        &self,
        value: &IrValue,
        cases: &[super::IrMatchCase],
        locals: &HashMap<String, String>,
    ) {
        let Some(ty) = self.infer_type(value, locals) else {
            return;
        };
        let ty = resource_base_type(&ty).to_string();
        // A Result scrutinee's CASE Ok/Error arms are rejected by
        // TYPE_RESULT_NOT_MATCHABLE; suppress the secondary exhaustiveness
        // cascade like typecheck does. Unknown types are skipped as always.
        if ty.is_empty() || ty == "Unknown" || ty == "Result" || ty.starts_with("Result OF ") {
            return;
        }
        // The complete member/variant set, and whether it is a union (for the
        // diagnostic wording). Any other *known* type is an open type: only an
        // unguarded CASE ELSE can make its MATCH exhaustive.
        let (all, is_union) = if let Some(variants) = self.union_variants(&ty) {
            (variants, true)
        } else if let Some(members) = self.enums.get(&ty) {
            (members.clone(), false)
        } else {
            if !cases.iter().any(|case| {
                case.guard.is_none() && matches!(case.pattern, super::IrMatchPattern::Else)
            }) {
                self.emit(
                    "TYPE_MATCH_NOT_EXHAUSTIVE",
                    format!("MATCH on open type {ty} requires an unguarded CASE ELSE."),
                );
            }
            return;
        };
        let pattern_name = |v: &IrValue| -> Option<String> {
            match v {
                IrValue::Local(name) => Some(name.clone()),
                IrValue::MemberAccess { member, .. } => Some(member.clone()),
                _ => None,
            }
        };
        let mut covered: HashSet<String> = HashSet::new();
        for case in cases {
            if case.guard.is_some() {
                continue; // a guarded arm may not fire → does not cover
            }
            match &case.pattern {
                super::IrMatchPattern::Else => return, // unguarded catch-all
                super::IrMatchPattern::Value(v) => {
                    if let Some(name) = pattern_name(v) {
                        covered.insert(name);
                    }
                }
                super::IrMatchPattern::OneOf(vs) => {
                    for v in vs {
                        if let Some(name) = pattern_name(v) {
                            covered.insert(name);
                        }
                    }
                }
            }
        }
        if all.difference(&covered).next().is_none() {
            return;
        }
        // Missing-member lists mirror typecheck's wording exactly: unions list
        // the uncovered variants in declaration order; enums list sorted
        // `Type.member` names.
        let missing = if is_union {
            let mut ordered: Vec<String> = self
                .unions
                .get(&ty)
                .map(|info| {
                    info.variant_order
                        .iter()
                        .filter(|v| !covered.contains(*v))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            // Variants reached through INCLUDES have no declaration slot here;
            // append them sorted so the list is complete and deterministic.
            let mut extra: Vec<String> = all
                .difference(&covered)
                .filter(|v| !ordered.contains(v))
                .cloned()
                .collect();
            extra.sort();
            ordered.extend(extra);
            ordered.join(", ")
        } else {
            let mut members: Vec<String> = all
                .difference(&covered)
                .map(|m| format!("{ty}.{m}"))
                .collect();
            members.sort();
            members.join(", ")
        };
        let detail = if is_union {
            format!(
                "MATCH on UNION `{ty}` does not cover {missing}; add unguarded CASE arms or CASE ELSE."
            )
        } else {
            format!(
                "MATCH on enum `{ty}` does not cover {missing}; add unguarded CASE arms or CASE ELSE."
            )
        };
        self.emit("TYPE_MATCH_NOT_EXHAUSTIVE", detail);
    }

    /// `typecheck`'s `TYPE_MATCH_PATTERN_MISMATCH` on the IR: a CASE pattern
    /// must fit the scrutinee — a union CASE must name one of the union's
    /// variants, a type-named CASE requires a union scrutinee, and a literal
    /// pattern's type must be compatible with the scrutinee type. Unknown
    /// scrutinee or pattern types are skipped (sound skip-if-unknown).
    fn check_match_patterns(
        &self,
        value: &IrValue,
        cases: &[super::IrMatchCase],
        locals: &HashMap<String, String>,
    ) {
        let Some(scrutinee) = self.infer_type(value, locals) else {
            return;
        };
        let scrutinee = resource_base_type(&scrutinee).to_string();
        if scrutinee.is_empty() || scrutinee == "Unknown" {
            return;
        }
        let union_variants = self.union_variants(&scrutinee);
        let check_pattern = |v: &IrValue| {
            // A pattern that names a declared type is a union-variant arm.
            let type_name = match v {
                IrValue::Local(name) => Some(name),
                IrValue::MemberAccess { member, .. } => Some(member),
                _ => None,
            }
            .filter(|n| {
                self.records.contains_key(n.as_str())
                    || self.unions.contains_key(n.as_str())
                    || self.enums.contains_key(n.as_str())
            });
            if let Some(type_name) = type_name {
                match &union_variants {
                    Some(variants) => {
                        if !variants.contains(type_name.as_str()) {
                            self.emit(
                                "TYPE_MATCH_PATTERN_MISMATCH",
                                format!(
                                    "CASE `{type_name}` is not a member of UNION `{scrutinee}`."
                                ),
                            );
                        }
                    }
                    None => {
                        // An enum scrutinee's member arms share member names
                        // with no type; a declared-type CASE against any
                        // non-union scrutinee is malformed.
                        self.emit(
                            "TYPE_MATCH_PATTERN_MISMATCH",
                            format!("CASE `{type_name}` requires a UNION value, got {scrutinee}."),
                        );
                    }
                }
                return;
            }
            // A literal (or expression) pattern: its type must fit the
            // scrutinee. Enum member arms are Local names with no local type
            // (infer_type -> None), so they fall through harmlessly here.
            if let Some(pattern_type) = self.infer_type(v, locals) {
                if !self.expression_compatible(&scrutinee, &pattern_type, v) {
                    self.emit(
                        "TYPE_MATCH_PATTERN_MISMATCH",
                        format!("CASE pattern has type {pattern_type}, expected {scrutinee}."),
                    );
                }
            }
        };
        for case in cases {
            self.current_line.set(case.loc.line);
            match &case.pattern {
                super::IrMatchPattern::Else => {}
                super::IrMatchPattern::Value(v) => check_pattern(v),
                super::IrMatchPattern::OneOf(vs) => {
                    for v in vs {
                        check_pattern(v);
                    }
                }
            }
        }
    }

    /// The unary counterpart of `check_binary_operands` (`typecheck`'s
    /// `infer_unary` / `TYPE_UNARY_OPERATOR_MISMATCH`): `NOT` requires a Boolean
    /// operand, unary `-` a numeric one. Same memory-safety rationale — codegen
    /// picks the instruction from the operand type. `Unknown` never rejects.
    fn check_unary_operand(&self, op: &str, operand: &IrValue, locals: &HashMap<String, String>) {
        let Some(t) = self.infer_type(operand, locals) else {
            return;
        };
        match op {
            "NOT" => {
                if !matches!(t.as_str(), "Boolean" | "Unknown") {
                    self.emit(
                        "TYPE_UNARY_OPERATOR_MISMATCH",
                        format!("Operator `NOT` requires a Boolean operand, got {t}."),
                    );
                }
            }
            "-" => {
                if !matches!(t.as_str(), "Integer" | "Byte" | "Float" | "Fixed" | "Unknown") {
                    self.emit(
                        "TYPE_UNARY_OPERATOR_MISMATCH",
                        format!("Unary `-` requires a numeric operand, got {t}."),
                    );
                }
            }
            other => {
                self.emit(
                    "TYPE_UNARY_OPERATOR_UNKNOWN",
                    format!("Unknown unary operator `{other}`."),
                );
            }
        }
    }

    /// Reject a direct call whose argument count cannot match the callee's
    /// signature. Only internal functions have a known signature; builtins,
    /// runtime helpers, imports and indirect (function-typed local) calls are
    /// skipped.
    fn check_call_arity(&self, target: &str, argc: usize, locals: &HashMap<String, String>) {
        if locals.contains_key(target) {
            // A local of function type — an indirect call; its arity is the
            // function type's, not a named signature.
            return;
        }
        let Some(sig) = self.functions.get(target) else {
            return;
        };
        let required = sig.total.saturating_sub(sig.optional);
        if argc < required || argc > sig.total {
            self.emit(
                "TYPE_CALL_ARITY_MISMATCH",
                format!(
                    "Call to `{target}` has {argc} argument(s), expected {required}..={}.",
                    sig.total
                ),
            );
        }
    }

    /// Reject a call to a known user function whose argument types are
    /// incompatible with the declared parameter types (`typecheck`'s
    /// `TYPE_CALL_ARGUMENT_MISMATCH`). On decoded package IR this is an ABI-level
    /// type confusion: codegen marshals each argument by its declared parameter
    /// type, so a crafted `String` passed where an `Integer` is expected is read
    /// as an integer at the callee boundary. Lowering has already normalized the
    /// call (positional, defaults filled, union members wrapped), so a direct
    /// arg-type-vs-param-type comparison is faithful. `Unknown` never rejects.
    fn check_call_argument_types(
        &self,
        target: &str,
        args: &[IrValue],
        locals: &HashMap<String, String>,
    ) {
        if locals.contains_key(target) {
            return; // indirect call — no named signature
        }
        let Some(sig) = self.functions.get(target) else {
            return;
        };
        for (index, arg) in args.iter().enumerate() {
            let Some(param_type) = sig.params.get(index) else {
                break;
            };
            let Some(actual) = self.infer_type(arg, locals) else {
                continue;
            };
            // Strip a resource argument's `STATE T` clause; the parameter type
            // is the bare resource type.
            let actual = resource_base_type(&actual).to_string();
            let param_type = resource_base_type(param_type);
            self.check_literal_range(param_type, arg);
            if !self.expression_compatible(param_type, &actual, arg) {
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Argument {} for `{target}` has type {actual}, expected {param_type}.",
                        index + 1
                    ),
                );
            }
        }
    }

    /// Reject a call to a numeric built-in whose argument types match no
    /// overload — the IR-level counterpart of `typecheck`'s per-built-in
    /// `TYPE_CALL_ARGUMENT_MISMATCH`, reusing the *same* `resolve_call` dispatch
    /// the compiler already uses for return-type inference (so there is one
    /// source of truth for the argument rules, not a re-implementation). On
    /// decoded package IR a crafted `math.sqrt("x")` would otherwise reach
    /// codegen, which selects the float instruction from the declared numeric
    /// type. Restricted to the pure-numeric packages (math/bits) where the
    /// arguments are ordinary values with no receiver/predicate normalization,
    /// so `resolve_call`'s None is unambiguously an argument mismatch. Skipped
    /// unless every argument type is known (no false rejection).
    fn check_builtin_call_args(
        &self,
        target: &str,
        args: &[IrValue],
        locals: &HashMap<String, String>,
    ) {
        // `collections` element searches compare elements for equality, so the
        // list's element type must be comparable — typecheck's
        // `check_special_builtin_arguments` arm of TYPE_REQUIRES_COMPARABLE.
        if matches!(
            target,
            "collections.contains" | "collections.replace" | "collections.find"
        ) {
            if let Some(first) = args.first() {
                if let Some(t) = self.infer_type(first, locals) {
                    if let Some(element) = resource_base_type(&t).strip_prefix("List OF ") {
                        if element != "Unknown" && !self.is_comparable(element) {
                            self.emit(
                                "TYPE_REQUIRES_COMPARABLE",
                                format!(
                                    "Call to `{target}` requires a comparable type, got `{element}`."
                                ),
                            );
                        }
                    }
                }
            }
        }
        // Strip the `STATE T` clause a resource argument carries in its type
        // string (`File STATE FileState` → `File`); resolve_call and the
        // parameter tables use the bare resource type.
        let arg_types: Option<Vec<String>> = args
            .iter()
            .map(|a| self.infer_type(a, locals).map(|t| resource_base_type(&t).to_string()))
            .collect();
        let Some(arg_types) = arg_types else {
            return;
        };
        // `term` exposes its per-name signatures (`arity`, `param_types`)
        // rather than an arg-typed `resolve_call`, so check against those with
        // the ported `expression_compatible` — the same data typecheck's
        // `check_term_builtin_call` uses, so term's signature is single-source.
        if builtins::term::is_term_call(target) {
            if let Some((min, max)) = builtins::term::arity(target) {
                if arg_types.len() < min || arg_types.len() > max {
                    let expected = if min == max {
                        min.to_string()
                    } else {
                        format!("{min} to {max}")
                    };
                    self.emit(
                        "TYPE_CALL_ARITY_MISMATCH",
                        format!(
                            "Call to `{target}` has {} argument(s), expected {expected}.",
                            arg_types.len()
                        ),
                    );
                    return;
                }
            }
            let params = builtins::term::param_types(target).unwrap_or(&[]);
            let mut mismatch = false;
            for (i, param) in params.iter().enumerate() {
                if let (Some(actual), Some(arg)) = (arg_types.get(i), args.get(i)) {
                    if !self.expression_compatible(param, actual, arg) {
                        mismatch = true;
                    }
                }
            }
            if mismatch {
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!("Call to `{target}` has argument type(s) that do not match its signature."),
                );
            }
            return;
        }
        // `collections`/`general` builtins: per-name arity, then arg-typed
        // overload resolution (typecheck's check_general_builtin_call arms).
        if builtins::collections::is_collections_call(target) {
            if let Some((min, max)) = builtins::collections::arity(target) {
                if arg_types.len() < min || arg_types.len() > max {
                    let expected = if min == max {
                        min.to_string()
                    } else {
                        format!("{min} to {max}")
                    };
                    self.emit(
                        "TYPE_CALL_ARITY_MISMATCH",
                        format!(
                            "Call to `{target}` has {} argument(s), expected {expected}.",
                            arg_types.len()
                        ),
                    );
                    return;
                }
            }
            if builtins::collections::resolve_call(target, &arg_types).is_none() {
                let expected = builtins::collections::expected_arguments(target)
                    .unwrap_or("supported overload");
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Call to `{target}` has argument type(s) ({}), expected {expected}.",
                        arg_types.join(", ")
                    ),
                );
            }
            return;
        }
        if builtins::general::is_general_call(target) {
            if let Some((min, max)) = builtins::general::arity(target) {
                if arg_types.len() < min || arg_types.len() > max {
                    let expected = if min == max {
                        min.to_string()
                    } else {
                        format!("{min} to {max}")
                    };
                    self.emit(
                        "TYPE_CALL_ARITY_MISMATCH",
                        format!(
                            "Call to `{target}` has {} argument(s), expected {expected}.",
                            arg_types.len()
                        ),
                    );
                    return;
                }
            }
            if builtins::general::resolve_call(target, &arg_types).is_none() {
                // A package-provided override may accept what the built-in
                // rejects (plan-01-overload §A.3.2) — never reject those.
                if builtins::general::is_overridable(target)
                    && arg_types.len() == 1
                    && builtins::general_override_target(target, &arg_types[0]).is_some()
                {
                    return;
                }
                let expected =
                    builtins::general::expected_arguments(target).unwrap_or("supported overload");
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Call to `{target}` has argument type(s) ({}), expected {expected}.",
                        arg_types.join(", ")
                    ),
                );
            }
            return;
        }
        let unresolved = if builtins::math::is_math_call(target) {
            builtins::math::resolve_call(target, &arg_types).is_none()
        } else if builtins::bits::is_bits_call(target) {
            builtins::bits::resolve_call(target, &arg_types).is_none()
        } else if builtins::vector::is_vector_call(target) {
            builtins::vector::resolve_call(target, &arg_types).is_none()
        } else if builtins::strings::is_strings_call(target) {
            builtins::strings::resolve_call(target, &arg_types).is_none()
        } else if builtins::encoding::is_encoding_call(target) {
            builtins::encoding::resolve_call(target, &arg_types).is_none()
        } else if builtins::io::is_io_call(target) {
            builtins::io::resolve_call(target, &arg_types).is_none()
        } else if builtins::fs::is_fs_call(target) {
            builtins::fs::resolve_call(target, &arg_types).is_none()
        } else if builtins::net::is_net_call(target) {
            builtins::net::resolve_call(target, &arg_types).is_none()
        } else {
            return;
        };
        if unresolved {
            self.emit(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                format!("Arguments to `{target}` do not match any overload."),
            );
        }
    }

    /// Type compatibility (`typecheck::compatible`), on canonical type-name
    /// strings. `Unknown` on either side is compatible; the `RES` ownership
    /// marker is stripped; container types recurse; a union accepts any of its
    /// variants. Anything unresolved falls back to string equality (never a
    /// false rejection because callers gate on both types being known).
    fn compatible(&self, expected: &str, actual: &str) -> bool {
        if expected == "Unknown" || actual == "Unknown" {
            return true;
        }
        let expected = expected.strip_prefix("RES ").unwrap_or(expected);
        let actual = actual.strip_prefix("RES ").unwrap_or(actual);
        if expected == actual {
            return true;
        }
        if let (Some(e), Some(a)) = (
            expected.strip_prefix("List OF "),
            actual.strip_prefix("List OF "),
        ) {
            return self.compatible(e, a);
        }
        if let (Some(e), Some(a)) = (
            expected.strip_prefix("Result OF "),
            actual.strip_prefix("Result OF "),
        ) {
            return self.compatible(e, a);
        }
        if let (Some((ek, ev)), Some((ak, av))) = (parse_map(expected), parse_map(actual)) {
            return self.compatible(ek, ak) && self.compatible(ev, av);
        }
        // Bare-name equality (an imported type is registered under its bare
        // name; a qualified `pkg.Type` reference resolves to the same type).
        let expected_bare = expected.rsplit('.').next().unwrap_or(expected);
        let actual_bare = actual.rsplit('.').next().unwrap_or(actual);
        if expected_bare == actual_bare {
            return true;
        }
        // A union accepts any of its variants.
        if let Some(variants) = self.union_variants(expected) {
            if variants.contains(actual_bare) {
                return true;
            }
        }
        false
    }

    /// `typecheck::expression_compatible`: `compatible`, plus the literal
    /// coercions that the AST checker allows for constant arguments — a `Byte`
    /// parameter accepts an in-range `Integer` literal, `Fixed` accepts an
    /// `Integer`/`Float` literal. The `Const` node carries the literal type and
    /// value, so the same check applies on the IR.
    fn expression_compatible(&self, expected: &str, actual: &str, value: &IrValue) -> bool {
        if self.compatible(expected, actual) {
            return true;
        }
        if let IrValue::Const { type_, value } = value {
            match (expected, type_.as_str()) {
                ("Byte", "Integer") => {
                    return value.parse::<u16>().is_ok_and(|n| n <= u8::MAX as u16);
                }
                ("Fixed", "Integer") | ("Fixed", "Float") => return true,
                _ => {}
            }
        }
        // Negated numeric literal into Fixed (`-1` etc.).
        if expected == "Fixed" {
            if let IrValue::Unary { op, operand, .. } = value {
                if op == "-" && matches!(operand.as_ref(), IrValue::Const { type_, .. } if type_ == "Integer" || type_ == "Float")
                {
                    return true;
                }
            }
        }
        false
    }

    /// Reject a `RETURN <value>` whose value type is incompatible with the
    /// function's declared return type (`typecheck`'s `TYPE_RETURN_MISMATCH`).
    /// Codegen places the return value into the ABI return slot by the declared
    /// type, so a crafted mismatch is a type confusion at the return boundary.
    fn check_return_type(&self, value: &IrValue, locals: &HashMap<String, String>) {
        let expected = self.current_return.borrow().clone();
        if expected.is_empty() || expected == "Nothing" || expected == "Unknown" {
            return;
        }
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        if !self.expression_compatible(&expected, &actual, value) {
            self.emit(
                "TYPE_RETURN_MISMATCH",
                format!("RETURN value has type {actual}, expected {expected}."),
            );
        }
    }

    /// Reject a binding whose initializer type is incompatible with its declared
    /// type — `typecheck`'s `TYPE_BINDING_MISMATCH`. The caller suppresses this
    /// when a literal-range error already fired for the same binding (matching
    /// typecheck's `!reported_range_error` guard), so a single out-of-range
    /// literal is reported once, as the more specific range error.
    fn check_binding_type(
        &self,
        name: &str,
        declared: &str,
        value: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        let expected = resource_base_type(declared);
        if expected.is_empty() || expected == "Nothing" || expected == "Unknown" {
            return;
        }
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        if !self.expression_compatible(expected, &actual, value) {
            self.emit(
                "TYPE_BINDING_MISMATCH",
                format!("Binding `{name}` has initializer type {actual}, expected {expected}."),
            );
        }
    }

    /// Reject a control-flow condition (IF/WHILE/LOOP UNTIL/WHEN guard) whose
    /// type is provably not Boolean — `typecheck`'s
    /// `TYPE_CONDITION_REQUIRES_BOOLEAN`. `what` is the statement-specific
    /// message prefix (`"IF condition"`, `"WHEN guard"`, …).
    fn check_condition_boolean(
        &self,
        what: &str,
        value: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        if !self.expression_compatible("Boolean", &actual, value) {
            self.emit(
                "TYPE_CONDITION_REQUIRES_BOOLEAN",
                format!("{what} has type {actual}, expected Boolean."),
            );
        }
    }

    /// Reject an assignment whose value type is incompatible with the target
    /// binding's settled type — `typecheck`'s `TYPE_ASSIGNMENT_MISMATCH`. The
    /// caller suppresses this when a literal-range error already fired
    /// (typecheck's `!reported_range_error` guard). Unlike `TYPE_BINDING_MISMATCH`
    /// no explicit-annotation gate applies: by assignment time the binding's
    /// type is settled regardless of how it was declared.
    fn check_assignment_type(
        &self,
        name: &str,
        declared: &str,
        value: &IrValue,
        locals: &HashMap<String, String>,
    ) {
        let expected = resource_base_type(declared);
        if expected.is_empty() || expected == "Nothing" || expected == "Unknown" {
            return;
        }
        let Some(actual) = self.infer_type(value, locals) else {
            return;
        };
        if !self.expression_compatible(expected, &actual, value) {
            self.emit(
                "TYPE_ASSIGNMENT_MISMATCH",
                format!("Assignment to `{name}` has type {actual}, expected {expected}."),
            );
        }
    }

    /// The typecheck constructor rules on a lowered `Constructor` value: the
    /// name must be a record TYPE (`TYPE_CONSTRUCTOR_REQUIRES_RECORD`), the
    /// argument count must equal the field count exactly — records have no
    /// field defaults — (`TYPE_CONSTRUCTOR_ARITY_MISMATCH`), and each argument
    /// must be compatible with its positional field
    /// (`TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH`). Lowering reorders named
    /// arguments into field order, so positional checking covers both forms.
    fn check_constructor(
        &self,
        type_name: &str,
        args: &[IrValue],
        locals: &HashMap<String, String>,
    ) {
        // Compiler-owned records may never be user-constructed (typecheck's
        // TYPE_READ_ONLY_RECORD_CONSTRUCTOR). The Error/ErrorLoc arm of that
        // rule stays in typecheck: lowering itself emits `Constructor{Error}`
        // for the `error()` builtin and trap machinery, so on the IR a user
        // `Error[..]` is indistinguishable from a legitimate synthesized one.
        if read_only_record_type(type_name) {
            self.emit(
                "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                format!("TYPE `{type_name}` is compiler-owned and cannot be constructed."),
            );
            return;
        }
        if !self.records.contains_key(type_name) {
            // A constructor naming a declared non-record type is malformed; an
            // unknown name is left alone (could be a builtin record).
            let kind = if self.unions.contains_key(type_name) {
                Some("UNION")
            } else if self.enums.contains_key(type_name) {
                Some("ENUM")
            } else {
                None
            };
            if let Some(kind) = kind {
                self.emit(
                    "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
                    format!("`{type_name}` is a {kind}, not a record TYPE."),
                );
            }
            return;
        }
        let Some(fields) = self.record_field_lists.get(type_name) else {
            return;
        };
        if args.len() != fields.len() {
            self.emit(
                "TYPE_CONSTRUCTOR_ARITY_MISMATCH",
                format!(
                    "Constructor `{type_name}` has {} argument(s), expected {}.",
                    args.len(),
                    fields.len()
                ),
            );
        }
        for (index, arg) in args.iter().enumerate() {
            let Some((field_name, field_type)) = fields.get(index) else {
                continue;
            };
            let Some(actual) = self.infer_type(arg, locals) else {
                continue;
            };
            if !self.expression_compatible(field_type, &actual, arg) {
                self.emit(
                    "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
                    format!(
                        "Argument {} for `{type_name}` has type {actual}, expected {field_type} for field `{field_name}`.",
                        index + 1
                    ),
                );
            }
        }
    }

    /// Reject a `UnionWrap` whose `member_type` is not a variant of the named
    /// union (a value smuggled under a tag the union does not define).
    fn check_union_wrap(&self, union_type: &str, member_type: &str) {
        if member_type.is_empty() {
            return;
        }
        if let Some(variants) = self.union_variants(union_type) {
            if !variants.contains(member_type) {
                self.emit(
                    VERIFY_TYPE,
                    format!("`{member_type}` is not a variant of union `{union_type}`"),
                );
            }
        }
    }

    /// Verify every `Capture` in a value addresses a slot within the enclosing
    /// closure's captured-slot count. Skipped when the closure shape is unknown.
    fn check_value_captures(&self, value: &IrValue, slots: Option<usize>) {
        let Some(slots) = slots else {
            return;
        };
        let mut violation = None;
        walk_captures(value, &mut |index| {
            if index >= slots && violation.is_none() {
                violation = Some(index);
            }
        });
        if let Some(index) = violation {
            self.emit(
                VERIFY_TYPE,
                format!("closure capture index {index} is out of range ({slots} slot(s))"),
            );
        }
    }

    /// The complete set of field names for a record type, expanding `includes`
    /// transitively. Returns `None` when the type is not a known record or when
    /// an include cannot be resolved (so the field set is incomplete and the
    /// member-existence check must be skipped).
    fn record_fields(&self, type_name: &str) -> Option<HashSet<String>> {
        // Built-in record types (io/net/term handles) carry their fields in the
        // builtin tables rather than the project type table.
        if let Some(fields) = builtin_type_fields(type_name) {
            return Some(fields.iter().map(|(name, _)| (*name).to_string()).collect());
        }
        let mut out = HashSet::new();
        let mut seen = HashSet::new();
        if self.collect_record_fields(type_name, &mut out, &mut seen) {
            Some(out)
        } else {
            None
        }
    }

    fn collect_record_fields(
        &self,
        type_name: &str,
        out: &mut HashSet<String>,
        seen: &mut HashSet<String>,
    ) -> bool {
        if !seen.insert(type_name.to_string()) {
            // A cycle in `includes` — treat as fully expanded to avoid looping.
            return true;
        }
        let Some(info) = self.records.get(type_name) else {
            return false;
        };
        for field in &info.fields {
            out.insert(field.clone());
        }
        for include in &info.includes {
            if !self.collect_record_fields(include, out, seen) {
                return false;
            }
        }
        true
    }

    /// The complete variant-name set of a union, expanding included unions.
    /// `None` when the union or one of its includes is unknown.
    fn union_variants(&self, union_type: &str) -> Option<HashSet<String>> {
        let mut out = HashSet::new();
        let mut seen = HashSet::new();
        if self.collect_union_variants(union_type, &mut out, &mut seen) {
            Some(out)
        } else {
            None
        }
    }

    fn collect_union_variants(
        &self,
        union_type: &str,
        out: &mut HashSet<String>,
        seen: &mut HashSet<String>,
    ) -> bool {
        if !seen.insert(union_type.to_string()) {
            return true;
        }
        let Some(info) = self.unions.get(union_type) else {
            return false;
        };
        for variant in &info.variants {
            out.insert(variant.clone());
        }
        for include in &info.includes {
            if !self.collect_union_variants(include, out, seen) {
                return false;
            }
        }
        true
    }

    /// Best-effort static type of a value. Returns `None` whenever the type
    /// cannot be determined with certainty; callers treat `None` as "unknown"
    /// and skip type-dependent rejections.
    ///
    /// Since format v3 (plan-20-B) every computed node carries its result type,
    /// so this resolves `Call`/`CallResult`/`Binary`/`Unary`/`ResultValue`/… as
    /// well — a member access on a *computed* primitive result is now caught,
    /// not just one on a local or constructor. `Local`/`Global` resolve through
    /// the binding environment (their type is not on the node); the `"Unknown"`
    /// marker a node carries when lowering could not name its type is treated as
    /// unresolved so it never forces a rejection (plan-20-C).
    fn infer_type(&self, value: &IrValue, locals: &HashMap<String, String>) -> Option<String> {
        match value {
            IrValue::Local(name) => return locals.get(name).cloned(),
            IrValue::Global(name) => return self.globals.get(name).cloned(),
            IrValue::MemberAccess { target, member, .. } => {
                // Prefer the annotated member type; fall back to resolving the
                // field through the target's record type for older shapes.
                if let Some(annotated) = usable_type(value.annotated_type()) {
                    return Some(annotated);
                }
                let target_type = self.infer_type(target, locals)?;
                return self.field_type(&target_type, member);
            }
            _ => {}
        }
        usable_type(value.annotated_type())
    }

    /// The declared type of a record member, for chained member-access
    /// inference. Only resolves through record types whose fields are known.
    fn field_type(&self, type_name: &str, member: &str) -> Option<String> {
        if let Some(fields) = builtin_type_fields(type_name) {
            return fields
                .iter()
                .find(|(name, _)| *name == member)
                .map(|(_, type_)| (*type_).to_string());
        }
        // Project records store field types on the IrType; look them up via the
        // dedicated map built alongside `records`.
        self.field_types
            .get(type_name)
            .and_then(|fields| fields.get(member).cloned())
    }
}

/// A node's annotated result type, or `None` when it is absent, empty, or the
/// explicit `"Unknown"` marker lowering stamps when it cannot name a type.
/// Filtering `"Unknown"` here is what keeps the type-relational rules from
/// rejecting a node whose type simply could not be reconstructed (plan-20-C).
fn usable_type(annotated: Option<&str>) -> Option<String> {
    match annotated {
        Some(t) if !t.is_empty() && t != "Unknown" => Some(t.to_string()),
        _ => None,
    }
}

/// Whether an IR value is a numeric literal equal to zero (possibly negated) —
/// mirrors `typecheck::helpers::numeric_literal_is_zero` on the IR shape.
fn numeric_literal_is_zero(value: &IrValue) -> bool {
    match value {
        IrValue::Const { type_, value }
            if matches!(type_.as_str(), "Integer" | "Float" | "Byte" | "Fixed") =>
        {
            value.parse::<f64>().is_ok_and(|n| n == 0.0)
        }
        IrValue::Unary { op, operand, .. } if op == "-" => numeric_literal_is_zero(operand),
        _ => false,
    }
}

/// Compiler-owned record types users may neither construct nor WITH-update —
/// mirrors `typecheck::helpers::read_only_record_type`.
fn read_only_record_type(type_name: &str) -> bool {
    type_name == builtins::term::TERM_COLOR_TYPE
        || type_name == builtins::term::TERM_SIZE_TYPE
        || type_name == builtins::net::ADDRESS_TYPE
        || type_name.starts_with("MapEntry OF ")
}

/// Whether `name` is a built-in resource type (has a registered close op).
fn is_resource_name(name: &str) -> bool {
    builtins::resource::builtin_resource_close_function(name).is_some()
}

/// The base resource type name, stripping the `RES ` ownership marker and a
/// trailing `STATE T` clause (`File STATE Cursor` → `File`).
fn resource_base_type(type_: &str) -> &str {
    let t = type_.strip_prefix("RES ").unwrap_or(type_);
    match t.find(" STATE ") {
        Some(idx) => &t[..idx],
        None => t,
    }
}

/// Collect the names of every `Local` read anywhere in an op's value positions
/// (not its nested bodies — those are traversed separately).
fn collect_local_reads_op(op: &IrOp, out: &mut Vec<String>) {
    let mut v = |value: &IrValue, out: &mut Vec<String>| collect_local_reads_value(value, out);
    match op {
        IrOp::Bind {
            value: Some(value), ..
        }
        | IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value, .. }
        | IrOp::ExitProgram { code: value, .. }
        | IrOp::Fail { error: value, .. }
        | IrOp::Return {
            value: Some(value), ..
        } => v(value, out),
        IrOp::If { condition, .. } | IrOp::While { condition, .. } => v(condition, out),
        IrOp::For {
            start, end, step, ..
        } => {
            v(start, out);
            v(end, out);
            v(step, out);
        }
        IrOp::ForEach { iterable, .. } => v(iterable, out),
        IrOp::Match { value, .. } => v(value, out),
        _ => {}
    }
}

/// Collect the names of every `Local` read within a value expression.
fn collect_local_reads_value(value: &IrValue, out: &mut Vec<String>) {
    match value {
        IrValue::Local(name) => out.push(name.clone()),
        IrValue::Call { args, .. } | IrValue::CallResult { args, .. } => {
            for a in args {
                collect_local_reads_value(a, out);
            }
        }
        IrValue::Constructor { args, .. } => {
            for a in args {
                collect_local_reads_value(a, out);
            }
        }
        IrValue::Closure { captures, .. } => {
            for c in captures {
                collect_local_reads_value(c, out);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => collect_local_reads_value(value, out),
        IrValue::Binary { left, right, .. } => {
            collect_local_reads_value(left, out);
            collect_local_reads_value(right, out);
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            collect_local_reads_value(target, out);
            for u in updates {
                collect_local_reads_value(&u.value, out);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for e in values {
                collect_local_reads_value(e, out);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, val) in entries {
                collect_local_reads_value(k, out);
                collect_local_reads_value(val, out);
            }
        }
        IrValue::Const { .. }
        | IrValue::Global(_)
        | IrValue::LocalRef { .. }
        | IrValue::FunctionRef { .. }
        | IrValue::Capture { .. } => {}
    }
}

/// Parse a `Map OF K TO V` type string into `(K, V)`.
fn parse_map(type_: &str) -> Option<(&str, &str)> {
    let rest = type_.strip_prefix("Map OF ")?;
    let idx = rest.find(" TO ")?;
    Some((&rest[..idx], &rest[idx + " TO ".len()..]))
}

/// Build a `member → type` map from a record's declared fields.
fn field_type_map(fields: &[IrField]) -> HashMap<String, String> {
    fields
        .iter()
        .map(|f| (f.name.clone(), f.type_.clone()))
        .collect()
}

/// Builtin record types (io/net/term) expose their fields through the builtins
/// tables. Consolidated here so the checker consults one accessor.
fn builtin_type_fields(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    // The runtime error records (typecheck types their members inline in
    // `infer_member`); listed here so member-access inference resolves
    // `err.source.line` chains and the read-only WITH check sees ErrorLoc.
    match name {
        "Error" => {
            return Some(&[
                ("code", "Integer"),
                ("message", "String"),
                ("source", "ErrorLoc"),
            ]);
        }
        "ErrorLoc" => {
            return Some(&[
                ("filename", "String"),
                ("line", "Integer"),
                ("char", "Integer"),
            ]);
        }
        _ => {}
    }
    builtins::io::builtin_type_fields(name)
        .or_else(|| builtins::net::builtin_type_fields(name))
        .or_else(|| builtins::term::builtin_type_fields(name))
}

/// Record every `Closure { name, captures }` site's captured-slot count so the
/// capture-bounds rule knows each closure body's env size.
fn collect_closures(value: &IrValue, out: &mut HashMap<String, HashSet<usize>>) {
    match value {
        IrValue::Closure { name, captures, .. } => {
            out.entry(name.clone()).or_default().insert(captures.len());
            for capture in captures {
                collect_closures(capture, out);
            }
        }
        IrValue::Call { args, .. } | IrValue::CallResult { args, .. } => {
            for arg in args {
                collect_closures(arg, out);
            }
        }
        IrValue::Constructor { args, .. } => {
            for arg in args {
                collect_closures(arg, out);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => collect_closures(value, out),
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            collect_closures(target, out);
            for update in updates {
                collect_closures(&update.value, out);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for v in values {
                collect_closures(v, out);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_closures(k, out);
                collect_closures(v, out);
            }
        }
        IrValue::Binary { left, right, .. } => {
            collect_closures(left, out);
            collect_closures(right, out);
        }
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::LocalRef { .. }
        | IrValue::FunctionRef { .. }
        | IrValue::Capture { .. } => {}
    }
}

fn collect_closures_ops(ops: &[IrOp], out: &mut HashMap<String, HashSet<usize>>) {
    for op in ops {
        match op {
            IrOp::Bind { value: Some(v), .. } => collect_closures(v, out),
            IrOp::Bind { value: None, .. } => {}
            IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::StateAssign { value, .. }
            | IrOp::Eval { value, .. }
            | IrOp::ExitProgram { code: value, .. }
            | IrOp::Fail { error: value, .. } => collect_closures(value, out),
            IrOp::Return { value: Some(v), .. } => collect_closures(v, out),
            IrOp::Return { value: None, .. } => {}
            IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
            IrOp::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                collect_closures(condition, out);
                collect_closures_ops(then_body, out);
                collect_closures_ops(else_body, out);
            }
            IrOp::Match { value, cases, .. } => {
                collect_closures(value, out);
                for case in cases {
                    match &case.pattern {
                        super::IrMatchPattern::Else => {}
                        super::IrMatchPattern::Value(v) => collect_closures(v, out),
                        super::IrMatchPattern::OneOf(vs) => {
                            for v in vs {
                                collect_closures(v, out);
                            }
                        }
                    }
                    if let Some(guard) = &case.guard {
                        collect_closures(guard, out);
                    }
                    collect_closures_ops(&case.body, out);
                }
            }
            IrOp::While {
                condition, body, ..
            } => {
                collect_closures(condition, out);
                collect_closures_ops(body, out);
            }
            IrOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_closures(start, out);
                collect_closures(end, out);
                collect_closures(step, out);
                collect_closures_ops(body, out);
            }
            IrOp::DoUntil {
                body, condition, ..
            } => {
                collect_closures_ops(body, out);
                collect_closures(condition, out);
            }
            IrOp::ForEach { iterable, body, .. } => {
                collect_closures(iterable, out);
                collect_closures_ops(body, out);
            }
            IrOp::Trap { body, .. } => collect_closures_ops(body, out),
        }
    }
}

/// Visit every `Capture` index reachable from a value expression (captures
/// never nest through ops — a closure body's captures live in leading binds).
fn walk_captures(value: &IrValue, visit: &mut impl FnMut(usize)) {
    match value {
        IrValue::Capture { index, .. } => visit(*index),
        IrValue::Call { args, .. } | IrValue::CallResult { args, .. } => {
            for arg in args {
                walk_captures(arg, visit);
            }
        }
        IrValue::Closure { captures, .. } => {
            for capture in captures {
                walk_captures(capture, visit);
            }
        }
        IrValue::Constructor { args, .. } => {
            for arg in args {
                walk_captures(arg, visit);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => walk_captures(value, visit),
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            walk_captures(target, visit);
            for update in updates {
                walk_captures(&update.value, visit);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for v in values {
                walk_captures(v, visit);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                walk_captures(k, visit);
                walk_captures(v, visit);
            }
        }
        IrValue::Binary { left, right, .. } => {
            walk_captures(left, visit);
            walk_captures(right, visit);
        }
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::LocalRef { .. }
        | IrValue::FunctionRef { .. } => {}
    }
}

#[cfg(test)]
mod tests;
