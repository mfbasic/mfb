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
