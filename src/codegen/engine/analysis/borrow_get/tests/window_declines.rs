//! By-name classification declines (`borrow_get/by_name.rs`) for three statement
//! shapes the other rows do not build. An element binding is element-bound (its
//! update written straight into the list's element) only when every read of `p`
//! between the `get` and the write-back is a field read; each row pairs the shape
//! that qualifies with one that reads `p` WHOLE where it must not, which would let
//! the callee see the element while it is being rewritten.

use super::*;

fn program(between: &str) -> String {
    bound("LET", between)
}

/// [`program`] with `p` bound by `keyword` (`MUT` for a body that reassigns it).
fn bound(keyword: &str, between: &str) -> String {
    format!(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
         {keyword} p AS P = collections::get(xs, 0)\n{between}  RETURN len(xs)\nEND FUNC\n"
    )
}

/// A write-back split over an `IF`: the condition may read `p` only through its
/// fields.
#[test]
fn an_if_condition_that_reads_the_element_whole_is_not_element_bound() {
    let arms = "    xs = collections::set(xs, 0, WITH p { age := p.age + 1 })\n  ELSE\n    \
                xs = collections::set(xs, 0, WITH p { age := p.age + 2 })\n  END IF\n";
    let by_field = classify(&program(&format!("  IF p.age > 1 THEN\n{arms}")));
    assert!(
        by_field.elements.contains_key("p"),
        "a field test qualifies"
    );
    let whole = classify(&program(&format!("  IF f(p) > 1 THEN\n{arms}")));
    assert!(
        !whole.elements.contains_key("p"),
        "a call given p whole does not"
    );
}

/// The `p = WITH p { … }` statement before the write-back may read `p` only through
/// fields, beside the `WITH`'s own target.
#[test]
fn an_update_that_reads_the_element_whole_is_not_element_bound() {
    let by_field = classify(&bound(
        "MUT",
        "  p = WITH p { age := p.age + 1 }\n  xs = collections::set(xs, 0, p)\n",
    ));
    assert!(
        by_field.elements.contains_key("p"),
        "a field read qualifies"
    );
    let whole = classify(&bound(
        "MUT",
        "  p = WITH p { age := f(p) }\n  xs = collections::set(xs, 0, p)\n",
    ));
    assert!(
        !whole.elements.contains_key("p"),
        "a call given p whole does not"
    );
}

/// The statement that ends the window must write the element back into the same
/// container at the same index.
#[test]
fn a_window_that_ends_in_no_write_back_is_not_element_bound() {
    let other = classify(&program(
        "  xs = collections::set(xs, 1, WITH p { age := p.age + 1 })\n",
    ));
    assert!(
        !other.elements.contains_key("p"),
        "a write to another index is not this element's write-back"
    );
    let append = classify(&program("  xs = collections::append(xs, p)\n"));
    assert!(
        !append.elements.contains_key("p"),
        "an append is no write-back"
    );
}
