use super::*;

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '[' {
            i += 2;
            while i < chars.len() && chars[i] != 'm' {
                i += 1;
            }
            i += 1; // skip the 'm'
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn plain_style(width: usize) -> Style {
    Style {
        width,
        color: false,
    }
}

fn display_width(line: &str) -> usize {
    strip_ansi(line).chars().count()
}

#[test]
fn h1_is_uppercased_and_underlined() {
    let out = render("# Hello World", &plain_style(80));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "HELLO WORLD");
    assert_eq!(lines[1], "═".repeat("HELLO WORLD".chars().count()));
}

#[test]
fn paragraph_wraps_to_width() {
    let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda";
    let out = render(text, &plain_style(20));
    for line in out.lines() {
        assert!(
            display_width(line) <= 20,
            "line exceeds width: {line:?} ({})",
            display_width(line)
        );
    }
    // The words survive the wrap.
    assert!(strip_ansi(&out).contains("lambda"));
}

/// In a table cell `\\|` is a literal pipe, in a code span too, as GFM
/// has it: the escape is undone before the cell's text is read.
#[test]
fn an_escaped_pipe_in_a_table_cell_is_a_pipe_even_in_code() {
    let md = "| Op | Text |\n| --- | --- |\n| `\\|>` | a \\| b |";
    let out = strip_ansi(&render(md, &plain_style(80)));
    assert!(out.contains("|>"), "{out}");
    assert!(out.contains("a | b"), "{out}");
    assert!(!out.contains('\\'), "{out}");
}

#[test]
fn code_fence_is_verbatim_and_unwrapped() {
    let md = "```\nLET x = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n```";
    let out = render(md, &plain_style(20));
    // Indented by two spaces, and NOT wrapped even though it exceeds width.
    assert_eq!(out, "  LET x = aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
}

#[test]
fn table_reflows_within_width() {
    let md = "\
| Name | Description |
| --- | --- |
| alpha | a fairly long description that will need to wrap when narrow |
| beta | short |";
    for width in [80, 40, 24] {
        let out = render(md, &plain_style(width));
        for line in out.lines() {
            assert!(
                display_width(line) <= width,
                "width {width}: line exceeds: {line:?} ({})",
                display_width(line)
            );
        }
        // Box drawing present, and content preserved.
        assert!(out.contains('│'), "width {width}: missing borders");
        assert!(strip_ansi(&out).contains("alpha"));
    }
}

#[test]
fn table_borders_stay_aligned_with_long_tokens() {
    // An unbreakable token wider than the squeezed column must not push the
    // right border out: every rendered row shares one display width.
    let md = "\
| Col | Path |
| --- | --- |
| a | src/target/shared/code.rs |
| b | x |";
    let out = render(md, &plain_style(20));
    let widths: Vec<usize> = out.lines().map(display_width).collect();
    assert!(
        widths.windows(2).all(|w| w[0] == w[1]),
        "rows have ragged widths: {widths:?}"
    );
}

#[test]
fn bullet_list_renders_marker() {
    let out = render("- first\n- second", &plain_style(80));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], " • first");
    assert_eq!(lines[1], " • second");
}

#[test]
fn ordered_list_keeps_numbers() {
    let out = render("1. one\n2. two", &plain_style(80));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], " 1. one");
    assert_eq!(lines[1], " 2. two");
}

#[test]
fn inline_markup_is_stripped_without_color() {
    let out = render("a **bold** and `code` and *em* word", &plain_style(80));
    assert_eq!(out, "a bold and code and em word");
}

#[test]
fn inline_markup_styled_with_color() {
    let style = Style {
        width: 80,
        color: true,
    };
    let out = render("**bold**", &style);
    assert!(out.contains("\x1b[1m"));
    assert_eq!(strip_ansi(&out), "bold");
}

#[test]
fn link_shows_text_and_url() {
    let out = render("see [the docs](https://example.com) now", &plain_style(80));
    assert_eq!(out, "see the docs (https://example.com) now");
}

#[test]
fn plain_strips_markup() {
    assert_eq!(plain("a **b** `c`"), "a b c");
}

#[test]
fn provenance_citations_are_stripped() {
    // Inline citations vanish and their surrounding spaces collapse.
    let out = render(
        "a value [[src/ir/value.rs:IrValue]] is flat",
        &plain_style(80),
    );
    assert_eq!(out, "a value is flat");
    // Also stripped from one-line summaries.
    assert_eq!(
        plain("Flat values [[src/foo.rs:bar]] only"),
        "Flat values only"
    );
    // A trailing citation leaves no dangling marker.
    assert_eq!(
        plain("see the memory spec [[src/docs/spec/memory/spec.md:1]]").trim(),
        "see the memory spec"
    );
}

#[test]
fn provenance_citations_are_stripped_in_headings() {
    // Headings bypass parse_inline; the citation must still not render, and
    // the underline rule must match the visible title length, not include it.
    let out = render("### entry[[src/ir/mod.rs:EntryPoint]]", &plain_style(80));
    assert_eq!(out, "entry");
    let h2 = render(
        "## bindings [[src/ir/types.rs:IrBinding]]",
        &plain_style(80),
    );
    assert_eq!(h2.lines().next().unwrap(), "bindings");
}

#[test]
fn blockquote_gets_bar() {
    let out = render("> quoted text", &plain_style(80));
    assert_eq!(out, "│ quoted text");
}

#[test]
fn horizontal_rule_spans_width() {
    let out = render("---", &plain_style(10));
    assert_eq!(out, "─".repeat(10));
}

fn color_style(width: usize) -> Style {
    Style { width, color: true }
}

#[test]
fn an_unclosed_fence_runs_to_the_end_and_a_bare_heading_is_empty() {
    let out = render("```\nlet x = 1\n", &plain_style(80));
    assert_eq!(out.lines().next(), Some("  let x = 1"));
    // Four or more hashes render as a level-3 heading; a bare run of hashes
    // is a heading with no text.
    assert_eq!(
        render("#### Deep", &plain_style(80)),
        render("### Deep", &plain_style(80))
    );
    assert_eq!(
        render("####", &plain_style(80)),
        render("###", &plain_style(80))
    );
}

#[test]
fn table_columns_align_left_center_and_right() {
    let md = "| a | b | c |\n|:---|:---:|---:|\n| 1 | 22 | 333 |\n| 4444 | 5 | 6 |\n";
    let out = render(md, &plain_style(80));
    // Left pads after the text, right before it, center splits the padding
    // (the odd space after).
    assert!(
        out.lines().any(|line| line == "│ 1    │ 22 │ 333 │"),
        "{out}"
    );
    assert!(
        out.lines().any(|line| line == "│ 4444 │ 5  │   6 │"),
        "{out}"
    );
    let wide = render("| x |\n|:---:|\n| abcde |\n| a |\n", &plain_style(80));
    assert!(wide.lines().any(|line| line == "│   a   │"), "{wide}");
}

#[test]
fn a_cell_ending_in_a_backslash_keeps_it() {
    let out = render("| a |\n|---|\n| x\\ |\n", &plain_style(80));
    assert!(out.contains("x\\"), "{out}");
}

#[test]
fn a_citation_left_open_and_a_link_without_a_target_render_as_text() {
    assert_eq!(plain("see [[file:sym and more"), "see [[file:sym and more");
    assert_eq!(plain("a [b] c"), "a [b] c");
    assert_eq!(plain("a [b](x c"), "a [b](x c");
}

#[test]
fn italic_and_bold_are_styled_with_color() {
    let out = render("*slanted* and **heavy**", &color_style(80));
    assert!(out.contains("\x1b[3mslanted"), "{out:?}");
    assert!(out.contains("heavy"), "{out:?}");
    let heading = render("# Title", &color_style(80));
    assert!(heading.contains("\x1b[1m"), "{heading:?}");
}

#[test]
fn an_empty_list_item_still_shows_its_marker() {
    let out = render("- ", &plain_style(80));
    assert!(out.contains('•'), "{out:?}");
}
