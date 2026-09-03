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
| plan-123-A is complete and archived | `ls planning/completed/plan-123-A-*` → one match | NOT MET |
| `encoding::Codepage` exists with its final variant set | `./target/release/mfb man encoding types \| grep -c Codepage` → non-zero | NOT MET |
| `__encoding_codepageTable` returns a 128-scalar literal per variant | read `src/codegen/builtins/encoding/helper_codepage_table.rs` | NOT MET |
| The vendored index files are committed | `ls tools/codepage-index/index-windows-1252.txt` | NOT MET |

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
| `Codepage` variants to cover | 28 (as landed by A) | `./target/release/mfb man encoding types` → count the `Codepage` variants |
| Distinct tables to search | 27 | `ls tools/codepage-index/*.txt \| wc -l` |
| Scalars searched per unmappable character (worst case) | 128 | table length, by construction |

### Verified properties

- **The reverse lookup is unambiguous.** Within one WHATWG single-byte index, no code
  point appears twice — so a character maps to at most one byte and
  `strings::find(table, ch)` cannot pick the wrong one. **UNVERIFIED as written:
  Phase 1's first task is to check this across all 27 vendored files** and record the
  result here. If any table does contain a duplicate, the design changes (lowest byte
  wins, documented) rather than the check being dropped.
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

**Where design uncertainty concentrates — schedule first:** whether any table has a
duplicate code point (§2 Verified properties), which decides whether a plain search
is sound. It is one script over 27 committed files; run it before writing the member.

**Byte-identity is NOT this plan's gate.** This adds a member to a widely-imported
injected package, so `.ir`/`.ncodesum` line shifts are expected in `encoding` and its
3 importers (csv, json, regex — measured in plan-123-A). The gate is the round-trip
rt fixture. A diff outside those packages is a real signal: root-cause it (objdump
one fixture), do not regenerate past it.

Rejected alternatives:

- **Build a `Map OF String TO Byte` per call** — rejected: allocates a 128-entry map
  per call to save a 128-scalar scan; the scan is the cheaper of the two, and this
  runtime punishes per-call allocation (see the browser example's measured per-edit
  tree-rebuild cost).
- **Generate a second, inverted table** — rejected for now: doubles the generated
  data and the audit surface to remove a bounded 128-scalar scan. Revisit only if
  Phase 3's throughput measurement says so, and then as a generator change in
  plan-123-A's file, not a hand-written table here.
- **Lossy fallback to `?`** — rejected: silently corrupts data. A caller that wants
  it can catch `77050003` and substitute.

## 4. Detailed Design

```
FUNC __encoding_codepageEncode(codepage AS Codepage, text AS String) AS List OF Byte
  LET table AS String = __encoding_codepageTable(codepage)
  MUT out AS List OF Byte = []
  FOR EACH ch IN strings::graphemes(text)
    ' A grapheme wider than one scalar has no single-byte representation.
    IF len(ch) <> 1 THEN
      FAIL error(77050003, "character not representable in this codepage")
    END IF
    LET cp AS Integer = <code point of ch>
    IF cp < 128 THEN
      out = collections::append(out, toByte(cp))
    ELSE
      ' Reject the sentinel BEFORE searching, or it matches a table hole.
      IF ch = "\u{FFFD}" THEN
        FAIL error(77050003, "character not representable in this codepage")
      END IF
      LET idx AS Integer = strings::find(table, ch) TRAP(e)
        RECOVER -1
      END TRAP
      IF idx < 0 THEN
        FAIL error(77050003, "character not representable in this codepage")
      END IF
      out = collections::append(out, toByte(idx + 128))
    END IF
  NEXT
  RETURN out
END FUNC
```

Notes the implementer must not skip:

- `strings::find` **raises rather than returning -1 when not found** (it is the
  not-found case the package documents); the inline `TRAP`/`RECOVER` above is load
  bearing. Confirm its actual not-found behavior by reading
  `mfb man strings find` before writing this — do not assume either way.
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

- [ ] Check all 27 vendored `tools/codepage-index/*.txt` for a code point appearing
      more than once within a single file. Record the result in §2 Verified
      properties, replacing the UNVERIFIED note.
- [ ] If any duplicate exists: document the tie-break (lowest byte wins) in §4 and
      add a fixture pinning it. If none: record that, so the plain search is
      justified by measurement rather than assumption.

Acceptance: §2's duplicate claim is answered with the command that answered it, and
§4's design reflects the answer.
Commit: `—`

### Phase 2 — the member

- [ ] Add `src/codegen/builtins/encoding/func_codepage_encode.rs` per §4, and
      register it in `src/codegen/builtins/encoding/mod.rs:register`.
- [ ] Read `mfb man strings find` and confirm the not-found behavior the body
      depends on; adjust §4 and the body to match what it actually does.
- [ ] Tests: **round-trip** rt fixture — for every `Codepage` variant, every byte
      0x80–0xFF that decodes without raising must survive
      `codepageEncode(cp, codepageDecode(cp, [b])) == [b]`. Whole-range, not a sample.
- [ ] Tests: `tests/rt-error/encoding/` fixture for the unrepresentable-character
      raise, including the **U+FFFD sentinel case** explicitly.
- [ ] Tests: `tests/syntax/encoding/func_encoding_codepageEncode_invalid` for wrong
      arg types / arity.
- [ ] A new rt fixture needs all four goldens (`build.log`/`.ast`/`.ir`/`.run`);
      `sync-goldens.sh` creates none, and a missing one only surfaces in a full
      `scripts/test-accept.sh` run.

Acceptance: the whole-range round-trip fixture passes for all 28 variants; encoding
U+FFFD raises `77050003` rather than emitting a hole byte.
Commit: `—`

### Phase 3 — docs, throughput, and full validation

- [ ] Write the member's `intro`/`desc`/`example` and per-`Parameter` `desc` per
      `.ai/man-content.md`; `scripts/man-census.sh --memory-scope` must report 0
      unclassified hits (no C/Rust memory vocabulary).
- [ ] Update `encoding`'s package `DESC` to name the encode direction, and sync the
      embedded spec per `.ai/specifications.md`.
- [ ] Add `codepageEncode` coverage to `tests/acceptance/src/encoding.mfb` (one
      project — FUNC names are global).
- [ ] Measure encode throughput on a ~100 KB string and record the number. If the
      128-scalar scan dominates, do **not** hand-write an inverted table here —
      record it as a correction against plan-123-A's generator.
- [ ] `scripts/man-run-examples.sh encoding --run` — every example must compile and run.
- [ ] `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

Acceptance: `cargo test --no-fail-fast` and `scripts/test-accept.sh` green (watch the
`N ran` count); `mfb man encoding codepageEncode` renders and its examples run; golden
deltas confined to `encoding` and its 3 importers.
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

- **Grapheme iteration vs. scalar iteration** — recommended graphemes, so a combining
  sequence raises cleanly instead of encoding its base and dropping the mark. The
  counter-case is that a lone combining mark then raises where a scalar loop would
  have emitted a byte; for a single-byte codepage that byte would be wrong anyway.
  (§4)
- **Whether `Utf8`/`Utf16Le`/`Utf16Be` variants encode too** — depends on
  plan-123-A's open decision on whether those variants exist. If A includes them,
  this member must handle them by delegating to `utf8Encode`/`utf16Encode`; if A
  excludes them, nothing to do. Resolve A's decision first. (§plan-123-A Open
  Decisions)

## Corrections

<Filled in DURING execution.>

## Summary

The whole plan is one member over data plan-123-A already generated and validated,
which is why it is medium rather than large. The only subtle correctness case is the
U+FFFD hole sentinel matching in a reverse search — pinned by its own test — and the
only unproven premise is whether any single index table repeats a code point, settled
in Phase 1 by a script over committed files. Untouched: the enum and its
discriminants, the generated tables, `codepageDecode`, and every pre-existing
`encoding` member.
