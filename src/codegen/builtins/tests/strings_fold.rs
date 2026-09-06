//! The four `strings::` members that fold a literal at compile time.
//!
//! `nir::constfold::native_strings_package_static_string_value` turns
//! `strings::upper("straße")` into the string constant `"STRASSE"` while the
//! plan is being laid out — four members do it (`upper`, `lower`, `caseFold`,
//! `normalizeNfc`), and none of the four had a test.
//!
//! **The fold names one place, and it used to name two.** Each of the four
//! `func_*.rs` bodies opened with its own copy of the same fold, through a
//! `static_strings_package_string` helper — and every one of those copies was
//! unreachable, because the NIR folder had already replaced the call before
//! codegen ran. Measured rather than reasoned: an `eprintln!` at the top of
//! `func_upper::lower` shows it is never called at all for
//! `strings::upper("literal")`, and called exactly once (fold declining,
//! correctly) for `strings::upper(s)`. All four copies and the helper are gone,
//! and this file is what remains to pin the behaviour — including the one shape
//! where the two folders could have differed, a `LET`-bound literal, which they
//! resolved through two different constant maps.
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

/// The argument is a local the program never writes again, so the folder has to
/// reach THROUGH the binding to see the literal.
///
/// This is the shape the deleted codegen-side copy of the fold handled, and the
/// only one where the two folders could have differed: they resolve a `Local`
/// through two different constant maps — the NIR folder through the plan's
/// `constants`, the codegen one through `CodeBuilder::locals[..].constant`. With
/// one folder left, this pins that the surviving map still sees a `LET`.
const LET_BOUND: &str = "\
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET a AS String = \"straße\"
  LET b AS String = \"STRASSE\"
  LET c AS String = \"Straße\"
  LET d AS String = \"e\\u{0301}\"
  io::print(strings::upper(a))
  io::print(strings::lower(b))
  io::print(strings::caseFold(c))
  io::print(strings::normalizeNfc(d))
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

/// A `LET`-bound literal folds exactly as the literal itself does.
///
/// Constant propagation through a binding is the fold's whole reach: a program
/// that names its strings — which is every program anyone writes — gets nothing
/// from a folder that only matches a literal spelled inside the call. Asserted
/// on both sides, because a folder that reached through the binding but produced
/// the *unmapped* text would satisfy either half alone: the four results must be
/// present as constants, and the Unicode tables must still be absent.
#[test]
fn a_let_bound_literal_folds_through_the_binding() {
    let plan = plan(LET_BOUND);
    let constants = string_constants(plan);
    for (call, expected) in [
        ("strings::upper(a)", "STRASSE"),
        ("strings::lower(b)", "strasse"),
        ("strings::caseFold(c)", "strasse"),
        ("strings::normalizeNfc(d)", "\u{e9}"),
    ] {
        assert!(
            constants.iter().any(|value| *value == expected),
            "{call} names a local bound to a literal and never written again, so \
             it must fold to {expected:?} at compile time; the plan's string \
             constants are {constants:?}"
        );
    }
    assert!(
        !has_data_object(plan, "_mfb_unicode_uppercase_entries"),
        "every case mapping in this program was folded, so it must not carry \
         `_mfb_unicode_uppercase_entries` -- if it does, the fold returned the \
         text unchanged and left the real mapping to run time"
    );
}
