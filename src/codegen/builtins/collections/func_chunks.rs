//! `collections::chunks` — descriptor entry + MFBASIC source body (Implementation::Mfb).
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;

const INTRO: &str =
    "Split a list into consecutive, non-overlapping blocks of at most `chunkSize` elements";

#[rustfmt::skip]
const BODY: &str =
"FUNC __collections_chunks OF T(value AS List OF T, chunkSize AS Integer) AS List OF List OF T
  IF chunkSize < 1 THEN
    FAIL error(77050002, \"Argument value is not valid for the requested operation.\")
  END IF
  MUT result AS List OF List OF T = []
  MUT i AS Integer = 0
  WHILE i < len(value)
    MUT stop AS Integer = i + chunkSize
    IF stop > len(value) THEN
      stop = len(value)
    END IF
    LET piece AS List OF T = __collections_slice(value, i, stop)
    result = collections::append(result, piece)
    i = i + chunkSize
  END WHILE
  RETURN result
END FUNC";

pub(crate) const CHUNKS: BuiltinFunction = BuiltinFunction::mfb(
    "collections.chunks",
    "chunks",
    INTRO,
    "",
    &["ErrInvalidArgument"],
    &[custom(&[
        req("value", &["list"], "List OF T"),
        req("chunkSize", &[], "Integer"),
    ])],
    BODY,
);
