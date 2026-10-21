//! plan-147 hand-over, lowered: an owned collection argument is GIVEN to the
//! callee (`<base>$own<mask>` variant) when the caller has no further use for it,
//! and an argument that is `OP(x, ..)` on such a local is built by updating `x`'s
//! block in place before the hand-over. The analysis is unit-tested over NIR in
//! `engine/analysis/handover/tests`; these pin what the builder does with a site
//! (`engine/builder/builder_emit_helpers.rs:lower_call_argument`).

use super::lowered::{calls_on, slots_on};
use crate::testutil::CodeTarget;

const HELPERS: &str = "\
IMPORT collections
IMPORT io

FUNC consume(xs AS List OF Integer, v AS Integer) AS List OF Integer
  RETURN collections::append(xs, v)
END FUNC

FUNC peek(xs AS List OF Integer) AS Integer
  RETURN len(xs)
END FUNC
";

fn program(body: &str) -> String {
    format!("{HELPERS}\nFUNC main() AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
}

fn owned_calls(source: &str) -> Vec<String> {
    calls_on(source, "main", CodeTarget::LinuxX86_64)
        .into_iter()
        .filter(|call| call.contains("$own"))
        .collect()
}

/// `xs = consume(xs, 2)`: `xs` is dead after the call except as the result, so
/// the callee takes the block (`consume$own1`); reading `xs` afterwards keeps the
/// lent call.
#[test]
fn a_local_at_its_last_use_is_handed_over_and_a_live_one_is_lent() {
    let handed = program(
        "  MUT xs AS List OF Integer = [1]\n  xs = consume(xs, 2)\n  io::print(toString(len(xs)))",
    );
    assert!(
        !owned_calls(&handed).is_empty(),
        "{:?}",
        owned_calls(&handed)
    );
    let lent = program(
        "  MUT xs AS List OF Integer = [1]\n  LET ys AS List OF Integer = consume(xs, 2)\n  io::print(toString(len(xs) + len(ys)))",
    );
    assert!(owned_calls(&lent).is_empty(), "{:?}", owned_calls(&lent));
}

/// `consume(append(xs, 5), 3)` with `xs` dead afterwards: the argument is built by
/// appending to `xs`'s own block (an in-place self-update whose result stays in
/// `xs`'s slot), then handed over; with `xs` read afterwards it is an ordinary
/// fresh temporary.
#[test]
fn an_update_argument_at_the_last_use_appends_in_place_then_hands_over() {
    let in_place = program(
        "  MUT xs AS List OF Integer = [1]\n  LET ys AS List OF Integer = consume(collections::append(xs, 5), 3)\n  io::print(toString(len(ys)))",
    );
    let calls = owned_calls(&in_place);
    assert!(!calls.is_empty(), "{calls:?}");
    let slots = slots_on(&in_place, "main", CodeTarget::LinuxX86_64);
    assert!(
        slots.iter().any(|slot| slot.starts_with("append_inplace")),
        "the argument's append is the in-place arm: {slots:?}"
    );
    let live = program(
        "  MUT xs AS List OF Integer = [1]\n  LET n AS Integer = len(consume(collections::append(xs, 5), 3))\n  io::print(toString(n + len(xs)))",
    );
    let slots = slots_on(&live, "main", CodeTarget::LinuxX86_64);
    assert!(
        !slots.iter().any(|slot| slot.starts_with("append_inplace")),
        "a live `xs` must not be appended to in place: {slots:?}"
    );
}
