//! plan-176-N: defaultability and `STATE` clauses — which types have a default value and
//! which `STATE` payload types and hosts are legal. Resource-move checking lives in
//! `ir/verify/resources/mod.rs`.

use super::*;

impl TypeEnv {
    /// Whether a type has a defined default value: primitives yes, functions/
    /// results/resources/threads/unions/enums no, collections and records
    /// recurse (cycle-guarded).
    ///
    /// plan-106-B: the container/FUNC/Result/RES/Thread tests are variant matches.
    /// The ` STATE ` test stays a name test on purpose: `parse` builds a
    /// [`Stateful`](ParameterType::Stateful) only for a single-token base or a
    /// thread handle, and a clause on any other composite still rides inside one
    /// opaque nominal spelling, where only the name can see it. The remaining tests
    /// are name lookups into the declaration tables.
    pub(in crate::ir::verify) fn is_defaultable(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<ParameterType>,
    ) -> bool {
        let type_name = type_.name();
        if is_comparable_defaultable_primitive(type_) {
            return true;
        }
        // `AttributedString` (plan-89-A) is defaultable (its default is empty
        // text + empty overlay) but NOT comparable, so it is a defaultable-only
        // delta here rather than in `is_comparable_defaultable_primitive`.
        if type_name == "AttributedString" {
            return true;
        }
        // bug-434: a collection is ALWAYS defaultable — its default is the empty
        // collection, which materializes NO element, so the element type's own
        // defaultability is irrelevant. Recursing into the element (as this did
        // before) wrongly rejected `List/Set/Map OF <union|enum|FUNC|RES>` and
        // cascaded that rejection into any record embedding such a field. Codegen
        // (`lower_default_value` → `lower_empty_collection`) already materializes
        // the empty form without recursing into the element, so this only lifts a
        // front-end block. Comparability of a `Set`/`Map`-key element is a
        // SEPARATE check and is unaffected. Kept ahead of the FUNC/RES/STATE and
        // union/enum arms so the collection short-circuit wins.
        if matches!(
            type_,
            ParameterType::ListOf(_) | ParameterType::SetOf(_) | ParameterType::MapOf(_, _)
        ) {
            return true;
        }
        if matches!(
            type_,
            ParameterType::Func(_, _, _) | ParameterType::ResultOf(_) | ParameterType::Res(_)
        ) || is_thread_type(type_)
            // plan-111-B: `type_name.contains(" STATE ")` — the BROAD question
            // (a clause anywhere in the spelling, not just this type's own), so
            // `split_state` would be wrong here. `contains_state` is that same
            // question computed structurally, pinned equal to the rendered form
            // by `contains_state_agrees_with_the_rendered_name`. It keeps the
            // opaque-nominal case a crafted `.mfp` can produce (PKG-02).
            || type_.contains_state()
        {
            return false;
        }
        // plan-172-N: an `ENUM` defaults to its FIRST declared variant. Its run-time
        // value is its ordinal, so the default is ordinal 0 — which codegen
        // (`lower_default_value_inner`'s enum arm) already materializes for the
        // temporary an inline `TRAP` binds. Ahead of the union/resource rejection
        // because an enum is neither.
        if self.enums.contains_key(type_) {
            return true;
        }
        if self.is_resource_base(type_) || self.unions.contains_key(type_) {
            return false;
        }
        if !seen.insert(type_.clone()) {
            return false;
        }
        let result = match self.record_field_lists.get(type_) {
            Some(fields) => fields.iter().all(|(_, ft)| self.is_defaultable(ft, seen)),
            // On the SOURCE path a name this table has never heard of is an
            // IMPORTED type, not an undefaultable one, and the difference is not
            // observable from here: `build` lowers with deliberately empty external
            // maps, so an importer's `record_field_lists` holds only its own types
            // and every imported record misses, whatever its spelling. Answering
            // "false" rejected legal programs (`MUT c AS pkg::Cursor`, and
            // `STATE Cursor` on an imported record — libsnd's exact shape) for
            // "having no default" when the record is very likely all-Integer
            // (bug-258).
            //
            // Same stance the RES axis takes a few hundred lines up: only a
            // POSITIVELY known type rejects, because an unknown name may be an
            // external package's. A typo cannot ride in on it — the former source checker rejects
            // an unresolvable name with `SYMBOL_UNKNOWN_TYPE` before this matters.
            //
            // The PACKAGE path keeps rejecting: there the merged IR carries the full
            // type table and every name is decoded from an id that must exist in it
            // (`decode_type_name` errors on an unknown id), so a miss is genuine
            // absence — and ir::verify is the sole rejecter for decoded `.mfp`, with
            // no the former source checker behind it.
            None => self.imported_types_unknown,
        };
        seen.remove(type_);
        result
    }

    /// Why a `STATE` clause's type is not a legal payload, or `None` when it is
    /// (bug-1013): a `STATE` type is a record `TYPE` that is copyable and
    /// defaultable, and nothing else.
    ///
    /// Native code generation treats the resource record's one `STATE` word as a
    /// pointer to an arena block the resource owns, with 0 meaning "no payload
    /// yet" (`emit_resource_state_init`, `emit_free_resource_state_block`). A
    /// scalar has no block and the default `String` is a shared constant, so
    /// those two were never payloads the back end could hold; a collection
    /// happened to work and is refused by the same rule, because the contract is
    /// "a record", not "whatever has a block today".
    ///
    /// The kind is decided from what is POSITIVELY known — the project's own
    /// declarations, the imported packages' (`record_decls`, `enums`, `unions`
    /// are all seeded from the `.mfp` type table on the source path) and the
    /// built-in registry. A nominal no table has heard of falls through to
    /// `is_defaultable`, which already carries the source/package split for it
    /// (bug-258): the resolver has rejected a typo, and a decoded `.mfp` has no
    /// unknown names.
    pub(in crate::ir::verify) fn state_type_fault(
        &self,
        state: &ParameterType,
    ) -> Option<StateTypeFault> {
        match state {
            // Unresolved, or a generic's own variable: nothing to judge here.
            // The resolver reports the first; the instantiation is judged with
            // its concrete type.
            ParameterType::Unknown | ParameterType::Var(_) | ParameterType::Arg(_) => return None,
            ParameterType::Named(_) | ParameterType::UserOf(..) => {}
            // A scalar, a collection, a `FUNC`, a `Result`, a thread handle, a
            // C ABI type, a `RES` slot or another stateful resource.
            _ => return Some(StateTypeFault::NotRecord),
        }
        // `AttributedString` is spelled as a nominal but is a built-in value
        // type, not a record (`Error` and `ErrorLoc`, the other two built-in
        // nominals, ARE records and pass through to `is_defaultable`).
        if state.is_named("AttributedString")
            || self.is_resource_or_resource_union(state)
            || self.enums.contains_key(state)
            || self.unions.contains_key(state)
        {
            return Some(StateTypeFault::NotRecord);
        }
        if !self.record_decls.contains(state) {
            use crate::codegen::registry::{registry, ResolvedType};
            match registry().resolve_type(&state.name()) {
                Some(ResolvedType::Record(_)) | None => {}
                Some(_) => return Some(StateTypeFault::NotRecord),
            }
        }
        if !self.is_defaultable(state, &mut HashSet::new()) {
            return Some(StateTypeFault::NotDefaultable);
        }
        None
    }

    /// Whether `host` — the type a `STATE` clause is written on — is positively
    /// NOT a resource (bug-998). Only a known data type answers `true`: a name
    /// no table has heard of may be an external package's resource, the stance
    /// [`provably_data_type`](Self::provably_data_type) takes for the `RES`
    /// marker. This adds the two kinds that predicate cannot see on the source
    /// path: an imported record and a built-in package's record or enum.
    pub(in crate::ir::verify) fn state_host_is_data(&self, host: &ParameterType) -> bool {
        let host = strip_res(host);
        if self.is_resource_or_resource_union(host) {
            return false;
        }
        if self.provably_data_type(host) || self.record_decls.contains(host) {
            return true;
        }
        use crate::codegen::registry::{registry, ResolvedType};
        matches!(host, ParameterType::Named(_))
            && matches!(
                registry().resolve_type(&host.name()),
                Some(ResolvedType::Record(_) | ResolvedType::Enum(_))
            )
    }

    /// Apply the two `STATE` clause rules to every clause `type_` spells, at any
    /// depth: its own top-level one, a collection element's, a record field's
    /// `RES` slot, a thread's resource plane, a thread view's.
    ///
    /// One clause reports at most one of the two, in this order:
    ///
    /// 1. `TYPE_STATE_REQUIRES_RESOURCE` (bug-998) — the clause is written on a
    ///    type that is not a resource. The clause has nothing to attach to, so
    ///    its own type is not examined.
    /// 2. `TYPE_STATE_INVALID` (bug-1013) — the host is a resource and the
    ///    `STATE` type is not a copyable, defaultable record `TYPE`.
    ///
    /// Every position that declares a type routes through here, so the rule
    /// cannot hold at a binding and lapse at an element — which is how a
    /// `List OF RES fs::File STATE Color`, a thread plane and a field's
    /// `STATE Integer` all went unchecked while the binding was refused.
    pub(in crate::ir::verify) fn check_state_clauses(&self, type_: &ParameterType, subject: &str) {
        match type_ {
            ParameterType::Stateful { base, state } => {
                if self.state_host_is_data(base) {
                    self.emit(
                        "TYPE_STATE_REQUIRES_RESOURCE",
                        format!(
                            "{subject} attaches `STATE {state}` to `{base}`, which is not a resource; remove the STATE clause."
                        ),
                    );
                } else {
                    match self.state_type_fault(state) {
                        Some(StateTypeFault::NotRecord) => self.emit(
                            "TYPE_STATE_INVALID",
                            format!(
                                "{subject} STATE type `{state}` is not a record; declare a record TYPE that holds the value and name it instead."
                            ),
                        ),
                        Some(StateTypeFault::NotDefaultable) => self.emit(
                            "TYPE_STATE_INVALID",
                            format!(
                                "{subject} STATE type `{state}` must be a copyable, defaultable record TYPE."
                            ),
                        ),
                        None => {}
                    }
                }
                self.check_state_clauses(base, subject);
            }
            ParameterType::ListOf(inner)
            | ParameterType::SetOf(inner)
            | ParameterType::ResultOf(inner)
            | ParameterType::Res(inner) => self.check_state_clauses(inner, subject),
            ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
                self.check_state_clauses(key, subject);
                self.check_state_clauses(value, subject);
            }
            ParameterType::UserOf(_, args) => {
                for arg in args {
                    self.check_state_clauses(arg, subject);
                }
            }
            ParameterType::Func(params, ret, _) => {
                for param in params {
                    self.check_state_clauses(param, subject);
                }
                self.check_state_clauses(ret, subject);
            }
            ParameterType::ThreadHandle { msg, res, out, .. } => {
                self.check_state_clauses(msg, subject);
                self.check_state_clauses(res, subject);
                self.check_state_clauses(out, subject);
            }
            _ => {}
        }
    }
}
