# bug-339: MFBASIC-source stdlib cleanup cluster — generated `vector_package.mfb` has DRIFTED from its generator, plus cross/within-package duplication, dead code, naming and DOC gaps

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW — **except item A1, which is a build-integrity defect, not tidiness**
Class: Other (cleanup) — **A1 flagged: generated-artifact integrity**

Status: Open
Regression Test: `scripts/` CI regeneration check (new, item A1) + per-item behavioral tests under `tests/rt-behavior/<pkg>/`

A cleanup cluster over the fourteen MFBASIC-source stdlib files in
`src/builtins/*.mfb` — the parts of the stdlib written *in* MFBASIC and embedded
into the compiler via `include_str!`. Most items are duplication, dead code, and
naming/documentation inconsistency, all LOW.

**One item is not cleanup.** `src/builtins/vector_package.mfb` carries a banner
declaring itself machine-generated and forbidding hand edits, but the checked-in
file no longer matches what `scripts/gen_vector_package.py` produces. A landed
performance optimization lives only in the artifact. Re-running the generator —
which its own banner instructs a maintainer to do — silently reverts that
optimization, and **nothing in the tree detects it**: the generator is not
invoked by `build.rs`, by any Makefile, or by CI, and there is no `.gitattributes`
marking the file as generated. The single correct behavior a fix produces:
running each generator reproduces its checked-in artifact byte-for-byte, and CI
fails if it does not.

References:

- `scripts/gen_vector_package.py`, `scripts/gen_regex_unicode.py` — the two generators.
- `planning/old-plans/plan-39-benchmark-perf.md` §C1/C2 — the FSQRT-seeded isqrt
  whose body now lives only in the artifact (commit `28548d75`).
- `planning/old-plans/goal-06-full-source-review.md:99-101` — goal-06 *excused*
  both generated files from review precisely because they are "machine-generated",
  which is how the drift survived a full source review.
- `bugs/bug-316-regex-posix-classes-never-match.md` — item D3 here is the same
  defect; see the cross-reference note.
- `bugs/bug-306-mfb-stdlib-low-cluster.md`, `bugs/bug-300-docs-deadcode-low-cluster.md`
  — sibling low-cluster documents over the same sources.
- Found during the cleanup review of `src/builtins/*.mfb` (Agent 18).

## Current State

### The generator drift, verbatim

```
$ python3 scripts/gen_vector_package.py > /tmp/regen_vector.mfb
$ diff -u src/builtins/vector_package.mfb /tmp/regen_vector.mfb
--- src/builtins/vector_package.mfb	2026-07-18 07:58:17
+++ /tmp/regen_vector.mfb	2026-07-18 08:32:24
@@ -66,28 +66,18 @@

 ' ---- shared integer helpers ----

-' Deterministic floor integer square root, n >= 0. plan-39 C2: seed from the
-' hardware Float sqrt, then correct to the exact floor with integer steps that
-' use division only (never seed*seed), so they cannot overflow and converge in
-' O(1) from the Float seed — replacing the Newton iteration that started from
-' x = n. The result is the exact floor(sqrt(n)), identical to the old method.
+' Deterministic floor integer square root (Newton's method), n >= 0.
 FUNC __vector_isqrtFloor(n AS Integer) AS Integer
   IF n <= 0 THEN
     RETURN 0
   END IF
-  MUT seed AS Integer = toInt(math::sqrt(toFloat(n)))
-  IF seed < 0 THEN
-    seed = 0
-  END IF
-  ' Bring seed down to a lower bound: seed*seed > n  <=>  seed > n / seed.
-  WHILE seed > 0 AND seed > n / seed
-    seed = seed - 1
-  WEND
-  ' Climb while (seed+1)^2 <= n  <=>  (seed+1) <= n / (seed+1).
-  WHILE seed + 1 <= n / (seed + 1)
-    seed = seed + 1
+  MUT x AS Integer = n
+  MUT y AS Integer = (x + 1) / 2
+  WHILE y < x
+    x = y
+    y = (x + n / x) / 2
   WEND
-  RETURN seed
+  RETURN x
 END FUNC

 ' Integer square root rounded half away from zero (n >= 0). The exact half
```

- Checked-in: 1529 lines. Regenerated: 1519 lines.
- **This is the entire diff.** Every other line of the 1519 regenerates exactly.
  The drift is confined to `__vector_isqrtFloor`: 17 checked-in lines against 7
  generated ones.
- Direction of loss: regenerating **replaces** the plan-39 C2 FSQRT-seeded isqrt
  (`src/builtins/vector_package.mfb:69-90`) with the pre-plan-39 Newton iteration
  still encoded at `scripts/gen_vector_package.py:47-71`. The optimization is
  destroyed, the program still compiles, every test still passes, and the only
  signal is a benchmark regression nobody is watching.

The contrasting file is clean:

```
$ python3 scripts/gen_regex_unicode.py > /tmp/regen_regex.mfb
$ diff -q src/builtins/regex_unicode.mfb /tmp/regen_regex.mfb   # (no output)
$ shasum -a 256 src/builtins/regex_unicode.mfb /tmp/regen_regex.mfb
a9075c34a1655cbb9f74fd249f730f46ef53dfa727b790e0699b724b1b1f483c  src/builtins/regex_unicode.mfb
a9075c34a1655cbb9f74fd249f730f46ef53dfa727b790e0699b724b1b1f483c  /tmp/regen_regex.mfb
$ python3 -c "import unicodedata; print(unicodedata.unidata_version)"
16.0.0
```

Byte-identical — **on this host**. That is a weaker guarantee than it looks; see A3.

### Verification method for the rest

Every item below was checked against the source. Claims that did not hold were
corrected in place or dropped; three leads were downgraded and are recorded as
such (A3 nuance, B2 nuance, C1/E1 corrections) so they are not re-litigated.

---

## Items

### (A) Generated-file integrity

#### A1 — `vector_package.mfb` has drifted from `gen_vector_package.py`; regenerating reverts landed perf work
- Artifact: `src/builtins/vector_package.mfb:3-4` (the "GENERATED … do not edit by
  hand" banner), `:69-90` (the drifted `__vector_isqrtFloor`).
- Generator: `scripts/gen_vector_package.py:47-71` (the stale Newton body it still
  emits), `:460` (where it writes the do-not-edit banner into its own output).
- Commit `28548d75` (plan-39 C1/C2) edited the artifact and not the generator.
- **This is the headline.** A maintainer who obeys the banner destroys a landed
  optimization with no test signal. See the diff above.

> **Fixed 2026-07-22.** Resolved in the *correct* direction: the generator's
> `HELPERS` block (`scripts/gen_vector_package.py`) now emits the plan-39-C2
> FSQRT-seeded `__vector_isqrtFloor`, so `python3 scripts/gen_vector_package.py`
> reproduces the checked-in `src/builtins/vector_package.mfb` **byte-for-byte**
> (verified: `diff -q` clean). The artifact is unchanged, so the compiler embeds
> the same optimized source and nothing downstream moves. Not "fixed" by
> regenerating the artifact — that would have reverted the optimization, which is
> the whole defect. The remaining B/C/D cleanup items in this doc are still open.

#### A2 — Neither generator is invoked by anything, so drift cannot be caught
- `build.rs` exists at the repo root and does **not** call either script.
- Repo-wide grep for `gen_vector_package` / `gen_regex_unicode` outside the scripts
  themselves finds only prose: `tools/math-kernels/capture.sh:14`,
  `planning/old-plans/goal-06-full-source-review.md:99-101`,
  `src/builtins/regex.rs:106`, and the banners at
  `src/builtins/vector_package.mfb:3` and `src/builtins/regex_unicode.mfb:2`.
- No Makefile target, no `build.rs` call, no CI step. `.github/workflows/` contains
  exactly one workflow, `coverage.yml`, which does not regenerate.
- There is **no `.gitattributes` anywhere in the repo** (confirmed by `find`), so
  neither output is marked `linguist-generated` and neither gets a collapsed diff
  or a reviewer hint.
- **Fixed 2026-07-22.** Added `scripts/check-generated.sh` — re-runs each
  generator and `cmp`s its output against the checked-in artifact — and wired it
  as the first step of `.github/workflows/coverage.yml` (before the Rust
  toolchain, since it only needs `python3`). Drift in either
  `vector_package.mfb` or `regex_unicode.mfb` now fails CI with a diff and the
  regenerate command. A3's soft Unicode pin is thereby hardened too: a host
  CPython on a different Unicode version makes the regex check fail loudly rather
  than silently reshaping the table.
- This absence is the mechanism by which A1 went unnoticed through a full source
  review that explicitly excused both files as machine-generated.

#### A3 — `gen_regex_unicode.py` soft-pins the Unicode version to the host CPython
- `scripts/gen_regex_unicode.py:9-13`: "Pinned Unicode version: whatever
  `python3 -c "import unicodedata"` reports; this script records it in the
  generated header."
- **Nuance (correction to the raw lead):** the artifact is byte-identical *today*
  only because this host ships CPython 3.14.5 / UCD 16.0.0, matching whatever host
  last generated it. A contributor on an older or newer CPython regenerating this
  file produces a different 4,109-line table — a real, silent Unicode-semantics
  change to `regex` **and** `strings` (see B1). The "byte-identical" result above
  is therefore evidence that the current pin holds, not that the pin is enforced.
- The CI check proposed for A1/A2 converts this soft pin into a hard one: a
  regeneration on a mismatched UCD would fail the build instead of landing quietly.

### (B) Cross-package duplication

#### B1 — The 4,109-line Unicode general-category table is embedded twice, joined by a string-replace hack
- `src/builtins/regex.rs:113-114` concatenates `regex_package.mfb` +
  `regex_unicode.mfb`.
- `src/builtins/strings.rs:266` does:
  `include_str!("regex_unicode.mfb").replace("__regex_genCat", "__strings_genCat")`,
  then `:267` concatenates it after `strings_package.mfb`.
- `src/builtins/regex_unicode.mfb` is 4,109 lines. A program that imports both
  `strings` and `regex` compiles **two full copies** of the table under two names.
- The consumer predicates are duplicated too, in different shapes over the same
  data: `src/builtins/strings_package.mfb:47` `__strings_isLetter` / `:56`
  `__strings_isWhitespace` against `src/builtins/regex_package.mfb:241`
  `__regex_catIsLetter` / `:249` `__regex_isSpaceCp`.
- A rename inside `regex_unicode.mfb` that touches the substring `__regex_genCat`
  silently changes `strings` behavior through a textual `.replace`, with no type
  or link check between the two.

#### B2 — `net` and `http` carry three byte-identical private helpers, and `http` already `IMPORT`s `net`
- `src/builtins/net_package.mfb:25-31` `__net_indexOf` ≡
  `src/builtins/http_package.mfb:40-46` `__http_indexOf` (identical bodies; the
  `net` copy carries three comment lines, the `http` copy two).
- `src/builtins/net_package.mfb:34-39` `__net_slice` ≡
  `src/builtins/http_package.mfb:48-53` `__http_slice` — byte-for-byte.
- `src/builtins/net_package.mfb:41-46` `__net_defaultPort` ≡
  `src/builtins/http_package.mfb:55-60` `__http_defaultPort` — byte-for-byte.
- `src/builtins/http_package.mfb:6` already has `IMPORT net`.
- `indexOf` and `slice` are generic String operations with nothing network-specific
  about them; they belong in `strings::`. Both exist only because `strings::find`
  fails `ErrNotFound` on a miss and, being inline-expanded, cannot be wrapped in an
  inline `TRAP` — the comment at `net_package.mfb:22-24` states exactly this. That
  is a `strings::` gap, so the fix is a `strings::` addition, not a third copy.
- **Nuance:** `__net_percentDecodeImpl` re-implementing `__encoding_percentDecodeBytes`
  is a *deliberate* duplication, justified by the comment at
  `src/builtins/net_package.mfb:209`. Leave it; do not fold it into this item.

#### B3 — Four crypto byte-slice helpers are one implementation under four names, all hand-rolling `collections::mid`
- `src/builtins/crypto_package.mfb:105-115` `__crypto_copyBytes`
- `src/builtins/crypto_package.mfb:313-321` `__crypto_truncate`
- `src/builtins/crypto_package.mfb:2101-2109` `__crypto_slice`
- `src/builtins/crypto_package.mfb:2140-2148` `__crypto_bytePrefix`
- `__crypto_truncate` and `__crypto_bytePrefix` are **byte-for-byte identical**,
  1,827 lines apart. `__crypto_copyBytes` is the same loop with `n = len(data)`;
  `__crypto_slice` is the same loop with an explicit `start`.
- All four are element-by-element `append` loops reimplementing what
  `collections::mid` does natively (a bulk range copy) — so this is a performance
  item as well as a duplication item.
- The same shape recurs outside crypto: `src/builtins/http_package.mfb`
  `__http_byteSlice` and `src/builtins/collections_package.mfb:87-95`
  `__collections_slice`.

### (C) Within-package duplication

#### C1 — `http` duplicates its transport and its de-chunk along two axes
- Plaintext vs TLS transport: `src/builtins/http_package.mfb:317-342`
  `__http_exchangeTcp` vs `:344-368` `__http_exchangeTls`; and the server
  counterpart `:1110-1150` `__http_handleRequest` vs `:1154-1191`
  `__http_handleRequestSSL` — the two server bodies are identical apart from the
  `net::`/`tls::` qualifier on four calls.
- String vs Byte de-chunk: `:211-241` `__http_dechunk` vs `:573-603`
  `__http_dechunkBytes` — the same framing walk over two element types.
- **Correction to the raw lead:** the transport duplication is *deliberate and
  documented*. `src/builtins/http_package.mfb:1152-1153` states: "TLS counterpart:
  identical core, `tls::` transport (server-side handshake in `tls::accept`). The
  two bodies cannot share one socket variable (§F.5.6)." A `RES` binding cannot be
  one variable across two resource types, so this cannot be collapsed without a
  language change. **Do not "fix" it.** Record the constraint; the item is that the
  duplication is currently unguarded — a change to one body will not be flagged if
  the other is missed.
- The de-chunk pair carries no such constraint and is a genuine dedup candidate.

#### C2 — `__http_parseResponse` inlines a header-block loop that `__http_headerMapFromHead` already provides
- `src/builtins/http_package.mfb:285-298` (inlined inside `__http_parseResponse`)
  vs `:515-532` `__http_headerMapFromHead`.
- Identical logic — lowercase the field name, trim both halves, last-wins into a
  map, skip blank lines, start at index 1 — differing only in local names
  (`index`/`idx`, `fieldName`/`name`, `fieldValue`/`value`).
- `src/builtins/http_package.mfb:644-656` (`__http_partHeader`) is a third copy of
  the same header-line split.

#### C3 — `regex::findAll` and `regex::replace` each carry the zero-width-match dance
- `src/builtins/regex_package.mfb:1744-1775` `__regex_findAll` vs `:1777-1811`
  `__regex_replace`.
- The tricky empty-match rule — `IF mstart = mend` → if `mend = lastMatch` then
  advance and `CONTINUE`, else record, set `lastMatch`, advance and `CONTINUE` — is
  written out twice, identically. This is exactly the kind of subtle loop that must
  not exist in two places.

#### C4 — `encoding` duplicates its UTF-8 lead-byte cascade and its LEB128 emit loop
- `src/builtins/encoding_package.mfb:75-136` `__encoding_utf8Valid` vs `:139-178`
  `__encoding_codepoints`: the same `>=240 / >=224 / >=194` lead-byte cascade and
  the same `codePoint * 64 + (cont - 128)` continuation accumulation; `utf8Valid`
  additionally applies the overlong/surrogate/range checks.
- `:890-909` `__encoding_uleb128Encode` vs `:996-1013` `__encoding_varintEncode`:
  the emit loops at `:899-908` and `:1001-1012` are line-for-line identical; only
  the seed differs (raw value vs the zigzag computed at `:997`).
- `:536-550` `__encoding_isUnreserved` vs `:552-563` `__encoding_isAlphaNum`: the
  first three range checks are identical; `isUnreserved` adds only `:545`.
- **Correction to the raw lead:** `__encoding_varintEncode` **cannot** simply
  `RETURN __encoding_uleb128Encode(zigzag)`. `__encoding_uleb128Encode:891-893`
  fails on negative input, and the zigzag at `:997`
  (`bxor(sl(value,1), sra(value,63))`) overflows to a negative Integer for
  `|value| >= 2^62`; those inputs encode correctly today via logical `bits::sr`, and
  delegating would turn them into `error(77050003, "negative value")` — a
  behavioral regression on an encoder. Note the asymmetry: the decode side already
  delegates (`__encoding_varintDecode:1016` calls `__encoding_uleb128Decode`). A
  safe dedup must hoist the negativity guard out of the shared core.
- Also `:449-474` `__encoding_base64Symbols` vs the equivalent loop inlined at
  `:505-525` inside `__encoding_base32Decode` — same padding (`c = 61`) and same
  `v < 0` rejection, but the second is a block inside a function, not a peer
  function, so the dedup is an extraction rather than a merge.

#### C5 — `datetime` duplicates its constructor ladder and its offset formatter
- `src/builtins/datetime_package.mfb:73-81` `__datetime_normInstant` vs `:83-91`
  `__datetime_normDuration` — byte-identical bodies apart from the constructed
  record name.
- `:113-131` (`__datetime_instant1..5`) vs `:137-155` (`__datetime_duration1..5`) —
  structurally identical 5-arity ladders; the `days*86400 + hours*3600 + mins*60 +
  seconds` arithmetic is character-for-character the same.
- `:365-375` `__datetime_offsetLabel` vs `:537-547` `__datetime_offsetLabelCompact`
  — identical sign/hh/mm derivation; the sole difference is the `":"` separator in
  the final concat (`:375` vs `:547`).
- **Downgraded:** `:851-860` vs `:939-957` was claimed as a fourth pair. It is
  weaker — the first is the `"f"` fractional-token branch of `__datetime_parseFields`,
  the second the ISO fractional-seconds scanner in `__datetime_parseIso`. They share
  only the `WHILE digits < 9 : frac = frac * 10` tail (`:855-858` vs `:949-952`).
  Overlapping, not duplicated; lowest priority of the five.

#### C6 — crypto's 256/512 ladders are mechanically duplicated in HKDF and PBKDF2
- `src/builtins/crypto_package.mfb:646-671` vs `:673-698` (HKDF);
  `:725-750` vs `:755-780` (PBKDF2).
- **Scope guard:** the twelve SHA-256/SHA-512 primitive pairs elsewhere in the file
  are word-size-inherent (32-bit vs 64-bit arithmetic). **Leave those alone.** Only
  the HKDF/PBKDF2 ladders — which merely select a hash — are in scope (~52 lines).

#### C7 — `audio` duplicates its s16 clamp-and-encode between its two subsystems
- Clamp appears three times: `src/builtins/audio_package.mfb:59-63` (in
  `__audio_render`), `:499-503` (`__mml_synth`), `:534-538` (`__mml_mix`) — the same
  `> 32767 / < -32768` saturation.
- The little-endian s16 emit appears twice: `:65-69` (inlined in `__audio_render`)
  and `:549-554` (`__mml_encode`) — the same `v + 65536` wrap and the same two
  `bits::band` appends.

#### C8 — `crypto_package.mfb` is 2,262 lines spanning eleven primitives, with the seams already marked
- The file carries **22** section banners (`grep -n "^' ---"`), e.g. `:116`
  SHA-256/224, `:573` HMAC, `:643` HKDF, `:699` PBKDF2, `:811` ChaCha20, `:902`
  Poly1305, `:1144` AES-256, `:1364` AES-256-GCM, `:1570`–`:2119` Ed25519
  (~700 lines), `:2134` NIST EC keygen.
- The concatenation pattern for splitting a package across several `.mfb` sources
  already exists and is in use at `src/builtins/strings.rs:266-267`, so a split is
  mechanical.

#### C9 — `collections_package.mfb` section banners no longer match their contents
- Only two banners survive: `src/builtins/collections_package.mfb:75`
  `' --- internal helpers ---` and `:97` `' --- new functions (§6.4) ---`.
- `__collections_take` sits at `:71`, *above* the internal-helpers banner, while its
  twin `__collections_drop` sits at `:159` — 88 lines below, buried among the §6.4
  functions. The two halves of one API are on opposite sides of two banners.

#### C10 — `__datetime_expect` is the only helper in its file defined after its caller
- Definition `src/builtins/datetime_package.mfb:964-969`; all four callers are at
  `:923`, `:925`, `:934`, `:936`, inside `__datetime_parseIso` (`:919-962`).
- Every other `FUNC` in this 991-line file is defined before first use (the two
  apparent counterexamples, `__datetime_civil` and `__datetime_format`, are
  substring false positives against `__datetime_civilFromDays:282` and
  `__datetime_formatToken:557`).

#### C11 — `audio_package.mfb` is two unrelated subsystems in one file
- Tone renderer: `src/builtins/audio_package.mfb:38-72` (`__audio_render`).
- MML parser/player: `:74-582` — banner at `:74`, `TYPE MmlEvent` at `:94`, helpers
  `:104-556`, public entry points `:561-582`.
- **Correction:** the file is 582 lines, not 583, and the `__audio_play*` entry
  points at `:561-582` belong to the MML half, so the split is `:38-72` against
  `:74-582`. C7 is the duplication that crosses this seam.

### (D) Dead code

#### D1 — `__collections_reverse` is dead, and the file header comment claims it is called
- Definition: `src/builtins/collections_package.mfb:77-85`.
- Repo-wide grep for `__collections_reverse` (excluding `.git`) returns exactly
  three hits: the definition, the header comment at `:9-10`, and a historical
  mention in `planning/old-plans/plan-39-benchmark-perf.md:560`. **Zero callers.**
- `src/builtins/collections.rs` contains no `reverse` entry at all — it is absent
  from the package function list and from the native member list — and there is no
  `man` page for it.
- The header comment at `:9-10` asserts the opposite: "The internal
  `__collections_slice`/`__collections_reverse` helpers are plain top-level
  functions and are called unqualified." Half of that sentence is false.

#### D2 — Four one-line character helpers exist only because plan-27 landed after them
- `src/builtins/csv_package.mfb:5-9` `__csv_crChar` — body is literally
  `RETURN "\r"`, and its own comment at `:5-6` says the lexer now decodes the escape.
  Called at `:173`.
- `src/builtins/http_package.mfb:62-66` `__http_crlf` — body is `RETURN "\r\n"`,
  comment likewise self-documenting as obsolete. It is called 3 times while the same
  file writes `"\r\n"` inline 9 times, so the file already disagrees with itself.
- `src/builtins/json_package.mfb:48-51` `__json_backspaceChar` and `:53-56`
  `__json_formfeedChar` — each builds a one-element `List OF Byte` and calls
  `toString` to produce a character the lexer can now spell directly.
- **Correction to the raw lead:** the replacements are `\u{8}` and `\u{C}`, not
  `\u{8}`/`\u{12}` — `\u{HEX}` takes hex, and form feed is decimal 12 = `0xC`.
  Writing `\u{12}` would emit U+0012, a different character. Worth stating
  explicitly because it is exactly the mistake this cleanup could introduce.

#### D3 — `__regex_posixProp` returns six sentinel tokens no matcher handles
- `src/builtins/regex_package.mfb:1048` `"posixAlnum"`, `:1063` `"posixWord"`,
  `:1066` `"posixXdigit"`, `:1069` `"posixBlank"`, `:1075` `"posixGraph"`,
  `:1078` `"posixPrint"`.
- `__regex_propTest` (`src/builtins/regex_package.mfb:466-500`) handles `L M N P S
  Z C`, `White_Space`, `Alphabetic`, then `__regex_isGcName`, then falls through to
  `__regex_scriptTest` — **no `posix*` case anywhere**. The six tokens reach
  `__regex_scriptTest` and return FALSE.
- Consequence: `[[:alnum:]]` parses cleanly and then matches nothing.
- **This is not a new finding.** It is `bugs/bug-316-regex-posix-classes-never-match.md`
  (MEDIUM, Correctness, Open), which already carries the reproduction, root cause,
  and fix design. It is listed here **only** so a reader of this cleanup document
  does not re-file it. **Fix it under bug-316, not under this document**; the six
  dead-token returns are the same defect viewed as dead code, not a separate one.

#### D4 — `json` linear-scans code points 0..31 instead of using the `encoding` package it already imports
- `src/builtins/json_package.mfb:224-232` `__json_escapeRawControlCharAt` and
  `:484-492` `__json_isRawControlCharAt` each recurse from code point 0 upward,
  calling `__json_codePointToString(codePoint)` at every step and comparing the
  built string against the input character.
- Worst case: **32 string constructions and 32 string comparisons to classify one
  character**, on the hot path of both JSON serialization and JSON parsing.
- `src/builtins/json_package.mfb:3` already has `IMPORT encoding`;
  `encoding::utf32Encode` answers the question in one call.
- Same file, same cause: `__json_hexDigit` (`:255-289`) is a 16-branch
  `IF/ELSEIF` chain, against `__encoding_hexDigit`
  (`src/builtins/encoding_package.mfb:294-296`), which is
  `RETURN strings::mid("0123456789abcdef", d, 1)`. (Note the case differs — `json`
  emits uppercase — so this is not a drop-in substitution.)

#### D5 — A stale comment cites `bug-01` as an open allocator blocker
- `src/builtins/crypto_package.mfb:1951-1954`: "NOTE: impractically slow under the
  current arena allocator (bug-01) … Blocked on the bug-01 allocator fix; see
  planning/bug-01."
- `planning/bug-01` does not exist. The only `bug-01` file in the tree is
  `bugs/completed-bugs/bug-01-resource-union-drop.md`, an unrelated
  resource-union-drop bug. The allocator document the comment means
  (`bug-01-collection-value-leaks.md`) is referenced by
  `bugs/bug-307-collection-hof-string-item-leak.md:28` but is **not present in the
  tree** — the citation is dangling from both ends.
- The arena work referenced has since landed (plan-25-A and successors). The comment
  should be re-benchmarked and rewritten with a live citation, or deleted.

### (E) Naming and style consistency

#### E1 — `audio` uses a `__mml_` prefix and snake_case against the stdlib's `__<pkg>_` + camelCase
- `src/builtins/audio_package.mfb`: **20** `FUNC`/`SUB` use `__mml_`, **4** use
  `__audio_` (`__audio_render:38`, `__audio_play_samples:561`, `__audio_play:569`,
  `__audio_play_tracks:576`). No other stdlib package uses a prefix that does not
  match its package name.
- snake_case names, all in this file: `__mml_render_samples:513`,
  `__audio_play_samples:561`, `__audio_play_tracks:576` — while the same file also
  uses camelCase (`__mml_noteSemitone:128`, `__mml_waveCode:148`,
  `__mml_trailingDots:164`, `__mml_clampFade:440`). The file is inconsistent with
  itself as well as with the stdlib.
- Unprefixed private types: `TYPE MmlEvent` at `src/builtins/audio_package.mfb:94`
  and `TYPE RouteMatch` at `src/builtins/http_package.mfb:427` (used at `:823`,
  `:879`, `:902`). Every other private stdlib type is prefixed
  (`__datetime_NumRead`, `__json_Node`, 25 `__regex_*`). These two are the only
  exceptions repo-wide.
- **Two claimed outliers do NOT hold — dropped, do not re-file:**
  - `vector_package.mfb:812 __vector_clamp_length_float2` is **not** an outlier. The
    whole vector package uses systematic `<op>_<type><n>` monomorphization suffixes
    across ~190 names (`__vector_lerp_unclamped_integer3`,
    `__vector_rotate_2d_fixed2`, …). That is the local convention. (It is also a
    generated file — see A1 — so renaming would have to happen in the generator.)
  - `crypto_package.mfb:251 __crypto_sha2_32` is **not** an outlier. It pairs with
    `__crypto_sha2_64` as the shared 32-/64-bit SHA-2 cores, inside a consistent
    suffix family (`__crypto_sha256_bytes`, `__crypto_pbkdf2Sha512_text`,
    `__crypto_bsig0_64`). The `_32`/`_64` is a width tag within that scheme.

#### E2 — Four different `isDigit` idioms across five packages
- `src/builtins/json_package.mfb:767-769` — substring search:
  `RETURN strings::contains("0123456789", ch)`.
- `src/builtins/regex_package.mfb:820-822` and
  `src/builtins/datetime_package.mfb:700-702` — range compare:
  `RETURN ch >= "0" AND ch <= "9"` (these two agree with each other).
- `src/builtins/audio_package.mfb:104-106` — a 10-way `OR` chain across one line.
- `src/builtins/strings_package.mfb:52-54` — Unicode table lookup:
  `RETURN __strings_genCat(toInt(sc)) = "Nd"`.
- The last is the only Unicode-correct one and is genuinely different in meaning
  (Nd covers non-ASCII digits); the first four are four spellings of one ASCII test.
  Any consolidation must preserve that distinction — folding the ASCII ones into the
  `strings::` Unicode one would change parser behavior on non-ASCII input.

#### E3 — `0 - 1` and `-1` both used for the same sentinel
- `0 - 1` appears in `src/builtins/http_package.mfb` (5), `encoding_package.mfb` (3),
  `audio_package.mfb` (1), `net_package.mfb` (1) — 10 sites in four files.
- Plain `-1` appears in `audio_package.mfb`, `encoding_package.mfb`,
  `datetime_package.mfb:174,176`, `regex_package.mfb:1739`, and in default parameter
  values (`collections_package.mfb:209`).
- `encoding_package.mfb` and `audio_package.mfb` use **both** spellings.
- (Historical: `0 - 1` predates unary-minus constant folding; `bugs/completed-bugs/bug-07-fixed-min-literal.md`
  covers the related literal-folding fix. Confirm folding before converting.)

#### E4 — `REM` and `'` comment markers mixed within single files
- Files that are consistently `'`: `audio` (64/0), `crypto` (156/0), `csv` (27/0),
  `datetime` (78/0), `encoding` (40/0), `money` (12/0), `net` (42/0),
  `vector` (28/0), `collections` (22/0).
- Files that are consistently `REM`: `strings_package.mfb` (17 `REM`, 0 `'`),
  `regex_unicode.mfb` (6/0).
- **Mixed:** `regex_package.mfb` (73 `REM`, 4 `'`), `http_package.mfb`
  (7 `REM`, 98 `'`), `json_package.mfb` (6 `REM`, 14 `'`).
- The three mixed files are the actionable ones; the split between the two
  consistent groups tracks file age and is a separate, larger decision.

#### E5 — `regex_unicode.mfb` is the only source not named `<pkg>_package.mfb`
- All thirteen others follow `<pkg>_package.mfb`. `regex_unicode.mfb` does not —
  **correctly**, because it is shared table data consumed by two packages
  (`src/builtins/regex.rs:114` and `src/builtins/strings.rs:266`), not a package
  body. The name should signal that shared-data role rather than looking like an
  inconsistency. Cosmetic; lowest priority. Note that renaming it touches both
  `include_str!` sites and `scripts/gen_regex_unicode.py:13`.

### (F) Documentation

#### F1 — No stdlib `.mfb` source carries a DOC block, though both shipped bindings do
- `grep -c "END DOC"` returns **0** for all fourteen files in `src/builtins/*.mfb`.
- Both shipped bindings use the feature: `bindings/sqlite3/src/lib.mfb` (DOC blocks
  at `:48`, `:65`, `:74`, `:84`, `:94`, …) and `bindings/libsnd/src/lib.mfb`.
- The feature is specified and implemented (plan-09-doc: lexer raw-capture, resolver
  validation, `.mfp` section id 17).
- **35** `EXPORT TYPE` / `EXPORT ENUM` / `EXPORT UNION` declarations across the
  stdlib sources are therefore user-facing surface documented only by `'` comments
  that no tooling reads — `mfb man` cannot surface them.

#### F2 — `http`'s public surface is 18 items; `man/builtins/http/` has 4 pages
- Public functions in `src/builtins/http.rs:10-26`: `read`, `write`, `server`,
  `serverSSL`, `handleRequest`, `route`, `responseDefault`, `ok`, `status`, `json`,
  `withHeader`, `bytes`, `respondFile`, `respondPath` — **14**. Public types: 
  `Response`, `Request`, `RequestPart`, `Route` — **4**.
- `src/docs/man/builtins/http/` contains: `bytes.txt`, `respondFile.txt`,
  `respondPath.txt`, `withHeader.txt`, `package.md` — **4 function pages**.
  Ten functions and all four types have no page.
- Contrast (verified): `json` is 4/4 and `csv` is 2/2 — both complete. `audio`'s four
  "missing" entries are correctly-omitted internals, not a gap.
- This converges with the `builtins`-Rust man-drift finding; land the two together.

#### F3 — Man pages split `.md`/`.txt` with no rule
- 12 directories under `src/docs/man/builtins/` are all-`.md`; 12 are `.txt` plus a
  `package.md`. Every directory does have a `package.md`.
- The split tracks package **age**, not content type or audience.

#### F4 — VERIFIED, NO ACTION: the tiny `money` and `strings` sources are correctly separate
- `src/builtins/money_package.mfb` is 19 lines; `src/builtins/strings_package.mfb`
  is 77 lines. Merging them into a neighbor looks tempting and is **wrong**.
- Each is `include_str!`'d only when its own package is imported
  (`src/builtins/money.rs:89`, `src/builtins/strings.rs:267`). One file per package
  is load-bearing: merging would pull unrelated declarations (e.g. `Rounding`) into
  every importer of the other package.
- Recorded here so this is not re-litigated in a future cleanup pass.

---

## Goal

- `python3 scripts/gen_vector_package.py` reproduces `src/builtins/vector_package.mfb`
  byte-for-byte, with the plan-39 C2 isqrt body living in the generator.
- CI fails if either generated artifact differs from a fresh regeneration.
- Both generated artifacts are marked generated in `.gitattributes`.
- The duplication, dead code, and naming items above are resolved, with behavioral
  equivalence proven by the acceptance suite and per-package tests.
- Every stdlib `EXPORT` declaration is reachable through `mfb man`.

### Non-goals (must NOT change)

- **Any observable stdlib behavior.** These sources compile into every program that
  imports them.
- The one-file-per-package `include_str!` structure (F4).
- The `net`↔`encoding` percent-decode duplication, which is deliberate and
  documented at `src/builtins/net_package.mfb:209` (B2).
- The plaintext/TLS transport duplication in `http`, which the language's `RES`
  binding rules currently make unavoidable (C1). **Tempting wrong fix, forbidden:**
  do not attempt to unify `__http_handleRequest`/`__http_handleRequestSSL` by
  weakening the resource typing.
- The twelve SHA-256/SHA-512 primitive pairs in `crypto`, which are
  word-size-inherent (C6).
- The distinct semantics of `__strings_isDigit` (Unicode Nd) versus the four ASCII
  `isDigit` helpers (E2).
- The regex POSIX-class defect, which belongs to bug-316 (D3).
- The `.mfp` package format, wire encodings, and every documented error code.

## Blast Radius

Every item is inside `src/builtins/`, but the reach is the whole language surface:
these files are compiled into user programs.

- `src/builtins/vector_package.mfb` + `scripts/gen_vector_package.py` — A1, fixed here.
- `scripts/gen_regex_unicode.py`, `src/builtins/regex_unicode.mfb` — A2/A3, fixed here
  (CI + `.gitattributes`); the UCD pin becomes enforced rather than assumed.
- `src/builtins/regex.rs:113-114`, `src/builtins/strings.rs:266-267` — B1; the
  `.replace` seam is Rust-side and must be changed with the `.mfb` side.
- `src/builtins/{net,http}_package.mfb` — B2, C1, C2; adding the helpers to
  `strings::` also touches `src/builtins/strings.rs` and its man pages.
- `src/builtins/crypto_package.mfb` — B3, C6, C8, D5.
- `src/builtins/{encoding,datetime,audio,json,csv,collections,regex}_package.mfb` —
  C3–C11, D1, D2, D4, E1–E4.
- `src/docs/man/builtins/**` — F2, F3.
- `bugs/bug-316-regex-posix-classes-never-match.md` — D3, **out of scope here**;
  fix under that document.
- `bugs/bug-307-collection-hof-string-item-leak.md:28` — cites the same missing
  `bug-01-collection-value-leaks.md` as D5; latent, same dangling citation, out of
  scope because it is a different document's reference list.
- `bindings/{sqlite3,libsnd}/src/lib.mfb` — unaffected; they are the DOC-block
  *model* for F1, not a target.
- `benchmark/` — unaffected in behavior, but the A1 regression would have shown up
  here first had anyone been watching; add an isqrt row if cheap.

## Fix Design

**The generator item comes first and is the only one with a real correctness stake.**

### 1. A1/A2/A3 — close the generated-artifact hole (do this before anything else)

1. Backport the plan-39 C2 FSQRT-seeded `__vector_isqrtFloor` body — including its
   five-line explanatory comment — from `src/builtins/vector_package.mfb:69-90` into
   `scripts/gen_vector_package.py:47-71`, replacing the stale Newton emission.
2. Re-run `python3 scripts/gen_vector_package.py > src/builtins/vector_package.mfb`
   and confirm `git diff` is **empty**. That empty diff is the proof; nothing else
   in the file may move.
3. Add a CI step that, for each generator, regenerates to a temp file and `diff`s
   against the checked-in artifact, failing the build on any difference. Wire it into
   `.github/workflows/` alongside `coverage.yml`.
4. Add a `.gitattributes` at the repo root marking both
   `src/builtins/vector_package.mfb` and `src/builtins/regex_unicode.mfb` as
   `linguist-generated=true`, so diffs collapse and reviewers get the signal the
   banner alone failed to give.
5. Have the CI step assert the Unicode version too — `gen_regex_unicode.py` already
   records the UCD version in the generated header, so compare that string rather
   than relying on the host CPython matching by luck (A3).

**That CI step is the regression test for this entire class of defect.** It is worth
more than every other item in this document combined, and it is roughly an hour of
work. Land it alone if nothing else lands.

Rejected alternatives, so they are not re-litigated:

- *Delete the banner and declare the file hand-maintained.* Rejected: the generator
  encodes ~190 monomorphized vector functions; hand-maintaining them is worse than
  the drift.
- *Delete the generator and keep the artifact.* Same objection, and it discards the
  ability to regenerate on a type-set change.
- *Move generation into `build.rs`.* Rejected: it would make every build depend on a
  working CPython with a matching UCD, and would make the artifact's provenance a
  build-time variable rather than a reviewed, checked-in fact. Check-in plus a CI
  diff keeps the artifact reviewable and pins the UCD.

### 2. B1 — the double Unicode table

The `.replace` hack at `src/builtins/strings.rs:266` is the load-bearing part; a
proper fix emits the table once under a neutral symbol and has both packages consume
it, deduplicating the consumer predicates
(`__strings_isLetter`/`__regex_catIsLetter`, `__strings_isWhitespace`/`__regex_isSpaceCp`)
at the same time. This is the largest single win here (4,109 lines of duplicated
source per dual-importing program) and also the riskiest, because it changes symbol
names inside a generated file — so it **must** land after step 1, on top of a working
CI regeneration check. Sequencing matters: doing B1 first would drift the generator
again, in exactly the way this document exists to prevent.

### 3. Everything else

Land in the (B)…(F) order, one item per commit, each with its own behavioral test.
There is no shared risk between them; a partial landing is fine and expected. B2's
`strings::indexOf`/`strings::slice` addition is a public-surface change and needs a
man page and a spec note, so treat it as the heaviest of the remainder.

**On the acceptance criterion.** Unlike the Rust-side cleanups in this review series,
**"byte-identical generated output" is NOT the criterion here.** These are stdlib
*sources*: any edit to a `.mfb` file legitimately shifts compiled output and
therefore shifts goldens. Removing a dead function changes symbol layout; replacing a
32-step linear scan with one call changes instruction counts. The criterion is
**behavioral equivalence proven by tests** — `scripts/test-accept.sh` green and the
per-package suites under `tests/rt-behavior/<pkg>/` and `tests/syntax/<pkg>/` green,
with every golden shift reviewed and attributed to the intended edit. Regenerate
goldens with `scripts/sync-goldens.sh <exe> <name-glob>` per item, never in bulk, so
each shift stays attributable. **The one exception is step 1**, where an empty
`git diff` *is* the acceptance criterion.

## Phases

### Phase 1 — generator integrity (do this alone, first)

- [ ] Backport the plan-39 C2 isqrt body into `scripts/gen_vector_package.py:47-71`.
- [ ] Regenerate `src/builtins/vector_package.mfb`; confirm `git diff` is empty.
- [ ] Add the CI regeneration-diff step for both generators, including the UCD
      version assertion.
- [ ] Add `.gitattributes` marking both artifacts generated.
- [ ] Verify the CI step actually fails: revert the generator backport locally,
      confirm red, restore.

Acceptance: regenerating either artifact produces no diff; CI fails on an
artificially introduced drift; no compiled output changes at all (the artifact is
byte-identical to what was already checked in).
Commit: —

### Phase 2 — dead code and stale documentation (smallest behavioral surface)

- [ ] Delete `__collections_reverse` (D1) and correct the false header comment at
      `src/builtins/collections_package.mfb:9-10`.
- [ ] Inline the four obsolete character helpers (D2), using `\u{8}` and `\u{C}`.
- [ ] Rewrite or delete the dangling `bug-01` comment at
      `src/builtins/crypto_package.mfb:1951-1954` (D5), re-benchmarking first.
- [ ] Replace the 0..31 linear scans in `json` with `encoding` calls (D4),
      preserving `__json_hexDigit`'s uppercase output.
- [ ] Cross-reference bug-316 for D3; **do not fix it here**.

Acceptance: per-item tests green; golden shifts reviewed and attributed.
Commit: —

### Phase 3 — duplication (B and C)

- [ ] B1 (double Unicode table) — on top of Phase 1's CI check.
- [ ] B2, B3 — cross-package helpers; `strings::` additions get man pages.
- [ ] C2–C11 — within-package, one commit each.
- [ ] Record the C1 transport constraint in a comment; do not collapse it.

Acceptance: acceptance suite green after each; goldens attributed per item.
Commit: —

### Phase 4 — naming, style, documentation

- [ ] E1–E5, F1–F3.
- [ ] DOC blocks for all 35 `EXPORT` declarations (F1); verify each renders via
      `mfb man`.

Acceptance: `mfb man` surfaces every stdlib `EXPORT`; full suite green.
Commit: —

## Validation Plan

- **Regression test (the important one):** the CI regeneration-diff step from
  Phase 1. It guards the whole class, not just today's instance.
- **Runtime proof for Phase 1:** the regenerated `vector_package.mfb` is byte-identical
  to the checked-in file, so the compiled output of every vector-importing program is
  unchanged — verifiable with a build diff.
- **Behavioral proof for Phases 2–4:** `scripts/test-accept.sh` plus the per-package
  dirs under `tests/rt-behavior/` and `tests/syntax/`. Golden shifts are expected and
  legitimate; each must be attributed to a specific edit. Use
  `scripts/sync-goldens.sh <exe> <name-glob>` per item.
- **Doc sync:** man pages for any new `strings::` surface (B2); `mfb man` coverage for
  F1/F2; `mfb spec stdlib regex` if B1 changes symbol names.
- **Full suite:** `scripts/test-accept.sh` on both platforms before each phase closes.

## Open Decisions

- **B1 shape** — emit the table under a neutral third symbol consumed by both
  packages (recommended) vs. keeping the `.replace` and merely deduplicating the
  consumer predicates (cheaper, leaves the textual seam). (§B1)
- **E2 scope** — unify the four ASCII `isDigit` helpers into one shared private
  helper (recommended) vs. leave them per-package. Must not touch
  `__strings_isDigit`. (§E2)
- **E4 direction** — standardize the three mixed files on `'` (matches 9 of 14 files)
  vs. on `REM` (matches the two generated/oldest files). Recommend `'`. (§E4)
- **C8 split** — split `crypto_package.mfb` at the existing banners (recommended,
  Ed25519 first at ~700 lines) vs. leave it and rely on the banners. (§C8)

## Summary

The engineering risk is concentrated in one item that is not really cleanup: a
generated stdlib source has drifted from its generator, the drift carries a landed
optimization, and there is no mechanism anywhere in the tree that would notice. Phase
1 fixes it in about an hour and installs the CI check that makes the whole class
impossible to repeat — that check is worth more than the rest of this document
combined and should land on its own.

The remaining items are genuine but LOW: 4,109 lines of Unicode table embedded twice
behind a string-replace, three helpers copied verbatim between `net` and `http`, four
crypto helpers that are one function under four names, a dead `__collections_reverse`
whose file comment claims otherwise, a JSON classifier doing 32 string constructions
per character, and 35 exported declarations with no DOC blocks.

Three raw leads were checked and **downgraded rather than filed** — the `http`
plaintext/TLS transport duplication is language-mandated and documented, two of the
claimed naming outliers are local conventions rather than deviations, and
`__encoding_varintEncode` cannot simply delegate without a behavioral regression on
large negative inputs. F4 records that the tiny `money`/`strings` sources are
correctly separate. Because these are sources compiled into every importing program,
the acceptance criterion for Phases 2–4 is behavioral equivalence proven by tests —
**not** byte-identical output, which every edit here legitimately changes.
