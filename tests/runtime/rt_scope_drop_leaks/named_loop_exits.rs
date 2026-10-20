//! plan-180-C Phase 4: a named `EXIT FOR` / `CONTINUE FOR` that leaves an inner loop
//! owning a `RES` and a `List` temporary runs that loop's cleanups on every outer
//! iteration, so the outer loop runs at constant RSS. `assert_flat` and the `{N}`
//! substitution live in the root, `rt_scope_drop_leaks.rs`.

use super::*;

/// `CONTINUE FOR i` leaves the inner `FOR j` once per outer iteration, past the
/// inner loop's `RES` and its `List` temporary, so each iteration must free both.
#[cfg(unix)]
const NAMED_CONTINUE_PAST_INNER_RESOURCE: &str = r#"IMPORT io
IMPORT fs

FUNC main AS Integer
  MUT hits AS Integer = 0
  FOR i = 1 TO {N}
    FOR j = 1 TO 2
      RES handle AS fs::File = fs::openFile("/dev/null", "r") TRAP(e)
        PROPAGATE
      END TRAP
      LET xs AS List OF Integer = [i, j]
      IF j = 2 THEN
        CONTINUE FOR i
      END IF
      hits = hits + len(xs)
    NEXT
  NEXT
  io::print(toString(hits))
  RETURN 0
END FUNC
"#;

/// `EXIT FOR i` leaves the inner `FOR j` past its `RES` and `List` temporary. The
/// exit fires on the last outer iteration, so the outer loop still runs `{N}` times.
#[cfg(unix)]
const NAMED_EXIT_PAST_INNER_RESOURCE: &str = r#"IMPORT io
IMPORT fs

FUNC main AS Integer
  MUT hits AS Integer = 0
  FOR i = 1 TO {N}
    FOR j = 1 TO 2
      RES handle AS fs::File = fs::openFile("/dev/null", "r") TRAP(e)
        PROPAGATE
      END TRAP
      LET xs AS List OF Integer = [i, j]
      IF i = {N} AND j = 2 THEN
        EXIT FOR i
      END IF
      hits = hits + len(xs)
    NEXT
  NEXT
  io::print(toString(hits))
  RETURN 0
END FUNC
"#;

#[cfg(unix)]
#[test]
fn a_named_continue_past_an_inner_resource_runs_at_constant_rss() {
    assert_flat(
        "named_continue_inner_res",
        NAMED_CONTINUE_PAST_INNER_RESOURCE,
        200_000,
        400_000,
    );
}

#[cfg(unix)]
#[test]
fn a_named_exit_past_an_inner_resource_runs_at_constant_rss() {
    assert_flat(
        "named_exit_inner_res",
        NAMED_EXIT_PAST_INNER_RESOURCE,
        200_000,
        400_000,
    );
}
