# Audit: strings:: & Unicode runtime — functionality & security review

Last updated: 2026-07-01

Read-only audit. **No `src/**` changes** were made (ongoing x86_64 work owns the
tree). This document records findings and ordered fix options only.

Re-verified 2026-07-01 against the live tree (each finding re-read in source).
Line numbers below are as of that re-read; the earlier draft's numbers had drifted
by ~100–150 lines. One prior finding (old #5, fold-vs-runtime `find` divergence)
was **retracted** — the fold path it described does not exist (see #5).

## Sources of truth cross-linked

- `mfb spec unicode tables-and-algorithms` — embedded utf8proc tables, two-stage
  property lookup, grapheme/NFC/case algorithms (`src/docs/spec/unicode/01_*`).
- `mfb spec unicode strings-model` — scalar/grapheme/byte indexing, mid/find/trim/
  split semantics (`src/docs/spec/unicode/02_*`).
- `mfb man strings` — per-function API + error codes. The strings error table lists
  exactly three: ErrIndexOutOfRange `77050001`, ErrInvalidArgument `77050002`,
  ErrNotFound `77050004` (`src/docs/man/builtins/strings/package.md:53-55`). (The
  earlier draft cited "ErrEncoding `77050003`" — wrong on both counts: `77050003` is
  `ErrInvalidFormat`, and the byte→String ingress encoding error is `ErrEncoding`
  `77020004`, raised by `toString(List OF Byte)`, not by any `strings::` member.)
- `mfb spec memory heap-values` — in-memory `String` byte layout (u64 length prefix
  + bytes + NUL).

## Scope reviewed

- Compile-time constant-fold path: `src/builtins/strings.rs`, `src/unicode_backend.rs`.
- Table generation/serialization: `src/unicode_runtime_tables.rs`
  (`third_party/utf8proc/utf8proc_data.c`).
- Runtime AArch64 emitters: `src/target/shared/code/private/unicode.rs`,
  `src/target/shared/code/builder_strings_builtins.rs`,
  `src/target/shared/code/builder_search.rs`,
  `src/target/shared/code/builder_strings.rs`.
- String ingress / UTF-8 validation: `toString(List OF Byte)`
  (`builder_strings.rs:857` `emit_byte_list_to_string_value`), `_mfb_rt_validate_utf8`
  (`codegen_utils.rs:215` `emit_validate_utf8`),
  fs/net/tls/io read helpers, `encoding` package.
- Table-embedding gate: `module_analysis.rs:603` (`module_uses_unicode_runtime_tables`).

## What is clean (verified, no action)

- **`String` UTF-8 invariant holds at every ingress.** Every runtime byte→String
  constructor validates RFC-3629 well-formed UTF-8 (rejects overlong, surrogates
  D800–DFFF, > U+10FFFF, truncation) before the bytes become a `String`:
  `toString(List OF Byte)` (`builder_strings.rs:887-1053`, incl. the E0/ED
  overlong+surrogate and F0/F4 overlong+>U+10FFFF special cases), fs/net/tls/io reads via
  `_mfb_rt_validate_utf8`, and every `encoding` decoder via `__encoding_utf8Valid` /
  `__encoding_fromCodepoint`. There is **no** `strings::fromBytes`/`chr` byte→String
  bypass. `strings::toBytes` is String→bytes only.
- **NFC canonical-ordering bubble sort and recompose stay in-bounds.** Sort index is
  guarded `idx+1 < count` and decrements only when `idx > 0` (no underflow); compose
  write cursor `≤ read cursor < count` (`builder_strings_builtins.rs:620-739`).
- **Two-pass allocations are deterministic.** Count-pass and write-pass for
  case-map / normalizeNfc / graphemes share identical width (`emit_utf8_encoded_width`)
  and encode (`emit_utf8_encode_next`) thresholds, so the write pass emits exactly the
  counted byte length. (Coupling is un-asserted — see #9. Both helpers verified to
  share the 128/2048/65536 thresholds: `private/unicode.rs:266` and `:298`.)
- **`mid` wrap check is present and correct.** `start<0`/`count<0` rejected (signed
  `b.lt`); `start+count` overflow rejected with unsigned `b.lo` (`builder_search.rs:604-610`).
  The runtime `mid` is the only `mid` path — `unicode_backend::mid`'s `checked_add`
  (`unicode_backend.rs:80-82`) is never called (see #5).
- **`left`/`right`/`graphemeAt` bounds are correct** (negatives via signed `b.lt`,
  high index `>= count` via `b.ge`, both → ErrIndexOutOfRange).
- **Table-embedding gate is correct.** A dynamic call to any of the 7 table-consuming
  builtins (upper/lower/caseFold/normalizeNfc/graphemes/graphemeAt/graphemesCount)
  embeds the tables even when a sibling constant call is folded away.

---

## Findings

### 1. [CRIT] `strings::repeat(value, times)` allocation-size multiply overflows → heap overflow

`lower_strings_repeat` checks `times >= 0` (signed `b.lt`, good) but then computes the
result size as a **truncating** 64-bit multiply with no overflow guard, while the copy
loop iterates the full untruncated `times`. `total = len * times` wraps mod 2^64; the
arena allocation is `total + 9` (small), but the outer copy loop reloads the original
`times` and writes `len` bytes per iteration — an attacker-controlled "allocate small,
write huge" heap overflow. Reachable from ordinary source with a small string and a
large `times` (e.g. 32-byte value, `times = 2^59` → `len*times = 2^64` → `total = 0`).

```
// src/target/shared/code/builder_strings_builtins.rs:1825-1834 (setup)
compare_immediate(times_rem,"0"); branch_lt(&invalid); // times >= 0  (only guard)
load_u64(len, val_ptr, 0);                             // len
multiply_registers(total, len, times_rem);            // total = len*times  (TRUNCATES, no ovf check)
add_immediate(return_register(), total, 9);           // alloc total+9
// copy loop (:1856-1879): times reloaded from slot = FULL untruncated times;
//   inner writes `len` bytes each outer iteration
```

- **O1 (most secure):** Compute the high 64 bits of the product with `umulh`; if
  nonzero (overflow) OR `total + 9` carries, raise `ErrInvalidArgument` (or
  ErrAllocation) *before* `arena_alloc`. Same guard shape everywhere a user Integer
  feeds a size multiply. Preserves all valid uses; only impossible sizes are rejected.
- **O2 (secured, no functionality lost):** Bound `times` against `arena` free capacity
  before multiplying — reject when `times > remaining_capacity / max(len,1)`. Rejects
  exactly the requests that could never allocate anyway; no valid program loses.
- **O3 (secure, functionality lost):** Cap `times` (and `len*times`) at a fixed ceiling
  (e.g. `2^31`) and raise on exceed. Simple, but forbids legitimately large repeats.

**Selected fix: O1.** It is the only option that is both secure and loses no defined
functionality: the `umulh`/carry guard rejects *only* products that cannot be
represented (which today corrupt the heap — undefined behavior, not defined
functionality), while every representable size still flows to the existing
`arena_alloc` path (which already raises a catchable allocation error when genuinely
too big). O3 forbids valid large repeats (functionality lost); O2 makes success depend
on current arena free space (a valid `repeat` could fail or succeed run-to-run — a
semantic change). The overflow raises the catchable `ErrInvalidArgument` (`77050002`),
the code the man page already assigns to `repeat`'s argument rejections, so no new
error identity is introduced. Requires a one-line note in `mfb man strings` /
`mfb spec unicode strings-model` that `repeat` also rejects representationally-impossible
sizes. Covered by `tests/security/unicode-01-repeat-overflow`.

---

### 2. [CRIT] `strings::padLeft`/`padRight` size multiply+add overflows → heap overflow

Same class as #1. `width >= 0` is checked, but `pad_count = max(0, width - scalarLen)`
(up to ~2^63) then feeds a truncating `pad_count * padLen` and an unchecked
`valueLen + …`. The pad-writing loop iterates the full untruncated `pad_count` writing
`padLen` bytes each. With a 4-byte padChar and large `width`, `pad_count * padLen` wraps
to a small allocation followed by an unbounded write. Trigger e.g.
`strings::padLeft("x", 2^62, "😀")`.

```
// src/target/shared/code/builder_strings_builtins.rs:2032-2037
multiply_registers(scratch12, scratch10, scratch11); // pad_count * padLen  (TRUNCATES, no ovf check)
add_registers(scratch11, scratch9, scratch12);       // total = valueLen + product  (no carry check)
add_immediate(return_register(), scratch11, 9);      // alloc total+9
// copy_pads (:2090-2118): outer loops full pad_count, inner writes padLen bytes
```

- **O1 (most secure):** `umulh` overflow check on `pad_count * padLen` **and** a
  checked-add on `valueLen + product` and the `+9`; on overflow raise ErrInvalidArgument
  before `arena_alloc`. Factor a shared `emit_checked_alloc_size` helper reused by #1/#2
  and the #8 sites.
- **O2 (secured, no functionality lost):** Reject `width` (hence `pad_count`) that
  exceeds what the arena can hold given `padLen`; those requests cannot succeed anyway.
- **O3 (secure, functionality lost):** Cap `width` at a fixed maximum; rejects
  legitimately huge pad widths.

**Selected fix: O1** (shared `emit_checked_alloc_size` with #1). Same reasoning as #1:
only unrepresentable sizes are rejected (via `umulh` on `pad_count*padLen` plus a
checked add on `valueLen + product` and the `+9`), raising the catchable
`ErrInvalidArgument` (`77050002`) — the code the man page already assigns to `pad`'s
argument rejections. Valid pad widths are unaffected; O2 couples success to arena
state, O3 forbids valid large widths. Covered by
`tests/security/unicode-02-pad-overflow`.

---

### 3. [HIGH] Runtime UTF-8 decode + property lookup are unchecked; memory safety rests solely on the "every String is valid UTF-8" invariant (no defense-in-depth)

`emit_utf8_decode_next` masks continuation bytes but does **not** verify them, does not
reject surrogates / overlongs / lead bytes 0xF5–0xF7, and unconditionally reads
`cursor[1..3]` without a remaining-length check. `emit_unicode_property_lookup` then does
a raw two-stage walk — `stage1[cp>>8]` with **no** `cp <= 0x10FFFF` guard. A malformed
4-byte lead can yield `cp` up to `0x1FFFFF`, so `cp>>8 = 0x1FFF = 8191` reads ~7.7 KB past
`stage1` (4352 u16), and that poisoned base cascades into wild `stage2` and `properties`
reads. **Not reachable today** because all four+ ingress validators enforce
`cp <= 0x10FFFF` (see "What is clean"), so this is a *latent* CRIT gated to HIGH: the
entire Unicode subsystem's memory safety is a single-point-of-failure on an invariant
maintained independently by every String producer, with zero self-defense in the
consumer.

```
// src/target/shared/code/private/unicode.rs:147-151 (emit_unicode_property_lookup)
lsr  x6, cp, #8          // NO check that cp <= 0x10FFFF
lsl  x6, x6, #1
add  x7, <stage1_base>, x6
ldrh x6, [x7]            // OOB read if cp > 0x10FFFF
// decoder (:63-136) never validates continuation bytes / surrogates / remaining length
```

- **O1 (most secure):** Make the consumer self-defending. In `emit_utf8_decode_next`,
  after computing `cp`, clamp/validate: if `cp > 0x10FFFF` or in `0xD800..=0xDFFF` or a
  continuation byte is out of `0x80..=0xBF`, substitute U+FFFD (or branch to a defined
  error). Then the table walk is safe by construction regardless of caller discipline.
  No functionality lost (valid strings unaffected).
- **O2 (secured, no functionality lost):** Add a cheap upper-bound guard *only* at
  `emit_unicode_property_lookup` — mask `cp` to 21 bits and branch any `cp > 0x10FFFF`
  to the "no property" / boundclass-OTHER path. Cheaper than O1, still converts the OOB
  read into a defined result; leaves the decoder permissive.
- **O3 (defense-in-depth, no code change to consumer):** Add a debug/assert build that
  re-validates strings at the Unicode-builtin boundary, plus a standing test that a
  crafted invalid-UTF-8 String (only constructible if an ingress validator regresses)
  is caught. Documents the invariant but leaves the latent OOB if the invariant breaks
  in a release build — weakest option.

**Selected fix: O1.** It is the only option that makes the subsystem safe *by
construction* rather than by trusting an external invariant, and it loses no defined
functionality: valid strings decode identically (every scalar is well-formed, so no
substitution ever fires), and the malformed inputs O1 guards against cannot occur on a
valid String today — so no defined behavior changes. Validating in the decoder
(continuation bytes ∈ `0x80..=0xBF`, reject surrogates/overlongs/lead `0xF5..`,
substitute U+FFFD) also closes the unchecked `cursor[1..3]` reads, which O2's
property-lookup-only clamp leaves open. O3 leaves the OOB latent in release builds.
This is purely internal hardening — no `mfb man`/`mfb spec` change. Its *reachable*
guard — the ingress invariant that keeps it latent — is exercised by
`tests/security/unicode-03-ingress-utf8-invariant` (a source program cannot construct
an invalid-UTF-8 String, so the OOB path itself is untriggerable from MFBASIC; the test
asserts the ingress validator that upholds the invariant).

---

### 4. [MED] `strings::count(value, needle)` under-reads when `needle` is longer than `value`

The loop limit is `cursor <= valueLen - needleLen` computed as an **unsigned**
`subtract_registers` with no prior `needleLen > valueLen` guard. When `needleLen >
valueLen`, `valueLen - needleLen` underflows to a huge unsigned value, so the
`branch_hi(done)` termination never fires; the loop then reads `needleLen` bytes past the
`value` buffer on each iteration — an out-of-bounds read and near-unbounded loop. The
sibling `contains` guards exactly this (`compare_registers(needleLen,valueLen); branch_hi(false)`
when `needleLen > valueLen`, `:993-994`); `count` is missing that guard. Trigger:
`strings::count("ab", "abcdef")`.

```
// src/target/shared/code/builder_strings_builtins.rs:1610-1612
subtract_registers(scratch13, scratch9, scratch10);  // valueLen - needleLen  (underflows if needle longer)
compare_registers(scratch14, scratch13);
branch_hi(&done);                                    // never taken -> OOB read loop
```

- **O1 (most secure) == O2 (no functionality lost):** Add the `contains`-style guard
  before the loop: `compare_registers(needleLen, valueLen); branch_hi(&done)` (returns
  count 0). One instruction pair; matches documented semantics (needle longer than value
  → 0 occurrences) and fixes the OOB read with zero behavior change for valid inputs.
- **O3:** n/a — the correct behavior loses nothing.

**Selected fix: O1.** Secure and lossless are the same option here: the guard returns
the already-correct answer (`0`) for `needleLen > valueLen` and eliminates the OOB read
+ runaway loop, with zero change for any input that currently behaves. No spec/man
change. Covered by `tests/security/unicode-04-count-underread`.

---

### 5. [RETRACTED] `find`/`mid` fold-vs-runtime divergence — the fold path does not exist

**Retracted after re-verification (2026-07-01).** The earlier draft claimed that
constant-folding `find` (via `unicode_backend::find` → `byte_offset_for_scalar_index`)
turns an out-of-range `start` into a compile-time build error, diverging from the
catchable runtime `ErrIndexOutOfRange`. That divergence cannot occur: **`find` and `mid`
are never constant-folded.**

`unicode_backend::find` and `unicode_backend::mid` are defined but have **zero callers**
anywhere in `src/**` (the module carries `#![allow(dead_code)]`, which masks the warning).
Every constant-fold site — `native_strings_package_static_string_value`
(`validate.rs:515-532`), `static_strings_package_string`
(`builder_strings_package.rs:116-134`), and the twins in `plan/symbols.rs` /
`code/type_utils.rs` — folds only `upper`/`lower`/`caseFold`/`normalizeNfc` (plus
`graphemes` via a separate helper). None match `find`/`mid`/`strings.find`/`strings.mid`.
So both are always lowered to the runtime path (`lower_find`/`lower_mid`), which raises
the catchable `77050001`. There is a single evaluation path and no divergence.

Verification: `grep -rn 'unicode_backend::\(find\|mid\)' src` → no matches; every file
importing `unicode_backend` was checked for a bare `find(`/`mid(` call — none.

- **Only residual (LO, non-security):** `unicode_backend::{find,mid}` (and their
  `byte_offset_for_scalar_index`/`scalar_index_for_byte_offset` support that `mid`/`find`
  reach) are dead code kept alive only by their own unit tests and `#![allow(dead_code)]`.
  They read like a live fold path and misled this audit. **O1:** delete them (and drop
  the blanket `#![allow(dead_code)]` so future dead helpers surface). **O2:** if kept as
  a reference oracle, add a comment stating they are test-only and not a fold path.
- **If folding `find`/`mid` is ever added**, the original concern re-applies: fold the
  error condition to the runtime path (don't turn a catchable `77050001` into a build
  error). Recorded here so the trap isn't reintroduced silently.

**Selected fix: O1** (delete the dead `unicode_backend::{find,mid}` and the blanket
`#![allow(dead_code)]`). Lossless — the helpers have no callers — and it removes the
very thing that misled this audit. Not a security change. The single-path guarantee it
documents (constant `find`/`mid` out-of-range args raise the catchable runtime error,
never a build error) is locked by `tests/security/unicode-05-find-fold-parity`.

---

### 6. [LO] `find` negative `start` is not explicitly range-checked (correct-by-accident)

Runtime `find` never tests `start >= 0`; a negative `start` (huge unsigned) simply never
equals the incrementing scalar index, so the locate loop exhausts and raises
`ErrIndexOutOfRange`. The outcome is safe, but it (a) does an O(n) walk on an obviously
bogus argument, (b) reports `ErrIndexOutOfRange` where `ErrInvalidArgument` is arguably
correct, and (c) is inconsistent with the *list* `find` (explicit signed `>= 0` check
via `branch_ge`, `builder_search.rs:273-274`) and with `mid` (explicit negativity checks,
`:604-607`).

```
// src/target/shared/code/builder_search.rs:164-165  (no start>=0 check; relies on loop exhaustion)
compare_registers(scalar_index, start); branch_eq(&start_ready);
// locate loop (:163-176) exits to invalid_start only when `remaining` hits 0
```

- **O1 (most secure) == O2:** Add an explicit signed `start >= 0` check up front,
  matching list-`find`/`mid`; decide deliberately whether negative → ErrInvalidArgument
  (consistent with pad/left/right) or keep ErrIndexOutOfRange (consistent with the man
  page, which scopes `find`'s range error to `0..scalar_len`). No valid input affected.
- **O3:** n/a.

**Selected fix: O1, keeping `ErrIndexOutOfRange` (`77050001`).** This is the choice that
changes no defined behavior: the man page already says `find` raises `77050001` when
`start` is "outside `0` through the scalar length" — a negative `start` is outside that
range, so `77050001` is the spec-consistent code, and it is exactly what the current
loop-exhaustion path already returns. The explicit up-front `start >= 0` check just
makes that O(n)-walk-then-fail into an immediate raise (and aligns `find` with list-`find`
and `mid`), with an identical observable result. Switching to `ErrInvalidArgument` was
tempting for cross-consistency but would change the error code the man page documents —
rejected to honor the "no spec change" constraint. Covered by
`tests/security/unicode-06-find-negative-start`.

---

### 7. [LO] `padChar` "exactly one scalar" check is byte-structural, not UTF-8-validating (defense-in-depth only)

`padLeft`/`padRight` accept `padChar` when it has exactly one non-continuation byte
(`byte & 0xC0 != 0x80`), which correctly rejects empty/multi-scalar pads but would accept
a structurally-malformed lead (e.g. a lone `0xFF`, or a truncated multibyte lead) and
copy it verbatim into the output, yielding an invalid-UTF-8 String. **Not reachable
today** because `padChar` is itself a `String` and therefore already valid UTF-8 by the
ingress invariant (#3) — so a lone `0xFF` padChar cannot exist. Flagged as defense-in-depth
and as a second consumer that trusts, rather than re-checks, the invariant.

```
// src/target/shared/code/builder_strings_builtins.rs:1981-1982
compare_immediate(scratch14,"1"); branch_ne(&invalid);   // non-continuation byte count == 1
// the count loop (:1966-1980) tests only (byte & 0xC0) != 0x80 — structural, not UTF-8-validating
```

- **O1 (most secure):** Validate `padChar` as a single well-formed scalar (decode one
  scalar, require it to consume the whole padChar and be `<= 0x10FFFF`, non-surrogate).
  Redundant today, but self-defending if #3's invariant ever regresses.
- **O2 (secured, no functionality lost):** Leave as-is but add a regression test that a
  crafted invalid padChar is rejected (guards the invariant from the pad side).
- **O3:** n/a.

**Selected fix: O1.** Validating `padChar` as one well-formed scalar is lossless (a
single valid scalar — the only thing constructible from source — still passes) and it
makes the check enforce exactly what the man page already promises ("`padChar` is not
exactly one Unicode scalar value" → `ErrInvalidArgument`), so it is spec-tightening, not
spec-changing. The malformed-lead case O1 additionally rejects is unreachable from
MFBASIC today (padChar is a String, hence already valid UTF-8), so O1 is pure
defense-in-depth with no behavior change. The *reachable* contract (empty and
multi-scalar padChar rejected, single-scalar accepted) is covered by
`tests/security/unicode-07-padchar-scalar`.

---

### 8. [LO] Remaining size-multiplies lack overflow guards (bounded today, not self-defending)

`toBytes` (`(ENTRY_SIZE+1)*count`, `builder_strings_builtins.rs:217`), grapheme
collection sizing (`COLLECTION_ENTRY_SIZE*count + header + byteLen`, `:88-98`), and
normalizeNfc's temp buffer (`decomposed_scalar_count * 8`, `:542-546`) all use truncating
multiplies without overflow checks.
Unlike #1/#2 the multiplier is **not** an attacker-supplied Integer — it is derived from
the (already-in-memory, arena-bounded) string length — so a wrap is unreachable on real
hardware. Recorded so the same `emit_checked_alloc_size` helper from #1/#2 O1 is applied
uniformly.

- **O1 (most secure):** Route every arena-size computation through one checked helper
  (`umulh`/carry → allocation error). Uniform, self-defending, no valid input affected.
- **O2:** Leave as-is (bounded by arena capacity) but add a comment/debug-assert noting
  the bound each relies on.
- **O3:** n/a.

**Selected fix: O1** (route these sites through the same `emit_checked_alloc_size`
helper landed for #1/#2). Lossless (the multiplier is a length already resident in an
arena-bounded buffer, so the guard never fires on real input) and it removes the last
unchecked size multiplies for uniformity — one helper, one invariant, everywhere. The
reachable behavior (these sizings stay correct for ordinary multi-byte strings) is
covered by `tests/security/unicode-08-tobytes-roundtrip`.

---

### 9. [LO] Count-pass/write-pass equality is a correctness-critical coupling with no runtime assertion

The two-pass allocators (case-map, normalizeNfc, graphemes) are safe only because the
counting pass and writing pass compute identical lengths from identical table lookups.
This holds today — `emit_utf8_encoded_width` (`private/unicode.rs:266`) and
`emit_utf8_encode_next` (`:298`) were re-verified to branch on the same 128/2048/65536
codepoint thresholds — but a future edit to only one of them (or a divergent table read)
would silently turn it into a heap overflow with no guard.

- **O1 (most secure):** In debug builds, have the write pass track bytes written and
  assert it equals the allocation size (trap on mismatch). Cheap insurance; no release
  cost if gated.
- **O2 (secured):** Add a golden/ULP-style test asserting the counted vs written length
  for a corpus of expanding scalars (ß→SS, İ lowering, Hangul, emoji ZWJ).
- **O3:** n/a.

**Selected fix: O1** (debug-build write-vs-alloc assertion), plus O2's corpus test as
standing regression coverage. O1 is zero release cost (gated to debug) and loses no
functionality — it only traps a future count/write divergence that would otherwise be a
silent heap overflow. O2 alone can't prove the coupling for *every* future edit, so it
complements rather than replaces O1. The expanding-scalar correctness corpus (which a
count/write divergence would break) is covered by
`tests/security/unicode-09-expanding-two-pass`.

---

## Suggested remediation order

1. **#1, #2 (CRIT):** land the shared `emit_checked_alloc_size` (umulh + checked-add →
   ErrInvalidArgument) and apply to `repeat`/`pad`; add `func_strings_repeat_invalid` /
   `func_strings_padLeft_invalid` runtime tests proving the overflow now raises.
2. **#4 (MED):** one-line `needleLen > valueLen` guard in `count` + regression test.
3. **#3 (HIGH-latent):** self-defend the decoder/property lookup (O1) — highest
   structural value; converts the whole subsystem from "safe by external invariant" to
   "safe by construction."
4. **#6, #7, #8, #9 (LO cleanup):** explicit `find` negativity check, padChar UTF-8
   validation, uniform checked alloc sizing, two-pass assert.
5. **#5 (LO, non-security):** delete the dead `unicode_backend::{find,mid}` helpers (or
   mark them test-only) and drop the blanket `#![allow(dead_code)]` so the next dead
   helper surfaces.

All fixes above are `src/**` changes and must wait for the x86_64 work to release the
tree, then follow the standard gate: test-first (`func_strings_*` valid/invalid), full
overload coverage, `scripts/test-accept.sh` green.

## Security test cases (added this pass)

One test case per finding lives under `tests/security/unicode-0N-*` (source +
`project.json` + a `golden/*.run` documenting the expected secure output). They are
**not yet wired into `scripts/test-accept.sh`** — that harness only iterates top-level
`tests/*` with a `project.json`, so it does not descend into `tests/security/`. Wiring
(and generating the `build.log`/AST/IR goldens by running the harness) is deferred until
the tree is free and the fixes land; each is authored test-first, so it asserts the
*post-fix* secure behavior and will fail against today's vulnerable code.

| Test | Finding | Asserts |
| --- | --- | --- |
| `unicode-01-repeat-overflow` | #1 | `repeat(v, 2^59)` on a 32-byte `v` raises catchable `77050002`, no heap overflow |
| `unicode-02-pad-overflow` | #2 | `padLeft(v, 2^62, "😀")` raises catchable `77050002`, no heap overflow |
| `unicode-03-ingress-utf8-invariant` | #3 | `toString` of overlong / surrogate / >U+10FFFF / truncated / bad-continuation byte lists each raises `77020004` (the invariant that keeps the decoder OOB latent) |
| `unicode-04-count-underread` | #4 | `count("ab", "abcdef")` returns `0`, no OOB read / runaway loop |
| `unicode-05-find-fold-parity` | #5 | constant-arg `find`/`mid` out-of-range raises the catchable runtime error, never a build error (single evaluation path) |
| `unicode-06-find-negative-start` | #6 | `find(v, needle, -1)` raises catchable `77050001`, no O(n) bogus walk crash |
| `unicode-07-padchar-scalar` | #7 | empty and multi-scalar `padChar` rejected with `77050002`; single scalar accepted |
| `unicode-08-tobytes-roundtrip` | #8 | `toBytes` of a multi-byte string round-trips to the exact bytes (derived-length sizing correct) |
| `unicode-09-expanding-two-pass` | #9 | expanding maps (`upper` ß→SS, `lower` İ, NFC compose, ZWJ `graphemes`) produce correct output/length (a count/write divergence would break these) |
