//! plan-176-C: `rt_scope_drop_leaks`' unbound helper result (bug-576), `getOr` / `get`
//! element (bug-592) and trapped-error retention (bug-593) tests. The shared helpers
//! live in the root, `rt_scope_drop_leaks.rs`.

use super::*;

// ---------------------------------------------------------------- bug-576

/// bug-576: an UNBOUND runtime-helper `String` result had no owner at all.
///
/// `LET s AS String = os::arch()` has always been flat — the `Bind` path's
/// `owns_freeable_value` registers a scope-drop `arena_free` for the block the
/// call yields. The same call written where nothing binds it —
/// `n = n + len(os::arch())`, `"x" & os::arch()`, an argument to another call —
/// yields a `ValueResult` no binding claims, and `register_pending_temp` declined
/// it because a bare `String` needs freshness provenance (bug-536 shape B) that
/// `emit_runtime_helper_call` never set. Measured on the pre-fix release binary,
/// 200k -> 400k iterations: 129 B/call for `os::hostName()`, 64 B/call for
/// `os::arch()`, 260 B/call for `fs::tempDirectory()`; the `LET`-bound spelling of
/// the same call moved 32 KB in total.
///
/// The fix ADDS an `arena_free`, so the hazard it carries is a DOUBLE free, not a
/// leak: every position that already owned its result is pinned below as a
/// positive case, and the value probe reads back every shape whose block now has
/// a statement-scope owner.
const SHAPE_576_UNBOUND_HOSTNAME: &str = "IMPORT io\n\
IMPORT os\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    n = n + len(os::hostName())\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// The helper that allocates NOTHING but its result — the row that proves this is
/// not bug-574's marshalling scratch, which this shape never allocates.
const SHAPE_576_UNBOUND_ARCH: &str = "IMPORT io\n\
IMPORT os\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    n = n + len(os::arch())\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// The largest measured row (260 B/call): a longer result block.
const SHAPE_576_UNBOUND_TEMPDIR: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    n = n + len(fs::tempDirectory())\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// The result consumed by `&` rather than by `len` — a different consumer, the
/// same ownerless block. The concat's own result is bound, so only the helper's
/// block is at stake here.
const SHAPE_576_UNBOUND_IN_CONCAT: &str = "IMPORT io\n\
IMPORT os\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET joined AS String = \"arch=\" & os::arch()\n\
    n = n + len(joined)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// The result consumed as ANOTHER call's argument — the third unbound position
/// named in the report. The outer call copies what it needs; the helper's block is
/// dead the moment it returns.
const SHAPE_576_UNBOUND_AS_ARGUMENT: &str = "IMPORT io\n\
IMPORT os\n\
IMPORT strings\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET shout AS String = strings::upper(os::arch())\n\
    n = n + len(shout)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// POSITIVE PIN. `LET s AS String = os::arch()` was flat BEFORE this change and
/// must stay flat: the bind claims the pending temp, so the block has exactly one
/// owner. A second free here would be a double free — the shape this cluster
/// nearly shipped twice.
const SHAPE_576_CONTRAST_BOUND_ARCH: &str = "IMPORT io\n\
IMPORT os\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = os::arch()\n\
    n = n + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// POSITIVE PIN, the report's second flat row.
const SHAPE_576_CONTRAST_BOUND_CWD: &str = "IMPORT io\n\
IMPORT fs\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = fs::currentDirectory()\n\
    n = n + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// `RETURN os::arch()` — measured RED at 12 MB / 200k too, and for the same
/// reason with one more step. `lower_returned_value` found no pending temp to
/// claim (there was none to register), so it fell through to the
/// `current_returns_fresh_string` arm and `copy_flat_block`ed the helper's block
/// to deliver the promise — leaving the ORIGINAL with no owner. Marking the
/// result makes it a claimable temp, so the block is MOVED to the caller and the
/// redundant copy disappears with the leak, exactly as bug-536 shape A's claim
/// arm does for a fresh constructor.
///
/// This is also a double-free pin: if the claim missed while the free was
/// registered, the caller reads a freed block — so it is read back in the value
/// probe as well as measured here.
const SHAPE_576_CONTRAST_RETURNED: &str = "IMPORT io\n\
IMPORT os\n\
FUNC whichArch() AS String\n  RETURN os::arch()\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET s AS String = whichArch()\n\
    n = n + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// The result passed straight into a collection — measured RED at 24 MB / 200k.
/// `append` COPIES the element's bytes into the container's data region, so the
/// helper's own block is dead the moment the call returns and nothing owned it.
///
/// It is simultaneously the sharpest double-free pin in the set: if `append` took
/// the pointer instead of copying, the new statement-scope free would `arena_free`
/// a block the list still points into, and the element read back on the next line
/// is what catches that.
///
/// The element is read into a BINDING deliberately. An UNBOUND
/// `collections::getOr(...)` `String` leaks 64 B per call by itself — measured at
/// exactly that on a list built from a LITERAL, with no runtime helper anywhere in
/// the program — which is a different producer's missing freshness mark, not this
/// one's. Binding it keeps this case measuring the helper's block; the unbound
/// `getOr` is filed separately.
const SHAPE_576_CONTRAST_INTO_COLLECTION: &str = "IMPORT io\n\
IMPORT os\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = []\n\
    names = collections::append(names, os::arch())\n\
    LET element AS String = collections::getOr(names, 0, \"\")\n\
    n = n + len(element)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

#[cfg(unix)]
#[test]
fn an_unbound_helper_hostname_runs_at_constant_rss() {
    assert_flat(
        "b576_unbound_hostname",
        SHAPE_576_UNBOUND_HOSTNAME,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_helper_result_that_allocates_only_its_result_runs_at_constant_rss() {
    assert_flat(
        "b576_unbound_arch",
        SHAPE_576_UNBOUND_ARCH,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_helper_tempdir_runs_at_constant_rss() {
    assert_flat(
        "b576_unbound_tempdir",
        SHAPE_576_UNBOUND_TEMPDIR,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_helper_result_consumed_by_concat_runs_at_constant_rss() {
    assert_flat(
        "b576_unbound_concat",
        SHAPE_576_UNBOUND_IN_CONCAT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_helper_result_passed_to_another_call_runs_at_constant_rss() {
    assert_flat(
        "b576_unbound_argument",
        SHAPE_576_UNBOUND_AS_ARGUMENT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_bound_helper_result_still_runs_at_constant_rss() {
    assert_flat(
        "b576_bound_arch",
        SHAPE_576_CONTRAST_BOUND_ARCH,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_bound_directory_helper_result_still_runs_at_constant_rss() {
    assert_flat(
        "b576_bound_cwd",
        SHAPE_576_CONTRAST_BOUND_CWD,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_returned_helper_result_still_runs_at_constant_rss() {
    assert_flat(
        "b576_returned",
        SHAPE_576_CONTRAST_RETURNED,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_helper_result_moved_into_a_collection_still_runs_at_constant_rss() {
    assert_flat(
        "b576_into_collection",
        SHAPE_576_CONTRAST_INTO_COLLECTION,
        200_000,
        400_000,
    );
}

/// The VALUE half, and the half that carries the risk. bug-576 ADDS an
/// `arena_free` on a position that previously had none, so the way it fails is a
/// block freed while someone still holds it — a wrong value or a later unrelated
/// allocation failure, never a failing free.
///
/// Every claimed position is exercised in one process, after a churn loop that
/// guarantees the arena has recycled anything freed too early into a later
/// allocation:
///
/// * the unbound positions themselves, read back through `len`, `&` and an outer
///   call;
/// * a `LET`-bound result, a reassigned `MUT`, a returned result and a result
///   moved into a `List OF String` — the four positions that already owned the
///   block and must not gain a second free;
/// * `thread::waitFor`, whose result the WORKER's arena allocated: it is
///   `runtime_call_result_is_foreign_arena`, so it must keep its old exemption.
///   A cross-arena free is the one failure mode in this family that corrupts
///   another thread's heap.
const SHAPE_576_VALUES: &str = "IMPORT io\n\
IMPORT os\n\
IMPORT fs\n\
IMPORT strings\n\
IMPORT collections\n\
IMPORT thread\n\
ISOLATED FUNC worker(w AS ThreadWorker OF String TO String, seed AS String) AS String\n\
  RETURN seed & \"-done\"\n\
END FUNC\n\
FUNC whichArch() AS String\n  RETURN os::arch()\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT churn AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < 2000\n\
    churn = churn + len(os::arch()) + len(fs::tempDirectory())\n\
    LET scratch AS String = \"pad-\" & toString(i) & \"-pad\"\n\
    churn = churn + len(scratch)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"churn=\" & toString(churn > 0))\n\
  LET arch AS String = os::arch()\n\
  io::print(\"boundMatchesUnbound=\" & toString(len(arch) = len(os::arch())))\n\
  io::print(\"concat=arch:\" & os::arch())\n\
  io::print(\"upper=\" & strings::upper(os::arch()))\n\
  io::print(\"returned=\" & whichArch())\n\
  io::print(\"returnedMatches=\" & toString(whichArch() = arch))\n\
  MUT reassigned AS String = \"seed\"\n\
  reassigned = os::arch()\n\
  io::print(\"reassigned=\" & reassigned)\n\
  MUT names AS List OF String = []\n\
  names = collections::append(names, os::arch())\n\
  names = collections::append(names, os::hostName())\n\
  io::print(\"element=\" & collections::getOr(names, 0, \"MISSING\"))\n\
  io::print(\"elements=\" & toString(len(names)))\n\
  io::print(\"host=\" & toString(len(os::hostName()) > 0))\n\
  io::print(\"tmp=\" & toString(len(fs::tempDirectory()) > 0))\n\
  io::print(\"cwd=\" & toString(len(fs::currentDirectory()) > 0))\n\
  RES t AS Thread OF String TO String = thread::start(worker, \"worker\")\n\
  LET out AS String = thread::waitFor(t) TRAP(e)\n\
    RECOVER \"thread-failed\"\n\
  END TRAP\n\
  io::print(\"thread=\" & out)\n\
  io::print(\"archAgain=\" & arch)\n\
END FUNC\n";

#[test]
fn every_unbound_helper_result_position_still_produces_the_right_value() {
    let project = common::temp_project("b576_values", SHAPE_576_VALUES);
    let exe = common::build_project(&project);
    // `os::arch()` is a fixed string for the host, so the expectation is derived
    // from the one line that reads it back through a shape this change does not
    // touch (an assignment from a `MUT`) rather than hard-coded per platform.
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the unbound-helper-result ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.trim().lines().collect();
        let arch = lines
            .iter()
            .find_map(|line| line.strip_prefix("reassigned="))
            .unwrap_or_else(|| panic!("run {run}: no `reassigned` line in:\n{stdout}"))
            .to_string();
        assert!(
            !arch.is_empty(),
            "run {run}: the architecture read back empty:\n{stdout}"
        );
        let expected = [
            "churn=TRUE".to_string(),
            "boundMatchesUnbound=TRUE".to_string(),
            format!("concat=arch:{arch}"),
            format!("upper={}", arch.to_uppercase()),
            format!("returned={arch}"),
            "returnedMatches=TRUE".to_string(),
            format!("reassigned={arch}"),
            format!("element={arch}"),
            "elements=2".to_string(),
            "host=TRUE".to_string(),
            "tmp=TRUE".to_string(),
            "cwd=TRUE".to_string(),
            "thread=worker-done".to_string(),
            format!("archAgain={arch}"),
        ]
        .join("\n");
        assert_eq!(
            stdout.trim(),
            expected,
            "run {run}: an unbound runtime-helper result read back wrong. bug-576 \
             adds a statement-scope `arena_free` to positions that had no owner, \
             so its failure direction is a block freed while someone still holds \
             it — a `thread=` mismatch means the free reached the WORKER's arena"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

// ---------------------------------------------------------------- bug-592

/// bug-592: an UNBOUND `collections::getOr` `String` element had no owner.
///
/// `§14.6` of `mfb spec language memory-semantics`: "Reads produce owned values,
/// not aliases into the buffer." An owned value nothing binds is dropped at
/// statement scope — but a bare `String` temp is freed there only with freshness
/// provenance (bug-536 shape B), and `getOr` lost its mark: the MISS path's
/// `emit_copy_owned_string` runs after the found path's materialization and
/// overwrites the mark with its own register, which is not the lowering's
/// result, so `lower_value`'s identity test rejected it. `get` has no copying
/// miss path (it raises), which is why `get` was always flat.
///
/// Measured on the pre-fix release binary (`integ-576-590`), 200k -> 400k
/// iterations, `/usr/bin/time -l` peak RSS:
///
/// | shape | per call |
/// | --- | --- |
/// | list `getOr` hit | 64 B |
/// | list `getOr` miss | 129 B |
/// | `Map OF String TO String` (hash probe) `getOr` hit / miss | 64 B / 128 B |
/// | `Map OF Scalar TO String` (entry scan) `getOr` hit / miss | 65 B / 130 B |
/// | unbound `get`, list / hash map / scan map | 0 B |
/// | `LET`-bound `getOr` | 0 B |
///
/// The fix ADDS a statement-scope `arena_free`, so the failure it risks is a
/// WILD or DOUBLE free, not a leak: a block the container, the caller's default,
/// or an owning binding still holds. The positive pins below and the value probe
/// are that half.
const SHAPE_592_LIST_GETOR_HIT: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"aarch64\"]\n\
    n = n + len(collections::getOr(names, 0, \"\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// The miss path: the result is `emit_copy_owned_string`'s copy of the caller's
/// default. Freeing it must never free the DEFAULT itself — the value probe reads
/// the default back after thousands of misses.
const SHAPE_592_LIST_GETOR_MISS: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"aarch64\"]\n\
    n = n + len(collections::getOr(names, 5, \"fallback\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// `lower_map_get_or`'s hash-probe arm (a `String` key is probe-eligible).
const SHAPE_592_HASH_MAP_GETOR_HIT: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT m AS Map OF String TO String = Map OF String TO String {\"k\" := \"aarch64\"}\n\
    n = n + len(collections::getOr(m, \"k\", \"\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

const SHAPE_592_HASH_MAP_GETOR_MISS: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT m AS Map OF String TO String = Map OF String TO String {\"k\" := \"aarch64\"}\n\
    n = n + len(collections::getOr(m, \"zz\", \"fallback\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// `lower_map_get_or`'s linear entry-scan arm (a `Scalar` key is not
/// probe-eligible) — a separate emission path with its own join point.
const SHAPE_592_SCAN_MAP_GETOR_HIT: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT m AS Map OF Scalar TO String = Map OF Scalar TO String {}\n\
    m = collections::set(m, `k`, \"aarch64\")\n\
    n = n + len(collections::getOr(m, `k`, \"\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

const SHAPE_592_SCAN_MAP_GETOR_MISS: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT m AS Map OF Scalar TO String = Map OF Scalar TO String {}\n\
    m = collections::set(m, `k`, \"aarch64\")\n\
    n = n + len(collections::getOr(m, `z`, \"fallback\"))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// `RETURN collections::getOr(..)` — with no pending temp to claim, the callee's
/// `function_returns_fresh_string` promise was delivered by a `copy_flat_block`
/// that left the original block ownerless (bug-576's returned row, one producer
/// over). Marked, the temp is claimed and MOVED to the caller. Also a
/// double-free pin: a missed claim with a registered free hands the caller a
/// freed block, which the value probe reads back.
const SHAPE_592_RETURNED: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC firstName(names AS List OF String) AS String\n  RETURN collections::getOr(names, 0, \"none\")\nEND FUNC\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"aarch64\"]\n\
    LET s AS String = firstName(names)\n\
    n = n + len(s)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// POSITIVE PIN. The report's flat row: the bind owns the block. With the mark
/// the bind CLAIMS the pending temp instead; a missed claim is a double free.
const SHAPE_592_CONTRAST_BOUND_GETOR: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"aarch64\"]\n\
    LET e AS String = collections::getOr(names, 0, \"\")\n\
    n = n + len(e)\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// POSITIVE PIN. Unbound `get` on a list and on both map arms was already flat —
/// the producer's own mark survived because the miss path raises. The join-point
/// mark restates it; it must not add a second free.
const SHAPE_592_CONTRAST_UNBOUND_GET: &str = "IMPORT io\n\
IMPORT collections\n\
FUNC main() AS Nothing\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"aarch64\"]\n\
    MUT m AS Map OF String TO String = Map OF String TO String {\"k\" := \"x86_64\"}\n\
    MUT s AS Map OF Scalar TO String = Map OF Scalar TO String {}\n\
    s = collections::set(s, `k`, \"riscv64\")\n\
    n = n + len(collections::get(names, 0)) + len(collections::get(m, \"k\")) + len(collections::get(s, `k`))\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

/// POSITIVE PIN — the ALIAS direction. plan-86 E: `LET e = get(L, i)` used only
/// as a `MATCH` scrutinee over an immutable `L` makes `e` an ALIAS into `L`'s
/// data region (`borrow_get_result`), never freed. Its element is a non-`String`
/// union, so `mark_fresh_element_result` never marks it, and
/// `register_pending_temp` early-returns while the flag is set. A wrong free here
/// is an `arena_free` into the container; the value probe reads the container
/// back (and adds a borrowed read whose map KEY is an unbound `String` `getOr`
/// inside the borrowed initializer) to see that as a corrupted neighbour.
const SHAPE_592_CONTRAST_BORROWED_GET: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE Dot\n  x AS Integer\nEND TYPE\n\
TYPE Tag\n  name AS String\nEND TYPE\n\
UNION Shape\n  Dot\n  Tag\nEND UNION\n\
FUNC main() AS Nothing\n\
  LET shapes AS List OF Shape = [Dot[1], Tag[\"tag-name-long-enough-to-matter\"], Dot[3]]\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    LET e AS Shape = collections::get(shapes, i - (i / 3) * 3)\n\
    MATCH e\n\
      CASE Dot(d)\n\
        n = n + d.x\n\
      CASE Tag(t)\n\
        n = n + len(t.name)\n\
    END MATCH\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

#[cfg(unix)]
#[test]
fn an_unbound_list_getor_hit_runs_at_constant_rss() {
    assert_flat(
        "b592_list_getor_hit",
        SHAPE_592_LIST_GETOR_HIT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_list_getor_miss_runs_at_constant_rss() {
    assert_flat(
        "b592_list_getor_miss",
        SHAPE_592_LIST_GETOR_MISS,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_hash_map_getor_hit_runs_at_constant_rss() {
    assert_flat(
        "b592_hash_map_getor_hit",
        SHAPE_592_HASH_MAP_GETOR_HIT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_hash_map_getor_miss_runs_at_constant_rss() {
    assert_flat(
        "b592_hash_map_getor_miss",
        SHAPE_592_HASH_MAP_GETOR_MISS,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_scan_map_getor_hit_runs_at_constant_rss() {
    assert_flat(
        "b592_scan_map_getor_hit",
        SHAPE_592_SCAN_MAP_GETOR_HIT,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_scan_map_getor_miss_runs_at_constant_rss() {
    assert_flat(
        "b592_scan_map_getor_miss",
        SHAPE_592_SCAN_MAP_GETOR_MISS,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_returned_getor_element_runs_at_constant_rss() {
    assert_flat("b592_returned", SHAPE_592_RETURNED, 200_000, 400_000);
}

#[cfg(unix)]
#[test]
fn a_bound_getor_element_still_runs_at_constant_rss() {
    assert_flat(
        "b592_bound_getor",
        SHAPE_592_CONTRAST_BOUND_GETOR,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn an_unbound_get_element_still_runs_at_constant_rss() {
    assert_flat(
        "b592_unbound_get",
        SHAPE_592_CONTRAST_UNBOUND_GET,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_borrowed_get_element_still_runs_at_constant_rss() {
    assert_flat(
        "b592_borrowed_get",
        SHAPE_592_CONTRAST_BORROWED_GET,
        200_000,
        400_000,
    );
}

/// The VALUE half, and the half that carries the risk. Every position whose
/// `getOr`/`get` `String` now has a statement-scope owner, and every position that
/// must NOT gain one, exercised in one process after a churn loop that recycles
/// anything freed too early into a later allocation:
///
/// * unbound `getOr` hit and miss on all three emission paths (list, hash map,
///   scan map) and unbound `get` on list and hash map;
/// * the caller's DEFAULT (`dflt`), read back after thousands of misses — the
///   miss path frees its copy, never the default;
/// * a `LET`-bound, a reassigned `MUT`, a returned, and an appended element — the
///   positions that own the block through a claim;
/// * two plan-86 E borrowed (ALIAS) reads: a union element used only as a `MATCH`
///   scrutinee, and one whose map KEY is itself an unbound `String` `getOr`
///   inside the borrowed initializer;
/// * the containers themselves, read back element by element at the end — a
///   free that landed INTO a container shows up here as a corrupted element.
const SHAPE_592_VALUES: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE Dot\n  x AS Integer\nEND TYPE\n\
TYPE Tag\n  name AS String\nEND TYPE\n\
UNION Shape\n  Dot\n  Tag\nEND UNION\n\
FUNC firstName(names AS List OF String) AS String\n  RETURN collections::getOr(names, 0, \"none\")\nEND FUNC\n\
FUNC main() AS Nothing\n\
  LET names AS List OF String = [\"alpha-element-000\", \"beta-element-1111\", \"gamma-element-22222\"]\n\
  LET byKey AS Map OF String TO String = Map OF String TO String {\"a\" := \"map-alpha-value\", \"b\" := \"map-beta-value-longer\"}\n\
  MUT byScalar AS Map OF Scalar TO String = Map OF Scalar TO String {}\n\
  byScalar = collections::set(byScalar, `a`, \"scalar-alpha-value\")\n\
  LET shapes AS List OF Shape = [Dot[1], Tag[\"tag-name-long-enough-to-matter\"], Dot[3]]\n\
  LET byShape AS Map OF String TO Shape = Map OF String TO Shape {\"alpha-element-000\" := Tag[\"via-map\"], \"none\" := Dot[9]}\n\
  LET dflt AS String = \"default-\" & toString(7)\n\
  MUT churn AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < 5000\n\
    churn = churn + len(collections::getOr(names, i - (i / 4) * 4, dflt))\n\
    churn = churn + len(collections::getOr(byKey, \"a\", dflt)) + len(collections::getOr(byKey, \"zz\", dflt))\n\
    churn = churn + len(collections::getOr(byScalar, `a`, dflt)) + len(collections::getOr(byScalar, `z`, dflt))\n\
    churn = churn + len(collections::get(names, 1)) + len(collections::get(byKey, \"b\"))\n\
    LET scratch AS String = \"pad-\" & toString(i) & \"-pad\"\n\
    churn = churn + len(scratch)\n\
    LET e AS Shape = collections::get(shapes, i - (i / 3) * 3)\n\
    MATCH e\n\
      CASE Dot(d)\n\
        churn = churn + d.x\n\
      CASE Tag(t)\n\
        churn = churn + len(t.name)\n\
    END MATCH\n\
    LET k AS Shape = collections::get(byShape, collections::getOr(names, 5, \"none\"))\n\
    MATCH k\n\
      CASE Dot(d)\n\
        churn = churn + d.x\n\
      CASE Tag(t)\n\
        churn = churn + len(t.name)\n\
    END MATCH\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"churn=\" & toString(churn))\n\
  io::print(\"concat=\" & collections::getOr(names, 2, \"x\") & \"|\" & collections::getOr(names, 9, dflt))\n\
  io::print(\"dflt=\" & dflt)\n\
  LET bound AS String = collections::getOr(names, 1, dflt)\n\
  MUT reassigned AS String = \"seed\"\n\
  reassigned = collections::getOr(byKey, \"b\", dflt)\n\
  io::print(\"bound=\" & bound & \" reassigned=\" & reassigned)\n\
  io::print(\"returned=\" & firstName(names))\n\
  MUT copied AS List OF String = []\n\
  copied = collections::append(copied, collections::getOr(names, 0, dflt))\n\
  copied = collections::append(copied, collections::getOr(byScalar, `q`, dflt))\n\
  io::print(\"copied=\" & collections::get(copied, 0) & \",\" & collections::get(copied, 1))\n\
  io::print(\"names=\" & collections::get(names, 0) & \",\" & collections::get(names, 1) & \",\" & collections::get(names, 2))\n\
  io::print(\"maps=\" & collections::get(byKey, \"a\") & \",\" & collections::get(byKey, \"b\") & \",\" & collections::get(byScalar, `a`))\n\
  io::print(\"bound=\" & bound & \" dflt=\" & dflt)\n\
END FUNC\n";

#[test]
fn every_unbound_collection_element_position_still_produces_the_right_value() {
    // The churn total, derived from the literal lengths rather than from any
    // compiler's output (a leaky binary prints the same number; a wrong free does
    // not).
    let names = [
        "alpha-element-000",
        "beta-element-1111",
        "gamma-element-22222",
    ];
    let dflt = "default-7";
    let tag = "tag-name-long-enough-to-matter";
    let mut churn: usize = 0;
    for i in 0..5000usize {
        churn += match i % 4 {
            3 => dflt.len(),
            j => names[j].len(),
        };
        churn += "map-alpha-value".len() + dflt.len();
        churn += "scalar-alpha-value".len() + dflt.len();
        churn += names[1].len() + "map-beta-value-longer".len();
        churn += format!("pad-{i}-pad").len();
        churn += match i % 3 {
            0 => 1,
            1 => tag.len(),
            _ => 3,
        };
        churn += 9; // byShape["none"] = Dot[9]
    }
    let expected = [
        format!("churn={churn}"),
        format!("concat={}|{dflt}", names[2]),
        format!("dflt={dflt}"),
        format!("bound={} reassigned=map-beta-value-longer", names[1]),
        format!("returned={}", names[0]),
        format!("copied={},{dflt}", names[0]),
        format!("names={},{},{}", names[0], names[1], names[2]),
        "maps=map-alpha-value,map-beta-value-longer,scalar-alpha-value".to_string(),
        format!("bound={} dflt={dflt}", names[1]),
    ]
    .join("\n");

    let project = common::temp_project("b592_values", SHAPE_592_VALUES);
    let exe = common::build_project(&project);
    for run in 1..=25 {
        let output = std::process::Command::new(&exe)
            .current_dir(&project)
            .output()
            .expect("run the unbound-collection-element ownership probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            stdout.trim(),
            expected,
            "run {run}: a collection element read back wrong. bug-592 adds a \
             statement-scope `arena_free` to an unbound `getOr` `String`, so its \
             failure direction is a block freed while someone still holds it — a \
             `names=`/`maps=` mismatch means the free landed INTO a container, a \
             `dflt=` mismatch that it freed the caller's default"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

/// bug-592 audit finding: plan-86 E's `borrow_get_result` flag covered the WHOLE
/// initializer, not the borrowed `get` node. So a fresh temp in one of the
/// borrowed call's OPERANDS — here an unbound `String` `getOr` used as the map
/// KEY — had its statement-scope registration suppressed too, and leaked 64 B per
/// evaluation (measured on the bug-592 join-point fix alone, 200k -> 400k). The
/// flag now applies only inside the borrowed node's own `lower_value` frame; each
/// operand frame lowers it with the flag clear, which is the ordinary copy + free
/// path. `materialize_owned_element` reads the same narrowed flag, so a nested
/// `get` operand is COPIED (never an unfreed alias) exactly when it is freed.
const SHAPE_592_BORROWED_GET_WITH_A_FRESH_KEY: &str = "IMPORT io\n\
IMPORT collections\n\
TYPE Dot\n  x AS Integer\nEND TYPE\n\
TYPE Tag\n  name AS String\nEND TYPE\n\
UNION Shape\n  Dot\n  Tag\nEND UNION\n\
FUNC main() AS Nothing\n\
  LET byShape AS Map OF String TO Shape = Map OF String TO Shape {\"alpha\" := Tag[\"via-map\"], \"none\" := Dot[9]}\n\
  MUT n AS Integer = 0\n\
  MUT i AS Integer = 0\n\
  WHILE i < {N}\n\
    MUT names AS List OF String = [\"alpha\"]\n\
    LET k AS Shape = collections::get(byShape, collections::getOr(names, 0, \"none\"))\n\
    MATCH k\n\
      CASE Dot(d)\n\
        n = n + d.x\n\
      CASE Tag(t)\n\
        n = n + len(t.name)\n\
    END MATCH\n\
    i = i + 1\n\
  END WHILE\n\
  io::print(\"n=\" & toString(n))\n\
END FUNC\n";

#[cfg(unix)]
#[test]
fn a_borrowed_get_with_a_fresh_key_operand_runs_at_constant_rss() {
    assert_flat(
        "b592_borrowed_get_fresh_key",
        SHAPE_592_BORROWED_GET_WITH_A_FRESH_KEY,
        200_000,
        400_000,
    );
}

// ---------------------------------------------------------------- bug-593
//
// A failing call under an inline `TRAP` grew memory by a flat ~1 KB per call
// when the binding's type was not a flat value — a `RES` (`fs::open`,
// `tcp::connect`, `tls::connect`, or a user callee returning a resource) or a
// `List OF net::Address`. bug-574 and bug-575 both measured it on runtime helpers
// and read it as "the failing helper"; it is not the helper. A user `FUNC` that
// `FAIL`s into a `RES` binding grows identically, and a failing `fs::readText`
// (a `String`) does not grow at all.
//
// The inline-`TRAP` desugar binds `$trap_resN : Result OF T = CallResult(..)`.
// That `{tag, size, payload}` wrapper is allocated by this frame and, on the
// error path, carries the whole trapped `Error` inlined at +16. A flat `T` gets
// an owned scope drop for it (`is_freeable_flat_value`); a non-flat `T` got
// none, so every failing iteration orphaned the wrapper and the `Error` in it.
// That is why the growth was flat in the ARGUMENT and differed by platform: its
// size is the error MESSAGE's.
//
// ## Why these are message-length comparisons and not `assert_flat`
//
// A `RES` binding's error path also allocates the default CLOSED resource record
// for `$trap_valN`, and a resource record is never reclaimed — it is the
// tombstone every alias reads the closed flag from (plan-52-B, Open Decisions:
// "Should the record itself ever be reclaimed? Not here."). A SUCCEEDING
// `fs::open` + `fs::close` loop grows the same way. When these cases were
// measured, a `List OF net::Address` binding likewise kept its default empty
// list, because `net::Address` was a pointer-`String` record with no drop
// (bug-599; plan-132 flattened it, and `a_looped_net_lookup_runs_at_constant_rss`
// now pins the list flat). Neither was this bug, and freeing either would have
// moved a lifetime, so neither shape could be flat here.
//
// What IS this bug is scaled by the trapped `Error`, and nothing else is. So each
// case runs the same program twice at the same count, differing only in the
// length of the message the callee `FAIL`s with; the tombstone and the default
// list are identical in both runs and cancel. Measured at 20 000 iterations,
// macOS aarch64, peak RSS:
//
// | binding | message | base (04c81a605) | fixed |
// | --- | --- | --- | --- |
// | `RES fs::File` | 14 chars | 17.8 MB | 8.8 MB |
// | `RES fs::File` | 4 014 chars | 328.7 MB | 8.8 MB |
// | `List OF net::Address` | 14 chars | 14.2 MB | 5.0 MB |
// | `List OF net::Address` | 4 014 chars | 328.8 MB | 5.0 MB |
//
// +311 MB before and 0 after on both; the threshold is 32 MB. The two cases take
// the fix's two drop kinds: a resource payload is a pointer word nothing aliases,
// so its wrapper is released on every path (`ResultWrapperDrop::Always`); a
// `List OF net::Address` payload is inlined in the wrapper and the Ok binding
// aliases it, so only the error-tagged wrapper is (`ErrorOnly`).
//
// The report's own shapes, measured the same way (base -> fixed): a refused
// `tcp::connect` 212.9 -> 424.3 MB became 81.4 -> 161.3 MB at 200 000 / 400 000;
// `tls::connect` refused 54.2 -> 101.5 MB became 38.7 -> 69.6 MB at
// 20 000 / 40 000; a failing `net::lookup` 23.2 -> 40.4 MB became
// 10.1 -> 14.1 MB. What remains on each is the tombstone or the default list.

/// A loop of `{N}` iterations whose callee always `FAIL`s with `message` into a
/// binding declared `binding`, produced by `producer` (whose success body is
/// never reached).
#[cfg(unix)]
fn b593_trapped_error_probe(imports: &str, binding: &str, producer: &str, message: &str) -> String {
    format!(
        "IMPORT io\n\
         {imports}\
         FUNC make(i AS Integer) AS {binding}\n\
        \x20 IF i >= 0 THEN\n\
        \x20   FAIL error(7, \"{message}\")\n\
        \x20 END IF\n\
        \x20 {producer}\n\
         END FUNC\n\
         FUNC main() AS Nothing\n\
        \x20 MUT n AS Integer = 0\n\
        \x20 MUT i AS Integer = 0\n\
        \x20 WHILE i < {{N}}\n\
        \x20   __BIND__ = make(i) TRAP(e)\n\
        \x20     n = n + 1\n\
        \x20     i = i + 1\n\
        \x20     CONTINUE WHILE\n\
        \x20   END TRAP\n\
        \x20   __USE__\n\
        \x20   i = i + 1\n\
        \x20 END WHILE\n\
        \x20 io::print(\"n=\" & toString(n))\n\
         END FUNC\n"
    )
}

/// Assert that the two message lengths cost the same peak RSS at `N` iterations.
#[cfg(unix)]
fn b593_assert_error_not_retained(name: &str, short: &str, long: &str) {
    const N: u64 = 20_000;
    let small = peak_rss(&format!("{name}_short"), short, N);
    let large = peak_rss(&format!("{name}_long"), long, N);
    let grew = large.saturating_sub(small);
    assert!(
        grew < 32 * 1024 * 1024,
        "{name}: a {N}-iteration loop of trapped errors cost {} MB more with a \
         4 014-character error message than with a 14-character one ({} MB -> {} MB). \
         The inline `TRAP` built a `Result` wrapper holding the whole `Error` inline \
         and nothing released it (bug-593)",
        grew / (1024 * 1024),
        small / (1024 * 1024),
        large / (1024 * 1024),
    );
}

#[cfg(unix)]
fn b593_long_message() -> String {
    format!("b593-long-msg-{}", "m".repeat(4_000))
}

#[cfg(unix)]
fn b593_resource_probe(message: &str) -> String {
    b593_trapped_error_probe(
        "IMPORT fs\n",
        "fs::File",
        "RES f AS fs::File = fs::open(\"/tmp/b593-does-not-exist/nope.txt\", \"r\")\n  RETURN f",
        message,
    )
    .replace("__BIND__", "RES f AS fs::File")
    .replace("__USE__", "fs::close(f)")
}

#[cfg(unix)]
fn b593_address_list_probe(message: &str) -> String {
    b593_trapped_error_probe(
        "IMPORT net\n",
        "List OF net::Address",
        "RETURN net::lookup(\"127.0.0.1\", 80)",
        message,
    )
    .replace("__BIND__", "LET xs AS List OF net::Address")
    .replace("__USE__", "n = n + len(xs)")
}

#[cfg(unix)]
#[test]
fn a_trapped_error_bound_as_a_resource_is_not_retained() {
    b593_assert_error_not_retained(
        "b593_res",
        &b593_resource_probe("b593-short-msg"),
        &b593_resource_probe(&b593_long_message()),
    );
}

#[cfg(unix)]
#[test]
fn a_trapped_error_bound_as_an_address_list_is_not_retained() {
    b593_assert_error_not_retained(
        "b593_list",
        &b593_address_list_probe("b593-short-msg"),
        &b593_address_list_probe(&b593_long_message()),
    );
}

/// The VALUE half, and the direction that matters: the fix ADDS a free of the
/// `Result` wrapper, and on the error path that wrapper holds the trapped `Error`
/// inline. Anything that still read that `Error` after the scope drop would read
/// freed memory — a wrong message or origin, or a fault at a later allocation,
/// never a failing free. So every way a handler can carry the `Error` across a
/// drop edge is here, each followed by more allocation before it is compared:
///
/// * `RETURN` from inside the handler, building the result from `e`;
/// * `FAIL inner` — a re-raise, where the drop runs on the `FAIL` edge and the
///   caller must still see the ORIGINAL origin (line 17, the `fs::open`);
/// * the hoisted chain form (bug-457), whose `Error` goes through `$trap_err`;
/// * a refused `tcp::connect`, and a user callee failing into `RES`;
/// * `List OF net::Address`, whose Ok payload IS inlined in the wrapper and
///   aliased by the binding — its Ok wrapper must NOT be released, and a resolved
///   address read back after more allocation is the pin for that;
/// * a SUCCEEDING `fs::open` read back, whose Ok wrapper now is released.
///
/// Every iteration's row is compared with the first iteration's, and the first
/// row with the text the base compiler printed, where nothing was freed.
const B593_VALUES: &str = "IMPORT io\n\
IMPORT fs\n\
IMPORT tcp\n\
IMPORT net\n\
IMPORT collections\n\
FUNC describe(e AS Error) AS String\n  RETURN toString(e.code) & \"|\" & e.message & \"|\" & e.source.filename & \":\" & toString(e.source.line)\n\
END FUNC\n\
FUNC openMissing(i AS Integer) AS String\n  RES f AS fs::File = fs::open(\"/tmp/b593-does-not-exist/nope.txt\", \"r\") TRAP(e)\n    RETURN \"open:\" & describe(e)\n  END TRAP\n  fs::close(f)\n  RETURN \"opened\"\n\
END FUNC\n\
FUNC reraiseMissing(i AS Integer) AS Integer\n  RES f AS fs::File = fs::open(\"/tmp/b593-does-not-exist/nope.txt\", \"r\") TRAP(inner)\n    FAIL inner\n  END TRAP\n  fs::close(f)\n  RETURN i\n\
END FUNC\n\
FUNC pathFor(i AS Integer) AS String\n  IF i < 0 THEN\n    FAIL error(9, \"negative\")\n  END IF\n  RETURN \"/tmp/b593-does-not-exist/hoisted.txt\"\n\
END FUNC\n\
FUNC hoisted(i AS Integer) AS String\n  RES f AS fs::File = fs::open(pathFor(i), \"r\") TRAP(e)\n    RETURN \"hoisted:\" & describe(e)\n  END TRAP\n  fs::close(f)\n  RETURN \"opened\"\n\
END FUNC\n\
FUNC refused(i AS Integer) AS String\n  RES s AS tcp::Socket = tcp::connect(\"127.0.0.1\", 1, 1000) TRAP(e)\n    RETURN \"tcp:\" & describe(e)\n  END TRAP\n  tcp::close(s)\n  RETURN \"connected\"\n\
END FUNC\n\
FUNC opener(i AS Integer) AS fs::File\n  IF i >= 0 THEN\n    FAIL error(7, \"opener-\" & toString(i MOD 1))\n  END IF\n  RES f AS fs::File = fs::open(\"/tmp/b593-does-not-exist/nope.txt\", \"r\")\n  RETURN f\n\
END FUNC\n\
FUNC userRes(i AS Integer) AS String\n  RES f AS fs::File = opener(i) TRAP(e)\n    RETURN \"user:\" & describe(e)\n  END TRAP\n  fs::close(f)\n  RETURN \"opened\"\n\
END FUNC\n\
FUNC okOpen(path AS String) AS String\n  RES f AS fs::File = fs::open(path, \"r\") TRAP(e)\n    RETURN \"ok-open-failed:\" & describe(e)\n  END TRAP\n  LET text AS String = fs::readAll(f)\n  fs::close(f)\n  RETURN \"ok:\" & text\n\
END FUNC\n\
FUNC lookupOk(i AS Integer) AS String\n  LET xs AS List OF net::Address = net::lookup(\"127.0.0.1\", 80 + i MOD 1) TRAP(e)\n    RETURN \"lookup-failed:\" & describe(e)\n  END TRAP\n  LET churn AS List OF String = [\"c1-\" & toString(i), \"c2-\" & toString(i), \"c3-\" & toString(i)]\n  LET first AS net::Address = collections::get(xs, 0)\n  RETURN \"lookup:\" & first.host & \":\" & toString(first.port) & \"/\" & toString(len(churn))\n\
END FUNC\n\
FUNC lookupFails(i AS Integer) AS String\n  LET xs AS List OF net::Address = net::lookup(\"b593-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz.invalid\", 80) TRAP(e)\n    RETURN \"lookup:\" & describe(e)\n  END TRAP\n  RETURN \"resolved:\" & toString(len(xs))\n\
END FUNC\n\
FUNC main() AS Nothing\n  LET okPath AS String = fs::pathJoin([fs::tempDirectory(), \"b593_values_probe.txt\"])\n  fs::writeText(okPath, \"hello\")\n  MUT firsts AS List OF String = []\n  MUT mismatches AS Integer = 0\n  MUT i AS Integer = 0\n  WHILE i < 500\n    LET r0 AS String = openMissing(i)\n    LET r1 AS String = toString(reraiseMissing(i)) TRAP(e1)\n      RECOVER \"reraised:\" & describe(e1)\n    END TRAP\n    LET r2 AS String = hoisted(i)\n    LET r3 AS String = refused(i)\n    LET r4 AS String = userRes(i)\n    LET r5 AS String = okOpen(okPath)\n    LET r6 AS String = lookupOk(i)\n    LET r7 AS String = lookupFails(i)\n    LET junk AS List OF String = [r0 & \"#\", r1 & \"#\", r2 & \"#\", r3 & \"#\", r4 & \"#\", r5 & \"#\", r6 & \"#\", r7 & \"#\"]\n    LET row AS List OF String = [r0, r1, r2, r3, r4, r5, r6, r7]\n    IF i = 0 THEN\n      firsts = row\n    ELSE\n      MUT k AS Integer = 0\n      WHILE k < len(row)\n        IF collections::get(row, k) <> collections::get(firsts, k) THEN\n          mismatches = mismatches + 1\n        END IF\n        k = k + 1\n      END WHILE\n    END IF\n    mismatches = mismatches + len(junk) - 8\n    i = i + 1\n  END WHILE\n  FOR EACH line IN firsts\n    io::print(line)\n  NEXT\n  io::print(\"mismatches=\" & toString(mismatches))\n\
END FUNC\n";

/// `B593_VALUES`' stdout on the base compiler (04c81a605), which released no
/// wrapper at all and so could not have read one back freed.
const B593_VALUES_EXPECTED: &str =
    "open:77030001|Filesystem path does not exist.|src/main.mfb:10\n\
reraised:77030001|Filesystem path does not exist.|src/main.mfb:17\n\
hoisted:77030001|Filesystem path does not exist.|src/main.mfb:30\n\
tcp:77070003|Network operation failed before a connection was established.|src/main.mfb:37\n\
user:7|opener-0|src/main.mfb:45\n\
ok:hello\n\
lookup:127.0.0.1:80/3\n\
lookup:77070002|Network host name or address could not be resolved.|src/main.mfb:74\n\
mismatches=0";

#[cfg(unix)]
#[test]
fn every_failing_resource_call_still_reports_its_error_and_origin() {
    let project = common::temp_project("b593_values", B593_VALUES);
    let exe = common::build_project(&project);
    // A use-after-free is not deterministic: a freed wrapper only reads back
    // wrong once the arena has handed its bytes to a later allocation.
    for run in 1..=10 {
        let output = std::process::Command::new(&exe)
            .output()
            .expect("run the bug-593 value probe");
        assert!(
            output.status.success(),
            "run {run}: {}\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            B593_VALUES_EXPECTED,
            "run {run}: a trapped error or a trapped value read back wrong. bug-593 \
             releases the `Result` wrapper an inline `TRAP` built, which holds the \
             trapped `Error` inline — a changed message or origin means something \
             still read the wrapper after its scope drop"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}
