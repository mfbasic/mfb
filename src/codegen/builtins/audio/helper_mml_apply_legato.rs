//! `__audio_mmlApplyLegato` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Give a legato run (events[startIdx..end]) one attack at the front and one
' release at the tail, with no fades at the interior joins so it sounds tied.
FUNC __audio_mmlApplyLegato(events AS List OF __audio_MmlEvent, startIdx AS Integer) AS List OF __audio_MmlEvent
  MUT out AS List OF __audio_MmlEvent = events
  LET last AS Integer = len(out) - 1
  MUT i AS Integer = startIdx
  WHILE i <= last
    MUT ev AS __audio_MmlEvent = collections::get(out, i)
    MUT fi AS Integer = 0
    MUT fo AS Integer = 0
    IF i = startIdx THEN
      fi = 48
    END IF
    IF i = last THEN
      fo = 48
    END IF
    ev = WITH ev { fadeIn := fi, fadeOut := fo }
    out = collections::set(out, i, ev)
    i = i + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlApplyLegato", BODY));
}
