//! `__regex_run` — the matcher: an explicit-stack backtracker (bug-510, DEC-01).
//!
//! Until bug-510 the matcher was continuation-passing recursion: every node visit,
//! every continuation step and every repeat iteration was a native frame, and the
//! frames were large (`__regex_matchNode` alone held ~300 stack slots). A group
//! repetition cost about ten of them, so the depth guard that kept the process from
//! dying of stack overflow fired at sixty repetitions: `^(ab)*$` raised
//! `ErrInvalidFormat` on a 200-character input, and the hostname pattern
//! `^([a-z0-9-]+\.)+[a-z]{2,}$` failed at 54 labels. Charging the guard "by input
//! position" would not have been sound — a frame is a frame whether or not it
//! consumed a character — so the recursion is gone instead.
//!
//! **Same order, same answers.** The engine explores exactly the tree the recursive
//! one did, in the same order: alternatives left to right, greedy repeats longest
//! first, lazy repeats shortest first, a group's capture closed when its child
//! succeeds, the empty-iteration guard on `ContRep`. What used to be "try the
//! preferred branch; on failure fall through to the next" is now "push the next
//! branch as a choice point; run the preferred one; on failure pop". The pinned
//! corpus in `tests/rt_regex_bounds.rs` is the proof that nothing observable moved.
//!
//! **The stack is a linked list of choice records, not a growable list.** A choice
//! point is a `__regex_Choice` record whose `nxt` is the previous top, so a push is one
//! constructor and a pop is `stack = c.nxt` — the same construct-wrapping-the-previous-
//! head shape the continuations (`__regex_ContSeq[parts, idx + 1, nxt]`) have always
//! used. The first version kept choice points in a flat `List OF Integer` with the
//! continuations, capture snapshots and `Repeat` records in append-only side tables,
//! and died of bug-538: `collections::get` of a recursive-type element aliases the
//! list's storage, and the next growing `append` frees it. A choice record is never
//! appended to anything, so nothing it points into is ever reallocated.
//!
//! **Cost bounds.** Node visits are charged to the per-search step budget as before
//! and, since bug-510, to the per-call budget `__regex_makeCtx` sets (DEC-02). The
//! number of pending choice points is capped by `__REGEX_PENDING_LIMIT`; that is the
//! matcher's memory bound, and it is two thousand times the sixty-odd frames the old
//! depth guard allowed.
//!
//! Registered via `add_helper`; body byte-significant (2-space indent → `.ncode`
//! columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' A pending choice point (`__regex_Choice`): `kind` 1 = the next alternative of an
' Alt (`alt`, `i`), 2 = stop repeating and run the continuation, 3 = one more (lazy)
' iteration of `rep` (`count`), 4 = give back one scalar of a greedy simple repeat
' (`rep`, `i` = scalars consumed so far, `count` = the iteration count before, `p` =
' the position after the scalars consumed). `cont`, `pos`, `caps` are what every kind
' resumes with; `nxt` is the choice below it. Fields a kind does not use carry the
' root node and the current repeat as placeholders.
FUNC __regex_run(root AS __regex_Node, start AS Integer, caps0 AS List OF Integer, ctx AS __regex_Ctx) AS __regex_Result
  MUT stack AS __regex_Choices = __regex_NoChoice[TRUE]
  MUT pending AS Integer = 0
  MUT isNode AS Boolean = TRUE
  MUT node AS __regex_Node = root
  MUT cont AS __regex_Cont = __regex_ContCap[0, __regex_ContDone[TRUE]]
  MUT pos AS Integer = start
  MUT caps AS List OF Integer = caps0
  MUT failing AS Boolean = FALSE
  MUT repPending AS Boolean = FALSE
  MUT repCount AS Integer = 0
  MUT repRec AS __regex_Repeat = __regex_Repeat[root, 0, 0, TRUE]
  WHILE TRUE
    IF failing THEN
      MATCH stack
        CASE __regex_NoChoice(none)
          RETURN __regex_fail()
        CASE __regex_Choice(c)
          stack = c.nxt
          pending = pending - 1
          pos = c.pos
          caps = c.caps
          cont = c.cont
          IF c.kind = 1 THEN
            MATCH c.alt
              CASE __regex_Alt(altNode)
                IF c.i + 1 < len(altNode.opts) THEN
                  stack = __regex_Choice[1, c.alt, c.rep, c.cont, c.pos, c.caps, c.i + 1, 0, 0, stack]
                  pending = pending + 1
                END IF
                node = collections::get(altNode.opts, c.i)
                isNode = TRUE
                failing = FALSE
              CASE ELSE
                RETURN __regex_fail()
            END MATCH
          ELSEIF c.kind = 2 THEN
            isNode = FALSE
            failing = FALSE
          ELSEIF c.kind = 3 THEN
            cont = __regex_ContRep[c.rep, c.count + 1, pos, cont]
            node = c.rep.child
            isNode = TRUE
            failing = FALSE
          ELSE
            ' Give one scalar back and try the continuation there -- while the repeat
            ' still holds more than it started with and no fewer than its minimum. Below
            ' the minimum there is nothing left to try, and the pop above stands.
            IF c.i > c.count AND c.i - 1 >= c.rep.lo THEN
              stack = __regex_Choice[4, c.alt, c.rep, c.cont, c.pos, c.caps, c.i - 1, c.count, c.p - 1, stack]
              pending = pending + 1
              pos = c.p - 1
              isNode = FALSE
              failing = FALSE
            END IF
          END IF
      END MATCH
    ELSEIF isNode THEN
      ' bug-315: every node visit is one step, so counting here bounds the whole
      ' search whatever construct is blowing up. bug-510 (DEC-02): the same visit is
      ' charged to the call-wide budget too, so `findAll`/`replace` cannot spend a
      ' fresh search budget on every match. And the pending choice points are the
      ' matcher's memory, so they are bounded as well.
      __regex_steps = __regex_steps + 1
      __regex_callSteps = __regex_callSteps + 1
      IF __regex_steps > __REGEX_STEP_BUDGET OR __regex_callSteps > __regex_callBudget THEN
        FAIL error(77050003, "regex: pattern too complex for this input (backtracking limit exceeded)")
      END IF
      IF pending > __REGEX_PENDING_LIMIT THEN
        FAIL error(77050003, "regex: pattern too complex for this input (backtracking limit exceeded)")
      END IF
      MATCH node
        CASE __regex_Lit(litNode)
          IF pos >= ctx.n THEN
            failing = TRUE
          ELSEIF __regex_charEq(litNode, collections::get(ctx.cps, pos)) THEN
            pos = pos + 1
            isNode = FALSE
          ELSE
            failing = TRUE
          END IF
        CASE __regex_Any(anyNode)
          IF pos >= ctx.n THEN
            failing = TRUE
          ELSEIF anyNode.dotall OR collections::get(ctx.cps, pos) <> 10 THEN
            pos = pos + 1
            isNode = FALSE
          ELSE
            failing = TRUE
          END IF
        CASE __regex_Class(clsNode)
          IF pos >= ctx.n THEN
            failing = TRUE
          ELSEIF __regex_classMatch(clsNode, pos, ctx) THEN
            pos = pos + 1
            isNode = FALSE
          ELSE
            failing = TRUE
          END IF
        CASE __regex_Anchor(anchorNode)
          IF __regex_anchorMatch(anchorNode, pos, ctx) THEN
            isNode = FALSE
          ELSE
            failing = TRUE
          END IF
        CASE __regex_Concat(seqNode)
          cont = __regex_ContSeq[seqNode.parts, 0, cont]
          isNode = FALSE
        CASE __regex_Alt(altNode)
          IF len(altNode.opts) = 0 THEN
            failing = TRUE
          ELSE
            IF len(altNode.opts) > 1 THEN
              stack = __regex_Choice[1, node, repRec, cont, pos, caps, 1, 0, 0, stack]
              pending = pending + 1
            END IF
            node = collections::get(altNode.opts, 0)
          END IF
        CASE __regex_Repeat(repNode)
          repRec = repNode
          repCount = 0
          repPending = TRUE
        CASE __regex_Group(grpNode)
          caps = __regex_setCap(caps, 2 * grpNode.slot, pos)
          cont = __regex_ContCap[grpNode.slot, cont]
          node = grpNode.child
      END MATCH
    ELSE
      MATCH cont
        CASE __regex_ContDone(doneCont)
          RETURN __regex_Result[TRUE, pos, caps]
        CASE __regex_ContSeq(seqCont)
          IF seqCont.idx >= len(seqCont.parts) THEN
            cont = seqCont.nxt
          ELSE
            node = collections::get(seqCont.parts, seqCont.idx)
            cont = __regex_ContSeq[seqCont.parts, seqCont.idx + 1, seqCont.nxt]
            isNode = TRUE
          END IF
        CASE __regex_ContCap(capCont)
          caps = __regex_setCap(caps, 2 * capCont.slot + 1, pos)
          cont = capCont.nxt
        CASE __regex_ContRep(repCont)
          ' The empty-iteration guard: an iteration that consumed nothing ends the
          ' repeat, or `(a*)*` would never terminate.
          cont = repCont.nxt
          IF pos <> repCont.startPos THEN
            repRec = repCont.rep
            repCount = repCont.count
            repPending = TRUE
          END IF
      END MATCH
    END IF
    IF repPending THEN
      repPending = FALSE
      LET canMore AS Boolean = (repRec.hi < 0) OR (repCount < repRec.hi)
      LET mustMore AS Boolean = repCount < repRec.lo
      IF repRec.greedy AND __regex_isSimpleNode(repRec.child) THEN
        ' bug-315: a greedy repeat over a one-scalar child consumes as far as it can
        ' in a loop, then gives back one scalar at a time (choice kind 4) -- longest
        ' first, exactly the order the recursion explored.
        MUT p AS Integer = pos
        MUT k AS Integer = repCount
        MUT more AS Boolean = canMore
        WHILE more
          IF __regex_simpleMatchAt(repRec.child, p, ctx) = FALSE THEN
            more = FALSE
          ELSE
            p = p + 1
            k = k + 1
            more = (repRec.hi < 0) OR (k < repRec.hi)
          END IF
        END WHILE
        IF k >= repRec.lo THEN
          stack = __regex_Choice[4, root, repRec, cont, pos, caps, k, repCount, p, stack]
          pending = pending + 1
          pos = p
          isNode = FALSE
        ELSE
          failing = TRUE
        END IF
      ELSEIF repRec.greedy THEN
        IF canMore THEN
          IF NOT mustMore THEN
            ' The alternative, should every way of iterating once more fail: stop
            ' here and run the continuation at this position.
            stack = __regex_Choice[2, root, repRec, cont, pos, caps, 0, 0, 0, stack]
            pending = pending + 1
          END IF
          cont = __regex_ContRep[repRec, repCount + 1, pos, cont]
          node = repRec.child
          isNode = TRUE
        ELSEIF mustMore THEN
          failing = TRUE
        ELSE
          isNode = FALSE
        END IF
      ELSE
        IF NOT mustMore THEN
          IF canMore THEN
            ' Lazy: the continuation first; one more iteration is the alternative.
            stack = __regex_Choice[3, root, repRec, cont, pos, caps, 0, repCount, 0, stack]
            pending = pending + 1
          END IF
          isNode = FALSE
        ELSEIF canMore THEN
          cont = __regex_ContRep[repRec, repCount + 1, pos, cont]
          node = repRec.child
          isNode = TRUE
        ELSE
          failing = TRUE
        END IF
      END IF
    END IF
  END WHILE
  RETURN __regex_fail()
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_run", BODY));
}
