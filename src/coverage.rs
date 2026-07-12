//! `mfb test --coverage` report generation (plan-18-C).
//!
//! The compiler instruments each user statement with a counter increment keyed to
//! a compile-time slot map, and writes that map to a `coverage.covmap.json`
//! sidecar. The instrumented binary writes its raw counts to `coverage.covdata`
//! (and any failed-case source lines to `coverage.covfail`) as it exits. This
//! module folds the three together into `coverage.html`: a file tree with
//! per-file line-coverage stats and color-coded, annotated source.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

/// One instrumented statement: the project-relative source file and 1-based line.
#[derive(Clone, Debug)]
pub(crate) struct CovSlot {
    pub(crate) file: String,
    pub(crate) line: usize,
}

/// Serialize the slot map to the `coverage.covmap.json` sidecar. Written by the
/// compiler during a `--coverage` build so `mfb test` can fold counts back to
/// source lines. The format is a small hand-emitted JSON array (no dependency).
pub(crate) fn write_covmap(path: &Path, slots: &[CovSlot]) -> std::io::Result<()> {
    let mut out = String::from("{\"slots\":[");
    for (index, slot) in slots.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"file\":{},\"line\":{}}}",
            crate::json_string(&slot.file),
            slot.line
        ));
    }
    out.push_str("]}\n");
    fs::write(path, out)
}

/// Parse a `coverage.covmap.json` sidecar back into the slot list. Tolerant of
/// the exact whitespace written by [`write_covmap`].
pub(crate) fn read_covmap(path: &Path) -> Option<Vec<CovSlot>> {
    let text = fs::read_to_string(path).ok()?;
    let value: tinyjson::JsonValue = text.parse().ok()?;
    let slots = value.get::<std::collections::HashMap<String, tinyjson::JsonValue>>()?;
    let array = slots.get("slots")?.get::<Vec<tinyjson::JsonValue>>()?;
    let mut result = Vec::with_capacity(array.len());
    for entry in array {
        let object = entry.get::<std::collections::HashMap<String, tinyjson::JsonValue>>()?;
        let file = object.get("file")?.get::<String>()?.clone();
        let line = *object.get("line")?.get::<f64>()? as usize;
        result.push(CovSlot { file, line });
    }
    Some(result)
}

/// Read a `coverage.covdata` file (one count per slot, in slot order).
pub(crate) fn read_counts(path: &Path) -> Vec<u64> {
    fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter(|line| !line.is_empty())
                .map(|line| line.trim().parse::<u64>().unwrap_or(0))
                .collect()
        })
        .unwrap_or_default()
}

/// Read a `coverage.covfail` file (`file:line` per line) into a set keyed by
/// `(file, line)`. Missing file → no annotations.
pub(crate) fn read_failed(path: &Path) -> HashSet<(String, usize)> {
    fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter_map(|line| {
                    let (file, number) = line.rsplit_once(':')?;
                    Some((file.to_string(), number.trim().parse::<usize>().ok()?))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Per-line state within a file view.
#[derive(Clone, Copy, PartialEq)]
enum LineState {
    /// No instrumented statement on this line.
    Neutral,
    /// Instrumented and executed at least once.
    Covered,
    /// Instrumented but never executed.
    Uncovered,
}

struct FileReport {
    /// Project-relative path.
    path: String,
    lines: Vec<(LineState, bool, String)>, // (state, is_failed, source)
    covered: usize,
    total: usize,
}

/// Fold the slot map, counts, and failed lines against the on-disk source into a
/// per-file report.
fn build_reports(
    project_dir: &Path,
    slots: &[CovSlot],
    counts: &[u64],
    failed: &HashSet<(String, usize)>,
) -> Vec<FileReport> {
    // line -> executed? per file (a line may host several statements; it counts
    // as covered iff any of its slots ran).
    let mut file_lines: BTreeMap<String, BTreeMap<usize, bool>> = BTreeMap::new();
    for (index, slot) in slots.iter().enumerate() {
        let executed = counts.get(index).copied().unwrap_or(0) > 0;
        let entry = file_lines
            .entry(slot.file.clone())
            .or_default()
            .entry(slot.line)
            .or_insert(false);
        *entry = *entry || executed;
    }

    let mut reports = Vec::new();
    for (path, hit_lines) in file_lines {
        let source = fs::read_to_string(project_dir.join(&path)).unwrap_or_default();
        let mut lines = Vec::new();
        let mut covered = 0;
        let mut total = 0;
        for (offset, text) in source.lines().enumerate() {
            let line_no = offset + 1;
            let state = match hit_lines.get(&line_no) {
                Some(true) => {
                    covered += 1;
                    total += 1;
                    LineState::Covered
                }
                Some(false) => {
                    total += 1;
                    LineState::Uncovered
                }
                None => LineState::Neutral,
            };
            let is_failed = failed.contains(&(path.clone(), line_no));
            lines.push((state, is_failed, text.to_string()));
        }
        reports.push(FileReport {
            path,
            lines,
            covered,
            total,
        });
    }
    reports
}

/// Build `coverage.html` from the slot map, counts, and failed-line set, reading
/// each source file relative to `project_dir`.
pub(crate) fn generate_html(
    project_dir: &Path,
    slots: &[CovSlot],
    counts: &[u64],
    failed: &HashSet<(String, usize)>,
) -> String {
    let reports = build_reports(project_dir, slots, counts, failed);
    let total_covered: usize = reports.iter().map(|report| report.covered).sum();
    let total_lines: usize = reports.iter().map(|report| report.total).sum();

    // Assign each report a unique HTML anchor up front, using one shared `used`
    // set, so the index link and the section id agree and distinct paths that
    // slugify to the same base (e.g. `a/b.mfb` vs `a.b.mfb`) get distinct ids
    // (bug-93.1).
    let mut used_anchors: HashSet<String> = HashSet::new();
    let mut anchors: BTreeMap<String, String> = BTreeMap::new();
    for report in &reports {
        let id = anchor(&report.path, &mut used_anchors);
        anchors.insert(report.path.clone(), id);
    }

    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<title>MFB coverage</title>\n<style>\n");
    out.push_str(STYLE);
    out.push_str("</style>\n</head>\n<body>\n");
    out.push_str("<h1>Coverage</h1>\n");
    out.push_str(&format!(
        "<p class=\"summary\">Total: {} / {} lines ({})</p>\n",
        total_covered,
        total_lines,
        percent(total_covered, total_lines)
    ));

    // File tree / index.
    out.push_str("<table class=\"tree\">\n<thead><tr><th>File</th><th>Covered</th><th>%</th></tr></thead>\n<tbody>\n");
    for report in &reports {
        out.push_str(&format!(
            "<tr><td><a href=\"#{anchor}\">{path}</a></td><td>{covered} / {total}</td><td>{pct}</td></tr>\n",
            anchor = anchors[&report.path],
            path = escape(&report.path),
            covered = report.covered,
            total = report.total,
            pct = percent(report.covered, report.total)
        ));
    }
    out.push_str("</tbody>\n</table>\n");

    // Per-file annotated source.
    for report in &reports {
        out.push_str(&format!(
            "<section class=\"file\" id=\"{anchor}\">\n<h2>{path} <span class=\"stat\">{covered} / {total} ({pct})</span></h2>\n<pre>\n",
            anchor = anchors[&report.path],
            path = escape(&report.path),
            covered = report.covered,
            total = report.total,
            pct = percent(report.covered, report.total)
        ));
        for (offset, (state, is_failed, text)) in report.lines.iter().enumerate() {
            let mut class = match state {
                LineState::Covered => "cov",
                LineState::Uncovered => "unc",
                LineState::Neutral => "neu",
            }
            .to_string();
            if *is_failed {
                class.push_str(" fail");
            }
            out.push_str(&format!(
                "<span class=\"line {class}\"><span class=\"ln\">{num:>5}</span>{marker}{src}</span>\n",
                class = class,
                num = offset + 1,
                marker = if *is_failed { " ✗ " } else { "   " },
                src = escape(text)
            ));
        }
        out.push_str("</pre>\n</section>\n");
    }

    out.push_str("</body>\n</html>\n");
    out
}

fn percent(covered: usize, total: usize) -> String {
    if total == 0 {
        "—".to_string()
    } else {
        format!("{:.0}%", (covered as f64 / total as f64) * 100.0)
    }
}

/// Slugify a source path into a unique HTML anchor id. De-duplicates with a `-N`
/// suffix (mirrors `doc::anchor`) so distinct paths that slugify to the same base
/// (e.g. `a/b.mfb` and `a.b.mfb`) never collide into the same id (bug-93.1).
fn anchor(path: &str, used: &mut HashSet<String>) -> String {
    let base: String = path
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let mut candidate = base.clone();
    let mut n = 2;
    while used.contains(&candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

fn escape(text: &str) -> String {
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

const STYLE: &str = "\
body { font-family: -apple-system, system-ui, sans-serif; margin: 2rem; color: #1a1a1a; }
h1 { font-size: 1.4rem; }
.summary { font-weight: 600; }
table.tree { border-collapse: collapse; margin-bottom: 2rem; }
table.tree th, table.tree td { text-align: left; padding: 0.2rem 0.8rem; border-bottom: 1px solid #ddd; }
section.file h2 { font-size: 1rem; border-bottom: 2px solid #ccc; padding-top: 1rem; }
section.file .stat { color: #666; font-weight: normal; font-size: 0.85rem; }
pre { background: #fafafa; border: 1px solid #eee; padding: 0; overflow-x: auto; }
.line { display: block; font-family: ui-monospace, Menlo, monospace; font-size: 0.82rem; white-space: pre; }
.line .ln { display: inline-block; width: 4rem; text-align: right; color: #999; padding-right: 0.6rem; user-select: none; }
.line.cov { background: #e6f6e6; }
.line.unc { background: #fde0e0; }
.line.neu { background: transparent; }
.line.fail { background: #ffd1a3; }
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_paths_that_slugify_alike_get_distinct_anchors() {
        // `a/b.mfb` and `a.b.mfb` both slugify to `a-b-mfb`; the shared `used` set
        // must disambiguate them so the two reports never share an HTML id
        // (bug-93.1).
        let mut used = HashSet::new();
        let first = anchor("a/b.mfb", &mut used);
        let second = anchor("a.b.mfb", &mut used);
        assert_ne!(first, second);
        assert_eq!(first, "a-b-mfb");
        assert_eq!(second, "a-b-mfb-2");
    }

    #[test]
    fn covmap_round_trips_through_disk() {
        // `write_covmap` emits the sidecar JSON; `read_covmap` folds it back to the
        // same slot list (order preserved, paths with a quote escaped by
        // `json_string`).
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("coverage.covmap.json");
        let slots = vec![
            CovSlot {
                file: "main.mfb".to_string(),
                line: 3,
            },
            CovSlot {
                file: "pkg/a\"b.mfb".to_string(),
                line: 17,
            },
        ];
        write_covmap(&path, &slots).expect("write covmap");
        let back = read_covmap(&path).expect("read covmap");
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].file, "main.mfb");
        assert_eq!(back[0].line, 3);
        assert_eq!(back[1].file, "pkg/a\"b.mfb");
        assert_eq!(back[1].line, 17);
    }

    #[test]
    fn read_covmap_returns_none_for_missing_or_malformed_input() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Missing file.
        assert!(read_covmap(&dir.path().join("nope.json")).is_none());
        // Present but not the expected shape.
        let garbage = dir.path().join("garbage.json");
        fs::write(&garbage, "not json at all").expect("write");
        assert!(read_covmap(&garbage).is_none());
    }

    #[test]
    fn read_counts_parses_numbers_and_tolerates_blank_or_junk_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("counts");
        fs::write(&path, "3\n\n  5  \nabc\n0\n").expect("write");
        // Blank lines are dropped; a non-numeric line parses to 0.
        assert_eq!(read_counts(&path), vec![3, 5, 0, 0]);
        // Missing file → empty.
        assert!(read_counts(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn read_failed_parses_file_colon_line_and_skips_unparseable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fail");
        // `pkg/a.mfb:3` splits on the *last* colon; a line without a colon and a
        // line whose number won't parse are both dropped.
        fs::write(&path, "main.mfb:12\npkg/a.mfb:3\nno-colon-here\nx:notanumber\n")
            .expect("write");
        let failed = read_failed(&path);
        assert_eq!(failed.len(), 2);
        assert!(failed.contains(&("main.mfb".to_string(), 12)));
        assert!(failed.contains(&("pkg/a.mfb".to_string(), 3)));
        // Missing file → empty set.
        assert!(read_failed(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn percent_formats_ratio_and_dashes_the_empty_case() {
        assert_eq!(percent(1, 1), "100%");
        assert_eq!(percent(1, 2), "50%");
        assert_eq!(percent(0, 4), "0%");
        // No instrumented lines → an em dash rather than a divide-by-zero.
        assert_eq!(percent(0, 0), "—");
    }

    #[test]
    fn escape_replaces_html_metacharacters() {
        assert_eq!(
            escape("a & b < c > d \"e\""),
            "a &amp; b &lt; c &gt; d &quot;e&quot;"
        );
        // Ordinary text passes through untouched.
        assert_eq!(escape("plain"), "plain");
    }

    #[test]
    fn generate_html_annotates_covered_uncovered_neutral_and_failed_lines() {
        // A two-line source: line 2 is instrumented and executed (covered), line 3
        // is instrumented but never ran (uncovered) *and* recorded as failed; line
        // 1 hosts no slot (neutral). The `<` in line 1 exercises source escaping.
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("main.mfb"),
            "REM a < b\n  x = 1\n  y = 2\n",
        )
        .expect("write source");

        let slots = vec![
            CovSlot {
                file: "main.mfb".to_string(),
                line: 2,
            },
            CovSlot {
                file: "main.mfb".to_string(),
                line: 3,
            },
        ];
        let counts = vec![1, 0];
        let mut failed = HashSet::new();
        failed.insert(("main.mfb".to_string(), 3));

        let html = generate_html(dir.path(), &slots, &counts, &failed);

        // Summary: 1 of 2 instrumented lines covered.
        assert!(html.contains("Total: 1 / 2 lines (50%)"), "{html}");
        // The file appears in the index with an anchored link.
        assert!(html.contains("<a href=\"#main-mfb\">main.mfb</a>"), "{html}");
        // Line 2 executed → covered; line 3 never ran and failed → uncovered + fail.
        assert!(html.contains("class=\"line cov\""), "{html}");
        assert!(html.contains("class=\"line unc fail\""), "{html}");
        // The failed line carries the ✗ marker; neutral line 1's source is escaped.
        assert!(html.contains(" ✗ "), "{html}");
        assert!(html.contains("REM a &lt; b"), "{html}");
        assert!(html.contains("class=\"line neu\""), "{html}");
    }
}
