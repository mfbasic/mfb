//! A record field named `result` is an ordinary field.
//!
//! `Builder::value_is_runtime_managed` still classed every `.result` member read
//! as the removed `t.result` thread outcome (TYPE_THREAD_RESULT_REMOVED refuses
//! that spelling for a `Thread`), so `LET s = b.result` bound the field's owning
//! copy without owning it: no scope-drop free, one leaked block per evaluation.
//! Measured with `mfb build --debug` on the loop below: 603 allocations and 403
//! frees (`live_bytes 12800`) for a field named `result`, 603 and 603 for the
//! same program with the field renamed `other`.
//!
//! The contract asserted: the two spellings lower to the same calls, in order.

use super::lowered::calls_on;
use crate::testutil::CodeTarget;

fn program(field: &str) -> String {
    format!(
        "IMPORT io

TYPE Box
  {field} AS List OF Integer
END TYPE

FUNC main() AS Integer
  MUT total AS Integer = 0
  MUT b AS Box = Box[{field} := [1, 2, 3]]
  FOR i = 1 TO 200
    LET s AS List OF Integer = b.{field}
    b = Box[{field} := [i, i, i]]
    total = total + len(s)
  NEXT
  io::print(toString(total))
  RETURN 0
END FUNC
"
    )
}

#[test]
fn a_field_named_result_is_owned_and_freed_like_any_other() {
    let (result, other) = (program("result"), program("other"));
    for target in CodeTarget::ALL {
        assert_eq!(
            calls_on(&result, "main", target),
            calls_on(&other, "main", target),
            "{}: `b.result` lowers differently from `b.other`",
            target.name()
        );
    }
}
