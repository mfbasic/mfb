//! The `big::Int` overload of `crypto::randomInt` — `__crypto_randomIntBig`, as ONE gated
//! source chunk (plan-127-D).
//!
//! Why a gated helper and not a second `Always` body: the chunk calls `big::` members, which
//! are undefined unless `big` is imported, so it is injected only when a program imports BOTH
//! `crypto` and `big` (`HelperGate::WhenBothImported`, the `term`/`astrings` bridge
//! precedent). A `crypto`-only program never sees it.
//!
//! The chunk composes native `big` members (`compare`, `add`, `subtract`, `bitLength`,
//! `fromBytes`) and `crypto::randomBytes`; it does no digit arithmetic of its own. The
//! sampling mirrors the `Integer` overload's contract: reject, never reduce.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' crypto::randomInt(min AS big::Int, max AS big::Int): an exactly uniform big::Int in
' [min, max], with no span limit.
'
' The span has `width` significant bits. Each attempt draws ceil(width / 8) fresh bytes
' from the CSPRNG, keeps only `width` bits (masking the top byte), and accepts the value
' when it is below the span. Masking bounds the expected number of attempts below two;
' reducing a wider draw modulo the span instead would skew toward small values.
IMPORT crypto
IMPORT big
IMPORT collections

FUNC __crypto_randomIntBig(min AS big::Int, max AS big::Int) AS big::Int
  IF big::compare(max, min) < 0 THEN
    FAIL error(77050002, "randomInt min > max")
  END IF
  IF big::equals(max, min) THEN
    RETURN min
  END IF
  LET span AS big::Int = big::add(big::subtract(max, min), big::fromInteger(1))
  LET width AS Integer = big::bitLength(span)
  LET byteCount AS Integer = (width + 7) / 8
  LET topBits AS Integer = width - (byteCount - 1) * 8
  MUT topLimit AS Integer = 1
  MUT bit AS Integer = 0
  WHILE bit < topBits
    topLimit = topLimit * 2
    bit = bit + 1
  END WHILE
  MUT candidate AS big::Int = big::fromInteger(0)
  MUT accepted AS Boolean = FALSE
  WHILE NOT accepted
    MUT draw AS List OF Byte = crypto::randomBytes(byteCount)
    LET top AS Integer = toInt(collections::get(draw, byteCount - 1)) MOD topLimit
    draw = collections::set(draw, byteCount - 1, toByte(top))
    candidate = big::fromBytes(draw, FALSE)
    accepted = big::compare(candidate, span) < 0
  END WHILE
  RETURN big::add(min, candidate)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_randomIntBig",
        gate: HelperGate::WhenBothImported("crypto", "big"),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
