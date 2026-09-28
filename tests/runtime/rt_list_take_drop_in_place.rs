//! bug-652: an in-place `take`/`drop`/`mid` self-update costs the elements it
//! removes or moves, never the size of the list it leaves alone.
//!
//! plan-142-B made `x = collections::take(x, k)` / `drop` / `mid` compact the
//! list in place (`lower_list_compact_in_place`). For a variable-width element
//! that compaction still walked every entry up to three times on every call — a
//! dropped-element pass, a payload-order probe, and an in-order slide — even when
//! the kept range was the whole list. So a build loop that rewrites a shared
//! `pending` list (`pending = take(pending, len(pending))`, then `append`) was
//! O(n²): 42 s at 100,000 nodes where the same loop with only `append` took
//! 0.07 s. The flat `List OF Integer` loops were already linear (the fixed-width
//! `KeepSource::Range` fast path).
//!
//! **The measure.** Each shape runs the same total number of elements two ways:
//! `ROUNDS` builds of `N` elements, and one build of `N * ROUNDS`. Linear work
//! costs the same both ways (×1); a per-call cost proportional to the list's size
//! costs ×`ROUNDS` more on the one long build. The ×3 bound sits between. The time
//! is the child's own CPU time (`wait4`'s `ru_utime + ru_stime`), not wall time:
//! on a loaded machine these programs measured 4.61 s wall for 0.71 s CPU, which
//! would make a wall-clock ratio meaningless. `SLACK` absorbs timer and start-up
//! jitter on the shapes that finish in tens of milliseconds; a quadratic at these
//! sizes costs seconds, far outside it. Each program also prints the element count
//! and the last element, so a fast path that loses or corrupts elements fails too.
//!
//! The value and `--debug` leak tests pin what the fast path must not change:
//! `take`/`drop`/`mid` bound to another name still copy (the source is
//! untouched), a self-update over a list whose payloads are out of entry order
//! (after `prepend`, `insert`, `set`, `sort`) still keeps the right elements, and
//! every removed graph-owning element is freed exactly once.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

const ROUNDS: u64 = 8;
const MAX_RATIO: f64 = 3.0;
const SLACK_SECONDS: f64 = 0.05;

/// The bug-652 repro's element types and loop shapes
/// (`bugs/repro/bug-652-take-drop-variable-width-compaction.mfb`), plus a
/// `rounds` argument and a read-back of the last element.
const SCALE_PROGRAM: &str = r#"IMPORT io
IMPORT os
IMPORT collections

TYPE Attribute
  name AS String
  value AS String
END TYPE

TYPE Text
  value AS String
END TYPE

TYPE Comment
  value AS String
END TYPE

TYPE ProcessingInstruction
  target AS String
  data AS String
END TYPE

TYPE Element
  name AS String
  attributes AS List OF Attribute
  children AS List OF Node
END TYPE

UNION Node
  Element
  Text
  Comment
  ProcessingInstruction
END UNION

FUNC leaf(name AS String) AS Node
  RETURN Element[name := name, attributes := [], children := []]
END FUNC

FUNC wideAppend(n AS Integer) AS List OF Node
  MUT pending AS List OF Node = []
  FOR i = 1 TO n
    pending = collections::append(pending, leaf(toString(i)))
  NEXT
  RETURN pending
END FUNC

FUNC wideTake(n AS Integer) AS List OF Node
  MUT pending AS List OF Node = []
  FOR i = 1 TO n
    pending = collections::take(pending, len(pending))
    pending = collections::append(pending, leaf(toString(i)))
  NEXT
  RETURN pending
END FUNC

FUNC wideDrop(n AS Integer) AS List OF Node
  MUT pending AS List OF Node = []
  FOR i = 1 TO n
    LET kids AS List OF Node = collections::drop(pending, len(pending))
    pending = collections::append(pending, leaf(toString(i)))
  NEXT
  RETURN pending
END FUNC

FUNC wideDropAll(n AS Integer) AS List OF Node
  MUT pending AS List OF Node = []
  FOR i = 1 TO n
    pending = collections::drop(pending, 0)
    pending = collections::append(pending, leaf(toString(i)))
  NEXT
  RETURN pending
END FUNC

FUNC intTake(n AS Integer) AS List OF Integer
  MUT pending AS List OF Integer = []
  FOR i = 1 TO n
    pending = collections::take(pending, len(pending))
    pending = collections::append(pending, i)
  NEXT
  RETURN pending
END FUNC

FUNC intDropAll(n AS Integer) AS List OF Integer
  MUT pending AS List OF Integer = []
  FOR i = 1 TO n
    pending = collections::drop(pending, 0)
    pending = collections::append(pending, i)
  NEXT
  RETURN pending
END FUNC

FUNC lastName(xs AS List OF Node) AS String
  MUT name AS String = "?"
  MATCH collections::get(xs, len(xs) - 1)
    CASE Element(e)
      name = e.name
    CASE ELSE
      name = "!"
  END MATCH
  RETURN name
END FUNC

FUNC lastInt(xs AS List OF Integer) AS String
  RETURN toString(collections::get(xs, len(xs) - 1))
END FUNC

FUNC main AS Integer
  LET args = os::args()
  LET shape = collections::get(args, 0)
  LET n = toInt(collections::get(args, 1))
  LET rounds = toInt(collections::get(args, 2))
  MUT total AS Integer = 0
  MUT last AS String = ""
  FOR r = 1 TO rounds
    IF shape = "wideAppend" THEN
      LET xs = wideAppend(n)
      total = total + len(xs)
      last = lastName(xs)
    ELSEIF shape = "wideTake" THEN
      LET xs = wideTake(n)
      total = total + len(xs)
      last = lastName(xs)
    ELSEIF shape = "wideDrop" THEN
      LET xs = wideDrop(n)
      total = total + len(xs)
      last = lastName(xs)
    ELSEIF shape = "wideDropAll" THEN
      LET xs = wideDropAll(n)
      total = total + len(xs)
      last = lastName(xs)
    ELSEIF shape = "intTake" THEN
      LET xs = intTake(n)
      total = total + len(xs)
      last = lastInt(xs)
    ELSEIF shape = "intDropAll" THEN
      LET xs = intDropAll(n)
      total = total + len(xs)
      last = lastInt(xs)
    END IF
  NEXT
  io::print(shape & " " & toString(total) & " " & last)
  RETURN 0
END FUNC
"#;

/// `take`/`drop`/`mid` as copies and as self-updates, over a union that reaches a
/// cycle (`Node`), a variable-width flat element (`String`) and a fixed-width one
/// (`Integer`), including lists whose payloads are out of entry order.
const VALUE_PROGRAM: &str = r#"IMPORT io
IMPORT collections
IMPORT strings

TYPE Element
  name AS String
  children AS List OF Node
END TYPE

TYPE Text
  value AS String
END TYPE

UNION Node
  Element
  Text
END UNION

FUNC el(name AS String) AS Node
  RETURN Element[name := name, children := [Text[value := name & "-kid"]]]
END FUNC

FUNC tx(value AS String) AS Node
  RETURN Text[value := value]
END FUNC

FUNC showN(xs AS List OF Node) AS String
  MUT out AS List OF String = []
  FOR EACH n IN xs
    MATCH n
      CASE Element(e)
        out = collections::append(out, e.name & "/" & toString(len(e.children)))
      CASE Text(t)
        out = collections::append(out, "'" & t.value & "'")
    END MATCH
  NEXT
  RETURN "[" & strings::join(out, " ") & "]"
END FUNC

FUNC showS(xs AS List OF String) AS String
  RETURN "[" & strings::join(xs, " ") & "]"
END FUNC

FUNC showI(xs AS List OF Integer) AS String
  MUT out AS List OF String = []
  FOR EACH n IN xs
    out = collections::append(out, toString(n))
  NEXT
  RETURN "[" & strings::join(out, " ") & "]"
END FUNC

FUNC copies() AS Nothing
  LET an AS List OF Node = [el("a"), tx("b"), el("c"), tx("d")]
  LET tn = collections::take(an, 2)
  LET dn = collections::drop(an, 1)
  LET mn = collections::mid(an, 1, 2)
  io::print("copy node " & showN(an) & " " & showN(tn) & " " & showN(dn) & " " & showN(mn))
  LET sa AS List OF String = ["a", "bb", "ccc", "dddd"]
  LET ts = collections::take(sa, 2)
  LET ds = collections::drop(sa, 1)
  LET ms = collections::mid(sa, 1, 2)
  io::print("copy str " & showS(sa) & " " & showS(ts) & " " & showS(ds) & " " & showS(ms))
  LET ai AS List OF Integer = [1, 2, 3, 4]
  LET ti = collections::take(ai, 2)
  LET di = collections::drop(ai, 1)
  LET mi = collections::mid(ai, 1, 2)
  io::print("copy int " & showI(ai) & " " & showI(ti) & " " & showI(di) & " " & showI(mi))
  MUT xn AS List OF Node = [el("p"), el("q"), tx("r")]
  LET keepN = xn
  xn = collections::take(xn, 1)
  io::print("alias node " & showN(keepN) & " " & showN(xn))
  MUT xs AS List OF String = ["p", "qq", "rrr"]
  LET keepS = xs
  xs = collections::drop(xs, 2)
  io::print("alias str " & showS(keepS) & " " & showS(xs))
  MUT xi AS List OF Integer = [7, 8, 9]
  LET keepI = xi
  xi = collections::mid(xi, 1, 1)
  io::print("alias int " & showI(keepI) & " " & showI(xi))
END FUNC

FUNC nodes() AS Nothing
  MUT x AS List OF Node = [el("a"), tx("b"), el("c"), tx("d"), el("e")]
  x = collections::take(x, len(x))
  io::print("n take-all " & showN(x))
  x = collections::drop(x, 0)
  io::print("n drop-none " & showN(x))
  x = collections::mid(x, 0, len(x))
  io::print("n mid-all " & showN(x))
  x = collections::take(x, 3)
  io::print("n take3 " & showN(x))
  x = collections::append(x, el("f"))
  io::print("n append " & showN(x))
  x = collections::take(x, 99)
  io::print("n take99 " & showN(x))
  x = collections::drop(x, 1)
  io::print("n drop1 " & showN(x))
  x = collections::mid(x, 1, 2)
  io::print("n mid12 " & showN(x))
  x = collections::prepend(x, el("front"))
  x = collections::take(x, len(x) - 1)
  io::print("n prepend-pop " & showN(x))
  x = collections::append(x, tx("g"))
  x = collections::take(x, len(x) - 1)
  io::print("n append-pop " & showN(x))
  x = collections::insert(x, 1, el("mid"))
  x = collections::take(x, 2)
  io::print("n insert-take2 " & showN(x))
  x = collections::set(x, 0, el("a-much-longer-name"))
  x = collections::take(x, 1)
  io::print("n set-take1 " & showN(x))
  x = collections::take(x, -3)
  io::print("n take-neg " & showN(x))
  x = collections::append(x, el("z"))
  x = collections::drop(x, 5)
  io::print("n drop-past " & showN(x))
END FUNC

FUNC strs() AS Nothing
  MUT x AS List OF String = ["d", "bb", "a", "ccc", "eeeee"]
  x = collections::take(x, len(x))
  x = collections::drop(x, 0)
  io::print("s all " & showS(x))
  x = collections::take(x, 4)
  io::print("s take4 " & showS(x))
  x = collections::sort(x)
  x = collections::take(x, 3)
  io::print("s sort-take3 " & showS(x))
  x = collections::take(x, 2)
  io::print("s take2 " & showS(x))
  x = collections::append(x, "tail")
  x = collections::set(x, 0, "a-very-long-replacement")
  x = collections::take(x, 2)
  io::print("s set-take2 " & showS(x))
  x = collections::prepend(x, "p")
  x = collections::drop(x, 1)
  io::print("s prepend-drop1 " & showS(x))
  x = collections::append(x, "q")
  x = collections::take(x, 0)
  io::print("s take0 " & showS(x) & " " & toString(len(x)))
  x = collections::append(x, "r")
  io::print("s reuse " & showS(x))
END FUNC

FUNC ints() AS Nothing
  MUT x AS List OF Integer = [1, 2, 3, 4, 5]
  x = collections::take(x, len(x))
  x = collections::drop(x, 0)
  x = collections::take(x, 4)
  x = collections::drop(x, 1)
  x = collections::mid(x, 1, 1)
  io::print("i " & showI(x))
END FUNC

FUNC main() AS Nothing
  copies()
  nodes()
  strs()
  ints()
END FUNC
"#;

const VALUE_OUTPUT: &str = "\
copy node [a/1 'b' c/1 'd'] [a/1 'b'] ['b' c/1 'd'] ['b' c/1]
copy str [a bb ccc dddd] [a bb] [bb ccc dddd] [bb ccc]
copy int [1 2 3 4] [1 2] [2 3 4] [2 3]
alias node [p/1 q/1 'r'] [p/1]
alias str [p qq rrr] [rrr]
alias int [7 8 9] [8]
n take-all [a/1 'b' c/1 'd' e/1]
n drop-none [a/1 'b' c/1 'd' e/1]
n mid-all [a/1 'b' c/1 'd' e/1]
n take3 [a/1 'b' c/1]
n append [a/1 'b' c/1 f/1]
n take99 [a/1 'b' c/1 f/1]
n drop1 ['b' c/1 f/1]
n mid12 [c/1 f/1]
n prepend-pop [front/1 c/1]
n append-pop [front/1 c/1]
n insert-take2 [front/1 mid/1]
n set-take1 [a-much-longer-name/1]
n take-neg []
n drop-past []
s all [d bb a ccc eeeee]
s take4 [d bb a ccc]
s sort-take3 [a bb ccc]
s take2 [a bb]
s set-take2 [a-very-long-replacement bb]
s prepend-drop1 [a-very-long-replacement bb]
s take0 [] 0
s reuse [r]
i [3]
";

fn scale_exe() -> &'static PathBuf {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let project = common::temp_project("b652_scale", SCALE_PROGRAM);
        common::build_project(&project)
    })
}

/// Run `exe shape n rounds`, require the full list back, and return the child's
/// CPU seconds.
fn cpu_run(exe: &Path, shape: &str, n: u64, rounds: u64) -> f64 {
    let mut child = Command::new(exe)
        .args([shape, &n.to_string(), &rounds.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run the program");
    let mut stdout_pipe = child.stdout.take().expect("child stdout");
    let mut stderr_pipe = child.stderr.take().expect("child stderr");
    let pid = child.id() as libc::pid_t;
    let mut status: libc::c_int = 0;
    // SAFETY: `rusage` is plain old data; `wait4` fills it for the reaped child.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // Reap this one child with `wait4`, the only call that reports ITS CPU time
    // (`getrusage(RUSAGE_CHILDREN)` would fold in the `mfb build` child). The
    // program writes one short line, far below a pipe buffer, so it cannot block
    // on a write while nobody reads; the pipes are drained after it exits.
    let reaped = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
    assert_eq!(
        reaped,
        pid,
        "wait4 failed: {}",
        std::io::Error::last_os_error()
    );
    let mut stdout = String::new();
    let mut stderr = String::new();
    stdout_pipe
        .read_to_string(&mut stdout)
        .expect("read stdout");
    stderr_pipe
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
        "{shape} n={n} rounds={rounds} failed (status {status}):\n{stdout}\n{stderr}"
    );
    // Element `i` of every shape is `i` (a leaf named `toString(i)`, or `i`
    // itself), so the last element of a build of `n` is `n`.
    assert_eq!(
        stdout.lines().next(),
        Some(format!("{shape} {} {n}", n * rounds).as_str()),
        "bug-652: {shape} n={n} rounds={rounds}: wrong element count or last element"
    );
    let seconds = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    seconds(usage.ru_utime) + seconds(usage.ru_stime)
}

/// Assert `shape` costs the same for `ROUNDS` builds of `n` as for one build of
/// `n * ROUNDS`.
fn assert_linear(shape: &str, n: u64) {
    let exe = scale_exe();
    // An untimed first run of each: a freshly written executable's first launch
    // pays a one-off start-up cost that would inflate only one side.
    cpu_run(exe, shape, n, ROUNDS);
    cpu_run(exe, shape, n * ROUNDS, 1);
    let short = (0..3)
        .map(|_| cpu_run(exe, shape, n, ROUNDS))
        .fold(f64::INFINITY, f64::min);
    let bound = short * MAX_RATIO + SLACK_SECONDS;
    let mut long = cpu_run(exe, shape, n * ROUNDS, 1);
    if long > bound {
        for _ in 0..2 {
            long = long.min(cpu_run(exe, shape, n * ROUNDS, 1));
        }
    }
    eprintln!(
        "{shape}: {ROUNDS}×{n} = {short:.3} s CPU; 1×{} = {long:.3} s CPU; ×{:.2}",
        n * ROUNDS,
        long / short.max(1e-9)
    );
    assert!(
        long <= bound,
        "bug-652: {shape}: {ROUNDS} builds of {n} elements took {short:.3} s CPU, one build of \
         {} took {long:.3} s: ×{:.1} for the same number of elements. Linear is ×1; a \
         self-update that walks the whole list on every call is ×{ROUNDS}.",
        n * ROUNDS,
        long / short.max(1e-9)
    );
}

/// The regression: `pending = take(pending, len(pending))` over a list of a union
/// that reaches a cycle. Red before bug-652's fix (×7.7).
#[test]
fn a_take_all_self_update_over_a_variable_width_list_is_linear() {
    assert_linear("wideTake", 5_000);
}

/// The regression: `pending = drop(pending, 0)`, the same list.
#[test]
fn a_drop_none_self_update_over_a_variable_width_list_is_linear() {
    assert_linear("wideDropAll", 5_000);
}

/// The shapes that were already linear must stay linear: `append` alone, a
/// `drop` bound to another name, and the fixed-width take/drop loops.
#[test]
fn the_already_linear_shapes_stay_linear() {
    assert_linear("wideAppend", 50_000);
    assert_linear("wideDrop", 50_000);
    assert_linear("intTake", 125_000);
    assert_linear("intDropAll", 125_000);
}

/// `take`/`drop`/`mid` bound to another name copy; a self-update keeps the right
/// elements whatever order the payloads are in.
#[test]
fn take_drop_mid_keep_value_semantics_and_the_right_elements() {
    let project = common::temp_project("b652_values", VALUE_PROGRAM);
    let exe = common::build_project(&project);
    let output = Command::new(&exe).output().expect("run the program");
    assert!(
        output.status.success(),
        "the value program failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), VALUE_OUTPUT);
    let _ = std::fs::remove_dir_all(&project);
}

/// Built with `--debug`, the same program frees every block it allocates: each
/// element a `take`/`drop`/`mid` removes — records and unions owning a graph —
/// is freed exactly once.
#[test]
fn take_drop_mid_free_every_removed_element_once() {
    let project = common::temp_project("b652_leaks", VALUE_PROGRAM);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let build_stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "build failed:\n{build_stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Vec<&str> = build_stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let exe = written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("no executable in the build output:\n{build_stdout}"))
        .to_string();
    let output = Command::new(&exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the program failed:\n{stdout}\n{stderr}"
    );
    assert_eq!(stdout, VALUE_OUTPUT, "the --debug build changed the output");
    let counter = |key: &str| {
        stderr
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|n| n.trim().parse::<u64>().ok())
            .unwrap_or_else(|| panic!("no `{key}<n>` in the report:\n{stderr}"))
    };
    let allocs = counter("arena.0.alloc_calls ");
    let frees = counter("arena.0.free_calls ");
    let live = counter("arena.0.live_bytes ");
    let skips = counter("arena.0.double_free_skips ");
    assert_eq!(skips, 0, "bug-652: a removed element was freed twice");
    assert_eq!(
        (allocs, live),
        (frees, 0),
        "bug-652: {} block(s) / {live} byte(s) never freed — a removed element's graph leaked",
        allocs.saturating_sub(frees)
    );
    let _ = std::fs::remove_dir_all(&project);
}
