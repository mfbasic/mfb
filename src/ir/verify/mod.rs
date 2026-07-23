//! IR-level semantic verification (plan-19-ir-semantic-verification.md).
//!
//! A compiled package (`.mfp`) carries hand-serializable IR that a consumer
//! decodes and lowers to native code. Only the source front end runs the AST
//! type checker (`src/syntaxcheck/`); the decoded package IR is otherwise trusted
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

use super::{IrField, IrFunction, IrOp, IrProject, IrType, IrValue};
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
/// **source** path `ir::verify` emits ONLY these (syntaxcheck still owns every
/// other rule, so emitting a non-relocated rule here would duplicate it); on
/// the **package** path there is no syntaxcheck, so `ir::verify` emits all of its
/// checks regardless. `syntaxcheck::report` skips this same set. A rule appears
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
    "TYPE_MONEY_LITERAL_OVERFLOW",
    "TYPE_MONEY_LITERAL_UNDERFLOW",
    "TYPE_MONEY_LITERAL_PRECISION",
    "TYPE_MONEY_OPERATION_INVALID",
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
    "TYPE_USE_AFTER_MOVE",
    "TYPE_UNKNOWN_ENUM_MEMBER",
    "SYMBOL_NOT_CALLABLE",
    "TYPE_BINDING_REQUIRES_TYPE_OR_VALUE",
    "TYPE_LET_REQUIRES_VALUE",
    "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
    "TYPE_DEFAULT_ARG_ORDER",
    "TYPE_PARAM_REQUIRES_TYPE",
    "TYPE_FUNC_REQUIRES_RETURN_TYPE",
    "EXIT_NO_MATCHING_LOOP",
    "CONTINUE_NO_MATCHING_LOOP",
    "TYPE_EXIT_PROGRAM_REQUIRES_INTEGER",
    "EXIT_PROGRAM_CODE_OUT_OF_RANGE",
    "TYPE_SUB_HAS_NO_VALUE",
    "TYPE_FUNC_MISSING_RETURN",
    "TYPE_FAIL_REQUIRES_ERROR",
    "TYPE_PROPAGATE_REQUIRES_TRAP",
    "TYPE_RESOURCE_REQUIRES_RES",
    "TYPE_RES_REQUIRES_RESOURCE",
    "TYPE_STATE_INVALID",
    "TYPE_UNION_STATE_FORBIDDEN",
    // ir::verify is the SOLE implementer of the STATE-agreement rule (plan-52-C/D)
    // — syntaxcheck has no twin of it to duplicate — so it is relocated from birth
    // rather than after a reproduction pass. Without this entry the source path
    // filters it out and it surfaces only via the package path's `check()`, which
    // renders unlocated (`error: TYPE_STATE_MISMATCH: …`, no file:line).
    "TYPE_STATE_MISMATCH",
    // plan-59-C: the opaque-narrowing rule is the STATE-agreement rule's sibling
    // and is likewise implemented only here — it needs the function's parameter
    // list to tell an opaque value from a stateless one, which syntaxcheck does
    // not track. Same reasoning as TYPE_STATE_MISMATCH above; without this entry
    // it renders unlocated.
    "TYPE_STATE_OPAQUE_NARROWING",
    // Likewise ir::verify is the sole implementer of the BIND STATE validation
    // (plan-53-B) — syntaxcheck never inspects a `LINK` function's BIND STATE.
    "NATIVE_BIND_STATE_INVALID",
    "TYPE_RESULT_NOT_MATCHABLE",
    "TYPE_RESULT_IS_IMPLICIT",
    "TYPE_THREAD_RESULT_REMOVED",
    "TYPE_MEMBER_NOT_VISIBLE",
    // ir::verify is the sole implementer: the condition is knowable only from
    // escape analysis' ownership decision, which syntaxcheck does not compute
    // (bug-291).
    "TYPE_RESOURCE_RETURN_ORDER",
];

/// Diagnostic prefix shared with the structural `verify_package` checks so a
/// rejection surfaces as a `PACKAGE_BINARY_REPRESENTATION_*` diagnostic.
const VERIFY_TYPE: &str = "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE";
const VERIFY_MATCH: &str = "PACKAGE_BINARY_REPRESENTATION_VERIFY_MATCH";

/// Scalar types a value can never be member-accessed through. A `MemberAccess`
/// whose target provably has one of these types is a type confusion.
const PRIMITIVE_TYPES: &[&str] = &[
    "Integer", "Float", "String", "Boolean", "Byte", "Fixed", "Nothing", "Money", "Scalar",
];

/// Collect every semantic-verification diagnostic for a merged `IrProject`, in
/// the traversal order the AST type checker uses (functions in declaration
/// order; each body's ops in order; each op's sub-values innermost-first). The
/// checker never short-circuits, so a program with several violations yields
/// them all.
pub(crate) fn collect_diagnostics(project: &IrProject) -> Vec<Diagnostic> {
    // The package path runs on merged IR, whose resource types are already in
    // `native_resources` or are the package's own, so it registers no extra rows.
    collect_diagnostics_with(project, false, &[])
}

/// `collect_diagnostics`, with `imported_types_unknown` telling the checker which
/// path it is on — the one question its type tables cannot answer for themselves.
///
/// On the **source** path `build` lowers with deliberately empty external maps, so
/// an importer's tables hold only its own types and every imported name misses. On
/// the **package** path the merged IR carries the full type table and every name
/// is decoded from an id that must exist in it. Same checker, different completeness
/// of information — so a miss means "imported, cannot say" on one and "genuinely
/// absent" on the other (bug-258).
fn collect_diagnostics_with(
    project: &IrProject,
    imported_types_unknown: bool,
    imported_resources: &[(String, String)],
) -> Vec<Diagnostic> {
    let mut env = TypeEnv::build(project);
    env.imported_types_unknown = imported_types_unknown;
    // bug-377: seed the imported packages' `RESOURCE_TABLE` rows. The project's
    // own `native_resources` win — an importer never overrides a declaration it
    // can see the source of.
    for (type_name, close_function) in imported_resources {
        env.resource_closers
            .entry(type_name.clone())
            .or_insert_with(|| close_function.clone());
    }
    let env = env;
    for function in &project.functions {
        env.current_file.replace(function.file.clone());
        env.current_return.replace(function.returns.clone());
        env.current_kind.replace(function.kind.clone());
        env.current_owners
            .replace(function.resource_owners.keys().cloned().collect());
        // plan-59-C: a parameter whose type is a resource and which names no
        // `STATE` is OPAQUE, not stateless — §15.5's parameter row accepts "any
        // state or none". Recorded per-function so the binding and return arms can
        // tell an opaque value from a provably stateless one; the two are
        // indistinguishable by type string alone.
        env.current_opaque_params.replace(
            function
                .params
                .iter()
                .filter(|p| {
                    env.is_resource_or_resource_union(resource_base_type(&p.type_))
                        && crate::builtins::resource::state_type_name(&p.type_).is_none()
                })
                .map(|p| p.name.clone())
                .collect(),
        );
        // A declared return type is a type reference too (`AS List OF File`
        // needs the RES element marking like any collection declaration).
        if !function.name.starts_with('$') {
            env.current_line.set(function.loc.line);
            env.check_collection_res_axis(resource_base_type(&function.returns));
            env.check_return_state_declaration(function);
        }
        // A declared FUNC must name its return type (`AS T`); lowering stamps
        // `Unknown` when the annotation is absent. Synthesized `$lambda` bodies
        // legitimately carry a computed (possibly Unknown) return — skip them.
        if function.kind == "func"
            && function.returns == "Unknown"
            && !function.name.starts_with('$')
        {
            env.current_line.set(function.loc.line);
            env.emit(
                "TYPE_FUNC_REQUIRES_RETURN_TYPE",
                format!("FUNC `{}` must declare an `AS` return type.", function.name),
            );
        }
        // A value-producing FUNC must return on every path (`AS Nothing`
        // FUNCs, like SUBs, may fall through). Synthesized `$lambda` bodies
        // always end in a lowered Return.
        if function.kind == "func"
            && function.returns != "Nothing"
            && function.returns != "Unknown"
            && !function.name.starts_with('$')
            && !env.block_always_returns(
                &function.body,
                &function
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), p.type_.clone()))
                    .collect(),
            )
        {
            env.current_line.set(function.loc.line);
            env.emit(
                "TYPE_FUNC_MISSING_RETURN",
                format!(
                    "FUNC `{}` must return a {} value.",
                    function.name, function.returns
                ),
            );
        }
        let mut locals: HashMap<String, String> = HashMap::new();
        let mut muts: HashMap<String, bool> = HashMap::new();
        let mut seen_default = false;
        for param in &function.params {
            env.current_line.set(param.loc.line);
            locals.insert(param.name.clone(), param.type_.clone());
            env.check_map_key_comparable(&param.type_);
            env.check_collection_res_axis(resource_base_type(&param.type_));
            // Every parameter must declare an `AS` type (lambda parameters
            // included — syntaxcheck checks both forms with this rule).
            if param.type_ == "Unknown" {
                env.emit(
                    "TYPE_PARAM_REQUIRES_TYPE",
                    format!("Parameter `{}` must declare an `AS` type.", param.name),
                );
            }
            // Once one parameter has a default, all later ones must too —
            // positional call sites could not otherwise bind them.
            if param.default.is_some() {
                seen_default = true;
            } else if seen_default {
                env.emit(
                    "TYPE_DEFAULT_ARG_ORDER",
                    format!(
                        "Parameter `{}` must have a default because an earlier parameter has one.",
                        param.name
                    ),
                );
            }
            // Parameters are immutable (syntaxcheck registers them
            // `mutable: false`), so assigning one is TYPE_ASSIGN_REQUIRES_MUT.
            muts.insert(param.name.clone(), false);
            if let Some(default) = &param.default {
                // bug-297: a parameter default is evaluated in the caller's frame,
                // which has no captured environment at all, so ANY `Capture` here
                // is malformed IR -- `None` selects the stray-capture rejection.
                env.check_value_captures(default, None);
                env.check_value(default, &locals);
                // A parameter default must match the declared parameter type —
                // syntaxcheck's TYPE_DEFAULT_VALUE_MISMATCH (skip-if-unknown).
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
        env.check_closure_capture_arity(&function.name);
        let closure_slots = env.closure_slot_count(&function.name);
        env.check_ops(
            &function.body,
            &mut locals.clone(),
            &mut muts,
            closure_slots,
            0,
        );
        // Resource use-after-move is a separate dataflow pass (straight-line
        // within a block; moves on any fall-through branch propagate past the
        // join, mirroring syntaxcheck's MaybeMoved).
        let mut non_owning: HashSet<String> = function
            .params
            .iter()
            .filter(|p| env.is_resource_or_resource_union(resource_base_type(&p.type_)))
            .map(|p| p.name.clone())
            .collect();
        // A RES binding whose ownership floats into a collection
        // (ResOwner::Float) is non-owning afterwards: the collection owns the
        // close obligation (§15.6).
        for (name, owner) in &function.resource_owners {
            if matches!(owner, crate::escape::ResOwner::Float(_)) {
                non_owning.insert(name.clone());
            }
            // bug-291: the resource flows into a collection this function
            // RETURNs, but the collection is declared after it, so it has no
            // runtime owned-list at the point the resource is produced and the
            // float cannot be honoured. Silently treating this as `Local`
            // compiled a program that closed the resource at function exit while
            // the returned collection still carried it -- the caller's adopted
            // owned-list then closed it a second time, a double close with no
            // diagnostic. Reject it, and name the order that fixes it.
            if let crate::escape::ResOwner::FloatBlocked(collection) = owner {
                env.emit(
                    "TYPE_RESOURCE_RETURN_ORDER",
                    format!(
                        "resource `{name}` is returned inside collection `{collection}`, but \
                         `{collection}` is declared after it, so it cannot take ownership; \
                         declare `{collection}` before `{name}`"
                    ),
                );
            }
        }
        env.check_resource_moves(
            &function.body,
            &mut locals,
            &mut HashSet::new(),
            &function.resource_owners,
            &non_owning,
            &mut HashMap::new(),
        );
    }
    // Global initializers are lowered into a synthetic function later; verify
    // their initializer expressions here with an empty local scope.
    for binding in &project.bindings {
        env.current_file.replace(binding.file.clone());
        env.current_line.set(binding.loc.line);
        if binding.explicit_type {
            env.check_map_key_comparable(&binding.type_);
            env.check_collection_res_axis(resource_base_type(&binding.type_));
        }
        if binding.value.is_none() {
            if !binding.explicit_type {
                env.emit(
                    "TYPE_BINDING_REQUIRES_TYPE_OR_VALUE",
                    format!(
                        "Binding `{}` needs a type annotation or initializer.",
                        binding.name
                    ),
                );
            } else if !binding.mutable {
                env.emit(
                    "TYPE_LET_REQUIRES_VALUE",
                    format!(
                        "Immutable binding `{}` must have an initializer.",
                        binding.name
                    ),
                );
            } else if !env.is_defaultable(&binding.type_, &mut HashSet::new()) {
                env.emit(
                    "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
                    format!(
                        "Mutable binding `{}` cannot omit its initializer because type `{}` does not have a defined default value.",
                        binding.name, binding.type_
                    ),
                );
            }
        }
        if let Some(value) = &binding.value {
            // bug-297: a global initializer runs before any closure exists, so a
            // `Capture` in one is malformed IR for the same reason.
            env.check_value_captures(value, None);
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
    env.check_link_functions(project);
    env.check_link_cstructs(project);
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

/// The relocated source-path diagnostics as unrendered `PendingDiagnostic`s, so
/// `build` can merge them with `syntaxcheck`'s stream and render both in one
/// line-ordered pass (plan-20-Z). Only rules in `RELOCATED_TO_IR_VERIFY` are
/// ir::verify's to emit on the source path; the rest are still syntaxcheck's.
///
/// `imported_resources` carries the `(type, close op)` rows of every imported
/// package's `RESOURCE_TABLE` (bug-377). A decoded package contributes no
/// `native_resources`, so without them every resource rule is inert for an
/// imported type — a double close of a package handle passed clean.
pub fn collect_source_diagnostics(
    project: &IrProject,
    project_dir: &Path,
    imported_resources: &[(String, String)],
) -> Vec<crate::rules::PendingDiagnostic> {
    collect_diagnostics_with(project, true, imported_resources)
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
    /// `func` or `sub` — a SUB call produces no value (TYPE_SUB_HAS_NO_VALUE).
    kind: String,
    /// The declared return type. A call node carries its own result type, which
    /// on decoded package IR is attacker-controlled; this is the independent
    /// truth it is reconciled against (`check_call_result_type`).
    returns: String,
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
    /// User-declared native resource type → its registered close op (dotted
    /// `alias.func`), complementing the builtin close table for the
    /// use-after-move pass.
    resource_closers: HashMap<String, String>,
    /// Function name → the distinct captured-slot counts observed at the
    /// `Closure` sites that target it. A single count means the closure shape is
    /// known; zero or multiple distinct counts leaves it ambiguous (skip).
    closure_counts: HashMap<String, HashSet<usize>>,
    /// Record type name → (member name → declared member type), for chained
    /// member-access type inference.
    field_types: HashMap<String, HashMap<String, String>>,
    /// Record type name → its direct fields as ordered (name, type) pairs, for
    /// positional constructor checking (mirrors syntaxcheck's `TypeInfo.fields`,
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
    /// Whether a type-poisoning rule fired while checking the current value —
    /// syntaxcheck's inference yields `Unknown` after an operator/constructor
    /// failure, cascading a TYPE_UNKNOWN_VALUE at the consuming statement even
    /// where lowering stamped a nominal result type. Reset per checked value.
    poisoned: Cell<bool>,
    /// Whether this run's type tables are missing the imported types (the source
    /// path lowers with empty external maps; the package path does not). When set,
    /// a type name absent from every table is treated as an unresolvable *import*
    /// rather than as a positively-known-bad type — see `is_defaultable` (bug-258).
    imported_types_unknown: bool,
    /// Whether the value currently being checked is a state assignment's
    /// right-hand side (`s.state = WITH s.state { … }`), whose `WITH` target reads
    /// `s.state`. Suppresses the `.state`-read rule there so the assign path's more
    /// precise diagnostic is the only one reported for the statement (plan-52-C).
    checking_state_assign: Cell<bool>,
    /// The enclosing loop kinds, innermost last — an EXIT/CONTINUE must name a
    /// kind present here. Checking is sequential, so a RefCell stack suffices.
    loop_stack: RefCell<Vec<crate::ast::LoopKind>>,
    /// Whether the value about to be checked sits in statement position, where
    /// a value-less SUB call is legal (syntaxcheck's `allow_value_less_call`).
    /// Consumed (reset) by the first Call node checked.
    allow_sub_call: Cell<bool>,
    /// The RES-declared binding names of the function currently being checked
    /// (its `resource_owners` table), for the RES ownership-axis rules.
    current_owners: RefCell<HashSet<String>>,
    /// plan-59-C: the names of this function's **bare `RES` parameters** — the one
    /// position where the checker deliberately does not know the concrete `STATE`
    /// (§15.5's parameter row: bare accepts "any state or none").
    ///
    /// A bare parameter and a genuinely stateless resource have the SAME type
    /// string, so `state_type_name` cannot tell them apart; only provenance can.
    /// This set is that provenance, and it is what makes
    /// `TYPE_STATE_OPAQUE_NARROWING` expressible.
    current_opaque_params: RefCell<HashSet<String>>,
    /// Type name → (declaring file, declared visibility) for cross-file
    /// visibility checks (private = same file only).
    type_decl_info: HashMap<String, (String, String)>,
    /// Type name → its explicitly `private` fields (same-file only; other
    /// fields are at least package-visible).
    private_fields: HashMap<String, HashSet<String>>,
}

/// Rules whose failure leaves the failing expression's type undeterminable in
/// syntaxcheck (its `infer_*` returns `Unknown` after reporting them).
const POISONING_RULES: &[&str] = &[
    "TYPE_BINARY_OPERATOR_MISMATCH",
    "TYPE_UNARY_OPERATOR_MISMATCH",
    "TYPE_UNARY_OPERATOR_UNKNOWN",
    "TYPE_REQUIRES_COMPARABLE",
    "TYPE_CALL_ARGUMENT_MISMATCH",
    "TYPE_CALL_ARITY_MISMATCH",
    "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
    "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
    "TYPE_READ_ONLY_RECORD_UPDATE",
    "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
    "TYPE_UNKNOWN_FIELD",
];

impl TypeEnv {
    // ===========================================================================
    // 1. Construction, diagnostic emission, closure-capture arity
    // ===========================================================================

    pub(super) fn build(project: &IrProject) -> Self {
        let mut records = HashMap::new();
        let mut unions = HashMap::new();
        let mut enums: HashMap<String, HashSet<String>> = HashMap::new();
        let mut field_types: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut record_field_lists: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut private_fields: HashMap<String, HashSet<String>> = HashMap::new();
        let type_decl_info: HashMap<String, (String, String)> = project
            .types
            .iter()
            .map(|t| (t.name.clone(), (t.file.clone(), t.visibility.clone())))
            .collect();
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
                    let private: HashSet<String> = ty
                        .fields
                        .iter()
                        .filter(|f| f.visibility.as_deref() == Some("private"))
                        .map(|f| f.name.clone())
                        .collect();
                    if !private.is_empty() {
                        private_fields.insert(ty.name.clone(), private);
                    }
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
                    kind: function.kind.clone(),
                    returns: function.returns.clone(),
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
        let resource_closers = project
            .native_resources
            .iter()
            .map(|r| (r.name.clone(), r.close_function.clone()))
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
            resource_closers,
            closure_counts,
            field_types,
            record_field_lists,
            enums,
            diags: RefCell::new(Vec::new()),
            current_line: Cell::new(0),
            current_file: RefCell::new(String::new()),
            current_return: RefCell::new(String::new()),
            current_kind: RefCell::new(String::new()),
            poisoned: Cell::new(false),
            // Strict by default: the package path (and every unit test) builds the
            // env directly and has the full merged type table. Only
            // `collect_source_diagnostics` opts into the leniency.
            imported_types_unknown: false,
            checking_state_assign: Cell::new(false),
            loop_stack: RefCell::new(Vec::new()),
            allow_sub_call: Cell::new(false),
            current_owners: RefCell::new(HashSet::new()),
            current_opaque_params: RefCell::new(HashSet::new()),
            type_decl_info,
            private_fields,
        }
    }

    /// Record one diagnostic at the current line/file.
    pub(super) fn emit(&self, rule: &str, detail: String) {
        if POISONING_RULES.contains(&rule) {
            self.poisoned.set(true);
        }
        self.diags.borrow_mut().push(Diagnostic {
            rule: rule.to_string(),
            detail,
            file: self.current_file.borrow().clone(),
            line: self.current_line.get(),
        });
    }

    /// The captured-slot bound for a closure-body function, or `None` when the
    /// function is never used as a closure body.
    ///
    /// Ambiguity must not disarm the capture-bounds check: returning `None` when a
    /// body was seen with two different capture-vector lengths let a crafted
    /// package pair `Closure{name:"$l", captures:[a]}` with
    /// `Closure{name:"$l", captures:[a,b]}` and then read `Capture{index:9999}`
    /// out of the environment. Bound against the *smallest* observed count — the
    /// only slot count every call site is guaranteed to have — so the check still
    /// runs. `check_closure_capture_arity` rejects the ambiguous shape itself.
    pub(super) fn closure_slot_count(&self, function: &str) -> Option<usize> {
        self.closure_counts.get(function)?.iter().min().copied()
    }

    /// Reject a closure-body function reached by capture vectors of differing
    /// length. Lowering emits one `Closure` node per body function, so differing
    /// arities cannot arise from source: it is a structural signal of a tampered
    /// package, and it is what disarmed the capture-bounds check above.
    pub(super) fn check_closure_capture_arity(&self, function: &str) {
        let Some(counts) = self.closure_counts.get(function) else {
            return;
        };
        if counts.len() < 2 {
            return;
        }
        let mut arities = counts.iter().copied().collect::<Vec<_>>();
        arities.sort_unstable();
        let arities = arities
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        self.emit(
            VERIFY_TYPE,
            format!(
                "closure body `{function}` is captured with differing capture counts ({arities})"
            ),
        );
    }

    // ===========================================================================
    // 13. Type-model lookup helpers (record_fields, union_variants, infer_type)
    // ===========================================================================

    /// The complete set of field names for a record type, expanding `includes`
    /// transitively. Returns `None` when the type is not a known record or when
    /// an include cannot be resolved (so the field set is incomplete and the
    /// member-existence check must be skipped).
    pub(super) fn record_fields(&self, type_name: &str) -> Option<HashSet<String>> {
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

    pub(super) fn collect_record_fields(
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
    pub(super) fn union_variants(&self, union_type: &str) -> Option<HashSet<String>> {
        let mut out = HashSet::new();
        let mut seen = HashSet::new();
        if self.collect_union_variants(union_type, &mut out, &mut seen) {
            Some(out)
        } else {
            None
        }
    }

    pub(super) fn collect_union_variants(
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
    pub(super) fn infer_type(
        &self,
        value: &IrValue,
        locals: &HashMap<String, String>,
    ) -> Option<String> {
        self.infer_type_depth(value, locals, 0)
    }

    /// Depth-bounded body of `infer_type`. Member-access chains recurse on
    /// expression depth, so — mirroring `check_ops`' cap — the recursion is
    /// bounded to `MAX_DEPTH` levels; past that it fails gracefully by leaving
    /// the type underived (`None`), which the type-relational rules treat
    /// permissively.
    pub(super) fn infer_type_depth(
        &self,
        value: &IrValue,
        locals: &HashMap<String, String>,
        depth: usize,
    ) -> Option<String> {
        if depth > MAX_DEPTH {
            return None;
        }
        match value {
            IrValue::Local(name) => return locals.get(name).cloned(),
            IrValue::Global(name) => return self.globals.get(name).cloned(),
            IrValue::MemberAccess { target, member, .. } => {
                // Prefer the annotated member type; fall back to resolving the
                // field through the target's record type for older shapes.
                if let Some(annotated) = usable_type(value.annotated_type()) {
                    return Some(annotated);
                }
                let target_type = self.infer_type_depth(target, locals, depth + 1)?;
                return self.field_type(&target_type, member);
            }
            _ => {}
        }
        usable_type(value.annotated_type())
    }

    /// The declared type of a record member, for chained member-access
    /// inference. Only resolves through record types whose fields are known.
    pub(super) fn field_type(&self, type_name: &str, member: &str) -> Option<String> {
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
/// The result type a binary operator produces from its operand types, or `None`
/// when it cannot be derived independently of the node's own annotation.
///
/// Comparisons and logical operators always produce `Boolean`, and `&` always
/// produces `String`, whatever their operands. Arithmetic produces its operand
/// type, but only when both operands agree — a mixed or unknown pair is left
/// underived so no valid program is rejected.
fn derived_binary_type(op: &str, left: Option<&str>, right: Option<&str>) -> Option<String> {
    match op {
        "AND" | "OR" | "XOR" | "<" | ">" | "<=" | ">=" | "=" | "<>" => Some("Boolean".to_string()),
        "&" => Some("String".to_string()),
        "+" | "-" | "*" | "/" | "MOD" | "^" => match (left, right) {
            // Money's dimensional algebra is not the "same type in, same type out"
            // heuristic (`M / M → Float`, `M * k → Money`), so consult the lattice
            // whenever a Money operand is present (plan-29-A §4.2).
            (Some(left), Some(right)) if left == "Money" || right == "Money" => {
                crate::numeric::money_result_type(op, left == "Money", right == "Money")
                    .map(str::to_string)
            }
            (Some(left), Some(right)) if left == right => Some(left.to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// The result type a unary operator produces from its operand type: `NOT` is
/// always `Boolean`, and negation preserves its operand's numeric type.
fn derived_unary_type(op: &str, operand: Option<&str>) -> Option<String> {
    match op {
        "NOT" => Some("Boolean".to_string()),
        "-" => operand.map(str::to_string),
        _ => None,
    }
}

fn usable_type(annotated: Option<&str>) -> Option<String> {
    match annotated {
        Some(t) if !t.is_empty() && t != "Unknown" => Some(t.to_string()),
        _ => None,
    }
}

/// Whether an IR value is a numeric literal equal to zero (possibly negated) —
/// mirrors `syntaxcheck::helpers::numeric_literal_is_zero` on the IR shape.
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

/// The source keyword for a loop kind — mirrors `syntaxcheck::helpers`.
fn loop_kind_keyword(kind: crate::ast::LoopKind) -> &'static str {
    match kind {
        crate::ast::LoopKind::For => "FOR",
        crate::ast::LoopKind::Do => "DO",
        crate::ast::LoopKind::While => "WHILE",
    }
}

/// The integer value of a constant expression (possibly negated) — mirrors
/// `syntaxcheck::helpers::integer_constant_value` on the IR shape.
fn integer_constant_value(value: &IrValue) -> Option<i128> {
    match value {
        IrValue::Const { type_, value } if type_ == "Integer" || type_ == "Byte" => {
            value.parse::<i128>().ok()
        }
        IrValue::Unary { op, operand, .. } if op == "-" => {
            integer_constant_value(operand).map(|n| -n)
        }
        _ => None,
    }
}

/// Whether an IR value is a `collections.get`/`getOr` call — a *pointer* to a
/// collection element (mirrors `syntaxcheck::helpers::is_resource_element_pointer`).
pub(crate) fn is_resource_element_pointer(value: &IrValue) -> bool {
    matches!(
        value,
        IrValue::Call { target, .. } | IrValue::CallResult { target, .. }
            if matches!(
                builtins::collections::native_member_bare(target),
                Some("get" | "getOr")
            )
    )
}

/// Compiler-owned record types users may neither construct nor WITH-update —
/// mirrors `syntaxcheck::helpers::read_only_record_type`.
fn read_only_record_type(type_name: &str) -> bool {
    type_name == builtins::term::TERM_COLOR_TYPE
        || type_name == builtins::term::TERM_SIZE_TYPE
        || type_name == builtins::net::ADDRESS_TYPE
        || type_name == builtins::audio::AUDIO_DEVICE_TYPE
        || type_name.starts_with("MapEntry OF ")
}

/// Whether `name` is a built-in resource type (has a registered close op).
fn is_resource_name(name: &str) -> bool {
    builtins::resource::builtin_resource_close_function(name).is_some()
}

/// The base resource type name, stripping the `RES ` ownership marker and a
/// trailing `STATE T` clause (`File STATE Cursor` → `File`). Composite-safe: a
/// `STATE` nested inside a thread plane (`Thread OF RES File STATE Cursor TO Out`)
/// is left intact (plan-54, via `base_resource_name`'s top-level guard).
fn resource_base_type(type_: &str) -> &str {
    let t = type_.strip_prefix("RES ").unwrap_or(type_);
    crate::builtins::resource::base_resource_name(t)
}

/// Collect the names of every `Local` read anywhere in an op's value positions
/// (not its nested bodies — those are traversed separately).
fn collect_local_reads_op(op: &IrOp, out: &mut Vec<String>) {
    let v = |value: &IrValue, out: &mut Vec<String>| collect_local_reads_value(value, out);
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
    collect_local_reads_value_depth(value, out, 0);
}

/// Depth-bounded body of `collect_local_reads_value`. Bounded to `MAX_DEPTH`
/// expression levels (mirroring `check_ops`' cap); past that it stops recursing
/// so a pathologically deep value expression cannot overflow the stack.
fn collect_local_reads_value_depth(value: &IrValue, out: &mut Vec<String>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match value {
        IrValue::Local(name) => out.push(name.clone()),
        IrValue::Call { args, .. } | IrValue::CallResult { args, .. } => {
            for a in args {
                collect_local_reads_value_depth(a, out, depth + 1);
            }
        }
        IrValue::Constructor { args, .. } => {
            for a in args {
                collect_local_reads_value_depth(a, out, depth + 1);
            }
        }
        IrValue::Closure { captures, .. } => {
            for c in captures {
                collect_local_reads_value_depth(c, out, depth + 1);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => {
            collect_local_reads_value_depth(value, out, depth + 1)
        }
        IrValue::Binary { left, right, .. } => {
            collect_local_reads_value_depth(left, out, depth + 1);
            collect_local_reads_value_depth(right, out, depth + 1);
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            collect_local_reads_value_depth(target, out, depth + 1);
            for u in updates {
                collect_local_reads_value_depth(&u.value, out, depth + 1);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for e in values {
                collect_local_reads_value_depth(e, out, depth + 1);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, val) in entries {
                collect_local_reads_value_depth(k, out, depth + 1);
                collect_local_reads_value_depth(val, out, depth + 1);
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
    // The runtime error records (syntaxcheck types their members inline in
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
        .or_else(|| builtins::audio::builtin_type_fields(name))
}

/// Record every `Closure { name, captures }` site's captured-slot count so the
/// capture-bounds rule knows each closure body's env size.
fn collect_closures(value: &IrValue, out: &mut HashMap<String, HashSet<usize>>) {
    collect_closures_depth(value, out, 0);
}

/// Depth-bounded body of `collect_closures`. Bounded to `MAX_DEPTH` expression
/// levels (mirroring `check_ops`' cap); past that it stops recursing so a
/// pathologically deep value expression cannot overflow the stack.
fn collect_closures_depth(
    value: &IrValue,
    out: &mut HashMap<String, HashSet<usize>>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    match value {
        IrValue::Closure { name, captures, .. } => {
            out.entry(name.clone()).or_default().insert(captures.len());
            for capture in captures {
                collect_closures_depth(capture, out, depth + 1);
            }
        }
        IrValue::Call { args, .. } | IrValue::CallResult { args, .. } => {
            for arg in args {
                collect_closures_depth(arg, out, depth + 1);
            }
        }
        IrValue::Constructor { args, .. } => {
            for arg in args {
                collect_closures_depth(arg, out, depth + 1);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => {
            collect_closures_depth(value, out, depth + 1)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            collect_closures_depth(target, out, depth + 1);
            for update in updates {
                collect_closures_depth(&update.value, out, depth + 1);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for v in values {
                collect_closures_depth(v, out, depth + 1);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_closures_depth(k, out, depth + 1);
                collect_closures_depth(v, out, depth + 1);
            }
        }
        IrValue::Binary { left, right, .. } => {
            collect_closures_depth(left, out, depth + 1);
            collect_closures_depth(right, out, depth + 1);
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
fn walk_captures(value: &IrValue, visit: &mut impl FnMut(u32)) {
    walk_captures_depth(value, visit, 0);
}

/// Depth-bounded body of `walk_captures`. Bounded to `MAX_DEPTH` expression
/// levels (mirroring `check_ops`' cap); past that it stops recursing so a
/// pathologically deep value expression cannot overflow the stack.
fn walk_captures_depth(value: &IrValue, visit: &mut impl FnMut(u32), depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    match value {
        IrValue::Capture { index, .. } => visit(*index),
        IrValue::Call { args, .. } | IrValue::CallResult { args, .. } => {
            for arg in args {
                walk_captures_depth(arg, visit, depth + 1);
            }
        }
        IrValue::Closure { captures, .. } => {
            for capture in captures {
                walk_captures_depth(capture, visit, depth + 1);
            }
        }
        IrValue::Constructor { args, .. } => {
            for arg in args {
                walk_captures_depth(arg, visit, depth + 1);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => {
            walk_captures_depth(value, visit, depth + 1)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            walk_captures_depth(target, visit, depth + 1);
            for update in updates {
                walk_captures_depth(&update.value, visit, depth + 1);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for v in values {
                walk_captures_depth(v, visit, depth + 1);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                walk_captures_depth(k, visit, depth + 1);
                walk_captures_depth(v, visit, depth + 1);
            }
        }
        IrValue::Binary { left, right, .. } => {
            walk_captures_depth(left, visit, depth + 1);
            walk_captures_depth(right, visit, depth + 1);
        }
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::LocalRef { .. }
        | IrValue::FunctionRef { .. } => {}
    }
}

mod calls;
mod compat;
mod link;
mod matching;
mod ops;
mod resources;
mod types;
mod values;

#[cfg(test)]
mod tests;
