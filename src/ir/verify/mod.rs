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
        env.check_ops(&function.body, &mut locals, closure_slots, 0);
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
        let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
        for ty in &project.types {
            match ty.kind.as_str() {
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
            | IrValue::ResultError { value }
            | IrValue::Unary { operand: value, .. } => {
                self.check_value(value, locals);
            }
            IrValue::Binary { left, right, .. } => {
                self.check_value(left, locals);
                self.check_value(right, locals);
            }
            IrValue::WithUpdate {
                target, updates, ..
            } => {
                self.check_value(target, locals);
                for update in updates {
                    self.check_value(&update.value, locals);
                }
            }
            IrValue::ListLiteral { values, .. } => {
                for v in values {
                    self.check_value(v, locals);
                }
            }
            IrValue::MapLiteral { entries, .. } => {
                for (k, v) in entries {
                    self.check_value(k, locals);
                    self.check_value(v, locals);
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
                VERIFY_TYPE,
                format!("member `{member}` accessed on a `{type_name}` value"),
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
                    VERIFY_TYPE,
                    format!("record `{type_name}` has no member `{member}`"),
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
                VERIFY_TYPE,
                format!(
                    "call to `{target}` passes {argc} argument(s), expected {required}..={}",
                    sig.total
                ),
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
