//! Codegen contracts for the `collections::` native fast paths.
//!
//! `groupBy`, `zip` and `findLastIndex` each have a hand-written emitter that
//! replaces the package's interpreted `.mfb` body for the exact instantiations
//! it can handle, and declines (`Ok(None)`) for every other one. Two things are
//! worth pinning and neither is visible end to end:
//!
//!   * that the fast path **fires** at all. Miss the instantiation and the
//!     `.mfb` body runs instead — same answer, so every behavioural fixture
//!     still passes while the emitter under test is dead code.
//!   * that it **declines** when firing would not be sound.
//!     `group_by_fast_path` re-reads its source-list argument, so an argument
//!     that is not re-eval-safe (a call) must fall back; taking the fast path
//!     there evaluates the call twice.
//!
//! The "fires" programs are committed rt fixtures rather than hand-written
//! source: the fast paths are keyed on exact instantiations
//! (`#collections_groupBy$String$Integer$String`), and a program that misses one
//! measures nothing while looking fine.

use crate::arch::ops::CodeOp;
use crate::codegen::builtins::collections;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::engine::tests::test_support::{BuilderHarness, Stream, TestPlatform};
use crate::codegen::engine::types::NativeCodePlan;
use crate::target::shared::nir::NirValue;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_fixture, code_for_src_cached, code_function, CodeTarget};

/// Labels the named fast path emits, anywhere in the program.
fn fast_path_labels(plan: &NativeCodePlan, prefix: &str) -> Vec<String> {
    plan.functions
        .iter()
        .flat_map(|f| Stream::of(f).labels())
        .map(|(_, name)| name)
        .filter(|name| name.starts_with(prefix))
        .collect()
}

/// Calls to the interpreted `.mfb` body of `member`, anywhere in the program.
fn interpreted_calls(plan: &NativeCodePlan, member: &str) -> Vec<String> {
    let want = format!("collections_5F{member}");
    plan.functions
        .iter()
        .flat_map(|f| {
            f.instructions
                .iter()
                .filter(|i| i.op == CodeOp::BranchLink)
                .filter_map(|i| i.get("target"))
        })
        .filter(|target| target.contains(&want))
        .collect()
}

// --- func_group_by.rs -----------------------------------------------------

/// The `groupBy` fast path fires for an Integer key with a String value.
#[test]
fn group_by_takes_the_native_path_for_a_supported_instantiation() {
    let plan = code_for_fixture("groupby-string-value-native-rt", CodeTarget::LinuxX86_64);
    let labels = fast_path_labels(plan, "gb_");
    assert!(
        !labels.is_empty(),
        "`#collections_groupBy$String$Integer$String` must lower through the \
         native emitter; no `gb_*` label was emitted, so the interpreted .mfb \
         body ran and func_group_by.rs is dead code in this program"
    );
}

/// A source list that is not re-eval-safe declines to the interpreted body.
///
/// The emitter reads `args[0]` more than once, which is only sound for a value
/// that re-reading cannot change: a local, a const, a global, a local ref. Hand
/// it a CALL and taking the fast path would run that call twice.
#[test]
fn group_by_declines_when_its_source_list_would_be_evaluated_twice() {
    const SRC: &str = "\
IMPORT collections
IMPORT io

FUNC parity(n AS Integer) AS Integer
  RETURN n MOD 2
END FUNC
FUNC same(n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC build(seed AS Integer) AS List OF Integer
  RETURN [seed, seed + 1, seed + 2, seed + 3]
END FUNC

FUNC main() AS Integer
  LET g = collections::groupBy(build(4), parity, same)
  LET n AS Integer = len(collections::keys(g))
  io::print(toString(n))
  RETURN 0
END FUNC
";
    let plan = code_for_src_cached(SRC, CodeTarget::LinuxX86_64, Console);
    let labels = fast_path_labels(plan, "gb_");
    assert!(
        labels.is_empty(),
        "collections::groupBy over a CALL must decline to the interpreted body — \
         the native emitter re-reads its source list, so taking it here evaluates \
         `build(4)` twice. It emitted {labels:?}"
    );
    let fallback = interpreted_calls(plan, "groupBy");
    assert!(
        !fallback.is_empty(),
        "having declined, the call must still reach the interpreted \
         `#collections_groupBy` body"
    );
}

// --- func_zip.rs ----------------------------------------------------------

/// `zip` has TWO native paths, and both must fire.
///
/// A `Pair OF String, String` is a variable-width record (two inlined Strings),
/// so it is built and appended one element at a time; a `Pair OF Integer,
/// Integer` is fixed-width and copies straight into a pre-sized block. They are
/// separate emitters, and the String fixture exercises only one of them — which
/// is exactly how the fixed-width half stayed at zero while `zip` looked tested.
#[test]
fn zip_takes_a_native_path_for_both_element_shapes() {
    let strings = code_for_fixture("zip-string-native-rt", CodeTarget::LinuxX86_64);
    assert!(
        !fast_path_labels(strings, "zips_").is_empty(),
        "`#collections_zip$String$String` must lower through the native emitter; \
         no `zips_*` label was emitted"
    );

    const FIXED: &str = "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  LET a AS List OF Integer = [1, 2, 3]
  LET b AS List OF Integer = [10, 20]
  LET z = collections::zip(a, b)
  LET n AS Integer = len(z)
  io::print(toString(n))
  RETURN 0
END FUNC
";
    let fixed = code_for_src_cached(FIXED, CodeTarget::LinuxX86_64, Console);
    let labels = fast_path_labels(fixed, "zip");
    assert!(
        !labels.is_empty(),
        "`#collections_zip$Integer$Integer` must lower through the fixed-width \
         native emitter; the program emitted no `zip*` label, so the interpreted \
         .mfb body ran"
    );
}

/// Every native `zip` picks a bound before its copy loop, and the loop follows.
///
/// `zip` truncates to the shorter list — that is its contract, and the fixture
/// exercises equal, a-short and b-short. The bound is selected by a branch that
/// lands on `zips_n_done`; a loop that ran without one would read past the end
/// of the shorter list, which is an out-of-bounds read of arena memory rather
/// than a wrong length.
#[test]
fn every_native_zip_selects_a_bound_before_its_copy_loop() {
    let plan = code_for_fixture("zip-string-native-rt", CodeTarget::LinuxX86_64);
    let main = code_function(plan, "main");
    let stream = Stream::of(main);
    let labels = stream.labels();
    let mut checked = 0;
    for (at, name) in &labels {
        if !name.starts_with("zips_n_done") {
            continue;
        }
        checked += 1;
        assert!(
            stream.branches_to(name),
            "`{name}` closes the shorter-length selection and must be branched to"
        );
        let loop_after = labels
            .iter()
            .any(|(n, l)| n > at && l.starts_with("zips_loop"));
        assert!(
            loop_after,
            "`{name}` must be followed by the copy loop it bounds"
        );
    }
    assert!(
        checked >= 3,
        "the fixture zips equal-length, a-short and b-short pairs, so at least \
         three bounded loops must be emitted; found {checked}"
    );
}

// --- func_find_last_index.rs ----------------------------------------------

/// The `groupBy` fast path also fires for an Integer VALUE.
///
/// The Integer-key/Integer-value instantiation shares the inline hash table with
/// the String-value one but reads and buckets its elements differently -- a
/// fixed-width element needs no per-iteration materialize-and-free. Covering
/// only the String fixture leaves that half unmeasured while `groupBy` reads as
/// tested.
#[test]
fn group_by_takes_the_native_path_for_a_fixed_width_value() {
    const SRC: &str = "\
IMPORT collections
IMPORT io

FUNC parity(n AS Integer) AS Integer
  RETURN n MOD 2
END FUNC
FUNC same(n AS Integer) AS Integer
  RETURN n
END FUNC

FUNC main() AS Integer
  LET xs AS List OF Integer = [1, 2, 3, 4, 5]
  LET g = collections::groupBy(xs, parity, same)
  LET n AS Integer = len(collections::keys(g))
  io::print(toString(n))
  RETURN 0
END FUNC
";
    let plan = code_for_src_cached(SRC, CodeTarget::LinuxX86_64, Console);
    assert!(
        !fast_path_labels(plan, "gb_").is_empty(),
        "`#collections_groupBy$Integer$Integer$Integer` must lower through the \
         native emitter"
    );
}

/// `findLastIndex` fires natively and its bounds check is a real branch.
///
/// The three-argument form takes a start index, and
/// `func_collection_findLastIndex_out_of_range` proves the runtime rejects a
/// start past the end. What that fixture cannot show is that the rejection is
/// reachable at all in the NATIVE body: the interpreted `.mfb` body would raise
/// the same error, so the fixture passes either way.
#[test]
fn find_last_index_emits_a_reachable_bounds_rejection() {
    let plan = code_for_fixture(
        "func_collection_findLastIndex_out_of_range",
        CodeTarget::LinuxX86_64,
    );
    let bounds = fast_path_labels(plan, "findlast_bounds");
    assert!(
        !bounds.is_empty(),
        "collections::findLastIndex must lower through the native emitter and \
         emit its bounds rejection; no `findlast_bounds*` label was emitted"
    );
    let reached = plan
        .functions
        .iter()
        .filter(|f| bounds.iter().any(|b| Stream::of(f).branches_to(b)))
        .count();
    assert!(
        reached > 0,
        "the `findlast_bounds*` rejection must be branched to; an unreachable \
         one accepts every start index and scans out of range"
    );
}

/// Both `findLastIndex` forms lower natively.
///
/// The two-argument form scans from the end; the three-argument form starts at a
/// caller-supplied index and has to validate it. They are different arms, and
/// the out-of-range fixture only reaches the second -- so a regression in the
/// plain form would be invisible to it.
#[test]
fn both_find_last_index_forms_take_the_native_path() {
    for fixture in [
        "findlast-native-rt",
        "func_collection_findLastIndex_not_found",
    ] {
        let plan = code_for_fixture(fixture, CodeTarget::LinuxX86_64);
        assert!(
            !fast_path_labels(plan, "findlast_").is_empty(),
            "{fixture}: collections::findLastIndex must lower through the native \
             emitter; no `findlast_*` label was emitted"
        );
    }
}

/// The three fast paths are dispatched by TARGET NAME, and each declines every
/// name but its own.
///
/// They are registered as `MfbFastPath`s and handed whatever runtime target the
/// dispatcher is lowering; the `strip_prefix` guard at the top of each is what
/// keeps `collections::sort` from being lowered as a `groupBy`. Nothing in a
/// real program can produce that pairing -- the dispatcher only offers a fast
/// path its own member -- so the guard is unreachable from any source program
/// and is exactly the kind of arm that rots.
#[test]
fn each_fast_path_declines_every_target_but_its_own() {
    type FastPath = fn(&mut CodeBuilder, &str, &[NirValue]) -> Result<Option<ValueResult>, String>;
    let paths: [(&str, &str, FastPath); 3] = [
        (
            "groupBy",
            "#collections_groupBy$Integer$Integer$Integer",
            collections::func_group_by::group_by_fast_path,
        ),
        (
            "zip",
            "#collections_zip$Integer$Integer",
            collections::func_zip::zip_fast_path,
        ),
        (
            "findLastIndex",
            "#collections_findLastIndex$String",
            collections::func_find_last_index::find_last_index_fast_path,
        ),
    ];
    let platform = TestPlatform;
    for (name, own, path) in paths {
        for (other, _, _) in paths {
            if other == name {
                continue;
            }
            let harness = BuilderHarness::default();
            let mut builder = harness.builder("_mfb_test", &platform);
            let target = format!("#collections_{other}$Integer");
            let declined = path(&mut builder, &target, &[]).map(|r| r.is_none());
            assert_eq!(
                declined,
                Ok(true),
                "the {name} fast path must decline `{target}`; lowering another \
                 member's call as a {name} would miscompile it"
            );
        }
        // ...and its own target with the wrong arity, which the arity guard is
        // there for.
        let harness = BuilderHarness::default();
        let mut builder = harness.builder("_mfb_test", &platform);
        let declined = path(&mut builder, own, &[]).map(|r| r.is_none());
        assert_eq!(
            declined,
            Ok(true),
            "the {name} fast path must decline its own target `{own}` with no \
             arguments rather than indexing past the end of the argument list"
        );
    }
}

/// `zip` copies each fixed-width element at ITS OWN width.
///
/// The fixed-width path selects a load by element size -- 1, 4 or 8 bytes -- and
/// a wrong choice reads neighbouring elements into the pair. Only an
/// 8-byte-element program exercises the 8-byte arm, so a suite that zips
/// Integers alone leaves the Byte and 4-byte arms unmeasured, and a regression
/// in them shows up as silently wrong data rather than a crash.
#[test]
fn zip_loads_each_fixed_width_element_at_its_own_size() {
    const SRC: &str = "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  LET ints AS List OF Integer = [1, 2, 3]
  LET bytes AS List OF Byte = [toByte(1), toByte(2)]
  LET flags AS List OF Boolean = [TRUE, FALSE]
  LET reals AS List OF Float = [1.5, 2.5]
  LET zi = collections::zip(ints, ints)
  LET zb = collections::zip(bytes, bytes)
  LET zf = collections::zip(flags, flags)
  LET zr = collections::zip(reals, reals)
  LET n AS Integer = len(zi) + len(zb) + len(zf) + len(zr)
  io::print(toString(n))
  RETURN 0
END FUNC
";
    let plan = code_for_src_cached(SRC, CodeTarget::LinuxX86_64, Console);
    let main = code_function(plan, "main");
    let widths: Vec<CodeOp> = main
        .instructions
        .iter()
        .map(|i| i.op)
        .filter(|op| matches!(op, CodeOp::LdrU8 | CodeOp::LdrU32 | CodeOp::LdrU64))
        .collect();
    for want in [CodeOp::LdrU8, CodeOp::LdrU64] {
        assert!(
            widths.contains(&want),
            "zipping a mix of element widths must emit a {want:?}; the emitter \
             loaded only {:?}",
            widths
        );
    }
    assert!(
        !fast_path_labels(plan, "zip").is_empty(),
        "every one of these zips must take the native path"
    );
}
