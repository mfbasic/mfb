use super::*;
use crate::ast::{Param, Visibility};
use crate::hir::{HirCallArg, HirParam};
use crate::manifest::validate_project_manifest;

fn quiet<T>(f: impl FnOnce() -> T) -> T {
    crate::testutil::silence_panics(f)
}

fn resolve_fixture(name: &str) -> Result<(), ()> {
    let dir = crate::testutil::fixture_dir(name);
    let manifest =
        validate_project_manifest(&dir.join("project.json")).expect("fixture manifest is valid");
    let pname = manifest
        .get("name")
        .and_then(|v| v.get::<String>())
        .cloned()
        .expect("fixture manifest has a name");
    let ast = crate::ast::parse_project(&pname, &dir, &manifest).expect("fixture parses");
    quiet(|| resolve_project(&dir, &manifest, &ast))
}

fn empty_expr() -> HirExpression {
    HirExpression::Number("1".into())
}

/// A `LINK` block's parameter — the one node HIR keeps as the verbatim AST
/// struct, so this stays in the AST domain.
fn param(name: &str, type_name: Option<&str>) -> Param {
    Param {
        name: name.into(),
        type_name: type_name.map(str::to_string),
        state_type: None,
        default: None,
        line: 1,
    }
}

fn hir_param(name: &str, type_name: Option<&str>) -> HirParam {
    HirParam {
        name: name.into(),
        type_: type_name.map_or(ParameterType::Unknown, ParameterType::parse),
        resource: false,
        state_type: None,
        stray_state: None,
        default: None,
        line: 1,
    }
}

fn func(name: &str, params: Vec<HirParam>) -> HirFunction {
    HirFunction {
        visibility: Visibility::Export,
        isolated: false,
        name: name.into(),
        template_params: Vec::new(),
        params,
        returns: ParameterType::Unknown,
        return_resource: false,
        return_state_type: None,
        return_stray_state: None,
        body: Vec::new(),
        trap: None,
        line: 1,
        internal_origin: false,
        instantiated_at: Vec::new(),
    }
}

fn hir_file(path: &str) -> HirFile {
    HirFile {
        path: path.into(),
        imports: Vec::new(),
        own_imports: Vec::new(),
        items: Vec::new(),
        internal: false,
    }
}

/// A one-file `HirProject` carrying `items`.
fn project_of(items: Vec<HirItem>) -> HirProject {
    HirProject {
        name: "p".into(),
        files: vec![HirFile {
            path: "a.mfb".into(),
            imports: Vec::new(),
            own_imports: Vec::new(),
            items,
            internal: false,
        }],
    }
}

#[test]
fn constructor_arg_value_positional_and_named() {
    let pos = HirConstructorArg::Positional(empty_expr());
    let named = HirConstructorArg::Named {
        name: "x".into(),
        value: empty_expr(),
        line: 1,
    };
    assert!(matches!(
        constructor_arg_value(&pos),
        HirExpression::Number(_)
    ));
    assert!(matches!(
        constructor_arg_value(&named),
        HirExpression::Number(_)
    ));
}

#[test]
fn call_arg_value_positional_and_named() {
    let pos = HirCallArg::Positional(empty_expr());
    let named = HirCallArg::Named {
        name: "x".into(),
        value: empty_expr(),
        line: 1,
    };
    assert!(matches!(call_arg_value(&pos), HirExpression::Number(_)));
    assert!(matches!(call_arg_value(&named), HirExpression::Number(_)));
}

#[test]
fn overload_types_match_variants() {
    let f = func(
        "g",
        vec![
            hir_param("a", Some("Integer")),
            hir_param("b", Some("String")),
        ],
    );
    assert!(overload_types_match(
        &f,
        &["Integer".to_string(), "String".to_string()]
    ));
    assert!(!overload_types_match(&f, &["Integer".to_string()]));
    assert!(!overload_types_match(
        &f,
        &["Integer".to_string(), "Float".to_string()]
    ));
}

#[test]
fn overload_types_match_none_type_name() {
    let f = func("g", vec![hir_param("a", None)]);
    assert!(overload_types_match(&f, &[String::new()]));
    assert!(!overload_types_match(&f, &["Integer".to_string()]));
}

#[test]
fn is_c_abi_type_recognizes_and_rejects() {
    // plan-113: these go through `parse`, not `named`. A source-written
    // `CPtr` is a `ParameterType::C` now, and `named("CPtr")` is a nominal
    // that merely *spells* one — a value the compiler no longer mints for a
    // C ABI type, so asserting over it would test an unreachable shape.
    for t in [
        "CPtr", "CString", "CInt8", "CInt16", "CInt32", "CInt64", "CUInt8", "CUInt16", "CUInt32",
        "CUInt64", "CFloat", "CDouble",
    ] {
        assert!(
            is_c_abi_type(&crate::types::ParameterType::parse(t)),
            "{t} should be a C ABI type"
        );
    }
    // The 12-of-16 list is DELIBERATE (§3 Risk 2 of plan-113): `CBool`,
    // `CByte` and `CVoid` are excluded per `17_native-libraries.md:94`, and
    // `CBuffer` with them. Rewriting the predicate as
    // `type_.c_abi().is_some()` compiles and silently adds these four to
    // what `NATIVE_CPTR_ESCAPE` rejects; these four assertions are the only
    // thing that catches it.
    for t in ["CBool", "CByte", "CVoid", "CBuffer"] {
        assert!(
            !is_c_abi_type(&crate::types::ParameterType::parse(t)),
            "{t} must NOT be a C ABI type for NATIVE_CPTR_ESCAPE"
        );
    }
    assert!(!is_c_abi_type(&crate::types::ParameterType::parse(
        "Integer"
    )));
    assert!(!is_c_abi_type(&crate::types::ParameterType::parse("CPtrX")));
    assert!(!is_c_abi_type(&crate::types::ParameterType::parse("")));
    // A nominal that merely spells a C type is not one.
    assert!(!is_c_abi_type(&crate::types::ParameterType::named("CPtr")));
}

#[test]
fn resource_base_type_strips_state_suffix() {
    assert_eq!(
        resource_base_type(&crate::types::ParameterType::parse("Handle STATE Open")).name(),
        "Handle"
    );
    assert_eq!(
        resource_base_type(&crate::types::ParameterType::parse("Handle")).name(),
        "Handle"
    );
    assert_eq!(
        resource_base_type(&crate::types::ParameterType::parse("")).name(),
        ""
    );
}

#[test]
fn visible_from_rules() {
    let empty = HirProject {
        name: "p".into(),
        files: Vec::new(),
    };
    let dir = std::path::Path::new(".");
    let resolver = Resolver::new(dir, &HashMap::new(), &empty);
    let here = hir_file("a.mfb");
    assert!(resolver.visible_from(&here, Visibility::Export, "other.mfb"));
    assert!(resolver.visible_from(&here, Visibility::Public, "other.mfb"));
    assert!(resolver.visible_from(&here, Visibility::Private, "a.mfb"));
    assert!(!resolver.visible_from(&here, Visibility::Private, "other.mfb"));
}

#[test]
fn top_level_and_function_visibility_lookups() {
    let file = HirFile {
        path: "a.mfb".into(),
        imports: Vec::new(),
        own_imports: Vec::new(),
        items: vec![binding("GLOBAL"), HirItem::Function(func("helper", vec![]))],
        internal: false,
    };
    let hir = HirProject {
        name: "p".into(),
        files: vec![file.clone()],
    };
    let dir = std::path::Path::new(".");
    let resolver = Resolver::new(dir, &HashMap::new(), &hir);
    assert!(resolver.top_level_visible_in_file(&file, "GLOBAL"));
    assert!(!resolver.top_level_visible_in_file(&file, "MISSING"));
    assert!(resolver.function_visible_in_file(&file, "helper"));
    assert!(!resolver.function_visible_in_file(&file, "missingfn"));
}

#[test]
fn link_target_signature_lookup() {
    let dir = std::path::Path::new(".");
    let hir = HirProject {
        name: "p".into(),
        files: vec![HirFile {
            path: "lib.mfb".into(),
            imports: Vec::new(),
            own_imports: Vec::new(),
            items: vec![HirItem::Link(crate::hir::elaborate_link_block(
                &crate::ast::LinkBlock {
                    library: "lib".into(),
                    alias: "db".into(),
                    cstructs: Vec::new(),
                    functions: vec![crate::ast::LinkFunction {
                        name: "open".into(),
                        params: vec![param("path", Some("CString"))],
                        return_type: Some("CPtr".into()),
                        return_resource: false,
                        return_state_type: None,
                        symbol: "open".into(),
                        abi: crate::ast::AbiSpec {
                            slots: Vec::new(),
                            return_name: "ret".into(),
                            return_ctype: "CPtr".into(),
                            line: 3,
                        },
                        consts: Vec::new(),
                        bind_in: Vec::new(),
                        bind_state: None,
                        buffers: Vec::new(),
                        result_length: None,
                        success_on: None,
                        result: None,
                        free: None,
                        line: 3,
                    }],
                    line: 1,
                },
            ))],
            internal: false,
        }],
    };
    let resolver = Resolver::new(dir, &HashMap::new(), &hir);
    assert!(resolver.link_target_signature("db.open").is_some());
    assert!(resolver.link_target_signature("db.missing").is_none());
    assert!(resolver.link_target_signature("other.open").is_none());
    assert!(resolver.link_target_signature("dbopen").is_none());
}

#[test]
fn resolve_valid_fixtures_succeed() {
    for name in [
        "parser-hello-world",
        "control-flow-match",
        "control-flow-match-when",
        "control-flow-if",
        "overload-func-valid",
        "overload-nothing-valid",
        "doc-block-valid",
        "native-resource-link-valid",
        "math_package_valid",
    ] {
        assert!(
            resolve_fixture(name).is_ok(),
            "fixture `{name}` should resolve"
        );
    }
}

#[test]
fn validate_project_docs_true_for_valid_docs() {
    let dir = crate::testutil::fixture_dir("doc-block-valid");
    let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
    let pname = manifest
        .get("name")
        .and_then(|v| v.get::<String>())
        .cloned()
        .unwrap();
    let ast = crate::ast::parse_project(&pname, &dir, &manifest).unwrap();
    assert!(validate_project_docs(&dir, &ast));
}

#[test]
fn validate_project_docs_false_for_invalid_docs() {
    let dir = crate::testutil::fixture_dir("doc-block-invalid");
    let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
    let pname = manifest
        .get("name")
        .and_then(|v| v.get::<String>())
        .cloned()
        .unwrap();
    let ast = crate::ast::parse_project(&pname, &dir, &manifest).unwrap();
    assert!(!quiet(|| validate_project_docs(&dir, &ast)));
}

#[test]
fn resolve_invalid_fixtures_fail() {
    for name in [
        "collections-cutover-invalid",
        "doc-block-invalid",
        "native-link-duplicate-resource-invalid",
        "native-resource-close-not-native-invalid",
        "native-resource-close-signature-invalid",
        "func_typesystem_result_invalid",
    ] {
        assert!(
            resolve_fixture(name).is_err(),
            "fixture `{name}` should fail to resolve"
        );
    }
}

#[test]
fn resolve_project_with_no_doc_validation() {
    let dir = crate::testutil::fixture_dir("doc-block-valid");
    let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
    let pname = manifest
        .get("name")
        .and_then(|v| v.get::<String>())
        .cloned()
        .unwrap();
    let ast = crate::ast::parse_project(&pname, &dir, &manifest).unwrap();
    assert!(quiet(|| resolve_project_with(&dir, &manifest, &ast, false)).is_ok());
}

#[test]
fn duplicate_top_level_function_reports() {
    let f1 = func("dup", vec![hir_param("a", Some("Integer"))]);
    let f2 = func("dup", vec![hir_param("a", Some("Integer"))]);
    let mut f3 = func("dup2", vec![hir_param("a", Some("Integer"))]);
    f3.returns = ParameterType::Integer;
    let mut f4 = func("dup2", vec![hir_param("a", Some("Integer"))]);
    f4.returns = ParameterType::String;
    let hir = project_of(vec![
        HirItem::Function(f1),
        HirItem::Function(f2),
        HirItem::Function(f3),
        HirItem::Function(f4),
    ]);
    let dir = std::path::Path::new(".");
    let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
    assert!(resolver.had_error);
}

#[test]
fn reserved_builtin_name_rejected() {
    let hir = project_of(vec![HirItem::Function(func("error", vec![]))]);
    let dir = std::path::Path::new(".");
    let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
    assert!(resolver.had_error);
}

#[test]
fn type_and_resource_names_registered() {
    let hir = project_of(vec![
        HirItem::Type(HirTypeDecl {
            kind: TypeDeclKind::Type,
            visibility: Visibility::Export,
            name: "Widget".into(),
            template_params: Vec::new(),
            fields: Vec::new(),
            includes: Vec::new(),
            variants: Vec::new(),
            members: Vec::new(),
            line: 1,
        }),
        binding_at("Widget", 2),
    ]);
    let dir = std::path::Path::new(".");
    let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
    assert!(resolver
        .types
        .contains(&crate::types::ParameterType::declared("Widget")));
    assert!(resolver.had_error, "duplicate top-level should report");
}

fn binding(name: &str) -> HirItem {
    binding_at(name, 1)
}

fn binding_at(name: &str, line: usize) -> HirItem {
    HirItem::Binding(HirTopLevelBinding {
        mutable: false,
        resource: false,
        state_type: None,
        name: name.into(),
        type_: ParameterType::Unknown,
        explicit_type: false,
        value: None,
        visibility: Visibility::Export,
        line,
    })
}

#[test]
fn function_name_collides_with_prior_binding_reports() {
    // A FUNC declared after a top-level binding of the same name collides.
    let hir = project_of(vec![binding("dup"), HirItem::Function(func("dup", vec![]))]);
    let dir = std::path::Path::new(".");
    let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
    assert!(resolver.had_error);
}

#[test]
fn binding_name_collides_with_prior_function_reports() {
    // A binding declared after a FUNC of the same name collides (the
    // `insert_top_level` function-table branch).
    let hir = project_of(vec![HirItem::Function(func("dup", vec![])), binding("dup")]);
    let dir = std::path::Path::new(".");
    let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
    assert!(resolver.had_error);
}

#[test]
fn alias_function_name_collides_with_prior_binding_reports() {
    // A FUNC re-export alias whose name matches a prior top-level binding
    // hits the `insert_alias_function` duplicate branch. The alias also needs
    // a LINK namespace so its target resolves.
    let link = HirItem::Link(crate::hir::elaborate_link_block(&crate::ast::LinkBlock {
        library: "lib".into(),
        alias: "db".into(),
        cstructs: Vec::new(),
        functions: vec![crate::ast::LinkFunction {
            name: "close".into(),
            params: vec![param("h", Some("CPtr"))],
            return_type: Some("Nothing".into()),
            return_resource: false,
            return_state_type: None,
            symbol: "close".into(),
            abi: crate::ast::AbiSpec {
                slots: Vec::new(),
                return_name: "ret".into(),
                return_ctype: "CInt32".into(),
                line: 2,
            },
            consts: Vec::new(),
            bind_in: Vec::new(),
            bind_state: None,
            buffers: Vec::new(),
            result_length: None,
            success_on: None,
            result: None,
            free: None,
            line: 2,
        }],
        line: 1,
    }));
    let alias = HirItem::FuncAlias(crate::ast::FuncAlias {
        visibility: Visibility::Export,
        name: "dup".into(),
        target: "db.close".into(),
        line: 5,
    });
    let hir = project_of(vec![link, binding("dup"), alias]);
    let dir = std::path::Path::new(".");
    let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
    assert!(resolver.had_error);
}

/// plan-136-C: a top-level binding with the given visibility.
fn binding_with(name: &str, visibility: Visibility) -> HirItem {
    HirItem::Binding(HirTopLevelBinding {
        mutable: false,
        resource: false,
        state_type: None,
        name: name.into(),
        type_: ParameterType::Unknown,
        explicit_type: false,
        value: None,
        visibility,
        line: 3,
    })
}

/// Collect `items` as the top level of file `a.mfb`, then introduce a local
/// `name` in `file`; whether `check_new_local` reported anything.
fn new_local_is_refused(items: Vec<HirItem>, file: &HirFile, name: &str) -> bool {
    let hir = project_of(items);
    let dir = std::path::Path::new(".");
    let mut resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
    assert!(!resolver.had_error, "the declarations themselves are valid");
    quiet(|| resolver.check_new_local(file, name, 7, &HashMap::new(), "duplicate".to_string()));
    resolver.had_error
}

fn private_secret() -> String {
    crate::internal_name::mangle_private(&crate::internal_name::file_scope_hash("a.mfb"), "secret")
}

#[test]
fn a_local_named_like_an_own_file_private_binding_is_refused() {
    assert!(new_local_is_refused(
        vec![binding_with(&private_secret(), Visibility::Private)],
        &hir_file("a.mfb"),
        "secret"
    ));
}

#[test]
fn a_local_named_like_another_files_private_binding_is_allowed() {
    assert!(!new_local_is_refused(
        vec![binding_with(&private_secret(), Visibility::Private)],
        &hir_file("b.mfb"),
        "secret"
    ));
}

#[test]
fn a_local_named_like_a_public_binding_of_another_file_is_refused() {
    assert!(new_local_is_refused(
        vec![binding_with("limit", Visibility::Public)],
        &hir_file("b.mfb"),
        "limit"
    ));
}

#[test]
fn a_local_named_like_a_function_is_allowed() {
    assert!(!new_local_is_refused(
        vec![HirItem::Function(func("helper", vec![]))],
        &hir_file("a.mfb"),
        "helper"
    ));
}

#[test]
fn an_internal_name_or_a_built_in_source_file_is_exempt() {
    // A compiler-internal local name no user can spell.
    assert!(!new_local_is_refused(
        vec![binding_with("limit", Visibility::Public)],
        &hir_file("a.mfb"),
        "$for_iter0"
    ));
    // A built-in package's source file: the user's bindings are not its own.
    let mut built_in = hir_file("builtins/json.mfb");
    built_in.internal = true;
    assert!(!new_local_is_refused(
        vec![binding_with("limit", Visibility::Public)],
        &built_in,
        "limit"
    ));
}

#[test]
fn alias_function_registers_when_unique() {
    // A unique alias registers as a callable carrying the target's params.
    let link = HirItem::Link(crate::hir::elaborate_link_block(&crate::ast::LinkBlock {
        library: "lib".into(),
        alias: "db".into(),
        cstructs: Vec::new(),
        functions: vec![crate::ast::LinkFunction {
            name: "close".into(),
            params: vec![param("h", Some("CPtr"))],
            return_type: Some("Nothing".into()),
            return_resource: false,
            return_state_type: None,
            symbol: "close".into(),
            abi: crate::ast::AbiSpec {
                slots: Vec::new(),
                return_name: "ret".into(),
                return_ctype: "CInt32".into(),
                line: 2,
            },
            consts: Vec::new(),
            bind_in: Vec::new(),
            bind_state: None,
            buffers: Vec::new(),
            result_length: None,
            success_on: None,
            result: None,
            free: None,
            line: 2,
        }],
        line: 1,
    }));
    let alias = HirItem::FuncAlias(crate::ast::FuncAlias {
        visibility: Visibility::Export,
        name: "closeDb".into(),
        target: "db.close".into(),
        line: 5,
    });
    let hir = project_of(vec![link, alias]);
    let dir = std::path::Path::new(".");
    let file = hir.files[0].clone();
    let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
    assert!(!resolver.had_error);
    assert!(resolver.function_visible_in_file(&file, "closeDb"));
}

/// A `FUNC name(x AS Integer)` of `visibility` declared on `line`.
fn func_at(name: &str, visibility: Visibility, line: usize) -> HirItem {
    let mut function = func(name, vec![hir_param("x", Some("Integer"))]);
    function.visibility = visibility;
    function.line = line;
    HirItem::Function(function)
}

/// bug-912: a PRIVATE `FUNC f` is file-scoped while the same file's PUBLIC
/// `FUNC f` keeps its name, so they no longer share a symbol-table key. They
/// are still one overload set inside the file, and the same signature twice
/// stays a duplicate declaration in either order.
#[test]
fn a_private_and_a_public_function_of_one_signature_in_one_file_are_duplicates() {
    let private_f =
        crate::internal_name::mangle_private(&crate::internal_name::file_scope_hash("a.mfb"), "f");
    let dir = std::path::Path::new(".");
    for items in [
        vec![
            func_at("f", Visibility::Public, 1),
            func_at(&private_f, Visibility::Private, 4),
        ],
        vec![
            func_at(&private_f, Visibility::Private, 1),
            func_at("f", Visibility::Public, 4),
        ],
    ] {
        let hir = project_of(items);
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
        assert!(resolver.had_error, "the second declaration is a duplicate");
    }
}

/// The neighbours of that rule: a different signature is a legal overload,
/// and another file's PRIVATE `f` never meets this file's PUBLIC `f`.
#[test]
fn a_private_overload_or_another_files_private_is_not_a_duplicate() {
    let private_f =
        crate::internal_name::mangle_private(&crate::internal_name::file_scope_hash("a.mfb"), "f");
    let dir = std::path::Path::new(".");
    let mut overload = func(&private_f, vec![hir_param("x", Some("String"))]);
    overload.visibility = Visibility::Private;
    let hir = project_of(vec![
        func_at("f", Visibility::Public, 1),
        HirItem::Function(overload),
    ]);
    assert!(!quiet(|| Resolver::new(dir, &HashMap::new(), &hir)).had_error);

    let other_private_f =
        crate::internal_name::mangle_private(&crate::internal_name::file_scope_hash("b.mfb"), "f");
    let mut other = hir_file("b.mfb");
    other
        .items
        .push(func_at(&other_private_f, Visibility::Private, 1));
    let mut hir = project_of(vec![func_at("f", Visibility::Public, 1)]);
    hir.files.push(other);
    assert!(!quiet(|| Resolver::new(dir, &HashMap::new(), &hir)).had_error);
}
