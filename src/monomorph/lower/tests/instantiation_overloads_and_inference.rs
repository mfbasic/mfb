//! plan-176-G: `monomorph::lower`'s tests of generic instantiation, overload selection
//! and control flow, and generic inference. The shared `project`/`monomorphize`
//! helpers live in `tests/mod.rs`.

use super::*;

#[test]
fn generic_function_instantiated_per_argument_type() {
    // A generic `AS Nothing` function called with Integer and String is monomorphized into two
    // concrete symbols (mangled by argument type); the template is dropped.
    let src = "\
IMPORT io
FUNC show OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Nothing
  show(42)
  show(\"hi\")
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let names = function_names(&project);
    assert!(names.iter().any(|n| n == "show$Integer"), "{names:?}");
    assert!(names.iter().any(|n| n == "show$String"), "{names:?}");
    // The open template `show` is not emitted.
    assert!(!names.iter().any(|n| n == "show"), "{names:?}");
}

#[test]
fn generic_function_deduplicates_repeated_instantiation() {
    // Two calls with the same type argument produce a single concrete symbol.
    let src = "\
IMPORT io
FUNC show OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Nothing
  show(1)
  show(2)
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let count = function_names(&project)
        .iter()
        .filter(|n| *n == "show$Integer")
        .count();
    assert_eq!(count, 1);
}

#[test]
fn total_instantiation_budget_halts_wide_fanout() {
    // The per-path depth cap cannot bound a generic that fans out in *breadth*
    // — ≥2 type-widening self-calls make an exponential tree of distinct
    // `name<args>` keys the depth cap never collapses (bug-399). A global
    // total-instantiation budget does. Drive the shared counter directly (an
    // end-to-end fan-out that actually reaches the several-thousand budget is
    // real but too slow for a unit test): it admits exactly
    // `MAX_TOTAL_INSTANTIATIONS` charges, then refuses every further one and
    // latches `instantiation_limit_reached` so enumeration stops after a single
    // bounded diagnostic.
    let ast = project(&[(
        "src/main.mfb",
        "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
    )]);
    let dir = std::env::temp_dir();
    let hir = crate::hir::elaborate(&ast);
    let mut mono = Monomorphizer::new(&dir, &hir);
    for i in 0..super::super::MAX_TOTAL_INSTANTIATIONS {
        assert!(
            mono.charge_instantiation("f", 1),
            "charge {i} within budget must succeed"
        );
        assert!(
            !mono.instantiation_limit_reached,
            "limit must not latch early"
        );
    }
    // Budget exhausted: this charge reports the single diagnostic and refuses.
    assert!(
        !mono.charge_instantiation("f", 1),
        "the charge at the budget must be refused"
    );
    assert!(mono.instantiation_limit_reached, "the limit must latch");
    assert!(mono.had_error, "refusal reports a diagnostic");
    // Once latched, every further charge short-circuits without incrementing.
    assert!(
        !mono.charge_instantiation("f", 1),
        "charges past the limit stay refused"
    );
    assert_eq!(
        mono.total_instantiations,
        super::super::MAX_TOTAL_INSTANTIATIONS,
        "no charge is admitted past the budget"
    );
}

#[test]
fn instantiate_type_disambiguates_mangle_colliding_arguments() {
    // `mangle_name` is lossy: `sanitize_type_name` collapses every
    // non-alphanumeric character to `$`, so two distinct type-argument strings
    // that differ only in punctuation produce one shared mangled symbol.
    // `instantiate_function` guards this via `unique_concrete_symbol` (bug-226);
    // `instantiate_type` must too, or the second instantiation overwrites the
    // first in `concrete_types` and both use-sites bind one shared — and
    // possibly wrong — concrete type (bug-400). Driven directly against
    // `instantiate_type` because no *valid* source spelling reaches the
    // collision (the grammar fixes each punctuation slot relative to the alnum
    // tokens; the mangled symbol is the only lossy layer), which is exactly why
    // bug-226/bug-400 are latent — the same reason
    // `total_instantiation_budget_halts_wide_fanout` drives its counter directly.
    let ast = project(&[(
        "src/main.mfb",
        "TYPE Box OF T\n  value AS T\nEND TYPE\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
    )]);
    let dir = std::env::temp_dir();
    let hir = crate::hir::elaborate(&ast);
    let mut mono = Monomorphizer::new(&dir, &hir);

    // Two distinct type-argument strings that `sanitize_type_name` maps to the
    // same suffix — `(`/`)` and `{`/`}` both sanitize to `$`.
    let a = "FUNC(Integer) AS Nothing".to_string();
    let b = "FUNC{Integer} AS Nothing".to_string();
    assert_ne!(a, b);
    assert_eq!(
        super::mangle_name("Box", std::slice::from_ref(&a)),
        super::mangle_name("Box", std::slice::from_ref(&b)),
        "the two arguments must mangle-collide for this test to exercise the guard"
    );

    let name_a = mono
        .instantiate_type("Box", &[ParameterType::parse(&a)])
        .name()
        .into_owned();
    let name_b = mono
        .instantiate_type("Box", &[ParameterType::parse(&b)])
        .name()
        .into_owned();

    // Each colliding instantiation must keep its own distinct concrete symbol...
    assert_ne!(
        name_a, name_b,
        "colliding type arguments must resolve to distinct concrete symbols"
    );
    // ...and its own concrete type declaration (no overwrite).
    let type_a = mono
        .concrete_types
        .get(&ParameterType::named(&name_a))
        .expect("first concrete Box survives");
    let type_b = mono
        .concrete_types
        .get(&ParameterType::named(&name_b))
        .expect("second concrete Box survives");
    assert_eq!(
        type_a.fields[0].type_.name().as_ref(),
        a.as_str(),
        "first keeps its field type"
    );
    assert_eq!(
        type_b.fields[0].type_.name().as_ref(),
        b.as_str(),
        "second keeps its field type"
    );

    // bug-697: each colliding mangled symbol must still map back to ITS OWN
    // template spelling — the disambiguation suffix that keeps the two
    // symbols distinct must not leak into, or be lost from, either
    // rendered spelling.
    let spellings = mono.template_spellings();
    assert_eq!(
        spellings.get(&name_a).map(String::as_str),
        Some("Box OF FUNC(Integer) AS Nothing"),
        "first colliding symbol keeps its own template spelling"
    );
    assert_eq!(
        spellings.get(&name_b).map(String::as_str),
        Some("Box OF FUNC{Integer} AS Nothing"),
        "second colliding symbol keeps its own template spelling"
    );
}

#[test]
fn nested_generic_container_argument_unifies() {
    // A `List OF T` parameter unifies T against the element type of the
    // argument, exercising the recursive container unification.
    let src = "\
IMPORT io
IMPORT collections
FUNC first OF T(items AS List OF T) AS T
  RETURN collections::get(items, 0)
END FUNC
FUNC main() AS Nothing
  LET xs AS List OF Integer = [1, 2, 3]
  LET a AS Integer = first(xs)
  io::print(toString(a))
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let names = function_names(&project);
    assert!(
        names.iter().any(|n| n.starts_with("first$")),
        "expected a mangled first instantiation, got {names:?}"
    );
}

#[test]
fn generic_type_instantiated_from_expected_constructor_type() {
    // A generic TYPE used with an expected `Box OF Integer` constructor type
    // is instantiated into a concrete mangled type declaration.
    let src = "\
IMPORT io
TYPE Box OF T
  value AS T
END TYPE
FUNC main() AS Integer
  LET b AS Box OF Integer = Box[5]
  io::print(toString(b.value))
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let type_names: Vec<&str> = types(&project).iter().map(|t| t.name.as_str()).collect();
    assert!(
        type_names.iter().any(|n| n.starts_with("Box$")),
        "expected a concrete Box instantiation, got {type_names:?}"
    );
}

#[test]
fn overload_selected_by_parameter_type() {
    // Two overloads differing by parameter type resolve to distinct mangled
    // concrete symbols selected from the call argument types.
    let src = "\
IMPORT io
FUNC label(n AS Integer) AS String
  RETURN \"int\"
END FUNC
FUNC label(s AS String) AS String
  RETURN \"str\"
END FUNC
FUNC main() AS Nothing
  io::print(label(1))
  io::print(label(\"x\"))
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let names = function_names(&project);
    assert!(names.iter().any(|n| n == "label$Integer"), "{names:?}");
    assert!(names.iter().any(|n| n == "label$String"), "{names:?}");
}

#[test]
fn return_type_overload_selected_by_expected_type() {
    // Two overloads share parameter types and differ only by return type; an
    // annotated LET target supplies the expected type to disambiguate.
    let src = "\
IMPORT io
FUNC make() AS Integer
  RETURN 1
END FUNC
FUNC make() AS String
  RETURN \"one\"
END FUNC
FUNC main() AS Nothing
  LET a AS Integer = make()
  LET b AS String = make()
  io::print(b)
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let names = function_names(&project);
    // Return-type disambiguation appends `AS <return>`.
    assert!(names.iter().any(|n| n.contains("AS$Integer")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("AS$String")), "{names:?}");
}

#[test]
fn control_flow_forms_are_lowered() {
    // FOR / FOR EACH / WHILE / DO UNTIL / IF bodies all pass through
    // statement lowering, and a generic call inside is still instantiated.
    let src = "\
IMPORT io
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  FOR i = 1 TO 3
    emit(i)
  NEXT
  LET xs AS List OF Integer = [1, 2]
  FOR EACH x IN xs
    emit(x)
  NEXT
  MUT n AS Integer = 0
  WHILE n < 2
    emit(n)
    n = n + 1
  END WHILE
  DO
    emit(n)
    n = n + 1
  LOOP UNTIL n > 4
  IF n > 0 THEN
    emit(n)
  ELSE
    emit(0)
  END IF
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(function_names(&project).iter().any(|n| n == "emit$Integer"));
}

#[test]
fn for_loop_float_bound_promotes_counter_type() {
    // A Float loop bound promotes the counter's type so a generic call using
    // the counter instantiates on Float, exercising promote_loop_numeric_type.
    let src = "\
IMPORT io
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  FOR i = 1.0 TO 3.0
    emit(i)
  NEXT
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(
        function_names(&project).iter().any(|n| n == "emit$Float"),
        "{:?}",
        function_names(&project)
    );
}

#[test]
fn match_union_variant_binding_is_lowered() {
    // A MATCH over a union binds the variant and lowers its body; a generic
    // call in the arm is instantiated on the bound type.
    let src = "\
IMPORT io
TYPE Circle
  r AS Integer
END TYPE
TYPE Square
  s AS Integer
END TYPE
UNION Shape
  Circle
  Square
END UNION
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  LET shape AS Shape = Circle[2]
  MATCH shape
    CASE Circle(c)
      emit(c.r)
    CASE Square(sq)
      emit(sq.s)
  END MATCH
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(function_names(&project).iter().any(|n| n == "emit$Integer"));
}

#[test]
fn arity_mismatch_reports_error() {
    // More arguments than the template has parameters -> error flag set.
    let src = "\
IMPORT io
FUNC one OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Nothing
  one(1, 2)
END FUNC
";
    assert!(monomorphize(src).is_err());
}

#[test]
fn top_level_binding_value_is_lowered() {
    // A module-level LET with a generic-call initializer lowers the binding
    // value (lower_binding) and instantiates the callee.
    let src = "\
IMPORT io
FUNC idOf OF T(value AS T) AS T
  RETURN value
END FUNC
LET g AS Integer = idOf(7)
FUNC main() AS Nothing
  io::print(toString(g))
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(
        function_names(&project)
            .iter()
            .any(|n| n.starts_with("idOf$")),
        "{:?}",
        function_names(&project)
    );
}

#[test]
fn trap_body_is_lowered() {
    // A function with a TRAP handler lowers the trap body too; a generic call
    // inside the handler is instantiated.
    let src = "\
IMPORT io
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC risky() AS Integer
  RETURN 1
  TRAP(err)
    emit(1)
    RETURN 0
  END TRAP
END FUNC
FUNC main() AS Integer
  io::print(toString(risky()))
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(function_names(&project).iter().any(|n| n == "emit$Integer"));
}

#[test]
fn plain_program_without_generics_passes_through() {
    // A concrete-only program monomorphizes to an equivalent project.
    let src = "\
IMPORT io
FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
FUNC main() AS Nothing
  io::print(toString(add(1, 2)))
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let names = function_names(&project);
    assert!(names.iter().any(|n| n == "add"));
    assert!(names.iter().any(|n| n == "main"));
}

#[test]
fn generic_over_map_and_result_container_params() {
    // Container-shaped parameter types exercise the Map recursion in
    // concrete_type_name / template_view_type / unify.
    let src = "\
IMPORT io
IMPORT collections
FUNC lookup OF (K, V)(items AS Map OF K TO V, key AS K, fallback AS V) AS V
  IF collections::hasKey(items, key) THEN
    RETURN collections::get(items, key)
  END IF
  RETURN fallback
END FUNC
FUNC main() AS Integer
  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }
  LET v AS Integer = lookup(m, \"a\", 0)
  io::print(toString(v))
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(
        function_names(&project)
            .iter()
            .any(|n| n.starts_with("lookup$")),
        "{:?}",
        function_names(&project)
    );
}

#[test]
fn generic_type_inferred_from_constructor_arguments() {
    // A generic constructor with NO expected-type annotation infers its type
    // argument from the constructor argument types (lines 1010-1038).
    let src = "\
IMPORT io
TYPE Box OF T
  value AS T
END TYPE
FUNC boxed() AS Box OF Integer
  RETURN Box[5]
END FUNC
FUNC main() AS Integer
  io::print(toString(boxed().value))
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let type_names: Vec<&str> = types(&project).iter().map(|t| t.name.as_str()).collect();
    assert!(
        type_names.iter().any(|n| n.starts_with("Box$")),
        "{type_names:?}"
    );
}

#[test]
fn constructor_infers_param_despite_unknown_field_declared_first() {
    // bug-442: a generic record whose `List OF T` field (bound to an empty
    // `[]` => `List OF Unknown`) is declared BEFORE the `FUNC(T)` field that
    // carries the concrete type must still resolve `T` from the later field,
    // instantiating `Box$Integer` rather than `Box$Unknown`.
    let src = "\
IMPORT io
TYPE Box OF T
  items AS List OF T
  fn    AS FUNC(T) AS Boolean
END TYPE
FUNC makeBox OF T(fn AS FUNC(T) AS Boolean) AS Box OF T
  RETURN Box[items := [], fn := fn]
END FUNC
FUNC even(n AS Integer) AS Boolean
  RETURN n MOD 2 = 0
END FUNC
FUNC main() AS Integer
  MUT a AS Box OF Integer = makeBox(even)
  io::print(toString(len(a.items)))
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let type_names: Vec<&str> = types(&project).iter().map(|t| t.name.as_str()).collect();
    assert!(
        type_names.iter().any(|n| n.starts_with("Box$Integer")),
        "expected Box$Integer, got {type_names:?}"
    );
    assert!(
        !type_names.iter().any(|n| n.starts_with("Box$Unknown")),
        "Box$Unknown must not be instantiated: {type_names:?}"
    );
}

#[test]
fn record_member_access_and_with_update_types_are_inferred() {
    // Member access and WITH-update expression typing feed a generic call so
    // the corresponding expression_type arms run.
    let src = "\
IMPORT io
TYPE Point
  x AS Integer
  y AS Integer
END TYPE
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  LET p AS Point = Point[1, 2]
  emit(p.x)
  LET q AS Point = WITH p { x := 9 }
  emit(q.y)
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    assert!(function_names(&project).iter().any(|n| n == "emit$Integer"));
}

#[test]
fn list_literal_and_string_concat_and_unary_types() {
    // List literal element typing, string-concat `&`, comparison, and unary
    // NOT all drive distinct expression_type branches through generic calls.
    let src = "\
IMPORT io
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  LET xs AS List OF String = [\"a\", \"b\"]
  emit(xs)
  LET joined AS String = \"x\" & \"y\"
  emit(joined)
  LET flag AS Boolean = NOT (1 < 2)
  emit(flag)
  LET sum AS Integer = 1 + 2
  emit(sum)
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    let names = function_names(&project);
    assert!(names.iter().any(|n| n == "emit$String"), "{names:?}");
    assert!(names.iter().any(|n| n == "emit$Boolean"), "{names:?}");
    assert!(names.iter().any(|n| n == "emit$Integer"), "{names:?}");
    assert!(
        names.iter().any(|n| n == "emit$List$OF$String"),
        "{names:?}"
    );
}

#[test]
fn general_builtin_override_selected_for_user_type() {
    // A user `FUNC toString(p AS Point)` overrides the general built-in for
    // its own type; the call routes to the mangled override symbol
    // (resolve_general_builtin_override).
    let src = "\
IMPORT io
TYPE Point
  x AS Integer
END TYPE
FUNC toString(p AS Point) AS String
  RETURN \"point\"
END FUNC
FUNC main() AS Integer
  LET p AS Point = Point[1]
  io::print(toString(p))
  RETURN 0
END FUNC
";
    let project = monomorphize(src).expect("monomorphizes");
    // The override is force-mangled so it never equals the built-in name.
    assert!(
        function_names(&project)
            .iter()
            .any(|n| n.starts_with("toString$")),
        "{:?}",
        function_names(&project)
    );
}

#[test]
fn overload_no_match_leaves_call_unresolved() {
    // Two overloads exist but neither matches the argument types; the call is
    // left as the bare name (resolve_overload returns None, no error).
    let src = "\
IMPORT io
FUNC pick(n AS Integer) AS String
  RETURN \"i\"
END FUNC
FUNC pick(s AS String) AS String
  RETURN \"s\"
END FUNC
FUNC main() AS Integer
  LET flag AS Boolean = TRUE
  io::print(pick(flag))
  RETURN 0
END FUNC
";
    // No matching overload for Boolean: monomorph does not error (resolution
    // is left to later stages), it simply leaves the callee unresolved.
    let project = monomorphize(src).expect("monomorphizes");
    // Both overloads still emitted under their mangled names.
    assert!(function_names(&project).iter().any(|n| n == "pick$Integer"));
}

#[test]
fn return_type_overload_ambiguous_without_expected_type_errors() {
    // A return-type overload set called with no expected (contextual) type is
    // ambiguous -> TYPE_OVERLOAD_AMBIGUOUS (resolve_overload error arm).
    let src = "\
IMPORT io
FUNC make() AS Integer
  RETURN 1
END FUNC
FUNC make() AS String
  RETURN \"one\"
END FUNC
FUNC emit OF T(value AS T) AS Nothing
  io::print(toString(value))
END FUNC
FUNC main() AS Integer
  emit(make())
  RETURN 0
END FUNC
";
    assert!(monomorphize(src).is_err());
}

#[test]
fn template_argument_unification_failure_errors() {
    // A `List OF T` parameter given a non-list argument cannot infer T ->
    // TYPE_CALL_ARGUMENT_MISMATCH (the unify-failure arm of instantiate).
    let src = "\
IMPORT io
IMPORT collections
FUNC firstOf OF T(items AS List OF T) AS T
  RETURN collections::get(items, 0)
END FUNC
FUNC main() AS Integer
  io::print(toString(firstOf(42)))
  RETURN 0
END FUNC
";
    assert!(monomorphize(src).is_err());
}
