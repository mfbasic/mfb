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
