use super::*;

impl<'a> FileParser<'a> {
    /// Structurally parse a `DOC` block captured by the lexer. Semantic checks
    /// (name resolution, duplicate/context rules, attribute validity) belong to
    /// the resolver; this only shapes the raw lines into a `DocBlock`.
    pub(super) fn parse_doc_block(&mut self, raw: lexer::DocRaw) -> Option<DocBlock> {
        let lexer::DocRaw { line, attrs, lines } = raw;

        // The header is the first non-blank body line.
        let header_index = lines.iter().position(|l| !l.text.trim().is_empty());
        let Some(header_index) = header_index else {
            self.report_at(
                "DOC_BAD_HEADER",
                "DOC block has no header line; expected FUNC, SUB, TYPE, UNION, ENUM, or PACKAGE.",
                line,
            );
            return None;
        };
        let header_line = lines[header_index].line;
        let (head_kw, head_rest) = split_first_word(lines[header_index].text.trim());
        let header_kind = match head_kw.to_ascii_uppercase().as_str() {
            "FUNC" => DocHeaderKind::Func,
            "SUB" => DocHeaderKind::Sub,
            "TYPE" => DocHeaderKind::Type,
            "UNION" => DocHeaderKind::Union,
            "ENUM" => DocHeaderKind::Enum,
            "RESOURCE" => DocHeaderKind::Resource,
            "PACKAGE" => DocHeaderKind::Package,
            _ => {
                self.report_at(
                    "DOC_BAD_HEADER",
                    &format!(
                        "`{head_kw}` is not a valid DOC header; expected FUNC, SUB, TYPE, UNION, ENUM, RESOURCE, or PACKAGE."
                    ),
                    header_line,
                );
                return None;
            }
        };
        // FUNC/SUB headers may carry a parenthesized parameter-type list to pick a
        // specific overload, e.g. `FUNC query(Db, String, List OF String)`.
        let callable = matches!(header_kind, DocHeaderKind::Func | DocHeaderKind::Sub);
        let (header_name, header_params) = if callable {
            self.parse_header_signature(head_rest.trim(), header_line)
        } else {
            (head_rest.trim().to_string(), None)
        };
        if header_kind == DocHeaderKind::Package {
            if !header_name.is_empty() {
                self.report_at(
                    "DOC_BAD_HEADER",
                    "A PACKAGE doc header takes no name.",
                    header_line,
                );
                return None;
            }
        } else if header_name.is_empty() {
            self.report_at(
                "DOC_BAD_HEADER",
                &format!(
                    "A {} doc header must name a declaration.",
                    header_kind.keyword()
                ),
                header_line,
            );
            return None;
        }

        let mut desc: Vec<DocProse> = Vec::new();
        // Accumulator for the current prose block: its kind plus the lines joined
        // so far. Consecutive lines of the same kind concatenate; a blank line, a
        // different prose kind, or any structured line flushes it.
        let mut current: Option<(DocProseKind, Vec<String>)> = None;
        let mut deprecated = Vec::new();
        let mut groups = Vec::new();
        let mut args = Vec::new();
        let mut rets = Vec::new();
        let mut errors = Vec::new();
        let mut props = Vec::new();
        let mut examples = Vec::new();

        let flush = |current: &mut Option<(DocProseKind, Vec<String>)>,
                     desc: &mut Vec<DocProse>| {
            if let Some((kind, parts)) = current.take() {
                if !parts.is_empty() {
                    desc.push(DocProse {
                        kind,
                        text: parts.join(" "),
                    });
                }
            }
        };

        let mut index = header_index + 1;
        while index < lines.len() {
            let line_no = lines[index].line;
            let raw_text = &lines[index].text;
            let trimmed = raw_text.trim();
            if trimmed.is_empty() {
                flush(&mut current, &mut desc);
                index += 1;
                continue;
            }
            let (kw, rest) = split_first_word(trimmed);
            if let Some(kind) = DocProseKind::from_keyword(kw) {
                // A prose keyword (DESC/WARN/INFO/SEC). Switching kinds or a blank
                // line ends the current block.
                if current.as_ref().is_some_and(|(k, _)| *k != kind) {
                    flush(&mut current, &mut desc);
                }
                if rest.trim().is_empty() {
                    flush(&mut current, &mut desc);
                } else {
                    current
                        .get_or_insert_with(|| (kind, Vec::new()))
                        .1
                        .push(rest.trim().to_string());
                }
                index += 1;
                continue;
            }
            match kw.to_ascii_uppercase().as_str() {
                "DEPRECATED" => {
                    flush(&mut current, &mut desc);
                    deprecated.push((rest.trim().to_string(), line_no));
                }
                "GROUP" => {
                    flush(&mut current, &mut desc);
                    groups.push((rest.trim().to_string(), line_no));
                }
                "RET" => {
                    flush(&mut current, &mut desc);
                    rets.push((rest.trim().to_string(), line_no));
                }
                "ARG" => {
                    flush(&mut current, &mut desc);
                    let (name, adesc) = split_first_word(rest.trim());
                    if name.is_empty() {
                        self.report_at(
                            "DOC_UNKNOWN_LINE",
                            "ARG line must name a parameter.",
                            line_no,
                        );
                    } else {
                        args.push(DocNamed {
                            name: name.to_string(),
                            desc: adesc.trim().to_string(),
                            line: line_no,
                        });
                    }
                }
                "PROP" => {
                    flush(&mut current, &mut desc);
                    let (name, pdesc) = split_first_word(rest.trim());
                    if name.is_empty() {
                        self.report_at(
                            "DOC_UNKNOWN_LINE",
                            "PROP line must name a member.",
                            line_no,
                        );
                    } else {
                        props.push(DocNamed {
                            name: name.to_string(),
                            desc: pdesc.trim().to_string(),
                            line: line_no,
                        });
                    }
                }
                "ERROR" => {
                    flush(&mut current, &mut desc);
                    let (code, edesc) = split_first_word(rest.trim());
                    if code.is_empty() {
                        self.report_at(
                            "DOC_UNKNOWN_LINE",
                            "ERROR line must name an error code.",
                            line_no,
                        );
                    } else {
                        errors.push(DocError {
                            code: code.to_string(),
                            desc: edesc.trim().to_string(),
                            line: line_no,
                        });
                    }
                }
                "EXAMPLE" => {
                    flush(&mut current, &mut desc);
                    // Collect verbatim lines until `END EXAMPLE`.
                    let mut body: Vec<&str> = Vec::new();
                    let mut closed = false;
                    index += 1;
                    while index < lines.len() {
                        let t = lines[index].text.trim();
                        let words: Vec<&str> = t.split_whitespace().collect();
                        if words.len() == 2
                            && words[0].eq_ignore_ascii_case("END")
                            && words[1].eq_ignore_ascii_case("EXAMPLE")
                        {
                            closed = true;
                            break;
                        }
                        body.push(&lines[index].text);
                        index += 1;
                    }
                    if !closed {
                        self.report_at(
                            "DOC_EXAMPLE_UNTERMINATED",
                            "EXAMPLE block reached END DOC before its `END EXAMPLE` line.",
                            line_no,
                        );
                    }
                    examples.push((dedent(&body), line_no));
                }
                _ => {
                    self.report_at(
                        "DOC_UNKNOWN_LINE",
                        &format!(
                            "`{kw}` is not a valid DOC line; expected DESC, WARN, INFO, SEC, DEPRECATED, GROUP, ARG, RET, ERROR, PROP, or EXAMPLE."
                        ),
                        line_no,
                    );
                }
            }
            index += 1;
        }
        flush(&mut current, &mut desc);

        Some(DocBlock {
            line,
            attrs,
            header_kind,
            header_name,
            header_params,
            header_line,
            desc,
            deprecated,
            groups,
            args,
            rets,
            errors,
            props,
            examples,
        })
    }
}

impl<'a> FileParser<'a> {
    /// Parse a FUNC/SUB doc header's name and optional parenthesized parameter-type
    /// disambiguator: `name` -> (name, None); `name(T1, T2)` -> (name, Some([T1, T2])).
    /// Type strings are whitespace-normalized; commas inside nested parens (function
    /// types) are not split on.
    fn parse_header_signature(
        &mut self,
        text: &str,
        header_line: usize,
    ) -> (String, Option<Vec<String>>) {
        let Some(open) = text.find('(') else {
            return (text.trim().to_string(), None);
        };
        let name = text[..open].trim().to_string();
        let rest = &text[open + 1..];
        // Find the matching close paren, tracking nesting.
        let mut depth = 1usize;
        let mut end = rest.len();
        for (idx, ch) in rest.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = idx;
                        break;
                    }
                }
                _ => {}
            }
        }
        // An unterminated `(` (no matching `)`) leaves `depth != 0`; the scan then
        // treats the whole remainder as the param list. Reject it instead of
        // silently accepting a malformed signature (bug-171 finding E).
        if depth != 0 {
            self.report_at(
                "DOC_BAD_HEADER",
                &format!("DOC header signature for `{name}` is missing a closing `)`."),
                header_line,
            );
        }
        let inner = &rest[..end];
        if inner.trim().is_empty() {
            return (name, Some(Vec::new()));
        }
        // Split on top-level commas.
        let mut params = Vec::new();
        let mut depth = 0usize;
        let mut start = 0usize;
        let bytes = inner.as_bytes();
        for (idx, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                b',' if depth == 0 => {
                    params.push(normalize_ws(&inner[start..idx]));
                    start = idx + 1;
                }
                _ => {}
            }
        }
        params.push(normalize_ws(&inner[start..]));
        (name, Some(params))
    }
}

/// Split a trimmed line into its first whitespace-delimited word and the rest.
fn split_first_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    match text.find(char::is_whitespace) {
        Some(idx) => (&text[..idx], text[idx..].trim_start()),
        None => (text, ""),
    }
}

/// Strip the common leading indentation from EXAMPLE body lines and join them.
///
/// Indentation is measured and stripped in **characters**, not bytes. `trim_start`
/// is Unicode-whitespace-aware, so a byte-count minimum taken across lines indented
/// with different-width whitespace (a space on one line, U+00A0 on another) could
/// land inside a multibyte char and panic the byte slice `l[min_indent..]` with
/// "byte index N is not a char boundary" (bug-19). A char prefix is also the
/// semantically intended "common indentation".
fn dedent(lines: &[&str]) -> String {
    let leading_whitespace = |l: &str| l.chars().take_while(|c| c.is_whitespace()).count();
    let min_indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| leading_whitespace(l))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            // Only a blank line can be shorter than the common indentation (it is
            // excluded from the minimum); trim it away entirely.
            let mut chars = l.chars();
            if chars.by_ref().take(min_indent).count() == min_indent {
                chars.as_str().trim_end().to_string()
            } else {
                l.trim().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}
