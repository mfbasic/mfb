//! Shared HTML helpers for the generated documentation and coverage-report
//! renderers (bug-340 B7).

/// Escape the five HTML metacharacters for safe embedding in generated markup.
///
/// The single home for what were two byte-identical `escape` copies, one in
/// `doc::html` and one in `coverage`.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_replaces_html_metacharacters() {
        assert_eq!(
            escape("a & b < c > d \"e\""),
            "a &amp; b &lt; c &gt; d &quot;e&quot;"
        );
        // Ordinary text passes through untouched.
        assert_eq!(escape("plain"), "plain");
    }
}
