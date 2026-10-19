//! `tests/guards/file_size_ceilings.txt`: parse it, compare it with the
//! measured counts, and lower it (`MFB_LOC_TIGHTEN=1`).
//!
//! A ceiling row is `ceiling <code|inline|test> <n> <path>`. `#` lines and blank
//! lines are comments. A ceiling lets one over-limit (file, kind) stay at `n` or
//! below; the tighten rewrite only ever lowers `n` or drops a row.

use std::collections::{BTreeMap, BTreeSet};

use crate::classify::{FileCount, Kind};

/// One parsed line of the ceiling file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    /// A comment or blank line, kept verbatim.
    Other(String),
    Ceiling {
        kind: Kind,
        n: usize,
        path: String,
    },
    /// A line that is neither (reported as STALE, kept verbatim).
    Bad(String),
}

pub fn parse(text: &str) -> Vec<Line> {
    text.lines()
        .map(|raw| {
            let t = raw.trim();
            if t.is_empty() || t.starts_with('#') {
                return Line::Other(raw.to_string());
            }
            let f: Vec<&str> = t.split_whitespace().collect();
            match f.as_slice() {
                ["ceiling", kind, n, path] => match (Kind::parse(kind), n.parse::<usize>()) {
                    (Some(kind), Ok(n)) => Line::Ceiling {
                        kind,
                        n,
                        path: path.to_string(),
                    },
                    _ => Line::Bad(raw.to_string()),
                },
                _ => Line::Bad(raw.to_string()),
            }
        })
        .collect()
}

/// One banner row.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Row {
    /// Over the limit with no ceiling.
    New { path: String, kind: Kind, n: usize },
    /// Over the limit and past its ceiling.
    Grew {
        path: String,
        kind: Kind,
        n: usize,
        ceiling: usize,
    },
    /// A ceiling that can be lowered to `n`, or dropped (`n` within the limit).
    Tighten {
        path: String,
        kind: Kind,
        n: usize,
        ceiling: usize,
    },
    /// A ceiling for a path that is not in scope, a repeated ceiling, or an unparseable line.
    Stale { what: String },
}

impl Row {
    pub fn render(&self) -> String {
        match self {
            Row::New { path, kind, n } => {
                format!("  NEW      {} {n} > {}  {path}", kind.name(), kind.limit())
            }
            Row::Grew {
                path,
                kind,
                n,
                ceiling,
            } => format!(
                "  GREW     {} {n} > {} (ceiling {ceiling})  {path}",
                kind.name(),
                kind.limit()
            ),
            Row::Tighten {
                path,
                kind,
                n,
                ceiling,
            } => {
                format!("  TIGHTEN  {} {n} (ceiling {ceiling})  {path}", kind.name())
            }
            Row::Stale { what } => format!("  STALE    {what}"),
        }
    }
}

/// What to do with one ceiling row given the measured counts.
enum Verdict {
    Keep,
    Lower(usize),
    Drop,
    Stale,
}

fn verdict(kind: Kind, n: usize, path: &str, counts: &BTreeMap<String, FileCount>) -> Verdict {
    let Some(fc) = counts.get(path) else {
        return Verdict::Stale;
    };
    let m = fc.count(kind);
    if m <= kind.limit() {
        Verdict::Drop
    } else if m < n {
        Verdict::Lower(m)
    } else {
        Verdict::Keep
    }
}

/// Every banner row for `ceilings` against `counts`, in path order.
pub fn compare(ceilings: &[Line], counts: &BTreeMap<String, FileCount>) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut seen: BTreeMap<(String, Kind), usize> = BTreeMap::new();
    for (i, line) in ceilings.iter().enumerate() {
        match line {
            Line::Other(_) => {}
            Line::Bad(raw) => rows.push(Row::Stale {
                what: format!("unparseable line {}: {}", i + 1, raw.trim()),
            }),
            Line::Ceiling { kind, n, path } => {
                if seen.contains_key(&(path.clone(), *kind)) {
                    rows.push(Row::Stale {
                        what: format!("repeated ceiling {} {n}  {path}", kind.name()),
                    });
                    continue;
                }
                seen.insert((path.clone(), *kind), *n);
                match verdict(*kind, *n, path, counts) {
                    Verdict::Stale => rows.push(Row::Stale {
                        what: format!(
                            "{} (ceiling {n})  {path}: not an in-scope file",
                            kind.name()
                        ),
                    }),
                    Verdict::Drop | Verdict::Lower(_) => rows.push(Row::Tighten {
                        path: path.clone(),
                        kind: *kind,
                        n: counts[path].count(*kind),
                        ceiling: *n,
                    }),
                    Verdict::Keep => {}
                }
            }
        }
    }
    for (path, fc) in counts {
        for (kind, m) in fc.kinds() {
            if m <= kind.limit() {
                continue;
            }
            match seen.get(&(path.clone(), kind)) {
                None => rows.push(Row::New {
                    path: path.clone(),
                    kind,
                    n: m,
                }),
                Some(&c) if m > c => rows.push(Row::Grew {
                    path: path.clone(),
                    kind,
                    n: m,
                    ceiling: c,
                }),
                Some(_) => {}
            }
        }
    }
    rows.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
    rows
}

fn sort_key(row: &Row) -> (u8, &str, Option<Kind>) {
    match row {
        Row::New { path, kind, .. } => (0, path, Some(*kind)),
        Row::Grew { path, kind, .. } => (1, path, Some(*kind)),
        Row::Stale { what } => (2, what, None),
        Row::Tighten { path, kind, .. } => (3, path, Some(*kind)),
    }
}

/// The ceiling file rewritten by `MFB_LOC_TIGHTEN=1`: every ceiling lowered to
/// its measured count, ceilings for files now within the limit or no longer in
/// scope dropped, repeated ceilings dropped. Never adds a row, never raises one.
/// Comments and unparseable lines are kept as written.
pub fn tighten(ceilings: &[Line], counts: &BTreeMap<String, FileCount>) -> String {
    let mut out = String::new();
    let mut seen = BTreeSet::new();
    for line in ceilings {
        match line {
            Line::Other(raw) | Line::Bad(raw) => {
                out.push_str(raw);
                out.push('\n');
            }
            Line::Ceiling { kind, n, path } => {
                if !seen.insert((path.clone(), *kind)) {
                    continue;
                }
                let n = match verdict(*kind, *n, path, counts) {
                    Verdict::Keep => *n,
                    Verdict::Lower(m) => m,
                    Verdict::Drop | Verdict::Stale => continue,
                };
                out.push_str(&format!("ceiling {} {n} {path}\n", kind.name()));
            }
        }
    }
    out
}
