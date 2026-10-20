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

// ---------------------------------------------------------------------------
// A global `String`'s capacity shadow (plan-142-H Open Decision 1).
// ---------------------------------------------------------------------------

/// The hidden global holding the spare capacity of the global `String` `name`'s
/// self-append buffer. `$` keeps it out of every user namespace.
fn global_string_capacity_name(name: &str) -> String {
    format!("$strcap${name}")
}

/// Declare a hidden `Integer` global beside every global `String` that is the
/// target of a self-append (`gs = gs & t`), or of another `String` self-update
/// whose arm needs a shadow (`is_string_self_update`, plan-146-B), anywhere in
/// `module`: the concat arm
/// keeps the buffer's spare capacity there, as it keeps a local's in a frame
/// slot. Every other store to the global frees with it and resets it to 0
/// (`StoreGlobal`), and the global's own initializer is such a store, so it starts
/// at 0. Runs after the optimizer, which would otherwise drop storage no NIR op
/// names.
pub(crate) fn add_global_string_capacities(module: &mut NirModule) {
    struct Finder<'m> {
        strings: &'m std::collections::HashSet<String>,
        found: std::collections::BTreeSet<String>,
    }
    impl NirVisitor for Finder<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::StoreGlobal {
                name,
                value: Some(value),
                ..
            } = op
            {
                if self.strings.contains(name)
                    && crate::codegen::collection::assign::string_self_update::is_string_self_update(
                        value,
                        &|root| matches!(root, NirValue::Global { name: g, .. } if g == name),
                    )
                {
                    self.found.insert(name.clone());
                }
            }
            crate::target::shared::nir::visit::walk_op(self, op);
        }
    }
    let strings: std::collections::HashSet<String> = module
        .globals
        .iter()
        .filter(|global| global.type_ == ParameterType::String)
        .map(|global| global.name.clone())
        .collect();
    let mut finder = Finder {
        strings: &strings,
        found: std::collections::BTreeSet::new(),
    };
    for function in &module.functions {
        finder.visit_ops(&function.body);
    }
    for name in finder.found {
        let hidden = global_string_capacity_name(&name);
        if module.globals.iter().any(|global| global.name == hidden) {
            continue;
        }
        module.globals.push(NirGlobal {
            symbol: crate::target::shared::nir::global_symbol(&module.project, &hidden),
            name: hidden,
            visibility: "private".to_string(),
            mutable: true,
            type_: ParameterType::Integer,
            value: None,
        });
    }
}

impl CodeBuilder<'_> {
    /// The hidden global holding the global `String` `name`'s self-append
    /// capacity, when `add_global_string_capacities` declared one.
    pub(crate) fn global_string_capacity(&self, name: &str) -> Option<String> {
        let hidden = global_string_capacity_name(name);
        self.globals.contains_key(&hidden).then_some(hidden)
    }
}

// ---------------------------------------------------------------------------
// The self-update scratch (plan-142-B Correction B1).
// ---------------------------------------------------------------------------

/// Builtins whose in-place arm keeps per-element state in the function's
/// self-update scratch. A function holding a self-update of one of these gets the
/// scratch slot (`prescan_self_update_scratch`).
pub(crate) const SCRATCH_ARMS: &[&str] = &[
    "filter",
    "distinct",
    "transform",
    "sort",
    "sortBy",
    "union",
    "intersection",
    "difference",
    "symmetricDifference",
    "mapValues",
    "merge",
];

/// Whether `ops` (recursively) hold a self-update `x = f(x, …)` whose call target
/// satisfies `wanted`.
fn ops_hold_self_update(ops: &[NirOp], wanted: &dyn Fn(&str) -> bool) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Assign { name, value } => {
            if with_holds_field_self_update(value, wanted) {
                return true;
            }
            let Some((target, args)) = self_update_call_parts(value) else {
                return false;
            };
            matches!(args.first(), Some(NirValue::Local(arg0)) if arg0 == name) && wanted(target)
        }
        NirOp::StoreGlobal {
            name,
            value: Some(value),
            ..
        } if self_update_call_parts(value).is_some() => {
            let (target, args) = self_update_call_parts(value).expect("matched above");
            matches!(args.first(), Some(NirValue::Global { name: arg0, .. }) if arg0 == name)
                && wanted(target)
        }
        // plan-145-G: `gR = WITH gR { f := g(gR.f, …) }`.
        NirOp::StoreGlobal {
            value: Some(value), ..
        } => with_holds_field_self_update(value, wanted),
        // plan-145-D: a field self-update `r = WITH r { f := g(r.f, …) }` or
        // `h.state = WITH h.state { f := g(h.state.f, …) }`, one field or mixed.
        NirOp::StateAssign { value, .. } => with_holds_field_self_update(value, wanted),
        // plan-147-B (site S11): `RETURN f(x, …)` on an owned local runs the same
        // arm as `x = f(x, …)`, so a function whose only self-update is at a
        // `RETURN` must still reserve the scratch that arm needs. Without this the
        // arm would find none.
        NirOp::Return { value: Some(value) } => {
            returned_self_update_local(value).is_some_and(|_| {
                self_update_call_parts(value).is_some_and(|(target, _)| wanted(target))
            })
                // plan-147-F: the field form of S11 runs the same arms, so a function
                // whose only self-update is `RETURN WITH r { f := OP(r.f, …) }` must
                // reserve the scratch those arms need.
                || with_holds_field_self_update(value, wanted)
        }
        NirOp::If {
            then_body,
            else_body,
            ..
        } => ops_hold_self_update(then_body, wanted) || ops_hold_self_update(else_body, wanted),
        NirOp::Match { cases, .. } => cases
            .iter()
            .any(|case| ops_hold_self_update(&case.body, wanted)),
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => ops_hold_self_update(body, wanted),
        _ => false,
    })
}

/// Whether `value` is a `WITH` over its owner (`r` or `h.state`) with a field
/// update `f := g(<owner>.f, …)` whose `g` is `wanted` — a field self-update.
/// The owner is the `WITH` target itself; `ops_hold_self_update`'s caller
/// matched the assignment's left side to it.
fn with_holds_field_self_update(value: &NirValue, wanted: &dyn Fn(&str) -> bool) -> bool {
    let NirValue::WithUpdate {
        target: owner,
        updates,
        ..
    } = value
    else {
        return false;
    };
    updates.iter().any(|update| match &update.value {
        NirValue::Call { target, args, .. } => {
            matches!(
                args.first(),
                Some(NirValue::MemberAccess { target: inner, member })
                    if *member == update.field && same_field_owner(inner, owner)
            ) && wanted(target)
        }
        // plan-145-F: `inner := WITH <owner>.inner { f := g(<owner>.inner.f, …) }`.
        NirValue::WithUpdate { target: nested, .. } => {
            matches!(
                nested.as_ref(),
                NirValue::MemberAccess { target: inner, member }
                    if *member == update.field && same_field_owner(inner, owner)
            ) && with_holds_field_self_update(&update.value, wanted)
        }
        _ => false,
    })
}

/// Whether two field owners name the same place: the local `r`, `h.state`, or
/// (plan-145-F) a field path under either, `o.inner`.
fn same_field_owner(a: &NirValue, b: &NirValue) -> bool {
    match (a, b) {
        (NirValue::Local(x), NirValue::Local(y)) => x == y,
        // plan-145-G: a module-level record.
        (NirValue::Global { name: x, .. }, NirValue::Global { name: y, .. }) => x == y,
        (
            NirValue::MemberAccess {
                target: x,
                member: mx,
            },
            NirValue::MemberAccess {
                target: y,
                member: my,
            },
        ) => mx == my && same_field_owner(x, y),
        _ => false,
    }
}

/// Whether `ops` create a closure whose lambda borrows its creator's scratch
/// (`scratch_closure_captures`).
fn ops_create_scratch_closure(builder: &CodeBuilder<'_>, ops: &[NirOp]) -> bool {
    struct Finder<'b, 'a> {
        builder: &'b CodeBuilder<'a>,
        found: bool,
    }
    impl NirVisitor for Finder<'_, '_> {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Closure { name, captures, .. } = value {
                if !captures.is_empty() && self.builder.function_needs_scratch(name) {
                    self.found = true;
                }
            }
            walk_value(self, value);
        }
    }
    let mut finder = Finder {
        builder,
        found: false,
    };
    finder.visit_ops(ops);
    finder.found
}

/// bug-767 A: whether `ops` hold a copying `collections::sort`/`sortBy` call that
/// the word-merge fast paths decline — the shapes `lower_copying_sort{,_by}`
/// serve by sorting a copy with the in-place arm's merge, which keeps its index
/// buffers in the self-update scratch. Only this function's own scratch: the
/// predicate is not part of `function_needs_scratch`, so no closure borrows a
/// creator's scratch for it (a `keyFn` cannot hold a self-update — only a
/// `forEach` lambda mutates a capture — so the merge never shares the block).
fn ops_hold_copying_sort(ops: &[NirOp]) -> bool {
    struct Finder {
        found: bool,
    }
    impl NirVisitor for Finder {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } =
                value
            {
                let copying_sort = (target.starts_with("#collections_sortBy$")
                    && args.len() == 2
                    && !sort_by_word_path_accepts(target, args))
                    || (target.starts_with("#collections_sort$")
                        && args.len() == 1
                        && !sort_word_path_accepts(target, args));
                if copying_sort {
                    self.found = true;
                }
            }
            walk_value(self, value);
        }
    }
    let mut finder = Finder { found: false };
    finder.visit_ops(ops);
    finder.found
}

/// Whether a self-update of `target` has an arm that keeps state in the
/// function's self-update scratch.
fn target_needs_self_update_scratch(target: &str) -> bool {
    self_update_builtin(target).is_some_and(|bare| SCRATCH_ARMS.contains(&bare))
        // plan-146-D/E: the `String` arms that build their result in the scratch
        // (`os::appResourcePath`'s prefix, the six rewrites), by qualified target.
        || crate::codegen::collection::assign::string_self_update::target_needs_string_scratch(
            target,
        )
        || crate::codegen::collection::assign::builder_inplace_rewrite::math_self_update_function(
            target,
        )
        .is_some()
}

/// plan-146-D: whether the module holds a self-update `x = f(x, …)` of the builtin
/// whose bare name is `bare`.
pub(crate) fn module_self_updates_with(module: &NirModule, bare: &str) -> bool {
    module.functions.iter().any(|function| {
        ops_hold_self_update(&function.body, &|target| {
            self_update_builtin(target) == Some(bare)
        })
    })
}

/// plan-146-D/G: whether `ops` hold `s = os::appResourcePath(s)`.
pub(crate) fn ops_hold_resource_path_self_update(ops: &[NirOp]) -> bool {
    ops_hold_self_update(ops, &|target| {
        self_update_builtin(target) == Some("appResourcePath")
    })
}

/// Whether the module holds a `collections::replace` self-update (plan-142-C).
/// Its arm writes through `lower_list_set_in_place`, whose rebuild path — never
/// taken from that arm, but always emitted — raises `ErrIndexOutOfRange`, so the
/// module needs that message's data object even when it calls no bounds-checked
/// member (`data_objects::string_symbols`).
pub(crate) fn module_self_updates_with_replace(module: &NirModule) -> bool {
    module.functions.iter().any(|function| {
        ops_hold_self_update(&function.body, &|target| {
            self_update_builtin(target) == Some("replace")
        })
    })
}

impl CodeBuilder<'_> {
    /// Whether the module function `name`'s body holds a self-update whose arm
    /// needs the scratch.
    pub(crate) fn function_needs_scratch(&self, name: &str) -> bool {
        self.functions.get(name).is_some_and(|function| {
            ops_hold_self_update(&function.body, &target_needs_self_update_scratch)
        })
    }

    /// plan-142-G: a lambda holding a scratch self-update (the self-update of a
    /// by-ref capture, site S9) runs once per element of the `forEach` that calls
    /// it, so a scratch of its own would be allocated once per element. It
    /// borrows its creator's instead: the closure env carries one word past its
    /// captures — the address of the creator's scratch slot. `Some(index of that
    /// word)` for such a lambda, found from the `Closure` node that creates it.
    pub(crate) fn scratch_closure_captures(&self, lambda: &str) -> Option<usize> {
        if !self.function_needs_scratch(lambda) {
            return None;
        }
        struct Finder<'n> {
            lambda: &'n str,
            captures: Option<usize>,
        }
        impl NirVisitor for Finder<'_> {
            fn visit_value(&mut self, value: &NirValue) {
                if let NirValue::Closure { name, captures, .. } = value {
                    if name == self.lambda && !captures.is_empty() {
                        self.captures = Some(captures.len());
                    }
                }
                walk_value(self, value);
            }
        }
        let mut finder = Finder {
            lambda,
            captures: None,
        };
        for function in self.functions.values() {
            finder.visit_ops(&function.body);
        }
        finder.captures
    }

    /// Give this function a self-update scratch slot if its body holds a
    /// self-update whose arm needs one, or creates a closure that borrows it
    /// (`scratch_closure_captures`). The slot is registered as a function-level
    /// owned `List OF Integer`, so the ordinary scope drop frees it on every exit
    /// (with the null guard and prologue zeroing that drop brings). A lambda that
    /// borrows its creator's scratch gets a working slot and no cleanup. A
    /// function without such a statement is untouched.
    pub(crate) fn prescan_self_update_scratch(&mut self, function: &str, ops: &[NirOp]) {
        if self.self_update_scratch.is_some() {
            return;
        }
        if let Some(index) = self.scratch_closure_captures(function) {
            self.self_update_scratch = Some(self.allocate_stack_object("su_scratch", 8));
            self.self_update_scratch_env = Some(index);
            return;
        }
        if !ops_hold_self_update(ops, &target_needs_self_update_scratch)
            && !ops_create_scratch_closure(self, ops)
            && !ops_hold_copying_sort(ops)
        {
            return;
        }
        let slot = self.allocate_stack_object("su_scratch", 8);
        self.self_update_scratch = Some(slot);
        self.active_cleanups
            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                type_: ParameterType::list_of(ParameterType::Integer),
                stack_offset: slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            }));
    }

    /// plan-146-D: give this function a slot for `os::appResourcePath`'s base block
    /// when its body holds `s = os::appResourcePath(s)` — the in-place arm computes the
    /// base at most once per call and reads it from there. Registered as a
    /// function-level owned `String`, so the ordinary scope drop frees it (with the
    /// null guard and prologue zeroing that drop brings), exactly as the self-update
    /// scratch is.
    pub(crate) fn prescan_string_app_resource_base(&mut self, ops: &[NirOp]) {
        if self.string_app_resource_base.is_some() || self.string_app_resource_base_env.is_some() {
            return;
        }
        // plan-146-G: a function that lends the cache to a lambda needs one too.
        let lends = self.closures_created_in(ops).into_iter().any(|lambda| {
            crate::codegen::collection::assign::string_self_update::lambda_shares_resource_base(
                self.functions,
                &lambda,
            )
        });
        if !ops_hold_resource_path_self_update(ops) && !lends {
            return;
        }
        let slot = self.allocate_stack_object("str_resource_base", 8);
        self.string_app_resource_base = Some(slot);
        self.active_cleanups
            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                type_: ParameterType::String,
                stack_offset: slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            }));
        // The prologue zeroes it from here: without that, a second call to this
        // function reads the block the first one cached AND freed (a lambda runs
        // once per element, and the third call died on the reused memory).
        self.owned_value_slots.push(slot);
    }

    /// plan-146-G: in a lambda, the closure-environment word holding the address
    /// of the creator's `os::appResourcePath` base-path cache, after the shadow words.
    pub(crate) fn prescan_string_app_resource_base_env(&mut self, function: &str) {
        if !crate::codegen::collection::assign::string_self_update::lambda_shares_resource_base(
            self.functions,
            function,
        ) {
            return;
        }
        let Some(total) = self.closure_capture_count(function) else {
            return;
        };
        let scratch_words = usize::from(self.scratch_closure_captures(function).is_some());
        let shadow_words =
            crate::codegen::collection::assign::string_self_update::string_shadow_captures(
                self.functions,
                function,
            )
            .len();
        self.string_app_resource_base_env = Some(total + scratch_words + shadow_words);
    }

    /// plan-146-G: in a lambda, record which closure-environment word holds the
    /// address of each by-ref-captured `String`'s OWNER shadow slot. The layout is
    /// `[captures…][the borrowed scratch, if any][one word per shadowed capture]`,
    /// and the creator writes the same words in the same order
    /// (`string_shadow_captures`).
    pub(crate) fn prescan_string_shadow_env(&mut self, function: &str) {
        let captures =
            crate::codegen::collection::assign::string_self_update::string_shadow_captures(
                self.functions,
                function,
            );
        if captures.is_empty() {
            return;
        }
        let Some(total) = self.closure_capture_count(function) else {
            return;
        };
        // The same predicate the creator uses to decide whether it lends its
        // scratch (`borrowed_scratch`), NOT `self_update_scratch_env` — that field
        // is set by a later prescan, and reading it here put the shadow word on top
        // of the scratch word (both sides then wrote through word 1: the scratch
        // pointer landed in the owner's shadow and the scratch leaked).
        let scratch_words = usize::from(self.scratch_closure_captures(function).is_some());
        for (position, (local, _)) in captures.into_iter().enumerate() {
            self.string_shadow_env
                .insert(local, total + scratch_words + position);
        }
    }

    /// The number of values the closure creating `lambda` captures, or `None` when
    /// no closure does (or it captures nothing, so there is no environment).
    pub(crate) fn closure_capture_count(&self, lambda: &str) -> Option<usize> {
        struct Finder<'n> {
            lambda: &'n str,
            captures: Option<usize>,
        }
        impl NirVisitor for Finder<'_> {
            fn visit_value(&mut self, value: &NirValue) {
                if let NirValue::Closure { name, captures, .. } = value {
                    if name == self.lambda && !captures.is_empty() {
                        self.captures = Some(captures.len());
                    }
                }
                walk_value(self, value);
            }
        }
        let mut finder = Finder {
            lambda,
            captures: None,
        };
        for function in self.functions.values() {
            finder.visit_ops(&function.body);
        }
        finder.captures
    }

    /// Make the self-update scratch hold at least the byte count in `need_slot`,
    /// and return a fresh frame slot holding the address of its first byte. The
    /// bytes' contents are unspecified. Grows to twice the request (at least 64
    /// bytes), so a loop over similar sizes allocates it once.
    ///
    /// `Err` when the function has no scratch slot: an arm that needs one must
    /// check `self_update_scratch` in its gates, before it emits anything.
    pub(crate) fn emit_reserve_self_update_scratch(
        &mut self,
        need_slot: usize,
    ) -> Result<usize, String> {
        let scratch_slot = self
            .self_update_scratch
            .ok_or("native self-update scratch requested in a function without one")?;
        self.emit_reserve_scratch_block(scratch_slot, self.self_update_scratch_env, need_slot)
    }

    /// [`Self::emit_reserve_self_update_scratch`] over the scratch block whose
    /// frame slot is `scratch_slot` — borrowed through closure-env word `env`
    /// when that is `Some` — shared with bug-766 B's record-temp scratch.
    fn emit_reserve_scratch_block(
        &mut self,
        scratch_slot: usize,
        env: Option<usize>,
        need_slot: usize,
    ) -> Result<usize, String> {
        let list_type = ParameterType::list_of(ParameterType::Integer);
        let layout = CollectionTypeLayout::from_type(&list_type)
            .ok_or("native self-update scratch has no List OF Integer layout")?;
        let block = self.temporary_vreg();
        let cap = self.temporary_vreg();
        let need = self.temporary_vreg();
        let size = self.temporary_vreg();
        let mask = self.temporary_vreg();
        let newcap_slot = self.allocate_stack_object("su_scratch_newcap", 8);
        let data_slot = self.allocate_stack_object("su_scratch_data", 8);
        let grow = self.label("su_scratch_grow");
        let no_free = self.label("su_scratch_no_free");
        let alloc_ok = self.label("su_scratch_alloc_ok");
        let ready = self.label("su_scratch_ready");

        // A borrowed scratch: take the creator's current block as the working copy.
        if let Some(index) = env {
            let holder = self.temporary_vreg();
            self.emit(abi::load_u64(
                &holder,
                CLOSURE_ENV_REGISTER,
                closure_env_word_offset(index),
            ));
            self.emit(abi::load_u64(&block, &holder, 0));
            self.emit(abi::store_u64(&block, abi::stack_pointer(), scratch_slot));
        }
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::compare_immediate(&block, "0"));
        self.emit(abi::branch_eq(&grow));
        self.emit(abi::load_u64(&cap, &block, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::load_u64(&need, abi::stack_pointer(), need_slot));
        self.emit(abi::compare_registers(&need, &cap));
        self.emit(abi::branch_hi(&grow));
        self.emit(abi::branch(&ready));

        // newCapacity = align8(2 * need + 64).
        self.emit(abi::label(&grow));
        self.emit(abi::load_u64(&need, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&size, &need, &need));
        self.emit(abi::add_immediate(&size, &size, 64 + 7));
        self.emit(abi::move_immediate(&mask, "Integer", &(!7u64).to_string()));
        self.emit(abi::and_registers(&size, &size, &mask));
        self.emit(abi::store_u64(&size, abi::stack_pointer(), newcap_slot));
        // Free the old block (its contents are scratch), sized as the drop sizes
        // it: HEADER + dataCapacity (a `List OF Integer` has no entry table).
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::compare_immediate(&block, "0"));
        self.emit(abi::branch_eq(&no_free));
        self.emit(abi::load_u64(&cap, &block, COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::add_immediate(
            abi::c_arg(1),
            &cap,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::move_register(abi::c_arg(0), &block));
        self.emit_arena_free_call();
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            scratch_slot,
        ));
        // The creator must never see the freed block, even if the alloc below
        // raises: its drop would free it again.
        self.emit_publish_borrowed_scratch(scratch_slot, env);
        self.emit(abi::label(&no_free));
        let size = self.temporary_vreg();
        self.emit(abi::load_u64(&size, abi::stack_pointer(), newcap_slot));
        self.emit(abi::add_immediate(
            abi::c_arg(0),
            &size,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            scratch_slot,
        ));
        let nb = self.temporary_vreg();
        let zero = self.temporary_vreg();
        let count_cap = self.temporary_vreg();
        let dcap = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), scratch_slot));
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::load_u64(&dcap, abi::stack_pointer(), newcap_slot));
        self.emit(abi::shift_right_immediate(&count_cap, &dcap, 3));
        self.emit_write_collection_header_full(&layout, &nb, &zero, &count_cap, &zero, &dcap);
        self.emit_publish_borrowed_scratch(scratch_slot, env);

        self.emit(abi::label(&ready));
        let block = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::add_immediate(&block, &block, COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64(&block, abi::stack_pointer(), data_slot));
        Ok(data_slot)
    }

    /// In a lambda borrowing its creator's scratch, store the working copy in
    /// `scratch_slot` back into the creator's slot. Nothing otherwise.
    fn emit_publish_borrowed_scratch(&mut self, scratch_slot: usize, env: Option<usize>) {
        let Some(index) = env else {
            return;
        };
        let block = self.temporary_vreg();
        let holder = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::load_u64(
            &holder,
            CLOSURE_ENV_REGISTER,
            closure_env_word_offset(index),
        ));
        self.emit(abi::store_u64(&block, &holder, 0));
    }
}

// ---------------------------------------------------------------------------
// The record-temp scratch (bug-766 B).
// ---------------------------------------------------------------------------

/// bug-766 B: the `WITH` items of `x = set(x, k, WITH …)` / `x = append(x, WITH …)`
/// statements in `ops` (recursively) — the items the self-update arms build in
/// the record-temp scratch (`CodeBuilder::lower_self_update_item`).
fn self_update_with_items<'o>(ops: &'o [NirOp], out: &mut Vec<&'o ParameterType>) {
    for op in ops {
        match op {
            NirOp::Assign { name, value } => {
                if let (Some(NirValue::WithUpdate { type_, .. }), Some((_, args))) = (
                    crate::codegen::engine::analysis::borrow_get::copied_item(value),
                    self_update_call_parts(value),
                ) {
                    if matches!(args.first(), Some(NirValue::Local(arg0)) if arg0 == name) {
                        out.push(type_);
                    }
                }
            }
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                self_update_with_items(then_body, out);
                self_update_with_items(else_body, out);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    self_update_with_items(&case.body, out);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => self_update_with_items(body, out),
            _ => {}
        }
    }
}

impl CodeBuilder<'_> {
    /// bug-766 B: the byte size of every value of `type_` when it is a record of
    /// compile-time fixed size with a single-`arena_free` drop — each field a
    /// scalar or a fixed-size inlined record (`record_field_is_inlined_fixed`),
    /// laid out as `8 × fields` slots followed by each inlined field's sub-block
    /// (`emit_build_inlined_record`). `None` for any other type.
    pub(crate) fn fixed_record_size(&self, type_: &ParameterType) -> Option<usize> {
        fn size(builder: &CodeBuilder<'_>, type_: &ParameterType) -> Option<usize> {
            let fields = builder.type_model.record_fields.get(type_)?;
            let mut total = 8 * fields.len();
            for (_, field) in fields {
                if builder.record_field_is_inlined(field) {
                    total += size(builder, field)?;
                }
            }
            Some(total)
        }
        if !self.record_field_is_inlined_fixed(type_)
            || !self.is_freeable_flat_value(type_)
            || *type_ == ParameterType::String
        {
            return None;
        }
        size(self, type_).filter(|total| *total > 0)
    }

    /// bug-766 B: give each scratch-copy `get` binding (`BorrowGets::scratch`) of a
    /// fixed-size record type its region. Offsets follow the name order, so the
    /// layout does not depend on hash order.
    pub(crate) fn install_scratch_copies(
        &mut self,
        scratch: std::collections::HashMap<String, ParameterType>,
    ) {
        let mut names: Vec<(String, ParameterType)> = scratch
            .into_iter()
            .filter(|(name, _)| !self.promotable_vector_locals.contains(name))
            .collect();
        names.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, type_) in names {
            let Some(size) = self.fixed_record_size(&type_) else {
                continue;
            };
            let offset = self.record_temps.size;
            self.record_temps.size += size;
            self.record_temps.bindings.insert(name, (offset, size));
        }
    }

    /// bug-766 B: after [`Self::install_scratch_copies`], give the `WITH` items of
    /// the function's `set`/`append` self-updates one shared region (the arm copies
    /// its item before it lowers anything else, so no two are live at once), and
    /// the function a record-temp scratch slot when any region exists. The slot is
    /// a function-level owned `List OF Integer`, zeroed at entry and freed at
    /// every exit, exactly as the self-update scratch is.
    pub(crate) fn install_record_temps(&mut self, ops: &[NirOp]) {
        let mut items = Vec::new();
        self_update_with_items(ops, &mut items);
        let item_size = items
            .into_iter()
            .filter_map(|type_| self.fixed_record_size(type_))
            .max();
        if let Some(item_size) = item_size {
            self.record_temps.with_item = Some((self.record_temps.size, item_size));
            self.record_temps.size += item_size;
        }
        if self.record_temps.size == 0 {
            return;
        }
        let slot = self.allocate_stack_object("rec_scratch", 8);
        self.record_temps.slot = Some(slot);
        self.active_cleanups
            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                type_: ParameterType::list_of(ParameterType::Integer),
                stack_offset: slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            }));
        self.owned_value_slots.push(slot);
    }

    /// bug-766 B: the address of the record-temp region at `offset`, reserving the
    /// function's record-temp scratch first (one allocation per activation: every
    /// reservation asks for the same `size`, so the block never grows or moves).
    pub(crate) fn emit_record_temp_region(
        &mut self,
        offset: usize,
    ) -> Result<crate::codegen::engine::operand::VirtualRegister, String> {
        let slot = self
            .record_temps
            .slot
            .ok_or("native record temp requested in a function without a record-temp scratch")?;
        let need = self.temporary_vreg();
        let need_slot = self.allocate_stack_object("rec_scratch_need", 8);
        self.emit(abi::move_immediate(
            &need,
            "Integer",
            &self.record_temps.size.to_string(),
        ));
        self.emit(abi::store_u64(&need, abi::stack_pointer(), need_slot));
        let data_slot = self.emit_reserve_scratch_block(slot, None, need_slot)?;
        let region = self.allocate_register();
        self.emit(abi::load_u64(&region, abi::stack_pointer(), data_slot));
        if offset != 0 {
            self.emit(abi::add_immediate(&region, &region, offset));
        }
        Ok(region)
    }

    /// bug-766 B: copy the `size` bytes (a multiple of 8) at `src` to `dst`, one
    /// word at a time. Nothing here calls out.
    pub(crate) fn emit_copy_record_words(
        &mut self,
        dst: impl Into<abi::Reg>,
        src: impl Into<abi::Reg>,
        size: usize,
    ) {
        let (dst, src) = (dst.into(), src.into());
        let word = self.allocate_register();
        for offset in (0..size).step_by(8) {
            self.emit(abi::load_u64(&word, &src, offset));
            self.emit(abi::store_u64(&word, &dst, offset));
        }
    }
}

#[cfg(test)]
mod tests;
