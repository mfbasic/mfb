//! plan-176-G: `ir::shape`'s walker-parity, named-argument and argument-type tests. The
//! shared `hir_from`/`shape_codes`/`rejects_with`/`accepts` helpers live in `tests/mod.rs`.

use super::*;

fn wrap_import(import: &str, body: &str) -> String {
    format!("IMPORT {import}\nFUNC main AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
}

/// Every named `Bind` in the lowered IR (the `$` temps lowering mints have
/// no HIR counterpart), in emission order.
fn lowered_binds(ops: &[IrOp], out: &mut Vec<(String, ParameterType)>) {
    for op in ops {
        match op {
            IrOp::Bind { name, type_, .. } => {
                if !name.starts_with('$') {
                    out.push((name.clone(), type_.clone()));
                }
            }
            IrOp::If {
                then_body,
                else_body,
                ..
            } => {
                lowered_binds(then_body, out);
                lowered_binds(else_body, out);
            }
            IrOp::Match { cases, .. } => {
                for case in cases {
                    lowered_binds(&case.body, out);
                }
            }
            // The FOR EACH element is bound by the loop op itself, not by a
            // `Bind` inside its body.
            IrOp::ForEach {
                name, type_, body, ..
            } => {
                out.push((name.clone(), type_.clone()));
                lowered_binds(body, out);
            }
            IrOp::For { body, .. }
            | IrOp::While { body, .. }
            | IrOp::DoUntil { body, .. }
            | IrOp::Trap { body, .. } => lowered_binds(body, out),
            _ => {}
        }
    }
}

// The seam-fidelity proof: the walker's scope tracking (parameters, LET
// with/without annotation, inline-TRAP LET, FOR with numeric promotion,
// FOR EACH element, MATCH case binding, trap binding, lambda parameter)
// yields, for every binding, the type lowering stamps on its `IrOp::Bind`.
#[test]
fn walker_types_bindings_exactly_as_lowering_does() {
    let src = "IMPORT collections\n\
                   TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
                   UNION Shape\n  Point\nEND UNION\n\
                   FUNC helper(n AS Integer) AS Float\n  RETURN n * 1.5\nEND FUNC\n\
                   FUNC main AS Integer\n\
                   \x20 LET a = 1\n\
                   \x20 LET b AS Float = 2.5\n\
                   \x20 LET c = helper(a)\n\
                   \x20 LET items = [1, 2, 3]\n\
                   \x20 FOR i = a TO 10 STEP 0.5\n\
                   \x20   LET d = i\n\
                   \x20 NEXT\n\
                   \x20 FOR EACH item IN items\n\
                   \x20   LET e = item\n\
                   \x20 NEXT\n\
                   \x20 LET s AS Shape = Point[1, 2]\n\
                   \x20 MATCH s\n\
                   \x20   CASE Point(p)\n\
                   \x20     LET f = p\n\
                   \x20 END MATCH\n\
                   \x20 LET g = collections::filter(items, LAMBDA(v AS Integer) -> v > 1)\n\
                   \x20 LET h = helper(a) TRAP(err)\n\
                   \x20   LET i2 = err\n\
                   \x20   RECOVER 0.0\n\
                   \x20 END TRAP\n\
                   \x20 RETURN 0\n\
                   TRAP(e2)\n\
                   \x20 LET j = e2\n\
                   \x20 RETURN 1\n\
                   END TRAP\n\
                   END FUNC\n";
    let hir = hir_from(src);
    let facts = lower::lower_facts(&hir, &HashMap::new(), &[], &[]);
    let no_imports = HashMap::new();
    let mut walker = Walker::new(Path::new("/proj"), &facts, &hir, &[], &no_imports, &[]);
    walker.walk_project(&hir);
    let emitted: Vec<_> = walker
        .diagnostics
        .iter()
        .map(|d| format!("{}:{} {}", d.line, d.rule, d.detail))
        .collect();
    assert!(
        emitted.is_empty(),
        "a clean program emits nothing: {emitted:?}"
    );

    let ir = lower::lower_augmented_project(&hir, None, &HashMap::new(), &[], &[]);
    let mut lowered = Vec::new();
    for function in ir.functions.iter().filter(|f| f.name == "main") {
        lowered_binds(&function.body, &mut lowered);
    }
    // Lowering emits the MATCH case binding and the FOR loop variable as
    // `Bind`s of their own; the walker records them through `bind` too, so
    // the two sequences line up name-for-name.
    let walked: Vec<_> = walker
        .bound_types
        .iter()
        .filter(|(name, _)| !name.starts_with('$'))
        .cloned()
        .collect();
    let lowered_by_name: HashMap<_, _> = lowered.iter().cloned().collect();
    for (name, type_) in &walked {
        let stamped = lowered_by_name
            .get(name)
            .unwrap_or_else(|| panic!("lowering emitted no Bind for `{name}`"));
        assert_eq!(stamped, type_, "binding `{name}` typed differently");
    }
    // Coverage of the scope forms the walker mirrors.
    let names: Vec<_> = walked.iter().map(|(n, _)| n.as_str()).collect();
    for expected in [
        "a", "b", "c", "items", "i", "d", "item", "e", "s", "f", "g", "h", "i2", "j",
    ] {
        assert!(names.contains(&expected), "walker never bound `{expected}`");
    }
    // The promoted FOR type and the inline-TRAP success type are the
    // non-trivial computations; pin them so a silent fallback to `Unknown`
    // on both sides cannot pass.
    assert_eq!(lowered_by_name["i"], ParameterType::Float);
    assert_eq!(lowered_by_name["h"], ParameterType::Float);
    assert_eq!(lowered_by_name["e"], ParameterType::Integer);
}

// The standalone entry renders nothing and passes on a clean program.
#[test]
fn check_project_accepts_clean_source() {
    let hir = hir_from("FUNC main AS Integer\n  RETURN 0\nEND FUNC\n");
    assert!(check_project(Path::new("/proj"), &hir, &HashMap::new()).is_ok());
}

// ---- named arguments: user functions ----------------------------------

#[test]
fn user_named_argument_valid() {
    assert!(accepts(
        "FUNC g(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(b := 2, a := 1)\nEND FUNC\n"
    ));
}

#[test]
fn user_named_argument_unknown_name() {
    assert!(rejects_with(
        "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(z := 1)\nEND FUNC\n",
        "TYPE_UNKNOWN_ARGUMENT_NAME"
    ));
}

#[test]
fn user_named_argument_duplicate() {
    // `g(1, a := 2)`: the positional already filled `a`.
    assert!(rejects_with(
        "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(1, a := 2)\nEND FUNC\n",
        "TYPE_DUPLICATE_ARGUMENT_NAME"
    ));
}

#[test]
fn user_named_positional_after_named_walk() {
    // A positional after a named argument fills the first free slot.
    assert!(accepts(
        "FUNC g(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(b := 2, 1)\nEND FUNC\n"
    ));
}

#[test]
fn user_private_function_in_another_file_is_not_a_call_target() {
    // The source checker never bound names against a function invisible
    // from the calling file; the rule stays silent there too.
    let main = parse_source(
        Path::new("main.mfb"),
        "main.mfb",
        "FUNC main AS Integer\n  RETURN g(z := 1)\nEND FUNC\n",
    )
    .expect("parses");
    let other = parse_source(
        Path::new("other.mfb"),
        "other.mfb",
        "PRIVATE FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\n",
    )
    .expect("parses");
    let project = AstProject {
        name: "test".to_string(),
        files: vec![main, other],
    };
    let hir =
        crate::resolver::augment_hir_project(&crate::hir::elaborate(&project)).expect("augments");
    let codes: Vec<_> =
        collect_diagnostics(Path::new("/proj"), &hir, &[], &[], &HashMap::new(), &[])
            .into_iter()
            .map(|d| d.rule)
            .collect();
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn imported_package_function_named_arguments() {
    // A `.mfp` function's parameter names come from the imported-signature
    // table; the call is spelled through the file's import binding.
    let mut imported = HashMap::new();
    imported.insert(
        "shapes.area".to_string(),
        ExternalSignature {
            params: vec![
                crate::ir::ExternalFunctionParam {
                    name: "width".to_string(),
                    type_: ParameterType::Integer,
                    default: crate::ir::ExternalDefault::None,
                },
                crate::ir::ExternalFunctionParam {
                    name: "height".to_string(),
                    type_: ParameterType::Integer,
                    default: crate::ir::ExternalDefault::None,
                },
            ],
            returns: ParameterType::Integer,
            isolated: false,
        },
    );
    let src = "IMPORT shapes AS sh\nFUNC main AS Integer\n  LET a = sh::area(width := 1, depth := 2)\n  LET b = sh::area(1, width := 2)\n  RETURN a + b\nEND FUNC\n";
    let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parses");
    let project = AstProject {
        name: "test".to_string(),
        files: vec![file],
    };
    let hir = crate::hir::elaborate(&project);
    let diagnostics = collect_diagnostics(Path::new("/proj"), &hir, &[], &[], &imported, &[]);
    let codes: Vec<_> = diagnostics.iter().map(|d| d.rule.as_str()).collect();
    // Line 3 supplies one bindable name of two required parameters, so the
    // arity rule follows the unknown name; line 4's duplicate leaves `height`
    // unsupplied, so it is followed by the arity rule too.
    assert_eq!(
        codes,
        [
            "TYPE_UNKNOWN_ARGUMENT_NAME",
            "TYPE_CALL_ARITY_MISMATCH",
            "TYPE_DUPLICATE_ARGUMENT_NAME",
            "TYPE_CALL_ARITY_MISMATCH",
        ]
    );
    assert_eq!(
        diagnostics[0].detail,
        "Call to `sh.area` does not have a parameter named `depth`."
    );
    assert_eq!(diagnostics[0].line, 3);
    assert_eq!(diagnostics[0].path, Path::new("/proj/main.mfb"));
}

// ---- named arguments: builtins ----------------------------------------

#[test]
fn builtin_named_argument_valid() {
    assert!(accepts(
        "IMPORT json\nIMPORT io\nFUNC main AS Integer\n  io::print(json::stringify(json::parse(value := \"null\")))\n  RETURN 0\nEND FUNC\n"
    ));
}

#[test]
fn builtin_named_argument_unknown_name() {
    assert!(rejects_with(
        "IMPORT json\nFUNC main AS Integer\n  LET x = json::parse(nope := \"null\")\n  RETURN 0\nEND FUNC\n",
        "TYPE_UNKNOWN_ARGUMENT_NAME"
    ));
}

#[test]
fn builtin_named_argument_matching_parameter_name_accepted() {
    // Every registry builtin carries its parameter names (`math::abs`'s is
    // `value`), so a matching name binds; the source checker's old test of
    // this call assumed a name-less fallback that no builtin reaches today.
    assert!(accepts(
        "IMPORT math\nFUNC main AS Integer\n  LET x = math::abs(value := -1)\n  RETURN 0\nEND FUNC\n"
    ));
}

#[test]
fn builtin_named_argument_duplicate() {
    assert!(rejects_with(
        "IMPORT json\nFUNC main AS Integer\n  LET x = json::parse(\"a\", value := \"b\")\n  RETURN 0\nEND FUNC\n",
        "TYPE_DUPLICATE_ARGUMENT_NAME"
    ));
}

#[test]
fn builtin_named_then_positional_reorders() {
    assert!(accepts(
        "IMPORT strings\nFUNC main AS Integer\n  LET b = strings::startsWith(prefix := \"a\", \"abc\")\n  RETURN 0\nEND FUNC\n"
    ));
}

#[test]
fn builtin_general_call_named_argument() {
    // `general` members dispatch through their own arm in the source
    // checker; the rule reaches them all the same.
    assert!(rejects_with(
        "FUNC main AS Integer\n  LET s = toString(v := 1)\n  RETURN 0\nEND FUNC\n",
        "TYPE_UNKNOWN_ARGUMENT_NAME"
    ));
}

#[test]
fn overloaded_named_unknown_argument_rejected() {
    assert!(rejects_with(
        &wrap_import("datetime", "  LET z = datetime::fixedOffset(bogus := 1)"),
        "TYPE_UNKNOWN_ARGUMENT_NAME"
    ));
}

#[test]
fn overloaded_named_duplicate_argument_rejected() {
    assert!(rejects_with(
        &wrap_import(
            "datetime",
            "  LET z = datetime::fixedOffset(hours := 1, hours := 2)"
        ),
        "TYPE_DUPLICATE_ARGUMENT_NAME"
    ));
}

#[test]
fn overloaded_duplicate_reported_once_and_before_unknown_names() {
    // A duplicate ends overload selection before any unknown name is
    // considered, and only the first duplicate is reported; the count rule
    // still runs over the source-order list the selection fell back to.
    let codes = shape_codes(&wrap_import(
        "datetime",
        "  LET z = datetime::fixedOffset(hours := 1, hours := 2, bogus := 3, bogus := 4)",
    ));
    assert_eq!(
        codes,
        [
            "TYPE_DUPLICATE_ARGUMENT_NAME",
            "TYPE_CALL_ARITY_MISMATCH",
            "TYPE_UNKNOWN_VALUE",
        ]
    );
}

#[test]
fn overloaded_named_valid_selection_accepted() {
    assert!(accepts(&wrap_import(
        "datetime",
        "  LET z = datetime::fixedOffset(hours := 1, mins := 2)",
    )));
}

#[test]
fn function_value_call_count_rejected() {
    // A local of FUNC type takes exactly its type's parameter count.
    let src = "FUNC main AS Integer\n  LET f AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + 1\n  LET r AS Integer = f(1, 2)\n  RETURN r\nEND FUNC\n";
    assert_eq!(shape_codes(src), ["TYPE_CALL_ARITY_MISMATCH"]);
    // The same program through the monomorphizer, as the build runs it.
    let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parses");
    let project = AstProject {
        name: "test".to_string(),
        files: vec![file],
    };
    let augmented = crate::resolver::augment_project(&project, false).expect("augments");
    let concrete =
        crate::monomorph::monomorphize_project(Path::new("."), &crate::hir::elaborate(&augmented))
            .expect("monomorphizes");
    let codes: Vec<_> = collect_diagnostics(
        Path::new("/proj"),
        &concrete,
        &[],
        &[],
        &HashMap::new(),
        &[],
    )
    .into_iter()
    .map(|d| d.rule)
    .collect();
    assert_eq!(codes, ["TYPE_CALL_ARITY_MISMATCH"]);
}

// ---- argument types: the source-path forms ----------------------------

#[test]
fn declared_function_argument_type_mismatch() {
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "FUNC g(a AS Integer, b AS String = \"x\") AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(\"no\")\nEND FUNC\n",
        ),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
    // Only the SUPPLIED argument is judged; the defaulted `b` is not.
    assert_eq!(
        details,
        ["Argument 1 for `g` has type String, expected Integer."]
    );
}

#[test]
fn declared_function_literal_coercions_accepted() {
    // A `Byte`/`Fixed` parameter accepts a fitting literal, and a list of
    // literals coerces element by element (the checker's rules).
    assert!(accepts(
        "FUNC g(a AS Byte, b AS Fixed, c AS List OF Fixed) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN g(200, -1, [1, 2.5])\nEND FUNC\n"
    ));
}

#[test]
fn builtin_table_argument_type_mismatch() {
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "IMPORT math\nFUNC main AS Integer\n  LET p = math::pow(\"a\", 2)\n  RETURN 0\nEND FUNC\n",
        ),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
    // The failed call types the binding Unknown: the cascade follows.
    assert_eq!(details.len(), 2, "{details:?}");
    assert!(
        details[0]
            .starts_with("Call to `math.pow` has argument type(s) (String, Integer), expected"),
        "{details:?}"
    );
    assert_eq!(
        details[1],
        "Initializer for binding `p` does not have a known type."
    );
}

#[test]
fn general_error_call_argument_mismatch() {
    // `error(code, message)` lowers to record constructors, so only the
    // shape pass can judge its arguments.
    assert!(rejects_with(
        "FUNC main AS Integer\n  LET e = error(1)\n  RETURN 0\nEND FUNC\n",
        "TYPE_CALL_ARGUMENT_MISMATCH"
    ));
}

#[test]
fn thread_start_entry_must_be_an_isolated_func() {
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "IMPORT thread\nFUNC main AS Integer\n  RES t = thread::start(main, \"x\", 1, 1)\n  RETURN 0\nEND FUNC\n",
        ),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
    // The entry rejection ends the call's checks (no count or type form
    // follows) and types the call Unknown, so the binding cascades.
    assert_eq!(
        details,
        [
            "thread.start entry point must name an ISOLATED FUNC.",
            "Initializer for binding `t` does not have a known type.",
        ]
    );
}

/// plan-115-A: the positive half of the rule above. A bare, unqualified
/// `ISOLATED FUNC` of the current project is a valid entry — the case
/// `thread_start_entry_valid` never even considered before (it bailed at
/// `split_once('.')`). Added because the negative test alone cannot tell
/// "the rule narrowed to ISOLATED" from "the rule rejects every bare name",
/// which is exactly the regression this phase could introduce.
#[test]
fn thread_start_accepts_a_bare_local_isolated_func() {
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "IMPORT thread\n\
                 ISOLATED FUNC w(t AS ThreadWorker OF Nothing TO Integer, seed AS Integer) AS Integer\n  \
                 RETURN seed\n\
                 END FUNC\n\
                 FUNC main AS Integer\n  \
                 RES t = thread::start(w, 1)\n  \
                 RETURN 0\n\
                 END FUNC\n",
        ),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
    assert!(
        details.is_empty(),
        "a bare local ISOLATED FUNC must be a valid entry, got {details:?}"
    );
}

/// plan-115-A: visibility is irrelevant to entry validity, so the `PRIVATE`
/// spelling of the test above must behave identically. Pinned separately
/// because `PRIVATE` is the visibility bug-227 rejected and this plan lifts.
#[test]
fn thread_start_accepts_a_bare_private_isolated_func() {
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "IMPORT thread\n\
                 PRIVATE ISOLATED FUNC w(t AS ThreadWorker OF Nothing TO Integer, seed AS Integer) AS Integer\n  \
                 RETURN seed\n\
                 END FUNC\n\
                 FUNC main AS Integer\n  \
                 RES t = thread::start(w, 1)\n  \
                 RETURN 0\n\
                 END FUNC\n",
        ),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
    assert!(
        details.is_empty(),
        "a bare PRIVATE ISOLATED FUNC must be a valid entry, got {details:?}"
    );
}

#[test]
fn term_draw_text_attributed_requires_astrings_import() {
    // An `AttributedString` value reaches the call through a parameter
    // without any `IMPORT astrings`, so the bridge body is not injected.
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "IMPORT term\nFUNC show(a AS AttributedString) AS Integer\n  term::drawText(1, 1, a)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        ),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
    assert_eq!(
        details,
        ["Call to `term.drawText` with an `AttributedString` requires `IMPORT astrings`."]
    );
}

#[test]
fn term_measure_text_attributed_requires_astrings_import() {
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "IMPORT term\nFUNC width(a AS AttributedString) AS Integer\n  RETURN term::measureText(a)\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        ),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
    assert_eq!(
        details,
        ["Call to `term.measureText` with an `AttributedString` requires `IMPORT astrings`."]
    );
}

#[test]
fn cascade_spares_typed_seam_cases() {
    // A package override of an overridable general builtin types by the
    // builtin's result (`toString(net::Url)` → String), and a MATCH on a
    // union that INCLUDES another binds the included variants — neither
    // is an `Unknown` for the checker.
    assert!(accepts(
        "IMPORT net\nTYPE Circle\n  r AS Integer\nEND TYPE\nUNION Shape\n  Circle\nEND UNION\nUNION Extra INCLUDES Shape\n  Circle\nEND UNION\nFUNC score(s AS Extra) AS Integer\n  MATCH s\n    CASE Circle(c)\n      RETURN c.r + 1\n  END MATCH\nEND FUNC\nFUNC main AS Integer\n  LET u AS net::Url = net::toUrl(\"http://x/\")\n  LET rendered AS String = toString(u)\n  RETURN len(rendered)\nEND FUNC\n"
    ));
}

// ---- export_in_executable_diagnostics (build-pipeline entry point) ------

#[test]
fn export_in_executable_flags_each_item_kind() {
    use crate::ast::{parse_source, AstProject};
    // Cover every item kind the visibility match walks: binding, type,
    // function, resource (HirItem::Resource arm), and func alias (HirItem::FuncAlias
    // arm). The resource/alias targets need only parse — this entry point runs
    // before import resolution.
    let src = "EXPORT LET g AS Integer = 5\nEXPORT TYPE Rec\n  x AS Integer\nEND TYPE\nEXPORT FUNC f() AS Integer\n  RETURN 1\nEND FUNC\nEXPORT RESOURCE Db CLOSE BY x::close\nEXPORT FUNC ff AS x::gg\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
    let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse");
    let project = AstProject {
        name: "t".to_string(),
        files: vec![file],
    };
    let diags = export_in_executable_diagnostics(false, &project);
    assert!(diags.iter().all(|d| d.rule == "EXPORT_IN_EXECUTABLE"));
    assert!(diags.len() >= 5, "expected an EXPORT diagnostic per item");
}

#[test]
fn export_in_executable_empty_for_package_project() {
    use crate::ast::{parse_source, AstProject};
    let src = "EXPORT FUNC f() AS Integer\n  RETURN 1\nEND FUNC\n";
    let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse");
    let project = AstProject {
        name: "t".to_string(),
        files: vec![file],
    };
    // A package project never flags EXPORT (that is its purpose).
    assert!(export_in_executable_diagnostics(true, &project).is_empty());
}

#[test]
fn export_in_executable_no_export_no_diagnostic() {
    use crate::ast::{parse_source, AstProject};
    let src =
        "FUNC f() AS Integer\n  RETURN 1\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
    let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse");
    let project = AstProject {
        name: "t".to_string(),
        files: vec![file],
    };
    assert!(export_in_executable_diagnostics(false, &project).is_empty());
}

#[test]
fn matched_table_builtin_call_with_an_unknown_argument_is_typed() {
    // `Hash.SHA224` is an unknown enum member (verify's
    // TYPE_UNKNOWN_ENUM_MEMBER); the checker still typed the matched
    // `crypto::hash` overload by its declared return type, so the binding
    // does not cascade.
    let src = "IMPORT crypto\nFUNC main AS Integer\n  LET a AS List OF Byte = crypto::hash(crypto::Hash.SHA224, \"abc\")\n  RETURN len(a)\nEND FUNC\n";
    assert_eq!(shape_codes(src), Vec::<String>::new());
}
