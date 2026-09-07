//! bug-536 shape B-2: who owns the bare `String` a `.mfb`-bodied function
//! returns — asserted on the emitted code, because the two failure modes are
//! invisible to a behavioural test in opposite ways.
//!
//! * **Too few owners** is a leak. Nothing goes red; the program is simply
//!   slower to run out of memory. That is how `csv::parse` came to cost 235 MB
//!   per repeat call with every test green.
//! * **Too many owners** is a double free of a block the caller still holds, and
//!   the arena's free list turns a wrong `arena_free` into "Allocation failed" on
//!   some *later*, unrelated allocation — or a wrong value read back from reused
//!   memory. A behavioural probe sees that only if it happens to reuse the block
//!   before the read.
//!
//! Counting owners in the instruction stream sees both directly. The two signals
//! are the `pending_temp` stack slot (`register_pending_temp` allocates exactly
//! one per registered temp) and the `_mfb_rt_drop_owned_string` call.
//!
//! Every assertion is COMPARATIVE — one program against a sibling that differs in
//! exactly one way — so it pins the difference the fix makes rather than an
//! absolute count that drifts with every unrelated codegen change.
//!
//! Build-only `-ncode` cross-built for `linux-x86_64`, matching the sibling
//! codegen-inspection suite: ownership is target-independent codegen.

#[path = "../common/mod.rs"]
mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

fn ncode(name: &str, source: &str) -> Value {
    let project = common::temp_project(name, source);
    let plan = common::build_ncode(&project, TARGET, name);
    let _ = std::fs::remove_dir_all(&project);
    plan
}

fn function<'a>(plan: &'a Value, symbol: &str) -> &'a Value {
    plan["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("code plan has no function '{symbol}'"))
}

/// How many statement-scope temporaries `symbol` registers. One slot per
/// `register_pending_temp`, named by `allocate_stack_object("pending_temp", 8)`.
fn pending_temps(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["stackSlots"]
        .as_array()
        .expect("stack slots array")
        .iter()
        .filter(|slot| slot["type"].as_str() == Some("pending_temp"))
        .count()
}

/// How many owned-`String` drops `symbol` emits — the count of places that will
/// free a `String` block.
fn string_drops(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["instructions"]
        .as_array()
        .expect("instructions array")
        .iter()
        .filter(|instr| instr["target"].as_str() == Some("_mfb_rt_drop_owned_string"))
        .count()
}

/// The one `SUB main` all three callee shapes are measured through: the call
/// result is left UNBOUND (consumed by `len`), which is exactly the position that
/// had no owner at all before this fix.
fn main_calling(callee: &str) -> String {
    format!(
        "IMPORT io\n\
         {callee}\
         SUB main()\n\
        \x20 LET held AS String = \"held-value\"\n\
        \x20 io::print(toString(len(f(held))))\n\
        \x20 io::print(held)\n\
         END SUB\n"
    )
}

/// `RETURN <param>` — the plan-86 K1 parameter passthrough. The result is a
/// BORROW of the caller's own argument block.
const BORROW_CALLEE: &str = "FUNC f(s AS String) AS String\n  RETURN s\nEND FUNC\n";

/// `RETURN <owned local>` — the block is allocated in the callee and moved to the
/// caller, so the caller is its sole owner.
const FRESH_CALLEE: &str =
    "FUNC f(s AS String) AS String\n  MUT out AS String = \"v\"\n  RETURN out\nEND FUNC\n";

/// bug-536 shape B-2, the leak half: a `String` returned by a `.mfb`-bodied
/// function and left unbound had no owner, because native freshness provenance
/// (`mark_fresh_string`) is set by the producer's own lowering and cannot travel
/// out of a callee. `function_returns_fresh_string` is the callee's promise read
/// from its NIR instead, and it makes the call site register the result for the
/// statement-scope free — exactly one new owner for exactly one new block.
#[test]
fn a_fresh_string_callee_result_gets_exactly_one_owner_at_the_call_site() {
    let borrow = ncode("b536_b2_borrow_callee", &main_calling(BORROW_CALLEE));
    let fresh = ncode("b536_b2_fresh_callee", &main_calling(FRESH_CALLEE));
    let borrow_temps = pending_temps(&borrow, "_mfb_fn_main");
    let fresh_temps = pending_temps(&fresh, "_mfb_fn_main");
    assert_eq!(
        fresh_temps,
        borrow_temps + 1,
        "a call to a fresh-String-returning callee must register exactly ONE more \
         statement-scope temp than the same call to a param-borrow callee \
         (borrow {borrow_temps}, fresh {fresh_temps}); fewer is bug-536 shape B-2's \
         leak, more is a double free"
    );
}

/// The POSITIVE pin, and the one the whole design is shaped around: a callee whose
/// every value-return is a bare parameter returns a BORROW of the caller's
/// argument block (plan-86 K1). Freeing that result frees a `String` the caller
/// still owns — SIGBUS if the argument was a literal, free-list corruption
/// otherwise, surfacing as "Allocation failed" in some later, unrelated
/// allocation.
///
/// So `function_returns_fresh_string` must exclude it, and the check that it does
/// is that introducing the borrow call adds NO owner: the same `main` with the
/// call spliced out registers the same number of temps.
///
/// This is the shape of guard bug-497 needed —
/// `every_byte_list_producer_still_passes_the_write_header_check` — read in the
/// other direction: not "does the new rule reject something valid" but "does the
/// new permission admit something it must not".
#[test]
fn a_param_borrow_string_callee_result_is_never_given_an_owner() {
    let borrow = ncode("b536_b2_borrow_callee2", &main_calling(BORROW_CALLEE));
    let no_call = ncode(
        "b536_b2_no_call",
        "IMPORT io\n\
         SUB main()\n\
        \x20 LET held AS String = \"held-value\"\n\
        \x20 io::print(toString(len(held)))\n\
        \x20 io::print(held)\n\
         END SUB\n",
    );
    let borrow_temps = pending_temps(&borrow, "_mfb_fn_main");
    let no_call_temps = pending_temps(&no_call, "_mfb_fn_main");
    assert_eq!(
        borrow_temps, no_call_temps,
        "a param-borrow callee's result aliases the caller's argument block, so the \
         call site must register NO temp for it (with call {borrow_temps}, without \
         {no_call_temps}) — freeing it would free the caller's live String"
    );
}

/// `toString` of a `String` is the IDENTITY: it returns its own argument's block.
/// It also spills that argument and reloads it into a fresh register, so the
/// result leaves under a different operand than it arrived — which silently broke
/// the pending-temp identity chain. `claim_pending_temp` no longer matched, so the
/// owning binding did NOT claim the temp: the statement-scope free ran anyway and
/// the binding was left holding freed memory.
///
/// That is a pre-existing use-after-free (it predates shape B-2; the equivalent
/// program printed the wrong text and then SIGSEGVed on the pre-fix compiler), and
/// it is visible right here as an owner count: on the pre-fix compiler the
/// identity-wrapped program emitted **4** `_mfb_rt_drop_owned_string` calls where
/// the unwrapped one emitted 3 — one extra free for the same three blocks.
///
/// The invariant, stated so it cannot drift: wrapping an expression in an identity
/// must not change how many owners its value has.
#[test]
fn the_tostring_identity_adds_no_owner() {
    let plain = ncode(
        "b536_b2_identity_plain",
        "IMPORT io\n\
         SUB main()\n\
        \x20 MUT i AS Integer = 0\n\
        \x20 WHILE i < 3\n\
        \x20   LET a AS String = \"x\" & toString(i)\n\
        \x20   io::print(a)\n\
        \x20   i = i + 1\n\
        \x20 END WHILE\n\
         END SUB\n",
    );
    let wrapped = ncode(
        "b536_b2_identity_wrapped",
        "IMPORT io\n\
         SUB main()\n\
        \x20 MUT i AS Integer = 0\n\
        \x20 WHILE i < 3\n\
        \x20   LET a AS String = toString(\"x\" & toString(i))\n\
        \x20   io::print(a)\n\
        \x20   i = i + 1\n\
        \x20 END WHILE\n\
         END SUB\n",
    );
    let plain_drops = string_drops(&plain, "_mfb_fn_main");
    let wrapped_drops = string_drops(&wrapped, "_mfb_fn_main");
    assert_eq!(
        wrapped_drops, plain_drops,
        "`toString(<String>)` is the identity, so wrapping an expression in it must \
         not change the number of owned-String drops (plain {plain_drops}, wrapped \
         {wrapped_drops}); an extra drop is a second free of the block the binding \
         owns — a use-after-free, not a leak"
    );
    assert_eq!(
        pending_temps(&wrapped, "_mfb_fn_main"),
        pending_temps(&plain, "_mfb_fn_main"),
        "the identity must RETARGET the argument's pending temp, not register a \
         second one for the same block"
    );
}

// ------------------------------------------------------------------ bug-562

/// How many block copies `symbol` emits. `copy_flat_block` allocates exactly one
/// `flat_copy_source` stack slot per call, so this counts the places the function
/// deep-copies a flat value — the ownership-establishing copy this whole file is
/// about, read from the other end than [`string_drops`].
fn flat_copies(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["stackSlots"]
        .as_array()
        .expect("stack slots array")
        .iter()
        .filter(|slot| slot["type"].as_str() == Some("flat_copy_source"))
        .count()
}

/// The same callee `f`, reached two ways: passed to `collections::transform` as a
/// `FunctionRef`, or called directly. `f`'s own body is identical in both, so any
/// difference in how many blocks it copies is caused solely by how it is REACHED.
fn callee_reached_as_callback(body: &str) -> String {
    format!(
        "IMPORT io\n\
         IMPORT collections\n\
         {body}\
         SUB main()\n\
        \x20 LET xs AS List OF String = [\"a\", \"bb\"]\n\
        \x20 io::print(collections::get(collections::transform(xs, f), 0))\n\
         END SUB\n"
    )
}

fn callee_reached_directly(body: &str) -> String {
    format!(
        "IMPORT io\n\
         IMPORT collections\n\
         {body}\
         SUB main()\n\
        \x20 LET xs AS List OF String = [\"a\", \"bb\"]\n\
        \x20 io::print(f(collections::get(xs, 0)))\n\
         END SUB\n"
    )
}

/// `RETURN toString(s)`. `toString`'s `String` arm is the IDENTITY, so lowering
/// cannot establish that the returned block is fresh — the callee owes the caller
/// a copy.
const IDENTITY_CALLEE: &str = "FUNC f(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n";

/// `RETURN <concat>`. The concat registers a pending temp the return CLAIMS, so
/// the block is already fresh and solely owned — no copy is owed, either way.
const CONCAT_CALLEE: &str = "FUNC f(s AS String) AS String\n  RETURN \"<\" & s & \">\"\nEND FUNC\n";

/// bug-562, stated as an invariant that cannot drift: **passing a function as a
/// callback must not remove its return-ownership obligation.**
///
/// It did. `function_returns_fresh_string` excluded every callback-referenced
/// name, so the identical callee copied its unprovable return when called
/// directly and did NOT when passed to `collections::transform` — 1 copy vs 0.
/// The `FunctionRef` ABI meanwhile materialises each `List OF String` element
/// into a fresh block, hands it to the callback, and frees it
/// (`free_collection_loop_item`). The identity returned that very block, so
/// `transform`/`sortBy`/`groupBy` over a `List OF String` were `[exit 139]`, and
/// a wrong value where the freed block did not fault.
///
/// Asserted as an equality between two reachings of the SAME body rather than an
/// absolute count, so it survives any unrelated change to how many copies the
/// shape needs.
#[test]
fn being_used_as_a_callback_never_removes_the_return_copy() {
    let as_callback = ncode(
        "b562_ident_callback",
        &callee_reached_as_callback(IDENTITY_CALLEE),
    );
    let direct = ncode(
        "b562_ident_direct",
        &callee_reached_directly(IDENTITY_CALLEE),
    );
    let callback_copies = flat_copies(&as_callback, "_mfb_fn_f");
    let direct_copies = flat_copies(&direct, "_mfb_fn_f");
    assert_eq!(
        callback_copies, direct_copies,
        "the same `RETURN toString(s)` callee must copy its result the same number \
         of times whether it is passed as a callback ({callback_copies}) or called \
         directly ({direct_copies}); copying fewer hands the `FunctionRef` ABI a \
         block it does not own, which it then frees — bug-562's SIGSEGV"
    );
    assert!(
        callback_copies >= 1,
        "an identity return has no provable freshness, so the callee owes the \
         caller exactly one copy — got {callback_copies}"
    );
}

/// The POSITIVE pin for bug-562, and the half that says the fix is a fix rather
/// than a blanket copy: a callback whose return is ALREADY a fresh, solely-owned
/// block must not gain a second copy. `lower_returned_value`'s arms are mutually
/// exclusive early returns, so the new catch-all is only reached when none of the
/// three freshness-establishing arms fired — but that is an argument, and this is
/// the measurement.
///
/// Two assertions, because either alone is satisfiable by a wrong fix: the
/// equality alone is satisfied by copying in BOTH reachings, and the inequality
/// alone by copying in neither.
#[test]
fn a_callback_whose_result_is_already_fresh_is_not_copied_twice() {
    let as_callback = ncode(
        "b562_concat_callback",
        &callee_reached_as_callback(CONCAT_CALLEE),
    );
    let direct = ncode(
        "b562_concat_direct",
        &callee_reached_directly(CONCAT_CALLEE),
    );
    let identity = ncode(
        "b562_ident_callback2",
        &callee_reached_as_callback(IDENTITY_CALLEE),
    );
    let callback_copies = flat_copies(&as_callback, "_mfb_fn_f");
    let direct_copies = flat_copies(&direct, "_mfb_fn_f");
    assert_eq!(
        callback_copies, direct_copies,
        "a `RETURN <concat>` callee's block is fresh by construction, so being \
         passed as a callback must not change its copy count (callback \
         {callback_copies}, direct {direct_copies})"
    );
    assert!(
        callback_copies < flat_copies(&identity, "_mfb_fn_f"),
        "the return-freshness copy must be inserted only where freshness is \
         UNPROVABLE: a claimed pending temp ({callback_copies} copies) must stay \
         below an identity return ({} copies), or the fix is a blanket copy of \
         every callback result",
        flat_copies(&identity, "_mfb_fn_f")
    );
}

// ------------------------------------------------------------------ bug-569

/// How many loop items `symbol` frees. `free_collection_loop_item` allocates
/// exactly one `loop_item_free_size` stack slot per emission and is a no-op for
/// every non-`String` type, so this counts the `String` blocks a callback-driving
/// loop takes ownership of — the ARGUMENT it materialises, plus (after bug-569)
/// the RESULT it collects.
fn loop_item_frees(plan: &Value, symbol: &str) -> usize {
    function(plan, symbol)["stackSlots"]
        .as_array()
        .expect("stack slots array")
        .iter()
        .filter(|slot| slot["type"].as_str() == Some("loop_item_free_size"))
        .count()
}

/// The symbol a monomorphized `.mfb` collections body is emitted under. The `$`
/// separators of `#collections_<name>$<args>` are mangled to `_24`, and the
/// leading `_` of the runtime target to `_5F`.
fn mfb_hof(name: &str, args: &str) -> String {
    let args: String = args.split('$').collect::<Vec<_>>().join("_24");
    format!("_mfb_ifn_collections_5F{name}_24{args}")
}

/// bug-569, per HOF, as the one comparison that isolates the callback's RESULT:
/// the same HOF, the same source collection, the same callback ARGUMENT type —
/// only the callback's RETURN type differs. A `String` return is a standalone
/// arena block the HOF must free; any fixed-width return materialises nothing.
///
/// So the `String` instantiation must own exactly ONE more block than its
/// fixed-width twin. Before the fix it owned the SAME number: the HOF freed the
/// argument it materialised and simply abandoned the result, 64 B per element per
/// call, on the most ordinary `collections::transform` there is.
///
/// Asserted as a delta rather than an absolute so it survives any unrelated
/// change to how many blocks these loops handle, and stated per HOF because each
/// frees on its own path — `transform`'s pin says nothing about `groupBy`'s.
///
/// `transform`: `abi_inline`, so the loop is emitted into the caller.
#[test]
fn transform_owns_the_string_block_its_callback_returns() {
    let to_string = ncode(
        "b569_transform_string",
        "IMPORT io\n\
         IMPORT collections\n\
         FUNC pick(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
         SUB main()\n\
        \x20 LET xs AS List OF String = [\"a\", \"bb\"]\n\
        \x20 io::print(collections::get(collections::transform(xs, pick), 0))\n\
         END SUB\n",
    );
    let to_integer = ncode(
        "b569_transform_integer",
        "IMPORT io\n\
         IMPORT collections\n\
         FUNC pick(s AS String) AS Integer\n  RETURN len(s)\nEND FUNC\n\
         SUB main()\n\
        \x20 LET xs AS List OF String = [\"a\", \"bb\"]\n\
        \x20 io::print(toString(collections::get(collections::transform(xs, pick), 0)))\n\
         END SUB\n",
    );
    let string_frees = loop_item_frees(&to_string, "_mfb_fn_main");
    let integer_frees = loop_item_frees(&to_integer, "_mfb_fn_main");
    assert_eq!(
        string_frees,
        integer_frees + 1,
        "`collections::transform` over the same `List OF String` must free exactly \
         one more block for a `String`-returning callback ({string_frees}) than for \
         an `Integer`-returning one ({integer_frees}): the argument it materialised, \
         plus the result it collected. Equal counts are bug-569's leak; two more is \
         a double free"
    );
}

/// `sortBy` with a `String` key declines the native fast path (its merge sorts
/// 8-byte keys), so the `.mfb` body runs and reaches the callback through
/// `collections::transform`. Measured on the monomorphized body, against the same
/// body with a `Float` key — which declines the fast path for the same reason and
/// so takes the identical route.
#[test]
fn sort_by_owns_the_string_key_its_callback_returns() {
    let string_key = ncode(
        "b569_sortby_string",
        "IMPORT io\n\
         IMPORT collections\n\
         FUNC key(v AS Integer) AS String\n  RETURN toString(v)\nEND FUNC\n\
         SUB main()\n\
        \x20 LET xs AS List OF Integer = [3, 1, 2]\n\
        \x20 io::print(toString(collections::get(collections::sortBy(xs, key), 0)))\n\
         END SUB\n",
    );
    let float_key = ncode(
        "b569_sortby_float",
        "IMPORT io\n\
         IMPORT collections\n\
         FUNC key(v AS Integer) AS Float\n  RETURN toFloat(v)\nEND FUNC\n\
         SUB main()\n\
        \x20 LET xs AS List OF Integer = [3, 1, 2]\n\
        \x20 io::print(toString(collections::get(collections::sortBy(xs, key), 0)))\n\
         END SUB\n",
    );
    let string_frees = loop_item_frees(&string_key, &mfb_hof("sortBy", "Integer$String"));
    let float_frees = loop_item_frees(&float_key, &mfb_hof("sortBy", "Integer$Float"));
    assert_eq!(
        string_frees,
        float_frees + 1,
        "`collections::sortBy` must free the `String` key its callback returns \
         (String key {string_frees}, Float key {float_frees}); the source is a \
         `List OF Integer` in both, so the ONLY block either loop can own is the key"
    );
}

/// `groupBy` takes the native fast path here (Integer key, re-eval-safe source),
/// which reaches both callbacks through `collections::transform` and emits the
/// whole thing into the caller. The twin differs only in `valFn`'s return type.
#[test]
fn group_by_owns_the_string_value_its_callback_returns() {
    let string_value = ncode(
        "b569_groupby_string",
        "IMPORT io\n\
         IMPORT collections\n\
         FUNC kf(s AS String) AS Integer\n  RETURN len(s)\nEND FUNC\n\
         FUNC vf(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
         SUB main()\n\
        \x20 LET xs AS List OF String = [\"a\", \"bb\"]\n\
        \x20 io::print(collections::get(collections::get(collections::groupBy(xs, kf, vf), 1), 0))\n\
         END SUB\n",
    );
    let integer_value = ncode(
        "b569_groupby_integer",
        "IMPORT io\n\
         IMPORT collections\n\
         FUNC kf(s AS String) AS Integer\n  RETURN len(s)\nEND FUNC\n\
         FUNC vf(s AS String) AS Integer\n  RETURN len(s) + 1\nEND FUNC\n\
         SUB main()\n\
        \x20 LET xs AS List OF String = [\"a\", \"bb\"]\n\
        \x20 io::print(toString(collections::get(collections::get(collections::groupBy(xs, kf, vf), 1), 0)))\n\
         END SUB\n",
    );
    let string_frees = loop_item_frees(&string_value, "_mfb_fn_main");
    let integer_frees = loop_item_frees(&integer_value, "_mfb_fn_main");
    // TWO more, and the decomposition is the point. With `V = String` the fast
    // path owns the projected value twice over:
    //
    //   * the block `valFn` RETURNED, freed by the `transform` that produced the
    //     values list — this fix;
    //   * the block the bucket loop materialises out of that list and copies into
    //     the bucket, freed by `func_group_by.rs`'s own
    //     `free_collection_loop_item(val_slot, value_type)` — pre-existing.
    //
    // With `V = Integer` neither exists, and both `keyFn` transforms contribute
    // their `String` ARGUMENT free either way. Pre-fix the delta was 1: the bucket
    // free alone.
    assert_eq!(
        string_frees,
        integer_frees + 2,
        "`collections::groupBy` must free the `String` value its `valFn` returns \
         AND the one it materialises into the bucket (String value {string_frees}, \
         Integer value {integer_frees}); `keyFn` is the identical `Integer` \
         projection in both, so nothing else differs"
    );
}

/// `mapValues` is the one HOF that does NOT reach its callback through
/// `transform`: its `.mfb` body invokes `f(e.value)` directly, so the result is
/// owned by the ordinary statement-scope temp rather than by a loop-item free.
/// The measurement is therefore the temp and its drop, not `loop_item_free_size`.
///
/// Both instantiations decline the native fast path (which requires `V == U`), so
/// they differ in nothing but the callback's return type.
#[test]
fn map_values_owns_the_string_value_its_callback_returns() {
    let string_value = ncode("b569_mapvalues_string", MAP_VALUES_STRING);
    let float_value = ncode(
        "b569_mapvalues_float",
        "IMPORT io\n\
         IMPORT collections\n\
         FUNC deco(v AS Integer) AS Float\n  RETURN toFloat(v)\nEND FUNC\n\
         SUB main()\n\
        \x20 MUT xs AS Map OF Integer TO Integer = Map OF Integer TO Integer {}\n\
        \x20 xs = collections::set(xs, 1, 7)\n\
        \x20 io::print(toString(collections::get(collections::mapValues(xs, deco), 1)))\n\
         END SUB\n",
    );
    let string_body = mfb_hof("mapValues", "Integer$Integer$String");
    let float_body = mfb_hof("mapValues", "Integer$Integer$Float");
    assert_eq!(
        pending_temps(&string_value, &string_body),
        pending_temps(&float_value, &float_body) + 1,
        "`collections::mapValues` must register a statement-scope temp for the \
         `String` its callback returns (String {}, Float {})",
        pending_temps(&string_value, &string_body),
        pending_temps(&float_value, &float_body)
    );
    assert_eq!(
        string_drops(&string_value, &string_body),
        string_drops(&float_value, &float_body) + 1,
        "…and must actually FREE it: one more owned-String drop than the Float \
         instantiation (String {}, Float {})",
        string_drops(&string_value, &string_body),
        string_drops(&float_value, &float_body)
    );
}

/// The `mapValues` probe above, with the callback named `deco`.
const MAP_VALUES_STRING: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC deco(v AS Integer) AS String\n  RETURN toString(v)\nEND FUNC\n\
SUB main()\n\
\x20 MUT xs AS Map OF Integer TO Integer = Map OF Integer TO Integer {}\n\
\x20 xs = collections::set(xs, 1, 7)\n\
\x20 io::print(collections::get(collections::mapValues(xs, deco), 1))\n\
END SUB\n";

/// The same program with the callback renamed to `f` — which is also the name of
/// `__collections_mapValues`'s own callable PARAMETER.
const MAP_VALUES_STRING_SHADOWING: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC f(v AS Integer) AS String\n  RETURN toString(v)\nEND FUNC\n\
SUB main()\n\
\x20 MUT xs AS Map OF Integer TO Integer = Map OF Integer TO Integer {}\n\
\x20 xs = collections::set(xs, 1, 7)\n\
\x20 io::print(collections::get(collections::mapValues(xs, f), 1))\n\
END SUB\n";

/// A `Call` carries only its target's NAME, so `f(e.value)` inside
/// `__collections_mapValues` is indistinguishable by shape from a direct call to
/// a top-level `f`. Resolving it through the function table therefore made the
/// answer depend on whether the user happened to name their callback `f`: with
/// any other name the lookup missed and the result leaked; named `f` it hit a
/// function this call never reaches and — by luck, because that function also
/// returned a fresh `String` — got the right answer for the wrong reason.
///
/// The two programs below are the same program modulo that name, so their owner
/// counts must be equal. Pre-fix they were 1 and 2.
///
/// The luck is the point: a top-level `f` that returned a parameter BORROW would
/// have answered "this result aliases the caller's argument, do not free it" for
/// an indirect call that reaches something else entirely.
#[test]
fn an_indirect_callback_result_is_owned_regardless_of_a_shadowing_name() {
    let plain = ncode("b569_mapvalues_plain", MAP_VALUES_STRING);
    let shadowing = ncode("b569_mapvalues_shadow", MAP_VALUES_STRING_SHADOWING);
    let body = mfb_hof("mapValues", "Integer$Integer$String");
    assert_eq!(
        pending_temps(&plain, &body),
        pending_temps(&shadowing, &body),
        "who owns `f(e.value)`'s block must not depend on whether a top-level \
         function shares the callable parameter's name (plain {}, shadowing {})",
        pending_temps(&plain, &body),
        pending_temps(&shadowing, &body)
    );
    assert_eq!(
        string_drops(&plain, &body),
        string_drops(&shadowing, &body),
        "…and neither must how many times it is freed (plain {}, shadowing {})",
        string_drops(&plain, &body),
        string_drops(&shadowing, &body)
    );
}

/// The POSITIVE pin for bug-569, and the shape the whole change has to survive: a
/// callback that returns its own bare parameter. The block it hands back is the
/// one `free_collection_loop_item` released on the way in, so a second free is a
/// double free — and the arena turns a wrong `arena_free` into "Allocation
/// failed" at some later, unrelated allocation rather than a red assertion.
///
/// plan-86 K1 makes this safe by FORCING the callee to copy (a callback-referenced
/// function is excluded from the borrow elision), and bug-562 extends that to
/// every unprovable return. This pin says the free does not RELY on it: the loop
/// compares the returned pointer against the item it materialised — `reduce`'s
/// model — and skips the free on identity, so a callee that ever handed back the
/// item block still leaves exactly one free.
///
/// Read as a count: a bare-parameter callback must own its result exactly as many
/// times as an identity callback does, because after the callee's copy the two
/// shapes are the same shape.
#[test]
fn a_callback_that_returns_its_own_parameter_is_freed_exactly_once() {
    let bare = ncode(
        "b569_transform_bare_param",
        "IMPORT io\n\
         IMPORT collections\n\
         FUNC pick(s AS String) AS String\n  RETURN s\nEND FUNC\n\
         SUB main()\n\
        \x20 LET xs AS List OF String = [\"a\", \"bb\"]\n\
        \x20 io::print(collections::get(collections::transform(xs, pick), 0))\n\
         END SUB\n",
    );
    let identity = ncode(
        "b569_transform_identity",
        "IMPORT io\n\
         IMPORT collections\n\
         FUNC pick(s AS String) AS String\n  RETURN toString(s)\nEND FUNC\n\
         SUB main()\n\
        \x20 LET xs AS List OF String = [\"a\", \"bb\"]\n\
        \x20 io::print(collections::get(collections::transform(xs, pick), 0))\n\
         END SUB\n",
    );
    assert_eq!(
        loop_item_frees(&bare, "_mfb_fn_main"),
        loop_item_frees(&identity, "_mfb_fn_main"),
        "a `RETURN <param>` callback and a `RETURN toString(<param>)` callback \
         hand the loop the same kind of block — the callee copies for both — so \
         the loop must own the same number either way (bare {}, identity {})",
        loop_item_frees(&bare, "_mfb_fn_main"),
        loop_item_frees(&identity, "_mfb_fn_main")
    );
    assert!(
        flat_copies(&bare, "_mfb_fn_pick") >= 1,
        "…and the reason it is the same kind of block is plan-86 K1: a \
         callback-referenced `RETURN <param>` callee still copies. Without that \
         copy the loop would be freeing a block the source list owns"
    );
}
