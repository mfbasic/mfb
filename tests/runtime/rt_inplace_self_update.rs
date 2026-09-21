//! plan-142-A: a self-update `x = f(x, …)` of a collection does not copy `x`.
//!
//! Every line of `inplace_self_update/cases.tsv` names one self-update-shaped
//! builtin overload (plus operator self-updates) and a statement exercising it.
//! For each line and each enabled binding site this builds the statement run `N`
//! and `2N` times with `mfb build --debug` and reads the debug report's
//! `arena.<k>.alloc_calls` — the number of blocks the program allocated.
//!
//! **The measure.** A copying lowering allocates a fresh block per statement, so
//! the extra `N` runs allocate at least `N` more blocks. An in-place lowering
//! allocates only when the collection outgrows its capacity (geometric growth), so
//! the extra `N` runs allocate a handful. The two bounds:
//!
//! * `arm` — `count(2N) - count(N) < N / 8`.
//! * `pending:<letter>` — `count(2N) - count(N) >= N`: the line still copies. A
//!   letter that lands the arm must flip its line, so an improvement that nobody
//!   recorded fails here too.
//! * `exempt` — no allocation bound (the result is a new value by definition);
//!   the value check below only.
//!
//! **Value semantics.** Every program first takes `LET before = x`, prints the
//! line's check expression over `before`, runs the loop, and prints it again: the
//! two must match. An arm that wrote through to a copy would change `before`.
//!
//! The census that keeps `cases.tsv` complete is
//! `tests/guards/inplace_self_update_census.rs`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

const DEFAULT_N: u64 = 2000;

const CASES: &str = include_str!("inplace_self_update/cases.tsv");

/// Helper functions every program carries: callbacks the statements pass, and the
/// renderers the check expressions call.
const PRELUDE: &str = "\
IMPORT collections
IMPORT compress
IMPORT crypto
IMPORT encoding
IMPORT io
IMPORT math

FUNC isPositive(n AS Integer) AS Boolean
  RETURN n > 0
END FUNC

FUNC negated(n AS Integer) AS Integer
  RETURN 0 - n
END FUNC

FUNC push(acc AS List OF Integer, n AS Integer) AS List OF Integer
  RETURN collections::append(acc, n)
END FUNC

FUNC showInts(xs AS List OF Integer) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showFloats(xs AS List OF Float) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showFixed(xs AS List OF Fixed) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showBytes(xs AS List OF Byte) AS String
  RETURN encoding::hexEncode(xs)
END FUNC

FUNC showSet(xs AS Set OF Integer) AS String
  MUT out AS String = \"\"
  FOR EACH v IN xs
    out = out & toString(v) & \",\"
  NEXT
  RETURN out
END FUNC

FUNC showMap(m AS Map OF String TO Integer) AS String
  MUT out AS String = \"\"
  FOR EACH k IN collections::keys(m)
    out = out & k & \"=\" & toString(collections::get(m, k)) & \",\"
  NEXT
  RETURN out
END FUNC
";

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Arm,
    Exempt,
    Pending(String),
}

#[derive(Clone, Debug)]
struct Case {
    signature: String,
    status: Status,
    /// `<type> = <init>` for `x`.
    decl: String,
    /// Further setup statements, run once before `before` is taken.
    setup: Vec<String>,
    statements: Vec<String>,
    check: String,
    n: u64,
}

impl Case {
    fn ty(&self) -> &str {
        self.decl
            .split_once(" = ")
            .map(|(ty, _)| ty)
            .unwrap_or_else(|| panic!("{}: setup has no `<type> = <init>`", self.signature))
    }
}

fn cases() -> Vec<Case> {
    CASES
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            assert!(
                cols.len() == 5 || cols.len() == 6,
                "cases.tsv line has {} columns, want 5 or 6: {line}",
                cols.len()
            );
            let status = match cols[1] {
                "arm" => Status::Arm,
                "exempt" => Status::Exempt,
                s => match s.strip_prefix("pending:") {
                    Some(letter) if !letter.is_empty() => Status::Pending(letter.to_string()),
                    _ => panic!("cases.tsv: unknown status `{s}` in: {line}"),
                },
            };
            let mut setup = cols[2].split(" ; ").map(str::to_string);
            let decl = setup.next().unwrap_or_default();
            Case {
                signature: cols[0].to_string(),
                status,
                decl,
                setup: setup.collect(),
                statements: cols[3].split(" ; ").map(str::to_string).collect(),
                check: cols[4].to_string(),
                n: cols
                    .get(5)
                    .map(|n| n.parse().expect("cases.tsv: n is not a number"))
                    .unwrap_or(DEFAULT_N),
            }
        })
        .collect()
}

/// The binding sites the statement runs at. plan-142-F, G and H each add theirs.
#[derive(Clone, Copy, Debug)]
enum Site {
    /// S1 — a `MUT` local in a function body.
    Local,
}

const ENABLED_SITES: &[Site] = &[Site::Local];

fn program(case: &Case, site: Site, n: u64) -> String {
    let mut src = String::from(PRELUDE);
    match site {
        Site::Local => {
            src.push_str("\nFUNC main() AS Integer\n");
            src.push_str(&format!("  MUT x AS {}\n", case.decl));
            for line in &case.setup {
                src.push_str(&format!("  {line}\n"));
            }
            src.push_str(&format!("  LET before AS {} = x\n", case.ty()));
            src.push_str(&format!("  io::print({})\n", case.check));
            src.push_str(&format!("  FOR i = 1 TO {n}\n"));
            for statement in &case.statements {
                src.push_str(&format!("    {statement}\n"));
            }
            src.push_str("  NEXT\n");
            src.push_str(&format!("  io::print({})\n", case.check));
            src.push_str("  io::print(toString(len(x)))\n  RETURN 0\nEND FUNC\n");
        }
    }
    src
}

/// Build `source` with `--debug` and return the executable (the host glibc one on
/// Linux).
fn build_debug(name: &str, source: &str) -> Result<PathBuf, String> {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .map_err(|e| format!("run mfb build: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        return Err(format!(
            "build failed:\n{stdout}\n{}\n--- source ---\n{source}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .map(PathBuf::from)
        .ok_or_else(|| format!("no executable in build output:\n{stdout}"))
}

/// Run the program: `(check before, check after, perf.mfb_alloc.count)`.
fn run(name: &str, source: &str) -> Result<(String, String, u64), String> {
    let exe = build_debug(name, source)?;
    let output = Command::new(&exe)
        .output()
        .map_err(|e| format!("run: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "program failed:\n{stdout}\n{stderr}\n--- source ---\n{source}"
        ));
    }
    let mut lines = stdout.lines();
    let before = lines.next().unwrap_or_default().to_string();
    let after = lines.next().unwrap_or_default().to_string();
    // Every arena's `alloc_calls`, summed. (`perf.mfb_alloc.count` is not usable
    // here: it counts the perf log's samples, and the log stops recording when full —
    // a compress round trip allocates enough to fill it, and N and 2N then read the
    // same number.)
    let mut count = 0u64;
    let mut seen = false;
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix("arena.") else {
            continue;
        };
        let Some((_, n)) = rest.split_once(".alloc_calls ") else {
            continue;
        };
        count += n
            .trim()
            .parse::<u64>()
            .map_err(|e| format!("bad alloc_calls line `{line}`: {e}"))?;
        seen = true;
    }
    if !seen {
        return Err(format!("no arena.*.alloc_calls in:\n{stderr}"));
    }
    Ok((before, after, count))
}

/// Check one case at one site; `Err` carries the failure message.
fn check(index: usize, case: &Case, site: Site) -> Result<(), String> {
    let label = format!("{} at {site:?}", case.signature);
    let n = case.n;
    let tag = format!("su{index}_{site:?}").to_lowercase();
    let (b1, a1, once) = run(&format!("{tag}_n"), &program(case, site, n))
        .map_err(|e| format!("{label} (N={n}): {e}"))?;
    let (b2, a2, twice) = run(&format!("{tag}_2n"), &program(case, site, 2 * n))
        .map_err(|e| format!("{label} (N={}): {e}", 2 * n))?;
    if b1 != a1 || b2 != a2 {
        return Err(format!(
            "{label}: the copy `before` changed across the self-updates \
             (N: `{b1}` -> `{a1}`; 2N: `{b2}` -> `{a2}`) — an arm wrote through an alias"
        ));
    }
    let extra = twice.saturating_sub(once);
    match &case.status {
        Status::Arm if extra >= n / 8 => Err(format!(
            "{label}: marked `arm`, but {n} more runs allocated {extra} more blocks \
             ({once} at N={n}, {twice} at 2N) — the statement copies"
        )),
        Status::Pending(letter) if extra < n => Err(format!(
            "{label}: marked `pending:{letter}`, but {n} more runs allocated only {extra} \
             more blocks ({once} -> {twice}) — it no longer copies; flip the line to `arm`"
        )),
        _ => Ok(()),
    }
}

#[test]
fn every_self_update_case_meets_its_allocation_bound() {
    let mut cases = cases();
    assert!(!cases.is_empty(), "cases.tsv has no case");
    // `MFB_SELF_UPDATE_FILTER=<substring>[|<substring>…]` runs only the lines whose
    // signature contains one of them — for iterating on an arm; the full run is
    // the gate.
    if let Ok(filter) = std::env::var("MFB_SELF_UPDATE_FILTER") {
        cases.retain(|c| filter.split('|').any(|part| c.signature.contains(part)));
        assert!(
            !cases.is_empty(),
            "MFB_SELF_UPDATE_FILTER={filter} matches no case"
        );
    }
    let work: Vec<(usize, Case, Site)> = cases
        .iter()
        .enumerate()
        .flat_map(|(i, c)| ENABLED_SITES.iter().map(move |s| (i, c.clone(), *s)))
        .collect();
    let queue = Mutex::new(work);
    let failures = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let Some((i, case, site)) = queue.lock().unwrap().pop() else {
                    break;
                };
                if let Err(e) = check(i, &case, site) {
                    failures.lock().unwrap().push(e);
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    assert!(
        failures.is_empty(),
        "{} of {} case/site pair(s) failed:\n\n{}",
        failures.len(),
        cases.len() * ENABLED_SITES.len(),
        failures.join("\n\n")
    );
}
