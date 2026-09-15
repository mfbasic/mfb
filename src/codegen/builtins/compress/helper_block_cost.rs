//! `__compress_treeBits` / `__compress_fixedLitBits` / `__compress_headerBits` /
//! `__compress_storedBits` — the exact bit cost of one block under each encoding (plan-137-E §4.2).
//!
//! The encoder core adds the pieces for the three candidates. Every candidate also pays 3 header
//! bits, and the fixed and dynamic ones pay the same extra bits:
//! - **dynamic:** the header entries' bits, then Σ frequency × dynamic length over both trees;
//! - **fixed:** Σ literal/length frequency × RFC 1951 §3.2.6 length, then 5 bits per distance;
//! - **stored:** 3 header bits, the padding to a byte boundary that the bits already written at the
//!   block's start leave, 32 bits of `LEN`/`NLEN`, and 8 bits per byte. A block longer than 65,535
//!   bytes is stored as several blocks, each after the first byte-aligned, so it pays 5 bits of
//!   padding.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on the encoders. A gated helper
//! is injected as its own file, so the body carries its own `IMPORT`s.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT compress
IMPORT bits
IMPORT collections

' Σ freqs[s] * lens[s].
FUNC __compress_treeBits(freqs AS List OF Integer, lens AS List OF Integer) AS Integer
  MUT total AS Integer = 0
  MUT s AS Integer = 0
  WHILE s < len(freqs)
    total = total + collections::get(freqs, s) * collections::get(lens, s)
    s = s + 1
  END WHILE
  RETURN total
END FUNC

' Σ literal/length frequency * fixed code length, plus five bits per distance.
FUNC __compress_fixedLitBits(litFreq AS List OF Integer, distFreq AS List OF Integer) AS Integer
  MUT total AS Integer = 0
  MUT s AS Integer = 0
  WHILE s < len(litFreq)
    total = total + collections::get(litFreq, s) * (collections::get(__COMPRESS_FIXED_LIT, s) MOD 16)
    s = s + 1
  END WHILE
  s = 0
  WHILE s < len(distFreq)
    total = total + collections::get(distFreq, s) * 5
    s = s + 1
  END WHILE
  RETURN total
END FUNC

' The bits of a `value * 32 + bit count` entry list.
FUNC __compress_headerBits(entries AS List OF Integer) AS Integer
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < len(entries)
    total = total + collections::get(entries, i) MOD 32
    i = i + 1
  END WHILE
  RETURN total
END FUNC

' The bits of storing `rawLen` bytes when `bitCount` bits (0..7) are pending at the block's start.
FUNC __compress_storedBits(rawLen AS Integer, bitCount AS Integer) AS Integer
  MUT chunks AS Integer = (rawLen + 65534) / 65535
  IF chunks = 0 THEN
    chunks = 1
  END IF
  LET pad AS Integer = (8 - (bitCount + 3) MOD 8) MOD 8
  RETURN chunks * 35 + pad + (chunks - 1) * 5 + 8 * rawLen
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "compress_block_cost",
        gate: HelperGate::WhenUsed(&["deflate", "zlibEncode", "gzipEncode"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
