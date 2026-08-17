//! `__regex_matchCont` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_matchCont(c AS __regex_Cont, pos AS Integer, caps AS List OF Integer, ctx AS __regex_Ctx, depth AS Integer) AS __regex_Result
  MATCH c
    CASE __regex_ContDone(doneCont)
      RETURN __regex_Result[TRUE, pos, caps]
    CASE __regex_ContSeq(seqCont)
      IF seqCont.idx >= len(seqCont.parts) THEN
        RETURN __regex_matchCont(seqCont.nxt, pos, caps, ctx, depth + 1)
      END IF
      LET part AS __regex_Node = collections::get(seqCont.parts, seqCont.idx)
      RETURN __regex_matchNode(part, pos, caps, __regex_ContSeq[seqCont.parts, seqCont.idx + 1, seqCont.nxt], ctx, depth + 1)
    CASE __regex_ContCap(capCont)
      LET caps2 AS List OF Integer = __regex_setCap(caps, 2 * capCont.slot + 1, pos)
      RETURN __regex_matchCont(capCont.nxt, pos, caps2, ctx, depth + 1)
    CASE __regex_ContRep(repCont)
      IF pos = repCont.startPos THEN
        RETURN __regex_matchCont(repCont.nxt, pos, caps, ctx, depth + 1)
      END IF
      RETURN __regex_matchRep(repCont.rep, repCont.count, pos, caps, repCont.nxt, ctx, depth + 1)
  END MATCH
END FUNC"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_matchCont", BODY));
}
