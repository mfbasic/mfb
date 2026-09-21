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
    (ArmId::RemoveAt, |b, s, v| {
        b.try_inplace_remove_at_assign(s, v)
    }),
    (ArmId::Insert, |b, s, v| b.try_inplace_insert_assign(s, v)),
    (ArmId::SetRemove, |b, s, v| {
        b.try_inplace_set_remove_assign(s, v)
    }),
    (ArmId::Concat, |b, s, v| b.try_inplace_concat_assign(s, v)),
    (ArmId::Filter, |b, s, v| b.try_inplace_filter_assign(s, v)),
    (ArmId::Take, |b, s, v| b.try_inplace_take_assign(s, v)),
    (ArmId::Drop, |b, s, v| b.try_inplace_drop_assign(s, v)),
    (ArmId::Mid, |b, s, v| b.try_inplace_mid_assign(s, v)),
    (ArmId::Distinct, |b, s, v| {
        b.try_inplace_distinct_assign(s, v)
    }),
    (ArmId::Math, |b, s, v| b.try_inplace_math_assign(s, v)),
    (ArmId::Replace, |b, s, v| b.try_inplace_replace_assign(s, v)),
    (ArmId::Transform, |b, s, v| {
        b.try_inplace_transform_assign(s, v)
    }),
    (ArmId::Sort, |b, s, v| b.try_inplace_sort_assign(s, v)),
    (ArmId::SortBy, |b, s, v| b.try_inplace_sort_by_assign(s, v)),
    (ArmId::Union, |b, s, v| b.try_inplace_union_assign(s, v)),
    (ArmId::Intersection, |b, s, v| {
        b.try_inplace_intersection_assign(s, v)
    }),
    (ArmId::Difference, |b, s, v| {
        b.try_inplace_difference_assign(s, v)
    }),
    (ArmId::SymmetricDifference, |b, s, v| {
        b.try_inplace_symmetric_difference_assign(s, v)
    }),
    (ArmId::Merge, |b, s, v| b.try_inplace_merge_assign(s, v)),
    (ArmId::MapValues, |b, s, v| {
        b.try_inplace_map_values_assign(s, v)
    }),
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
///   `collections.take` → `take`.
///
/// `None` for anything that is not a `collections` builtin.
pub(crate) fn self_update_builtin(target: &str) -> Option<&'static str> {
    if let Some(bare) = crate::codegen::builtins::native_builtin_target(target) {
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
            let NirValue::Call { target, args, .. } = value else {
                return false;
            };
            matches!(args.first(), Some(NirValue::Local(arg0)) if arg0 == name) && wanted(target)
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
    /// Give this function a self-update scratch slot if its body holds a
    /// self-update whose arm needs one. The slot is registered as a function-level
    /// owned `List OF Integer`, so the ordinary scope drop frees it on every exit
    /// (with the null guard and prologue zeroing that drop brings). A function
    /// without such a statement is untouched.
    pub(crate) fn prescan_self_update_scratch(&mut self, ops: &[NirOp]) {
        if self.self_update_scratch.is_some()
            || !ops_hold_self_update(ops, &target_needs_self_update_scratch)
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

        self.emit(abi::label(&ready));
        let block = self.temporary_vreg();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), scratch_slot));
        self.emit(abi::add_immediate(&block, &block, COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64(&block, abi::stack_pointer(), data_slot));
        Ok(data_slot)
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
    /// Not yet in place; names the plan-142 letter that lands it. Letter I deletes
    /// this variant.
    Pending(&'static str),
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

#[cfg(test)]
const fn pending(
    function: &'static str,
    letter: &'static str,
    probes: &'static [Probe],
) -> SelfUpdateRow {
    SelfUpdateRow {
        function,
        kind: SelfUpdate::Pending(letter),
        probes,
    }
}

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
    pending(
        "collections::reduce",
        "E",
        &[probe_with(
            C,
            PUSH,
            LI,
            "[1, 2, 3]",
            "collections::reduce(x, [0], push)",
        )],
    ),
    pending(
        "collections::reduceRight",
        "E",
        &[probe_with(
            C,
            PUSH,
            LI,
            "[1, 2, 3]",
            "collections::reduceRight(x, [0], push)",
        )],
    ),
    pending(
        "compress::deflate",
        "E",
        &[probe(Z, LB, BYTES, "compress::deflate(x, 6)")],
    ),
    pending(
        "compress::inflate",
        "E",
        &[probe(
            Z,
            LB,
            "compress::deflate(encoding::utf8Encode(\"hello\"), 6)",
            "compress::inflate(x, 1048576)",
        )],
    ),
    pending(
        "compress::gzipEncode",
        "E",
        &[probe(Z, LB, BYTES, "compress::gzipEncode(x, 6)")],
    ),
    pending(
        "compress::gzipDecode",
        "E",
        &[probe(
            Z,
            LB,
            "compress::gzipEncode(encoding::utf8Encode(\"hello\"), 6)",
            "compress::gzipDecode(x, 1048576, FALSE)",
        )],
    ),
    pending(
        "compress::zlibEncode",
        "E",
        &[probe(Z, LB, BYTES, "compress::zlibEncode(x, 6)")],
    ),
    pending(
        "compress::zlibDecode",
        "E",
        &[probe(
            Z,
            LB,
            "compress::zlibEncode(encoding::utf8Encode(\"hello\"), 6)",
            "compress::zlibDecode(x, 1048576, FALSE)",
        )],
    ),
    pending(
        "crypto::argon2id",
        "E",
        &[probe(
            K,
            LB,
            BYTES,
            "crypto::argon2id(x, encoding::utf8Encode(\"saltsaltsalt\"), 32, 1, 1, 32)",
        )],
    ),
    pending(
        "crypto::shake256",
        "E",
        &[probe(K, LB, BYTES, "crypto::shake256(x, 32)")],
    ),
];

#[cfg(test)]
impl ArmId {
    /// The stack-slot type names only this arm allocates (plan-141 Appendix C:
    /// each occurs exactly once in `src/`). The slot's presence in a function
    /// proves the arm fired there. `Set` has one per collection kind.
    pub(crate) fn markers(self) -> &'static [&'static str] {
        match self {
            ArmId::Append => &["inplace_append_item"],
            ArmId::BulkAppend => &["inplace_bulk_append_rhs"],
            ArmId::SetAdd => &["inplace_set_add_item"],
            ArmId::Set => &["inplace_set_index", "inplace_set_key"],
            ArmId::RemoveKey => &["inplace_remove_key"],
            ArmId::Prepend => &["inplace_prepend_item"],
            ArmId::RemoveAt => &["inplace_remove_at_index"],
            ArmId::Insert => &["inplace_insert_index"],
            ArmId::SetRemove => &["inplace_set_remove_item"],
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

/// The binding sites the matrix test compiles every arm probe at. F, G and H
/// each append theirs.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum Site {
    /// S1 — a `MUT` local in a function body.
    Local,
}

#[cfg(test)]
pub(crate) const ENABLED_SITES: &[Site] = &[Site::Local];

#[cfg(test)]
impl Probe {
    /// The whole program performing this probe's self-update at `site`, in `main`.
    pub(crate) fn source(&self, site: Site) -> String {
        let mut src = String::from("IMPORT io\n");
        for import in self.imports {
            src.push_str(&format!("IMPORT {import}\n"));
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
        }
        src
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::codegen::registry::{registry, self_update_shaped};
    use crate::target::NativeBuildMode::Console;
    use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

    /// Operator self-updates: rows with no registry function behind them.
    const OPERATORS: &[&str] = &["&"];

    /// Every function with at least one self-update-shaped overload, as `pkg::name`.
    fn shaped_functions() -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for package in registry().packages() {
            for function in package.functions() {
                if function.implementations.iter().any(self_update_shaped) {
                    out.insert(format!("{}::{}", package.import_name(), function.name));
                }
            }
        }
        out
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

    #[test]
    fn self_update_table_has_no_stale_rows() {
        let shaped = shaped_functions();
        let arms: BTreeSet<ArmId> = SELF_UPDATE_ARMS.iter().map(|(id, _)| *id).collect();
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
                shaped.contains(row.function) || OPERATORS.contains(&row.function),
                "row {} names no registry function with a self-update-shaped overload",
                row.function
            );
            assert!(!row.probes.is_empty(), "row {} has no probe", row.function);
            if let SelfUpdate::Pending(letter) = row.kind {
                assert!(
                    ["B", "C", "D", "E"].contains(&letter),
                    "row {} is pending on `{letter}`, which is not a plan-142 letter that \
                     lands arms or exemptions",
                    row.function
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
        let arms: BTreeSet<ArmId> = SELF_UPDATE_ARMS.iter().map(|(id, _)| *id).collect();
        let mut failures = Vec::new();
        for row in SELF_UPDATE_TABLE {
            let SelfUpdate::Arm(ids) = row.kind else {
                continue;
            };
            for &site in ENABLED_SITES {
                let mut fired = BTreeSet::new();
                for probe in row.probes {
                    let src = probe.source(site);
                    let code = code_for_src_cached(&src, CodeTarget::LinuxX86_64, Console);
                    let main = code_function(code, "main");
                    let hit: Vec<ArmId> = ids
                        .iter()
                        .copied()
                        .filter(|id| {
                            main.stack_slots
                                .iter()
                                .any(|slot| id.markers().contains(&slot.type_.as_str()))
                        })
                        .collect();
                    if hit.is_empty() {
                        failures.push(format!(
                            "{} at {site:?}: `x = {}` fired none of {ids:?}",
                            row.function, probe.call
                        ));
                    }
                    fired.extend(hit);
                }
                for id in ids {
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
}
