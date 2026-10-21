//! Statement forms at the edges: a write to an imported package's `EXPORT MUT`
//! (`pkg::Name = value`), postfix TRAP bindings and depth, union CASE patterns
//! cut short, and the statement-nesting bound.

use super::*;

fn details(src: &str) -> Vec<String> {
    crate::rules::collecting_diagnostics(|| {
        let _ = parse_source(Path::new("main.mfb"), "main.mfb", src);
    })
    .1
    .into_iter()
    .map(|(_, detail)| detail)
    .collect()
}

fn in_main(body: &str) -> String {
    format!("FUNC main AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
}

#[test]
fn a_qualified_name_followed_by_equals_is_an_assignment_to_package_state() {
    let file = parse_ok(&in_main("  counter::total = 1 + 2"));
    let Item::Function(function) = &file.items[0] else {
        panic!("expected a function");
    };
    let Statement::Assign { name, .. } = &function.body[0] else {
        panic!("expected an assignment, got {:?}", function.body[0]);
    };
    assert_eq!(name, "counter.total");
    // With a postfix TRAP the statement ends with the trap block.
    let file = parse_ok(&in_main(
        "  counter::total = risky() TRAP (e)\n    RETURN 1\n  END TRAP",
    ));
    let Item::Function(function) = &file.items[0] else {
        panic!("expected a function");
    };
    assert!(matches!(
        &function.body[0],
        Statement::Assign {
            value: Expression::Trapped { .. },
            ..
        }
    ));
}

#[test]
fn a_postfix_trap_binding_must_close_and_its_subject_is_bounded() {
    assert_eq!(
        details(&in_main("  LET v AS Integer = f() TRAP (e x)\n  END TRAP"))[0],
        "TRAP error binding must close with `)`."
    );
    let full = format!("1{}", " + 1".repeat(256));
    assert_eq!(
        details(&in_main(&format!(
            "  LET v AS Integer = {full} TRAP (e)\n    RETURN 1\n  END TRAP"
        ))),
        ["Expression nesting is too deep."]
    );
}

#[test]
fn a_union_case_pattern_binding_must_close() {
    let with_case = |pattern: &str| {
        in_main(&format!(
            "  MATCH shape\n    CASE {pattern}\n      RETURN 1\n  END MATCH"
        ))
    };
    assert_eq!(
        details(&with_case("Circle(c d)"))[0],
        "Union CASE pattern binding must close with `)`."
    );
}

/// Nested statement blocks past the bound unwind with one diagnostic instead
/// of overflowing the stack. Run on a thread with room for the debug build's
/// frames (a real build parses on the main thread's 8 MiB in release).
#[test]
fn nested_blocks_and_inline_traps_past_the_bound_are_refused() {
    let nested_ifs = format!(
        "{}  RETURN 0\n{}",
        "  IF TRUE THEN\n".repeat(300),
        "  END IF\n".repeat(300)
    );
    let nested_traps = format!(
        "  LET v AS Integer = {}0{}",
        "f() TRAP (e)\n LET w AS Integer = ".repeat(300),
        "\n END TRAP".repeat(300)
    );
    let found = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            (
                details(&in_main(&nested_ifs)),
                details(&in_main(&nested_traps)),
            )
        })
        .unwrap()
        .join()
        .unwrap();
    for (what, diagnostics) in [("IF", found.0), ("TRAP", found.1)] {
        assert!(
            diagnostics
                .iter()
                .any(|detail| detail.contains("nest") || detail.contains("deep")),
            "{what}: {diagnostics:?}"
        );
    }
}
