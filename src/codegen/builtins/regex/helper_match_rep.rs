//! `__regex_matchRep` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_matchRep(rep AS __regex_Repeat, count AS Integer, pos AS Integer, caps AS List OF Integer, c AS __regex_Cont, ctx AS __regex_Ctx, depth AS Integer) AS __regex_Result
  LET canMore AS Boolean = (rep.hi < 0) OR (count < rep.hi)
  LET mustMore AS Boolean = count < rep.lo
  ' bug-315: a greedy repeat over a SIMPLE one-scalar child is consumed with a
  ' loop rather than one native frame per iteration. The recursive form grew stack
  ' depth proportional to the number of repetitions, so `^a*$` crashed with
  ' SIGSEGV between 800 and 1000 scalars -- an uncatchable process death on
  ' ordinary paragraph-length input.
  '
  ' Semantics are unchanged: consume as far as the bound allows, then give back
  ' one scalar at a time, trying the continuation at each length from longest to
  ' shortest. That is exactly the order the recursion explored, so the FIRST match
  ' found is the same one -- greedy leftmost, longest-first.
  IF rep.greedy AND __regex_isSimpleNode(rep.child) THEN
    MUT p AS Integer = pos
    MUT k AS Integer = count
    ' Consume greedily up to `hi` (or unbounded when hi < 0).
    MUT more AS Boolean = (rep.hi < 0) OR (k < rep.hi)
    WHILE more
      IF __regex_simpleMatchAt(rep.child, p, ctx) = FALSE THEN
        more = FALSE
      ELSE
        p = p + 1
        k = k + 1
        more = (rep.hi < 0) OR (k < rep.hi)
      END IF
    END WHILE
    ' Give back one at a time until the continuation matches or we fall below the
    ' minimum. `k = count` is the floor: nothing was consumed by this call.
    MUT backtracking AS Boolean = TRUE
    WHILE backtracking
      IF k >= rep.lo THEN
        LET rr AS __regex_Result = __regex_matchCont(c, p, caps, ctx, depth + 1)
        IF rr.ok THEN
          RETURN rr
        END IF
      END IF
      IF k <= count THEN
        backtracking = FALSE
      ELSE
        p = p - 1
        k = k - 1
      END IF
    END WHILE
    RETURN __regex_fail()
  END IF
  IF rep.greedy THEN
    IF canMore THEN
      LET r AS __regex_Result = __regex_matchNode(rep.child, pos, caps, __regex_ContRep[rep, count + 1, pos, c], ctx, depth + 1)
      IF r.ok THEN
        RETURN r
      END IF
    END IF
    IF mustMore THEN
      RETURN __regex_fail()
    END IF
    RETURN __regex_matchCont(c, pos, caps, ctx, depth + 1)
  END IF
  IF mustMore = FALSE THEN
    LET r2 AS __regex_Result = __regex_matchCont(c, pos, caps, ctx, depth + 1)
    IF r2.ok THEN
      RETURN r2
    END IF
  END IF
  IF canMore THEN
    RETURN __regex_matchNode(rep.child, pos, caps, __regex_ContRep[rep, count + 1, pos, c], ctx, depth + 1)
  END IF
  RETURN __regex_fail()
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_matchRep", BODY));
}
