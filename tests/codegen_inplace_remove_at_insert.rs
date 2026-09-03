//! plan-121-B: `insert`, `removeAt` and Set `remove` take an in-place path on a
//! uniquely-owned plain `MUT` local — and, just as importantly, DECLINE when a
//! live `FOR EACH` is walking the collection.
//!
//! Both halves need a codegen-inspection test, for opposite reasons:
//!
//! * **Taken.** A black-box rt fixture cannot see a missed fast path. If an arm
//!   silently stops matching, every behavioral test still passes and the program
//!   just gets slow again — `list (Fixed) insert` was 61× c -O0 and
//!   `set (Fixed) remove` 677× precisely because no arm existed, and nothing was
//!   red.
//! * **Refused.** A missed *decline* does not get slow, it MISCOMPILES, and the
//!   symptom is a skipped or repeated element rather than a crash. That is the
//!   asymmetry plan-121-A's gate inventory exists to pin: `append` may proceed
//!   under a live `FOR EACH` because it writes only beyond the count the loop
//!   snapshotted at entry, while `removeAt` shifts survivors *down* — rewriting
//!   entries below that snapshot — and `insert` shifts up from an index inside
//!   it. Both must decline where `append` does not.
//!
//! Build-only `--ncode` checks cross-built for `linux-x86_64`, matching the
//! sibling suites: the in-place gates are shared, target-independent codegen, so
//! there is no wall-clock timing here. The presence of the lowering's own label
//! IS the fast path.

mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

/// Count `label` instructions whose name contains `needle` in the named
/// function.
fn label_count(plan: &Value, symbol: &str, needle: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|instr| {
            instr["op"].as_str() == Some("label")
                && instr["name"]
                    .as_str()
                    .is_some_and(|name| name.contains(needle))
        })
        .count()
}

fn function<'a>(plan: &'a Value, symbol: &str) -> &'a Value {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("code plan has no function '{symbol}'"))
}

fn ncode(name: &str, source: &str) -> Value {
    let project = common::temp_project(name, source);
    let plan = common::build_ncode(&project, TARGET, name);
    let _ = std::fs::remove_dir_all(&project);
    plan
}

/// `xs = collections::removeAt(xs, i)` on a uniquely-owned `MUT` local closes
/// the hole in the live buffer instead of allocating a fresh tight block and
/// copying every survivor into it. Spike 3 measured the difference: at N = 6400
/// a `List OF Integer` is 51 KB, a `memmove` of it is ≈ 2 µs, and the
/// out-of-place `removeAt` cost 72 µs — the allocate/copy/free is 36× the data
/// movement, and that is what this path deletes.
#[test]
fn remove_at_on_a_plain_local_mutates_in_place() {
    let plan = ncode(
        "inplace_removeat",
        "IMPORT collections\n\
         FUNC shrink(n AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF Integer = []\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   xs = collections::append(xs, i)\n\
        \x20 NEXT\n\
        \x20 FOR i = 0 TO n - 2\n\
        \x20   xs = collections::removeAt(xs, 0)\n\
        \x20 NEXT\n\
        \x20 RETURN len(xs)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN shrink(8)\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_shrink", "remove_inplace") >= 1,
        "removeAt on a uniquely-owned MUT local must close the hole in the live \
         buffer (`lower_list_remove_at_in_place`), not allocate a fresh block and \
         copy every survivor into it"
    );
}

/// The decline that matters. `removeAt` shifts survivors DOWN, rewriting entries
/// below the count a live `FOR EACH` snapshotted at loop entry — which the loop
/// can observe as a skipped element. `append` is safe in the same position
/// because it writes only *past* that count; `removeAt` is not, and may not
/// borrow `append`'s reasoning.
#[test]
fn remove_at_declines_under_a_live_for_each() {
    let plan = ncode(
        "inplace_removeat_foreach",
        "IMPORT collections\n\
         FUNC walk(n AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF Integer = []\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   xs = collections::append(xs, i)\n\
        \x20 NEXT\n\
        \x20 MUT seen AS Integer = 0\n\
        \x20 FOR EACH v IN xs\n\
        \x20   seen = seen + v\n\
        \x20   xs = collections::removeAt(xs, 0)\n\
        \x20 NEXT\n\
        \x20 RETURN seen\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN walk(4)\n\
         END FUNC\n",
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_walk", "remove_inplace"),
        0,
        "a `removeAt` inside a live `FOR EACH` over the same binding must take \
         the COPYING path: the in-place shift moves entries below the count the \
         loop snapshotted at entry, so the loop would observe it. This is the one \
         place `removeAt` may not reuse `append`'s permissive gate."
    );
}

/// `xs = collections::insert(xs, i, v)` mutates in place. The shift is O(N) and
/// stays that way — that is the operation's defined cost and C pays it too; what
/// the arm removes is the per-call allocate + copy + free around it.
#[test]
fn insert_on_a_plain_local_mutates_in_place() {
    let plan = ncode(
        "inplace_insert",
        "IMPORT collections\n\
         FUNC grow(n AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF Integer = [0]\n\
        \x20 FOR i = 1 TO n - 1\n\
        \x20   xs = collections::insert(xs, 1, i)\n\
        \x20 NEXT\n\
        \x20 RETURN len(xs)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN grow(8)\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_grow", "insert_inplace") >= 1,
        "insert on a uniquely-owned MUT local must shift within the live buffer \
         (`lower_list_insert_in_place`), not rebuild the list per call"
    );
}

/// `insert` shifts entries UP starting at an index inside the live range, so a
/// concurrent `FOR EACH` can observe the move just as it can for `removeAt`.
#[test]
fn insert_declines_under_a_live_for_each() {
    let plan = ncode(
        "inplace_insert_foreach",
        "IMPORT collections\n\
         FUNC walk(n AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF Integer = []\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   xs = collections::append(xs, i)\n\
        \x20 NEXT\n\
        \x20 MUT seen AS Integer = 0\n\
        \x20 FOR EACH v IN xs\n\
        \x20   seen = seen + v\n\
        \x20   xs = collections::insert(xs, 0, v)\n\
        \x20 NEXT\n\
        \x20 RETURN seen\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN walk(4)\n\
         END FUNC\n",
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_walk", "insert_inplace"),
        0,
        "an `insert` inside a live `FOR EACH` over the same binding must take the \
         copying path — the shift moves entries the loop already snapshotted"
    );
}

/// Set `remove` had no arm anywhere, which is the whole of its 677× against
/// c -O0: `collections::remove` on a `Set` already reuses the Map `removeKey`
/// lowering out of place, so the in-place form is the same reuse of
/// `lower_map_remove_key_in_place` — entry compaction plus `BUCKETS_READY = 0`.
#[test]
fn set_remove_on_a_plain_local_mutates_in_place() {
    let plan = ncode(
        "inplace_set_remove",
        "IMPORT collections\n\
         FUNC drain(n AS Integer) AS Integer\n\
        \x20 MUT s AS Set OF Integer = Set OF Integer { }\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   s = collections::add(s, i)\n\
        \x20 NEXT\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   s = collections::remove(s, i)\n\
        \x20 NEXT\n\
        \x20 RETURN len(s)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN drain(8)\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_drain", "mrk_scan_loop") >= 1,
        "Set `remove` on a uniquely-owned MUT local must delete the entry in \
         place (`lower_map_remove_key_in_place`, whose scan emits `mrk_scan_loop`), \
         not rebuild the whole set per call"
    );
}

/// The entry compaction moves entries below a live iterator's snapshot, exactly
/// as the Map `removeKey` arm's own guard recognises (bug-142's non-freeing
/// twin), so Set `remove` declines there too.
#[test]
fn set_remove_declines_under_a_live_for_each() {
    let plan = ncode(
        "inplace_set_remove_foreach",
        "IMPORT collections\n\
         FUNC walk(n AS Integer) AS Integer\n\
        \x20 MUT s AS Set OF Integer = Set OF Integer { }\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   s = collections::add(s, i)\n\
        \x20 NEXT\n\
        \x20 MUT seen AS Integer = 0\n\
        \x20 FOR EACH v IN s\n\
        \x20   seen = seen + v\n\
        \x20   s = collections::remove(s, v)\n\
        \x20 NEXT\n\
        \x20 RETURN seen\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN walk(4)\n\
         END FUNC\n",
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_walk", "mrk_scan_loop"),
        0,
        "a Set `remove` inside a live `FOR EACH` over the same binding must take \
         the copying path — the entry compaction shifts entries the loop \
         snapshotted"
    );
}

/// The in-place `insert` carries its OWN bounds check, and it must, because the
/// copying path's check is no longer reached for this shape. Valid range is
/// `0 <= index <= count` — inserting at `count` is an append and is legal —
/// matching `lower_list_insert`'s gate exactly.
///
/// This is a regression guard with a real history: the first cut of
/// `lower_list_splice_in_place` had no check at all, because it was generalized
/// from `prepend`, where index 0 is always in range. A self-assigned
/// `insert(xs, 99, v)` would have written past the end instead of raising. The
/// existing `func_collection_insert_out_of_range` fixture could not catch it —
/// it spells the call as a `LET`, which is not the self-assignment the arm
/// matches, so it exercises the copying path.
#[test]
fn insert_in_place_still_bounds_checks() {
    let plan = ncode(
        "inplace_insert_bounds",
        "IMPORT collections\n\
         FUNC put(i AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF Integer = [1, 2]\n\
        \x20 xs = collections::insert(xs, i, 9)\n\
        \x20 RETURN len(xs)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN put(1)\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_put", "insert_inplace") >= 1,
        "precondition: this shape must take the in-place path, or the assertion \
         below would pass vacuously"
    );
    assert!(
        label_count(&plan, "_mfb_fn_put", "insert_inplace_invalid") >= 1,
        "the in-place `insert` must raise ErrIndexOutOfRange for an index outside \
         `0..=count`, before mutating anything. Generalizing `prepend` into a \
         splice loses this by default — index 0 is always valid — so the check is \
         explicit and this pins it."
    );
}

/// The same for `removeAt`, whose valid range is the stricter `0 <= index <
/// count` (there is no element at `count` to remove).
#[test]
fn remove_at_in_place_still_bounds_checks() {
    let plan = ncode(
        "inplace_removeat_bounds",
        "IMPORT collections\n\
         FUNC drop(i AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF Integer = [1, 2]\n\
        \x20 xs = collections::removeAt(xs, i)\n\
        \x20 RETURN len(xs)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN drop(0)\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_drop", "remove_inplace") >= 1,
        "precondition: this shape must take the in-place path"
    );
    assert!(
        label_count(&plan, "_mfb_fn_drop", "remove_inplace_invalid") >= 1,
        "the in-place `removeAt` must raise ErrIndexOutOfRange for an index \
         outside `0..count`, before any byte moves"
    );
}

/// G24. The in-place `removeAt` COMPACTS the data region — it relocates
/// surviving payloads inside the live buffer — which no other arm in the family
/// does (`append` writes past the live data; `insert` and `prepend` shift only
/// the 40-byte lookup entries). For a RECURSIVE element type that relocation is
/// observable: `type_participates_in_cycle` marks exactly the values that are
/// pointer-linked graphs needing a per-type runtime copy function, so an ordinary
/// `collections::get` of one does not hand back the independent deep copy a
/// String, record or nested-list element gets.
///
/// Before this guard, `get(xs, 0)` then `xs = removeAt(xs, 0)` then using the
/// value read gave `?` for every element whose removal moved bytes — correct only
/// for the last, where `count == 1` makes the shift length zero. Behavior is
/// pinned by `tests/rt-behavior/collections/p121b-removeat-recursive-union-rt`;
/// this pins that the arm *declines*, which the fixture alone cannot show — it
/// would pass either way if the copying path were also correct.
#[test]
fn remove_at_declines_for_a_recursive_element_type() {
    let plan = ncode(
        "inplace_removeat_recursive",
        "IMPORT collections\n\
         TYPE ElementNode\n\
        \x20 tag AS String\n\
        \x20 children AS List OF Node\n\
         END TYPE\n\
         TYPE TextNode\n\
        \x20 text AS String\n\
         END TYPE\n\
         UNION Node\n\
        \x20 ElementNode\n\
        \x20 TextNode\n\
         END UNION\n\
         FUNC drain(n AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF Node = []\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   LET nd AS Node = TextNode[\"t\"]\n\
        \x20   xs = collections::append(xs, nd)\n\
        \x20 NEXT\n\
        \x20 xs = collections::removeAt(xs, 0)\n\
        \x20 RETURN len(xs)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN drain(4)\n\
         END FUNC\n",
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_drain", "remove_inplace"),
        0,
        "`removeAt` on a list whose element type participates in a cycle must take \
         the COPYING path: the in-place compaction relocates payloads, and a \
         recursive element is a pointer-linked graph whose `get` result is not an \
         independent deep copy, so a value already read would follow moved bytes."
    );
}

/// The companion that stops the guard from being over-broad: the SAME program
/// shape with a NON-recursive union must still take the fast path. Dropping
/// `children` is the only difference, and it is what isolated the predicate in the
/// first place.
#[test]
fn remove_at_still_fires_for_a_non_recursive_union_element() {
    let plan = ncode(
        "inplace_removeat_nonrecursive",
        "IMPORT collections\n\
         TYPE ElementNode\n\
        \x20 tag AS String\n\
         END TYPE\n\
         TYPE TextNode\n\
        \x20 text AS String\n\
         END TYPE\n\
         UNION Node\n\
        \x20 ElementNode\n\
        \x20 TextNode\n\
         END UNION\n\
         FUNC drain(n AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF Node = []\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   LET nd AS Node = TextNode[\"t\"]\n\
        \x20   xs = collections::append(xs, nd)\n\
        \x20 NEXT\n\
        \x20 xs = collections::removeAt(xs, 0)\n\
        \x20 RETURN len(xs)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN drain(4)\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_drain", "remove_inplace") >= 1,
        "a NON-recursive union element must still take the in-place path — G24 \
         gates on participating in a cycle, not on being a union"
    );
}

/// A fixed-width list is **entry-free**: `list_entry_stride` returns 0 for
/// exactly the `list_element_is_fixed_width` predicate
/// (`builder_collection_layout.rs`), so element `i` is found at `i * payload` by
/// arithmetic and there is no 40-byte entry record to maintain.
///
/// An "identity mapping over entries 0..count" loop used to be emitted anyway, in
/// both the in-place splice (`prepend`/`insert`) and the out-of-place
/// `lower_list_insert`. Every store in it sat behind `entry_stride != 0`, which
/// is unreachable inside a fixed-width branch, so the loop ran `count` iterations
/// per call and wrote nothing into the block -- 20 instructions per element whose
/// only architectural effects were a spill slot overwritten three times and never
/// read, plus an `add_imm x8, x8, 0`.
///
/// That is why this is a codegen-inspection test and not a benchmark row: the
/// loop was pure waste, so **no behavioral test could ever see it** -- every
/// fixture passed the whole time. Only the emitted instruction stream shows it,
/// and only a test asserting on the absence keeps it gone.
#[test]
fn a_fixed_width_splice_emits_no_identity_entry_loop() {
    let plan = ncode(
        "inplace_ident_fixed",
        "IMPORT collections\n\
         FUNC grow(n AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF Integer = []\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   xs = collections::prepend(xs, i)\n\
        \x20 NEXT\n\
        \x20 MUT ys AS List OF Integer = [0]\n\
        \x20 FOR i = 1 TO n - 1\n\
        \x20   ys = collections::insert(ys, 1, i)\n\
        \x20 NEXT\n\
        \x20 RETURN len(xs) + len(ys)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN grow(8)\n\
         END FUNC\n",
    );
    // Both arms must still be taken...
    assert!(
        label_count(&plan, "_mfb_fn_grow", "prepend_inplace") >= 1,
        "prepend must still take the in-place path"
    );
    assert!(
        label_count(&plan, "_mfb_fn_grow", "insert_inplace") >= 1,
        "insert must still take the in-place path"
    );
    // ...and neither may emit the dead per-element loop.
    assert_eq!(
        label_count(&plan, "_mfb_fn_grow", "ident_loop"),
        0,
        "a fixed-width list has no lookup entries, so an identity-entry loop is \
         `count` iterations of dead work per call"
    );
}

/// The other half of the pair, and the reason the fix is a deletion confined to
/// one branch rather than a blanket removal: a **variable-width** element type
/// genuinely has a 40-byte entry table, so its splice must still shift those
/// entries.
///
/// Without this case, deleting the entry loop outright would also satisfy the
/// test above -- and silently miscompile every `List OF String` prepend.
#[test]
fn a_variable_width_splice_still_shifts_its_entry_table() {
    let plan = ncode(
        "inplace_ident_var",
        "IMPORT collections\n\
         FUNC grow(n AS Integer) AS Integer\n\
        \x20 MUT xs AS List OF String = []\n\
        \x20 FOR i = 0 TO n - 1\n\
        \x20   xs = collections::prepend(xs, \"s\")\n\
        \x20 NEXT\n\
        \x20 RETURN len(xs)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 RETURN grow(8)\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_grow", "prepend_inplace") >= 1,
        "prepend on a String list must still take the in-place path"
    );
    assert!(
        label_count(&plan, "_mfb_fn_grow", "shift_loop") >= 1,
        "a variable-width list keeps its lookup table, so the entry shift is \
         load-bearing and must survive the fixed-width deletion"
    );
}
