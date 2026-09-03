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
| Total single-byte mappings | **3,342** (was written 3,371 — arithmetic slip, see Corrections) | `python3 scripts/audit_codepage_index.py` → `total mappings: 3342` (19 full tables x 128 = 2,432, plus 121+83+125+92+120+125+118+126 = 910) |
| Multi-byte table sizes (**out of scope**) | jis0208 7,724 · big5 18,590 · gb18030 23,940 · euc-kr 17,048 = **67,302** | `grep -cvE '^\s*(#\|$)' idx-<name>.txt` per file |
| Existing `encoding` members | 28 | `ls src/codegen/builtins/encoding/func_*.rs \| wc -l` → 28 |
| Existing `tests/syntax/encoding` fixture dirs | 29 | `ls tests/syntax/encoding \| wc -l` → 29 |
| Existing `tests/rt-error/encoding` fixture dirs | 1 | `ls tests/rt-error/encoding \| wc -l` → 1 |
| `.ir` goldens in the tree (churn denominator) | 824 | `find tests -name '*.ir' \| wc -l` → 824 |
| `.ncodesum` goldens in the tree | 141 | `find tests -name '*.ncodesum' \| wc -l` → 141 |
| Builtin packages that `IMPORT encoding` **via `add_imports`** | 3 (csv, json, regex) | `grep -rn 'add_imports' src/codegen/builtins/*/mod.rs \| grep encoding` → csv:76, json:93, regex:159 |
| Builtin packages that `IMPORT encoding` **from an injected body** (the real blast radius — see Corrections) | 4 more: `strings`, `crypto`, `udp`, `encoding` itself | `grep -rn 'IMPORT encoding' src/codegen/builtins/` → `strings/helper_scalar_seam.rs:36`, `crypto/func_hash.rs:113,127`, `crypto/func_hkdf.rs:55`, `udp/func_receive.rs:44` |

The 67,302 : 3,342 ratio is the whole reason multi-byte is a separate plan: the
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
- **MEASURED (Phase 1) — golden blast radius: 64 `.ir` goldens, nothing else.**
  `scripts/test-accept.sh ./target/release/mfb /tmp/p123-accept` on the Phase-1
  throwaway (3 enum variants, 2 tables) → `acceptance tests failed: 64 mismatch(es)
  (1348 test(s) ran)`. **All 64 are `.ir`** (`grep 'mismatch:' … | sed 's|.*\.||' |
  sort | uniq -c` → `64 ir`); no `.ast`, `.run`, `build.log` or `.ncodesum` moved.
  All 64 diffs mention `codepage`/`Codepage`, so every one is attributed to this
  change and none is a pre-existing or unrelated signal
  (`python3 /tmp/attribute_mm.py` → `UNATTRIBUTED: 0`). The diff shape is exactly
  the predicted one: line-number shifts inside the injected `encoding` source
  (embedded `ErrorLoc` line values move) plus the new member's own IR.
- **CORRECTED — the blast radius is NOT "encoding + its 3 importers".** The 64
  span `astrings`(9), `crypto`(15), `tcp`(7), `json`(5), `tls`(5), `udp`(3),
  `csv`(3), `regex`(3), `general`(2), plus `encoding`, `strings`, `native`, `term`,
  `threads`, `trap`, `security`. Cause: `strings`' scalar seam carries
  `IMPORT encoding` (`src/codegen/builtins/strings/helper_scalar_seam.rs:36`), so
  **any** program that reaches `strings` embeds the injected `encoding` source. The
  `add_imports` census counted only the 3 direct registry importers. See Corrections.
- **MEASURED (Phase 1) — injected-source size is ~28 KB, nowhere near any limit.**
  The 29-variant generated file is 35,740 bytes of Rust
  (`python3 scripts/gen_codepage_tables.py | wc -c`), of which the injected MFBASIC
  portion is 27 tables × 128 × 8 chars = 27,648 characters of `\u{XXXX}` escapes
  plus the `MATCH` scaffolding. Each table is a rodata string constant, not code, and
  `__encoding_codepageTable` is a 29-arm `MATCH` — three orders of magnitude below
  the ~1 MiB AArch64 function branch-range limit. Release build of `mfb` with the
  full tables: see the Phase 2 measurement.

## 3. Design Overview

Three layers, each landing on its own.

**(a) The table data.** Each single-byte codepage becomes one **128-scalar MFBASIC
`String` literal**: scalar *i* is the code point for byte `128 + i`, and an unmapped
byte is `\u{FFFD}`. Decoding a high byte is then `strings::mid(table, b - 128, 1)` —
O(1)-ish, no branch chain. This is the piece that makes the whole feature cheap, and
it is why the `IF`-chain style of `helper_html_entity.rs` is *not* mirrored: 27 × 128
= 3,456 `IF` arms would be unreadable, slow, and a large-function hazard.

The literals are **generated, not typed**: `scripts/gen_codepage_tables.py` reads the
vendored WHATWG index files and writes the whole Rust file to stdout, per the
`scripts/check-generated.sh` contract. Hand-transcribing 3,342 mappings is the single
largest correctness risk in this plan and is designed out rather than reviewed.

**(b) The enum + dispatch.** The generator emits the `RegistryEnum` and the `MATCH`
arms **together**, from one `CODEPAGES` list — so a variant cannot exist without a
table nor a table without a variant (see Corrections 5). The dispatch is a private
`__encoding_codepageTable(cp AS Codepage) AS String`. **No `CASE ELSE` is needed**:
the enum is declared in the same injected source that matches on it, so the match is
exhaustive and `ir::verify` accepts it — the `.ai/resources-packages.md` note that
prompted this concern is about an *imported* union that verify reads as open.
Measured: the Phase-1 build compiled and ran with no `CASE ELSE`. ISO-8859-8-I
returns ISO-8859-8's table; that is the spec's own position, not a shortcut, and
`iso_8859_8_i_shares_the_iso_8859_8_table` pins it.

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
*shape*: only line-number shifts inside the injected `encoding` source and the new
member's own entries. **The shape check is "every moved golden's diff mentions
`codepage`/`Codepage`", NOT "the fixture imports `encoding`"** — `strings`' scalar
seam `IMPORT encoding`s, so the churn reaches every package that reaches `strings`
(Corrections 2). A moved golden whose diff mentions neither is a real signal —
root-cause it (objdump one fixture), do not regenerate past it.

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
- `scripts/fetch_codepage_index.py` re-fetches them, so refreshing is one command
  and the diff against upstream is reviewable.
  `scripts/audit_codepage_index.py` checks the three data premises this design rests
  on (U+FFFD is a safe sentinel, every mapping is one BMP scalar, no code point
  repeats within a file) and exits non-zero if any fails.
- `scripts/gen_codepage_tables.py` reads those files and emits
  `src/codegen/builtins/encoding/helper_codepage_table.rs` **to stdout** — `VARIANTS`
  (the enum's variant list, each with its index-file label) *and* the `const BODY`
  holding `__encoding_codepageTable`, together, so the two cannot drift
  (Corrections 5). Its output is committed and
  `scripts/check-generated.sh` — the repo's existing generated-artifact gate, run in
  CI — re-runs it and fails on any difference, so a hand edit of the artifact cannot
  land (Corrections 6).

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
  IF codepage = Codepage.Utf8 THEN
    RETURN __encoding_utf8Decode(bytes)          ' existing overload, List OF Byte
  END IF
  LET table AS String = __encoding_codepageTable(codepage)
  MUT parts AS List OF String = []               ' see Corrections 4, not `out & ch`
  FOR EACH b IN bytes
    LET n AS Integer = toInt(b)
    IF n < 128 THEN
      parts = collections::append(parts, __encoding_fromCodepoint(n))  ' existing helper
    ELSE
      LET ch AS String = strings::mid(table, n - 128, 1)
      IF ch = "\u{FFFD}" THEN
        FAIL error(77050003, "byte not mapped in this codepage")
      END IF
      parts = collections::append(parts, ch)
    END IF
  NEXT
  RETURN strings::join(parts, "")
END FUNC
```

`__encoding_fromCodepoint` already exists (`helper_from_codepoint.rs`); reuse it
rather than adding a second path.

**Repeated `out = out & ch` is a known cost shape in this runtime**, so the body above
already accumulates into a `List OF String` and `strings::join`s once (Corrections 4)
— `parts` stays a same-function local, which is the in-place `collections::append`
shape. Phase 3 still measures decode throughput on a realistic body (a ~100 KB page)
before the plan is called done; if that is not acceptable, the fix stays inside the
body and does not change the interface.

### 4.4 Registration

`func_codepage_decode.rs`, mirroring `func_hex_decode.rs`:
`Parameter { name: "codepage", ty: ParameterType::named("Codepage") }`,
`Parameter { name: "bytes", ty: ParameterType::list_of(ParameterType::Byte) }`,
`return_type: ParameterType::String`, and **`errors: vec!["ErrInvalidFormat"]`** —
resolved by measurement, not by copying `func_hex_decode.rs`'s empty list. See
§Open Decisions for why: the field is inert for a `Body::Mfb` member except in
`mfb man`, and 25 `Body::mfb` members elsewhere (including `json::parse`,
`csv::parse`, `regex::match`) declare this exact error.

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

- [x] Add a hand-written `Codepage` enum and 128-scalar table literals, plus a
      minimal `codepageDecode`, to `src/codegen/builtins/encoding/`. Confirm
      `strings::mid(table, i, 1)` returns the intended scalar for a spread of
      indices including a `\u{FFFD}` hole. **Three variants, not one**: `Utf8`
      (the delegation arm), `Windows1252` (a full 128-entry table) and
      `Windows874` (120 entries, so it HAS holes — `Windows1252` alone could not
      have tested the sentinel at all). Proven by
      `tests/rt-behavior/encoding/func_encoding_codepageDecode_rt`: `café`,
      ASCII passthrough, the empty input, the `Utf8` arm, and whole-range
      position-weighted digests `Windows1252 mapped=128 sum=3860234` /
      `Windows874 mapped=120 sum=26122587` — both byte-for-byte equal to the
      vendored index files (`python3 /tmp/check_digest.py`), and
      `tests/rt-error/encoding/func_encoding_codepageDecode_unmapped` raises
      `7-705-0003` on hole byte `0xDB`.
- [x] Measure the golden blast radius: `cargo test --no-fail-fast` and
      `scripts/test-accept.sh`, then record **exactly** which `.ir`/`.ncodesum`
      goldens moved and confirm every one is attributable to this change — i.e. its
      diff mentions `codepage`/`Codepage`. (The plan's original check, "an `encoding`
      or csv/json/regex fixture", is wrong: `strings`' scalar seam `IMPORT
      encoding`s, so the churn reaches everything that reaches `strings`. See
      Corrections 2.) Write the counts into §Measured populations, replacing the two
      UNVERIFIED rows. **Measured** — see §Verified properties: 64 `.ir` goldens
      moved and nothing else (`64 ir`, 1348 ran), 100% attributed
      (`UNATTRIBUTED: 0`); `cargo test --no-fail-fast` over 92 test binaries gave
      3 unit failures, all direct consequences of this change and all fixed here
      (`encoding_registered_on_the_clean_room_registry` 28 → 29 members, and the
      two `cli::man` tests that used `encoding` as their "package with no public
      types" exemplar, re-pointed at `bits` with their assertions unchanged).
      `artifact_gate_all` did not run — it refused to start on a lock held by a
      concurrent gate, which is a refusal and not a result; re-run uncontended in
      Phase 3.
- [x] Measure injected-source growth and build time for one table; extrapolate to 27
      and confirm it is nowhere near the AArch64 large-function limit. **Measured**:
      the full 29-variant generated file is 35,740 bytes of Rust
      (`python3 scripts/gen_codepage_tables.py | wc -c`), whose injected MFBASIC
      portion is 27 x 128 x 8 = 27,648 characters of escapes plus the `MATCH`
      scaffolding — string constants in rodata, reached from a 29-arm `MATCH`, three
      orders of magnitude below the ~1 MiB AArch64 function branch-range limit.
      Release build of `mfb` with the Phase-1 tables: 2m 32s
      (`cargo build --release --bin mfb`), against 2m 47s for the untouched tree —
      i.e. within noise, not a compile-time concern.

Acceptance: a one-codepage `codepageDecode` decodes `99 97 102 233` to `café` and
raises `77050003` on a hole byte, run as an rt fixture; the golden delta is fully
enumerated and every moved golden's diff is attributed to this change.
Commit: `—`

### Phase 2 — vendor the index files and generate all 27 tables

- [ ] Vendor `tools/codepage-index/index-<label>.txt` for all 27 distinct tables,
      with a `README.md` recording the source URL and retrieval date, plus
      `scripts/fetch_codepage_index.py` to re-fetch them.
- [ ] Add `scripts/audit_codepage_index.py`, which checks the three data premises
      (safe U+FFFD sentinel, all-BMP mappings, no repeated code point within a file)
      and exits non-zero if any fails.
- [ ] Write `scripts/gen_codepage_tables.py`; generate
      `src/codegen/builtins/encoding/helper_codepage_table.rs`. It emits the enum's
      `VARIANTS` and the `MATCH` arms together (Corrections 5) and writes to stdout,
      per the `scripts/check-generated.sh` contract (Corrections 6).
- [ ] Replace Phase 1's hand-written enum with the generated one (29 variants:
      `Utf8` plus the 28 WHATWG single-byte labels; ISO-8859-8-I dispatches to
      ISO-8859-8's table) — the generated file's own `register` replaces the
      hand-written `add_enum` in `src/codegen/builtins/encoding/mod.rs:register`.
- [ ] Tests: register the generator in `scripts/check-generated.sh`, so re-running
      it must reproduce `helper_codepage_table.rs` byte-for-byte.
- [ ] Tests: `codepage_tables_match_the_vendored_index_files` — check every scalar of
      every table against `tools/codepage-index/` at test time (the differential on
      the data), plus `every_vendored_index_file_has_a_codepage_variant`,
      `codepage_enum_is_registered_in_generator_order`, and
      `iso_8859_8_i_shares_the_iso_8859_8_table`.
- [ ] Widen Phase 1's rt fixture from 2 codepages to all 28 single-byte variants.

Acceptance: `sh scripts/check-generated.sh` reproduces the committed table file
byte-for-byte; `codepage_tables_match_the_vendored_index_files` passes over all
28 x 128 mappings; and `mfb man encoding types` renders all 29 `Codepage` variants.
Commit: `—`

### Phase 3 — the member, differential validation, and docs (largest blast radius last)

- [ ] Promote Phase 1's member into `func_codepage_decode.rs` with full
      `intro`/`desc`/`example` and per-`Parameter` `desc`, per `.ai/man-content.md`
      (no C/Rust memory vocabulary — check with `scripts/man-census.sh --memory-scope`,
      which must report 0 unclassified hits).
- [ ] Tests: **differential** rt fixture — for every `Codepage` variant, decode all
      128 bytes 0x80–0xFF and compare against the vendored index file. A spot check
      is not acceptable; a whole table off by one must fail. Realized as a
      position-weighted per-codepage digest in the fixture (so the golden stays 28
      short lines) plus `codepage_digests_match_the_vendored_index_files`, which
      recomputes every one of those lines from `tools/codepage-index/` and compares
      against the fixture's golden — so the golden cannot be blessed wrong.
- [ ] Tests: `tests/syntax/encoding/func_encoding_codepageDecode_invalid` (wrong arg
      types / arity), and an `tests/rt-error/encoding/` fixture for the unmapped-byte
      `77050003` raise. A new rt fixture needs all four goldens
      (`build.log`/`.ast`/`.ir`/`.run`) — `sync-goldens.sh` creates none, and a
      missing one only surfaces in a full `test-accept.sh` run.
- [ ] Tests: add `codepageDecode` coverage to `tests/acceptance/src/encoding.mfb`
      (one project — FUNC names are global).
- [ ] Docs: update `encoding`'s `DESC` (`mod.rs`) to name the codepage family;
      update the embedded spec per `.ai/specifications.md`.
- [x] Add `codepageDecode` to `tests/byte-identity/encoding/src/main.mfb`, so the
      `encoding_codegen_cover_rt` `.ncodesum` goldens actually hash the new member
      (Corrections 8), and regenerate them with `scripts/regen-ncodesum.sh`.
      **Landed in Phase 1's commit, not Phase 3's**: a member the cover fixture does
      not call is invisible to the `.ncodesum` sentinel, so leaving it uncovered for
      two phases would have meant two phases of green runs that never hashed the new
      code. `scripts/regen-ncodesum.sh ./target/release/mfb` → `141 golden(s)
      refreshed, 0 missing`; 39 moved, all in encoding-importing packages
      (`byte-identity/{crypto,csv,encoding,json,regex,strings,tls}` and
      `rt-behavior/crypto/crypto-ec-valid`) — the injected source's `ErrorLoc` line
      constants ride into the emitted code, which is the same line-shift mechanism
      as the `.ir` churn. `scripts/artifact-gate.sh ./target/release/mfb all` then
      reported `1330 tests, 1493 build(s), 1832 golden(s) checked, 0 diff(s)`.
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

All three are **RESOLVED**; the evidence is recorded here and in Corrections.

- **RESOLVED — enum variant naming: the underscore form.** `Windows1252`,
  `Iso8859_2`, `Koi8R`, `Ibm866`, `Macintosh`, `MacCyrillic`, `Windows874`, as
  recommended. `_` is a legal identifier-continuation character
  (`is_identifier_continue` at `src/lexer.rs:1309-1311` → `ch.is_ascii_alphanumeric()
  || ch == '_'`), and it is only a line-continuation when it *starts* a token, which
  it never does here. The variant list is data in
  `scripts/gen_codepage_tables.py:CODEPAGES`; order fixes the discriminants and is
  append-only from the moment this lands, pinned by
  `codepage_enum_is_registered_in_generator_order`.
- **RESOLVED — `Codepage` carries `Utf8` but NOT `Utf16Le`/`Utf16Be`.** The plan
  priced all three at "3 `MATCH` arms delegating to the existing members"; that is
  true only of `Utf8`. Measured: `encoding::utf16Decode(value AS List OF Integer) AS
  String` (`./target/release/mfb man encoding utf16Decode`) takes UTF-16 **code
  units**, not bytes, so a `Utf16Le`/`Utf16Be` variant of
  `codepageDecode(cp, List OF Byte)` would need new byte-order pairing logic — new
  behavior, not a delegation, and squarely inside this plan's "no change to
  `utf16*`" non-goal. `Utf8` *is* a clean delegation: `__encoding_utf8Decode(value AS
  List OF Byte) AS String` and `__encoding_utf8Encode(value AS String) AS List OF
  Byte` already exist (`helper_utf8_decode.rs:11`, `helper_utf8_encode.rs:11`), so
  `Utf8` is discriminant 0 and the two UTF-16 variants are left to the follow-up
  label-resolution plan, which is where a byte-order-mark decision belongs anyway.
- **RESOLVED — declare the error: `errors: vec!["ErrInvalidFormat"]`.** Measured, not
  copied. `errors` on a `Body::Mfb` implementation feeds exactly two things: the
  `mfb man` Errors table (`src/cli/man.rs:267,519`) and `declares_error`, whose only
  non-test consumer is a `debug_assert!` on the **native** `raise_error` emission
  path (`src/codegen/error/emission/builder_error_emission.rs:22-32`) that an
  MFB-bodied `FAIL error(...)` never reaches. So declaring it is inert for codegen
  and strictly better documentation. It is also the convention: **25** `Body::mfb`
  members across regex/json/csv/canvas/crypto/audio declare their errors, including
  `json::parse`, `csv::parse` and `regex::match`, which declare this exact
  `"ErrInvalidFormat"`. `encoding`'s 26 empty `errors` lists are the outlier — see
  Corrections for why they are left alone here.

## Corrections

1. **The total mapping count was 3,371; it is 3,342.** The eight per-file
   non-full counts in §Measured populations were all correct — the sum was not.
   19 full tables x 128 = 2,432, plus 121+83+125+92+120+125+118+126 = 910, is
   **3,342**. Command: `python3 scripts/audit_codepage_index.py` →
   `total mappings: 3342`. Nothing downstream was scoped off the wrong number (it is
   a size argument against the multi-byte tables, and 3,342 vs 67,302 is the same
   argument), so no other letter's scope moved. Fixed in §Measured populations,
   §Summary and §3.

2. **The golden blast radius is NOT "encoding + its 3 importers" — it is anything
   that reaches `strings`.** The plan derived the shape check from
   `add_imports`-declared importers (csv, json, regex) and said "a diff in a package
   that does not import `encoding` would be a real signal". Measured, that check
   would have fired on 20 correct diffs. `strings`' scalar seam carries
   `IMPORT encoding` (`src/codegen/builtins/strings/helper_scalar_seam.rs:36`, plus
   `crypto/func_hash.rs:113,127`, `crypto/func_hkdf.rs:55`, `udp/func_receive.rs:44`),
   so every program that reaches `strings` embeds the injected `encoding` source and
   its `.ir` moves. The corrected shape check — **every moved golden's diff must
   mention `codepage`/`Codepage`** — is what §Phase 1 now uses, and it holds for
   64/64 (`python3 /tmp/attribute_mm.py` → `UNATTRIBUTED: 0`).

3. **The three Open Decisions are resolved with measurements, not preference** —
   see §Open Decisions. The consequential one: the plan priced `Utf16Le`/`Utf16Be`
   at "3 `MATCH` arms", but `encoding::utf16Decode` takes `List OF Integer` code
   units rather than `List OF Byte`, so those two would be **new byte-pairing
   behavior**, not a delegation, and they are dropped. `Utf8` is a true delegation
   and is kept, at discriminant 0.

4. **§4.3's body accumulates into a `List OF String` and `strings::join`s once,
   rather than `out = out & ch`.** The plan itself flagged repeated `&` as "a known
   cost shape in this runtime" and made the join form the contingency; it is used
   from the start instead, because the contingency's trigger (a per-character string
   fold) is a certainty at 100 KB, not a risk. The interface is unchanged, which is
   the constraint the plan actually set on this. Throughput is still measured in
   Phase 3.

5. **`helper_codepage_table.rs` is generated and owns the `Codepage` enum as well as
   the `MATCH` arms.** §4.1 described the generator as emitting only the table body,
   with the `RegistryEnum` hand-written in `mod.rs` (§Phase 2). That splits one fact
   — the variant set — across two files that must agree, which is precisely the
   drift class the generator exists to remove. The generated file now emits
   `VARIANTS` (name, index-file label, description) and builds the enum from it, so
   a variant cannot exist without a table nor a table without a variant. Carrying the
   index-file label in `VARIANTS` is also what lets
   `codepage_tables_match_the_vendored_index_files` check the data at test time
   without re-reading the generator.

6. **The generator conforms to the repo's existing generated-artifact gate rather
   than inventing one.** §4.1 said the generator "writes"
   `helper_codepage_table.rs` and that byte-identical regeneration "is itself a
   test". The repo already has that test: `scripts/check-generated.sh` (run in CI,
   `.github/workflows/coverage.yml:47`) re-runs each generator and diffs it against
   the committed artifact, on the contract that a generator writes the artifact to
   **stdout**. `scripts/gen_codepage_tables.py` follows that contract and is
   registered there, so no bespoke idempotence mechanism was added. Script names use
   the repo's `gen_*.py` underscore convention, not the plan's
   `gen-codepage-tables.py`.

7. **Phase 1's rt fixture is the same fixture Phase 3 keeps, not a throwaway.**
   Phase 1's acceptance required the behavioral proof to run "as an rt fixture";
   rather than prove it in a scratch project and build the real fixture later (which
   would have left the criterion unmet at the phase boundary), the fixture was
   created in Phase 1 against the throwaway enum and widened in place in Phase 2.

8. **`tests/byte-identity/encoding/src/main.mfb` did not cover the new member.** The
   `encoding_codegen_cover_rt` fixture exists so that a codegen change to any
   `encoding` builtin flips its `.ncodesum`; a new member is invisible to it until
   it is called. `codepageDecode` (and, in plan-123-B, `codepageEncode`) were added
   to it, so the `.ncodesum` goldens are a real sentinel for them rather than a
   green run that never hashed the code.

## Summary

The engineering risk is the table data, and it is designed out: the tables are
generated from vendored upstream index files and validated differentially against
those same files, so no reviewer has to eyeball 3,342 mappings. The two unproven
premises — that a 128-scalar escaped literal survives the injected-source pipeline,
and that growing `encoding` does not spray golden churn beyond its 3 importers — are
cheap and are tested first, on one throwaway table, before any bulk generation.
Untouched: every existing `encoding` member, `toString`'s UTF-8-only semantics, and
`bug-486`, which is independent and must not be folded in.
