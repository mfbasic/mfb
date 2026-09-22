# plan-147-A: TrueType collections and face names

Last updated: 2026-09-21
Overall Effort: x-large (1d–3d) — the whole plan-147 feature (`canvas::listSystemFonts`,
`canvas::loadSystemFont`, their three native backends, collections, docs)
Effort: large (3h–1d)
Depends on: nothing

plan-147 adds two members to the `canvas` package:

```
canvas::listSystemFonts() AS List OF String
canvas::loadSystemFont(name AS String) AS canvas::Font
```

`listSystemFonts` answers the **full name** (sfnt `name` table nameID 4, e.g.
`"Helvetica Bold"`) of every installed face this build's loader can draw, sorted and
de-duplicated. `loadSystemFont(name)` finds that face and loads it exactly as
`canvas::loadFont` would, returning a `RES canvas::Font`; an unknown name fails with
`ErrNotFound`. Discovery is native per OS: CoreText on macOS (B), fontconfig through
`dlopen` on Linux (C), DirectWrite through COM on Windows (D). E adds the public
members and the documentation.

This letter, A, is the part that is pure MFBASIC and useful on its own: **the loader
learns TrueType Collections (`.ttc`)**, and gains the two primitives every backend needs
to turn "a file plus a name" into a font — reading a face's names out of its `name`
table, and choosing a collection face by PostScript name. Checkable outcome:
`canvas::loadFont("x.ttc")` draws face 0 of a collection instead of failing
`ErrBadFontFile`, and the internal `canvas::loadFontFace(path, postScript, fullName)`
loads the named face of a collection.

References:

- `src/codegen/builtins/canvas/func_load_font.rs` — `LOAD_FONT`, `IS_TRUETYPE`,
  `lower_font_from_bytes`, the `loadFont` man prose (`DESC`).
- `src/codegen/builtins/canvas/helper_font.rs` — `__canvas_fontTable` and the readers.
- `.ai/compiler.md`, `.ai/codegen-invariants.md`, `.ai/resources-packages.md`,
  `.ai/man-content.md`, `.ai/specifications.md`, `.ai/testing-gates.md`.
- OpenType spec: *Font Collections* (TTC header), *name* table (formats 0/1, platform
  and encoding IDs).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Work happens in the plan-147 worktree | `git -C . rev-parse --abbrev-ref HEAD` → `worktree-system-fonts` | MET (2026-09-21) |
| The build is green before starting | `cargo build` → exit 0 | re-run before starting |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the current status of *all*
> prerequisites.

The sub-plans run strictly A → B → C → D → E. B, C and D each depend on A; E depends on
B, C and D. No letter may start before the previous letter is archived.

## 1. Goal

- `canvas::loadFont(path)` accepts a `ttcf` file and loads its **face 0**.
- An internal member `canvas::loadFontFace(path AS String, postScript AS String,
  fullName AS String) AS canvas::Font` loads the face of `path` whose nameID 6 equals
  `postScript` (or, when `postScript` is `""`, whose nameID 4 equals `fullName`). A
  plain sfnt file is its own single face; a `ttcf` is searched face by face. No match
  fails `ErrBadFontFile` naming the file and the wanted name.
- Both go through one validation path, so `unitsPerEm` and the TrueType-outline rule
  apply to the *chosen face*, not the container.

### Non-goals (explicit constraints)

- The `Font` resource record layout (`gen_font.rs:FONT_BYTES`, the 96-byte resource
  record), the cross-thread font table (`_mfb_rt_canvas_fonts`, `gen_font_table.rs`) and
  every downstream helper signature (`__canvas_fontTable` and its 11 callers) do **not**
  change. The face is extracted at load time into a standalone sfnt (§3).
- No CFF (`OTTO`), WOFF/WOFF2, or variable-font (`gvar`) support. A collection face with
  CFF outlines is refused `ErrBadFontFile` exactly as an `OTTO` file is.
- No public member is added in this letter (`loadFontFace` is `internal_only`).
- Rendering output for every existing TrueType file is unchanged: the bytes handed to
  `fontFromBytes` for a plain sfnt are the file's bytes, as today.

## 2. Current State

- `__canvas_loadFont` (`func_load_font.rs:LOAD_FONT`) reads the file with
  `fs::readBytes`, refuses anything `__canvas_isTrueType` rejects, checks
  `head.unitsPerEm` in 16..16384 (bug-509), then calls `canvas::fontFromBytes(bytes)`,
  which stamps the resource record (`lower_font_from_bytes`) and registers the block in
  the global font table.
- `__canvas_isTrueType` (`IS_TRUETYPE`) accepts only version `0x00010000` or `true`,
  and deliberately names `ttcf` as refused ("several fonts in one file, so 'the font' is
  ambiguous").
- `__canvas_fontTable(b, tag)` (`helper_font.rs:FONT_TABLE`) is the **only** reader of
  the table directory: `numTables` at byte 4, records at `12 + i*16`, answer is the
  table's `offset` field. Every other offset in the helpers is relative to a table the
  directory returned (cmap subtables, `glyf + loca[gid]`, `hmtx + i*4`, `head + 18`).
- Native renderers never parse font bytes: the MFBASIC helpers rasterise glyph
  coverage and the software/Metal/Vulkan paths only blit it.
- There is no `name` table reader anywhere
  (`rg -n '"name"' src/codegen/builtins/canvas/` → only group-API parameter names).
- `encoding::utf16Decode(List OF Integer) AS String` exists and fails `ErrInvalidFormat`
  on an unpaired surrogate; `encoding::codepageDecode(Codepage.Macintosh, bytes)` exists.
- The existing test `load_font_accepts_truetype_outlines_and_refuses_every_other_container`
  (`tests/canvas/rt_canvas_font.rs`) writes a 12-byte `ttcf` header with `numFonts = 0`
  and asserts `ttcf: refused 77050022`. With collection support that file must *still*
  be refused (it holds no face), for a different reason — the assertion stays; the
  comments that say ttcf is refused *as a container* (module doc and the loop comment)
  are corrected.

### Measured populations

| What | Count | Command |
|---|---|---|
| Call sites of `__canvas_fontTable` (excluding its definition) | 11 | `rg -n '__canvas_fontTable' src/ \| wc -l` → 12 |
| Font files checked into the repo | 0 | `find . \( -path ./target -o -path ./.git \) -prune -o \( -iname '*.ttf' -o -iname '*.ttc' -o -iname '*.otf' \) -print` → nothing |
| macOS installed faces the finished feature would list (glyf, not variable) | 413 | `/tmp/ctprobe/p v \| awk -F'\t' '$5=="glyf=1 var=0"' \| wc -l` (CoreText probe, 2026-09-21) |
| …of which live in a `.ttc` | 327 | same, `\| grep -c '\.ttc$'` |
| macOS font bytes installed | 545M + 130M | `du -sh /System/Library/Fonts /System/Library/Fonts/Supplemental` |

The 327/413 figure is why collections come first: without them most macOS faces could
be listed but never loaded. The 675 MB figure is why names come from the OS (B/C/D) and
not from reading every file.

### Verified properties

- **The face extraction is sound**: in a TTC, each face's table directory stores table
  offsets from the start of the *file* (OpenType spec, Font Collections), and nothing
  downstream of `__canvas_fontTable` uses an offset relative to anything other than its
  own table; checksums are never verified
  (`rg -i checksum src/codegen/builtins/canvas/` → only a PNG mention). Verified by
  reading `helper_font.rs` and `helper_glyph.rs` in full.
- **CoreText gives no collection face index.** `CTFontManagerCopyAvailableFontURLs`
  returns one URL per face (550 URLs for 550 faces on this machine) and
  `CTFontManagerCreateFontDescriptorsFromURL` returns one descriptor each, so the face
  index is not recoverable from the API. Every backend therefore reports the
  **PostScript name**, and A resolves the face by nameID 6. (Probe: `/tmp/ctprobe/p.c`,
  output `urls=550 faces=550`.)

## 3. Design Overview

Three MFBASIC pieces in `func_load_font.rs` (and a new `helper_font_name.rs`):

1. **`__canvas_sfntFaces(bytes) AS List OF Integer`** — the table-directory offsets of
   every face. A plain sfnt answers `[0]`; a `ttcf` answers `beU32(bytes, 12 + 4*i)` for
   `i < numFonts`, each bounds-checked (`12 <= off`, `off + 12 <= len(bytes)`); a file
   with zero valid faces answers `[]`. `numFonts` above 4096 is refused (a real
   collection has tens of faces; the cap bounds work on a hostile file, bug-509's
   principle).
2. **`__canvas_extractFace(bytes, dir) AS List OF Byte`** — builds a standalone sfnt
   from the directory at `dir`: a 12-byte header copied from the face (version,
   numTables, search fields), `numTables` 16-byte records whose `offset` is rewritten,
   then each table's bytes copied with `collections::mid` and padded to 4 bytes. A table
   whose `offset + length` exceeds `len(bytes)` refuses the face (`ErrBadFontFile`).
   `dir = 0` on a plain sfnt returns `bytes` unchanged — the zero-cost path that keeps
   every existing font byte-identical.
3. **Face names** (`helper_font_name.rs`): `__canvas_faceName(bytes, dir, nameId) AS
   String` reads the `name` table *of the face at `dir`* (it scans that face's directory
   itself — it cannot use `__canvas_fontTable`, which reads the directory at 0). Record
   preference: platform 3 / encoding 1 or 10 / language 0x0409, then any platform 3 /
   encoding 1 or 10, then platform 1 / encoding 0 (Mac Roman). UTF-16BE is decoded with
   `encoding::utf16Decode` inside a `TRAP` so a malformed name answers `""` rather than
   failing a whole listing. Missing table or record answers `""`.

Then:

- `__canvas_loadFontBytes(path, bytes, dir) AS canvas::Font` — the shared tail:
  extract, `__canvas_isTrueType` on the **extracted** bytes, the `unitsPerEm` check,
  `fontFromBytes`.
- `__canvas_loadFont(path)` = read; if the file is a `ttcf`, `dir` = first face (none →
  `ErrBadFontFile "font collection holds no faces"`); else the existing version check;
  then `__canvas_loadFontBytes`.
- `canvas::loadFontFace(path, postScript, fullName)` (internal, `Body::mfb`) = read,
  walk `__canvas_sfntFaces`, pick the first face whose nameID 6 = `postScript` (nameID 4
  = `fullName` when `postScript = ""`), `__canvas_loadFontBytes`.

**Correctness risk** concentrates in `__canvas_extractFace`: an off-by-one in a rewritten
offset renders garbage or reads past a table. It is covered by a round-trip test that
draws the same glyph from a plain file and from the same face wrapped in a two-face
collection and compares frames exactly.

**Design uncertainty**: none left that a probe can settle before coding — the CoreText
face-index question was the one, and it is settled above.

**Gate class**: behavior-changing. The gate is runtime tests. Byte-identity is not the
gate. **Expected diffs**: the canvas helpers are `RegistryHelper::always`, so every
canvas app's IR changes — `tests/syntax/app/app-mouse-surface/golden/*` (`.ir`,
`.macos-aarch64.app.nir`, `.macos-aarch64.app.nplan`, and the four `.ncodesum`) are
expected to diff, and only those plus goldens of other canvas-importing fixtures
(`rg -l 'IMPORT canvas' tests/syntax` names them). Any other golden diff is a bug.

**Rejected alternatives**

- *Thread a face offset through the helpers* — 12 signatures, 4 callers, the resource
  record and the cross-thread font-table slot format would change
  (census: `rg -n '__canvas_fontTable' src/`). Extraction touches one function.
- *Refuse `.ttc` and list only standalone files* — rejected by the user; 327 of 413
  macOS faces live in collections.
- *Resolve a collection face by index from the OS* — CoreText cannot supply one (§2).

## 4. `name` table detail

`name` header: `format` u16, `count` u16, `stringOffset` u16, then `count` 12-byte
records `platformID, encodingID, languageID, nameID, length, offset` (u16 each). String
bytes live at `name + stringOffset + offset`. Format 1's language-tag records follow the
name records and are ignored. Bounds: a record whose string runs past `len(bytes)` is
skipped.

## Compatibility / Format Impact

- `loadFont` now **accepts** `ttcf` files (face 0) that it used to refuse with
  `ErrBadFontFile`. A `ttcf` with no faces, or whose face 0 is CFF, is still refused
  with `ErrBadFontFile`.
- No change to the Font record, the font table, any public signature, or pixels drawn
  from an existing `.ttf`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work; `- [~]` for partial with what remains; moot tasks struck through with
> evidence; fill `Commit:` the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Collection faces and extraction

`loadFont` loads face 0 of a `.ttc`; plain `.ttf` behavior is unchanged.

- [x] `func_load_font.rs`: add `__canvas_sfntFaces` and `__canvas_extractFace`
      (as `RegistryHelper::always`, beside `canvas_isTrueType`), and the shared
      `__canvas_loadFontBytes`; rewrite `LOAD_FONT` to use them.
- [x] `func_load_font.rs`: rewrite the `IS_TRUETYPE` doc comment (ttcf is now a
      container handled before this check, not a refusal) and the `loadFont` `DESC`
      paragraph "What this build reads" to say collections load face 0.
- [x] `src/codegen/builtins/errorcode/mod.rs` and `src/docs/spec/diagnostics/02_error-codes.md`:
      correct the `ErrBadFontFile` description where it lists `ttcf` as refused.
      Check: `cargo test --bin mfb table_matches_registry` → 1 passed.
- [x] Tests (`tests/canvas/rt_canvas_font.rs`): a Rust-built two-face `.ttc` wrapping
      `truetype_fixture()` twice → `loadFont` succeeds and draws a frame
      `compare_exact` to the same text from the plain fixture; a `ttcf` with
      `numFonts = 1` whose face offset points past EOF → refused 77050022; a `ttcf`
      whose face table runs past EOF → refused 77050022. Keep the existing `ttcf`
      (`numFonts = 0`) assertion and correct its comments.

Acceptance: a collection's face 0 draws exactly what the standalone file draws, and
every malformed collection is refused with `ErrBadFontFile`.
  Check: `cargo test --test rt_canvas_font` → all pass, including the three new cases
  (est. 3 min). **Observed: `test result: ok. 22 passed; 0 failed` (150.55 s).**
Commit: —

### Phase 2 — Face names and `loadFontFace`

The primitives B/C/D feed: read a face's names; load a face by PostScript name.

- [ ] New `src/codegen/builtins/canvas/helper_font_name.rs`: `__canvas_faceName`
      (§3.3, §4), registered in `canvas/mod.rs`.
- [ ] `func_load_font.rs`: register internal member `canvas::loadFontFace(path,
      postScript, fullName) AS canvas::Font`, `Body::mfb`, errors `ErrBadFontFile`,
      `ErrOutOfMemory`; add `"canvas.loadFontFace"` to nothing — it is `Body::mfb`, not a
      runtime call (confirm: `rg -n '"canvas.loadFont"' src/target` → no hits today).
- [ ] Tests: extend the Rust fixture builder with a `name` table (platform 3 UTF-16BE
      nameID 4 and 6, plus a platform-1 Mac Roman nameID 4 on a second face); a two-face
      collection with distinct PostScript names → `loadFontFace(path, "FaceB", "")`
      draws face B (distinguish faces by giving them different advance widths and
      asserting `measureText` width); `loadFontFace(path, "", "Face A Full")` picks face
      A by full name; an absent name → refused 77050022 with the name in the message.

Acceptance: a named face of a collection loads and is the right face.
  Check: `cargo test --test rt_canvas_font` → all pass incl. the new cases (est. 3 min).
Commit: —

### Phase 3 — Goldens and docs sync

- [ ] Regenerate only the canvas-importing fixture goldens that diff, after proving each
      diff is the new helper bodies (inspect `app_mouse_surface.ir` for the added
      `__canvas_sfntFaces`/`__canvas_extractFace`/`__canvas_faceName` functions and
      nothing else) — per AGENTS.md, answer the four questions in the commit message.
- [ ] `src/docs/spec/app/06_canvas.md`: add a *Fonts* paragraph — collections load face
      0 through `loadFont`, faces are extracted to a standalone sfnt at load
      (`[[src/codegen/builtins/canvas/func_load_font.rs:LOAD_FONT]]`).
- [ ] `mfb man canvas loadFont` renders the new "What this build reads" paragraph;
      `scripts/man-census.sh --memory-scope` → 0 unclassified.

Acceptance: goldens reflect only the added helpers; man and spec say collections load.
  Check: `cargo test --test acceptance app_mouse_surface` (or the fixture's golden test
  name, `rg -n app_mouse_surface tests/*.rs`) → pass (est. 5 min);
  `cargo test --bin mfb spec` → pass (est. 3 min).
Commit: —

## Validation Plan

- Tests: `tests/canvas/rt_canvas_font.rs` (positive round-trip, face selection, three
  negative containers, the kept `numFonts = 0` refusal).
- Coverage check: every new helper is reached by a new test case (extraction by the
  round-trip, `faceName` by both selection tests, the bounds refusals by the negative
  cases).
- Runtime proof: a headless `--app` program loading
  `/System/Library/Fonts/Helvetica.ttc` draws non-empty text (`rt_canvas_metal.rs`-style
  host-font test, gated to macOS like `every_glyph_of_a_text_run_draws_its_own_bitmap`).
- Doc sync: `loadFont` DESC, `IS_TRUETYPE` comment, `ErrBadFontFile` description,
  `06_canvas.md`.
- Final gate for the whole feature runs once, in plan-147-E.

## Open Decisions

- Face cap for a collection — 4096 (recommended; bounds a hostile header) vs no cap.

## Corrections

- **The error-code registry lives at `src/codegen/builtins/errorcode/mod.rs`**, not
  `src/codegen/errorcode/mod.rs` as Phase 1 first said (`grep -rln ErrBadFontFile src/`).
- **The two past-EOF cases are one test** (`a_collection_that_runs_past_its_end_is_refused`,
  printing a line per case), and the round-trip is `a_collection_loads_its_first_face`;
  "three new cases" = those two negatives plus the round-trip. Test helpers `run` and
  `render_env` gained `run_files` / `render_env_files` variants to drop fixture files.
- MFBASIC note: `next` is a keyword (FOR … NEXT); the offset cursor in
  `__canvas_extractFace` is named `cursor`.

## Summary

Risk is confined to `__canvas_extractFace`'s offset rewrite, guarded by an exact-frame
round-trip. The Font record, font table, helper signatures and every native renderer are
untouched.
