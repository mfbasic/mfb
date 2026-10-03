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
' bug-773: the popped subtree is read where it sits in `work` -- only MATCHed, and
' `work` is not written until that MATCH is done, so the read is a borrow, not a copy
' of the whole subtree (the old `get` + `removeAt` order copied the whole tree for the
' root). A Concat or Alt reads its children in place (FOR EACH) and fills a leaf
' child's op on the spot -- a leaf reserves no ops -- so a leaf is never copied onto
' the stack. Its other children are copied into `pend` and pushed, in order, once the
' parent is off the stack.
FUNC __regex_flatten(root AS __regex_Node, groups AS Integer, names AS Map OF String TO Integer) AS __regex_Program
  MUT kinds AS List OF Integer = [0]
  MUT opA AS List OF Integer = [0]
  MUT opB AS List OF Integer = [0]
  MUT opC AS List OF Integer = [0]
  MUT kids AS List OF Integer = []
  MUT leaves AS List OF __regex_Leaf = []
  MUT work AS List OF __regex_Node = [root]
  MUT workOps AS List OF Integer = [0]
  MUT pend AS List OF __regex_Node = []
  MUT pendOps AS List OF Integer = []
  WHILE len(work) > 0
    LET last AS Integer = len(work) - 1
    LET op AS Integer = collections::get(workOps, last)
    LET node AS __regex_Node = collections::get(work, last)
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
        LET partBase AS Integer = len(kinds)
        kinds = collections::set(kinds, op, 3)
        opA = collections::set(opA, op, len(kids))
        opB = collections::set(opB, op, len(seqNode.parts))
        MUT partAt AS Integer = 0
        WHILE partAt < len(seqNode.parts)
          kinds = collections::append(kinds, 0)
          opA = collections::append(opA, 0)
          opB = collections::append(opB, 0)
          opC = collections::append(opC, 0)
          kids = collections::append(kids, partBase + partAt)
          partAt = partAt + 1
        END WHILE
        MUT partOp AS Integer = partBase
        FOR EACH part IN seqNode.parts
          MATCH part
            CASE __regex_Lit(partLit)
              LET partLitLeaf AS __regex_Leaf = partLit
              kinds = collections::set(kinds, partOp, 1)
              opA = collections::set(opA, partOp, len(leaves))
              leaves = collections::append(leaves, partLitLeaf)
            CASE __regex_Any(partAny)
              LET partAnyLeaf AS __regex_Leaf = partAny
              kinds = collections::set(kinds, partOp, 1)
              opA = collections::set(opA, partOp, len(leaves))
              leaves = collections::append(leaves, partAnyLeaf)
            CASE __regex_Class(partCls)
              LET partClsLeaf AS __regex_Leaf = partCls
              kinds = collections::set(kinds, partOp, 1)
              opA = collections::set(opA, partOp, len(leaves))
              leaves = collections::append(leaves, partClsLeaf)
            CASE __regex_Anchor(partAnchor)
              LET partAnchorLeaf AS __regex_Leaf = partAnchor
              kinds = collections::set(kinds, partOp, 2)
              opA = collections::set(opA, partOp, len(leaves))
              leaves = collections::append(leaves, partAnchorLeaf)
            CASE ELSE
              pend = collections::append(pend, part)
              pendOps = collections::append(pendOps, partOp)
          END MATCH
          partOp = partOp + 1
        NEXT
      CASE __regex_Alt(altNode)
        LET optBase AS Integer = len(kinds)
        kinds = collections::set(kinds, op, 4)
        opA = collections::set(opA, op, len(kids))
        opB = collections::set(opB, op, len(altNode.opts))
        MUT optAt AS Integer = 0
        WHILE optAt < len(altNode.opts)
          kinds = collections::append(kinds, 0)
          opA = collections::append(opA, 0)
          opB = collections::append(opB, 0)
          opC = collections::append(opC, 0)
          kids = collections::append(kids, optBase + optAt)
          optAt = optAt + 1
        END WHILE
        MUT optOp AS Integer = optBase
        FOR EACH opt IN altNode.opts
          MATCH opt
            CASE __regex_Lit(optLit)
              LET optLitLeaf AS __regex_Leaf = optLit
              kinds = collections::set(kinds, optOp, 1)
              opA = collections::set(opA, optOp, len(leaves))
              leaves = collections::append(leaves, optLitLeaf)
            CASE __regex_Any(optAny)
              LET optAnyLeaf AS __regex_Leaf = optAny
              kinds = collections::set(kinds, optOp, 1)
              opA = collections::set(opA, optOp, len(leaves))
              leaves = collections::append(leaves, optAnyLeaf)
            CASE __regex_Class(optCls)
              LET optClsLeaf AS __regex_Leaf = optCls
              kinds = collections::set(kinds, optOp, 1)
              opA = collections::set(opA, optOp, len(leaves))
              leaves = collections::append(leaves, optClsLeaf)
            CASE __regex_Anchor(optAnchor)
              LET optAnchorLeaf AS __regex_Leaf = optAnchor
              kinds = collections::set(kinds, optOp, 2)
              opA = collections::set(opA, optOp, len(leaves))
              leaves = collections::append(leaves, optAnchorLeaf)
            CASE ELSE
              pend = collections::append(pend, opt)
              pendOps = collections::append(pendOps, optOp)
          END MATCH
          optOp = optOp + 1
        NEXT
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
        pend = collections::append(pend, repNode.child)
        pendOps = collections::append(pendOps, repChild)
      CASE __regex_Group(grpNode)
        LET grpChild AS Integer = len(kinds)
        kinds = collections::set(kinds, op, 7)
        opA = collections::set(opA, op, grpChild)
        opB = collections::set(opB, op, grpNode.slot)
        kinds = collections::append(kinds, 0)
        opA = collections::append(opA, 0)
        opB = collections::append(opB, 0)
        opC = collections::append(opC, 0)
        pend = collections::append(pend, grpNode.child)
        pendOps = collections::append(pendOps, grpChild)
    END MATCH
    work = collections::removeAt(work, last)
    workOps = collections::removeAt(workOps, last)
    IF len(pendOps) > 0 THEN
      ' One element at a time: appending the whole `pend` list leaks the pushed
      ' subtrees' graphs (a compiler bug in the list-to-list append of a recursive
      ' element type, reported with bug-773).
      FOR EACH pendKid IN pend
        work = collections::append(work, pendKid)
      NEXT
      workOps = collections::append(workOps, pendOps)
      pend = []
      pendOps = []
    END IF
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
  ' bug-773: no fixed first literal -- attach the first-scalar set, if the program has
  ' one, after `kids`; `firstCp` = -2 - (its offset) = 128 - len(setKids). Computed
  ' here, before the program is built, so the program is not rebuilt to carry it.
  IF firstCp < 0 THEN
    LET setKids AS List OF Integer = __regex_firstSetKids(kinds, opA, opB, opC, kids, leaves, 0)
    IF len(setKids) > 0 THEN
      firstCp = 128 - len(setKids)
      kids = setKids
    END IF
  END IF
  RETURN __regex_Program[kinds, opA, opB, opC, kids, leaves, 0, firstCp, groups, names]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_flatten", BODY));
}
