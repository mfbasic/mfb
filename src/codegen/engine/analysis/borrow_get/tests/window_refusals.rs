//! A by-name classification refusal (`borrow_get/by_name.rs`) no other row
//! reaches: an element update whose index local a callback holds by reference
//! (the callback could move the index between the `get` and the `set`, so the
//! update is not element-bound).

use super::*;

#[test]
fn an_element_update_whose_index_is_held_by_reference_is_not_element_bound() {
    let program = |hold: &str| {
        format!(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT j AS Integer = 0\n  \
             {hold}\n  LET p AS P = collections::get(xs, j)\n  \
             xs = collections::set(xs, j, WITH p {{ age := p.age + 1 }})\n  \
             RETURN len(xs)\nEND FUNC\n"
        )
    };
    let free = classify(&program("j = 1"));
    assert!(free.elements.contains_key("p"), "with a plain index it is");
    let held = classify(&program(
        "collections::forEach([1, 2], LAMBDA(v AS Integer) -> j = v)",
    ));
    assert!(
        !held.elements.contains_key("p"),
        "with the index captured by reference it is not"
    );
}
