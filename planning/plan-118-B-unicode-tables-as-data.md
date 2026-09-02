# plan-118-B: Unicode category/script tables as native data, not code

Last updated: 2026-09-01
Effort: large (3h–1d)
Depends on: plan-118-A (its `-vv` instrumentation is this letter's acceptance instrument)

Replace the three generated Unicode IF-chain functions — `#regex_genCat`,
`#strings_genCat` (a byte-equal twin of the same generated source), and
`#regex_scriptOf` — with pinned native data tables plus a small emitted lookup.
They are **2,556,471 machine instructions, 15.0 % of the whole module**
(`SPIKEFN` census, plan-118-A §2), the three slowest `lower_function` rows
(`planning/speed.md` §5.3: 3.44 s of 19.0 s release lowering), and — because
each is a linear chain of up to ~4,100 compares — an O(ranges) *runtime* cost
per queried scalar that a table lookup makes O(log n) or O(1).

References:

- `planning/speed.md` §5.3 and recommendation 7.
- `scripts/gen_regex_unicode.py`, `scripts/gen_regex_scripts.py` — the
  generators; their headers pin Unicode 16.0.0 / Python 3.14 and are enforced
  by `scripts/check-generated.sh` (CI job).
- `src/codegen/string/unicode/unicode_gencat.mfb` (4,109 lines),
  `unicode_script_of.mfb` (1,891 lines) — the generated IF-chains.
- `src/codegen/builtins/regex/mod.rs:912-916` (regex embeds both);
  `src/codegen/builtins/strings/helper_scalar_seam.rs:25` (strings embeds the
  gencat table again, renamed `__strings_genCat` — the twin).
- Precedent for native Unicode tables read at runtime:
  `src/unicode/runtime_tables.rs` (`PackedProperty`, the two-stage trie,
  `stage1_hex`/`stage2_hex`/`properties_hex` emitted as data objects) and
  `builder.emit_unicode_property_lookup` (used by
  `src/codegen/builtins/strings/func_display_width.rs:110-190` at runtime).
- `mfb spec stdlib regex` / `mfb spec stdlib strings` — spec pages naming the
  pinned-table behavior (`.ai/specifications.md` sync duty).

## Prerequisites

plan-118-A's family gate (see that doc) plus:

| Must be true | Command | Status |
|---|---|---|
| plan-118-A landed | `-vv` prints the "costliest expansion" tally | MET — `f86af39a7`; verified `grep -c "costliest expansion" /tmp/p118_vv_phase2.log → 1` |
| The gencat generator's stated reason for code-not-data is understood | header of `scripts/gen_regex_unicode.py` (read: "MFBASIC list reads copy the whole list, and the native backends cannot hold a large constant array cheaply") | MET — see §2: that reasoning predates the native data-object trie and does not apply to a rodata table + native lookup |

## 1. Goal

- `#regex_genCat`, `#strings_genCat`, `#regex_scriptOf` no longer exist as
  lowered MFBASIC functions; category/script queries go through a native
  rodata table lookup. The module's machine-instruction counter over
  `tests/acceptance` drops by ≥ 2.4 M (from 17.08 M), and the §5.3
  leaderboard's top three rows disappear.

### Non-goals (explicit constraints)

- **No behavioral change to any regex or strings API**: same categories, same
  script names, same `\d`/`\w`/`\s`/`\b`/`\p{gc}`/`\p{Script}` semantics, same
  pinned Unicode 16.0.0 answers for every scalar 0..=0x10FFFF.
- The Unicode version pinning workflow stays: generators still run under
  Python 3.14, `scripts/check-generated.sh` still proves reproducibility.
- No new heap allocation on the query path (the IF-chain allocated its 2-char
  `String` return; the replacement must return interned/static strings the
  same way the callers already consume them — see Open Decisions).

## 2. Current State

- The generator emits "one flat IF-chain function" *by design*, its header
  arguing an MFBASIC list table would be copied per read and "the native
  backends cannot hold a large constant array cheaply". Both claims are about
  **MFBASIC-level** tables. The compiler has since grown native data-object
  tables read in place at runtime: `runtime_tables.rs` embeds the utf8proc
  two-stage trie as hex blobs emitted as data objects, and
  `emit_unicode_property_lookup` indexes them inline (displayWidth, NFC,
  graphemes all use this at runtime). The stated obstacle no longer exists.
- `PackedProperty` (`src/unicode/runtime_tables.rs:25`) does **not** currently
  carry the general category: fields are combining_class, comb_index/length,
  flags (bits 0–6 used), boundclass, indic_conjunct_break. Category needs 5
  bits (30 two-letter categories); flags bits 7–15 are free (read the
  constants at `runtime_tables.rs:42-51`).
- Callers of `__regex_genCat` / `__strings_genCat` / `__regex_scriptOf` are
  MFBASIC helper sources (regex property tests, shorthand classes, strings
  classification predicates): census them with
  `grep -rn "genCat\|scriptOf" src/codegen/builtins/regex/*.rs src/codegen/builtins/strings/*.rs src/codegen/string/unicode/*.mfb` — UNMEASURED precisely
  at plan time; Phase 1 task.

### Measured populations

| What | Count | Command |
|---|---|---|
| `#regex_genCat` instructions | 1,057,783 | spike `SPIKEFN` line (plan-118-A §2) |
| `#strings_genCat` instructions | 1,057,783 (equal ⇒ twin) | ditto |
| `#regex_scriptOf` instructions | 440,905 | ditto |
| gencat source lines | 4,109 | `wc -l src/codegen/string/unicode/unicode_gencat.mfb` |
| scriptOf source lines | 1,891 | `wc -l src/codegen/string/unicode/unicode_script_of.mfb` |
| Free flag bits in PackedProperty | 9 (bits 7–15) | read `runtime_tables.rs:42-51` |
| MFBASIC call sites of the three helpers | **11** (5 regex, 6 strings) | census below |
| utf8proc categories disagreeing with pinned 16.0.0 | **4,804 scalars** | `python3 /tmp/p118_catcheck.py` (§3 correction) |
| utf8proc property rows carrying >1 pinned-16.0.0 category | **19** of 8,385 | `python3 /tmp/p118_rowcheck.py` |
| `__regex_scriptCanonName` (rides in `unicode_script_of.mfb`) | 171 arms, lines 1719–1891 | `grep -n '^FUNC \|^END FUNC' unicode_script_of.mfb` |

Census of the three helpers' call sites
(`grep -rn "genCat\|scriptOf" src/codegen/builtins/regex/*.rs src/codegen/builtins/strings/*.rs`):

| Caller | File | Uses |
|---|---|---|
| `__regex_isWord` | `regex/helper_is_word.rs:12` | `__regex_genCat(cp)` |
| `__regex_propTest` | `regex/helper_prop_test.rs:13` | `__regex_genCat(cp)` |
| `__regex_shorthandMatch` | `regex/helper_shorthand_match.rs:13` | `__regex_genCat(cp)` |
| `__regex_scriptTest` | `regex/helper_script_test.rs:14` | `__regex_scriptOf(cp) = name` |
| `__strings_isLetter` | `strings/helper_scalar_seam.rs:85` | `__strings_genCat(cp)` |
| `__strings_isDigit` | `strings/helper_scalar_seam.rs:94` | `= "Nd"` |
| `__strings_isWhitespace` | `strings/helper_scalar_seam.rs:102` | `__strings_genCat(cp)` |
| `__strings_isUpper` | `strings/helper_scalar_seam.rs:120` | `= "Lu"` |
| `__strings_isLower` | `strings/helper_scalar_seam.rs:128` | `= "Ll"` |

Every call site consumes the returned `String` immediately (a comparison or a
5-way `OR` of comparisons) and none stores or frees it — which is what makes the
Open Decision's "return the interned static string" safe. All five `strings`
predicates already short-circuit `cp < 128` before reaching the table.

### Verified properties

- The twins really are the same generated source included twice — read
  `regex/mod.rs:912` and `strings/helper_scalar_seam.rs:25` (both
  `include_str!` the same `unicode_gencat.mfb`; strings renames the FUNC).
  Byte-equal instruction counts corroborate.

  **Verified mechanically (Phase 1, 2026-09-01).** A scratch project importing
  both packages, built with `mfb build --ncode`, yields `#regex_genCat` and
  `#strings_genCat` at 1,057,783 instructions each. Normalising only the
  package-prefixed symbol (`_mfb_ifn_{regex,strings}_5FgenCat`), the local label
  names, and the interned `_mfb_str_N` constant names, the two instruction
  streams are **identical JSON**, and their opcode histograms match exactly
  (`/tmp/p118_twin3.py`). They are one generated table compiled twice.

## 3. Design Overview

**CORRECTED 2026-09-01 — piece 1 was impossible as written; see Corrections 1.**
Both halves now use the same mechanism, which is strictly simpler and touches no
existing Unicode consumer.

Two independent pieces, one shape:

1. **Category via its own range table.** Emit `unicode_gencat_ranges.txt`
   (phase 1) as a sorted rodata table of `(run-end codepoint, category index)`
   records plus a fixed-stride table of the 30 category names as
   `mfb.string.v1` records, and a native binary search over it. The pinned
   Unicode 16.0.0 answers are carried by our own generated data, so they cannot
   drift with a utf8proc bump.
2. **Script via its own range table.** Identical shape over
   `unicode_script_ranges.txt`: `(run-end codepoint, script index)` plus a
   fixed-stride name table (171 names, longest 22 bytes).

Both lookups live as `internal_only: true` registry members with
`Body::abi_function` bodies, so each is emitted ONCE per module as a runtime
symbol and every call site is a `bl` — the "one copy" property the plan's Open
Decision asks for, using the seam `astrings::scalarLen`
(`builtins/astrings/func_scalar_len.rs`) already establishes for a native
primitive that only toolchain-provided source may call. `regex` and `strings`
have no `RuntimeHelper` family of their own, so `abi_function_family` routes
them to the shared `Abi` family (exactly as `crypto` does).

**Correctness risk** is now confined to the two new lookups: no existing
Unicode consumer (displayWidth, NFC, graphemes, case mapping) is touched at
all, because the utf8proc `PackedProperty` record is not re-encoded. The
remaining risk is the binary search itself — an off-by-one at a run boundary —
which the pinned-vector tests in the Validation Plan target directly.
**Design uncertainty** is low: range table + rodata name table + `abi_function`
member all have in-tree precedents.

This letter's outputs change codegen massively, so **byte-identity is NOT the
gate**; the gates are behavioral (regex/strings suites, rt fixtures,
acceptance) plus the quantified instruction-count drop. `.ncode`/`.ncodesum`
goldens for regex/strings-touching fixtures are EXPECTED to diff — regenerate
via `scripts/sync-goldens.sh` / `regen-ncodesum.sh` and prove the delta is
confined to the expected functions.

Rejected: an MFBASIC `List` table (the generator header's original objection —
list reads copy); keeping both twins with one shared symbol only (saves 6 %
but leaves the linear-scan runtime and the 1.06 M instructions of the
survivor); **packing the category into utf8proc's `PackedProperty`** (the
plan's own original design — measured impossible, Corrections 1).

## Phases

### Phase 1 — census + twin verification (no behavior change)

- [x] Census the three helpers' call sites (command in §2) and record them here.
- [x] Verify the twin claim mechanically: dump both functions' instruction
      streams from one acceptance `--ncode`-style build and diff (names aside).
- [x] Extend the generators to ALSO emit the category index / script range
      table data (new output files under `src/codegen/string/unicode/`),
      keeping the current `.mfb` outputs untouched. Wire
      `scripts/check-generated.sh` to cover the new artifacts.
      — `scripts/gen_unicode_gencat_table.py` → `unicode_gencat_ranges.txt`
      (4,099 runs, 30 categories), `scripts/gen_unicode_script_table.py` →
      `unicode_script_ranges.txt` (1,708 runs, 171 scripts). Each imports the
      `.mfb` generator's new `runs()` rather than recomputing, so the code and
      data forms cannot disagree about a scalar; both `.mfb` outputs stay
      byte-identical (`cmp` clean).
- [x] Added: fix the two generators' stale artifact paths
      (`src/codegen/unicode/` → `src/codegen/string/unicode/`, wrong since the
      `src/codegen` tier relocation) in their docstrings and `scripts/README.md`,
      and give `gen_regex_scripts.py` a README entry it never had.

Acceptance: census table filled in; generated data artifacts reproduce under
`scripts/check-generated.sh`; `cargo test --no-fail-fast` green; artifact-gate
0 diffs (nothing consumes the new data yet).

MET: `./scripts/check-generated.sh` → exit 0, all five artifacts `ok:`
(173 vector bodies, both `.mfb`s, both new `.txt`s).
`scripts/artifact-gate.sh all`: 1325 tests, 1823 goldens, **0 diffs** — no Rust
changed in this phase (`git diff --stat HEAD -- '*.rs'` empty), so nothing
consumes the new data yet.
Commit: e8bd233c3

### Phase 2 — genCat through the trie

- [ ] `runtime_tables.rs`: pack the category index into flags bits 7–11;
      update `encode_le`/decode + the hex-blob generator; add a
      `category()` accessor; extend the pinned-property unit tests.
- [ ] Emit the 30-entry category string table as data objects; add the native
      lookup emission (mirror `emit_unicode_property_lookup`).
- [ ] Re-point every `__regex_genCat`/`__strings_genCat` call site at the
      native lookup; delete the IF-chain from both packages' injected sources;
      delete `unicode_gencat.mfb` and its generator arm once nothing embeds it.
- [ ] Regenerate churned goldens (`sync-goldens.sh`, `regen-ncodesum.sh`);
      prove the `.ncodesum` delta is confined to regex/strings-using fixtures.

Acceptance: full `cargo test --no-fail-fast` green (regex + strings suites in
particular); `scripts/test-accept.sh` full-count green; `-vv` over
`tests/acceptance` shows both genCat rows gone from the size leaderboard and
machine instructions down ≥ 2.0 M; pinned category answers spot-checked against
Python 3.14 `unicodedata` for a sampled scalar set (add a unit test doing this
against the vendored table, not live Python).
Commit: —

### Phase 3 — scriptOf as a range table

- [ ] Emit the script range table + string table as data objects; synthesize
      `runtime.unicode_script_of` via the `RuntimeHelper` mechanism
      (spec + builder arm, mirroring `runtime.mapProbe`).
- [ ] Re-point `__regex_scriptOf` call sites; delete the IF-chain and
      `unicode_script_of.mfb`.
- [ ] Regenerate churned goldens as in Phase 2.
- [ ] Doc sync: `mfb spec stdlib regex` (pinned-table wording), the generator
      README headers, and `planning/speed.md` §5.3 (append: resolved by this
      letter, with the new numbers).

Acceptance: as Phase 2 plus `#regex_scriptOf` gone; total module instructions
over `tests/acceptance` down ≥ 2.4 M vs the plan-118-A baseline; a regex
`\p{Script}` rt fixture still passes.
Commit: —

## Validation Plan

- Tests: pinned-vector unit tests for category/script lookups over boundary
  scalars (0, 0x1F, surrogates, 0x10FFFF, range edges); existing regex/strings
  suites; the property-table round-trip tests in `runtime_tables.rs`.
- Coverage check: the acceptance corpus exercises `\p{gc}`/`\p{Script}`
  (verify with `grep -rn 'p{' tests/acceptance/src/regex.mfb` before trusting
  green).
- Runtime proof: a regex benchmark from `benchmark/` (or a small script doing
  10^6 `genCat` queries) — expected FASTER (linear chain → table lookup);
  record before/after.
- Doc sync: as Phase 3.
- Acceptance: full `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all` with regenerated goldens, both-root fmt,
  `cargo check --all-targets`.

## Open Decisions

- **Where the native lookup lives** — an `abi_inline` registry body on the
  regex/strings members vs a synthesized `runtime.*` function each call site
  `bl`s to. Recommended: `runtime.*` function (one copy; call sites shrink to
  a call), consistent with this family's direction; `abi_inline` re-inlines
  per site, which is the disease this family treats.
- **String returns** — return the interned static 2-char string (rodata
  pointer, zero alloc) vs allocate per call as today. Recommended: static
  (callers compare/consume immediately; verify no caller frees it — the
  "standalone String temps are never freed" rule at
  `builder_values.rs:register_pending_temp` already exempts bare Strings).

## Corrections

1. **§3 piece 1 — "category via the existing trie" — is impossible, not merely
   risky.** The plan proposed packing a 5-bit general-category index into
   `PackedProperty`'s free `flags` bits 7–11 and reading it through
   `emit_unicode_property_lookup`. Two measurements kill it:

   * **The two tables disagree on 4,804 scalars.** The vendored utf8proc
     2.11.3 carries a NEWER UCD than the pinned Unicode 16.0.0 the `.mfb`
     tables are generated from: 4,803 scalars are `Cn` (unassigned) in 16.0.0
     and assigned in utf8proc (4,584 of them `Lo`), plus U+0295, `Ll` in 16.0.0
     and `Lo` in utf8proc. Routing `genCat` through the trie would change 4,804
     answers — a behavior change this letter's non-goals forbid outright.
     Measured by decoding utf8proc's two-stage trie in Python and comparing
     every scalar 0..0x10FFFF against `unicode_gencat.mfb`'s parsed runs
     (`/tmp/p118_catcheck.py`).
   * **The trie cannot even represent the pinned answers.** The property
     records are deduplicated on utf8proc's OWN field set, so a row is shared
     by scalars that agree there. **19 of the 8,385 rows are shared by scalars
     whose Unicode 16.0.0 categories differ** — e.g. row 349 is reached by
     scalars that are `Cn`, `Ll` *and* `Lo` in 16.0.0. No per-row field can
     hold three values (`/tmp/p118_rowcheck.py`).

   This is a design correction, not a falsified premise: the letter's goal —
   delete the three IF-chain functions, −2.4 M instructions — is untouched, and
   the mechanism piece 2 already prescribed for scripts (a dedicated sorted
   range table + native binary search) covers categories too. It is also
   strictly better: it re-encodes nothing, so displayWidth / NFC / graphemes /
   case mapping are not in the blast radius at all, and the pinned answers stay
   pinned to *our* generated data instead of following a utf8proc bump.

2. **`unicode_script_of.mfb` cannot be deleted outright (phase 3).** It holds
   two functions, not one: `__regex_scriptOf` (lines 7–1717, the 1,708-arm
   IF-chain this letter replaces) and `__regex_scriptCanonName` (lines
   1719–1891, 171 arms mapping a lowercased script name to its canonical
   spelling). The second is small, is not a per-scalar lookup, and has no
   reason to move. Phase 3 shrinks the file to that function rather than
   deleting it.

## Summary

Risk sits in the packed-property re-encode (shared by every Unicode consumer);
the win is 15 % of all machine instructions, the three §5.3 hotspots, and an
asymptotic runtime improvement, with both mechanisms already proven in-tree.
