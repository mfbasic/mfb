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

use super::{IrField, IrOp, IrProject, IrValue};
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
        for param in &function.params {
            env.current_line.set(param.loc.line);
            locals.insert(param.name.clone(), param.type_.clone());
            if let Some(default) = &param.default {
                env.check_value(default, &locals);
            }
        }
        let closure_slots = env.closure_slot_count(&function.name);
        env.check_ops(&function.body, &mut locals.clone(), closure_slots, 0);
        // Resource use-after-move is a separate straight-line dataflow pass.
        env.check_resource_moves(&function.body, &mut locals, &mut HashSet::new());
    }
    // Global initializers are lowered into a synthetic function later; verify
    // their initializer expressions here with an empty local scope.
    for binding in &project.bindings {
        env.current_file.replace(String::new());
        env.current_line.set(binding.loc.line);
        if let Some(value) = &binding.value {
            env.check_value(value, &HashMap::new());
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
    let diagnostics = collect_diagnostics(project);
    for d in &diagnostics {
        let path = if d.file.is_empty() {
            project_dir.join("<generated>")
        } else {
            project_dir.join(&d.file)
        };
        crate::rules::show_diagnostic(&d.rule, &d.detail, &path, d.line as usize, 1, 1);
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(())
    }
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
    /// Function name → the distinct captured-slot counts observed at the
    /// `Closure` sites that target it. A single count means the closure shape is
    /// known; zero or multiple distinct counts leaves it ambiguous (skip).
    closure_counts: HashMap<String, HashSet<usize>>,
    /// Record type name → (member name → declared member type), for chained
    /// member-access type inference.
    field_types: HashMap<String, HashMap<String, String>>,
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
                }
                "union" => {
                    unions.insert(
                        ty.name.clone(),
                        UnionInfo {
                            variants: ty.variants.iter().map(|v| v.name.clone()).collect(),
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
            closure_counts,
            field_types,
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
        for op in ops {
            let line = op.loc().line;
            self.current_line.set(line);
            match op {
                IrOp::Bind {
                    name, type_, value, ..
                } => {
                    if let Some(value) = value {
                        self.check_value_captures(value, closure_slots);
                        self.check_value(value, locals);
                    }
                    locals.insert(name.clone(), type_.clone());
                }
                IrOp::Assign { value, .. }
                | IrOp::AssignGlobal { value, .. }
                | IrOp::StateAssign { value, .. }
                | IrOp::Eval { value, .. }
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
                    let mut branch = locals.clone();
                    self.check_ops(then_body, &mut branch, closure_slots, depth + 1);
                    let mut branch = locals.clone();
                    self.check_ops(else_body, &mut branch, closure_slots, depth + 1);
                }
                IrOp::Match { value, cases, .. } => {
                    if cases.is_empty() {
                        self.emit(VERIFY_MATCH, "MATCH has no cases (not exhaustive)".to_string());
                    }
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                    self.check_match_exhaustive(value, cases, locals);
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
                            case_locals = locals.clone();
                        }
                        self.check_ops(&case.body, &mut case_locals, closure_slots, depth + 1);
                        self.current_line.set(line);
                    }
                }
                IrOp::While {
                    condition, body, ..
                } => {
                    self.check_value_captures(condition, closure_slots);
                    self.check_value(condition, locals);
                    let mut branch = locals.clone();
                    self.check_ops(body, &mut branch, closure_slots, depth + 1);
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
                    let mut branch = locals.clone();
                    branch.insert(name.clone(), type_.clone());
                    self.check_ops(body, &mut branch, closure_slots, depth + 1);
                }
                IrOp::DoUntil {
                    body, condition, ..
                } => {
                    let mut branch = locals.clone();
                    self.check_ops(body, &mut branch, closure_slots, depth + 1);
                    // The trailing condition is reported at the loop's own line.
                    self.current_line.set(line);
                    self.check_value_captures(condition, closure_slots);
                    self.check_value(condition, locals);
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
                    let mut branch = locals.clone();
                    branch.insert(name.clone(), type_.clone());
                    self.check_ops(body, &mut branch, closure_slots, depth + 1);
                }
                IrOp::Trap { name, body, .. } => {
                    let mut branch = locals.clone();
                    branch.insert(name.clone(), "Error".to_string());
                    self.check_ops(body, &mut branch, closure_slots, depth + 1);
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
                self.check_constructor_arity(type_, args.len());
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
                target, updates, ..
            } => {
                self.check_value(target, locals);
                for update in updates {
                    self.check_value(&update.value, locals);
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
                if let Some((key_type, value_type)) = parse_map(type_) {
                    for (k, v) in entries {
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
        self.current_file.replace(String::new());
        for ty in &project.types {
            self.current_line.set(ty.loc.line);
            match ty.kind.as_str() {
                "type" | "record" => {
                    for field in &ty.fields {
                        if is_resource_name(resource_base_type(&field.type_)) {
                            self.emit(
                                "TYPE_RESOURCE_FIELD_FORBIDDEN",
                                format!(
                                    "Record `{}` field `{}` is resource `{}`; records cannot own resources.",
                                    ty.name, field.name, field.type_
                                ),
                            );
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
                _ => {}
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
            // thread::transfer(handle, res, ...) moves the resource at arg 1 to
            // the other side (invalidation event #2, §15).
            if target == "thread.transferResource" || target == "thread.transfer" {
                if let Some(IrValue::Local(name)) = args.get(1) {
                    if locals
                        .get(name)
                        .is_some_and(|t| is_resource_name(resource_base_type(t)))
                    {
                        return Some(name.clone());
                    }
                }
                return None;
            }
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
        // The complete member/variant set, and whether it is a union (for the
        // diagnostic wording). Anything else (Boolean Result flag, primitive,
        // unresolved include) is skipped — no false rejection.
        let (all, is_union) = if let Some(variants) = self.union_variants(&ty) {
            (variants, true)
        } else if let Some(members) = self.enums.get(&ty) {
            (members.clone(), false)
        } else {
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
        let mut missing: Vec<&String> = all.difference(&covered).collect();
        if missing.is_empty() {
            return;
        }
        missing.sort();
        let kind = if is_union { "UNION" } else { "ENUM" };
        self.emit(
            "TYPE_MATCH_NOT_EXHAUSTIVE",
            format!(
                "MATCH on {kind} `{ty}` does not cover {}; add unguarded CASE arms or CASE ELSE.",
                missing[0]
            ),
        );
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
            _ => {}
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
        let arg_types: Option<Vec<String>> =
            args.iter().map(|a| self.infer_type(a, locals)).collect();
        let Some(arg_types) = arg_types else {
            return;
        };
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

    /// Reject a record constructor supplied with more arguments than the record
    /// has fields (an overflow of the record's positional slots). Under-supply
    /// is left unchecked: `IrField` carries no default marker, so the minimum
    /// arity cannot be reconstructed soundly.
    fn check_constructor_arity(&self, type_name: &str, argc: usize) {
        if let Some(fields) = self.record_fields(type_name) {
            if argc > fields.len() {
                self.emit(
                    VERIFY_TYPE,
                    format!(
                        "constructor `{type_name}` passes {argc} argument(s) but the record has {} field(s)",
                        fields.len()
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
