# plan-126-D: The doc table and the doc page model

Last updated: 2026-09-06
Effort: medium (1h–2h)
Depends on: plan-126-C

Moves MFPC section 17 — the `DOC` table the compiler embeds in every documented
`.mfp` — and the renderable page model built from it into `mfb_wire`. After this
sub-plan the registry can decode a published package's documentation, and
`mfb doc`'s standalone HTML page and the registry's future page are two renderers
over one model rather than two models.

Behavioral outcome: **nothing observable changes.** `mfb pkg doc` produces a
byte-identical HTML page, every `.mfp` is byte-identical, and the `-ast` dump is
unchanged. The new capability is latent: `mfb_wire` can now decode section 17, which
plan-126-E consumes.

References:

- `src/binary_repr/writer.rs:1112-1119` — the doc section is emitted only when
  non-empty, and "does not affect execution or the ABI".
- `src/doc/mod.rs:1-6` — the module doc describing the two entry points
  (`from_source` and `from_package`) that share one model.
- `src/ast/types.rs:150-151` — `DocProseKind::code` is "Stable on-wire code for the
  `.mfp` doc section and `-ast` output", so it is wire format, not just an AST enum.

## Prerequisites

See plan-126-A § Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-126-C complete (`wire/src/mfpc.rs` owns the section ids and table reader) | `grep -c SECTION_DOC_TABLE wire/src/mfpc.rs` → **≥ 1** (corrected from "→ 1") | MET (measured 2026-09-12: **2**; C landed as 8af4a40eb, 3fd644f54, 427af4b4f). The count is 2, not 1, because `the_section_ids_are_frozen_wire_values` asserts the constant by name as well as defining it — the intent (mfpc.rs owns the id) holds, the expected count was miscalibrated. |

If plan-126-C is not complete, this sub-plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop, and report
> the status of *all* prerequisites if you stop.

## 1. Goal

- `PackageDocs` / `PackageDocEntry` / `DeclDocEntry`, the `DOC_KIND_*` codes,
  `DocProseKind`, `read_doc_table`, `encode_doc_table` and `doc_kind_name` live in
  `mfb_wire`.
- The `DocPage` / `DocGroup` / `DocDecl` / `Prose` model and `from_package` live in
  `mfb_wire`; `from_source` stays in the compiler and calls the shared helpers.
- `mfb_wire` exposes a one-call path from `.mfp` bytes to a `DocPage`, with no
  compiler dependency.
- `mfb pkg doc` output is byte-identical to before.

### Non-goals (explicit constraints)

- **`docs_from_ir` does not move.** `src/binary_repr/writer.rs:1291-1325` converts
  `crate::ir::ProjectDocs` → `PackageDocs`; `ProjectDocs` is compiler IR and stays
  compiler-side. Only the `PackageDocs` half of the boundary moves.
- **`from_source` does not move.** `src/doc/mod.rs:215-348` needs `AstProject`,
  `DocBlock`, `DocHeaderKind`, `Function`, `Item`, `Visibility` — all compiler AST.
- **`src/doc/html.rs` does not move.** It is the standalone `mfb doc` renderer (734
  lines) and depends on `crate::html::escape` (`src/html.rs`, shared with the
  coverage renderer, bug-340 B7). The registry will get its own maud renderer in
  plan-126-F; neither `doc/html.rs` nor `src/html.rs` is shared code.
- **No wire-format change.** Section 17's encoding, field order, `DOC_KIND_*` codes
  and `DocProseKind::code` values are frozen; a change would invalidate every
  already-published package.
- **No change to `mfb doc` or `mfb pkg doc` output**, including whitespace.

## 2. Current State

### The doc table is self-contained

`read_doc_table` (`src/binary_repr/reader.rs:88-147`) decodes section 17 using only
inline, length-prefixed data: no string-pool ids, no type-table references. Read the
field list at `src/binary_repr/mod.rs:466-480` — `signature` is a **pre-rendered
string**, `args`/`props`/`errors` are `Vec<(String, String)>`, `desc` is
`Vec<(u8, String)>`. Nothing in the decode path reaches another section.

This is what makes the move cheap and the registry-side decode possible at all.

### It is emitted only when non-empty, and real packages carry it

`src/binary_repr/writer.rs:1112-1119` pushes the section only `if !self.docs.is_empty()`.
Measured across every `.mfp` in the tree by walking the MFPC section table:

| package | section 17 |
|---|---|
| `packages/jwt/jwt.mfp` | 27,951 B |
| `packages/json_schema/json_schema.mfp` | 17,739 B |
| `packages/libsnd/libsnd.mfp` | 15,440 B |
| `packages/mustache/mustache.mfp` | 11,231 B |
| `packages/sqlite3/sqlite3.mfp` | 10,404 B |
| `packages/yaml/yaml.mfp` | 9,027 B |
| `examples/browser/{display,dom,fetch}` | absent |

So the "package has no documentation" path is not hypothetical — three of the nine
`.mfp` files in the tree take it.

### The doc model is shared between two entry points

`src/doc/mod.rs` is 402 lines in three parts:

| Lines | Content | Destination |
|---|---|---|
| 13–52 | `DocPage`, `DocGroup`, `DocDecl`, `Prose` | `mfb_wire` |
| 54–162 | `kind_label`, `badge_class`, `member_label`, `group_title`, `prose_from_codes`, `assemble_groups`, `reserved_anchors`, `anchor` | `mfb_wire` |
| 166–211 | `from_package` | `mfb_wire` |
| 215–348 | `from_source` | stays |
| 352–388 | `source_decl_meta` | stays |
| 391–398 | `split_subtitle` | `mfb_wire` (both callers use it) |

### Measured populations

| What | Count | Command |
|---|---|---|
| `read_doc_table` / `doc_kind_name` lines | 60 / 16 | `sed -n '88,147p' src/binary_repr/reader.rs \| wc -l`; `sed -n '15,30p' … \| wc -l` |
| `encode_doc_table` / `docs_from_ir` lines | 37 / 35 | `sed -n '1254,1290p' src/binary_repr/writer.rs \| wc -l`; `sed -n '1291,1325p' … \| wc -l` |
| `PackageDocs` type block lines | 39 | `sed -n '443,481p' src/binary_repr/mod.rs \| wc -l` |
| `DocProseKind` block lines / references | 36 / 48 | `sed -n '130,165p' src/ast/types.rs \| wc -l`; `grep -rho DocProseKind src --include='*.rs' \| wc -l` |
| `src/doc/mod.rs` / `src/doc/html.rs` lines | 402 / 734 | `wc -l src/doc/*.rs` |
| Tests in `src/doc/html.rs` | 16 | `grep -c '#\[test\]' src/doc/html.rs` |
| Tests in `src/binary_repr/tests/doc_table_tests.rs` | 1 (77 lines) | `grep -c '#\[test\]' …`; `wc -l …` |
| `read_package_docs` references in `src/` | 8 | `grep -rho read_package_docs src --include='*.rs' \| wc -l` |
| `DOC_KIND_*` codes | 6 (FUNC 0 … RESOURCE 5) | `sed -n '483,489p' src/binary_repr/mod.rs` |

### Verified properties

- **The doc decode path touches nothing compiler-specific.** Read
  `read_doc_table` and every helper it calls: `cursor_string`, `cursor_prose_list`,
  `cursor_pair_list`, `cursor_optional_str`, `cursor_u16`, `cursor_u32`,
  `bounded_capacity` — all already in `mfb_wire` after plan-126-B. The four
  non-module references in `src/binary_repr/reader.rs` (`crate::manifest::MAX_DESCRIPTION_BYTES`
  at `:73`/`:76`, `crate::types::format_thread_type` at `:800`/`:823`) are in the
  section-18 and type-table paths, **not** the doc path.
- **`split_subtitle` and the eight naming/grouping helpers are genuinely shared.**
  Verified by reading `from_source`: it calls `reserved_anchors` (`:259`),
  `group_title` (`:295`), `anchor` (`:297`), `kind_label` (`:298`), `badge_class`
  (`:299`), `member_label` (`:300`), `split_subtitle` (`:338`) and `assemble_groups`
  (`:339`). So moving the model requires making these `pub` in `mfb_wire` rather
  than keeping them private — a deliberate widening of the shared crate's API.
- **`DocProseKind::code` is wire format, not an internal enum.** Its own doc comment
  at `src/ast/types.rs:150` says "Stable on-wire code for the `.mfp` doc section and
  `-ast` output". Moving it therefore also touches `-ast` golden output if its
  values change — they must not.
- **`src/html.rs::escape` is not shared with the registry.** Read it: it is used by
  `src/doc/html.rs:2` and the coverage renderer (bug-340 B7). maud escapes
  automatically, so plan-126-F needs nothing from it.

## 3. Design Overview

Two moves, in dependency order:

1. **`wire/src/docs.rs`** — the wire layer: `DocProseKind`, `PackageDocs` and its
   two entry types, `DOC_KIND_*`, `read_doc_table`, `encode_doc_table`,
   `doc_kind_name`.
2. **`wire/src/docpage.rs`** — the presentation-independent model: `DocPage`,
   `DocGroup`, `DocDecl`, `Prose`, the nine shared helpers (now `pub`), and
   `from_package`.

Plus a convenience entry point in `mfb_wire` that goes from `.mfp` bytes to
`Option<PackageDocs>` using `mfpc::read_section_table` from plan-126-C — this is the
single function plan-126-E calls.

**Where correctness risk concentrates:** `DocProseKind` moving out of
`src/ast/types.rs`. It has 48 references and its numeric codes appear in `-ast`
golden output. A renumbering would be invisible to the type checker and would break
every already-published package's doc section. Pin the codes with an explicit test.

**Where design uncertainty concentrates:** nothing substantive. The self-containment
of section 17 was verified by reading the decoder, not inferred.

**Byte-identity IS this sub-plan's gate.** Provably-neutral code motion:
`scripts/artifact-gate.sh target/release/mfb all` must report `diffs=0` (the `-ast`
dumps in particular, since `DocProseKind::code` feeds them), and a rebuilt
`packages/jwt/jwt.mfp` must be `cmp`-identical to the pre-change build. A diff is a
bug introduced by the move — localize and fix it; it is never a reason to abandon
the design.

**Rejected alternative — leave the model in the compiler and have the registry build
its own.** That is a second implementation of anchor generation, group ordering and
subtitle splitting, drifting from the first. The whole point of the shared crate is
that a package's documentation renders the same way wherever it is rendered.

**Rejected alternative — move `src/doc/html.rs` too, and have the registry embed it.**
Blocked on the registry's CSP: `repository/src/web/mod.rs:44` is
`default-src 'none'; style-src 'self'` and `src/doc/html.rs:172` emits an inline
`<style>`. Injecting the HTML would also require `maud::PreEscaped`, the bypass the
registry's module doc calls out as the one thing to keep greppable. plan-126-F
writes a maud renderer instead.

## Compatibility / Format Impact

Nothing externally observable changes. Section 17's encoding, the `DOC_KIND_*`
codes, `DocProseKind::code` values, `mfb doc` / `mfb pkg doc` HTML output and the
`-ast` dump format are all unchanged. The new public surface is internal to the
workspace.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; `- [~]` plus one line for partial; `- [x] ~~text~~ —
> moot: <evidence>` for moot. Fill `Commit:` the moment a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — The wire layer

- [x] Create `wire/src/docs.rs` with `DocProseKind` — **all four** methods
      (`from_keyword`, `code`, `from_code`, `label`), not the two the plan named;
      see Corrections — plus `PackageDocs`, `PackageDocEntry`, `DeclDocEntry`, the
      six `DOC_KIND_*` codes, `doc_kind_name`, `read_doc_table` and
      `encode_doc_table`, each moved verbatim. Derives kept exactly
      (`PackageDocs` is `Clone, Default` and deliberately **not** `Debug`). The
      module doc records which consumer freezes the numeric codes (the `.mfp`
      wire) and which freezes the labels (the `-ast` dump).
- [x] Re-export `DocProseKind` from `src/ast/types.rs` as
      `pub use mfb_wire::docs::DocProseKind;` — it reaches `crate::ast` through
      `pub use types::*` in `ast/mod.rs`, so all 48 references resolve unchanged.
      `src/binary_repr/mod.rs` glob-re-exports `mfb_wire::docs::*` for
      `PackageDocs` and the codec.
- [x] Leave `docs_from_ir` in the compiler; it now constructs the `mfb_wire`
      types (kept; the deletion script asserted it survived).
- [x] Add `mfb_wire::docs::read_package_doc_section(payload)`: section table →
      section 17 → `read_doc_table`, `Ok(empty)` when the section is absent.
      **Refined:** it returns `Err` — not `Ok(empty)` — for a payload that is not
      a container or a section 17 that is malformed, so plan-126-E's backfill can
      count a malformed doc section as a finding rather than confusing it with an
      undocumented package.
- [x] Update the comment in `writer.rs` that said the decoders "stay in
      reader.rs". Rewritten to say the codec moved to `mfb_wire::docs` and only
      `docs_from_ir` stays. The matching section header in `reader.rs` was
      corrected too.
- [x] Tests: moved `doc_table_round_trips` **verbatim** from
      `src/binary_repr/tests/doc_table_tests.rs` into `wire/src/docs.rs` (file
      deleted, `mod` line removed). Added `the_doc_codes_and_labels_are_frozen`,
      pinning all six `DOC_KIND_*` ids, all four prose codes **and all four
      `-ast` labels** by literal, and `trailing_bytes_after_a_doc_table_are_rejected`
      (bug-282 B3). Also `a_truncated_doc_table_is_rejected` and
      `read_package_doc_section_separates_absent_from_malformed`.
- [x] Add a decode test against a **real** package — using the **committed**
      `repository/tests/fixtures/libsnd.mfp` (real 15,440-byte section 17), not the
      gitignored `packages/jwt/jwt.mfp`; see Corrections.
      `a_real_packages_doc_section_round_trips_byte_for_byte` asserts a non-zero
      decl count **and** that decoding then re-encoding reproduces section 17
      byte for byte.
- [x] Added task: delete the compiler's local `read_doc_table`, `doc_kind_name`,
      `encode_doc_table` and `DOC_KIND_*` rather than leaving them beside the glob,
      because a local item silently shadows a glob import (the plan-126-C trap).
      Verified empty by
      `grep -rnE "fn (read_doc_table|doc_kind_name|encode_doc_table)\b|const DOC_KIND_|pub enum DocProseKind|pub struct (PackageDocs|PackageDocEntry|DeclDocEntry)\b" --include='*.rs' src/`.

Acceptance: MET.
`cargo check --all-targets` clean (only the three pre-existing
`repository/src/server.rs` `unused axum::Json` warnings).
`cargo test -p mfb_wire` → **51 passed**, all six `docs::tests::*` included.
`cargo test --bin mfb binary_repr` → **170 passed** (171 before; the one test that
moved out). `cargo test --bin mfb ast::` → **225 passed**. `cargo test --bin mfb
doc::` → **27 passed**, including the 16 `src/doc/html.rs` tests whose module
imports the doc types from `crate::binary_repr` — green through the glob with
`html.rs` untouched.
`scripts/artifact-gate.sh target/release/mfb all` → 1427 tests, 1593 builds,
**2001 goldens checked, 0 diffs**, `git status tests/` clean — the `-ast` dumps
did not move.
`mfb pkg doc` byte-identity, against output from the **pre-change binary** saved
before plan-126-B (`/tmp/p126-mfb-prechange`), both reading the same pre-change
`jwt.mfp`: the new render is **`cmp`-identical** (54,788 B, exit 0). The plan's
literal `packages/jwt/jwt.mfp` path is a gitignored artifact, so the package was
built first and the comparison run against that saved build.
A rebuilt `packages/jwt/jwt.mfp` is `cmp`-identical to the pre-change build.
The whole-workspace `cargo test --no-fail-fast` is the plan-wide final gate in
follow-plan §5.
Commit: —

### Phase 2 — The page model

- [ ] Create `wire/src/docpage.rs` with `DocPage`, `DocGroup`, `DocDecl`, `Prose`
      (`src/doc/mod.rs:13-52`), the nine shared helpers (`:54-162` and `:391-398`)
      made `pub`, and `from_package` (`:166-211`).
- [ ] Reduce `src/doc/mod.rs` to `from_source` (`:215-348`) and `source_decl_meta`
      (`:352-388`), importing the model and helpers from `mfb_wire` and re-exporting
      the model so `src/doc/html.rs` and `src/cli/doc.rs` are unchanged.
- [ ] Verify `src/doc/html.rs` (734 lines, 16 tests) and `src/html.rs` are untouched;
      if either needed an edit, the re-export is incomplete.
- [ ] Confirm neither extraction orphaned a doc comment onto a neighbouring item in
      `src/doc/mod.rs`, `src/ast/types.rs`, `src/binary_repr/mod.rs` or `reader.rs`.
- [ ] Tests: move any `from_package` coverage out of `src/doc/html.rs`'s 16 tests
      into `wire/src/docpage.rs` (`from_package_no_package_uses_fallback_and_empty_render`
      at `src/doc/html.rs:402` and `from_package_full_page_renders_every_element` at
      `:414` both exercise it); add a test that `from_source` and `from_package`
      produce the same anchors for the same symbol names, pinning the shared helper.

Acceptance: `rustup run 1.96.0 cargo test --no-fail-fast` passes with the 16
`src/doc/html.rs` tests still green; `mfb doc <a source project> --out /tmp/b.html`
and `mfb pkg doc packages/jwt/jwt.mfp --out /tmp/c.html` both produce files
`cmp`-identical to pre-change output. The `from_source`/`from_package` anchor-parity
test is what proves the helpers were shared rather than duplicated.
Commit: —

## Validation Plan

- **Tests:** `wire/src/docs.rs` (moved round-trip + frozen-code pins + trailing-bytes
  negative + real-package decode), `wire/src/docpage.rs` (moved `from_package` tests
  + anchor parity), and the 16 existing `src/doc/html.rs` tests unchanged.
- **Coverage check:** `src/binary_repr/tests/doc_table_tests.rs` holds exactly 1 test
  for a 97-line codec — the pre-existing coverage here is thin, so a green run is
  weak evidence. The real-package decode test added in Phase 1 is what makes it
  meaningful. Confirm the moved tests actually run under `mfb_wire`
  (`cargo test --no-fail-fast 2>&1 | grep 'Running.*mfb_wire'`).
- **Runtime proof:** `mfb pkg doc` on all six documented packages
  (`packages/{jwt,json_schema,libsnd,mustache,sqlite3,yaml}`) and on one undocumented
  one (`examples/browser/dom/dom.mfp`, section 17 absent) — the latter must still
  take the `render_empty_html` path at `src/cli/pkg.rs:1823-1827` and exit 0.
- **Byte-identity:** `scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`,
  with `scripts/gate-lock.sh` acquired first. Additionally `cmp` a rebuilt
  `packages/jwt/jwt.mfp` against the pre-change build — the `mfp` dump kind is
  deliberately outside the artifact gate (`.ai/testing-gates.md`), so the gate cannot
  see a doc-encoding regression.
- **Doc sync:** `src/binary_repr/writer.rs:1250-1253` (the "decoders stay in
  reader.rs" comment). Check `grep -rn "doc section\|DOC block" .ai/ src/docs/spec/`
  for anything describing where the codec lives.
- **Acceptance:** `rustup run 1.96.0 cargo test --no-fail-fast`.
- **Format:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Does `DocHeaderKind` move with `DocProseKind`?** It sits beside it
  (`src/ast/types.rs:101`) and has 69 references, but it is consumed only by
  `from_source` and the AST — it never reaches the wire. Recommended: **leave it**,
  and note in `wire/src/docs.rs` why its sibling did not follow. (§Phase 1)
- **One `wire/src/docs.rs` or split codec from model?** Recommended two files
  (`docs.rs` wire, `docpage.rs` model) so the frozen-format half is visibly separate
  from the presentation half, which is free to evolve. (§3)

## Corrections

- **`-ast` does not print `DocProseKind::code` — it prints `label()`.** The plan's
  § Verified properties and § Design Overview both say the numeric codes "appear
  in `-ast` golden output", and use that to argue the move risks `-ast` goldens.
  Read the serializer: `src/ast/serialize.rs:214` writes
  `json_string(prose.kind.label())` — the **strings** `"desc"`/`"warn"`/`"info"`/
  `"sec"`. The numeric `code()` is consumed only by `src/ir/docs.rs:55`
  (`(prose.kind.code(), prose.text.clone())`), the IR → section-17 path. So two
  different things are frozen for two different consumers: numeric codes by the
  `.mfp` wire, labels by the `-ast` goldens. The risk conclusion stands — both
  must not change — but the plan named the wrong method for `-ast`. The
  frozen-value test in `wire/src/docs.rs` pins **both**, and the module doc
  records which consumer freezes which.

- **All four `DocProseKind` methods move, not two.** Phase 1 says to move it
  "keeping `from_keyword` and `code` verbatim". The impl has four:
  `from_keyword`, `code`, `from_code` and `label`
  (`sed -n '/^impl DocProseKind/,/^}/p' src/ast/types.rs`). `from_code` is how
  `doc::from_package` turns wire codes back into kinds, and `label` is the `-ast`
  serializer's — leaving either behind would split the enum's vocabulary across
  two crates. All four moved verbatim.

- **The real-package decode test uses `repository/tests/fixtures/libsnd.mfp`, not
  `packages/jwt/jwt.mfp`.** `packages/*.mfp` are gitignored build artifacts
  (plan-126-B § Verified properties: `git ls-files 'packages/*.mfp'` → nothing),
  so a test reading `jwt.mfp` passes on a machine that happened to build it and
  fails in CI and every fresh worktree. `libsnd.mfp` is tracked and carries a real
  15,440-byte section 17 (measured by walking its MFPC section table). The test is
  also *stronger* than the planned "non-zero decl count": it asserts
  `encode_doc_table(read_doc_table(section)) == section` **byte for byte**, which a
  synthetic fixture cannot prove.

- **`read_package_docs` stays in the compiler; `read_package_doc_section` is
  additive.** They are not the same function at different addresses.
  `binary_repr::read_package_docs(path)` is
  `read_package_binary_repr(path)?.project.docs` — a **full** package decode
  including container identity validation. The new
  `mfb_wire::docs::read_package_doc_section(payload)` reads only the section
  table and section 17. Replacing the former with the latter in `mfb pkg doc`
  would silently drop the identity check for a tampered package. The new
  function's doc says it is not a substitute. It also deliberately returns
  `Ok(empty)` for an absent section but `Err` for a malformed one, because
  plan-126-E's backfill must count the second as a finding.

- **`read_package_docs` has 7 references, not 8.**
  `grep -rn read_package_docs src --include='*.rs'` → the definition, **one**
  production caller (`src/cli/pkg.rs`), and five test references.

- **The glob-shadowing trap from plan-126-C applies here in full.**
  `binary_repr` will glob-re-export `mfb_wire::docs::*` so `src/doc/html.rs`'s
  test module — which imports `DeclDocEntry, PackageDocEntry, PackageDocs` from
  `crate::binary_repr` — stays untouched. But a local `pub(super) fn
  read_doc_table` / `doc_kind_name` / `encode_doc_table` and the local
  `DOC_KIND_*` consts would each **silently shadow** that glob and compile with
  two copies. They must be deleted, not re-exported over.

## Summary

The risk is concentrated in one place that does not look risky: `DocProseKind`'s
numeric codes are frozen wire format feeding both section 17 and the `-ast` dumps,
and nothing in the type system would catch a renumbering. The frozen-code pin test
and the byte-identity gate exist for exactly that. Everything else is a clean
extraction that the self-contained shape of section 17 makes cheap. Deliberately
left behind: `docs_from_ir`, `from_source`, `src/doc/html.rs` and `src/html.rs`.
