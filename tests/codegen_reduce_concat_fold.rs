//! plan-121-G: a `collections::reduce` with a String accumulator and a
//! self-concat reducer is rewritten into the loop it is sugar for.
//!
//! ## Why this file has to exist
//!
//! `p121g-reduce-accumulator-rt` proves the fifteen observable answers are
//! unchanged — and they were unchanged *before* this optimization too, because
//! the old lowering was correct and merely O(N²). So a green fixture cannot
//! distinguish "the rewrite fired" from "the rewrite never ran".
//!
//! It is not a hypothetical distinction. The first version of this pass compiled,
//! kept every fixture green, and did **nothing**: it declined `len(reduce(...))`
//! — the benchmark's own shape — because it treated the enclosing `len` call as
//! the first effectful node instead of descending into its arguments. Only the
//! measurement caught it. These assertions are what make that visible next time.
//!
//! The instrument: when the rewrite fires, `lower_collection_reduce_impl` is
//! never reached, so its `reduce_call_loop` label is absent. When it declines,
//! the label is there.

mod common;

use serde_json::Value;

const TARGET: &str = "linux-x86_64";

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

/// `reducer` is spliced in as the reducer's body; `call` as the folding call.
fn src(reducer: &str, call: &str) -> String {
    format!(
        "IMPORT collections\n\
         FUNC catRight(acc AS String, x AS String) AS String\n\
        \x20 {reducer}\n\
         END FUNC\n\
         FUNC fold(xs AS List OF String) AS Integer\n\
        \x20 RETURN len({call})\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 LET xs AS List OF String = [\"a\", \"b\"]\n\
        \x20 RETURN fold(xs)\n\
         END FUNC\n"
    )
}

#[test]
fn a_self_concat_reduce_is_rewritten_into_a_loop() {
    let plan = ncode(
        "p121g_taken",
        &src("RETURN acc & x", "collections::reduce(xs, \"\", catRight)"),
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_fold", "reduce_call_loop"),
        0,
        "plan-121-G: `reduce` with a String accumulator and a `RETURN acc & x` \
         reducer must be rewritten into the loop it is sugar for, so the native \
         fold is never emitted. Calling the reducer N times is what makes this \
         O(N^2) -- each call returns a fresh tight len(acc)+len(x) string."
    );
}

/// The benchmark's own shape: the call is nested inside `len(...)`. A
/// statement-level rewrite would miss it, and the first version of this pass did.
#[test]
fn the_rewrite_reaches_a_call_nested_inside_len() {
    let plan = ncode(
        "p121g_nested",
        &src("RETURN acc & x", "collections::reduce(xs, \"\", catRight)"),
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_fold", "reduce_call_loop"),
        0,
        "plan-121-G: the fold sits inside `len(...)`, which is exactly how the \
         benchmark row spells it. A call's ARGUMENTS are evaluated before the \
         call, so a fold inside `len` is still the first effectful node and is \
         hoistable. Treating the enclosing `len` as the first effectful node \
         declines it -- which is the bug the measurement caught."
    );
}

#[test]
fn reduce_right_is_rewritten_too() {
    let plan = ncode(
        "p121g_right",
        &src(
            "RETURN acc & x",
            "collections::reduceRight(xs, \"\", catRight)",
        ),
    );
    assert_eq!(
        label_count(&plan, "_mfb_fn_fold", "reduce_call_loop"),
        0,
        "plan-121-G: `reduceRight` is rewritten as well. Phase 1's fixture \
         established by TEST that a self-concat `reduceRight` is still a \
         left-append (`543210`), just fed in reverse -- so it shares the loop \
         body and differs only in iteration direction."
    );
}

// --------------------------------------------------------------------------
// Declines. Each is a shape the rewrite must refuse, and each refusal is what
// keeps an answer correct rather than merely fast.
// --------------------------------------------------------------------------

#[test]
fn a_left_concat_reducer_declines() {
    let plan = ncode(
        "p121g_left",
        &src("RETURN x & acc", "collections::reduce(xs, \"\", catRight)"),
    );
    assert!(
        label_count(&plan, "_mfb_fn_fold", "reduce_call_loop") >= 1,
        "plan-121-G: `x & acc` PREPENDS. An append-into-a-buffer cannot express \
         it, and rewriting it would reverse the result -- Phase 1 pins that this \
         shape answers `543210` where `acc & x` answers `012345`."
    );
}

#[test]
fn a_reducer_that_reads_the_accumulator_declines() {
    let plan = ncode(
        "p121g_reads",
        &src(
            "RETURN acc & toString(len(acc)) & x",
            "collections::reduce(xs, \"\", catRight)",
        ),
    );
    assert!(
        label_count(&plan, "_mfb_fn_fold", "reduce_call_loop") >= 1,
        "plan-121-G: this reducer OBSERVES the accumulator mid-fold, so its \
         answer depends on the accumulator's value at each step. Rewriting it \
         into an append would still be correct here, but the condition is \
         deliberately narrow: `hir_mentions` refuses any right operand naming \
         the accumulator rather than reasoning case by case."
    );
}

#[test]
fn a_reducer_that_ignores_the_accumulator_declines() {
    let plan = ncode(
        "p121g_ignores",
        &src("RETURN x", "collections::reduce(xs, \"\", catRight)"),
    );
    assert!(
        label_count(&plan, "_mfb_fn_fold", "reduce_call_loop") >= 1,
        "plan-121-G: the result is the LAST element, not a concatenation. The \
         body is not a concat at all, so there is nothing to append."
    );
}

/// An Integer accumulator must keep the native fold byte-for-byte: it is already
/// grade A (1.73–1.82×), and `try_inplace_concat_assign` has a grown buffer for
/// `String` only.
#[test]
fn an_integer_accumulator_declines() {
    let plan = ncode(
        "p121g_int",
        "IMPORT collections\n\
         FUNC addFn(acc AS Integer, x AS Integer) AS Integer\n\
        \x20 RETURN acc + x\n\
         END FUNC\n\
         FUNC fold(xs AS List OF Integer) AS Integer\n\
        \x20 RETURN collections::reduce(xs, 0, addFn)\n\
         END FUNC\n\
         FUNC main AS Integer\n\
        \x20 LET xs AS List OF Integer = [1, 2]\n\
        \x20 RETURN fold(xs)\n\
         END FUNC\n",
    );
    assert!(
        label_count(&plan, "_mfb_fn_fold", "reduce_call_loop") >= 1,
        "plan-121-G: an Integer fold must keep the native `reduce`. It is not \
         quadratic (each step is a register add, not a fresh allocation) and \
         `try_inplace_concat_assign` -- the whole mechanism this rewrite exists \
         to reach -- is String-only."
    );
}
