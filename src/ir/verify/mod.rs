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
use crate::codegen::builtins;
use crate::types::ParameterType;
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

/// One imported package's `RESOURCE_TABLE` row as the source-path checker needs
/// it (bug-377): the type's close op, and whether the exporting package marked
/// the type thread-sendable. A decoded package contributes no
/// `native_resources`, so without these rows every resource rule is inert for
/// an imported type — a double close of a package handle passed clean, and a
/// non-sendable package resource crossed a thread boundary unchallenged.
#[derive(Clone, Debug)]
pub struct ImportedResource {
    pub type_name: String,
    pub close_function: String,
    pub sendable: bool,
}

/// Rules for which `ir::verify` is the sole rejecter (plan-20-Z). On the
/// **source** path `ir::verify` emits ONLY these (syntaxcheck still owns every
/// other rule, so emitting a non-relocated rule here would duplicate it); on
/// the **package** path there is no syntaxcheck, so `ir::verify` emits all of its
/// checks regardless. `syntaxcheck::report` skips this same set. A rule appears
/// here only once `ir::verify` reproduces it completely (verified against every
/// `*-invalid` fixture).
///
/// The pre-lowering shape pass's rules (`ir::shape`, plan-107-E) are listed
/// here too: the list is the single "no longer syntaxcheck's" register that the
/// `syntaxcheck::report` guard reads, and `ir::verify` never emits them, so the
/// source-path filter is unaffected.
pub const RELOCATED_TO_IR_VERIFY: &[&str] = &[
    // ir::shape (plan-107-E): lowering normalizes named arguments away, so the
    // name the source wrote survives only in the HIR.
    "TYPE_UNKNOWN_ARGUMENT_NAME",
    "TYPE_DUPLICATE_ARGUMENT_NAME",
    // plan-107-E: builtin and function-value forms in ir::verify; the user-FUNC
    // form is ir::shape's on the source path (lowering normalizes the argument
    // list — extras dropped, defaults filled — so the count is erased) and
    // verify's on the package path; the named-argument omission forms are
    // ir::shape's.
    "TYPE_CALL_ARITY_MISMATCH",
    // plan-107-E: the argument-TYPE forms (declared FUNC, function value, every
    // builtin arm) are ir::verify's on both paths; the three source-only forms
    // — named arguments on a function value, thread.start's entry, term's
    // drawText(AttributedString) without IMPORT astrings — are ir::shape's.
    "TYPE_CALL_ARGUMENT_MISMATCH",
    // plan-107-E: the cascade — ir::shape's for an initializer/RETURN/default
    // the checker could not type (lowering's seam has no type, or the shape's
    // own call rules typed the call Unknown) and for the two not-a-local-binding
    // target forms; ir::verify's for a typed node its own rules poisoned.
    "TYPE_UNKNOWN_VALUE",
    // plan-107-D: ir::shape's — `EXIT FUNC` lowers to nothing and `EXIT SUB` to
    // a bare Return, so neither statement exists as such in the IR.
    "EXIT_FUNC_FORBIDDEN",
    "EXIT_SUB_IN_FUNC",
    // plan-107-D: ir::shape's — the bare-RETURN-in-a-SUB form (verify keeps
    // the valued form), a stray RECOVER (lowered to a `$recover_stray` temp),
    // RECOVER's two count forms (verify keeps the value-type form), the inline
    // handler's fall-through edge, and EXIT SUB/FUNC/PROGRAM's unreachable tail
    // (verify keeps the loop-exit forms).
    "SUB_RETURN_FORBIDDEN",
    "TYPE_RECOVER_OUTSIDE_INLINE_TRAP",
    "TYPE_RECOVER_TYPE_MISMATCH",
    "TYPE_INLINE_TRAP_FALLS_THROUGH",
    "UNREACHABLE_AFTER_EXIT",
    // plan-107-D: ir::shape's — the assertion builtins' argument rules; lowering
    // expands `expectX(...)` into comparisons + FAIL or a trap guard.
    "TESTING_EXPECT_ARITY",
    "TESTING_EXPECT_TYPE_MISMATCH",
    "TESTING_EXPECT_INCOMPARABLE",
    "TESTING_EXPECT_NOT_PRINTABLE",
    "TESTING_EXPECT_CODE_TYPE",
    "TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE",
    // plan-107-D: ir::shape's — the literal's SPELLING (`1.08` vs `1.08f`) is the
    // evidence, and lowering stamps both as the same Float const.
    "MONEY_INEXACT_FLOAT_LITERAL",
    // plan-107-D: ir::shape's constructor form (lowering reorders named
    // arguments into field order, so the repetition is gone); verify keeps the
    // WITH form.
    "TYPE_DUPLICATE_FIELD",
    // plan-107-D: split with ir::shape — shape holds the "not a constant the
    // compiler can fold" CONST form (lowering folds the pin) and the FREE
    // deallocator-signature form (`IrFree` keeps slot + symbol only); verify
    // keeps the unknown-slot, `AS RES` producer and empty-symbol forms.
    "NATIVE_CONST_UNKNOWN_SLOT",
    "NATIVE_FREE_INVALID",
    // plan-107-D: split — ir::shape holds the `Error`/`ErrorLoc` form (lowering
    // synthesizes `Constructor{Error}` itself); verify the compiler-owned and
    // `AttributedString` forms.
    "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
    // plan-107-D: ir::shape's import walk (the (I) relocation of the checker's
    // package-metadata validation).
    "PACKAGE_INVALID",
    "TYPE_BINARY_OPERATOR_MISMATCH",
    "TYPE_UNARY_OPERATOR_MISMATCH",
    "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
    "TYPE_UNKNOWN_FIELD",
    "TYPE_RETURN_MISMATCH",
    "TYPE_LIST_ELEMENT_MISMATCH",
    "TYPE_SET_ELEMENT_MISMATCH",
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
    // plan-74 retired TYPE_UNION_STATE_FORBIDDEN (a resource union may carry
    // uniform STATE); it is no longer emitted, so it leaves this located-rules list.
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
    // plan-107-A pilots (decl-level / expression-level / inference-fact port).
    "TYPE_ISOLATED_NOT_VISIBLE",
    "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
    "TYPE_THREAD_NOT_SENDABLE",
    // plan-107-B: the general semantic cluster.
    "TYPE_INLINE_TRAP_DEAD_HANDLER",
    "TYPE_TRAP_FALLTHROUGH",
    "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
    // plan-107-C: the native LINK/ABI family.
    "NATIVE_ABI_NO_RESULT",
    "NATIVE_ABI_RESULT_MARKER",
    "NATIVE_ABI_UNBOUND_PARAM",
    "NATIVE_BIND_IN_INVALID",
    "NATIVE_CSTRUCT_ESCAPE",
    // The eight the shared `ir::link` fault helpers also produce, listed
    // together with the removal of syntaxcheck's fault loops.
    "NATIVE_ABI_UNBOUND_SLOT",
    "NATIVE_ABI_UNKNOWN_CTYPE",
    "NATIVE_CONST_OUT",
    "NATIVE_CPTR_ESCAPE",
    "NATIVE_CSTRUCT_INVALID",
    "NATIVE_STRUCT_FIELD_MISMATCH",
    "NATIVE_BUFFER_INVALID",
    "NATIVE_CSTRUCT_TOO_LARGE",
];

/// Diagnostic prefixes shared with the structural `verify_package` checks so a
/// rejection surfaces as a `PACKAGE_BINARY_REPRESENTATION_*` diagnostic. Forward
/// to the single `crate::ir` source (bug-342 A3) so the two enforcement points
/// spell each rule identically.
const VERIFY_TYPE: &str = crate::ir::VERIFY_TYPE;
const VERIFY_MATCH: &str = crate::ir::VERIFY_MATCH;

/// The base primitive/scalar type set (bug-342 A9). A value can never be
/// member-accessed through one of these, so a `MemberAccess` whose target
/// provably has one is a type confusion. Every other primitive-set predicate in
/// this module derives from this base plus explicitly-named deltas, so adding a
/// primitive here flows to all of them and none can silently drift.
const PRIMITIVE_TYPES: &[&str] = &[
    "Integer", "Float", "String", "Boolean", "Byte", "Fixed", "Nothing", "Money", "Scalar",
];

/// Whether `type_` is BOTH directly comparable and directly defaultable — the
/// identical set `is_comparable_seen` and `is_defaultable` each opened with
/// (bug-342 A9). It is the `PRIMITIVE_TYPES` base plus two deltas: `Error`/
/// `ErrorLoc` (errors compare and default as ordinary values) and `Unknown`
/// (treated permissively — an unresolved type is not rejected here).
fn is_comparable_defaultable_primitive(type_: &str) -> bool {
    PRIMITIVE_TYPES.contains(&type_) || matches!(type_, "Error" | "ErrorLoc" | "Unknown")
}

/// Collect every semantic-verification diagnostic for a merged `IrProject`, in
/// the traversal order the AST type checker uses (functions in declaration
/// order; each body's ops in order; each op's sub-values innermost-first). The
/// checker never short-circuits, so a program with several violations yields
/// them all.
pub(crate) fn collect_diagnostics(project: &IrProject) -> Vec<Diagnostic> {
    // The package path runs on merged IR, whose resource types are already in
    // `native_resources` or are the package's own, so it registers no extra rows;
    // and a decoded package has no source, so its LINK rules report unlocated.
    collect_diagnostics_with(project, false, &[], &crate::ir::LinkSpans::default(), false)
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
    imported_resources: &[ImportedResource],
    link_spans: &crate::ir::LinkSpans,
    source_path: bool,
) -> Vec<Diagnostic> {
    let mut env = TypeEnv::build(project);
    env.imported_types_unknown = imported_types_unknown;
    env.link_spans = link_spans.clone();
    env.source_path.set(source_path);
    // bug-377: seed the imported packages' `RESOURCE_TABLE` rows. The project's
    // own `native_resources` win — an importer never overrides a declaration it
    // can see the source of.
    for imported in imported_resources {
        env.resource_closers
            .entry(imported.type_name.clone())
            .or_insert_with(|| imported.close_function.clone());
        env.resource_sendable
            .entry(imported.type_name.clone())
            .or_insert(imported.sendable);
    }
    let env = env;
    for function in &project.functions {
        env.current_file.replace(function.file.clone());
        env.current_return.replace(function.returns.clone());
        env.current_kind.replace(function.kind.clone());
        env.current_function.replace(function.name.clone());
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
                    env.is_resource_or_resource_union(&resource_base_type(&p.type_).name())
                        && p.type_.state().is_none()
                })
                .map(|p| p.name.clone())
                .collect(),
        );
        // An ISOLATED function is a thread entry point, reached by name from
        // another package's `thread::start`, so it must be a project-visible
        // FUNC (bug-227). `IrFunction` carries all three facts.
        if function.isolated && (function.kind != "func" || function.visibility == "private") {
            env.current_line.set(function.loc.line);
            env.emit(
                "TYPE_ISOLATED_NOT_VISIBLE",
                format!(
                    "ISOLATED function `{}` must be a project-visible FUNC declaration \
                     (PUBLIC — the default — or EXPORT, not PRIVATE).",
                    function.name
                ),
            );
        }
        // A declared return type is a type reference too (`AS List OF File`
        // needs the RES element marking like any collection declaration).
        if !function.name.starts_with('$') {
            env.current_line.set(function.loc.line);
            env.check_collection_res_axis(&resource_base_type(&function.returns));
            env.check_return_state_declaration(function);
            env.check_thread_sendability(&function.returns.without_state());
            if let Some(state) = function.returns.state() {
                env.check_thread_sendability(&state);
            }
        }
        // A declared FUNC must name its return type (`AS T`); lowering stamps
        // `Unknown` when the annotation is absent. Synthesized `$lambda` bodies
        // legitimately carry a computed (possibly Unknown) return — skip them.
        if function.kind == "func"
            && function.returns == ParameterType::Unknown
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
            && function.returns != ParameterType::Nothing
            && function.returns != ParameterType::Unknown
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
        let mut locals: HashMap<String, ParameterType> = HashMap::new();
        let mut muts: HashMap<String, bool> = HashMap::new();
        let mut seen_default = false;
        for param in &function.params {
            env.current_line.set(param.loc.line);
            locals.insert(param.name.clone(), param.type_.clone());
            env.check_map_key_comparable(&param.type_);
            env.check_collection_res_axis(&resource_base_type(&param.type_));
            env.check_thread_sendability(&param.type_.without_state());
            if let Some(state) = param.type_.state() {
                env.check_thread_sendability(&state);
            }
            // Every parameter must declare an `AS` type (lambda parameters
            // included — syntaxcheck checks both forms with this rule).
            if param.type_ == ParameterType::Unknown {
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
                if !matches!(expected, ParameterType::Unknown | ParameterType::Nothing)
                    && !expected.name().is_empty()
                {
                    if let Some(actual) = env.infer_type(default, &locals) {
                        if !env.expression_compatible(&expected, &actual, default) {
                            let (actual, expected) = (actual.name(), expected.name());
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
            .filter(|p| env.is_resource_or_resource_union(&resource_base_type(&p.type_).name()))
            .map(|p| p.name.clone())
            .collect();
        // A RES binding whose ownership floats into a collection
        // (ResOwner::Float) is non-owning afterwards: the collection owns the
        // close obligation (§15.6).
        for (name, owner) in &function.resource_owners {
            if matches!(owner, crate::ir::resource_escape::ResOwner::Float(_)) {
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
            if let crate::ir::resource_escape::ResOwner::FloatBlocked(collection) = owner {
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
        let binding_type = &binding.type_;
        if binding.explicit_type {
            env.check_map_key_comparable(binding_type);
            env.check_collection_res_axis(&resource_base_type(binding_type));
            env.check_thread_sendability(&binding_type.without_state());
            if let Some(state) = binding_type.state() {
                env.check_thread_sendability(&state);
            }
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
            } else if !env.is_defaultable(binding_type, &mut HashSet::new()) {
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
            let range_errored =
                env.check_literal_range_errored(&resource_base_type(binding_type), value);
            if !range_errored && binding.explicit_type {
                env.check_binding_type(&binding.name, binding_type, value, &HashMap::new());
            }
        }
    }
    env.check_type_declarations(project);
    env.check_link_blocks(project);
    env.diags.take()
}

/// Verify the merged `IrProject` on the **package path** (`merge_packages`).
/// Returns `Ok(())` when the IR is well formed, or the first violation as an
/// error string. Package-path diagnostics carry no source context (the decoded
/// `.mfp` has no source file), so first-error is sufficient here.
pub fn check(project: &IrProject) -> Result<(), String> {
    // Only an error-severity rule rejects: an advisory (`Severity::Warn`) rule
    // such as TYPE_INLINE_TRAP_DEAD_HANDLER is rendered by the source path and
    // must not fail the merged-project gate (plan-107-B found this the moment
    // the first warning-severity rule moved here).
    // The structural `PACKAGE_BINARY_REPRESENTATION_*` pseudo-rules are not
    // table rows (they are prefixes shared with `verify_package`); they are
    // always rejections.
    match collect_diagnostics(project).into_iter().find(|d| {
        d.rule == VERIFY_TYPE || d.rule == VERIFY_MATCH || crate::rules::is_error(&d.rule)
    }) {
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
    imported_resources: &[ImportedResource],
    link_spans: &crate::ir::LinkSpans,
) -> Vec<crate::rules::PendingDiagnostic> {
    collect_diagnostics_with(project, true, imported_resources, link_spans, true)
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
/// own functions), so it re-imposes the bound independently. Forwards to the
/// single `crate::ir` source (bug-342 A3) so it cannot drift from the decoder.
const MAX_DEPTH: usize = crate::ir::MAX_IR_NESTING_DEPTH;

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
    params: Vec<ParameterType>,
    /// `func` or `sub` — a SUB call produces no value (TYPE_SUB_HAS_NO_VALUE).
    kind: String,
    /// The declared return type. A call node carries its own result type, which
    /// on decoded package IR is attacker-controlled; this is the independent
    /// truth it is reconciled against (`check_call_result_type`).
    returns: ParameterType,
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
    globals: HashMap<String, ParameterType>,
    /// Global binding name → whether it was declared `MUT` (assignable).
    global_muts: HashMap<String, bool>,
    /// User-declared native resource type → its registered close op (dotted
    /// `alias.func`), complementing the builtin close table for the
    /// use-after-move pass.
    resource_closers: HashMap<String, String>,
    /// User-declared native resource type → whether it may cross a thread
    /// boundary (`RESOURCE … THREAD_SENDABLE`), plus the imported packages'
    /// `RESOURCE_TABLE` sendable bits. Built-in resources are not here; the
    /// registry answers for them (`is_builtin_sendable_resource_type`).
    resource_sendable: HashMap<String, bool>,
    /// The LINK declarations' source spans (plan-107-C), present on the source
    /// path only; the native-ABI rules report at these lines, and unlocated
    /// (the `<generated>` form) when a declaration has none.
    link_spans: crate::ir::LinkSpans,
    /// Function name → the distinct captured-slot counts observed at the
    /// `Closure` sites that target it. A single count means the closure shape is
    /// known; zero or multiple distinct counts leaves it ambiguous (skip).
    closure_counts: HashMap<String, HashSet<usize>>,
    /// Record type name → (member name → declared member type), for chained
    /// member-access type inference.
    field_types: HashMap<String, HashMap<String, ParameterType>>,
    /// Record type name → its direct fields as ordered (name, type) pairs, for
    /// positional constructor checking (mirrors syntaxcheck's `TypeInfo.fields`,
    /// which is declaration-ordered and not include-expanded).
    record_field_lists: HashMap<String, Vec<(String, ParameterType)>>,
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
    current_return: RefCell<ParameterType>,
    /// `kind` (`func`/`sub`) of the function currently being checked.
    current_kind: RefCell<String>,
    /// Name of the function currently being checked, for diagnostics that name
    /// it (the normal-flow form of TYPE_TRAP_FALLTHROUGH).
    current_function: RefCell<String>,
    /// The mutability of every local in scope at the op whose values are being
    /// checked — a snapshot of `check_ops`'s `muts`, taken only for ops that
    /// carry a `Closure`, for the lambda-capture rule (a by-value capture of a
    /// `MUT` local is the "mutable capture" rejection).
    current_muts: RefCell<HashMap<String, bool>>,
    /// Whether a type-poisoning rule fired while checking the current value —
    /// syntaxcheck's inference yields `Unknown` after an operator/constructor
    /// failure, cascading a TYPE_UNKNOWN_VALUE at the consuming statement even
    /// where lowering stamped a nominal result type. Reset per checked value.
    poisoned: Cell<bool>,
    /// Whether this walk is the source path (build: `syntaxcheck` and
    /// `ir::shape` run beside it) rather than the package path (`check`, where
    /// verify is the only checker). A rule whose evidence lowering erases on
    /// the source path — the user-FUNC call arity — is `ir::shape`'s there and
    /// verify's structural check here only on the package path (plan-107-E).
    source_path: Cell<bool>,
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
        let mut field_types: HashMap<String, HashMap<String, ParameterType>> = HashMap::new();
        let mut record_field_lists: HashMap<String, Vec<(String, ParameterType)>> = HashMap::new();
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
        let resource_sendable = project
            .native_resources
            .iter()
            .map(|r| (r.name.clone(), r.sendable))
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
            resource_sendable,
            link_spans: crate::ir::LinkSpans::default(),
            closure_counts,
            field_types,
            record_field_lists,
            enums,
            diags: RefCell::new(Vec::new()),
            current_line: Cell::new(0),
            current_file: RefCell::new(String::new()),
            current_return: RefCell::new(ParameterType::Unknown),
            current_kind: RefCell::new(String::new()),
            current_function: RefCell::new(String::new()),
            current_muts: RefCell::new(HashMap::new()),
            poisoned: Cell::new(false),
            source_path: Cell::new(false),
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
        // The read-only compiler/runtime records `Error`/`ErrorLoc` carry their
        // fields in a local table rather than the project type table.
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
        locals: &HashMap<String, ParameterType>,
    ) -> Option<ParameterType> {
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
        locals: &HashMap<String, ParameterType>,
        depth: usize,
    ) -> Option<ParameterType> {
        if depth > MAX_DEPTH {
            return None;
        }
        match value {
            IrValue::Local(name) => return locals.get(name).cloned(),
            IrValue::Global(name) => return self.globals.get(name).cloned(),
            IrValue::MemberAccess { target, member, .. } => {
                // Prefer the annotated member type; fall back to resolving the
                // field through the target's record type for older shapes.
                if let Some(annotated) = usable_type(value.annotated_parameter_type()) {
                    return Some(annotated);
                }
                let target_type = self.infer_type_depth(target, locals, depth + 1)?;
                return self.field_type(&target_type, member);
            }
            _ => {}
        }
        usable_type(value.annotated_parameter_type())
    }

    /// The declared type of a record member, for chained member-access
    /// inference. Only resolves through record types whose fields are known.
    pub(super) fn field_type(&self, type_: &ParameterType, member: &str) -> Option<ParameterType> {
        // The record is identified by NAME (the field tables are name-keyed
        // declaration maps), so the type renders for the lookup.
        let type_name = type_.name();
        if let Some(fields) = builtin_type_fields(&type_name) {
            return fields
                .iter()
                .find(|(name, _)| *name == member)
                .map(|(_, type_)| type_.clone());
        }
        // Project records store field types on the IrType; look them up via the
        // dedicated map built alongside `records`.
        self.field_types
            .get(type_name.as_ref())
            .and_then(|fields| fields.get(member).cloned())
    }
}

/// The result type a binary operator produces from its operand types, or `None`
/// when it cannot be derived independently of the node's own annotation.
///
/// Comparisons and logical operators always produce `Boolean`, and `&` always
/// produces `String`, whatever their operands. Arithmetic produces its operand
/// type, but only when both operands agree — a mixed or unknown pair is left
/// underived so no valid program is rejected.
fn derived_binary_type(
    op: &str,
    left: Option<&ParameterType>,
    right: Option<&ParameterType>,
) -> Option<ParameterType> {
    match op {
        "AND" | "OR" | "XOR" | "<" | ">" | "<=" | ">=" | "=" | "<>" => Some(ParameterType::Boolean),
        "&" => Some(ParameterType::String),
        "+" | "-" | "*" | "/" | "MOD" | "^" => match (left, right) {
            // Money's dimensional algebra is not the "same type in, same type out"
            // heuristic (`M / M → Float`, `M * k → Money`), so consult the lattice
            // whenever a Money operand is present (plan-29-A §4.2).
            (Some(left), Some(right))
                if matches!(left, ParameterType::Money)
                    || matches!(right, ParameterType::Money) =>
            {
                crate::numeric::typed_money_result_type(
                    op,
                    matches!(left, ParameterType::Money),
                    matches!(right, ParameterType::Money),
                )
            }
            (Some(left), Some(right)) if left == right => Some(left.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// The result type a unary operator produces from its operand type: `NOT` is
/// always `Boolean`, and negation preserves its operand's numeric type.
fn derived_unary_type(op: &str, operand: Option<&ParameterType>) -> Option<ParameterType> {
    match op {
        "NOT" => Some(ParameterType::Boolean),
        "-" => operand.cloned(),
        _ => None,
    }
}

/// A node's annotated result type, or `None` when it is absent, empty, or the
/// explicit `"Unknown"` marker lowering stamps when it cannot name a type.
/// Filtering `"Unknown"` here is what keeps the type-relational rules from
/// rejecting a node whose type simply could not be reconstructed (plan-20-C).
/// The annotated type of a node, when it is usable at all.
///
/// plan-106-A/B: takes and returns a [`ParameterType`]. It reproduces the string
/// form's two rejections exactly:
///
/// * the `Unknown` **sentinel** — and the variant test is complete for every
///   parsed input, because `parse("Unknown")` returns the variant rather than a
///   nominal (`src/types.rs:274`);
/// * an **empty** spelling, which is what a malformed or hostile decoded package
///   IR node yields (`parse("")` → `Named("")`). Kept as a name test because that
///   is precisely a check on the *spelling*, and this is a hardening path.
fn usable_type(annotated: Option<ParameterType>) -> Option<ParameterType> {
    match annotated {
        None | Some(ParameterType::Unknown) => None,
        Some(type_) if type_.name().is_empty() => None,
        Some(type_) => Some(type_),
    }
}

/// Whether an IR value is a numeric literal equal to zero (possibly negated) —
/// mirrors `syntaxcheck::helpers::numeric_literal_is_zero` on the IR shape.
fn numeric_literal_is_zero(value: &IrValue) -> bool {
    match value {
        IrValue::Const { type_, value }
            if matches!(
                type_.name().as_ref(),
                "Integer" | "Float" | "Byte" | "Fixed"
            ) =>
        {
            value.parse::<f64>().is_ok_and(|n| n == 0.0)
        }
        IrValue::Unary { op, operand, .. } if op == "-" => numeric_literal_is_zero(operand),
        _ => false,
    }
}

/// bug-342 A5: the shared MATCH-coverage fold used by both `match_covers_all`
/// (`verify/resources.rs`) and `check_match_exhaustive` (`verify/matching.rs`).
/// Walks the cases, skipping guarded arms (a guard may not fire, so it covers
/// nothing), folds every `Value`/`OneOf` variant/member name into `covered`, and
/// reports whether an unguarded `CASE ELSE` (a catch-all) was seen. Both callers
/// early-out on `has_unguarded_else`, so folding cases after an ELSE is harmless
/// and keeps behaviour identical to the two hand-rolled loops this replaces.
fn fold_match_coverage(
    cases: &[crate::ir::IrMatchCase],
) -> (std::collections::HashSet<String>, bool) {
    let name_of = |v: &IrValue| match v {
        IrValue::Local(name) => Some(name.clone()),
        IrValue::MemberAccess { member, .. } => Some(member.clone()),
        _ => None,
    };
    let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut has_unguarded_else = false;
    for case in cases {
        if case.guard.is_some() {
            continue;
        }
        match &case.pattern {
            crate::ir::IrMatchPattern::Else => has_unguarded_else = true,
            crate::ir::IrMatchPattern::Value(v) => {
                if let Some(name) = name_of(v) {
                    covered.insert(name);
                }
            }
            crate::ir::IrMatchPattern::OneOf(vs) => {
                for v in vs {
                    if let Some(name) = name_of(v) {
                        covered.insert(name);
                    }
                }
            }
        }
    }
    (covered, has_unguarded_else)
}

/// The integer value of a constant expression (possibly negated) — mirrors
/// `syntaxcheck::helpers::integer_constant_value` on the IR shape.
fn integer_constant_value(value: &IrValue) -> Option<i128> {
    match value {
        IrValue::Const { type_, value }
            if matches!(
                type_,
                crate::types::ParameterType::Integer | crate::types::ParameterType::Byte
            ) =>
        {
            value.parse::<i128>().ok()
        }
        IrValue::Unary { op, operand, .. } if op == "-" => {
            // `wrapping_neg` so a negated `i128::MIN` operand does not
            // overflow-panic in debug. Wrapping preserves the release-build
            // behavior exactly (`-i128::MIN` wraps back to `i128::MIN`), which
            // is still out of the 0..255 exit-code range and thus reported as
            // `EXIT_PROGRAM_CODE_OUT_OF_RANGE` rather than silently accepted.
            integer_constant_value(operand).map(|n| n.wrapping_neg())
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
                crate::codegen::registry::native_bare_target(target),
                Some("get" | "getOr")
            )
    )
}

/// Compiler-owned record types users may neither construct nor WITH-update —
/// mirrors `syntaxcheck::helpers::read_only_record_type`.
fn read_only_record_type(type_: &ParameterType) -> bool {
    // A `MapEntry OF K TO V` is read-only structurally; the rest are nominal
    // lookups into per-package read-only tables, which are keyed by NAME.
    if matches!(type_, ParameterType::MapEntryOf(_, _)) {
        return true;
    }
    let type_name = type_.name();
    crate::codegen::builtins::term::is_read_only_record(&type_name)
        || type_name == crate::codegen::builtins::net::ADDRESS_TYPE
        || type_name == crate::codegen::builtins::audio::AUDIO_DEVICE_TYPE
}

/// Whether `name` is a built-in resource type (has a registered close op).
fn is_resource_name(name: &str) -> bool {
    crate::codegen::resource::builtin_resource_close_function(name).is_some()
}

/// The base resource type name, stripping the `RES ` ownership marker and a
/// trailing `STATE T` clause (`File STATE Cursor` → `File`). Composite-safe: a
/// `STATE` nested inside a thread plane (`Thread OF RES File STATE Cursor TO Out`)
/// is left intact (plan-54, via `base_resource_name`'s top-level guard).
fn resource_base_type(type_: &ParameterType) -> ParameterType {
    // plan-106-C: `ParameterType::without_state` is the structural splitter, so
    // this no longer renders the type and re-parses the base out of the spelling.
    // It keeps the same top-level guard, which leaves a `STATE` nested inside a
    // thread plane intact (plan-54).
    strip_res(type_).without_state()
}

/// The name-domain twin of [`resource_base_type`], for the callers that hold a
/// type SPELLING rather than a type: the `LINK` block's raw AST strings (an
/// un-elaborated `crate::ast::LinkBlock` — see `src/hir/mod.rs:435`).
fn resource_base_type_name(type_: &str) -> String {
    // plan-106-E: routed through [`resource_base_type`] so the `RES` peel and the
    // top-level `STATE` guard are stated once, structurally, instead of a local
    // `strip_prefix` beside them.
    resource_base_type(&ParameterType::parse(type_))
        .name()
        .into_owned()
}

/// Whether a type is a thread handle — structurally, **or** by a spelling that
/// merely begins `Thread`/`ThreadWorker`.
///
/// The name test is not redundant, and it is not laziness: this checker's whole
/// job is decoded, attacker-controlled package IR (PKG-02), where a type string
/// need not be well formed. `ParameterType::parse` builds a
/// [`ThreadHandle`](ParameterType::ThreadHandle) only from the complete
/// `Thread OF Msg [RES R] TO Out` shape; a truncated `Thread OF Integer` falls
/// through to an opaque `Named`. A crafted `.mfp` can carry exactly that, and so
/// can a legitimate `Map OF Thread OF Integer TO Integer`, whose key the
/// `Map OF K TO V` grammar cannot spell unambiguously (`split_top_level_to`
/// takes the FIRST ` TO `). The pre-plan-106 code was a bare
/// `starts_with("Thread")`, so keeping the name arm preserves its exact reach
/// while the structural arm handles every well-formed handle.
///
/// The name arm is checked on a WORD boundary — `Thread`/`ThreadWorker` alone,
/// or followed by ` OF `. The pre-plan-106 test was a bare
/// `starts_with("Thread")`, which also matched an ordinary user record named
/// `Threadbare` and would have mis-reported it as owning a thread handle. That
/// over-match is deliberately dropped; nothing in the corpus relies on it
/// (verified by the full `*-invalid` diagnostic corpus + `artifact-gate all`).
///
/// Pinned by `truncated_thread_spelling_still_counts_as_a_thread`.
fn is_thread_type(type_: &ParameterType) -> bool {
    if matches!(type_, ParameterType::ThreadHandle { .. }) {
        return true;
    }
    let name = type_.name();
    for keyword in [crate::types::THREAD_TYPE, crate::types::THREAD_WORKER_TYPE] {
        if name == keyword || name.starts_with(&format!("{keyword} OF ")) {
            return true;
        }
    }
    false
}

/// A type with its `RES ` ownership marker removed, if it carries one — the
/// structural form of the `strip_prefix("RES ")` this replaced.
fn strip_res(type_: &ParameterType) -> &ParameterType {
    match type_ {
        ParameterType::Res(inner) => inner,
        other => other,
    }
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
/// Collect every `Local` name read anywhere in `value`. Descends through the
/// shared depth-bounded [`visit_value`](crate::ir::value::visit_value) seam
/// (bug-328), which preserves the `MAX_DEPTH` cutoff the hand-written walk had.
fn collect_local_reads_value(value: &IrValue, out: &mut Vec<String>) {
    crate::ir::value::visit_value(value, &mut |v| {
        if let IrValue::Local(name) = v {
            out.push(name.clone());
        }
    });
}

/// Build a `member → type` map from a record's declared fields.
fn field_type_map(fields: &[IrField]) -> HashMap<String, ParameterType> {
    fields
        .iter()
        .map(|f| (f.name.clone(), f.type_.clone()))
        .collect()
}

/// The read-only compiler/runtime records `Error`/`ErrorLoc` expose their fields
/// through this local table rather than the project type table. Syntaxcheck types
/// their members inline in `infer_member`; listed here so member-access inference
/// resolves `err.source.line` chains and the read-only WITH check sees ErrorLoc.
fn builtin_type_fields(name: &str) -> Option<Vec<(&'static str, ParameterType)>> {
    match name {
        "Error" => Some(vec![
            ("code", ParameterType::Integer),
            ("message", ParameterType::String),
            ("source", ParameterType::named("ErrorLoc")),
        ]),
        "ErrorLoc" => Some(vec![
            ("filename", ParameterType::String),
            ("line", ParameterType::Integer),
            ("char", ParameterType::Integer),
        ]),
        _ => None,
    }
}

/// Record every `Closure { name, captures }` site's captured-slot count so the
/// capture-bounds rule knows each closure body's env size. Descends through the
/// shared depth-bounded [`visit_value`](crate::ir::value::visit_value) seam
/// (bug-328); the `MAX_DEPTH` cutoff is preserved.
fn collect_closures(value: &IrValue, out: &mut HashMap<String, HashSet<usize>>) {
    crate::ir::value::visit_value(value, &mut |v| {
        if let IrValue::Closure { name, captures, .. } = v {
            out.entry(name.clone()).or_default().insert(captures.len());
        }
    });
}

/// Whether one of `op`'s OWN values (not a nested body's) contains a `Closure`
/// node — the trigger for snapshotting local mutability before the op's
/// values are checked (see `TypeEnv::current_muts`).
fn op_carries_closure(op: &IrOp) -> bool {
    fn has_closure(value: &IrValue) -> bool {
        let mut found = false;
        crate::ir::value::visit_value(value, &mut |v| {
            if matches!(v, IrValue::Closure { .. }) {
                found = true;
            }
        });
        found
    }
    match op {
        IrOp::Bind { value: Some(v), .. } | IrOp::Return { value: Some(v), .. } => has_closure(v),
        IrOp::Bind { value: None, .. }
        | IrOp::Return { value: None, .. }
        | IrOp::ExitLoop { .. }
        | IrOp::ContinueLoop { .. }
        | IrOp::Trap { .. } => false,
        IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value, .. }
        | IrOp::ExitProgram { code: value, .. }
        | IrOp::Fail { error: value, .. }
        | IrOp::If {
            condition: value, ..
        }
        | IrOp::Match { value, .. }
        | IrOp::While {
            condition: value, ..
        }
        | IrOp::DoUntil {
            condition: value, ..
        }
        | IrOp::ForEach {
            iterable: value, ..
        } => has_closure(value),
        IrOp::For {
            start, end, step, ..
        } => has_closure(start) || has_closure(end) || has_closure(step),
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
/// Call `visit` with the env slot index of every `Capture` anywhere in `value`.
/// Descends through the shared depth-bounded
/// [`visit_value`](crate::ir::value::visit_value) seam (bug-328); the
/// `MAX_DEPTH` cutoff is preserved.
fn walk_captures(value: &IrValue, visit: &mut impl FnMut(u32)) {
    crate::ir::value::visit_value(value, &mut |v| {
        if let IrValue::Capture { index, .. } = v {
            visit(*index);
        }
    });
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
