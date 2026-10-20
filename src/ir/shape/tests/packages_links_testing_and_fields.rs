//! plan-176-G: `ir::shape`'s package-metadata, link/constructor/money, testing and
//! control-flow, and foreign-record-field tests. The shared helpers live in `tests/mod.rs`.

use super::*;

// ---- imported package metadata ---------------------------------------

#[test]
fn corrupt_package_tables_are_package_invalid() {
    // A garbage `.mfp` reaches this pass only when the decode boundary is
    // bypassed (as a unit test does); each of the three table reads
    // reports, in the checker's order: types, resources, then functions.
    let dir = std::env::temp_dir().join(format!("mfb_shape_pkg_{}", std::process::id()));
    let pkgs = dir.join("packages");
    std::fs::create_dir_all(&pkgs).unwrap();
    std::fs::write(pkgs.join("brokenpkg.mfp"), b"not a valid mfp container").unwrap();
    let file = crate::ast::parse_source(
        Path::new("main.mfb"),
        "main.mfb",
        "IMPORT brokenpkg\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
    )
    .unwrap();
    let hir = crate::hir::elaborate(&crate::ast::AstProject {
        name: "t".into(),
        files: vec![file],
    });
    let diagnostics = collect_diagnostics(&dir, &hir, &[], &[], &HashMap::new(), &[]);
    let _ = std::fs::remove_dir_all(&dir);
    let details: Vec<_> = diagnostics
        .iter()
        .map(|d| (d.rule.as_str(), d.detail.as_str(), d.line))
        .collect();
    assert_eq!(details.len(), 3, "{details:?}");
    assert!(details
        .iter()
        .all(|(rule, _, line)| *rule == "PACKAGE_INVALID" && *line == 1));
    assert!(details[0]
        .1
        .ends_with("has unreadable or invalid type metadata."));
    assert!(details[1].1.ends_with("has an unreadable resource table."));
    assert!(details[2]
        .1
        .ends_with("has unreadable or invalid function metadata."));
}

#[test]
fn package_type_validation_arms() {
    let src = "ENUM Color\n  Red, Green\nEND ENUM\nTYPE Point\n  x AS Integer\nEND TYPE\nUNION Shape\n  Point\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
    let hir = hir_from(src);
    let facts = lower::lower_facts(&hir, &HashMap::new(), &[], &[]);
    let no_imports = HashMap::new();
    let mut walker = Walker::new(Path::new("/proj"), &facts, &hir, &[], &no_imports, &[]);
    let pkg = Path::new("packages/fake.mfp");
    let validate = |walker: &mut Walker, type_: &ParameterType| {
        walker.validate_package_type(pkg, type_, "ctx", 7, &mut HashSet::new());
        walker
            .diagnostics
            .drain(..)
            .map(|d| d.detail)
            .collect::<Vec<_>>()
    };
    // A map keyed by a List is not comparable; its Res value recurses silently.
    let map = ParameterType::MapOf(
        Box::new(ParameterType::ListOf(Box::new(ParameterType::Integer))),
        Box::new(ParameterType::Res(Box::new(ParameterType::String))),
    );
    assert_eq!(
        validate(&mut walker, &map),
        ["Imported package `packages/fake.mfp` has ctx with non-comparable map key type `List OF Integer`."]
    );
    // An undeclared nominal, directly and through a wrapper.
    assert_eq!(
        validate(&mut walker, &ParameterType::named("Nope")),
        ["Imported package `packages/fake.mfp` has ctx that references unknown type `Nope`."]
    );
    assert_eq!(
        validate(
            &mut walker,
            &ParameterType::SetOf(Box::new(ParameterType::named("Nope")))
        )
        .len(),
        1
    );
    // Declared enum / record / union, built-in nominals, resources, and the
    // structural shapes all walk silently.
    for accepted in [
        ParameterType::named("Color"),
        ParameterType::named("Point"),
        ParameterType::named("Shape"),
        ParameterType::named("Error"),
        ParameterType::named("tcp.Socket"),
        ParameterType::ResultOf(Box::new(ParameterType::Money)),
        ParameterType::Func(
            vec![ParameterType::Integer, ParameterType::String],
            Box::new(ParameterType::Boolean),
            false,
        ),
        ParameterType::ThreadHandle {
            worker: false,
            msg: Box::new(ParameterType::Integer),
            res: Box::new(ParameterType::String.with_state(&ParameterType::Boolean)),
            out: Box::new(ParameterType::Float),
        },
        ParameterType::ThreadHandle {
            worker: true,
            msg: Box::new(ParameterType::Integer),
            res: Box::new(ParameterType::Nothing),
            out: Box::new(ParameterType::Nothing),
        },
    ] {
        assert!(
            validate(&mut walker, &accepted).is_empty(),
            "{}",
            accepted.name()
        );
    }
    // plan-111-B: a stateful resource must be recognized as a RESOURCE both
    // when validating an imported signature and when asking comparability.
    // Both paths route "every other spelling" through a re-wrap into one
    // opaque nominal, and the structural `without_state` cannot peel a
    // re-wrapped spelling — so both need the clause peeled BEFORE the
    // re-wrap. Without that, `Db STATE DbInfo` reads as an unknown type
    // here and as a COMPARABLE one below.
    //
    // An imported package's stateful resource (`Db STATE DbInfo`, as a
    // signature spells it) is a resource, not an unknown type — the
    // libsnd / native-resource-state fixtures build clean.
    let mut walker = Walker::new(
        Path::new("/proj"),
        &facts,
        &hir,
        &[],
        &no_imports,
        &["Db".to_string()],
    );
    for spelled in ["Db", "Db STATE DbInfo", "pkg.Db STATE DbInfo"] {
        assert!(
            validate(&mut walker, &ParameterType::parse(spelled)).is_empty(),
            "{spelled}"
        );
        // ...and a resource is never comparable, stateful or not.
        assert!(
            !walker.is_comparable(&ParameterType::parse(spelled)),
            "`{spelled}` is a resource and must not be comparable"
        );
    }
    // A stateful type whose base is NOT a resource keeps the opaque-nominal
    // behaviour: unknown to the package validator, permissively comparable.
    assert!(
        !validate(&mut walker, &ParameterType::parse("Nope STATE S")).is_empty(),
        "a non-resource stateful type is still an unknown type"
    );
    assert!(walker.is_comparable(&ParameterType::parse("Nope STATE S")));
    // A union whose variant record carries an unknown field type reports it.
    assert_eq!(
        validate(
            &mut walker,
            &ParameterType::ThreadHandle {
                worker: false,
                msg: Box::new(ParameterType::named("Ghost")),
                res: Box::new(ParameterType::Nothing),
                out: Box::new(ParameterType::Nothing),
            }
        )
        .len(),
        1
    );
}

/// bug-823: an imported package's exported signatures and type fields may
/// name any builtin package's type, and the importer resolves it whether or
/// not it imports that builtin itself. The walker here imports nothing, so
/// no builtin declaration is in its type table.
#[test]
fn a_dependency_s_builtin_types_resolve_without_the_importer_s_import() {
    let hir = hir_from("FUNC main AS Integer\n  RETURN 0\nEND FUNC\n");
    let facts = lower::lower_facts(&hir, &HashMap::new(), &[], &[]);
    let no_imports = HashMap::new();
    let mut walker = Walker::new(Path::new("/proj"), &facts, &hir, &[], &no_imports, &[]);
    let pkg = Path::new("packages/fake.mfp");
    let mut validate = |type_: &ParameterType| {
        walker.validate_package_type(pkg, type_, "ctx", 7, &mut HashSet::new());
        walker
            .diagnostics
            .drain(..)
            .map(|d| d.detail)
            .collect::<Vec<_>>()
    };
    // Every builtin value type an injected builtin source exports. The
    // project imports every builtin package, so this is the whole set a
    // package's signatures can name, not a hand-picked sample.
    let everything = hir_from(&format!(
        "{}FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        builtins::BUILTIN_IMPORTS
            .iter()
            .map(|package| format!("IMPORT {package}\n"))
            .collect::<String>()
    ));
    let exported: Vec<String> = everything
        .files
        .iter()
        .filter(|file| file.internal)
        .flat_map(|file| file.items.iter())
        .filter_map(|item| match item {
            HirItem::Type(decl) if decl.visibility == Visibility::Export => Some(decl.name.clone()),
            _ => None,
        })
        .collect();
    for name in ["json.Json", "datetime.DateTime", "crypto.KeyPair"] {
        assert!(
            exported.iter().any(|seen| seen == name),
            "{name}: {exported:?}"
        );
    }
    for name in &exported {
        for type_ in [
            ParameterType::named(name),
            ParameterType::ListOf(Box::new(ParameterType::named(name))),
        ] {
            assert_eq!(validate(&type_), Vec::<String>::new(), "{name}");
        }
    }
    // Only a builtin package's own declared type resolves this way: an
    // unknown leaf under a builtin qualifier, and a user package's type the
    // importer has no definition for, are still reported.
    for unknown in ["json.Nope", "jpkg.Json"] {
        assert_eq!(
            validate(&ParameterType::named(unknown)),
            [format!(
                "Imported package `packages/fake.mfp` has ctx that references unknown type `{unknown}`."
            )],
        );
    }
    // bug-466: resolving a dependency's builtin type must not make its
    // FIELDS readable. The field rule's owner map still holds the record,
    // so `d.date` on a `datetime::DateTime` still needs `IMPORT datetime`.
    assert_eq!(
        walker
            .builtin_record_owner
            .get(&ParameterType::named("datetime.DateTime")),
        Some(&"datetime")
    );
}

// ---- native LINK facts lowering folds away ---------------------------

fn link_project(body: &str) -> String {
    format!(
        "RESOURCE Handle CLOSE BY demoLink::release\nLINK \"demo\" AS demoLink\n  FUNC release(h AS Handle) AS Nothing\n    SYMBOL \"demo_release\"\n    ABI (h CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n{body}END LINK\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
    )
}

#[test]
fn const_pin_must_fold_to_an_immediate() {
    let src = link_project(
        "  FUNC f(n AS Integer) AS Integer\n    SYMBOL \"demo_f\"\n    ABI (n CInt32, s CInt32, t CInt32, u CInt32, value OUT CInt32) AS status CInt32\n    CONST s = \"literal\"\n    CONST t = 1 + 1\n    CONST u = -1\n    RETURN value\n    SUCCESS_ON status = 0\n  END FUNC\n",
    );
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(&src),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let lines: Vec<_> = diagnostics
        .iter()
        .map(|d| (d.rule.as_str(), d.line))
        .collect();
    // `s` and `t` are unfoldable; the negated literal `u` folds.
    assert_eq!(
        lines,
        [
            ("NATIVE_CONST_UNKNOWN_SLOT", 11),
            ("NATIVE_CONST_UNKNOWN_SLOT", 12)
        ]
    );
}

#[test]
fn free_deallocator_signature() {
    let describe = |abi: &str| {
        link_project(&format!(
            "  FUNC describe(h AS Handle) AS String\n    SYMBOL \"demo_describe\"\n    ABI (h CPtr) AS text CPtr\n    RETURN text\n    FREE text\n      SYMBOL \"demo_free\"\n      ABI {abi}\n    END FREE\n  END FUNC\n"
        ))
    };
    assert!(accepts(&describe("(ptr CPtr) AS CVoid")));
    for malformed in ["(ptr CInt32) AS CVoid", "(ptr CPtr) AS CInt32"] {
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(&describe(malformed)),
            &[],
            &[],
            &HashMap::new(),
            &[],
        );
        let lines: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.line))
            .collect();
        assert_eq!(lines, [("NATIVE_FREE_INVALID", 12)], "{malformed}");
    }
}

// ---- record constructors ---------------------------------------------

#[test]
fn error_records_cannot_be_constructed() {
    let src = "FUNC main AS Integer\n  LET e = Error[1, \"boom\"]\n  LET l = ErrorLoc[\"f\", 1]\n  RETURN 0\nEND FUNC\n";
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(src),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics
        .iter()
        .map(|d| (d.rule.as_str(), d.detail.as_str(), d.line))
        .collect();
    assert_eq!(
        details,
        [
            (
                "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                "`Error` is a read-only built-in record and cannot be constructed; use `error(code, message)` to create an Error.",
                2
            ),
            (
                "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                "`ErrorLoc` is a read-only built-in record and cannot be constructed; use `error(code, message)` to create an Error.",
                3
            ),
        ]
    );
    // The `AttributedString` and compiler-owned forms are verify's.
    let src = "FUNC main AS Integer\n  LET a AS AttributedString = AttributedString[\"hi\"]\n  RETURN 0\nEND FUNC\n";
    assert!(!rejects_with(src, "TYPE_READ_ONLY_RECORD_CONSTRUCTOR"));
}

#[test]
fn constructor_named_field_set_twice() {
    let src = "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\nFUNC main AS Integer\n  LET a AS Point = Point[x := 1, x := 2]\n  LET b AS Point = WITH a { y := 10, y := 20 }\n  RETURN a.x + b.y\nEND FUNC\n";
    // The constructor form only; the WITH form is verify's.
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(src),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics
        .iter()
        .map(|d| (d.rule.as_str(), d.detail.as_str(), d.line))
        .collect();
    assert_eq!(
        details,
        [(
            "TYPE_DUPLICATE_FIELD",
            "Constructor `Point` sets field `x` more than once.",
            6
        )]
    );
    // Not for an unconstructible record (the checker never reached the
    // argument check); the read-only rule reports instead.
    let src = "FUNC main AS Integer\n  LET e = Error[code := 1, code := 2]\n  RETURN 0\nEND FUNC\n";
    assert!(!rejects_with(src, "TYPE_DUPLICATE_FIELD"));
}

// ---- the Money exactness nudge ---------------------------------------

#[test]
fn money_scaled_by_a_bare_decimal_literal_warns() {
    let src = |expr: &str| {
        format!("FUNC main AS Integer\n  LET price AS Money = 10.00m\n  LET scaled AS Money = {expr}\n  RETURN 0\nEND FUNC\n")
    };
    for warned in [
        "price * 1.08",
        "1.08 * price",
        "price / 1.08",
        "price * -1.08",
    ] {
        assert_eq!(
            shape_codes(&src(warned)),
            ["MONEY_INEXACT_FLOAT_LITERAL"],
            "{warned}"
        );
    }
    // A suffixed literal is intrinsically typed; a Float variable never
    // warns; `literal / Money` is not the scaling shape; `+` is not scaling.
    for silent in [
        "price * 1.08F",
        "price * 1.08f",
        "price * 2",
        "price + 1.08m",
    ] {
        assert!(
            accepts(&src(silent)),
            "{silent}: {:?}",
            shape_codes(&src(silent))
        );
    }
    let float_var = "FUNC main AS Integer\n  LET price AS Money = 10.00m\n  LET rate AS Float = 1.08\n  LET scaled AS Money = price * rate\n  RETURN 0\nEND FUNC\n";
    assert!(accepts(float_var), "{:?}", shape_codes(float_var));
}

// ---- TESTING assertions lowering expands away -------------------------

fn tcase(body: &str) -> String {
    // The assertion builtins are recognized by name in the Call arm
    // (`is_testing_call`), so a plain FUNC body reaches `check_expect_call`
    // without the TESTING desugaring (`mfb test` only).
    format!("FUNC main AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
}

#[test]
fn expect_arity_forms() {
    assert_eq!(
        shape_codes(&tcase("  expectEqual(1)")),
        ["TESTING_EXPECT_ARITY"]
    );
    // `expectTrap` has a 1–2 range: the range wording.
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(&tcase("  expectTrap()")),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| d.detail.as_str())
            .collect::<Vec<_>>(),
        ["`expectTrap` expects 1\u{2013}2 argument(s), got 0."]
    );
}

#[test]
fn expect_typed_operands_must_be_the_named_type() {
    assert_eq!(
        shape_codes(&tcase("  expectFloat(1, 2)")),
        [
            "TESTING_EXPECT_TYPE_MISMATCH",
            "TESTING_EXPECT_TYPE_MISMATCH"
        ]
    );
    assert!(accepts(&tcase("  expectInteger(1, 2)")));
    assert!(accepts(&tcase("  expectNString(\"a\", \"b\")")));
}

#[test]
fn expect_equal_operands_must_be_comparable_and_printable() {
    assert_eq!(
        shape_codes(&tcase("  expectEqual(\"a\", 1)")),
        ["TESTING_EXPECT_INCOMPARABLE"]
    );
    // A Map is neither `=`-comparable nor printable — both, per operand.
    let body =
        "  LET m AS Map OF String TO Integer = Map OF String TO Integer {}\n  expectEqual(m, m)";
    assert_eq!(
        shape_codes(&tcase(body)),
        [
            "TESTING_EXPECT_INCOMPARABLE",
            "TESTING_EXPECT_NOT_PRINTABLE",
            "TESTING_EXPECT_NOT_PRINTABLE"
        ]
    );
    // A record of comparable fields compares but does not print.
    let src = "TYPE Point\n  x AS Integer\nEND TYPE\nFUNC main AS Integer\n  LET p AS Point = Point[1]\n  expectEqual(p, p)\n  RETURN 0\nEND FUNC\n";
    assert_eq!(
        shape_codes(src),
        [
            "TESTING_EXPECT_NOT_PRINTABLE",
            "TESTING_EXPECT_NOT_PRINTABLE"
        ]
    );
    assert!(accepts(&tcase("  expectEqual(1, 1)")));
    assert!(accepts(&tcase("  expectEqual(1, 2.5)")));
}

#[test]
fn expect_trap_code_must_be_integer() {
    let body =
        "  LET xs AS List OF Integer = [1, 2, 3]\n  expectTrap(collections::get(xs, 0), \"x\")";
    let src = format!("IMPORT collections\n{}", tcase(body));
    assert_eq!(shape_codes(&src), ["TESTING_EXPECT_CODE_TYPE"]);
}

#[test]
fn expect_trap_requires_a_call_to_guard() {
    assert_eq!(
        shape_codes(&tcase("  expectTrap(5)")),
        ["TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"]
    );
    assert_eq!(
        shape_codes(&tcase("  expectNTrap(42)")),
        ["TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"]
    );
    let src = format!("IMPORT math\n{}", tcase("  expectTrap(math::pi)"));
    assert_eq!(shape_codes(&src), ["TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"]);
    let src = format!("IMPORT math\n{}", tcase("  expectTrap(math::pi())"));
    assert_eq!(shape_codes(&src), ["TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"]);
}

// ---- control-flow shapes lowering erases -------------------------------

#[test]
fn loop_exit_tail_inside_a_handler_is_shapes() {
    // `treeify_handler` truncates the handler after `EXIT FOR`, so the
    // RECOVER/PROPAGATE behind it never reach the IR.
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "FUNC f(v AS Integer) AS Integer\n  RETURN v\nEND FUNC\nFUNC main AS Integer\n  FOR i = 1 TO 3\n    LET a = f(i) TRAP(e)\n      EXIT FOR\n      RECOVER 0\n      PROPAGATE\n    END TRAP\n  NEXT\n  FOR j = 1 TO 3\n    CONTINUE FOR\n    LET dead = 1\n  NEXT\n  RETURN 0\nEND FUNC\n",
        ),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let lines: Vec<_> = diagnostics
        .iter()
        .map(|d| (d.rule.as_str(), d.line))
        .collect();
    // The second loop's tail is outside any handler: verify's form.
    assert_eq!(
        lines,
        [("UNREACHABLE_AFTER_EXIT", 8), ("UNREACHABLE_AFTER_EXIT", 9)]
    );
}

#[test]
fn recover_outside_a_handler_and_the_count_forms() {
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "FUNC f(v AS Integer) AS Integer\n  RETURN v\nEND FUNC\nFUNC g() AS Nothing\n  RETURN\nEND FUNC\nFUNC main AS Integer\n  LET a = f(1) TRAP(e)\n    RECOVER\n  END TRAP\n  g() TRAP(e)\n    RECOVER 2\n  END TRAP\n  RECOVER 1\n  RETURN a\nEND FUNC\n",
        ),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
    assert_eq!(
        details,
        [
            "RECOVER must supply a Integer value for the trapped expression.",
            "RECOVER must not supply a value for a value-less trapped expression.",
            "RECOVER is valid only inside an inline TRAP handler.",
        ]
    );
}

#[test]
fn inline_trap_handler_must_diverge_on_every_path() {
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "IMPORT io\nFUNC f(v AS Integer) AS Integer\n  RETURN v\nEND FUNC\nFUNC main AS Integer\n  LET a = f(1) TRAP(e)\n    io::print(e.message)\n  END TRAP\n  LET b = f(2) TRAP(e)\n    IF a > 0 THEN\n      RECOVER 0\n    END IF\n  END TRAP\n  LET c = f(3) TRAP(e)\n    IF a > 0 THEN\n      RECOVER 0\n    ELSE\n      RETURN 1\n    END IF\n  END TRAP\n  RETURN a + b + c\nEND FUNC\n",
        ),
        &[],
        &[],
        &HashMap::new(),
        &[],
    );
    let lines: Vec<_> = diagnostics
        .iter()
        .map(|d| (d.rule.as_str(), d.line))
        .collect();
    assert_eq!(
        lines,
        [
            ("TYPE_INLINE_TRAP_FALLS_THROUGH", 6),
            ("TYPE_INLINE_TRAP_FALLS_THROUGH", 9),
        ]
    );
}

#[test]
fn nested_call_rule_follows_enclosing_call_rule() {
    // Outer `g(z := ...)` reports before the inner `g(y := 1)` argument.
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(z := g(y := 1))\nEND FUNC\n",
        ),
        &[], &[], &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
    assert_eq!(
        details,
        [
            "Call to `g` does not have a parameter named `z`.",
            "Call to `g` has 0 argument(s), expected 1 to 1.",
            "Call to `g` does not have a parameter named `y`.",
            "Call to `g` has 0 argument(s), expected 1 to 1.",
        ]
    );
}

#[test]
fn excess_positional_arguments_are_counted_in_the_arity_detail() {
    // bug-653: the excess argument has no parameter slot, but the call still
    // supplied it.
    let diagnostics = collect_diagnostics(
        Path::new("/proj"),
        &hir_from(
            "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(1, 2)\nEND FUNC\n",
        ),
        &[], &[], &HashMap::new(),
        &[],
    );
    let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
    assert_eq!(details, ["Call to `g` has 2 argument(s), expected 1 to 1."]);
}

// --- bug-466: field access on a record whose owning package is not imported ---
//
// `tcp::localAddress` returns a `net::Address`. Imports are not transitive
// and a builtin package cannot re-export another's types, so `Address`'s
// FIELDS are readable only where the file itself imports `net` (the rule
// `mfb man tcp localAddress` states). The checker caught the resulting
// `Unknown` at a BINDING and at an operator, but not when it was passed as a
// call argument: there it survived to native lowering, which failed with the
// bare, unlocated `native plan has no storage class for type 'Unknown'`.
//
// The gate below is a pre-lowering rule on the field access itself, so the
// verdict no longer depends on where the `Unknown` happened to land — nor,
// as `unrelated_import_does_not_make_a_foreign_record_field_readable` pins,
// on which UNRELATED package the file also imported.

/// The rule/line pairs for `src`, for the location assertions.
fn shape_reports(src: &str) -> Vec<(String, usize)> {
    collect_diagnostics(
        Path::new("/proj"),
        &hir_from(src),
        &[],
        &[],
        &HashMap::new(),
        &[],
    )
    .into_iter()
    .map(|d| (d.rule, d.line))
    .collect()
}

/// bug-466 Reproduction 1: the `Unknown` field read as an argument to an
/// OVERLOADED BUILTIN (`tcp::connect`), which unified it against a candidate
/// rather than rejecting it.
#[test]
fn foreign_record_field_as_builtin_argument_is_rejected() {
    let src = "IMPORT tcp\n\
                   FUNC main AS Integer\n\
                   \x20 RES s = tcp::listen(\"127.0.0.1\", 0)\n\
                   \x20 LET b = tcp::localAddress(s)\n\
                   \x20 RES c = tcp::connect(\"127.0.0.1\", b.port)\n\
                   \x20 RETURN 0\n\
                   END FUNC\n";
    assert_eq!(
        shape_reports(src),
        [("TYPE_UNKNOWN_VALUE".to_string(), 5)],
        "the field read must be refused AT ITS OWN LINE, not absorbed by lowering"
    );
}

/// bug-466 Reproduction 2: the same `Unknown` as an argument to a USER
/// `FUNC`, whose declared `Integer` parameter accepted it. The unlocated
/// codegen error this replaces blamed `io.print`, three calls away.
#[test]
fn foreign_record_field_as_user_func_argument_is_rejected() {
    let src = "IMPORT tcp\n\
                   IMPORT io\n\
                   FUNC take(n AS Integer) AS Integer\n\
                   \x20 RETURN n\n\
                   END FUNC\n\
                   FUNC main AS Integer\n\
                   \x20 RES s = tcp::listen(\"127.0.0.1\", 0)\n\
                   \x20 LET b = tcp::localAddress(s)\n\
                   \x20 io::print(toString(take(b.port)))\n\
                   \x20 RETURN 0\n\
                   END FUNC\n";
    assert_eq!(
        shape_reports(src),
        [("TYPE_UNKNOWN_VALUE".to_string(), 9)],
        "the field read must be refused at its own line, not at `io::print`"
    );
}

/// bug-466 Reproduction 4: `IMPORT udp` (never referenced) used to make the
/// identical `tcp` program compile, because `udp` declares a record whose
/// field is a `net::Address` and that dragged `Address`'s definition into the
/// project-wide type table. Whether a program compiled therefore depended on
/// which UNRELATED packages it imported. The gate keys on the file's own
/// imports, so the verdict is now the same either way.
#[test]
fn unrelated_import_does_not_make_a_foreign_record_field_readable() {
    let src = "IMPORT tcp\n\
                   IMPORT udp\n\
                   FUNC main AS Integer\n\
                   \x20 RES s = tcp::listen(\"127.0.0.1\", 0)\n\
                   \x20 LET b = tcp::localAddress(s)\n\
                   \x20 RES c = tcp::connect(\"127.0.0.1\", b.port)\n\
                   \x20 RETURN 0\n\
                   END FUNC\n";
    assert_eq!(
        shape_reports(src),
        [("TYPE_UNKNOWN_VALUE".to_string(), 6)],
        "an unrelated import must not change the verdict"
    );
}

/// The rule this bug is about ENFORCING, not tightening: with `IMPORT net`
/// the field read is exactly as legal as it always was.
#[test]
fn foreign_record_field_with_the_owning_import_is_accepted() {
    let src = "IMPORT tcp\n\
                   IMPORT net\n\
                   IMPORT io\n\
                   FUNC main AS Integer\n\
                   \x20 RES s = tcp::listen(\"127.0.0.1\", 0)\n\
                   \x20 LET b = tcp::localAddress(s)\n\
                   \x20 io::print(b.host & \":\" & toString(b.port))\n\
                   \x20 RES c = tcp::connect(\"127.0.0.1\", b.port)\n\
                   \x20 RETURN 0\n\
                   END FUNC\n";
    assert!(accepts(src), "got {:?}", shape_codes(src));
}

/// bug-466 Reproduction 3, characterization: the position the checker
/// ALREADY caught — the annotated binding — keeps reporting, now preceded by
/// the field access's own report.
#[test]
fn foreign_record_field_in_a_binding_still_reports() {
    let src = "IMPORT tcp\n\
                   FUNC main AS Integer\n\
                   \x20 RES s = tcp::listen(\"127.0.0.1\", 0)\n\
                   \x20 LET b = tcp::localAddress(s)\n\
                   \x20 LET p AS Integer = b.port\n\
                   \x20 RETURN 0\n\
                   END FUNC\n";
    assert_eq!(
        shape_reports(src),
        [
            ("TYPE_UNKNOWN_VALUE".to_string(), 5),
            ("TYPE_UNKNOWN_VALUE".to_string(), 5),
        ],
        "the binding cascade must survive alongside the new field-access gate"
    );
}
