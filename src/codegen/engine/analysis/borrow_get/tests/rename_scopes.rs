//! bug-784's per-binding classification (`collect_borrow_gets` over a renamed copy)
//! for the scopes and site shapes `rename_forms.rs` leaves out: a function-level
//! `TRAP` handler (its own scope, binding the error), an element binding whose
//! write-back sits in both arms of an `IF` (two sites, both mapped back from the
//! copy to the real ops), and a re-bound name that is address-taken (so under
//! each of its unique names too).

use super::*;

/// A binding in the handler is a binding of its own: borrowed like the two in the
/// body, each from its own container.
#[test]
fn a_binding_in_a_function_level_handler_is_classified_on_its_own() {
    let got = rebound_roles(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT ys AS List OF P = mk()\n  \
             MUT zs AS List OF P = mk()\n  MUT t AS Integer = 0\n  FOR j = 0 TO 1\n    \
             LET h AS P = collections::get(xs, j)\n    t = t + h.age\n  NEXT\n  FOR j = 0 TO 1\n    \
             LET h AS P = collections::get(ys, j)\n    t = t + h.age\n  NEXT\n  RETURN t\n\
             TRAP(e)\n  LET h AS P = collections::get(zs, 0)\n  RETURN h.age\nEND TRAP\nEND FUNC\n",
        "h",
    );
    assert_eq!(
        got,
        roles(&[("xs", "borrow"), ("ys", "borrow"), ("zs", "borrow")])
    );
}

/// An element binding with a write-back in each arm of an `IF` keeps both sites
/// when its name is bound again elsewhere: the per-binding pass maps each site of
/// the renamed copy back to the real statement.
#[test]
fn an_element_binding_with_two_write_backs_survives_a_rebind() {
    let module = lower(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT t AS Integer = 0\n  \
             FOR j = 0 TO 1\n    MUT h AS P = collections::get(xs, j)\n    \
             IF j > 0 THEN\n      h = WITH h { age := h.age + 1 }\n      \
             xs = collections::set(xs, j, h)\n    ELSE\n      h = WITH h { age := h.age + 2 }\n      \
             xs = collections::set(xs, j, h)\n    END IF\n  NEXT\n  FOR j = 0 TO 1\n    \
             LET h AS P = collections::get(xs, j)\n    t = t + h.age\n  NEXT\n  RETURN t\nEND FUNC\n",
    );
    let main = function(&module, "main");
    let got = collect_borrow_gets(&main.body, &HashSet::new(), &|_, _| false);
    let elements: Vec<&ElementBinding> = got
        .element_bindings()
        .filter(|(name, _)| *name == "h")
        .map(|(_, binding)| binding)
        .collect();
    assert_eq!(
        elements.len(),
        1,
        "one of the two bindings of h is element-bound"
    );
    let sites: Vec<(Option<usize>, Option<usize>)> = elements[0].sites().collect();
    assert_eq!(sites.len(), 2, "both arms are sites: {sites:?}");
    assert!(sites
        .iter()
        .all(|(update, write_back)| update.is_some() && write_back.is_some()));
}

/// A re-bound name that is address-taken (a callback holds a reference to it) is
/// address-taken under every unique name the per-binding pass gives it, so none of
/// its bindings borrows; with no reference held, both do.
#[test]
fn an_address_taken_rebound_name_borrows_in_no_binding() {
    let module = lower(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT ys AS List OF P = mk()\n  \
             MUT t AS Integer = 0\n  FOR j = 0 TO 1\n    LET h AS P = collections::get(xs, j)\n    \
             t = t + h.age\n  NEXT\n  FOR j = 0 TO 1\n    LET h AS P = collections::get(ys, j)\n    \
             t = t + h.age\n  NEXT\n  RETURN t\nEND FUNC\n",
    );
    let main = function(&module, "main");
    let borrows = |address_taken: HashSet<String>| {
        collect_borrow_gets(&main.body, &address_taken, &|_, _| false)
            .rebound
            .values()
            .filter(|rebound| rebound.name == "h" && matches!(rebound.role, ReboundRole::Borrow))
            .count()
    };
    assert_eq!(borrows(HashSet::new()), 2);
    assert_eq!(borrows(HashSet::from(["h".to_string()])), 0);
}
