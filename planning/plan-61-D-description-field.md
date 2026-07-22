# plan-61-D: The `description` manifest field and MFPC section 18

Last updated: 2026-07-21
Effort: medium (1h–2h)
Depends on: nothing (beyond the plan-61 Prerequisites)
Produces:
- `description` in `project.json`, validated (warning-only in this sub-plan)
- `BinaryReprMetadata.description`
- `SECTION_PACKAGE_META: u16 = 18` — writer, compiler reader, and repository
  reader
- `src/docs/spec/package/15_package-meta-section.md` — the new spec topic

Adds a `description` field to `project.json` and carries it inside the signed
`.mfp` payload as a **new optional MFPC section**. In this sub-plan the field is
**optional with a warning** when absent on a `kind: "package"` project; plan-61-E
migrates the tree and flips it to a hard error.

The single behavioral outcome: `mfb build --sign` on a package with a
`description` produces a `.mfp` whose section 18 carries that string, and a
toolchain built *before* this change still parses that `.mfp` successfully.

References:
- `plan-61-repo-web.md` §2 Verified properties — the section-skip proof this
  design rests on
- `src/docs/spec/package/02_binary-representation.md:49` — sections are a map
  keyed by id
- `src/docs/spec/package/11_doc-section.md` — the precedent: an optional,
  self-contained, non-ABI-affecting section
- `.ai/specifications.md:12-18` — spec sync is part of the Hard Completion Gate

## Prerequisites

See `plan-61-repo-web.md` §Prerequisites. This sub-plan has no additional ones —
it is independent of A, B, and C.

## 1. Goal

- `description` exists in `project.json`, flows into the `.mfp`, and is readable
  by the repository server.
- **Old readers parse new packages.** The container version stays 1.0.

### Non-goals

- **No container version bump.** Both readers hard-check exactly 1.0
  (`repository/src/package.rs:109-113`, `src/manifest/package.rs:143-150`), so a
  bump is itself a break. If this sub-plan finds itself wanting one, the design
  is wrong — stop and record it in `plan-61-repo-web.md` §Corrections.

  > **Two different version numbers — do not "fix" the wrong one.** The outer
  > `.mfp` *container* is **1.0** (the constant above). The inner MFPC payload
  > carries its own `MFPC_MAJOR_VERSION`, which is **2**
  > (`src/binary_repr/mod.rs:52`), hard-rejected on mismatch at
  > `src/binary_repr/reader.rs:314`. Adding section 18 changes **neither**: the
  > section table is the extension point precisely so that it doesn't. An
  > implementer who reads "the version stays 1.0" and then finds a `2` in the
  > payload header has found both correct values, not a bug.
- **No change to the `.mfp` header.** It is a fixed-order positional record with
  no skip mechanism; appending to it breaks both directions.
- **No change to MANIFEST section 1.** `read_manifest`
  (`src/binary_repr/reader.rs:973-1014`) ends with
  `if offset != bytes.len() { return Err("invalid trailing bytes in manifest") }`.
- **No reuse of the DOC section (17).** Different author, different source of
  truth — see `plan-61-repo-web.md` §3 rejected alternatives.
- **No `license`, `keywords`, or `readme`.** Section 18 is designed to accept them
  later; adding them is not this plan's scope.
- **No hard error for a missing `description` yet.** That is plan-61-E, after the
  migration.
- No change to `abiHash` — verified impossible, since the ABI serializer reads no
  manifest fields.

## 2. Why a new section, not the header or MANIFEST

This was the plan's one genuinely uncertain premise and it was resolved by
reading both parsers before writing. The evidence is recorded in
`plan-61-repo-web.md` §2 Verified properties; the short form:

- The section table is walked at `src/binary_repr/reader.rs:320-352` and
  `repository/src/abi.rs:168-195`. In both, the id is used **only** as a map key.
  There is no match on id, no known-set membership test, no unknown-section error.
- Optional sections are handled by absence-from-map — `reader.rs:407-410` for
  DOC.
- The intent is documented: `src/binary_repr/mod.rs:40-43` says of DOC, "a
  consumer that does not understand it skips it entirely".
- The only two ways a new section breaks an old reader are shipping it **twice**
  (`duplicate MFPC section id <n>` is enforced) or declaring `offset + length`
  past the payload end. A well-formed producer hits neither.

Therefore section 18 is forward-compatible and the header/MANIFEST routes are
not. This is not a preference; it is the difference between a flag-day rebuild of
every package and a no-op for existing artifacts.

**One honest caveat to record in the spec:** the format has no "critical section"
marker. An old reader accepts a new package and silently ignores section 18. That
is exactly what we want for `description` — a missing description is cosmetic —
but it means section 18 must never carry semantically load-bearing data. Say so
in the new spec topic so a future author does not put something security-relevant
there.

## 3. Section 18 layout

Named `PACKAGE_META` rather than `DESCRIPTION`, so `license`/`keywords` can join
it later without another section. Self-contained and length-prefixed like DOC
(`src/docs/spec/package/11_doc-section.md:14-20`) — it does **not** intern into
the string pool, so it can be parsed without section 2.

```
u32          fieldCount
  per field:
    u16      fieldId        (1 = description; 2..=  reserved)
    u32      byteLength
    u8[]     utf8 value
```

A `fieldCount`/`fieldId` design rather than a positional record, so that a later
field is additive *within* the section too, and an unknown `fieldId` is skipped
by the same logic that makes the section itself skippable. Readers must skip
unknown `fieldId`s rather than erroring.

Caps, mirroring the existing header string limits (`author` 512, `url` 2048 at
`repository/src/package.rs:118-129`): `description` is capped at **4096 bytes**,
validated at manifest-parse time with a clear diagnostic and re-validated at
section-read time. Pick the reader's error message to match the existing
`.mfp <field> exceeds the N byte limit` idiom.

Section 18 participates in the payload hash and therefore the signature — like
every section — so a description cannot be altered without invalidating the
package. That is the point: `plan-61-repo-web.md` §3 rejects taking description
from the unsigned publish request for exactly this reason.

## 4. Manifest validation

Mirror the existing idiom. `validate_project_manifest` (`src/manifest/mod.rs`)
already calls `validate_optional_string(manifest, project_path, &contents,
"author")` at `:166` and the same for `url`, `entry`, `icon`. `validate_kind`
(`:385-410`) is the model for a required-field diagnostic.

In this sub-plan `description` is validated as an optional string, plus a
**warning** when it is absent and `kind == "package"`. Warning, not error, so the
81 existing package manifests keep building — **plan-61-F** migrates them and
flips it (F Phase 2 then F Phase 4; plan-61-E only surfaces the value).

`kind: "executable"` neither requires nor rejects `description`. Executables are
never published, so the field is inert there, but forbidding it would make a
`kind` flip needlessly lossy.

### Diagnostic codes — RESOLVED

An earlier draft recorded this as UNVERIFIED because
`grep -oE 'PROJECT_JSON_[A-Z_]+' src/docs/spec/diagnostics/02_error-codes.md`
returned empty, and inferred the codes must be spelled some other way. **The
grep was against the wrong file.** There are two independent registries:

| Registry | File | Governs | Consumed by |
|---|---|---|---|
| Runtime `errorCode::` | `src/docs/spec/diagnostics/02_error-codes.md` | `Error.code` integers visible to MFBASIC programs | `build.rs` generates constants from it |
| **Compiler rules** | **`src/docs/spec/diagnostics/01_rule-codes.md`** | **`PROJECT_JSON_*` and every other diagnostic** | **`src/rules/table.rs`** |

`02_error-codes.md` says so itself, near its head: the compiler-facing rule set
"is a separate registry and is not surfaced here." The `.ai/specifications.md:45-50`
build-input rule is quoted accurately but ends "Edit that table for any **runtime**
error-code change" — it does not govern this work, and neither `build.rs` nor
`table_matches_registry` is involved.

The scheme is therefore **confirmed, not unconfirmed**:

- `PROJECT_JSON_*` lives at `src/docs/spec/diagnostics/01_rule-codes.md:260-274`,
  numbered `2-200-NNNN`, spelled exactly as `validate_kind` uses them.
- `2-200-0001` … `2-200-0015` are allocated (`0011` is `PROJECT_ENTRY_INVALID`,
  not a gap). The high block `2-200-0100`/`0101` is build orchestration.
- **Next free: `2-200-0016`.**

Allocating a code is **two edits, and the pair is enforced**: add a `Rule { code,
name, severity, message }` entry to `src/rules/table.rs` (the `PROJECT_JSON_*`
block ends at `:1122`) *and* a row to `01_rule-codes.md`. The drift guard is
`every_rule_is_documented_in_the_spec` (`src/rules/mod.rs:231-249`), which fails
if a rule exists in `table.rs` with no matching code and name in the spec. Doing
only one of the two edits is a red test, not a silent divergence.

> **Also update the prose above the table.** `01_rule-codes.md:248-255` narrates
> the block as `0001`-`0013` and names "exactly six `warn` rules". Both are
> already stale before this plan touches anything — the table runs to `0015`, and
> there are **eight** `warn` rules (the prose omits `2-203-0115
> NATIVE_LIBRARY_TARGET_UNCOVERED` and `2-203-0117 NATIVE_LIBRARY_UNUSED`). The
> drift test only checks code and name presence, so it does not catch prose
> counts. D adds a ninth `warn` rule and must leave that sentence correct.

## Phases

> Tick `- [x]` in the same commit as the work. **An unticked box means NOT DONE.**

### Phase 1 — Manifest field and diagnostics

- [x] Allocate the new `warn` rule as `2-200-0016` per §4 — re-run
      `grep -n '2-200-00' src/docs/spec/diagnostics/01_rule-codes.md` first to
      confirm `0016` is still free, since other plans also allocate here. Add the
      `Rule {}` entry to `src/rules/table.rs` **and** the table row to
      `01_rule-codes.md` in the same commit; `every_rule_is_documented_in_the_spec`
      (`src/rules/mod.rs:231-249`) fails on either alone.
- [x] Update the `01_rule-codes.md:248-255` prose: the block range (`0001`-`0016`)
      and the `warn` count, which is already stale at "six" against eight rules
      today and becomes nine here. See the §4 note.
- [x] Add `description` to `validate_project_manifest` (`src/manifest/mod.rs`)
      via `validate_optional_string`, alongside the `author` call at `:166`.
- [x] Add the 4096-byte cap with its own diagnostic.
- [x] Add the missing-description **warning** for `kind: "package"`.
- [x] Add `description` to `package_metadata` (`src/manifest/package.rs:420-455`)
      and to `BinaryReprMetadata` (`src/binary_repr/mod.rs:137-151`).
- [x] Tests: `src/manifest/mod.rs` inline tests — description present/absent/
      wrong-type/over-cap; warning fires for `kind: "package"` without it;
      warning does **not** fire for `kind: "executable"`.

Acceptance: **MET** — full `cargo test --bin mfb` → 3183 passed, 0 failed,
including `every_rule_is_documented_in_the_spec` (the drift guard that fails if
`table.rs` and the spec disagree).
`description_is_optional_capped_and_only_warned_about` covers present,
absent-on-executable (silent), absent-on-package (**Ok**, so the warning cannot
fail a build), wrong type (error), one byte over the cap (error), and exactly at
the cap (accepted — the boundary itself, not a value comfortably past it).
`2-200-0016` was re-confirmed free before allocation.
Commit: —

### Phase 2 — Write and read section 18

- [x] Add `SECTION_PACKAGE_META: u16 = 18` to `src/binary_repr/mod.rs` (highest
      in use today is 17, `SECTION_DOC_TABLE` — re-confirm with
      `grep -nE 'SECTION_[A-Z_]+: u16 = [0-9]+' src/binary_repr/mod.rs` before
      claiming 18 is free).
- [x] Emit the section in the writer (`src/binary_repr/writer.rs` / the section
      assembly in `src/binary_repr/sections.rs`), per the §3 layout. **Omit the
      section entirely when there is no description** — do not emit an empty
      section, so packages without one are byte-identical to today.
- [x] Read it in the compiler reader (`src/binary_repr/reader.rs`), following the
      DOC absence idiom at `:407-410`: `match sections.get(&SECTION_PACKAGE_META)`
      → `None` yields a default.
- [x] Read it in the repository reader (`repository/src/abi.rs`), using the
      existing `read_section_table` (`:168-195`).
- [x] Skip unknown `fieldId`s rather than erroring, per §3.
- [x] Tests: round-trip a description through write→read; a package with no
      description emits no section 18 and reads back as `None`; a section with an
      unknown `fieldId` is skipped without error; an over-cap length in the
      section is rejected at read time.
- [x] Tests — **the forward-compatibility regression test**, which is the whole
      premise: construct a payload containing a section with an id no reader
      knows (e.g. 99) and assert `read_binary_repr_package` and the repository's
      `read_section_table` both parse it successfully. This guards the property
      that a *future* section will not break *this* reader.

Acceptance: **MET.**
`a_description_round_trips_through_section_eighteen` encodes a real project
twice — with and without a description — and asserts the value survives the
round trip *and* that the description-free payload is **strictly shorter**,
which is the observable proof no empty section was emitted.
`a_section_with_an_unknown_id_is_parsed_and_ignored` is the
forward-compatibility test: it decomposes a real payload, appends sections with
ids `99` and `4242`, re-encodes, and asserts `read_binary_repr_package` parses
it **and** that every field the reader does understand is byte-for-byte what the
baseline produced. `reads_the_description_from_section_eighteen` covers the
repository reader (present / absent / non-container / unknown fieldId /
over-cap). `an_unknown_package_meta_field_id_is_skipped_not_rejected` places
unknown ids on **both sides** of the known one, so a reader that bailed on the
first unknown field would fail it.

**Golden churn: zero, as the plan required — and measured, not assumed.**
`artifact-gate.sh` reports 18 diffs, but a stash-and-rerun on a clean tree
reports the *same 18*, and `diff` of the two sorted DIFF lists is empty. All 18
are pre-existing `codegen-cover` `.ncode` diffs unrelated to this work. Nothing
was re-baselined.
Commit: —

### Phase 3 — Spec sync

Part of the Hard Completion Gate, not cleanup (`.ai/specifications.md:12-18`).

- [x] Add `description` to the schema table in
      `src/docs/spec/tooling/01_project-manifest.md` (table header at `:27`), with type,
      required-ness (`no` for now — plan-61-E changes this row), meaning, and the
      4096-byte cap.
- [x] Add section 18 to the section table in
      `src/docs/spec/package/02_binary-representation.md:53-70`.
- [x] Write a new topic `src/docs/spec/package/15_package-meta-section.md`
      (next free `NN`) describing the §3 layout, following the DOC topic
      (`11_doc-section.md`) as the model. **Include the §2 caveat**: the format
      has no critical-section marker, so section 18 must never carry
      security-relevant data.
- [x] Add invisible `[[src/file.rs:Symbol]]` provenance citations and confirm
      each with grep before citing (`.ai/specifications.md:28-53`).
- [x] Verify: `cargo build` (regenerates the embedded table; `touch build.rs` if
      a brand-new file is not picked up), `cargo test --bin mfb spec`, then
      `mfb spec package --all` renders with no leaked `[[` markers.

Acceptance: **MET** — `cargo test --bin mfb spec` → 48 passed, 0 failed,
including `spec_links_resolve` and `spec_citations_resolve` (all four new
`[[…]]` citations were grepped against real symbols before being written).
`mfb spec package --all` renders with **0** leaked `[[` markers, lists
`package-meta-section` in the topic index, and includes the new topic's body.
Commit: —

## Validation Plan

- Tests: inline tests in `src/manifest/mod.rs`, `src/binary_repr/reader.rs`,
  `repository/src/abi.rs`. Negative cases: over-cap, wrong type, unknown
  `fieldId`, unknown section id.
- Coverage check: `sh scripts/coverage.sh && sh scripts/coverage-check.sh`.
- Runtime proof: build and sign `bindings/sqlite3` with a description added,
  then confirm the description survives a build→read round trip. Then check out
  a **pre-change** `mfb` binary and confirm it still parses the new `.mfp`
  without error — this is the forward-compatibility claim, and it deserves a real
  binary, not only a unit test.
- Doc sync: `src/docs/spec/tooling/01_project-manifest.md`,
  `src/docs/spec/package/02_binary-representation.md`, and the new topic.
- Acceptance: `scripts/artifact-gate.sh target/debug/mfb` (fast codegen gate) and
  `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

> **Golden churn note.** Because Phase 2 omits section 18 when there is no
> description, and no fixture has one yet, this sub-plan should produce **zero**
> golden diffs. If `artifact-gate.sh` reports diffs here, something emitted an
> empty section — investigate, do not re-baseline. All golden churn is
> deliberately concentrated in plan-61-E.

## Open Decisions

- **Section name: `PACKAGE_META` vs `DESCRIPTION`** — *recommended:* `PACKAGE_META`,
  so `license`/`keywords` join it later without consuming another section id.
- **4096-byte cap** — *recommended:* 4096, twice the `url` cap of 2048 and
  clearly enough for a one-paragraph summary. If a longer form is ever wanted,
  that is what the DOC section already is.

## Corrections

- **The runtime proof used a purpose-built fixture, not `bindings/sqlite3`.**
  §Validation Plan says to add a description to `bindings/sqlite3` and rebuild
  it. That package is **modified in the working tree by another agent** (as is
  `bindings/libsnd`), and editing a fixture mid-edit by someone else is how work
  gets stashed away. A throwaway package at `/tmp` gave the identical evidence
  without touching a shared file.
- **The forward-compatibility proof got a genuinely old binary, as asked.**
  `HEAD` was D Phase 1 — the manifest field, but *not* section 18 — so stashing
  Phase 2 and building produced a real reader that has never heard of section 18.
  It read a `.mfp` containing one with `mfb pkg info` (exit 0, all known metadata
  intact) and `mfb pkg validate` (`payload hash: OK`, `result: valid`). The
  section was silently ignored, which is exactly the claim.
- **`artifact-gate.sh` must be run with `bash`, not `sh`.** It uses process
  substitution (`done < <(find …)`), so `sh scripts/artifact-gate.sh` dies with
  a syntax error at line 103 — and, because the failure is inside the script,
  the wrapper still exits 0. A gate that reports success while never having run
  is worth recording; the golden-churn check was re-run under `bash`.
- **§4's warning about stale prose was itself stale.** It says
  `01_rule-codes.md` narrates the block as `0001`-`0013` with "exactly six
  `warn` rules", and instructs D to leave the sentence correct. As found, the
  prose already read `0001`-`0015` and "exactly eight `warn` rules", listing all
  eight including `2-203-0115` and `2-203-0117` — someone corrected it between
  the plan being written and D running. D updated it to `0001`-`0016` and
  "nine", which is the same obligation, just from a different starting point.
  Recorded because the plan predicted a defect that no longer existed, and
  "fixing" it from the stated baseline would have *introduced* one.
- **`Severity::Warn`, not `Severity::Warning`.** §4 gives the `Rule {}` shape as
  `{ code, name, severity, message }` without naming the variant; the enum in
  `src/rules/table.rs` spells it `Warn`.
- **The cap needed a named constant, not a literal.** `MAX_DESCRIPTION_BYTES`
  lives in `src/manifest/mod.rs` because §3 requires the same limit to be
  enforced twice — at manifest-parse time *and* at section-read time — and two
  hand-written `4096`s are two things that can drift apart.
