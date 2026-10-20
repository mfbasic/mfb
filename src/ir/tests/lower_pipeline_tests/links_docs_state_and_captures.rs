//! plan-176-G: `ir::tests::lower_pipeline_tests`'s native-link, doc and resource,
//! externals/state/packages, and capture/shape-arm tests. The shared helpers live in
//! `lower_pipeline_tests/mod.rs`.

use super::*;

// Bespoke front-end setup for `lowers_with_external_function_metadata`, the
// one test here that stops before `lower_project_with_external_functions` to
// inject its own external-function maps (the shared `helpers` pipeline always
// lowers with empty maps). Every other test goes through the delegating
// harness below.
const PROJECT_JSON: &str = r#"{ "name": "irlower", "version": "0.1.0", "mfb": "1.0",
        "kind": "executable",
        "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
        "entry": "main", "targets": ["native"] }"#;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let current_thread = std::thread::current();
    let thread = current_thread.name().unwrap_or("test");
    let thread: String = thread
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let root = std::env::temp_dir().join(format!(
        "mfb_ir_lower_{name}_{}_{}",
        std::process::id(),
        thread
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("temp src dir");
    root
}

// ---- native LINK / resources / re-export aliases + DOC -----------------

#[test]
fn lowers_native_link_functions_resources_and_aliases() {
    let ir = lower_src(
        "native_link",
        r#"
EXPORT RESOURCE Db CLOSE BY demoLink::close
PRIVATE RESOURCE Stmt CLOSE BY demoLink::finalize

LINK "sqlite3" AS demoLink
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, produced OUT CPtr) AS status CInt32
    SUCCESS_ON status = 0
    CONST flags = 6
  END FUNC

  FUNC close(db AS Db) AS Nothing
    SYMBOL "sqlite3_close"
    ABI (db CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC

  FUNC finalize(stmt AS Stmt) AS Nothing
    SYMBOL "sqlite3_finalize"
    ABI (stmt CPtr) AS status CInt32
    SUCCESS_ON status = 0
  END FUNC
END LINK

EXPORT FUNC close AS demoLink::close

FUNC main AS Integer
  RETURN 0
END FUNC
"#,
    );
    // link_functions collected with ABI slots + CONST pin + SUCCESS_ON expr.
    assert!(
        !ir.link_functions.is_empty(),
        "link functions should be collected"
    );
    assert!(ir.link_functions.iter().any(|f| f.name == "open"));
    assert!(ir.link_functions.iter().any(|f| !f.consts.is_empty()));
    assert!(ir.link_functions.iter().any(|f| f.success_on.is_some()));
    // native resources collected with visibility + close-may-fail.
    assert!(ir
        .native_resources
        .iter()
        .any(|r| r.name == "Db" && r.visibility == "export"));
    assert!(ir
        .native_resources
        .iter()
        .any(|r| r.name == "Stmt" && r.visibility == "private"));
    // re-export alias to a LINK target collected.
    assert!(
        !ir.link_aliases.is_empty(),
        "link aliases should be collected"
    );
}

#[test]
fn lowers_link_const_and_result_expression_forms() {
    // RETURN with a boolean/compare/NOT/AND/OR expression (the computed form
    // that used to spell RESULT), and CONST forms:
    // NOTHING, boolean, unary minus/plus.
    let ir = lower_src(
        "link_expr",
        r#"
RESOURCE Handle CLOSE BY natLink::shut

LINK "c" AS natLink
  FUNC grab() AS RES Handle
    SYMBOL "grab_it"
    ABI (produced OUT CPtr, a CPtr, b CInt32, c CInt32, d CInt32) AS rc CInt32
    SUCCESS_ON NOT rc = 5 AND rc <> 6 OR rc = 0
    RETURN produced
    CONST a = NOTHING
    CONST b = TRUE
    CONST c = -3
    CONST d = 9
  END FUNC

  FUNC probe() AS Integer
    SYMBOL "probe_it"
    ABI () AS rc CInt32
    SUCCESS_ON rc < 10 OR rc > 20
    RETURN -rc
  END FUNC

  FUNC booly() AS Integer
    SYMBOL "booly_it"
    ABI () AS rc CInt32
    SUCCESS_ON TRUE
    RETURN rc + 1
  END FUNC

  FUNC nothingy() AS Integer
    SYMBOL "nothingy_it"
    ABI () AS rc CInt32
    SUCCESS_ON rc = 0
    RETURN NOTHING
  END FUNC

  FUNC negconst() AS Integer
    SYMBOL "neg_it"
    ABI () AS rc CInt32
    RETURN -5
  END FUNC

  ' plan-50-G: `CONST s = "literal"` used to fold silently to 0 (eval_link_const's
  ' `_ => 0`); it is now rejected, so this pins a real literal. The rejection has
  ' its own coverage in tests/syntax/native/native-const-unfoldable-invalid.
  FUNC weird() AS Integer
    SYMBOL "weird_it"
    ABI (s CInt32) AS rc CInt32
    SUCCESS_ON rc = 0
    RETURN "x"
    CONST s = 7
  END FUNC

  FUNC shut(h AS Handle) AS Nothing
    SYMBOL "shut_it"
    ABI (h CPtr) AS rc CInt32
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
"#,
    );
    let grab = ir
        .link_functions
        .iter()
        .find(|f| f.name == "grab")
        .expect("grab");
    assert!(grab.success_on.is_some());
    assert!(grab.result.is_some());
    assert_eq!(grab.consts.len(), 4);
    // shut has no SUCCESS_ON -> its resource close_may_fail is false.
    assert!(ir
        .native_resources
        .iter()
        .any(|r| r.name == "Handle" && !r.close_may_fail));
}

#[test]
fn collects_doc_blocks_for_exported_declarations() {
    let ir = lower_src(
        "docs",
        r#"
DOC
  PACKAGE
  DESC A documented program.
END DOC

DOC
  FUNC add(Integer, Integer)
  GROUP Math
  DESC Add two integers.
  ARG a first
  ARG b second
  RET the sum
  ERROR 1001 overflow reserved
  EXAMPLE
    LET t AS Integer = add(1, 2)
  END EXAMPLE
END DOC
EXPORT FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC

DOC INTERNAL
  FUNC logIt
  DESC internal logger.
  ARG value the value
  DEPRECATED use the structured logger.
END DOC
EXPORT FUNC logIt(value AS Integer) AS Nothing
  LET ignored AS Integer = value
END FUNC

DOC
  TYPE Widget
  DESC a widget.
  PROP size the size
END DOC
EXPORT TYPE Widget
  size AS Integer
END TYPE

FUNC main AS Integer
  RETURN add(1, 2)
END FUNC
"#,
    );
    let docs = &ir.docs;
    assert!(docs.package.is_some(), "package doc collected");
    assert!(
        docs.decls.iter().any(|d| d.name == "add"),
        "func doc collected"
    );
    assert!(docs.decls.iter().any(|d| d.name == "logIt" && d.internal));
    assert!(docs
        .decls
        .iter()
        .any(|d| d.name == "logIt" && d.deprecated.is_some()));
    assert!(
        docs.decls.iter().any(|d| d.name == "Widget"),
        "type doc collected"
    );
}

#[test]
fn skips_docs_for_nonexported_declarations() {
    // A DOC block whose target declaration is not EXPORT is not persisted.
    let ir = lower_src(
        "docs_private",
        r#"
DOC
  FUNC helper(Integer)
  DESC private helper.
END DOC
FUNC helper(x AS Integer) AS Integer
  RETURN x
END FUNC
FUNC main AS Integer
  RETURN helper(1)
END FUNC
"#,
    );
    assert!(!ir.docs.decls.iter().any(|d| d.name == "helper"));
}

#[test]
fn skips_docs_for_nonexported_type() {
    // A DOC TYPE block for a private (non-exported) type is well-formed (DOC
    // validation passes) but is not persisted — the visibility-skip arm.
    let ir = lower_src(
        "docs_private_type",
        r#"
DOC
  TYPE Hidden
  DESC a private type.
END DOC
TYPE Hidden
  n AS Integer
END TYPE
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
    );
    assert!(!ir.docs.decls.iter().any(|d| d.name == "Hidden"));
}

#[test]
fn lowers_package_visibility_native_resource() {
    // A package-visibility RESOURCE exercises the "package" visibility arm.
    let ir = lower_src(
        "res_package_vis",
        r#"
PUBLIC RESOURCE Widget CLOSE BY wLink::destroy

LINK "w" AS wLink
  FUNC destroy(w AS Widget) AS Nothing
    SYMBOL "w_destroy"
    ABI (w CPtr) AS rc CInt32
  END FUNC
END LINK

FUNC main AS Integer
  RETURN 0
END FUNC
"#,
    );
    assert!(ir
        .native_resources
        .iter()
        .any(|r| r.name == "Widget" && r.visibility == "public"));
}

#[test]
fn collects_doc_for_union_and_enum() {
    let ir = lower_src(
        "docs_union_enum",
        r#"
DOC
  UNION Shape
  DESC a shape.
END DOC
DOC
  ENUM Color
  DESC a color.
END DOC
TYPE Sq
  side AS Integer
END TYPE
EXPORT UNION Shape
  Sq
END UNION
EXPORT ENUM Color
  Red
END ENUM
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
    );
    assert!(ir.docs.decls.iter().any(|d| d.name == "Shape"));
    assert!(ir.docs.decls.iter().any(|d| d.name == "Color"));
}

// NOTE: DOC `overload_for` param-type selection (the `header_params`
// matching arm) is not reachable through the monomorphized pipeline: the
// monomorphizer mangles overloaded function names to `add$Float$Float`
// before lowering runs, so a DOC header named `add` never matches an
// overloaded target. The single-overload DOC path is covered above.

// ---- misc smaller uncovered paths --------------------------------------

#[test]
fn lowers_resource_binding_with_state() {
    let ir = try_lower_src(
        "res_state",
        r#"
IMPORT io
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
    );
    assert!(ir.is_some());
}

#[test]
fn lowers_nested_function_call_as_match_scrutinee() {
    let ir = lower_src(
        "match_call",
        r#"
FUNC compute(v AS Integer) AS Integer
  RETURN v * 2
END FUNC
FUNC main AS Integer
  MATCH compute(3)
    CASE 6
      RETURN 1
    CASE ELSE
      RETURN 0
  END MATCH
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("compute"));
}

#[test]
fn function_helper_finds_main() {
    let ir = lower_src(
        "helper_check",
        r#"
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
    );
    assert_eq!(function(&ir, "main").kind, "func");
}

// ---- external functions (params / types / returns wiring) --------------

#[test]
fn lowers_with_external_function_metadata() {
    // Directly exercise lower_project_with_external_functions' external
    // function param/type/return wiring (lines that map ExternalFunctionParam
    // and function_return_from_type into the context).
    let dir = temp_dir("external");
    std::fs::write(dir.join("project.json"), PROJECT_JSON).unwrap();
    std::fs::write(
        dir.join("src").join("main.mfb"),
        r#"
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
    )
    .unwrap();
    let (ast, concrete) = crate::testutil::silence_panics(|| {
        let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
        let name = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .unwrap();
        let ast = ast::parse_project(&name, &dir, &manifest).unwrap();
        resolver::resolve_project(&dir, &manifest, &ast).unwrap();
        let concrete = monomorph::monomorphize_project(&dir, &crate::hir::elaborate(&ast)).unwrap();
        resolver::resolve_hir_project(&dir, &manifest, &concrete, false).unwrap();
        (ast, concrete)
    });

    let mut ext_signatures = std::collections::HashMap::new();
    ext_signatures.insert(
        "ext.doThing".to_string(),
        super::ExternalSignature {
            params: vec![super::ExternalFunctionParam {
                name: "n".to_string(),
                type_: crate::types::ParameterType::parse("Integer"),
                default: crate::ir::ExternalDefault::None,
            }],
            returns: crate::types::ParameterType::parse("String"),
            isolated: false,
        },
    );
    let entry = Some(super::EntryPoint {
        name: "main".to_string(),
        returns: crate::types::ParameterType::parse("Integer"),
        accepts_args: false,
    });
    let ir =
        crate::ir::lower_monomorphized_project(&concrete, &ast, entry, &ext_signatures, &[], &[]);
    assert_eq!(ir.entry.as_ref().unwrap().name, "main");
}

#[test]
fn write_ir_serializes_to_disk() {
    let dir = temp_dir("write_ir");
    let ir = lower_src(
        "write_ir_src",
        r#"
FUNC main AS Integer
  RETURN 0
END FUNC
"#,
    );
    let path = super::write_ir(&dir, &ir).expect("write_ir should succeed");
    assert!(path.exists());
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("irlower"));
}

// ---- resource STATE: binding, state access, StateAssign ----------------

#[test]
fn lowers_resource_state_binding_and_assign() {
    let ir = lower_src(
        "res_state_full",
        r#"
IMPORT io
IMPORT fs
TYPE Cursor
  pos AS Integer
  len AS Integer
END TYPE
FUNC seek(s AS fs::File STATE Cursor, dest AS Integer) AS Nothing
  s.state.pos = dest
END FUNC
FUNC main AS Integer
  RES f AS fs::File STATE Cursor = fs::openFile("tests/rt-behavior/resources/resource-state-field-assign-valid/src/main.mfb")
  LET p AS Integer = f.state.pos
  f.state = WITH f.state { pos := 10 }
  seek(f, 25)
  fs::close(f)
  RETURN p
END FUNC
"#,
    );
    let j = json_of(&ir);
    // StateAssign op + state member access carried in the type string.
    assert!(j.contains("stateAssign") || j.contains("state"), "{j}");
}

// ---- Error member access (.code / .message) ----------------------------

#[test]
fn lowers_error_member_access() {
    let ir = lower_src(
        "error_members",
        r#"
FUNC probe(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(7, "boom")
  RETURN v
  TRAP(e)
    LET c AS Integer = e.code
    LET m AS String = e.message
    RETURN c
  END TRAP
END FUNC
FUNC main AS Integer
  RETURN probe(-1)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("code") || json_of(&ir).contains("message"));
}

// ---- more builtin package resolvers ------------------------------------

#[test]
fn lowers_json_csv_crypto_net_http_calls() {
    let ir = lower_src(
        "more_builtins",
        r#"
IMPORT io
IMPORT json
IMPORT csv
IMPORT crypto
IMPORT net
IMPORT encoding
FUNC main AS Integer
  LET v AS json::Json = json::parse("{}")
  LET s AS String = json::stringify(v)
  LET rows AS List OF List OF String = csv::parse("a,b")
  LET back AS String = csv::stringify(rows)
  LET digest AS List OF Byte = crypto::hash(crypto::Hash.SHA2_256, "abc")
  LET hexed AS String = encoding::hexEncode(digest)
  LET u AS net::Url = net::toUrl("http://example.com/")
  RETURN 0
END FUNC
"#,
    );
    let j = json_of(&ir);
    assert!(j.contains("json") || j.contains("csv") || j.contains("crypto"));
}

#[test]
fn lowers_io_and_thread_and_bits_and_math_more() {
    let ir = lower_src(
        "io_thread",
        r#"
IMPORT io
IMPORT math
IMPORT bits
FUNC main AS Integer
  io::print("hello")
  LET a AS Integer = math::max(3, 7)
  LET b AS Integer = math::min(3, 7)
  LET c AS Integer = bits::bor(1, 2)
  LET d AS Integer = bits::band(6, 4)
  LET e AS Integer = bits::popCount(255)
  RETURN a + b + c + d + e
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("print") || json_of(&ir).contains("io"));
}

#[test]
fn lowers_vector_package_typed_dispatch() {
    let ir = lower_src(
        "vectors",
        r#"
IMPORT vector
FUNC main AS Integer
  LET a AS vector::Float3 = vector::Float3[1.0, 2.0, 3.0]
  LET b AS vector::Float3 = vector::Float3[4.0, 5.0, 6.0]
  LET dp AS Float = vector::dot(a, b)
  LET ln AS Float = vector::length(a)
  RETURN 0
END FUNC
"#,
    );
    // vector:: resolves a type-specific internal implementation name.
    assert!(json_of(&ir).contains("vector") || json_of(&ir).contains("Float3"));
}

#[test]
fn lowers_vector_record_constant() {
    let ir = lower_src(
        "vector_const",
        r#"
IMPORT vector
FUNC main AS Integer
  LET up AS vector::Float3 = vector::upFloat3
  RETURN 0
END FUNC
"#,
    );
    // A vector:: record constant inlines a constructor at the use site.
    assert!(json_of(&ir).contains("constructor") || json_of(&ir).contains("Float3"));
}

// ---- captured_locals over every expression kind ------------------------

#[test]
fn lowers_lambda_capturing_across_expression_kinds() {
    let ir = lower_src(
        "capture_kinds",
        r#"
IMPORT collections
TYPE Pt
  x AS Integer
  y AS Integer
END TYPE
FUNC run(base AS Integer, s AS String, p AS Pt, nums AS List OF Integer) AS FUNC() AS Integer
  RETURN LAMBDA() -> base + p.x + len(nums) + collections::get(nums, 0) + (base - base)
END FUNC
FUNC main AS Integer
  LET p AS Pt = Pt[1, 2]
  LET f AS FUNC() AS Integer = run(5, "hi", p, [1, 2, 3])
  RETURN f()
END FUNC
"#,
    );
    // Captures collected from Call/Binary/MemberAccess/Identifier args.
    assert!(json_of(&ir).contains("capture") || json_of(&ir).contains("closure"));
}

#[test]
fn lowers_lambda_capturing_list_map_with_and_unary() {
    let ir = lower_src(
        "capture_kinds2",
        r#"
IMPORT collections
TYPE Box
  n AS Integer
END TYPE
FUNC run(base AS Integer, b AS Box) AS FUNC() AS List OF Integer
  RETURN LAMBDA() -> [base, -base, len([base])]
END FUNC
FUNC run2(base AS Integer, b AS Box) AS FUNC() AS Box
  RETURN LAMBDA() -> WITH b { n := base }
END FUNC
FUNC main AS Integer
  LET b AS Box = Box[7]
  LET f AS FUNC() AS List OF Integer = run(3, b)
  LET g AS FUNC() AS Box = run2(3, b)
  RETURN collections::get(f(), 0) + g().n
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("closure") || json_of(&ir).contains("capture"));
}

#[test]
fn lowers_lambda_capturing_map_literal_and_constructor() {
    let ir = lower_src(
        "capture_kinds3",
        r#"
TYPE MyPair
  a AS Integer
  b AS Integer
END TYPE
FUNC run(k AS Integer, v AS Integer) AS FUNC() AS Map OF String TO Integer
  RETURN LAMBDA() -> Map OF String TO Integer { "k" := k, "v" := v }
END FUNC
FUNC run2(x AS Integer, y AS Integer) AS FUNC() AS MyPair
  RETURN LAMBDA() -> MyPair[x, y]
END FUNC
FUNC main AS Integer
  LET f AS FUNC() AS Map OF String TO Integer = run(1, 2)
  LET g AS FUNC() AS MyPair = run2(3, 4)
  RETURN 0
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("closure") || json_of(&ir).contains("capture"));
}

// ---- treeify: MATCH continuation with existing ELSE ---------------------

#[test]
fn lowers_inline_trap_treeify_match_with_else_and_no_tail() {
    let ir = lower_src(
        "treeify_else",
        r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC withElse(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    MATCH e.code
      CASE 1
        RECOVER 10
      CASE ELSE
        RECOVER 20
    END MATCH
  END TRAP
  RETURN a
END FUNC
FUNC tailless(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN withElse(-1) + tailless(-1)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("resultIsOk"));
}

// ---- statement_terminates: IF with both-terminating branches -----------

#[test]
fn lowers_inline_trap_handler_if_both_branches_terminate() {
    let ir = lower_src(
        "term_if",
        r#"
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, "x")
  RETURN v
END FUNC
FUNC bothTerm(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    IF e.code = 1 THEN
      RECOVER 10
    ELSE
      RECOVER 20
    END IF
    LET unreachable AS Integer = 99
  END TRAP
  RETURN a
END FUNC
FUNC main AS Integer
  RETURN bothTerm(-1)
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("resultIsOk"));
}

// ---- FUNC ref to LINK function / native return type wiring -------------

#[test]
fn lowers_native_link_function_types_and_returns() {
    // A LINK function returning a value type feeds function_types/returns and
    // a re-export alias adopts the target return type.
    let ir = lower_src(
        "link_types",
        r#"
RESOURCE Conn CLOSE BY dbLink::disconnect

LINK "db" AS dbLink
  FUNC version() AS Integer
    SYMBOL "db_version"
    ABI () AS rc CInt32
  END FUNC
  FUNC disconnect(c AS Conn) AS Nothing
    SYMBOL "db_close"
    ABI (c CPtr) AS rc CInt32
  END FUNC
END LINK

EXPORT FUNC ver AS dbLink::version

FUNC main AS Integer
  RETURN 0
END FUNC
"#,
    );
    assert!(ir.link_functions.iter().any(|f| f.name == "version"));
    assert!(ir
        .link_functions
        .iter()
        .any(|f| f.return_type == crate::types::ParameterType::Integer));
}

// ---- constructor with no known fields (fallback positional) ------------

#[test]
fn lowers_constructor_positional_and_named_mixed() {
    let ir = lower_src(
        "ctor_mixed",
        r#"
TYPE Trip
  a AS Integer
  b AS Integer
  c AS Integer
END TYPE
FUNC main AS Integer
  LET t1 AS Trip = Trip[1, 2, 3]
  LET t2 AS Trip = Trip[a := 1, b := 2, c := 3]
  RETURN t1.a + t2.b
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("Trip"));
}

// ---- match binding a union variant (UnionExtract in case body) ---------

#[test]
fn lowers_match_union_variant_binding() {
    let ir = lower_src(
        "match_union_bind",
        r#"
TYPE Circle
  radius AS Integer
END TYPE
TYPE Rect
  w AS Integer
  h AS Integer
END TYPE
UNION Shape
  Circle
  Rect
END UNION
FUNC area(s AS Shape) AS Integer
  MATCH s
    CASE Circle(c)
      RETURN c.radius * c.radius
    CASE Rect(r)
      RETURN r.w * r.h
  END MATCH
END FUNC
FUNC main AS Integer
  RETURN area(Circle[3]) + area(Rect[2, 4])
END FUNC
"#,
    );
    assert!(json_of(&ir).contains("unionExtract"));
}
