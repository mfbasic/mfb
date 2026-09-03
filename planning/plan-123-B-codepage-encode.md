# plan-123-B: `encoding::codepageEncode`

Last updated: 2026-09-02
Effort: medium (1h–2h)
Depends on: plan-123-A

Adds the inverse of plan-123-A:
`encoding::codepageEncode(codepage AS Codepage, text AS String) AS List OF Byte`,
over the same `Codepage` enum and the same generated single-byte tables.

**The single behavioral outcome:**
`encoding::codepageEncode(Codepage.Windows1252, "café")` returns the four bytes
`99 97 102 233`, and `codepageEncode(cp, codepageDecode(cp, bytes))` returns `bytes`
unchanged for every byte sequence that decodes without raising, for every
single-byte `Codepage` variant. A character the selected codepage cannot represent
raises `ErrInvalidFormat` (`77050003`).

References:

- `planning/plan-123-A-codepage-decode.md` — the enum, the table representation, the
  vendored index files, and the measured populations this plan reuses. Read it first;
  it is not restated here.
- WHATWG Encoding Standard §"single-byte encoder" — the algorithm this implements.
- `AGENTS.md` → `.ai/man-content.md` (man page content standard),
  `.ai/testing-gates.md` (acceptance harness, golden regeneration),
  `.ai/specifications.md` (embedded spec sync).

## Prerequisites

Stated in full in plan-123-A §Prerequisites; they apply unchanged here. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-123-A is complete | every box in `planning/plan-123-A-codepage-decode.md` ticked, all three phases' `Commit:` lines filled, `cargo test --no-fail-fast` + `scripts/test-accept.sh` + `scripts/artifact-gate.sh` green | MET |
| `encoding::Codepage` exists with its final variant set | `./target/release/mfb man encoding types \| grep -c '^ • '` → 29 | MET |
| `__encoding_codepageTable` returns a 128-scalar literal per variant | `src/codegen/builtins/encoding/helper_codepage_table.rs`, generated; `codepage_tables_match_the_vendored_index_files` checks all 28 x 128 scalars against the index files | MET |
| The vendored index files are committed | `ls tools/codepage-index/index-windows-1252.txt` → present (27 files) | MET |

**Archiving note.** The plan's first row originally read "complete **and archived**
(`ls planning/completed/plan-123-A-*`)". Archiving is the last step of the whole
`plan-123` feature, after B lands and merges, so requiring it here would deadlock
A against B. The row is corrected to test what it was actually protecting —
that A's work is complete and its gates are green — see Corrections 5.

**If plan-123-A is not complete, this plan cannot start, full stop.** It is not scope
this plan absorbs, not a soft preference, and there is no fallback that hand-rolls a
table or ships an encoder against a partial enum.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `codepageEncode(Codepage.Windows1252, "café")` returns bytes `99 97 102 233`.
- For every single-byte `Codepage` variant, round-tripping every decodable byte
  sequence through `codepageDecode` then `codepageEncode` returns the original bytes.
- A character with no representation in the selected codepage raises
  `ErrInvalidFormat` (`77050003`).
- `mfb man encoding codepageEncode` renders, and its examples compile and run.

### Non-goals (explicit constraints)

- **No change to anything plan-123-A landed** — not the enum, not its variant order
  or discriminants, not the generated tables, not `codepageDecode`'s behavior or
  signature. If the encoder needs a different table shape, that is a correction to
  plan-123-A's design recorded in *its* Corrections section, not a silent edit here.
- **No label resolution and no multi-byte encodings** — same boundaries as
  plan-123-A; both remain separate follow-up plans.
- **No lossy encoding.** An unrepresentable character raises; it does not become
  `?`, an HTML numeric reference, or U+FFFD. (The spec's *error mode* for encoders
  is a caller policy, not something this member decides.)
- **No new error code**; reuse `77050003`.
- `toString(List OF Byte)` and every pre-existing `encoding` member stay unchanged.

## 2. Current State

After plan-123-A: `EXPORT ENUM Codepage` is rendered into the injected `encoding`
source, `__encoding_codepageTable(cp AS Codepage) AS String` returns that codepage's
128-scalar table (scalar *i* = the code point for byte `128 + i`, `\u{FFFD}` marking
an unmapped byte), and `codepageDecode` consumes it.

The encoder is the same table read backwards. Nothing else in the package changes.

### Measured populations

Inherited from plan-123-A and **re-measured before this plan is scheduled** (per the
write-plan rule on scope derived from another sub-plan's estimate):

| What | Count | Command |
|---|---|---|
| `Codepage` variants to cover | **29** as landed by A — `Utf8` plus the 28 WHATWG single-byte labels (see plan-123-A Open Decisions) | `./target/release/mfb man encoding types` → count the `Codepage` variants |
| Distinct tables to search | 27 | `ls tools/codepage-index/*.txt \| wc -l` |
| Scalars searched per unmappable character (worst case) | 128 | table length, by construction |

### Verified properties

- **MEASURED — the reverse lookup is unambiguous.** Within one WHATWG single-byte
  index, no code point appears twice, so a character maps to at most one byte and a
  scalar search over the table cannot pick the wrong one.
  `python3 scripts/audit_codepage_index.py` → `files with a repeated code point: 0`
  across all 27 vendored files (and it exits non-zero if that ever stops holding, so
  this is a standing check rather than a one-time observation). No tie-break rule is
  needed; the plain search is justified by measurement.
- **U+FFFD never appears as a real mapping** (plan-123-A measured max code point
  U+FB02), so the hole sentinel cannot be mistaken for an encodable character — the
  encoder must still reject U+FFFD in the *input* explicitly rather than letting it
  match a hole.

## 3. Design Overview

One member, one helper, no new data. `codepageEncode` walks the input's graphemes;
an ASCII scalar below U+0080 emits its own byte, and anything else is looked up in
the codepage's table by scalar search.

**Where correctness risk concentrates:** the hole sentinel. A naive
`strings::find(table, ch)` on input U+FFFD would match the sentinel and silently emit
whatever byte the hole occupies — a wrong-bytes bug that byte-count assertions would
not catch. The encoder must reject U+FFFD before the search. This is the single
subtle case in the plan and Phase 2's test pins it explicitly.

**Absence is guarded, not trapped.** Measured (`mfb man strings find`):
`strings::find` "always returns a valid index on success and never reports absence
with a sentinel such as -1. When needle does not occur at or after start it raises
ErrNotFound", and the page's own guidance is "When absence is an ordinary, expected
outcome, guard the two-argument form with `strings::contains`". Absence *is* the
ordinary outcome here — every unrepresentable character takes that path — so §4 uses
`strings::contains` and calls `find` only once a match is known to exist, rather than
the inline `TRAP`/`RECOVER` the plan sketched. See Corrections.

**Where design uncertainty concentrates — schedule first:** whether any table has a
duplicate code point (§2 Verified properties), which decides whether a plain search
is sound. It is one script over 27 committed files; run it before writing the member.

**Byte-identity is NOT this plan's gate.** This adds a member to a widely-imported
injected package, so `.ir` line shifts are expected — and they reach **every** package
that reaches `strings`, not just csv/json/regex, because `strings`' scalar seam
carries `IMPORT encoding` (plan-123-A Corrections 2; measured at 64 moved `.ir`
goldens for the decode half alone). The gate is the round-trip rt fixture. The shape
check is that every moved golden's diff mentions `codepage`/`Codepage`; one that
mentions neither is a real signal — root-cause it (objdump one fixture), do not
regenerate past it.

Rejected alternatives:

- **Build a `Map OF String TO Byte` per call** — rejected: allocates a 128-entry map
  per call to save a 128-scalar scan; the scan is the cheaper of the two, and this
  runtime punishes per-call allocation (see the browser example's measured per-edit
  tree-rebuild cost).
- **Generate a second, inverted table** — rejected for now: doubles the generated
  data and the audit surface to remove a bounded 128-scalar scan. Revisit only if
  Phase 3's throughput measurement says so, and then as a generator change in
  plan-123-A's file, not a hand-written table here. **Measured in Phase 3 and still
  rejected — but the number is now on the record rather than assumed; see
  Corrections 7 for what a future session would need to know to revisit it.**
- **Lossy fallback to `?`** — rejected: silently corrupts data. A caller that wants
  it can catch `77050003` and substitute.

## 4. Detailed Design

```
FUNC __encoding_codepageEncode(codepage AS Codepage, text AS String) AS List OF Byte
  IF codepage = Codepage.Utf8 THEN
    RETURN __encoding_utf8Encode(text)          ' existing overload, -> List OF Byte
  END IF
  LET table AS String = __encoding_codepageTable(codepage)
  MUT out AS List OF Byte = []
  FOR EACH ch IN strings::graphemes(text)
    ' A grapheme wider than one scalar has no single-byte representation.
    IF len(ch) <> 1 THEN
      FAIL error(77050003, "character not representable in this codepage")
    END IF
    LET point AS Integer = collections::get(__encoding_codepoints(ch), 0)
    IF point < 128 THEN
      out = collections::append(out, toByte(point))
    ELSE
      ' Reject the sentinel BEFORE searching, or it matches a table hole.
      IF ch = "\u{FFFD}" THEN
        FAIL error(77050003, "character not representable in this codepage")
      END IF
      IF NOT strings::contains(table, ch) THEN
        FAIL error(77050003, "character not representable in this codepage")
      END IF
      out = collections::append(out, toByte(strings::find(table, ch) + 128))
    END IF
  NEXT
  RETURN out
END FUNC
```

Notes the implementer must not skip:

- `strings::find` **raises `ErrNotFound` rather than returning -1 when not found**
  — confirmed by reading `mfb man strings find`, which also says to guard with
  `strings::contains` when absence is an ordinary outcome. It is: every
  unrepresentable character takes that path. Hence the `contains` guard above rather
  than the inline `TRAP`/`RECOVER` this plan first sketched (Corrections 2).
- `__encoding_codepoints(String) AS List OF Integer` is the package's existing
  scalar reader (`helper_codepoints.rs`); reuse it rather than adding a second path.
  `len(ch)` counts Unicode **scalars** for a `String`
  (`mfb man general len`), so `len(ch) <> 1` is exactly "this grapheme is more than
  one scalar".
- Iterating graphemes (not scalars) is deliberate: it makes "a combining sequence has
  no single-byte form" a clean raise instead of a partial encode.
- `collections::append` in a loop over a local `out` is the in-place shape; keep `out`
  a same-function local (passing it through a helper would copy the whole list per
  element).

Registration mirrors plan-123-A's `func_codepage_decode.rs`:
`func_codepage_encode.rs`, params `codepage AS ParameterType::named("Codepage")` and
`text AS ParameterType::String`, `return_type: ParameterType::list_of(ParameterType::Byte)`.

## Compatibility / Format Impact

- **Added public surface:** `encoding::codepageEncode`. Nothing existing is shadowed
  or changed.
- **Unchanged:** the `Codepage` enum and its discriminants, `codepageDecode`, every
  pre-existing `encoding` member, `toString(List OF Byte)`, all layout/ABI.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. Use `- [~]` for partial with one line on what remains. Fill `Commit:`
> the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — prove the reverse lookup is unambiguous

The one unproven premise, and it is a script over committed files.

- [x] Check all 27 vendored `tools/codepage-index/*.txt` for a code point appearing
      more than once within a single file. Record the result in §2 Verified
      properties, replacing the UNVERIFIED note. `python3
      scripts/audit_codepage_index.py` → `dups=0` on every file and
      `files with a repeated code point: 0`.
- [x] ~~If any duplicate exists: document the tie-break (lowest byte wins) in §4 and
      add a fixture pinning it.~~ — moot: **no duplicate exists**, per the command
      above. Recorded instead: the plain scalar search is justified by measurement,
      and the check is standing rather than one-shot — `audit_codepage_index.py`
      exits non-zero if a future re-fetch ever introduces a duplicate, so the
      justification cannot silently expire.

Acceptance: §2's duplicate claim is answered with the command that answered it, and
§4's design reflects the answer.
Commit: `—`

### Phase 2 — the member

- [x] Add `src/codegen/builtins/encoding/func_codepage_encode.rs` per §4, and
      register it in `src/codegen/builtins/encoding/mod.rs:register`. `encoding` is
      now 30 members (2 overloaded + 28 non-overloaded).
- [x] Read `mfb man strings find` and confirm the not-found behavior the body
      depends on; adjust §4 and the body to match what it actually does. It raises
      `ErrNotFound` and never returns `-1`; the page itself says to guard with
      `strings::contains` when absence is an ordinary outcome, which it is here. §4
      and the body use the guard rather than the sketched inline `TRAP`/`RECOVER`
      (Corrections 2).
- [x] Add `codepageEncode` to `tests/byte-identity/encoding/src/main.mfb` so the
      `encoding_codegen_cover_rt` `.ncodesum` goldens actually hash it, and
      regenerate them with `scripts/regen-ncodesum.sh`. Three arms covered
      (windows-1252, windows-874, and the `Utf8` delegation);
      `141 golden(s) refreshed, 0 missing`.
- [x] Tests: **round-trip** rt fixture — for every `Codepage` variant, every byte
      0x80–0xFF that decodes without raising must survive
      `codepageEncode(cp, codepageDecode(cp, [b])) == [b]`. Whole-range, not a
      sample. `tests/rt-behavior/encoding/func_encoding_codepageEncode_rt` walks all
      **256** bytes (the ASCII half too, not just the high half) of all 28 single-byte
      variants and reports `ok`/`hole`/`bad` per codepage. **Every line reports
      `bad=0`**, and `codepage_roundtrip_counts_match_the_vendored_index_files`
      re-derives every `ok`/`hole` pair from `tools/codepage-index/` (`ok` = 128 ASCII
      + the codepage's mapped count) so the golden cannot be blessed wrong.
- [x] Tests: `tests/rt-error/encoding/` fixture for the unrepresentable-character
      raise, including the **U+FFFD sentinel case** explicitly.
      `func_encoding_codepageEncode_unrepresentable` lets the raise escape
      (`Error: 7-705-0003`, exit 255) rather than trapping it, so the fixture pins
      that nothing is substituted in its place. The U+FFFD case is pinned in the
      round-trip fixture and the acceptance suite, against **both** `Windows874`
      (which has holes) and `Windows1252` (which has none) — the second is what
      proves the sentinel guard is unconditional rather than hole-dependent.
- [x] Tests: `tests/syntax/encoding/func_encoding_codepageEncode_invalid` for wrong
      arg types / arity (0, 1 and 3 arguments; a `String` codepage; a `List OF Byte`
      where the `String` goes).
- [x] A new rt fixture needs all four goldens (`build.log`/`.ast`/`.ir`/`.run`);
      `sync-goldens.sh` creates none, and a missing one only surfaces in a full
      `scripts/test-accept.sh` run. Confirmed the hard way in plan-123-A: a fixture
      with no `golden/` directory is treated as a *behavioral* test and run through
      `mfb test`, producing a `test.log` and no artifact goldens at all. The seeding
      order that works is: create `golden/`, `touch` `build.log` and `<pkg>.run`
      (the `.run` is an empty execute-marker whose contents are never compared),
      then run the harness and copy its actual output in.

Acceptance: the whole-range round-trip fixture passes for all 28 single-byte
variants; encoding U+FFFD raises `77050003` rather than emitting a hole byte.
Commit: `—`

### Phase 3 — docs, throughput, and full validation

- [x] Write the member's `intro`/`desc`/`example` and per-`Parameter` `desc` per
      `.ai/man-content.md`; `scripts/man-census.sh --memory-scope` must report 0
      unclassified hits (no C/Rust memory vocabulary). `--memory-scope encoding` → 0,
      `--scope encoding` → 0, `--fill encoding` → 30 pages with 30/30 intro, desc and
      example, 32/32 parameter descriptions, 29/29 types.
- [x] Update `encoding`'s package `DESC` to name the encode direction, and sync the
      embedded spec per `.ai/specifications.md`. Both were written to cover the pair
      in plan-123-A's Phase 3, so `DESC` already names
      `codepageDecode`/`codepageEncode` and the spec's "Legacy single-byte codepages"
      section already documents the encoder, the exactness of the reverse lookup, and
      the U+FFFD guard.
- [x] Add `codepageEncode` coverage to `tests/acceptance/src/encoding.mfb` (one
      project — FUNC names are global). 3 new `TCASE`s (8 in the `codepage` group
      overall); `Tests: 737  Pass: 737  Fail: 0`. Note that a `List OF Byte` is not
      comparable, so the byte assertions compare `encoding::hexEncode` output as a
      `String` rather than the lists themselves.
- [x] Measure encode throughput on a ~100 KB string and record the number. If the
      128-scalar scan dominates, do **not** hand-write an inverted table here —
      record it as a correction against plan-123-A's generator. **~26 ms for a
      realistic 100 KB page** (against ~7 ms to decode it). Decomposed by varying only
      the number of table searches: 23 ms with none, 26 ms at one-in-17, 215 ms with
      every character high — so the scan is 2 ms of 26 on a Western page but ~190 ms
      of 215 on an all-non-ASCII one. Kept, deliberately, with the exact remedy
      written down for whoever needs it (Corrections 7).
- [x] `scripts/man-run-examples.sh encoding --run` — every example must compile and
      run. `bash scripts/man-run-examples.sh encoding --run codepageEncode` →
      `examples: 2   built: 2   ran: 2   failed: 0`, printing `636166e9` and
      `c8e9` / `unrepresentable`.
- [x] `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

Acceptance: `cargo test --no-fail-fast` and `scripts/test-accept.sh` green (watch the
`N ran` count); `mfb man encoding codepageEncode` renders and its examples run; every
moved golden's diff mentions `codepage`/`Codepage` — NOT "confined to `encoding` and
its 3 importers", which plan-123-A measured to be the wrong shape check (its
Corrections 2: `strings`' scalar seam `IMPORT encoding`s, so the churn reaches
everything that reaches `strings`).
Commit: `—`

## Validation Plan

- Tests: the whole-range round-trip rt fixture (the real gate), the
  unrepresentable-character and U+FFFD rt-error fixtures, the syntax fixture, and the
  acceptance-suite addition.
- Coverage check: confirm the new member is in the suite's denominator — grep for it
  in the `codegen_cover` fixtures rather than trusting a green run.
- Runtime proof: take a real `windows-1252` page body, `codepageDecode` it, edit the
  text, `codepageEncode` it back, and confirm the bytes match for the untouched span.
- Doc sync: the member's registry prose, `encoding`'s package `DESC`, the embedded
  spec, and `scripts/man-census.sh --fill encoding` showing full coverage.
- Acceptance: `cargo test --no-fail-fast` **and** `scripts/test-accept.sh` — the
  acceptance harness is not part of `cargo test`.

## Open Decisions

Both are **RESOLVED**.

- **RESOLVED — grapheme iteration**, as recommended. `strings::graphemes(value AS
  String) AS List OF String` walks user-perceived characters, and `len(ch)` counts
  Unicode scalars for a `String` (`mfb man general len`), so `len(ch) <> 1` is a
  clean, exact test for "this grapheme has more than one scalar and therefore no
  single-byte form". A scalar loop would instead encode the base letter and silently
  drop its combining mark — a wrong-bytes result rather than a raise. The
  counter-case (a lone combining mark now raises) is the right outcome: no
  single-byte codepage in the set has a byte for a combining mark, so a scalar loop
  would have raised on it too.
- **RESOLVED — `Utf8` encodes; there are no `Utf16Le`/`Utf16Be` variants.**
  plan-123-A settled this (its Open Decisions): `utf16Decode`/`utf16Encode` work in
  UTF-16 **code units**, not bytes, so those two variants would have been new
  byte-order behavior rather than a delegation, and they were dropped.
  `codepageEncode` therefore has exactly one delegation arm,
  `RETURN __encoding_utf8Encode(text)` — the `String -> List OF Byte` overload,
  selected by this member's declared return type.

## Corrections

1. **§2's UNVERIFIED duplicate premise is now measured, and it holds.** No code
   point repeats within any of the 27 vendored index files
   (`python3 scripts/audit_codepage_index.py` → `files with a repeated code point:
   0`), so a plain scalar search over a table is unambiguous and no lowest-byte-wins
   tie-break is needed. The check is standing, not one-shot: the script exits
   non-zero if a future re-fetch introduces a duplicate.

2. **The not-found guard is `strings::contains`, not an inline `TRAP`/`RECOVER`.**
   §4 sketched `LET idx = strings::find(...) TRAP(e) RECOVER -1 END TRAP` and told
   the implementer to confirm `find`'s behavior first. Confirmed
   (`mfb man strings find`): it raises `ErrNotFound` and never returns a `-1`
   sentinel — *and* the same page says to guard with `strings::contains` when absence
   is an ordinary outcome. It is ordinary here: every unrepresentable character takes
   that path, so it is a control-flow branch, not an exception. The body now guards
   with `contains` and calls `find` only once a match is known to exist. Same
   behavior, no `TRAP` on the common failure path.

3. **`Codepage` has 29 variants, not 28.** plan-123-A added `Utf8` at discriminant 0
   (see its Open Decisions). §2's inherited count is corrected; the round-trip
   fixture covers the 28 single-byte variants and `Utf8` is covered separately, since
   a lone high byte is not valid UTF-8 and has no round trip through it.

4. **The scalar of a grapheme comes from `__encoding_codepoints`.** §4 wrote
   `<code point of ch>` as a placeholder. The package's existing
   `__encoding_codepoints(String) AS List OF Integer` (`helper_codepoints.rs`) is
   that reader; no second path was added.

5. **The first Prerequisites row required plan-123-A to be *archived*, which it
   cannot be until this plan lands.** Archiving moves the plan documents to
   `planning/completed/`, and that is the final step of the whole `plan-123` feature
   — after B is written, landed and merged. As written the two plans deadlocked: A
   is not archived until B is done, and B could not start until A was archived. The
   row is corrected to check what it was protecting — that A's work is complete and
   its gates are green — which is verifiable now and is the actual precondition for
   building an encoder on A's enum and tables.

7. **The reverse scan was measured, and it is kept — with the caveat written down
   rather than left implicit.** Phase 3's task said: measure, and *if the
   128-scalar scan dominates, do not hand-write an inverted table here — record it
   as a correction against plan-123-A's generator*. Measured, on three 100 KB
   windows-1252 strings that differ only in how many characters need the table
   search (`/tmp/p123bench`, 3 runs each):

   | body | table searches | encode ms |
   |---|---|---|
   | all ASCII (every char takes the `< 128` fast path) | 0 | 23, 25, 23 |
   | one high byte in 17 (a realistic Western page) | ~6,000 | 26, 25, 25 |
   | every character high (a Cyrillic / Greek / Hebrew page) | 102,400 | 273, 215, 215 |

   So the scan does **not** dominate a realistic page — 2 ms of 26. The ~23 ms floor
   is the per-grapheme walk itself (`strings::graphemes`, `len`,
   `__encoding_codepoints`, `collections::append`), paid whether or not a search
   happens. It **does** dominate an all-non-ASCII body, at ~190 ms of 215 — about
   1.9 µs per character, i.e. the expected ~128 scalar comparisons.

   Kept anyway, deliberately: 215 ms to encode a 100 KB fully-non-ASCII body is slow
   but not pathological, encoding is the rarer direction (you decode a page to read
   it), and a second generated table doubles the data a reviewer must trust in a plan
   whose non-goals forbid touching A's tables. **What a future session needs to know
   to revisit it:** the fix is not a hand-written table and not a per-call `Map` —
   it is a generator change in plan-123-A's `gen_codepage_tables.py`, emitting per
   codepage a 128-scalar String of its code points **sorted ascending** plus a
   parallel 128-scalar String giving the byte at each sorted position, so the lookup
   becomes a 7-step binary search over `strings::mid` instead of a 128-scalar scan.
   That would take the all-high case from ~215 ms to roughly the ~25 ms floor; it
   cannot improve the ASCII case at all, because that case never searches.

8. **The plan's `out = out & ch` warning does NOT apply to this member, but for a
   different reason than plan-123-A's.** A measured that a `String` accumulator beats
   a `List OF String` + `strings::join` by ~3x on the decode path (its Corrections 4).
   The encoder accumulates a `List OF Byte`, which has no `&` operator at all, so
   `collections::append` into a same-function local is the only shape available and
   is also the in-place one. No change was needed here; recording it so the next
   reader does not "fix" the encoder to match the decoder.

## Summary

The whole plan is one member over data plan-123-A already generated and validated,
which is why it is medium rather than large. The only subtle correctness case is the
U+FFFD hole sentinel matching in a reverse search — pinned by its own test — and the
only unproven premise is whether any single index table repeats a code point, settled
in Phase 1 by a script over committed files. Untouched: the enum and its
discriminants, the generated tables, `codepageDecode`, and every pre-existing
`encoding` member.
