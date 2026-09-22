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
//! the extra `N` runs allocate a handful. The two bounds — and the only two
//! statuses a finished line may have (plan-142-I):
//!
//! * `arm` — `count(2N) - count(N) < N / 8`.
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
//! plan-146-A reopens two statuses for the `String` lines it adds, each asserting
//! the statement still copies (`count(2N) - count(N) >= N`), so the letter that
//! changes it has to flip the line:
//!
//! * `pending:<letter>` — the plan-146 letter that lands the arm or the exemption
//!   (plan-146-H deletes this status again);
//! * `deferred:<tag>` — another plan owns the row (`attributed-string`).
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
//!
//! **Field sites (plan-145-A).** Every line also runs at the 15 field sites of
//! plan-144's audit — a record field (`S3`–`S10`) and a `RES … STATE` payload field
//! (`T1`–`T8`) — with `x = f(x, …)` rewritten to update the field:
//! `r = WITH r { b := f(r.b, …) }`, `h.state.b = f(h.state.b, …)`. Each (line,
//! site) pair's expected outcome is in `inplace_self_update/field_expect.tsv`:
//!
//! * `arm` — meets the line's bound (above);
//! * `rebuild:<reason>` / `deferred:<plan>` — still rebuilds, and is meant to;
//! * `na:<diagnostic>` — the program does not compile, with that diagnostic.
//!
//! `inplace_self_update/field_kinds.tsv` does the same for every field KIND that is
//! not a collection (the scalars, `String`, `json::Json`, every package record
//! type). A kind whose new value allocates on its own has the bound `arm+value`:
//! `count(2N) - count(N) <= 1.125 * (control(2N) - control(N))`, the control
//! program computing the same value and handing it to a no-op `SUB kindSink`
//! instead of storing it (an assignment would add the copy of a borrowed value).
//! At a field site the program also prints a sibling field before and after the
//! loop, so a write that lands on the wrong part of the owner fails.
//!
//! `MFB_SELF_UPDATE_SITES=<code>[,<code>…]` runs only those sites (`Local`,
//! `ForEach`, `Lambda`, `Global`, `S3`…`S10`, `T1`…`T8`); `MFB_SELF_UPDATE_FILTER`
//! narrows the lines, by signature or kind.

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
const FIELD_EXPECT: &str = include_str!("inplace_self_update/field_expect.tsv");
const FIELD_KINDS: &str = include_str!("inplace_self_update/field_kinds.tsv");

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
    /// plan-146-A: still copies until the named plan-146 letter lands.
    Pending(String),
    /// plan-146-A: still copies; the named plan owns it.
    Deferred(String),
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

/// A `String` expression for the length of `value`, of type `ty`: `len` for a
/// collection or a `String`; `len` has no `AttributedString` overload, so its byte
/// length (plan-146-A).
fn len_of(ty: &str, value: &str) -> String {
    if ty == "AttributedString" {
        format!("toString(strings::byteLen({value}))")
    } else {
        format!("toString(len({value}))")
    }
}

/// `IMPORT` lines for every package `rest` names (`pkg::`) that `prelude` does not
/// import already.
fn extra_imports(prelude: &str, rest: &str) -> String {
    let mut imported: Vec<&str> = prelude
        .lines()
        .filter_map(|l| l.strip_prefix("IMPORT "))
        .collect();
    let mut extra = String::new();
    for (i, _) in rest.match_indices("::") {
        let start = rest[..i]
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map_or(0, |p| p + 1);
        let pkg = &rest[start..i];
        if pkg.starts_with(|c: char| c.is_ascii_lowercase()) && !imported.contains(&pkg) {
            imported.push(pkg);
            extra.push_str(&format!("IMPORT {pkg}\n"));
        }
    }
    extra
}

/// [`PRELUDE`] plus an `IMPORT` for every further package `rest` names.
fn prelude_for(rest: &str) -> String {
    let extra = extra_imports(PRELUDE, rest);
    PRELUDE.replacen("IMPORT math\n", &format!("IMPORT math\n{extra}"), 1)
}

/// The number of stdin lines fed to a program that reads them (`io::input`):
/// more than any line's `2N` statements read.
const STDIN_LINES: usize = 4 * DEFAULT_N as usize + 64;

/// Run `command`, feeding it [`STDIN_LINES`] empty lines when `source` reads stdin.
fn output_with_stdin(command: &mut Command, source: &str) -> std::io::Result<std::process::Output> {
    if !source.contains("io::input(") {
        return command.output();
    }
    use std::io::Write;
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let feeder = std::thread::spawn(move || {
        // The program may exit before reading everything; a broken pipe is fine.
        let _ = stdin.write_all("\n".repeat(STDIN_LINES).as_bytes());
    });
    let output = child.wait_with_output();
    let _ = feeder.join();
    output
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
            let status = match cols[1].split_once(':') {
                None if cols[1] == "arm" => Status::Arm,
                None if cols[1] == "exempt" => Status::Exempt,
                Some(("pending", letter)) if !letter.is_empty() => {
                    Status::Pending(letter.to_string())
                }
                Some(("deferred", tag)) if !tag.is_empty() => Status::Deferred(tag.to_string()),
                _ => panic!(
                    "cases.tsv: status `{}` is not `arm`, `exempt`, `pending:<letter>` or \
                     `deferred:<tag>` in: {line} — a self-update needs an in-place arm or a \
                     proven exemption",
                    cols[1]
                ),
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
    /// Whether `case` has a form at this site: no `FOR EACH` walks a `String`
    /// (`TYPE_FOR_EACH_REQUIRES_COLLECTION`). A global's shadow lives in a hidden
    /// global (plan-142-H), and a by-ref capture shares its owner's through the
    /// closure environment (plan-146-G), so a `String` runs everywhere but S7. An
    /// `AttributedString` (plan-146-A, deferred) runs at S1 and S2 only.
    fn applies(self, case: &Case) -> bool {
        match case.ty() {
            "String" => !matches!(self, Site::ForEach),
            "AttributedString" => matches!(self, Site::Local | Site::Global),
            _ => true,
        }
    }
}

/// The whole program: `x` declared as `decl` — a `main` local, or at S2 a
/// module-level global with `body` in a `SUB` — and `body` (main-body lines).
fn frame(site: Site, decl: &str, body: &str) -> String {
    let rest = match site {
        Site::Global => format!(
            "\nMUT x AS {decl}\n\nSUB run1()\n{body}END SUB\n\n\
             FUNC main() AS Integer\n  run1()\n  RETURN 0\nEND FUNC\n"
        ),
        _ => format!("\nFUNC main() AS Integer\n  MUT x AS {decl}\n{body}  RETURN 0\nEND FUNC\n"),
    };
    format!("{}{rest}", prelude_for(&rest))
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
    src.push_str(&format!("  io::print({})\n", len_of(case.ty(), "x")));
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
    src.push_str(&format!("  io::print({})\n", len_of(case.ty(), "x")));
    frame(site, &sized(&case.decl, m), &src)
}

/// Run a program: `(first output line, peak live bytes over every arena)`.
fn run_peak(name: &str, source: &str) -> Result<(u64, u64), String> {
    let exe = build_debug(name, source)?;
    let output =
        output_with_stdin(&mut Command::new(&exe), source).map_err(|e| format!("run: {e}"))?;
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
    let (lines, count) = run_lines(name, source, Build::Console)?;
    let mut lines = lines.into_iter();
    let before = lines.next().unwrap_or_default();
    let after = lines.next().unwrap_or_default();
    Ok((before, after, count))
}

/// How a program is built: a console program, or an app-mode one (a `canvas::`
/// kind: the package requires app mode), run headless.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Build {
    Console,
    App,
}

/// Build `source` with `--debug` — and `-app` for [`Build::App`] — and return
/// `(the executable, its project directory)`.
fn build_debug_as(name: &str, source: &str, build: Build) -> Result<(PathBuf, PathBuf), String> {
    if build == Build::Console {
        let exe = build_debug(name, source)?;
        let project = exe
            .ancestors()
            .find(|dir| dir.join("project.json").is_file())
            .map(PathBuf::from)
            .ok_or_else(|| format!("no project above {}", exe.display()))?;
        return Ok((exe, project));
    }
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(common::APP_BUILD_FLAG)
        .arg("--debug")
        .arg(&project)
        .output()
        .map_err(|e| format!("run mfb build: {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&project);
        return Err(format!(
            "build failed:\n{}\n{}\n--- source ---\n{source}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok((common::app_binary(&project, name), project))
}

/// Run the program: `(every stdout line, the arenas' summed alloc_calls)`. The
/// project is removed afterwards: a field run builds thousands of them.
fn run_lines(name: &str, source: &str, build: Build) -> Result<(Vec<String>, u64), String> {
    let (exe, project) = build_debug_as(name, source, build)?;
    let mut command = Command::new(&exe);
    if build == Build::App {
        command
            .env("MFB_MACAPP_HEADLESS", "1")
            .env("MFB_WINAPP_HEADLESS", "1")
            .env("MFB_GTKAPP_HEADLESS", "1");
    }
    let output = output_with_stdin(&mut command, source);
    let _ = std::fs::remove_dir_all(&project);
    let output = output.map_err(|e| format!("run: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "program failed:\n{stdout}\n{stderr}\n--- source ---\n{source}"
        ));
    }
    let lines: Vec<String> = stdout.lines().map(str::to_string).collect();
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
    Ok((lines, count))
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
        Status::Exempt => exempt_check(index, case, site, &label),
        Status::Arm => Ok(()),
        // plan-146-A: a copy allocates at least one block per statement run.
        Status::Pending(owner) | Status::Deferred(owner) if extra < n => Err(format!(
            "{label}: marked `{}`, but {n} more runs allocated only {extra} more blocks \
             ({once} at N={n}, {twice} at 2N) — the statement no longer copies; flip the \
             line (owner: {owner})",
            if matches!(case.status, Status::Pending(_)) {
                "pending"
            } else {
                "deferred"
            }
        )),
        Status::Pending(_) | Status::Deferred(_) => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// plan-145-A: field sites.
// ---------------------------------------------------------------------------

/// The field sites of plan-144's audit (its findings' site legend). `S*` update a
/// field of a record, `T*` a field of a `RES … STATE` payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldSite {
    /// A local record's not-last field: `r = WITH r { a := f(r.a, …) }` on
    /// `Rec { a, b }`.
    S3,
    /// The same record's last field `b`.
    S4,
    /// The last field of a module-level record `gR`, updated in a `SUB`.
    S5,
    /// `b` of `inner AS Rec` in `Out { n, inner }`:
    /// `o = WITH o { inner := WITH o.inner { b := f(o.inner.b, …) } }`.
    S6,
    /// S4 inside a `FOR EACH each1 IN r.b` (the first visit runs the statements,
    /// then `EXIT FOR`, as at plain S7).
    S7,
    /// S4 in a `collections::forEach` lambda capturing `r`.
    S9,
    /// S4 plus a scalar update in the same `WITH`, on `RecN { a, b, n }`.
    S10,
    /// Field `a` of an owner handle's payload `P { a, b, n }`.
    T1,
    /// Field `b` of the same payload.
    T2,
    /// T1 through a `RES` parameter: the body runs in `SUB run1(RES h …)`.
    T3,
    /// T2 through a `RES` parameter.
    T4,
    /// Two updates over the whole payload:
    /// `h.state = WITH h.state { b := f(h.state.b, …), n := … }`.
    T5,
    /// `b` of `inner AS In` in `Q { inner, n }`:
    /// `h.state.inner = WITH h.state.inner { b := f(h.state.inner.b, …) }`.
    T6,
    /// T2 inside a `FOR EACH each1 IN h.state.b`.
    T7,
    /// T2 on a resource-union handle `RES h AS Stream STATE P`.
    T8,
}

const FIELD_SITES: &[FieldSite] = &[
    FieldSite::S3,
    FieldSite::S4,
    FieldSite::S5,
    FieldSite::S6,
    FieldSite::S7,
    FieldSite::S9,
    FieldSite::S10,
    FieldSite::T1,
    FieldSite::T2,
    FieldSite::T3,
    FieldSite::T4,
    FieldSite::T5,
    FieldSite::T6,
    FieldSite::T7,
    FieldSite::T8,
];

impl FieldSite {
    fn code(self) -> String {
        format!("{self:?}")
    }

    fn from_code(code: &str) -> Option<Self> {
        FIELD_SITES.iter().copied().find(|s| s.code() == code)
    }

    /// The field the statement updates.
    fn field(self) -> &'static str {
        use FieldSite::*;
        match self {
            S3 => "r.a",
            S4 | S7 | S9 | S10 => "r.b",
            S5 => "gR.b",
            S6 => "o.inner.b",
            T1 | T3 => "h.state.a",
            T2 | T4 | T5 | T7 | T8 => "h.state.b",
            T6 => "h.state.inner.b",
        }
    }

    /// A field of the same owner the statement must leave alone.
    fn sibling(self) -> &'static str {
        use FieldSite::*;
        match self {
            S3 => "r.b",
            S4 | S7 | S9 | S10 => "r.a",
            S5 => "gR.a",
            S6 => "o.inner.a",
            T1 | T3 => "h.state.b",
            T2 | T4 | T5 | T7 | T8 => "h.state.a",
            T6 => "h.state.inner.a",
        }
    }

    /// `x = <rhs>` rewritten to update [`field`](Self::field).
    fn statement(self, statement: &str, label: &str) -> String {
        use FieldSite::*;
        let rhs = statement
            .strip_prefix("x = ")
            .unwrap_or_else(|| panic!("{label}: statement `{statement}` is not `x = …`"));
        let v = replace_ident(rhs, "x", self.field());
        match self {
            S3 => format!("r = WITH r {{ a := {v} }}"),
            S4 | S7 | S9 => format!("r = WITH r {{ b := {v} }}"),
            S5 => format!("gR = WITH gR {{ b := {v} }}"),
            S6 => format!("o = WITH o {{ inner := WITH o.inner {{ b := {v} }} }}"),
            S10 => format!("r = WITH r {{ b := {v}, n := k }}"),
            T1 | T3 => format!("h.state.a = {v}"),
            T2 | T4 | T7 | T8 => format!("h.state.b = {v}"),
            T5 => format!("h.state = WITH h.state {{ b := {v}, n := k }}"),
            T6 => format!("h.state.inner = WITH h.state.inner {{ b := {v} }}"),
        }
    }

    /// The statement computing the same value and handing it to a no-op `SUB` (the
    /// `arm+value` control: the value's own allocations, and nothing stored — an
    /// assignment would add the copy of a value borrowed from the field).
    fn control(self, statement: &str, label: &str) -> String {
        let rhs = statement
            .strip_prefix("x = ")
            .unwrap_or_else(|| panic!("{label}: statement `{statement}` is not `x = …`"));
        format!("kindSink({})", replace_ident(rhs, "x", self.field()))
    }

    /// The statements that build the owner from `x` (declared just before).
    fn owner_init(self) -> &'static str {
        use FieldSite::*;
        match self {
            S3 | S4 | S7 | S9 => "  MUT r AS Rec = Rec[a := x, b := x]\n",
            S10 => "  MUT r AS RecN = RecN[a := x, b := x, n := 0]\n",
            S6 => "  MUT o AS Out = Out[n := 1, inner := Rec[a := x, b := x]]\n",
            S5 => "  gR = Rec[a := x, b := x]\n",
            T6 => "  h.state = Q[inner := PIn[a := x, b := x], n := 0]\n",
            T1 | T2 | T3 | T4 | T5 | T7 | T8 => "  h.state = P[a := x, b := x, n := 0]\n",
        }
    }
}

/// A line's expected outcome at one field site.
#[derive(Clone, Debug, PartialEq)]
enum FieldExpect {
    /// Meets the line's bound.
    Arm,
    /// Still rebuilds, by design (the reason is the row's proof).
    Rebuild(String),
    /// Still rebuilds; the named plan owns it.
    Deferred(String),
    /// Does not compile, with this diagnostic.
    Na(String),
}

impl FieldExpect {
    fn parse(text: &str, context: &str) -> Self {
        let (kind, arg) = text.split_once(':').unwrap_or((text, ""));
        match (kind, arg) {
            ("arm", "") => FieldExpect::Arm,
            // plan-145-I: every letter has landed, so a pending copy is a hole.
            ("copy", _) => panic!(
                "{context}: `{text}` — a field self-update needs an in-place lowering, a \
                 `Rebuild` row with its proof, or a deferral to a named plan"
            ),
            ("rebuild", r) if !r.is_empty() => FieldExpect::Rebuild(r.to_string()),
            ("deferred", p) if !p.is_empty() => FieldExpect::Deferred(p.to_string()),
            ("na", d) if !d.is_empty() => FieldExpect::Na(d.to_string()),
            _ => panic!(
                "{context}: expectation `{text}` is not arm, rebuild:<reason>, \
                 deferred:<plan> or na:<diagnostic>"
            ),
        }
    }
}

/// `field_expect.tsv`: `(signature, site) -> expectation`. Every entry must name a
/// `cases.tsv` line and a field site.
fn field_expectations(cases: &[Case]) -> std::collections::HashMap<(String, String), FieldExpect> {
    let mut out = std::collections::HashMap::new();
    for line in FIELD_EXPECT
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 3, "field_expect.tsv: want 3 columns: {line}");
        assert!(
            cases.iter().any(|c| c.signature == cols[0]),
            "field_expect.tsv names no cases.tsv line: {line}"
        );
        assert!(
            FieldSite::from_code(cols[1]).is_some(),
            "field_expect.tsv names no field site: {line}"
        );
        let expect = FieldExpect::parse(cols[2], &format!("field_expect.tsv `{line}`"));
        let dup = out.insert((cols[0].to_string(), cols[1].to_string()), expect);
        assert!(dup.is_none(), "field_expect.tsv: duplicate line {line}");
    }
    out
}

/// What a field program performs: a `cases.tsv` line or a `field_kinds.tsv` kind.
#[derive(Clone, Debug)]
struct FieldCase {
    label: String,
    build: Build,
    /// The field's type.
    ty: String,
    /// `x`'s declaration: `<type>` or `<type> = <init>`.
    decl: String,
    setup: Vec<String>,
    statements: Vec<String>,
    check: String,
    /// The bound an `arm` meets: `false` = `arm`, `true` = `arm+value`.
    value_bound: bool,
    /// Whether the field has a `len` (the program prints it last).
    has_len: bool,
    n: u64,
    /// The expectation at each field site.
    expect: Vec<(FieldSite, FieldExpect)>,
}

impl FieldCase {
    fn from_case(
        case: &Case,
        expectations: &std::collections::HashMap<(String, String), FieldExpect>,
    ) -> Self {
        let expect = FIELD_SITES
            .iter()
            .map(|&site| {
                let e = expectations
                    .get(&(case.signature.clone(), site.code()))
                    .unwrap_or_else(|| {
                        panic!(
                            "field_expect.tsv has no line for `{}` at {}",
                            case.signature,
                            site.code()
                        )
                    });
                (site, e.clone())
            })
            .collect();
        FieldCase {
            label: case.signature.clone(),
            build: Build::Console,
            ty: case.ty().to_string(),
            decl: case.decl.clone(),
            setup: case.setup.clone(),
            statements: case.statements.clone(),
            check: case.check.clone(),
            value_bound: false,
            // `len` has no `AttributedString` overload (plan-146-A).
            has_len: case.ty() != "AttributedString",
            n: case.n,
            expect,
        }
    }
}

/// `field_kinds.tsv`, one [`FieldCase`] per kind.
fn field_kinds() -> Vec<FieldCase> {
    FIELD_KINDS
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            assert_eq!(cols.len(), 7, "field_kinds.tsv: want 7 columns: {line}");
            let build = match cols[1] {
                "console" => Build::Console,
                "app" => Build::App,
                b => panic!("field_kinds.tsv: build `{b}` is neither console nor app: {line}"),
            };
            let mut setup = cols[2].split(" ; ").map(str::to_string);
            let decl = setup.next().unwrap_or_default();
            let ty = decl.split(" = ").next().unwrap_or_default().to_string();
            let value_bound = match cols[5] {
                "arm" => false,
                "arm+value" => true,
                b => panic!("field_kinds.tsv: bound `{b}` is neither arm nor arm+value: {line}"),
            };
            let mut expect = Vec::new();
            for item in cols[6].split_whitespace() {
                let (site, e) = item.split_once('=').unwrap_or_else(|| {
                    panic!("field_kinds.tsv: `{item}` is not site=expect: {line}")
                });
                let site = FieldSite::from_code(site)
                    .unwrap_or_else(|| panic!("field_kinds.tsv: no field site `{site}`: {line}"));
                expect.push((
                    site,
                    FieldExpect::parse(e, &format!("field_kinds.tsv `{}`", cols[0])),
                ));
            }
            for &site in FIELD_SITES {
                assert!(
                    expect.iter().filter(|(s, _)| *s == site).count() == 1,
                    "field_kinds.tsv: `{}` needs exactly one expectation at {}",
                    cols[0],
                    site.code()
                );
            }
            FieldCase {
                label: cols[0].to_string(),
                build,
                ty,
                decl,
                setup: setup.collect(),
                statements: cols[3].split(" ; ").map(str::to_string).collect(),
                check: cols[4].to_string(),
                value_bound,
                has_len: false,
                n: DEFAULT_N,
                expect,
            }
        })
        .collect()
}

/// Helper declarations a `field_kinds.tsv` statement may call, and the user
/// record kinds; a program carries the ones its text names.
fn kind_helpers(ty: &str, text: &str) -> String {
    let mut out = String::new();
    if text.contains("KFix") {
        out.push_str("TYPE KFix\n  p AS Integer\n  q AS Float\nEND TYPE\n\n");
    }
    if text.contains("KVar") {
        out.push_str("TYPE KVar\n  s AS String\nEND TYPE\n\n");
    }
    if text.contains("kindSame(") {
        out.push_str(&format!(
            "FUNC kindSame(v AS {ty}) AS {ty}\n  RETURN v\nEND FUNC\n\n"
        ));
    }
    if text.contains("kindFresh(") {
        // A newly built value, owned by the caller: a pointer field stores it
        // without the deep copy a value borrowed from `v` would need.
        out.push_str(&format!(
            "FUNC kindFresh(v AS {ty}) AS {ty}\n  MUT fresh AS {ty}\n  RETURN fresh\nEND FUNC\n\n"
        ));
    }
    if text.contains("kindJson(") {
        // `json::Json` has no default value; this is its fresh one.
        out.push_str(
            "FUNC kindJson(v AS json::Json) AS json::Json\n  \
             RETURN json::JsonNum[1.0]\nEND FUNC\n\n",
        );
    }
    if text.contains("kindAddress(") {
        // `net::Address` is compiler-owned: no literal, but a default value.
        out.push_str(
            "FUNC kindAddress() AS net::Address\n  MUT a AS net::Address\n  RETURN a\nEND FUNC\n\n",
        );
    }
    if text.contains("kindSink(") {
        out.push_str(&format!("SUB kindSink(v AS {ty})\nEND SUB\n\n"));
    }
    if text.contains("kindRoute") {
        out.push_str(
            "FUNC kindRoute(req AS http::Request) AS http::Response\n  \
             MUT resp AS http::Response\n  RETURN resp\nEND FUNC\n\n",
        );
    }
    out
}

/// A whole field program: [`PRELUDE`], the imports and declarations `body` needs,
/// the owner types, and `body` placed where `site` runs it.
fn field_frame(fc: &FieldCase, site: FieldSite, body: &str) -> String {
    use FieldSite::*;
    let ty = &fc.ty;
    let mut decls = format!(
        "TYPE Rec\n  a AS {ty}\n  b AS {ty}\nEND TYPE\n\n\
         TYPE RecN\n  a AS {ty}\n  b AS {ty}\n  n AS Integer\nEND TYPE\n\n\
         TYPE Out\n  n AS Integer\n  inner AS Rec\nEND TYPE\n\n\
         TYPE P\n  a AS {ty}\n  b AS {ty}\n  n AS Integer\nEND TYPE\n\n\
         TYPE PIn\n  a AS {ty}\n  b AS {ty}\nEND TYPE\n\n\
         TYPE Q\n  inner AS PIn\n  n AS Integer\nEND TYPE\n\n"
    );
    if site == T8 {
        decls.push_str("UNION Stream\n  fs::File\n  tcp::Socket\nEND UNION\n\n");
    }
    let open = "fs::openFile(\"/dev/null\")";
    let program = match site {
        S5 => {
            // A module-level record: default-initialized when the field type is,
            // else from the line's own initializer.
            let init = match fc.decl.split_once(" = ") {
                Some((_, init)) => {
                    let init = sized(init, VALUE_M);
                    format!(" = Rec[a := {init}, b := {init}]")
                }
                None => String::new(),
            };
            format!(
                "MUT gR AS Rec{init}\n\nSUB run1()\n{body}END SUB\n\n\
                 FUNC main() AS Integer\n  run1()\n  RETURN 0\nEND FUNC\n"
            )
        }
        T3 | T4 => format!(
            "SUB run1(RES h AS fs::File STATE P)\n{body}END SUB\n\n\
             FUNC main() AS Integer\n  RES h AS fs::File STATE P = {open}\n  run1(h)\n  \
             RETURN 0\nEND FUNC\n"
        ),
        T1 | T2 | T5 | T7 => format!(
            "FUNC main() AS Integer\n  RES h AS fs::File STATE P = {open}\n{body}  \
             RETURN 0\nEND FUNC\n"
        ),
        T6 => format!(
            "FUNC main() AS Integer\n  RES h AS fs::File STATE Q = {open}\n{body}  \
             RETURN 0\nEND FUNC\n"
        ),
        T8 => format!(
            "FUNC main() AS Integer\n  RES h AS Stream STATE P = {open}\n{body}  \
             RETURN 0\nEND FUNC\n"
        ),
        S3 | S4 | S6 | S7 | S9 | S10 => {
            format!("FUNC main() AS Integer\n{body}  RETURN 0\nEND FUNC\n")
        }
    };
    let rest = format!(
        "{}{decls}{program}",
        kind_helpers(ty, &format!("{decls}{program}"))
    );
    // Every package the program names, beyond the prelude's.
    format!("{}\n{rest}", prelude_for(&rest))
}

/// `body` (the loop, already indented) placed at `site`: inside a `FOR EACH` over
/// the field at S7/T7, with each statement in a `forEach` lambda over `one` at S9
/// (`[0]` when `live`, else empty — the idle twin).
fn at_field_site(site: FieldSite, body: &str, statements: &[String], live: bool) -> String {
    match site {
        FieldSite::S9 => {
            let one = if live { "[0]" } else { "[]" };
            let mut out = format!("  LET one AS List OF Integer = {one}\n");
            for line in body.lines() {
                let indent = &line[..line.len() - line.trim_start().len()];
                if statements.iter().any(|s| s == line.trim_start()) {
                    out.push_str(&format!(
                        "{indent}collections::forEach(one, LAMBDA(each1 AS Integer) -> {})\n",
                        line.trim_start()
                    ));
                } else {
                    out.push_str(&format!("{line}\n"));
                }
            }
            out
        }
        FieldSite::S7 | FieldSite::T7 => {
            let mut out = format!(
                "  MUT ran AS Boolean = FALSE\n  FOR EACH each1 IN {}\n",
                site.field()
            );
            for line in body.lines() {
                out.push_str(&format!("  {line}\n"));
            }
            out.push_str("    ran = TRUE\n    EXIT FOR\n  NEXT\n");
            out.push_str("  IF NOT ran THEN\n    RETURN 3\n  END IF\n");
            out
        }
        _ => body.to_string(),
    }
}

/// `x`'s declaration, the setup, and the owner built from `x`.
fn field_prologue(fc: &FieldCase, site: FieldSite) -> String {
    let mut src = format!("  MUT x AS {}\n", sized(&fc.decl, VALUE_M));
    for line in &fc.setup {
        src.push_str(&format!("  {}\n", sized(line, VALUE_M)));
    }
    src.push_str(site.owner_init());
    // S10/T5's second update is `n := k`, a local read (plan-144's site legend).
    if matches!(site, FieldSite::S10 | FieldSite::T5) {
        src.push_str("  LET k AS Integer = 7\n");
    }
    src
}

/// The measured program: the field's statements run `n` times at `site` (or,
/// with `control`, the same values computed into `x`). Prints the check over a
/// copy of the field and over the sibling, before and after, then the field's
/// length.
fn field_program(fc: &FieldCase, site: FieldSite, n: u64, live: bool, control: bool) -> String {
    let statements: Vec<String> = fc
        .statements
        .iter()
        .map(|s| {
            if control {
                site.control(s, &fc.label)
            } else {
                site.statement(s, &fc.label)
            }
        })
        .collect();
    let mut src = field_prologue(fc, site);
    let sibling = replace_ident(&fc.check, "before", site.sibling());
    src.push_str(&format!("  LET before AS {} = {}\n", fc.ty, site.field()));
    src.push_str(&format!("  io::print({})\n", fc.check));
    src.push_str(&format!("  io::print({sibling})\n"));
    let mut body = format!("  FOR i = 1 TO {n}\n");
    for statement in &statements {
        body.push_str(&format!("    {statement}\n"));
    }
    body.push_str("  NEXT\n");
    // The control computes values only; it needs no lambda around them.
    let wrapped: &[String] = if control { &[] } else { &statements };
    src.push_str(&at_field_site(site, &body, wrapped, live));
    src.push_str(&format!("  io::print({})\n", fc.check));
    src.push_str(&format!("  io::print({sibling})\n"));
    if fc.has_len {
        src.push_str(&format!("  io::print(toString(len({})))\n", site.field()));
    }
    field_frame(fc, site, &src)
}

/// The result check at a field site: the statements once, beside the same result
/// through chained `LET`s (the copying lowering).
fn field_result_program(fc: &FieldCase, site: FieldSite) -> String {
    let ty = &fc.ty;
    let mut src = field_prologue(fc, site);
    src.push_str(&format!("  LET e0 AS {ty} = {}\n", site.field()));
    for (k, statement) in fc.statements.iter().enumerate() {
        let rhs = statement
            .strip_prefix("x = ")
            .unwrap_or_else(|| panic!("{}: statement `{statement}` is not `x = …`", fc.label));
        let rhs = replace_ident(rhs, "x", &format!("e{k}"));
        src.push_str(&format!("  LET e{} AS {ty} = {rhs}\n", k + 1));
    }
    let statements: Vec<String> = fc
        .statements
        .iter()
        .map(|s| site.statement(s, &fc.label))
        .collect();
    let mut body = String::new();
    for statement in &statements {
        body.push_str(&format!("  {statement}\n"));
    }
    src.push_str(&at_field_site(site, &body, &statements, true));
    let last = format!("e{}", fc.statements.len());
    src.push_str(&format!(
        "  io::print({})\n",
        replace_ident(&fc.check, "before", site.field())
    ));
    src.push_str(&format!(
        "  io::print({})\n",
        replace_ident(&fc.check, "before", &last)
    ));
    field_frame(fc, site, &src)
}

/// `count(2N) - count(N)` for a program family at `site` (the idle twins
/// subtracted at S9), after checking the copy and the sibling did not change.
fn field_extra(
    fc: &FieldCase,
    site: FieldSite,
    tag: &str,
    label: &str,
    control: bool,
) -> Result<u64, String> {
    let n = fc.n;
    let mut counts = Vec::new();
    for (k, m) in [n, 2 * n].into_iter().enumerate() {
        let (lines, mut count) = run_lines(
            &format!("{tag}_{k}"),
            &field_program(fc, site, m, true, control),
            fc.build,
        )
        .map_err(|e| {
            format!(
                "{label} (N={m}{}): {e}",
                if control { ", control" } else { "" }
            )
        })?;
        if site == FieldSite::S9 && !control {
            let (_, idle) = run_lines(
                &format!("{tag}_{k}i"),
                &field_program(fc, site, m, false, control),
                fc.build,
            )
            .map_err(|e| format!("{label} (idle, N={m}): {e}"))?;
            count = count.saturating_sub(idle);
        }
        if !control && (lines.len() < 4 || lines[0] != lines[2] || lines[1] != lines[3]) {
            return Err(format!(
                "{label} (N={m}): the copy of the field or the sibling changed across the \
                 self-updates — an update wrote through an alias or into the wrong field:\n{}",
                lines.join("\n")
            ));
        }
        counts.push(count);
    }
    Ok(counts[1].saturating_sub(counts[0]))
}

/// Check one field case at one field site against its expectation.
fn check_field(
    index: usize,
    fc: &FieldCase,
    site: FieldSite,
    expect: &FieldExpect,
) -> Result<(), String> {
    let label = format!("{} at {}", fc.label, site.code());
    let tag = format!("fs{index}_{}", site.code()).to_lowercase();
    if let FieldExpect::Na(diagnostic) = expect {
        let source = field_program(fc, site, 1, true, false);
        return match build_debug_as(&format!("{tag}_na"), &source, fc.build) {
            Ok((_, project)) => {
                let _ = std::fs::remove_dir_all(&project);
                Err(format!(
                    "{label}: expected `na:{diagnostic}`, but the program compiles — the line \
                     needs a real expectation"
                ))
            }
            Err(e) if e.contains(diagnostic.as_str()) => Ok(()),
            Err(e) => Err(format!("{label}: expected `na:{diagnostic}`, got:\n{e}")),
        };
    }
    let (in_place, copied) = {
        let (lines, _) = run_lines(
            &format!("{tag}_res"),
            &field_result_program(fc, site),
            fc.build,
        )
        .map_err(|e| format!("{label} (result check): {e}"))?;
        (
            lines.first().cloned().unwrap_or_default(),
            lines.get(1).cloned().unwrap_or_default(),
        )
    };
    if in_place != copied {
        return Err(format!(
            "{label}: the self-update computed `{in_place}`, the copying path `{copied}`"
        ));
    }
    let n = fc.n;
    let extra = field_extra(fc, site, &tag, &label, false)?;
    let (meets, bound) = if fc.value_bound {
        let control = field_extra(fc, site, &format!("{tag}c"), &label, true)?;
        (
            extra as f64 <= 1.125 * control as f64,
            format!("<= 1.125 x the control's {control}"),
        )
    } else {
        (extra < n / 8, format!("< {}", n / 8))
    };
    match (expect, meets) {
        (FieldExpect::Arm, true) => Ok(()),
        (FieldExpect::Arm, false) => Err(format!(
            "{label}: marked `arm`, but {n} more runs allocated {extra} more blocks (want \
             {bound}) — the statement rebuilds the owner"
        )),
        (_, false) => Ok(()),
        (e, true) => Err(format!(
            "{label}: marked `{e:?}`, but {n} more runs allocated only {extra} more blocks \
             ({bound}) — the update is in place now; flip the line to `arm`"
        )),
    }
}

/// `MFB_SELF_UPDATE_SITES`: the site codes to run, or `None` for every site.
fn site_filter() -> Option<Vec<String>> {
    std::env::var("MFB_SELF_UPDATE_SITES")
        .ok()
        .map(|v| v.split([',', '|']).map(|s| s.trim().to_string()).collect())
}

/// `MFB_SELF_UPDATE_FILTER`: keep the labels containing one of its parts.
fn label_filter() -> Option<Vec<String>> {
    std::env::var("MFB_SELF_UPDATE_FILTER")
        .ok()
        .map(|v| v.split('|').map(str::to_string).collect())
}

/// One unit of work: a line at a plain site, or a line or kind at a field site.
enum Work {
    Plain(usize, Case, Site),
    Field(usize, FieldCase, FieldSite, FieldExpect),
}

/// Run `work` on up to 8 threads; panic listing every failure.
fn run_work(work: Vec<Work>) {
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
                let Some(item) = queue.lock().unwrap().pop() else {
                    break;
                };
                let result = match &item {
                    Work::Plain(i, case, site) => check(*i, case, *site),
                    Work::Field(i, fc, site, expect) => check_field(*i, fc, *site, expect),
                };
                if let Err(e) = result {
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

#[test]
fn every_field_kind_meets_its_expectation() {
    let mut kinds = field_kinds();
    assert!(!kinds.is_empty(), "field_kinds.tsv has no kind");
    if let Some(parts) = label_filter() {
        kinds.retain(|k| parts.iter().any(|p| k.label.contains(p.as_str())));
    }
    let sites = site_filter();
    let work: Vec<Work> = kinds
        .into_iter()
        .enumerate()
        .flat_map(|(i, k)| {
            let k2 = k.clone();
            let sites = sites.clone();
            k.expect
                .into_iter()
                .filter(move |(s, _)| sites.as_ref().is_none_or(|l| l.contains(&s.code())))
                .map(move |(s, e)| Work::Field(1000 + i, k2.clone(), s, e))
        })
        .collect();
    run_work(work);
}

#[test]
fn every_self_update_case_meets_its_allocation_bound() {
    let mut cases = cases();
    assert!(!cases.is_empty(), "cases.tsv has no case");
    // Read every field expectation first: a missing or stale line fails even when
    // the filters would not run it.
    let expectations = field_expectations(&cases);
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
    let sites = site_filter();
    let wanted = |code: &str| sites.as_ref().is_none_or(|l| l.iter().any(|s| s == code));
    let mut work = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        for site in ENABLED_SITES.iter().filter(|s| s.applies(c)) {
            if wanted(&format!("{site:?}")) {
                work.push(Work::Plain(i, c.clone(), *site));
            }
        }
        let fc = FieldCase::from_case(c, &expectations);
        for (site, expect) in &fc.expect {
            if wanted(&site.code()) {
                work.push(Work::Field(i, fc.clone(), *site, expect.clone()));
            }
        }
    }
    run_work(work);
}
