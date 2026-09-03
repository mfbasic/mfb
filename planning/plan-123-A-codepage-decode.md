# plan-123-A: the `Codepage` enum, the single-byte tables, and `encoding::codepageDecode`

Last updated: 2026-09-02
Overall Effort: x-large (1d–3d)
Effort: large (3h–1d)
Depends on: nothing

Adds `EXPORT ENUM encoding::Codepage` and
`encoding::codepageDecode(codepage AS Codepage, bytes AS List OF Byte) AS String`,
backed by the WHATWG Encoding Standard's legacy single-byte index tables.

**The single behavioral outcome:** `encoding::codepageDecode(Codepage.Windows1252,
<bytes 99 97 102 233>)` returns `"café"`, and the same call for every other
single-byte `Codepage` variant returns that codepage's defined text — where today
the only decoder is `toString`/`utf8Decode`, which raises `77020004` on those exact
bytes. A byte with no mapping in the selected codepage raises `ErrInvalidFormat`
(`77050003`), matching every other decoder in the package.

The motivation is concrete: `http::Response.body` is a `List OF Byte` and the only
way to read it as text is UTF-8. A page served as `windows-1252` — still a large
share of the web — cannot be read at all. This is the gap recorded in the
`examples/browser` audit; `bugs/bug-486` is the *separate* defect that the UTF-8
failure is also uncatchable at the call site.

References:

- WHATWG Encoding Standard — <https://encoding.spec.whatwg.org/>; the machine-readable
  index at `https://encoding.spec.whatwg.org/encodings.json` and the per-encoding
  tables at `https://encoding.spec.whatwg.org/index-<name>.txt`. **Fetch the index
  files; do not transcribe a table from memory or from prose.**
- §"single-byte decoder" of that spec — the algorithm this plan implements verbatim.
- `AGENTS.md` → "Read before that kind of work": `.ai/resources-packages.md`
  (builtin-package authoring seams), `.ai/testing-gates.md` (acceptance harness,
  golden regeneration), `.ai/man-content.md` (man page content standard).
- `.ai/specifications.md` — the embedded spec must stay current with the change.
- Precedents this design mirrors, all read before writing this plan:
  `src/codegen/builtins/money/mod.rs:register` (a registry `add_enum` rendered into
  injected source), `src/codegen/builtins/datetime/func_weekday.rs` (an MFB-bodied
  member consuming and returning a registry enum unqualified — `RETURN Weekday.Monday`),
  `src/codegen/builtins/encoding/func_hex_decode.rs` (a full `RegistryFunction`
  registration with `Body::mfb` and `FAIL error(77050003, …)`),
  `src/codegen/builtins/encoding/helper_html_entity.rs` (how a lookup table is
  written today).

## Prerequisites

These are a precondition on the whole `plan-123` feature, not a dependency to
negotiate. Sub-plan B points here.

| Must be true | Command | Status |
|---|---|---|
| The `encoding` package registers on the clean-room registry with `Body::mfb` members | `grep -c 'Body::mfb' src/codegen/builtins/encoding/func_*.rs` → non-zero | MET |
| `ErrInvalidFormat` (`77050003`) is already an `encoding` error code, so no new `data_objects.rs` row is needed | `grep -rn '77050003' src/codegen/builtins/encoding/ \| head -1` → hits `func_hex_decode.rs:33` | MET |
| A registry enum can be consumed by an MFB-bodied member in the same package | read `src/codegen/builtins/datetime/func_weekday.rs:61` — `RETURN Weekday.Monday` inside a `Body::mfb` body | MET |
| The release compiler builds | `cargo build --release --bin mfb` | MET |

`bugs/bug-486` (an inline `TRAP` does not catch `toString(List OF Byte)`) is **not**
a prerequisite and must not be folded in. `codepageDecode` is an ordinary fallible
`Body::mfb` member — it is not in `inline_builtin_is_infallible`'s census, so an
inline `TRAP` on it works today. The two are independent; do not braid them.

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `encoding::codepageDecode(Codepage.Windows1252, bytes)` decodes bytes 0x80–0xFF
  through the WHATWG `index-windows-1252` table and bytes 0x00–0x7F as ASCII,
  returning the correct `String`.
- Every single-byte `Codepage` variant decodes its whole 0x80–0xFF range to exactly
  the code points its WHATWG index file lists — verified against the index files
  themselves, not by spot-check.
- A byte that the selected codepage leaves unmapped raises `ErrInvalidFormat`
  (`77050003`), consistent with every other `encoding` decoder.
- `mfb man encoding codepageDecode` and `mfb man encoding types` render the member
  and the enum, and every example on those pages compiles and runs.

### Non-goals (explicit constraints)

- **No label resolution.** Mapping the *string* `"windows-1252"` (or any of the
  spec's ~220 aliases) to a `Codepage` is not in this plan. It is the natural
  follow-up for the browser use case and gets its own plan.
- **No multi-byte encodings.** GBK, gb18030, Big5, EUC-JP, ISO-2022-JP, Shift_JIS
  and EUC-KR are out — see the measured populations below for why they cannot share
  this representation. A separate plan, not a phase of this one.
- **`toString(List OF Byte)` semantics are unchanged.** It stays UTF-8-only and
  keeps raising `77020004`. This plan adds a decoder; it does not redefine an
  existing one, and it does not fix `bug-486`.
- **No change to `utf8Encode`/`utf8Decode`, `utf16*`, `utf32*`,** or any existing
  `encoding` member's signature, body, or error behavior.
- **No lossy decoding.** An unmapped byte raises; it does not silently become
  U+FFFD. (U+FFFD *is* used as the internal table hole sentinel — that is an
  implementation detail of the table, never an output value.)
- **No new error code.** Reuse `77050003`; adding an `Err*` would require a
  `src/codegen/data_objects.rs` row and is unnecessary here.
- The enum's variant *order* fixes its discriminants. Once landed, variants may only
  be **appended**, never reordered or removed.

## 2. Current State

`encoding` is a pure-MFBasic builtin package: each member carries a `Body::mfb`
MFBASIC source body from its own `func_*.rs`, and shared `__encoding_*` helpers
register via `add_helper` (`src/codegen/builtins/encoding/mod.rs:register`). It is
injected by a dedicated late pass (`augmented_project`) rather than the generic
`registry::augment_project`, because `crypto`/`strings` depend on it
(`src/codegen/builtins/encoding/mod.rs:26-42`).

Lookup tables today are written as MFBASIC `IF`-chains — `helper_html_entity.rs`
is 151 lines for the common HTML named set. That representation does not scale to
this plan (see Design).

Decoders raise `FAIL error(77050003, "<reason>")` on malformed input
(`func_hex_decode.rs:33,43`), and the package description already documents
`ErrInvalidFormat` as the decoder failure mode
(`src/codegen/builtins/encoding/mod.rs:DESC`).

A registry enum is declared with `pkg.add_enum(RegistryEnum { name, export, variants })`
and rendered into the injected source as `EXPORT ENUM … END ENUM`
(`src/codegen/registry/mod.rs:RegistryEnum::render:713`). An enum-typed parameter is
declared `ParameterType::named("Codepage")`
(`src/codegen/builtins/money/func_set_rounding.rs:65`).

### Measured populations

| What | Count | Command |
|---|---|---|
| Encodings in the WHATWG standard | 40 | `curl -sS https://encoding.spec.whatwg.org/encodings.json` → summed `encodings` across groups |
| **Legacy single-byte** encodings (this plan's scope) | **28** labels / **27** distinct index files | same JSON, "Legacy single-byte encodings" group = 28; `index-iso-8859-8-i.txt` is HTTP 404 — ISO-8859-8-I shares ISO-8859-8's table |
| Mappings per single-byte table | ≤ 128 | `grep -cvE '^\s*(#\|$)' idx-windows-1252.txt` → 128 |
| Single-byte tables that are **not** full 128 | 8 of 27 | measured per file: ISO-8859-3 = 121, ISO-8859-6 = 83, ISO-8859-7 = 125, ISO-8859-8 = 92, windows-874 = 120, windows-1253 = 125, windows-1255 = 118, windows-1257 = 126 |
| Total single-byte mappings | 3,371 | sum of the per-file row counts above |
| Multi-byte table sizes (**out of scope**) | jis0208 7,724 · big5 18,590 · gb18030 23,940 · euc-kr 17,048 = **67,302** | `grep -cvE '^\s*(#\|$)' idx-<name>.txt` per file |
| Existing `encoding` members | 28 | `ls src/codegen/builtins/encoding/func_*.rs \| wc -l` → 28 |
| Existing `tests/syntax/encoding` fixture dirs | 29 | `ls tests/syntax/encoding \| wc -l` → 29 |
| Existing `tests/rt-error/encoding` fixture dirs | 1 | `ls tests/rt-error/encoding \| wc -l` → 1 |
| `.ir` goldens in the tree (churn denominator) | 824 | `find tests -name '*.ir' \| wc -l` → 824 |
| `.ncodesum` goldens in the tree | 141 | `find tests -name '*.ncodesum' \| wc -l` → 141 |
| Builtin packages that `IMPORT encoding` | 3 (csv, json, regex) | `grep -rn 'add_imports' src/codegen/builtins/*/mod.rs \| grep encoding` → csv:76, json:93, regex:159 |

The 67,302 : 3,371 ratio is the whole reason multi-byte is a separate plan: the
single-byte set fits in 27 string literals, and gb18030 alone does not.

### Verified properties

- **U+FFFD is a safe hole sentinel.** The highest code point across all 27
  single-byte index files is U+FB02 (measured by parsing every fetched index and
  taking the max), so no table maps any byte to U+FFFD and the sentinel can never
  collide with a real mapping. *Verified by measurement, not assumption.*
- **Every mapped code point is BMP** (max U+FB02), so each table entry is exactly
  one UTF-16-independent scalar and a 128-scalar `String` literal holds a whole
  table with one scalar per byte. *Verified by the same measurement.*
- **Bytes 0x00–0x7F are ASCII in every single-byte encoding.** The WHATWG
  single-byte decoder's first step is "If byte is an ASCII byte, return a code point
  whose value is byte", and the index files themselves only cover 0x80–0xFF (128
  rows). *Verified by reading the spec algorithm and the file format.*
- **An MFB-bodied member can consume a registry enum.** `datetime::weekday` is
  `Body::mfb` and its body writes `RETURN Weekday.Monday` unqualified
  (`func_weekday.rs:61-67`) while its registration uses
  `ParameterType::named("Weekday")` (`:86`). *Verified by reading both.*
- **No new `data_objects.rs` row is needed.** `77050003` is already emitted by
  `encoding` members. *Verified by grep.*
- **UNVERIFIED — golden blast radius.** Adding declarations to the injected
  `encoding` source shifts subsequent line numbers, and embedded `ErrorLoc`s ride
  those lines. How many `.ir`/`.ncodesum` goldens actually move is unmeasured;
  measuring it is Phase 1's first task, not an assumption.
- **UNVERIFIED — injected-source size.** 27 tables × 128 `\u{XXXX}` escapes is on the
  order of tens of KB of added injected source. Compile-time and the AArch64
  large-function branch-range limit are believed fine at this size but are not
  measured; Phase 1 measures both.

## 3. Design Overview

Three layers, each landing on its own.

**(a) The table data.** Each single-byte codepage becomes one **128-scalar MFBASIC
`String` literal**: scalar *i* is the code point for byte `128 + i`, and an unmapped
byte is `\u{FFFD}`. Decoding a high byte is then `strings::mid(table, b - 128, 1)` —
O(1)-ish, no branch chain. This is the piece that makes the whole feature cheap, and
it is why the `IF`-chain style of `helper_html_entity.rs` is *not* mirrored: 27 × 128
= 3,456 `IF` arms would be unreadable, slow, and a large-function hazard.

The literals are **generated, not typed**: a script reads the vendored WHATWG index
files and emits the Rust `const`. Hand-transcribing 3,371 mappings is the single
largest correctness risk in this plan and is designed out rather than reviewed.

**(b) The enum + dispatch.** `RegistryEnum { name: "Codepage", export: true, … }`
with one variant per label, and a private
`__encoding_codepageTable(cp AS Codepage) AS String` whose `MATCH` returns the right
literal (`CASE ELSE` required — see `.ai/resources-packages.md`). ISO-8859-8-I
returns ISO-8859-8's table; that is the spec's own position, not a shortcut.

**(c) `codepageDecode`.** An ordinary `Body::mfb` member: ASCII passthrough below
0x80, table lookup above, `FAIL error(77050003, …)` on a `\u{FFFD}` hole.

**Where design uncertainty concentrates — schedule first.** Two premises are
unproven and both are cheap to test, so Phase 1 tests them before any table is
generated: (1) that a 128-scalar `\u{…}`-escaped literal round-trips through the
injected-source pipeline with the scalar indices intact, and (2) the golden/size
blast radius of growing the injected `encoding` source. A single throwaway table
proves both in minutes. Do not generate 27 tables before Phase 1 answers them.

**Where correctness risk concentrates — schedule last.** The generated tables
themselves. Phase 3's acceptance is a *differential* check against the index files
(decode every byte 0x80–0xFF for every codepage and compare to the file), not a
sample — a hand-picked spot check would pass with a whole table shifted by one.

**Byte-identity is NOT this plan's gate.** This plan adds behavior and adds
declarations to a widely-imported injected package; `.ir` and `.ncodesum` goldens
are *expected* to move. The gate is rt-behavior plus the differential table check.
A golden diff here reads as the plan working. What must be checked is the diff's
*shape*: only line-number shifts and the new member's own entries. A diff in a
package that does not import `encoding` would be a real signal — root-cause it
(objdump one fixture), do not regenerate past it.

Rejected alternatives:

- **`IF`-chain tables** (mirroring `helper_html_entity.rs`) — rejected: 3,456 arms,
  linear lookup, and a large-function branch-range hazard on AArch64.
- **A `Map OF Byte TO String` per codepage built at call time** — rejected: rebuilds
  27 maps' worth of work per call; the string literal is already O(1) and free.
- **One member per codepage** (`windows1252Decode`, …) — rejected: 27 members, 27 man
  pages, and the caller still needs a runtime dispatch for the browser use case.
- **Taking the codepage as a `String` label instead of an enum** — rejected: the user
  asked for an enum, and an enum makes an unknown codepage a *compile* error rather
  than a runtime one. Label resolution is the separate follow-up plan.
- **Including the multi-byte encodings** — rejected on the measured 67,302-mapping
  table size; a fundamentally different storage mechanism, hence its own plan.

## 4. Detailed Design

### 4.1 The generator and vendored index files

- Vendor the WHATWG index files under `tools/codepage-index/index-<label>.txt`,
  committed, with the fetch URL and retrieval date recorded in a `README.md`
  beside them. Vendoring makes the build network-free and the tables auditable by
  `diff` against upstream.
- `scripts/gen-codepage-tables.py` reads those files and writes
  `src/codegen/builtins/encoding/helper_codepage_table.rs` — the `const BODY` holding
  `__encoding_codepageTable`. The script is re-runnable and its output is committed;
  regenerating must produce a byte-identical file (that is itself a test).

### 4.2 Table encoding

For codepage *C* with index rows `(pointer, codepoint)`:

```
table[i] = codepoint for pointer i, or U+FFFD when the index has no row for i
```

rendered as one MFBASIC string literal of 128 `\u{XXXX}` escapes. Escapes are used
uniformly — including for ASCII-range and printable values — so the generated line
is mechanical and diffable, and no literal ever contains a quote, backslash or
newline that would need its own escaping rule.

### 4.3 `codepageDecode`

```
FUNC __encoding_codepageDecode(codepage AS Codepage, bytes AS List OF Byte) AS String
  LET table AS String = __encoding_codepageTable(codepage)
  MUT out AS String = ""
  FOR EACH b IN bytes
    LET n AS Integer = toInt(b)
    IF n < 128 THEN
      out = out & __encoding_fromCodepoint(n)      ' existing helper
    ELSE
      LET ch AS String = strings::mid(table, n - 128, 1)
      IF ch = "\u{FFFD}" THEN
        FAIL error(77050003, "byte not mapped in this codepage")
      END IF
      out = out & ch
    END IF
  NEXT
  RETURN out
END FUNC
```

`__encoding_fromCodepoint` already exists (`helper_from_codepoint.rs`); reuse it
rather than adding a second path.

**Repeated `out = out & ch` is a known cost shape in this runtime.** Phase 3 measures
decode throughput on a realistic body (a ~100 KB page) before the plan is called done;
if it is not acceptable, the fix is to accumulate into a `List OF String` and
`strings::join` once, not to change the interface.

### 4.4 Registration

`func_codepage_decode.rs`, mirroring `func_hex_decode.rs`:
`Parameter { name: "codepage", ty: ParameterType::named("Codepage") }`,
`Parameter { name: "bytes", ty: ParameterType::list_of(ParameterType::Byte) }`,
`return_type: ParameterType::String`, `errors: vec![<ErrInvalidFormat>]` (match the
convention the other decoders use — read one that declares an error before filling
this in; `func_hex_decode.rs` registers `errors: vec![]` despite raising, so confirm
which is correct rather than copying blindly).

## Compatibility / Format Impact

- **Added public surface:** `encoding::Codepage` (enum) and
  `encoding::codepageDecode`. Both are new names; nothing existing is shadowed
  (`mfb man encoding` lists no `codepage*` member today).
- **Unchanged:** every existing `encoding` member's signature, body, and errors;
  `toString(List OF Byte)`; all wire/file formats; layout/ABI.
- **Enum discriminant stability:** variant order is a compatibility surface from the
  moment this lands. Append-only thereafter.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work. Use `- [~]` for partial with one line on what remains. Mark a task
> moot with `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` the moment a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — falsify the two premises (one throwaway table)

Cheapest experiment that could invalidate the design, before any bulk work.

- [ ] Add a single hand-written `Codepage` enum with one variant (`Windows1252`) and
      one 128-scalar table literal, plus a minimal `codepageDecode`, to
      `src/codegen/builtins/encoding/`. Confirm `strings::mid(table, i, 1)` returns
      the intended scalar for a spread of indices including a `\u{FFFD}` hole.
- [ ] Measure the golden blast radius: `cargo test --no-fail-fast` and
      `scripts/test-accept.sh`, then record **exactly** which `.ir`/`.ncodesum`
      goldens moved and confirm every one is either an `encoding` fixture or a
      csv/json/regex fixture (the 3 measured importers). Write the counts into
      §Measured populations, replacing the two UNVERIFIED rows.
- [ ] Measure injected-source growth and build time for one table; extrapolate to 27
      and confirm it is nowhere near the AArch64 large-function limit.

Acceptance: a one-codepage `codepageDecode` decodes `99 97 102 233` to `café` and
raises `77050003` on a hole byte, run as an rt fixture; the golden delta is fully
enumerated and every moved golden is attributed to `encoding` or one of its 3
importers.
Commit: `—`

### Phase 2 — vendor the index files and generate all 27 tables

- [ ] Vendor `tools/codepage-index/index-<label>.txt` for all 27 distinct tables,
      with a `README.md` recording the source URL and retrieval date.
- [ ] Write `scripts/gen-codepage-tables.py`; generate
      `src/codegen/builtins/encoding/helper_codepage_table.rs`.
- [ ] Replace Phase 1's hand-written enum with the full `RegistryEnum` (28 variants;
      ISO-8859-8-I dispatches to ISO-8859-8's table) in
      `src/codegen/builtins/encoding/mod.rs:register`.
- [ ] Tests: a generator idempotence check — re-running the script produces a
      byte-identical `helper_codepage_table.rs`.

Acceptance: the generator reproduces the committed table file byte-for-byte, and
`mfb man encoding types` renders all 28 `Codepage` variants.
Commit: `—`

### Phase 3 — the member, differential validation, and docs (largest blast radius last)

- [ ] Promote Phase 1's member into `func_codepage_decode.rs` with full
      `intro`/`desc`/`example` and per-`Parameter` `desc`, per `.ai/man-content.md`
      (no C/Rust memory vocabulary — check with `scripts/man-census.sh --memory-scope`,
      which must report 0 unclassified hits).
- [ ] Tests: **differential** rt fixture — for every `Codepage` variant, decode all
      128 bytes 0x80–0xFF and compare against the vendored index file. A spot check
      is not acceptable; a whole table off by one must fail.
- [ ] Tests: `tests/syntax/encoding/func_encoding_codepageDecode_invalid` (wrong arg
      types / arity), and an `tests/rt-error/encoding/` fixture for the unmapped-byte
      `77050003` raise. A new rt fixture needs all four goldens
      (`build.log`/`.ast`/`.ir`/`.run`) — `sync-goldens.sh` creates none, and a
      missing one only surfaces in a full `test-accept.sh` run.
- [ ] Tests: add `codepageDecode` coverage to `tests/acceptance/src/encoding.mfb`
      (one project — FUNC names are global).
- [ ] Docs: update `encoding`'s `DESC` (`mod.rs`) to name the codepage family;
      update the embedded spec per `.ai/specifications.md`.
- [ ] Measure decode throughput on a ~100 KB body (§4.3) and record the number.
- [ ] Run `scripts/man-run-examples.sh encoding --run` — every example on the new
      page must compile and run.
- [ ] `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

Acceptance: the differential fixture passes for all 28 variants against the vendored
index files; `cargo test --no-fail-fast` and `scripts/test-accept.sh` are green (watch
the `N ran` count); `mfb man encoding codepageDecode` renders and its examples run;
golden deltas are only line shifts plus the new member's own entries.
Commit: `—`

## Validation Plan

- Tests: the differential all-bytes/all-codepages rt fixture (the real gate), the
  `77050003` rt-error fixture, the syntax fixture, and the acceptance-suite addition.
- Coverage check: confirm the new member is actually exercised — a `codegen_cover`
  fixture may not cover a new member; grep for it rather than trusting a green run.
- Runtime proof: decode a real `windows-1252` page body end-to-end and print it —
  bytes that `toString` raises `77020004` on must render as text.
- Doc sync: `encoding` package `DESC`, the new member's registry prose, the embedded
  spec (`.ai/specifications.md`), and `scripts/man-census.sh --fill encoding` showing
  full coverage.
- Acceptance: `cargo test --no-fail-fast` **and** `scripts/test-accept.sh` — the
  acceptance harness is not part of `cargo test`, so a green cargo run hides stale
  goldens.

## Open Decisions

- **Enum variant naming for numbered labels** — recommended `Windows1252`,
  `Iso8859_2`, `Koi8R`, `Ibm866`, `Macintosh`, `MacCyrillic`, `Windows874`
  (underscore only where digits would otherwise run together ambiguously) vs. strict
  PascalCase with no underscores (`Iso88592`, which reads as "ISO-8859-92"). The
  underscore form is recommended for exactly that ambiguity. Decide in Phase 2 —
  variant order and names are a compatibility surface from the moment it lands.
- **Whether `Codepage` also carries `Utf8`, `Utf16Le`, `Utf16Be`** — recommended
  **yes**, delegating to the existing members. The motivating caller has a charset
  label in hand and wants *one* dispatch point; without these it needs a separate
  branch for the most common encoding of all. Cost is 3 `MATCH` arms. The counter-case
  is that it duplicates existing surface. (§4.2)
- **`errors: vec![]` vs. a declared error entry** — `func_hex_decode.rs:93` registers
  `errors: vec![]` even though its body raises `77050003`. Determine which is correct
  for `mfb man` rendering before filling the field; do not copy the existing value
  blindly. (§4.4)

## Corrections

<Filled in DURING execution.>

## Summary

The engineering risk is the table data, and it is designed out: the tables are
generated from vendored upstream index files and validated differentially against
those same files, so no reviewer has to eyeball 3,371 mappings. The two unproven
premises — that a 128-scalar escaped literal survives the injected-source pipeline,
and that growing `encoding` does not spray golden churn beyond its 3 importers — are
cheap and are tested first, on one throwaway table, before any bulk generation.
Untouched: every existing `encoding` member, `toString`'s UTF-8-only semantics, and
`bug-486`, which is independent and must not be folded in.
