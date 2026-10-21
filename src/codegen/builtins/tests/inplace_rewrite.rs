//! The same-length rewrites of a uniquely-owned list: `replace` and `transform`
//! on a `List OF String` (variable-width elements).
//!
//! A fixed-width list rewrites each slot where it lies. A String list cannot: a
//! longer replacement does not fit the slot's payload, so the in-place arm first
//! COUNTS the matches and reserves tail room for all of them in one place
//! (`inplace_replace_count_*`, `inplace_tail_fits_*`), so that the only
//! allocation - and the only `ErrOutOfMemory` - happens before the first write.
//! `transform` has a stronger obligation: its callback can fail on any element,
//! and `x` must be untouched when it does, so the results are parked first and a
//! failure frees what was parked (`inplace_transform_unwind_*`) before routing
//! the error.

use super::stems::{has, stems};

const REPLACE_STRING: &str = "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT xs AS List OF String = [\"a\", \"bb\", \"a\"]
  xs = collections::replace(xs, \"a\", \"a longer replacement\")
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
";

const REPLACE_INTEGER: &str = "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT xs AS List OF Integer = [1, 2, 1]
  xs = collections::replace(xs, 1, 9)
  io::print(toString(len(xs)))
  RETURN 0
END FUNC
";

const TRANSFORM_STRING: &str = "\
IMPORT collections
IMPORT io

FUNC up(s AS String) AS String
  IF len(s) = 0 THEN FAIL error(1, \"empty\")
  RETURN s & \"!\"
END FUNC

FUNC main() AS Integer
  MUT ys AS List OF String = [\"a\", \"b\"]
  ys = collections::transform(ys, up)
  io::print(toString(len(ys)))
  RETURN 0
END FUNC
";

const TRANSFORM_INTEGER: &str = "\
IMPORT collections
IMPORT io

FUNC twice(n AS Integer) AS Integer
  IF n < 0 THEN FAIL error(1, \"negative\")
  RETURN n * 2
END FUNC

FUNC main() AS Integer
  MUT ys AS List OF Integer = [1, 2]
  ys = collections::transform(ys, twice)
  io::print(toString(len(ys)))
  RETURN 0
END FUNC
";

/// Only a variable-width list needs the count-and-reserve pass.
#[test]
fn replace_reserves_tail_room_only_for_variable_width_elements() {
    let string = stems(REPLACE_STRING);
    assert!(
        has(&string, "inplace_replace_count"),
        "a String list's longer replacement needs the match-count pass: {string:?}"
    );
    assert!(
        has(&string, "inplace_tail_fits"),
        "the counted bytes must be reserved up front, repacking only when the \
         tail is short: {string:?}"
    );
    let integer = stems(REPLACE_INTEGER);
    assert!(
        has(&integer, "inplace_replace_loop"),
        "an Integer replace still rewrites in place: {integer:?}"
    );
    assert!(
        !has(&integer, "inplace_replace_count"),
        "an 8-byte slot never grows, so counting matches is wasted work: {integer:?}"
    );
}

/// A String-returning callback that can fail parks its results and unwinds them
/// on failure; an Integer one has nothing to free.
#[test]
fn transform_unwinds_parked_string_results_when_the_callback_fails() {
    let string = stems(TRANSFORM_STRING);
    for family in [
        "inplace_transform_unwind",
        "inplace_transform_size",
        "inplace_transform_item_kept",
    ] {
        assert!(has(&string, family), "missing `{family}`: {string:?}");
    }
    let integer = stems(TRANSFORM_INTEGER);
    assert!(
        has(&integer, "inplace_transform_loop"),
        "an Integer transform still runs in place: {integer:?}"
    );
    assert!(
        !has(&integer, "inplace_transform_unwind"),
        "Integer results own no memory, so nothing needs unwinding: {integer:?}"
    );
}
