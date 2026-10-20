//! plan-176-E: `no_type_strings`'s self-checks: the scanners fire on their own
//! needles, and the closed lists (operand classes, type-keyed tables, the grammar
//! file, the boundary list) match the tree. Every scanner, path list and allowlist
//! lives in the root, `no_type_strings.rs`.

use super::*;

/// The gate is only worth its budgets if the scanner actually fires. Each class
/// gets one positive fixture and, where it is the difference between a real
/// hit and a lookalike, one negative.
#[test]
fn scanners_fire_on_their_own_needles() {
    assert_eq!(
        declared_sites("let t = ParameterType::declared(name);").len(),
        1,
        "plan-111-F: `declared` is `parse` under another name — count it, so \
         class 1 cannot be dodged by renaming the call"
    );
    assert_eq!(
        declared_sites("let t = ParameterType::named(name);").len(),
        0,
        "…but `named` is a CONSTRUCTOR, not a grammar entry"
    );
    assert_eq!(
        parse_sites("let t = ParameterType::declared(name);").len(),
        0,
        "the two classes are disjoint, so a site is never counted twice"
    );
    assert_eq!(
        parse_sites("let t = ParameterType::parse(name);").len(),
        1,
        "parse_sites must count a ParameterType::parse( call"
    );
    assert_eq!(
        parse_sites("// let t = ParameterType::parse(name);").len(),
        0,
        "a commented-out call is not a site"
    );
    assert_eq!(
        parse_sites("ParameterType::map_of(ParameterType::parse(k), ParameterType::parse(v))")
            .len(),
        2,
        "two calls on one line are two sites, not one line"
    );

    // plan-113 — the C FFI vocabulary, both shapes.
    assert_eq!(
        c_type_strings("src/ir/link.rs", "    pub(crate) ctype: String,").len(),
        1,
        "c_type_strings must count a `ctype: String` carrier"
    );
    assert_eq!(
        c_type_strings("src/ir/link.rs", "fn f(abi_return_ctype: &'a str) {}").len(),
        1,
        "…and a `&'a str` one"
    );
    assert_eq!(
        c_type_strings("src/ir/link.rs", "    param_ctype: Option<String>,").len(),
        1,
        "…and an `Option<String>` one"
    );
    assert_eq!(
        c_type_strings("src/ir/link.rs", "    pub(crate) ctype: ParameterType,").len(),
        0,
        "the typed carrier is the conversion's DESTINATION, not a violation"
    );
    assert_eq!(
        c_type_strings("src/ast/types.rs", "    pub ctype: String,").len(),
        0,
        "`src/ast/**` is the NAME world and may carry a spelling; \
         `hir::elaborate_link_block` is where it becomes a type"
    );
    assert_eq!(
        c_type_strings("src/ir/link.rs", r#"if slot.ctype == "CInt32" {"#).len(),
        1,
        "a spelling compare is caught by the LITERAL half; the carrier half needs \
         a `name:` declaration, which a compare does not have"
    );
    assert_eq!(
        c_type_strings("src/ir/link.rs", r#"    ctype: String, // was "CInt32""#).len(),
        2,
        "both halves fire on one line — and note `code_lines` only drops a line \
         that STARTS with `//`, so a trailing comment is still scanned"
    );
    assert_eq!(
        c_type_strings("src/ast/link_items.rs", r#"matches!(c, "CPtr" | "CVoid")"#).len(),
        2,
        "the `src/ast/**` exemption covers the CARRIER half only — a spelling \
         DECISION is forbidden there too"
    );
    assert_eq!(
        c_type_strings(
            "src/ir/link.rs",
            r#"// a crafted `ctype: "CVoid"` is rejected"#
        )
        .len(),
        0,
        "a comment is not code (`code_lines` strips them)"
    );
    assert_eq!(
        c_type_strings("src/ir/link.rs", r#"let s = "CInt32is not a spelling";"#).len(),
        0,
        "the literal must be the WHOLE quoted string"
    );

    assert_eq!(
        str_type_params("fn f(element_type: &str) {}").len(),
        1,
        "str_type_params must count a type-named &str parameter"
    );
    assert_eq!(
        str_type_params("fn f(type_: &'a str) {}").len(),
        1,
        "a lifetime does not hide the parameter"
    );
    assert_eq!(
        str_type_params("fn f(type_name: &ParameterType) {}").len(),
        0,
        "an already-typed parameter is not a violation"
    );
    assert_eq!(
        str_type_params("fn f(sub_type_: &str) {}").len(),
        0,
        "`type_` must match at a word boundary, not inside another identifier"
    );

    assert_eq!(
        spelling_match_arms("        \"Integer\" | \"Float\" => 8,").len(),
        1,
        "spelling_match_arms must count a multi-spelling arm"
    );
    assert_eq!(
        spelling_match_arms("        (\"sin\", \"Float\") => Kernel::Sin,").len(),
        1,
        "plan-111-D Correction D1: a TUPLE arm dispatches on a spelling too"
    );
    assert_eq!(
        spelling_match_arms("        (\"min\", \"Integer\" | \"Fixed\") => Kernel::MinSigned,")
            .len(),
        1,
        "a tuple arm with an or-pattern is still one arm"
    );
    assert_eq!(
        spelling_match_arms("            other => self.render(other, \"Integer\"),").len(),
        0,
        "a spelling to the RIGHT of the arrow is an argument, not a pattern"
    );
    assert_eq!(
        spelling_match_arms("        _ if t == \"Integer\" => 8,").len(),
        0,
        "a compare inside a guard belongs to spelling_compares, not here"
    );
    assert_eq!(
        spelling_compares("        _ if t == \"Integer\" => 8,").len(),
        1,
        "…and spelling_compares must be the one that counts it"
    );
    assert_eq!(
        spelling_match_arms("            ) || __t.is_named(\"Scalar\") =>").len(),
        0,
        "plan-111-E: `is_named` is the TYPED nominal test, not a spelling match"
    );
    assert_eq!(
        spelling_compares("        if !type_.is_named(\"Error\") {").len(),
        0,
        "…and it is not a spelling compare either"
    );
    assert_eq!(
        spelling_match_arms("            ) || __t.is_named(\"Scalar\") || x == \"Integer\" =>")
            .len(),
        1,
        "…but masking `is_named` must not hide a real spelling beside it"
    );
    assert_eq!(
        spelling_compares("            if x.get(\"type\").as_deref() == Some(\"Integer\") {").len(),
        1,
        "plan-111-D Correction D1: a compare wrapped in Some() is still a compare"
    );
    assert_eq!(
        spelling_compares("            let v = Some(\"Integer\");").len(),
        0,
        "…but constructing a Some() is not a comparison"
    );
    assert_eq!(
        spelling_match_arms("        \"someKey\" => 8,").len(),
        0,
        "a non-type string arm is not a violation"
    );

    assert_eq!(
        spelling_compares("if t == \"Money\" { }").len(),
        1,
        "spelling_compares must count an == against a spelling"
    );
    assert_eq!(
        spelling_compares("if t != \"Result\" { }").len(),
        1,
        "and a != too"
    );
    assert_eq!(
        spelling_compares("let x = \"Integer\";").len(),
        0,
        "a plain assignment is not a comparison"
    );

    assert_eq!(
        hand_rolled_grammar("name.split_once(\" STATE \")").len(),
        1,
        "hand_rolled_grammar must count a grammar split"
    );
    assert_eq!(
        hand_rolled_grammar("name.strip_prefix(\"List OF \")").len(),
        1,
        "and a container strip"
    );
    assert_eq!(
        hand_rolled_grammar("path.starts_with(\"src/\")").len(),
        0,
        "a non-grammar token is not a violation"
    );

    assert_eq!(
        format_type_construction("format!(\"List OF {inner}\")").len(),
        1,
        "format_type_construction must count a constructed spelling"
    );
    assert_eq!(
        format_type_construction("format!(\"unknown type {t}\")").len(),
        0,
        "a message mentioning a type is not construction"
    );

    assert_eq!(
        string_keyed_type_maps(
            "src/codegen/engine/builder/mod.rs",
            "    pub(crate) union_names: HashSet<String>,"
        )
        .len(),
        1,
        "string_keyed_type_maps must count a listed table keyed by String"
    );
    assert_eq!(
        string_keyed_type_maps(
            "src/codegen/engine/builder/mod.rs",
            "    pub(crate) union_names: HashSet<ParameterType>,"
        )
        .len(),
        0,
        "re-keying by ParameterType is what lowers the count"
    );
    assert_eq!(
        string_keyed_type_maps("src/ir/lower.rs", "    binding_types: HashMap<String, X>,").len(),
        0,
        "a table not on the curated list is not scanned"
    );
}

/// The NIR `mov_imm` operand-class vocabulary is CLOSED, and holds no type.
///
/// plan-111-G Correction G1. `abi::move_immediate`'s second argument and the
/// `instruction.get("type")` reads in `src/optimizer/opt2/**` look like a type
/// site and are not one: the attribute is the immediate encoder's
/// width/interpretation class. Its whole vocabulary is six tokens, two of which
/// (`UnionTag`, `EnumOrdinal`) name no MFBASIC type at all.
///
/// So those sites are excluded from classes 2 and 4 — but the exclusion is
/// *checked*, not asserted. A bare exclusion would let a real type spelling
/// drift into the slot silently; this fails the moment one does. That is the
/// property the plan actually wants, and it is stronger than the `0` the phase
/// originally asked for.
#[test]
fn immediate_operand_class_vocabulary_is_closed() {
    const ALLOWED: &[&str] = &[
        "Integer",
        "Byte",
        "Boolean",
        "Fixed",
        // Reached only through `abi::immediate_class` (a scalar default, a
        // scalar `Const`, `collections::sum`'s accumulator seed).
        "Float",
        "Money",
        "Nothing",
        // Not types: a union's discriminant and an enum's ordinal.
        "UnionTag",
        "EnumOrdinal",
    ];
    /// The one sanctioned way to DERIVE a class from a value's type.
    const DERIVED: &[&str] = &["immediate_class(", "IMMEDIATE_CLASS_"];
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders: Vec<String> = Vec::new();

    for path in rs_files(&[manifest.join("src")]) {
        let rel = path
            .strip_prefix(manifest)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if is_excluded_from_scan(&rel) {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read source");
        for (n, line) in src.lines().enumerate() {
            if let Some(arg) = immediate_class_argument(line, "move_immediate(") {
                match as_literal(&arg) {
                    Some(lit) if !ALLOWED.contains(&lit) => {
                        offenders.push(format!("{rel}:{} — mov_imm class `{lit}`", n + 1));
                    }
                    Some(_) => {}
                    // Correction G7: a COMPUTED class argument is the hole this
                    // test used to have. Three emitters passed `&type_.name()`
                    // here, so a real type spelling reached the attribute and
                    // this scan — literals only — reported a clean vocabulary.
                    // The `.ncode` goldens carry `"type": "Money"` and
                    // `"type": "Nothing"` as the proof. A derived class must now
                    // come from the single mapping in `target::shared::abi`,
                    // which is where the tokens are enumerated.
                    None if !DERIVED.iter().any(|d| arg.contains(d)) => {
                        offenders.push(format!(
                            "{rel}:{} — mov_imm class computed as `{arg}`",
                            n + 1
                        ));
                    }
                    None => {}
                }
            }
            // The READ side: `instruction.get("type") … Some("X")`.
            if line.contains("get(\"type\")") {
                for lit in quoted_literals_after(line, "Some(") {
                    if !ALLOWED.contains(&lit.as_str()) {
                        offenders.push(format!("{rel}:{} — mov_imm class read `{lit}`", n + 1));
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the `mov_imm` operand class is a CLOSED encoder vocabulary, not a type. \
         A spelling outside {ALLOWED:?} reached it — if that is a real MFBASIC \
         type, this attribute has become a type site and belongs in classes 2 \
         and 4 after all:\n{}",
        offenders.join("\n")
    );
}

/// The `mov_imm` operand-class ARGUMENT on `line`, if it is a literal.
///
/// Argument index 1 of `move_immediate(dst, class, value)`, taken by splitting
/// the call's argument list on top-level commas. Index matters both ways: `dst`
/// is often a `"x0"`-style literal and `value` is always one (`"0"`, `"8"`, a
/// bit pattern), and reading either as a class is how the first version of this
/// test both false-positived on values AND missed a planted `"List OF Integer"`
/// sitting in the class slot behind a literal `dst`.
fn quoted_literals_after(line: &str, needle: &str) -> Vec<String> {
    let Some(at) = line.find(needle) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut rest = &line[at + needle.len()..];
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        match after.find('"') {
            Some(close) => {
                out.push(after[..close].to_string());
                rest = &after[close + 1..];
            }
            None => break,
        }
    }
    out
}

fn immediate_class_argument(line: &str, needle: &str) -> Option<String> {
    let at = line.find(needle)? + needle.len();
    let rest = &line[at..];
    let mut depth = 0i32;
    let mut args: Vec<&str> = Vec::new();
    let mut start = 0usize;
    let mut in_str = false;
    for (i, c) in rest.char_indices() {
        match c {
            '"' => in_str = !in_str,
            _ if in_str => {}
            '(' | '[' => depth += 1,
            ')' | ']' if depth == 0 => {
                args.push(&rest[start..i]);
                break;
            }
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                args.push(&rest[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    let arg = args.get(1)?.trim();
    Some(arg.to_string())
}

/// The literal spelling inside `arg`, or `None` when the argument is computed.
fn as_literal(arg: &str) -> Option<&str> {
    arg.strip_prefix('"')?.strip_suffix('"')
}

/// The curated `TYPE_KEYED_TABLES` list is only trustworthy if every entry
/// still names something real. A renamed or deleted table would otherwise
/// silently drop out of the population and look like progress.
#[test]
fn curated_type_keyed_tables_all_exist() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut missing: Vec<String> = Vec::new();
    for (file, ident) in TYPE_KEYED_TABLES {
        let src = std::fs::read_to_string(manifest.join(file))
            .unwrap_or_else(|_| panic!("TYPE_KEYED_TABLES names a missing file: {file}"));
        if !test_free_lines(&src).iter().any(|(_, l)| l.contains(ident)) {
            missing.push(format!("{file}: {ident}"));
        }
    }
    assert!(
        missing.is_empty(),
        "TYPE_KEYED_TABLES entries no longer present — if a table was renamed, \
         update the entry; if it was genuinely removed, delete the entry AND \
         lower the string_keyed_type_maps budget:\n{}",
        missing.join("\n")
    );
}

/// The grammar-file exemption covers exactly one file, and it is the one that
/// actually defines `parse` and `name`.
///
/// The exemption is total — every needle class — so it is the widest allowance
/// in this gate, and the only thing keeping it honest is that it names one file
/// for a reason that is true of no other. This asserts both halves.
#[test]
fn the_grammar_file_is_exactly_one() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let exempt: Vec<String> = rs_files(&[manifest.join("src")])
        .into_iter()
        .map(|p| {
            p.strip_prefix(manifest)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .filter(|rel| is_grammar_file(rel))
        .collect();
    assert_eq!(
        exempt,
        vec!["src/types.rs".to_string()],
        "the grammar-file exemption must cover exactly one file"
    );
    let src = std::fs::read_to_string(manifest.join("src/types.rs")).expect("read types.rs");
    assert!(
        src.contains("pub(crate) fn parse(name: &str) -> ParameterType"),
        "src/types.rs must be the file that DEFINES `parse` — that is the whole \
         justification for exempting it"
    );
    assert!(
        src.contains("pub(crate) fn name(&self) -> Cow<'static, str>"),
        "…and `name`"
    );
}

/// The boundary list is CLOSED, and adding to it requires editing this test.
///
/// plan-111-A named five boundaries; letter G added the decoder half of #4
/// (`binary_repr/reader.rs`, Correction G2), so the list is six files in five
/// groups. It is the plan's one sanctioned allowance for the name-handling
/// classes, and the way an allowance rots is by growing quietly — so the exact
/// membership is asserted here rather than merely justified in a string.
///
/// A seventh entry is not forbidden. It is required to be *deliberate*: whoever
/// adds one has to come here, read why the list is closed, and say what makes
/// their file a boundary rather than a place that skipped the conversion.
#[test]
fn boundary_list_is_closed() {
    let listed: Vec<&str> = BOUNDARY_FILES.iter().map(|(f, _)| *f).collect();
    assert_eq!(
        listed,
        vec![
            // #1 the parser's own recursion
            "src/types.rs",
            // #2 the IR binary codec
            "src/ir/binary/mod.rs",
            "src/ir/binary/link_codec.rs",
            "src/ir/binary/op_codec.rs",
            "src/ir/binary/value_codec.rs",
            // #3 the AST -> HIR elaboration seam
            "src/hir/mod.rs",
            "src/hir/build.rs",
            // #4 the .mfp wire codec, both directions
            "src/binary_repr/writer.rs",
            "src/binary_repr/sections.rs",
            "src/binary_repr/reader.rs",
            "src/binary_repr/builder.rs",
            // #5 the package manifest
            "src/manifest/package.rs",
        ],
        "the boundary list is closed at five GROUPS. Adding an entry means \
         claiming a file converts between the name world and the type world — \
         not that it has not been converted yet."
    );
}

/// Every `BOUNDARY_FILES` entry carries a justification and still exists.
#[test]
fn boundary_files_exist_and_are_justified() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (file, why) in BOUNDARY_FILES {
        assert!(
            manifest.join(file).is_file(),
            "BOUNDARY_FILES names a missing file: {file}"
        );
        assert!(
            !why.is_empty(),
            "BOUNDARY_FILES entry {file} has no justification"
        );
    }
}
