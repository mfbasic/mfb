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
}

/// A binding being self-updated: which one, its type, and where its block lives.
///
/// Built once per assignment by the site that recognised it; every arm reads the
/// destination from here rather than from the frame, which is what lets a new
/// site reuse every arm.
pub(crate) struct SelfUpdateSite<'a> {
    /// The binding's name — the statement is `name = f(name, …)`.
    pub(crate) name: &'a str,
    /// The binding's declared type.
    pub(crate) type_: ParameterType,
    /// Where the binding's block pointer lives.
    pub(crate) dest: InPlaceDest,
    /// `G1` — the local is a by-ref capture whose slot holds a pointer to the
    /// parent's slot, not the block.
    pub(crate) by_ref: bool,
}

/// An arm: `Ok(true)` when it lowered the statement in place, `Ok(false)` to
/// decline (having emitted nothing).
pub(crate) type ArmFn =
    fn(&mut CodeBuilder<'_>, &SelfUpdateSite<'_>, &NirValue) -> Result<bool, String>;

/// The dispatch list. Every arm-backed `SELF_UPDATE_TABLE` row names ids from
/// here; `self_update_table_has_no_stale_rows` checks both directions.
pub(crate) const SELF_UPDATE_ARMS: &[(ArmId, ArmFn)] = &[
    // `append` and `bulk_append` share the builtin name and split on G11
    // (element vs list item type); keep single-element first.
    (ArmId::Append, |b, s, v| b.try_inplace_append_assign(s, v)),
    (ArmId::BulkAppend, |b, s, v| {
        b.try_inplace_bulk_append_assign(s, v)
    }),
    (ArmId::SetAdd, |b, s, v| b.try_inplace_set_add_assign(s, v)),
    (ArmId::Set, |b, s, v| b.try_inplace_set_assign(s, v)),
    (ArmId::RemoveKey, |b, s, v| {
        b.try_inplace_remove_key_assign(s, v)
    }),
    (ArmId::Prepend, |b, s, v| b.try_inplace_prepend_assign(s, v)),
    (ArmId::RemoveAt, |b, s, v| b.try_inplace_remove_at_assign(s, v)),
    (ArmId::Insert, |b, s, v| b.try_inplace_insert_assign(s, v)),
    (ArmId::SetRemove, |b, s, v| {
        b.try_inplace_set_remove_assign(s, v)
    }),
    (ArmId::Concat, |b, s, v| b.try_inplace_concat_assign(s, v)),
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
        for (_, arm) in SELF_UPDATE_ARMS {
            if arm(self, site, value)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
