//! Element-update shapes beyond the bug-784 benchmark rows (`element_update.rs`):
//! the single-expression update (`xs = set(xs, i, WITH get(xs, i) { .. })`,
//! bug-689), a global container, a nested `WITH` that replaces a `String`
//! inside the element, a destination call followed by a fallible update, and
//! re-bound `get` names. Each is lowered for Linux x86-64 and its label stems
//! (serials dropped) asserted: an element update that stopped being recognised
//! falls back to the general `WITH` + `set`, which prints the same answer but
//! rebuilds the whole element.

use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_for_src_lowered, code_function, CodeTarget};

const PRELUDE: &str = "\
IMPORT collections
IMPORT io
IMPORT math

TYPE Inner
  a AS Integer
  b AS Integer
  name AS String
END TYPE

TYPE Holder
  id AS Integer
  label AS String
  inner AS Inner
  touches AS Integer
END TYPE

FUNC newInner(i AS Integer) AS Inner
  RETURN Inner[a := i, b := 0, name := \"in-\" & toString(i)]
END FUNC

FUNC stepIn(w AS Inner, i AS Integer, tag AS String) AS Inner
  RETURN WITH w { a := w.a + 1, b := w.b + i, name := tag }
END FUNC

FUNC touchesOf(h AS Holder) AS Integer
  RETURN h.touches
END FUNC

FUNC holders() AS List OF Holder
  MUT hs AS List OF Holder = []
  FOR k = 0 TO 7
    hs = collections::append(hs, Holder[id := k, label := \"h-\" & toString(k), inner := newInner(k), touches := 0])
  NEXT
  RETURN hs
END FUNC
";

/// Label stems of function `name` in `body` lowered after the prelude.
fn stems_of(body: &str, name: &str) -> Vec<String> {
    let source = format!("{PRELUDE}\n{body}");
    let plan = code_for_src_lowered(&source, CodeTarget::LinuxX86_64, Console);
    let function = code_function(plan, name);
    let mut stems: Vec<String> = Stream::of(function)
        .labels()
        .into_iter()
        .map(|(_, label)| {
            label
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .trim_end_matches('_')
                .to_string()
        })
        .collect();
    stems.sort();
    stems.dedup();
    stems
}

fn has(stems: &[String], stem: &str) -> bool {
    stems.iter().any(|s| s == stem)
}

const MAIN_OPEN: &str = "FUNC main() AS Integer\n  MUT hs AS List OF Holder = holders()\n";
const MAIN_CLOSE: &str = "  io::print(toString(len(hs)))\n  RETURN 0\nEND FUNC\n";

fn main_with(loop_body: &str) -> String {
    format!("{MAIN_OPEN}  FOR k = 0 TO 7\n{loop_body}\n  NEXT\n{MAIN_CLOSE}")
}

/// Whether `main` carries a stack object registered for a hidden
/// single-expression element binding (`$element<n>`).
fn has_hidden_element_slot(body: &str) -> bool {
    let source = format!("{PRELUDE}\n{body}");
    let plan = code_for_src_cached(&source, CodeTarget::LinuxX86_64, Console);
    code_function(plan, "main")
        .stack_slots
        .iter()
        .any(|slot| slot.type_.starts_with("$element"))
}

/// Whether `main` of the full `source` keeps the destination call's result.
fn keeps_into_result(source: &str) -> bool {
    let plan = code_for_src_lowered(source, CodeTarget::LinuxX86_64, Console);
    let main = code_function(plan, "main");
    Stream::of(main)
        .labels()
        .iter()
        .any(|(_, label)| label.starts_with("into_result_kept"))
}

/// bug-689: `xs = set(xs, i, WITH get(xs, i) { .. })` binds a hidden borrowed
/// element and updates it; a `WITH` that hands the element whole to a call
/// (`touchesOf(get(hs, k))`, which could keep the alias) is ruled out and takes the general rebuild.
#[test]
fn the_single_expression_update_binds_a_hidden_element() {
    let single = main_with("    hs = collections::set(hs, k, WITH collections::get(hs, k) { touches := collections::get(hs, k).touches + 1 })");
    assert!(has_hidden_element_slot(&single));
    let nested = main_with("    hs = collections::set(hs, k, WITH collections::get(hs, k) { inner := stepIn(collections::get(hs, k).inner, k, \"x\"), touches := 1 })");
    assert!(has_hidden_element_slot(&nested));
    let reads_container = main_with(
        "    hs = collections::set(hs, k, WITH collections::get(hs, k) { touches := touchesOf(collections::get(hs, k)) })",
    );
    assert!(!has_hidden_element_slot(&reads_container));
}

/// The same over a global container: admitted while no update can store to the
/// global, refused when an update calls something that does.
#[test]
fn the_single_expression_update_over_a_global_needs_no_store_to_it() {
    let global = |touches: &str| {
        format!(
            "MUT gs AS List OF Holder = []\n\
             FUNC reset() AS Integer\n  gs = []\n  RETURN 0\nEND FUNC\n\
             FUNC main() AS Integer\n  gs = holders()\n  FOR k = 0 TO 7\n    \
             gs = collections::set(gs, k, WITH collections::get(gs, k) {{ touches := {touches} }})\n  NEXT\n  \
             io::print(toString(len(gs)))\n  RETURN 0\nEND FUNC\n"
        )
    };
    assert!(has_hidden_element_slot(&global(
        "collections::get(gs, k).touches + 1"
    )));
    assert!(!has_hidden_element_slot(&global("reset()")));
}

/// A `WITH` that replaces a `String` inside the nested record of a bound
/// element writes into the element where the size is unchanged, and otherwise
/// runs the element's write-back.
#[test]
fn a_nested_string_replace_has_an_in_place_branch_and_a_write_back() {
    let body = "    LET h AS Holder = collections::get(hs, k)\n    hs = collections::set(hs, k, WITH h { inner := WITH h.inner { name := \"tg\" & toString(k), a := 1 }, touches := h.touches + 1 })";
    let stems = stems_of(&main_with(body), "main");
    assert!(has(&stems, "element_replace_write_back"));
    assert!(has(&stems, "element_replace_done"));
}

/// The destination call (`stepIn` writing straight into the element) is kept
/// only when nothing after it can observe or be hurt by that write.
#[test]
fn the_destination_call_is_gated_on_what_follows_it() {
    let kept = |touches: &str, tail: &str| {
        let body = format!(
            "    LET h AS Holder = collections::get(hs, k)\n    hs = collections::set(hs, k, WITH h {{ inner := stepIn(h.inner, k, \"y\"), touches := {touches} }})"
        );
        let source = format!(
            "{PRELUDE}\nMUT gn AS Integer = 0\n\
             FUNC main() AS Integer\n  MUT hs AS List OF Holder = holders()\n  LET d AS Integer = len(hs) - 8\n  LET q AS Integer = 3\n  \
             FOR k = 0 TO 7\n{body}\n  NEXT\n  io::print(toString(len(hs)))\n  RETURN 0\n{tail}END FUNC\n"
        );
        keeps_into_result(&source)
    };
    // Scalars and infallible built-ins over them cannot see the destination.
    assert!(kept("-q + len(\"abc\") + h.id", ""));
    assert!(kept("gn", ""));
    // A fallible later update is fine in a plain function...
    assert!(kept("h.touches / d", ""));
    // ...but not where the failure is observable (a function-level TRAP).
    assert!(!kept(
        "h.touches / d",
        "  TRAP(e)\n    RETURN 1\n  END TRAP\n"
    ));
    // Reading the owner whole or the container sees the destination.
    assert!(!kept("h.inner.a", ""));
    assert!(!kept("len(hs)", ""));
}

/// The destination call over a global container.
#[test]
fn the_destination_call_over_a_global_container() {
    let kept = |touches: &str| {
        let source = format!(
            "{PRELUDE}\nMUT gs AS List OF Holder = []\nMUT gn AS Integer = 0\n\
             FUNC main() AS Integer\n  gs = holders()\n  FOR k = 0 TO 7\n    LET h AS Holder = collections::get(gs, k)\n    \
             gs = collections::set(gs, k, WITH h {{ inner := stepIn(h.inner, k, \"y\"), touches := {touches} }})\n  NEXT\n  \
             io::print(toString(len(gs)))\n  RETURN 0\nEND FUNC\n"
        );
        keeps_into_result(&source)
    };
    assert!(kept("gn"));
    // A fallible later update cannot be hidden when the container is global.
    assert!(!kept("h.touches / (gn + 1)"));
    assert!(!kept("len(gs)"));
}

/// A name bound twice: the element-update loop keeps its element update beside
/// a read-only re-binding in a later loop (bug-784).
#[test]
fn a_rebound_get_binding_keeps_its_element_update() {
    let body = "  FOR i = 0 TO 15\n    LET k AS Integer = i MOD 8\n    LET h AS Holder = collections::get(hs, k)\n    \
         hs = collections::set(hs, k, WITH h { inner := stepIn(h.inner, i, \"t\"), touches := h.touches + 1 })\n  NEXT\n  \
         MUT sumv AS Integer = 0\n  FOR k = 0 TO 7\n    LET h AS Holder = collections::get(hs, k)\n    \
         sumv = sumv + h.id + h.touches + len(h.label)\n  NEXT\n  io::print(toString(sumv))\n  RETURN 0\nEND FUNC\n";
    let source = format!("{MAIN_OPEN}{body}");
    let stems = stems_of(&source, "main");
    assert!(has(&stems, "element_replace_write_back"));
}

/// The negative: a binding whose container is written while it is live stays
/// a copy, so no element update is formed for any of the bindings.
#[test]
fn an_unsafe_rebinding_forms_no_element_update() {
    let source = "FUNC main() AS Integer\n  MUT hs AS List OF Holder = holders()\n  MUT gs AS List OF Holder = holders()\n  \
        MUT out AS Integer = 0\n  FOR j = 0 TO 2\n    LET h AS Holder = collections::get(hs, j)\n    out = out + h.touches\n  NEXT\n  \
        FOR j = 0 TO 2\n    LET h AS Holder = collections::get(gs, j)\n    \
        gs = collections::set(gs, j, Holder[id := 99, label := \"r\", inner := newInner(5), touches := 7])\n    out = out + h.id\n  NEXT\n  \
        io::print(toString(out))\n  RETURN 0\nEND FUNC\n";
    let stems = stems_of(source, "main");
    assert!(!has(&stems, "element_replace_write_back"));
}

fn holder_update(update: &str) -> Vec<String> {
    let body = format!(
        "    LET h AS Holder = collections::get(hs, k)\n    hs = collections::set(hs, k, WITH h {{ {update} }})"
    );
    stems_of(&main_with(&body), "main")
}

/// A replaced `String` beside a nested scalar `WITH`: the size-change branch's
/// write-back re-reads the nested update's values from their spill slots.
#[test]
fn a_string_replace_beside_a_nested_scalar_update_writes_back_from_slots() {
    let stems =
        holder_update("label := \"x\" & toString(k), inner := WITH h.inner { a := h.inner.a + 1 }");
    assert!(has(&stems, "element_replace_write_back"));
    assert!(has(&stems, "element_replace_done"));
}

/// A `String` replaced one level down a path (`inner := WITH h.inner { name :=
/// .. }` as the only update): the write-back rebuilds the path's levels around
/// the spilled value.
#[test]
fn a_string_replace_down_a_path_writes_back_through_the_path() {
    let stems = holder_update("inner := WITH h.inner { name := \"x\" & toString(k) }");
    assert!(has(&stems, "element_replace_write_back"));
    assert!(has(&stems, "field_replace_wloop"));
}

const BAG: &str = "\
TYPE Bag
  id AS Integer
  items AS List OF Integer
  count AS Integer
END TYPE

FUNC bags() AS List OF Bag
  MUT bs AS List OF Bag = []
  FOR k = 0 TO 7
    bs = collections::append(bs, Bag[id := k, items := [k], count := 0])
  NEXT
  RETURN bs
END FUNC
";

fn bag_stems(update: &str) -> Vec<String> {
    let body = format!(
        "{BAG}\nFUNC main() AS Integer\n  MUT bs AS List OF Bag = bags()\n  FOR k = 0 TO 7\n    LET h AS Bag = collections::get(bs, k)\n    bs = collections::set(bs, k, WITH h {{ {update} }})\n  NEXT\n  io::print(toString(len(bs)))\n  RETURN 0\nEND FUNC\n"
    );
    stems_of(&body, "main")
}

/// A collection field is not stored over: the element's `WITH` falls back to
/// `set(bs, k, WITH h { .. })`, which rebuilds the payload, while a scalar-only
/// update of the same element writes its bytes where they lie.
#[test]
fn a_collection_field_update_takes_the_set_write_back() {
    let scalar = bag_stems("count := h.count + 1");
    assert!(!has(&scalar, "collection_copy_inline_wloop"));
    for update in [
        "items := collections::append(h.items, k)",
        "items := collections::append(h.items, k), count := h.count + 1",
        "items := [k, k]",
    ] {
        let general = bag_stems(update);
        assert!(has(&general, "collection_copy_inline_wloop"), "{update}");
    }
}

/// A replaced field whose new value is not a call (a record built in place) has
/// no destination variant: the update is the element replace alone.
#[test]
fn a_replace_that_is_not_a_call_keeps_no_destination_result() {
    let stems = holder_update("inner := Inner[a := k, b := 0, name := \"z\"], touches := 1");
    assert!(has(&stems, "element_replace_write_back"));
    assert!(!has(&stems, "into_result_kept"));
}

/// A later update the destination check cannot prove independent of the write
/// (a conversion it does not look through) refuses the destination call.
#[test]
fn a_later_update_the_check_cannot_see_through_refuses_the_destination_call() {
    let stems = holder_update(
        "inner := stepIn(h.inner, k, \"y\"), touches := math::floor(toFloat(k) * 2.0)",
    );
    assert!(has(&stems, "element_replace_write_back"));
    assert!(!has(&stems, "into_result_kept"));
}

/// The destination argument must be the replaced field itself: `second :=
/// stepIn(p.first, ..)` reads another field, so it takes the plain call.
#[test]
fn a_call_whose_argument_is_another_field_takes_no_destination() {
    let source = "\
TYPE Duo
  first AS Inner
  second AS Inner
  n AS Integer
END TYPE

FUNC pairs() AS List OF Duo
  MUT ps AS List OF Duo = []
  FOR k = 0 TO 7
    ps = collections::append(ps, Duo[first := newInner(k), second := newInner(k + 1), n := 0])
  NEXT
  RETURN ps
END FUNC

FUNC main() AS Integer
  MUT ps AS List OF Duo = pairs()
  FOR k = 0 TO 7
    LET p AS Duo = collections::get(ps, k)
    ps = collections::set(ps, k, WITH p { second := stepIn(p.first, k, \"y\"), n := 1 })
  NEXT
  io::print(toString(len(ps)))
  RETURN 0
END FUNC
";
    let stems = stems_of(source, "main");
    assert!(has(&stems, "element_replace_write_back"));
    assert!(!has(&stems, "into_result_kept"));
}

/// Whether `main` of `source` holds a stack object of the given kind.
fn has_slot(source: &str, kind: &str) -> bool {
    let plan = code_for_src_cached(source, CodeTarget::LinuxX86_64, Console);
    code_function(plan, "main")
        .stack_slots
        .iter()
        .any(|slot| slot.type_ == kind)
}

fn bucket_program(between: &str) -> String {
    format!(
        "IMPORT collections\nIMPORT io\n\
         FUNC main() AS Integer\n  MUT m AS Map OF String TO List OF Integer = Map OF String TO List OF Integer {{}}\n  \
         LET empty AS List OF Integer = []\n  FOR k = 0 TO 7\n    MUT bucket AS List OF Integer = collections::getOr(m, \"a\", empty)\n{between}    \
         bucket = collections::append(bucket, k)\n    m = collections::set(m, \"a\", bucket)\n  NEXT\n  \
         io::print(toString(len(m)))\n  RETURN 0\nEND FUNC\n"
    )
}

/// bug-770 C: `MUT b = getOr(m, k, [])` whose next statement is
/// `b = append(b, item)` takes its owned copy with growth headroom; a statement
/// between the two keeps the tight copy.
#[test]
fn a_bucket_copy_followed_by_its_append_has_headroom() {
    assert!(has_slot(&bucket_program(""), "headroom_copy_result"));
    assert!(!has_slot(
        &bucket_program("    io::print(\"x\")\n"),
        "headroom_copy_result"
    ));
}

/// bug-766: the update split over a `LET` — `LET u = WITH h { .. }` then
/// `hs = set(hs, k, u)` — is the same element update: `u` is never bound, its
/// one read is the write-back.
#[test]
fn a_let_bound_with_before_its_set_is_an_element_update() {
    let body = "    LET h AS Holder = collections::get(hs, k)\n    \
        LET u AS Holder = WITH h { inner := stepIn(h.inner, k, \"y\"), touches := 1 }\n    \
        hs = collections::set(hs, k, u)";
    let stems = stems_of(&main_with(body), "main");
    assert!(has(&stems, "element_replace_write_back"));
    assert!(has(&stems, "into_result_kept"));
}

fn string_program(second_loop: &str) -> String {
    format!(
        "IMPORT collections\nIMPORT io\n\
         FUNC main() AS Integer\n  MUT ss AS List OF String = [\"a\", \"b\", \"c\"]\n  \
         FOR k = 0 TO 2\n    LET s AS String = collections::get(ss, k)\n    ss = collections::set(ss, k, s)\n  NEXT\n{second_loop}  \
         io::print(toString(len(ss)))\n  RETURN 0\nEND FUNC\n"
    )
}

/// A `get` of a `String` element written back to the same slot is not an
/// element-bound record: the binding owns its copy, so the update loop lowers
/// with no hidden element slot and no element-replace labels, rebound or not.
#[test]
fn a_string_element_write_back_is_no_element_binding() {
    for source in [
        string_program(""),
        string_program(
            "  FOR k = 0 TO 2\n    LET s AS String = collections::get(ss, k)\n    io::print(s)\n  NEXT\n",
        ),
    ] {
        assert!(!has(&stems_of(&source, "main"), "element_replace_write_back"));
        assert!(!has_hidden_element_slot(&source));
    }
}

/// bug-689: the single-expression update reads the element only through field
/// reads; the rewrite must see through every value kind that can wrap such a read
/// (list/set/map literals, a record literal, a closure, a union wrap, a unary), or
/// the statement quietly falls back to the copying `WITH` + `set`.
#[test]
fn the_single_expression_rewrite_sees_through_every_value_kind() {
    let body = "\
TYPE Dot
  x AS Integer
END TYPE

UNION Shape
  Dot
  Inner
END UNION

FUNC shapeSize(s AS Shape) AS Integer
  MATCH s
    CASE Dot(d)
      RETURN d.x
    CASE Inner(i)
      RETURN i.a
  END MATCH
END FUNC

FUNC apply(f AS FUNC(Integer) AS Integer, v AS Integer) AS Integer
  RETURN f(v)
END FUNC

FUNC main() AS Integer
  MUT hs AS List OF Holder = holders()
  LET base AS Integer = 2
  FOR k = 0 TO 7
    hs = collections::set(hs, k, WITH collections::get(hs, k) { touches := collections::get(hs, k).touches + len([1, 2]) + len(Set OF Integer { 1 }) + len(Map OF String TO Integer { \"a\" := 1 }) + (-collections::get(hs, k).id) + apply(LAMBDA(x AS Integer) -> x + base, 1) + len(Inner[a := 1, b := 2, name := \"n\"].name) + shapeSize(Dot[3]) })
  NEXT
  io::print(toString(len(hs)))
  RETURN 0
END FUNC
";
    assert!(has_hidden_element_slot(body));
}

/// The destination call stays on under a function-level `TRAP` only when every
/// later update of the element provably cannot fail (a comparison, `NOT`, a field
/// read, an infallible built-in); arithmetic can overflow, so an update after the
/// call that does arithmetic declines it.
#[test]
fn only_a_later_update_that_cannot_fail_keeps_the_destination_call_under_a_trap() {
    let kept = |update: &str| {
        let source = format!(
            "{PRELUDE}
TYPE Gate
  inner AS Inner
  ok AS Boolean
  n AS Integer
END TYPE

FUNC gates() AS List OF Gate
  MUT gs AS List OF Gate = []
  FOR k = 0 TO 7
    gs = collections::append(gs, Gate[inner := newInner(k), ok := FALSE, n := k])
  NEXT
  RETURN gs
END FUNC

FUNC main() AS Integer
  MUT gs AS List OF Gate = gates()
  FOR k = 0 TO 7
    LET g AS Gate = collections::get(gs, k)
    gs = collections::set(gs, k, WITH g {{ inner := stepIn(g.inner, k, \"y\"), {update} }})
  NEXT
  io::print(toString(len(gs)))
  RETURN 0

  TRAP(e)
    RETURN 1
  END TRAP
END FUNC
"
        );
        keeps_into_result(&source)
    };
    assert!(kept("ok := g.n > 2"));
    assert!(kept("ok := NOT (g.n > 2)"));
    assert!(kept("ok := g.ok"));
    assert!(kept("n := len(\"abc\")"));
    assert!(!kept("n := g.n + 1"));
    assert!(!kept("n := -g.n"));
}
