# plan-61-D: The `description` manifest field and MFPC section 18

Last updated: 2026-07-21
Effort: medium (1h–2h)
Depends on: nothing (beyond the plan-61 Prerequisites)
Produces:
- `description` in `project.json`, validated (warning-only in this sub-plan)
- `BinaryReprMetadata.description`
- `SECTION_PACKAGE_META: u16 = 18` — writer, compiler reader, and repository
  reader
- `src/docs/spec/package/NN_package-meta-section.md` — the new spec topic

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
- The intent is documented: `src/binary_repr/mod.rs:35-41` says of DOC, "a
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
81 existing package manifests keep building — plan-61-E migrates them and flips
it.

`kind: "executable"` neither requires nor rejects `description`. Executables are
never published, so the field is inert there, but forbidding it would make a
`kind` flip needlessly lossy.

### Diagnostic codes — UNVERIFIED, resolve in Phase 1

`grep -oE 'PROJECT_JSON_[A-Z_]+' src/docs/spec/diagnostics/02_error-codes.md`
returned **empty**, so the registry table does not spell the codes the way
`validate_kind` does and the numbering scheme is unconfirmed. Per
`.ai/specifications.md:45-50`, that table is **build input**: `build.rs`
generates the `errorCode::` constants from it, asserting that hyphen-stripping
each code equals its integer column, guarded by a `table_matches_registry`
drift test. **Read the table and the generator before claiming any code is
free.** Do not invent a code number.

## Phases

> Tick `- [x]` in the same commit as the work. **An unticked box means NOT DONE.**

### Phase 1 — Manifest field and diagnostics

- [ ] Read `src/docs/spec/diagnostics/02_error-codes.md` and `build.rs` and
      determine the actual code naming/numbering. **Record the finding in this
      file** — it is currently UNVERIFIED and §4 depends on it.
- [ ] Add `description` to `validate_project_manifest` (`src/manifest/mod.rs`)
      via `validate_optional_string`, alongside the `author` call at `:166`.
- [ ] Add the 4096-byte cap with its own diagnostic.
- [ ] Add the missing-description **warning** for `kind: "package"`.
- [ ] Add `description` to `package_metadata` (`src/manifest/package.rs:420-455`)
      and to `BinaryReprMetadata` (`src/binary_repr/mod.rs:137-151`).
- [ ] Tests: `src/manifest/mod.rs` inline tests — description present/absent/
      wrong-type/over-cap; warning fires for `kind: "package"` without it;
      warning does **not** fire for `kind: "executable"`.

Acceptance: `cargo test --bin mfb manifest` passes; building a package without a
description emits the warning and still succeeds with exit 0.
Commit: —

### Phase 2 — Write and read section 18

- [ ] Add `SECTION_PACKAGE_META: u16 = 18` to `src/binary_repr/mod.rs` (highest
      in use today is 17, `SECTION_DOC_TABLE` — re-confirm with
      `grep -nE 'SECTION_[A-Z_]+: u16 = [0-9]+' src/binary_repr/mod.rs` before
      claiming 18 is free).
- [ ] Emit the section in the writer (`src/binary_repr/writer.rs` / the section
      assembly in `src/binary_repr/sections.rs`), per the §3 layout. **Omit the
      section entirely when there is no description** — do not emit an empty
      section, so packages without one are byte-identical to today.
- [ ] Read it in the compiler reader (`src/binary_repr/reader.rs`), following the
      DOC absence idiom at `:407-410`: `match sections.get(&SECTION_PACKAGE_META)`
      → `None` yields a default.
- [ ] Read it in the repository reader (`repository/src/abi.rs`), using the
      existing `read_section_table` (`:168-195`).
- [ ] Skip unknown `fieldId`s rather than erroring, per §3.
- [ ] Tests: round-trip a description through write→read; a package with no
      description emits no section 18 and reads back as `None`; a section with an
      unknown `fieldId` is skipped without error; an over-cap length in the
      section is rejected at read time.
- [ ] Tests — **the forward-compatibility regression test**, which is the whole
      premise: construct a payload containing a section with an id no reader
      knows (e.g. 99) and assert `read_binary_repr_package` and the repository's
      `read_section_table` both parse it successfully. This guards the property
      that a *future* section will not break *this* reader.

Acceptance: `mfb build --sign` on a package with a description produces a `.mfp`
containing section 18; a package without one produces a file with no section 18;
and the unknown-section-id test passes in both readers.
Commit: —

### Phase 3 — Spec sync

Part of the Hard Completion Gate, not cleanup (`.ai/specifications.md:12-18`).

- [ ] Add `description` to the schema table in
      `src/docs/spec/tooling/01_project-manifest.md` (table at `:26`), with type,
      required-ness (`no` for now — plan-61-E changes this row), meaning, and the
      4096-byte cap.
- [ ] Add section 18 to the section table in
      `src/docs/spec/package/02_binary-representation.md:53-70`.
- [ ] Write a new topic `src/docs/spec/package/NN_package-meta-section.md`
      (next free `NN`) describing the §3 layout, following the DOC topic
      (`11_doc-section.md`) as the model. **Include the §2 caveat**: the format
      has no critical-section marker, so section 18 must never carry
      security-relevant data.
- [ ] Add invisible `[[src/file.rs:Symbol]]` provenance citations and confirm
      each with grep before citing (`.ai/specifications.md:28-53`).
- [ ] Verify: `cargo build` (regenerates the embedded table; `touch build.rs` if
      a brand-new file is not picked up), `cargo test --bin mfb spec`, then
      `mfb spec package --all` renders with no leaked `[[` markers.

Acceptance: `cargo test --bin mfb spec` passes, including `spec_links_resolve`
and `spec_citations_resolve`; `mfb spec package --all` renders the new topic.
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

- *(none yet)*
