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

use crate::codegen::collection::assign::self_update::{FieldContainer, SelfUpdateSite};
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
    /// A by-ref capture (plan-142-G, site S9): `ref_slot` holds the address of the
    /// parent binding's slot. [`CodeBuilder::open_inplace_ref_dest`] loads the
    /// parent's block pointer into `block_slot`, the arm mutates it there as if it
    /// were a plain local, and [`CodeBuilder::close_inplace_dest`] stores the
    /// (possibly reallocated) pointer back through the reference.
    Ref { ref_slot: usize, block_slot: usize },
    /// A module-level global (plan-142-H, site S2): the block pointer lives in the
    /// global's slot. [`CodeBuilder::open_inplace_ref_dest`] loads it into
    /// `block_slot`, the arm mutates it there, and
    /// [`CodeBuilder::close_inplace_dest`] stores it back into the global.
    Global { name: String, block_slot: usize },
    /// plan-145-B: a `RES … STATE` payload field, NOT yet opened. Resolving a
    /// field site emits nothing (`O-order-1`), so the STATE pointer load waits for
    /// the arm: [`CodeBuilder::open_inplace_dest`] turns this into an
    /// [`InPlaceDest::Inlined`] with a write-back at exactly the point the STATE
    /// arm loaded it (`O-order-4`).
    StateField {
        resource: String,
        field_index: usize,
    },
}

impl InPlaceDest {
    /// The slot the lowering helper repoints on a realloc — the collection block
    /// for a plain local or a reference's working copy, the owning record block
    /// for an inlined field.
    pub(crate) fn block_slot(&self) -> usize {
        match self {
            InPlaceDest::Direct { slot } => *slot,
            InPlaceDest::Inlined { block_slot, .. }
            | InPlaceDest::Ref { block_slot, .. }
            | InPlaceDest::Global { block_slot, .. } => *block_slot,
            InPlaceDest::StateField { .. } => {
                unreachable!("an unopened STATE destination has no block slot: open it first")
            }
        }
    }

    /// plan-145-B: whether this destination is a record or `STATE` field's
    /// inlined sub-block (opened or not), which an arm serves by its field route.
    pub(crate) fn is_field(&self) -> bool {
        matches!(
            self,
            InPlaceDest::Inlined { .. } | InPlaceDest::StateField { .. }
        )
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

/// A matched self-update destination: `name = <op>(name, …)` on a uniquely-owned
/// binding, at whichever site built the [`SelfUpdateSite`].
pub(crate) struct SelfUpdateTarget<'v> {
    pub(crate) dest: InPlaceDest,
    /// The call's arguments. `args[0]` is the collection itself (already proven
    /// to be this same binding); the operands start at `args[1]`.
    pub(crate) args: &'v [NirValue],
    /// The binding's declared collection type.
    pub(crate) collection_type: ParameterType,
}

impl CodeBuilder<'_> {
    /// Resolve `name = <op>(name, …)` at a self-update site as an in-place
    /// destination, running the container gates every self-update arm shares:
    /// `G1` `by_ref`, `G2` the value is a `Call`, `G3` the call target is
    /// `builtin`, `G4` the arity, `G5` `args[0]` is a bare local, `G6` it is *this*
    /// binding, `G7` no live `FOR EACH` over it, and `G10` its type has a
    /// collection layout. (`G8`, "the local exists", is discharged by whoever built
    /// the site: the site carries the binding's type.)
    ///
    /// The operation-specific gates stay with the caller: `G9` (which collection
    /// kind this op needs), `G11` (element-vs-bulk classification) and `G12` (the
    /// self-alias). Emits nothing; `None` = decline.
    pub(crate) fn resolve_self_update<'v>(
        &self,
        site: &SelfUpdateSite<'_>,
        value: &'v NirValue,
        builtin: &str,
        arity: usize,
    ) -> Option<SelfUpdateTarget<'v>> {
        // `G2` — only a direct builtin call can be recognised.
        let NirValue::Call { target, args, .. } = value else {
            return None;
        };
        // `G3`/`G4` — every post-lowering spelling of the builtin (a `Body::Mfb`
        // member arrives as its `#collections_X$T` monomorph, plan-142-B).
        if crate::codegen::collection::assign::self_update::self_update_builtin(target)
            != Some(builtin)
            || args.len() != arity
        {
            return None;
        }
        // `G5`/`G6` — the mutated collection must be exactly the binding being
        // assigned. `name = append(other, x)` installs a fresh value and must
        // take the copying path.
        if !site.is_self(&args[0]) {
            return None;
        }
        // `G-global-operand` (plan-142-H) — a global's block is reachable from any
        // function, so nothing the statement runs may store to it: not an operand
        // (`g = append(g, f())` with `f` writing `g`) and not the call itself (a
        // callback writing `g` while the arm walks it). Either would reallocate or
        // free the block under the arm; the copying path keeps bug-496's snapshot
        // semantics instead.
        if let InPlaceDest::Global { name, .. } = &site.dest {
            let leaf = crate::codegen::engine::value::store_reach::StoreLeaf::Global(name);
            // A `Body::Mfb` member arrives as its `#collections_X$T` monomorph, whose
            // body calls the callback through a `FUNC` parameter — opaque to the
            // walk. Asked as the builtin it is, the walk follows the callback itself.
            let reach_target = match target.strip_prefix("#collections_") {
                Some(_) => format!("collections.{builtin}"),
                None => target.clone(),
            };
            if self.values_reach_store(&args[1..], leaf)
                || self.call_reaches_store(&reach_target, args, leaf)
            {
                return None;
            }
        }
        match &site.field {
            // `G1`/`G7`/`G10`. A by-ref local reached through a `Ref` destination
            // has discharged `G1`: the arm works on the parent's block, not the slot.
            None => {
                if !(InPlaceGate {
                    by_ref: site.by_ref && !matches!(site.dest, InPlaceDest::Ref { .. }),
                    for_each_local: Some(site.name),
                    layout_of: Some(&site.type_),
                    ..InPlaceGate::default()
                })
                .admits(self)
                {
                    return None;
                }
            }
            // plan-145-B: the field containers' gates, once for every arm (they
            // were `resolve_inplace_record_field` / `resolve_inplace_state_field`).
            Some(field) => {
                // `G17` — only a *last-inlined* collection field grows without
                // shifting a later sibling sub-block and the offsets stored into it.
                let (index, _) =
                    self.record_collection_last_inlined(&field.record_type, field.field)?;
                if index != field.field_index {
                    return None;
                }
                match field.container {
                    // `G1`/`G15`/`G10`.
                    FieldContainer::Record { local } => {
                        if !(InPlaceGate {
                            by_ref: site.by_ref,
                            for_each_record_field: Some((local, field.field)),
                            layout_of: Some(&site.type_),
                            ..InPlaceGate::default()
                        })
                        .admits(self)
                        {
                            return None;
                        }
                    }
                    // `G16`/`G10`. There is no `G1`: a resource handle is never a
                    // `by_ref` collection local.
                    FieldContainer::State { resource } => {
                        if !(InPlaceGate {
                            for_each_state_field: Some((resource, field.field)),
                            layout_of: Some(&site.type_),
                            ..InPlaceGate::default()
                        })
                        .admits(self)
                        {
                            return None;
                        }
                        // `G25` — an operand that can reach a `STATE` assignment
                        // would write this same block while the arm holds it
                        // (bug-487).
                        if self.inplace_state_operands_reach_a_state_assign(args) {
                            return None;
                        }
                    }
                }
            }
        }
        Some(SelfUpdateTarget {
            dest: site.dest.clone(),
            args,
            collection_type: site.type_.clone(),
        })
    }

    /// plan-145-B: open a field destination at the arm's own open point. A
    /// [`InPlaceDest::StateField`] loads the STATE pointer
    /// ([`Self::open_inplace_state_dest`]); every other destination is already
    /// open and is returned as is.
    ///
    /// Emits for a `STATE` field. Must run after every gate (`O-order-1`) and
    /// before the mutated operand is lowered (`O-order-4`).
    pub(crate) fn open_inplace_dest(&mut self, dest: &InPlaceDest) -> Result<InPlaceDest, String> {
        // plan-145-C: a mixed `WITH`'s scalar values, evaluated here — the first
        // thing the arm emits, after every gate — so they run before the arm's
        // operands and its mutation, and an arm that declines never emits them.
        if let Some(values) = self.field_pre_emit.take() {
            let mut slots = Vec::with_capacity(values.len());
            for (index, value) in &values {
                let lowered = self.lower_value(value)?;
                // Observation boundary: a `Float` field must be finite (plan-17).
                self.observe_float(value, &lowered)?;
                let lowered = self.materialize_value(lowered)?;
                let slot = self.allocate_stack_object("mixed_with_scalar", 8);
                self.emit(abi::store_u64(
                    &lowered.location,
                    abi::stack_pointer(),
                    slot,
                ));
                slots.push((*index, slot));
            }
            self.field_pre_emitted = Some(slots);
        }
        match dest {
            InPlaceDest::StateField {
                resource,
                field_index,
            } => self.open_inplace_state_dest(resource, *field_index),
            _ => Ok(dest.clone()),
        }
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

    /// Materialize the **address of the inlined collection sub-block** into a
    /// fresh stack slot, so a lowering written against a plain local's
    /// `buffer_slot`/`map_slot` can mutate a record or `STATE` field's collection
    /// where it lies — no second lowering, no copy.
    ///
    /// The record block stores each inlined field as a **block-relative offset**
    /// at `record_ptr + 8 * field_index`; the sub-block therefore begins at
    /// `record_ptr + fieldOffset`. That is the same arithmetic
    /// [`Self::lower_inline_list_append_in_place`] does inline, named once here.
    ///
    /// # Only sound for a mutation that cannot reallocate
    ///
    /// The returned slot holds an **address inside the record block**, not a
    /// collection pointer the runtime owns. A lowering that reallocates writes the
    /// new block pointer back into the slot it was given — which here would
    /// scribble a fresh heap pointer into a scratch slot and silently lose it,
    /// leaving the record's field offset pointing at the old, freed bytes.
    ///
    /// So this is for the **shrinking and same-size** operations only —
    /// `removeAt`, `removeKey`, Set `remove`, and a `set` whose replacement is the
    /// same width. Each of those was checked to only ever *load* its slot (for
    /// example every `map_slot` access in `lower_map_remove_key_in_place` is an
    /// `abi::load_u64`). A growing operation — `append`, `insert`, `prepend`,
    /// `add` — must instead go through the inline grow path
    /// ([`Self::lower_inplace_inlined_list_grow`]), which grows the **record**
    /// block and repoints `block_slot`.
    ///
    /// A `Direct` destination is a caller error rather than a decline, for the
    /// same reason [`Self::lower_inplace_inlined_list_grow`] treats it as one: by
    /// the time a destination exists the container has been matched, so asking a
    /// plain local for its inlined sub-block is a bug in the arm.
    /// Read the inlined field's **block-relative offset** into a fresh stack slot,
    /// once, before any grow.
    ///
    /// The record block stores each inlined field's offset at
    /// `record_ptr + 8 * field_index`. That offset is **invariant across a
    /// realloc** — the prefix `[0, fieldOffset)` is copied verbatim, so the field
    /// still begins at the same distance into the new block — which is exactly why
    /// it can be read before the grow and used after it. The sub-block *address*
    /// cannot: it moves.
    ///
    /// Growing lowerings need this because they must size the new record as
    /// `fieldOffset + <new collection size>` and then find the sub-block inside it,
    /// and they must do so without re-reading a record pointer that the
    /// allocation is about to invalidate.
    pub(crate) fn open_inplace_inlined_field_offset(
        &mut self,
        dest: &InPlaceDest,
    ) -> Result<usize, String> {
        let InPlaceDest::Inlined {
            block_slot,
            field_index,
            ..
        } = dest
        else {
            return Err(
                "native in-place inlined field offset needs an inlined-field destination, \
                 got a plain local slot"
                    .to_string(),
            );
        };
        let base = self.allocate_register();
        let offset = self.allocate_register();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), *block_slot));
        self.emit(abi::load_u64(&offset, &base, 8 * *field_index));
        let slot = self.allocate_stack_object("inplace_inlined_field_off", 8);
        self.emit(abi::store_u64(&offset, abi::stack_pointer(), slot));
        Ok(slot)
    }

    pub(crate) fn open_inplace_inlined_subblock(
        &mut self,
        dest: &InPlaceDest,
    ) -> Result<usize, String> {
        let InPlaceDest::Inlined {
            block_slot,
            field_index,
            ..
        } = dest
        else {
            return Err(
                "native in-place inlined sub-block needs an inlined-field destination, \
                 got a plain local slot"
                    .to_string(),
            );
        };
        let base = self.allocate_register();
        let offset = self.allocate_register();
        // record pointer, then the field's block-relative offset beside it.
        self.emit(abi::load_u64(&base, abi::stack_pointer(), *block_slot));
        self.emit(abi::load_u64(&offset, &base, 8 * *field_index));
        self.emit(abi::add_registers(&base, &base, &offset));
        let slot = self.allocate_stack_object("inplace_inlined_subblock", 8);
        self.emit(abi::store_u64(&base, abi::stack_pointer(), slot));
        Ok(slot)
    }

    /// Open a [`InPlaceDest::Ref`] or [`InPlaceDest::Global`]: copy the block
    /// pointer — read through the reference in `ref_slot`, or out of the global —
    /// into the working `block_slot`.
    pub(crate) fn open_inplace_ref_dest(&mut self, dest: &InPlaceDest) -> Result<(), String> {
        let (holder, block_slot) = match dest {
            InPlaceDest::Ref {
                ref_slot,
                block_slot,
            } => {
                let parent = self.allocate_register();
                self.emit(abi::load_u64(&parent, abi::stack_pointer(), *ref_slot));
                (parent.render(), *block_slot)
            }
            InPlaceDest::Global { name, block_slot } => {
                (self.load_global_address(name)?, *block_slot)
            }
            InPlaceDest::Direct { .. }
            | InPlaceDest::Inlined { .. }
            | InPlaceDest::StateField { .. } => return Ok(()),
        };
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, holder.as_str(), 0));
        self.emit(abi::store_u64(&block, abi::stack_pointer(), block_slot));
        Ok(())
    }

    /// Discharge obligation `O4`: publish a reallocated `STATE` block pointer
    /// through the resource's shared STATE slot, so the owner and every alias
    /// observe the grown block (§15); for a [`InPlaceDest::Ref`], store the
    /// working block pointer back into the parent binding's slot. A no-op for a
    /// plain local or a record field, neither of which has a second holder.
    pub(crate) fn close_inplace_dest(&mut self, dest: &InPlaceDest) -> Result<(), String> {
        if let InPlaceDest::Ref {
            ref_slot,
            block_slot,
        } = dest
        {
            let block = self.allocate_register();
            self.emit(abi::load_u64(&block, abi::stack_pointer(), *block_slot));
            let parent = self.allocate_register();
            self.emit(abi::load_u64(&parent, abi::stack_pointer(), *ref_slot));
            self.emit(abi::store_u64(&block, &parent, 0));
            return Ok(());
        }
        if let InPlaceDest::Global { name, block_slot } = dest {
            // The arm's `arena_*` calls clobber the caller-saved registers, so the
            // global's address is derived here, after it (as `StoreGlobal` does).
            let block = self.allocate_register();
            self.emit(abi::load_u64(&block, abi::stack_pointer(), *block_slot));
            let address = self.load_global_address(name)?;
            self.emit(abi::store_u64(&block, address.as_str(), 0));
            return Ok(());
        }
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

// ---------------------------------------------------------------------------
// `G25` — an operand that can run user code which assigns a `STATE` field
// (bug-487).
// ---------------------------------------------------------------------------

impl CodeBuilder<'_> {
    /// `G25` — whether any of an in-place `RES … STATE` update's operands can run
    /// user code that performs a `STATE` assignment.
    ///
    /// `mfb spec language resource-management` §15 defines the statement these
    /// arms recognise as a shorthand: *"It is updated either by assigning a
    /// single field in place (`s.state.field = value`) or by assigning a
    /// whole-state `WITH` update (`s.state = WITH s.state { field := value }`);
    /// the former is shorthand for the latter."* The `WITH` form **reads**
    /// `s.state` and stores a record built from that read, and `mfb spec language
    /// memory-semantics` §14.6 says *"Reads produce owned values, not aliases
    /// into the buffer"* — so a `STATE` write performed while the operands are
    /// being evaluated is (correctly) overwritten by the store and cannot be
    /// observed in the result.
    ///
    /// An in-place arm mutates the live block instead, which §14 permits — *"The
    /// compiler may choose stack storage, inline storage, heap allocation, or
    /// destructive update, but those choices cannot change the ownership behavior
    /// described here"* — only while nothing else writes that block in between. A
    /// `RES` is an alias to one live resource (§15), so an operand that reaches a
    /// `STATE` assignment is exactly the case where something does: the nested
    /// write would survive into the result, which the shorthand's `WITH` form
    /// does not produce, and for a *growing* arm that nested write **reallocates
    /// and frees** the very block the arm snapshotted before lowering the
    /// operand — so the mutation and the `O4` write-back both run on freed memory
    /// (bug-487: `7-701-0001`, or SIGSEGV).
    ///
    /// Declining is always correct: the caller falls through to the whole-record
    /// STATE replace, whose operand snapshot (bug-496,
    /// `engine/value/operand_snapshot.rs`) already makes that path exact.
    ///
    /// The question is deliberately "can it reach a `STATE` assignment", not
    /// "does it call user code at all". `f.state.xs = append(f.state.xs,
    /// clamp(v))` calls a user function that assigns no `STATE`, so it keeps the
    /// fast path — that is the 20 000× cliff
    /// `tests/codegen_inplace_append_call_result.rs` measures, and
    /// `tests/rt_res_state_inplace_mutation.rs` pins both halves.
    ///
    /// The call-graph walk, and what it assumes about calls it cannot see, is
    /// shared with the global-store guards of bug-665/666
    /// (`engine/value/store_reach.rs`).
    ///
    /// Emits nothing and allocates nothing, so it is a gate in the `O-order-1`
    /// sense and runs before the first `lower_value`.
    pub(crate) fn inplace_state_operands_reach_a_state_assign(
        &self,
        operands: &[NirValue],
    ) -> bool {
        self.values_reach_store(
            operands,
            crate::codegen::engine::value::store_reach::StoreLeaf::StateAssign,
        )
    }
}
