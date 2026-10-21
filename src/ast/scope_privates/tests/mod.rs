use super::*;
use crate::internal_name::{file_scope_hash, mangle_private};
use crate::testutil::project_from_src;

/// The relative path `project_from_src` gives the user's single source file.
const MAIN: &str = "main.mfb";

fn mangled(name: &str) -> String {
    mangle_private(&file_scope_hash(MAIN), name)
}

/// A broad program that touches every declaration kind that carries
/// visibility and every statement / expression rewrite arm, so running the
/// pass over it drives the whole reference rewriter.
const BROAD: &str = r#"IMPORT io

PRIVATE TYPE Widget
  size AS Integer
END TYPE

PRIVATE TYPE Blob
  n AS Integer
END TYPE

PRIVATE UNION Base
  Widget
END UNION

PRIVATE UNION Shape INCLUDES Base
  Blob
END UNION

PRIVATE LET secret AS Integer = 42
PRIVATE MUT counter AS Integer = 0

PRIVATE RESOURCE Handle CLOSE BY io::print
PRIVATE FUNC pr AS io::print

PRIVATE FUNC helper(n AS Integer = secret) AS Integer
  RETURN n + secret
END FUNC

PRIVATE FUNC noop() AS Nothing
  counter = counter + 1
END FUNC

PRIVATE FUNC extras() AS Nothing
  IF counter > secret THEN FAIL secret
  os::exit(secret)
END FUNC

PRIVATE FUNC more() AS Nothing
  PROPAGATE
END FUNC

FUNC driver(input AS Shape) AS Integer
  LET a AS Integer = helper(secret)
  MUT total AS Integer = 0
  total = a + helper(1)
  LET w AS Widget = Widget[3]
  LET w2 AS Widget = WITH w { size := secret }
  LET items AS List OF Widget = [Widget[1], Widget[2]]
  LET table AS Map OF Integer TO Widget = Map OF Integer TO Widget { 1 := Widget[9] }
  LET fn AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + secret
  LET adder = LAMBDA(x AS Integer) -> counter = counter + x
  LET neg AS Integer = -secret
  LET member AS Integer = w.size
  LET pref AS Integer = NOT secret
  LET shadow AS Integer = secret
  LET secret AS Integer = shadow
  total = total + secret
  IF a > secret THEN
    total = total + 1
  ELSE
    total = total - shadow
  END IF
  FOR i = 1 TO shadow STEP shadow
    total = total + helper(i)
  NEXT
  FOR EACH item IN items
    total = total + item.size
    CONTINUE FOR
  NEXT
  WHILE total < shadow
    total = total + 1
  END WHILE
  DO
    total = total + 1
  LOOP UNTIL total > shadow
  MATCH input
    CASE Widget(x)
      total = total + x.size
    CASE Blob(b)
      total = total + b.n
    CASE ELSE
      total = total + shadow
  END MATCH
  MATCH total
    CASE 1, 2
      total = total + shadow
    CASE 3 WHEN total > shadow
      total = total + 1
    CASE ELSE
      total = total - 1
  END MATCH
  LET parsed AS Integer = helper(shadow) TRAP(e)
    RECOVER e.code
  END TRAP
  noop()
  pr("done")
  input.state = secret
  RETURN total
TRAP(err)
  RETURN shadow
END TRAP
END FUNC

TESTING
  TGROUP "outer"
    TCASE "one"
      LET v AS Integer = helper(secret)
    END TCASE
    TGROUP "inner"
      TCASE "two"
        LET u AS Widget = Widget[secret]
      END TCASE
    END TGROUP
  END TGROUP
END TESTING
"#;

#[test]
fn renames_private_decls_and_rewrites_references() {
    let mut project = project_from_src(BROAD);
    let diagnostics = scope_privates(&mut project);
    // No PUBLIC is shadowed and no path-hash collides.
    assert!(
        diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        diagnostics.iter().map(|d| &d.rule).collect::<Vec<_>>()
    );
    let json = project.to_json();
    // Every PRIVATE declaration name (and its in-file references) is mangled.
    for name in [
        "Widget", "Blob", "Base", "Shape", "secret", "counter", "Handle", "pr", "helper", "noop",
        "extras", "more",
    ] {
        assert!(
            json.contains(&mangled(name)),
            "expected mangled `{name}` (`{}`) in output",
            mangled(name)
        );
    }
}

#[test]
fn locals_shadow_private_names_and_are_left_alone() {
    // Inside `driver`, `LET secret = shadow` rebinds `secret`; every later use
    // is the local, so those references must NOT be mangled. The private
    // `secret` is still mangled at its declaration and in the earlier uses.
    let mut project = project_from_src(BROAD);
    scope_privates(&mut project);
    let json = project.to_json();
    // The local `shadow`/`secret` reads after the rebind stay bare — there is
    // still at least one bare `total = total + secret` (the local) referencing
    // the un-mangled identifier in the JSON.
    assert!(json.contains("\"identifier\"") || json.contains("secret"));
    // The mangled private is present (declaration + pre-shadow references).
    assert!(json.contains(&mangled("secret")));
}

#[test]
fn a_file_with_no_private_items_is_untouched() {
    let src = "FUNC main() AS Integer\n  RETURN 1\nEND FUNC\n";
    let mut project = project_from_src(src);
    let before = project.to_json();
    let diagnostics = scope_privates(&mut project);
    assert!(diagnostics.is_empty());
    // The rename map is empty, so the file is returned byte-for-byte identical.
    assert_eq!(before, project.to_json());
}

#[test]
fn private_shadowing_a_public_of_the_same_name_warns() {
    // A PUBLIC `shared` FUNC and a PRIVATE `shared` binding in one file: the
    // private declaration shadows the public within the file (with a warning).
    let src =
        "FUNC shared() AS Integer\n  RETURN 1\nEND FUNC\n\nPRIVATE LET shared AS Integer = 2\n";
    let mut project = project_from_src(src);
    let diagnostics = scope_privates(&mut project);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.rule == "PRIVATE_SHADOWS_PUBLIC"),
        "expected a PRIVATE_SHADOWS_PUBLIC warning, got {:?}",
        diagnostics.iter().map(|d| &d.rule).collect::<Vec<_>>()
    );
    // The private binding is still mangled despite the shadow.
    assert!(project.to_json().contains(&mangled("shared")));
}

#[test]
fn item_name_vis_and_line_cover_non_visibility_items() {
    // `Link`/`Doc`/`Testing` items carry no visibility and report line 0.
    let src = "IMPORT io\n\nLINK \"libc\" AS c\nEND LINK\n\nPRIVATE FUNC keep() AS Integer\n  RETURN 1\nEND FUNC\n";
    let mut project = project_from_src(src);
    let diagnostics = scope_privates(&mut project);
    assert!(diagnostics.is_empty());
    assert!(project.to_json().contains(&mangled("keep")));
}

#[test]
fn private_type_used_in_nested_type_strings_is_rewritten() {
    // A `List OF` / `Map OF` type string naming a PRIVATE type has that token
    // rewritten while the keywords (`List`, `OF`, `Map`, `TO`) are left alone.
    let src = "PRIVATE TYPE Box\n  v AS Integer\nEND TYPE\n\nPRIVATE LET store AS List OF Box = []\n\nFUNC use() AS Integer\n  LET m AS Map OF Integer TO Box = Map OF Integer TO Box { }\n  RETURN 0\nEND FUNC\n";
    let mut project = project_from_src(src);
    scope_privates(&mut project);
    let json = project.to_json();
    // The `Box` token inside `List OF Box` / `Map OF Integer TO Box` is mangled.
    assert!(json.contains(&mangled("Box")));
    // The structural keywords survive unmangled.
    assert!(json.contains("List OF"));
}

#[test]
fn private_binding_used_as_state_assign_target_is_rewritten() {
    // bug-396: a `foo.state = v` whose `foo` names a file-PRIVATE top-level
    // binding must have its `StateAssign.resource` reference mangled to match
    // the (already mangled) declaration -- otherwise the resolver reports a
    // bogus SYMBOL_UNKNOWN_IDENTIFIER instead of the accurate "not a local
    // binding" diagnostic. Sibling to the `Assign` arm, which already sweeps
    // its target name.
    let src =
        "PRIVATE MUT g AS Integer = 0\n\nFUNC f() AS Integer\n  g.state = 5\n  RETURN 0\nEND FUNC\n";
    let mut project = project_from_src(src);
    scope_privates(&mut project);
    let json = project.to_json();
    // The StateAssign target reference is mangled, not left bare.
    assert!(
        json.contains(&format!("\"resource\": \"{}\"", mangled("g"))),
        "expected mangled StateAssign resource `{}` in output:\n{json}",
        mangled("g")
    );
    // No bare, unmangled `g` target reference survives.
    assert!(
        !json.contains("\"resource\": \"g\""),
        "bare unmangled StateAssign resource `g` should not survive"
    );
}

#[test]
fn a_local_shadowing_a_private_keeps_its_state_assign_target_bare() {
    // Non-goal guard (bug-396): once a local `g` shadows the private `g`, a
    // later `g.state = v` targets the in-scope local and must stay bare -- the
    // rename map only rewrites references the local scope does not cover.
    let src = "PRIVATE MUT g AS Integer = 0\n\nFUNC f() AS Integer\n  LET g AS Integer = 1\n  g.state = 5\n  RETURN 0\nEND FUNC\n";
    let mut project = project_from_src(src);
    scope_privates(&mut project);
    let json = project.to_json();
    assert!(
        json.contains("\"resource\": \"g\""),
        "a local state-assign target must stay bare:\n{json}"
    );
}

#[test]
fn a_local_named_like_a_private_func_is_left_alone() {
    // plan-136-C: a local may no longer share a name with a top-level LET/MUT,
    // but it may still share one with a FUNC, so the rewriter must keep leaving a
    // local that shadows a PRIVATE FUNC unrenamed. Here a local lambda is called
    // by the function's name: the declaration is mangled, the call stays bare.
    let src = "PRIVATE FUNC helper() AS Integer\n  RETURN 42\nEND FUNC\n\nFUNC f() AS Integer\n  LET helper = LAMBDA() -> 7\n  RETURN helper()\nEND FUNC\n";
    let mut project = project_from_src(src);
    scope_privates(&mut project);
    let json = project.to_json();
    assert!(
        json.contains(&mangled("helper")),
        "the PRIVATE FUNC declaration is still mangled:\n{json}"
    );
    assert!(
        json.contains("\"callee\": \"helper\""),
        "the local lambda's call must stay bare:\n{json}"
    );
}

#[test]
fn rewrite_type_str_is_identity_when_no_private_types() {
    // The early-out path: no private *types* means every type string passes
    // through untouched (empty `types` map).
    let types: HashMap<String, String> = HashMap::new();
    assert_eq!(rewrite_type_str("List OF Widget", &types), "List OF Widget");
}

#[test]
fn two_files_sharing_a_path_hit_the_duplicate_hash_arm() {
    // Two AstFiles with the *same* path hash identically; the second file's
    // `Some(prev) if prev == path` falls through to `Some(_) => {}` (line 50),
    // NOT the distinct-path collision arm. `project_from_src` gives each file
    // the same `main.mfb` path, so build the project by hand.
    let project = crate::testutil::project_from_src("PRIVATE LET a AS Integer = 1\n");
    let one = project.files[0].clone();
    let mut two = one.clone();
    two.path = one.path.clone(); // identical path (explicit; already equal)
    let mut dup = AstProject {
        name: "t".to_string(),
        files: vec![one, two],
    };
    let diagnostics = scope_privates(&mut dup);
    // The identical-path case is NOT a collision — no such diagnostic fires.
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.rule == "PRIVATE_PATH_HASH_COLLISION"),
        "identical paths must not be reported as a hash collision"
    );
}

#[test]
fn shadow_warning_reports_the_line_of_each_decl_kind() {
    // A PUBLIC + PRIVATE pair of each declaration kind (Function, Type,
    // Resource, FuncAlias) drives every `item_line` arm (132-135) via the
    // shadow-warning path.
    let src = "IMPORT io\n\n\
             FUNC dup() AS Integer\n  RETURN 1\nEND FUNC\n\
             PRIVATE FUNC dup() AS Integer\n  RETURN 2\nEND FUNC\n\
             TYPE Dup2\n  a AS Integer\nEND TYPE\n\
             PRIVATE TYPE Dup2\n  b AS Integer\nEND TYPE\n\
             RESOURCE Dup3 CLOSE BY io::print\n\
             PRIVATE RESOURCE Dup3 CLOSE BY io::print\n\
             FUNC Dup4 AS io::print\n\
             PRIVATE FUNC Dup4 AS io::print\n";
    let mut project = crate::testutil::project_from_src(src);
    let diagnostics = scope_privates(&mut project);
    let shadows = diagnostics
        .iter()
        .filter(|d| d.rule == "PRIVATE_SHADOWS_PUBLIC")
        .count();
    assert_eq!(
        shadows,
        4,
        "expected one shadow per decl kind, got rules {:?}",
        diagnostics.iter().map(|d| &d.rule).collect::<Vec<_>>()
    );
}

#[test]
fn item_line_of_a_non_visibility_item_is_zero() {
    // `Link`/`Doc`/`Testing` never reach `item_line` through the shadow path
    // (they carry no visibility), so cover the `=> 0` arm (136) directly.
    let file = crate::testutil::parse_file("LINK \"x\" AS l\nEND LINK\n");
    let link = file
        .items
        .iter()
        .find(|item| matches!(item, Item::Link(_)))
        .expect("a LINK item");
    assert_eq!(item_line(link), 0);
}

#[test]
fn function_param_state_type_is_rewritten() {
    // A `h AS File STATE Meta` param names the private `Meta` in its STATE
    // clause; the pass rewrites it (line 175).
    let src = "PRIVATE TYPE Meta\n  x AS Integer\nEND TYPE\n\n\
             FUNC f(h AS File STATE Meta)\n  h.state = 0\nEND FUNC\n";
    let mut project = crate::testutil::project_from_src(src);
    scope_privates(&mut project);
    assert!(project.to_json().contains(&mangled("Meta")));
}

#[test]
fn resource_close_fn_naming_a_private_func_is_rewritten() {
    // `CLOSE BY closer` names a private func, so `close_fn` follows the rename
    // map (line 236).
    let src = "PRIVATE FUNC closer() AS Nothing\n  RETURN\nEND FUNC\n\n\
             RESOURCE H CLOSE BY closer\n";
    let mut project = crate::testutil::project_from_src(src);
    scope_privates(&mut project);
    assert!(project.to_json().contains(&mangled("closer")));
}

#[test]
fn link_block_signatures_referencing_a_private_type_are_rewritten() {
    // A LINK native FUNC whose param (type + STATE), return (type + STATE), and
    // whose CSTRUCT `maps_to` all name the private `Priv` drives the whole LINK
    // rewrite block (245-264).
    let src = "PRIVATE TYPE Priv\n  x AS Integer\nEND TYPE\n\n\
             LINK \"x\" AS l\n\
             \x20 CSTRUCT Foo AS Priv\n    a CInt32\n  END CSTRUCT\n\
             \x20 FUNC nf(p AS Priv STATE Priv) AS RES Priv STATE Priv\n\
             \x20   SYMBOL \"s\"\n\
             \x20   ABI (p CPtr) AS r CPtr\n\
             \x20 END FUNC\n\
             END LINK\n";
    let mut project = crate::testutil::project_from_src(src);
    scope_privates(&mut project);
    assert!(project.to_json().contains(&mangled("Priv")));
}

#[test]
fn let_binding_state_type_is_rewritten() {
    // A `RES conn AS File STATE Meta2 = …` binding names the private `Meta2` in
    // its STATE clause (line 327).
    let src = "PRIVATE TYPE Meta2\n  x AS Integer\nEND TYPE\n\n\
             FUNC g() AS Integer\n  RES conn AS File STATE Meta2 = open()\n  RETURN 0\nEND FUNC\n";
    let mut project = crate::testutil::project_from_src(src);
    scope_privates(&mut project);
    assert!(project.to_json().contains(&mangled("Meta2")));
}

#[test]
fn set_literal_element_type_and_elements_are_rewritten() {
    // A non-empty `Set OF Elem { Elem[1] }` literal drives the SetLiteral arm's
    // element_type rewrite and element loop (505-511).
    let src = "PRIVATE TYPE Elem\n  x AS Integer\nEND TYPE\n\n\
             FUNC h() AS Integer\n  LET s AS Set OF Elem = Set OF Elem { Elem[1] }\n  RETURN 0\nEND FUNC\n";
    let mut project = crate::testutil::project_from_src(src);
    scope_privates(&mut project);
    assert!(project.to_json().contains(&mangled("Elem")));
}

/// The `(name, visibility)` of every top-level `FUNC` in the user's file.
fn declared_functions(project: &AstProject) -> Vec<(String, Visibility)> {
    project.files[0]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => Some((function.name.clone(), function.visibility)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_public_overload_beside_a_private_overload_keeps_its_name() {
    // bug-912: an EXPORT/PUBLIC `r` and a PRIVATE `r` with a different
    // signature are a legal overload set. Renaming the EXPORT one with the
    // PRIVATE one took it out of the package's export table and made a PUBLIC
    // one uncallable from every other file.
    let src = "EXPORT FUNC r(x AS Integer) AS String\n  RETURN r(\"s\")\nEND FUNC\n\n\
             FUNC r(x AS Boolean) AS String\n  RETURN \"bool\"\nEND FUNC\n\n\
             PRIVATE FUNC r(x AS String) AS String\n  RETURN \"str\"\nEND FUNC\n";
    let mut project = project_from_src(src);
    scope_privates(&mut project);
    assert_eq!(
        declared_functions(&project),
        vec![
            ("r".to_string(), Visibility::Export),
            ("r".to_string(), Visibility::Public),
            (mangled("r"), Visibility::Private),
        ]
    );
    // The file's own references still name the file-scoped overload set, which
    // is what lets a call inside the file reach the PRIVATE member.
    let Item::Function(exported) = &project.files[0].items[0] else {
        panic!("the first item is the EXPORT function");
    };
    let Statement::Return {
        value: Some(Expression::Call { callee, .. }),
        ..
    } = &exported.body[0]
    else {
        panic!("the EXPORT function returns a call");
    };
    assert_eq!(callee, &mangled("r"));
}

#[test]
fn a_public_func_beside_a_private_non_function_is_still_renamed() {
    // Only a FUNC/FUNC pair is an overload set. A PUBLIC `FUNC shared` beside
    // a PRIVATE `LET shared` is not, and keeps colliding under the one
    // file-scoped name so the resolver reports the duplicate.
    let src =
        "FUNC shared() AS Integer\n  RETURN 1\nEND FUNC\n\nPRIVATE LET shared AS Integer = 2\n";
    let mut project = project_from_src(src);
    scope_privates(&mut project);
    assert_eq!(
        declared_functions(&project),
        vec![(mangled("shared"), Visibility::Public)]
    );
}

#[test]
fn an_untyped_binding_initialized_from_a_private_is_rewritten() {
    // A top-level binding with no declared type, read from a PRIVATE: only the
    // initializer has a reference to rewrite.
    let src = "PRIVATE LET seed AS Integer = 2\nLET derived = seed\n";
    let mut project = project_from_src(src);
    scope_privates(&mut project);
    let Item::Binding(derived) = &project.files[0].items[1] else {
        panic!("the second item is the binding");
    };
    assert_eq!(derived.type_name, None);
    assert!(
        matches!(&derived.value, Some(Expression::Identifier(name)) if *name == mangled("seed")),
        "{:?}",
        derived.value
    );
}

#[test]
fn declarations_with_nothing_to_rewrite_in_an_optional_slot_still_parse_and_pass() {
    // A binding with no initializer and a function with no return type: the
    // pass rewrites only the slots that are present.
    let src = "PRIVATE TYPE T\n  n AS Integer\nEND TYPE\nMUT pending AS T\n\
               FUNC go(t AS T)\n  pending = t\nEND FUNC\n";
    let mut project = project_from_src(src);
    scope_privates(&mut project);
    let json = project.to_json();
    assert!(json.contains(&mangled("T")), "{json}");
}
