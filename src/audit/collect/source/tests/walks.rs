// ---------------------------------------------------------------------------
// audit/collect/source.rs — the walk's less common shapes: compiler-injected
// files, a resource acquired by reassignment, bare LET/RECOVER, STATE
// assignment, constructor and set literals, loop-nested trap classification,
// the audio/crypto capability split and an ungated LINK function.
// ---------------------------------------------------------------------------

use super::*;

fn callees(flow: &[FlowFunction], function: &str) -> Vec<(String, usize)> {
    flow.iter()
        .find(|entry| entry.function == function)
        .unwrap_or_else(|| panic!("no function `{function}`"))
        .calls
        .iter()
        .map(|call| (call.callee.clone(), call.line))
        .collect()
}

/// A compiler-injected file is not reported, yet its functions still decide the
/// fallibility of the user's calls into them.
#[test]
fn an_internal_file_is_not_reported_but_still_decides_fallibility() {
    let mut ast = project("FUNC main()\n  LET n = helper()\nEND FUNC\n");
    let mut internal = crate::ast::parse_source(
        std::path::Path::new("builtins/pkg.mfb"),
        "builtins/pkg.mfb",
        "FUNC helper() AS String\n  RETURN fs::readText(\"p\")\nEND FUNC\n",
    )
    .expect("parse");
    internal.internal = true;
    ast.files.push(internal);
    let (flow, permissions, ..) = collect_source(&ast);
    let reported: Vec<&str> = flow.iter().map(|entry| entry.function.as_str()).collect();
    assert_eq!(reported, vec!["main"]);
    assert!(permissions.is_empty(), "{:?}", permissions.len());
    assert_eq!(callees(&flow, "main"), vec![("helper".to_string(), 2)]);
    assert!(flow[0].fallible);
}

/// Two calls to the same capability-bearing builtin on one line are one
/// permission row.
#[test]
fn the_same_builtin_twice_on_one_line_is_one_permission() {
    let (_, permissions, ..) = collect_source(&project(
        "FUNC f()\n  LET s = os::getEnv(\"A\") & os::getEnv(\"B\")\nEND FUNC\n",
    ));
    let rows: Vec<(&str, usize)> = permissions
        .iter()
        .map(|permission| (permission.function.as_str(), permission.line))
        .collect();
    assert_eq!(rows, vec![("os.getEnv", 2)]);
}

/// `h = fs::open(p)` acquires a resource just as `LET h = fs::open(p)` does.
#[test]
fn a_resource_acquired_by_reassignment_is_listed() {
    let (_, _, resources, _) = collect_source(&project(
        "FUNC f()\n  MUT h = fs::open(\"a\")\n  h = fs::open(\"b\")\nEND FUNC\n",
    ));
    let rows: Vec<(&str, usize, &str, &str)> = resources
        .iter()
        .map(|resource| {
            (
                resource.name.as_str(),
                resource.line,
                resource.resource_type.as_str(),
                resource.close_op.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("h", 2, "fs.File", "fs.close"),
            ("h", 3, "fs.File", "fs.close")
        ]
    );
}

/// A LET without an initializer and a bare RECOVER carry no call, and the walk
/// goes on past them; a STATE assignment's value, a constructor's positional
/// argument and a set literal's elements are walked.
#[test]
fn calls_inside_state_assignment_constructor_and_set_literal_are_found() {
    let source = concat!(
        "TYPE Point\n  x AS String\n  y AS Integer\nEND TYPE\n",
        "FUNC f(h AS Integer)\n",
        "  MUT later AS Integer\n",
        "  h.state = fs::readText(\"a\")\n",
        "  LET p = Point[fs::readText(\"b\"), y := 2]\n",
        "  LET s = Set OF String { fs::readText(\"c\") }\n",
        "  TRAP(e)\n",
        "    io::print(\"x\")\n",
        "    RECOVER\n",
        "  END TRAP\n",
        "END FUNC\n",
    );
    let (flow, ..) = collect_source(&project(source));
    assert_eq!(
        callees(&flow, "f"),
        vec![
            ("fs.readText".to_string(), 7),
            ("fs.readText".to_string(), 8),
            ("fs.readText".to_string(), 9),
            ("io.print".to_string(), 11),
        ]
    );
}

/// A PROPAGATE, FAIL or valued RETURN nested in a FOR EACH (or a WHILE) in the
/// trap handler classifies the trap.
#[test]
fn a_trap_classifies_through_for_each_and_while_loops() {
    let trap = |handler: &str| {
        trap_classification(&format!(
            "FUNC f(xs AS List OF Integer) AS Integer\n  RETURN leaf()\n  TRAP(e)\n{handler}  \
             END TRAP\nEND FUNC\n"
        ))
    };
    assert_eq!(
        trap("    FOR EACH x IN xs\n      PROPAGATE\n    NEXT\n"),
        "propagates"
    );
    assert_eq!(
        trap("    FOR EACH x IN xs\n      FAIL error(1, \"x\")\n    NEXT\n"),
        "fails"
    );
    assert_eq!(
        trap("    WHILE TRUE\n      FAIL error(1, \"x\")\n    END WHILE\n"),
        "fails"
    );
    assert_eq!(
        trap("    FOR EACH x IN xs\n      RETURN x\n    NEXT\n"),
        "returns value"
    );
    assert_eq!(
        trap("    WHILE TRUE\n      RETURN 1\n    END WHILE\n"),
        "returns value"
    );
}

/// Reading the microphone is its own capability apart from playback; only the
/// entropy-drawing crypto builtins disclose randomness.
#[test]
fn audio_capture_and_crypto_randomness_are_disclosed_apart() {
    let source = concat!(
        "FUNC f()\n",
        "  RES m = audio::openInput()\n",
        "  RES o = audio::openOutput()\n",
        "  LET u = crypto::uuid4()\n",
        "  LET d = crypto::sha256(b)\n",
        "END FUNC\n",
    );
    let (_, permissions, ..) = collect_source(&project(source));
    let disclosed: Vec<(&str, &str)> = permissions
        .iter()
        .map(|permission| (permission.capability.as_str(), permission.function.as_str()))
        .collect();
    assert_eq!(
        disclosed,
        vec![
            ("audio", "audio.openOutput"),
            ("microphone", "audio.openInput"),
            ("randomness", "crypto.uuid4"),
        ]
    );
}

/// A LINK function without a SUCCESS_ON gate raises nothing, so a call to it
/// is not fallible; `toInt` over anything this walk cannot prove an enum (a
/// nested field read, an arithmetic result) stays fallible.
#[test]
fn an_ungated_link_call_is_total_and_unproven_to_int_stays_fallible() {
    let source = concat!(
        "LINK \"libm\" AS m\n",
        "  FUNC cosine(x AS Float) AS Float\n",
        "    SYMBOL \"cos\"\n",
        "    ABI (x CDouble) AS r CDouble\n",
        "    RETURN r\n",
        "  END FUNC\n",
        "END LINK\n",
        "FUNC f(p AS Integer) AS Integer\n",
        "  LET c = m::cosine(1.0)\n",
        "  LET a = toInt(p.x.y)\n",
        "  LET b = toInt(1 + 2)\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    let (flow, ..) = collect_source(&project(source));
    assert_eq!(
        callees(&flow, "f"),
        vec![("toInt".to_string(), 10), ("toInt".to_string(), 11)]
    );
}
