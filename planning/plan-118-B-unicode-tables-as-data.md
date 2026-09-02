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

### Phase 2 — genCat through its own range table

Phases 2 and 3 landed together: after Correction 1 both halves use one
mechanism, one data-object seam, one golden regeneration, and the script half
also forces the `unicode_script_of.mfb` split (Correction 2). Splitting the
commit would have left the tree with two lookup styles for one property pair.

- [x] ~~`runtime_tables.rs`: pack the category index into flags bits 7–11;
      update `encode_le`/decode + the hex-blob generator; add a
      `category()` accessor; extend the pinned-property unit tests.~~ — **moot:
      impossible, measured.** utf8proc's categories disagree with pinned Unicode
      16.0.0 on 4,804 scalars, and 19 of its 8,385 deduplicated property rows are
      shared by scalars whose 16.0.0 categories differ, so no per-row field can
      hold the answers (Corrections 1). Replaced by `src/unicode/range_tables.rs`,
      which reads our own generated runs and touches no existing consumer.
- [x] Emit the 30-entry category string table as data objects; add the native
      lookup emission (mirror `emit_unicode_property_lookup`). —
      `_mfb_unicode_gencat_ranges` (4,099 × `(u32 end, u32 index)`) and
      `_mfb_unicode_gencat_names` (30 × 16-byte `mfb.string.v1` records);
      `CodeBuilder::emit_unicode_range_lookup` is the 12-step bisection.
- [x] Re-point every `__regex_genCat`/`__strings_genCat` call site at the
      native lookup; delete the IF-chain from both packages' injected sources;
      delete `unicode_gencat.mfb` and its generator arm once nothing embeds it.
      — 9 sites; `unicode_gencat.mfb` and `scripts/gen_regex_unicode.py` deleted
      (its `gc()`/`runs()` moved into `gen_unicode_gencat_table.py`).
- [x] Regenerate churned goldens (`sync-goldens.sh`, `regen-ncodesum.sh`);
      prove the `.ncodesum` delta is confined to regex/strings-using fixtures.
- [x] Added: classify the lookups' result as a **rodata String** in
      `value_needs_owning_copy`. Without it `LET cat AS String = regex::genCat(cp)`
      binds the local straight to the rodata pointer and scope-drop `arena_free`s
      a read-only constant — reproduced as SIGBUS (exit 138) on `\d`. Same class
      the `typeName`-fold comment beside it warns about.
- [x] Added: strengthen the two shape assertions this change extends rather than
      re-baselining them — `regex_registered_on_the_clean_room_registry` now
      asserts the public/internal split BY NAME (so publishing `genCat` by
      accident fails), and `unicode_runtime_data_objects_emit_only_referenced_tables`
      adds the four new symbols to its leak check (46 KB a grapheme walk must not
      carry) as well as its count.

Acceptance: full `cargo test --no-fail-fast` green (regex + strings suites in
particular); `scripts/test-accept.sh` full-count green; `-vv` over
`tests/acceptance` shows both genCat rows gone from the size leaderboard and
machine instructions down ≥ 2.0 M; pinned category answers spot-checked against
Python 3.14 `unicodedata` for a sampled scalar set (add a unit test doing this
against the vendored table, not live Python).

MET, and the pinned-answer check is **exhaustive rather than sampled**: while
the IF-chains still existed, `range_tables` was checked against their parsed
arms for all 1,114,112 scalars of both tables and matched everywhere. That form
died with its fixture; what stays is
`binary_search_matches_a_linear_scan_for_every_scalar` (two algorithms, one
dataset, all 1.1 M scalars — the bisection off-by-one class), with
`scripts/check-generated.sh` owning the "are the runs really Unicode 16.0.0"
half by regenerating both artifacts from `unicodedata` / the vendored UCD.
Commit: —

### Phase 3 — scriptOf as a range table

- [x] Emit the script range table + string table as data objects; ~~synthesize
      `runtime.unicode_script_of` via the `RuntimeHelper` mechanism
      (spec + builder arm, mirroring `runtime.mapProbe`)~~ — corrected to an
      `internal_only` registry member with an `abi_inline` body (Corrections 3):
      a `runtime.*` function is only reachable from a native lowering seam, and
      these lookups are called from MFBASIC companion source.
- [x] Re-point `__regex_scriptOf` call sites; delete the IF-chain and
      ~~`unicode_script_of.mfb`~~ — shrink it: the file also holds
      `__regex_scriptCanonName`, which stays (Corrections 2). Renamed to
      `unicode_script_names.mfb`; `gen_regex_scripts.py` emits that now and
      exposes `runs()` for the data generator.
- [x] Regenerate churned goldens as in Phase 2.
- [x] Doc sync: `mfb spec stdlib regex` (pinned-table wording), the generator
      README headers, and `planning/speed.md` §5.3 (append: resolved by this
      letter, with the new numbers).

Acceptance: as Phase 2 plus `#regex_scriptOf` gone; total module instructions
over `tests/acceptance` down ≥ 2.4 M vs the plan-118-A baseline; a regex
`\p{Script}` rt fixture still passes.

MET. `cd tests/acceptance && ../../target/release/mfb test -vv`:

```
                        plan-118-A      this letter
machine instructions    17,079,160      14,523,769    -2,555,391
NIR ops (recursive)         52,548          32,733
largest lower_function  #regex_genCat   __mfb_test_case_266
                          1,057,783           71,647
                        #strings_genCat
                          1,057,783   -> gone
                        #regex_scriptOf
                            440,905   -> gone
```

Tests: 732 pass / 0 fail. `scripts/test-accept.sh`: **1346 test(s) ran**, passed
(`rt-behavior/regex/regex-posix-classes-rt` and `regex-from-string-rt` among
them). `cargo test --no-fail-fast`: 89 suites green + the 2 shape assertions
above, both strengthened and passing. `scripts/artifact-gate.sh all`: 1823
goldens, **0 diffs** after regeneration; `regen-ncodesum.sh` refreshed 132 and
exactly **10** differed — the five per-target sums of `byte-identity/regex` and
`byte-identity/strings`, nothing else. `scripts/check-generated.sh`: all four
artifacts reproduce.
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

  **Measured** (`/tmp/p118bench`, 600 k non-ASCII `strings::isLetter`/`isUpper`/
  `isDigit` queries past the `cp < 128` fast path, so every one reaches the
  table; identical output `hits=200000` from both binaries):

  | | before (main `00dbc5102`) | after |
  |---|---|---|
  | runtime, best of 3 | 2.90 s | **0.04 s** (~73x) |
  | compile `tests/byte-identity/regex`, best of 2 | 6.60 s | **1.75 s** (~3.8x) |

  Both directions of the win, and the runtime one is the asymptotic change: up
  to 4,099 compares per query becomes 12.
- Doc sync: as Phase 3.
- Acceptance: full `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all` with regenerated goldens, both-root fmt,
  `cargo check --all-targets`.

## Open Decisions

- ~~**Where the native lookup lives**~~ — **DECIDED: `abi_inline` on
  `internal_only` registry members**, not a synthesized `runtime.*` function.
  The recommendation was the other way, on the grounds that `abi_inline`
  re-inlines per site. Two facts overrode it:
  1. A `runtime.*` function is reachable only from a native lowering seam
     (`NirValue::RuntimeCall`). These lookups are called from **MFBASIC
     companion source**, which cannot name one. The seam that source CAN name
     is a registry member, and `internal_only: true` keeps it unreachable from
     user code — the pattern `astrings::scalarLen` already establishes.
  2. The re-inlining objection does not bite here: there are **9 call sites in
     total** and the lookup is ~22 instructions, so all the inlining together
     is ~200 instructions against the 2,556,471 removed. `abi_function` would
     have bought ~180 instructions in exchange for the runtime-call catalog,
     the per-target `SUPPORTED_RUNTIME_CALLS` gates and the symbol-routing seam.
- ~~**String returns**~~ — **DECIDED: static**, the rodata pointer, zero alloc.
  The verification the decision asked for found the opposite of what it
  expected: the `register_pending_temp` exemption covers a bare *temp*, but a
  `LET cat AS String = regex::genCat(cp)` binds an **owned slot**, and scope-drop
  `arena_free`d the read-only constant — SIGBUS on `\d`, reproduced before the
  fix. Resolved by classifying these calls in `value_needs_owning_copy`, which
  deep-copies at an owning store (as a string literal already does) and leaves
  bare temps unfreed. 7 of the 9 sites are bare temps and allocate nothing; the
  2 that bind a local allocate exactly what the old IF-chain allocated anyway.

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
   deleting it. It is renamed `unicode_script_names.mfb` to say what it now is.

3. **A `runtime.*` synthesized function cannot serve these lookups.** §3 piece 2
   and the Open Decisions both prescribed one ("`runtime.unicode_script_of`,
   synthesized once per module via the existing `RuntimeHelper` mechanism —
   precedent: `runtime.mapProbe`"). A `runtime.*` function is reachable only
   through `NirValue::RuntimeCall`, which a native lowering seam emits; these
   lookups are called from **MFBASIC companion source**, which has no way to
   name one. The seam that source can name is a registry member, so they are
   `internal_only` members with `abi_inline` bodies — the `astrings::scalarLen`
   pattern. See the Open Decisions for why the per-site inlining objection does
   not bite at 9 call sites.

4. **Returning a rodata pointer needs an ownership classification, and the
   plan's stated reason it was safe was wrong.** The Open Decision said the
   "standalone String temps are never freed" rule already exempts bare Strings.
   It exempts a bare *temp*; a `LET cat AS String = regex::genCat(cp)` binds an
   **owned slot**, whose scope-drop `arena_free`d the read-only constant.
   Reproduced as SIGBUS (exit 138) on `regex::match("5", "\\d")` before the fix.
   Fixed by a `call_returns_rodata_string` arm in `value_needs_owning_copy`,
   which is the same classification a string literal gets: deep-copy at an
   owning store, no free of a bare temp. The `typeName`-fold comment a few lines
   away in `builder_value_semantics.rs` documents this exact crash class.

5. **Phases 2 and 3 landed in one commit.** After Correction 1 the two halves
   share one mechanism, one data-object seam, one golden regeneration and the
   `unicode_script_of.mfb` split; landing them apart would have left the tree
   with two lookup styles for one property pair. Both phases' acceptance
   criteria are recorded and met separately.

## Summary

Risk sits in the packed-property re-encode (shared by every Unicode consumer);
the win is 15 % of all machine instructions, the three §5.3 hotspots, and an
asymptotic runtime improvement, with both mechanisms already proven in-tree.
