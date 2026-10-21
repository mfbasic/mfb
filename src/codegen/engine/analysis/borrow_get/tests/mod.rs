use super::*;
use crate::target::shared::nir::{NirFunction, NirModule};
use crate::testutil::{nir_for_src, CodeTarget};

mod predicate_shapes;
mod rename_forms;

const TYPES: &str = "\
IMPORT collections
IMPORT io

TYPE P
  age AS Integer
  trail AS List OF Float
END TYPE

FUNC mk AS List OF P
  RETURN [P[age := 1, trail := [0.0]], P[age := 2, trail := [0.0]]]
END FUNC

FUNC f(p AS P) AS Integer
  RETURN p.age
END FUNC
";

fn lower(body: &str) -> NirModule {
    nir_for_src(
        &format!("{TYPES}\n{body}"),
        CodeTarget::MacosAarch64,
        crate::target::NativeBuildMode::Console,
    )
    .unwrap_or_else(|error| panic!("the probe lowers to NIR: {error}"))
}

fn function<'m>(module: &'m NirModule, name: &str) -> &'m NirFunction {
    module
        .functions
        .iter()
        .find(|f| f.name == name || f.name.rsplit(['.', ':']).next() == Some(name))
        .unwrap_or_else(|| panic!("no function `{name}`"))
}

/// The classification of `main`'s bindings, with no global ever written.
fn classify(body: &str) -> BorrowGets {
    let module = lower(body);
    let main = function(&module, "main");
    let mut address_taken = HashSet::new();
    crate::codegen::engine::function::function_lowering::collect_address_taken_locals(
        &main.body,
        &mut address_taken,
    );
    collect_borrow_gets(&main.body, &address_taken, &|_, _| false)
}

/// bug-776: plan-149's place forwarding deletes the `$match0 = e` temp the
/// IR's `MATCH` desugar introduces, so the classifier sees `MATCH e` and
/// `UnionExtract(e)` directly. `e` is still read only as the match's
/// scrutinee and its variant bindings, so it must stay a pinned borrow.
#[test]
fn a_directly_matched_binding_after_place_forwarding_is_pinned() {
    use crate::optimizer::{with_opt_level, OptLevel};
    let body = "TYPE Num\n  value AS Integer\nEND TYPE\n\
             TYPE Add\n  left AS Integer\n  right AS Integer\nEND TYPE\n\
             UNION Expr\n  Num\n  Add\nEND UNION\n\
             FUNC main() AS Integer\n  LET xs AS List OF Expr = [Num[3], Add[1, 2]]\n  \
             LET e AS Expr = collections::get(xs, 1)\n  MATCH e\n    CASE Num(n)\n      \
             RETURN n.value\n    CASE Add(a)\n      RETURN a.left + a.right\n  END MATCH\n  \
             RETURN 0\nEND FUNC\n";
    let mut module = with_opt_level(OptLevel(0), || lower(body));
    with_opt_level(OptLevel(1), || {
        crate::optimizer::opt1::placefwd::forward(&mut module)
    });
    let main = function(&module, "main");
    struct MatchTemp(bool);
    impl NirVisitor for MatchTemp {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::Bind { name, .. } = op {
                self.0 |= name.starts_with("$match");
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Local(name) = value {
                self.0 |= name.starts_with("$match");
            }
            walk_value(self, value);
        }
    }
    let mut temp = MatchTemp(false);
    temp.visit_ops(&main.body);
    assert!(
        !temp.0,
        "the probe needs the forwarded form: the `$match` temp must be gone"
    );
    let mut address_taken = HashSet::new();
    crate::codegen::engine::function::function_lowering::collect_address_taken_locals(
        &main.body,
        &mut address_taken,
    );
    let got = collect_borrow_gets(&main.body, &address_taken, &|_, _| false);
    assert!(
        got.names.contains("e") && got.pinned.contains("e"),
        "a get binding read only through MATCH and its variant bindings is a pinned \
             borrow after place forwarding; got names {:?} pinned {:?}",
        got.names,
        got.pinned
    );
}

#[test]
fn a_field_read_binding_over_an_immutable_list_is_pinned() {
    let got = classify(
        "FUNC main() AS Integer\n  LET xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  RETURN p.age + len(p.trail)\nEND FUNC\n",
    );
    assert!(got.names.contains("p") && got.pinned.contains("p"));
    assert!(got.elements.is_empty());
}

#[test]
fn a_field_read_binding_over_a_mut_list_is_windowed() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  LET n AS Integer = p.age + len(xs)\n  \
             xs = collections::append(xs, p)\n  RETURN n\nEND FUNC\n",
    );
    // `p` is passed whole to `append` — not a field read — so it is no borrow.
    assert!(!got.names.contains("p"));
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  LET n AS Integer = p.age + len(xs)\n  \
             xs = collections::append(xs, P[age := n, trail := []])\n  RETURN n\nEND FUNC\n",
    );
    assert!(got.names.contains("p") && !got.pinned.contains("p"));
}

#[test]
fn a_write_to_the_container_inside_the_window_refuses_the_borrow() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  \
             xs = collections::append(xs, P[age := 9, trail := []])\n  RETURN p.age\nEND FUNC\n",
    );
    assert!(!got.names.contains("p"));
}

#[test]
fn a_non_lookup_read_of_the_container_inside_the_window_refuses_the_borrow() {
    // `xs` handed to a user function could be moved or handed over there.
    let got = classify(
        "FUNC g(ys AS List OF P) AS Integer\n  RETURN len(ys)\nEND FUNC\n\
             FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  xs = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  LET n AS Integer = g(xs)\n  \
             RETURN p.age + n\nEND FUNC\n",
    );
    assert!(!got.names.contains("p"));
}

#[test]
fn a_get_or_with_a_built_default_is_no_borrow() {
    let got = classify(
        "FUNC main() AS Integer\n  LET xs AS List OF P = mk()\n  \
             LET p AS P = collections::getOr(xs, 5, P[age := 0, trail := []])\n  \
             RETURN p.age\nEND FUNC\n",
    );
    assert!(!got.names.contains("p"));
    let got = classify(
        "FUNC main() AS Integer\n  LET xs AS List OF P = mk()\n  \
             LET d AS P = P[age := 0, trail := []]\n  \
             LET p AS P = collections::getOr(xs, 5, d)\n  RETURN p.age\nEND FUNC\n",
    );
    assert!(got.pinned.contains("p"));
}

#[test]
fn get_with_set_on_the_same_slot_is_element_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             MUT p AS P = collections::get(xs, i)\n    LET n AS Integer = p.age\n    \
             p = WITH p { age := n + 1 }\n    xs = collections::set(xs, i, p)\n  NEXT\n  \
             RETURN len(xs)\nEND FUNC\n",
    );
    let binding = got.elements.get("p").expect("p is element-bound");
    assert!(binding.update.is_some() && binding.write_back.is_some());
    assert!(got.names.contains("p"));
}

/// bug-766: `set(xs, i, WITH p { … })` is the S1+W pair in one statement.
#[test]
fn a_with_over_the_binding_as_the_set_item_is_element_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET p AS P = collections::get(xs, i)\n    LET n AS Integer = p.age\n    \
             xs = collections::set(xs, i, WITH p { age := n + p.age })\n  NEXT\n  \
             RETURN len(xs)\nEND FUNC\n",
    );
    let binding = got.elements.get("p").expect("p is element-bound");
    // The fused statement is the update and its own write-back.
    assert!(binding.update.is_some() && binding.write_back.is_none());
    assert!(binding.other_sites.is_empty() && got.names.contains("p"));
}

/// bug-766: `LET u = WITH p { … }` / `set(xs, i, u)` — `u` is S1's result.
#[test]
fn a_let_bound_with_before_its_set_is_element_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET p AS P = collections::get(xs, i)\n    LET u AS P = WITH p { age := p.age + 1 }\n    \
             xs = collections::set(xs, i, u)\n  NEXT\n  RETURN len(xs)\nEND FUNC\n",
    );
    let binding = got.elements.get("p").expect("p is element-bound");
    assert!(binding.update.is_some() && binding.write_back.is_some());
    // `u` read again is a second use of S1's result: no element binding.
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  LET u AS P = WITH p { age := p.age + 1 }\n  \
             xs = collections::set(xs, 0, u)\n  RETURN u.age\nEND FUNC\n",
    );
    assert!(got.elements.is_empty());
}

/// bug-766: a write-back in every arm of an `IF`, each arm its own site.
#[test]
fn write_backs_in_both_if_arms_are_element_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET p AS P = collections::get(xs, i)\n    IF p.age > 1 THEN\n      \
             xs = collections::set(xs, i, WITH p { age := p.age + 1 })\n    ELSE\n      \
             LET n AS Integer = p.age * 2\n      LET u AS P = WITH p { age := n }\n      \
             xs = collections::set(xs, i, u)\n    END IF\n  NEXT\n  RETURN len(xs)\nEND FUNC\n",
    );
    let binding = got.elements.get("p").expect("p is element-bound");
    assert_eq!(binding.sites().count(), 2);
    assert!(binding.sites().all(|(update, _)| update.is_some()));
}

#[test]
fn a_write_back_in_one_if_arm_only_is_not_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET p AS P = collections::get(xs, i)\n    IF p.age > 1 THEN\n      \
             xs = collections::set(xs, i, WITH p { age := p.age + 1 })\n    ELSE\n      \
             io::print(toString(p.age))\n    END IF\n  NEXT\n  RETURN len(xs)\nEND FUNC\n",
    );
    assert!(got.elements.is_empty() && !got.names.contains("p"));
    // No ELSE at all.
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET p AS P = collections::get(xs, i)\n    IF p.age > 1 THEN\n      \
             xs = collections::set(xs, i, WITH p { age := p.age + 1 })\n    END IF\n  NEXT\n  \
             RETURN len(xs)\nEND FUNC\n",
    );
    assert!(got.elements.is_empty() && !got.names.contains("p"));
}

#[test]
fn a_read_of_the_binding_after_a_with_item_set_is_not_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  \
             xs = collections::set(xs, 0, WITH p { age := 7 })\n  RETURN p.age\nEND FUNC\n",
    );
    assert!(got.elements.is_empty() && !got.names.contains("p"));
}

/// bug-766 B: the n-body pair step — two elements live across a write to the
/// container, each read only through fields and as a `set` item's `WITH`
/// target. Neither can alias its slot (`pi` is read after its write-back,
/// `pj`'s slot is written first), so both stay copies, of the scratch kind.
#[test]
fn two_elements_live_across_a_write_are_scratch_copies() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET pi AS P = collections::get(xs, i)\n    LET pj AS P = collections::get(xs, 1 - i)\n    \
             xs = collections::set(xs, i, WITH pi { age := pi.age + pj.age })\n    \
             xs = collections::set(xs, 1 - i, WITH pj { age := pj.age - pi.age })\n  NEXT\n  \
             RETURN len(xs)\nEND FUNC\n",
    );
    assert!(!got.names.contains("pi") && !got.names.contains("pj"));
    assert!(got.scratch.contains_key("pi") && got.scratch.contains_key("pj"));
}

/// bug-766 B: any read that could keep the binding's block — passed whole,
/// bound to another name, returned, a `WITH` that is not a copied item — or a
/// `getOr` leaves it an ordinary owned copy.
#[test]
fn an_escaping_or_get_or_binding_is_no_scratch_copy() {
    for (tail, why) in [
        ("RETURN f(pi) + pj.age", "passed whole"),
        (
            "MUT q AS P = pi\n  q = WITH q { age := 1 }\n  RETURN q.age + pj.age",
            "bound to another name",
        ),
        (
            "LET q AS P = WITH pi { age := 1 }\n  RETURN q.age + pj.age",
            "a WITH bound to a name",
        ),
    ] {
        let got = classify(&format!(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
                 LET pi AS P = collections::get(xs, 0)\n  LET pj AS P = collections::get(xs, 1)\n  \
                 xs = collections::set(xs, 1, WITH pj {{ age := pi.age }})\n  {tail}\nEND FUNC\n"
        ));
        assert!(!got.scratch.contains_key("pi"), "pi {why}");
    }
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET d AS P = P[age := 0, trail := []]\n  \
             LET pi AS P = collections::getOr(xs, 0, d)\n  \
             xs = collections::set(xs, 0, WITH pi { age := 1 })\n  RETURN pi.age\nEND FUNC\n",
    );
    assert!(!got.scratch.contains_key("pi"), "a getOr binding");
}

#[test]
fn a_write_to_the_container_between_the_bind_and_the_set_is_not_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  \
             xs = collections::append(xs, P[age := 9, trail := []])\n  \
             xs = collections::set(xs, 0, WITH p { age := p.age + 1 })\n  RETURN 0\nEND FUNC\n",
    );
    assert!(got.elements.is_empty() && !got.names.contains("p"));
    // Nor inside an arm before its write-back.
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  IF p.age > 1 THEN\n    \
             xs = collections::set(xs, 0, WITH p { age := 1 })\n  ELSE\n    \
             xs = collections::append(xs, P[age := 9, trail := []])\n    \
             xs = collections::set(xs, 0, WITH p { age := 2 })\n  END IF\n  RETURN 0\nEND FUNC\n",
    );
    assert!(got.elements.is_empty() && !got.names.contains("p"));
    // Nor by the update's own values.
    let got = classify(
        "FUNC g(ys AS List OF P) AS Integer\n  RETURN len(ys)\nEND FUNC\n\
             FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  \
             xs = collections::set(xs, 0, WITH p { age := g(xs) })\n  RETURN 0\nEND FUNC\n",
    );
    assert!(got.elements.is_empty());
}

#[test]
fn an_element_update_that_is_not_adjacent_to_its_write_back_is_not_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             MUT p AS P = collections::get(xs, 0)\n  p = WITH p { age := 7 }\n  \
             io::print(toString(collections::get(xs, 0).age))\n  \
             xs = collections::set(xs, 0, p)\n  RETURN 0\nEND FUNC\n",
    );
    assert!(got.elements.is_empty() && !got.names.contains("p"));
}

#[test]
fn an_element_read_after_its_write_back_is_not_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             MUT p AS P = collections::get(xs, 0)\n  p = WITH p { age := 7 }\n  \
             xs = collections::set(xs, 0, p)\n  RETURN p.age\nEND FUNC\n",
    );
    assert!(got.elements.is_empty() && !got.names.contains("p"));
}

#[test]
fn a_write_back_to_another_slot_is_not_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             MUT p AS P = collections::get(xs, 0)\n  p = WITH p { age := 7 }\n  \
             xs = collections::set(xs, 1, p)\n  RETURN 0\nEND FUNC\n",
    );
    assert!(got.elements.is_empty());
    // The index names the same slot only while its locals hold still.
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT i AS Integer = 0\n  \
             MUT p AS P = collections::get(xs, i)\n  i = 1\n  p = WITH p { age := 7 }\n  \
             xs = collections::set(xs, i, p)\n  RETURN 0\nEND FUNC\n",
    );
    assert!(got.elements.is_empty());
}

#[test]
fn an_element_passed_whole_before_its_update_is_not_bound() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             MUT p AS P = collections::get(xs, 0)\n  LET n AS Integer = f(p)\n  \
             p = WITH p { age := n }\n  xs = collections::set(xs, 0, p)\n  RETURN 0\nEND FUNC\n",
    );
    assert!(got.elements.is_empty() && !got.names.contains("p"));
}

#[test]
fn a_global_container_needs_no_reachable_store() {
    let body = "MUT gs AS List OF P = []\n\
             FUNC main() AS Integer\n  gs = mk()\n  LET p AS P = collections::get(gs, 0)\n  \
             RETURN p.age\nEND FUNC\n";
    let module = lower(body);
    let main = function(&module, "main");
    let none = HashSet::new();
    assert!(collect_borrow_gets(&main.body, &none, &|_, _| false)
        .names
        .contains("p"));
    assert!(!collect_borrow_gets(&main.body, &none, &|_, _| true)
        .names
        .contains("p"));
}

#[test]
fn the_single_expression_update_names_its_container_and_index() {
    let module = lower(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             xs = collections::set(xs, i, WITH collections::get(xs, i) { age := collections::get(xs, i).age + 1 })\n  \
             NEXT\n  RETURN 0\nEND FUNC\n",
    );
    let main = function(&module, "main");
    let mut found = None;
    for_each_block(&main.body, &mut |block| {
        for op in block {
            if let NirOp::Assign { name, value } = op {
                if name == "xs" {
                    if let Some((container, index, with)) = single_expression_update(value) {
                        found = Some((container, index.clone(), with.clone()));
                    }
                }
            }
        }
    });
    let (container, index, with) = found.expect("the update is recognised");
    assert_eq!(container, ContainerRef::Local("xs".to_string()));
    let bound = bind_single_expression_element(&with, &container, &index, "$e")
        .expect("its element is read only through fields");
    struct Count(usize);
    impl NirVisitor for Count {
        fn visit_value(&mut self, value: &NirValue) {
            if matches!(value, NirValue::Local(name) if name == "$e") {
                self.0 += 1;
            }
            walk_value(self, value);
        }
    }
    let mut count = Count(0);
    count.visit_value(&bound);
    // The WITH's target and the one field read.
    assert_eq!(count.0, 2);
}

/// bug-784: the role of each binding of `name` in `main`, by the container
/// its `get` reads, sorted by container.
fn rebound_roles(body: &str, name: &str) -> Vec<(String, &'static str)> {
    let module = lower(body);
    let main = function(&module, "main");
    let mut address_taken = HashSet::new();
    crate::codegen::engine::function::function_lowering::collect_address_taken_locals(
        &main.body,
        &mut address_taken,
    );
    let got = collect_borrow_gets(&main.body, &address_taken, &|_, _| false);
    struct Binds<'a> {
        name: &'a str,
        got: &'a BorrowGets,
        out: Vec<(String, &'static str)>,
    }
    impl NirVisitor for Binds<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::Bind {
                name,
                value: Some(value),
                ..
            } = op
            {
                if name == self.name {
                    let container = get_call(value)
                        .and_then(|(_, args)| match args.first() {
                            Some(NirValue::Local(c)) => Some(c.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    let role = match self.got.rebound.get(&op_key(op)).map(|r| &r.role) {
                        Some(ReboundRole::Element(_)) => "element",
                        Some(ReboundRole::Borrow) => "borrow",
                        Some(ReboundRole::Copy) => "copy",
                        None => "unswitched",
                    };
                    self.out.push((container, role));
                }
            }
            walk_op(self, op);
        }
    }
    let mut binds = Binds {
        name,
        got: &got,
        out: Vec::new(),
    };
    binds.visit_ops(&main.body);
    binds.out.sort();
    binds.out
}

fn roles(pairs: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
    pairs.iter().map(|(c, r)| (c.to_string(), *r)).collect()
}

/// bug-784: the benchmark's shape — `LET h = get(…)` in one loop and again in a
/// later one (and the loop variable `j` bound twice too). Each binding is a
/// binding of its own; the old name-keyed rule made both copies.
#[test]
fn a_name_bound_in_two_sequential_loops_borrows_in_each() {
    let got = rebound_roles(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT ys AS List OF P = mk()\n  \
             MUT t AS Integer = 0\n  FOR j = 0 TO 1\n    LET h AS P = collections::get(xs, j)\n    \
             t = t + h.age\n  NEXT\n  FOR j = 0 TO 1\n    LET h AS P = collections::get(ys, j)\n    \
             t = t + h.age\n  NEXT\n  RETURN t\nEND FUNC\n",
        "h",
    );
    assert_eq!(got, roles(&[("xs", "borrow"), ("ys", "borrow")]));
}

/// bug-784: the same name in the two arms of an `IF` (sibling scopes).
#[test]
fn a_name_bound_in_sibling_arms_borrows_in_each() {
    let got = rebound_roles(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT ys AS List OF P = mk()\n  \
             MUT t AS Integer = 0\n  IF len(xs) > 1 THEN\n    LET h AS P = collections::get(xs, 0)\n    \
             t = t + h.age\n  ELSE\n    LET h AS P = collections::get(ys, 0)\n    t = t + h.age\n  \
             END IF\n  RETURN t\nEND FUNC\n",
        "h",
    );
    assert_eq!(got, roles(&[("xs", "borrow"), ("ys", "borrow")]));
}

/// bug-784: a re-bind inside the same loop body — once in a nested `IF` block,
/// once after it ends.
#[test]
fn a_name_rebound_inside_the_same_loop_borrows_in_each() {
    let got = rebound_roles(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT ys AS List OF P = mk()\n  \
             MUT t AS Integer = 0\n  FOR j = 0 TO 1\n    IF j > 0 THEN\n      \
             LET h AS P = collections::get(xs, j)\n      t = t + h.age\n    END IF\n    \
             LET h AS P = collections::get(ys, j)\n    t = t + h.age\n  NEXT\n  RETURN t\nEND FUNC\n",
        "h",
    );
    assert_eq!(got, roles(&[("xs", "borrow"), ("ys", "borrow")]));
}

/// bug-784: `test_record_nested`'s exact pattern — an element-bound binding in
/// the update loop and a read-only one of the same name in the checksum loop.
#[test]
fn an_element_update_survives_a_later_read_only_rebind() {
    let got = rebound_roles(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT t AS Integer = 0\n  \
             FOR j = 0 TO 1\n    LET h AS P = collections::get(xs, j)\n    \
             xs = collections::set(xs, j, WITH h { age := h.age + 1 })\n  NEXT\n  \
             FOR j = 0 TO 1\n    LET h AS P = collections::get(xs, j)\n    t = t + h.age\n  NEXT\n  \
             RETURN t\nEND FUNC\n",
        "h",
    );
    let kinds: Vec<&str> = got.iter().map(|(_, role)| *role).collect();
    let mut kinds_sorted = kinds.clone();
    kinds_sorted.sort();
    assert_eq!(kinds_sorted, vec!["borrow", "element"], "{got:?}");
}

/// bug-784, the negative: a binding that is NOT safe to borrow stays a copy
/// even though another binding of its name borrows — its container is written
/// while it is live, or it escapes whole into a call. One binding's verdict
/// never leaks to another.
#[test]
fn an_unsafe_rebinding_stays_a_copy_beside_a_borrowing_one() {
    let got = rebound_roles(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT ys AS List OF P = mk()\n  \
             MUT zs AS List OF P = mk()\n  MUT t AS Integer = 0\n  FOR j = 0 TO 1\n    \
             LET h AS P = collections::get(xs, j)\n    t = t + h.age\n  NEXT\n  FOR j = 0 TO 1\n    \
             LET h AS P = collections::get(ys, j)\n    \
             ys = collections::set(ys, j, P[age := 9, trail := [1.0]])\n    t = t + h.age\n  NEXT\n  \
             FOR j = 0 TO 1\n    LET h AS P = collections::get(zs, j)\n    t = t + f(h)\n  NEXT\n  \
             RETURN t\nEND FUNC\n",
        "h",
    );
    assert_eq!(
        got,
        roles(&[("xs", "borrow"), ("ys", "copy"), ("zs", "copy")])
    );
}

/// bug-784: a function that binds no name twice is classified exactly as by
/// name — nothing is switched per binding.
#[test]
fn a_function_without_rebinds_has_no_per_binding_roles() {
    let got = classify(
        "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  LET h AS P = collections::get(xs, 0)\n  \
             RETURN h.age\nEND FUNC\n",
    );
    assert!(got.rebound.is_empty());
    assert!(got.names.contains("h"));
}
