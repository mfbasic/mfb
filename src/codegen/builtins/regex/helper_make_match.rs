//! `__regex_makeMatch` — shared private helper for the `regex` package.
//!
//! bug-532: turns one engine result into the public `MatchInfo`. The engine already
//! records every span this needs — `__regex_tryAt` seeds capture slot `0` with the
//! start and the outer `ContCap[0, ContDone]` closes it with the end, and each
//! group's `Group` node writes slots `2k`/`2k+1` — so nothing here re-runs the
//! matcher; it only reports what the search already computed and used to discard.
//!
//! The slices are taken with the same `strings::mid` call `__regex_lookupNum` uses
//! to expand `$N` in a replacement template, over the same capture slots, so
//! `groups[k].text` and `$k` are the same text by construction. Slot indices are
//! Unicode scalar positions throughout, so no index conversion happens here.
//!
//! An unset slot is `-1` (the value `__regex_initCaps` seeds), which is how a group
//! that took no part in the match is reported; group `0` is always set on a
//! successful match, so `groups[0]` restates the whole match.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"FUNC __regex_makeMatch(r AS __regex_Result, value AS String, prog AS __regex_Program) AS MatchInfo
  MUT gs AS List OF Group = []
  MUT i AS Integer = 0
  WHILE i <= prog.groups
    LET gstart AS Integer = collections::get(r.caps, 2 * i)
    LET gend AS Integer = collections::get(r.caps, 2 * i + 1)
    IF gstart < 0 OR gend < 0 THEN
      gs = collections::append(gs, Group[-1, -1, ""])
    ELSE
      gs = collections::append(gs, Group[gstart, gend, strings::mid(value, gstart, gend - gstart)])
    END IF
    i = i + 1
  END WHILE
  LET ms AS Integer = collections::get(r.caps, 0)
  RETURN MatchInfo[ms, r.pos, strings::mid(value, ms, r.pos - ms), gs, prog.names]
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_makeMatch", BODY));
}
