//! `__regex_matchNode` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_matchNode(node AS __regex_Node, pos AS Integer, caps AS List OF Integer, c AS __regex_Cont, ctx AS __regex_Ctx, depth AS Integer) AS __regex_Result
  ' bug-315: every node visit is one step. This is the single dispatch every
  ' alternative, repetition and continuation passes through, so counting here
  ' bounds the whole search regardless of which construct is blowing up.
  __regex_steps = __regex_steps + 1
  IF __regex_steps > __REGEX_STEP_BUDGET THEN
    FAIL error(77050003, "regex: pattern too complex for this input (backtracking limit exceeded)")
  END IF
  ' bug-315: and a recursion-depth guard. A greedy repeat over a SIMPLE child is
  ' consumed iteratively below and never reaches this, but a repeat over a GROUP
  ' -- `(ab)*` -- still recurses once per repetition, and the native stack was
  ' exhausted somewhere between 800 and 1000 frames, killing the process with an
  ' uncatchable SIGSEGV. This turns that into an ordinary catchable failure well
  ' before the stack runs out.
  IF depth > __REGEX_DEPTH_LIMIT THEN
    FAIL error(77050003, "regex: pattern too complex for this input (nesting limit exceeded)")
  END IF
  MATCH node
    CASE __regex_Lit(litNode)
      IF pos >= ctx.n THEN
        RETURN __regex_fail()
      END IF
      IF __regex_charEq(litNode.ch, collections::get(ctx.text, pos), litNode.fold) THEN
        RETURN __regex_matchCont(c, pos + 1, caps, ctx, depth + 1)
      END IF
      RETURN __regex_fail()
    CASE __regex_Any(anyNode)
      IF pos >= ctx.n THEN
        RETURN __regex_fail()
      END IF
      IF anyNode.dotall OR collections::get(ctx.text, pos) <> "\n" THEN
        RETURN __regex_matchCont(c, pos + 1, caps, ctx, depth + 1)
      END IF
      RETURN __regex_fail()
    CASE __regex_Class(clsNode)
      IF pos >= ctx.n THEN
        RETURN __regex_fail()
      END IF
      IF __regex_classMatch(clsNode, pos, ctx) THEN
        RETURN __regex_matchCont(c, pos + 1, caps, ctx, depth + 1)
      END IF
      RETURN __regex_fail()
    CASE __regex_Anchor(anchorNode)
      IF __regex_anchorMatch(anchorNode, pos, ctx) THEN
        RETURN __regex_matchCont(c, pos, caps, ctx, depth + 1)
      END IF
      RETURN __regex_fail()
    CASE __regex_Concat(seqNode)
      RETURN __regex_matchCont(__regex_ContSeq[seqNode.parts, 0, c], pos, caps, ctx, depth + 1)
    CASE __regex_Alt(altNode)
      RETURN __regex_matchAlt(altNode.opts, 0, pos, caps, c, ctx, depth + 1)
    CASE __regex_Repeat(repNode)
      RETURN __regex_matchRep(repNode, 0, pos, caps, c, ctx, depth + 1)
    CASE __regex_Group(grpNode)
      LET caps2 AS List OF Integer = __regex_setCap(caps, 2 * grpNode.slot, pos)
      RETURN __regex_matchNode(grpNode.child, pos, caps2, __regex_ContCap[grpNode.slot, c], ctx, depth + 1)
  END MATCH
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_matchNode", BODY));
}
