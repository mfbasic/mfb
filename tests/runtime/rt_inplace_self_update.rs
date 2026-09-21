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
//! * `exempt` — the result is a new value by definition (plan-142-E), so the
//!   bound is on *live bytes*, and says `x` is read, never copied. A copy of `x`
//!   has to be live while `x` still is (the assignment has not happened), so it
//!   raises the program's peak live bytes by `|x|` widths. The statement's share of
//!   the peak — the peak with it minus the peak of the same program without it, so
//!   building `x` cancels out — may grow from `|x| = M` to `2M` by less than *half*
//!   of `M` widths plus the growth of the result's own storage (`4 * Δlen * width`,
//!   the most a list built by geometric appends can hold) and a small constant. A
//!   setup line says `{M}` for the size; `M` is above deflate's 32 KiB window, so
//!   that working table is the same size at both. (Total bytes *allocated* cannot
//!   make this distinction: these functions allocate per-block temporaries that
//!   they free as they go — plan-142-E Correction E2.)
//!
//! **Value semantics.** Every program first takes `LET before = x`, prints the
//! line's check expression over `before`, runs the loop, and prints it again: the
//! two must match. An arm that wrote through to a copy would change `before`.
//!
//! **The result.** A third program runs the statements once on `x` and computes
//! the same result through chained `LET`s (not self-updates, so the copying
//! lowering); the renderings must agree — the bound says the arm ran, this says it
//! computed the right value.
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

/// `{M}` in an `exempt` line's setup: the size of `x` for the byte check, and the
/// (small) size every other program builds.
const EXEMPT_M: u64 = 65536;
const VALUE_M: u64 = 64;

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

FUNC keepAcc(acc AS List OF Integer, n AS Integer) AS List OF Integer
  RETURN acc
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
    /// S7 — a `MUT` local inside a `FOR EACH` over itself (plan-142-F): the
    /// statements run inside the loop's first visit, which then `EXIT FOR`s, so the
    /// loop's one entry copy of `x` is taken once whatever `N` is. A line whose `x`
    /// is empty when the loop starts would never run its statements, so the program
    /// fails instead (`RETURN 3`).
    ForEach,
    /// S9 — a `MUT` captured by reference in a `collections::forEach` lambda
    /// (plan-142-G): each statement is its own `forEach(one, LAMBDA(each1 AS
    /// Integer) -> x = …)`, `one` a one-element list, so the statements interleave
    /// exactly as at S1. Every such call allocates its closure, so each program has
    /// an *idle* twin whose `one` is empty — the same closures, no statement run —
    /// and the bound is on the difference.
    Lambda,
    /// S2 — a module-level `MUT x` (plan-142-H): the program's body runs in
    /// `SUB run1()`, which `main` calls.
    Global,
}

const ENABLED_SITES: &[Site] = &[Site::Local, Site::ForEach, Site::Lambda, Site::Global];

impl Site {
    /// Whether `case` has a form at this site: no `FOR EACH` walks a `String`, and
    /// a by-ref `String` has no capacity shadow to append into (plan-142-G
    /// Correction G1). A global's lives in a hidden global (plan-142-H).
    fn applies(self, case: &Case) -> bool {
        matches!(self, Site::Local | Site::Global) || case.ty() != "String"
    }
}

/// The whole program: `x` declared as `decl` — a `main` local, or at S2 a
/// module-level global with `body` in a `SUB` — and `body` (main-body lines).
fn frame(site: Site, decl: &str, body: &str) -> String {
    let mut src = String::from(PRELUDE);
    match site {
        Site::Global => src.push_str(&format!(
            "\nMUT x AS {decl}\n\nSUB run1()\n{body}END SUB\n\n\
             FUNC main() AS Integer\n  run1()\n  RETURN 0\nEND FUNC\n"
        )),
        _ => src.push_str(&format!(
            "\nFUNC main() AS Integer\n  MUT x AS {decl}\n{body}  RETURN 0\nEND FUNC\n"
        )),
    }
    src
}

/// The main-body lines running `body` (already indented for its position) at
/// `site`: at S1 as-is, at S7 inside a `FOR EACH` over `x`'s first element, at S9
/// with every `x = …` line in a `forEach` lambda over `one` — `[0]` when `live`,
/// else empty (the idle twin).
fn at_site(site: Site, body: &str, live: bool) -> String {
    match site {
        Site::Local | Site::Global => body.to_string(),
        Site::Lambda => {
            let one = if live { "[0]" } else { "[]" };
            let mut out = format!("  LET one AS List OF Integer = {one}\n");
            for line in body.lines() {
                let indent = &line[..line.len() - line.trim_start().len()];
                match line.trim_start().strip_prefix("x = ") {
                    Some(rhs) => out.push_str(&format!(
                        "{indent}collections::forEach(one, LAMBDA(each1 AS Integer) -> x = {rhs})\n"
                    )),
                    None => out.push_str(&format!("{line}\n")),
                }
            }
            out
        }
        Site::ForEach => {
            let mut out = String::from("  MUT ran AS Boolean = FALSE\n  FOR EACH each1 IN x\n");
            for line in body.lines() {
                out.push_str(&format!("  {line}\n"));
            }
            out.push_str("    ran = TRUE\n    EXIT FOR\n  NEXT\n");
            out.push_str("  IF NOT ran THEN\n    RETURN 3\n  END IF\n");
            out
        }
    }
}

/// `text` with `{M}` replaced by `m`.
fn sized(text: &str, m: u64) -> String {
    text.replace("{M}", &m.to_string())
}

fn program(case: &Case, site: Site, n: u64, live: bool) -> String {
    let mut src = String::new();
    for line in &case.setup {
        src.push_str(&format!("  {}\n", sized(line, VALUE_M)));
    }
    src.push_str(&format!("  LET before AS {} = x\n", case.ty()));
    src.push_str(&format!("  io::print({})\n", case.check));
    let mut body = format!("  FOR i = 1 TO {n}\n");
    for statement in &case.statements {
        body.push_str(&format!("    {statement}\n"));
    }
    body.push_str("  NEXT\n");
    src.push_str(&at_site(site, &body, live));
    src.push_str(&format!("  io::print({})\n", case.check));
    src.push_str("  io::print(toString(len(x)))\n");
    frame(site, &sized(&case.decl, VALUE_M), &src)
}

/// Replace every whole-word `from` in `text` (outside string literals) by `to`.
fn replace_ident(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let c = bytes[i] as char;
        if c == '"' {
            in_string = !in_string;
        }
        let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        if !in_string
            && text[i..].starts_with(from)
            && (i == 0 || !word(bytes[i - 1]))
            && bytes.get(i + from.len()).is_none_or(|b| !word(*b))
        {
            out.push_str(to);
            i += from.len();
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The result check: run the statements once on `x` at `site` and, alongside,
/// compute the same result through chained `LET`s — `LET e1 = f(e0)` is not a
/// self-update, so it takes the copying lowering. The two renderings must agree:
/// the allocation bound says the arm ran, this says it computed the right value.
fn result_program(case: &Case, site: Site) -> String {
    let ty = case.ty();
    let mut src = String::new();
    for line in &case.setup {
        src.push_str(&format!("  {}\n", sized(line, VALUE_M)));
    }
    src.push_str(&format!("  LET e0 AS {ty} = x\n"));
    for (k, statement) in case.statements.iter().enumerate() {
        let rhs = statement.strip_prefix("x = ").unwrap_or_else(|| {
            panic!("{}: statement `{statement}` is not `x = …`", case.signature)
        });
        let rhs = replace_ident(rhs, "x", &format!("e{k}"));
        src.push_str(&format!("  LET e{} AS {ty} = {rhs}\n", k + 1));
    }
    let mut body = String::new();
    for statement in &case.statements {
        body.push_str(&format!("  {statement}\n"));
    }
    src.push_str(&at_site(site, &body, true));
    let last = format!("e{}", case.statements.len());
    src.push_str(&format!(
        "  io::print({})\n",
        replace_ident(&case.check, "before", "x")
    ));
    src.push_str(&format!(
        "  io::print({})\n",
        replace_ident(&case.check, "before", &last)
    ));
    frame(site, &sized(&case.decl, VALUE_M), &src)
}

/// The `exempt` byte check's program: `x` built at size `m`, then — when
/// `with_statement` — the line's first statement once. Prints `len(x)`.
///
/// At S7 both programs hold the statement inside the `FOR EACH`, behind a guard
/// only the run decides (`len(x) >= 0` or `< 0`), so both take the loop's entry
/// copy of `x` and the difference is the statement alone. At S9 both hold the
/// `forEach` and its closure; only the live one's `one` has an element.
fn exempt_program(case: &Case, site: Site, m: u64, with_statement: bool) -> String {
    let mut src = String::new();
    for line in &case.setup {
        src.push_str(&format!("  {}\n", sized(line, m)));
    }
    match site {
        Site::Local | Site::Global => {
            if with_statement {
                src.push_str(&format!("  {}\n", case.statements[0]));
            }
        }
        Site::ForEach => {
            let guard = if with_statement { ">=" } else { "<" };
            src.push_str(&at_site(
                site,
                &format!(
                    "  IF len(x) {guard} 0 THEN\n    {}\n  END IF\n",
                    case.statements[0]
                ),
                true,
            ));
        }
        Site::Lambda => {
            src.push_str(&at_site(
                site,
                &format!("  {}\n", case.statements[0]),
                with_statement,
            ));
        }
    }
    src.push_str("  io::print(toString(len(x)))\n");
    frame(site, &sized(&case.decl, m), &src)
}

/// Run a program: `(first output line, peak live bytes over every arena)`.
fn run_peak(name: &str, source: &str) -> Result<(u64, u64), String> {
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
    let len = stdout
        .lines()
        .next()
        .and_then(|l| l.trim().parse().ok())
        .ok_or_else(|| format!("no length printed:\n{stdout}"))?;
    let mut peak = 0u64;
    for line in stderr.lines() {
        if let Some((_, n)) = line
            .strip_prefix("arena.")
            .and_then(|rest| rest.split_once(".peak_live_bytes "))
        {
            peak += n
                .trim()
                .parse::<u64>()
                .map_err(|e| format!("bad peak_live_bytes line `{line}`: {e}"))?;
        }
    }
    Ok((len, peak))
}

/// The `exempt` bound: `x` is read, never copied.
fn exempt_check(index: usize, case: &Case, site: Site, label: &str) -> Result<(), String> {
    let width: u64 = if case.ty().ends_with("Byte") { 1 } else { 8 };
    let tag = format!("su{index}_{site:?}_ex").to_lowercase();
    let mut costs = Vec::new();
    for m in [EXEMPT_M, 2 * EXEMPT_M] {
        let (_, without) = run_peak(
            &format!("{tag}_{m}_base"),
            &exempt_program(case, site, m, false),
        )
        .map_err(|e| format!("{label} (byte check, M={m}): {e}"))?;
        let (len, with) = run_peak(
            &format!("{tag}_{m}_stmt"),
            &exempt_program(case, site, m, true),
        )
        .map_err(|e| format!("{label} (byte check, M={m}): {e}"))?;
        costs.push((with.saturating_sub(without), len));
    }
    let ((cost_m, len_m), (cost_2m, len_2m)) = (costs[0], costs[1]);
    let growth = cost_2m.saturating_sub(cost_m);
    let bound = EXEMPT_M * width / 2 + 4 * len_2m.saturating_sub(len_m) * width + 1024;
    if growth >= bound {
        return Err(format!(
            "{label}: marked `exempt`, but doubling |x| from {EXEMPT_M} to {} grew the \
             statement's peak live bytes by {growth} ({cost_m} -> {cost_2m}; result length \
             {len_m} -> {len_2m}), not under {bound} — a live copy of `x`",
            2 * EXEMPT_M
        ));
    }
    Ok(())
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
    let (b1, a1, mut once) = run(&format!("{tag}_n"), &program(case, site, n, true))
        .map_err(|e| format!("{label} (N={n}): {e}"))?;
    let (b2, a2, mut twice) = run(&format!("{tag}_2n"), &program(case, site, 2 * n, true))
        .map_err(|e| format!("{label} (N={}): {e}", 2 * n))?;
    if matches!(site, Site::Lambda) {
        // The idle twins: the closures alone.
        let (_, _, idle_once) = run(&format!("{tag}_n_idle"), &program(case, site, n, false))
            .map_err(|e| format!("{label} (idle, N={n}): {e}"))?;
        let (_, _, idle_twice) = run(
            &format!("{tag}_2n_idle"),
            &program(case, site, 2 * n, false),
        )
        .map_err(|e| format!("{label} (idle, N={}): {e}", 2 * n))?;
        once = once.saturating_sub(idle_once);
        twice = twice.saturating_sub(idle_twice);
    }
    if b1 != a1 || b2 != a2 {
        return Err(format!(
            "{label}: the copy `before` changed across the self-updates \
             (N: `{b1}` -> `{a1}`; 2N: `{b2}` -> `{a2}`) — an arm wrote through an alias"
        ));
    }
    let (in_place, copied, _) = run(&format!("{tag}_res"), &result_program(case, site))
        .map_err(|e| format!("{label} (result check): {e}"))?;
    if in_place != copied {
        return Err(format!(
            "{label}: the self-update computed `{in_place}`, the copying call `{copied}`"
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
        Status::Exempt => exempt_check(index, case, site, &label),
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
        .flat_map(|(i, c)| {
            ENABLED_SITES
                .iter()
                .filter(|s| s.applies(c))
                .map(move |s| (i, c.clone(), *s))
        })
        .collect();
    let pairs = work.len();
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
        pairs,
        failures.join("\n\n")
    );
}
