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
//! There is one further status, for a row another plan owns:
//!
//! * `deferred:<tag>` — the statement still copies (`count(2N) - count(N) >= N`),
//!   and is meant to; the named plan (`attributed-string`, plan-146-A Open
//!   Decision 1) flips it. plan-146-A's temporary `pending:<letter>` is gone
//!   again, deleted by plan-146-H: every other line is an arm or a proven
//!   exemption.
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
//! program computing the same value and handing it to a no-op `FUNC kindSink … AS Nothing`
//! instead of storing it (an assignment would add the copy of a borrowed value).
//! At a field site the program also prints a sibling field before and after the
//! loop, so a write that lands on the wrong part of the owner fails.
//!
//! **Through a `LET` (plan-149-C2).** A sample of five lines also runs at every
//! field site with the operand read into a local first —
//! `LET cur0 AS T = r.b` then `r = WITH r { b := f(cur0, …) }` — the shape real
//! code (and the benchmark) writes. The Level-1 "Place copy forwarding" row turns
//! it back into the direct form, so each cell expects exactly what the direct form
//! expects in `field_expect.tsv`, except at `S9`: a `LAMBDA` body is one
//! expression, so the `LET` sits outside it and the lambda captures `cur0`, which
//! the row must decline. Run it with
//! `cargo test --release --test rt_inplace_self_update via_let -- --ignored`.
//!
//! `MFB_SELF_UPDATE_SITES=<code>[,<code>…]` runs only those sites (`Local`,
//! `ForEach`, `Lambda`, `Global`, `Return`, `S3`…`S10`, `T1`…`T8`);
//! `MFB_SELF_UPDATE_FILTER` narrows the lines, by signature or kind.
//!
//! **Not in the default run.** The three grid tests build thousands of programs, so
//! they are `#[ignore]`d and a plain `cargo test` skips them; the `test` job in
//! `.github/workflows/coverage.yml` runs them on every Unix row. Run them locally
//! with `cargo test --release --test rt_inplace_self_update -- --ignored`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

#[path = "rt_inplace_self_update/cases.rs"]
mod cases;

use cases::*;
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

/// plan-147-B/E: [`SCRATCH_ARMS`] here must name exactly the compiler's
/// `SCRATCH_ARMS` (`src/codegen/collection/assign/self_update.rs`). The harness is an
/// integration test and cannot see a `pub(crate)` const, so the list is duplicated —
/// this reads the compiler's source and fails if the two ever drift, which would
/// silently give some line the wrong bound at S12.
#[test]
fn scratch_arms_match_the_compiler() {
    const SOURCE: &str = include_str!("../../src/codegen/collection/assign/self_update.rs");
    let start = SOURCE
        .find("pub(crate) const SCRATCH_ARMS: &[&str] = &[")
        .expect("the compiler's SCRATCH_ARMS declaration");
    let body = &SOURCE[start..];
    let body = &body[..body.find("];").expect("the end of SCRATCH_ARMS")];
    let theirs: Vec<&str> = body
        .lines()
        .skip(1)
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|line| line.split('"').next())
        .collect();
    assert!(
        !theirs.is_empty(),
        "parsed no names out of the compiler's SCRATCH_ARMS"
    );
    assert_eq!(
        theirs, SCRATCH_ARMS,
        "this harness's SCRATCH_ARMS has drifted from the compiler's; a line given \
         the wrong bound at S12 would pass or fail for the wrong reason"
    );
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
    /// The last field of a module-level record `gR`, updated in an `AS Nothing` function.
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
    /// T1 through a resource parameter: the body runs in `FUNC run1(RES h …) AS Nothing`.
    T3,
    /// T2 through a resource parameter.
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
        self.statement_with(statement, label, self.field())
    }

    /// [`statement`](Self::statement) with `operand` read in place of `x` on the
    /// right-hand side: the field itself, or a local holding a copy of it.
    fn statement_with(self, statement: &str, label: &str, operand: &str) -> String {
        use FieldSite::*;
        let rhs = statement
            .strip_prefix("x = ")
            .unwrap_or_else(|| panic!("{label}: statement `{statement}` is not `x = …`"));
        let v = replace_ident(rhs, "x", operand);
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

    /// The statement computing the same value and handing it to a no-op `AS Nothing` function (the
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
    /// Read the field into `LET cur<k>` before each statement and update through
    /// it (plan-149-C2), instead of naming the field in the statement.
    via_let: bool,
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
            via_let: false,
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
                via_let: false,
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
        out.push_str(&format!(
            "FUNC kindSink(v AS {ty}) AS Nothing\nEND FUNC\n\n"
        ));
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
                "MUT gR AS Rec{init}\n\nFUNC run1() AS Nothing\n{body}END FUNC\n\n\
                 FUNC main() AS Integer\n  run1()\n  RETURN 0\nEND FUNC\n"
            )
        }
        T3 | T4 => format!(
            "FUNC run1(h AS fs::File STATE P) AS Nothing\n{body}END FUNC\n\n\
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

/// The lines a field program runs for `fc`'s statements at `site`, and the
/// subset that are the updates themselves (what `S9` wraps in its lambda). Via a
/// `LET`, each update is preceded by `LET cur<k> AS <T> = <field>`, which stays
/// outside the lambda.
fn field_statement_lines(fc: &FieldCase, site: FieldSite) -> (Vec<String>, Vec<String>) {
    let mut lines = Vec::new();
    let mut updates = Vec::new();
    for (k, statement) in fc.statements.iter().enumerate() {
        let update = if fc.via_let {
            let local = format!("cur{k}");
            lines.push(format!("LET {local} AS {} = {}", fc.ty, site.field()));
            site.statement_with(statement, &fc.label, &local)
        } else {
            site.statement(statement, &fc.label)
        };
        lines.push(update.clone());
        updates.push(update);
    }
    (lines, updates)
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
    let (statements, updates) = if control {
        let controls: Vec<String> = fc
            .statements
            .iter()
            .map(|s| site.control(s, &fc.label))
            .collect();
        (controls, Vec::new())
    } else {
        field_statement_lines(fc, site)
    };
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
    src.push_str(&at_field_site(site, &body, &updates, live));
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
    let (statements, updates) = field_statement_lines(fc, site);
    let mut body = String::new();
    for statement in &statements {
        body.push_str(&format!("  {statement}\n"));
    }
    src.push_str(&at_field_site(site, &body, &updates, true));
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
#[ignore = "slow: run by coverage.yml; locally `-- --ignored`"]
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
#[ignore = "slow: run by coverage.yml; locally `-- --ignored`"]
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

/// The plan-149-C2 sample: one line per operation class the benchmark's `LET cur`
/// rows use, plus the growing `append` (which rebuilds at a not-last site).
const VIA_LET_SAMPLE: &[&str] = &[
    "collections::set(value AS List OF T, index AS Integer, item AS T) AS List OF T",
    "collections::removeAt(value AS List OF T, index AS Integer) AS List OF T",
    "collections::removeKey(value AS Map OF K TO V, key AS K) AS Map OF K TO V",
    "collections::remove(value AS Set OF T, item AS T) AS Set OF T",
    "collections::append(value AS List OF T, item AS T) AS List OF T",
];

#[test]
#[ignore = "slow: run by coverage.yml; locally `-- --ignored`"]
fn every_via_let_field_cell_meets_its_expectation() {
    let cases = cases();
    let expectations = field_expectations(&cases);
    let sites = site_filter();
    let mut work = Vec::new();
    for (i, signature) in VIA_LET_SAMPLE.iter().enumerate() {
        let case = cases
            .iter()
            .find(|c| c.signature == *signature)
            .unwrap_or_else(|| panic!("VIA_LET_SAMPLE names no cases.tsv line: {signature}"));
        let mut fc = FieldCase::from_case(case, &expectations);
        fc.via_let = true;
        fc.label = format!("{} via_let", fc.label);
        for (site, direct) in &fc.expect {
            if sites.as_ref().is_some_and(|l| !l.contains(&site.code())) {
                continue;
            }
            // After the forward the NIR is the direct form, so the cell expects
            // what the direct form does — except in the `S9` lambda, which must
            // capture `cur<k>`, and a captured binding is never forwarded.
            let expect = if *site == FieldSite::S9 {
                FieldExpect::Rebuild("via-let-captured-by-the-lambda".to_string())
            } else {
                direct.clone()
            };
            work.push(Work::Field(2000 + i, fc.clone(), *site, expect));
        }
    }
    if sites.is_none() {
        assert_eq!(
            work.len(),
            VIA_LET_SAMPLE.len() * FIELD_SITES.len(),
            "every sampled line runs at every field site"
        );
    }
    run_work(work);
}
