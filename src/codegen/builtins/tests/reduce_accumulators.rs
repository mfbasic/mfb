//! `collections::reduce` and `reduceRight` own and free a block accumulator.
//!
//! A reducer whose accumulator is a block — a record, a list, a `String` — returns a
//! FRESH block each step. The loop owns the superseded accumulator and must free it
//! when the reducer hands back a different one, free the item when it is not adopted,
//! never free the caller's SEED (the first accumulator belongs to the caller's
//! argument), copy the seed when the loop never replaced it, and — when a reducer
//! FAILS — free the item and the accumulator it still owns before the error leaves.
//! Each is a guarded free of memory the arena will reuse, so a wrong guard is a double
//! free or a leak that no output reveals.
//!
//! The program below reduces with every accumulator shape: a flat record (fresh result
//! per step, handed back unchanged, empty input so the seed is the result), a list, a
//! record holding a `String`, a `String` accumulator over `String` items, `reduceRight`,
//! and a failing reducer for the record, list and string accumulators under an inline
//! `TRAP`. The guards are emitted as labelled branches, so their presence is the
//! assertion: a build that dropped one has no label.

use crate::codegen::engine::tests::test_support::Stream;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, code_function, CodeTarget};

const REDUCERS: &str = r#"IMPORT io
IMPORT collections

TYPE AccFlat
  n AS Integer
  m AS Integer
END TYPE

TYPE AccStr
  n AS Integer
  label AS String
END TYPE

FUNC stepFlat(a AS AccFlat, x AS Integer) AS AccFlat
  RETURN AccFlat[a.n + x, a.m + 1]
END FUNC

FUNC stepKeep(a AS AccFlat, x AS Integer) AS AccFlat
  RETURN a
END FUNC

FUNC stepFail(a AS AccFlat, x AS Integer) AS AccFlat
  IF x < 0 THEN
    FAIL error(77010001, "negative")
  END IF
  RETURN AccFlat[a.n + x, a.m]
END FUNC

FUNC stepList(acc AS List OF Integer, x AS Integer) AS List OF Integer
  RETURN collections::append(acc, x)
END FUNC

FUNC stepListFail(acc AS List OF Integer, x AS Integer) AS List OF Integer
  IF x < 0 THEN
    FAIL error(77010002, "negative")
  END IF
  RETURN collections::append(acc, x)
END FUNC

FUNC stepStr(a AS AccStr, x AS Integer) AS AccStr
  RETURN AccStr[a.n + x, a.label & "x"]
END FUNC

FUNC stepStrItem(acc AS String, s AS String) AS String
  RETURN acc & s
END FUNC

FUNC stepStrFail(acc AS String, s AS String) AS String
  IF s = "!" THEN
    FAIL error(77010003, "bang")
  END IF
  RETURN acc & s
END FUNC

FUNC main() AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  LET bad AS List OF Integer = [1, -1, 2]
  LET none AS List OF Integer = []
  LET empty AS List OF Integer = []
  LET a = collections::reduce(xs, AccFlat[0, 0], stepFlat)
  LET b = collections::reduceRight(xs, AccFlat[0, 0], stepFlat)
  LET c = collections::reduce(none, AccFlat[5, 5], stepFlat)
  LET d = collections::reduce(xs, AccFlat[0, 0], stepKeep)
  LET e = collections::reduce(xs, empty, stepList)
  LET f = collections::reduce(xs, AccStr[0, ""], stepStr)
  LET g = collections::reduce(["a", "b"], "", stepStrItem)
  LET h = collections::reduce(bad, AccFlat[0, 0], stepFail) TRAP(err)
    RECOVER AccFlat[-1, -1]
  END TRAP
  LET i = collections::reduce(bad, empty, stepListFail) TRAP(err)
    RECOVER [0]
  END TRAP
  LET j = collections::reduce(["a", "!", "b"], "", stepStrFail) TRAP(err)
    RECOVER "failed"
  END TRAP
  io::print(toString(a.n + b.n + c.n + d.n + len(e) + f.n + len(g) + h.n + len(i) + len(j)))
  RETURN 0
END FUNC
"#;

fn main_labels(target: CodeTarget) -> Vec<String> {
    let plan = code_for_src_cached(REDUCERS, target, Console);
    Stream::of(code_function(plan, "main"))
        .labels()
        .into_iter()
        .map(|(_, name)| name)
        .collect()
}

fn count(labels: &[String], prefix: &str) -> usize {
    labels
        .iter()
        .filter(|name| name.starts_with(prefix))
        .count()
}

/// Every accumulator shape lowers on every backend.
#[test]
fn every_accumulator_shape_lowers_on_every_backend() {
    for target in CodeTarget::ALL {
        let result = crate::testutil::try_code_for_src(REDUCERS, target, Console);
        assert!(result.is_ok(), "{}: {:?}", target.name(), result.err());
    }
}

/// The ownership guards each exist, once per reduction that needs them.
#[test]
fn a_block_accumulator_is_freed_only_when_the_loop_owns_it() {
    let labels = main_labels(CodeTarget::LinuxX86_64);
    // Ten reductions, so ten loops.
    assert_eq!(count(&labels, "reduce_call_loop"), 10, "{labels:?}");
    // The superseded block accumulator is freed behind an ownership test: the record,
    // list and record-with-string accumulators, in both directions of the fold.
    assert!(
        count(&labels, "reduce_acc_block_kept") >= 4,
        "a block accumulator must be freed only behind the owned/replaced test"
    );
    // A String accumulator has its own guard.
    assert!(count(&labels, "reduce_acc_kept") >= 1);
    // The caller's seed is copied when the loop never replaced it.
    assert!(
        count(&labels, "reduce_result_fresh") >= 1,
        "a result that is still the seed must be copied, or the seed has two owners"
    );
    // A failing reducer frees the accumulator it still owns before the error leaves.
    assert!(
        count(&labels, "reduce_fail_acc_not_owned") >= 3,
        "each failing reduction must test who owns the accumulator before freeing it"
    );
}
