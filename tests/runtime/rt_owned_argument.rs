//! plan-147-A Phase 2: the allocation tests for the owned-argument work.
//! Every case has since landed; the file is now a regression suite.
//!
//! plan-147 hands a collection or `String` the caller no longer needs over to the
//! callee instead of lending it, so the callee updates it in place. Each case here
//! is one shape that copies per call today and must stop copying when the named
//! letter lands.
//!
//! **The measure** (plan-147-A §3, plan-142 Correction A4). Run the shape to depth
//! or count `N` and `2N` under `mfb build --debug` and sum the debug report's
//! `arena.<k>.alloc_calls` — the number of blocks the program allocated. A copying
//! lowering allocates at least one block per call, so the extra `N` calls allocate
//! at least `N` more blocks. An in-place lowering allocates only when the
//! collection outgrows its capacity, which is geometric, so the extra `N` calls
//! allocate a handful. The bound is therefore
//!
//! ```text
//! count(2N) - count(N) < N / 8
//! ```
//!
//! Every case also checks the program's printed result at both sizes, so a case
//! cannot go green by computing the wrong thing.
//!
//! **Status.** A case that has not landed yet is `#[ignore]`d with the letter
//! expected to turn it green, so the suite stays green while plan-147 is in flight.
//! Run the open ones with `cargo test --test rt_owned_argument -- --ignored`.
//! Each letter un-ignores its own case and moves its name from `want` to `LANDED`
//! in [`the_ignored_set_is_exactly_the_five_open_cases`], the non-ignored guard
//! that a case is never quietly dropped or un-ignored without its letter's change.
//!
//! * `local-return` — **landed**, plan-147-B (site S11).
//! * `helper-append`, `helper-map-set` — **landed**, plan-147-D (owned variants).
//! * `helper-concat` — **landed**, plan-147-D, but asserting `Expect::StillCopies`:
//!   the hand-over happens and buys nothing, because a `String` block must be tight
//!   to leave its frame. See the case's doc comment for the measurement.
//! * `recursive-fill` — **landed**, plan-147-E (transitive hand-over).
//!
//! Nothing is ignored: the guard asserts the ignored set is empty.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

/// One shape: `(case name, the letter that turns it green, N, module text, main
/// body, the expected printed line at N and at 2N)`.
///
/// `{N}` in the body is replaced by the size. The body prints exactly one line.
struct Case {
    name: &'static str,
    letter: &'static str,
    n: u64,
    module: &'static str,
    body: &'static str,
    /// `want(n)` — the line the program must print when run at size `n`.
    want: fn(u64) -> String,
    /// What the shape's allocation slope must be.
    expect: Expect,
}

/// What a case asserts about its allocation slope.
#[derive(Clone, Copy, Debug)]
enum Expect {
    /// `count(2N) - count(N) < N/8`: the shape stopped copying.
    Flat,
    /// `count(2N) - count(N) >= N`: the shape still allocates at least one block per
    /// call, and is MEANT to — the named reason says why, and the named plan is what
    /// would change it. Asserted in both directions: if the slope ever drops below
    /// `N`, the case fails and says to flip the line, exactly as
    /// `rt_inplace_self_update`'s `deferred:` status does.
    StillCopies { why: &'static str },
}

/// The recursion-driven cases use a smaller `N`: the copying lowering recurses `N`
/// deep, and the point is the allocation slope, not the depth.
const DEEP_N: u64 = 600;
/// The loop-driven cases run the statement `N` times in one frame.
const FLAT_N: u64 = 2000;

fn cases() -> Vec<Case> {
    vec![
        // B: `RETURN collections::append(out, x)` with `out` an owned local at its
        // last use. plan-147-B §3 step 5's `chain` shape: one level per call, so a
        // copying `RETURN` allocates once per level and the slope is N.
        Case {
            name: "local-return",
            letter: "B",
            n: DEEP_N,
            module: "FUNC chain(k AS Integer) AS List OF Integer
  IF k = 0 THEN RETURN []
  MUT x AS List OF Integer = chain(k - 1)
  RETURN collections::append(x, k)
END FUNC",
            body: "LET r AS List OF Integer = chain({N})
  io::print(\"len=\" & toString(len(r)))",
            want: |n| format!("len={n}"),
            expect: Expect::Flat,
        },
        // D: the accumulator threaded through a helper. `acc` is dead after the
        // call, so it may be handed over instead of lent.
        Case {
            name: "helper-append",
            letter: "D",
            n: FLAT_N,
            module: "FUNC addOne(xs AS List OF Integer, v AS Integer) AS List OF Integer
  RETURN collections::append(xs, v)
END FUNC",
            body: "MUT acc AS List OF Integer = []
  FOR i = 1 TO {N}
    acc = addOne(acc, i)
  NEXT
  io::print(\"len=\" & toString(len(acc)))",
            want: |n| format!("len={n}"),
            expect: Expect::Flat,
        },
        // D: the same, into a Map.
        Case {
            name: "helper-map-set",
            letter: "D",
            n: FLAT_N,
            module:
                "FUNC put(m AS Map OF Integer TO Integer, k AS Integer) AS Map OF Integer TO Integer
  RETURN collections::set(m, k, k * 2)
END FUNC",
            body: "MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer { }
  FOR i = 1 TO {N}
    m = put(m, i)
  NEXT
  io::print(\"len=\" & toString(len(m)))",
            want: |n| format!("len={n}"),
            expect: Expect::Flat,
        },
        // D: the same, for a String.
        Case {
            name: "helper-concat",
            letter: "D",
            n: FLAT_N,
            module: "FUNC grow(s AS String, t AS String) AS String
  RETURN s & t
END FUNC",
            body: "MUT s AS String = \"\"
  FOR i = 1 TO {N}
    s = grow(s, \"x\")
  NEXT
  io::print(\"len=\" & toString(len(s)))",
            want: |n| format!("len={n}"),
            expect: Expect::StillCopies {
                why: "a `String` block must be TIGHT to leave its frame -- `arena_free` is \
                      caller-sized and bins by size class, and a caller frees a returned \
                      `String` by `byteLength` alone (bug-560) -- while every in-place \
                      `String` growth leaves spare. So the owned variant's `RETURN s & t` \
                      has to copy tight, which costs exactly the one allocation the copying \
                      `grow` would have made. MEASURED with the hand-over forced (the \
                      variant `grow$own1` is emitted and called): 2003 at N=2000 and 4003 at \
                      2N=4000, identical to without it. Flattening this needs the String \
                      block to carry its own capacity, which is another plan's change",
            },
        },
        // E: transitive hand-over. The argument is a fresh temp
        // (`collections::append(xs, n)`), and the callee is the function itself, so
        // the hand-over has to survive one level of recursion to pay off.
        Case {
            name: "recursive-fill",
            letter: "E",
            n: DEEP_N,
            module: "FUNC fill(xs AS List OF Integer, n AS Integer) AS List OF Integer
  IF n = 0 THEN RETURN xs
  RETURN fill(collections::append(xs, n), n - 1)
END FUNC",
            body: "LET r AS List OF Integer = fill([], {N})
  io::print(\"len=\" & toString(len(r)))",
            want: |n| format!("len={n}"),
            expect: Expect::Flat,
        },
    ]
}

fn case(name: &str) -> Case {
    cases()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no case named {name}"))
}

fn source(case: &Case, n: u64) -> String {
    let body = case.body.replace("{N}", &n.to_string());
    format!(
        "IMPORT collections\nIMPORT io\n\n{}\n\nFUNC main() AS Integer\n  {body}\n  RETURN 0\nEND FUNC\n",
        case.module
    )
}

/// Build with `--debug` and return the executable (the host glibc one on Linux).
fn build_debug(name: &str, source: &str) -> Result<(PathBuf, PathBuf), String> {
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
    let exe = written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .map(PathBuf::from)
        .ok_or_else(|| format!("no executable in build output:\n{stdout}"))?;
    Ok((exe, project))
}

/// Run the shape at size `n`: `(the printed line, the arenas' summed alloc_calls)`.
fn measure(case: &Case, n: u64) -> Result<(String, u64), String> {
    let src = source(case, n);
    let label = format!("owned_argument_{}_{n}", case.name.replace('-', "_"));
    let (exe, project) = build_debug(&label, &src)?;
    let output = Command::new(&exe).output();
    let _ = std::fs::remove_dir_all(&project);
    let output = output.map_err(|e| format!("run: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!(
            "program failed at N={n}:\n{stdout}\n{stderr}\n--- source ---\n{src}"
        ));
    }
    // Every arena's `alloc_calls`, summed. (`perf.mfb_alloc.count` counts the perf
    // log's samples and saturates when the log fills, so N and 2N can read the same
    // number — plan-142-E Correction E2.)
    let mut count = 0u64;
    let mut seen = false;
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix("arena.") else {
            continue;
        };
        let Some((_, value)) = rest.split_once(".alloc_calls ") else {
            continue;
        };
        count += value
            .trim()
            .parse::<u64>()
            .map_err(|e| format!("bad alloc_calls line `{line}`: {e}"))?;
        seen = true;
    }
    if !seen {
        return Err(format!("no arena.*.alloc_calls in:\n{stderr}"));
    }
    Ok((stdout.lines().next().unwrap_or_default().to_string(), count))
}

/// The shared body of every case test. Panics with a message naming the case.
fn check(name: &str) {
    let case = case(name);
    let n = case.n;
    let bound = n / 8;

    let (line_n, count_n) = match measure(&case, n) {
        Ok(v) => v,
        Err(e) => panic!("{name} (plan-147-{}): at N={n}: {e}", case.letter),
    };
    let (line_2n, count_2n) = match measure(&case, 2 * n) {
        Ok(v) => v,
        Err(e) => panic!("{name} (plan-147-{}): at 2N={}: {e}", case.letter, 2 * n),
    };

    let want_n = (case.want)(n);
    let want_2n = (case.want)(2 * n);
    assert_eq!(
        line_n, want_n,
        "{name} (plan-147-{}): wrong result at N={n}",
        case.letter
    );
    assert_eq!(
        line_2n,
        want_2n,
        "{name} (plan-147-{}): wrong result at 2N={}",
        case.letter,
        2 * n
    );

    let slope = count_2n.saturating_sub(count_n);
    match case.expect {
        Expect::Flat => assert!(
            slope < bound,
            "{name} (plan-147-{}) still allocates per call: alloc_calls went {count_n} (N={n}) \
             -> {count_2n} (2N={}), a slope of {slope}, which is not below the N/8 bound of \
             {bound}. A copying lowering allocates at least one block per call; an in-place \
             one allocates only the arm's geometric growth.",
            case.letter,
            2 * n
        ),
        // Asserted in BOTH directions, so this cannot rot into a silently-skipped
        // case: if the shape ever stops copying, the case fails and says so.
        Expect::StillCopies { why } => assert!(
            slope >= n,
            "{name} (plan-147-{}) NO LONGER allocates per call: alloc_calls went {count_n} \
             (N={n}) -> {count_2n} (2N={}), a slope of {slope}, below the {n} a copying \
             lowering must show. That is good news -- flip this case to `Expect::Flat` and \
             delete the reason, which said: {why}",
            case.letter,
            2 * n
        ),
    }
}

/// plan-147-B landed site S11, so this one is no longer ignored: `RETURN
/// collections::append(x, k)` on an owned local updates `x`'s block in place and
/// moves it out. Measured over the `chain` shape, alloc_calls went 1203 -> 12 at
/// N = 600, and the slope 1200 -> 1.
#[test]
fn local_return() {
    check("local-return");
}

/// plan-147-D landed the owned variants, so `acc = addOne(acc, i)` hands `acc`
/// over and `addOne$own1` appends into it in place. alloc_calls 4003 -> 15 at N=2000.
#[test]
fn helper_append() {
    check("helper-append");
}

/// plan-147-D: `m = put(m, i)` hands `m` over to `put$own1`.
#[test]
fn helper_map_set() {
    check("helper-map-set");
}

/// plan-147-D: the hand-over happens (`grow$own1` is emitted and called), but a
/// `String` block must be tight to leave its frame, so the variant's `RETURN s & t`
/// still copies once per call. Measured identical with and without the hand-over:
/// 2003 at N=2000, 4003 at 2N. The case asserts that, in both directions.
#[test]
fn helper_concat() {
    check("helper-concat");
}

/// plan-147-E: transitive hand-over. `fill(collections::append(xs, n), n - 1)`
/// needed two things beyond letter D — the fresh temporary argument handed over, and
/// the temporary BUILT by updating `xs` in place rather than copying it — and both
/// only apply inside `fill$own1`, where `xs` is owned. alloc_calls 1203 -> 12 at
/// N = 600.
#[test]
fn recursive_fill() {
    check("recursive-fill");
}

/// The guard plan-147-A Phase 2 asks for: the ignored set is exactly the five open
/// cases, and each carries the letter that owns it.
///
/// **Each later letter edits this test** when it un-ignores its case: drop the
/// `#[ignore]` on the case's `fn`, and drop that row here. A case that is dropped,
/// renamed, or un-ignored without its letter's change fails this test instead of
/// silently disappearing.
#[test]
fn the_ignored_set_is_exactly_the_five_open_cases() {
    const THIS_FILE: &str = include_str!("rt_owned_argument.rs");

    // `(test fn, the letter in its #[ignore] reason)`, in source order.
    // Every case has landed: plan-147 leaves nothing ignored.
    let want: BTreeSet<(&str, &str)> = BTreeSet::new();

    let mut found: BTreeSet<(String, String)> = BTreeSet::new();
    let lines: Vec<&str> = THIS_FILE.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let Some(rest) = line.trim().strip_prefix("#[ignore = \"plan-147-") else {
            continue;
        };
        let letter = rest
            .split('"')
            .next()
            .expect("an #[ignore] reason")
            .to_string();
        let name = lines[i + 1..]
            .iter()
            .find_map(|l| {
                l.trim()
                    .strip_prefix("fn ")
                    .and_then(|f| f.split('(').next())
            })
            .unwrap_or_else(|| panic!("no fn after the #[ignore] on line {}", i + 1))
            .to_string();
        found.insert((name, letter));
    }

    let found_refs: BTreeSet<(&str, &str)> = found
        .iter()
        .map(|(n, l)| (n.as_str(), l.as_str()))
        .collect();
    assert_eq!(
        found_refs, want,
        "the set of #[ignore]d plan-147 cases changed. A letter that turns a case \
         green must drop both its #[ignore] and its row in this test's `want`; a new \
         case must add both."
    );

    // Every case in `cases()` is either still ignored or has landed. A case that is
    // neither — deleted from `cases()` while a test still names it, or added to
    // `cases()` with no test at all — is the drift this guard exists to catch.
    const LANDED: &[&str] = &[
        // plan-147-B, site S11.
        "local_return",
        // plan-147-D, the owned variants. `helper_concat` is landed but asserts
        // `Expect::StillCopies`; see its doc comment.
        "helper_append",
        "helper_concat",
        "helper_map_set",
        // plan-147-E, transitive hand-over.
        "recursive_fill",
    ];
    let declared: BTreeSet<String> = cases().iter().map(|c| c.name.replace('-', "_")).collect();
    let ignored: BTreeSet<String> = found.iter().map(|(n, _)| n.clone()).collect();
    let accounted: BTreeSet<String> = ignored
        .iter()
        .cloned()
        .chain(LANDED.iter().map(|n| (*n).to_string()))
        .collect();
    assert_eq!(
        declared
            .difference(&accounted)
            .cloned()
            .collect::<Vec<String>>(),
        Vec::<String>::new(),
        "a case in `cases()` is neither #[ignore]d nor listed in LANDED. A letter that \
         turns a case green moves its name from `want` to `LANDED`."
    );
    assert_eq!(
        accounted
            .difference(&declared)
            .cloned()
            .collect::<Vec<String>>(),
        Vec::<String>::new(),
        "a test or LANDED entry names a case that `cases()` no longer declares"
    );
}

/// plan-147-D Phase 2: the failure route of an owned parameter.
///
/// An owned variant frees its parameter on **every** exit, and the error and trap
/// routes are where that is easiest to get wrong. A missed path leaks — unobservable
/// to the program, but still a bug. A doubled path double-frees.
///
/// The helper below takes its collection owned and then `FAIL`s. Two details make it
/// actually exercise the owned route rather than quietly falling back to lending:
///
/// * it has a **consuming `RETURN`** (`IF v < 0 THEN RETURN collections::append(xs, v)`),
///   which is what puts `xs` in `consumable_params` and so satisfies plan-147-C's H6.
///   A helper that only ever `FAIL`s consumes nothing, is never handed anything, and
///   would make this test pass while testing nothing;
/// * `v` is always >= 1, so that branch is never taken at runtime and every call
///   reaches the `FAIL` — with `xs` owned and unconsumed, which is exactly the path
///   the variant's cleanup has to free. (A parameter is immutable, so the helper
///   cannot update `xs` in place itself; the consuming `RETURN` is what makes the
///   parameter owned, and the `FAIL` is the route under test.)
///
/// The caller traps without reading `acc`, so the hand-over is not refused by H3.
/// [`the_hand_over_really_happens`] pins that the variant is emitted at all, so this
/// test cannot pass vacuously by quietly falling back to lending. Running the whole
/// thing `N` and `2N` times pins both directions:
///
/// * **no double free** — the arena's `--debug` report counts
///   `arena.<k>.double_free_skips`, which must stay 0, and the program must exit 0;
/// * **no leak** — `arena.<k>.live_bytes` must be 0 at exit, and
///   `peak_live_bytes` must not grow with `N`. A leak of one block per failing call
///   would make the peak scale.
#[test]
fn an_owned_parameter_is_freed_once_on_the_failure_route() {
    /// `(peak_live_bytes, live_bytes, double_free_skips, exit code, stdout)`.
    fn run(n: u64) -> (u64, u64, u64, Option<i32>, String) {
        let source = format!(
            "IMPORT collections\nIMPORT io\n\n\
             FUNC growThenFail(xs AS List OF Integer, v AS Integer) AS List OF Integer\n  \
               IF v < 0 THEN RETURN collections::append(xs, v)\n  \
               FAIL error(7, \"len \" & toString(len(xs)))\n\
             END FUNC\n\n\
             FUNC main() AS Integer\n  \
               MUT hits AS Integer = 0\n  \
               FOR i = 1 TO {n}\n    \
                 MUT acc AS List OF Integer = [1, 2, 3]\n    \
                 acc = growThenFail(acc, i) TRAP(e)\n      \
                   hits = hits + 1\n      \
                   RECOVER []\n    \
                 END TRAP\n  \
               NEXT\n  \
               io::print(\"hits=\" & toString(hits))\n  \
               RETURN 0\n\
             END FUNC\n"
        );
        let (exe, project) =
            build_debug(&format!("owned_argument_fail_{n}"), &source).expect("the probe builds");
        let output = Command::new(&exe).output();
        let _ = std::fs::remove_dir_all(&project);
        let output = output.expect("the probe runs");
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        // `arena.<index>.<key> <value>`, matching `<key>` EXACTLY: a substring match
        // would make `live_bytes` also collect every `peak_live_bytes` line.
        let sum = |key: &str| -> u64 {
            stderr
                .lines()
                .filter_map(|line| line.strip_prefix("arena."))
                .filter_map(|rest| rest.split_once(' '))
                .filter_map(|(name, value)| {
                    let (_index, field) = name.split_once('.')?;
                    (field == key).then(|| value.trim().parse::<u64>().ok())?
                })
                .sum()
        };
        (
            sum("peak_live_bytes"),
            sum("live_bytes"),
            sum("double_free_skips"),
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        )
    }

    const N: u64 = 200;
    let (peak_n, live_n, dfree_n, code_n, out_n) = run(N);
    let (peak_2n, live_2n, dfree_2n, code_2n, out_2n) = run(2 * N);

    assert_eq!(code_n, Some(0), "the probe exited non-zero at N={N}");
    assert_eq!(
        code_2n,
        Some(0),
        "the probe exited non-zero at 2N={}",
        2 * N
    );
    assert_eq!(out_n, format!("hits={N}"), "every call must have failed");
    assert_eq!(
        out_2n,
        format!("hits={}", 2 * N),
        "every call must have failed"
    );
    assert_eq!(
        (dfree_n, dfree_2n),
        (0, 0),
        "the arena skipped a double free: the owned parameter is freed on more than \
         one path ({dfree_n} at N={N}, {dfree_2n} at 2N)"
    );
    assert_eq!(
        (live_n, live_2n),
        (0, 0),
        "bytes were still live at exit: the owned parameter is not freed on the \
         failure route ({live_n} at N={N}, {live_2n} at 2N)"
    );
    // A leak of one block per failing call would scale the peak with N. Allowing a
    // small absolute slack keeps this from tripping on arena bookkeeping.
    assert!(
        peak_2n <= peak_n + 4096,
        "peak_live_bytes grew with N ({peak_n} at N={N} -> {peak_2n} at 2N): the owned \
         parameter leaks on the failure route"
    );
}

/// plan-147-D: the hand-over is not silently absent.
///
/// Every other case here measures ALLOCATIONS, and a lowering that stopped handing
/// anything over would fail them for a reason that reads like a regression in the
/// arms rather than in the calling convention. This one looks at the emitted code
/// instead: an approved site must call `<base>$own<mask>`, and that symbol must be a
/// function of the program.
#[test]
fn the_hand_over_really_happens() {
    let source = "IMPORT collections\nIMPORT io\n\n\
         FUNC addOne(xs AS List OF Integer, v AS Integer) AS List OF Integer\n  \
           RETURN collections::append(xs, v)\n\
         END FUNC\n\n\
         FUNC main() AS Integer\n  \
           MUT acc AS List OF Integer = []\n  \
           FOR i = 1 TO 3\n    \
             acc = addOne(acc, i)\n  \
           NEXT\n  \
           io::print(toString(len(acc)))\n  \
           RETURN 0\n\
         END FUNC\n";
    let project = common::temp_project("owned_argument_variant", source);
    let plan = common::build_ncode(&project, "macos-aarch64", "owned_argument_variant");
    let text = plan.to_string();
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        text.contains("addOne$own1"),
        "no owned variant in the emitted code: an approved hand-over site must call \
         `addOne$own1`, and that variant must be lowered. Without it every other case \
         in this file would be measuring the ordinary lending path."
    );
}

/// plan-147-E Phase 2: the error route of a handed-over TEMPORARY.
///
/// This is the letter's named correctness risk. A temporary claimed for hand-over and
/// then also freed by `emit_call_error_exit` is a double free; one neither claimed
/// nor handed over is a leak. The claim therefore has to happen before the call
/// instruction, and this case is what pins it.
///
/// The helper takes its collection owned — it has a consuming `RETURN` on a branch
/// that never runs — and then `FAIL`s, and the argument at the call site is a **fresh
/// temporary** (`collections::append(acc, i)`), not a local. The caller traps without
/// reading anything, so the hand-over is not refused.
#[test]
fn a_handed_over_temporary_is_freed_once_on_the_failure_route() {
    fn run(n: u64) -> (u64, u64, u64, Option<i32>, String) {
        let source = format!(
            "IMPORT collections\nIMPORT io\n\n\
             FUNC sink(xs AS List OF Integer, v AS Integer) AS List OF Integer\n  \
               IF v < 0 THEN RETURN collections::append(xs, v)\n  \
               FAIL error(9, \"len \" & toString(len(xs)))\n\
             END FUNC\n\n\
             FUNC main() AS Integer\n  \
               MUT hits AS Integer = 0\n  \
               FOR i = 1 TO {n}\n    \
                 MUT acc AS List OF Integer = [1, 2, 3]\n    \
                 LET out AS List OF Integer = sink(collections::append(acc, i), i) TRAP(e)\n      \
                   hits = hits + 1\n      \
                   RECOVER []\n    \
                 END TRAP\n    \
                 IF len(out) > 99 THEN\n      \
                   io::print(\"unreachable\")\n    \
                 END IF\n  \
               NEXT\n  \
               io::print(\"hits=\" & toString(hits))\n  \
               RETURN 0\n\
             END FUNC\n"
        );
        let (exe, project) = build_debug(&format!("owned_argument_temp_fail_{n}"), &source)
            .expect("the probe builds");
        let output = Command::new(&exe).output();
        let _ = std::fs::remove_dir_all(&project);
        let output = output.expect("the probe runs");
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let sum = |key: &str| -> u64 {
            stderr
                .lines()
                .filter_map(|line| line.strip_prefix("arena."))
                .filter_map(|rest| rest.split_once(' '))
                .filter_map(|(name, value)| {
                    let (_index, field) = name.split_once('.')?;
                    (field == key).then(|| value.trim().parse::<u64>().ok())?
                })
                .sum()
        };
        (
            sum("peak_live_bytes"),
            sum("live_bytes"),
            sum("double_free_skips"),
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        )
    }

    const N: u64 = 200;
    let (peak_n, live_n, dfree_n, code_n, out_n) = run(N);
    let (peak_2n, live_2n, dfree_2n, code_2n, out_2n) = run(2 * N);

    assert_eq!(code_n, Some(0), "the probe exited non-zero at N={N}");
    assert_eq!(
        code_2n,
        Some(0),
        "the probe exited non-zero at 2N={}",
        2 * N
    );
    assert_eq!(out_n, format!("hits={N}"), "every call must have failed");
    assert_eq!(
        out_2n,
        format!("hits={}", 2 * N),
        "every call must have failed"
    );
    assert_eq!(
        (dfree_n, dfree_2n),
        (0, 0),
        "the arena skipped a double free: a handed-over temporary is freed both by \
         the claim's owner and by the statement ({dfree_n} at N={N}, {dfree_2n} at 2N)"
    );
    assert_eq!(
        (live_n, live_2n),
        (0, 0),
        "bytes were still live at exit: a handed-over temporary is freed by nobody \
         ({live_n} at N={N}, {live_2n} at 2N)"
    );
    assert!(
        peak_2n <= peak_n + 4096,
        "peak_live_bytes grew with N ({peak_n} at N={N} -> {peak_2n} at 2N): a \
         handed-over temporary leaks on the failure route"
    );
}
