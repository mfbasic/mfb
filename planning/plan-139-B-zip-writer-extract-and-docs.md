# plan-139-B: `zip` package — writer, `extractTo`, README and doc.html

Last updated: 2026-09-15
Effort: large (3h–1d)
Depends on: plan-139-A (complete). Whole-feature prerequisites: plan-139-A § Prerequisites.

Adds the zip writer (`create`, `addFile`, `addText`, `addDirectory`, `finish`) and `extractTo` to
`packages/zip`, and the package's README/doc.html. Outcome: **every archive `zip::finish` produces is
read back by `zip::open` (both overloads), by Python `zipfile.testzip()` with no bad entry, and by
`unzip -t` with "No errors"; `extractTo` writes exactly the archive's files under the target
directory and refuses any entry that would land outside it.**

References:

- plan-139-A §4 (types, errors, `readAt`); PKWARE APPNOTE 6.3.10 §4.3.7, §4.3.12, §4.3.16,
  §4.3.14–15, §4.5.3 (ZIP64 extra field).
- `mfb man fs isWithin`, `fs createDirectories`, `fs open`, `fs writeAllBytes`, `fs pathJoin`,
  `fs pathNormalize`.
- `packages/jwt/README.md`, `packages/jwt/doc.html` — README/doc.html precedent.

## Prerequisites

See plan-139-A § Prerequisites — all rows must be MET, and its two `fs` rows are delivered by
**plan-139-E**, which gates the whole chain `E -> A -> B -> C -> D`. Plus:

| Must be true | Command | Status |
|---|---|---|
| plan-139-A complete | every `- [ ]` in `planning/plan-139-A-*.md` ticked; `target/release/mfb test packages/zip` → pass | **MET** (2026-09-19: plan-139-A has 0 unticked boxes and is archived to `planning/completed/`; `target/release/mfb test packages/zip` → `Tests: 44  Pass: 44  Fail: 0`) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every command
> before you continue and before you stop; if you stop, report the status of *all* prerequisites.

## 1. Goal

- Writer output round-trips through `zip::open`, Python `zipfile`, and `unzip -t`.
- `zip::extractTo` extracts files and directories safely from both source kinds.

### Non-goals

- The writer is in-memory: a `Builder` holds every added entry's (compressed) bytes; `finish`
  returns `List OF Byte`. Writing directly to an `fs::File` is not part of plan-139 (Open Decisions).
- No file permission or timestamp restoration on extract — `fs` has no chmod/utime
  (`grep -h -o 'name: "[a-zA-Z]*"' src/codegen/builtins/fs/func_*.rs` → no `chmod`/`setMode`/
  `setModified`). Documented on the page.
- No change to letter A's read API or types.

## 2. Current State

After plan-139-A: `packages/zip` reads; `Archive`/`Entry`/`Source` exist; `ErrorChecksum = 93150001`.
`compress::deflate(data, [level])` produces raw DEFLATE; `compress::crc32` checksums
(`mfb man compress`). `fs::isWithin` resolves `.`, `..` and symlinks against the disk
(`mfb man fs` Description).

## 3. Design Overview

`Builder` is a value (each call returns an updated copy, like the todo proposed): `entries AS List OF
Entry` + `body AS List OF Byte` (local headers + data concatenated). `finish` appends the central
directory and EOCD. ZIP64 records are emitted **only** when a size ≥ 0xFFFFFFFF, an offset ≥
0xFFFFFFFF, or the entry count ≥ 0xFFFF, so small archives are byte-for-byte classic zip.
Deterministic output: fixed "version made by" 3.0/Unix (`0x031E`), no extra timestamp field unless
`modifiedSeconds > 0`, entries in insertion order.

`extractTo` is the risk point (path traversal). It pre-validates **every** entry before writing
anything, so a hostile archive leaves the directory untouched.

## 4. Detailed Design

### 4.1 Writer

- `create() AS Builder` — empty.
- `addFile(builder, name, data AS List OF Byte, store AS Boolean = FALSE, modifiedSeconds AS Integer = 0) AS Builder`
  — validate name (§4.3 rules 1–3, else `ErrInvalidPath`); crc = `compress::crc32(data)`;
  `store` → method 0; else `compressed = compress::deflate(data)` and fall back to method 0 when
  `len(compressed) >= len(data)` (this is the format's normal choice, what `zip(1)` does, not an
  error fallback). Local header with flag bit 11 set (names are UTF-8), DOS time from
  `modifiedSeconds` (0 → 1980-01-01 00:00), external attributes `0o100644 << 16`.
- `addText(builder, name, text, store = FALSE, modifiedSeconds = 0)` — UTF-8 bytes of `text`.
- `addDirectory(builder, name, modifiedSeconds = 0)` — name gets a trailing `/`, method 0, size 0,
  attributes `0o040755 << 16`.
- `finish(builder, comment AS String = "") AS List OF Byte` — comment UTF-8 length > 65535 →
  `ErrTooLarge`.
- Adding a name already present is allowed (the format permits it; letter A's `find` returns the
  first); documented on the page. See Open Decisions.

### 4.2 `extractTo(archive, directory AS String, maxTotalBytes AS Integer = 1073741824) AS Integer`

1. `fs::directoryExists(directory)` else `ErrNotFound`.
2. Pre-pass over `archive.entries`: apply §4.3 to each; sum `size`; sum > `maxTotalBytes` →
   `ErrTooLarge`. Any failure raises before any write.
3. Write pass: directories via `fs::createDirectories`; files: `fs::createDirectories(parent)`, then
   check `fs::isWithin(directory, parent)` (catches a symlink already on disk), then
   `RES out = fs::open(target, "write")`. Method 0 from a file source: copy in 1 MiB `readAt` chunks
   with a running `compress::crc32`, verify at the end (bytes stay bounded at 1 MiB). Method 8:
   `read(archive, entry, entry.size)` then `fs::writeAllBytes`. A checksum failure deletes the
   partial file (`fs::deleteFile`) and raises `ErrorChecksum`.
4. Returns the number of files written (directories not counted).

### 4.3 Entry-name safety rules (shared with letter C)

1. Non-empty, no NUL, no `\` (zip names use `/`; a backslash is treated as hostile).
2. Not absolute: no leading `/`, no drive prefix `X:`.
3. After `fs::pathNormalize`, no component is `..`.
4. (extract only) `fs::isWithin(directory, fs::pathJoin(directory, name))` → TRUE.

Violations → `ErrInvalidPath`, message `zip: unsafe entry name "<name>"`.

## Phases

> **NOTE — keep the checkboxes current as you go** (same rules as plan-139-A).

### Phase 1 — writer

- [x] `packages/zip/src/writer.mfb`: `Builder`, §4.1; `create`/`addFile`/`addText`/
      `addDirectory`/`finish` exported from `lib.mfb` with DOC comments and the planned defaults.
      **Rewritten once for performance** — the obvious shape was quadratic; see Corrections.
- [x] Tests `src/test_writer.mfb` — 18 cases, all passing. Round trip through both overloads;
      determinism (the same calls twice produce identical bytes); empty archive is exactly 22
      bytes; directory entries; deflated vs stored chosen by which is smaller; `store = TRUE`
      honoured; empty file stored; timestamps round-tripping through the DOS fields including the
      two-second grain; modes; a non-ASCII name; duplicate names allowed with `find` returning the
      first; and every §4.3 unsafe name refused at ADD time.
- [x] ZIP64 emission test, two cases in `test_writer.mfb`: **65536 entries** (one past what the
      16-bit EOCD count can express) emits the ZIP64 records and reads back through `zip::open`
      with the first and last entry findable; **65535 entries** emits no ZIP64 locator, so an
      archive that fits a classic zip stays one. The entry list is built as a bare local and
      handed to `finishBuilder` once, rather than through 65536 `add*` calls — the public API
      copies the builder per call and a 65536-entry run through it was killed by the host after
      exhausting memory (see Corrections). This tests exactly the code under test, the ZIP64
      trailer emission.
      **Externally validated**: the 65536-entry archive (6,269,342 bytes) was written out once and
      checked — `unzip -tq` → "No errors detected"; Python `zipfile` → 65536 entries, first
      `e1.txt`, last `e65536.txt`, `testzip()` → `None`; and both ZIP64 signatures present
      (`PK\x06\x06` and `PK\x06\x07` found in the bytes).
- [x] External check, from `/tmp/zipw-probe` (a probe, not in the tree), writing five archives to
      `/tmp/zipw/`. **`unzip -tq`**: "No errors detected" for `simple`, `stored`, `unicode` and
      `stamped`; "zipfile is empty" for `empty.zip`, which is the correct report for an archive
      with no entries, not a failure. **Python `zipfile`**: `testzip()` returned `None` (no bad
      entry) for all five, and every entry read back. Two decisions Python confirms independently:
      `stamped.zip`'s `date_time` reads back as exactly `(2026, 9, 19, 12, 30, 0)`, and
      `unicode.zip`'s names decode as `['café/naïve.txt', '日本語.txt']`. `simple.zip` shows the
      method choice working: `readme.txt` stored at 27 bytes, `data/squish.txt` deflated 6000 → 42.

Acceptance: writer output is valid to three readers. **Met** — our reader, `unzip`, and Python
`zipfile`, including the ZIP64 case.
  Check: `target/release/mfb test packages/zip` → `Tests: 76  Pass: 76  Fail: 0`; `unzip -tq` →
  "No errors detected" for every non-empty archive ("zipfile is empty" for the empty one, which is
  correct); Python `testzip()` → `None` for all.
Commit: 82a39e0cf, 6f30dec15 (the ZIP64 case landed with Phase 2)

### Phase 2 — extractTo

- [x] `packages/zip/src/extract.mfb`: §4.2, §4.3; `extractTo` exported from `lib.mfb` with a DOC
      comment stating the safety rules, the limit and the fact that permissions and timestamps are
      not restored. Stored entries copy in 1 MiB chunks with a running CRC, so extracting a huge
      entry from a file-backed archive holds a chunk rather than the entry.
- [x] Tests `src/test_extract.mfb` — 12 cases, all passing. Files and directories written with
      the right bytes from BOTH sources, returning the file count (directories not counted); empty
      archive writes nothing; a missing target → `ErrNotFound`. Every traversal case is refused
      with `ErrInvalidPath` **and asserts the target directory is still empty**: `../bb.txt`,
      `a/../../cc.txt`, `/tmp/x.txt`, `a\\b.txt`, `C:evil.txt`, and a two-entry archive whose
      SAFE entry comes first (so a lazily-validating extractor would already have written it).
      Over `maxTotalBytes` → `ErrTooLarge` with nothing written; a corrupted stored entry →
      `ErrorChecksum` with the partial file deleted. The hostile archives are assembled by patching
      raw bytes, because this package's own writer refuses such names. ~~pre-existing symlink~~ —
      moot: `fs` has no symlink-creation function (`grep -h -o 'name: "[a-zA-Z]*"'
      src/codegen/builtins/fs/func_*.rs | grep -i link` → no matches), so the case cannot be set
      up from MFBASIC. The defence it tests is still implemented and is what `fs::isWithin` is
      called for; letter D's corpus can stage one from Python.

Acceptance: safe extraction and all-or-nothing refusal. **Met** — every refusal case asserts the
target directory is still empty afterwards.
  Check: `target/release/mfb test packages/zip` → `Tests: 74  Pass: 74  Fail: 0`.
Commit: 6f30dec15

### Phase 3 — README and doc.html

- [x] `packages/zip/README.md` — the two `open` forms and why the memory one needs a typed
      binding; what a file-backed archive keeps in memory, in developer terms; the reading and
      writing surfaces; the entry-count cost of the writer, stated plainly; extraction safety; an
      explicit "what this package does not do"; the error table; names and character sets;
      Testing. The banned-vocabulary grep (widened to every term `.ai/man-content.md` lists, not
      just the seven in the plan's line) exits 1 with no matches.
- [x] `packages/zip/doc.html`, generated by **`mfb doc packages/zip --out packages/zip/doc.html`**
      — recorded here as the plan asked. The cited precedent does not exist: `packages/jwt` has no
      `doc.html` and neither does any other package (`find . -name doc.html` → nothing), and the
      grep the plan suggested finds no generator. See Corrections; this also uncovered that the
      package had no real documentation at all.
- [x] Every README example compiled and run from `/tmp/zipreadme` against `packages/zip/zip.mfp`.
      Output: the listing example prints `notes.txt (21 bytes)`, `images/ (0 bytes)`,
      `images/caption.txt (9 bytes)`; both `open` forms report 3 entries; the `maxBytes` example
      reads 21 bytes; the writing example produces a 321-byte archive; `extractTo` returns 2 and
      the extracted `notes.txt` reads back as written. Exit 0.

Acceptance: README examples run and print what the README says. **Met** — all five ran, exit 0.
  Check: `mfb build /tmp/zipreadme && /tmp/zipreadme/build/zipreadme.out` → the output above.
Commit: PENDING3

## Validation Plan

- Tests: `test_writer.mfb`, `test_extract.mfb`; external `unzip -t` and `zipfile.testzip`.
- Coverage check: every new export appears in a `TCASE` (same grep as plan-139-A).
- Runtime proof: Phase 1 external readers; Phase 3 README probes.
- Doc sync: README.md, doc.html, DOC comments.
- Final gate: plan-139-D.

## Open Decisions

- `finishTo(builder, RES file AS fs::File)` streaming writer — not in plan-139 (recommended: land when
  a user needs archives bigger than memory; it requires a Builder that writes as it goes, a different
  type) vs. add now.
- Duplicate names in the writer — allowed (recommended; format permits, readers handle it) vs.
  `ErrAlreadyExists`.

## Corrections

### 2026-09-19 — the package had no real documentation, and the cited precedent does not exist

References names "`packages/jwt/README.md`, `packages/jwt/doc.html` — README/doc.html precedent".
**There is no `packages/jwt/doc.html`**, and no `doc.html` anywhere in the tree
(`find . -name doc.html -not -path './target/*'` → nothing). The generator the plan asked to be
found by grepping those files is `mfb doc <path> [--out <file>]` (`src/cli/doc.rs`), and there is
also `mfb pkg doc` that renders from a compiled `.mfp`.

Running it exposed a bigger gap. The first `mfb doc packages/zip` produced a page reading **"No
documentation is available."** Letter A's Validation Plan asked for "DOC comments on every export in
`lib.mfb`", and what had been written were ordinary `'` comments — which the compiler does not
read and a compiled `.mfp` does not carry. `mfb spec language documentation` specifies a distinct
`DOC … END DOC` block that is validated against the declaration it names and persisted into the
package binary; without one, an importer of `zip.mfp` gets nothing, because a package ships without
source.

`packages/zip/src/doc.mfb` now carries proper DOC blocks: a `PACKAGE` block, every public type with
`PROP` per field, every public function with `ARG`/`RET`/`ERROR`/`EXAMPLE`, `GROUP` headings for
Reading and Writing, and `DOC INTERNAL` on the four types that are exported only so importers can
name a field type. The rendered page went from 4,753 bytes of "no documentation" to 35,030 bytes
with sections `Types`, `Reading`, `Writing` and `Internal — not part of the public API`.

One spelling to remember: a DOC header selects an overload by parameter type, and the type must be
written the way `ParameterType::name()` renders it — **`FUNC open(fs.File)`**, with a dot. Both
`fs::File` and a bare `File` are rejected with `DOC_OVERLOAD_UNRESOLVED`.

### 2026-09-19 — internal helpers were leaking into the package's public API

While writing the DOC blocks it became clear that `readAt`, `sizeOf`, `u16le`/`u32le`/`u64le`,
`putU16`/`putU32`/`putU64`/`putBytes`, `cp437Decode`, `parseArchive`, `readEntry`, `readEntryText`
and the five `*Builder`/`*Entry` writer primitives were all declared `EXPORT`, which put every one
of them in the compiled `.mfp`'s public API.

That was a misreading of the visibility levels. `mfb spec language modules-and-packages`: `PUBLIC`
(the default) is "visible to all files in the same package, hidden from importers", while `EXPORT`
"is the flag that writes a symbol into the compiled `.mfp` public API". Cross-file use inside a
package needs only the default. All eighteen are now plain `PUBLIC`, and the package's exported
surface is exactly what the plan specifies. The 76 tests pass unchanged, which is the point: they
live inside the package and never needed the wider visibility either.

### 2026-09-19 — the writer as designed was quadratic; §3's Builder is reshaped

§3 specified a `Builder` holding `entries` **and** `body`, with each `add*` appending to both. Built
that way it was unusable past a few thousand entries. Measured with `/tmp/zipgrow` (a probe, not in
the tree), total build time by entry count:

| entries | first design | after the rewrite |
|---|---|---|
| 500 | 1.4 s | 0.045 s |
| 1000 | 5.7 s | 0.118 s |
| 2000 | 25.2 s | 0.442 s |
| 4000 | 165.5 s | 1.86 s |

The 4000-entry case is **89× faster**, and the extrapolated 65536-entry archive went from hours to
minutes.

**Root cause, measured rather than guessed.** Four probes in `/tmp/zipappend`, timing one append at
list lengths 0 / 4000 / 16000 / 64000, in nanoseconds:

| shape | 0 | 4000 | 16000 | 64000 | verdict |
|---|---|---|---|---|---|
| `out = collections::append(out, b)` on a bare local | 28 | 12 | 25 | 10 | **O(1)** |
| `box = WITH box { bytes := append(box.bytes, b) }` | 17 | 18 | 37 | 13 | **O(1)** |
| `box = Box[append(box.bytes, b)]` (positional rebuild) | 871 | 4288 | 15970 | 78086 | O(n) |
| `MUT out = box.bytes` then append then `WITH` | 1563 | 8897 | 30679 | 137402 | O(n) |
| pass the list to a FUNC and return it | 411 | 2220 | 7195 | 23639 | O(n) |

So: **a growing list is cheap to append to, and expensive to move.** Rebuilding a record around it
positionally copies it; binding it to a local copies it; handing it to a function copies it. Only an
in-place `WITH` update, with the field read inline in the update expression, leaves it alone.

The writer is now shaped around that. `finish` assembles the whole archive in ONE bare local that is
never passed to a function and never stored in a record until it is returned; the fixed-size headers
are still built by helper functions, because those lists are ~46 bytes and copying one is free. Only
the archive-sized list must not move.

### 2026-09-19 — the remaining O(n²) is the planned API, not a defect, and is documented

After the rewrite, `finish` is **linear** — 8.1, 8.0, 8.1, 8.8 µs per entry at 500/1000/2000/4000,
flat. The residual quadratic is entirely in `add`: 67, 107, 234, 493 µs per entry over the same
counts.

It is not an implementation mistake. `addFile(builder AS Builder, …) AS Builder` — the API §4.1
specifies — passes the builder **by value**, and the table above shows passing a growing collection
into a function copies it. MFBASIC v1 has no by-reference parameters: `mfb spec language functions`
offers one narrowly-scoped exception ("a lambda passed directly into a compiler-proven non-escaping
callback position … may capture an outer MUT by reference") and states plainly that "non-escaping
closures are not part of the v1 source language". No arrangement of the record's FIELDS avoids it —
appending a scalar record, a record with a String, and a record with a `List OF Byte` are all O(1)
(127/147/261 ns at length 0, 29/49/82 ns at 8000); the cost is the parameter, not the element.

The API is kept as planned, because it is the right shape for callers and the cost only matters for
archives with very many entries. The practical limit is recorded on the package page rather than
papered over: a few thousand entries is comfortable, tens of thousands is minutes. Changing this
needs either by-reference parameters in the language or a batch-add API, and neither belongs in
plan-139.

### 2026-09-19 — the 65536-entry ZIP64 test: not a probe either, but a package-internal case

The entry above said the 65535-entry case would move to a `/tmp` probe because it takes minutes.
That turned out to be optimistic: the probe was **killed by the host (exit 137)** after ~30 minutes
without producing a file. The O(n²) add path is not only slow, it allocates a copy of the
accumulated entry list per call, and 65536 entries exhausts memory before it exhausts patience.

It is now a `TCASE` in `test_writer.mfb` that builds the `List OF Pending` as a bare local —
where appends are O(1) — and calls `finishBuilder` once. This exercises precisely the code the
acceptance criterion is about (ZIP64 trailer emission and the all-ones EOCD fields) and skips only
the API path whose cost is separately measured and documented. A second case asserts the boundary
in the other direction: 65535 entries emits **no** ZIP64 locator.

The archive was written out once for external judgement: `unzip -tq` reported "No errors detected",
and Python `zipfile` read all 65536 entries with `testzip()` returning `None`, with both
`PK\x06\x06` and `PK\x06\x07` present in the bytes. So the ZIP64 write path is confirmed by two
independent readers, not only by ours.

### 2026-09-19 — the 65535-entry ZIP64 test runs as a probe, not a TCASE (superseded, see above)

Phase 1 asked for it as a test. Because of the O(n²) add path it takes minutes, which is not a unit
test. It runs instead from `/tmp/zipmany`, which builds 65536 entries (one more than the 16-bit EOCD
count field can express, so the ZIP64 records are mandatory), writes `/tmp/zipw/many.zip`, reads it
back through `zip::open` over an `fs::File`, and is checked by `unzip` and Python. This is a change
of venue, not a weakening: the assertion is the same and it is still run.


## Summary

Risk is in `extractTo` path safety, handled by an all-entries pre-pass plus an on-disk `isWithin`
check per parent. The writer is deterministic and emits ZIP64 only when required.
