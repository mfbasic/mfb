//! `__regex_run` — the matcher: an explicit-stack backtracker (bug-510, DEC-01) over the
//! flat program `__regex_flatten` builds (plan-134-I).
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
//! succeeds, the empty-iteration guard on a repeat frame. What used to be "try the
//! preferred branch; on failure fall through to the next" is "push the next branch as a
//! choice point; run the preferred one; on failure pop". The pinned corpus in
//! `tests/rt_regex_bounds.rs` is the proof that nothing observable moved.
//!
//! **Everything the loop holds is an integer** (plan-134-I). A node is an op index into
//! the program's tables, the continuation is a frame index, and a choice point is eight
//! integers plus a capture snapshot. Since plan-134 a recursive value is really copied
//! and really freed, and the bug-510 engine held pattern subtrees and continuation
//! chains by value: `rt_regex_bounds`' findAll program went from 1.08 s to 30.57 s and
//! from 7.5 M to 307 M allocations. Here a node visit or continuation step allocates
//! nothing beyond the growth of three tables:
//!
//! - `frames`, five integers per continuation frame — tag (1 sequence: `a` = the Concat
//!   op, `b` = the next part; 2 capture close: `a` = the slot; 3 repeat: `a` = the Repeat
//!   op, `b` = its iteration count, `c` = the position the iteration started at) and the
//!   frame below (-1 = done). Frames are never rewritten, so a choice point that saved a
//!   frame index resumes exactly that continuation. They grow with the steps of one
//!   search, which the step budgets bound.
//! - `choices`, eight integers per pending choice point — kind, `alt`, `rep`, `cont`,
//!   `pos`, `i`, `count`, `p` — and `snaps`, its capture list. Popping lowers `pending`
//!   and a push overwrites the slot above, so both tables stop growing at the deepest
//!   stack the search reached, which `__REGEX_PENDING_LIMIT` bounds. Holding only
//!   integers, they are immune to bug-538 (the growable-list aliasing that sank the
//!   first bug-510 attempt, whose side tables held recursive records).
//!
//! **Cost bounds.** Node visits are charged to the per-search step budget as before
//! and, since bug-510, to the per-call budget `__regex_makeCtx` sets (DEC-02). The
//! number of pending choice points is capped by `__REGEX_PENDING_LIMIT`.
//!
//! Registered via `add_helper`; body byte-significant (2-space indent → `.ncode`
//! columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' A pending choice point: `kind` 1 = the next option `i` of the Alt op `alt`, 2 = stop
' repeating `rep` and run the continuation, 3 = one more (lazy) iteration of `rep` after
' `count`, 4 = give back one scalar of the greedy simple repeat `rep` (`i` = scalars
' consumed so far, `count` = the iteration count before, `p` = the position after the
' scalars consumed). `cont`, `pos` and the capture snapshot are what every kind resumes
' with. An iteration pushes at most one choice point: it is recorded (`push*`) where the
' engine decides it and pushed at the end of the iteration, when the captures are still
' the ones it was decided with. A continuation frame is recorded (`frame*`) the same way
' and written at the end of the iteration: an iteration pushes at most one frame, and
' nothing reads a frame in the iteration that pushes it.
'
' bug-773: one call runs the whole search -- every start position from `from` up, in
' order, the first match winning -- so the tables are built once per search and reset,
' not rebuilt, at each start: `frameTop` goes back to the root's capture frame,
' `pending` is already 0 when a start has failed, and `caps` is refilled. A start
' position whose scalar cannot begin a match is skipped: past a fixed first literal
' (`firstCp` >= 0, plan-77 R5), or outside the first-scalar set `__regex_compile`
' attaches (`firstCp` <= -2, see __regex_firstSetKids), where the skip charges both
' budgets the node visits the failed attempt would have made.
FUNC __regex_run(prog AS __regex_Program, leaves AS List OF __regex_Leaf, from AS Integer, ctx AS __regex_Ctx) AS __regex_Result
  LET firstCp AS Integer = prog.firstCp
  MUT setAt AS Integer = -1
  MUT setNonAscii AS Boolean = FALSE
  MUT rejectCost AS Integer = 0
  IF firstCp <= -2 THEN
    setAt = -2 - firstCp
    setNonAscii = collections::get(prog.kids, setAt + 128) <> 0
    rejectCost = collections::get(prog.kids, setAt + 129)
  END IF
  MUT frames AS List OF Integer = [2, 0, 0, 0, -1]
  MUT frameTop AS Integer = 1
  MUT frameTag AS Integer = 0
  MUT frameA AS Integer = 0
  MUT frameB AS Integer = 0
  MUT frameC AS Integer = 0
  MUT frameBelow AS Integer = 0
  MUT choices AS List OF Integer = []
  MUT snaps AS List OF Integer = []
  MUT pending AS Integer = 0
  MUT isNode AS Boolean = TRUE
  MUT node AS Integer = prog.start
  MUT cont AS Integer = 0
  MUT pos AS Integer = from
  MUT caps AS List OF Integer = __regex_initCaps(prog.groups)
  LET capsLen AS Integer = len(caps)
  ' The search begins as a failed attempt at `from - 1`, so the first iteration moves
  ' to the first start position exactly as every later failure moves to the next.
  MUT s AS Integer = from - 1
  MUT failing AS Boolean = TRUE
  MUT repPending AS Boolean = FALSE
  MUT repCount AS Integer = 0
  MUT rep AS Integer = prog.start
  MUT pushKind AS Integer = 0
  MUT pushAlt AS Integer = 0
  MUT pushRep AS Integer = 0
  MUT pushCont AS Integer = 0
  MUT pushPos AS Integer = 0
  MUT pushI AS Integer = 0
  MUT pushCount AS Integer = 0
  MUT pushP AS Integer = 0
  WHILE TRUE
    IF failing AND pending = 0 THEN
      s = s + 1
      IF firstCp >= 0 THEN
        WHILE s < ctx.n AND collections::get(ctx.cps, s) <> firstCp
          s = s + 1
        END WHILE
      ELSEIF setAt >= 0 THEN
        MUT skipping AS Boolean = TRUE
        WHILE skipping AND s < ctx.n
          LET sc AS Integer = collections::get(ctx.cps, s)
          MUT may AS Boolean = setNonAscii
          IF sc < 128 THEN
            may = collections::get(prog.kids, setAt + sc) <> 0
          END IF
          ' A skip that would cross a budget runs the attempt instead, which fails the
          ' call with the same error the attempt always raised there.
          IF may OR __regex_steps + rejectCost > __REGEX_STEP_BUDGET OR __regex_callSteps + rejectCost > __regex_callBudget THEN
            skipping = FALSE
          ELSE
            __regex_steps = __regex_steps + rejectCost
            __regex_callSteps = __regex_callSteps + rejectCost
            s = s + 1
          END IF
        END WHILE
      END IF
      IF s > ctx.n THEN
        RETURN __regex_fail()
      END IF
      frameTop = 1
      MUT c0 AS Integer = 0
      WHILE c0 < capsLen
        caps = collections::set(caps, c0, -1)
        c0 = c0 + 1
      END WHILE
      caps = collections::set(caps, 0, s)
      node = prog.start
      cont = 0
      pos = s
      isNode = TRUE
      failing = FALSE
    END IF
    IF failing THEN
      pending = pending - 1
      LET top AS Integer = 8 * pending
      LET ck AS Integer = collections::get(choices, top)
      LET cAlt AS Integer = collections::get(choices, top + 1)
      LET cRep AS Integer = collections::get(choices, top + 2)
      cont = collections::get(choices, top + 3)
      pos = collections::get(choices, top + 4)
      LET ci AS Integer = collections::get(choices, top + 5)
      LET cCount AS Integer = collections::get(choices, top + 6)
      LET cP AS Integer = collections::get(choices, top + 7)
      LET snapAt AS Integer = capsLen * pending
      MUT r AS Integer = 0
      WHILE r < capsLen
        caps = collections::set(caps, r, collections::get(snaps, snapAt + r))
        r = r + 1
      END WHILE
      IF ck = 1 THEN
        IF ci + 1 < collections::get(prog.opB, cAlt) THEN
          pushKind = 1
          pushAlt = cAlt
          pushRep = cRep
          pushCont = cont
          pushPos = pos
          pushI = ci + 1
          pushCount = 0
          pushP = 0
        END IF
        node = collections::get(prog.kids, collections::get(prog.opA, cAlt) + ci)
        isNode = TRUE
        failing = FALSE
      ELSEIF ck = 2 THEN
        isNode = FALSE
        failing = FALSE
      ELSEIF ck = 3 THEN
        frameTag = 3
        frameA = cRep
        frameB = cCount + 1
        frameC = pos
        frameBelow = cont
        cont = frameTop
        node = collections::get(prog.opA, cRep)
        isNode = TRUE
        failing = FALSE
      ELSE
        ' Give one scalar back and try the continuation there -- while the repeat
        ' still holds more than it started with and no fewer than its minimum. Below
        ' the minimum there is nothing left to try, and the pop above stands.
        IF ci > cCount AND ci - 1 >= collections::get(prog.opB, cRep) THEN
          pushKind = 4
          pushAlt = cAlt
          pushRep = cRep
          pushCont = cont
          pushPos = pos
          pushI = ci - 1
          pushCount = cCount
          pushP = cP - 1
          pos = cP - 1
          isNode = FALSE
          failing = FALSE
        END IF
      END IF
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
      LET kind AS Integer = collections::get(prog.kinds, node)
      IF kind = 1 THEN
        IF __regex_simpleMatchAt(leaves, collections::get(prog.opA, node), pos, ctx) THEN
          pos = pos + 1
          isNode = FALSE
        ELSE
          failing = TRUE
        END IF
      ELSEIF kind = 2 THEN
        LET anchorLeaf AS __regex_Leaf = collections::get(leaves, collections::get(prog.opA, node))
        MATCH anchorLeaf
          CASE __regex_Anchor(anchorNode)
            IF __regex_anchorMatch(anchorNode, pos, ctx) THEN
              isNode = FALSE
            ELSE
              failing = TRUE
            END IF
          CASE ELSE
            failing = TRUE
        END MATCH
      ELSEIF kind = 3 THEN
        frameTag = 1
        frameA = node
        frameB = 0
        frameC = 0
        frameBelow = cont
        cont = frameTop
        isNode = FALSE
      ELSEIF kind = 4 THEN
        LET optCount AS Integer = collections::get(prog.opB, node)
        IF optCount = 0 THEN
          failing = TRUE
        ELSE
          IF optCount > 1 THEN
            pushKind = 1
            pushAlt = node
            pushRep = rep
            pushCont = cont
            pushPos = pos
            pushI = 1
            pushCount = 0
            pushP = 0
          END IF
          node = collections::get(prog.kids, collections::get(prog.opA, node))
        END IF
      ELSEIF kind = 5 OR kind = 6 THEN
        rep = node
        repCount = 0
        repPending = TRUE
      ELSE
        LET slot AS Integer = collections::get(prog.opB, node)
        caps = collections::set(caps, 2 * slot, pos)
        frameTag = 2
        frameA = slot
        frameB = 0
        frameC = 0
        frameBelow = cont
        cont = frameTop
        node = collections::get(prog.opA, node)
      END IF
    ELSE
      IF cont < 0 THEN
        RETURN __regex_Result[TRUE, pos, caps]
      END IF
      LET at AS Integer = 5 * cont
      LET tag AS Integer = collections::get(frames, at)
      LET fa AS Integer = collections::get(frames, at + 1)
      LET fb AS Integer = collections::get(frames, at + 2)
      LET below AS Integer = collections::get(frames, at + 4)
      IF tag = 1 THEN
        IF fb >= collections::get(prog.opB, fa) THEN
          cont = below
        ELSE
          node = collections::get(prog.kids, collections::get(prog.opA, fa) + fb)
          frameTag = 1
          frameA = fa
          frameB = fb + 1
          frameC = 0
          frameBelow = below
          cont = frameTop
          isNode = TRUE
        END IF
      ELSEIF tag = 2 THEN
        caps = collections::set(caps, 2 * fa + 1, pos)
        cont = below
      ELSE
        ' The empty-iteration guard: an iteration that consumed nothing ends the
        ' repeat, or `(a*)*` would never terminate.
        cont = below
        IF pos <> collections::get(frames, at + 3) THEN
          rep = fa
          repCount = fb
          repPending = TRUE
        END IF
      END IF
    END IF
    IF repPending THEN
      repPending = FALSE
      LET child AS Integer = collections::get(prog.opA, rep)
      LET lo AS Integer = collections::get(prog.opB, rep)
      LET hi AS Integer = collections::get(prog.opC, rep)
      LET canMore AS Boolean = (hi < 0) OR (repCount < hi)
      LET mustMore AS Boolean = repCount < lo
      LET greedy AS Boolean = collections::get(prog.kinds, rep) = 5
      IF greedy AND collections::get(prog.kinds, child) = 1 THEN
        ' bug-315: a greedy repeat over a one-scalar child consumes as far as it can
        ' in a loop, then gives back one scalar at a time (choice kind 4) -- longest
        ' first, exactly the order the recursion explored.
        LET childLeaf AS Integer = collections::get(prog.opA, child)
        MUT p AS Integer = pos
        MUT k AS Integer = repCount
        MUT more AS Boolean = canMore
        WHILE more
          IF __regex_simpleMatchAt(leaves, childLeaf, p, ctx) = FALSE THEN
            more = FALSE
          ELSE
            p = p + 1
            k = k + 1
            more = (hi < 0) OR (k < hi)
          END IF
        END WHILE
        IF k >= lo THEN
          pushKind = 4
          pushAlt = prog.start
          pushRep = rep
          pushCont = cont
          pushPos = pos
          pushI = k
          pushCount = repCount
          pushP = p
          pos = p
          isNode = FALSE
        ELSE
          failing = TRUE
        END IF
      ELSEIF greedy THEN
        IF canMore THEN
          IF NOT mustMore THEN
            ' The alternative, should every way of iterating once more fail: stop
            ' here and run the continuation at this position.
            pushKind = 2
            pushAlt = prog.start
            pushRep = rep
            pushCont = cont
            pushPos = pos
            pushI = 0
            pushCount = 0
            pushP = 0
          END IF
          frameTag = 3
          frameA = rep
          frameB = repCount + 1
          frameC = pos
          frameBelow = cont
          cont = frameTop
          node = child
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
            pushKind = 3
            pushAlt = prog.start
            pushRep = rep
            pushCont = cont
            pushPos = pos
            pushI = 0
            pushCount = repCount
            pushP = 0
          END IF
          isNode = FALSE
        ELSEIF canMore THEN
          frameTag = 3
          frameA = rep
          frameB = repCount + 1
          frameC = pos
          frameBelow = cont
          cont = frameTop
          node = child
          isNode = TRUE
        ELSE
          failing = TRUE
        END IF
      END IF
    END IF
    IF pushKind > 0 THEN
      LET slotAt AS Integer = 8 * pending
      IF slotAt < len(choices) THEN
        choices = collections::set(choices, slotAt, pushKind)
        choices = collections::set(choices, slotAt + 1, pushAlt)
        choices = collections::set(choices, slotAt + 2, pushRep)
        choices = collections::set(choices, slotAt + 3, pushCont)
        choices = collections::set(choices, slotAt + 4, pushPos)
        choices = collections::set(choices, slotAt + 5, pushI)
        choices = collections::set(choices, slotAt + 6, pushCount)
        choices = collections::set(choices, slotAt + 7, pushP)
      ELSE
        choices = collections::append(choices, pushKind)
        choices = collections::append(choices, pushAlt)
        choices = collections::append(choices, pushRep)
        choices = collections::append(choices, pushCont)
        choices = collections::append(choices, pushPos)
        choices = collections::append(choices, pushI)
        choices = collections::append(choices, pushCount)
        choices = collections::append(choices, pushP)
      END IF
      LET savedAt AS Integer = capsLen * pending
      MUT w AS Integer = 0
      IF savedAt < len(snaps) THEN
        WHILE w < capsLen
          snaps = collections::set(snaps, savedAt + w, collections::get(caps, w))
          w = w + 1
        END WHILE
      ELSE
        WHILE w < capsLen
          snaps = collections::append(snaps, collections::get(caps, w))
          w = w + 1
        END WHILE
      END IF
      pending = pending + 1
      pushKind = 0
    END IF
    IF frameTag > 0 THEN
      LET frameAt AS Integer = 5 * frameTop
      IF frameAt < len(frames) THEN
        frames = collections::set(frames, frameAt, frameTag)
        frames = collections::set(frames, frameAt + 1, frameA)
        frames = collections::set(frames, frameAt + 2, frameB)
        frames = collections::set(frames, frameAt + 3, frameC)
        frames = collections::set(frames, frameAt + 4, frameBelow)
      ELSE
        frames = collections::append(frames, frameTag)
        frames = collections::append(frames, frameA)
        frames = collections::append(frames, frameB)
        frames = collections::append(frames, frameC)
        frames = collections::append(frames, frameBelow)
      END IF
      frameTop = frameTop + 1
      frameTag = 0
    END IF
  END WHILE
  RETURN __regex_fail()
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_run", BODY));
}
