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

See plan-139-A § Prerequisites (all rows must be MET), plus:

| Must be true | Command | Status |
|---|---|---|
| plan-139-A complete | every `- [ ]` in `planning/plan-139-A-*.md` ticked; `target/release/mfb test packages/zip` → pass | NOT MET (re-verified 2026-09-15: `grep -c '^- \[ \]' planning/plan-139-A-*.md` → 13 unticked; `ls packages/zip` → `No such file or directory`. plan-139-A is itself blocked on its own two `fs` prerequisite rows.) |

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

- [ ] `packages/zip/src/writer.mfb`: `Builder`, §4.1; exports in `lib.mfb` with DOC comments.
- [ ] Tests `src/test_writer.mfb`: build → `zip::open` (bytes and temp-file) → names, sizes, crc,
      contents equal; empty archive; directory entry; stored vs deflated choice; incompressible data
      stored; comment round-trip; unsafe names rejected.
- [ ] ZIP64 emission test: construct a builder with a synthetic entry count of 65535 empty files
      (measures only headers) → `zip::open` reads 65535 entries.
- [ ] External check: write each test archive to `/tmp/zipw/*.zip` from a probe (`/tmp`, not in tree),
      then `for f in /tmp/zipw/*.zip; do unzip -tq "$f"; done` and
      `python3 -c "import zipfile,sys,glob;[sys.exit(1) for f in glob.glob('/tmp/zipw/*.zip') if zipfile.ZipFile(f).testzip()]"`.

Acceptance: writer output is valid to three readers.
  Check: `target/release/mfb test packages/zip` → pass; `unzip -tq` → `No errors detected` per file;
  the Python one-liner exits 0 (est. 3 min).
Commit: —

### Phase 2 — extractTo

- [ ] `packages/zip/src/extract.mfb`: §4.2, §4.3; export in `lib.mfb`.
- [ ] Tests `src/test_extract.mfb` (into `fs::createTempFile`'s directory + a fresh subdirectory):
      files/dirs extracted with right bytes from both sources; returns file count; `../x`, `/x`,
      `a/../../x`, `C:x`, `a\\b` each → `ErrInvalidPath` and the directory is still empty;
      pre-existing symlink `dir/link → /tmp` then entry `link/x` → `ErrInvalidPath`; total over
      `maxTotalBytes` → `ErrTooLarge`, directory empty; corrupted stored entry → `ErrorChecksum`,
      no partial file left.

Acceptance: safe extraction and all-or-nothing refusal.
  Check: `target/release/mfb test packages/zip` → all `extract` cases pass (est. 1 min).
Commit: —

### Phase 3 — README and doc.html

- [ ] `packages/zip/README.md` (mirror `packages/jwt/README.md` sections): what it reads/writes, the
      two `open` forms, memory model in developer terms ("reading from a File keeps only the list of
      entries and the entry you read"), limits and defaults, extraction safety, what is not supported
      (encryption, methods other than stored/deflate, permissions on extract), Testing.
      Vocabulary: `.ai/man-content.md` bans apply — no borrow/ownership/heap/etc.;
      check `grep -n -i -E 'borrow|ownership|heap|lifetime|allocate|refcount|dangling' packages/zip/README.md`
      → none.
- [ ] `packages/zip/doc.html` generated the way `packages/jwt/doc.html` is (find the generator with
      `grep -rn doc.html packages/jwt/README.md scripts/README.md` and record it here).
- [ ] Every README example compiled and run from a `/tmp` probe against `packages/zip/zip.mfp`.

Acceptance: README examples run and print what the README says.
  Check: run each example probe → output equals the README's stated output (est. 5 min).
Commit: —

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

## Summary

Risk is in `extractTo` path safety, handled by an all-entries pre-pass plus an on-disk `isWithin`
check per parent. The writer is deterministic and emits ZIP64 only when required.
