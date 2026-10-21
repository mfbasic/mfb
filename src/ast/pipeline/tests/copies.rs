//! `node_count` per expression form, read back from the bound bug-501 B puts
//! on pipeline copies: a right-hand side with `n` placeholders copies the
//! left-hand side `n - 1` times, and the refusal names how many nodes that is.

use std::path::Path;

/// The refusal for `left |> f(_, …)` with enough placeholders to copy more
/// than 4096 nodes of a left-hand side `nodes` nodes big.
fn refusal(left: &str, nodes: usize) -> (String, String) {
    let placeholders = 4096 / nodes + 2;
    let arguments = vec!["_"; placeholders].join(", ");
    let src =
        format!("FUNC main AS Nothing\n  LET v AS Integer = {left} |> f({arguments})\nEND FUNC\n");
    let details: Vec<String> = crate::rules::collecting_diagnostics(|| {
        let _ = crate::ast::parse_source(Path::new("main.mfb"), "main.mfb", &src);
    })
    .1
    .into_iter()
    .map(|(_, detail)| detail)
    .collect();
    let copied = (placeholders - 1) * nodes;
    (
        details.join("\n"),
        format!(
            "Pipeline placeholder substitution is too large: `_` occurs {placeholders} times, \
             which would copy {copied} nodes of the left-hand side (limit 4096)."
        ),
    )
}

#[test]
fn each_expression_form_counts_itself_and_its_children() {
    for (left, nodes) in [
        ("(1 + 2)", 3),
        ("(-x)", 2),
        ("P[1, y := 2]", 3),
        ("(LAMBDA() -> 1)", 2),
        ("[1, 2]", 3),
        ("Set OF Integer { 1, 2 }", 3),
        ("Map OF Integer TO Integer { 1 := 2 }", 3),
        ("p.x", 2),
        ("(WITH p { x := 1 })", 3),
    ] {
        let (found, wanted) = refusal(left, nodes);
        assert_eq!(found, wanted, "{left}");
    }
}
