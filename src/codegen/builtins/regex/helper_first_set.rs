//! `__regex_firstSetKids` — shared private helper for the `regex` package.
//!
//! bug-773 (Fix 4): the first-scalar set. `__regex_flatten` records one literal code
//! point a match must begin with (`firstCp`); any other pattern — an alternation, a
//! class, a repeat — ran the whole engine at every start position, although most of
//! them cannot begin a match (`cat|dog|bird|fish|owl` at a `z`). This computes, from
//! the flat program's tables, the set of scalars a match can begin with, and the exact
//! number of node visits the engine spends at a start position whose scalar is outside
//! it, so `__regex_run` can skip that position and charge the budgets exactly what
//! running it would have cost. `__regex_compile` attaches the result to the program.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' bug-773: the first-scalar set of a program that has no fixed first literal.
'
' A start position is REJECTED when its scalar cannot be the first scalar any match
' consumes. There no consuming leaf the engine can reach before consuming anything
' accepts the scalar, so every one of them fails, the search at that position fails,
' and it fails after a node-visit count D that depends only on the program: every
' branch it explores consumes nothing and reaches no anchor (a program that could
' reach an anchor, an Any or a case-folded non-ASCII literal there gets no set). The
' search loop in __regex_run skips such a position and charges both step budgets D,
' so a skip is exactly the failed attempt it replaces -- the same match, the same
' budget error at the same input.
'
' D comes from V(op, K) = f + e * K, the visits spent matching `op` at a rejected
' position when running its continuation costs K: `f` is the visits inside `op` and
' `e` the number of times it succeeds without consuming (each runs the continuation).
' The rules mirror __regex_run's visit order op kind by op kind. A child op always
' has a larger index than its parent (__regex_flatten reserves it later), so one pass
' from the last op to the first computes every f and e; a program that breaks that
' order gets no set rather than a wrong one. The root's continuation is the match
' itself, so a root with e > 0 is nullable -- it can match empty anywhere -- and gets
' no set; otherwise D = f(root). Counts saturate at `sat`, and a saturated D gets no
' set.
'
' The result is `kids` with the set appended -- 128 ASCII membership integers, one
' "any non-ASCII scalar may begin a match" integer, then D -- which __regex_compile
' stores as the program's `kids`, with `firstCp` = -2 - (the set's offset). The matcher
' reads `kids` only through Concat/Alt operands, never past them. An empty result
' means no set: the program keeps scanning every start position.
FUNC __regex_firstSetKids(kinds AS List OF Integer, opA AS List OF Integer, opB AS List OF Integer, opC AS List OF Integer, kids AS List OF Integer, leaves AS List OF __regex_Leaf, root AS Integer) AS List OF Integer
  LET ops AS Integer = len(kinds)
  ' A rejected position's pending choice points are bounded by the op count, so under
  ' this bound the pending cap cannot fire there.
  IF ops > __REGEX_PENDING_LIMIT THEN
    RETURN []
  END IF
  LET sat AS Integer = 1000000000000
  ' f(o) at 2o and e(o) at 2o + 1.
  MUT fe AS List OF Integer = collections::repeat([], 0, 2 * ops)
  MUT o AS Integer = ops - 1
  WHILE o >= 0
    LET kind AS Integer = collections::get(kinds, o)
    MUT f AS Integer = 1
    MUT e AS Integer = 0
    IF kind = 1 THEN
      ' A consuming leaf fails at a rejected position.
      e = 0
    ELSEIF kind = 2 THEN
      ' An anchor may pass or fail; reaching one withdraws the set below.
      e = 1
    ELSEIF kind = 3 THEN
      ' Concat: 1 + V(p1, V(p2, ... V(pn, K))), folded from the last part.
      LET firstPart AS Integer = collections::get(opA, o)
      MUT accF AS Integer = 0
      MUT accE AS Integer = 1
      MUT k AS Integer = collections::get(opB, o) - 1
      WHILE k >= 0
        LET part AS Integer = collections::get(kids, firstPart + k)
        IF part <= o THEN
          RETURN []
        END IF
        LET pf AS Integer = collections::get(fe, 2 * part)
        LET pe AS Integer = collections::get(fe, 2 * part + 1)
        MUT viaF AS Integer = 0
        IF pe > 0 AND accF > 0 THEN
          IF accF > sat / pe THEN
            viaF = sat
          ELSE
            viaF = pe * accF
          END IF
        END IF
        accF = pf + viaF
        IF accF > sat THEN
          accF = sat
        END IF
        IF pe = 0 OR accE = 0 THEN
          accE = 0
        ELSEIF accE > sat / pe THEN
          accE = sat
        ELSE
          accE = pe * accE
        END IF
        k = k - 1
      END WHILE
      f = 1 + accF
      e = accE
    ELSEIF kind = 4 THEN
      ' Alt: 1 + the sum of V(option, K) -- every option runs, in order, and fails.
      LET firstOpt AS Integer = collections::get(opA, o)
      MUT k AS Integer = 0
      WHILE k < collections::get(opB, o)
        LET opt AS Integer = collections::get(kids, firstOpt + k)
        IF opt <= o THEN
          RETURN []
        END IF
        f = f + collections::get(fe, 2 * opt)
        e = e + collections::get(fe, 2 * opt + 1)
        IF f > sat THEN
          f = sat
        END IF
        IF e > sat THEN
          e = sat
        END IF
        k = k + 1
      END WHILE
    ELSEIF kind = 5 OR kind = 6 THEN
      ' Repeat, at iteration count 0. An iteration that consumes nothing ends the
      ' repeat and runs the continuation (the empty-iteration guard), so a child's
      ' empty success costs K whatever the minimum.
      LET child AS Integer = collections::get(opA, o)
      IF child <= o THEN
        RETURN []
      END IF
      LET lo AS Integer = collections::get(opB, o)
      LET hi AS Integer = collections::get(opC, o)
      LET canMore AS Boolean = hi <> 0
      LET mustMore AS Boolean = lo > 0
      LET cf AS Integer = collections::get(fe, 2 * child)
      LET ce AS Integer = collections::get(fe, 2 * child + 1)
      IF kind = 5 AND collections::get(kinds, child) = 1 THEN
        ' The greedy one-scalar loop consumes nothing and visits no node; with no
        ' minimum it runs the continuation, and its give-back choice has nothing to give.
        IF NOT mustMore THEN
          e = 1
        END IF
      ELSEIF kind = 5 THEN
        IF canMore THEN
          f = 1 + cf
          e = ce
          IF NOT mustMore THEN
            e = ce + 1
          END IF
        ELSEIF NOT mustMore THEN
          e = 1
        END IF
      ELSE
        IF NOT mustMore THEN
          e = 1
          IF canMore THEN
            f = 1 + cf
            e = 1 + ce
          END IF
        ELSEIF canMore THEN
          f = 1 + cf
          e = ce
        END IF
      END IF
      IF f > sat THEN
        f = sat
      END IF
      IF e > sat THEN
        e = sat
      END IF
    ELSEIF kind = 7 THEN
      ' Group: 1 + V(child, K); closing the capture is not a visit.
      LET child AS Integer = collections::get(opA, o)
      IF child <= o THEN
        RETURN []
      END IF
      f = 1 + collections::get(fe, 2 * child)
      e = collections::get(fe, 2 * child + 1)
      IF f > sat THEN
        f = sat
      END IF
    ELSE
      RETURN []
    END IF
    fe = collections::set(fe, 2 * o, f)
    fe = collections::set(fe, 2 * o + 1, e)
    o = o - 1
  END WHILE
  LET rejectCost AS Integer = collections::get(fe, 2 * root)
  IF collections::get(fe, 2 * root + 1) > 0 OR rejectCost >= sat THEN
    RETURN []
  END IF
  ' The ops a rejected position reaches -- parents first, since a child's index is
  ' larger; an op is marked reached by f(o) = -1, f being spent -- and the scalars
  ' their consuming leaves accept, written straight into the table after `kids`.
  LET setAt AS Integer = len(kids)
  MUT table AS List OF Integer = collections::repeat(kids, 0, 130, setAt)
  table = collections::set(table, setAt + 129, rejectCost)
  fe = collections::set(fe, 2 * root, -1)
  o = root
  WHILE o < ops
    IF collections::get(fe, 2 * o) < 0 THEN
      LET kind AS Integer = collections::get(kinds, o)
      IF kind = 1 THEN
        LET leaf AS __regex_Leaf = collections::get(leaves, collections::get(opA, o))
        MATCH leaf
          CASE __regex_Lit(litNode)
            LET cp AS Integer = litNode.cp
            IF litNode.fold THEN
              ' An ASCII scalar folds to its lower case, so it matches a folded ASCII
              ' literal only in either case; a non-ASCII scalar may fold to it (U+212A
              ' KELVIN SIGN to k), and a folded non-ASCII literal may match ASCII.
              IF cp > 127 THEN
                RETURN []
              END IF
              table = collections::set(table, setAt + cp, 1)
              IF cp >= 65 AND cp <= 90 THEN
                table = collections::set(table, setAt + cp + 32, 1)
              END IF
              IF cp >= 97 AND cp <= 122 THEN
                table = collections::set(table, setAt + cp - 32, 1)
              END IF
              table = collections::set(table, setAt + 128, 1)
            ELSEIF cp > 127 THEN
              table = collections::set(table, setAt + 128, 1)
            ELSE
              table = collections::set(table, setAt + cp, 1)
            END IF
          CASE __regex_Class(clsNode)
            ' The ASCII table already has folding baked in; `neg` is applied here as
            ' __regex_classMatch applies it. Past ASCII, only a plain class of ASCII
            ' ranges and singles provably rejects every scalar.
            MUT c AS Integer = 0
            WHILE c < 128
              IF collections::get(clsNode.ascii, c) <> clsNode.neg THEN
                table = collections::set(table, setAt + c, 1)
              END IF
              c = c + 1
            END WHILE
            IF clsNode.neg OR clsNode.fold THEN
              table = collections::set(table, setAt + 128, 1)
            ELSE
              FOR EACH item IN clsNode.items
                MATCH item
                  CASE __regex_Range(rng)
                    IF rng.hi > "\u{7F}" THEN
                      table = collections::set(table, setAt + 128, 1)
                    END IF
                  CASE __regex_Single(sng)
                    IF sng.ch > "\u{7F}" THEN
                      table = collections::set(table, setAt + 128, 1)
                    END IF
                  CASE ELSE
                    table = collections::set(table, setAt + 128, 1)
                END MATCH
              NEXT
            END IF
          CASE ELSE
            ' Any accepts nearly every scalar; an unknown leaf, anything.
            RETURN []
        END MATCH
      ELSEIF kind = 2 THEN
        RETURN []
      ELSEIF kind = 3 THEN
        LET firstPart AS Integer = collections::get(opA, o)
        LET partCount AS Integer = collections::get(opB, o)
        MUT k AS Integer = 0
        MUT more AS Boolean = partCount > 0
        WHILE more
          LET part AS Integer = collections::get(kids, firstPart + k)
          fe = collections::set(fe, 2 * part, -1)
          k = k + 1
          more = k < partCount AND collections::get(fe, 2 * part + 1) > 0
        END WHILE
      ELSEIF kind = 4 THEN
        LET firstOpt AS Integer = collections::get(opA, o)
        MUT k AS Integer = 0
        WHILE k < collections::get(opB, o)
          fe = collections::set(fe, 2 * collections::get(kids, firstOpt + k), -1)
          k = k + 1
        END WHILE
      ELSEIF kind = 5 OR kind = 6 THEN
        IF collections::get(opC, o) <> 0 THEN
          fe = collections::set(fe, 2 * collections::get(opA, o), -1)
        END IF
      ELSE
        fe = collections::set(fe, 2 * collections::get(opA, o), -1)
      END IF
    END IF
    o = o + 1
  END WHILE
  RETURN table
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_firstSetKids", BODY));
}
