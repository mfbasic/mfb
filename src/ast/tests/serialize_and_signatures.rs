//! plan-176-K: `ast::tests` — the serialize_and_signatures tests. The shared `parse_ok`/`parse_err` helpers\n//! and the project/source-root tests live in `ast/tests/mod.rs`.

use super::*;

#[test]
fn serialize_covers_all_item_kinds() {
    let src = r#"
IMPORT io AS term
IMPORT math

EXPORT LET greeting AS String = "hi"
MUT counter AS Integer = 0
RES handle AS File STATE Integer

EXPORT TYPE Point OF T
  PUBLIC x AS T
  y AS Integer
END TYPE

EXPORT UNION Shape INCLUDES Other
  Point
  Circle
END UNION

ENUM Color
  Red, Green, Blue
END ENUM

EXPORT RESOURCE Db CLOSE BY sqlite::close THREAD_SENDABLE

EXPORT FUNC dbClose AS Db::teardown

LINK "sqlite3" AS sqlite
  FUNC open(path AS String) AS RES Db
    SYMBOL "sqlite3_open"
    ABI (path CString, db OUT CPtr) AS ret CInt32
    CONST flags = 1
    SUCCESS_ON ret = 0
    RETURN db
    FREE db
      SYMBOL "sqlite3_free"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
END LINK

DOC INTERNAL
FUNC greet(String)
DESC Greets a person.
WARN Be nice.
DEPRECATED use hello
GROUP greetings
ARG name the name
RET the greeting
ERROR MFB_BAD bad thing
EXAMPLE
  greet("x")
END EXAMPLE
END DOC

DOC
TYPE Point
PROP x the x coord
END DOC
"#;
    let json = project_json(src);
    // Item kinds.
    assert!(json.contains("\"project\": \"demo\""));
    assert!(json.contains("\"module\": \"io\", \"alias\": \"term\""));
    assert!(json.contains("\"module\": \"math\""));
    assert!(json.contains("\"kind\": \"binding\""));
    assert!(json.contains("\"resource\": true, \"state\": \"Integer\""));
    assert!(json.contains("\"kind\": \"type\""));
    assert!(json.contains("\"templateParams\": [\"T\"]"));
    assert!(json.contains("\"kind\": \"union\""));
    assert!(json.contains("\"includes\": [\"Other\"]"));
    assert!(json.contains("\"kind\": \"enum\""));
    assert!(json.contains("\"kind\": \"resource\""));
    assert!(json.contains("\"threadSendable\": true"));
    assert!(json.contains("\"kind\": \"funcAlias\""));
    assert!(json.contains("\"target\": \"Db.teardown\""));
    assert!(json.contains("\"kind\": \"link\""));
    assert!(json.contains("\"kind\": \"linkFunc\""));
    assert!(json.contains("\"returnResource\": true"));
    assert!(json.contains("\"kind\": \"doc\""));
    // Doc details.
    assert!(json.contains("\"header\": \"FUNC\""));
    assert!(json.contains("\"signature\": [\"String\"]"));
    assert!(json.contains("\"attrs\": [\"INTERNAL\"]"));
    assert!(json.contains("\"kind\": \"warn\""));
    assert!(json.contains("\"deprecated\": [\"use hello\"]"));
    assert!(json.contains("\"group\": [\"greetings\"]"));
    assert!(json.contains("\"code\": \"MFB_BAD\""));
    assert!(json.contains("\"example\": [\"greet(\\\"x\\\")\"]"));
    // The PACKAGE-visibility field prefix and OUT ABI slot.
    assert!(json.contains("\"visibility\": \"public\""));
    assert!(json.contains("\"out\": true"));
}

#[test]
fn serialize_covers_all_statement_and_expression_kinds() {
    let src = r#"
ISOLATED FUNC compute(a AS Integer, b AS Integer = 2) AS RES Db STATE Integer
  LET total AS Integer = a + b
  notify()
  MUT items AS List OF Integer = [1, 2, 3]
  RES conn AS Db STATE Integer = open()
  total = total - 1
  conn.state = 5
  conn.state.field = 9
  IF total > 0 THEN
    total = total + 1
  ELSEIF total < 0 THEN
    total = 0
  ELSE
    total = 2
  END IF
  IF a = 1 THEN RETURN a ELSE RETURN b
  MATCH total
    CASE 0
      total = 1
    CASE 1, 2 WHEN a > 0
      total = 2
    CASE Shape(s)
      total = 3
    CASE ELSE
      total = 4
  END MATCH
  FOR i = 1 TO 10 STEP 2
    total = total + i
  NEXT
  FOR EACH item IN items
    total = total + item
  NEXT
  WHILE total < 100
    total = total + 1
  END WHILE
  DO
    total = total + 1
  LOOP UNTIL total > 200
  DO WHILE total < 300
    total = total + 1
  LOOP
  LET pt AS Point = Point[x := 1, 2]
  LET updated AS Point = WITH pt { x := 9 }
  LET m AS Map OF String TO Integer = Map OF String TO Integer { "a" := 1 }
  LET member AS Integer = pt.x
  LET neg AS Integer = -total
  LET flag AS Boolean = TRUE
  LET flag2 AS Boolean = FALSE
  LET truth AS Boolean = NOT (a = b)
  LET both AS Boolean = a > 0 AND b > 0 OR a < 0
  LET cat AS String = "a" & "b"
  LET pw AS Integer = a ^ b
  LET md AS Integer = a MOD b
  LET cb AS Integer = greet(name := "x", 5)
  LET fn2 AS Integer = piped |> add(_, 1)
  LET lam AS Integer = LAMBDA(x AS Integer) -> x + 1
  LET lam2 AS Integer = LAMBDA(x AS Integer) -> total = total + x
  LET trapped AS Integer = risky() TRAP(err)
    total = 0
  END TRAP
  EXIT FOR
  CONTINUE FOR
  FAIL "boom"
  RECOVER 0
  PROPAGATE
  EXIT PROGRAM 1
  RETURN conn
  TRAP(err)
    RETURN open()
  END TRAP
END FUNC
"#;
    let json = project_json(src);
    // Statement kinds.
    for kind in [
        "\"kind\": \"binding\"",
        "\"kind\": \"assignment\"",
        "\"kind\": \"stateAssignment\"",
        "\"kind\": \"if\"",
        "\"kind\": \"match\"",
        "\"kind\": \"for\"",
        "\"kind\": \"forEach\"",
        "\"kind\": \"while\"",
        "\"kind\": \"doUntil\"",
        "\"kind\": \"exit\"",
        "\"kind\": \"continue\"",
        "\"kind\": \"fail\"",
        "\"kind\": \"recover\"",
        "\"kind\": \"propagate\"",
        "\"kind\": \"return\"",
        "\"kind\": \"expression\"",
    ] {
        assert!(json.contains(kind), "missing statement kind {kind}");
    }
    // Match patterns + guard.
    assert!(json.contains("\"kind\": \"else\""));
    assert!(json.contains("\"kind\": \"literal\""));
    assert!(json.contains("\"kind\": \"oneOf\""));
    assert!(json.contains("\"kind\": \"union\", \"type\": \"Shape\", \"binding\": \"s\""));
    assert!(json.contains("\"guard\":"));
    // Expression kinds.
    for kind in [
        "\"kind\": \"binary\"",
        "\"kind\": \"unary\"",
        "\"kind\": \"call\"",
        "\"kind\": \"lambda\"",
        "\"kind\": \"constructor\"",
        "\"kind\": \"with\"",
        "\"kind\": \"list\"",
        "\"kind\": \"map\"",
        "\"kind\": \"memberAccess\"",
        "\"kind\": \"trapped\"",
        "\"kind\": \"identifier\"",
        "\"kind\": \"string\"",
        "\"kind\": \"number\"",
        "\"kind\": \"boolean\"",
        "\"kind\": \"named\"",
    ] {
        assert!(json.contains(kind), "missing expression kind {kind}");
    }
    // Function metadata: isolated + return resource/state + trap + default param.
    assert!(json.contains("\"kind\": \"func\""));
    assert!(json.contains("\"returnResource\": true, \"returnState\": \"Integer\""));
    assert!(json.contains("\"trap\":"));
    assert!(json.contains("\"assignTarget\": \"total\""));
    assert!(json.contains("\"loop\": \"for\""));
    assert!(json.contains("\"target\": \"program\""));
}

#[test]
fn function_signature_line_renders_visibility_isolated_and_params() {
    let file = parse_file(
        "EXPORT ISOLATED FUNC f(x AS Integer, h AS File) AS String\n  RETURN \"\"\nEND FUNC\n",
    );
    let f = function(&file, "f");
    assert_eq!(
        f.signature_line(),
        "EXPORT ISOLATED FUNC f(x AS Integer, h AS File) AS String"
    );

    // An `AS Nothing` return clause is kept verbatim; a PACKAGE bare param has no `AS`.
    let file = parse_file("PRIVATE FUNC g(a) AS Nothing\nEND FUNC\n");
    let g = function(&file, "g");
    assert_eq!(g.signature_line(), "PRIVATE FUNC g(a) AS Nothing");

    // A private FUNC with no explicit return type defaults to Nothing.
    let file = parse_file("FUNC h()\nEND FUNC\n");
    let h = function(&file, "h");
    assert_eq!(h.signature_line(), "FUNC h() AS Nothing");
}

#[test]
fn type_decl_signature_line_covers_all_kinds_and_visibility() {
    let file = parse_file("EXPORT TYPE T\n  x AS Integer\nEND TYPE\n");
    let Item::Type(decl) = &file.items[0] else {
        panic!("type");
    };
    assert_eq!(decl.signature_line(), "EXPORT TYPE T");

    let file = parse_file("PRIVATE UNION U\n  T\nEND UNION\n");
    let Item::Type(decl) = &file.items[0] else {
        panic!("union");
    };
    assert_eq!(decl.signature_line(), "PRIVATE UNION U");

    let file = parse_file("ENUM E\n  A, B\nEND ENUM\n");
    let Item::Type(decl) = &file.items[0] else {
        panic!("enum");
    };
    assert_eq!(decl.signature_line(), "ENUM E");
}

#[test]
fn pipeline_placeholder_substitutes_across_all_expression_forms() {
    // A pipeline `x |> rhs` requires `_` somewhere in rhs; the parser rewrites it
    // via substitute_placeholder, walking every expression form.
    let file = parse_file(
        "FUNC main AS Integer\n  RETURN base |> wrap(-_, [_, 1], Point[_], WITH _ { f := _ }, m.field, _ + 1)\nEND FUNC\n",
    );
    let f = function(&file, "main");
    let Statement::Return {
        value: Some(expr), ..
    } = &f.body[0]
    else {
        panic!("return");
    };
    // The rewritten call has `base` substituted in for each `_`.
    assert!(matches!(expr, Expression::Call { .. }));
    let json = project_json(
        "FUNC main AS Integer\n  RETURN base |> wrap(-_, [_, 1], Point[_], WITH _ { f := _ }, m.field, _ + 1)\nEND FUNC\n",
    );
    assert!(json.contains("\"kind\": \"call\""));
    assert!(json.contains("\"value\": \"base\""));
    // No literal placeholder identifier survives.
    assert!(!json.contains("\"value\": \"_\""));
}

#[test]
fn pipeline_placeholder_missing_is_rejected() {
    assert!(try_parse("FUNC main AS Integer\n  RETURN x |> add(1)\nEND FUNC\n").is_err());
}

#[test]
fn pipeline_placeholder_in_lambda_and_map_and_boolean() {
    // Exercise the Lambda/MapLiteral/Boolean/String/Number arms of the
    // placeholder walk (both contains_placeholder and substitute_placeholder).
    let file = parse_file(
        "FUNC main AS Integer\n  RETURN seed |> pick(LAMBDA(x AS Integer) -> _, Map OF String TO Integer { \"k\" := _ }, TRUE, \"s\", 3)\nEND FUNC\n",
    );
    let f = function(&file, "main");
    assert!(matches!(&f.body[0], Statement::Return { .. }));
}

#[test]
fn serialize_covers_none_and_default_branches() {
    let src = r#"
LET plain = 1
RES bare AS File
FUNC doer(a, h AS File)
  LET x = 1
  MUT y
  RETURN
  RECOVER
  FOR i = 1 TO 3
    y = i
    EXIT FOR
  NEXT
  MATCH y
    CASE ELSE
      y = 0
  END MATCH
END FUNC

LINK "lib" AS l
  FUNC noRet(a AS Integer)
    SYMBOL "sym"
    ABI (a CInt32) AS r CInt32
  END FUNC
END LINK
"#;
    let json = project_json(src);
    // A binding/param/return with no type serializes `null`.
    assert!(json.contains("\"type\": null"));
    // A RES declaration with no STATE serializes state null.
    assert!(json.contains("\"resource\": true, \"state\": null"));
    // A FUNC with no return type.
    assert!(json.contains("\"kind\": \"func\""));
    assert!(json.contains("\"returnType\": null"));
    // RETURN / RECOVER with no value.
    assert!(json.contains("\"kind\": \"return\", \"value\": null"));
    assert!(json.contains("\"kind\": \"recover\", \"value\": null"));
    // EXIT FOR (no code).
    assert!(json.contains("\"kind\": \"exit\", \"target\": \"for\", \"code\": null"));
    // FOR without STEP.
    assert!(json.contains("\"step\": null"));
    // MATCH else.
    assert!(json.contains("\"kind\": \"else\""));
    // A LINK func with no successOn / result / free.
    assert!(json.contains("\"successOn\": null"));
    assert!(json.contains("\"result\": null"));
    assert!(json.contains("\"free\": null"));
    // A LINK func whose MFBASIC return is absent renders returnType null.
    assert!(json.contains("\"returnType\": null"));
}

#[test]
fn serialize_exit_targets_func_and_do_and_while() {
    let src = r#"
FUNC f AS Integer
  DO
    EXIT DO
    EXIT WHILE
    EXIT FUNC
    CONTINUE DO
    CONTINUE WHILE
  LOOP UNTIL TRUE
  RETURN 0
END FUNC
"#;
    let json = project_json(src);
    assert!(json.contains("\"target\": \"do\""));
    assert!(json.contains("\"target\": \"while\""));
    assert!(json.contains("\"target\": \"func\""));
    assert!(json.contains("\"loop\": \"do\""));
    assert!(json.contains("\"loop\": \"while\""));
}

#[test]
fn serialize_lambda_without_assign_target() {
    let json = project_json(
        "FUNC f AS Integer\n  LET g AS Integer = LAMBDA(x AS Integer) -> x + 1\n  RETURN 0\nEND FUNC\n",
    );
    // The plain-body lambda arm (no assignTarget key).
    assert!(json.contains("\"kind\": \"lambda\", \"params\":"));
    assert!(!json.contains("\"assignTarget\""));
}

#[test]
fn pipeline_placeholder_walks_named_args_and_trapped_and_with() {
    // Named call/constructor args + a WITH update + member access all recurse
    // through both contains_placeholder and substitute_placeholder.
    let json = project_json(
        "FUNC main AS Integer\n  RETURN seed |> build(k := _, Point[y := _], WITH base { z := _ })\nEND FUNC\n",
    );
    assert!(json.contains("\"value\": \"seed\""));
    assert!(!json.contains("\"value\": \"_\""));

    // A named argument that does NOT hold the placeholder still parses (the
    // false branch of the named-arg walk), with `_` supplied elsewhere.
    let json =
        project_json("FUNC main AS Integer\n  RETURN seed |> build(k := 1, v := _)\nEND FUNC\n");
    assert!(json.contains("\"value\": \"seed\""));
}

#[test]
fn all_comparison_and_arithmetic_operators_serialize() {
    let src = r#"
FUNC f AS Boolean
  LET a = x <= y
  LET b = x >= y
  LET c = x <> y
  LET d = x < y
  LET e = x > y
  LET g = x = y
  LET h = x * y / z
  LET i = -x
  RETURN a
END FUNC
"#;
    let json = project_json(src);
    for op in ["<=", ">=", "<>", "<", ">", "=", "*", "/"] {
        assert!(
            json.contains(&format!("\"operator\": \"{op}\"")),
            "missing operator {op}"
        );
    }
}
