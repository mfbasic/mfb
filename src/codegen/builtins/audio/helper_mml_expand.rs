//! `__audio_mmlExpand` — shared private helper for the `audio` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source, in the order `mod.rs` calls the helpers. Body byte-significant
//! (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' Expand every `{ .. }<count>` repeat, innermost first, into a flat token list.
' Raises on an unbalanced brace or a count < 1.
FUNC __audio_mmlExpand(tokens AS List OF String) AS List OF String
  MUT work AS List OF String = tokens
  MUT guard AS Boolean = TRUE
  WHILE guard
    MUT closeIdx AS Integer = -1
    MUT i AS Integer = 0
    WHILE i < len(work)
      IF strings::mid(collections::get(work, i), 0, 1) = "}" THEN
        closeIdx = i
        i = len(work)
      ELSE
        i = i + 1
      END IF
    END WHILE
    IF closeIdx < 0 THEN
      IF __audio_mmlHasOpen(work) THEN
        FAIL error(77050002, "audio::play: unbalanced '{' in MML")
      END IF
      RETURN work
    END IF
    LET closeTk AS String = collections::get(work, closeIdx)
    LET count AS Integer = __audio_mmlParseUint(strings::mid(closeTk, 1, len(closeTk) - 1))
    IF count < 1 THEN
      FAIL error(77050002, "audio::play: repeat count must be >= 1 in '" & closeTk & "'")
    END IF
    MUT openIdx AS Integer = -1
    MUT j AS Integer = closeIdx - 1
    WHILE j >= 0
      IF collections::get(work, j) = "{" THEN
        openIdx = j
        j = -1
      ELSE
        j = j - 1
      END IF
    END WHILE
    IF openIdx < 0 THEN
      FAIL error(77050002, "audio::play: '}' without matching '{' in MML")
    END IF
    ' The expansion is bounded before it is built (bug-509, DEC-55). Repeats nest
    ' multiplicatively -- `{ { { { C }64 }64 }64 }64` is thirty characters and sixteen
    ' million notes -- and building that list was 38 GB and a process killed before
    ' any raise could fire. 65,536 tokens is two hours of sixteenth notes at T120, far
    ' past any tune a program embeds. The count is refused on its own first, so the
    ' product below cannot overflow.
    IF count > 65536 THEN
      FAIL error(77050002, "audio::play: MML expands past 65536 tokens at '" & closeTk & "'")
    END IF
    LET inner AS Integer = closeIdx - openIdx - 1
    IF (len(work) - inner - 2) + inner * count > 65536 THEN
      FAIL error(77050002, "audio::play: MML expands past 65536 tokens at '" & closeTk & "'")
    END IF
    MUT rebuilt AS List OF String = []
    FOR k = 0 TO openIdx - 1
      rebuilt = collections::append(rebuilt, collections::get(work, k))
    NEXT
    MUT rep AS Integer = 0
    WHILE rep < count
      FOR k = openIdx + 1 TO closeIdx - 1
        rebuilt = collections::append(rebuilt, collections::get(work, k))
      NEXT
      rep = rep + 1
    END WHILE
    FOR k = closeIdx + 1 TO len(work) - 1
      rebuilt = collections::append(rebuilt, collections::get(work, k))
    NEXT
    work = rebuilt
  END WHILE
  RETURN work
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("audio_mmlExpand", BODY));
}
