//! `__crypto_argon2id` — shared private helper for the `crypto` package.
//!
//! Argon2id version 19 (RFC 9106) — the single body behind BOTH `crypto::argon2id`
//! spellings. The explicit-cost overload rewrites straight onto it; the profile
//! overload resolves its `Argon2Profile` to concrete costs and calls it
//! (`__crypto_argon2idProfile`), so there is one validation site and one fill site.
//!
//! Registered via `add_helper` under [`HelperGate::WhenUsed`] on `argon2id`, so a
//! program that imports `crypto` without calling `crypto::argon2id` carries none of
//! the BLAKE2b/Argon2 source. Renders in the helper section of the assembled source.
//! Body byte-significant (2-space indent → `.ncode` columns); do not reformat.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

#[rustfmt::skip]
const BODY: &str =
r#"IMPORT crypto
IMPORT bits
IMPORT collections

' Argon2id version 19 (RFC 9106): validate the cost parameters, build the block
' matrix, run `iterations` passes over it slice by slice and lane by lane, and hash the
' XOR of the last column down to `length` bytes. This is the single validation site and
' the single fill site for BOTH `crypto::argon2id` spellings. Four scratch blocks are
' carried past the end of the matrix (zero, the data-independent address input, and two
' address blocks) so the fill never needs a temporary outside itself.
FUNC __crypto_argon2id(password AS List OF Byte, salt AS List OF Byte, memoryKiB AS Integer, iterations AS Integer, parallelism AS Integer, length AS Integer) AS List OF Byte
  IF parallelism < 1 OR parallelism > 16777215 THEN
    FAIL error(77050002, "argon2id parallelism out of range")
  END IF
  IF iterations < 1 OR iterations > 4294967295 THEN
    FAIL error(77050002, "argon2id iterations out of range")
  END IF
  IF length < 4 THEN
    FAIL error(77050002, "argon2id length out of range")
  END IF
  IF len(salt) < 8 THEN
    FAIL error(77050002, "argon2id salt out of range")
  END IF
  IF memoryKiB < 8 * parallelism OR memoryKiB > 2097152 THEN
    FAIL error(77050002, "argon2id memoryKiB out of range")
  END IF
  LET blocks AS Integer = 4 * parallelism * (memoryKiB / (4 * parallelism))
  LET laneLen AS Integer = blocks / parallelism
  LET segLen AS Integer = laneLen / 4
  LET zeroOff AS Integer = blocks * 128
  LET inputOff AS Integer = zeroOff + 128
  LET scratchOff AS Integer = inputOff + 128
  LET addrOff AS Integer = scratchOff + 128
  LET h0 AS List OF Byte = __crypto_argon2H0(password, salt, memoryKiB, iterations, parallelism, length)
  MUT block AS List OF Integer = []
  MUT i AS Integer = 0
  WHILE i < 128
    block = collections::append(block, 0)
    i = i + 1
  END WHILE
  MUT mem AS List OF Integer = []
  i = 0
  WHILE i < blocks + 4
    mem = collections::append(mem, block)
    i = i + 1
  END WHILE
  MUT lane AS Integer = 0
  MUT j AS Integer = 0
  MUT seed AS List OF Byte = []
  MUT raw AS List OF Byte = []
  MUT base AS Integer = 0
  MUT k AS Integer = 0
  WHILE lane < parallelism
    j = 0
    WHILE j < 2
      seed = __crypto_copyBytes(h0)
      seed = __crypto_argon2Le32(seed, j)
      seed = __crypto_argon2Le32(seed, lane)
      raw = __crypto_argon2HPrime(seed, 1024)
      base = (lane * laneLen + j) * 128
      k = 0
      WHILE k < 128
        mem = collections::set(mem, base + k, __crypto_leLane(raw, k * 8))
        k = k + 1
      END WHILE
      j = j + 1
    END WHILE
    lane = lane + 1
  END WHILE
  MUT pass AS Integer = 0
  MUT slice AS Integer = 0
  MUT dataIndep AS Boolean = FALSE
  MUT startIdx AS Integer = 0
  MUT curr AS Integer = 0
  MUT prev AS Integer = 0
  MUT idx AS Integer = 0
  MUT rnd AS Integer = 0
  MUT refLane AS Integer = 0
  MUT refIdx AS Integer = 0
  WHILE pass < iterations
    slice = 0
    WHILE slice < 4
      lane = 0
      WHILE lane < parallelism
        dataIndep = pass = 0 AND slice < 2
        startIdx = 0
        IF pass = 0 AND slice = 0 THEN
          startIdx = 2
        END IF
        IF dataIndep THEN
          k = 0
          WHILE k < 128
            mem = collections::set(mem, inputOff + k, 0)
            k = k + 1
          END WHILE
          mem = collections::set(mem, inputOff, pass)
          mem = collections::set(mem, inputOff + 1, lane)
          mem = collections::set(mem, inputOff + 2, slice)
          mem = collections::set(mem, inputOff + 3, blocks)
          mem = collections::set(mem, inputOff + 4, iterations)
          mem = collections::set(mem, inputOff + 5, 2)
        END IF
        curr = lane * laneLen + slice * segLen + startIdx
        prev = curr - 1
        IF (curr MOD laneLen) = 0 THEN
          prev = curr + laneLen - 1
        END IF
        idx = startIdx
        WHILE idx < segLen
          IF (curr MOD laneLen) = 1 THEN
            prev = curr - 1
          END IF
          IF dataIndep THEN
            IF idx = startIdx OR (idx MOD 128) = 0 THEN
              mem = collections::set(mem, inputOff + 6, collections::get(mem, inputOff + 6) + 1)
              block = __crypto_argon2Fill(mem, zeroOff, inputOff, zeroOff, FALSE)
              k = 0
              WHILE k < 128
                mem = collections::set(mem, scratchOff + k, collections::get(block, k))
                k = k + 1
              END WHILE
              block = __crypto_argon2Fill(mem, zeroOff, scratchOff, zeroOff, FALSE)
              k = 0
              WHILE k < 128
                mem = collections::set(mem, addrOff + k, collections::get(block, k))
                k = k + 1
              END WHILE
            END IF
            rnd = collections::get(mem, addrOff + (idx MOD 128))
          ELSE
            rnd = collections::get(mem, prev * 128)
          END IF
          refLane = bits::sr(rnd, 32) MOD parallelism
          IF pass = 0 AND slice = 0 THEN
            refLane = lane
          END IF
          refIdx = __crypto_argon2IndexAlpha(pass, laneLen, segLen, slice, idx, bits::band(rnd, 4294967295), refLane = lane)
          base = curr * 128
          block = __crypto_argon2Fill(mem, prev * 128, (refLane * laneLen + refIdx) * 128, base, pass <> 0)
          k = 0
          WHILE k < 128
            mem = collections::set(mem, base + k, collections::get(block, k))
            k = k + 1
          END WHILE
          curr = curr + 1
          prev = prev + 1
          idx = idx + 1
        END WHILE
        lane = lane + 1
      END WHILE
      slice = slice + 1
    END WHILE
    pass = pass + 1
  END WHILE
  MUT cb AS List OF Byte = []
  k = 0
  WHILE k < 128
    rnd = 0
    lane = 0
    WHILE lane < parallelism
      rnd = bits::bxor(rnd, collections::get(mem, (lane * laneLen + laneLen - 1) * 128 + k))
      lane = lane + 1
    END WHILE
    cb = __crypto_appendLeLane(cb, rnd)
    k = k + 1
  END WHILE
  RETURN __crypto_argon2HPrime(cb, length)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "crypto_argon2id",
        gate: HelperGate::WhenUsed(&["argon2id"]),
        body: Some(BODY),
        import_name: None,
        natively_called: false,
    });
}
