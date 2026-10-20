//! plan-176-K: `ast::tests` — the parse_errors tests. The shared `parse_ok`/`parse_err` helpers\n//! and the project/source-root tests live in `ast/tests/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// Parse error paths (items / stmt / expr / lexical). Valid-program e2e tests
// never exercise these, so they concentrate the uncovered lines.
// ---------------------------------------------------------------------------

#[test]
fn exit_and_continue_take_only_a_loop_kind() {
    // plan-180-B: the follower of EXIT/CONTINUE is one of FOR, DO, WHILE. Every
    // other spelling fails to parse; the rule codes are pinned by the
    // `exit-unknown-invalid` and `continue-unknown-invalid` syntax fixtures.
    for stmt in [
        "EXIT FUNC",
        "EXIT PROGRAM 1",
        "EXIT",
        "EXIT foo",
        "CONTINUE FUNC",
        "CONTINUE PROGRAM 1",
        "CONTINUE",
        "CONTINUE foo",
    ] {
        let src = format!("FUNC main AS Integer\n  {stmt}\n  RETURN 0\nEND FUNC\n");
        parse_err(&src);
    }
    parse_ok("FUNC main AS Integer\n  EXIT FOR\n  EXIT DO\n  EXIT WHILE\n  CONTINUE FOR\n  CONTINUE DO\n  CONTINUE WHILE\n  RETURN 0\nEND FUNC\n");
}

#[test]
fn top_level_errors_are_reported() {
    // Unexpected top-level token.
    assert!(try_parse("garbage\n").is_err());
    // IMPORT with no name.
    assert!(try_parse("IMPORT\n").is_err());
    // Binding with no name.
    assert!(try_parse("LET\n").is_err());
    // Function with no name.
    assert!(try_parse("FUNC\n").is_err());
    // Unterminated function.
    assert!(try_parse("FUNC main AS Integer\n  RETURN 0\n").is_err());
    // Type with no name.
    assert!(try_parse("TYPE\n").is_err());
    // Unterminated type.
    assert!(try_parse("TYPE T\n  x AS Integer\n").is_err());
    // END without matching kind.
    assert!(try_parse("FUNC main\nEND TYPE\n").is_err());
}

#[test]
fn function_header_and_body_errors() {
    // Missing close paren on params.
    assert!(try_parse("FUNC f(a AS Integer\nEND FUNC\n").is_err());
    // Two TRAPs.
    assert!(try_parse(
        "FUNC f AS Integer\n  RETURN 0\n  TRAP(a)\n    RETURN 1\n  END TRAP\n  TRAP(b)\n    RETURN 2\n  END TRAP\nEND FUNC\n"
    )
    .is_err());
    // Statement after TRAP.
    assert!(try_parse(
        "FUNC f AS Integer\n  TRAP(a)\n    RETURN 1\n  END TRAP\n  RETURN 0\nEND FUNC\n"
    )
    .is_err());
    // Bad parameter name.
    assert!(try_parse("FUNC f(123)\nEND FUNC\n").is_err());
}

#[test]
fn function_declaration_requires_the_func_keyword() {
    // bug-292: with a visibility prefix, `check_top_level_item_start` stops
    // looking after `ISOLATED`, so `parse_function` is the only place that can
    // reject what follows it. It used to default every unrecognized token to
    // `Func` and compile these to an executable.
    assert!(try_parse("PUBLIC ISOLATED BOGUS weird AS Integer\n  RETURN 1\nEND FUNC\n").is_err());
    assert!(try_parse("PUBLIC ISOLATED ISOLATED f AS Integer\n  RETURN 1\nEND FUNC\n").is_err());
    // The valid spellings the check must leave alone.
    assert!(try_parse("PUBLIC ISOLATED FUNC f AS Integer\n  RETURN 0\nEND FUNC\n").is_ok());
    assert!(try_parse("ISOLATED FUNC f AS Integer\n  RETURN 0\nEND FUNC\n").is_ok());
    assert!(try_parse("PUBLIC FUNC f AS Integer\n  RETURN 0\nEND FUNC\n").is_ok());
    assert!(try_parse("EXPORT FUNC s AS Nothing\nEND FUNC\n").is_ok());
}

#[test]
fn trap_header_errors() {
    // Bare TRAP (no `(ident)` binding) is a valid function-level trap: plan-37
    // makes the error binding optional. It parses at the AST level.
    assert!(try_parse(
        "FUNC f AS Integer\n  RETURN 0\n  TRAP\n    RETURN 1\n  END TRAP\nEND FUNC\n"
    )
    .is_ok());
    // TRAP with an empty binding `()` is still an error (an identifier is
    // required once the `(` is present).
    assert!(try_parse("FUNC f AS Integer\n  RETURN 0\n  TRAP()\n  END TRAP\nEND FUNC\n").is_err());
    // TRAP unterminated.
    assert!(
        try_parse("FUNC f AS Integer\n  RETURN 0\n  TRAP(e)\n    RETURN 1\nEND FUNC\n").is_err()
    );
}

#[test]
fn type_decl_errors() {
    // Field without AS.
    assert!(try_parse("TYPE T\n  x\nEND TYPE\n").is_err());
    // Union member that is not a name.
    assert!(try_parse("UNION U\n  123\nEND UNION\n").is_err());
    // Enum member that is not a name.
    assert!(try_parse("ENUM E\n  123\nEND ENUM\n").is_err());
    // Template param that is not a name.
    assert!(try_parse("TYPE T OF 123\n  x AS Integer\nEND TYPE\n").is_err());
    // END names wrong kind.
    assert!(try_parse("TYPE T\n  x AS Integer\nEND UNION\n").is_err());
}

#[test]
fn statement_errors() {
    // Inline IF branch must be simple (not a block statement).
    assert!(
        try_parse("FUNC f AS Integer\n  IF TRUE THEN FOR i = 1 TO 2\n  NEXT\nEND FUNC\n").is_err()
    );
    // EXIT with bad target.
    assert!(try_parse("FUNC f\n  EXIT NOPE\nEND FUNC\n").is_err());
    // CONTINUE with bad target.
    assert!(try_parse("FUNC f\n  CONTINUE NOPE\nEND FUNC\n").is_err());
    // IF without THEN.
    assert!(try_parse("FUNC f\n  IF TRUE\n  END IF\nEND FUNC\n").is_err());
    // MATCH without CASE.
    assert!(try_parse("FUNC f\n  MATCH x\n    total = 1\n  END MATCH\nEND FUNC\n").is_err());
    // FOR without `=`.
    assert!(try_parse("FUNC f\n  FOR i 1 TO 2\n  NEXT\nEND FUNC\n").is_err());
    // FOR without TO.
    assert!(try_parse("FUNC f\n  FOR i = 1 2\n  NEXT\nEND FUNC\n").is_err());
    // FOR EACH without IN.
    assert!(try_parse("FUNC f\n  FOR EACH i xs\n  NEXT\nEND FUNC\n").is_err());
    // WHILE without END WHILE.
    assert!(try_parse("FUNC f\n  WHILE TRUE\n    x = 1\nEND FUNC\n").is_err());
    // DO without LOOP.
    assert!(try_parse("FUNC f\n  DO\n    x = 1\nEND FUNC\n").is_err());
}

#[test]
fn expression_errors() {
    // Only identifiers can be called.
    assert!(try_parse("FUNC f AS Integer\n  RETURN (1)(2)\nEND FUNC\n").is_err());
    // Only identifiers can be constructors.
    assert!(try_parse("FUNC f AS Integer\n  RETURN (1)[2]\nEND FUNC\n").is_err());
    // WITH without brace.
    assert!(try_parse("FUNC f AS Integer\n  RETURN WITH x y\nEND FUNC\n").is_err());
    // Member access without identifier.
    assert!(try_parse("FUNC f AS Integer\n  RETURN x.\nEND FUNC\n").is_err());
    // Unterminated grouping.
    assert!(try_parse("FUNC f AS Integer\n  RETURN (1\nEND FUNC\n").is_err());
    // Map literal missing TO.
    assert!(
        try_parse("FUNC f AS Integer\n  RETURN Map OF String Integer { }\nEND FUNC\n").is_err()
    );
    // Bare expression that is not recognized.
    assert!(try_parse("FUNC f\n  *\nEND FUNC\n").is_err());
    // Three-part qualified name.
    assert!(try_parse("FUNC f AS Integer\n  RETURN a::b::c\nEND FUNC\n").is_err());
}

// ---------------------------------------------------------------------------
// types.rs — DocHeaderKind / DocProseKind helper methods.
// ---------------------------------------------------------------------------

#[test]
fn doc_header_kind_keywords() {
    assert_eq!(DocHeaderKind::Func.keyword(), "FUNC");
    assert_eq!(DocHeaderKind::Type.keyword(), "TYPE");
    assert_eq!(DocHeaderKind::Union.keyword(), "UNION");
    assert_eq!(DocHeaderKind::Enum.keyword(), "ENUM");
    assert_eq!(DocHeaderKind::Package.keyword(), "PACKAGE");
}

#[test]
fn doc_prose_kind_roundtrips_all_variants() {
    for (kw, kind, code, label) in [
        ("DESC", DocProseKind::Desc, 0u8, "desc"),
        ("warn", DocProseKind::Warn, 1, "warn"),
        ("Info", DocProseKind::Info, 2, "info"),
        ("SEC", DocProseKind::Sec, 3, "sec"),
    ] {
        assert_eq!(DocProseKind::from_keyword(kw), Some(kind));
        assert_eq!(kind.code(), code);
        assert_eq!(DocProseKind::from_code(code), kind);
        assert_eq!(kind.label(), label);
    }
    assert_eq!(DocProseKind::from_keyword("nope"), None);
    // Unknown code falls back to Desc.
    assert_eq!(DocProseKind::from_code(99), DocProseKind::Desc);
}

// ---------------------------------------------------------------------------
// lexical.rs — qualified member after `::` may be a keyword or a
// number-adjacent identifier (e.g. `blake::2b`).
// ---------------------------------------------------------------------------

#[test]
fn qualified_member_may_be_keyword_or_numeric() {
    // A keyword in the member position (`STEP` reserved word) via
    // consume_name_or_keyword.
    let file = parse_file("FUNC main AS Integer\n  RETURN hash::step\nEND FUNC\n");
    let f = function(&file, "main");
    let Statement::Return {
        value: Some(Expression::Identifier(name)),
        ..
    } = &f.body[0]
    else {
        panic!("identifier");
    };
    assert_eq!(name, "hash.step");

    // A number-adjacent identifier member (`2b`) via consume_numeric_identifier_part.
    let file = parse_file("FUNC main AS Integer\n  RETURN blake::2b\nEND FUNC\n");
    let f = function(&file, "main");
    let Statement::Return {
        value: Some(Expression::Identifier(name)),
        ..
    } = &f.body[0]
    else {
        panic!("identifier");
    };
    assert_eq!(name, "blake.2b");
}

#[test]
fn qualified_member_missing_after_colon_is_error() {
    // `::` at end of file: consume_qualified_identifier_part fails.
    assert!(try_parse("FUNC main AS Integer\n  RETURN pkg::\nEND FUNC\n").is_err());
}

#[test]
fn qualified_numeric_member_edge_cases() {
    // A `::` member that is a bare number not followed by an adjacent identifier
    // falls back to consume_name_or_keyword, which rejects the number.
    assert!(try_parse("FUNC f AS Integer\n  RETURN pkg::2 + 1\nEND FUNC\n").is_err());
    // A number and identifier separated by whitespace are not fused (not adjacent).
    assert!(try_parse("FUNC f AS Integer\n  RETURN pkg::2 b\nEND FUNC\n").is_err());
}

// ---------------------------------------------------------------------------
// items.rs — DOC block structural parsing, LINK / native FUNC / ABI / FREE /
// CONST error and edge paths, and malformed ABI / FREE-ABI / native param
// error paths.
// ---------------------------------------------------------------------------

#[test]
fn doc_block_prose_kinds_and_continuation() {
    // INFO/SEC prose, multi-line continuation, blank-line flush, a kind switch.
    let src = r#"
DOC
FUNC f
DESC First line
DESC continues here
INFO an info note
SEC a security note
WARN warning

DESC after a blank
END DOC
FUNC f AS Integer
  RETURN 0
END FUNC
"#;
    let json = project_json(src);
    assert!(json.contains("\"kind\": \"info\""));
    assert!(json.contains("\"kind\": \"sec\""));
    assert!(json.contains("\"kind\": \"warn\""));
    // Continued DESC lines joined with a space.
    assert!(json.contains("First line continues here"));
    assert!(json.contains("after a blank"));
}

#[test]
fn doc_block_header_variants_and_errors() {
    // PACKAGE header takes no name.
    assert!(try_parse("DOC\nPACKAGE\nDESC ok\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n").is_ok());
    // PACKAGE with a name is rejected.
    assert!(try_parse("DOC\nPACKAGE thing\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n").is_err());
    // A FUNC header with no name is rejected.
    assert!(try_parse("DOC\nFUNC\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n").is_err());
    // Unknown header keyword.
    assert!(try_parse("DOC\nWIDGET x\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n").is_err());
    // Empty DOC (no header line) is rejected.
    assert!(try_parse("DOC\n\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n").is_err());
    // A TYPE header with a signature disambiguator is fine (non-callable path).
    assert!(
        try_parse("DOC\nTYPE Widget\nDESC ok\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n").is_ok()
    );
}

#[test]
fn doc_block_named_line_errors_and_unknown() {
    // ARG with no name.
    assert!(try_parse("DOC\nFUNC f\nARG\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n").is_err());
    // PROP with no name.
    assert!(try_parse("DOC\nTYPE T\nPROP\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n").is_err());
    // ERROR with no code.
    assert!(try_parse("DOC\nFUNC f\nERROR\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n").is_err());
    // Unknown line keyword.
    assert!(
        try_parse("DOC\nFUNC f\nFLARB nope\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n").is_err()
    );
    // Unterminated EXAMPLE (no END EXAMPLE before END DOC).
    assert!(try_parse(
        "DOC\nFUNC f\nEXAMPLE\n  code here\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n"
    )
    .is_err());
}

#[test]
fn doc_func_header_signature_forms() {
    // A header with a function-type param whose commas are inside parens must not
    // be split (exercises parse_header_signature nesting).
    let src = r#"
DOC
FUNC apply(FUNC(Integer, Integer) AS Integer, List OF Integer)
DESC ok
END DOC
FUNC apply AS Integer
  RETURN 0
END FUNC
"#;
    let json = project_json(src);
    assert!(json.contains("FUNC(Integer, Integer) AS Integer"));
    // An empty parameter list `f()` yields an empty signature array.
    let json = project_json("DOC\nFUNC g()\nDESC ok\nEND DOC\nFUNC main AS Nothing\nEND FUNC\n");
    assert!(json.contains("\"signature\": []"));
}

#[test]
fn link_block_errors() {
    // LINK without a library string.
    assert!(try_parse("LINK foo AS l\nEND LINK\n").is_err());
    // LINK without AS.
    assert!(try_parse("LINK \"x\" l\nEND LINK\n").is_err());
    // LINK without alias.
    assert!(try_parse("LINK \"x\" AS\nEND LINK\n").is_err());
    // A non-FUNC statement inside LINK.
    assert!(try_parse("LINK \"x\" AS l\n  LET y = 1\nEND LINK\n").is_err());
    // Unterminated LINK.
    assert!(try_parse("LINK \"x\" AS l\n").is_err());
}

#[test]
fn native_func_missing_symbol_or_abi() {
    // Missing SYMBOL.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f(a AS Integer) AS Integer\n    ABI (a CInt32) AS r CInt32\n  END FUNC\nEND LINK\n"
    )
    .is_err());
    // Missing ABI.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f(a AS Integer) AS Integer\n    SYMBOL \"s\"\n  END FUNC\nEND LINK\n"
    )
    .is_err());
    // Unknown clause in native FUNC body.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f AS Integer\n    SYMBOL \"s\"\n    ABI () AS r CInt32\n    BOGUS 1\n  END FUNC\nEND LINK\n"
    )
    .is_err());
    // END must name FUNC.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f AS Integer\n    SYMBOL \"s\"\n    ABI () AS r CInt32\n  END TYPE\nEND LINK\n"
    )
    .is_err());
}

#[test]
fn native_func_error_on_and_return_and_const() {
    // ERROR_ON is stored as a NOT of SUCCESS_ON; CONST pin; an OUT slot named by
    // `RETURN` (plan-50-H — the slot name is ordinary, not magic).
    let src = r#"
LINK "x" AS l
  FUNC f(a AS Integer) AS Integer
    SYMBOL "s"
    ABI (a CInt32, produced OUT CInt32) AS r CInt32
    CONST a = 7
    ERROR_ON r <> 0
    RETURN r
  END FUNC
END LINK
"#;
    let json = project_json(src);
    // ERROR_ON becomes a NOT unary in successOn.
    assert!(json.contains("\"kind\": \"unary\", \"operator\": \"NOT\""));
    assert!(json.contains("\"slot\": \"a\""));
    // The OUT slot serializes with its own name, and is marked out.
    assert!(json.contains("\"name\": \"produced\", \"ctype\": \"CInt32\", \"out\": true"));
}

#[test]
fn native_func_free_block_and_errors() {
    // A well-formed FREE block.
    let src = r#"
LINK "x" AS l
  FUNC f() AS RES Db
    SYMBOL "s"
    ABI (produced OUT CPtr) AS r CInt32
    FREE produced
      SYMBOL "free_it"
      ABI (ptr CPtr) AS CVoid
    END FREE
  END FUNC
END LINK
"#;
    assert!(try_parse(src).is_ok());
    // FREE with an unknown clause.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f() AS RES Db\n    SYMBOL \"s\"\n    ABI (produced OUT CPtr) AS r CInt32\n    FREE produced\n      NONSENSE 1\n    END FREE\n  END FUNC\nEND LINK\n"
    )
    .is_err());
    // FREE END must name FREE.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f() AS RES Db\n    SYMBOL \"s\"\n    ABI (produced OUT CPtr) AS r CInt32\n    FREE produced\n      SYMBOL \"g\"\n      ABI (ptr CPtr) AS CVoid\n    END FUNC\n  END FUNC\nEND LINK\n"
    )
    .is_err());
}

#[test]
fn resource_declaration_errors() {
    // Missing CLOSE.
    assert!(try_parse("RESOURCE Db foo\n").is_err());
    // CLOSE not followed by BY.
    assert!(try_parse("RESOURCE Db CLOSE foo\n").is_err());
    // Missing close op after BY.
    assert!(try_parse("RESOURCE Db CLOSE BY\n").is_err());
    // Missing name.
    assert!(try_parse("RESOURCE CLOSE BY x::y\n").is_err());
}

#[test]
fn func_alias_errors() {
    // Missing AS after the alias name.
    assert!(try_parse("FUNC alias foo::bar\n").is_err());
}

#[test]
fn abi_spec_malformed_paths() {
    // ABI missing opening `(`.
    assert!(try_parse(&link_fn("    SYMBOL \"s\"\n    ABI a CInt32\n")).is_err());
    // ABI missing closing `)`.
    assert!(try_parse(&link_fn("    SYMBOL \"s\"\n    ABI (a CInt32\n")).is_err());
    // ABI missing AS after slot list.
    assert!(try_parse(&link_fn("    SYMBOL \"s\"\n    ABI (a CInt32) r CInt32\n")).is_err());
    // ABI slot missing its C type.
    assert!(try_parse(&link_fn("    SYMBOL \"s\"\n    ABI (a)\n")).is_err());
    // A native FUNC param list left unclosed.
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC f(a AS Integer\n    SYMBOL \"s\"\n  END FUNC\nEND LINK\n"
    )
    .is_err());
}

#[test]
fn free_block_malformed_abi_paths() {
    let head = "LINK \"x\" AS l\n  FUNC f() AS RES Db\n    SYMBOL \"s\"\n    ABI (produced OUT CPtr) AS r CInt32\n";
    let tail = "  END FUNC\nEND LINK\n";
    // FREE ABI missing `(`.
    assert!(try_parse(&format!(
        "{head}    FREE produced\n      SYMBOL \"g\"\n      ABI ptr CPtr\n    END FREE\n{tail}"
    ))
    .is_err());
    // FREE ABI missing `)`.
    assert!(try_parse(&format!(
        "{head}    FREE produced\n      SYMBOL \"g\"\n      ABI (ptr CPtr\n    END FREE\n{tail}"
    ))
    .is_err());
    // FREE ABI missing AS.
    assert!(try_parse(&format!(
        "{head}    FREE produced\n      SYMBOL \"g\"\n      ABI (ptr CPtr) CVoid\n    END FREE\n{tail}"
    ))
    .is_err());
    // FREE missing SYMBOL must be diagnosed (bug-90): silently dropping the
    // block (`free = None`) let the declared deallocator vanish and leaked the
    // native return buffer on every call.
    assert!(try_parse(&format!(
        "{head}    FREE produced\n      ABI (ptr CPtr) AS CVoid\n    END FREE\n{tail}"
    ))
    .is_err());
    // FREE missing ABI must likewise be diagnosed, not silently dropped.
    assert!(try_parse(&format!(
        "{head}    FREE produced\n      SYMBOL \"g\"\n    END FREE\n{tail}"
    ))
    .is_err());
}

#[test]
fn const_pin_errors() {
    // CONST with no slot name.
    assert!(try_parse(&link_fn(
        "    SYMBOL \"s\"\n    ABI () AS r CInt32\n    CONST = 1\n"
    ))
    .is_err());
    // CONST with no `=`.
    assert!(try_parse(&link_fn(
        "    SYMBOL \"s\"\n    ABI () AS r CInt32\n    CONST a 1\n"
    ))
    .is_err());
}

#[test]
fn doc_block_bare_prose_keyword_flushes() {
    // A bare `DESC` line (no text) flushes the current prose block; the following
    // `DESC text` starts a fresh one.
    let src = r#"
DOC
FUNC f
DESC one
DESC
DESC two
END DOC
FUNC f AS Integer
  RETURN 0
END FUNC
"#;
    let json = project_json(src);
    // Two separate desc entries (the bare DESC split them).
    assert!(json.contains("\"text\": \"one\""));
    assert!(json.contains("\"text\": \"two\""));
}

#[test]
fn doc_example_dedent_handles_multibyte_whitespace() {
    // bug-19: `dedent` measured indentation in BYTES but sliced every line at the
    // byte minimum. `trim_start` is Unicode-whitespace-aware, so an EXAMPLE mixing
    // a one-byte space with a two-byte NBSP (U+00A0) put the minimum (1) inside
    // the NBSP line's first char and panicked "byte index 1 is not a char
    // boundary", aborting the whole compile. Indentation is now a CHAR prefix.
    let src =
        "DOC\nFUNC foo()\nEXAMPLE\n a\n\u{a0}\u{a0}b\nEND EXAMPLE\nEND DOC\n\nFUNC foo() AS Nothing\nEND FUNC\n";
    let json = project_json(src);
    // One char stripped from each line: " a" -> "a", "\u{a0}\u{a0}b" -> "\u{a0}b".
    assert!(json.contains("\"example\""), "doc block parsed: {json}");
    assert!(json.contains("a\\n\u{a0}b"), "dedent by one char: {json}");

    // All-ASCII indentation is unchanged (the overwhelmingly common case).
    let ascii = project_json(
        "DOC\nFUNC foo()\nEXAMPLE\n    a\n      b\nEND EXAMPLE\nEND DOC\n\nFUNC foo() AS Nothing\nEND FUNC\n",
    );
    assert!(ascii.contains("a\\n  b"), "ascii dedent: {ascii}");
}

// ---------------------------------------------------------------------------
// stmt.rs — inline-trap propagation, match/for/do edge and error paths.
// ---------------------------------------------------------------------------

#[test]
fn inline_trap_error_paths() {
    // A bare-expression statement whose inline TRAP has no binding.
    assert!(try_parse("FUNC f\n  notify() TRAP()\n  END TRAP\nEND FUNC\n").is_err());
    // An inline TRAP missing its closing `)`.
    assert!(try_parse("FUNC f\n  LET x = risky() TRAP(e\n  END TRAP\nEND FUNC\n").is_err());
    // An inline TRAP that is well-formed on a bare expression statement.
    let json = project_json("FUNC f\n  risky() TRAP(e)\n    notify()\n  END TRAP\nEND FUNC\n");
    assert!(json.contains("\"kind\": \"trapped\""));
    // plan-37: a bare inline TRAP (no `(e)` binding) parses to the same
    // `trapped` node, with the synthesized `#err` binding.
    let json = project_json("FUNC f AS Integer\n  LET n = risky() TRAP\n    RECOVER 0\n  END TRAP\n  RETURN n\nEND FUNC\n");
    assert!(json.contains("\"kind\": \"trapped\""));
    assert!(json.contains("\"binding\": \"#err\""));
    // A state assignment with an inline trap.
    let json = project_json(
        "FUNC f(h AS File STATE Integer)\n  h.state = risky() TRAP(e)\n    notify()\n  END TRAP\nEND FUNC\n",
    );
    assert!(json.contains("\"kind\": \"stateAssignment\""));
}

#[test]
fn deeply_nested_inline_traps_hit_the_statement_depth_cap() {
    // bug-289: a handler statement carrying its own postfix trap re-enters
    // `parse_statement` through `maybe_attach_postfix_trap`, a recursion funnel
    // the `MAX_STMT_DEPTH` latch did not count — thousands of levels aborted the
    // compiler with a native stack overflow and zero diagnostics. The cap is 256,
    // so a nesting depth well past it must unwind as an ordinary parse error.
    // Parsing (and dropping) a legal 200-deep nest is itself recursive and needs
    // more than a test thread's default stack, so run on a generous one; the
    // parser's own cap is what we assert.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let nest = |depth: usize| {
                let mut src = String::from("FUNC f AS Integer\n");
                for index in 0..depth {
                    src.push_str(&format!("  LET x{index} = risky() TRAP\n"));
                }
                for _ in 0..depth {
                    src.push_str("  END TRAP\n");
                }
                src.push_str("  RETURN 0\nEND FUNC\n");
                src
            };
            // Within the cap this still parses, so the guard did not shrink the
            // usable nesting budget.
            assert!(try_parse(&nest(200)).is_ok());
            // Past the cap, and far past the depth that used to overflow.
            assert!(try_parse(&nest(4000)).is_err());
        })
        .expect("spawn")
        .join()
        .expect("deep inline-trap nesting must not overflow the stack");
}

#[test]
fn match_union_pattern_errors() {
    // Union CASE pattern missing `(`.
    assert!(
        try_parse("FUNC f\n  MATCH x\n    CASE Shape y\n      z = 1\n  END MATCH\nEND FUNC\n")
            .is_err()
    );
    // Union CASE pattern missing closing `)`.
    assert!(
        try_parse("FUNC f\n  MATCH x\n    CASE Shape(y\n      z = 1\n  END MATCH\nEND FUNC\n")
            .is_err()
    );
    // MATCH not closed with END MATCH.
    assert!(try_parse("FUNC f\n  MATCH x\n    CASE 1\n      z = 1\nEND FUNC\n").is_err());
}

#[test]
fn if_elseif_error_paths() {
    // ELSEIF with a bad condition.
    assert!(try_parse(
        "FUNC f\n  IF a THEN\n    x = 1\n  ELSEIF *\n    x = 2\n  END IF\nEND FUNC\n"
    )
    .is_err());
    // ELSEIF missing THEN.
    assert!(try_parse(
        "FUNC f\n  IF a THEN\n    x = 1\n  ELSEIF b\n    x = 2\n  END IF\nEND FUNC\n"
    )
    .is_err());
    // A full IF / ELSEIF / ELSE chain parses.
    let json = project_json(
        "FUNC f\n  IF a THEN\n    x = 1\n  ELSEIF b THEN\n    x = 2\n  ELSE\n    x = 3\n  END IF\nEND FUNC\n",
    );
    assert!(json.contains("\"kind\": \"if\""));
}

#[test]
fn loop_close_errors() {
    // FOR block not closed with NEXT.
    assert!(try_parse("FUNC f\n  FOR i = 1 TO 2\n    x = 1\nEND FUNC\n").is_err());
    // FOR EACH block not closed with NEXT.
    assert!(try_parse("FUNC f\n  FOR EACH i IN xs\n    x = 1\nEND FUNC\n").is_err());
    // DO WHILE block not closed with LOOP.
    assert!(try_parse("FUNC f\n  DO WHILE a\n    x = 1\nEND FUNC\n").is_err());
    // DO block closed with LOOP but missing UNTIL.
    assert!(try_parse("FUNC f\n  DO\n    x = 1\n  LOOP\nEND FUNC\n").is_err());
}

#[test]
fn enum_multiple_member_lines() {
    // Two separate member lines exercise the enum-member loop across statements.
    let json = project_json("ENUM E\n  A, B\n  C\nEND ENUM\n");
    assert!(json.contains("\"name\": \"A\""));
    assert!(json.contains("\"name\": \"C\""));
}

#[test]
fn pipeline_placeholder_each_expression_kind_as_rhs() {
    // Each pipeline RHS is a *single* expression whose sole placeholder forces
    // contains_placeholder to visit that specific match arm (they short-circuit,
    // so one placeholder per RHS is required to reach every arm).
    let cases = [
        "x |> -_",                                   // Unary
        "x |> _ + 1",                                // Binary
        "x |> [_]",                                  // ListLiteral
        "x |> Point[_]",                             // Constructor
        "x |> LAMBDA() -> _",                        // Lambda
        "x |> Map OF Integer TO Integer { 1 := _ }", // MapLiteral (value)
        "x |> Map OF Integer TO Integer { _ := 1 }", // MapLiteral (key)
        "x |> _.field",                              // MemberAccess
        "x |> WITH base { z := _ }",                 // WithUpdate value
        "x |> g(_)",                                 // Call positional
        "x |> g(k := _)",                            // Call named
        "x |> Point[k := _]",                        // Constructor named
    ];
    for rhs in cases {
        // Wrap in parentheses so a placeholder that would otherwise be the final
        // token on the line is not consumed as a `_` line-continuation marker.
        let src = format!("FUNC f AS Integer\n  RETURN ({rhs})\nEND FUNC\n");
        assert!(try_parse(&src).is_ok(), "failed to parse: {rhs}");
    }
}

#[test]
fn lambda_and_function_type_close_errors() {
    // Lambda missing `(`.
    assert!(try_parse("FUNC f AS Integer\n  LET g = LAMBDA -> 1\n  RETURN 0\nEND FUNC\n").is_err());
    // Lambda missing `)`.
    assert!(
        try_parse("FUNC f AS Integer\n  LET g = LAMBDA(x -> 1\n  RETURN 0\nEND FUNC\n").is_err()
    );
    // Lambda missing `->`.
    assert!(try_parse("FUNC f AS Integer\n  LET g = LAMBDA(x) 1\n  RETURN 0\nEND FUNC\n").is_err());
    // Function type missing the closing `)` of its parameter list.
    assert!(try_parse("FUNC s(f AS FUNC(Integer AS Boolean)\nEND FUNC\n").is_err());
}

#[test]
fn native_symbol_must_be_string_and_name_required() {
    // SYMBOL with a non-string operand.
    assert!(try_parse(&link_fn("    SYMBOL 123\n    ABI () AS r CInt32\n")).is_err());
    // A native FUNC with no name (FUNC immediately followed by `(`).
    assert!(try_parse(
        "LINK \"x\" AS l\n  FUNC (a AS Integer)\n    SYMBOL \"s\"\n    ABI (a CInt32) AS r CInt32\n  END FUNC\nEND LINK\n"
    )
    .is_err());
}

// ---------------------------------------------------------------------------
// TESTING / TGROUP / TCASE block parsing (src/ast/testing.rs)
// ---------------------------------------------------------------------------

#[test]
fn parse_testing_block_with_nested_groups_and_cases() {
    // A top-level TESTING block: an outer group with a direct case and a nested
    // sub-group holding its own case. Exercises parse_testing_block /
    // parse_test_group / parse_test_case plus the contextual END matching.
    let file = parse_file(
        "TESTING\n\
         \x20 TGROUP \"outer\"\n\
         \x20   TCASE \"one\"\n\
         \x20     x = 1\n\
         \x20   END TCASE\n\
         \x20   TGROUP \"inner\"\n\
         \x20     TCASE \"two\"\n\
         \x20       y = 2\n\
         \x20     END TCASE\n\
         \x20   END TGROUP\n\
         \x20 END TGROUP\n\
         END TESTING\n",
    );
    let Item::Testing(block) = &file.items[0] else {
        panic!("expected a TESTING item, got {:?}", file.items);
    };
    assert_eq!(block.groups.len(), 1);
    let outer = &block.groups[0];
    assert_eq!(outer.description, "outer");
    assert_eq!(outer.members.len(), 2);
    // First member is the direct TCASE.
    let TestGroupMember::Case(case) = &outer.members[0] else {
        panic!("expected a TCASE first");
    };
    assert_eq!(case.description, "one");
    assert_eq!(case.body.len(), 1);
    // Second member is the nested TGROUP with its own case.
    let TestGroupMember::Group(inner) = &outer.members[1] else {
        panic!("expected a nested TGROUP second");
    };
    assert_eq!(inner.description, "inner");
    assert_eq!(inner.members.len(), 1);
    assert!(matches!(inner.members[0], TestGroupMember::Case(_)));
}

#[test]
fn parse_testing_rejects_a_non_group_member() {
    // A TESTING block may contain only TGROUP groups; a bare statement is an error.
    assert!(try_parse("TESTING\n  x = 1\nEND TESTING\n").is_err());
}

#[test]
fn parse_tgroup_rejects_a_non_case_member() {
    // A TGROUP may contain only TCASE cases and nested TGROUP groups.
    assert!(try_parse("TESTING\n  TGROUP \"g\"\n    x = 1\n  END TGROUP\nEND TESTING\n").is_err());
}

#[test]
fn parse_tgroup_requires_a_string_description() {
    // A missing TGROUP description is reported by consume_test_description.
    assert!(try_parse("TESTING\n  TGROUP\n  END TGROUP\nEND TESTING\n").is_err());
}

#[test]
fn parse_tcase_requires_a_string_description() {
    // A missing TCASE description is likewise rejected.
    assert!(try_parse(
        "TESTING\n  TGROUP \"g\"\n    TCASE\n    END TCASE\n  END TGROUP\nEND TESTING\n"
    )
    .is_err());
}

#[test]
fn parse_tcase_without_end_tcase_is_rejected() {
    // The case body terminates at the enclosing END TGROUP, so the missing
    // END TCASE is reported rather than the parser running away.
    assert!(try_parse(
        "TESTING\n  TGROUP \"g\"\n    TCASE \"c\"\n      x = 1\n  END TGROUP\nEND TESTING\n"
    )
    .is_err());
}

// ---- bug-468: `record.field = value` in statement position ----
// MFBASIC spells assignment and equality both `=` and records are `WITH`-only
// (§4), so before the parser guard these statements fell through to
// `parse_expression` and the `=` bound as the equality operator: the write
// became a Boolean comparison that was computed and discarded, with no
// diagnostic at all whenever the two sides were comparable.

#[test]
fn parse_rejects_a_record_field_assignment_statement() {
    assert!(try_parse("FUNC main() AS Nothing\n  p.x = 77\nEND FUNC\n").is_err());
}

#[test]
fn parse_rejects_a_nested_record_field_assignment_statement() {
    assert!(try_parse("FUNC main() AS Nothing\n  o.inner.x = 77\nEND FUNC\n").is_err());
}

#[test]
fn parse_rejects_a_record_field_assignment_in_an_inline_if_branch() {
    // The inline-IF branch is statement position too, reached through
    // `parse_simple_statement(true)`.
    assert!(try_parse("FUNC main() AS Nothing\n  IF c THEN p.x = 77\nEND FUNC\n").is_err());
}

#[test]
fn parse_keeps_the_state_member_assignment_forms() {
    // `resource.state = value` and `resource.state.field = value` are the one
    // member-target assignment in the language (§15) and must be untouched by
    // the record-field guard. The nested form desugars to a single-field
    // `WITH` over the current state.
    let file = try_parse("FUNC main() AS Nothing\n  f.state = c\n  f.state.pos = 10\nEND FUNC\n")
        .expect("state assignment forms still parse");
    let Item::Function(function) = &file.items[0] else {
        panic!("expected function item");
    };
    let Statement::StateAssign {
        resource, value, ..
    } = &function.body[0]
    else {
        panic!("expected a STATE assignment for `f.state = c`");
    };
    assert_eq!(resource, "f");
    assert!(matches!(value, Expression::Identifier(name) if name == "c"));
    let Statement::StateAssign {
        resource, value, ..
    } = &function.body[1]
    else {
        panic!("expected a STATE assignment for `f.state.pos = 10`");
    };
    assert_eq!(resource, "f");
    let Expression::WithUpdate { updates, .. } = value else {
        panic!("expected the nested form to desugar to a WITH update");
    };
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].field, "pos");
}

#[test]
fn parse_keeps_a_member_comparison_in_expression_position() {
    // `IF p.x = 77 THEN` is a genuine equality comparison — the guard fires only
    // in statement position, so this must still parse as a `=` binary.
    let file =
        try_parse("FUNC main() AS Nothing\n  IF p.x = 77 THEN\n    q = 1\n  END IF\nEND FUNC\n")
            .expect("member comparison in a condition still parses");
    let Item::Function(function) = &file.items[0] else {
        panic!("expected function item");
    };
    let Statement::If { condition, .. } = &function.body[0] else {
        panic!("expected an IF statement");
    };
    let Expression::Binary { left, operator, .. } = condition else {
        panic!("expected a binary condition");
    };
    assert_eq!(*operator, BinaryOp::Equal);
    assert!(matches!(**left, Expression::MemberAccess { .. }));
}

#[test]
fn parse_keeps_a_plain_identifier_assignment() {
    // The single-identifier assignment shape is unaffected by the guard.
    let file = try_parse("FUNC main() AS Nothing\n  p = 77\nEND FUNC\n")
        .expect("plain assignment still parses");
    let Item::Function(function) = &file.items[0] else {
        panic!("expected function item");
    };
    assert!(matches!(&function.body[0], Statement::Assign { name, .. } if name == "p"));
}

#[test]
fn parse_leaves_a_malformed_member_path_to_its_own_diagnostic() {
    // `a.b. = c` runs out of identifiers mid-chain, so the record-field guard
    // declines it and the ordinary syntax diagnostic reports the statement.
    assert!(try_parse("FUNC main() AS Nothing\n  a.b. = c\nEND FUNC\n").is_err());
}

// ---- an inline IF ends at its line: an ELSE on the next line is the enclosing block IF's ----
// `consume_simple_statement_end` skipped the newline after an inline IF's THEN
// statement, so `parse_if_statement` then claimed the enclosing block's ELSE as
// the inline IF's own, and the statement it expected after that ELSE was the
// newline: "parser expected an expression" on valid code (§10 gives an inline IF
// only a same-line ELSE).

#[test]
fn parse_gives_a_next_line_else_to_the_enclosing_block_if() {
    let file = try_parse(
        "FUNC main() AS Nothing\n  IF a THEN\n    IF b THEN n = 5\n  ELSE\n    n = 7\n  END IF\nEND FUNC\n",
    )
    .expect("an inline IF followed by the block's ELSE parses");
    let Item::Function(function) = &file.items[0] else {
        panic!("expected function item");
    };
    let Statement::If {
        then_body,
        else_body,
        ..
    } = &function.body[0]
    else {
        panic!("expected the block IF");
    };
    assert_eq!(then_body.len(), 1);
    let Statement::If {
        else_body: inner_else,
        ..
    } = &then_body[0]
    else {
        panic!("expected the inline IF inside the THEN block");
    };
    assert!(
        inner_else.is_empty(),
        "the inline IF has no ELSE of its own"
    );
    assert_eq!(else_body.len(), 1, "the block IF keeps its ELSE branch");
}

#[test]
fn parse_still_binds_a_same_line_else_to_the_inline_if() {
    let file = try_parse("FUNC main() AS Nothing\n  IF b THEN n = 5 ELSE n = 7\nEND FUNC\n")
        .expect("inline IF with a same-line ELSE parses");
    let Item::Function(function) = &file.items[0] else {
        panic!("expected function item");
    };
    let Statement::If {
        then_body,
        else_body,
        ..
    } = &function.body[0]
    else {
        panic!("expected the inline IF");
    };
    assert_eq!(then_body.len(), 1);
    assert_eq!(else_body.len(), 1);
}

/// bug-976: a parameter carries no `RES` — its type already makes it a resource.
/// The keyword is a located parse error in a FUNC, a LINK FUNC and a LAMBDA
/// parameter list, and `STATE` parses after any parameter type (whether the type
/// is a resource is decided after the parse).
#[test]
fn a_res_parameter_keyword_is_a_parse_error_and_state_needs_no_keyword() {
    for source in [
        "IMPORT fs\nFUNC f(RES h AS fs::File) AS Nothing\nEND FUNC\n",
        "IMPORT fs\nFUNC f(n AS Integer, RES h AS fs::File) AS Nothing\nEND FUNC\n",
        "IMPORT fs\nFUNC f AS Nothing\n  LET p = LAMBDA(RES h AS fs::File) -> TRUE\nEND FUNC\n",
        "RESOURCE Db CLOSE BY l::close\nLINK \"x\" AS l\n  FUNC close(RES db AS Db) AS Nothing\n    SYMBOL \"x_close\"\n    ABI (db CPtr) AS st CInt32\n  END FUNC\nEND LINK\n",
    ] {
        assert!(
            crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", source)
                .is_err(),
            "`RES` on a parameter must not parse: {source}"
        );
    }
    let json = project_json(
        "TYPE Cur\n  pos AS Integer\nEND TYPE\nFUNC f(h AS File STATE Cur, n AS Integer) AS Nothing\nEND FUNC\n",
    );
    assert!(json.contains("\"name\": \"h\", \"type\": \"File\", \"state\": \"Cur\""));
    assert!(!json.contains("\"resource\": true, \"state\": null"));
}
