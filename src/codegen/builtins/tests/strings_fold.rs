//! The four `strings::` members that fold a literal at compile time.
//!
//! `static_strings_package_string` turns `strings::upper("straße")` into the
//! string constant `"STRASSE"` while the program is being lowered — four
//! members do it (`upper`, `lower`, `caseFold`, `normalizeNfc`), each through
//! one `if let Some(value) = ...` at the top of its `lower`.
//!
//! Not one of those four `if let`s had ever been taken. Every fixture that calls
//! them calls them on a *variable*, which is what a program that does anything
//! interesting looks like — so the fold is exactly the branch a realistic corpus
//! never reaches, and each of `func_upper.rs`, `func_lower.rs` and
//! `func_case_fold.rs` sat at 82.93%, five uncovered lines each, all five being
//! that block.
//!
//! What the fold is worth is visible in the emitted plan rather than in the
//! answer. A folded call needs no case mapping at run time, so the Unicode
//! mapping tables never enter the binary at all: `_mfb_unicode_uppercase_entries`
//! alone is 25,280 bytes. The two programs below differ only in whether their
//! argument is a literal, and one carries those tables while the other does not.
//!
//! The literals are chosen so a *wrong* fold is visible. `ß` is the case that
//! separates a real Unicode implementation from a byte-wise one: it uppercases
//! to two characters, so `upper("straße")` is seven characters where the input
//! was six, and an ASCII-only fold would return the input unchanged.

use crate::codegen::engine::types::NativeCodePlan;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

/// Every argument is a literal, so every call folds.
const FOLDED: &str = "\
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::upper(\"straße\"))
  io::print(strings::lower(\"STRASSE\"))
  io::print(strings::caseFold(\"Straße\"))
  io::print(strings::normalizeNfc(\"e\\u{0301}\"))
  RETURN 0
END FUNC
";

/// The same four calls on a string that is only known at run time.
const UNFOLDED: &str = "\
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT s AS String = \"straße\"
  s = s & io::input(\"\")
  io::print(strings::upper(s))
  io::print(strings::lower(s))
  io::print(strings::caseFold(s))
  io::print(strings::normalizeNfc(s))
  RETURN 0
END FUNC
";

fn plan(source: &str) -> &'static NativeCodePlan {
    code_for_src_cached(source, CodeTarget::LinuxX86_64, Console)
}

/// The text of every string constant the plan emits.
fn string_constants(plan: &NativeCodePlan) -> Vec<&str> {
    plan.data_objects
        .iter()
        .filter(|object| object.symbol.starts_with("_mfb_str_"))
        .map(|object| object.value.as_str())
        .collect()
}

/// True when the plan carries the Unicode table named by `symbol`.
fn has_data_object(plan: &NativeCodePlan, symbol: &str) -> bool {
    plan.data_objects
        .iter()
        .any(|object| object.symbol == symbol)
}

/// Each of the four folds produces the Unicode-correct string.
///
/// Spelled out rather than compared against `unicode::backend`, which is what
/// the fold itself calls — asserting those agree would only prove the wiring.
/// These are the answers Unicode specifies: `ß` uppercases to `SS` and folds to
/// `ss`, and `e` + U+0301 composes to the single scalar U+00E9.
#[test]
fn a_literal_argument_folds_to_the_unicode_correct_constant() {
    let constants = string_constants(plan(FOLDED));
    for (call, expected) in [
        ("strings::upper(\"straße\")", "STRASSE"),
        ("strings::lower(\"STRASSE\")", "strasse"),
        ("strings::caseFold(\"Straße\")", "strasse"),
        ("strings::normalizeNfc(\"e\\u{0301}\")", "\u{e9}"),
    ] {
        assert!(
            constants.iter().any(|value| *value == expected),
            "{call} must fold to the constant {expected:?} at compile time; the \
             plan's string constants are {constants:?}"
        );
    }
}

/// A folded program carries no Unicode mapping table.
///
/// This is what the fold BUYS, and the only way to see it: the answer printed at
/// run time is the same either way. `_mfb_unicode_uppercase_entries` is 25,280
/// bytes on its own, and a program whose only case mapping is on a literal has
/// no reason to ship it.
#[test]
fn folding_keeps_the_unicode_tables_out_of_the_binary() {
    let folded = plan(FOLDED);
    let unfolded = plan(UNFOLDED);
    for table in [
        "_mfb_unicode_uppercase_entries",
        "_mfb_unicode_uppercase_sequences",
        "_mfb_unicode_nfd_entries",
    ] {
        assert!(
            !has_data_object(folded, table),
            "every `strings::` call in the folded program takes a literal, so it \
             does no case mapping at run time and must not carry `{table}`"
        );
        assert!(
            has_data_object(unfolded, table),
            "the unfolded program maps case at run time and must carry \
             `{table}`; if it does not, this test's two halves are no longer \
             distinguishable and the assertion above proves nothing"
        );
    }
}

/// A non-literal argument does NOT fold.
///
/// The mirror of the test above, from the constants' side: the unfolded program
/// must not contain a folded result, because there is nothing to fold. Without
/// it, a fold that fired on every argument — returning the *un*mapped input for
/// the ones it cannot evaluate — would still pass everything above.
#[test]
fn a_runtime_argument_leaves_no_folded_constant_behind() {
    let constants = string_constants(plan(UNFOLDED));
    assert!(
        !constants.iter().any(|value| *value == "STRASSE"),
        "nothing in the unfolded program can be folded, so `STRASSE` must not \
         appear as a constant; the constants are {constants:?}"
    );
}
