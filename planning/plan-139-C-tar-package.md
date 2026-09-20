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

See plan-139-A § Prerequisites — all rows must be MET, and its two `fs` rows are delivered by
**plan-139-E**, which gates the whole chain `E -> A -> B -> C -> D`. Plus:

| Must be true | Command | Status |
|---|---|---|
| plan-139-B complete | every `- [ ]` in `planning/plan-139-B-*.md` ticked | **MET** (2026-09-19: plan-139-B has 0 unticked boxes and is archived to `planning/completed/`; `target/release/mfb test packages/zip` → `Tests: 76  Pass: 76  Fail: 0`) |
| `packages/tar` does not exist | `ls packages/tar` → `No such file or directory` | MET (re-verified 2026-09-19: `No such file or directory`) |

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

- [x] `packages/tar/project.json`; `src/source.mfb` copied from `packages/zip` with only the
      error-message prefix changed; `src/header.mfb` (octal and GNU base-256 numbers, the
      signed/unsigned checksum, ustar `prefix`/`name`, PAX record parsing, lenient name decoding);
      `src/lib.mfb` with `open` ×2, `entries`, `has`, `find`, `read`, `readText`, the five `Kind*`
      constants and `ErrorChecksum = 93160001`. The walk steps over each entry's data by
      arithmetic, so listing a file-backed archive reads one 512-byte header per entry.
- [x] `packages/tar/oracle/fixtures.py` generates five fixtures — `ustar`, `gnu`, `pax`, `links`
      and `bsdtar` — into `src/test_fixtures.mfb` and `oracle/corpus/*.tar`. The 200-byte name and
      the 117-byte link target are shared constants, so the `gnu` and `pax` fixtures carry the
      **same** name stored two different ways; a non-ASCII name, a symlink and a hardlink are
      covered. Bytes are embedded Base64, for the same reason as the zip fixtures.
- [x] Tests `src/test_read.mfb` — 18 cases, all passing. Every fixture is compared field by field
      against the oracle (name, kind, isDirectory, size, mode, mtime, uid, gid, user, group,
      linkTarget and contents) through **both** overloads, plus a case asserting the two sources
      return identical entry bytes. Named cases pin the GNU `L` and `K` records, the PAX `x`
      record, a non-ASCII PAX name, link listing, and the `bsdtar`-written archive — and one case
      asserts **GNU and PAX agree on the same 200-byte name**, which is the reader's whole point.
      Refusals: reading a symlink, a hardlink or a directory → `ErrUnsupported`; `maxBytes` below
      the size → `ErrTooLarge` with the archive still usable; a missing name → `ErrNotFound`; a
      corrupted header → `ErrorChecksum`; an over-large size field → refused; an archive ending
      mid-block → `ErrInvalidFormat`.
- [x] Audit: the grep over executable lines of the non-test sources exits 1 with no matches, and
      the positive form is stronger — the package's executable code calls exactly `fs::readBytesAt`
      once and `fs::size` once, and nothing else in `fs`.

Acceptance: all three header formats read identically from both sources. **Met** — and the
`gnu`/`pax` agreement case proves it directly rather than by inference.
  Check: `target/release/mfb test packages/tar` → `Tests: 18  Pass: 18  Fail: 0`; audit grep →
  exit 1, no match.
Commit: 49eb54af5

### Phase 2 — writer

- [x] `src/writer.mfb` with all five, at the planned defaults, names validated by plan-139-B §4.3
      rules 1–3. Built from the start around plan-139-B's measured cost model: the body is
      assembled once in `finish` in a single bare local, and `add*` accumulates small records
      through a `WITH` update, so `finish` is linear.
- [x] Tests `src/test_writer.mfb` — 13 cases. Round trip through both overloads; determinism;
      an empty archive is exactly two zero blocks; data padded to a block boundary; mode and
      mtime round-tripping. The PAX decisions are pinned in both directions: a 300-byte name, a
      non-ASCII name and a 140-byte link target each produce **2** header blocks, while three
      plain short names produce **3** — header count equal to entry count, so nothing extra was
      written — and a 120-byte name that splits at a `/` into fields that fit also produces just
      **1**. Refusals cover every §4.3 unsafe name plus an empty symlink target.
- [x] External check from `/tmp/tarw-probe`, six archives into `/tmp/tarw/`. **`bsdtar -tvf`**
      lists every one correctly, including the 300-byte PAX name, both Unicode names, the
      140-byte symlink target (`link -> ttt…`) and the 120-byte split name. **Python `tarfile`**
      agrees field for field with what the builder was given: `readme.txt` size 27 mode 644 mtime
      1789821000; the directory `data` mode 755; `link` mode 777 with its target; the long name
      reported at 300 bytes, the long link at 140, the split name at 120; `café/naïve.txt` and
      `日本語.txt` decoded correctly; every regular entry extracted without error; the empty
      archive read as 0 entries. Exit 0.

Acceptance: writer output lists identically in `bsdtar` and `tarfile`. **Met**, including every
PAX case.
  Check: `target/release/mfb test packages/tar` → `Tests: 31  Pass: 31  Fail: 0`; both listings
  match the builder's inputs (above).
Commit: fdb2768c6

### Phase 3 — extractTo, README, doc.html

- [x] `src/extract.mfb`, exported as `extractTo` with a DOC block. Same two rules as the zip
      extractor — validate every entry before writing any byte, and check containment against the
      DISK via `fs::isWithin` on the deepest existing ancestor. Regular files copy in 1 MiB
      pieces. **Open Decision resolved as recommended:** an archive containing a symlink, hardlink
      or device node is refused entirely with `ErrUnsupported`, because `fs` cannot create any of
      them and a silently incomplete tree is worse than a refusal.
- [x] Tests `src/test_extract.mfb` — 13 cases. Files and directories written with the right
      bytes from both sources; the file count returned; empty archive writes nothing; missing
      target → `ErrNotFound`. Every traversal case refused with `ErrInvalidPath` **and the
      directory asserted still empty**: `../escape.txt`, `a/../../escape.txt`, `/tmp/escape.txt`,
      `a\\b.txt`, `C:evil.txt`, and a two-entry archive whose SAFE entry comes first. Over
      `maxTotalBytes` → `ErrTooLarge`, nothing written. Links: an archive with a symlink and one
      with a hardlink are each refused with `ErrUnsupported` before anything is written — the
      symlink case puts a safe entry first so the "nothing written" claim is tested, not assumed.
      The hostile archives are built by patching header bytes and repairing the checksum, since
      this package's writer refuses such names.
- [x] `packages/tar/README.md` — why listing is cheap, the three dialects and their precedence,
      the two numeric spellings, the reading and writing surfaces, the PAX-only-when-needed rule,
      the entry-count cost, extraction safety and the link refusal with its reasoning, the
      `.tar.gz` composition, and the error table. Banned-vocabulary grep → exit 1, no matches.
      `packages/tar/src/doc.mfb` carries proper DOC blocks (PACKAGE, every public type with PROP,
      every function with ARG/RET/ERROR/EXAMPLE, GROUP headings, `DOC INTERNAL` on the four
      internal types); `mfb doc packages/tar --out packages/tar/doc.html` renders 34,337 bytes
      with sections Types / Reading / Writing / Internal. Every README example compiled and ran
      from `/tmp/tarreadme`: the listing prints `notes.txt (21 bytes)`, `data/ (0 bytes)`,
      `data/inner.txt (6 bytes)`; writing produces 4096 bytes; `extractTo` returns 2 and the text
      reads back; the `.tar.gz` round trip reports 1 entry and `hello`. Exit 0.

Acceptance: safe extraction; README examples run. **Met.**
  Check: `target/release/mfb test packages/tar` → `Tests: 44  Pass: 44  Fail: 0`; the README probe
  prints the documented output and exits 0.
Commit: f72611cab

## Validation Plan

- Tests: `test_read.mfb`, `test_writer.mfb`, `test_extract.mfb`; external `bsdtar`/`tarfile`.
- Coverage check: **done** — `open` (both overloads), `entries`, `has`, `find`, `read`,
  `readText`, `create`, `addFile`, `addText`, `addDirectory`, `addSymlink`, `finish` and
  `extractTo` are each called from at least one `TCASE`, across `test_read.mfb` (18),
  `test_writer.mfb` (13) and `test_extract.mfb` (13).
- Runtime proof: plan-139-D.
- Doc sync: README.md, doc.html, DOC comments.
- Final gate: plan-139-D.

## Open Decisions

- Link entries during `extractTo` — **resolved 2026-09-19 as the recommended option**: refuse the
  whole extraction with `ErrUnsupported`, in the pre-pass, before anything is written. Skipping
  would hand back a tree that looks complete and is not, and the failure would surface far from
  the extraction that caused it. `fs::createSymlink` remains a possible future prerequisite plan;
  it is not one plan-139 needs.
- Hardlink to an earlier regular entry — **resolved 2026-09-19 as recommended**: treated the same
  as a symlink and refused. Materialising it as a copy would silently turn one file into two, which
  is a different archive from the one the caller was given.

## Corrections

### 2026-09-19 — GNU's ustar magic is `"ustar "`, not `"ustar\0"`, and getting it wrong is silent

The first reader compared the 6-byte magic field against `"ustar"` and `"ustar  "`. POSIX writes
`"ustar\0"`, which reads back as `"ustar"` because the text stops at the NUL — but **GNU writes
`"ustar "`**, six bytes with a trailing space and no NUL, which matched neither.

The symptom was not "GNU archives are rejected". `hasUstarMagic` gates only the `prefix`, `uname`
and `gname` fields, so a GNU archive still read correctly except that every entry's `user` and
`group` came back empty — caught by the oracle comparison as
`gnu entry 0: user/group disagree: /`, and invisible to any test that only checked names and
contents. The check now compares the trimmed field against `"ustar"`, which accepts both spellings.

Worth recording because it is the exact failure mode the oracle fixtures exist to catch: a
plausible-looking archive that reads fine until you compare a field nobody thought to assert.

### 2026-09-19 — truncating a tar from the end does not truncate the archive

The planned "truncated data → `ErrInvalidFormat`" case cut 100 bytes off the end of a fixture and
expected a refusal. It does not refuse, correctly: a tar ends with two zero blocks and then padding
to a record boundary, so removing 100 trailing bytes removes padding only. The walk still meets the
two zero blocks and stops there, exactly as it should.

The case now truncates to 700 bytes — inside the first entry's data, before any terminator — which
leaves the walk with a partial block and no end marker. That is the condition the criterion was
about, and it raises `ErrInvalidFormat`. A strengthening, not a weakening: the original cut tested
nothing.


## Summary

Low format risk (fixed 512-byte blocks) but three header dialects; checked against two independent
readers. File-backed open reads only headers.
