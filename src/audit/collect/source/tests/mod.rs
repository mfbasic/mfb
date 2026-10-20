use super::*;

/// Parse standalone MFBASIC source into a single-file project.
fn project(source: &str) -> ast::AstProject {
    let path = std::path::Path::new("main.mfb");
    let file = crate::ast::parse_source(path, "main.mfb", source).expect("parse");
    ast::AstProject {
        name: "app".to_string(),
        files: vec![file],
    }
}

#[test]
fn callee_qualified_name_uses_dot_separator() {
    // Sanity: the collector matches `fs.open`, and `fs::open` parses to it.
    let ast = project("FUNC f()\n  LET h = fs::open(\"p\")\nEND FUNC\n");
    let (_, _, resources, ..) = collect_source(&ast);
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].resource_type, "fs.File");
    assert_eq!(resources[0].close_op, "fs.close");
}

#[test]
fn overloads_get_their_own_fallibility_verdict() {
    // A name-keyed map analyzed one `parse` body and broadcast its verdict to
    // the other. Each declaration must be judged on its own body, while a
    // call site — which has no types yet — resolves across the overload set.
    let source = concat!(
        "FUNC parse(n AS Integer) AS Integer\n",
        "  RETURN n\n",
        "END FUNC\n",
        "FUNC parse(s AS String) AS Integer\n",
        "  RETURN len(fs::readText(s))\n",
        "END FUNC\n",
        "FUNC main() AS Nothing\n",
        "  LET x = parse(\"f.txt\")\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    let at = |line: usize| {
        flow.iter()
            .find(|entry| entry.line == line)
            .unwrap_or_else(|| panic!("no function declared at line {line}"))
    };
    assert!(!at(1).fallible, "the Integer overload is pure");
    assert!(
        at(4).fallible,
        "the String overload calls a fallible builtin"
    );
    assert!(at(7).fallible, "main calls the fallible overload");
}

/// bug-679: `toInt(<enum>)` is the ordinal the value already holds and cannot
/// fail (plan-140-B), so a function whose only `toInt` is over an enum is
/// total and must not be reported — nor must its callers, which is where the
/// over-reporting actually hurt (one `toInt(f)` marked 29 further functions).
/// `toString(<enum>)` was total then too; since the owner's 2026-10-04
/// decision every `toString` overload is fallible (it builds a `String` and
/// can raise `ErrOutOfMemory` — bug-811), so the two halves no longer agree.
#[test]
fn toint_over_an_enum_is_not_fallible() {
    let source = concat!(
        "ENUM Color\n",
        "  Red, Green, Blue\n",
        "END ENUM\n",
        "FUNC ordinalOf(c AS Color) AS Integer\n",
        "  RETURN toInt(c)\n",
        "END FUNC\n",
        "FUNC nameOf(c AS Color) AS String\n",
        "  RETURN toString(c)\n",
        "END FUNC\n",
        "FUNC literalOrdinal() AS Integer\n",
        "  RETURN toInt(Color.Blue)\n",
        "END FUNC\n",
        "FUNC viaLet(c AS Color) AS Integer\n",
        "  LET held AS Color = c\n",
        "  RETURN toInt(held)\n",
        "END FUNC\n",
        "FUNC parseIt(s AS String) AS Integer\n",
        "  RETURN toInt(s)\n",
        "END FUNC\n",
        "FUNC caller(c AS Color) AS Integer\n",
        "  RETURN ordinalOf(c) + 1\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    let at = |line: usize| {
        flow.iter()
            .find(|entry| entry.line == line)
            .unwrap_or_else(|| panic!("no function declared at line {line}"))
    };
    assert!(!at(4).fallible, "toInt of an enum parameter cannot fail");
    assert!(
        at(7).fallible,
        "toString of an enum builds a String and can raise ErrOutOfMemory"
    );
    assert!(
        !at(10).fallible,
        "toInt of an enum member literal cannot fail"
    );
    assert!(!at(13).fallible, "toInt of an enum-typed LET cannot fail");
    // The non-goal half: every other overload really can fail, and a caller of
    // one stays fallible. Marking the name infallible outright would delete the
    // only guard between a bad parse and a dead process.
    assert!(at(17).fallible, "toInt of a String can fail on a bad parse");
    // The transitive half — the actual blast radius of the over-report.
    assert!(!at(20).fallible, "a caller of a total function is total");
}

/// bug-811: every `toString` overload is fallible — each builds a new `String`
/// and can raise `ErrOutOfMemory`, and the `List OF Byte` one also raises
/// `ErrEncoding` on ill-formed UTF-8 (owner decision 2026-10-04; the
/// inline-`TRAP` census made the same change). `mfb audit` reported all of them
/// total, because its name list never mentioned `toString`.
#[test]
fn tostring_is_fallible_on_every_overload() {
    let source = concat!(
        "ENUM Color\n",
        "  Red, Green\n",
        "END ENUM\n",
        "FUNC decode(b AS List OF Byte) AS String\n",
        "  RETURN toString(b)\n",
        "END FUNC\n",
        "FUNC show(n AS Integer) AS String\n",
        "  RETURN toString(n)\n",
        "END FUNC\n",
        "FUNC name(c AS Color) AS String\n",
        "  RETURN toString(c)\n",
        "END FUNC\n",
        "FUNC width(s AS String) AS Integer\n",
        "  RETURN len(s)\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    let at = |line: usize| {
        flow.iter()
            .find(|entry| entry.line == line)
            .unwrap_or_else(|| panic!("no function declared at line {line}"))
    };
    for (line, what) in [(4, "List OF Byte"), (7, "Integer"), (10, "enum")] {
        assert!(at(line).fallible, "toString of a {what} is fallible");
        assert_eq!(
            at(line)
                .calls
                .iter()
                .map(|call| call.callee.as_str())
                .collect::<Vec<_>>(),
            vec!["toString"],
            "toString of a {what} gets a call row",
        );
    }
    // `len` builds nothing and declares no error: it stays total.
    assert!(!at(13).fallible, "len cannot fail");
}

/// bug-811: for every built-in the inline-`TRAP` census answers — the bare
/// `general` names and the inline-lowered members — `mfb audit` and the census
/// read the same fact, the member's declared errors (`mfb spec language
/// error-model` rule 11: a member is infallible only if it declares no error).
/// Before, audit's hand list called `collections::append`, `strings::upper`,
/// `math::sqrt`, the variable `bits` shifts and `toString` total while the
/// census, correctly, kept their handlers live.
#[test]
fn census_builtins_are_fallible_exactly_when_they_declare_an_error() {
    let no_names = HashSet::new();
    let mut measured = 0;
    for package in crate::codegen::registry::registry().packages() {
        for function in package.functions() {
            let callee = if package.import_name() == "general" {
                function.name.to_string()
            } else {
                format!("{}.{}", package.import_name(), function.name)
            };
            if !in_inline_census(&callee) {
                continue;
            }
            measured += 1;
            let declares = function
                .implementations
                .iter()
                .any(|implementation| !implementation.errors.is_empty());
            assert_eq!(
                is_fallible_call(&callee, &no_names, &[], &no_names, &no_names),
                declares,
                "{callee}: audit must call it fallible exactly when it declares an error",
            );
        }
    }
    assert!(
        measured > 100,
        "the census domain shrank to {measured} members"
    );
    for (callee, fallible) in [
        ("toString", true),
        ("len", false),
        ("typeName", false),
        ("collections.append", true),
        ("collections.contains", false),
        ("bits.band", false),
        ("bits.sl", true),
    ] {
        assert_eq!(
            is_fallible_call(callee, &no_names, &[], &no_names, &no_names),
            fallible,
            "{callee}",
        );
    }
}

/// Every built-in that declares an error is reported fallible, census or not.
/// The hand lists that used to follow `is_fallible_call` were derived from the man-page Errors
/// tables once (bug-278) and drifted: `process` and `term` were never added,
/// and since bug-698 every allocating member declares `ErrOutOfMemory`. 208
/// declaring members outside the inline census were reported total.
#[test]
fn every_declaring_builtin_is_fallible() {
    let no_names = HashSet::new();
    let mut measured = 0;
    for package in crate::codegen::registry::registry().packages() {
        for function in package.functions() {
            let callee = if package.import_name() == "general" {
                function.name.to_string()
            } else {
                format!("{}.{}", package.import_name(), function.name)
            };
            let declares = function
                .implementations
                .iter()
                .any(|implementation| !implementation.errors.is_empty());
            if !declares {
                continue;
            }
            measured += 1;
            assert!(
                is_fallible_call(&callee, &no_names, &[], &no_names, &no_names),
                "{callee} declares an error, so a call to it is fallible",
            );
        }
    }
    assert!(measured > 400, "only {measured} declaring members found");
}

/// bug-814: every built-in, census or not, is fallible in audit exactly when it
/// declares an error. The hand lists that used to sit under `is_fallible_call`
/// called five members fallible that declare and raise nothing
/// (`io::isBuffered`/`isInputTerminal`/`isOutputTerminal`/`isErrorTerminal`,
/// `thread::isCancelled`; `declared_errors_all.rs` holds each one's lowering to
/// its empty list) through a whole-package rule, and named nine members that
/// do not exist. Since every `Body::Rewrite` member is now checked against its body,
/// the declaration is the whole answer.
#[test]
fn every_builtin_is_fallible_exactly_when_it_declares_an_error() {
    let no_names = HashSet::new();
    let mut wrong = Vec::new();
    let mut measured = 0;
    for package in crate::codegen::registry::registry().packages() {
        for function in package.functions() {
            let callee = if package.import_name() == "general" {
                function.name.to_string()
            } else {
                format!("{}.{}", package.import_name(), function.name)
            };
            measured += 1;
            let declares = function
                .implementations
                .iter()
                .any(|implementation| !implementation.errors.is_empty());
            if is_fallible_call(&callee, &no_names, &[], &no_names, &no_names) != declares {
                wrong.push(format!("{callee} (declares an error: {declares})"));
            }
        }
    }
    assert!(measured > 500, "only {measured} built-in members found");
    assert!(
        wrong.is_empty(),
        "audit must call a built-in fallible exactly when it declares an error:\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn every_host_capability_is_disclosed() {
    // net/os/math/datetime and native LINK calls each disclose a capability;
    // pure builtins in the same packages disclose nothing.
    let source = concat!(
        "LINK \"sqlite3\" AS sql\n",
        "  FUNC open(path AS String) AS Nothing\n",
        "    SYMBOL \"sqlite3_open\"\n",
        "    ABI (path CString) AS status CInt32\n",
        "    SUCCESS_ON status = 0\n",
        "  END FUNC\n",
        "END LINK\n",
        "FUNC f()\n",
        "  tcp::close(s)\n",
        "  os::getEnv(\"HOME\")\n",
        "  os::pid()\n",
        "  math::rand(1, 6)\n",
        "  math::floor(1.5)\n",
        "  datetime::now()\n",
        "  datetime::toIso(x)\n",
        "  sql::open(\":memory:\")\n",
        "END FUNC\n",
    );
    let (_, permissions, _, ..) = collect_source(&project(source));
    let disclosed = permissions
        .iter()
        .map(|permission| (permission.capability.as_str(), permission.function.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        disclosed,
        vec![
            ("clock", "datetime.now"),
            ("environment", "os.getEnv"),
            ("native", "sql.open"),
            ("network", "tcp.close"),
            ("process", "os.pid"),
            ("randomness", "math.rand"),
        ]
    );
    // Native calls are tagged as such; builtin calls stay "standard".
    let native = permissions
        .iter()
        .find(|permission| permission.capability == "native")
        .expect("native permission");
    assert_eq!(native.kind, "native");
    assert_eq!(native.package, "sql");
}

// bug-477: `mfb audit` must surface a relaxed-trust connection, and must not
// invent one. The negative half matters as much as the positive: reporting a
// FALSE (or omitted) call would train reviewers to ignore the finding.
#[test]
fn relaxed_trust_is_reported_only_when_the_flag_is_literally_true() {
    let source = concat!(
        "FUNC f()\n",
        "  RES a = tls::connect(\"h\", 443, 1000, \"h\", allowSelfSigned := TRUE)\n",
        "  RES b = tls::connect(\"h\", 443, 1000, \"h\", allowSelfSigned := FALSE)\n",
        "  RES c = tls::connect(\"h\", 443)\n",
        "END FUNC\n",
    );
    let (_, _, _, relaxed) = collect_source(&project(source));
    assert_eq!(
        relaxed.len(),
        1,
        "exactly the TRUE call site is a relaxed-trust finding; FALSE and the \
             omitted form are ordinary verified connections"
    );
    assert_eq!(relaxed[0].line, 2);
}

#[test]
fn permissions_are_collected_and_deduplicated() {
    let source = concat!(
        "FUNC f()\n",
        "  io::print(\"a\")\n",
        "  io::print(\"b\")\n",
        "  LET h = fs::open(\"p\")\n",
        "  RES t = thread::start(worker)\n",
        "END FUNC\n",
    );
    let (_, permissions, _, ..) = collect_source(&project(source));
    let caps: Vec<&str> = permissions.iter().map(|p| p.capability.as_str()).collect();
    assert!(caps.contains(&"terminal"));
    assert!(caps.contains(&"filesystem"));
    assert!(caps.contains(&"threads"));
}

#[test]
fn resources_found_across_nested_control_flow() {
    let source = concat!(
        "FUNC f(n AS Integer)\n",
        "  IF n > 0 THEN\n",
        "    LET a = fs::open(\"a\")\n",
        "  ELSE\n",
        "    LET b = fs::openFile(\"b\")\n",
        "  END IF\n",
        "  FOR i = 1 TO n\n",
        "    LET c = tcp::connect(addr)\n",
        "  NEXT\n",
        "  FOR EACH x IN items\n",
        "    LET d = tcp::listen(addr)\n",
        "  NEXT\n",
        "  WHILE n > 0\n",
        "    RES e = thread::start(worker)\n",
        "  END WHILE\n",
        "  DO\n",
        "    LET g = tcp::accept(listener)\n",
        "  LOOP UNTIL n < 0\n",
        "END FUNC\n",
    );
    let (_, _, resources, ..) = collect_source(&project(source));
    let types: Vec<&str> = resources.iter().map(|r| r.resource_type.as_str()).collect();
    assert!(types.contains(&"fs.File"));
    // plan-110-E: the stream producers are tcp's, and the audit names them
    // package-qualified like `fs.File` rather than by the bare spelling that
    // three packages now share.
    assert!(types.contains(&"tcp.Socket"));
    assert!(types.contains(&"tcp.Listener"));
    assert!(types.contains(&"Thread"));
}

#[test]
fn resources_found_inside_match_case() {
    let source = concat!(
        "TYPE Circle\n  radius AS Integer\nEND TYPE\n",
        "UNION Shape\n  Circle\nEND UNION\n",
        "FUNC f(s AS Shape)\n",
        "  MATCH s\n",
        "    CASE Circle(c)\n",
        "      LET a = fs::createTempFile(\"t\")\n",
        "    CASE ELSE\n",
        "      RETURN\n",
        "  END MATCH\n",
        "END FUNC\n",
    );
    let (_, _, resources, ..) = collect_source(&project(source));
    assert_eq!(resources.len(), 1);
}

#[test]
fn trap_classification_propagates() {
    let source = concat!(
        "FUNC f() AS Integer\n",
        "  RETURN leaf()\n",
        "  TRAP(e)\n",
        "    PROPAGATE\n",
        "  END TRAP\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    let f = flow.iter().find(|f| f.function == "f").unwrap();
    assert_eq!(f.trap.as_ref().unwrap().classification, "propagates");
}

#[test]
fn trap_classification_fails() {
    let source = concat!(
        "FUNC f() AS Integer\n",
        "  RETURN leaf()\n",
        "  TRAP(e)\n",
        "    FAIL error(2, \"x\")\n",
        "  END TRAP\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    let f = flow.iter().find(|f| f.function == "f").unwrap();
    assert_eq!(f.trap.as_ref().unwrap().classification, "fails");
}

#[test]
fn trap_classification_returns_value() {
    let source = concat!(
        "FUNC f() AS Integer\n",
        "  RETURN leaf()\n",
        "  TRAP(e)\n",
        "    RETURN 7\n",
        "  END TRAP\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    let f = flow.iter().find(|f| f.function == "f").unwrap();
    assert_eq!(f.trap.as_ref().unwrap().classification, "returns value");
}

#[test]
fn trap_classification_recovers() {
    let source = concat!(
        "FUNC f() AS Integer\n",
        "  RETURN leaf()\n",
        "  TRAP(e)\n",
        "    RECOVER 0\n",
        "  END TRAP\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    let f = flow.iter().find(|f| f.function == "f").unwrap();
    assert_eq!(f.trap.as_ref().unwrap().classification, "recovers");
}

#[test]
fn user_function_becomes_fallible_by_calling_fallible_builtin() {
    let source = concat!(
        "FUNC reader() AS String\n",
        "  RETURN fs::readText(\"p\")\n",
        "END FUNC\n",
        "FUNC caller() AS String\n",
        "  RETURN reader()\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    let reader = flow.iter().find(|f| f.function == "reader").unwrap();
    let caller = flow.iter().find(|f| f.function == "caller").unwrap();
    // `reader` calls a fallible builtin; `caller` calls fallible `reader`.
    assert!(reader.fallible);
    assert!(caller.fallible);
}

/// bug-791: `tcp`/`udp` calls were missing from the census, so a function
/// acquiring a socket reported `fallible: false` and no call rows.
#[test]
fn tcp_and_udp_calls_are_fallible_and_propagate() {
    let source = concat!(
        "IMPORT tcp\n",
        "IMPORT udp\n",
        "FUNC open() AS Integer\n",
        "  RES listener = tcp::listen(\"127.0.0.1\", 0)\n",
        "  RES socket = udp::bind(\"127.0.0.1\", 0)\n",
        "  RETURN 0\n",
        "END FUNC\n",
        "FUNC caller() AS Integer\n",
        "  RETURN open()\n",
        "END FUNC\n",
        "FUNC guarded() AS Integer\n",
        "  RES conn = tcp::connect(\"127.0.0.1\", 1)\n",
        "  RETURN 0\n",
        "  TRAP(e)\n",
        "    RETURN 1\n",
        "  END TRAP\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    let open = flow.iter().find(|f| f.function == "open").unwrap();
    assert!(open.fallible);
    let callees: Vec<&str> = open.calls.iter().map(|c| c.callee.as_str()).collect();
    assert!(
        callees.contains(&"tcp.listen") && callees.contains(&"udp.bind"),
        "{callees:?}"
    );
    assert!(
        flow.iter()
            .find(|f| f.function == "caller")
            .unwrap()
            .fallible
    );
    // A handler that returns keeps the error from leaving the function.
    let guarded = flow.iter().find(|f| f.function == "guarded").unwrap();
    assert!(!guarded.fallible);
    assert!(guarded.calls.iter().any(|c| c.callee == "tcp.connect"));
}

#[test]
fn fail_statement_makes_function_fallible() {
    let source = concat!(
        "FUNC boom() AS Integer\n",
        "  FAIL error(1, \"x\")\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    assert!(flow.iter().find(|f| f.function == "boom").unwrap().fallible);
}

#[test]
fn walk_visits_loop_control_and_named_and_lambda_and_trapped() {
    // Covers walk_statements' Continue/Exit/Recover arms, For step, ForEach,
    // and walk_expression's named-call-arg / lambda / constructor-named /
    // trapped-expression / member-access branches — each wrapping a fallible
    // builtin so a permission and call site are recorded.
    let source = concat!(
        "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n",
        "FUNC wrap(text AS String) AS String\n",
        "  RETURN text\n",
        "END FUNC\n",
        "FUNC f(n AS Integer) AS Integer\n",
        "  FOR i = 1 TO n STEP 2\n",
        "    io::print(toString(i))\n",
        "    CONTINUE FOR\n",
        "  NEXT\n",
        "  FOR EACH e IN items\n",
        "    EXIT FOR\n",
        "  NEXT\n",
        "  LET g AS FUNC(Integer) AS String = LAMBDA(v AS Integer) -> fs::read(\"p\")\n",
        "  LET named AS String = wrap(text := fs::read(\"n\"))\n",
        "  LET p AS Point = Point[x := fs::read(\"q\"), y := 1]\n",
        "  LET r AS String = fs::read(\"r\") TRAP(e)\n",
        "    RECOVER \"x\"\n",
        "  END TRAP\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    let (flow, permissions, _, ..) = collect_source(&project(source));
    assert!(flow.iter().any(|f| f.function == "f"));
    assert!(permissions.iter().any(|p| p.capability == "filesystem"));
}

#[test]
fn match_guard_and_case_bodies_are_walked() {
    let source = concat!(
        "TYPE Circle\n  radius AS Integer\nEND TYPE\n",
        "UNION Shape\n  Circle\nEND UNION\n",
        "FUNC f(s AS Shape) AS Integer\n",
        "  MATCH s\n",
        "    CASE Circle(c) WHEN c.radius > 0\n",
        "      RETURN fs::readText(\"p\") & \"\"\n",
        "    CASE ELSE\n",
        "      RETURN 0\n",
        "  END MATCH\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    // The case body raises via a fallible builtin, so `f` is fallible.
    assert!(flow.iter().find(|f| f.function == "f").unwrap().fallible);
}

#[test]
fn nested_fail_and_propagate_inside_control_flow_is_detected() {
    // `statements_fail_or_propagate` / `statements_contain_*` recurse through
    // IF / FOR / MATCH; a FAIL nested in an IF still makes the function
    // fallible, and a PROPAGATE nested in a trap's FOR classifies it.
    let source = concat!(
        "FUNC boom(n AS Integer) AS Integer\n",
        "  IF n > 0 THEN\n",
        "    FAIL error(1, \"x\")\n",
        "  END IF\n",
        "  RETURN 0\n",
        "END FUNC\n",
        "FUNC classified() AS Integer\n",
        "  RETURN leaf()\n",
        "  TRAP(e)\n",
        "    FOR i = 1 TO 2\n",
        "      PROPAGATE\n",
        "    NEXT\n",
        "  END TRAP\n",
        "END FUNC\n",
    );
    let (flow, _, _, ..) = collect_source(&project(source));
    assert!(flow.iter().find(|f| f.function == "boom").unwrap().fallible);
    let classified = flow.iter().find(|f| f.function == "classified").unwrap();
    assert_eq!(
        classified.trap.as_ref().unwrap().classification,
        "propagates"
    );
}

/// Classify the trap of the first function in `source`.
fn trap_classification(source: &str) -> String {
    let (flow, _, _, ..) = collect_source(&project(source));
    flow.iter()
        .find_map(|f| f.trap.as_ref().map(|t| t.classification.clone()))
        .expect("a trap")
}

#[test]
fn classify_recurses_through_if_match_and_loop_for_propagate() {
    // PROPAGATE nested inside IF, MATCH, and a loop each classify as
    // "propagates", exercising every recursive arm of the propagate scanner.
    assert_eq!(
        trap_classification(concat!(
            "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
            "    IF TRUE THEN\n      PROPAGATE\n    END IF\n",
            "  END TRAP\nEND FUNC\n",
        )),
        "propagates"
    );
    assert_eq!(
        trap_classification(concat!(
            "TYPE Circle\n  radius AS Integer\nEND TYPE\n",
            "UNION Shape\n  Circle\nEND UNION\n",
            "FUNC f(s AS Shape) AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
            "    MATCH s\n      CASE Circle(c)\n        PROPAGATE\n      CASE ELSE\n        RECOVER 0\n    END MATCH\n",
            "  END TRAP\nEND FUNC\n",
        )),
        "propagates"
    );
    assert_eq!(
        trap_classification(concat!(
            "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
            "    WHILE TRUE\n      PROPAGATE\n    END WHILE\n",
            "  END TRAP\nEND FUNC\n",
        )),
        "propagates"
    );
}

#[test]
fn classify_recurses_through_if_match_and_loop_for_fail() {
    // FAIL nested inside IF / MATCH / loop classifies as "fails" and drives
    // the fail scanner's recursive arms (only reached when no PROPAGATE).
    assert_eq!(
        trap_classification(concat!(
            "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
            "    IF TRUE THEN\n      FAIL error(1, \"x\")\n    END IF\n",
            "  END TRAP\nEND FUNC\n",
        )),
        "fails"
    );
    assert_eq!(
        trap_classification(concat!(
            "TYPE Circle\n  radius AS Integer\nEND TYPE\n",
            "UNION Shape\n  Circle\nEND UNION\n",
            "FUNC f(s AS Shape) AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
            "    MATCH s\n      CASE Circle(c)\n        FAIL error(1, \"x\")\n      CASE ELSE\n        RECOVER 0\n    END MATCH\n",
            "  END TRAP\nEND FUNC\n",
        )),
        "fails"
    );
    assert_eq!(
        trap_classification(concat!(
            "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
            "    FOR i = 1 TO 2\n      FAIL error(1, \"x\")\n    NEXT\n",
            "  END TRAP\nEND FUNC\n",
        )),
        "fails"
    );
}

#[test]
fn classify_recurses_through_if_match_and_loop_for_return_value() {
    // A value RETURN nested in IF / MATCH / loop classifies as "returns value"
    // (reached only when neither PROPAGATE nor FAIL is present).
    assert_eq!(
        trap_classification(concat!(
            "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
            "    IF TRUE THEN\n      RETURN 1\n    END IF\n",
            "  END TRAP\nEND FUNC\n",
        )),
        "returns value"
    );
    assert_eq!(
        trap_classification(concat!(
            "TYPE Circle\n  radius AS Integer\nEND TYPE\n",
            "UNION Shape\n  Circle\nEND UNION\n",
            "FUNC f(s AS Shape) AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
            "    MATCH s\n      CASE Circle(c)\n        RETURN 1\n      CASE ELSE\n        RECOVER 0\n    END MATCH\n",
            "  END TRAP\nEND FUNC\n",
        )),
        "returns value"
    );
    assert_eq!(
        trap_classification(concat!(
            "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
            "    FOR i = 1 TO 2\n      RETURN 1\n    NEXT\n",
            "  END TRAP\nEND FUNC\n",
        )),
        "returns value"
    );
}

#[test]
fn walk_visits_diverse_expression_and_statement_forms() {
    // Exercises binary/unary/constructor/with-update/list/map/member access
    // plus assign / exit-program / expression statements, all containing a
    // fallible builtin call so the walker reaches every branch.
    let source = concat!(
        "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n",
        "FUNC f(n AS Integer) AS Integer\n",
        "  LET a AS String = fs::read(\"p\") & \"z\"\n",
        "  LET b AS Integer = -n\n",
        "  LET p AS Point = Point(fs::read(\"q\"), 2)\n",
        "  LET p2 AS Point = WITH p { x := fs::read(\"r\") }\n",
        "  LET list AS List OF String = [fs::read(\"s\")]\n",
        "  LET map AS Map OF String TO String = Map OF String TO String { \"k\" := fs::read(\"t\") }\n",
        "  LET m AS Integer = p.x\n",
        "  a = fs::read(\"u\")\n",
        "  io::print(toString(n))\n",
        "  os::exit(n)\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    let (flow, permissions, _, ..) = collect_source(&project(source));
    assert!(flow.iter().any(|f| f.function == "f"));
    assert!(permissions.iter().any(|p| p.capability == "filesystem"));
}
