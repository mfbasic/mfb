//! `__regex_flatten` — shared private helper for the `regex` package.
//!
//! plan-134-I: turns the parser's `__regex_Node` tree into the flat program the matcher
//! runs, once per compile. See `helper_run.rs` for why the matcher holds no subtree.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-134-I: the program is the parse tree flattened into integer tables, so the matcher
' reads a node with scalar loads and never copies a subtree. Op `o` is `kinds[o]` with
' operands `opA[o]`, `opB[o]`, `opC[o]`:
'   1 a one-scalar leaf (Lit, Any, Class) and 2 an anchor -- opA = its index in `leaves`
'   3 Concat and 4 Alt -- opA = the first of its parts or options in `kids`, opB = how many
'   5 a greedy and 6 a lazy Repeat -- opA = the child op, opB = lo, opC = hi
'   7 Group -- opA = the child op, opB = the capture slot
' The root is op 0. `kids` holds op indices. The walk keeps an explicit stack of
' (subtree, op) pairs: a parent reserves its children's ops, in order, when it is
' flattened, and each child fills its op when it is popped.
FUNC __regex_flatten(root AS __regex_Node, groups AS Integer, names AS Map OF String TO Integer) AS __regex_Program
  MUT kinds AS List OF Integer = [0]
  MUT opA AS List OF Integer = [0]
  MUT opB AS List OF Integer = [0]
  MUT opC AS List OF Integer = [0]
  MUT kids AS List OF Integer = []
  MUT leaves AS List OF __regex_Leaf = []
  MUT work AS List OF __regex_Node = [root]
  MUT workOps AS List OF Integer = [0]
  WHILE len(work) > 0
    LET last AS Integer = len(work) - 1
    LET node AS __regex_Node = collections::get(work, last)
    LET op AS Integer = collections::get(workOps, last)
    work = collections::removeAt(work, last)
    workOps = collections::removeAt(workOps, last)
    MATCH node
      CASE __regex_Lit(litNode)
        LET litLeaf AS __regex_Leaf = litNode
        kinds = collections::set(kinds, op, 1)
        opA = collections::set(opA, op, len(leaves))
        leaves = collections::append(leaves, litLeaf)
      CASE __regex_Any(anyNode)
        LET anyLeaf AS __regex_Leaf = anyNode
        kinds = collections::set(kinds, op, 1)
        opA = collections::set(opA, op, len(leaves))
        leaves = collections::append(leaves, anyLeaf)
      CASE __regex_Class(clsNode)
        LET clsLeaf AS __regex_Leaf = clsNode
        kinds = collections::set(kinds, op, 1)
        opA = collections::set(opA, op, len(leaves))
        leaves = collections::append(leaves, clsLeaf)
      CASE __regex_Anchor(anchorNode)
        LET anchorLeaf AS __regex_Leaf = anchorNode
        kinds = collections::set(kinds, op, 2)
        opA = collections::set(opA, op, len(leaves))
        leaves = collections::append(leaves, anchorLeaf)
      CASE __regex_Concat(seqNode)
        kinds = collections::set(kinds, op, 3)
        opA = collections::set(opA, op, len(kids))
        opB = collections::set(opB, op, len(seqNode.parts))
        MUT partAt AS Integer = 0
        WHILE partAt < len(seqNode.parts)
          LET partOp AS Integer = len(kinds)
          kinds = collections::append(kinds, 0)
          opA = collections::append(opA, 0)
          opB = collections::append(opB, 0)
          opC = collections::append(opC, 0)
          kids = collections::append(kids, partOp)
          work = collections::append(work, collections::get(seqNode.parts, partAt))
          workOps = collections::append(workOps, partOp)
          partAt = partAt + 1
        END WHILE
      CASE __regex_Alt(altNode)
        kinds = collections::set(kinds, op, 4)
        opA = collections::set(opA, op, len(kids))
        opB = collections::set(opB, op, len(altNode.opts))
        MUT optAt AS Integer = 0
        WHILE optAt < len(altNode.opts)
          LET optOp AS Integer = len(kinds)
          kinds = collections::append(kinds, 0)
          opA = collections::append(opA, 0)
          opB = collections::append(opB, 0)
          opC = collections::append(opC, 0)
          kids = collections::append(kids, optOp)
          work = collections::append(work, collections::get(altNode.opts, optAt))
          workOps = collections::append(workOps, optOp)
          optAt = optAt + 1
        END WHILE
      CASE __regex_Repeat(repNode)
        LET repChild AS Integer = len(kinds)
        IF repNode.greedy THEN
          kinds = collections::set(kinds, op, 5)
        ELSE
          kinds = collections::set(kinds, op, 6)
        END IF
        opA = collections::set(opA, op, repChild)
        opB = collections::set(opB, op, repNode.lo)
        opC = collections::set(opC, op, repNode.hi)
        kinds = collections::append(kinds, 0)
        opA = collections::append(opA, 0)
        opB = collections::append(opB, 0)
        opC = collections::append(opC, 0)
        work = collections::append(work, repNode.child)
        workOps = collections::append(workOps, repChild)
      CASE __regex_Group(grpNode)
        LET grpChild AS Integer = len(kinds)
        kinds = collections::set(kinds, op, 7)
        opA = collections::set(opA, op, grpChild)
        opB = collections::set(opB, op, grpNode.slot)
        kinds = collections::append(kinds, 0)
        opA = collections::append(opA, 0)
        opB = collections::append(opB, 0)
        opC = collections::append(opC, 0)
        work = collections::append(work, grpNode.child)
        workOps = collections::append(workOps, grpChild)
    END MATCH
  END WHILE
  ' plan-77 R5: the code point a match MUST begin with, or -1 when the pattern has no
  ' fixed first literal (so __regex_searchFrom can fast-skip start offsets). Only a
  ' non-folding literal yields a fixed cp; Concat and Group are transparent to their
  ' first child. Everything else (Any, Class, Anchor, Alt, Repeat -- which may be
  ' optional or multi-valued) is conservatively "unknown".
  MUT firstCp AS Integer = -1
  MUT at AS Integer = 0
  MUT walking AS Boolean = TRUE
  WHILE walking
    LET atKind AS Integer = collections::get(kinds, at)
    walking = FALSE
    IF atKind = 1 THEN
      LET firstLeaf AS __regex_Leaf = collections::get(leaves, collections::get(opA, at))
      MATCH firstLeaf
        CASE __regex_Lit(firstLit)
          IF firstLit.fold = FALSE THEN
            firstCp = firstLit.cp
          END IF
        CASE ELSE
          firstCp = -1
      END MATCH
    ELSEIF atKind = 3 THEN
      IF collections::get(opB, at) > 0 THEN
        at = collections::get(kids, collections::get(opA, at))
        walking = TRUE
      END IF
    ELSEIF atKind = 7 THEN
      at = collections::get(opA, at)
      walking = TRUE
    END IF
  END WHILE
  RETURN __regex_Program[kinds, opA, opB, opC, kids, leaves, 0, firstCp, groups, names]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_flatten", BODY));
}
