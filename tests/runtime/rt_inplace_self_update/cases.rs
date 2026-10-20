//! plan-176-B: `rt_inplace_self_update`'s self-update case grid — the `cases.tsv` lines,
//! their program builders per site, and the build/run/check harness. The field-site
//! machinery and the grid tests live in the root, `rt_inplace_self_update.rs`.

use super::*;

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
pub(super) enum Status {
    Arm,
    Exempt,
    /// plan-146-A: still copies; the named plan owns it (plan-146-H deleted the
    /// `pending:` twin — a line with no arm is a missing arm).
    Deferred(String),
}

#[derive(Clone, Debug)]
pub(super) struct Case {
    pub(super) signature: String,
    pub(super) status: Status,
    /// `<type> = <init>` for `x`.
    pub(super) decl: String,
    /// Further setup statements, run once before `before` is taken.
    pub(super) setup: Vec<String>,
    pub(super) statements: Vec<String>,
    pub(super) check: String,
    pub(super) n: u64,
}

impl Case {
    pub(super) fn ty(&self) -> &str {
        self.decl
            .split_once(" = ")
            .map(|(ty, _)| ty)
            .unwrap_or_else(|| panic!("{}: setup has no `<type> = <init>`", self.signature))
    }
}

/// A `String` expression for the length of `value`, of type `ty`: `len` for a
/// collection or a `String`; `len` has no `AttributedString` overload, so its byte
/// length (plan-146-A).
pub(super) fn len_of(ty: &str, value: &str) -> String {
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
pub(super) fn prelude_for(rest: &str) -> String {
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

pub(super) fn cases() -> Vec<Case> {
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
                Some(("deferred", tag)) if !tag.is_empty() => Status::Deferred(tag.to_string()),
                _ => panic!(
                    "cases.tsv: status `{}` is not `arm`, `exempt` or `deferred:<tag>` in: \
                     {line} — a self-update needs an in-place arm or a proven exemption",
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
pub(super) enum Site {
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
    /// `FUNC run1() AS Nothing`, which `main` calls.
    Global,
    /// S11 — `RETURN OP(x, …)` on an owned local at its last use (plan-147-B). The
    /// loop is replaced by RECURSION: `chain(k)` binds `chain(k - 1)`'s result to a
    /// local `x` and returns the statement's value, so one block threads through
    /// all `N` levels and a copying `RETURN` allocates once per level. `N` is the
    /// depth.
    Return,
    /// S12 — `RETURN OP(x, …)` on an owned PARAMETER (plan-147-E): the ordinary
    /// `N`-iteration loop, but each iteration goes `x = step(x)`, so the caller hands
    /// `x` over and the statement lowers inside `step`'s owned variant. Back to one
    /// frame, so none of S11's per-frame costs apply and the plain N/8 bound holds.
    OwnedParam,
}

pub(super) const ENABLED_SITES: &[Site] = &[
    Site::Local,
    Site::ForEach,
    Site::Lambda,
    Site::Global,
    Site::Return,
    Site::OwnedParam,
];

impl Site {
    /// Whether `case` has a form at this site: no `FOR EACH` walks a `String`
    /// (`TYPE_FOR_EACH_REQUIRES_COLLECTION`). A global's shadow lives in a hidden
    /// global (plan-142-H), and a by-ref capture shares its owner's through the
    /// closure environment (plan-146-G), so a `String` runs everywhere but S7. An
    /// `AttributedString` (plan-146-A, deferred) runs at S1 and S2 only.
    pub(super) fn applies(self, case: &Case) -> bool {
        // plan-147-B (S11): a `RETURN` runs only `arm` lines. An `exempt` line's
        // bound is on the LIVE BYTES of `x` while `x` is still live (plan-142-E) —
        // at a `RETURN` the local is gone by definition, so that comparison has
        // nothing to compare. A `deferred:` line is owned by another plan and its
        // "still copies" assertion is about ITS site, not this one.
        if matches!(self, Site::Return | Site::OwnedParam) && !matches!(case.status, Status::Arm) {
            return false;
        }
        // plan-147-B: no `String` line runs at S11. A `String` block that leaves its
        // frame must be TIGHT — `arena_free(ptr, size)` is caller-sized and bins by
        // size class, and a caller frees a returned `String` by `byteLength` alone,
        // so a block carrying capacity spare cannot be moved out (bug-560,
        // `plan_returned_move`'s decline). Every in-place `String` update leaves
        // spare by construction, so a `RETURN` must copy tight — which costs exactly
        // the allocation the copying builtin would have made. MEASURED: the `chain`
        // shape over `strings::left(x, 150)` allocates 603 blocks at N=600 and 1203
        // at 2N=1200 whether the arm fires or declines. The compiler-side pin is
        // `RETURN_NEVER` in `self_update/tests/census/mod.rs`, which asserts these arms do not fire.
        // S12 is S11 inside an owned variant, so it inherits S11's `String` exclusion.
        if matches!(self, Site::Return | Site::OwnedParam) && case.ty() == "String" {
            return false;
        }
        match case.ty() {
            "String" => !matches!(self, Site::ForEach),
            "AttributedString" => matches!(self, Site::Local | Site::Global),
            _ => true,
        }
    }
}

/// The whole program: `x` declared as `decl` — a `main` local, or at S2 a
/// module-level global with `body` in an `AS Nothing` function — and `body` (main-body lines).
pub(super) fn frame(site: Site, decl: &str, body: &str) -> String {
    let rest = match site {
        Site::Global => format!(
            "\nMUT x AS {decl}\n\nFUNC run1() AS Nothing\n{body}END FUNC\n\n\
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
pub(super) fn at_site(site: Site, body: &str, live: bool) -> String {
    match site {
        // plan-147-B: S11 has no loop to place a body in — the recursion IS the
        // repetition, so `return_program` builds the whole program and `program`
        // and `result_program` both dispatch to it before reaching here.
        Site::Return => unreachable!("S11 builds its program in `return_program`"),
        Site::OwnedParam => unreachable!("S12 builds its program in `owned_param_program`"),
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
pub(super) fn sized(text: &str, m: u64) -> String {
    text.replace("{M}", &m.to_string())
}

/// plan-147-B (S11): the `chain` shape of plan-147-B §3 step 4.
///
/// ```text
/// FUNC chain(k AS Integer) AS T
///   IF k = 0 THEN RETURN seed()
///   MUT x AS T = chain(k - 1)
///   <every statement but the last, as the ordinary S1 self-update>
///   RETURN <the last statement's right-hand side>
/// END FUNC
/// ```
///
/// The recursion replaces the loop: ONE block threads through all `n` levels, so a
/// copying `RETURN` allocates once per level and fails the N/8 bound, while the arm
/// allocates only its geometric growth. `seed()` is the line's setup, hoisted into
/// its own function because `chain`'s base case needs a value, not a binding.
///
/// `result` picks the two printed lines:
///
/// * `false` (the bound program) prints the check twice, over a `before` that is an
///   independent `seed()`. At a `RETURN` the local is gone by the time the caller
///   sees anything, so there is no caller-side `x` to have been written through —
///   the `before` comparison degenerates here, exactly as plan-147-B §3 step 6 says,
///   and the real result proof is the `result` program below.
/// * `true` (the result program) runs `chain(1)` — one level, so the statements run
///   exactly once — and prints it beside the same statements computed through
///   chained `LET`s, which are not self-updates and so take the copying lowering.
///   The two renderings must agree: the bound says the arm ran, this says it
///   computed the right value.
pub(super) fn return_program(case: &Case, n: u64, result: bool, live: bool) -> String {
    let ty = case.ty();
    let decl = sized(&case.decl, VALUE_M);
    let (head, last) = case
        .statements
        .split_at(case.statements.len().saturating_sub(1));
    let last_rhs = last
        .first()
        .and_then(|statement| statement.strip_prefix("x = "))
        .unwrap_or_else(|| panic!("{}: the last statement is not `x = …`", case.signature));

    // The setup's auxiliary `LET`s (`LET ys AS List OF Integer = [4, 5]`) are the
    // statement's OTHER operands, so they must be in scope where the statement runs
    // — inside `chain`. They are NOT emitted there: a fresh `[4, 5]` per level
    // allocates one block per level, which by itself puts the slope at `N` and makes
    // the bound unreachable however well the arm works (measured: 2015 -> 4017 at
    // N=2000/4000). Instead each becomes a PARAMETER of `chain`, built once in
    // `main` and lent down the recursion (measured for the same line: 16 -> 18, a
    // slope of 2 against a bound of 250). None of them reads `x`, which the
    // assertions below pin, so hoisting them out of the recursion is sound.
    let aux: Vec<String> = case
        .setup
        .iter()
        .map(|line| sized(line, VALUE_M).trim().to_string())
        .collect();
    // `(name, type)` for each, taken from `LET <name> AS <type> = <init>`.
    let aux_params: Vec<(String, String)> = aux
        .iter()
        .map(|line| {
            let rest = line.strip_prefix("LET ").unwrap_or_else(|| {
                panic!(
                    "{}: S11 expects every auxiliary setup line to be a `LET`, got `{line}`",
                    case.signature
                )
            });
            let (name, rest) = rest.split_once(" AS ").unwrap_or_else(|| {
                panic!(
                    "{}: auxiliary setup line has no `AS`: `{line}`",
                    case.signature
                )
            });
            let (ty, _) = rest.split_once(" = ").unwrap_or_else(|| {
                panic!(
                    "{}: auxiliary setup line has no `=`: `{line}`",
                    case.signature
                )
            });
            assert!(
                replace_ident(line, "x", "\u{1}") == *line,
                "{}: S11 expects no auxiliary setup line to read `x`, got `{line}`",
                case.signature
            );
            (name.trim().to_string(), ty.trim().to_string())
        })
        .collect();
    // `, ys AS List OF Integer` for the signature, `, ys` for each call.
    let param_decls: String = aux_params
        .iter()
        .map(|(name, ty)| format!(", {name} AS {ty}"))
        .collect();
    let param_args: String = aux_params
        .iter()
        .map(|(name, _)| format!(", {name}"))
        .collect();
    // `seed` takes the same operands (its head statements may read them), but leads
    // with them rather than with `k`.
    let seed_decls: String = param_decls.trim_start_matches(", ").to_string();
    let seed_args: String = param_args.trim_start_matches(", ").to_string();

    let mut src = String::new();

    // `seed()` — `x` built by the line's declaration. Every statement runs once per
    // level, as at every other site: a line's statements are often a balanced pair
    // (`x = append(x, 9)` then `x = removeAt(x, 0)`) that holds `len(x)` steady, and
    // running only the last one per level drains the list and traps out of bounds.
    //
    // A head statement is an ordinary `x = OP(x, …)` in the recursive frame, and one
    // can carry a per-level cost of its own that has nothing to do with S11
    // (measured: `x = collections::append(x, 9)` is flat at 4 allocations for any N,
    // but `x = collections::replace(x, 1, 5)` costs one block per level). That is
    // what the idle twin below subtracts.
    src.push_str(&format!(
        "FUNC seed({seed_decls}) AS {ty}\n  MUT x AS {decl}\n  RETURN x\nEND FUNC\n\n"
    ));

    // The live program dispatches the line's last statement at the `RETURN` (S11);
    // its twin dispatches the SAME statement as an ordinary assignment (S1) and then
    // returns `x`. Everything else — the recursion, its depth, the head statements,
    // the values flowing through — is identical, so the twin carries every per-frame
    // cost the probe's shape imposes rather than the statement:
    //
    //   * a scratch arm allocates its scratch once per FRAME, and S11's repetition
    //     is recursion, so that is one block per level (measured on
    //     `collections::difference` and the 15 `math` array rows);
    //   * some head statements cost a block per level of their own (measured:
    //     `x = collections::append(x, 9)` is flat, `x = collections::replace(x, 1, 5)`
    //     is one per level).
    //
    // Subtracting the twin leaves exactly what this site is about: whether
    // dispatching a self-update at a `RETURN` costs MORE than dispatching it as an
    // assignment. It must not.
    //
    // Running the statement in both — rather than guarding it off in the twin — is
    // what keeps a balanced pair balanced. A line is often `x = append(x, 9)` then
    // `x = removeAt(x, 0)`, or `x = log10(x)` then `x = clamp(x, 1.5, 2.5)`; a twin
    // that skipped the second drained the list (`7-705-0001`) or walked out of
    // `log`'s domain (`7-705-0012`) within a few levels.
    src.push_str(&format!(
        "FUNC chain(k AS Integer{param_decls}) AS {ty}\n  \
         IF k = 0 THEN RETURN seed({seed_args})\n  \
         MUT x AS {ty} = chain(k - 1{param_args})\n"
    ));
    for statement in head {
        src.push_str(&format!("  {}\n", statement.trim()));
    }
    if live {
        src.push_str(&format!("  RETURN {last_rhs}\nEND FUNC\n\n"));
    } else {
        src.push_str(&format!("  x = {last_rhs}\n  RETURN x\nEND FUNC\n\n"));
    }

    src.push_str("FUNC main() AS Integer\n");
    for line in &aux {
        src.push_str(&format!("  {line}\n"));
    }
    if result {
        // The copying twin: the same statements as chained `LET`s, which are not
        // self-updates and so take the copying lowering.
        src.push_str(&format!("  LET e0 AS {ty} = seed({seed_args})\n"));
        for (k, statement) in case.statements.iter().enumerate() {
            let rhs = statement.strip_prefix("x = ").unwrap_or_else(|| {
                panic!("{}: statement `{statement}` is not `x = …`", case.signature)
            });
            let rhs = replace_ident(rhs, "x", &format!("e{k}"));
            src.push_str(&format!("  LET e{} AS {ty} = {rhs}\n", k + 1));
        }
        let last_e = format!("e{}", case.statements.len());
        src.push_str(&format!("  LET x AS {ty} = chain(1{param_args})\n"));
        src.push_str(&format!(
            "  io::print({})\n",
            replace_ident(&case.check, "before", "x")
        ));
        src.push_str(&format!(
            "  io::print({})\n",
            replace_ident(&case.check, "before", &last_e)
        ));
    } else {
        src.push_str(&format!("  LET before AS {ty} = seed({seed_args})\n"));
        src.push_str(&format!("  io::print({})\n", case.check));
        src.push_str(&format!("  LET x AS {ty} = chain({n}{param_args})\n"));
        src.push_str(&format!("  io::print({})\n", case.check));
        src.push_str(&format!("  io::print({})\n", len_of(ty, "x")));
    }
    src.push_str("  RETURN 0\nEND FUNC\n");

    format!("{}\n{src}", prelude_for(&src))
}

/// plan-147-B/E: the arms that keep their working state in the function's
/// self-update scratch — the compiler's `SCRATCH_ARMS`, plus every `math::` array arm
/// (which redirects the member's own result list into the scratch). Kept in step with
/// the compiler by [`scratch_arms_match_the_compiler`].
///
/// This matters at **S12**. Every other site runs its `N` statements inside one frame,
/// so the scratch is allocated once; at S12 the statement runs inside the callee's
/// owned variant, a FRESH FRAME per call, so each call allocates its own scratch —
/// one block per call that the site's shape imposes, not the statement. Measured on
/// `collections::difference(x, ys)` at N = 2000 / 2N = 4000:
///
/// | | N | 2N | Slope |
/// |---|---|---|---|
/// | handed over, arm fires | 2006 | 4006 | 2000 — **one** block per call (the scratch) |
/// | lent, copying (`MUT y = difference(x, ys)` / `RETURN y`) | 4006 | 8006 | 4000 — **two** per call |
///
/// So the bound at S12 for these lines is one block per call plus the usual slack,
/// which still separates the arm (1/call) from the copy (2/call).
pub(super) const SCRATCH_ARMS: &[&str] = &[
    "filter",
    "distinct",
    "transform",
    "sort",
    "sortBy",
    "union",
    "intersection",
    "difference",
    "symmetricDifference",
    "mapValues",
    "merge",
];

/// The qualified builtin a statement self-updates through:
/// `x = collections::sort(x)` -> `collections::sort`. `None` for an operator
/// self-update (`x = x & t`).
fn statement_target(statement: &str) -> Option<&str> {
    let rhs = statement.split_once("= ")?.1;
    Some(rhs.split_once('(')?.0.trim())
}

/// Whether any of `case`'s statements dispatches an arm that needs the scratch —
/// the compiler's `target_needs_self_update_scratch`.
pub(super) fn case_needs_scratch(case: &Case) -> bool {
    case.statements
        .iter()
        .filter_map(|statement| statement_target(statement))
        .any(|target| {
            target.starts_with("math::")
                || target
                    .rsplit("::")
                    .next()
                    .is_some_and(|bare| SCRATCH_ARMS.contains(&bare))
        })
}

/// plan-147-E (S12): `x = handOver(x, …)` run `n` times in ONE frame.
///
/// The caller hands `x` over at its last use, so inside `handOver`'s owned variant
/// the parameter is an owned local and letter B's S11 updates it in place. Unlike
/// S11's own probe this is an ordinary loop, not recursion, so none of S11's
/// per-frame costs apply and the plain N/8 bound holds with no idle twin.
///
/// The helper is not called `step`: `STEP` is a keyword (`FOR i = 1 TO 10 STEP 2`)
/// and keywords are case-insensitive, so `FUNC step(…)` does not parse.
///
/// A line's head statements stay in the loop as ordinary S1 assignments — that keeps
/// a balanced pair balanced — and only the LAST statement is dispatched through the
/// hand-over. `result` swaps the bound program for the chained-`LET` comparison.
fn owned_param_program(case: &Case, n: u64, result: bool) -> String {
    let ty = case.ty();
    let decl = sized(&case.decl, VALUE_M);
    let (head, last) = case
        .statements
        .split_at(case.statements.len().saturating_sub(1));
    let last_rhs = last
        .first()
        .and_then(|statement| statement.strip_prefix("x = "))
        .unwrap_or_else(|| panic!("{}: the last statement is not `x = …`", case.signature));

    // The setup's auxiliary `LET`s are the statement's other operands, so the helper
    // takes them as parameters — built once in `main` and lent down, exactly as
    // `return_program` does and for the same reason.
    let aux: Vec<String> = case
        .setup
        .iter()
        .map(|line| sized(line, VALUE_M).trim().to_string())
        .collect();
    let aux_params: Vec<(String, String)> = aux
        .iter()
        .map(|line| {
            let rest = line.strip_prefix("LET ").unwrap_or_else(|| {
                panic!(
                    "{}: auxiliary setup line is not a `LET`: `{line}`",
                    case.signature
                )
            });
            let (name, rest) = rest
                .split_once(" AS ")
                .unwrap_or_else(|| panic!("{}: no `AS` in `{line}`", case.signature));
            let (ty, _) = rest
                .split_once(" = ")
                .unwrap_or_else(|| panic!("{}: no `=` in `{line}`", case.signature));
            (name.trim().to_string(), ty.trim().to_string())
        })
        .collect();
    let param_decls: String = aux_params
        .iter()
        .map(|(name, ty)| format!(", {name} AS {ty}"))
        .collect();
    let param_args: String = aux_params
        .iter()
        .map(|(name, _)| format!(", {name}"))
        .collect();

    let mut src = String::new();
    src.push_str(&format!(
        "FUNC handOver(x AS {ty}{param_decls}) AS {ty}\n  RETURN {last_rhs}\nEND FUNC\n\n"
    ));
    src.push_str("FUNC main() AS Integer\n");
    src.push_str(&format!("  MUT x AS {decl}\n"));
    for line in &aux {
        src.push_str(&format!("  {line}\n"));
    }
    if result {
        // The copying twin: every statement as a chained `LET`.
        src.push_str(&format!("  LET e0 AS {ty} = x\n"));
        for (k, statement) in case.statements.iter().enumerate() {
            let rhs = statement.strip_prefix("x = ").unwrap_or_else(|| {
                panic!("{}: statement `{statement}` is not `x = …`", case.signature)
            });
            let rhs = replace_ident(rhs, "x", &format!("e{k}"));
            src.push_str(&format!("  LET e{} AS {ty} = {rhs}\n", k + 1));
        }
        let last_e = format!("e{}", case.statements.len());
        for statement in head {
            src.push_str(&format!("  {}\n", statement.trim()));
        }
        src.push_str(&format!("  x = handOver(x{param_args})\n"));
        src.push_str(&format!(
            "  io::print({})\n",
            replace_ident(&case.check, "before", "x")
        ));
        src.push_str(&format!(
            "  io::print({})\n",
            replace_ident(&case.check, "before", &last_e)
        ));
    } else {
        src.push_str(&format!("  LET before AS {ty} = x\n"));
        src.push_str(&format!("  io::print({})\n", case.check));
        src.push_str(&format!("  FOR i = 1 TO {n}\n"));
        for statement in head {
            src.push_str(&format!("    {}\n", statement.trim()));
        }
        src.push_str(&format!("    x = handOver(x{param_args})\n"));
        src.push_str("  NEXT\n");
        src.push_str(&format!("  io::print({})\n", case.check));
        src.push_str(&format!("  io::print({})\n", len_of(ty, "x")));
    }
    src.push_str("  RETURN 0\nEND FUNC\n");

    format!("{}\n{src}", prelude_for(&src))
}

pub(super) fn program(case: &Case, site: Site, n: u64, live: bool) -> String {
    if matches!(site, Site::Return) {
        return return_program(case, n, false, live);
    }
    if matches!(site, Site::OwnedParam) {
        return owned_param_program(case, n, false);
    }
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
pub(super) fn replace_ident(text: &str, from: &str, to: &str) -> String {
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
pub(super) fn result_program(case: &Case, site: Site) -> String {
    if matches!(site, Site::Return) {
        return return_program(case, 1, true, true);
    }
    if matches!(site, Site::OwnedParam) {
        return owned_param_program(case, 1, true);
    }
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
        // plan-147-B: `Site::applies` refuses every non-`arm` line at S11, and only
        // an `exempt` line reaches this program.
        Site::Return => unreachable!("S11 runs `arm` lines only"),
        Site::OwnedParam => unreachable!("S12 runs `arm` lines only"),
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
pub(super) fn run_peak(name: &str, source: &str) -> Result<(u64, u64), String> {
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
pub(super) fn run(name: &str, source: &str) -> Result<(String, String, u64), String> {
    let (lines, count) = run_lines(name, source, Build::Console)?;
    let mut lines = lines.into_iter();
    let before = lines.next().unwrap_or_default();
    let after = lines.next().unwrap_or_default();
    Ok((before, after, count))
}

/// How a program is built: a console program, or an app-mode one (a `canvas::`
/// kind: the package requires app mode), run headless.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Build {
    Console,
    App,
}

/// Build `source` with `--debug` — and `-app` for [`Build::App`] — and return
/// `(the executable, its project directory)`.
pub(super) fn build_debug_as(
    name: &str,
    source: &str,
    build: Build,
) -> Result<(PathBuf, PathBuf), String> {
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
pub(super) fn run_lines(
    name: &str,
    source: &str,
    build: Build,
) -> Result<(Vec<String>, u64), String> {
    let (exe, project) = build_debug_as(name, source, build)?;
    let mut command = Command::new(&exe);
    if build == Build::App {
        command.env("MFB_HEADLESS", "1");
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
pub(super) fn check(index: usize, case: &Case, site: Site) -> Result<(), String> {
    let label = format!("{} at {site:?}", case.signature);
    let n = case.n;
    let tag = format!("su{index}_{site:?}").to_lowercase();
    let (b1, a1, mut once) = run(&format!("{tag}_n"), &program(case, site, n, true))
        .map_err(|e| format!("{label} (N={n}): {e}"))?;
    let (b2, a2, mut twice) = run(&format!("{tag}_2n"), &program(case, site, 2 * n, true))
        .map_err(|e| format!("{label} (N={}): {e}", 2 * n))?;
    if matches!(site, Site::Lambda | Site::Return) {
        // The idle twins. At S9 they are the closures alone; at S11 they are the
        // same recursion, to the same depth, running the same head statements, with
        // only the measured `RETURN` statement guarded off — so what remains after
        // the subtraction is that statement's own allocations and nothing else.
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
    // plan-147-B: at S11 `once`/`twice` have already had the idle twin subtracted
    // (see `return_program`), so the per-frame costs the recursion imposes — a
    // scratch arm's scratch, a head statement's own allocations — are gone and the
    // plain N/8 bound applies to what is left: the cost of dispatching at a `RETURN`
    // over dispatching as an assignment.
    // plan-147-E: at S12 the statement runs inside the callee's owned variant, a fresh
    // FRAME per call, so a scratch arm allocates one scratch per call — the site's
    // shape, not the statement. The copying form measures TWO per call there, so this
    // bound still separates them. Every other site, and every non-scratch arm, keeps
    // the plain N/8 bound. (S11's own per-frame costs are removed by its idle twin
    // instead; see `return_program`.)
    let scratch_per_call = matches!(site, Site::OwnedParam) && case_needs_scratch(case);
    let bound = if scratch_per_call { n + n / 8 } else { n / 8 };
    match &case.status {
        Status::Arm if extra >= bound => Err(format!(
            "{label}: marked `arm`, but {n} more runs allocated {extra} more blocks \
             ({once} at N={n}, {twice} at 2N), which is not below the bound of {bound}\
             {} — the statement copies",
            if scratch_per_call {
                " (N for the per-call scratch S12's fresh frame forces, plus N/8)"
            } else {
                ""
            }
        )),
        Status::Exempt => exempt_check(index, case, site, &label),
        Status::Arm => Ok(()),
        // plan-146-A: a copy allocates at least one block per statement run.
        Status::Deferred(plan) if extra < n => Err(format!(
            "{label}: marked `deferred:{plan}`, but {n} more runs allocated only {extra} \
             more blocks ({once} at N={n}, {twice} at 2N) — the statement no longer copies; \
             flip the line"
        )),
        Status::Deferred(_) => Ok(()),
    }
}
