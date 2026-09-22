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
//!   overload (`registry::self_update_shaped`), naming the arm(s) that serve it or
//!   why none is needed.
//!
//! A binding site (a function local, a module-level global, …) is only a different
//! way of building a [`SelfUpdateSite`]: every arm is automatically an arm at every
//! site, and no arm is written per site.

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
    ("os.resourcePath", "resourcePath"),
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

/// Whether `value` is shaped `f(name, …)` for a builtin with a self-update arm —
/// the test for giving a by-ref local's statement a `Ref` destination (plan-142-G)
/// before any slot is allocated for it.
pub(crate) fn is_self_update_call(value: &NirValue, name: &str) -> bool {
    matches!(value, NirValue::Call { target, args, .. }
        if self_update_builtin(target).is_some()
            && matches!(args.first(), Some(NirValue::Local(arg0)) if arg0 == name))
}

/// [`is_self_update_call`] for the module-level global `name` (plan-142-H).
pub(crate) fn is_global_self_update_call(value: &NirValue, name: &str) -> bool {
    matches!(value, NirValue::Call { target, args, .. }
        if self_update_builtin(target).is_some()
            && matches!(args.first(), Some(NirValue::Global { name: arg0, .. }) if arg0 == name))
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
            let NirValue::Call { target, args, .. } = value else {
                return false;
            };
            matches!(args.first(), Some(NirValue::Local(arg0)) if arg0 == name) && wanted(target)
        }
        NirOp::StoreGlobal {
            name,
            value: Some(NirValue::Call { target, args, .. }),
            ..
        } => {
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

/// Whether a self-update of `target` has an arm that keeps state in the
/// function's self-update scratch.
fn target_needs_self_update_scratch(target: &str) -> bool {
    self_update_builtin(target).is_some_and(|bare| SCRATCH_ARMS.contains(&bare))
        || crate::codegen::collection::assign::builder_inplace_rewrite::math_self_update_function(
            target,
        )
        .is_some()
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
        {
            return;
        }
        let slot = self.allocate_stack_object("su_scratch", 8);
        self.self_update_scratch = Some(slot);
        self.active_cleanups
            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                type_: ParameterType::list_of(ParameterType::Integer),
                stack_offset: slot,
                closure_captures: None,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            }));
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
        if let Some(index) = self.self_update_scratch_env {
            let holder = self.temporary_vreg();
            self.emit(abi::load_u64(&holder, CLOSURE_ENV_REGISTER, index * 8));
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
        self.emit_publish_borrowed_scratch(scratch_slot);
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
        self.emit_publish_borrowed_scratch(scratch_slot);

        self.emit(abi::label(&ready));
        let block = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::add_immediate(&block, &block, COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64(&block, abi::stack_pointer(), data_slot));
        Ok(data_slot)
    }

    /// In a lambda borrowing its creator's scratch, store the working copy in
    /// `scratch_slot` back into the creator's slot. Nothing otherwise.
    fn emit_publish_borrowed_scratch(&mut self, scratch_slot: usize) {
        let Some(index) = self.self_update_scratch_env else {
            return;
        };
        let block = self.temporary_vreg();
        let holder = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::load_u64(&holder, CLOSURE_ENV_REGISTER, index * 8));
        self.emit(abi::store_u64(&block, &holder, 0));
    }
}

// ---------------------------------------------------------------------------
// The table. It exists for the census guards below — the dispatch reads
// `SELF_UPDATE_ARMS` only — so it is compiled for tests.
// ---------------------------------------------------------------------------

/// How a self-update-shaped function avoids copying `x`.
#[cfg(test)]
#[derive(Debug)]
pub(crate) enum SelfUpdate {
    /// In place at every site, by these arm ids (a function may need two, e.g.
    /// `append` single-element and bulk).
    Arm(&'static [ArmId]),
    /// No copy of `x` exists to avoid: the result is not built from `x`'s block
    /// (plan-142-E). `proof` cites the lowering that shows `x` is only read.
    Exempt {
        reason: &'static str,
        proof: &'static str,
    },
    /// plan-146-A: still copies; the plan-146 letter named here lands its arm or
    /// its proven exemption. Letter H deletes this variant again, as plan-142-I did.
    Pending(&'static str),
    /// Still copies; the named plan owns it (plan-146-A Open Decision 1: the
    /// `AttributedString` forms, `attributed-string`).
    Deferred(&'static str),
}

/// A program fragment that performs one self-update of `x`, for the matrix test.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Probe {
    /// Packages the fragment needs besides `io`.
    pub(crate) imports: &'static [&'static str],
    /// Top-level helper `FUNC`s the call refers to (callbacks), or empty.
    pub(crate) helpers: &'static str,
    /// `x`'s declared type.
    pub(crate) ty: &'static str,
    /// `x`'s initial value.
    pub(crate) init: &'static str,
    /// The right-hand side of `x = …`.
    pub(crate) call: &'static str,
}

/// One row: a self-update-shaped function and how it is served.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct SelfUpdateRow {
    /// `pkg::name` of a registry function, or an operator spelling (`&`) for an
    /// operator self-update, which has no registry entry.
    pub(crate) function: &'static str,
    pub(crate) kind: SelfUpdate,
    /// Fragments exercising the row; together they must fire every arm it names.
    pub(crate) probes: &'static [Probe],
}

#[cfg(test)]
const C: &[&str] = &["collections"];
#[cfg(test)]
const M: &[&str] = &["math"];
#[cfg(test)]
const Z: &[&str] = &["compress", "encoding"];
#[cfg(test)]
const K: &[&str] = &["crypto", "encoding"];
#[cfg(test)]
const ST: &[&str] = &["strings"];
#[cfg(test)]
const FS: &[&str] = &["fs"];
#[cfg(test)]
const OS: &[&str] = &["os"];
#[cfg(test)]
const EN: &[&str] = &["encoding"];
#[cfg(test)]
const NET: &[&str] = &["net"];
#[cfg(test)]
const RE: &[&str] = &["regex"];
#[cfg(test)]
const AS: &[&str] = &["astrings"];
#[cfg(test)]
const AST: &[&str] = &["astrings", "strings"];

#[cfg(test)]
const fn probe(
    imports: &'static [&'static str],
    ty: &'static str,
    init: &'static str,
    call: &'static str,
) -> Probe {
    Probe {
        imports,
        helpers: "",
        ty,
        init,
        call,
    }
}

#[cfg(test)]
const fn probe_with(
    imports: &'static [&'static str],
    helpers: &'static str,
    ty: &'static str,
    init: &'static str,
    call: &'static str,
) -> Probe {
    Probe {
        imports,
        helpers,
        ty,
        init,
        call,
    }
}

/// plan-146-A: a `String` probe.
#[cfg(test)]
const fn str_probe(
    imports: &'static [&'static str],
    init: &'static str,
    call: &'static str,
) -> Probe {
    probe(imports, "String", init, call)
}

#[cfg(test)]
const STR: &str = "\"abcdef\"";
#[cfg(test)]
const PATH: &str = "\"/tmp/a/b.txt\"";
#[cfg(test)]
const ATTR: &str = "AttributedString";
#[cfg(test)]
const ATTR_INIT: &str = "astrings::fromString(\"abcdef\")";
#[cfg(test)]
const LI: &str = "List OF Integer";
#[cfg(test)]
const LF: &str = "List OF Float";
#[cfg(test)]
const LX: &str = "List OF Fixed";
#[cfg(test)]
const FIXEDS: &str = "[1.5F, 2.25F, 0.5F]";
#[cfg(test)]
const LB: &str = "List OF Byte";
#[cfg(test)]
const SI: &str = "Set OF Integer";
#[cfg(test)]
const MSI: &str = "Map OF String TO Integer";
#[cfg(test)]
const FLOATS: &str = "[0.1, 0.2, 0.3]";
#[cfg(test)]
const BYTES: &str = "encoding::utf8Encode(\"hello, hello, hello\")";
#[cfg(test)]
const IS_POSITIVE: &str = "FUNC isPositive(n AS Integer) AS Boolean\n  RETURN n > 0\nEND FUNC\n\n";
#[cfg(test)]
const NEGATED: &str = "FUNC negated(n AS Integer) AS Integer\n  RETURN 0 - n\nEND FUNC\n\n";
#[cfg(test)]
const KEYLEN: &str = "FUNC keyLen(s AS String) AS Integer\n  RETURN len(s)\nEND FUNC\n\n";
#[cfg(test)]
const KEYSTR: &str = "FUNC keyStr(n AS Integer) AS String\n  RETURN toString(0 - n)\nEND FUNC\n\n";
#[cfg(test)]
const PUSH: &str = "FUNC push(acc AS List OF Integer, n AS Integer) AS List OF Integer\n  RETURN collections::append(acc, n)\nEND FUNC\n\n";

#[cfg(test)]
const REDUCE_REASON: &str =
    "The result is built from `initial`, not from `x`: the fold only walks `x`.";
#[cfg(test)]
const REDUCE_PROOF: &str = "`lower_collection_reduce_impl` (`builtins/collections/gen_memory.rs`) stores `args[0]` in `reduce_collection` and only walks it (`initialize_collection_loop_slots`, `load_collection_loop_item`); the accumulator starts from `args[1]`.";
#[cfg(test)]
const COMPRESS_REASON: &str =
    "The result is a new byte stream whose length is unrelated to `x`'s; `x` is only read.";
#[cfg(test)]
const COMPRESS_PROOF: &str = "`Body::Rewrite` to an MFBASIC helper that reads its `data` parameter only through `len`, `collections::get`/`getOr` and a header-sized `mid` (`helper_deflate_core.rs`, `helper_inflate_core.rs`, `helper_gzip_frame.rs`, `helper_zlib_frame.rs`, `helper_crc32.rs`, `helper_adler32.rs`): no binding of `data`, so no copy.";
#[cfg(test)]
const ARGON_REASON: &str = "The result is a derived key; the password is only read.";
#[cfg(test)]
const ARGON_PROOF: &str = "`__crypto_argon2H0` hashes `header || password || tail` with `__crypto_blake2b3`, which compresses whole blocks of the password where they lie (`helper_blake2b.rs`); it used to concatenate the password into a buffer (fixed by plan-142-E).";
#[cfg(test)]
const SHAKE_REASON: &str = "The result is a derived digest; `data` is only read.";
#[cfg(test)]
const SHAKE_PROOF: &str = "`__crypto_keccakSponge` absorbs whole blocks straight from `data` and builds only the final padded block (`helper_keccak_sponge.rs`); it used to copy all of `data` into a padded buffer (fixed by plan-142-E).";

/// Every registry function with a self-update-shaped overload
/// (`registry::self_update_shaped`), plus the `String` self-concat.
#[cfg(test)]
pub(crate) const SELF_UPDATE_TABLE: &[SelfUpdateRow] = &[
    // --- in place today (site S1) ---
    SelfUpdateRow {
        function: "collections::append",
        kind: SelfUpdate::Arm(&[ArmId::Append, ArmId::BulkAppend]),
        probes: &[
            probe(C, LI, "[1, 2, 3]", "collections::append(x, 4)"),
            probe(C, LI, "[1, 2, 3]", "collections::append(x, [4, 5])"),
        ],
    },
    SelfUpdateRow {
        function: "collections::set",
        kind: SelfUpdate::Arm(&[ArmId::Set]),
        probes: &[
            probe(C, LI, "[1, 2, 3]", "collections::set(x, 0, 9)"),
            probe(
                C,
                MSI,
                "Map OF String TO Integer { \"a\" := 1 }",
                "collections::set(x, \"b\", 2)",
            ),
        ],
    },
    SelfUpdateRow {
        function: "collections::add",
        kind: SelfUpdate::Arm(&[ArmId::SetAdd]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::add(x, 3)",
        )],
    },
    SelfUpdateRow {
        function: "collections::remove",
        kind: SelfUpdate::Arm(&[ArmId::SetRemove]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::remove(x, 1)",
        )],
    },
    SelfUpdateRow {
        function: "collections::removeKey",
        kind: SelfUpdate::Arm(&[ArmId::RemoveKey]),
        probes: &[probe(
            C,
            MSI,
            "Map OF String TO Integer { \"a\" := 1, \"b\" := 2 }",
            "collections::removeKey(x, \"a\")",
        )],
    },
    SelfUpdateRow {
        function: "collections::prepend",
        kind: SelfUpdate::Arm(&[ArmId::Prepend]),
        probes: &[probe(C, LI, "[1, 2, 3]", "collections::prepend(x, 0)")],
    },
    SelfUpdateRow {
        function: "collections::removeAt",
        kind: SelfUpdate::Arm(&[ArmId::RemoveAt]),
        probes: &[probe(
            C,
            LI,
            "[1, 2, 3, 4, 5]",
            "collections::removeAt(x, 0)",
        )],
    },
    SelfUpdateRow {
        function: "collections::insert",
        kind: SelfUpdate::Arm(&[ArmId::Insert]),
        probes: &[probe(C, LI, "[1, 2, 3]", "collections::insert(x, 1, 7)")],
    },
    SelfUpdateRow {
        function: "&",
        kind: SelfUpdate::Arm(&[ArmId::Concat]),
        probes: &[probe(&[], "String", "\"a\"", "x & \"b\"")],
    },
    // --- letter B: shrink / compact ---
    SelfUpdateRow {
        function: "collections::filter",
        kind: SelfUpdate::Arm(&[ArmId::Filter]),
        probes: &[probe_with(
            C,
            IS_POSITIVE,
            LI,
            "[1, -2, 3]",
            "collections::filter(x, isPositive)",
        )],
    },
    SelfUpdateRow {
        function: "collections::take",
        kind: SelfUpdate::Arm(&[ArmId::Take]),
        probes: &[probe(C, LI, "[1, 2, 3, 4, 5]", "collections::take(x, 4)")],
    },
    SelfUpdateRow {
        function: "collections::drop",
        kind: SelfUpdate::Arm(&[ArmId::Drop]),
        probes: &[probe(C, LI, "[1, 2, 3, 4, 5]", "collections::drop(x, 1)")],
    },
    SelfUpdateRow {
        function: "collections::mid",
        kind: SelfUpdate::Arm(&[ArmId::Mid]),
        probes: &[probe(C, LI, "[1, 2, 3, 4, 5]", "collections::mid(x, 0, 4)")],
    },
    SelfUpdateRow {
        function: "collections::distinct",
        kind: SelfUpdate::Arm(&[ArmId::Distinct]),
        probes: &[probe(C, LI, "[1, 1, 2]", "collections::distinct(x)")],
    },
    // --- letter C: element rewrite / reorder ---
    SelfUpdateRow {
        function: "collections::replace",
        kind: SelfUpdate::Arm(&[ArmId::Replace]),
        probes: &[probe(C, LI, "[1, 2, 1]", "collections::replace(x, 1, 5)")],
    },
    SelfUpdateRow {
        function: "collections::transform",
        kind: SelfUpdate::Arm(&[ArmId::Transform]),
        probes: &[probe_with(
            C,
            NEGATED,
            LI,
            "[1, 2, 3]",
            "collections::transform(x, negated)",
        )],
    },
    SelfUpdateRow {
        function: "collections::sort",
        kind: SelfUpdate::Arm(&[ArmId::Sort]),
        probes: &[
            probe(C, LI, "[3, 1, 2]", "collections::sort(x)"),
            probe(
                C,
                "List OF String",
                "[\"b\", \"a\"]",
                "collections::sort(x)",
            ),
            probe(C, LF, FLOATS, "collections::sort(x)"),
            probe(
                C,
                "List OF Byte",
                "[toByte(3), toByte(1)]",
                "collections::sort(x)",
            ),
        ],
    },
    SelfUpdateRow {
        function: "collections::sortBy",
        kind: SelfUpdate::Arm(&[ArmId::SortBy]),
        probes: &[
            probe_with(
                C,
                NEGATED,
                LI,
                "[3, 1, 2]",
                "collections::sortBy(x, negated)",
            ),
            probe_with(
                C,
                KEYLEN,
                "List OF String",
                "[\"bb\", \"a\"]",
                "collections::sortBy(x, keyLen)",
            ),
            probe_with(C, KEYSTR, LI, "[3, 1, 2]", "collections::sortBy(x, keyStr)"),
        ],
    },
    SelfUpdateRow {
        function: "math::abs",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::abs(x)"),
            probe(M, LI, "[1, -2, 3]", "math::abs(x)"),
            probe(M, LX, FIXEDS, "math::abs(x)"),
        ],
    },
    SelfUpdateRow {
        function: "math::acos",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::acos(x)")],
    },
    SelfUpdateRow {
        function: "math::asin",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::asin(x)")],
    },
    SelfUpdateRow {
        function: "math::atan",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::atan(x)")],
    },
    SelfUpdateRow {
        function: "math::atan2",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::atan2(x, [1.0, 1.0, 1.0])")],
    },
    SelfUpdateRow {
        function: "math::clamp",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::clamp(x, 0.0, 0.25)"),
            probe(M, LI, "[1, -2, 3]", "math::clamp(x, -1, 2)"),
            probe(M, LX, FIXEDS, "math::clamp(x, 0.5F, 2.0F)"),
        ],
    },
    SelfUpdateRow {
        function: "math::cos",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::cos(x)")],
    },
    SelfUpdateRow {
        function: "math::exp",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::exp(x)")],
    },
    SelfUpdateRow {
        function: "math::log",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::log(x)"),
            probe(M, LX, FIXEDS, "math::log(x)"),
        ],
    },
    SelfUpdateRow {
        function: "math::log10",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::log10(x)"),
            probe(M, LX, FIXEDS, "math::log10(x)"),
        ],
    },
    SelfUpdateRow {
        function: "math::max",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::max(x, [0.5, 0.0, 0.5])"),
            probe(M, LI, "[1, -2, 3]", "math::max(x, [2, 2, 2])"),
            probe(M, LX, FIXEDS, "math::max(x, [1.0F, 1.0F, 1.0F])"),
        ],
    },
    SelfUpdateRow {
        function: "math::min",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::min(x, [0.5, 0.0, 0.5])"),
            probe(M, LI, "[1, -2, 3]", "math::min(x, [2, 2, 2])"),
            probe(M, LX, FIXEDS, "math::min(x, [1.0F, 1.0F, 1.0F])"),
        ],
    },
    SelfUpdateRow {
        function: "math::pow",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::pow(x, [1.0, 2.0, 1.0])")],
    },
    SelfUpdateRow {
        function: "math::sin",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::sin(x)")],
    },
    SelfUpdateRow {
        function: "math::sqrt",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[
            probe(M, LF, FLOATS, "math::sqrt(x)"),
            probe(M, LX, FIXEDS, "math::sqrt(x)"),
        ],
    },
    SelfUpdateRow {
        function: "math::tan",
        kind: SelfUpdate::Arm(&[ArmId::Math]),
        probes: &[probe(M, LF, FLOATS, "math::tan(x)")],
    },
    // --- letter D: Set algebra and Map ---
    SelfUpdateRow {
        function: "collections::union",
        kind: SelfUpdate::Arm(&[ArmId::Union]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::union(x, Set OF Integer { 2, 3 })",
        )],
    },
    SelfUpdateRow {
        function: "collections::intersection",
        kind: SelfUpdate::Arm(&[ArmId::Intersection]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::intersection(x, Set OF Integer { 2, 3 })",
        )],
    },
    SelfUpdateRow {
        function: "collections::difference",
        kind: SelfUpdate::Arm(&[ArmId::Difference]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::difference(x, Set OF Integer { 2, 3 })",
        )],
    },
    SelfUpdateRow {
        function: "collections::symmetricDifference",
        kind: SelfUpdate::Arm(&[ArmId::SymmetricDifference]),
        probes: &[probe(
            C,
            SI,
            "Set OF Integer { 1, 2 }",
            "collections::symmetricDifference(x, Set OF Integer { 2, 3 })",
        )],
    },
    SelfUpdateRow {
        function: "collections::merge",
        kind: SelfUpdate::Arm(&[ArmId::Merge]),
        probes: &[probe(
            C,
            MSI,
            "Map OF String TO Integer { \"a\" := 1 }",
            "collections::merge(x, Map OF String TO Integer { \"b\" := 2 }, TRUE)",
        )],
    },
    SelfUpdateRow {
        function: "collections::mapValues",
        kind: SelfUpdate::Arm(&[ArmId::MapValues]),
        probes: &[probe_with(
            C,
            NEGATED,
            MSI,
            "Map OF String TO Integer { \"a\" := 1 }",
            "collections::mapValues(x, negated)",
        )],
    },
    // --- letter E: exempt, proven copy-free ---
    SelfUpdateRow {
        function: "collections::reduce",
        kind: SelfUpdate::Exempt {
            reason: REDUCE_REASON,
            proof: REDUCE_PROOF,
        },
        probes: &[probe_with(
            C,
            PUSH,
            LI,
            "[1, 2, 3]",
            "collections::reduce(x, [0], push)",
        )],
    },
    SelfUpdateRow {
        function: "collections::reduceRight",
        kind: SelfUpdate::Exempt {
            reason: REDUCE_REASON,
            proof: REDUCE_PROOF,
        },
        probes: &[probe_with(
            C,
            PUSH,
            LI,
            "[1, 2, 3]",
            "collections::reduceRight(x, [0], push)",
        )],
    },
    SelfUpdateRow {
        function: "compress::deflate",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(Z, LB, BYTES, "compress::deflate(x, 6)")],
    },
    SelfUpdateRow {
        function: "compress::inflate",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(
            Z,
            LB,
            "compress::deflate(encoding::utf8Encode(\"hello\"), 6)",
            "compress::inflate(x, 1048576)",
        )],
    },
    SelfUpdateRow {
        function: "compress::gzipEncode",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(Z, LB, BYTES, "compress::gzipEncode(x, 6)")],
    },
    SelfUpdateRow {
        function: "compress::gzipDecode",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(
            Z,
            LB,
            "compress::gzipEncode(encoding::utf8Encode(\"hello\"), 6)",
            "compress::gzipDecode(x, 1048576, FALSE)",
        )],
    },
    SelfUpdateRow {
        function: "compress::zlibEncode",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(Z, LB, BYTES, "compress::zlibEncode(x, 6)")],
    },
    SelfUpdateRow {
        function: "compress::zlibDecode",
        kind: SelfUpdate::Exempt {
            reason: COMPRESS_REASON,
            proof: COMPRESS_PROOF,
        },
        probes: &[probe(
            Z,
            LB,
            "compress::zlibEncode(encoding::utf8Encode(\"hello\"), 6)",
            "compress::zlibDecode(x, 1048576, FALSE)",
        )],
    },
    SelfUpdateRow {
        function: "crypto::argon2id",
        kind: SelfUpdate::Exempt {
            reason: ARGON_REASON,
            proof: ARGON_PROOF,
        },
        probes: &[probe(
            K,
            LB,
            BYTES,
            "crypto::argon2id(x, encoding::utf8Encode(\"saltsaltsalt\"), 32, 1, 1, 32)",
        )],
    },
    SelfUpdateRow {
        function: "crypto::shake256",
        kind: SelfUpdate::Exempt {
            reason: SHAKE_REASON,
            proof: SHAKE_PROOF,
        },
        probes: &[probe(K, LB, BYTES, "crypto::shake256(x, 32)")],
    },
    // --- plan-146: `String` self-updates (Pending until their letter lands) ---
    SelfUpdateRow {
        function: "toString",
        kind: SelfUpdate::Pending("B"),
        probes: &[str_probe(&[], STR, "toString(x)")],
    },
    SelfUpdateRow {
        function: "strings::left",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(ST, STR, "strings::left(x, 3)")],
    },
    SelfUpdateRow {
        function: "strings::right",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(ST, STR, "strings::right(x, 3)")],
    },
    SelfUpdateRow {
        function: "strings::mid",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(ST, STR, "strings::mid(x, 1, 3)")],
    },
    SelfUpdateRow {
        function: "strings::stripPrefix",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(ST, STR, "strings::stripPrefix(x, \"a\")")],
    },
    SelfUpdateRow {
        function: "strings::stripSuffix",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(ST, STR, "strings::stripSuffix(x, \"f\")")],
    },
    SelfUpdateRow {
        function: "strings::trim",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(ST, STR, "strings::trim(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimStart",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(ST, STR, "strings::trimStart(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimEnd",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(ST, STR, "strings::trimEnd(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimChars",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(ST, STR, "strings::trimChars(x, \"a\")")],
    },
    SelfUpdateRow {
        function: "strings::graphemeAt",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(ST, STR, "strings::graphemeAt(x, 0)")],
    },
    SelfUpdateRow {
        function: "fs::pathBaseName",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(FS, PATH, "fs::pathBaseName(x)")],
    },
    SelfUpdateRow {
        function: "fs::pathDirName",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(FS, PATH, "fs::pathDirName(x)")],
    },
    SelfUpdateRow {
        function: "fs::pathExtension",
        kind: SelfUpdate::Pending("C"),
        probes: &[str_probe(FS, PATH, "fs::pathExtension(x)")],
    },
    SelfUpdateRow {
        function: "strings::padLeft",
        kind: SelfUpdate::Pending("D"),
        probes: &[str_probe(ST, STR, "strings::padLeft(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padRight",
        kind: SelfUpdate::Pending("D"),
        probes: &[str_probe(ST, STR, "strings::padRight(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padLeftToWidth",
        kind: SelfUpdate::Pending("D"),
        probes: &[str_probe(ST, STR, "strings::padLeftToWidth(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padRightToWidth",
        kind: SelfUpdate::Pending("D"),
        probes: &[str_probe(ST, STR, "strings::padRightToWidth(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::repeat",
        kind: SelfUpdate::Pending("D"),
        probes: &[str_probe(ST, STR, "strings::repeat(x, 2)")],
    },
    SelfUpdateRow {
        function: "os::resourcePath",
        kind: SelfUpdate::Pending("D"),
        probes: &[str_probe(OS, STR, "os::resourcePath(x)")],
    },
    SelfUpdateRow {
        function: "strings::upper",
        kind: SelfUpdate::Pending("E"),
        probes: &[str_probe(ST, STR, "strings::upper(x)")],
    },
    SelfUpdateRow {
        function: "strings::lower",
        kind: SelfUpdate::Pending("E"),
        probes: &[str_probe(ST, STR, "strings::lower(x)")],
    },
    SelfUpdateRow {
        function: "strings::caseFold",
        kind: SelfUpdate::Pending("E"),
        probes: &[str_probe(ST, STR, "strings::caseFold(x)")],
    },
    SelfUpdateRow {
        function: "strings::normalizeNfc",
        kind: SelfUpdate::Pending("E"),
        probes: &[str_probe(ST, STR, "strings::normalizeNfc(x)")],
    },
    SelfUpdateRow {
        function: "strings::replace",
        kind: SelfUpdate::Pending("E"),
        probes: &[str_probe(ST, STR, "strings::replace(x, \"a\", \"b\")")],
    },
    SelfUpdateRow {
        function: "fs::pathNormalize",
        kind: SelfUpdate::Pending("E"),
        probes: &[str_probe(FS, PATH, "fs::pathNormalize(x)")],
    },
    SelfUpdateRow {
        function: "encoding::formUrlDecode",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(EN, STR, "encoding::formUrlDecode(x)")],
    },
    SelfUpdateRow {
        function: "encoding::formUrlEncode",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(EN, STR, "encoding::formUrlEncode(x)")],
    },
    SelfUpdateRow {
        function: "encoding::htmlEscape",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(EN, STR, "encoding::htmlEscape(x)")],
    },
    SelfUpdateRow {
        function: "encoding::htmlUnescape",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(EN, STR, "encoding::htmlUnescape(x)")],
    },
    SelfUpdateRow {
        function: "encoding::percentDecode",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(EN, STR, "encoding::percentDecode(x)")],
    },
    SelfUpdateRow {
        function: "encoding::percentEncode",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(EN, STR, "encoding::percentEncode(x)")],
    },
    SelfUpdateRow {
        function: "encoding::punycodeDecode",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(EN, STR, "encoding::punycodeDecode(x)")],
    },
    SelfUpdateRow {
        function: "encoding::punycodeEncode",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(EN, STR, "encoding::punycodeEncode(x)")],
    },
    SelfUpdateRow {
        function: "fs::canonicalPath",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(FS, PATH, "fs::canonicalPath(x)")],
    },
    SelfUpdateRow {
        function: "fs::readText",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(FS, PATH, "fs::readText(x)")],
    },
    SelfUpdateRow {
        function: "io::input",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(&[], STR, "io::input(x)")],
    },
    SelfUpdateRow {
        function: "net::percentDecode",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(NET, STR, "net::percentDecode(x)")],
    },
    SelfUpdateRow {
        function: "os::getEnv",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(OS, STR, "os::getEnv(x)")],
    },
    SelfUpdateRow {
        function: "os::getEnvOr",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(OS, STR, "os::getEnvOr(x, \"x\")")],
    },
    SelfUpdateRow {
        function: "regex::replace",
        kind: SelfUpdate::Pending("F"),
        probes: &[str_probe(RE, STR, "regex::replace(x, \"a\", \"b\")")],
    },
    // --- plan-146-A Open Decision 1: `AttributedString`, deferred ---
    SelfUpdateRow {
        function: "astrings::addAttribute",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AS,
            ATTR,
            ATTR_INIT,
            "astrings::addAttribute(x, 0, 1, astrings::bold())",
        )],
    },
    SelfUpdateRow {
        function: "astrings::clearAttributes",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[
            probe(AS, ATTR, ATTR_INIT, "astrings::clearAttributes(x)"),
            probe(AS, ATTR, ATTR_INIT, "astrings::clearAttributes(x, 0, 1)"),
        ],
    },
    SelfUpdateRow {
        function: "astrings::removeAttribute",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AS,
            ATTR,
            ATTR_INIT,
            "astrings::removeAttribute(x, 0, 1, astrings::bold())",
        )],
    },
    SelfUpdateRow {
        function: "&@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AS,
            ATTR,
            ATTR_INIT,
            "x & astrings::fromString(\"b\")",
        )],
    },
    SelfUpdateRow {
        function: "strings::left@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::left(x, 3)")],
    },
    SelfUpdateRow {
        function: "strings::right@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::right(x, 3)")],
    },
    SelfUpdateRow {
        function: "strings::mid@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::mid(x, 1, 3)")],
    },
    SelfUpdateRow {
        function: "strings::trim@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::trim(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimStart@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::trimStart(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimEnd@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::trimEnd(x)")],
    },
    SelfUpdateRow {
        function: "strings::trimChars@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::trimChars(x, \"a\")")],
    },
    SelfUpdateRow {
        function: "strings::stripPrefix@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AST,
            ATTR,
            ATTR_INIT,
            "strings::stripPrefix(x, \"a\")",
        )],
    },
    SelfUpdateRow {
        function: "strings::stripSuffix@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AST,
            ATTR,
            ATTR_INIT,
            "strings::stripSuffix(x, \"f\")",
        )],
    },
    SelfUpdateRow {
        function: "strings::padLeft@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::padLeft(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padLeftToWidth@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::padLeftToWidth(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padRight@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::padRight(x, 8)")],
    },
    SelfUpdateRow {
        function: "strings::padRightToWidth@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AST,
            ATTR,
            ATTR_INIT,
            "strings::padRightToWidth(x, 8)",
        )],
    },
    SelfUpdateRow {
        function: "strings::repeat@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::repeat(x, 2)")],
    },
    SelfUpdateRow {
        function: "strings::replace@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(
            AST,
            ATTR,
            ATTR_INIT,
            "strings::replace(x, \"a\", \"b\")",
        )],
    },
    SelfUpdateRow {
        function: "strings::upper@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::upper(x)")],
    },
    SelfUpdateRow {
        function: "strings::lower@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::lower(x)")],
    },
    SelfUpdateRow {
        function: "strings::caseFold@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::caseFold(x)")],
    },
    SelfUpdateRow {
        function: "strings::normalizeNfc@AttributedString",
        kind: SelfUpdate::Deferred("attributed-string"),
        probes: &[probe(AST, ATTR, ATTR_INIT, "strings::normalizeNfc(x)")],
    },
];

#[cfg(test)]
impl ArmId {
    /// The stack-slot type names only this arm allocates (plan-141 Appendix C:
    /// each occurs exactly once in `src/`). The slot's presence in a function
    /// proves the arm fired there. `Set` has one per collection kind.
    ///
    /// plan-145-B: a field-capable arm also lists its field route's slots, one per
    /// container (`inplace_recfield_*` for a record, `inplace_state_*` for a
    /// `STATE` payload). The single and bulk `append` share their field slot, as
    /// `insert` and `prepend` share their item slot: the record and `STATE` arms
    /// they absorbed did, and the names are in the `.ncode` (byte identity).
    pub(crate) fn markers(self) -> &'static [&'static str] {
        match self {
            ArmId::Append => &[
                "inplace_append_item",
                "inplace_recfield_rhs",
                "inline_state_rhs",
            ],
            ArmId::BulkAppend => &[
                "inplace_bulk_append_rhs",
                "inplace_recfield_rhs",
                "inline_state_rhs",
            ],
            ArmId::SetAdd => &[
                "inplace_set_add_item",
                "inplace_recfield_add_item",
                "inplace_state_add_item",
            ],
            ArmId::Set => &[
                "inplace_set_index",
                "inplace_set_key",
                "inplace_recfield_set_index",
                "inplace_recfield_set_key",
                "inplace_state_set_index",
                "inplace_state_set_key",
            ],
            ArmId::RemoveKey => &[
                "inplace_remove_key",
                "inplace_recfield_remove_key",
                "inplace_state_remove_key",
            ],
            ArmId::Prepend => &[
                "inplace_prepend_item",
                "inplace_recfield_splice_item",
                "inplace_state_splice_item",
            ],
            ArmId::RemoveAt => &[
                "inplace_remove_at_index",
                "inplace_recfield_remove_at_index",
                "inplace_state_remove_at_index",
            ],
            ArmId::Insert => &[
                "inplace_insert_index",
                "inplace_recfield_splice_index",
                "inplace_state_splice_index",
            ],
            ArmId::SetRemove => &[
                "inplace_set_remove_item",
                "inplace_recfield_set_remove",
                "inplace_state_set_remove",
            ],
            ArmId::Concat => &["concat_self_right"],
            ArmId::Filter => &["inplace_filter_action"],
            ArmId::Take => &["inplace_take_count"],
            ArmId::Drop => &["inplace_drop_count"],
            ArmId::Mid => &["inplace_mid_start"],
            ArmId::Distinct => &["inplace_distinct_count"],
            ArmId::Math => &["inplace_math_result"],
            ArmId::Replace => &["inplace_replace_old"],
            ArmId::Transform => &["inplace_transform_action"],
            ArmId::Sort => &["inplace_sort_count"],
            ArmId::SortBy => &["inplace_sortby_action"],
            ArmId::Union => &["inplace_union_other"],
            ArmId::Intersection => &["inplace_intersection_other"],
            ArmId::Difference => &["inplace_difference_other"],
            ArmId::SymmetricDifference => &["inplace_symmetricDifference_other"],
            ArmId::Merge => &["inplace_merge_prefer"],
            ArmId::MapValues => &["inplace_mapvalues_action"],
        }
    }
}

/// plan-145-A: how a record field of a given type is laid out, which decides the
/// only in-place lowering a self-update of that field can have.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FieldKindClass {
    /// Stored by value in its 8-byte slot (`!record_field_is_inlined &&
    /// !record_field_is_pointer`): the scalars, an enum, `Nothing`, a resource
    /// handle.
    Scalar,
    /// The slot holds an owned pointer to its own block (`record_field_is_pointer`
    /// and not inlined): `json::Json` and the records that hold one.
    Pointer,
    /// Inlined into the owner's data region, with a size fixed at compile time:
    /// every field is `Scalar` or itself `InlinedFixed`.
    InlinedFixed,
    /// Inlined, but its size depends on its value: it holds a `String`, a
    /// collection, a data union, or an `InlinedVariable` record.
    InlinedVariable,
    /// A `List`, `Map` or `Set` — `cases.tsv`'s arms.
    Collection,
}

/// Classify `type_` as a record field (plan-145-A census). `model` must know the
/// record types `type_` reaches.
#[cfg(test)]
pub(crate) fn field_kind_class(model: &TypeModel, type_: &ParameterType) -> FieldKindClass {
    use crate::codegen::collection::layout::{record_field_is_inlined, record_field_is_pointer};
    if matches!(
        type_,
        ParameterType::ListOf(_) | ParameterType::MapOf(..) | ParameterType::SetOf(_)
    ) {
        return FieldKindClass::Collection;
    }
    if !record_field_is_inlined(model, type_) {
        return if record_field_is_pointer(model, type_) {
            FieldKindClass::Pointer
        } else {
            FieldKindClass::Scalar
        };
    }
    match model.record_fields.get(type_) {
        Some(fields)
            if fields.iter().all(|(_, field)| {
                matches!(
                    field_kind_class(model, field),
                    FieldKindClass::Scalar | FieldKindClass::InlinedFixed
                )
            }) =>
        {
            FieldKindClass::InlinedFixed
        }
        _ => FieldKindClass::InlinedVariable,
    }
}

/// plan-145-A: a field kind's in-place status (`FIELD_KIND_TABLE`).
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum FieldKindRow {
    /// Updated in place; the plan-145 letter that lands it (`C` scalar and pointer
    /// stores, `F` fixed-size overwrites, `E` the last collection arms).
    Arm(char),
    /// Rebuilt by design; the proof says why no in-place form exists.
    Rebuild {
        reason: &'static str,
        proof: &'static str,
    },
    /// Owned by another plan.
    Deferred(&'static str),
}

/// plan-145-A Open Decision 3: an inlined field whose size depends on its value.
#[cfg(test)]
const SIZE_VARIES: FieldKindRow = FieldKindRow::Rebuild {
    reason: "size-varies",
    proof: "the new value's size is known only after it is built, and an inlined field \
            without a capacity word cannot take a larger value in place; reallocating the \
            owner's tail instead copies its prefix, which costs what the rebuild does",
};

/// plan-145-A Open Decision 1: a `String` field (and an `AttributedString`, whose
/// text is one) needs a `String` arm at a field, which neither plan-145 nor
/// plan-146 lands; the follow-up plan written after both owns it (plan-146-A Open
/// Decision 1). `field_kinds.tsv` and `field_expect.tsv` spell it `deferred:string`.
#[cfg(test)]
const STRING_PLAN: FieldKindRow = FieldKindRow::Deferred("string");

/// plan-145-A: one row per field kind a record can hold (`field_kind_census`
/// enumerates them from the compiler). A new package record type fails
/// `field_kind_census_covers_every_record_field_type` until it has a row, and so
/// does a row whose kind no longer exists. The black-box twin is
/// `tests/guards/inplace_self_update_census.rs`, over `field_kinds.tsv`.
#[cfg(test)]
pub(crate) const FIELD_KIND_TABLE: &[(&str, FieldKindRow)] = &[
    ("Integer", FieldKindRow::Arm('C')),
    ("Float", FieldKindRow::Arm('C')),
    ("Fixed", FieldKindRow::Arm('C')),
    ("Money", FieldKindRow::Arm('C')),
    ("Boolean", FieldKindRow::Arm('C')),
    ("Byte", FieldKindRow::Arm('C')),
    ("String", STRING_PLAN),
    ("AttributedString", STRING_PLAN),
    ("json.Json", FieldKindRow::Arm('C')),
    ("List OF Integer", FieldKindRow::Arm('E')),
    ("Map OF String TO Integer", FieldKindRow::Arm('E')),
    ("Set OF Integer", FieldKindRow::Arm('E')),
    ("astrings.AttrFlag", FieldKindRow::Arm('F')),
    ("astrings.AttrText", SIZE_VARIES),
    ("astrings.AttrNumber", FieldKindRow::Arm('F')),
    ("audio.AudioDevice", SIZE_VARIES),
    ("audio.AudioEnvelope", FieldKindRow::Arm('F')),
    ("audio.AudioNote", FieldKindRow::Arm('F')),
    ("big.Int", SIZE_VARIES),
    ("big.DivResult", SIZE_VARIES),
    ("canvas.Point", FieldKindRow::Arm('F')),
    ("canvas.MouseEvent", FieldKindRow::Arm('F')),
    ("canvas.Size", FieldKindRow::Arm('F')),
    ("canvas.Bounds", FieldKindRow::Arm('F')),
    ("canvas.TextMetrics", FieldKindRow::Arm('F')),
    ("canvas.Transform", FieldKindRow::Arm('F')),
    ("canvas.GradientStop", FieldKindRow::Arm('F')),
    ("canvas.Gradient", SIZE_VARIES),
    ("canvas.Paint", SIZE_VARIES),
    ("canvas.Rectangle", SIZE_VARIES),
    ("canvas.RoundedRect", SIZE_VARIES),
    ("canvas.Line", SIZE_VARIES),
    ("canvas.Polygon", SIZE_VARIES),
    ("canvas.Circle", SIZE_VARIES),
    ("canvas.Arc", SIZE_VARIES),
    ("canvas.Text", SIZE_VARIES),
    ("canvas.Picture", SIZE_VARIES),
    ("canvas.Group", SIZE_VARIES),
    ("canvas.Ellipse", SIZE_VARIES),
    ("canvas.DrawLayer", SIZE_VARIES),
    ("csv.CsvReader", SIZE_VARIES),
    ("csv.CsvRow", SIZE_VARIES),
    ("json.JsonNull", FieldKindRow::Arm('F')),
    ("json.JsonBool", FieldKindRow::Arm('F')),
    ("json.JsonNum", FieldKindRow::Arm('F')),
    ("json.JsonStr", SIZE_VARIES),
    ("json.JsonArr", FieldKindRow::Arm('C')),
    ("json.JsonObj", FieldKindRow::Arm('C')),
    ("regex.Group", SIZE_VARIES),
    ("regex.MatchInfo", SIZE_VARIES),
    ("term.TermSize", FieldKindRow::Arm('F')),
    ("term.MouseEvent", FieldKindRow::Arm('F')),
    ("datetime.Instant", FieldKindRow::Arm('F')),
    ("datetime.Duration", FieldKindRow::Arm('F')),
    ("datetime.Date", FieldKindRow::Arm('F')),
    ("datetime.Time", FieldKindRow::Arm('F')),
    ("datetime.Zone", SIZE_VARIES),
    ("datetime.DateTime", SIZE_VARIES),
    ("crypto.Sealed", SIZE_VARIES),
    ("crypto.KeyPair", SIZE_VARIES),
    ("udp.Datagram", SIZE_VARIES),
    ("vector.Float2", FieldKindRow::Arm('F')),
    ("vector.Float3", FieldKindRow::Arm('F')),
    ("vector.Float4", FieldKindRow::Arm('F')),
    ("vector.Fixed2", FieldKindRow::Arm('F')),
    ("vector.Fixed3", FieldKindRow::Arm('F')),
    ("vector.Fixed4", FieldKindRow::Arm('F')),
    ("vector.Integer2", FieldKindRow::Arm('F')),
    ("vector.Integer3", FieldKindRow::Arm('F')),
    ("vector.Integer4", FieldKindRow::Arm('F')),
    ("http.Response", SIZE_VARIES),
    ("http.PendingState", SIZE_VARIES),
    ("http.Request", SIZE_VARIES),
    ("http.RequestPart", SIZE_VARIES),
    ("http.Route", SIZE_VARIES),
    ("net.Url", SIZE_VARIES),
    ("net.Address", SIZE_VARIES),
    ("net.PingResult", SIZE_VARIES),
    ("color.Color", FieldKindRow::Arm('F')),
    ("color.Hsl", FieldKindRow::Arm('F')),
];

/// The binding sites the matrix test compiles every arm probe at: plan-142's four
/// plain sites, and plan-145's fifteen field sites (plan-144's audit legend).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Site {
    /// S1 — a `MUT` local in a function body.
    Local,
    /// S7 — a `MUT` local inside a `FOR EACH` over itself (plan-142-F). A `String`
    /// is not a collection, so no `FOR EACH` walks one: the `&` row has no S7.
    ForEach,
    /// S9 — a `MUT` captured by reference in a `collections::forEach` lambda
    /// (plan-142-G). The self-update lowers in the lifted lambda. A by-ref
    /// `String` has no capacity shadow to append into (Correction G1), so the `&`
    /// row has no S9.
    Lambda,
    /// S2 — a module-level `MUT` global (plan-142-H), self-updated in a `SUB`.
    Global,
    /// A local record's not-last field: `r = WITH r { a := f(r.a, …) }`.
    S3,
    /// The same record's last field `b`.
    S4,
    /// The last field of a module-level record, updated in a `SUB`.
    S5,
    /// `b` of `inner AS Rec` in `Out { n, inner }`, through a nested `WITH`.
    S6,
    /// S4 inside a `FOR EACH` over the field.
    S7,
    /// S4 in a `collections::forEach` lambda capturing the record.
    S9,
    /// S4 plus a scalar update in the same `WITH` (`RecN { a, b, n }`).
    S10,
    /// Field `a` of an owner handle's `STATE` payload `P { a, b, n }`.
    T1,
    /// Field `b` of the same payload.
    T2,
    /// T1 through a `RES` parameter (`SUB run1(RES h …)`).
    T3,
    /// T2 through a `RES` parameter.
    T4,
    /// Two updates over the whole payload (`h.state = WITH h.state { b := …, n := … }`).
    T5,
    /// `b` of `inner AS PIn` in `Q { inner, n }`.
    T6,
    /// T2 inside a `FOR EACH` over the field.
    T7,
    /// T2 on a resource-union handle `RES h AS Stream STATE P`.
    T8,
}

#[cfg(test)]
pub(crate) const ENABLED_SITES: &[Site] = &[Site::Local, Site::ForEach, Site::Lambda, Site::Global];

/// plan-145-A: the field sites the matrix compiles every arm probe at.
#[cfg(test)]
pub(crate) const FIELD_SITES: &[Site] = &[
    Site::S3,
    Site::S4,
    Site::S5,
    Site::S6,
    Site::S7,
    Site::S9,
    Site::S10,
    Site::T1,
    Site::T2,
    Site::T3,
    Site::T4,
    Site::T5,
    Site::T6,
    Site::T7,
    Site::T8,
];

/// plan-145-A: `(arm, probe type or "" for every probe, field sites, reason)` — the
/// probes an arm never fires for at those sites, by design. The matrix asserts
/// they do not fire, and does not require them to.
#[cfg(test)]
pub(crate) const FIELD_NEVER: &[(ArmId, &str, &[&str], &str)] = {
    const NOT_LAST: &[&str] = &["S3", "T1", "T3"];
    const NOT_LAST_GROW: &str = "a grow at a not-last field would shift the next sibling \
                                 (plan-145-E non-goal)";
    &[
        (ArmId::Append, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::BulkAppend, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::SetAdd, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::Insert, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::Prepend, "", NOT_LAST, NOT_LAST_GROW),
        (
            ArmId::Set,
            "Map OF String TO Integer",
            NOT_LAST,
            NOT_LAST_GROW,
        ),
        (ArmId::Union, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::SymmetricDifference, "", NOT_LAST, NOT_LAST_GROW),
        (ArmId::Merge, "", NOT_LAST, NOT_LAST_GROW),
        (
            ArmId::Concat,
            "",
            &[
                "S3", "S4", "S5", "S6", "S7", "S9", "S10", "T1", "T2", "T3", "T4", "T5", "T6",
                "T7", "T8",
            ],
            "a `String` field is deferred:string (plan-145-A Open Decision 1)",
        ),
    ]
};

#[cfg(test)]
impl Site {
    /// Whether the self-update at this site lowers in the function `name`.
    pub(crate) fn lowers_in(self, name: &str) -> bool {
        match self {
            Site::Local | Site::ForEach => name == "main",
            Site::Lambda | Site::S9 => name.starts_with("$lambda"),
            Site::Global | Site::S5 | Site::T3 | Site::T4 => name == "run1",
            _ => name == "main",
        }
    }

    /// The site's code in `FIELD_NEVER` and the harness tables (`S4`, `T2`, …).
    pub(crate) fn code(self) -> String {
        format!("{self:?}")
    }

    /// The field a field site updates.
    fn field(self) -> &'static str {
        match self {
            Site::S3 => "r.a",
            Site::S5 => "gR.b",
            Site::S6 => "o.inner.b",
            Site::T1 | Site::T3 => "h.state.a",
            Site::T2 | Site::T4 | Site::T5 | Site::T7 | Site::T8 => "h.state.b",
            Site::T6 => "h.state.inner.b",
            _ => "r.b",
        }
    }
}

/// `text` with every whole-word `from` replaced by `to` (outside string literals).
#[cfg(test)]
fn replace_word(text: &str, from: &str, to: &str) -> String {
    let bytes = text.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut i = 0;
    while i < text.len() {
        if bytes[i] == b'"' {
            in_string = !in_string;
        }
        if !in_string
            && text[i..].starts_with(from)
            && (i == 0 || !word(bytes[i - 1]))
            && bytes.get(i + from.len()).is_none_or(|b| !word(*b))
        {
            out.push_str(to);
            i += from.len();
            continue;
        }
        let c = text[i..].chars().next().expect("in bounds");
        out.push(c);
        i += c.len_utf8();
    }
    out
}

#[cfg(test)]
impl Probe {
    /// The whole program performing this probe's self-update at `site`, in `main`;
    /// `None` when `site` has no form for the probe's type.
    pub(crate) fn source(&self, site: Site) -> Option<String> {
        if FIELD_SITES.contains(&site) {
            return self.field_source(site);
        }
        let mut src = String::from("IMPORT io\n");
        for import in self.imports {
            src.push_str(&format!("IMPORT {import}\n"));
        }
        // S9 calls `collections::forEach`.
        if matches!(site, Site::Lambda) && !self.imports.contains(&"collections") {
            src.push_str("IMPORT collections\n");
        }
        src.push('\n');
        src.push_str(self.helpers);
        match site {
            Site::Local => src.push_str(&format!(
                "FUNC main() AS Integer\n  MUT x AS {ty} = {init}\n  FOR i = 1 TO 3\n    \
                 x = {call}\n  NEXT\n  io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n",
                ty = self.ty,
                init = self.init,
                call = self.call,
            )),
            Site::ForEach | Site::Lambda if self.ty == "String" => return None,
            Site::ForEach => src.push_str(&format!(
                "FUNC main() AS Integer\n  MUT x AS {ty} = {init}\n  FOR EACH each1 IN x\n    \
                 x = {call}\n  NEXT\n  io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n",
                ty = self.ty,
                init = self.init,
                call = self.call,
            )),
            Site::Global => src.push_str(&format!(
                "MUT x AS {ty} = {init}\n\nSUB run1()\n  FOR i = 1 TO 3\n    x = {call}\n  \
                 NEXT\nEND SUB\n\nFUNC main() AS Integer\n  run1()\n  \
                 io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n",
                ty = self.ty,
                init = self.init,
                call = self.call,
            )),
            Site::Lambda => src.push_str(&format!(
                "FUNC main() AS Integer\n  MUT x AS {ty} = {init}\n  \
                 LET one AS List OF Integer = [0]\n  FOR i = 1 TO 3\n    \
                 collections::forEach(one, LAMBDA(each1 AS Integer) -> x = {call})\n  \
                 NEXT\n  io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n",
                ty = self.ty,
                init = self.init,
                call = self.call,
            )),
            _ => unreachable!("field sites return above"),
        }
        Some(src)
    }

    /// plan-145-A: the probe's self-update applied to a field at `site`: the same
    /// templates as the runtime harness (`tests/runtime/rt_inplace_self_update.rs`).
    fn field_source(&self, site: Site) -> Option<String> {
        let ty = self.ty;
        if ty == "String" && matches!(site, Site::S7 | Site::T7) {
            return None;
        }
        let field = site.field();
        let v = replace_word(self.call, "x", field);
        let statement = match site {
            Site::S3 => format!("r = WITH r {{ a := {v} }}"),
            Site::S5 => format!("gR = WITH gR {{ b := {v} }}"),
            Site::S6 => format!("o = WITH o {{ inner := WITH o.inner {{ b := {v} }} }}"),
            Site::S10 => format!("r = WITH r {{ b := {v}, n := k }}"),
            Site::T5 => format!("h.state = WITH h.state {{ b := {v}, n := k }}"),
            Site::T6 => format!("h.state.inner = WITH h.state.inner {{ b := {v} }}"),
            Site::T1 | Site::T2 | Site::T3 | Site::T4 | Site::T7 | Site::T8 => {
                format!("{field} = {v}")
            }
            _ => format!("r = WITH r {{ b := {v} }}"),
        };
        let owner = match site {
            Site::S10 => "  MUT r AS RecN = RecN[a := x, b := x, n := 0]\n",
            Site::S6 => "  MUT o AS Out = Out[n := 1, inner := Rec[a := x, b := x]]\n",
            Site::S5 => "  gR = Rec[a := x, b := x]\n",
            Site::T6 => "  h.state = Q[inner := PIn[a := x, b := x], n := 0]\n",
            Site::T1 | Site::T2 | Site::T3 | Site::T4 | Site::T5 | Site::T7 | Site::T8 => {
                "  h.state = P[a := x, b := x, n := 0]\n"
            }
            _ => "  MUT r AS Rec = Rec[a := x, b := x]\n",
        };
        let looped = match site {
            Site::S7 | Site::T7 => {
                format!("  FOR EACH each1 IN {field}\n    {statement}\n  NEXT\n")
            }
            Site::S9 => format!(
                "  LET one AS List OF Integer = [0]\n  FOR i = 1 TO 3\n    \
                 collections::forEach(one, LAMBDA(each1 AS Integer) -> {statement})\n  NEXT\n"
            ),
            _ => format!("  FOR i = 1 TO 3\n    {statement}\n  NEXT\n"),
        };
        // S10/T5's second update is `n := k`, a local read (plan-144's site legend).
        let k = if matches!(site, Site::S10 | Site::T5) {
            "  LET k AS Integer = 7\n"
        } else {
            ""
        };
        let body = format!(
            "  MUT x AS {ty} = {init}\n{owner}{k}{looped}  io::print(toString(len({field})))\n",
            init = self.init
        );
        let open = "fs::openFile(\"/dev/null\")";
        let program = match site {
            Site::S5 => format!(
                "MUT gR AS Rec\n\nSUB run1()\n{body}END SUB\n\n\
                 FUNC main() AS Integer\n  run1()\n  RETURN 0\nEND FUNC\n"
            ),
            Site::T3 | Site::T4 => format!(
                "SUB run1(RES h AS fs::File STATE P)\n{body}END SUB\n\n\
                 FUNC main() AS Integer\n  RES h AS fs::File STATE P = {open}\n  run1(h)\n  \
                 RETURN 0\nEND FUNC\n"
            ),
            Site::T6 => format!(
                "FUNC main() AS Integer\n  RES h AS fs::File STATE Q = {open}\n{body}  \
                 RETURN 0\nEND FUNC\n"
            ),
            Site::T8 => format!(
                "FUNC main() AS Integer\n  RES h AS Stream STATE P = {open}\n{body}  \
                 RETURN 0\nEND FUNC\n"
            ),
            Site::T1 | Site::T2 | Site::T5 | Site::T7 => format!(
                "FUNC main() AS Integer\n  RES h AS fs::File STATE P = {open}\n{body}  \
                 RETURN 0\nEND FUNC\n"
            ),
            _ => format!("FUNC main() AS Integer\n{body}  RETURN 0\nEND FUNC\n"),
        };
        let mut src = String::from("IMPORT io\n");
        let mut imports: Vec<&str> = self.imports.to_vec();
        imports.push("fs");
        imports.push("collections");
        if site == Site::T8 {
            imports.push("tcp");
        }
        let mut seen = Vec::new();
        for import in imports {
            if !seen.contains(&import) {
                seen.push(import);
                src.push_str(&format!("IMPORT {import}\n"));
            }
        }
        src.push('\n');
        src.push_str(self.helpers);
        src.push_str(&format!(
            "TYPE Rec\n  a AS {ty}\n  b AS {ty}\nEND TYPE\n\n\
             TYPE RecN\n  a AS {ty}\n  b AS {ty}\n  n AS Integer\nEND TYPE\n\n\
             TYPE Out\n  n AS Integer\n  inner AS Rec\nEND TYPE\n\n\
             TYPE P\n  a AS {ty}\n  b AS {ty}\n  n AS Integer\nEND TYPE\n\n\
             TYPE PIn\n  a AS {ty}\n  b AS {ty}\nEND TYPE\n\n\
             TYPE Q\n  inner AS PIn\n  n AS Integer\nEND TYPE\n\n"
        ));
        if site == Site::T8 {
            src.push_str("UNION Stream\n  fs::File\n  tcp::Socket\nEND UNION\n\n");
        }
        src.push_str(&program);
        Some(src)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::codegen::registry::{registry, self_update_shaped};
    use crate::target::NativeBuildMode::Console;
    use crate::testutil::{code_for_src_cached, CodeTarget};

    /// Rows with no registry function behind them: the operator self-updates, the
    /// unqualified `toString` (plan-146-A), and `&` on two `AttributedString`s,
    /// which reaches NIR as a call to `#astrings_concat` (plan-143 findings F5).
    const NON_REGISTRY: &[&str] = &["&", "toString", "&@AttributedString"];

    /// The `@AttributedString` row spelling of a Tier-B transform: the
    /// `AttributedString` "overload" of a `strings::` member, typed by
    /// `strings::resolve_return_type` rather than registered (plan-143 Correction 1).
    fn tier_b_row(member: &str) -> String {
        format!("{}@AttributedString", member.replacen('.', "::", 1))
    }

    /// Every function with at least one self-update-shaped overload, as `pkg::name`.
    /// An `internal_only` member is outside it: user source can never call it
    /// (`astrings::writeSpans`, plan-146-A Correction A1).
    fn shaped_functions() -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for package in registry().packages() {
            for function in package.functions() {
                if !function.internal_only
                    && function.implementations.iter().any(self_update_shaped)
                {
                    out.insert(format!("{}::{}", package.import_name(), function.name));
                }
            }
        }
        out
    }

    /// plan-145-A: every field kind a record can hold, classified by the compiler —
    /// `(kind, class)`, the kind spelled as its type renders (`vector.Float3`).
    ///
    /// The population is read off a program that imports every importable package
    /// (in app mode: `canvas` requires it) and declares one record holding each
    /// builtin kind: every EXPORTed package record type, the builtin scalars,
    /// `String`, `AttributedString`, `json::Json` and the three collections.
    fn field_kind_census() -> Vec<(String, FieldKindClass)> {
        let mut src = String::new();
        for package in registry().packages() {
            if !package.is_unqualified_global() {
                src.push_str(&format!("IMPORT {}\n", package.import_name()));
            }
        }
        src.push_str(
            "\nTYPE FieldKinds\n  i AS Integer\n  f AS Float\n  x AS Fixed\n  m AS Money\n  \
             b AS Boolean\n  y AS Byte\n  s AS String\n  t AS AttributedString\n  \
             j AS json::Json\n  l AS List OF Integer\n  p AS Map OF String TO Integer\n  \
             e AS Set OF Integer\nEND TYPE\n\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        );
        let target = CodeTarget::MacosAarch64;
        let mode = target.app_mode().expect("macOS has an app mode");
        let nir = crate::testutil::nir_for_src(&src, target, mode).expect("the census lowers");
        let model = TypeModel::from_module(&nir).expect("the census model builds");
        let mut kinds = Vec::new();
        for t in &nir.types {
            if t.name == "FieldKinds" {
                for field in &t.fields {
                    kinds.push((
                        field.type_.name().into_owned(),
                        field_kind_class(&model, &field.type_),
                    ));
                }
            } else if t.kind == "type" && t.visibility == "export" {
                let ty = ParameterType::named(&t.name);
                kinds.push((t.name.clone(), field_kind_class(&model, &ty)));
            }
        }
        kinds
    }

    #[test]
    fn field_kind_census_covers_every_record_field_type() {
        let census = field_kind_census();
        let mut failures = Vec::new();
        for (kind, class) in &census {
            let Some((_, row)) = FIELD_KIND_TABLE.iter().find(|(k, _)| k == kind) else {
                failures.push(format!(
                    "{kind} ({class:?}) has no FIELD_KIND_TABLE row — classify it: `Arm(letter)`, \
                     `Rebuild {{ reason, proof }}` or `Deferred(plan)`"
                ));
                continue;
            };
            let consistent = match row {
                FieldKindRow::Arm(letter) => {
                    "BCDEFGH".contains(*letter)
                        && matches!(
                            class,
                            FieldKindClass::Scalar
                                | FieldKindClass::Pointer
                                | FieldKindClass::InlinedFixed
                                | FieldKindClass::Collection
                        )
                }
                FieldKindRow::Rebuild { reason, proof } => {
                    *class == FieldKindClass::InlinedVariable
                        && !reason.trim().is_empty()
                        && !proof.trim().is_empty()
                }
                FieldKindRow::Deferred(plan) => !plan.trim().is_empty(),
            };
            if !consistent {
                failures.push(format!(
                    "{kind}: row {row:?} does not fit its class {class:?}"
                ));
            }
        }
        for (kind, _) in FIELD_KIND_TABLE {
            if !census.iter().any(|(k, _)| k == kind) {
                failures.push(format!("FIELD_KIND_TABLE row {kind} names no field kind"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// plan-142-B Phase 1: every spelling a shrink self-update's call target has
    /// after lowering (recorded from a `--nir` build in plan-142-B) resolves to its
    /// bare name, and nothing else does.
    #[test]
    fn self_update_builtin_names_every_spelling() {
        // `Body::abi_inline` / `Body::Intrinsic` members keep the qualified name.
        assert_eq!(self_update_builtin("collections.filter"), Some("filter"));
        assert_eq!(self_update_builtin("collections.mid"), Some("mid"));
        assert_eq!(self_update_builtin("collections.append"), Some("append"));
        // `Body::Mfb` members arrive as their internalized monomorph.
        assert_eq!(
            self_update_builtin("#collections_take$Integer"),
            Some("take")
        );
        assert_eq!(
            self_update_builtin("#collections_take$String"),
            Some("take")
        );
        assert_eq!(
            self_update_builtin("#collections_drop$Integer"),
            Some("drop")
        );
        assert_eq!(
            self_update_builtin("#collections_distinct$Integer"),
            Some("distinct")
        );
        // … or, before monomorphization, qualified.
        assert_eq!(self_update_builtin("collections.take"), Some("take"));
        // plan-146-B: every spelling of a `String` row plan-146 arms (C, D, E) and
        // of `toString`, from the `--nir` probe (plan-146-B Phase 1).
        for (target, bare) in [
            ("strings.left", "left"),
            ("strings.right", "right"),
            ("strings.mid", "mid"),
            ("strings.stripPrefix", "stripPrefix"),
            ("strings.stripSuffix", "stripSuffix"),
            ("strings.trim", "trim"),
            ("strings.trimStart", "trimStart"),
            ("strings.trimEnd", "trimEnd"),
            ("strings.trimChars", "trimChars"),
            ("strings.graphemeAt", "graphemeAt"),
            ("fs.pathBaseName", "pathBaseName"),
            ("fs.pathDirName", "pathDirName"),
            ("fs.pathExtension", "pathExtension"),
            ("strings.padLeft", "padLeft"),
            ("strings.padRight", "padRight"),
            ("#strings_padLeftToWidth", "padLeftToWidth"),
            ("#strings_padRightToWidth", "padRightToWidth"),
            ("strings.repeat", "repeat"),
            ("os.resourcePath", "resourcePath"),
            ("strings.upper", "upper"),
            ("strings.lower", "lower"),
            ("strings.caseFold", "caseFold"),
            ("strings.normalizeNfc", "normalizeNfc"),
            ("strings.replace", "replace"),
            ("fs.pathNormalize", "pathNormalize"),
            ("toString", "toString"),
        ] {
            assert_eq!(self_update_builtin(target), Some(bare), "{target}");
        }
        assert_eq!(self_update_builtin("#strings_nope"), None);
        // A Rewrite helper that is not a row stays invisible.
        assert_eq!(self_update_builtin("#strings_padToWidthCopies"), None);
        // Not a collections builtin.
        assert_eq!(self_update_builtin("#collections_nope$Integer"), None);
        assert_eq!(self_update_builtin("#json_parse"), None);
        assert_eq!(self_update_builtin("take"), None);
        assert_eq!(self_update_builtin("userFunction$Integer"), None);
    }

    #[test]
    fn self_update_census_covers_every_registry_overload() {
        let rows: BTreeSet<&str> = SELF_UPDATE_TABLE.iter().map(|r| r.function).collect();
        let missing: Vec<String> = shaped_functions()
            .into_iter()
            .filter(|f| !rows.contains(f.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "self-update-shaped builtin(s) with no SELF_UPDATE_TABLE row — add an `Arm` \
             (an in-place lowering) or an `Exempt` with its proof: {missing:?}"
        );
    }

    /// plan-146-A: the Tier-B `AttributedString` transforms are a second census
    /// source — `mfb man` and the registry cannot see them.
    #[test]
    fn tier_b_transforms_have_rows() {
        let rows: BTreeSet<&str> = SELF_UPDATE_TABLE.iter().map(|r| r.function).collect();
        let missing: Vec<String> = crate::codegen::builtins::strings::tier_b_transforms()
            .iter()
            .map(|(member, _)| tier_b_row(member))
            .filter(|row| !rows.contains(row.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "Tier-B AttributedString transform(s) with no SELF_UPDATE_TABLE row: {missing:?}"
        );
    }

    #[test]
    fn self_update_table_has_no_stale_rows() {
        let shaped = shaped_functions();
        let tier_b: BTreeSet<String> = crate::codegen::builtins::strings::tier_b_transforms()
            .iter()
            .map(|(member, _)| tier_b_row(member))
            .collect();
        let arms: BTreeSet<ArmId> = SELF_UPDATE_ARMS.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(
            arms.len(),
            SELF_UPDATE_ARMS.len(),
            "an arm id appears twice in SELF_UPDATE_ARMS"
        );
        let mut seen = BTreeSet::new();
        let mut referenced = BTreeSet::new();
        for row in SELF_UPDATE_TABLE {
            assert!(seen.insert(row.function), "duplicate row {}", row.function);
            assert!(
                shaped.contains(row.function)
                    || NON_REGISTRY.contains(&row.function)
                    || tier_b.contains(row.function),
                "row {} names no registry function with a self-update-shaped overload",
                row.function
            );
            assert!(!row.probes.is_empty(), "row {} has no probe", row.function);
            if let SelfUpdate::Exempt { reason, proof } = row.kind {
                assert!(
                    !reason.trim().is_empty() && !proof.trim().is_empty(),
                    "row {} is Exempt without a reason and a proof",
                    row.function
                );
            }
            if let SelfUpdate::Pending(owner) | SelfUpdate::Deferred(owner) = row.kind {
                assert!(
                    !owner.trim().is_empty(),
                    "row {} is {:?} without naming who lands it",
                    row.function,
                    row.kind
                );
            }
            if let SelfUpdate::Arm(ids) = row.kind {
                assert!(!ids.is_empty(), "row {} lists no arm", row.function);
                for id in ids {
                    assert!(
                        arms.contains(id),
                        "row {} names {id:?}, which is not in SELF_UPDATE_ARMS",
                        row.function
                    );
                    referenced.insert(*id);
                }
            }
        }
        let unreferenced: Vec<&ArmId> = arms.difference(&referenced).collect();
        assert!(
            unreferenced.is_empty(),
            "SELF_UPDATE_ARMS entries no row names: {unreferenced:?}"
        );
    }

    #[test]
    fn every_arm_row_fires_at_every_enabled_site() {
        let arms: BTreeSet<ArmId> = SELF_UPDATE_ARMS.iter().map(|(id, _, _)| *id).collect();
        let never = |id: ArmId, probe: &Probe, site: Site| {
            FIELD_NEVER.iter().any(|(arm, ty, sites, _)| {
                *arm == id
                    && (ty.is_empty() || *ty == probe.ty)
                    && sites.contains(&site.code().as_str())
            })
        };
        let mut failures = Vec::new();
        for row in SELF_UPDATE_TABLE {
            let SelfUpdate::Arm(ids) = row.kind else {
                continue;
            };
            for &site in ENABLED_SITES.iter().chain(FIELD_SITES) {
                let field = FIELD_SITES.contains(&site);
                let mut fired = BTreeSet::new();
                // The ids some compiled probe is expected to be able to fire.
                let mut reachable = BTreeSet::new();
                for probe in row.probes {
                    let Some(src) = probe.source(site) else {
                        continue;
                    };
                    let code = code_for_src_cached(&src, CodeTarget::LinuxX86_64, Console);
                    let hit: Vec<ArmId> = ids
                        .iter()
                        .copied()
                        .filter(|id| {
                            code.functions
                                .iter()
                                .filter(|function| site.lowers_in(&function.name))
                                .flat_map(|function| &function.stack_slots)
                                .any(|slot| id.markers().contains(&slot.type_.as_str()))
                        })
                        .collect();
                    for id in &hit {
                        if never(*id, probe, site) {
                            failures.push(format!(
                                "{} at {site:?}: `x = {}` fired {id:?}, which FIELD_NEVER says \
                                 it never does there",
                                row.function, probe.call
                            ));
                        }
                    }
                    if !field && hit.is_empty() {
                        failures.push(format!(
                            "{} at {site:?}: `x = {}` fired none of {ids:?}",
                            row.function, probe.call
                        ));
                    }
                    reachable.extend(ids.iter().copied().filter(|id| !never(*id, probe, site)));
                    fired.extend(hit);
                }
                for id in ids {
                    if !reachable.contains(id) {
                        continue;
                    }
                    // plan-145-I: every pair not in `FIELD_NEVER` must fire — a field
                    // self-update needs an in-place lowering, a `Rebuild` row with its
                    // proof, or a deferral to a named plan.
                    if !fired.contains(id) {
                        failures.push(format!(
                            "{} at {site:?}: no probe fired {id:?}",
                            row.function
                        ));
                    }
                    if !arms.contains(id) {
                        failures.push(format!(
                            "{} at {site:?}: {id:?} is not dispatched (not in SELF_UPDATE_ARMS)",
                            row.function
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// plan-145-A: `FIELD_NEVER` names only field sites, and a pair appears in it
    /// at most once.
    #[test]
    fn field_never_names_field_sites_once() {
        let codes: Vec<String> = FIELD_SITES.iter().map(|s| s.code()).collect();
        let mut seen = BTreeSet::new();
        let mut failures = Vec::new();
        for (arm, ty, sites, reason) in FIELD_NEVER {
            assert!(
                !reason.trim().is_empty(),
                "FIELD_NEVER {arm:?} has no reason"
            );
            for site in *sites {
                if !codes.iter().any(|c| c == site) {
                    failures.push(format!("FIELD_NEVER {arm:?} names no field site {site}"));
                }
                if !seen.insert((*arm, site.to_string(), ty.to_string())) {
                    failures.push(format!("FIELD_NEVER lists {arm:?} ({ty}) at {site} twice"));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// plan-145-I: an arm that serves no field site (`FieldReach::None` — today the
    /// `String` concat, plan-145-A Open Decision 1) must be declared so, not left
    /// to fail quietly: `FIELD_NEVER` lists it at every field site, and every
    /// `field_expect.tsv` line of its rows is a deferral to a named plan (or `na:`,
    /// where the program does not compile).
    #[test]
    fn an_arm_with_no_field_reach_is_deferred_at_every_field_site() {
        const FIELD_EXPECT: &str =
            include_str!("../../../../tests/runtime/inplace_self_update/field_expect.tsv");
        let mut failures = Vec::new();
        for (id, _, reach) in SELF_UPDATE_ARMS {
            if *reach != FieldReach::None {
                continue;
            }
            for site in FIELD_SITES {
                let code = site.code();
                if !FIELD_NEVER
                    .iter()
                    .any(|(arm, _, sites, _)| arm == id && sites.contains(&code.as_str()))
                {
                    failures.push(format!(
                        "{id:?} has FieldReach::None but FIELD_NEVER does not list it at {code}"
                    ));
                }
            }
            let rows = SELF_UPDATE_TABLE
                .iter()
                .filter(|row| matches!(row.kind, SelfUpdate::Arm(ids) if ids.contains(id)));
            for row in rows {
                let prefixes = [format!("{}(", row.function), format!("{} (", row.function)];
                let lines: Vec<&str> = FIELD_EXPECT
                    .lines()
                    .filter(|line| !line.starts_with('#'))
                    .filter(|line| prefixes.iter().any(|p| line.starts_with(p.as_str())))
                    .collect();
                if lines.is_empty() {
                    failures.push(format!(
                        "{id:?}: field_expect.tsv has no line for row {}",
                        row.function
                    ));
                }
                for line in lines {
                    let expect = line.rsplit('\t').next().unwrap_or("");
                    // `na:` — the program does not compile there at all.
                    if !expect.starts_with("deferred:") && !expect.starts_with("na:") {
                        failures.push(format!(
                            "{id:?} has FieldReach::None, but field_expect.tsv says `{line}` — \
                             a field self-update needs an in-place lowering, a `Rebuild` row \
                             with its proof, or a deferral to a named plan"
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
