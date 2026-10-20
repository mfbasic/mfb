//! plan-142-A: the self-update table and the one dispatch every binding site shares.
//!
//! A **self-update** is `x = f(x, …)` where `x` is a `List`, `Map` or `Set` (or the
//! `String` self-concat `s = s & t`). Lowered naively it copies `x` into a fresh
//! block every statement. An *arm* recognises one such `f` and mutates `x`'s
//! existing block instead.
//!
//! Two lists live here, and the census tests below tie them to the registry:
//!
//! * [`SELF_UPDATE_ARMS`] — the dispatch list, in order. [`CodeBuilder::try_inplace_self_update`]
//!   runs the arms against a [`SelfUpdateSite`] and stops at the first that fires.
//!   Every arm declines without emitting (inventory rule `O-order-1`), so the order
//!   only matters for the one name-shared pair, `append` before `bulk_append`.
//! * `SELF_UPDATE_TABLE` — one row per registry function with a self-update-shaped
//!   overload (`registry::self_update_shaped`: a `List`, `Map`, `Set`, `String` or
//!   `AttributedString` first parameter whose return type can equal it), naming the
//!   arm(s) that serve it or why none is needed. The 19 Tier-B `AttributedString`
//!   transforms are not registry overloads and come from `TIER_B_TRANSFORMS`.
//!
//! A binding site (a function local, a module-level global, …) is only a different
//! way of building a [`SelfUpdateSite`]: every arm is automatically an arm at every
//! site, and no arm is written per site.

use crate::codegen::collection::assign::builder_inplace_sort::{
    sort_by_word_path_accepts, sort_word_path_accepts,
};
use crate::codegen::collection::assign::inplace_dest::InPlaceDest;
use crate::codegen::engine::builder::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::visit::{walk_value, NirVisitor};
use crate::target::shared::nir::*;
use crate::types::ParameterType;

mod global_capacity;
mod record_temps;
mod scratch;
pub(crate) use global_capacity::add_global_string_capacities;
use scratch::with_holds_field_self_update;
pub(crate) use scratch::{
    module_self_updates_with, module_self_updates_with_replace, ops_hold_resource_path_self_update,
};

/// One in-place arm, by name. The table refers to arms by id so a row can be
/// checked against [`SELF_UPDATE_ARMS`] without comparing function pointers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ArmId {
    /// `xs = append(xs, element)`.
    Append,
    /// `xs = append(xs, otherList)` — a concatenation.
    BulkAppend,
    /// `s = add(s, element)` on a `Set`.
    SetAdd,
    /// `xs = set(xs, i, v)` on a `List`, `m = set(m, k, v)` on a `Map`.
    Set,
    /// `m = removeKey(m, k)`.
    RemoveKey,
    /// `xs = prepend(xs, element)`.
    Prepend,
    /// `xs = removeAt(xs, i)`.
    RemoveAt,
    /// `xs = insert(xs, i, element)`.
    Insert,
    /// `s = remove(s, element)` on a `Set`.
    SetRemove,
    /// `s = s & t` on a `String`.
    Concat,
    /// `xs = filter(xs, predicate)` (plan-142-B).
    Filter,
    /// `xs = take(xs, n)` (plan-142-B).
    Take,
    /// `xs = drop(xs, n)` (plan-142-B).
    Drop,
    /// `xs = mid(xs, start, n)` (plan-142-B).
    Mid,
    /// `xs = distinct(xs)` (plan-142-B).
    Distinct,
    /// `xs = math::f(xs, …)` for the 16 element-wise `math` functions (plan-142-C).
    Math,
    /// `xs = replace(xs, old, new)` (plan-142-C).
    Replace,
    /// `xs = transform(xs, f)` with `f` returning the element type (plan-142-C).
    Transform,
    /// `xs = sort(xs)` (plan-142-C).
    Sort,
    /// `xs = sortBy(xs, keyFn)` (plan-142-C).
    SortBy,
    /// `s = union(s, t)` (plan-142-D).
    Union,
    /// `s = intersection(s, t)` (plan-142-D).
    Intersection,
    /// `s = difference(s, t)` (plan-142-D).
    Difference,
    /// `s = symmetricDifference(s, t)` (plan-142-D).
    SymmetricDifference,
    /// `m = merge(m, n, preferB)` (plan-142-D).
    Merge,
    /// `m = mapValues(m, f)` with `f` returning the value type (plan-142-D).
    MapValues,
    /// `s = toString(s)` on a `String` — the identity (plan-146-B).
    StrIdentity,
    /// `s = f(s, …)` for the 13 `String` builtins whose result is a window of `s`
    /// (plan-146-C).
    StrWindow,
    /// `s = f(s, …)` for the `String` builtins whose result is `s` with bytes
    /// added (plan-146-D).
    StrGrow,
    /// `s = f(s, …)` for the `String` builtins that rewrite `s`'s bytes
    /// (plan-146-E).
    StrRewrite,
    /// `xs = repeat(xs, item, count, start)` (plan-153-B).
    Repeat,
}

/// A binding being self-updated: which one, its type, and where its block lives.
///
/// Built once per assignment by the site that recognised it; every arm reads the
/// destination from here rather than from the frame, which is what lets a new
/// site reuse every arm.
pub(crate) struct SelfUpdateSite<'a> {
    /// The binding's name — the statement is `name = f(name, …)`. For a field site
    /// it is the owner: the record local, or the `RES` handle.
    pub(crate) name: &'a str,
    /// The binding's declared type — for a field site, the FIELD's type.
    pub(crate) type_: ParameterType,
    /// Where the binding's block pointer lives.
    pub(crate) dest: InPlaceDest,
    /// `G1` — the local is a by-ref capture whose slot holds a pointer to the
    /// parent's slot, not the block.
    pub(crate) by_ref: bool,
    /// plan-145-B: `Some` when the self-update is a record or `STATE` FIELD's
    /// (`r = WITH r { f := op(r.f, …) }`, `h.state.f = op(h.state.f, …)`).
    pub(crate) field: Option<FieldSite<'a>>,
}

/// plan-145-F: one outer level of a nested field path — `inner` in `o.inner.b`:
/// the level's field, its position in its record, and that record's type.
#[derive(Clone)]
pub(crate) struct FieldLevel<'a> {
    pub(crate) field: &'a str,
    pub(crate) field_index: usize,
    pub(crate) record_type: ParameterType,
    /// The level is a POINTER record field (its slot holds the record's own
    /// block) rather than an inlined one; only the store routine descends one
    /// (`peel_field_path`'s `pointer_levels`).
    pub(crate) pointer: bool,
}

/// plan-145-B: which field of which owner a field site updates.
pub(crate) struct FieldSite<'a> {
    pub(crate) container: FieldContainer<'a>,
    /// plan-145-F: the inlined record fields from the owner down to the updated
    /// field's record, outermost first — empty for a field of the owner itself
    /// (`r.f`), `[inner]` for `o.inner.f`.
    pub(crate) path: Vec<FieldLevel<'a>>,
    /// The updated field's name.
    pub(crate) field: &'a str,
    /// Its position in the owner record's fields (the slot the record's block
    /// stores its block-relative offset in).
    pub(crate) field_index: usize,
    /// The owner record's type.
    pub(crate) record_type: ParameterType,
}

impl FieldSite<'_> {
    /// plan-145-F: the path's field indices, outermost first
    /// (`InPlaceDest::Inlined::path`).
    pub(crate) fn path_indices(&self) -> Vec<usize> {
        self.path.iter().map(|level| level.field_index).collect()
    }
}

/// plan-145-B: who owns the field's block.
#[derive(Clone, Copy)]
pub(crate) enum FieldContainer<'a> {
    /// A record local, whose frame slot holds the record block pointer.
    Record { local: &'a str },
    /// A `RES … STATE` handle, whose resource record holds the payload pointer.
    State { resource: &'a str },
    /// plan-145-G: a module-level record, whose global slot holds the block
    /// pointer.
    Global { name: &'a str },
}

impl SelfUpdateSite<'_> {
    /// Whether `value` is this binding itself: the local, or — for a global
    /// destination — the global; for a field site, the field (`G18`).
    pub(crate) fn is_self(&self, value: &NirValue) -> bool {
        if let Some(field) = &self.field {
            return field_place_is(value, field.container, &field.path, field.field);
        }
        match (&self.dest, value) {
            (InPlaceDest::Global { .. }, NirValue::Global { name, .. }) => name == self.name,
            (InPlaceDest::Global { .. }, _) => false,
            (_, NirValue::Local(name)) => name == self.name,
            _ => false,
        }
    }

    /// Whether evaluating `value` reads this binding anywhere inside it. For a
    /// field site that is any read of the owner — conservative, and exactly the
    /// self-alias test the record and `STATE` arms made (`G12`).
    pub(crate) fn read_by(&self, value: &NirValue) -> bool {
        let global_owner = matches!(self.dest, InPlaceDest::Global { .. })
            || matches!(
                self.field.as_ref().map(|field| field.container),
                Some(FieldContainer::Global { .. })
            );
        if !global_owner {
            return crate::codegen::engine::control::nir_value_reads_local(value, self.name);
        }
        struct Finder<'n> {
            name: &'n str,
            found: bool,
        }
        impl NirVisitor for Finder<'_> {
            fn visit_value(&mut self, value: &NirValue) {
                if matches!(value, NirValue::Global { name, .. } if name == self.name) {
                    self.found = true;
                }
                walk_value(self, value);
            }
        }
        let mut finder = Finder {
            name: self.name,
            found: false,
        };
        finder.visit_value(value);
        finder.found
    }
}

/// Whether `value` is the owner itself: the record local, or the handle's
/// `.state`.
pub(crate) fn field_owner_is(value: &NirValue, container: FieldContainer<'_>) -> bool {
    match container {
        FieldContainer::Record { local } => matches!(value, NirValue::Local(n) if n == local),
        FieldContainer::Global { name } => {
            matches!(value, NirValue::Global { name: g, .. } if g == name)
        }
        FieldContainer::State { resource } => matches!(
            value,
            NirValue::MemberAccess { target, member }
                if member == "state"
                    && matches!(target.as_ref(), NirValue::Local(n) if n == resource)
        ),
    }
}

/// plan-145-F: whether `value` reads exactly the place `<owner>.<path…>.field`.
pub(crate) fn field_place_is(
    value: &NirValue,
    container: FieldContainer<'_>,
    path: &[FieldLevel<'_>],
    field: &str,
) -> bool {
    let NirValue::MemberAccess { target, member } = value else {
        return false;
    };
    if member != field {
        return false;
    }
    match path.split_last() {
        Some((last, rest)) => field_place_is(target, container, rest, last.field),
        None => field_owner_is(target, container),
    }
}

/// An arm: `Ok(true)` when it lowered the statement in place, `Ok(false)` to
/// decline (having emitted nothing).
pub(crate) type ArmFn =
    fn(&mut CodeBuilder<'_>, &SelfUpdateSite<'_>, &NirValue) -> Result<bool, String>;

/// plan-145-B: whether an arm serves a field site, and how. D and E replace
/// `Existing` with the arm's reallocation class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldReach {
    /// The arm declines every field site.
    None,
    /// The arm serves a field site exactly as the record and `STATE` arm it
    /// absorbed did (plan-121-C/D). Its growing routes need the last-inlined
    /// field (`field_is_last_inlined`); its others may mutate any inlined
    /// collection field where it lies.
    Existing,
    /// plan-145-D: for a fixed-width element (or value) the arm never stores a new
    /// block pointer, so it mutates the field's sub-block where it lies — at any
    /// inlined collection field, last or not (Open Decision 2). plan-145-E: a
    /// variable-width kind may repack (a reallocation), which reaches the field
    /// through `InlineGrow` at its owner's last inlined field only
    /// (`field_realloc_admitted`, in the arm).
    NoRealloc,
    /// plan-145-E: the arm reallocates (its inserts grow the collection), so it
    /// runs at a field only through `InlineGrow`, at its owner's last inlined
    /// field, with no pointer operand read from the owner (`field_realloc_admitted`).
    Realloc,
}

/// The dispatch list. Every arm-backed `SELF_UPDATE_TABLE` row names ids from
/// here; `self_update_table_has_no_stale_rows` checks both directions.
pub(crate) const SELF_UPDATE_ARMS: &[(ArmId, ArmFn, FieldReach)] = &[
    // `append` and `bulk_append` share the builtin name and split on G11
    // (element vs list item type); keep single-element first.
    (
        ArmId::Append,
        |b, s, v| b.try_inplace_append_assign(s, v),
        FieldReach::Existing,
    ),
    (
        ArmId::BulkAppend,
        |b, s, v| b.try_inplace_bulk_append_assign(s, v),
        FieldReach::Existing,
    ),
    (
        ArmId::SetAdd,
        |b, s, v| b.try_inplace_set_add_assign(s, v),
        FieldReach::Existing,
    ),
    (
        ArmId::Set,
        |b, s, v| b.try_inplace_set_assign(s, v),
        FieldReach::Existing,
    ),
    (
        ArmId::RemoveKey,
        |b, s, v| b.try_inplace_remove_key_assign(s, v),
        FieldReach::Existing,
    ),
    (
        ArmId::Prepend,
        |b, s, v| b.try_inplace_prepend_assign(s, v),
        FieldReach::Existing,
    ),
    (
        ArmId::RemoveAt,
        |b, s, v| b.try_inplace_remove_at_assign(s, v),
        FieldReach::Existing,
    ),
    (
        ArmId::Insert,
        |b, s, v| b.try_inplace_insert_assign(s, v),
        FieldReach::Existing,
    ),
    (
        ArmId::Repeat,
        |b, s, v| b.try_inplace_repeat_assign(s, v),
        FieldReach::Realloc,
    ),
    (
        ArmId::SetRemove,
        |b, s, v| b.try_inplace_set_remove_assign(s, v),
        FieldReach::Existing,
    ),
    (
        ArmId::Concat,
        |b, s, v| b.try_inplace_concat_assign(s, v),
        FieldReach::None,
    ),
    (
        ArmId::Filter,
        |b, s, v| b.try_inplace_filter_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::Take,
        |b, s, v| b.try_inplace_take_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::Drop,
        |b, s, v| b.try_inplace_drop_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::Mid,
        |b, s, v| b.try_inplace_mid_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::Distinct,
        |b, s, v| b.try_inplace_distinct_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::Math,
        |b, s, v| b.try_inplace_math_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::Replace,
        |b, s, v| b.try_inplace_replace_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::Transform,
        |b, s, v| b.try_inplace_transform_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::Sort,
        |b, s, v| b.try_inplace_sort_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::SortBy,
        |b, s, v| b.try_inplace_sort_by_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::Union,
        |b, s, v| b.try_inplace_union_assign(s, v),
        FieldReach::Realloc,
    ),
    (
        ArmId::Intersection,
        |b, s, v| b.try_inplace_intersection_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::Difference,
        |b, s, v| b.try_inplace_difference_assign(s, v),
        FieldReach::NoRealloc,
    ),
    (
        ArmId::SymmetricDifference,
        |b, s, v| b.try_inplace_symmetric_difference_assign(s, v),
        FieldReach::Realloc,
    ),
    (
        ArmId::Merge,
        |b, s, v| b.try_inplace_merge_assign(s, v),
        FieldReach::Realloc,
    ),
    (
        ArmId::MapValues,
        |b, s, v| b.try_inplace_map_values_assign(s, v),
        FieldReach::NoRealloc,
    ),
    // plan-146: the `String` arms. Each declines every field site (a `String`
    // field is `deferred:string`, plan-145-A Open Decision 1).
    (
        ArmId::StrIdentity,
        |b, s, v| b.try_inplace_string_identity_assign(s, v),
        FieldReach::None,
    ),
    // After the collection `Mid` arm, which declines a `String` at G10: the two
    // share the bare name `mid`, and both decline without emitting.
    (
        ArmId::StrWindow,
        |b, s, v| b.try_inplace_string_window_assign(s, v),
        FieldReach::None,
    ),
    (
        ArmId::StrGrow,
        |b, s, v| b.try_inplace_string_grow_assign(s, v),
        FieldReach::None,
    ),
    // After the collection `Replace` arm, which declines a `String` at G10.
    (
        ArmId::StrRewrite,
        |b, s, v| b.try_inplace_string_rewrite_assign(s, v),
        FieldReach::None,
    ),
];

/// The bare builtin name a self-update's call target names, for every spelling a
/// call has after lowering (plan-142-B):
///
/// * a native member — `collections.append` → `append` (and `strings.mid` →
///   `mid`: the arm's collection-type gate tells the two apart), exactly
///   [`native_builtin_target`](crate::codegen::builtins::native_builtin_target);
/// * a `Body::Mfb` member's monomorph — `#collections_take$Integer` → `take` (the
///   injected `__collections_take OF T`, internalized and mangled per instance;
///   `native_builtin_target` answers `None` for it);
/// * the unmonomorphized qualified spelling of such a member —
///   `collections.take` → `take`;
/// * plan-146-B: the `String` arm rows whose target is none of those
///   ([`STRING_SELF_UPDATE_SPELLINGS`]).
///
/// `None` for anything else.
pub(crate) fn self_update_builtin(target: &str) -> Option<&'static str> {
    if let Some(bare) = crate::codegen::builtins::native_builtin_target(target) {
        return Some(bare);
    }
    if let Some((_, bare)) = STRING_SELF_UPDATE_SPELLINGS
        .iter()
        .find(|(spelling, _)| *spelling == target)
    {
        return Some(bare);
    }
    let member = match target.strip_prefix("#collections_") {
        Some(rest) => rest.split('$').next()?,
        None => target.strip_prefix("collections.")?,
    };
    crate::codegen::registry::registry()
        .packages()
        .iter()
        .find(|package| package.import_name() == "collections")?
        .function(member)
        .map(|function| function.name)
}

/// plan-146-B Phase 1: the call targets of the `String` rows plan-146 arms that
/// `native_builtin_target` does not name, recorded from a `mfb build --nir` probe
/// of `s = f(s, …)` at S1 and S2: two `Body::Rewrite` members (their MFBASIC
/// helpers, internalized), one `Body::abi_function` member, and the unqualified
/// `toString`. Every other plan-146 arm row (`strings.left`, `fs.pathBaseName`, …)
/// is a `Body::abi_inline`/`Intrinsic` native the first rule already answers.
pub(crate) const STRING_SELF_UPDATE_SPELLINGS: &[(&str, &str)] = &[
    ("#strings_padLeftToWidth", "padLeftToWidth"),
    ("#strings_padRightToWidth", "padRightToWidth"),
    ("os.appResourcePath", "appResourcePath"),
    ("os.appDataPath", "appDataPath"),
    ("os.appCachePath", "appCachePath"),
    ("os.userHomePath", "userHomePath"),
    ("os.userDocumentsPath", "userDocumentsPath"),
    ("toString", "toString"),
];

impl CodeBuilder<'_> {
    /// Lower `site.name = value` in place if any arm recognises it. `false` =
    /// every arm declined and nothing was emitted; the caller takes the copying
    /// reassignment.
    pub(crate) fn try_inplace_self_update(
        &mut self,
        site: &SelfUpdateSite<'_>,
        value: &NirValue,
    ) -> Result<bool, String> {
        // A `Ref` or `Global` destination works on a copy of the block pointer:
        // load it before any arm reads the slot, publish it back once one has run.
        self.open_inplace_ref_dest(&site.dest)?;
        for (_, arm, reach) in SELF_UPDATE_ARMS {
            // plan-145-B: an arm that serves no field declines a field site.
            if site.field.is_some() && *reach == FieldReach::None {
                continue;
            }
            if arm(self, site, value)? {
                self.close_inplace_dest(&site.dest)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// plan-146-D: a call's `(target, args)`, whichever node the lowering produced —
/// a plain `Call`, or the `RuntimeCall` an `abi_function` member's call site
/// becomes (`os::appResourcePath`). Both are `f(args…)` to every gate here.
pub(crate) fn self_update_call_parts(value: &NirValue) -> Option<(&str, &[NirValue])> {
    match value {
        NirValue::Call { target, args, .. } => Some((target.as_str(), args.as_slice())),
        NirValue::RuntimeCall { target, args, .. } => Some((target.as_str(), args.as_slice())),
        _ => None,
    }
}

/// Whether `value` is shaped `f(name, …)` for a builtin with a self-update arm —
/// the test for giving a by-ref local's statement a `Ref` destination (plan-142-G)
/// before any slot is allocated for it.
pub(crate) fn is_self_update_call(value: &NirValue, name: &str) -> bool {
    self_update_call_parts(value).is_some_and(|(target, args)| {
        self_update_builtin(target).is_some()
            && matches!(args.first(), Some(NirValue::Local(arg0)) if arg0 == name)
    })
}

/// plan-147-B (site S11): the local `x` of a `RETURN f(x, …)` whose `f` has a
/// self-update arm, so the return can update `x`'s block in place and then move
/// that block out instead of building — and then copying — a second one.
///
/// This names only the SHAPE. Whether `x` may actually be updated and moved is
/// `CodeBuilder::returned_move_admits`, which applies `plan_returned_move`'s
/// ownership gates.
pub(crate) fn returned_self_update_local(value: &NirValue) -> Option<&str> {
    let (target, args) = self_update_call_parts(value)?;
    // Every spelling the seam dispatches, not just the collection/`String` ones:
    // `math::abs(xs)` and friends are arms too (`MATH_SELF_UPDATE`), and they reach
    // the dispatcher under a `math.` target that `self_update_builtin` does not
    // name. Missing them made S11 silently skip all 15 `math` array rows.
    if self_update_builtin(target).is_none()
        && crate::codegen::collection::assign::builder_inplace_rewrite::math_self_update_function(
            target,
        )
        .is_none()
    {
        return None;
    }
    match args.first() {
        Some(NirValue::Local(name)) => Some(name.as_str()),
        _ => None,
    }
}

/// plan-147-F: the owner local of a `RETURN WITH r { f := OP(r.f, …) }` whose `OP`
/// has a self-update arm — the FIELD form of letter B's site S11.
///
/// The census's record accumulators are threaded exactly like the collection ones
/// (`state = walkSchema(…, state, depth + 1)`), and the helper returns
/// `WITH s { items := collections::append(s.items, i) }`. Without this the whole
/// record is copied to update one field.
pub(crate) fn returned_field_self_update_local(value: &NirValue) -> Option<&str> {
    let NirValue::WithUpdate { target, .. } = value else {
        return None;
    };
    let NirValue::Local(name) = target.as_ref() else {
        return None;
    };
    // The same spellings letter B's plain form accepts, field-form.
    with_holds_field_self_update(value, &|target| {
        self_update_builtin(target).is_some()
            || crate::codegen::collection::assign::builder_inplace_rewrite::math_self_update_function(
                target,
            )
            .is_some()
    })
    .then_some(name.as_str())
}

/// [`is_self_update_call`] for the module-level global `name` (plan-142-H).
pub(crate) fn is_global_self_update_call(value: &NirValue, name: &str) -> bool {
    self_update_call_parts(value).is_some_and(|(target, args)| {
        self_update_builtin(target).is_some()
            && matches!(args.first(), Some(NirValue::Global { name: arg0, .. }) if arg0 == name)
    })
}

#[cfg(test)]
mod tests;
