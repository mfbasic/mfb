//! `__regex_asciiClassBitset` — shared private helper for the `regex` package.
//!
//! Registered via `add_helper`; renders in the helper section of the assembled
//! source (before the member bodies), in the order `mod.rs` calls the helpers.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"' plan-77 R4: precompute a 0..127 ASCII membership table for a character class so
' __regex_classMatch does an O(1) lookup per input position instead of an
' O(items) scan (plus its string comparisons). Built ONCE when the class node is
' constructed, and bit-for-bit what __regex_classMatchOne would answer for each
' scalar, with case folding applied when the class is case-insensitive. `neg` is
' NOT baked in here; __regex_classMatch negates the looked-up bit so a single table
' serves both `[…]` and `[^…]`.
' bug-773: built from code points, item by item, not from a String per code point.
' A Range or Single holds one-scalar Strings (__regex_parseClass builds them from
' pattern scalars and literal escapes), so `ch >= lo AND ch <= hi` is `lo <= cp <= hi`
' on their code points and `ch = sng.ch` is code-point equality: those bits are set
' directly. A Short or Prop item is asked only for a code point no earlier item
' matched -- exactly the calls __regex_classMatchOne's first-hit scan makes. Only a
' folding class builds a String, and only for a code point the class misses.
FUNC __regex_asciiClassBitset(items AS List OF __regex_ClassItem, fold AS Boolean) AS List OF Boolean
  MUT bits AS List OF Boolean = collections::repeat([], FALSE, 128)
  FOR EACH item IN items
    MATCH item
      CASE __regex_Range(rng)
        MUT lo AS Integer = toInt(toScalar(rng.lo))
        LET hi AS Integer = toInt(toScalar(rng.hi))
        WHILE lo <= hi AND lo <= 127
          bits = collections::set(bits, lo, TRUE)
          lo = lo + 1
        END WHILE
      CASE __regex_Single(sng)
        LET one AS Integer = toInt(toScalar(sng.ch))
        IF one <= 127 THEN
          bits = collections::set(bits, one, TRUE)
        END IF
      CASE __regex_Short(sh)
        MUT scp AS Integer = 0
        WHILE scp <= 127
          IF collections::get(bits, scp) = FALSE AND __regex_shorthandMatch(sh.kind, scp) THEN
            bits = collections::set(bits, scp, TRUE)
          END IF
          scp = scp + 1
        END WHILE
      CASE __regex_Prop(pr)
        MUT pcp AS Integer = 0
        WHILE pcp <= 127
          IF collections::get(bits, pcp) = FALSE AND __regex_propMatchItem(pr.name, pr.neg, pcp) THEN
            bits = collections::set(bits, pcp, TRUE)
          END IF
          pcp = pcp + 1
        END WHILE
    END MATCH
  NEXT
  IF fold = FALSE THEN
    RETURN bits
  END IF
  ' A folding class also holds cp when its lower- or upper-case form (one scalar, not
  ' cp itself) is in the class: that form's bit when it is ASCII, else the item scan.
  MUT folded AS List OF Boolean = []
  MUT cp AS Integer = 0
  WHILE cp <= 127
    MUT hit AS Boolean = collections::get(bits, cp)
    IF hit = FALSE THEN
      LET ch AS String = toString(toScalar(cp))
      LET lc AS String = strings::lower(ch)
      IF lc <> ch AND len(lc) = 1 THEN
        LET lcp AS Integer = toInt(toScalar(lc))
        IF lcp <= 127 THEN
          hit = collections::get(bits, lcp)
        ELSE
          hit = __regex_classMatchOne(items, lc, lcp)
        END IF
      END IF
      IF hit = FALSE THEN
        LET uc AS String = strings::upper(ch)
        IF uc <> ch AND len(uc) = 1 THEN
          LET ucp AS Integer = toInt(toScalar(uc))
          IF ucp <= 127 THEN
            hit = collections::get(bits, ucp)
          ELSE
            hit = __regex_classMatchOne(items, uc, ucp)
          END IF
        END IF
      END IF
    END IF
    folded = collections::append(folded, hit)
    cp = cp + 1
  END WHILE
  RETURN folded
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("regex_asciiClassBitset", BODY));
}
