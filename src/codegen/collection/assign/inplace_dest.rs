//! plan-121-A: the shared destination-slot seam for in-place collection updates.
//!
//! Every `try_inplace_*` arm answers the same two questions before it lowers
//! anything: **where does this collection live, and may I mutate it in place?**
//! Before this module each arm re-derived both, so the family grew ten
//! near-duplicate guard sets — and a gate could be present in nine of them and
//! forgotten in the tenth. (It was: writing the inventory found exactly that,
//! see `planning/plan-121-gate-inventory.md` §"DEFECT FOUND".) This module
//! states the answer once.
//!
//! Three layers, matching `planning/plan-121-gate-inventory.md`:
//!
//! * [`InPlaceGate`] — the proof obligations, named after the inventory's `G*`
//!   codes. An arm names the subset that applies to its shape and
//!   [`InPlaceGate::admits`] runs them.
//! * [`InPlaceDest`] — where the mutation writes, and what must happen after it.
//! * The `resolve_*` / `open_*` / `close_*` helpers below, which run the gates
//!   and hand back a destination.
//!
//! Declining is **always correct**: the caller falls through to the general
//! copying reassignment, which has exact value semantics. In-place is an
//! implementation strategy the program cannot observe.
//!
//! ## Byte-identity
//!
//! This seam is pure code motion. It allocates no register, emits no
//! instruction and reserves no stack slot, except in
//! [`CodeBuilder::open_inplace_state_dest`] / [`CodeBuilder::close_inplace_dest`],
//! which emit exactly the STATE-pointer load and write-back the STATE arm
//! emitted inline. Register and stack-slot allocation *order* is observable in
//! the emitted bytes (`.ai/codegen-invariants.md`), so nothing here may allocate
//! speculatively: **every gate runs before the first `lower_value`.** That is
//! inventory rule `O-order-1`, and it is what makes this refactor provably
//! neutral — `.ncode`/`.ncodesum` must be byte-identical across it.

use crate::codegen::engine::builder::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

/// How the destination collection is reached, and what must happen after a
/// (possibly reallocating) mutation.
///
/// The distinction is not cosmetic. A plain local's frame slot holds the
/// collection block pointer itself, so a realloc simply repoints that slot. A
/// record or `STATE` field is *inlined* — the collection lives inside the owning
/// record's block at a field offset — so a realloc grows the **record** block
/// and repoints the pointer to that, with the field index selecting the
/// sub-block. The lowering helpers differ accordingly
/// (`lower_list_append_in_place` vs `lower_inline_list_append_in_place`).
#[derive(Debug, Clone)]
pub(crate) enum InPlaceDest {
    /// The collection block pointer lives directly at `slot`; the lowering
    /// helper repoints `slot` on a realloc and nothing else observes it.
    Direct { slot: usize },
    /// The collection is inlined at `field_index` of a record block whose
    /// pointer lives at `block_slot`; the lowering helper repoints `block_slot`.
    Inlined {
        block_slot: usize,
        field_index: usize,
        /// `RES … STATE` only: that block pointer is *shared* with the resource
        /// record, so after the mutation the (possibly new) pointer must be
        /// published back through the resource's STATE slot (§15). A plain
        /// record local has no second holder and carries `None`.
        write_back: Option<StateWriteBack>,
    },
}

impl InPlaceDest {
    /// The slot the lowering helper repoints on a realloc — the collection block
    /// for a plain local, the owning record block for an inlined field.
    pub(crate) fn block_slot(&self) -> usize {
        match self {
            InPlaceDest::Direct { slot } => *slot,
            InPlaceDest::Inlined { block_slot, .. } => *block_slot,
        }
    }
}

/// Where to publish a reallocated `STATE` block pointer so the resource owner
/// and every alias observe the grown block. Inventory obligation `O4`.
#[derive(Debug, Clone)]
pub(crate) struct StateWriteBack {
    /// Frame slot holding the resource handle (the *resource* block pointer).
    pub(crate) resource_slot: usize,
    /// The resource's declared type, needed to find its record pointer.
    pub(crate) resource_type: ParameterType,
}

/// The proof obligations an in-place mutation must discharge, named after the
/// `G*` codes in `planning/plan-121-gate-inventory.md`.
///
/// An arm builds one describing *its* shape and calls [`Self::admits`].
/// Conditions that cannot apply to a shape are left unset — but the ones that do
/// apply are enforced identically for every container, which is the point: a
/// gate present in any arm is present here, so it cannot be forgotten in one.
#[derive(Debug, Default, Clone)]
pub(crate) struct InPlaceGate<'a> {
    /// `G1` — a `by_ref` local's slot holds a pointer to the *parent* slot, not
    /// to the buffer, so an in-place write would corrupt the caller's binding.
    pub(crate) by_ref: bool,
    /// `G7` — a live `FOR EACH` over this plain local. The loop snapshots the
    /// buffer pointer and count at entry, so a realloc frees that buffer under
    /// the iterator (bug-142) and an entry *shift* is observable even without
    /// one.
    pub(crate) for_each_local: Option<&'a str>,
    /// `G15` — a live `FOR EACH` over this `(record local, field)`.
    pub(crate) for_each_record_field: Option<(&'a str, &'a str)>,
    /// `G16` — a live `FOR EACH` over this `(resource, state field)`.
    pub(crate) for_each_state_field: Option<(&'a str, &'a str)>,
    /// `G10` — the collection type must have a `CollectionTypeLayout`; without
    /// one there is no header to update in place.
    pub(crate) layout_of: Option<&'a ParameterType>,
}

/// The live-iterator state a gate consults: which bindings a `FOR EACH` is
/// currently walking. A borrowed view rather than the whole `CodeBuilder`, so
/// [`InPlaceGate::admits_with`] is a pure function of its inputs and can be
/// exercised directly by a unit test — a gate whose only entry point needs a
/// fully-built `CodeBuilder` is a gate that never gets tested in isolation, and
/// this one carries the aliasing proofs for three containers at once.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LiveIterables<'a> {
    pub(crate) locals: &'a [String],
    pub(crate) record_fields: &'a [(String, String)],
    pub(crate) state_fields: &'a [(String, String)],
}

impl InPlaceGate<'_> {
    /// Run every set condition against the builder's live-iterator state.
    /// `false` = decline, and the caller falls through to the copying
    /// reassignment path.
    ///
    /// Emits nothing and allocates nothing, so a decline is free and the gate
    /// may run before any value is lowered (inventory rule `O-order-1`).
    pub(crate) fn admits(&self, builder: &CodeBuilder<'_>) -> bool {
        self.admits_with(LiveIterables {
            locals: &builder.for_each_iterable_locals,
            record_fields: &builder.for_each_iterable_record_fields,
            state_fields: &builder.for_each_iterable_state_fields,
        })
    }

    /// The gate itself: a pure predicate over the conditions and the live
    /// iterables. Each branch names the `G*` code it enforces.
    pub(crate) fn admits_with(&self, live: LiveIterables<'_>) -> bool {
        // `G1` — a `by_ref` slot holds a pointer to the parent slot.
        if self.by_ref {
            return false;
        }
        // `G7` — a live `FOR EACH` over this plain local.
        if let Some(name) = self.for_each_local {
            if live.locals.iter().any(|n| n == name) {
                return false;
            }
        }
        // `G15` — a live `FOR EACH` over this record field.
        if let Some((base, field)) = self.for_each_record_field {
            if live
                .record_fields
                .iter()
                .any(|(b, f)| b == base && f == field)
            {
                return false;
            }
        }
        // `G16` — a live `FOR EACH` over this state field.
        if let Some((res, field)) = self.for_each_state_field {
            if live
                .state_fields
                .iter()
                .any(|(r, f)| r == res && f == field)
            {
                return false;
            }
        }
        // `G10` — no collection layout, no header to update in place.
        if let Some(type_) = self.layout_of {
            if CollectionTypeLayout::from_type(type_).is_none() {
                return false;
            }
        }
        true
    }
}

/// A matched plain-local in-place destination: `name = <op>(name, …)` on a
/// uniquely-owned `MUT` local.
pub(crate) struct PlainLocalTarget<'v> {
    pub(crate) dest: InPlaceDest,
    /// The call's arguments. `args[0]` is the collection itself (already proven
    /// to be this same binding); the operands start at `args[1]`.
    pub(crate) args: &'v [NirValue],
    /// The local's declared collection type.
    pub(crate) collection_type: ParameterType,
}

/// A matched inlined-field in-place destination: the record/`STATE` field being
/// self-updated, and the call that updates it.
pub(crate) struct InlinedFieldTarget<'v> {
    pub(crate) field_index: usize,
    pub(crate) field_type: ParameterType,
    /// The updated field's name — needed for the `G18`/`G12` field-identity
    /// checks the caller still owns.
    pub(crate) field: &'v str,
    pub(crate) args: &'v [NirValue],
}

impl CodeBuilder<'_> {
    /// Resolve `name = <op>(name, …)` on a uniquely-owned plain `MUT` local as an
    /// in-place destination, running the container gates every plain-local arm
    /// shares: `G1` `by_ref`, `G2` the value is a `Call`, `G3` the call target is
    /// `builtin`, `G4` the arity, `G5` `args[0]` is a bare local, `G6` it is *this*
    /// binding, `G7` no live `FOR EACH` over it, `G8` the local exists, and `G10`
    /// its type has a collection layout.
    ///
    /// The operation-specific gates stay with the caller: `G9` (which collection
    /// kind this op needs), `G11` (element-vs-bulk classification) and `G12` (the
    /// self-alias). Emits nothing; `None` = decline.
    pub(crate) fn resolve_inplace_plain_local<'v>(
        &self,
        name: &str,
        value: &'v NirValue,
        stack_offset: usize,
        by_ref: bool,
        builtin: &str,
        arity: usize,
    ) -> Option<PlainLocalTarget<'v>> {
        // `G2` — only a direct builtin call can be recognised.
        let NirValue::Call { target, args, .. } = value else {
            return None;
        };
        // `G3`/`G4`.
        if crate::codegen::builtins::native_builtin_target(target) != Some(builtin)
            || args.len() != arity
        {
            return None;
        }
        // `G5`/`G6` — the mutated collection must be exactly the binding being
        // assigned. `name = append(other, x)` installs a fresh value and must
        // take the copying path.
        let NirValue::Local(arg0) = &args[0] else {
            return None;
        };
        if arg0 != name {
            return None;
        }
        // `G8`.
        let local = self.locals.get(name)?;
        let collection_type = local.type_.clone();
        // `G1`/`G7`/`G10`.
        if !(InPlaceGate {
            by_ref,
            for_each_local: Some(name),
            layout_of: Some(&collection_type),
            ..InPlaceGate::default()
        })
        .admits(self)
        {
            return None;
        }
        Some(PlainLocalTarget {
            dest: InPlaceDest::Direct { slot: stack_offset },
            args,
            collection_type,
        })
    }

    /// Resolve `local = WITH local { field := <op>(local.field, …) }` — the
    /// record-field container — running `G1`, `G2` (`WithUpdate` then `Call`),
    /// `G13` (self-update of this same local), `G14` (exactly one updated field),
    /// `G15` (no live `FOR EACH` over this field), `G17` (the field is the
    /// record's last-inlined `List`), `G10`, `G3` and `G4`.
    ///
    /// `G18` (the appended-to source is this same field), `G11` and `G12` stay
    /// with the caller, which knows the operation. Emits nothing.
    pub(crate) fn resolve_inplace_record_field<'v>(
        &self,
        name: &str,
        value: &'v NirValue,
        by_ref: bool,
        builtin: &str,
        arity: usize,
    ) -> Option<InlinedFieldTarget<'v>> {
        let NirValue::WithUpdate {
            type_,
            target,
            updates,
        } = value
        else {
            return None;
        };
        // `G13` — the update must rebuild THIS same local, not install some
        // other record as the new value.
        if !matches!(target.as_ref(), NirValue::Local(n) if n == name) {
            return None;
        }
        // `G14` — a second updated field means the whole-record rebuild is not
        // redundant, so eliding it would drop that field's new value.
        if updates.len() != 1 {
            return None;
        }
        let update = &updates[0];
        // `G17` — only a *last-inlined* `List` field grows without shifting a
        // sibling sub-block and the offsets stored into it.
        let (field_index, field_type) =
            self.record_collection_last_inlined(type_, &update.field)?;
        // `G1`/`G15`/`G10`.
        if !(InPlaceGate {
            by_ref,
            for_each_record_field: Some((name, update.field.as_str())),
            layout_of: Some(&field_type),
            ..InPlaceGate::default()
        })
        .admits(self)
        {
            return None;
        }
        let args = self.inplace_call_args(&update.value, builtin, arity)?;
        Some(InlinedFieldTarget {
            field_index,
            field_type,
            field: update.field.as_str(),
            args,
        })
    }

    /// Resolve `resource.state.field = <op>(resource.state.field, …)` — the
    /// `RES … STATE` container. `src/ast/stmt.rs` desugars that statement to a
    /// single-field `WITH` update over `resource.state`, so this matches the same
    /// `WithUpdate` shape as the record container with `G13` reading the
    /// resource's `.state` instead of the local itself, and `G16` replacing
    /// `G15`.
    ///
    /// Emits nothing — the STATE pointer load is
    /// [`Self::open_inplace_state_dest`], which must run *after* this and
    /// *before* the operand is lowered (inventory rule `O-order-4`).
    pub(crate) fn resolve_inplace_state_field<'v>(
        &self,
        resource: &str,
        value: &'v NirValue,
        builtin: &str,
        arity: usize,
    ) -> Option<InlinedFieldTarget<'v>> {
        let NirValue::WithUpdate {
            type_,
            target,
            updates,
        } = value
        else {
            return None;
        };
        // `G13` — the target must be exactly this resource's `.state`.
        let NirValue::MemberAccess {
            target: inner,
            member,
        } = target.as_ref()
        else {
            return None;
        };
        if member != "state" || !matches!(inner.as_ref(), NirValue::Local(n) if n == resource) {
            return None;
        }
        if updates.len() != 1 {
            return None;
        }
        let update = &updates[0];
        // `G16`/`G17`/`G10`. There is no `G1`: a resource handle is never a
        // `by_ref` collection local, and the STATE arms are dispatched off
        // `NirOp::StateAssign`, which carries no `by_ref` flag.
        if !(InPlaceGate {
            for_each_state_field: Some((resource, update.field.as_str())),
            ..InPlaceGate::default()
        })
        .admits(self)
        {
            return None;
        }
        let (field_index, field_type) =
            self.record_collection_last_inlined(type_, &update.field)?;
        if CollectionTypeLayout::from_type(&field_type).is_none() {
            return None;
        }
        let args = self.inplace_call_args(&update.value, builtin, arity)?;
        Some(InlinedFieldTarget {
            field_index,
            field_type,
            field: update.field.as_str(),
            args,
        })
    }

    /// `G2`/`G3`/`G4` for the value inside a `WITH` update: it must be a direct
    /// call of the named builtin with the expected arity.
    fn inplace_call_args<'v>(
        &self,
        value: &'v NirValue,
        builtin: &str,
        arity: usize,
    ) -> Option<&'v [NirValue]> {
        let NirValue::Call { target, args, .. } = value else {
            return None;
        };
        if crate::codegen::builtins::native_builtin_target(target) != Some(builtin)
            || args.len() != arity
        {
            return None;
        }
        Some(args)
    }

    /// Materialize a `RES … STATE` destination: load the shared STATE record
    /// pointer out of the resource record into a fresh slot the grow helper may
    /// repoint.
    ///
    /// Emits. Must run *after* every gate has passed (`O-order-1`) and *before*
    /// the mutated operand is lowered (`O-order-4` — the operand's own lowering
    /// must not observe a stale STATE pointer).
    pub(crate) fn open_inplace_state_dest(
        &mut self,
        resource: &str,
        field_index: usize,
    ) -> Result<InPlaceDest, String> {
        let local = self
            .locals
            .get(resource)
            .ok_or_else(|| format!("native code state assignment unknown local '{resource}'"))?;
        let resource_slot = local.stack_offset;
        let resource_type = local.type_.clone();
        let block_slot = self.allocate_stack_object("inline_state_ptr", 8);
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), resource_slot));
        let record = self.emit_resource_record_ptr(&block, &resource_type)?;
        let state_ptr = self.allocate_register();
        self.emit(abi::load_u64(&state_ptr, &record, RESOURCE_OFFSET_STATE));
        self.emit(abi::store_u64(&state_ptr, abi::stack_pointer(), block_slot));
        Ok(InPlaceDest::Inlined {
            block_slot,
            field_index,
            write_back: Some(StateWriteBack {
                resource_slot,
                resource_type,
            }),
        })
    }

    /// Grow the inlined `List` this destination names, by one element (`bulk =
    /// false`) or by a whole list (`bulk = true`), with the appended value already
    /// spilled at `rhs_slot`.
    ///
    /// This is where the destination stops being a description and becomes the
    /// call: the record and `STATE` arms differ in how their `block_slot` was
    /// obtained, not in what happens to it, so they share this. Reading
    /// `field_index` back out of the destination (rather than each arm passing its
    /// own copy alongside) is what keeps the destination the single answer to
    /// "where does this write go".
    ///
    /// A `Direct` destination has no inlined field, so it is a caller error rather
    /// than a decline: by the time a destination exists the container has already
    /// been matched, and matching a plain local and then asking for an inlined grow
    /// is a bug in the arm, not a program the compiler should fall back on.
    pub(crate) fn lower_inplace_inlined_list_grow(
        &mut self,
        dest: &InPlaceDest,
        bulk: bool,
        field_type: &ParameterType,
        element_type: &ParameterType,
        rhs_slot: usize,
    ) -> Result<(), String> {
        let InPlaceDest::Inlined {
            block_slot,
            field_index,
            ..
        } = dest
        else {
            return Err(format!(
                "native in-place inlined grow of {field_type} needs an inlined-field \
                 destination, got a plain local slot"
            ));
        };
        if bulk {
            self.lower_inline_list_bulk_append_in_place(
                *block_slot,
                *field_index,
                field_type,
                element_type,
                rhs_slot,
            )
        } else {
            self.lower_inline_list_append_in_place(
                *block_slot,
                *field_index,
                field_type,
                element_type,
                rhs_slot,
            )
        }
    }

    /// Discharge obligation `O4`: publish a reallocated `STATE` block pointer
    /// through the resource's shared STATE slot, so the owner and every alias
    /// observe the grown block (§15). A no-op for a plain local or a record
    /// field, neither of which has a second holder.
    pub(crate) fn close_inplace_dest(&mut self, dest: &InPlaceDest) -> Result<(), String> {
        let InPlaceDest::Inlined {
            block_slot,
            write_back: Some(write_back),
            ..
        } = dest
        else {
            return Ok(());
        };
        let nb = self.allocate_register();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), *block_slot));
        let block2 = self.allocate_register();
        self.emit(abi::load_u64(
            &block2,
            abi::stack_pointer(),
            write_back.resource_slot,
        ));
        let record2 = self.emit_resource_record_ptr(&block2, &write_back.resource_type)?;
        self.emit(abi::store_u64(&nb, &record2, RESOURCE_OFFSET_STATE));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! plan-121-A Phase 3 — the gate's decline conditions, exercised directly.
    //!
    //! [`InPlaceGate`] carries the aliasing proofs for all three containers at
    //! once, so a condition that silently stops firing un-protects three arms
    //! simultaneously and the symptom is a use-after-free, not a test failure.
    //! The codegen-inspection tests elsewhere prove a fast path is *taken*;
    //! these prove it is *refused*, which no black-box fixture can see — a
    //! missed decline miscompiles, and a spurious decline only gets slow.
    //!
    //! One case per inventory condition in
    //! `planning/plan-121-gate-inventory.md` that [`InPlaceGate`] owns:
    //! `G1`, `G7`, `G10`, `G15`, `G16`.

    use super::*;

    fn list_of_integer() -> ParameterType {
        ParameterType::parse("List OF Integer")
    }

    /// No condition triggers: the gate admits. The control for every case below
    /// — without it a gate that *always* declined would pass them all.
    #[test]
    fn an_unaliased_uniquely_owned_local_is_admitted() {
        let list = list_of_integer();
        let gate = InPlaceGate {
            for_each_local: Some("out"),
            layout_of: Some(&list),
            ..InPlaceGate::default()
        };
        assert!(gate.admits_with(LiveIterables {
            locals: &[],
            record_fields: &[],
            state_fields: &[],
        }));
    }

    /// `G1` — a `by_ref` local's frame slot holds a pointer to the *caller's*
    /// slot, not to the buffer, so an in-place write would corrupt the caller's
    /// binding rather than this one.
    #[test]
    fn g1_a_by_ref_local_is_declined() {
        let list = list_of_integer();
        let gate = InPlaceGate {
            by_ref: true,
            for_each_local: Some("out"),
            layout_of: Some(&list),
            ..InPlaceGate::default()
        };
        assert!(!gate.admits_with(LiveIterables {
            locals: &[],
            record_fields: &[],
            state_fields: &[],
        }));
    }

    /// `G7` — a live `FOR EACH` over this plain local. The loop snapshots the
    /// buffer pointer and count at entry; a grow frees that buffer under the
    /// iterator (bug-142).
    #[test]
    fn g7_a_live_for_each_over_this_local_is_declined() {
        let list = list_of_integer();
        let gate = InPlaceGate {
            for_each_local: Some("out"),
            layout_of: Some(&list),
            ..InPlaceGate::default()
        };
        assert!(!gate.admits_with(LiveIterables {
            locals: &["out".to_string()],
            record_fields: &[],
            state_fields: &[],
        }));
    }

    /// `G7` is name-scoped: iterating a *different* local is not a hazard, so
    /// the gate must not decline on it. Pins that the check is an identity
    /// test and not "any loop is running".
    #[test]
    fn g7_a_live_for_each_over_another_local_is_admitted() {
        let list = list_of_integer();
        let gate = InPlaceGate {
            for_each_local: Some("out"),
            layout_of: Some(&list),
            ..InPlaceGate::default()
        };
        assert!(gate.admits_with(LiveIterables {
            locals: &["other".to_string()],
            record_fields: &[],
            state_fields: &[],
        }));
    }

    /// `G15` — a live `FOR EACH` over exactly this `(record local, field)`.
    #[test]
    fn g15_a_live_for_each_over_this_record_field_is_declined() {
        let list = list_of_integer();
        let gate = InPlaceGate {
            for_each_record_field: Some(("rec", "xs")),
            layout_of: Some(&list),
            ..InPlaceGate::default()
        };
        assert!(!gate.admits_with(LiveIterables {
            locals: &[],
            record_fields: &[("rec".to_string(), "xs".to_string())],
            state_fields: &[],
        }));
    }

    /// `G15` matches on the *pair*. Iterating a sibling field of the same record
    /// aliases a different sub-block, so it is not a hazard — a gate that
    /// compared only the base name would over-decline every record.
    #[test]
    fn g15_a_live_for_each_over_a_sibling_field_is_admitted() {
        let list = list_of_integer();
        let gate = InPlaceGate {
            for_each_record_field: Some(("rec", "xs")),
            layout_of: Some(&list),
            ..InPlaceGate::default()
        };
        assert!(gate.admits_with(LiveIterables {
            locals: &[],
            record_fields: &[("rec".to_string(), "ys".to_string())],
            state_fields: &[],
        }));
    }

    /// `G16` — a live `FOR EACH` over exactly this `(resource, state field)`.
    #[test]
    fn g16_a_live_for_each_over_this_state_field_is_declined() {
        let list = list_of_integer();
        let gate = InPlaceGate {
            for_each_state_field: Some(("f", "raw")),
            layout_of: Some(&list),
            ..InPlaceGate::default()
        };
        assert!(!gate.admits_with(LiveIterables {
            locals: &[],
            record_fields: &[],
            state_fields: &[("f".to_string(), "raw".to_string())],
        }));
    }

    /// `G16` matches on the pair, like `G15`: another resource's identically
    /// named state field is a different block.
    #[test]
    fn g16_a_live_for_each_over_another_resources_state_field_is_admitted() {
        let list = list_of_integer();
        let gate = InPlaceGate {
            for_each_state_field: Some(("f", "raw")),
            layout_of: Some(&list),
            ..InPlaceGate::default()
        };
        assert!(gate.admits_with(LiveIterables {
            locals: &[],
            record_fields: &[],
            state_fields: &[("g".to_string(), "raw".to_string())],
        }));
    }

    /// `G10` — without a `CollectionTypeLayout` there is no header to update in
    /// place, so the mutation has no in-place form at all.
    #[test]
    fn g10_a_type_with_no_collection_layout_is_declined() {
        let scalar = ParameterType::Integer;
        assert!(
            CollectionTypeLayout::from_type(&scalar).is_none(),
            "precondition: a scalar has no collection layout, so this case \
             really exercises G10 rather than passing vacuously"
        );
        let gate = InPlaceGate {
            layout_of: Some(&scalar),
            ..InPlaceGate::default()
        };
        assert!(!gate.admits_with(LiveIterables {
            locals: &[],
            record_fields: &[],
            state_fields: &[],
        }));
    }

    /// The conditions are independent: each is checked on its own, so a gate
    /// that only ever looked at the first `Some` field would pass every case
    /// above and fail here.
    #[test]
    fn every_condition_is_checked_independently() {
        let list = list_of_integer();
        let live = LiveIterables {
            locals: &[],
            record_fields: &[("rec".to_string(), "xs".to_string())],
            state_fields: &[],
        };
        let gate = InPlaceGate {
            for_each_local: Some("out"),
            for_each_record_field: Some(("rec", "xs")),
            layout_of: Some(&list),
            ..InPlaceGate::default()
        };
        assert!(
            !gate.admits_with(live),
            "the record-field hazard must decline even though the plain-local \
             condition ahead of it is clear"
        );
    }
}
