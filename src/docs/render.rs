//! A deliberately small Markdown -> terminal renderer for `mfb spec` and the
//! Markdown `mfb man` pages.
//!
//! Docs are authored in Markdown so they keep rendering in GitHub and editors
//! during review; this module turns that Markdown into width-aware plain text
//! (optionally ANSI-styled) for the terminal. The whole point is that tables
//! reflow to the actual terminal width instead of being frozen as hand-aligned
//! ASCII, so the table path below is the part that earns its keep.
//!
//! Supported subset — intentionally narrow, so the renderer stays a few hundred
//! lines and authors do not reach for constructs it cannot show:
//!   - ATX headings `#`..`###` (deeper levels clamp to 3)
//!   - paragraphs (word-wrapped to width)
//!   - bullet (`-`, `*`) and ordered (`1.`) lists, nesting by leading spaces
//!   - fenced code blocks ```` ``` ```` (printed verbatim, never wrapped)
//!   - block quotes `>`
//!   - horizontal rules (`---`, `***`, `___`)
//!   - GFM pipe tables (width-aware reflow with box drawing)
//!   - inline `**bold**`, `*italic*`, `` `code` ``, and `[text](url)` links
//!
//! Everything else is out of scope on purpose. Display width is counted in
//! Unicode scalar values; full East-Asian width is not modelled (fine for the
//! mostly-ASCII spec corpus).

/// Rendering parameters: the target terminal `width` and whether to emit ANSI
/// styling (off when stdout is not a TTY, or when piped/redirected).
pub(crate) struct Style {
    pub(crate) width: usize,
    pub(crate) color: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sgr {
    Plain,
    Bold,
    Italic,
    Code,
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

/// A styled run of text with no internal spaces once split into words.
#[derive(Clone)]
struct Span {
    text: String,
    sgr: Sgr,
}

/// A whitespace-delimited word, possibly spanning several styles.
type Word = Vec<Span>;

/// One rendered output line plus its display width (the styled string may carry
/// zero-width ANSI escapes, so the width is tracked separately).
struct VisualLine {
    styled: String,
    width: usize,
}

/// Render a Markdown document to terminal text. The result has no trailing
/// newline; callers print it with `println!`.
pub(crate) fn render(markdown: &str, style: &Style) -> String {
    let width = style.width.max(1);
    let color = style.color;
    let lines: Vec<&str> = markdown.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Fenced code block: emit verbatim (indented), never wrapped.
        if fence_lang(trimmed).is_some() {
            i += 1;
            while i < lines.len() && lines[i].trim() != "```" {
                out.push(format!("  {}", lines[i]));
                i += 1;
            }
            if i < lines.len() {
                i += 1; // closing fence
            }
            continue;
        }

        // Pipe table: a row followed by an alignment separator.
        if table_at(&lines, i) {
            let (block, next) = render_table_block(&lines, i, width, color);
            out.extend(block);
            i = next;
            continue;
        }

        if let Some((level, text)) = atx_heading(line) {
            out.extend(render_heading(level, text, width, color));
            i += 1;
            continue;
        }

        if is_hr(trimmed) {
            out.push("─".repeat(width));
            i += 1;
            continue;
        }

        if trimmed.is_empty() {
            out.push(String::new());
            i += 1;
            continue;
        }

        if is_quote(line) {
            let mut buf = Vec::new();
            while i < lines.len() && is_quote(lines[i]) {
                buf.push(strip_quote(lines[i]));
                i += 1;
            }
            let text = buf.join(" ");
            let words = pieces_to_words(parse_inline(text.trim()));
            let inner = width.saturating_sub(2).max(1);
            let bar = if color { "\x1b[2m│\x1b[0m" } else { "│" };
            for vl in wrap_words(words, inner, color) {
                out.push(format!("{bar} {}", vl.styled));
            }
            continue;
        }

        if list_item(line).is_some() {
            while i < lines.len() {
                let Some(item) = list_item(lines[i]) else {
                    break;
                };
                out.extend(render_list_item(&item, width, color));
                i += 1;
            }
            continue;
        }

        // Paragraph: gather soft-wrapped source lines until the next block.
        let mut buf = Vec::new();
        while i < lines.len() {
            let l = lines[i];
            if l.trim().is_empty()
                || atx_heading(l).is_some()
                || table_at(&lines, i)
                || is_quote(l)
                || list_item(l).is_some()
                || fence_lang(l.trim()).is_some()
                || is_hr(l.trim())
            {
                break;
            }
            buf.push(l.trim());
            i += 1;
        }
        let text = buf.join(" ");
        let words = pieces_to_words(parse_inline(&text));
        for vl in wrap_words(words, width, color) {
            out.push(vl.styled);
        }
    }

    normalize(out).join("\n")
}

/// Strip inline Markdown markup down to plain text — used for one-line summaries
/// in package and topic listings.
pub(crate) fn plain(s: &str) -> String {
    parse_inline(s).into_iter().map(|span| span.text).collect()
}

// ---------------------------------------------------------------------------
// Block helpers
// ---------------------------------------------------------------------------

/// Remove `[[file:symbol]]` provenance citations from a heading line. Headings
/// bypass [`parse_inline`] (they are uppercased/bolded whole), so the citation
/// stripping that hides markers in body text must be applied here too — otherwise
/// a citation on a `### heading` would render and inflate the underline rule.
fn strip_citations(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if let Some(end) = find(&chars, i + 2, "]]") {
                i = end + 2;
                if i < chars.len() && chars[i] == ' ' && out.ends_with(' ') {
                    i += 1;
                }
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn render_heading(level: usize, raw: &str, width: usize, color: bool) -> Vec<String> {
    let stripped = strip_citations(raw);
    let text = stripped.trim();
    match level {
        1 => {
            let title = text.to_uppercase();
            let rule = "═".repeat(title.chars().count().min(width));
            vec![bold(&title, color), rule]
        }
        2 => {
            let rule = "─".repeat(text.chars().count().min(width));
            vec![bold(text, color), rule]
        }
        _ => vec![bold(text, color)],
    }
}

struct ListItem {
    indent: usize,
    marker: String,
    content: String,
}

/// Base indent applied to every list item, on top of any source nesting, so
/// bullets sit slightly inside the surrounding prose (` • item`, not `• item`).
const LIST_INDENT: usize = 1;

fn render_list_item(item: &ListItem, width: usize, color: bool) -> Vec<String> {
    let indent = LIST_INDENT + item.indent;
    let marker_width = item.marker.chars().count();
    let avail = width.saturating_sub(indent + marker_width).max(1);
    let words = pieces_to_words(parse_inline(&item.content));
    let wrapped = wrap_words(words, avail, color);

    let mut res = Vec::new();
    for (k, vl) in wrapped.iter().enumerate() {
        let prefix = if k == 0 {
            format!("{}{}", " ".repeat(indent), item.marker)
        } else {
            " ".repeat(indent + marker_width)
        };
        res.push(format!("{prefix}{}", vl.styled));
    }
    if res.is_empty() {
        res.push(format!("{}{}", " ".repeat(indent), item.marker));
    }
    res
}

fn list_item(line: &str) -> Option<ListItem> {
    let indent = line.chars().take_while(|c| *c == ' ').count();
    let rest = line.trim_start();
    if let Some(content) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")) {
        return Some(ListItem {
            indent,
            marker: "• ".to_string(),
            content: content.to_string(),
        });
    }
    // Ordered: one or more digits, then ". ".
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let after = &rest[digits..];
        if let Some(content) = after.strip_prefix(". ") {
            return Some(ListItem {
                indent,
                marker: format!("{}. ", &rest[..digits]),
                content: content.to_string(),
            });
        }
    }
    None
}

fn atx_heading(line: &str) -> Option<(usize, &str)> {
    let rest = line.trim_start();
    let hashes = rest.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let after = &rest[hashes..];
    if after.is_empty() {
        return Some((hashes.min(3), ""));
    }
    let text = after.strip_prefix(' ')?;
    Some((hashes.min(3), text.trim()))
}

fn fence_lang(trimmed: &str) -> Option<&str> {
    trimmed.strip_prefix("```")
}

fn is_hr(trimmed: &str) -> bool {
    let stripped: String = trimmed.chars().filter(|c| *c != ' ').collect();
    stripped.len() >= 3
        && (stripped.chars().all(|c| c == '-')
            || stripped.chars().all(|c| c == '*')
            || stripped.chars().all(|c| c == '_'))
}

fn is_quote(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

fn strip_quote(line: &str) -> String {
    let t = line.trim_start();
    let t = t.strip_prefix('>').unwrap_or(t);
    t.strip_prefix(' ').unwrap_or(t).to_string()
}

/// Collapse leading/trailing blank lines and runs of >1 blank into a single one.
fn normalize(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        if line.is_empty() && (out.is_empty() || out.last().is_some_and(String::is_empty)) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

fn table_at(lines: &[&str], i: usize) -> bool {
    lines[i].contains('|') && i + 1 < lines.len() && is_separator_row(lines[i + 1])
}

fn is_separator_row(line: &str) -> bool {
    let cells = split_cells(line);
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let c = cell.trim();
            !c.is_empty() && c.chars().all(|ch| ch == ':' || ch == '-') && c.contains('-')
        })
}

fn split_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    // Split on unescaped `|` only, leaving `\|` in the cell for `parse_inline`
    // to turn back into a literal pipe — so a pipe inside cell text cannot break
    // the column count.
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                cur.push('\\');
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            '|' => {
                cells.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn parse_aligns(line: &str) -> Vec<Align> {
    split_cells(line)
        .iter()
        .map(|cell| {
            let c = cell.trim();
            let left = c.starts_with(':');
            let right = c.ends_with(':');
            match (left, right) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            }
        })
        .collect()
}

fn render_table_block(lines: &[&str], i: usize, width: usize, color: bool) -> (Vec<String>, usize) {
    let header = split_cells(lines[i]);
    let aligns = parse_aligns(lines[i + 1]);
    let mut rows = Vec::new();
    let mut j = i + 2;
    while j < lines.len() && lines[j].contains('|') && !lines[j].trim().is_empty() {
        rows.push(split_cells(lines[j]));
        j += 1;
    }
    (render_table(&header, &aligns, &rows, width, color), j)
}

fn render_table(
    header: &[String],
    aligns: &[Align],
    rows: &[Vec<String>],
    width: usize,
    color: bool,
) -> Vec<String> {
    let ncols = std::iter::once(header.len())
        .chain(rows.iter().map(Vec::len))
        .max()
        .unwrap_or(1)
        .max(1);

    let cell_words = |cells: &[String], c: usize| -> Vec<Word> {
        pieces_to_words(parse_inline(cells.get(c).map(String::as_str).unwrap_or("")))
    };

    let header_words: Vec<Vec<Word>> = (0..ncols).map(|c| cell_words(header, c)).collect();
    let row_words: Vec<Vec<Vec<Word>>> = rows
        .iter()
        .map(|r| (0..ncols).map(|c| cell_words(r, c)).collect())
        .collect();

    // Natural (unwrapped) column widths, and a per-column floor: a column can
    // never shrink below its widest single word, since words are not hyphenated.
    // Honouring the floor keeps the borders aligned even when an unbreakable
    // token (e.g. a path) is wider than the column would otherwise get; the
    // table may then exceed the terminal width, which is preferable to a ragged
    // right border.
    let mut col = vec![1usize; ncols];
    let mut floor = vec![1usize; ncols];
    for c in 0..ncols {
        let widest_word = |cells: &[Word]| cells.iter().map(word_width).max().unwrap_or(0);
        col[c] = col[c].max(line_width(&header_words[c]));
        floor[c] = floor[c].max(widest_word(&header_words[c]));
        for r in &row_words {
            col[c] = col[c].max(line_width(&r[c]));
            floor[c] = floor[c].max(widest_word(&r[c]));
        }
    }

    // Shrink the widest shrinkable column until the table fits the terminal.
    // Overhead is "│ " + " │" per column boundary: 3 per column plus the closing
    // border.
    let overhead = ncols * 3 + 1;
    let budget = width.saturating_sub(overhead).max(ncols);
    while col.iter().sum::<usize>() > budget {
        let widest = col
            .iter()
            .enumerate()
            .filter(|(c, w)| **w > floor[*c])
            .max_by_key(|(_, w)| **w)
            .map(|(idx, _)| idx);
        match widest {
            Some(idx) => col[idx] -= 1,
            None => break,
        }
    }

    let mut out = Vec::new();
    out.push(border(&col, '┌', '┬', '┐'));
    out.extend(render_row(&header_words, &col, aligns, color));
    out.push(border(&col, '├', '┼', '┤'));
    for r in &row_words {
        out.extend(render_row(r, &col, aligns, color));
    }
    out.push(border(&col, '└', '┴', '┘'));
    out
}

fn border(col: &[usize], left: char, mid: char, right: char) -> String {
    let segments: Vec<String> = col.iter().map(|w| "─".repeat(w + 2)).collect();
    format!("{left}{}{right}", segments.join(&mid.to_string()))
}

fn render_row(cells: &[Vec<Word>], col: &[usize], aligns: &[Align], color: bool) -> Vec<String> {
    let wrapped: Vec<Vec<VisualLine>> = col
        .iter()
        .enumerate()
        .map(|(c, &w)| wrap_words(cells.get(c).cloned().unwrap_or_default(), w, color))
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);

    let mut lines = Vec::new();
    for r in 0..height {
        let mut s = String::from("│");
        for (c, &w) in col.iter().enumerate() {
            let align = aligns.get(c).copied().unwrap_or(Align::Left);
            s.push(' ');
            s.push_str(&pad_cell(wrapped[c].get(r), w, align));
            s.push(' ');
            s.push('│');
        }
        lines.push(s);
    }
    lines
}

fn pad_cell(vl: Option<&VisualLine>, w: usize, align: Align) -> String {
    let (styled, width) = match vl {
        Some(v) => (v.styled.as_str(), v.width),
        None => ("", 0),
    };
    let pad = w.saturating_sub(width);
    match align {
        Align::Left => format!("{styled}{}", " ".repeat(pad)),
        Align::Right => format!("{}{styled}", " ".repeat(pad)),
        Align::Center => {
            let lp = pad / 2;
            let rp = pad - lp;
            format!("{}{styled}{}", " ".repeat(lp), " ".repeat(rp))
        }
    }
}

// ---------------------------------------------------------------------------
// Inline parsing, words, and wrapping
// ---------------------------------------------------------------------------

fn parse_inline(s: &str) -> Vec<Span> {
    let chars: Vec<char> = s.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut cur = String::new();
    let mut i = 0;

    let flush = |cur: &mut String, spans: &mut Vec<Span>| {
        if !cur.is_empty() {
            spans.push(Span {
                text: std::mem::take(cur),
                sgr: Sgr::Plain,
            });
        }
    };

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() => {
                cur.push(chars[i + 1]);
                i += 2;
            }
            '`' => {
                if let Some(end) = find(&chars, i + 1, "`") {
                    flush(&mut cur, &mut spans);
                    spans.push(Span {
                        text: chars[i + 1..end].iter().collect(),
                        sgr: Sgr::Code,
                    });
                    i = end + 1;
                } else {
                    cur.push(c);
                    i += 1;
                }
            }
            '*' if i + 1 < chars.len() && chars[i + 1] == '*' => {
                if let Some(end) = find(&chars, i + 2, "**") {
                    flush(&mut cur, &mut spans);
                    spans.push(Span {
                        text: chars[i + 2..end].iter().collect(),
                        sgr: Sgr::Bold,
                    });
                    i = end + 2;
                } else {
                    cur.push(c);
                    i += 1;
                }
            }
            '*' => {
                if let Some(end) = find(&chars, i + 1, "*") {
                    flush(&mut cur, &mut spans);
                    spans.push(Span {
                        text: chars[i + 1..end].iter().collect(),
                        sgr: Sgr::Italic,
                    });
                    i = end + 1;
                } else {
                    cur.push(c);
                    i += 1;
                }
            }
            '[' if i + 1 < chars.len() && chars[i + 1] == '[' => {
                // `[[file:symbol]]` provenance citations are maintainer-facing
                // markers kept in the source Markdown for traceability; they
                // never render in `mfb spec` output or one-line summaries.
                if let Some(end) = find(&chars, i + 2, "]]") {
                    i = end + 2;
                    // Collapse the surrounding spaces of an inline citation so
                    // "text [[..]] more" renders as "text more".
                    if i < chars.len() && chars[i] == ' ' && cur.ends_with(' ') {
                        i += 1;
                    }
                } else {
                    cur.push(c);
                    i += 1;
                }
            }
            '[' => {
                if let Some(link) = parse_link(&chars, i) {
                    flush(&mut cur, &mut spans);
                    spans.push(Span {
                        text: link.text,
                        sgr: Sgr::Plain,
                    });
                    if !link.url.is_empty() {
                        spans.push(Span {
                            text: format!(" ({})", link.url),
                            sgr: Sgr::Plain,
                        });
                    }
                    i = link.end;
                } else {
                    cur.push(c);
                    i += 1;
                }
            }
            _ => {
                cur.push(c);
                i += 1;
            }
        }
    }
    flush(&mut cur, &mut spans);
    spans
}

struct Link {
    text: String,
    url: String,
    end: usize,
}

fn parse_link(chars: &[char], start: usize) -> Option<Link> {
    let close = find(chars, start + 1, "]")?;
    if close + 1 >= chars.len() || chars[close + 1] != '(' {
        return None;
    }
    let url_end = find(chars, close + 2, ")")?;
    Some(Link {
        text: chars[start + 1..close].iter().collect(),
        url: chars[close + 2..url_end].iter().collect(),
        end: url_end + 1,
    })
}

/// Find the start index of `needle` in `chars[from..]`, or `None`.
fn find(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let pat: Vec<char> = needle.chars().collect();
    if pat.is_empty() || from > chars.len() {
        return None;
    }
    let mut i = from;
    while i + pat.len() <= chars.len() {
        if chars[i..i + pat.len()] == pat[..] {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Split styled spans into whitespace-delimited words. A word may carry several
/// spans when styles abut without a space between them.
fn pieces_to_words(spans: Vec<Span>) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    let mut cur: Word = Vec::new();
    for span in spans {
        let parts: Vec<&str> = span.text.split(' ').collect();
        for (k, part) in parts.iter().enumerate() {
            if k > 0 && !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            if !part.is_empty() {
                cur.push(Span {
                    text: (*part).to_string(),
                    sgr: span.sgr,
                });
            }
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

fn word_width(word: &Word) -> usize {
    word.iter().map(|span| span.text.chars().count()).sum()
}

fn line_width(words: &[Word]) -> usize {
    words.iter().map(word_width).sum::<usize>() + words.len().saturating_sub(1)
}

fn wrap_words(words: Vec<Word>, width: usize, color: bool) -> Vec<VisualLine> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut cur: Vec<Word> = Vec::new();
    let mut cur_width = 0;

    for word in words {
        let ww = word_width(&word);
        let need = if cur.is_empty() {
            ww
        } else {
            cur_width + 1 + ww
        };
        if !cur.is_empty() && need > width {
            lines.push(emit_line(&cur, color));
            cur_width = ww;
            cur = vec![word];
        } else {
            cur_width = need;
            cur.push(word);
        }
    }
    if !cur.is_empty() {
        lines.push(emit_line(&cur, color));
    }
    lines
}

fn emit_line(words: &[Word], color: bool) -> VisualLine {
    let mut styled = String::new();
    let mut width = 0;
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            styled.push(' ');
            width += 1;
        }
        for span in word {
            styled.push_str(&style_span(&span.text, span.sgr, color));
            width += span.text.chars().count();
        }
    }
    VisualLine { styled, width }
}

fn style_span(text: &str, sgr: Sgr, color: bool) -> String {
    if !color || sgr == Sgr::Plain {
        return text.to_string();
    }
    let code = match sgr {
        Sgr::Bold => "1",
        Sgr::Italic => "3",
        Sgr::Code => "36",
        Sgr::Plain => unreachable!(),
    };
    format!("\x1b[{code}m{text}\x1b[0m")
}

fn bold(text: &str, color: bool) -> String {
    if color {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
}
