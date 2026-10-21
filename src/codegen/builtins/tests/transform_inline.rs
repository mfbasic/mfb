//! `collections::transform` evaluates a record-update callback itself, or calls it.
//!
//! bug-766 E: a callback whose whole body is `RETURN param` or
//! `RETURN WITH param { f := e, ... }` — every `f` a scalar field and every `e` a
//! scalar expression over `param`'s scalar fields, constants, globals and by-value
//! captures — is evaluated inside the `transform` loop, so a record update costs no
//! per-element allocation. Anything else is CALLED, element by element.
//!
//! The two shapes differ in exactly one emitted thing: the call path ends in a
//! `transform_call_ok` label (the success branch after the callback returns) and the
//! inline path never emits one. Whether a callback is inlined is therefore a count of
//! labels, and the table below states, case by case, which side of the line each
//! callback falls on — including every refusal of `inline_record_callback` and of
//! the expression rewrite, because a refusal that accepted a callback it must not
//! evaluate inline would skip a call that can fail, allocate, or read a `String`.

use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

/// `(label, declarations, the callback expression, evaluated inline)`.
type Case = (&'static str, &'static str, &'static str, bool);

const CASES: &[Case] = &[
    ("identity lambda", "", "LAMBDA(it AS Item) -> it", true),
    (
        "scalar updates with arithmetic and NOT",
        "",
        "LAMBDA(it AS Item) -> WITH it { score := it.score * 2.0, ok := NOT it.ok }",
        true,
    ),
    (
        "declared FUNC",
        "FUNC bump(it AS Item) AS Item\n  RETURN WITH it { n := it.n + 1 }\nEND FUNC\n",
        "bump",
        true,
    ),
    (
        "declared FUNC returning its parameter",
        "FUNC same(it AS Item) AS Item\n  RETURN it\nEND FUNC\n",
        "same",
        true,
    ),
    (
        "by-value capture",
        "",
        "LAMBDA(it AS Item) -> WITH it { n := it.n + k }",
        true,
    ),
    (
        "module-level constant",
        "LET bonus AS Integer = 5\n",
        "LAMBDA(it AS Item) -> WITH it { n := it.n + bonus }",
        true,
    ),
    (
        "unary minus",
        "",
        "LAMBDA(it AS Item) -> WITH it { n := -it.n }",
        true,
    ),
    (
        "two fields at once",
        "",
        "LAMBDA(it AS Item) -> WITH it { n := it.n + k, score := it.score + 1.0 }",
        true,
    ),
    (
        "a String field is not a scalar slot",
        "",
        "LAMBDA(it AS Item) -> WITH it { label := \"y\" }",
        false,
    ),
    (
        "String concatenation allocates",
        "",
        "LAMBDA(it AS Item) -> WITH it { label := it.label & \"z\" }",
        false,
    ),
    (
        "a builtin call is not evaluated inline",
        "",
        "LAMBDA(it AS Item) -> WITH it { n := len(it.label) }",
        false,
    ),
    (
        "a field of some other record",
        "",
        "LAMBDA(it AS Item) -> WITH it { n := other.n }",
        false,
    ),
    (
        "a body that is not the parameter",
        "",
        "LAMBDA(it AS Item) -> other",
        false,
    ),
    (
        "a body that builds a fresh record",
        "FUNC fresh(it AS Item) AS Item\n  RETURN Item[9, 1.0, TRUE, \"x\"]\nEND FUNC\n",
        "fresh",
        false,
    ),
    (
        "a prologue statement before the return",
        "FUNC twoSteps(it AS Item) AS Item\n  LET t = it.n + 1\n  RETURN WITH it { n := t }\nEND FUNC\n",
        "twoSteps",
        false,
    ),
    (
        "a body that does not end in a single return",
        "FUNC branching(it AS Item) AS Item\n  IF it.ok THEN\n    RETURN it\n  END IF\n  RETURN WITH it { n := 0 }\nEND FUNC\n",
        "branching",
        false,
    ),
    (
        "a body ending in an IF/ELSE rather than a return",
        "FUNC choose(it AS Item) AS Item\n  IF it.ok THEN\n    RETURN it\n  ELSE\n    RETURN WITH it { n := 0 }\n  END IF\nEND FUNC\n",
        "choose",
        false,
    ),
    (
        "an ISOLATED FUNC is called",
        "ISOLATED FUNC iso(it AS Item) AS Item\n  RETURN WITH it { n := it.n + 1 }\nEND FUNC\n",
        "iso",
        false,
    ),
    (
        "a callback that changes the element type",
        "FUNC toLen(it AS Item) AS Integer\n  RETURN len(it.label)\nEND FUNC\n",
        "toLen",
        false,
    ),
];

fn program(declarations: &str, callback: &str) -> String {
    format!(
        "IMPORT io\n\
         IMPORT collections\n\
         \n\
         TYPE Item\n  n AS Integer\n  score AS Float\n  ok AS Boolean\n  label AS String\nEND TYPE\n\
         \n\
         {declarations}\n\
         FUNC main() AS Integer\n\
         \x20 LET xs AS List OF Item = [Item[1, 1.5, TRUE, \"a\"], Item[2, 2.5, FALSE, \"b\"]]\n\
         \x20 LET other = Item[7, 0.5, TRUE, \"o\"]\n\
         \x20 LET k AS Integer = 3\n\
         \x20 LET ys = collections::transform(xs, {callback})\n\
         \x20 io::print(toString(len(ys)) & other.label)\n\
         \x20 RETURN 0\n\
         END FUNC\n"
    )
}

/// `(transform loops, transform loops that call the callback)` in `main`.
fn loops(source: &str, target: CodeTarget) -> (usize, usize) {
    let plan = code_for_src_cached(source, target, Console);
    let main = crate::testutil::code_function(plan, "main");
    let labels = Stream::of(main).labels();
    let count = |prefix: &str| {
        labels
            .iter()
            .filter(|(_, name)| name.starts_with(prefix))
            .count()
    };
    (count("transform_call_loop"), count("transform_call_ok"))
}

/// Each callback is evaluated inline exactly when the table says.
#[test]
fn a_record_update_callback_is_inlined_only_when_it_is_safe_to() {
    for (label, declarations, callback, inline) in CASES {
        let (all, called) = loops(&program(declarations, callback), CodeTarget::LinuxX86_64);
        assert_eq!(
            all, 1,
            "{label}: the program has exactly one transform loop"
        );
        assert_eq!(
            called == 0,
            *inline,
            "{label}: `{callback}` must be {} (the call-path success label appeared \
             {called} time(s))",
            if *inline {
                "evaluated inline, with no call"
            } else {
                "called per element"
            }
        );
    }
}

/// A callback received as a parameter is only known at run time, so it is called.
#[test]
fn a_function_valued_parameter_is_always_called() {
    let source = "\
IMPORT collections

TYPE Item
  n AS Integer
END TYPE

FUNC apply(xs AS List OF Item, cb AS FUNC(Item) AS Item) AS List OF Item
  RETURN collections::transform(xs, cb)
END FUNC

FUNC main() AS Integer
  LET xs AS List OF Item = [Item[1], Item[2]]
  LET ys = apply(xs, LAMBDA(it AS Item) -> WITH it { n := it.n + 1 })
  RETURN len(ys)
END FUNC
";
    let plan = code_for_src_cached(source, CodeTarget::LinuxX86_64, Console);
    let apply = crate::testutil::code_function(plan, "apply");
    let labels = Stream::of(apply).labels();
    assert!(
        labels
            .iter()
            .any(|(_, name)| name.starts_with("transform_call_ok")),
        "`apply` cannot know `cb`, so its transform must call it"
    );
}

/// A callback result that owns a graph (a function value, or a record holding one)
/// is appended whole: the result lowers on every backend.
#[test]
fn a_transform_whose_results_own_a_graph_lowers_on_every_backend() {
    let source = "\
IMPORT collections

TYPE Bag
  run AS FUNC() AS Integer
  tag AS String
END TYPE

TYPE Tree
  v AS Integer
  kids AS List OF Tree
END TYPE

FUNC deep(i AS Integer) AS Tree
  RETURN Tree[i, []]
END FUNC

FUNC main() AS Integer
  LET bags = collections::transform([1, 2], LAMBDA(v AS Integer) -> Bag[LAMBDA() -> v, \"x\"])
  LET trees = collections::transform([1, 2], deep)
  LET fns = collections::transform([1, 2], LAMBDA(v AS Integer) -> LAMBDA() -> v)
  RETURN len(bags) + len(trees) + len(fns)
END FUNC
";
    for target in CodeTarget::ALL {
        let result = crate::testutil::try_code_for_src(source, target, Console);
        assert!(
            result.is_ok(),
            "graph-owning transform results must lower on {}: {:?}",
            target.name(),
            result.err()
        );
    }
}
