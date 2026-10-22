use crate::manifest::validate_project_manifest;
use std::fs;
use tempfile::TempDir;

mod diagnostics;

fn quiet<T>(f: impl FnOnce() -> T) -> T {
    crate::testutil::silence_panics(f)
}

/// Resolve an inline single-file executable project whose `src/main.mfb` is
/// `source`. Returns `true` when resolution failed (or the source did not
/// even parse).
fn resolve_source_fails(source: &str) -> bool {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("project.json"),
        r#"{ "name": "scratch", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
                 "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
                 "entry": "main", "targets": ["native"] }"#,
    )
    .unwrap();
    fs::write(root.join("src").join("main.mfb"), source).unwrap();
    let manifest = validate_project_manifest(&root.join("project.json")).unwrap();
    let Ok(ast) = quiet(|| crate::ast::parse_project("scratch", root, &manifest)) else {
        return true;
    };
    quiet(|| crate::resolver::resolve_project(root, &manifest, &ast)).is_err()
}

fn resolve_fixture_fails(name: &str) -> bool {
    let dir = crate::testutil::fixture_dir(name);
    let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
    let pname = manifest
        .get("name")
        .and_then(|v| v.get::<String>())
        .cloned()
        .unwrap();
    let ast = crate::ast::parse_project(&pname, &dir, &manifest).unwrap();
    quiet(|| crate::resolver::resolve_project(&dir, &manifest, &ast)).is_err()
}

/// The unified `crypto::generate(Certificate)` replaced the per-type
/// `generateP256`/`generateP384`/`generateP521`/`generateEd25519` members (and
/// their long-gone `generateP*Raw` internals). Every removed spelling must fail
/// to resolve from user source, while `generate` resolves.
#[test]
fn crypto_removed_key_generators_are_not_user_callable() {
    for gone in [
        "generateP256Raw",
        "generateP384Raw",
        "generateP521Raw",
        "generateP256",
        "generateP384",
        "generateP521",
        "generateEd25519",
    ] {
        assert!(
            resolve_source_fails(&format!(
                "IMPORT crypto\nFUNC main() AS Nothing\n  LET k = crypto::{gone}()\nEND FUNC\n"
            )),
            "crypto::{gone} must not resolve from user source"
        );
    }
    assert!(
        !resolve_source_fails(
            "IMPORT crypto\nFUNC main() AS Nothing\n  LET k = crypto::generate(crypto::Certificate.P256)\nEND FUNC\n"
        ),
        "crypto::generate(Certificate) must resolve as the unified generator"
    );
}

/// The unified `crypto::hash(Hash, …)`, `crypto::sign(Certificate, …)`, and
/// `crypto::verify(Certificate, …)` replaced the per-digest `sha*` and per-curve
/// `p*Sign`/`p*Verify`/`ed25519Sign`/`ed25519Verify` members. Every removed
/// spelling must fail to resolve from user source, while the unified members do.
#[test]
fn crypto_removed_hash_and_signature_members_are_not_user_callable() {
    for gone in [
        "crypto::sha224(\"x\")",
        "crypto::sha256(\"x\")",
        "crypto::sha384(\"x\")",
        "crypto::sha512(\"x\")",
        "crypto::p256Sign(crypto::randomBytes(97), crypto::randomBytes(8))",
        "crypto::p384Sign(crypto::randomBytes(145), crypto::randomBytes(8))",
        "crypto::p521Sign(crypto::randomBytes(199), crypto::randomBytes(8))",
        "crypto::ed25519Sign(crypto::randomBytes(32), crypto::randomBytes(8))",
        "crypto::p256Verify(crypto::randomBytes(65), crypto::randomBytes(8), crypto::randomBytes(72))",
        "crypto::p384Verify(crypto::randomBytes(97), crypto::randomBytes(8), crypto::randomBytes(104))",
        "crypto::p521Verify(crypto::randomBytes(133), crypto::randomBytes(8), crypto::randomBytes(139))",
        "crypto::ed25519Verify(crypto::randomBytes(32), crypto::randomBytes(8), crypto::randomBytes(64))",
    ] {
        assert!(
            resolve_source_fails(&format!(
                "IMPORT crypto\nFUNC main() AS Nothing\n  LET k = {gone}\nEND FUNC\n"
            )),
            "{gone} must not resolve from user source"
        );
    }
    for ok in [
        "crypto::hash(crypto::Hash.SHA2_256, crypto::randomBytes(4))",
        "crypto::sign(crypto::Certificate.Ed25519, crypto::randomBytes(32), crypto::randomBytes(8))",
        "crypto::verify(crypto::Certificate.Ed25519, crypto::randomBytes(32), crypto::randomBytes(8), crypto::randomBytes(64))",
    ] {
        assert!(
            !resolve_source_fails(&format!(
                "IMPORT crypto\nFUNC main() AS Nothing\n  LET k = {ok}\nEND FUNC\n"
            )),
            "{ok} must resolve as a unified crypto member"
        );
    }
}

#[test]
fn broad_valid_fixtures_resolve() {
    for name in [
        // control-flow-valid was consolidated into control-flow-behavior by
        // the tests reorganization; it stays a valid resolve fixture.
        "control-flow-behavior",
        "control-flow-match",
        "control-flow-match-when",
        "control-flow-match-destructuring",
        "control-flow-match-else",
        "control-flow-if",
        "lambda-capture-valid",
        "collection-list-bindings",
        "collection-map-bindings",
        "func_return_overload_valid",
        "native-resource-link-valid",
    ] {
        let dir = crate::testutil::fixture_dir(name);
        let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
        let pname = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .unwrap();
        let ast = crate::ast::parse_project(&pname, &dir, &manifest).unwrap();
        assert!(
            quiet(|| crate::resolver::resolve_project(&dir, &manifest, &ast)).is_ok(),
            "fixture `{name}` should resolve cleanly"
        );
    }
}

#[test]
fn unknown_identifier_reports() {
    assert!(resolve_source_fails(
        "IMPORT io\n\nFUNC main() AS Nothing\n  io::print(missingVar)\nEND FUNC\n"
    ));
}

#[test]
fn unknown_type_reports() {
    assert!(resolve_source_fails(
        "FUNC main() AS Nothing\n  LET x AS NoSuchType = 0\nEND FUNC\n"
    ));
}

#[test]
fn result_type_not_user_visible_reports() {
    assert!(resolve_source_fails(
        "FUNC main() AS Nothing\n  LET x AS Result = 0\nEND FUNC\n"
    ));
}

#[test]
fn duplicate_local_reports() {
    assert!(resolve_source_fails(
        "FUNC main() AS Nothing\n  LET x AS Integer = 1\n  LET x AS Integer = 2\nEND FUNC\n"
    ));
}

#[test]
fn duplicate_parameter_reports() {
    assert!(resolve_source_fails(
        "FUNC doit(a AS Integer, a AS Integer) AS Nothing\nEND FUNC\n\nFUNC main() AS Nothing\nEND FUNC\n"
    ));
}

#[test]
fn unknown_import_qualified_use_reports() {
    assert!(resolve_source_fails(
        "FUNC main() AS Nothing\n  foo::bar()\nEND FUNC\n"
    ));
}

#[test]
fn duplicate_import_binding_reports() {
    assert!(resolve_source_fails(
        "IMPORT io\nIMPORT io\n\nFUNC main() AS Nothing\nEND FUNC\n"
    ));
}

#[test]
fn unknown_type_field_reports() {
    assert!(resolve_source_fails(
        "TYPE Widget\n  size AS NoSuchType\nEND TYPE\n\nFUNC main() AS Nothing\nEND FUNC\n"
    ));
}

#[test]
fn builtin_member_unknown_reports() {
    assert!(resolve_source_fails(
        "IMPORT io\n\nFUNC main() AS Nothing\n  io::notARealFunction()\nEND FUNC\n"
    ));
}

#[test]
fn callable_not_top_level_reports() {
    assert!(resolve_source_fails(
        "FUNC main() AS Nothing\n  notAFunction()\nEND FUNC\n"
    ));
}

#[test]
fn resolution_error_fixtures_fail() {
    for name in [
        "native-resource-close-not-native-invalid",
        "native-resource-close-signature-invalid",
        "native-link-duplicate-resource-invalid",
        "func_typesystem_result_invalid",
        "collections-cutover-invalid",
        "doc-block-invalid",
    ] {
        assert!(
            resolve_fixture_fails(name),
            "fixture `{name}` should fail to resolve"
        );
    }
}

/// Resolve an inline source and assert it resolves cleanly (no errors). Used
/// to drive the *success* side of statement / expression / type_name arms.
fn assert_source_ok(source: &str) {
    assert!(
        !resolve_source_fails(source),
        "source should resolve cleanly:\n{source}"
    );
}

// --- statement arms (success side) ---

#[test]
fn statements_return_exit_fail_recover_continue() {
    assert_source_ok(concat!(
        "IMPORT io\n",
        "IMPORT os\n\n",
        "FUNC pick AS Integer\n",
        "  FOR i = 1 TO 3\n",
        "    IF i = 2 THEN CONTINUE FOR\n",
        "  NEXT\n",
        "  RETURN 1\n",
        "  TRAP(err)\n",
        "    RECOVER err.code\n",
        "  END TRAP\n",
        "END FUNC\n\n",
        "FUNC stop() AS Nothing\n",
        "  os::exit(0)\n",
        "END FUNC\n\n",
        "FUNC boom AS Integer\n",
        "  FAIL error(1, \"x\")\n",
        "END FUNC\n\n",
        "FUNC main() AS Nothing\n",
        "  io::print(toString(pick()))\n",
        "END FUNC\n",
    ));
}

#[test]
fn statements_assign_and_propagate() {
    assert_source_ok(concat!(
        "IMPORT io\n\n",
        "FUNC helper() AS Integer\n",
        "  RETURN 1\n",
        "END FUNC\n\n",
        "FUNC main() AS Nothing\n",
        "  MUT total AS Integer = 0\n",
        "  total = total + helper()\n",
        "  io::print(toString(total))\n",
        "END FUNC\n",
    ));
}

#[test]
fn nested_blocks_if_while_dountil() {
    assert_source_ok(concat!(
        "IMPORT io\n\n",
        "FUNC main() AS Nothing\n",
        "  MUT n AS Integer = 0\n",
        "  IF n < 1 THEN\n",
        "    n = 1\n",
        "  ELSE\n",
        "    n = 2\n",
        "  END IF\n",
        "  WHILE n < 3\n",
        "    n = n + 1\n",
        "  END WHILE\n",
        "  DO\n",
        "    n = n + 1\n",
        "  LOOP UNTIL n > 5\n",
        "  io::print(toString(n))\n",
        "END FUNC\n",
    ));
}

#[test]
fn foreach_with_list_and_map_types() {
    assert_source_ok(concat!(
        "IMPORT io\n\n",
        "FUNC main() AS Nothing\n",
        "  LET xs AS List OF Integer = [1, 2, 3]\n",
        "  FOR EACH x IN xs\n",
        "    io::print(toString(x))\n",
        "  NEXT\n",
        "  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }\n",
        "  FOR EACH e IN m\n",
        "    io::print(e.key)\n",
        "  NEXT\n",
        "END FUNC\n",
    ));
}

#[test]
fn duplicate_local_in_for_loop_reports() {
    assert!(resolve_source_fails(concat!(
        "FUNC main() AS Nothing\n",
        "  LET i AS Integer = 0\n",
        "  FOR i = 1 TO 3\n",
        "  NEXT\n",
        "END FUNC\n",
    )));
}

#[test]
fn duplicate_local_in_foreach_reports() {
    assert!(resolve_source_fails(concat!(
        "FUNC main() AS Nothing\n",
        "  LET item AS Integer = 0\n",
        "  LET xs AS List OF Integer = [1]\n",
        "  FOR EACH item IN xs\n",
        "  NEXT\n",
        "END FUNC\n",
    )));
}

// --- match arms ---

#[test]
fn match_with_union_guard_and_oneof() {
    assert_source_ok(concat!(
        "IMPORT io\n\n",
        "TYPE Circle\n  radius AS Integer\nEND TYPE\n\n",
        "UNION Shape\n  Circle\nEND UNION\n\n",
        "FUNC describe(shape AS Shape) AS Integer\n",
        "  MATCH shape\n",
        "    CASE Circle(c) WHEN c.radius\n",
        "      RETURN 1\n",
        "    CASE ELSE\n",
        "      RETURN 0\n",
        "  END MATCH\n",
        "END FUNC\n\n",
        "FUNC main() AS Nothing\n",
        "  MATCH 2\n",
        "    CASE 1, 2\n",
        "      io::print(\"lo\")\n",
        "    CASE ELSE\n",
        "      io::print(\"hi\")\n",
        "  END MATCH\n",
        "END FUNC\n",
    ));
}

// --- expression arms ---

#[test]
fn expressions_binary_unary_lambda_constructor() {
    assert_source_ok(concat!(
        "IMPORT io\n\n",
        "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\n",
        "FUNC main() AS Nothing\n",
        "  LET p AS Point = Point[1, 2]\n",
        "  LET q AS Point = WITH p { x := 5 }\n",
        "  LET neg AS Integer = -p.x\n",
        "  LET sum AS Integer = p.x + q.y\n",
        "  LET f AS FUNC(Integer) AS Integer = LAMBDA(v AS Integer) -> v + sum\n",
        "  LET applied AS Integer = f(neg)\n",
        "  LET xs AS List OF Integer = [1, 2, 3]\n",
        "  io::print(toString(applied))\n",
        "END FUNC\n",
    ));
}

#[test]
fn trapped_expression_binding_in_scope() {
    assert_source_ok(concat!(
        "IMPORT io\n\n",
        "FUNC risky AS Integer\n",
        "  RETURN 1\n",
        "END FUNC\n\n",
        "FUNC main() AS Nothing\n",
        "  LET v AS Integer = risky() TRAP(err)\n",
        "    io::print(toString(err.code))\n",
        "    RECOVER 0\n",
        "  END TRAP\n",
        "  io::print(toString(v))\n",
        "END FUNC\n",
    ));
}

// --- type_name grammar arms ---

#[test]
fn type_name_function_type_and_nested_generics() {
    assert_source_ok(concat!(
        "FUNC main() AS Nothing\n",
        "  LET a AS FUNC(Integer, String) AS Boolean = LAMBDA(n AS Integer, s AS String) -> n > 0\n",
        "  LET b AS List OF List OF Integer = [[1], [2]]\n",
        "  LET c AS Map OF String TO List OF Integer = Map OF String TO List OF Integer {}\n",
        "  LET e AS FUNC() AS Integer = LAMBDA() -> 0\n",
        "  LET used AS Boolean = a(1, \"x\")\n",
        "  LET zero AS Integer = e()\n",
        "END FUNC\n",
    ));
}

#[test]
fn malformed_function_type_reports() {
    // `FUNC(...)` without `) AS` return separator is malformed.
    assert!(resolve_source_fails(concat!(
        "FUNC main() AS Nothing\n",
        "  LET a AS FUNC(Integer = 0\n",
        "END FUNC\n",
    )));
}

#[test]
fn union_and_enum_declarations_resolve() {
    assert_source_ok(concat!(
        "IMPORT io\n\n",
        "TYPE Circle\n  radius AS Integer\nEND TYPE\n\n",
        "TYPE Square\n  side AS Integer\nEND TYPE\n\n",
        "UNION Shape\n  Circle\n  Square\nEND UNION\n\n",
        "ENUM Color\n  Red, Green, Blue\nEND ENUM\n\n",
        "FUNC main() AS Nothing\n",
        "  io::print(\"ok\")\n",
        "END FUNC\n",
    ));
}

// --- LINK / resource / func-alias arms ---

#[test]
fn link_block_and_resource_and_alias_resolve() {
    assert_source_ok(concat!(
        "EXPORT RESOURCE Db CLOSE BY dbLink::close\n\n",
        "LINK \"sqlite3\" AS dbLink\n",
        "  FUNC open(path AS String) AS RES Db\n",
        "    SYMBOL \"sqlite3_open\"\n",
        "    ABI (path CString, produced OUT CPtr) AS status CInt32\n",
        "    SUCCESS_ON status = 0\n",
        "  END FUNC\n\n",
        "  FUNC close(db AS Db) AS Nothing\n",
        "    SYMBOL \"sqlite3_close\"\n",
        "    ABI (db CPtr) AS status CInt32\n",
        "    SUCCESS_ON status = 0\n",
        "  END FUNC\n",
        "END LINK\n\n",
        "EXPORT FUNC closeDb AS dbLink::close\n\n",
        "FUNC main() AS Nothing\n",
        "END FUNC\n",
    ));
}

#[test]
fn resource_close_unknown_alias_reports() {
    assert!(resolve_source_fails(concat!(
        "RESOURCE Db CLOSE BY ghostLink::close\n\n",
        "FUNC main() AS Nothing\n",
        "END FUNC\n",
    )));
}

#[test]
fn resource_close_missing_func_in_link_reports() {
    assert!(resolve_source_fails(concat!(
        "RESOURCE Db CLOSE BY dbLink::missing\n\n",
        "LINK \"sqlite3\" AS dbLink\n",
        "  FUNC close(db AS Db) AS Nothing\n",
        "    SYMBOL \"sqlite3_close\"\n",
        "    ABI (db CPtr) AS status CInt32\n",
        "    SUCCESS_ON status = 0\n",
        "  END FUNC\n",
        "END LINK\n\n",
        "FUNC main() AS Nothing\n",
        "END FUNC\n",
    )));
}

#[test]
fn func_alias_unknown_target_reports() {
    assert!(resolve_source_fails(concat!(
        "EXPORT FUNC bogus AS ghostLink::nope\n\n",
        "FUNC main() AS Nothing\n",
        "END FUNC\n",
    )));
}

#[test]
fn link_member_unknown_reports() {
    // `dbLink::notThere` names an unknown member of the LINK namespace.
    assert!(resolve_source_fails(concat!(
        "LINK \"sqlite3\" AS dbLink\n",
        "  FUNC close(db AS Db) AS Nothing\n",
        "    SYMBOL \"sqlite3_close\"\n",
        "    ABI (db CPtr) AS status CInt32\n",
        "    SUCCESS_ON status = 0\n",
        "  END FUNC\n",
        "END LINK\n\n",
        "RESOURCE Db CLOSE BY dbLink::close\n\n",
        "FUNC main() AS Nothing\n",
        "  dbLink::notThere()\n",
        "END FUNC\n",
    )));
}

// --- DOC block success + additional error branches ---

#[test]
fn doc_block_valid_variants_resolve() {
    assert_source_ok(concat!(
        "DOC\n  PACKAGE\n  DESC A package.\nEND DOC\n\n",
        "DOC INTERNAL\n  FUNC add\n  ARG a first\n  ARG b second\n  RET the sum\n",
        "  ERROR 1001 when it overflows\n  GROUP Math\nEND DOC\n",
        "EXPORT FUNC add(a AS Integer, b AS Integer) AS Integer\n",
        "  RETURN a + b\n",
        "END FUNC\n\n",
        "DOC\n  TYPE Point\n  PROP x the x\n  PROP y the y\nEND DOC\n",
        "EXPORT TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\n",
        "DOC\n  ENUM Color\n  PROP Red the red\nEND DOC\n",
        "EXPORT ENUM Color\n  Red, Green\nEND ENUM\n\n",
        "DOC\n  UNION Shape\n  PROP Point a point\nEND DOC\n",
        "EXPORT UNION Shape\n  Point\nEND UNION\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    ));
}

#[test]
fn doc_overload_disambiguated_by_types() {
    assert_source_ok(concat!(
        "DOC\n  FUNC scale(Integer)\n  ARG n the value\nEND DOC\n",
        "EXPORT FUNC scale(n AS Integer) AS Integer\n",
        "  RETURN n * 2\n",
        "END FUNC\n\n",
        "DOC\n  FUNC scale(Float)\n  ARG n the value\nEND DOC\n",
        "EXPORT FUNC scale(n AS Float) AS Float\n",
        "  RETURN n * 2.0\n",
        "END FUNC\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    ));
}

#[test]
fn doc_ret_on_non_callable_reports() {
    // RET is only valid on FUNC; here it sits on a TYPE block.
    assert!(resolve_source_fails(concat!(
        "DOC\n  TYPE Point\n  RET nonsense\nEND DOC\n",
        "TYPE Point\n  x AS Integer\nEND TYPE\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

#[test]
fn doc_type_name_mismatch_reports() {
    // `TYPE add` names a FUNC → DOC_NAME_MISMATCH.
    assert!(resolve_source_fails(concat!(
        "DOC\n  TYPE add\n  DESC wrong kind.\nEND DOC\n",
        "EXPORT FUNC add(a AS Integer) AS Integer\n",
        "  RETURN a\n",
        "END FUNC\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

// --- additional DOC error branches ---

#[test]
fn doc_duplicate_internal_attr_reports() {
    assert!(resolve_source_fails(concat!(
        "DOC INTERNAL INTERNAL\n  FUNC f\nEND DOC\n",
        "EXPORT FUNC f() AS Integer\n  RETURN 0\nEND FUNC\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

#[test]
fn doc_arg_and_error_invalid_context_reports() {
    // ARG / RET / ERROR on a TYPE block are all invalid.
    assert!(resolve_source_fails(concat!(
        "DOC\n  TYPE Point\n  ARG a nope\n  ERROR 1 nope\nEND DOC\n",
        "EXPORT TYPE Point\n  x AS Integer\nEND TYPE\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

#[test]
fn doc_duplicate_ret_example_deprecated_group_reports() {
    assert!(resolve_source_fails(concat!(
        "DOC\n  FUNC f\n  RET one\n  RET two\n",
        "  EXAMPLE\n    LET a AS Integer = 1\n  END EXAMPLE\n",
        "  EXAMPLE\n    LET b AS Integer = 2\n  END EXAMPLE\n",
        "  DEPRECATED first\n  DEPRECATED second\n",
        "  GROUP A\n  GROUP B\nEND DOC\n",
        "EXPORT FUNC f() AS Integer\n  RETURN 0\nEND FUNC\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

#[test]
fn doc_prop_invalid_context_and_internal_on_package_reports() {
    assert!(resolve_source_fails(concat!(
        "DOC\n  FUNC f\n  PROP x nope\nEND DOC\n",
        "EXPORT FUNC f() AS Integer\n  RETURN 0\nEND FUNC\n\n",
        "DOC INTERNAL\n  PACKAGE\nEND DOC\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

#[test]
fn doc_type_kind_mismatch_reports() {
    // `TYPE Color` names an ENUM (a type decl of the wrong kind).
    assert!(resolve_source_fails(concat!(
        "DOC\n  TYPE Color\n  DESC wrong kind.\nEND DOC\n",
        "EXPORT ENUM Color\n  Red, Green\nEND ENUM\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

#[test]
fn doc_type_unresolved_reports() {
    assert!(resolve_source_fails(concat!(
        "DOC\n  TYPE NoSuchType\n  DESC nothing.\nEND DOC\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

#[test]
fn doc_duplicate_block_for_same_target_reports() {
    assert!(resolve_source_fails(concat!(
        "DOC\n  TYPE Point\n  DESC first.\nEND DOC\n",
        "DOC\n  TYPE Point\n  DESC second (duplicate).\nEND DOC\n",
        "EXPORT TYPE Point\n  x AS Integer\nEND TYPE\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

// --- resource close (no-dot form) ---

#[test]
fn resource_close_not_dotted_reports() {
    // A close op with no `alias.func` form is RESOURCE_CLOSE_NOT_NATIVE.
    assert!(resolve_source_fails(concat!(
        "RESOURCE Db CLOSE BY plainName\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

// --- link block param/return type resolution ---

#[test]
fn link_block_unknown_param_type_reports() {
    // A non-C-ABI param type inside a LINK function must resolve; `Bogus`
    // does not.
    assert!(resolve_source_fails(concat!(
        "LINK \"lib\" AS l\n",
        "  FUNC f(x AS Bogus) AS Nothing\n",
        "    SYMBOL \"f\"\n",
        "    ABI (x CPtr) AS status CInt32\n",
        "    SUCCESS_ON status = 0\n",
        "  END FUNC\n",
        "END LINK\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

// --- binding with declared type + value ---

#[test]
fn top_level_binding_with_type_and_value_resolves() {
    assert_source_ok(concat!(
        "IMPORT io\n\n",
        "LET GREETING AS String = \"hi\"\n\n",
        "FUNC main() AS Nothing\n",
        "  io::print(GREETING)\n",
        "END FUNC\n",
    ));
}

// --- UNION includes + duplicate variant / duplicate enum member ---

#[test]
fn union_with_includes_resolves() {
    assert_source_ok(concat!(
        "IMPORT io\n\n",
        "TYPE Circle\n  radius AS Integer\nEND TYPE\n\n",
        "TYPE Square\n  side AS Integer\nEND TYPE\n\n",
        "UNION Round\n  Circle\nEND UNION\n\n",
        "UNION Shape INCLUDES Round\n  Square\nEND UNION\n\n",
        "FUNC main() AS Nothing\n",
        "  io::print(\"ok\")\n",
        "END FUNC\n",
    ));
}

#[test]
fn union_duplicate_variant_reports() {
    assert!(resolve_source_fails(concat!(
        "TYPE Circle\n  radius AS Integer\nEND TYPE\n\n",
        "UNION Shape\n  Circle\n  Circle\nEND UNION\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

#[test]
fn enum_duplicate_member_reports() {
    assert!(resolve_source_fails(concat!(
        "ENUM Color\n  Red, Red\nEND ENUM\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

// --- StateAssign statement ---

#[test]
fn state_assign_statement_resolves() {
    assert_source_ok(concat!(
        "IMPORT fs\nIMPORT io\n\n",
        "TYPE FileState\n  pos AS Integer\nEND TYPE\n\n",
        "FUNC advance(f AS fs::File STATE FileState, by AS Integer) AS Nothing\n",
        "  f.state = WITH f.state { pos := f.state.pos + by }\n",
        "END FUNC\n\n",
        "FUNC main AS Integer\n",
        "  RES f AS fs::File STATE FileState = fs::createTempFile()\n",
        "  f.state = WITH f.state { pos := 10 }\n",
        "  advance(f, 5)\n",
        "  fs::close(f)\n",
        "  RETURN 0\n",
        "END FUNC\n",
    ));
}

// --- type_name grammar: thread + MapEntry ---

#[test]
fn thread_and_mapentry_type_names_resolve() {
    assert_source_ok(concat!(
        "IMPORT thread\nIMPORT io\n\n",
        "EXPORT ISOLATED FUNC echo(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n",
        "  RETURN 0\n",
        "END FUNC\n\n",
        "FUNC main() AS Nothing\n",
        "  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }\n",
        "  FOR EACH e IN m\n",
        "    LET pair AS MapEntry OF String TO Integer = e\n",
        "    io::print(pair.key)\n",
        "  NEXT\n",
        "END FUNC\n",
    ));
}

// --- import-alias conflicts ---

#[test]
fn import_alias_conflicts_with_builtin_reports() {
    // Aliasing an import to a built-in package name conflicts.
    assert!(resolve_source_fails(concat!(
        "IMPORT thread AS io\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

#[test]
fn import_alias_conflicts_with_top_level_reports() {
    // An import binding that shadows a visible top-level declaration.
    assert!(resolve_source_fails(concat!(
        "IMPORT io AS helper\n\n",
        "FUNC helper() AS Integer\n  RETURN 0\nEND FUNC\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

// --- duplicate TYPE field ---

#[test]
fn duplicate_type_field_reports() {
    assert!(resolve_source_fails(concat!(
        "TYPE Point\n  x AS Integer\n  x AS Integer\nEND TYPE\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

// --- PROPAGATE statement ---

#[test]
fn propagate_statement_resolves() {
    assert_source_ok(concat!(
        "FUNC leaf AS Integer\n",
        "  FAIL error(1, \"leaf\")\n",
        "END FUNC\n\n",
        "FUNC relay AS Integer\n",
        "  RETURN leaf()\n",
        "  TRAP(err)\n",
        "    PROPAGATE\n",
        "  END TRAP\n",
        "END FUNC\n\n",
        "FUNC main AS Integer\n",
        "  RETURN relay()\n",
        "  TRAP(err)\n",
        "    RETURN err.code\n",
        "  END TRAP\n",
        "END FUNC\n",
    ));
}

// --- ISOLATED FUNC type name ---

#[test]
fn isolated_func_type_name_resolves() {
    assert_source_ok(concat!(
        "FUNC run(job AS ISOLATED FUNC(Integer) AS Integer) AS Nothing\n",
        "  LET r AS Integer = job(1)\n",
        "END FUNC\n\n",
        "FUNC main() AS Nothing\n",
        "  run(LAMBDA(n AS Integer) -> n + 1)\n",
        "END FUNC\n",
    ));
}

// --- DOC func header naming a type (funcs miss, types hit) ---

#[test]
fn doc_func_names_a_type_reports() {
    // `FUNC Point` — Point is a TYPE, so the func lookup misses but the type
    // table contains it → DOC_NAME_MISMATCH.
    assert!(resolve_source_fails(concat!(
        "DOC\n  FUNC Point\n  DESC Point is a type, not a func.\nEND DOC\n",
        "EXPORT TYPE Point\n  x AS Integer\nEND TYPE\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}

// --- duplicate DOC block for an overloaded FUNC (with header params) ---

#[test]
fn doc_duplicate_overload_block_reports() {
    assert!(resolve_source_fails(concat!(
        "DOC\n  FUNC scale(Integer)\n  ARG n first.\nEND DOC\n",
        "DOC\n  FUNC scale(Integer)\n  ARG n duplicate.\nEND DOC\n",
        "EXPORT FUNC scale(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\n\n",
        "FUNC main() AS Nothing\nEND FUNC\n",
    )));
}
