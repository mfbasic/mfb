# plan-139-C: `tar` package — reader, writer, `extractTo`, README and doc.html

Last updated: 2026-09-15
Effort: large (3h–1d)
Depends on: plan-139-B (complete). Whole-feature prerequisites: plan-139-A § Prerequisites.

Adds `packages/tar` with the same API shape as `packages/zip`. Outcome: **`tar::open` over an
`fs::File` and over the same bytes list the same entries with the same contents for ustar, PAX and
GNU-long-name archives; `tar::finish` output is listed identically by `bsdtar -tvf` and Python
`tarfile`; `extractTo` applies plan-139-B §4.3's safety rules plus link-target checks.**

References:

- POSIX.1-2017 `pax` — ustar header layout and the pax extended header (`path`, `linkpath`, `size`,
  `mtime`, `uid`, `gid`, `uname`, `gname`); GNU tar manual "Basic Tar Format" (`L`/`K` long
  name/link records).
- plan-139-A §3 (source layer — copied, not shared), §4.5 (error policy);
  plan-139-B §4.3 (name safety rules).

## Prerequisites

See plan-139-A § Prerequisites (all rows must be MET), plus:

| Must be true | Command | Status |
|---|---|---|
| plan-139-B complete | every `- [ ]` in `planning/plan-139-B-*.md` ticked | NOT MET (2026-09-15) |
| `packages/tar` does not exist | `ls packages/tar` → `No such file or directory` | MET (2026-09-15) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run before you
> continue and before you stop; if you stop, report all prerequisites.

## 1. Goal

- tar reading from both sources with identical results; writer output accepted by `bsdtar` and
  `tarfile`; safe `extractTo`.

### Non-goals

- No compressed tar from a file handle. `.tar.gz` works in memory by composition:
  `tar::open(compress::gzipDecode(bytes, limit))` / `compress::gzipEncode(tar::finish(b))`. A
  file-backed `.tar.gz` needs streaming gunzip, which `compress::` does not have.
- Symlink/hardlink/device/FIFO entries are **listed and readable as metadata**; `read` of a non-file
  entry → `ErrUnsupported`. `extractTo` creation of links: see Open Decisions (fs has no symlink
  creation: `grep -h -o 'name: "[a-zA-Z]*"' src/codegen/builtins/fs/func_*.rs | grep -i link` →
  none).
- No sparse files (`S`, PAX `GNU.sparse.*`) → `ErrUnsupported` on `read`; still listed.
- No permission/owner restoration on extract (same reason as plan-139-B).

## 2. Current State

After plan-139-B: `packages/zip` complete, with `source.mfb`/`bytes.mfb` and the §4.3 name rules in
`packages/zip/src/extract.mfb`. `packages/tar` absent.

## 3. Design Overview

Copy `source.mfb` from `packages/zip` verbatim (only the error-message prefix differs). Headers are
walked sequentially with `readAt(source, offset, 512)`; data is **skipped** by offset arithmetic, so
opening a file-backed tar reads 512 bytes per entry plus PAX/GNU records, never the file contents.
Octal and base-256 (GNU, high bit set) numeric fields both parsed. Header checksum verified
(sum of header bytes with the checksum field as spaces; accept both unsigned and signed sums, as GNU
tar does) → mismatch `tar::ErrorChecksum = 93160001`.

Types:

```
EXPORT TYPE Entry
  name AS String, kind AS Integer, isDirectory AS Boolean, size AS Integer, mode AS Integer,
  modifiedSeconds AS Integer, uid AS Integer, gid AS Integer, user AS String, group AS String,
  linkTarget AS String, dataOffset AS Integer
EXPORT TYPE Archive   source AS Source, entries AS List OF Entry
EXPORT TYPE Builder   body AS List OF Byte
```

Reader precedence per entry: PAX `x` record overrides GNU `L`/`K`, which override ustar
`prefix/name`; PAX `g` (global) values apply to all following entries until replaced. PAX/GNU
record bodies > 1048576 bytes → `ErrTooLarge`. End: two consecutive zero blocks, or end of source
exactly on a block boundary (accepted, as GNU tar does); a partial block → `ErrInvalidFormat`.

Writer: ustar headers, `magic "ustar\0" version "00"`; a PAX `x` record is emitted **only** when the
name doesn't fit `prefix(155)/name(100)`, link target > 100 bytes, size > 8^11−1, or a non-ASCII
name; ends with two zero blocks. `finish` output is padded to a multiple of 512 (no 10240 record
padding; both oracles accept it — verified in Phase 2).

`extractTo`: plan-139-B §4.3 rules 1–4 per entry; directories and regular files written
(regular-file data copied in 1 MiB `readAt` chunks for both sources); link entries per Open Decision.
All entries validated before any write.

## Phases

> **NOTE — keep the checkboxes current as you go** (same rules as plan-139-A).

### Phase 1 — reader

- [ ] `packages/tar/project.json` (name `tar`, version `0.1.0`), `src/source.mfb` (copied),
      `src/header.mfb` (octal/base-256, checksum, ustar/PAX/GNU), `src/lib.mfb`: `open` ×2,
      `entries`, `has`, `find`, `read`, `readText`, constants `KindFile = 0`, `KindDirectory = 1`,
      `KindSymlink = 2`, `KindHardlink = 3`, `KindOther = 4`, `ErrorChecksum = 93160001`.
- [ ] Fixtures from `packages/tar/oracle/fixtures.py` (Python `tarfile` with `USTAR_FORMAT`,
      `GNU_FORMAT`, `PAX_FORMAT`; a 200-byte name; a non-ASCII name; a symlink; a hardlink;
      a >100-byte link target) plus one `bsdtar -cf` archive, embedded in `src/test_fixtures.mfb`.
- [ ] Tests `src/test_read.mfb`: every fixture, both sources — entry fields equal the generator's
      printed `TarInfo` values, contents equal; bad checksum → `ErrorChecksum`; truncated data →
      `ErrInvalidFormat`; `read` of symlink → `ErrUnsupported`; `maxBytes` below size → `ErrTooLarge`.
- [ ] Audit: `grep -n -E 'readAllBytes|readAll\(|readBytes\(' packages/tar/src/*.mfb` → no non-test
      match.

Acceptance: all three header formats read identically from both sources.
  Check: `target/release/mfb test packages/tar` → pass; audit grep → none (est. 1 min).
Commit: —

### Phase 2 — writer

- [ ] `src/writer.mfb`: `create`, `addFile(builder, name, data, mode = 420, modifiedSeconds = 0)`,
      `addText(builder, name, text, mode = 420, modifiedSeconds = 0)`,
      `addDirectory(builder, name, mode = 493, modifiedSeconds = 0)`,
      `addSymlink(builder, name, target, modifiedSeconds = 0)`, `finish(builder)`; names validated by
      plan-139-B §4.3 rules 1–3 (`ErrInvalidPath`).
- [ ] Tests `src/test_writer.mfb`: round-trip through `tar::open` (both sources); a 300-byte name
      forces PAX; plain short names produce no PAX record (header count = entries).
- [ ] External check from a `/tmp` probe: `bsdtar -tvf` and
      `python3 -c "import tarfile,sys;[print(m.name,m.size) for m in tarfile.open(sys.argv[1])]"` on
      each written archive, compared to the builder's inputs.

Acceptance: writer output lists identically in `bsdtar` and `tarfile`.
  Check: `target/release/mfb test packages/tar` → pass; the two listings match the inputs (est. 3 min).
Commit: —

### Phase 3 — extractTo, README, doc.html

- [ ] `src/extract.mfb`: as §3, exported with DOC comments.
- [ ] Tests `src/test_extract.mfb`: same traversal cases as plan-139-B Phase 2; hardlink/symlink
      targets outside the directory refused before any write.
- [ ] `packages/tar/README.md` + `doc.html` (same structure and vocabulary check as plan-139-B
      Phase 3), including the `.tar.gz` composition example; examples run from `/tmp` probes.

Acceptance: safe extraction; README examples run.
  Check: `target/release/mfb test packages/tar` → pass; README probes print documented output
  (est. 5 min).
Commit: —

## Validation Plan

- Tests: `test_read.mfb`, `test_writer.mfb`, `test_extract.mfb`; external `bsdtar`/`tarfile`.
- Coverage check: every export of `packages/tar/src/lib.mfb` appears in a `TCASE`.
- Runtime proof: plan-139-D.
- Doc sync: README.md, doc.html, DOC comments.
- Final gate: plan-139-D.

## Open Decisions

- Link entries during `extractTo` — refuse the whole extraction with `ErrUnsupported` before writing
  anything when any symlink/hardlink is present (recommended: silently skipping produces a wrong tree;
  `fs` cannot create links) vs. skip and return a list of skipped names (changes the return type away
  from zip's) vs. a prerequisite fs plan for `fs::createSymlink`.
- Hardlink to an earlier regular entry — could be materialised as a copy of that entry's bytes;
  recommended to treat as the decision above for consistency.

## Corrections

## Summary

Low format risk (fixed 512-byte blocks) but three header dialects; checked against two independent
readers. File-backed open reads only headers.
