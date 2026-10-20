//! plan-176-Q: record field-layout classification — the type predicates and their `CodeBuilder`
//! wrappers. The list-layout helpers live in
//! `collection/layout/builder_collection_layout/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// True when `field_type` occupies a record slot as a pointer to a separate
    /// allocation (nested record/union/collection/`Result`/`Error`). These stay
    /// pointers in Phase 2 (later phases inline them).
    pub(crate) fn record_field_is_pointer(&self, field_type: &ParameterType) -> bool {
        record_field_is_pointer(&self.type_model, field_type)
    }

    /// True when a `memcpy` of this value's block is a correct **copy within one
    /// thread**. Copyable types: scalars, `String`, a record whose every field is
    /// copyable, a **data** union whose every variant is copyable, a collection
    /// whose payloads are copyable, and a **resource handle** — the 8-byte slot
    /// is a pointer to the one resource record, and copying it aliases that
    /// resource rather than duplicating it (§15.6), which is the same rule
    /// [`Self::is_pointer_collection_payload_type`] already applies to a
    /// collection slot. Not copyable: resource *unions* (a `{tag, ptr}` block,
    /// not a plain slot), `Result` of a non-copyable payload, and any recursive
    /// type (broken by the `visited` path set, so a cyclic type stays a pointer).
    pub(crate) fn type_is_memcpy_copyable(&self, type_: &ParameterType) -> bool {
        type_is_memcpy_copyable(&self.type_model, type_)
    }

    /// True when this value's block may be **relocated into another thread's
    /// arena**. Strictly stronger than [`Self::type_is_memcpy_copyable`]: arenas
    /// are per-thread, so a resource handle anywhere inside the block would
    /// arrive pointing into the *sender's* arena. A resource — bare, `RES`-marked,
    /// or nested in a field or payload — makes this false; everything else
    /// answers exactly as memcpy-copyability does.
    pub(crate) fn type_is_arena_transferable(&self, type_: &ParameterType) -> bool {
        type_is_arena_transferable(&self.type_model, type_)
    }

    /// See [`is_resource_nominal`].
    pub(crate) fn is_resource_nominal(&self, type_: &ParameterType) -> bool {
        is_resource_nominal(&self.type_model, type_)
    }

    /// See [`is_sendable_resource_nominal`].
    pub(crate) fn is_sendable_resource_nominal(&self, type_: &ParameterType) -> bool {
        is_sendable_resource_nominal(&self.type_model, type_)
    }

    /// True when a record field of type `field_type` is inlined into the record's
    /// trailing data region (the slot holds a block-relative offset): an inlined
    /// `String`, or a fully-flat composite — a nested record, a flat data union,
    /// or a flat collection (plan-02 §4.2–§4.4). Scalars stay inline in the slot;
    /// not-yet-flat composites stay pointers.
    pub(crate) fn record_field_is_inlined(&self, field_type: &ParameterType) -> bool {
        record_field_is_inlined(&self.type_model, field_type)
    }
}

// ---------------------------------------------------------------------------
// Record field-layout classification (spec/memory/03_heap-values.md §Record).
//
// These are the single source of truth for how a record slot stores each field —
// inline scalar, block-relative offset into the data region (inlined String /
// flat composite), or an 8-byte pointer to a separate allocation. The
// `CodeBuilder` methods above delegate here so the call-site record builder
// (`emit_build_inlined_record`) and the **helper-tier** record marshaller
// (`crate::codegen::memory::marshal::emit_build_inlined_record`, driven from a
// `Body::abi_function` runtime-helper emitter) classify fields identically
// and can never drift.
// ---------------------------------------------------------------------------

/// True when `field_type` occupies a record slot as a pointer to a separate
/// allocation (nested record/union/collection/`Result`/`Error`).
///
/// **A resource field is NOT one of these** (plan-114-B). A concrete resource
/// nominal matches no arm below — resources are registered in `resource_names`,
/// not `record_fields` (`engine/validation/validation.rs:264` populates the
/// latter only for `"type" | "record"` kinds) — so it is classified as a plain
/// 8-byte inline scalar slot holding the handle pointer. That is the wanted
/// layout, and it is the same rule `is_pointer_collection_payload_type` (`:49`)
/// already applies to a collection slot: "a resource handle is a single 8-byte
/// pointer to its record; a slot stores a copy of that pointer exactly like any
/// other pointer payload (§15.6)". A bare resource *union* IS a pointer composite
/// and is caught by the `union_names` arm; a `RES`-marked one — the only form a
/// record field takes — holds its box inline ([`field_is_inlined_union_box`],
/// bug-1012), so it is neither a pointer field nor a value slot.
///
/// EXHAUSTIVE BY CONSTRUCTION — do not add a `_` arm. Its default answer is
/// "a plain 8-byte value slot", so a variant nobody classified gets laid out as
/// a scalar; see the fall-through note in `.ai/codegen-invariants.md`.
#[deny(clippy::wildcard_enum_match_arm)]
pub(crate) fn record_field_is_pointer(model: &TypeModel, field_type: &ParameterType) -> bool {
    match field_type {
        // A collection field is a pointer to its own block.
        ParameterType::ListOf(_) | ParameterType::MapOf(..) | ParameterType::SetOf(_) => true,
        ParameterType::ResultOf(_) => true,
        // A resource handle — bare, `RES`-marked, or a thread handle — is a
        // plain 8-byte slot holding the pointer, NOT a pointer composite
        // (plan-114-B). `is_pointer_collection_payload_type` applies the same
        // rule to a collection slot. A `RES`-marked resource UNION is not a
        // pointer field either: the record holds its box inline (bug-1012,
        // `record_field_is_inlined`).
        ParameterType::Res(_) | ParameterType::ThreadHandle { .. } => false,
        // bug-781: a function value's slot points to the closure object the record
        // owns — its own allocation, copied with the record and dropped with it.
        ParameterType::Func(..) => true,
        // Scalars occupy their slot by value.
        ParameterType::Boolean
        | ParameterType::Byte
        | ParameterType::Integer
        | ParameterType::Fixed
        | ParameterType::Float
        | ParameterType::Money
        | ParameterType::Nothing
        | ParameterType::String => false,
        // Everything left is a NOMINAL (or a shape only the model can place), so
        // the answer is a model lookup rather than a property of the variant.
        ParameterType::Named(_)
        | ParameterType::UserOf(..)
        | ParameterType::Stateful { .. }
        | ParameterType::MapEntryOf(..)
        | ParameterType::AttributeString
        | ParameterType::C(_)
        | ParameterType::Var(_)
        | ParameterType::Arg(_)
        | ParameterType::Unknown => named_field_is_pointer(model, field_type),
    }
}

/// The model half of [`record_field_is_pointer`] — the part exhaustiveness
/// cannot reach, because a nominal's answer depends on its NAME.
fn named_field_is_pointer(model: &TypeModel, field_type: &ParameterType) -> bool {
    model.record_fields.contains_key(field_type)
        // A resource union is a pointer composite (its value is a pointer to a
        // `{tag, ptr}` block), never a flat block. A transferred stateful union
        // is spelled `Stream STATE Cursor`; base-strip so the STATE suffix does
        // not misclassify it as a flat scalar (plan-75 gap 3, else
        // `type_is_arena_transferable` would route the transfer copy to
        // `copy_flat_block` and alias the +8 variant record).
        || model
            .union_names
            .contains(&base_resource_type(field_type))
        || field_type.is_named("Error")
}

/// The payload value types a collection stores: the element type for a `List`,
/// the key and value types for a `Map`.
fn collection_payload_types(type_: &ParameterType) -> Vec<ParameterType> {
    if let Some(element) = typed_list_element_type(type_) {
        vec![element.clone()]
    } else if let Some((key, value)) = typed_map_type_parts(type_) {
        vec![key.clone(), value.clone()]
    } else {
        Vec::new()
    }
}

/// A resource type's base, as a type: the `STATE T` clause stripped. A
/// transferred stateful union is spelled `Stream STATE Cursor` but the union set
/// is keyed on the bare `Stream` (plan-75 gap 3), so every `union_names` lookup
/// goes through this. `Stateful` is a variant since plan-111-A, so this is a
/// match, not a suffix strip — but the composite-base spelling `parse` declines
/// to split still arrives as one opaque `Named`, which is why the `&str` adapter
/// remains the authority for that case.
/// True when `type_` is a resource **nominal**: one of the language's own
/// (`fs.File`, `tcp.Socket`, …) or a user-declared `RESOURCE T CLOSE BY op`,
/// whether this module declares it or imports it.
///
/// bug-546: `builtins::is_resource_type` answers for the built-in registry
/// alone. Every classification that asked it directly therefore treated a
/// user-declared resource as an ordinary nominal — and the answer that fell out
/// of the default was "a plain 8-byte flat scalar", i.e. a copyable, relocatable
/// block. It is not: a `LINK` function returning `AS RES T` arena-allocates the
/// canonical plan-80 resource record (`link_thunk/lower_thunk.rs`, `if function.return_resource`)
/// with its own lifetime and its own close op, exactly as a built-in does.
///
/// plan-156-D: a parent `Thread` view is one too — a resource record in the arena
/// of whoever holds it, closed by `thread.drop`. It has no nominal, so it is matched
/// structurally; a `ThreadWorker` is not (the runtime owns it, and it is never
/// stored or sent).
pub(crate) fn is_resource_nominal(model: &TypeModel, type_: &ParameterType) -> bool {
    crate::codegen::builtins::is_resource_type(type_)
        || matches!(
            base_resource_type(type_),
            ParameterType::ThreadHandle { worker: false, .. }
        )
        || model.resource_names.contains(&base_resource_type(type_))
}

/// True when a collection payload of `type_` is an **alias** to a resource: a
/// resource nominal (an 8-byte pointer to its record) or a resource union (the
/// `{tag, record-ptr}` box held inline). The element accessors strip the `RES`
/// marker, so this is asked of the bare payload type.
fn is_resource_alias_payload(model: &TypeModel, type_: &ParameterType) -> bool {
    if is_resource_nominal(model, type_) {
        return true;
    }
    let base = base_resource_type(type_);
    model.union_names.contains(&base) && !union_is_data(model, &base)
}

/// True when a resource nominal may cross a thread boundary — a built-in with
/// the registry's `sendable` bit, or a user declaration carrying
/// `THREAD_SENDABLE` (17_native-libraries.md).
///
/// A resource this returns `false` for is never deep-copied into another arena:
/// the frontend forbids transferring it, and codegen must not quietly widen that
/// rule for a resource whose author did not opt in.
pub(crate) fn is_sendable_resource_nominal(model: &TypeModel, type_: &ParameterType) -> bool {
    crate::codegen::builtins::is_thread_sendable_resource_type(type_)
        || model
            .sendable_resource_names
            .contains(&base_resource_type(type_))
}

pub(super) fn base_resource_type(type_: &ParameterType) -> ParameterType {
    match type_ {
        ParameterType::Stateful { base, .. } => (**base).clone(),
        other => other.without_state(),
    }
}

/// Which of the two questions [`flatness_walk`] is answering. plan-114-B split
/// these apart: `type_is_flat` used to answer both with one predicate, which was
/// invisible only because no type existed for which they differ. A record holding
/// a resource pointer is the first that does — within a thread a `memcpy` is
/// exactly right (it copies the handle pointer, aliasing the one resource,
/// §15.6), but across an arena boundary the same `memcpy` produces a pointer into
/// the *sender's* arena.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flatness {
    /// "Does a `memcpy` correctly COPY this, within one thread?"
    MemcpyCopyable,
    /// "May this block be RELOCATED into another thread's arena?"
    ArenaTransferable,
}

/// True when a `memcpy` of this value's block is a correct copy within one
/// thread. See [`CodeBuilder::type_is_memcpy_copyable`] for the rule set.
pub(crate) fn type_is_memcpy_copyable(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut visited = std::collections::HashSet::new();
    flatness_walk(model, type_, Flatness::MemcpyCopyable, &mut visited)
}

/// True when this value's block may be relocated into another thread's arena.
/// See [`CodeBuilder::type_is_arena_transferable`] for the rule set.
pub(crate) fn type_is_arena_transferable(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut visited = std::collections::HashSet::new();
    flatness_walk(model, type_, Flatness::ArenaTransferable, &mut visited)
}

/// The one walk behind both predicates. The structural arms (collection payloads,
/// record fields, union variants, `ResultOf`, the cycle guard) exist once and so
/// cannot drift apart; `mode` changes the answer in exactly the three leaf arms
/// marked below.
/// EXHAUSTIVE BY CONSTRUCTION — do not add a `_` arm.
///
/// This function answered two production bugs wrong in one week (bug-546, a
/// user-declared `RESOURCE`; bug-479, a `ThreadHandle`) for the same reason: it
/// was an `if`/`else if` chain whose final `else` returned a DEFAULT, and that
/// default was "flat". A type nobody had thought about was therefore claimed as
/// a copyable, arena-relocatable block — silently, and in the
/// `type_is_arena_transferable` mode that decides whether a block may cross a
/// thread's arena boundary. The tree already records two earlier instances of the
/// same shape: `src/types.rs` notes that the `Stateful` variant "cost a real bug"
/// and that `C(CAbiType)` "changed answer" for every `Named(_)` guard when it
/// landed.
///
/// So the variant dispatch is a `match` with **no wildcard arm**. Adding a 25th
/// `ParameterType` variant now fails to compile here until someone decides what
/// it is, which is the whole point — `cargo build` is the enforcement, not review.
///
/// What this canNOT catch is the bug-546 class, and the boundary is worth being
/// precise about: `Db` is a `Named(Symbol)`, an OPEN namespace, so the variant
/// arm exists and is correct while the answer still depends on a `TypeModel`
/// lookup. Every such lookup is quarantined in [`flatness_of_model_type`] rather
/// than scattered through the arms, so there is exactly one place where a
/// "the model does not recognise this name" default still lives.
#[deny(clippy::wildcard_enum_match_arm)]
fn flatness_walk(
    model: &TypeModel,
    type_: &ParameterType,
    mode: Flatness,
    visited: &mut std::collections::HashSet<ParameterType>,
) -> bool {
    if !visited.insert(type_.clone()) {
        // Already on the current path: a type cycle. Cyclic values cannot be
        // a single finite flat block, so treat them as pointers.
        return false;
    }
    let result = match type_ {
        ParameterType::String => true,
        // DIVERGENCE 1 — a `RES`-marked element/map value. The slot holds one
        // 8-byte pointer to the resource record, so a `memcpy` copying it is a
        // correct alias (§15.6) but relocating it into another arena is not.
        // A `RES`-marked resource union in a record field holds its
        // `{tag, record-ptr}` box inline instead (bug-1012); the same two answers
        // hold, since the box is two plain words, one of them that pointer.
        //
        // Before plan-114-B there was no `Res` arm at all: it fell through to
        // `!record_field_is_pointer(..)`, which has no `Res` arm either, so the
        // answer was `true` INCIDENTALLY rather than by decision — and `true` is
        // the wrong answer for the transfer path. bug-483 is what that class of
        // accident costs when the default happens to be wrong.
        ParameterType::Res(_) => mode == Flatness::MemcpyCopyable,
        // bug-479: a thread handle is flat in NEITHER mode, for the same reason a
        // bare resource nominal is not (the arm below) — and, like it, this used
        // to be decided by accident. There was no `ThreadHandle` arm, and
        // `record_field_is_pointer` in the `else` has none either, so the walk
        // answered `true` for both modes.
        //
        // It is emphatically not flat. The value is a pointer to a heap control
        // block (`CONTROL_BLOCK_SIZE`, plan-156-A) that owns a lock, four queue
        // rings and a block arena, and is shared with a running worker.
        // "Memcpy-copyable" sent
        // an inline-TRAP'd `thread::start` result to `copy_flat_block`, which then
        // asked for an inlined field size for a handle — the reported build
        // failure. "Arena-transferable" would be worse and silent: it is the gate
        // on relocating a block into another thread's arena, and a copied block
        // would duplicate the block while the worker kept using the original.
        //
        // `ir::verify` has never treated a thread view as copyable data — both
        // views are resources there (`is_resource_base`); this arm is codegen
        // finally agreeing with it.
        ParameterType::ThreadHandle { .. } => false,
        // bug-781: a function value is flat in NEITHER mode. Its word points to a
        // closure object the holder owns — copying the holder copies the closure
        // and its environment (`_mfb_rt_closure_copy`), dropping it frees them —
        // so a `memcpy` that shared the pointer would leave two owners of one
        // closure. (It never crosses a thread: `FUNC` is unsendable.)
        ParameterType::Func(..) => false,
        // A flat `Result` `{tag, size, payload}` is pointer-free when its
        // success payload is flat (the `Err` variant is the now-flat `Error`).
        ParameterType::ResultOf(payload) => flatness_walk(model, payload, mode, visited),
        // A collection is flat when every payload is flat — including a nested
        // flat collection, which is inlined in the data region (plan-02 §4.4,
        // Phase 5a). A resource or recursive payload makes it non-flat.
        //
        // These are exactly `typed_is_collection_type`'s three. `MapEntryOf` is
        // deliberately NOT one of them and never was — it falls to the
        // model-decided arm below, where it lands on the same `true` the old
        // `else` gave it. Naming it there rather than here is the point: that
        // answer is inherited, not chosen.
        ParameterType::ListOf(_) | ParameterType::MapOf(..) | ParameterType::SetOf(_) => {
            // DIVERGENCE 2 — a resource ELEMENT (or map value) is an alias, exactly
            // as a `RES` record field is: the element-type accessors strip the `RES`
            // marker, so it arrives here as the bare nominal, but the slot is the
            // same 8-byte pointer to the one resource record (a resource union's is
            // its `{tag, record-ptr}` box, two plain words held inline). Copying the
            // block copies aliases and duplicates no resource; freeing it closes
            // nothing (`mfb spec language resource-management` 15.6: "the collection
            // is still an ordinary copyable collection of references and owns
            // nothing"). So a collection of aliases is plain data within a thread,
            // and, like the `Res(_)` arm, never relocatable into another arena.
            collection_payload_types(type_).into_iter().all(|p| {
                (mode == Flatness::MemcpyCopyable && is_resource_alias_payload(model, &p))
                    || flatness_walk(model, &p, mode, visited)
            })
        }
        // Every remaining variant is either a NOMINAL, whose answer only the
        // `TypeModel` knows, or a scalar that has always taken the same default
        // path. Listed one by one so a new variant cannot join them by accident.
        ParameterType::Named(_)
        | ParameterType::UserOf(..)
        | ParameterType::Stateful { .. }
        | ParameterType::MapEntryOf(..)
        | ParameterType::C(_)
        | ParameterType::AttributeString
        | ParameterType::Var(_)
        | ParameterType::Arg(_)
        | ParameterType::Unknown
        | ParameterType::Boolean
        | ParameterType::Byte
        | ParameterType::Integer
        | ParameterType::Fixed
        | ParameterType::Float
        | ParameterType::Money
        | ParameterType::Nothing => flatness_of_model_type(model, type_, mode, visited),
    };
    visited.remove(type_);
    result
}

/// The half of [`flatness_walk`] the compiler cannot make exhaustive: the answer
/// for a nominal depends on what the `TypeModel` knows about that NAME, and the
/// name namespace is open.
///
/// Kept as one function, and called from exactly one arm, so the residual
/// "nothing matched" default lives in a single place with a name — rather than
/// as the tail of a chain where two bugs have already hidden. Making THIS total
/// is a separate, larger change (a closed `TypeModel::classify` enum); until
/// then the default below is explicit rather than incidental.
fn flatness_of_model_type(
    model: &TypeModel,
    type_: &ParameterType,
    mode: Flatness,
    visited: &mut std::collections::HashSet<ParameterType>,
) -> bool {
    if model.record_fields.contains_key(type_) {
        model
            .record_fields
            .get(type_)
            .cloned()
            .unwrap_or_default()
            .iter()
            .all(|(_, ft)| flatness_walk(model, ft, mode, visited))
    } else if union_is_data(model, type_) {
        model
            .variants_for_union(type_)
            .cloned()
            .collect::<Vec<_>>()
            .iter()
            .all(|variant| flatness_walk(model, variant, mode, visited))
    } else if is_resource_nominal(model, type_) {
        // NOT a divergence — `false` for both modes, unchanged from `type_is_flat`.
        //
        // plan-114-B C6: the plan's §4.1 table said `true` for MemcpyCopyable
        // here, by analogy with the `RES`-marked element above. That analogy is
        // wrong, and the artifact gate proved it. The two positions are different:
        //
        //   * `Res(inner)` is an **element/field marker**. The enclosing block owns
        //     an 8-byte slot holding the handle, so the enclosing block is still a
        //     single pointer-free run of bytes and a `memcpy` of it is correct.
        //   * A **bare resource nominal** is the value's OWN type. "Flat" would
        //     assert that the resource record itself is a copyable block that
        //     `arena_free` reclaims as a unit — and it is not. It is separately
        //     allocated with its own lifetime and its own close op.
        //
        // Answering `true` here does not stay local to resources, either: it
        // propagates through every structural arm. `Result OF tcp.Socket` became
        // "flat", so `is_freeable_flat_value` newly claimed it and registered a
        // `pending_temp` free that had never existed — measured as a real
        // `.ncode` diff in `tests/byte-identity/tcp`, a fixture with no thread in
        // it at all.
        //
        // bug-546: this arm asked `builtins::is_resource_type`, so it saw the
        // language's own resources only. A user-declared `RESOURCE Db CLOSE BY
        // sql::close` matched no arm at all and fell to the `else` below, where
        // `record_field_is_pointer` has no resource arm either — so it answered
        // `true` for BOTH modes, by exactly the accident this comment already
        // records for `Res(_)` before plan-114-B. The memcpy half sent a bare
        // `Db` bind to `copy_flat_block`, which is the reported build failure
        // ("native inlined field size not available for type 'Db'"). The
        // arena-transfer half is worse and silent: it is the predicate that says
        // a thread transfer may relocate a block wholesale, and a relocated
        // resource handle points into the *sender's* arena.
        false
    } else {
        // DIVERGENCE 3 (by inheritance) — a resource *union* reaches
        // `record_field_is_pointer`, which routes it to the pointer-composite
        // path: a `{tag, record-ptr}` block is not a plain slot, so it is false
        // for both modes. Written down here so the next reader does not have to
        // re-derive why the resource arms above do not cover it.
        //
        // Everything else is a scalar (not a pointer composite, `String`, or
        // resource) and is flat for both modes.
        !record_field_is_pointer(model, type_)
    }
}

/// True when a record field of type `field_type` is inlined into the record's
/// trailing data region (the slot holds a block-relative offset): an inlined
/// `String`, or a fully-flat composite. Scalars stay inline in the slot;
/// not-yet-flat composites stay pointers. The answer depends on the field's type
/// alone: every record, builtin or declared, has the same layout (plan-132).
///
/// **A concrete resource field is NOT inlined** (plan-114-B): it is not a
/// composite, so `is_composite` is false and the `type_is_memcpy_copyable` term
/// is never reached. It stays a value slot holding the handle pointer, which is
/// what makes `emit_record_block_size_to_slot` contribute exactly its 8 bytes
/// from the fixed `8 * fields.len()` term and `continue` past it without walking
/// a sub-block that does not exist.
///
/// **A `RES <resource union>` field IS inlined** (bug-1012): the 16-byte
/// `{tag, record-ptr}` box sits in the data region
/// ([`field_is_inlined_union_box`]).
pub(crate) fn record_field_is_inlined(model: &TypeModel, field_type: &ParameterType) -> bool {
    if *field_type == ParameterType::String {
        return true;
    }
    // bug-1012: a `RES <resource union>` field holds the union's `{tag,
    // record-ptr}` box INLINE, as a collection element does.
    if field_is_inlined_union_box(model, field_type) {
        return true;
    }
    let is_composite = model.record_fields.contains_key(field_type)
        || model.union_names.contains(field_type)
        || typed_is_collection_type(field_type)
        || matches!(field_type, ParameterType::ResultOf(_));
    is_composite && type_is_memcpy_copyable(model, field_type)
}

/// Bytes in a resource union's `{tag@0, record-ptr@8}` box.
pub(crate) const RESOURCE_UNION_BOX_SIZE: usize = 16;

/// bug-1012: true when a record field of type `field_type` is a `RES`-marked
/// resource union, whose 16-byte `{tag@0, record-ptr@8}` box the record holds
/// inline: the slot stores the block-relative offset of the box in the record's
/// data region, and a read of the field yields that address.
///
/// The field used to hold the box's ADDRESS. A box is 16 bytes somebody else
/// allocated and frees — the binding that wrapped it, or the statement that built
/// it — so the record read freed memory once that owner was gone (`RETURN
/// keep(u)`, where `keep` builds a record around its parameter), and a box built
/// for the field alone was never freed. Holding the two words makes the record
/// the holder of its own copy: a `memcpy` of the record copies it, the record's
/// one block free releases it, and nothing has to know who made the box the value
/// came from. It is the layout a `List OF RES <Union>` element already has.
///
/// The `RES` marker is what a record field's declared type carries
/// (`model.record_fields` does not strip it); a bare resource union is not a
/// record field (`TYPE_RESOURCE_REQUIRES_RES`).
pub(crate) fn field_is_inlined_union_box(model: &TypeModel, field_type: &ParameterType) -> bool {
    let ParameterType::Res(inner) = field_type else {
        return false;
    };
    let base = base_resource_type(inner);
    model.union_names.contains(&base) && !union_is_data(model, &base)
}

/// True when `type_` is a **data** union (all variants are data records, no
/// resource variants). Data unions use the flat `{tag, size, data}` layout;
/// resource unions keep `{tag, resource-ptr}` and are never reshaped.
pub(crate) fn union_is_data(model: &TypeModel, type_: &ParameterType) -> bool {
    // A transferred stateful union spells `Stream STATE Cursor`; the union set
    // is keyed on the bare name `Stream` (plan-75 gap 3). Strip the suffix so a
    // resource union with STATE still classifies as all-resource.
    let type_ = base_resource_type(type_);
    if !model.union_names.contains(&type_) {
        return false;
    }
    let mut saw_variant = false;
    for variant in model.variants_for_union(&type_) {
        saw_variant = true;
        // bug-851: a `RESOURCE`-declared variant makes it a resource union too.
        if is_resource_nominal(model, variant) {
            return false;
        }
    }
    saw_variant
}

/// The immediate structural components of a type — the types reachable in one
/// step: a record/variant's field types, a union's variant types, a collection's
/// payload types, a `Result`'s success type. Scalars, `String`, and resources
/// have none. Used to detect recursive types for thread-transfer deep copy
/// (bug-391): a recursive value is a pointer-linked graph that inline copy
/// codegen cannot reproduce without unbounded compile-time recursion.
pub(crate) fn type_components(model: &TypeModel, type_: &ParameterType) -> Vec<ParameterType> {
    if let Some(fields) = model.record_fields.get(type_) {
        return fields.iter().map(|(_, ft)| ft.clone()).collect();
    }
    if let Some(fields) = model.union_variant_fields.get(type_) {
        return fields.iter().map(|(_, ft)| ft.clone()).collect();
    }
    if model.union_names.contains(type_) {
        return model.variants_for_union(type_).cloned().collect();
    }
    if let Some(element) = typed_list_element_type(type_) {
        return vec![element.clone()];
    }
    if let Some((key, value)) = typed_map_type_parts(type_) {
        return vec![key.clone(), value.clone()];
    }
    if let ParameterType::ResultOf(payload) = type_ {
        return vec![(**payload).clone()];
    }
    Vec::new()
}

/// True when copying `type_` can transitively re-encounter `type_` itself — it
/// participates in a recursive type definition (e.g. `dom::Node`, whose
/// `ElementNode.children` is `List OF Node`). Such a value needs a per-type
/// runtime copy function rather than inline copy codegen.
pub(crate) fn type_participates_in_cycle(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut stack = type_components(model, type_);
    let mut visited: std::collections::HashSet<ParameterType> = std::collections::HashSet::new();
    while let Some(current) = stack.pop() {
        if current == *type_ {
            return true;
        }
        if !visited.insert(current.clone()) {
            continue;
        }
        stack.extend(type_components(model, &current));
    }
    false
}

/// True when a value of `type_` is a **pointer-linked graph because of a type
/// cycle** — `type_` itself participates in one, or some type reachable from it
/// does. Strictly wider than [`type_participates_in_cycle`], which only answers
/// for the cycle members themselves: in
///
/// ```text
/// TYPE Rep      : child AS Tree, lo AS Integer
/// UNION Tree    : Leaf | Node2
/// TYPE Node2    : left AS Tree, right AS Tree
/// ```
///
/// `Tree` and `Node2` participate; `Rep` does not — yet a `Rep` value still owns
/// a pointer to a separately-allocated `Tree` graph, so it is exactly as
/// alias-prone as the cycle members are.
///
/// bug-538: this is the class where an ordinary `collections::get` handed back an
/// ALIAS into the container's data region instead of the independent value
/// `materialize_owned_element` gives every other element type, because
/// `is_freeable_flat_value` (which needs `type_is_memcpy_copyable`) is false for
/// all of them. It is used ONLY to widen that owning copy; the cycle-member
/// predicate still decides which types get a runtime copy function emitted.
pub(crate) fn type_reaches_cycle(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut stack = vec![type_.clone()];
    let mut visited: std::collections::HashSet<ParameterType> = std::collections::HashSet::new();
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if type_participates_in_cycle(model, &current) {
            return true;
        }
        stack.extend(type_components(model, &current));
    }
    false
}

/// True when a resource is reachable from `type_` — the type itself, a `RES`
/// marker, a field, a variant or a collection payload.
///
/// A resource is move-only: its handle names an OS object with its own close op,
/// so "copy the value" is not a meaning it has. Every deep-copy path that meets
/// one therefore MOVES the handle. bug-538's owning copy must not be applied to
/// such a value — an alias is the correct and existing behaviour there — so the
/// widened gate excludes this set explicitly rather than by accident.
pub(crate) fn type_contains_resource(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut stack = vec![type_.clone()];
    let mut visited: std::collections::HashSet<ParameterType> = std::collections::HashSet::new();
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        // bug-851: a `RESOURCE`-declared nominal (a resource-union variant) too.
        if matches!(
            current,
            ParameterType::Res(_) | ParameterType::ThreadHandle { .. }
        ) || is_resource_nominal(model, &current)
        {
            return true;
        }
        if let ParameterType::Res(inner) = &current {
            stack.push((**inner).clone());
        }
        stack.extend(type_components(model, &current));
    }
    false
}

/// bug-781: true when a function value is reachable from `type_` — the type itself, a
/// field, a variant or a collection / `Result` payload. Such a value owns the closures it
/// holds: an owning store copies them (`_mfb_rt_closure_copy`) and its owner's drop frees
/// them (`_mfb_rt_closure_drop`), so it joins the graph-owned class beside the values
/// whose type reaches a cycle. A function's own parameter and return types are not
/// components: a function value holds code and an environment, never a value of them.
pub(crate) fn type_contains_func(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut stack = vec![type_.clone()];
    let mut visited: std::collections::HashSet<ParameterType> = std::collections::HashSet::new();
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if matches!(current, ParameterType::Func(..)) {
            return true;
        }
        stack.extend(type_components(model, &current));
    }
    false
}

/// plan-134-H: the recursive-value drop walker's kinds (`TypeModel::graph_drop_kinds`), as
/// rendered names. First every [`recursive_transfer_types`] member in that set's order, so a
/// kind index names the same type in the copy walker and the drop walker. Then every
/// resource-free record, union or collection type that only REACHES a cycle — `TYPE Rep /
/// child AS Node`, a `List OF Rep`, a `Map OF String TO Node` — in rendered-name order.
///
/// Such a type is freed by its owner like a cycle member, but it is never a component of the
/// type model when only a local declares it. So the walk is seeded from the model's records
/// and unions and also from every type the module spells: global, parameter and return types,
/// `LET`/`FOR`/`FOR EACH` binding types, and collection-literal, constructor and union-wrap
/// types. Everything reachable from a seed is visited.
pub(crate) fn graph_drop_kind_names(
    model: &TypeModel,
    module: &crate::target::shared::nir::NirModule,
) -> Vec<String> {
    use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
    use crate::target::shared::nir::{NirOp, NirValue};
    struct Seeds(Vec<ParameterType>);
    impl NirVisitor for Seeds {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { type_, .. }
                | NirOp::For { type_, .. }
                | NirOp::ForEach { type_, .. } => self.0.push(type_.clone()),
                _ => {}
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Constructor { type_, .. }
                | NirValue::ListLiteral { type_, .. }
                | NirValue::SetLiteral { type_, .. }
                | NirValue::MapLiteral { type_, .. } => self.0.push(type_.clone()),
                NirValue::UnionWrap { union_type, .. } => self.0.push(union_type.clone()),
                _ => {}
            }
            walk_value(self, value);
        }
    }
    let mut seeds = Seeds(Vec::new());
    for global in &module.globals {
        seeds.0.push(global.type_.clone());
    }
    for function in &module.functions {
        seeds
            .0
            .extend(function.params.iter().map(|param| param.type_.clone()));
        seeds.0.push(function.returns.clone());
        seeds.visit_ops(&function.body);
    }
    seeds.0.extend(model.record_fields.keys().cloned());
    seeds.0.extend(model.union_names.iter().cloned());

    let members = recursive_transfer_types(model);
    let mut reaching: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut visited: std::collections::HashSet<ParameterType> = std::collections::HashSet::new();
    let mut stack = seeds.0;
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        // bug-781: a composite holding a function value is owned and dropped like a
        // value that reaches a cycle. A bare function value needs no kind: its owner
        // calls `_mfb_rt_closure_drop` directly.
        let is_block_type = model.record_fields.contains_key(&current)
            || model.union_names.contains(&current)
            || crate::codegen::engine::types::typed_is_collection_type(&current);
        if is_block_type
            && !type_participates_in_cycle(model, &current)
            && (type_reaches_cycle(model, &current) || type_contains_func(model, &current))
            && !type_contains_resource(model, &current)
        {
            let rendered = current.name().into_owned();
            if !members.contains(&rendered) {
                reaching.insert(rendered);
            }
        }
        stack.extend(type_components(model, &current));
    }
    members.into_iter().chain(reaching).collect()
}

/// Every type in the program that participates in a cycle: the set that needs a
/// runtime thread-transfer deep-copy function emitted (bug-391).
pub(crate) fn recursive_transfer_types(model: &TypeModel) -> std::collections::BTreeSet<String> {
    let mut seen: std::collections::HashSet<ParameterType> = std::collections::HashSet::new();
    let mut result: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // plan-111-E: the WALK is typed; only the RESULT renders, because it feeds
    // emitted symbol names (`thread_copy_symbol`). The `BTreeSet<String>` return
    // also fixes the emission order to the rendered names' order, which is
    // observable in the `.ncode` — keep it.
    let mut stack: Vec<ParameterType> = model
        .record_fields
        .keys()
        .chain(model.union_names.iter())
        .cloned()
        .collect();
    while let Some(current) = stack.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }
        if type_participates_in_cycle(model, &current) {
            result.insert(current.name().into_owned());
        }
        stack.extend(type_components(model, &current));
    }
    result
}

/// The internal symbol of the per-type thread-transfer deep-copy function.
pub(crate) fn thread_copy_symbol(type_: &ParameterType) -> String {
    // An emitted symbol name, so the type renders here — the one legitimate
    // render in this file, at the type -> symbol boundary.
    let type_ = type_.name();
    let mut sanitized = String::new();
    for ch in type_.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    let mut hash: u64 = 1469598103934665603;
    for byte in type_.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("_mfb_thread_copy_{sanitized}_{hash:016x}")
}
