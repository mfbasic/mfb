//! plan-176-H: `registry::tests`' rewrite/unify, `get_mfb` rendering, declaration
//! rendering, descriptor round-trip and type-gate tests. The shared builders live in
//! `registry/tests/mod.rs`.

use super::*;

fn enum_variant(name: &'static str) -> EnumVariant {
    EnumVariant {
        name,
        description: "variant doc",
        advisory: None,
    }
}

fn enm(name: &'static str, export: bool, variants: Vec<EnumVariant>) -> RegistryEnum {
    RegistryEnum {
        name,
        export,
        variants,
    }
}

fn intrinsic(return_type: ParameterType) -> Implementation {
    Implementation {
        params: vec![],
        return_type,
        errors: vec![],
        body: Body::Intrinsic,
    }
}

fn rewrite_impl(symbol: &'static str) -> Implementation {
    Implementation {
        params: vec![],
        return_type: ParameterType::String,
        errors: vec![],
        body: Body::Rewrite(symbol),
    }
}

#[test]
fn select_scalar_mismatch_and_wrong_arity_are_rejected() {
    let f = func(
        "f",
        vec![generic_impl(
            vec![ParameterType::String, ParameterType::Integer],
            ParameterType::Boolean,
        )],
    );
    assert!(f.dispatch(&call(&["String", "Integer"])).is_some());
    // Two different known scalars never unify.
    assert!(f.dispatch(&call(&["Boolean", "Integer"])).is_none());
    // Too few / too many arguments.
    assert!(f.dispatch(&call(&["String"])).is_none());
    assert!(f
        .dispatch(&call(&["String", "Integer", "Integer"]))
        .is_none());
}

#[test]
fn public_member_for_rewrite_names_the_member_a_user_calls() {
    // bug-642: a diagnostic about an instantiated body names the member the
    // user wrote, not the internal symbol it rewrites to.
    assert_eq!(
        public_member_for_rewrite("#collections_sort").as_deref(),
        Some("collections::sort")
    );
    assert_eq!(
        public_member_for_rewrite("#collections_sortBy").as_deref(),
        Some("collections::sortBy")
    );
    // The arity-routed second body of `findLastIndex` (bug-527) still names it.
    assert_eq!(
        public_member_for_rewrite("#collections_findLastIndexFromEnd").as_deref(),
        Some("collections::findLastIndex")
    );
    // An internal helper is not a member anyone calls.
    assert_eq!(public_member_for_rewrite("#collections_slice"), None);
    assert_eq!(public_member_for_rewrite("sort"), None);
}

#[test]
fn contains_var_detects_generic_types() {
    use ParameterType::{Integer, String};
    assert!(contains_var(&ParameterType::var("T")));
    assert!(contains_var(&list_of(ParameterType::var("T"))));
    assert!(contains_var(&map_of(String, ParameterType::var("V"))));
    assert!(contains_var(&ParameterType::map_entry_of(
        ParameterType::var("K"),
        Integer,
    )));
    assert!(contains_var(&ParameterType::result_of(ParameterType::var(
        "T"
    ))));
    assert!(!contains_var(&list_of(Integer)));
    assert!(!contains_var(&ParameterType::result_of(Integer)));
    assert!(!contains_var(&ParameterType::named("Instant")));
}

#[test]
fn unify_substitute_over_map_entry_and_result_variants() {
    use ParameterType::{Integer, String};
    // MapEntry OF K TO V: unify binds K/V from the concrete pair, then substitute
    // rebuilds the concrete pair from a fully-generic pattern.
    let pattern = ParameterType::map_entry_of(ParameterType::var("K"), ParameterType::var("V"));
    let concrete = ParameterType::map_entry_of(String, Integer);
    let mut bindings = BTreeMap::new();
    assert!(unify(&pattern, &concrete, &mut bindings, false));
    assert_eq!(substitute(&pattern, &bindings), Some(concrete.clone()));
    // A mismatched shape (Map OF vs MapEntry OF) does not unify.
    assert!(!unify(
        &pattern,
        &map_of(String, Integer),
        &mut BTreeMap::new(),
        false
    ));

    // Result OF T: bind T, substitute back.
    let rpattern = ParameterType::result_of(ParameterType::var("T"));
    let rconcrete = ParameterType::result_of(Integer);
    let mut rbindings = BTreeMap::new();
    assert!(unify(&rpattern, &rconcrete, &mut rbindings, false));
    assert_eq!(substitute(&rpattern, &rbindings), Some(rconcrete));
    assert!(!unify(
        &rpattern,
        &list_of(Integer),
        &mut BTreeMap::new(),
        false
    ));
}

#[test]
fn add_function_takes_a_function_value() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("t", "intro", "desc");
    pkg.add_function(func("f", vec![intrinsic(ParameterType::Nothing)]));
    r.add_package(pkg);
    assert_eq!(r.packages().len(), 1);
    assert_eq!(r.resolve_package("t").unwrap().functions().len(), 1);
}

#[test]
fn resolve_func_finds_the_owning_package() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("csv", "i", "d");
    pkg.add_function(func("parse", vec![rewrite_impl("__csv_parse")]));
    r.add_package(pkg);

    assert_eq!(
        r.resolve_func("csv.parse")
            .map(|resolved| resolved.package.import_name()),
        Some("csv"),
    );
    assert!(r.resolve_func("csv.nope").is_none());
    assert!(r.resolve_func("nope.parse").is_none());
    assert!(r.resolve_func("toString").is_none());
    // Works against the frozen registry too (a real migrated member).
    assert_eq!(
        registry()
            .resolve_func("csv.parse")
            .map(|resolved| resolved.package.import_name()),
        Some("csv"),
    );
}

fn sample_fast_path<'a>(
    _b: &mut crate::codegen::engine::builder::CodeBuilder<'a>,
    _target: &str,
    _args: &[crate::target::shared::nir::NirValue],
) -> Result<Option<crate::codegen::engine::builder::ValueResult>, String> {
    Ok(None)
}

#[test]
fn mfb_body_carries_a_rewrite_target_and_optional_fast_path() {
    let plain = Body::mfb("FUNC __x() ... END FUNC", "__x");
    assert!(matches!(
        plain,
        Body::Mfb {
            fast_path: None,
            ..
        }
    ));
    assert_eq!(plain.rewrite_target(), Some("__x"));

    let accelerated = Body::mfb_with_fast_path("FUNC __x() ... END FUNC", "__x", sample_fast_path);
    assert!(matches!(
        accelerated,
        Body::Mfb {
            fast_path: Some(_),
            ..
        }
    ));

    assert_eq!(Body::Rewrite("__y").rewrite_target(), Some("__y"));
    assert_eq!(Body::Intrinsic.rewrite_target(), None);
}

#[test]
fn get_mfb_renders_helper_functions_before_member_bodies() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("demo", "intro", "desc");
    pkg.add_helper(RegistryHelper::always(
        "demo",
        "FUNC __demo_helper() AS Nothing\nEND FUNC",
    ));
    pkg.add_function(func(
        "a",
        vec![mfb_impl(
            "FUNC __demo_a() AS String\n  RETURN \"a\"\nEND FUNC",
        )],
    ));
    pkg.add_function(func("b", vec![rewrite_impl("__demo_b")]));
    pkg.add_function(func(
        "c",
        vec![mfb_impl(
            "FUNC __demo_c() AS String\n  RETURN \"c\"\nEND FUNC",
        )],
    ));
    r.add_package(pkg);

    let src = r.resolve_package("demo").unwrap().get_mfb();
    assert_eq!(
        src,
        "FUNC __demo_helper() AS Nothing\nEND FUNC\n\n\
             FUNC __demo_a() AS String\n  RETURN \"a\"\nEND FUNC\n\n\
             FUNC __demo_c() AS String\n  RETURN \"c\"\nEND FUNC\n",
    );
    assert!(!src.contains("__demo_b"));
}

/// plan-130-E: a `debug_split` helper contributes its normal body to a normal
/// build's source and its debug body to a `--debug` build's, in the same position,
/// and the normal source carries nothing from the debug body.
#[test]
fn a_debug_split_helper_renders_the_half_matching_the_build() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("demo", "intro", "desc");
    pkg.add_helper(RegistryHelper::always(
        "demo_before",
        "FUNC __demo_before() AS Nothing\nEND FUNC",
    ));
    for helper in RegistryHelper::debug_split(
        "demo_hook",
        "FUNC __demo_hook() AS Nothing\nEND FUNC",
        "FUNC __demo_hook() AS Nothing\n  __demo_report()\nEND FUNC\n\nFUNC __demo_report() AS Nothing\nEND FUNC",
    ) {
        pkg.add_helper(helper);
    }
    pkg.add_helper(RegistryHelper::always(
        "demo_after",
        "FUNC __demo_after() AS Nothing\nEND FUNC",
    ));
    r.add_package(pkg);
    let package = r.resolve_package("demo").unwrap();

    let normal = package.get_mfb_for(false);
    assert_eq!(
        normal,
        "FUNC __demo_before() AS Nothing\nEND FUNC\n\n\
             FUNC __demo_hook() AS Nothing\nEND FUNC\n\n\
             FUNC __demo_after() AS Nothing\nEND FUNC\n",
    );
    assert_eq!(package.get_mfb(), normal);
    assert!(!normal.contains("__demo_report"));

    assert_eq!(
        package.get_mfb_for(true),
        "FUNC __demo_before() AS Nothing\nEND FUNC\n\n\
             FUNC __demo_hook() AS Nothing\n  __demo_report()\nEND FUNC\n\n\
             FUNC __demo_report() AS Nothing\nEND FUNC\n\n\
             FUNC __demo_after() AS Nothing\nEND FUNC\n",
    );
}

#[test]
fn get_mfb_is_empty_when_the_package_has_no_mfb_member() {
    // A package with only Intrinsic/Rewrite members and no records/unions/helpers
    // injects nothing.
    let mut empty = Registry::new();
    let mut pkg = RegistryPackage::new("nomfb", "i", "d");
    pkg.add_function(func("a", vec![rewrite_impl("__a")]));
    empty.add_package(pkg);
    assert_eq!(empty.resolve_package("nomfb").unwrap().get_mfb(), "");

    // A package whose only injectable content is a helper-function chunk (the
    // native `process` shape: an `EXPORT ENUM` companion, no records/unions/bodies)
    // still emits that chunk — helper chunks are standalone injectable source.
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("bare", "i", "d");
    pkg.add_imports(vec!["strings"]);
    pkg.add_helper(RegistryHelper::always(
        "demo",
        "FUNC __helper() AS Nothing\nEND FUNC",
    ));
    r.add_package(pkg);
    assert_eq!(
        r.resolve_package("bare").unwrap().get_mfb(),
        "IMPORT strings\n\nFUNC __helper() AS Nothing\nEND FUNC\n",
    );
}

#[test]
fn add_record_renders_the_type_declaration() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("json", "intro", "desc");
    pkg.add_record(rec(
        "JsonNum",
        true,
        vec![prop("value", ParameterType::Float)],
    ));
    pkg.add_record(rec(
        "Pair",
        false,
        vec![
            prop("key", ParameterType::String),
            prop("val", ParameterType::Integer),
        ],
    ));
    r.add_package(pkg);

    let pkg = r.resolve_package("json").unwrap();
    assert_eq!(pkg.records().len(), 2);
    assert!(pkg.records()[0].export);
    assert!(!pkg.records()[1].export);
    assert_eq!(
        pkg.records()[0].render(),
        "EXPORT TYPE JsonNum\n  value AS Float\nEND TYPE"
    );
    assert_eq!(
        pkg.records()[1].render(),
        "TYPE Pair\n  key AS String\n  val AS Integer\nEND TYPE"
    );
}

#[test]
fn add_union_renders_the_union_declaration() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("json", "intro", "desc");
    pkg.add_union(uni(
        "Json",
        true,
        vec![variant("JsonNull"), variant("JsonNum"), variant("JsonStr")],
    ));
    pkg.add_union(uni("Internal", false, vec![variant("A")]));
    r.add_package(pkg);

    let pkg = r.resolve_package("json").unwrap();
    assert_eq!(pkg.unions().len(), 2);
    assert!(pkg.unions()[0].export);
    assert!(!pkg.unions()[1].export);
    assert_eq!(
        pkg.unions()[0].render(),
        "EXPORT UNION Json\n  JsonNull\n  JsonNum\n  JsonStr\nEND UNION"
    );
    assert_eq!(pkg.unions()[1].render(), "UNION Internal\n  A\nEND UNION");
}

#[test]
fn add_enum_renders_the_enum_declaration() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("process", "intro", "desc");
    pkg.add_enum(enm(
        "Stream",
        true,
        vec![enum_variant("StdOut"), enum_variant("StdErr")],
    ));
    pkg.add_enum(enm("Internal", false, vec![enum_variant("A")]));
    r.add_package(pkg);

    let pkg = r.resolve_package("process").unwrap();
    assert_eq!(pkg.enums().len(), 2);
    assert!(pkg.enums()[0].export);
    assert!(!pkg.enums()[1].export);
    assert_eq!(
        pkg.enums()[0].render(),
        "EXPORT ENUM Stream\n  StdOut\n  StdErr\nEND ENUM"
    );
    assert_eq!(pkg.enums()[1].render(), "ENUM Internal\n  A\nEND ENUM");
}

#[test]
fn enum_variant_advisory_is_keyed_by_package_enum_and_member() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("demo", "i", "d");
    let advisory = EnumAdvisory {
        rule: "CRYPTO_SHA1_INSECURE",
        detail: "legacy only",
    };
    pkg.add_enum(enm(
        "Algo",
        true,
        vec![
            EnumVariant {
                name: "Weak",
                description: "weak",
                advisory: Some(advisory),
            },
            enum_variant("Strong"),
        ],
    ));
    r.add_package(pkg);
    // A same-named enum in ANOTHER package carries no advisory: the lookup is
    // keyed by the owning package, never a bare enum-name scan.
    let mut other = RegistryPackage::new("other", "i", "d");
    other.add_enum(enm("Algo", true, vec![enum_variant("Weak")]));
    r.add_package(other);

    assert_eq!(
        r.enum_variant_advisory("demo", "Algo", "Weak"),
        Some(advisory)
    );
    assert_eq!(r.enum_variant_advisory("demo", "Algo", "Strong"), None);
    assert_eq!(r.enum_variant_advisory("demo", "Algo", "Missing"), None);
    assert_eq!(r.enum_variant_advisory("demo", "Nope", "Weak"), None);
    assert_eq!(r.enum_variant_advisory("other", "Algo", "Weak"), None);
    assert_eq!(r.enum_variant_advisory("absent", "Algo", "Weak"), None);
    // The advisory's rule must be a real `warn` row of the rule table, or the
    // `ir::verify` emit site would trip the unknown-rule guard.
    let production = registry().enum_variant_advisory("crypto", "Hash", "SHA1");
    let rule = production
        .expect("crypto Hash.SHA1 carries an advisory")
        .rule;
    assert!(
        !crate::rules::is_error(rule),
        "{rule} must be warn-severity"
    );
    assert_eq!(
        registry().enum_variant_advisory("crypto", "Hash", "SHA256"),
        None
    );
}

#[test]
fn enums_resolve_as_builtin_types() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("process", "i", "d");
    pkg.add_enum(enm("Stream", true, vec![enum_variant("StdOut")]));
    r.add_package(pkg);

    assert!(matches!(
        r.resolve_type("process.Stream"),
        Some(ResolvedType::Enum(e)) if e.name == "Stream"
    ));
    assert!(r.resolve_type("process.Nope").is_none());
}

#[test]
fn add_resource_records_name_export_and_close_function() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("fs", "i", "d");
    pkg.add_resource(res("File", true, "fs.close"));
    pkg.add_resource(res("Internal", false, "fs.__drop"));
    r.add_package(pkg);

    let pkg = r.resolve_package("fs").unwrap();
    assert_eq!(pkg.resources().len(), 2);
    assert_eq!(pkg.resources()[0].name, "File");
    assert!(pkg.resources()[0].export);
    assert_eq!(pkg.resources()[0].close_function, "fs.close");
    assert!(!pkg.resources()[1].export);
    // Resources carry no injectable source, so they never render into get_mfb.
    assert!(!pkg.get_mfb().contains("File"));
    // resolve_type resolves a resource type by name.
    assert!(matches!(
        r.resolve_type("fs.File"),
        Some(ResolvedType::Resource(resource)) if resource.close_function == "fs.close"
    ));
}

#[test]
fn get_mfb_places_enums_between_unions_and_helpers() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("process", "intro", "desc");
    pkg.add_union(uni("U", true, vec![variant("A")]));
    pkg.add_enum(enm(
        "Stream",
        true,
        vec![enum_variant("StdOut"), enum_variant("StdErr")],
    ));
    pkg.add_helper(RegistryHelper::always(
        "process",
        "FUNC __process_helper() AS Nothing\nEND FUNC",
    ));
    r.add_package(pkg);

    assert_eq!(
        r.resolve_package("process").unwrap().get_mfb(),
        "EXPORT UNION U\n  A\nEND UNION\n\n\
             EXPORT ENUM Stream\n  StdOut\n  StdErr\nEND ENUM\n\n\
             FUNC __process_helper() AS Nothing\nEND FUNC\n",
    );
}

#[test]
fn get_mfb_orders_imports_records_unions_helpers_functions() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("json", "intro", "desc");
    pkg.add_imports(vec!["collections"]);
    pkg.add_imports(vec!["strings"]);
    pkg.add_record(rec(
        "JsonNum",
        true,
        vec![prop("value", ParameterType::Float)],
    ));
    pkg.add_record(rec(
        "JsonBool",
        true,
        vec![prop("flag", ParameterType::Boolean)],
    ));
    pkg.add_union(uni(
        "Json",
        true,
        vec![variant("JsonNum"), variant("JsonBool")],
    ));
    pkg.add_helper(RegistryHelper::always(
        "json",
        "FUNC __json_helper() AS Nothing\nEND FUNC",
    ));
    pkg.add_function(func(
        "render",
        vec![mfb_impl(
            "FUNC __json_render() AS String\n  RETURN \"\"\nEND FUNC",
        )],
    ));
    r.add_package(pkg);

    let pkg = r.resolve_package("json").unwrap();
    assert_eq!(pkg.imports(), &["collections", "strings"]);
    assert_eq!(
        pkg.get_mfb(),
        "IMPORT collections\nIMPORT strings\n\n\
             EXPORT TYPE JsonNum\n  value AS Float\nEND TYPE\n\n\
             EXPORT TYPE JsonBool\n  flag AS Boolean\nEND TYPE\n\n\
             EXPORT UNION Json\n  JsonNum\n  JsonBool\nEND UNION\n\n\
             FUNC __json_helper() AS Nothing\nEND FUNC\n\n\
             FUNC __json_render() AS String\n  RETURN \"\"\nEND FUNC\n",
    );
}

#[test]
fn descriptor_round_trips_the_full_surface() {
    let mut r = Registry::new();
    let mut pkg = RegistryPackage::new("demo", "pkg intro", "pkg desc");
    pkg.add_imports(vec!["strings"]);
    pkg.add_helper(RegistryHelper::always(
        "demo",
        "FUNC __demo_helper()\nEND FUNC",
    ));
    pkg.add_record(rec("Rec", true, vec![prop("f", ParameterType::Integer)]));
    pkg.add_union(uni("Uni", false, vec![variant("V")]));
    pkg.add_function(RegistryFunction {
        name: "fn1",
        intro: "fn intro",
        desc: "fn desc",
        example: "fn example",
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "req",
                    desc: "",
                    aliases: &["r"],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "opt",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Fill {
                        type_name: ParameterType::String,
                        expr: ",",
                    },
                },
                Parameter {
                    name: "zone",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Optional,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec!["SOME_ERROR"],
            body: Body::Rewrite("__demo_fn1"),
        }],
    });
    r.add_package(pkg);

    let pkg = r.resolve_package("demo").unwrap();
    assert_eq!(pkg.intro(), "pkg intro");
    assert_eq!(pkg.desc(), "pkg desc");
    assert_eq!(
        pkg.always_helper_bodies(false),
        vec!["FUNC __demo_helper()\nEND FUNC"]
    );

    let rec = &pkg.records()[0];
    assert_eq!(rec.name, "Rec");
    assert_eq!(rec.props[0].name, "f");
    assert_eq!(rec.props[0].description, "field doc");

    let uni = &pkg.unions()[0];
    assert_eq!(uni.name, "Uni");
    assert_eq!(uni.variants[0].name, "V");

    let f = pkg.function("fn1").expect("fn1");
    assert_eq!(f.name, "fn1");
    assert_eq!(f.intro, "fn intro");
    assert_eq!(f.example, "fn example");

    let imp = &f.implementations[0];
    assert_eq!(imp.errors, vec!["SOME_ERROR"]);
    assert_eq!(imp.params[0].aliases, &["r"]);
    assert!(matches!(imp.params[0].default, DefaultValue::None));
    assert!(matches!(
        imp.params[1].default,
        DefaultValue::Fill {
            type_name: ParameterType::String,
            expr: ","
        }
    ));
    assert!(matches!(imp.params[2].default, DefaultValue::Optional));
}

#[test]
fn is_imported_by_checks_the_program_imports() {
    let mut r = Registry::new();
    r.add_package(RegistryPackage::new("csv", "i", "d"));
    let csv = r.resolve_package("csv").expect("csv package");

    let parse = |src: &str| {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    };
    assert!(csv.is_imported_by(&ProjectView::of_ast(&parse(
        "IMPORT csv\nFUNC main AS Nothing\nEND FUNC\n"
    ))));
    assert!(!csv.is_imported_by(&ProjectView::of_ast(&parse(
        "FUNC main AS Nothing\nEND FUNC\n"
    ))));
}

/// plan-164-B: a `WhenCalledOrNamed` chunk and its `WhenNeitherCalledNorNamed`
/// complement. Exactly one of the pair is injected into a program that imports the
/// package — which is what lets the two define the same functions — and the real
/// one opens on the type being NAMED anywhere: constructed, or spelled in a
/// binding's, a parameter's, a return type's or a field's annotation, however
/// deep in the spelling. That is the case a constructor gate misses: a record
/// declared for its default and filled in with `WITH`.
#[test]
fn a_named_type_opens_its_gate_and_closes_the_exact_complement() {
    let mut pkg = RegistryPackage::new("demo", "i", "d");
    pkg.add_helper(RegistryHelper {
        name: "demo_real",
        gate: HelperGate::WhenCalledOrNamed(&["poke"], &["Widget"]),
        body: Some("FUNC __demo_step() AS Integer\n  RETURN 1\nEND FUNC"),
        import_name: None,
        natively_called: false,
    });
    pkg.add_helper(RegistryHelper {
        name: "demo_absent",
        gate: HelperGate::WhenNeitherCalledNorNamed(&["poke"], &["Widget"]),
        body: Some("FUNC __demo_step() AS Integer\n  RETURN 0\nEND FUNC"),
        import_name: None,
        natively_called: false,
    });
    let mut r = Registry::new();
    r.add_package(pkg);

    let injected = |src: &str| -> Vec<String> {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse");
        let ast = crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        };
        let from_ast: Vec<String> = r
            .synthetic_files(&ProjectView::of_ast(&ast), false)
            .expect("synthetic files")
            .into_iter()
            .map(|f| f.path)
            .filter(|path| path.contains("demo_"))
            .collect();
        // The HIR view must decide the same way.
        let hir = crate::hir::elaborate(&ast);
        let from_hir: Vec<String> = r
            .synthetic_files(&ProjectView::of_hir(&hir), false)
            .expect("synthetic files")
            .into_iter()
            .map(|f| f.path)
            .filter(|path| path.contains("demo_"))
            .collect();
        assert_eq!(
            from_ast, from_hir,
            "the AST and HIR views disagree for:\n{src}"
        );
        from_ast
    };
    let real = vec!["builtins/demo_real.mfb".to_string()];
    let absent = vec!["builtins/demo_absent.mfb".to_string()];

    // Nothing names a Widget and nothing calls `poke`: the complement, alone.
    assert_eq!(
        injected("IMPORT demo\nFUNC main AS Nothing\nEND FUNC\n"),
        absent
    );
    // A default-declared binding, never constructed -- the case this gate is for.
    assert_eq!(
        injected("IMPORT demo\nFUNC main AS Nothing\n  MUT w AS demo::Widget\nEND FUNC\n"),
        real
    );
    // Inside a longer spelling, in a nested block.
    assert_eq!(
        injected(
            "IMPORT demo\nFUNC main AS Nothing\n  IF TRUE THEN\n    LET ws AS List OF Widget = []\n  END IF\nEND FUNC\n"
        ),
        real
    );
    // A parameter, a return type, a record field, a module-level binding.
    assert_eq!(
        injected("IMPORT demo\nFUNC show(w AS demo::Widget) AS Nothing\nEND FUNC\nFUNC main AS Nothing\nEND FUNC\n"),
        real
    );
    assert_eq!(
        injected("IMPORT demo\nFUNC make() AS List OF demo::Widget\n  RETURN []\nEND FUNC\nFUNC main AS Nothing\nEND FUNC\n"),
        real
    );
    assert_eq!(
        injected("IMPORT demo\nTYPE Box\n  held AS demo::Widget\nEND TYPE\nFUNC main AS Nothing\nEND FUNC\n"),
        real
    );
    assert_eq!(
        injected("IMPORT demo\nMUT spare AS demo::Widget\nFUNC main AS Nothing\nEND FUNC\n"),
        real
    );
    // A constructor names it too.
    assert_eq!(
        injected(
            "IMPORT demo\nFUNC main AS Nothing\n  show([demo::Widget[size := 1]])\nEND FUNC\n"
        ),
        real
    );
    // The gated CALL opens the real chunk AND closes the complement.
    assert_eq!(
        injected("IMPORT demo\nFUNC main AS Nothing\n  demo::poke()\nEND FUNC\n"),
        real
    );
    // A type that merely contains the name as part of a longer word opens nothing.
    assert_eq!(
        injected("IMPORT demo\nFUNC main AS Nothing\n  MUT w AS WidgetBox\nEND FUNC\n"),
        absent
    );
    // A program that does not import the package gets neither.
    assert!(injected("FUNC main AS Nothing\n  MUT w AS Widget\nEND FUNC\n").is_empty());
}

/// plan-172-K: a `WhenUsedOrConstructed` chunk and its `WhenNotConstructed`
/// complement. Exactly one of the pair is injected into a program that imports the
/// package, and which one turns on whether the program constructs the record —
/// qualified or bare, at any depth of expression — or calls a gated name. A program
/// that does not import the package gets neither.
#[test]
fn a_constructed_record_opens_its_gate_and_closes_the_complement() {
    let mut pkg = RegistryPackage::new("demo", "i", "d");
    pkg.add_helper(RegistryHelper {
        name: "demo_real",
        gate: HelperGate::WhenUsedOrConstructed(&["poke"], &["Widget"]),
        body: Some("FUNC __demo_step() AS Integer\n  RETURN 1\nEND FUNC"),
        import_name: None,
        natively_called: false,
    });
    pkg.add_helper(RegistryHelper {
        name: "demo_absent",
        gate: HelperGate::WhenNotConstructed(&["Widget"]),
        body: Some("FUNC __demo_step() AS Integer\n  RETURN 0\nEND FUNC"),
        import_name: None,
        natively_called: false,
    });
    let mut r = Registry::new();
    r.add_package(pkg);

    let injected = |src: &str| -> Vec<String> {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse");
        let ast = crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        };
        r.synthetic_files(&ProjectView::of_ast(&ast), false)
            .expect("synthetic files")
            .into_iter()
            .map(|f| f.path)
            .filter(|path| path.contains("demo_"))
            .collect()
    };

    // Nothing constructs a Widget: the complement, alone.
    assert_eq!(
        injected("IMPORT demo\nFUNC main AS Nothing\nEND FUNC\n"),
        vec!["builtins/demo_absent.mfb"]
    );
    // A qualified constructor, nested in a list in a call argument.
    assert_eq!(
        injected(
            "IMPORT demo\nFUNC main AS Nothing\n  show([demo::Widget[size := 1]])\nEND FUNC\n"
        ),
        vec!["builtins/demo_real.mfb"]
    );
    // A bare one, in a module-level binding.
    assert_eq!(
        injected(
            "IMPORT demo\nLET w AS Widget = Widget[size := 1]\nFUNC main AS Nothing\nEND FUNC\n"
        ),
        vec!["builtins/demo_real.mfb"]
    );
    // The gated CALL opens the chunk without closing the complement: the complement
    // is keyed on the record alone, so a chunk gated on both names must not share a
    // function with it. Canvas's does not (`helper_hit.rs`, `helper_autopaint.rs`).
    assert_eq!(
        injected("IMPORT demo\nFUNC main AS Nothing\n  demo::poke()\nEND FUNC\n"),
        vec!["builtins/demo_real.mfb", "builtins/demo_absent.mfb"]
    );
    // A record that merely shares a gated CALL's name opens nothing.
    assert_eq!(
        injected("IMPORT demo\nFUNC main AS Nothing\n  LET p AS poke = poke[n := 1]\nEND FUNC\n"),
        vec!["builtins/demo_absent.mfb"]
    );
    // Not imported: neither.
    assert!(
        injected("FUNC main AS Nothing\n  LET w AS Widget = Widget[size := 1]\nEND FUNC\n")
            .is_empty()
    );
}

/// plan-104-C: the typed entry and the string wrapper must agree on every
/// call shape — same overload selection, same return (typed's `name()` ==
/// string's answer) — across containers, `RES` markers, unions, `Unknown`,
/// and a strict-`Nothing` rejection. The corpus resolves against the real
/// frozen registry so it exercises the production descriptors.
#[test]
fn typed_and_string_resolution_agree() {
    let corpus: &[(&str, &[&str], bool)] = &[
        // scalar + container generics
        ("collections.get", &["List OF Integer", "Integer"], false),
        ("collections.get", &["List OF Integer", "Integer"], true),
        ("collections.append", &["List OF String", "String"], false),
        ("collections.keys", &["Map OF String TO Integer"], false),
        // RES-marked collection element: Arg-echo preserves the marker
        (
            "collections.append",
            &["List OF RES fs.File", "fs.File"],
            false,
        ),
        // set + map-entry shapes
        ("collections.toList", &["Set OF Integer"], false),
        // Unknown wildcard argument (lenient dispatch accepts)
        ("collections.get", &["List OF Integer", "Unknown"], false),
        // higher-order FUNC parameter
        (
            "collections.transform",
            &["List OF Integer", "FUNC(Integer) AS String"],
            false,
        ),
        // strict-Nothing rejection: both must reject identically
        ("collections.get", &["Nothing", "Integer"], true),
        // union-typed argument through a value-union parameter (json)
        ("json.encode", &["Integer"], false),
        // a call no package owns
        ("nope.missing", &["Integer"], false),
    ];
    for (callee, args, strict) in corpus {
        let arg_strings: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let arg_typed: Vec<ParameterType> = args.iter().map(|a| ParameterType::parse(a)).collect();
        let via_string = resolve_call(callee, &arg_strings, *strict);
        let via_typed =
            resolve_call_typed(callee, &arg_typed, *strict).map(|type_| type_.name().into_owned());
        assert_eq!(
            via_string, via_typed,
            "typed/string disagree for {callee}({args:?}, strict={strict})"
        );
    }
    // The strict-Nothing case really is a rejection, not a vacuous agreement.
    assert_eq!(
        resolve_call_typed(
            "collections.get",
            &[ParameterType::Nothing, ParameterType::Integer],
            true
        ),
        None
    );
    // And the RES echo really preserves the marker through the typed path.
    let echoed = resolve_call_typed(
        "collections.append",
        &[
            ParameterType::parse("List OF RES fs.File"),
            ParameterType::parse("fs.File"),
        ],
        false,
    );
    assert_eq!(
        echoed.map(|type_| type_.name().into_owned()),
        Some("List OF RES fs.File".to_string())
    );
}

/// plan-120-D. `argument_types_typed` declines for an overload SET, and IR
/// lowering uses it to decide union wrapping — so without an answer for the
/// positions the overloads AGREE on, a member that gains a second overload
/// silently stops wrapping a union-typed argument. That failure has no
/// diagnostic: `json::stringify(json::JsonNull[NOTHING])` returned `""` and
/// `json::stringify(json::JsonStr["Ada"])` returned `"null"`, the tag read
/// from the wrong place.
///
/// This pins both halves of the rule, so a future overload cannot quietly
/// reintroduce it: agreement answers, disagreement stays `None`.
#[test]
fn agreed_argument_type_answers_where_overloads_agree() {
    // `json::stringify` is overloaded three ways and every one takes `Json`
    // first — the position that must stay wrapped.
    assert_eq!(
        agreed_argument_type("json.stringify", 0).map(|t| t.name().into_owned()),
        Some("json.Json".to_string()),
        "position 0 is `Json` in all three overloads and must be answered"
    );
    // Position 1 is `Integer` in one overload and `String` in another, so
    // there is no single expected type and the honest answer is `None`.
    assert_eq!(
        agreed_argument_type("json.stringify", 1),
        None,
        "position 1 disagrees (Integer vs String) and must NOT be guessed"
    );
    // Past every overload's parameter list.
    assert_eq!(agreed_argument_type("json.stringify", 2), None);
    // A single-implementation member is `argument_types_typed`'s job; this
    // function deliberately declines so the two cannot both answer.
    // `json.get` stands in for `json.parse` here, which plan-120-E gave a
    // second overload — the parse cases moved below.
    assert_eq!(agreed_argument_type("json.get", 0), None);
    // And it agrees with `argument_types_typed` about what position 0 IS,
    // for a member where that function does answer.
    assert_eq!(
        argument_types_typed("json.get").and_then(|p| p.first().map(|t| t.name().into_owned())),
        Some("json.Json".to_string())
    );
    // plan-120-E: now that `json.parse` is overloaded the two roles swap —
    // this function answers and `argument_types_typed` declines. They are
    // exact complements (exactly 1 implementation vs 2 or more), which is
    // what stops both from answering for the same member; asserting BOTH
    // halves is what makes that a property rather than a coincidence.
    // bug-815 added the two `fs::File` overloads, so position 0 is `String` in
    // two overloads and `File` in two: disagreement, which must decline.
    assert_eq!(
        agreed_argument_type("json.parse", 0),
        None,
        "parse takes String or File first since bug-815 and must NOT be guessed"
    );
    assert_eq!(argument_types_typed("json.parse"), None);
    // Position 1 exists in only ONE of the two overloads. A position past
    // some overload's parameter list is not agreement, so it must decline —
    // otherwise a 1-arg call site would be handed the reviver's type.
    assert_eq!(agreed_argument_type("json.parse", 1), None);
    // An unknown member is not an answer.
    assert_eq!(agreed_argument_type("nope.missing", 0), None);
}
