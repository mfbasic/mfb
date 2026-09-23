//! plan-147-A Phase 2: the RED allocation tests for the owned-argument work.
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
//! * `helper-append`, `helper-map-set`, `helper-concat` — open, plan-147-D.
//! * `recursive-fill` — open, plan-147-E.

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
    assert!(
        slope < bound,
        "{name} (plan-147-{}) still allocates per call: alloc_calls went {count_n} (N={n}) \
         -> {count_2n} (2N={}), a slope of {slope}, which is not below the N/8 bound of \
         {bound}. A copying lowering allocates at least one block per call; an in-place \
         one allocates only the arm's geometric growth.",
        case.letter,
        2 * n
    );
}

/// plan-147-B landed site S11, so this one is no longer ignored: `RETURN
/// collections::append(x, k)` on an owned local updates `x`'s block in place and
/// moves it out. Measured over the `chain` shape, alloc_calls went 1203 -> 12 at
/// N = 600, and the slope 1200 -> 1.
#[test]
fn local_return() {
    check("local-return");
}

#[test]
#[ignore = "plan-147-D"]
fn helper_append() {
    check("helper-append");
}

#[test]
#[ignore = "plan-147-D"]
fn helper_map_set() {
    check("helper-map-set");
}

#[test]
#[ignore = "plan-147-D"]
fn helper_concat() {
    check("helper-concat");
}

#[test]
#[ignore = "plan-147-E"]
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
    let want: BTreeSet<(&str, &str)> = [
        // ("local_return", "B") — landed in plan-147-B (site S11).
        ("helper_append", "D"),
        ("helper_concat", "D"),
        ("helper_map_set", "D"),
        ("recursive_fill", "E"),
    ]
    .into_iter()
    .collect();

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
