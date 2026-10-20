use super::*;

fn fmt(source: &str) -> String {
    format_source(source, 2).expect("well-nested source formats")
}

// ---- Nesting cap (audit-3 FE-02 / bug-502) ----

/// `FUNC main() AS Nothing` plus `opens` nested `IF TRUE THEN` lines (unterminated on
/// purpose: the cap must trip on the way IN, before any output is built).
fn nested_ifs(opens: usize) -> String {
    let mut source = String::from("FUNC main() AS Nothing\n");
    for _ in 0..opens {
        source.push_str("IF TRUE THEN\n");
    }
    source
}

#[test]
fn nesting_at_the_cap_still_formats() {
    // `FUNC main` is frame 1, so MAX_NESTING_DEPTH - 1 `IF`s fill the stack exactly.
    let source = nested_ifs(MAX_NESTING_DEPTH - 1);
    let formatted = format_source(&source, 2).expect("a stack exactly at the cap formats");
    // Bounded output: at most depth × width bytes of indentation per line.
    assert!(formatted.len() <= source.len() + MAX_NESTING_DEPTH * MAX_NESTING_DEPTH * 2);
}

#[test]
fn nesting_past_the_cap_is_refused_at_the_crossing_line() {
    // The IF on line MAX_NESTING_DEPTH + 1 opens frame MAX_NESTING_DEPTH + 1.
    let source = nested_ifs(MAX_NESTING_DEPTH);
    assert_eq!(
        format_source(&source, 2),
        Err(FormatError::NestingTooDeep {
            line: MAX_NESTING_DEPTH + 1
        })
    );
    // A 2 000-deep tower (the 40 KB → 8 MB shape) is refused just the same,
    // at the same line, having built nothing past it.
    assert_eq!(
        format_source(&nested_ifs(2000), 2),
        Err(FormatError::NestingTooDeep {
            line: MAX_NESTING_DEPTH + 1
        })
    );
}

#[test]
fn link_block_nesting_past_the_cap_is_refused() {
    // Inside `LINK` the depth starts at 1 and every `FUNC` opener adds one,
    // so the FUNC on line MAX_NESTING_DEPTH + 1 crosses the cap.
    let mut source = String::from("LINK \"c\" AS c\n");
    for _ in 0..MAX_NESTING_DEPTH {
        source.push_str("FUNC f\n");
    }
    assert_eq!(
        format_source(&source, 2),
        Err(FormatError::NestingTooDeep {
            line: MAX_NESTING_DEPTH + 1
        })
    );
    let mut at_cap = String::from("LINK \"c\" AS c\n");
    for _ in 0..MAX_NESTING_DEPTH - 1 {
        at_cap.push_str("FUNC f\n");
    }
    assert!(format_source(&at_cap, 2).is_ok());
}

#[test]
fn format_error_display_names_the_line_and_cap() {
    let message = FormatError::NestingTooDeep { line: 7 }.to_string();
    assert!(message.contains("line 7"), "{message}");
    assert!(
        message.contains(&MAX_NESTING_DEPTH.to_string()),
        "{message}"
    );
}

#[test]
fn reindents_nested_blocks_and_uppercases_keywords() {
    let input = "func main as Integer\nlet x = 1\nif x > 0 then\nio::print(\"pos\")\nend if\nreturn 0\nend func\n";
    let expected = "FUNC main AS Integer\n  LET x = 1\n  IF x > 0 THEN\n    io::print(\"pos\")\n  END IF\n  RETURN 0\nEND FUNC\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn match_case_indentation_matches_spec() {
    let input = "FUNC area(s AS Shape) AS Float\nMATCH s\nCASE Circle(c) : RETURN c.radius\nCASE Rect(r) : RETURN r.w\nEND MATCH\nEND FUNC\n";
    let expected = "FUNC area(s AS Shape) AS Float\n  MATCH s\n    CASE Circle(c) : RETURN c.radius\n    CASE Rect(r) : RETURN r.w\n  END MATCH\nEND FUNC\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn elseif_chain_dedents() {
    let input = "IF a THEN\nx\nELSEIF b THEN\ny\nELSE\nz\nEND IF\n";
    let expected = "IF a THEN\n  x\nELSEIF b THEN\n  y\nELSE\n  z\nEND IF\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn field_and_package_names_keep_case() {
    // `next`, `to`, `step` are keywords but here name a field, a package
    // member, and (after `::`) a function; none should be uppercased.
    let input = "LET a = node.next\nLET b = pkg::step\n";
    assert_eq!(fmt(input), input);
}

#[test]
fn single_line_if_does_not_indent_following_line() {
    let input = "IF x > 0 THEN io::print(\"p\") ELSE io::print(\"n\")\nlet y = 1\n";
    let expected = "IF x > 0 THEN io::print(\"p\") ELSE io::print(\"n\")\nLET y = 1\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn one_line_block_with_separators_is_net_zero() {
    let input = "FOR i = 1 TO 3 : io::print(toString(i)) : NEXT\nlet done = 1\n";
    let expected = "FOR i = 1 TO 3 : io::print(toString(i)) : NEXT\nLET done = 1\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn exit_and_continue_targets_do_not_open_blocks() {
    let input = "DO\nEXIT DO\nCONTINUE DO\nLOOP UNTIL done\nlet after = 1\n";
    let expected = "DO\n  EXIT DO\n  CONTINUE DO\nLOOP UNTIL done\nLET after = 1\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn named_exit_and_continue_targets_do_not_open_blocks() {
    // `FOR` after `EXIT`/`CONTINUE` is a target, and the variable after it is an
    // identifier: neither opens a block nor shifts the nested loops' indentation.
    let input =
        "FOR i = 1 TO 3\nFOR j = 1 TO 3\nEXIT FOR i\nCONTINUE FOR i\nNEXT\nNEXT\nLET done = 1\n";
    let expected =
        "FOR i = 1 TO 3\n  FOR j = 1 TO 3\n    EXIT FOR i\n    CONTINUE FOR i\n  NEXT\nNEXT\nLET done = 1\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn doc_block_reindents_body_preserving_text() {
    // The body indents one level under DOC; internal spacing and casing of
    // the free-form text are preserved (it is prose, not code).
    let input = "TYPE T\ndoc\nfree  form   text  IF prose\n  END DOC\nx AS Integer\nEND TYPE\n";
    let expected =
        "TYPE T\n  DOC\n    free  form   text  IF prose\n  END DOC\n  x AS Integer\nEND TYPE\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn doc_example_source_is_anchored_keeping_relative_indent() {
    let input =
        "DOC\nFUNC f\nEXAMPLE\nLET x = 1\nIF x THEN\nwork()\nEND IF\nEND EXAMPLE\nEND DOC\n";
    let expected = "DOC\n  FUNC f\n  EXAMPLE\n    LET x = 1\n    IF x THEN\n    work()\n    END IF\n  END EXAMPLE\nEND DOC\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn comments_blank_lines_and_strings_are_preserved() {
    let input = "' a comment\n\nLET s = \"IF nested THEN keyword\"   ' trailing\n";
    assert_eq!(fmt(input), input);
}

#[test]
fn line_continuation_keeps_following_lines() {
    let input = "let msg = \"hello \" & _\n          \"world\"\n";
    let expected = "LET msg = \"hello \" & _\n          \"world\"\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn inline_trap_block_indents_handler() {
    let input = "FUNC f() AS Integer\nRES x = open(p) TRAP(e)\nio::print(e.message)\nRECOVER 0\nEND TRAP\nRETURN 0\nEND FUNC\n";
    let expected = "FUNC f() AS Integer\n  RES x = open(p) TRAP(e)\n    io::print(e.message)\n    RECOVER 0\n  END TRAP\n  RETURN 0\nEND FUNC\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn bare_trap_block_indents_handler_both_positions() {
    // plan-37: a bare `TRAP` (no `(e)` binding) opens and indents its handler
    // exactly like `TRAP(e)`, in both the inline postfix and function-level
    // positions, and round-trips unchanged.
    let input = "FUNC f() AS Integer\nLET n = toInt(s) TRAP\nRECOVER 0\nEND TRAP\nRETURN n\nTRAP\nPROPAGATE\nEND TRAP\nEND FUNC\n";
    let expected = "FUNC f() AS Integer\n  LET n = toInt(s) TRAP\n    RECOVER 0\n  END TRAP\n  RETURN n\n  TRAP\n    PROPAGATE\n  END TRAP\nEND FUNC\n";
    assert_eq!(fmt(input), expected);
    // Idempotent: formatting the already-formatted output is a no-op.
    assert_eq!(fmt(expected), expected);
}

#[test]
fn do_while_opens_one_block_not_two() {
    // The `WHILE` in `DO WHILE` is a condition, not a second block opener.
    let input = "DO WHILE running\nwork()\nLOOP\nlet after = 1\n";
    let expected = "DO WHILE running\n  work()\nLOOP\nLET after = 1\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn exported_declarations_open_a_block() {
    let input = "EXPORT FUNC f() AS Integer\nRETURN 0\nEND FUNC\n";
    let expected = "EXPORT FUNC f() AS Integer\n  RETURN 0\nEND FUNC\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn custom_indent_width() {
    let input = "FUNC f()\nx\nEND FUNC\n";
    let expected = "FUNC f()\n    x\nEND FUNC\n";
    assert_eq!(format_source(input, 4).as_deref(), Ok(expected));
}

#[test]
fn nothing_is_a_type_after_as_but_a_value_elsewhere() {
    let input = "type t\nvalue AS Nothing\nend type\nLET x = nothing\n";
    let expected = "TYPE t\n  value AS Nothing\nEND TYPE\nLET x = NOTHING\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn function_type_annotation_does_not_open_a_block() {
    // The `FUNC(Integer) AS Integer` return type must not open a second block.
    let input = "FUNC makeAdder(base AS Integer) AS FUNC(Integer) AS Integer\nRETURN base\nEND FUNC\nlet x = 1\n";
    let expected = "FUNC makeAdder(base AS Integer) AS FUNC(Integer) AS Integer\n  RETURN base\nEND FUNC\nLET x = 1\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn nothing_is_a_type_after_of_and_to() {
    let input = "res t AS Thread OF Nothing TO Integer = start()\n";
    assert_eq!(
        fmt(input),
        "RES t AS Thread OF Nothing TO Integer = start()\n"
    );
}

#[test]
fn func_alias_does_not_open_a_block() {
    let input = "EXPORT FUNC close AS pkg::close\nlet after = 1\n";
    let expected = "EXPORT FUNC close AS pkg::close\nLET after = 1\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn parameterless_func_returning_a_qualified_type_opens_a_block() {
    // bug-480 settled the parse: `FUNC name AS pkg::X` is a function, not an
    // alias, when `pkg::X` names a built-in type or resource. The formatter
    // must agree, or it flattens the body to the enclosing level.
    let input = "FUNC face AS canvas::Font\nRETURN x\nEND FUNC\nFUNC tint AS color::Color\nRETURN y\nEND FUNC\nlet after = 1\n";
    let expected = "FUNC face AS canvas::Font\n  RETURN x\nEND FUNC\nFUNC tint AS color::Color\n  RETURN y\nEND FUNC\nLET after = 1\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn parameterless_func_returning_a_collection_of_a_qualified_type_opens_a_block() {
    // The `::` is not the first thing after `AS`, so this is never alias-shaped.
    let input = "FUNC items AS List OF canvas::DrawItem\nRETURN []\nEND FUNC\nlet after = 1\n";
    let expected = "FUNC items AS List OF canvas::DrawItem\n  RETURN []\nEND FUNC\nLET after = 1\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn single_line_while_inside_if_is_balanced() {
    let input = "FUNC f() AS Integer\nIF TRUE THEN RETURN 3 ELSE WHILE FALSE : END WHILE\nRETURN 0\nEND FUNC\n";
    let expected = "FUNC f() AS Integer\n  IF TRUE THEN RETURN 3 ELSE WHILE FALSE : END WHILE\n  RETURN 0\nEND FUNC\n";
    assert_eq!(fmt(input), expected);
}

/// bug-357: the `WHILE` in `END WHILE` names the block being closed, not a
/// new opener — otherwise the closer line nets to open and every line after
/// it indents one level too deep.
#[test]
fn end_while_closes_the_block_without_reopening() {
    let input = "FUNC s() AS Nothing\nWHILE go\nwork()\nEND WHILE\nlet after = 1\nEND FUNC\n";
    let expected =
        "FUNC s() AS Nothing\n  WHILE go\n    work()\n  END WHILE\n  LET after = 1\nEND FUNC\n";
    assert_eq!(fmt(input), expected);
    assert_eq!(fmt(expected), expected);
    // The single-line form round-trips on one line, netting to zero.
    let single = "WHILE FALSE : work() : END WHILE\nLET after = 1\n";
    assert_eq!(fmt(single), single);
}

/// bug-348: `TESTING`, `TGROUP` and `TCASE` open real, nestable blocks, but
/// `classify` had no arm for `TESTING` and the other two are contextual
/// identifiers that never scan as keywords -- so nothing was pushed while the
/// matching `END` lines *were* classified as `Op::End` and popped. Every line
/// of every `TESTING` block printed at column 0, which made `mfb fmt --check`
/// fail on 36 committed sources and meant running `mfb fmt` over the test tree
/// produced a large meaningless diff.
#[test]
fn testing_blocks_indent_and_are_a_fixed_point() {
    // Flattened input is restored to the authored nesting.
    let flat = concat!(
        "TESTING
",
        "TGROUP \"outer\"\n",
        "TGROUP \"inner\"\n",
        "TCASE \"c\"\n",
        "expectInteger(1, 1)\n",
        "END TCASE\n",
        "END TGROUP\n",
        "END TGROUP\n",
        "END TESTING\n",
    );
    let expected = concat!(
        "TESTING
",
        "  TGROUP \"outer\"\n",
        "    TGROUP \"inner\"\n",
        "      TCASE \"c\"\n",
        "        expectInteger(1, 1)\n",
        "      END TCASE\n",
        "    END TGROUP\n",
        "  END TGROUP\n",
        "END TESTING\n",
    );
    assert_eq!(fmt(flat), expected);
    // ...and correctly-indented input is a fixed point.
    assert_eq!(fmt(expected), expected);
}

/// The `END` lines popped a stack the openers never pushed, so a `TESTING`
/// block corrupted the indentation of everything that followed it.
#[test]
fn a_testing_block_does_not_disturb_the_code_after_it() {
    let input = concat!(
        "TESTING\n",
        "  TGROUP \"g\"\n",
        "    TCASE \"c\"\n",
        "      expectInteger(1, 1)\n",
        "    END TCASE\n",
        "  END TGROUP\n",
        "END TESTING\n",
        "\n",
        "FUNC helper() AS Integer\n",
        "  IF 1 = 1 THEN\n",
        "    RETURN 1\n",
        "  END IF\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    assert_eq!(fmt(input), input);
}

/// bug-356: `CSTRUCT … END CSTRUCT` and `BIND IN … END BIND` are block
/// constructs inside a LINK block, but `format_link_block` recognized only
/// `FUNC`/`FREE` as openers -- so both bodies were emitted at the
/// enclosing depth, silently de-indenting them one level. `BIND STATE` is a
/// single line with no body and must NOT open a block, which is why the
/// opener test is `BIND` + `IN` rather than `BIND` alone.
#[test]
fn link_cstruct_and_bind_in_blocks_indent_their_bodies() {
    let flat = concat!(
        "LINK \"snd\" AS snd\n",
        "CSTRUCT SfInfo AS AudioFormat\n",
        "frames CInt64\n",
        "END CSTRUCT\n",
        "FUNC open(p AS String) AS RES Sf\n",
        "SYMBOL \"sf_open\"\n",
        "BIND IN info\n",
        "rate = 44100\n",
        "END BIND\n",
        "BIND STATE h = info\n",
        "END FUNC\n",
        "END LINK\n",
    );
    let expected = concat!(
        "LINK \"snd\" AS snd\n",
        "  CSTRUCT SfInfo AS AudioFormat\n",
        "    frames CInt64\n",
        "  END CSTRUCT\n",
        "  FUNC open(p AS String) AS RES Sf\n",
        "    SYMBOL \"sf_open\"\n",
        "    BIND IN info\n",
        "      rate = 44100\n",
        "    END BIND\n",
        // BIND STATE has no body: it stays a sibling of BIND IN, not a child.
        "    BIND STATE h = info\n",
        "  END FUNC\n",
        "END LINK\n",
    );
    assert_eq!(fmt(flat), expected);
    assert_eq!(
        fmt(expected),
        expected,
        "correct input must be a fixed point"
    );
}

#[test]
fn link_block_reindents_from_func_free_nesting_preserving_text() {
    // Mis-indented LINK body, with a contextual `return` in the ABI line and
    // a nested FREE block — re-indented from nesting, text/casing preserved.
    let input = concat!(
        "LINK \"sqlite3\" AS link\n",
        "FUNC columnText(stmt AS Stmt) AS String\n",
        "SYMBOL \"sqlite3_column_text\"\n",
        "ABI (stmt CPtr) AS produced CPtr\n",
        "FREE produced\n",
        "SYMBOL \"sqlite3_free\"\n",
        "END FREE\n",
        "END FUNC\n",
        "END LINK\n",
        "let after = 1\n",
    );
    let expected = concat!(
        "LINK \"sqlite3\" AS link\n",
        "  FUNC columnText(stmt AS Stmt) AS String\n",
        "    SYMBOL \"sqlite3_column_text\"\n",
        "    ABI (stmt CPtr) AS produced CPtr\n",
        "    FREE produced\n",
        "      SYMBOL \"sqlite3_free\"\n",
        "    END FREE\n",
        "  END FUNC\n",
        "END LINK\n",
        "LET after = 1\n",
    );
    assert_eq!(fmt(input), expected);
}

#[test]
fn empty_source_stays_empty() {
    assert_eq!(format_source("", 2).as_deref(), Ok(""));
}

#[test]
fn blank_line_inside_block_is_emitted_empty() {
    // A whitespace-only physical line inside a block re-emits as empty (not
    // indented), exercising the empty-first-cased-line branch.
    let input = "FUNC f()\n   \n  x\nEND FUNC\n";
    let expected = "FUNC f()\n\n  x\nEND FUNC\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn missing_trailing_newline_is_added() {
    // Input without a final newline still ends in one.
    assert_eq!(fmt("LET x = 1"), "LET x = 1\n");
}

#[test]
fn escaped_characters_in_strings_are_preserved() {
    // Both an escaped quote and a trailing backslash-escape are copied verbatim.
    let input = "LET s = \"a\\\"b\\\\\"\n";
    assert_eq!(fmt(input), input);
}

/// A backtick Scalar literal is opaque, so a quote or apostrophe inside it
/// cannot desynchronize the scanner (bug-293).
///
/// `` `"` `` used to open the scanner's *string* mode, which then closed at
/// the next real string's opening quote — leaving that string's body scanned
/// as code, where any word spelling a keyword got uppercased. That silently
/// rewrote a literal the fmt spec guarantees byte-for-byte, changing what the
/// rebuilt program prints.
#[test]
fn scalar_literals_containing_quotes_do_not_desynchronize_the_scanner() {
    // The reported case: the string after a `"` scalar kept its casing.
    let input = "LET c AS Scalar = `\"` : LET s AS String = \"print if then\"\n";
    assert_eq!(fmt(input), input);

    // An apostrophe scalar used to turn the rest of the line into a comment.
    let input = "LET c AS Scalar = `'` : LET s AS String = \"if then\"\n";
    assert_eq!(fmt(input), input);

    // A backtick escaped inside a scalar must not close the token early.
    let input = "LET c AS Scalar = `\\`` : LET s AS String = \"if\"\n";
    assert_eq!(fmt(input), input);

    // Keywords outside any literal are still normalized, so the new arm did
    // not simply stop the scanner from doing its job.
    assert_eq!(
        fmt("LET c AS Scalar = `\"` : if x then\n"),
        "LET c AS Scalar = `\"` : IF x THEN\n"
    );
}

#[test]
fn float_literals_are_preserved() {
    let input = "LET x = 3.14\n";
    assert_eq!(fmt(input), input);
}

#[test]
fn rem_comment_at_statement_start_is_verbatim() {
    let input = "REM this IF is not a keyword\n";
    assert_eq!(fmt(input), input);
}

#[test]
fn walrus_assignment_operator_is_preserved() {
    let input = "LET m AS Bag = WITH b { n := b.n + 1 }\n";
    assert_eq!(fmt(input), input);
}

#[test]
fn union_and_enum_open_blocks() {
    let input = "UNION Shape\nCircle\nEND UNION\nENUM Color\nRed\nEND ENUM\n";
    let expected = "UNION Shape\n  Circle\nEND UNION\nENUM Color\n  Red\nEND ENUM\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn sub_declaration_opens_a_block() {
    let input = "FUNC greet() AS Nothing\nio::print(\"hi\")\nEND FUNC\n";
    let expected = "FUNC greet() AS Nothing\n  io::print(\"hi\")\nEND FUNC\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn doc_header_keeps_attribute_words() {
    // `is_doc_start` accepts alphabetic attribute words; `doc_header` keeps them.
    let input = "DOC INTERNAL\nsome text\nEND DOC\n";
    let expected = "DOC INTERNAL\n  some text\nEND DOC\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn doc_block_preserves_blank_lines() {
    let input = "DOC\nfirst\n\nsecond\nEND DOC\n";
    let expected = "DOC\n  first\n\n  second\nEND DOC\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn unterminated_doc_example_is_flushed() {
    // No END EXAMPLE / END DOC: the pending example source is still emitted,
    // and its blank line is preserved.
    let input = "DOC\nEXAMPLE\nLET x = 1\n\nLET y = 2\n";
    let out = fmt(input);
    assert!(out.contains("EXAMPLE"));
    assert!(out.contains("LET x = 1"));
    assert!(out.contains("LET y = 2"));
}

#[test]
fn link_block_preserves_blank_lines() {
    let input = concat!(
        "LINK \"lib\" AS link\n",
        "FUNC f() AS Integer\n",
        "\n",
        "SYMBOL \"sym\"\n",
        "END FUNC\n",
        "END LINK\n",
    );
    let expected = concat!(
        "LINK \"lib\" AS link\n",
        "  FUNC f() AS Integer\n",
        "\n",
        "    SYMBOL \"sym\"\n",
        "  END FUNC\n",
        "END LINK\n",
    );
    assert_eq!(fmt(input), expected);
}

// ---- bug-821: lines inside an open bracket ----

/// Formats `input`, checks the result against `expected`, and checks that
/// formatting the result again changes nothing (idempotence).
fn fmt_fixed_point(input: &str, expected: &str) {
    let once = fmt(input);
    assert_eq!(once, expected);
    assert_eq!(
        fmt(&once),
        once,
        "formatting twice must equal formatting once"
    );
}

#[test]
fn multi_line_bracket_layouts_are_preserved() {
    // Lines inside an open bracket continue the logical line: they keep
    // their own leading whitespace, are never re-indented to the block
    // level, and the block depth after the literal is unchanged.
    fmt_fixed_point(
        concat!(
            "func main() as nothing\n",
            "let a = [\n",
            "    \"x\", \"y\",\n",
            "\"z\"\n",
            "  ]\n",
            "let m = Map OF String TO Integer {\n",
            "      \"k\" := 1,  ' trailing comment ]\n",
            "      \"(\" := 2,\n",
            "  }\n",
            "if f(1,\n",
            "     2) then\n",
            "return\n",
            "end if\n",
            "end func\n",
        ),
        concat!(
            "FUNC main() AS Nothing\n",
            "  LET a = [\n",
            "    \"x\", \"y\",\n",
            "\"z\"\n",
            "  ]\n",
            "  LET m = Map OF String TO Integer {\n",
            "      \"k\" := 1,  ' trailing comment ]\n",
            "      \"(\" := 2,\n",
            "  }\n",
            "  IF f(1,\n",
            "     2) THEN\n",
            "    RETURN\n",
            "  END IF\n",
            "END FUNC\n",
        ),
    );
}

#[test]
fn multi_line_parameter_list_opens_one_block() {
    fmt_fixed_point(
        concat!(
            "func add(\n",
            "  a as Integer,\n",
            "  b as Integer,\n",
            ") as Integer\n",
            "return a + b\n",
            "end func\n",
        ),
        concat!(
            "FUNC add(\n",
            "  a AS Integer,\n",
            "  b AS Integer,\n",
            ") AS Integer\n",
            "  RETURN a + b\n",
            "END FUNC\n",
        ),
    );
}

#[test]
fn leading_commas_become_trailing_commas() {
    fmt_fixed_point(
        concat!(
            "let codes = [ \"AED\"  ' first\n",
            "            , \"AFN\"\n",
            "            ,\n",
            "            \"ALL\" _\n",
            "            , \"AMD\"\n",
            "            ]\n",
            "let n = f(1\n",
            ",2)\n",
        ),
        concat!(
            "LET codes = [ \"AED\",  ' first\n",
            "              \"AFN\",\n",
            "            \"ALL\", _\n",
            "              \"AMD\"\n",
            "            ]\n",
            "LET n = f(1,\n",
            " 2)\n",
        ),
    );
}

#[test]
fn malformed_leading_commas_are_left_as_written() {
    // No element before the comma, or a doubled comma: nothing to move.
    fmt_fixed_point("LET a = [\n, 1]\n", "LET a = [\n, 1]\n");
    fmt_fixed_point("LET a = [1,\n, 2]\n", "LET a = [1,\n, 2]\n");
    // Outside a bracket a leading comma is not a continuation at all.
    fmt_fixed_point("LET a = f(1)\n, 2\n", "LET a = f(1)\n, 2\n");
}

#[test]
fn brackets_in_strings_scalars_and_comments_do_not_continue_a_line() {
    fmt_fixed_point(
        concat!(
            "func main() as nothing\n",
            "let s = \"[(\" ' {\n",
            "let c = `[`\n",
            "rem (\n",
            "let t = 1\n",
            "end func\n",
        ),
        concat!(
            "FUNC main() AS Nothing\n",
            "  LET s = \"[(\" ' {\n",
            "  LET c = `[`\n",
            "  rem (\n",
            "  LET t = 1\n",
            "END FUNC\n",
        ),
    );
}

#[test]
fn rem_inside_a_continuation_is_an_identifier_like_the_lexer_reads_it() {
    // The lexer emits no statement break inside a bracket or after `_`, so
    // a line-leading `REM` there is an identifier, not a comment: the words
    // after it are code and are cased.
    fmt_fixed_point(
        "LET a = [\nrem, nothing]\nLET b = 1 + _\nrem + nothing\n",
        "LET a = [\nrem, NOTHING]\nLET b = 1 + _\nrem + NOTHING\n",
    );
}

#[test]
fn unclosed_bracket_stops_at_a_statement_keyword() {
    // An unclosed bracket must not swallow the rest of the file: the line
    // that opens with a statement-only keyword starts a new logical line,
    // so block indentation after the error is still computed.
    fmt_fixed_point(
        concat!(
            "func main() as nothing\n",
            "let xs = [1, 2\n",
            "if true then\n",
            "let y = f(1\n",
            "end if\n",
            "end func\n",
        ),
        concat!(
            "FUNC main() AS Nothing\n",
            "  LET xs = [1, 2\n",
            "  IF TRUE THEN\n",
            "    LET y = f(1\n",
            "  END IF\n",
            "END FUNC\n",
        ),
    );
    // Unclosed at end of file: everything left is one logical line.
    fmt_fixed_point("let a = (\n  1\n", "LET a = (\n  1\n");
}
