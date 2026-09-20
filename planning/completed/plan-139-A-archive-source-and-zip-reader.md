# plan-139-A: `zip` / `tar` packages — shared source layer and the zip reader

Last updated: 2026-09-15
Overall Effort: x-large (1d–3d) — the whole plan-139 feature: two pure-MFBASIC (non-builtin)
packages, `packages/zip` and `packages/tar`, that read archives from either an open `fs::File`
(never loading the whole file) or a `List OF Byte`, through one identical `open` overload pair,
and write archives to a `List OF Byte`, checked against Python's `zipfile`/`tarfile`, `/usr/bin/zip`
and `bsdtar` as oracles
Effort: large (3h–1d)
Depends on: **plan-139-E** (complete) — the two `fs` builtins this letter's source layer reads through

plan-139 adds `packages/zip` and `packages/tar`. The behavioral outcome of the whole feature:
**for any archive, `zip::open`/`tar::open` over an `fs::File` and over the same file's bytes as a
`List OF Byte` produce identical entry lists and identical entry contents, and both agree with the
oracle (Python `zipfile`/`tarfile`) or both refuse; opening a file-backed archive and reading one
entry holds memory proportional to that entry and the archive's metadata, never to the file.**

This letter (A) proves the source layer (the two `open` overloads over one internal `readAt`) and
builds the complete zip reader: `open` ×2, `entries`, `comment`, `has`, `find`, `read`, `readText`.

| Letter | Delivers | Effort |
|---|---|---|
| E | `fs::size`, `fs::readBytesAt` — the two builtins this letter is gated on | medium |
| **A** (this) | package skeleton, source layer, zip reader (ZIP64, CP437 names, CRC, limits) | large |
| B | zip writer (`create`/`add*`/`finish`), `zip::extractTo`, README/doc.html | large |
| C | tar reader + writer + `tar::extractTo` (ustar, PAX, GNU long names), README/doc.html | large |
| D | oracle probes, corpus, fuzz, file-vs-memory differential, RSS proof, final gate, archive | medium |

References:

- `planning/todo.md` — row 5 "zip/tar archives | Pure MFB package" and "# Proposed API:
  `compress::`, `zip::`, `tar::`" (the starting API this plan revises; see §3 for what changed).
- PKWARE APPNOTE.TXT 6.3.10 — §4.3.7 local header, §4.3.12 central directory, §4.3.16 EOCD,
  §4.3.14/§4.3.15 ZIP64 EOCD record/locator, §4.4.4 general-purpose flag bit 11 (UTF-8), Appendix D
  (CP437).
- `mfb spec language resource-management` (`src/docs/spec/language/15_resource-management.md`) —
  a record field may hold `RES fs::File` (§15, "A record field may hold a resource"); aliasing and
  `ErrResourceClosed` semantics.
- `mfb spec language functions` (`src/docs/spec/language/06_functions.md`, "Overloading") —
  parameter-type overloads resolve across the package boundary.
- `mfb man compress` — `compress::inflate`, `compress::deflate`, `compress::crc32` (whole-buffer).
- `packages/jwt/` and `packages/yaml/` — package layout, `EXPORT LET Error* AS Integer = 931NNNNN`,
  `TESTING` blocks run by `mfb test packages/<name>`, oracle `probe/` layout.
- `bugs/bug-631-imported-overload-ambiguous-on-imported-record-field-argument.md` — imported
  overload resolution gap; see Verified properties for why it does not gate this plan.

## Prerequisites

These gate the whole plan-139 feature; letters B–D point here.

| Must be true | Command | Status |
|---|---|---|
| `fs` exposes a size query on an open handle: `fs::size(file AS fs::File) AS Integer` | `target/release/mfb man fs size` → a page whose Declaration is that signature | **MET** (plan-139-E Phase 1, landed `f177b5ff7`; verified 2026-09-19: `target/release/mfb man fs size` prints Declaration `fs::size(file AS fs::File) AS Integer` — exactly this row's signature. Registry probe `grep -h -o 'name: "[a-zA-Z]*"' src/codegen/builtins/fs/func_*.rs | grep -i size` → `name: "size"`.) |
| `fs` exposes a positional read on an open handle that does not require reading from the start: `fs::readBytesAt(file AS fs::File, offset AS Integer, count AS Integer) AS List OF Byte` (returns fewer than `count` bytes only at end of file) | `target/release/mfb man fs readBytesAt` → a page whose Declaration is that signature | **MET** (plan-139-E Phase 2, landed `f177b5ff7`; verified 2026-09-19: `target/release/mfb man fs readBytesAt` prints Declaration `fs::readBytesAt(file AS fs::File, offset AS Integer, count AS Integer) AS List OF Byte` — exactly this row's signature. The short-read contract is measured by `tests/rt-behavior/fs/func_fs_readBytesAt_valid`: `count`=100 at offset 18 of a 23-byte file returns 5 bytes, `offset`>=EOF returns an empty list, and neither raises.) |
| `packages/zip` and `packages/tar` do not exist yet | `ls packages/zip packages/tar` → `No such file or directory` ×2 | MET (re-verified 2026-09-19: both `No such file or directory`) |
| Package parameter defaults work for importers (plan-136-B) | `ls planning/completed/plan-136-B-package-parameter-defaults.md` → present | MET (re-verified 2026-09-19: file present) |
| Release compiler is current with HEAD | `cargo build --release` → `Finished` | MET (re-verified 2026-09-19 in `.claude/worktrees/P-139` at HEAD `23c06f46b`: `Finished \`release\` profile [optimized] target(s) in 1m 51s`) |
| Python 3 with `zipfile`/`tarfile`, `/usr/bin/zip`, `bsdtar` (oracle, letter D; corpus, every letter) | `python3 -c "import zipfile,tarfile"; which zip bsdtar` → no error, two paths | MET (re-verified 2026-09-19: `python3 -c "import zipfile,tarfile"` → ok; `which zip bsdtar` → `/usr/bin/zip`, `/usr/bin/bsdtar`) |

The two `fs` rows are **builtin** work (a new registry function each, codegen, man pages, spec).
They were originally deferred to "their own plan"; no such plan was ever written, so on 2026-09-19
they were absorbed into plan-139 as **letter E**, which gates this letter (see Corrections). Until
letter E lands, this letter cannot start, full stop. The packages will not emulate positional reads
with `readAllBytes` — that is exactly the whole-file load requirement 1 forbids. The exact
signatures above are what every letter of this plan is written against, and letter E is written to
produce exactly them; if letter E lands a different spelling, update this table and the one `readAt`
function in §4 before starting.

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify — a
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you.

## 1. Goal

- `zip::open(RES file AS fs::File)` and `zip::open(data AS List OF Byte)` both return a
  `zip::Archive`; for every corpus archive, `zip::entries` and `zip::read` of every entry are
  byte-identical between the two, and `read` verifies each entry's CRC-32.

### Non-goals (explicit constraints)

- **Not builtin.** No change under `src/` in this plan. Both packages are `kind: "package"` MFBASIC
  sources under `packages/`, built to `.mfp` and imported through a manifest `packages` entry.
- **No whole-file load on the file path.** Nothing reachable from `open(RES file)` calls
  `fs::readAllBytes`, `fs::readAll` or `fs::readBytes`. Memory held by a file-backed `Archive` is
  the parsed central directory (zip) or header list (tar); `read` additionally holds one entry.
- **No streaming decompression.** `compress::` is whole-buffer (`mfb man compress`: "every member
  takes a whole List OF Byte"). A DEFLATE entry is read, inflated and CRC-checked in memory,
  bounded by `maxBytes`. Streaming inflate is a `compress::` change and out of scope.
- **No encryption, no methods other than 0 (stored) and 8 (deflate)**, no multi-disk archives,
  no `.tar.gz`/`.tgz` from a file handle (see letter C, Open Decisions).
- **The `fs::File` stays the caller's.** `open` never closes it; the `Archive` aliases it
  (§15: a `RES` record field "owns nothing by itself"). Closing the file makes later reads fail with
  `ErrResourceClosed`.
- **No new shared error codes.** Shared conditions reuse existing `errorCode::` values; only the
  checksum mismatch is package-specific (§4.5).

## 2. Current State

- `fs::File` handle surface: `fs::open`, `openFile`, `openFileNoFollow`, `openWithin`,
  `createTempFile`, `readLine`, `readAll`, `readAllBytes`, `writeAll`, `writeAllBytes`, `eof`,
  `flush`, `setBuffered`, `isBuffered`, `close` (`target/release/mfb man fs` Description; registry
  names via `grep -h -o 'name: "[a-zA-Z]*"' src/codegen/builtins/fs/func_*.rs`). There is no seek,
  no positional read and no size — hence the prerequisites.
- `compress::` is a builtin written in MFBASIC with no system library (plan-137-A…E, archived in
  `planning/completed/`; `mfb man compress`). `compress::inflate(data, [maxBytes])` decodes raw
  DEFLATE and raises `ErrTooLarge` past `maxBytes` (default 64 MiB);
  `compress::deflate(data, [level])`; `compress::crc32(data, [running])`. The todo's
  "`ErrUnavailable` on Windows" note is obsolete: there is no zlib dependency.
- `encoding::Codepage` has no CP437 member (`target/release/mfb man encoding types | grep -o -E
  '\b(Cp|Ibm|Dos|Windows|Iso)[A-Za-z0-9_]*'` → `Ibm866`, `Iso8859_*`, `Windows*` only), so the todo's
  "decode names with `encoding::codepageDecode`" does not work; the zip package carries its own
  256-entry CP437 table (§4.4).
- Package precedent: `packages/jwt/project.json` (`kind: "package"`, `sources` root `src`,
  `role: "package"`), tests as `TESTING`/`TGROUP`/`TCASE` blocks in `src/test_*.mfb`
  (`packages/jwt/src/test_roundtrip.mfb`), run by `mfb test packages/jwt` (`packages/jwt/README.md`
  "Testing"). Error constants: `EXPORT LET ErrorExpired AS Integer = 93130001`
  (`packages/jwt/src/core.mfb`), raised with `FAIL error(ErrorExpired, "jwt: …")`.
- Records are constructed positionally, `Name[field1, field2]`
  (`examples/browser/dom/src/lib.mfb:element`); unions of records are `EXPORT UNION … END UNION`
  and matched with `MATCH x / CASE Member(v)`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `fs` registry function files | 41 | `ls src/codegen/builtins/fs/func_*.rs \| wc -l` → 41 |
| positional-read / size functions in `fs` | 0 | `grep -h -o 'name: "[a-zA-Z]*"' src/codegen/builtins/fs/func_*.rs \| grep -i -E 'size\|seek\|at"'` → none |
| `compress` members | 7 | `ls src/codegen/builtins/compress/func_*.rs \| wc -l` → 7 |
| package-specific error ranges in use | `9311` yaml, `9312` json_schema+mustache, `9313` jwt; `9314` claimed by plan-138 (xml) → `9315` zip, `9316` tar | `grep -rhn 'EXPORT LET Error[A-Za-z]* AS Integer = ' packages/*/src/*.mfb \| awk -F'= ' '{print $2}' \| cut -c1-4 \| sort \| uniq -c`; `grep -n 9314 planning/plan-138-A-*.md` |
| shared codes this plan reuses, present in the table | `ErrInvalidFormat`, `ErrUnsupported`, `ErrNotFound`, `ErrTooLarge`, `ErrInvalidPath`, `ErrResourceClosed` — 6 of 6 | `grep -o -E '\`Err[A-Za-z]+\`' src/docs/spec/diagnostics/02_error-codes.md \| sort -u` |
| existing CRC/checksum shared code | 0 | same command, `grep -i check` → none |

### Verified properties

- **The two-overload `open` returning a record whose union field holds `RES fs::File` compiles and
  runs across a package boundary.** Verified 2026-09-15 by a spike in `/tmp/archspike`: package
  `arch` exporting `TYPE MemorySource { bytes AS List OF Byte }`, `TYPE FileSource { handle AS RES
  fs::File }`, `UNION Source`, `TYPE Archive { source AS Source, names AS List OF String }`,
  `open(data AS List OF Byte) AS Archive`, `open(RES file AS fs::File) AS Archive`, and a
  `describe(archive)` that `MATCH`es the source and reads through `f.handle`; consumer built against
  `packages/arch.mfp` printed `memory 5` / `file 5`, exit 0.
- **An untyped list literal selects neither overload.** Same spike: `arch::open([1, 2, 3])` →
  `TYPE_UNKNOWN_VALUE`; a `LET raw AS List OF Byte` argument resolves. Documented as the language's
  rule (`06_functions.md` "Overloading"); the man page example must use a typed binding.
- **bug-631 does not gate this plan.** Read the bug: it bites only when an *overloaded imported*
  function is called with a field of an *imported* record. Only `open` is overloaded in either
  package; its arguments are a `List OF Byte` or `fs::File`, and no exported zip/tar record has a
  field of either type a caller would pass back to `open`. `read`, `find`, etc. are not overloaded.
- **UNVERIFIED — cost of slicing a `List OF Byte`.** The memory `readAt` must copy `count` bytes in
  O(count), not O(list length). Phase 1 measures it before the reader is built on it.
- **UNVERIFIED — `fs::readBytesAt` short-read and closed-handle behavior.** Depends on the
  prerequisite plan's contract; Phase 1 pins it with a test.

## 3. Design Overview

Each package is self-contained (no dependency between `zip` and `tar`, no third shared package):
a private `source.mfb` in each defines the `Source` union and one function

```
FUNC readAt(source AS Source, offset AS Integer, count AS Integer) AS List OF Byte
```

that dispatches to a list slice (memory) or `fs::readBytesAt` (file), plus `sizeOf(source)`. Every
parser in the package reads bytes **only** through `readAt`/`sizeOf`. That single seam is what makes
"file and memory give identical results" true by construction and what letter D's differential test
checks. Duplicating ~50 lines across two packages is deliberately preferred to a third package both
must depend on (rejected: `packages/archive-core` — a versioned dependency for 50 lines, and it would
put an internal type in two packages' public surfaces).

**API changes from `planning/todo.md`'s proposal**, decided here:

1. `open` gains the `RES file AS fs::File` overload in both packages (requirement 1).
2. `tar::read`/`readText` gain `maxBytes` (a file-backed tar entry can be arbitrarily large — the
   todo's in-memory version never needed a cap).
3. `tar::Entry` gains `isDirectory` so code written against the common subset of `Entry`
   (`name`, `isDirectory`, `size`, `mode`, `modifiedSeconds`) works for both packages.
4. Error constants: shared `errorCode::` values replace the todo's `ErrorInvalid`, `ErrorUnsupported`,
   `ErrorNotFound`, `ErrorTooLarge`, `ErrorUnsafePath`; only `ErrorChecksum` is package-specific.
5. `encoding::codepageDecode` is not used for CP437 (no such member) — package-local table.

**Correctness risk** concentrates in the central-directory/ZIP64 arithmetic (offsets read from
untrusted input driving `readAt` on a file) — every offset/length is bounds-checked against
`sizeOf(source)` before the read, and letter D fuzzes it. **Design uncertainty** concentrates in the
list-slice cost and the `readBytesAt` contract — Phase 1, before any parser exists.

**Gate class:** new packages, behavior-adding. Byte-identity of compiler output is not a gate for any
letter; no `src/` file changes, so no compiler golden may diff. The gates are `mfb test
packages/zip` and runtime probes.

## 4. Detailed Design

### 4.1 Package layout

```
packages/zip/
  project.json          kind "package", name "zip", version "0.1.0", sources root "src"
  README.md             (letter B)
  doc.html              (letter B)
  src/lib.mfb           exported API + DOC comments
  src/source.mfb        Source union, readAt, sizeOf
  src/bytes.mfb         little-endian u16/u32/u64 readers over List OF Byte
  src/cp437.mfb         256-entry CP437 → String table
  src/central.mfb       EOCD search, ZIP64 locator/record, central directory parse
  src/entry.mfb         local header parse, read/verify
  src/test_*.mfb        TESTING blocks
```

### 4.2 Types (zip)

```
EXPORT TYPE MemorySource   bytes AS List OF Byte
EXPORT TYPE FileSource     handle AS RES fs::File
EXPORT UNION Source        MemorySource | FileSource
EXPORT TYPE Entry
  name AS String, isDirectory AS Boolean, method AS Integer, size AS Integer,
  compressedSize AS Integer, crc AS Integer, modifiedSeconds AS Integer, mode AS Integer,
  comment AS String, headerOffset AS Integer
EXPORT TYPE Archive
  source AS Source, entries AS List OF Entry, comment AS String
```

`Source`, `MemorySource`, `FileSource` are exported only because an exported record's field type must
be nameable by importers; their DOC says "internal — use `open`". An `Archive` over a file is not
comparable and cannot cross threads (§15: a record carrying a resource).

### 4.3 `open`

1. `size = sizeOf(source)`; `size < 22` → `ErrInvalidFormat`.
2. `tail = readAt(source, max(0, size - 65557), min(size, 65557))` (22-byte EOCD + 65535 comment).
   Scan backwards for signature `PK\x05\x06` whose comment length lands exactly at end of file.
   None → `ErrInvalidFormat`.
3. If the 20 bytes before the EOCD are a ZIP64 locator (`PK\x06\x07`), `readAt` the ZIP64 EOCD
   record at the locator's offset (56 bytes), verify `PK\x06\x06`, take entry count, CD size, CD
   offset from it. Disk numbers ≠ 0 → `ErrUnsupported`.
4. Bounds: `cdOffset + cdSize <= size` else `ErrInvalidFormat`. `cdSize > 268435456` (256 MiB of
   metadata) → `ErrTooLarge`.
5. `cd = readAt(source, cdOffset, cdSize)`; parse `entryCount` headers (`PK\x01\x02`), applying the
   ZIP64 extra field (0x0001) for any `0xFFFFFFFF` size/offset. Name bytes: flag bit 11 set → UTF-8
   (invalid → `ErrInvalidFormat`); else CP437 table. `isDirectory` = name ends in `/`.
   `mode` = external attributes `>> 16` when "version made by" host is 3 (Unix), else 0.
   `modifiedSeconds` = DOS date/time as UTC seconds since the Unix epoch (extended-timestamp extra
   field 0x5455 overrides when present). Count mismatch → `ErrInvalidFormat`.

### 4.4 `read(archive, entry, maxBytes = 67108864)`

1. `entry.size > maxBytes` → `ErrTooLarge` (before reading anything).
2. Flag bit 0 (encrypted) → `ErrUnsupported`; method ∉ {0, 8} → `ErrUnsupported`.
3. `lh = readAt(source, entry.headerOffset, 30)`; verify `PK\x03\x04`; `dataOffset = headerOffset +
   30 + nameLen + extraLen`; `dataOffset + compressedSize <= size` else `ErrInvalidFormat`.
4. `raw = readAt(source, dataOffset, compressedSize)`. Method 0: `compressedSize ≠ size` →
   `ErrInvalidFormat`; data = raw. Method 8: `compressedSize > maxBytes + 5 * (maxBytes / 65535 + 1)`
   → `ErrTooLarge` (larger than any valid DEFLATE of `maxBytes` bytes); `data =
   compress::inflate(raw, maxBytes)`; `len(data) ≠ entry.size` → `ErrInvalidFormat`.
5. `compress::crc32(data) ≠ entry.crc` → `FAIL error(ErrorChecksum, "zip: …")`.

Sizes come from the central directory; a data descriptor (flag bit 3) is therefore never needed on
read. `readText` = `read` + strict UTF-8 decode (`ErrInvalidFormat` on invalid).

### 4.5 Errors

`zip::ErrorChecksum = 93150001` (`EXPORT LET`). Everything else: `ErrInvalidFormat` (malformed),
`ErrUnsupported` (encryption, method, multi-disk), `ErrNotFound` (`find`), `ErrTooLarge` (limits),
`ErrInvalidPath` (letter B, `extractTo`), `ErrResourceClosed` (from `fs`, propagated). Every message
is prefixed `zip: `.

## Compatibility / Format Impact

New packages only. No compiler, spec, builtin or existing-package change. The surface introduced here
(`zip::Archive`, `zip::Entry`, `zip::Source` and members, `open` ×2, `entries`, `comment`, `has`,
`find`, `read`, `readText`, `ErrorChecksum`) is the contract letters B–D build on.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as the work;
> `- [~]` for partial with what remains; moot tasks struck through with evidence, never deleted;
> fill `Commit:` the moment a phase lands. **An unticked box means NOT DONE.**

### Phase 1 — skeleton and source layer

Proves the two premises the reader rests on (slice cost, `readBytesAt` contract) before any parser.

- [x] Created `packages/zip/project.json` mirroring `packages/jwt/project.json`.
- [x] `packages/zip/src/source.mfb`: `Source` union, `readAt`, `sizeOf`, all four refusals as
      specified. The memory branch is `collections::mid` (the plan did not name a primitive;
      `collections::slice` does not exist — `mid(list, start, count)` is the one with exactly this
      signature). Union members are written one per line, not `A | B`, which is a lexer error.
- [x] `packages/zip/src/bytes.mfb`: `u16le`, `u32le`, `u64le` over `(bytes, index)`, each
      bounds-checked against the block. `u64le` refuses a value above `2^63 - 1` with
      `ErrTooLarge` rather than returning a wrapped negative — see Corrections; a negative would
      silently pass the `<= size` bounds checks the parsers rely on.
- [x] Slice cost measured with `/tmp/zipslice` (a program, not in the tree), 1000 reads of a
      4 KiB window in a 64 MiB `List OF Byte`: **offset 0 → 1,389,000 ns; offset 60 MiB →
      1,383,000 ns; ratio 0.99**. `collections::mid` is O(count), not O(list length), so the
      memory branch needs no replacement. The UNVERIFIED property in §Verified properties is now
      measured.
- [x] Tests `packages/zip/src/test_source.mfb` — 11 cases, all passing
      (`target/release/mfb test packages/zip` → `Tests: 11  Pass: 11  Fail: 0`): identical bytes
      from both sources at start/middle/end/whole/last-byte, zero-byte reads, out-of-range and
      negative arguments refused by both, a file read that does not move the handle's position,
      and `readAt` after a callee closed the handle → `ErrResourceClosed`. The `u64le` of `FF×8`
      case asserts `ErrTooLarge`, not a round trip — see Corrections.

Acceptance: both sources return identical bytes and fail identically. **Met** — the seven
`source` cases compare the two sources byte-for-byte at five windows and assert the same error
code from both for every refusal.
  Check: `target/release/mfb test packages/zip` → `Tests: 11  Pass: 11  Fail: 0`.
Commit: 22a7747d9

### Phase 2 — central directory

- [x] `src/cp437.mfb`: 256 code points + `cp437Decode`. **Generated**, not hand-typed, by
      `packages/zip/oracle/cp437_table.py` from Python's own `cp437` codec — the same repertoire
      APPNOTE Appendix D defines. A hand-typed 256-entry table has exactly one failure mode (a
      single wrong character nobody notices until an archive with that byte shows up), and
      transcribing it from a codec removes it.
- [x] `src/central.mfb`: §4.3 steps 1–5, producing `Archive` — EOCD backwards scan (accepting a
      candidate only when its comment length lands exactly at end of archive), ZIP64
      locator/record, the 256 MiB central-directory cap, per-entry parse with the ZIP64 extra
      field applied only to the fields written as all-ones, CP437/UTF-8 name selection on flag bit
      11, Unix mode from the external attributes, and DOS date/time converted to epoch seconds by
      `daysFromCivil` (extended-timestamp field 0x5455 overriding where present).
- [x] `src/lib.mfb`: `open` ×2, `entries`, `comment`, `has`, `find`, `Entry`, `Archive`,
      `ErrorChecksum`, each with a DOC comment; the memory example uses a typed
      `LET raw AS List OF Byte` binding, as the overload rule requires.
- [x] `packages/zip/oracle/fixtures.py` generates six fixtures — `simple` (stored + deflated),
      `comment`, `dirs`, `zip64` (`force_zip64=True`), `cp437`, `ziptool` (`/usr/bin/zip`) — into
      `src/test_fixtures.mfb` **and** into `oracle/corpus/*.zip` for letter D. Two corrections:
      the CP437 fixture is hand-assembled because `zipfile` cannot write a non-UTF-8 name with
      flag bit 11 clear, and the bytes are embedded Base64 rather than as a byte-list literal —
      see Corrections.
- [x] Tests `src/test_central.mfb` — 18 cases. Every fixture's names/sizes/crc/method/
      isDirectory match the oracle through BOTH overloads, and a third case asserts the two
      sources agree field for field (including `compressedSize`, `modifiedSeconds`, `mode` and
      `headerOffset`, which the oracle does not report). Named cases pin the CP437 decode
      (`café.txt`), ZIP64 sizes, the archive comment, directory flagging, the Unix mode (33188)
      and the epoch conversion (1789821000). Malformed: too short, empty, broken EOCD signature,
      truncation, CD offset past EOF, CD size past EOF, count mismatch, multi-disk (both disk
      fields) and a broken central-header signature → the §4.5 codes.

Acceptance: every fixture's entry list matches Python's `ZipInfo` values through both overloads.
**Met** — all six fixtures, both overloads, per field.
  Check: `target/release/mfb test packages/zip` → `Tests: 29  Pass: 29  Fail: 0`.
Commit: dedf0a160

### Phase 3 — entry reads

- [x] `src/entry.mfb` implements §4.4 as `readEntry`/`readEntryText`; `lib.mfb` exports `read`
      and `readText` over them with `maxBytes AS Integer = 67108864` defaults. Sizes, CRC and
      method come from the central directory, so no data descriptor is ever needed — which is what
      lets a file-backed read stay a single positional read rather than a forward scan.
- [x] Tests `src/test_read.mfb` — 15 cases. Every fixture entry reads back to the oracle's bytes
      through both overloads, plus a case asserting the two sources return identical entry bytes.
      Refusals: a flipped data byte → `ErrorChecksum` (checked from memory AND from a file);
      `maxBytes` below the entry size → `ErrTooLarge`, with a follow-up read proving the archive is
      still usable and the refusal read nothing; encrypted flag → `ErrUnsupported`; method 12 →
      `ErrUnsupported`; stored entry with `compressedSize ≠ size` → `ErrInvalidFormat`; broken
      local-header signature → `ErrInvalidFormat`. One case pins the ORDER of checks: a corrupted
      entry fails its CRC before `readText` attempts a decode.
- [x] No-whole-file-load audit. The plan's grep has one match — `fs::readBytes("photos.zip")`
      inside a DOC-comment example in `lib.mfb`, showing what a CALLER does to get bytes for the
      memory overload, not what the package does. Re-run over executable lines only
      (`| grep -v -E ":[0-9]+:[[:space:]]*'"`) it exits 1 with no matches. The positive form is
      stronger and is the one recorded: the package's executable code calls exactly
      **`fs::readBytesAt` once and `fs::size` once**, and nothing else in `fs`
      (`grep -h 'fs::' <non-test sources> | grep -v "^[[:space:]]*'" | grep -oE 'fs::[a-zA-Z]+' |
      sort | uniq -c` → `2 fs::File`, `1 fs::readBytesAt`, `1 fs::size`).

Acceptance: every fixture entry reads back byte-identically from both sources; each defect maps to
its code. **Met.**
  Check: `target/release/mfb test packages/zip` → `Tests: 44  Pass: 44  Fail: 0`; the audit grep
  over executable lines → exit 1, no match.
Commit: PENDING3

## Validation Plan

- Tests: `packages/zip/src/test_source.mfb`, `test_central.mfb`, `test_read.mfb` (positive per
  fixture ×2 sources, negative per error code).
- Coverage check: **done** — all seven exports of `lib.mfb` are called from a `TCASE`
  (`open` 37 call sites, `find` 18, `entries` 15, `read` 13, `comment` 4, `has` 3, `readText` 3).
- Runtime proof: letter D (differential + RSS). This letter's proof is the test run above.
- Doc sync: DOC comments on every export in `lib.mfb` (README/doc.html are letter B).
- Final gate: letter D (run once at the end of plan-139).

## Open Decisions

- Duplicate entry names — `find` returns the first in central-directory order (recommended; matches
  `unzip` listing order and keeps `read(archive, entry)` the precise path) vs. last (Python
  `zipfile.getinfo`). §4.4, letter D records the Python difference in `divergences.json`.
- Central-directory cap — 256 MiB fixed (recommended; no real archive's metadata approaches it) vs.
  an `open` parameter (would break the identical two-overload shape).

## Corrections

### 2026-09-19 — Phase 2: fixture-generation corrections and four language surprises

**The CP437 fixture cannot come from `zipfile`.** Phase 2 asked for "a non-UTF-8 CP437 name via
`ZipInfo` with flag bit 11 clear". Python will not write one: it encodes any non-ASCII name as
UTF-8 and sets bit 11, with no documented way to override. That fixture is therefore assembled by
hand in `fixtures.py` from `struct.pack` — which is arguably better, since the case the CP437 table
exists for is now built deliberately rather than coaxed out of a writer that does not want to
produce it.

**An archive comment containing a `PK\x05\x06` lookalike cannot be an oracle fixture.** The first
draft of the `comment` fixture embedded one, to check that the EOCD scan accepts only a candidate
whose comment length lands exactly at end of file. Python's own reader refuses such an archive
(`BadZipFile: File is not a zip file`), so there is no oracle to compare against. The fixture's
comment is now plain, and the adversarial case moves to letter D's hand-damaged corpus, where
"both refuse" is the expected outcome. **The scan itself still implements the exact-fit rule** —
it is just not provable against Python.

**Fixture bytes are Base64, not a byte-list literal.** The plan said "embed each as a byte-list
literal". Six fixtures totalling a few KB would be several thousand `toByte(...)` calls to compile
for no added clarity; `encoding::base64Decode` of a single string literal carries the identical
bytes. Recorded rather than done silently.

Four language facts the plan's design did not anticipate, all corrected in place:

- `DIV` is the **Float** escape, not integer division (`mfb spec language operators`: "DIV always
  returns Float"). Integer division is `/` on two Integers. Eleven sites in `central.mfb` were
  written the wrong way round and produced `TYPE_BINDING_MISMATCH` against an `Integer` binding.
- A union's members are written one per line; `A | B` is a lexer error.
- `RETURN <expr> TRAP(e) ... END TRAP` does not parse — a `TRAP` attaches to a binding, so the
  UTF-8 name decode binds first and returns after.
- The `expect*` builtins are valid **only directly inside a `TCASE` body**
  (`TESTING_EXPECT_OUTSIDE_TCASE`), so the per-field comparison loops are helper FUNCs returning a
  description of the first disagreement, asserted with one `expectString(..., "")` in the case.
  This is better than it sounds: a failure now names the fixture, the entry index and the field.

### 2026-09-19 — Phase 1 measurements and three plan corrections

**The slice is O(count) — the design's central memory premise holds.** The plan listed "cost of
slicing a `List OF Byte`" as UNVERIFIED and required a measurement before the reader was built on
it. `/tmp/zipslice` (a program, not a `TCASE` — a test asserts, it does not time) read a 4 KiB
window out of a 64 MiB `List OF Byte` 1000 times at offset 0 and 1000 times at offset 60 MiB:

    nearNanos=1389000   farNanos=1383000   farOverNearHundredths=99

A ratio of 0.99 against the plan's "switch primitives if > 2" threshold. `collections::mid` costs
what the window costs, not what the list costs, so the memory branch stands as written.

**`collections::slice` does not exist.** §4.1 named no primitive and the first draft assumed that
spelling. The collections member with exactly the needed signature is
`collections::mid(value AS List OF T, start AS Integer, count AS Integer)`, which raises rather
than clamping when `start + count` exceeds the length — the behaviour `readAt` wants.

**`u64le` of `FF×8` cannot "round-trip as the documented Integer value".** Phase 1's test list
asked for that. MFBASIC's Integer is signed 64-bit, so `0xFFFFFFFFFFFFFFFF` has no representable
value: the only round trip available is to `-1`, and a negative length would then pass every
`offset + count <= size` bounds check the parsers use to police untrusted offsets. `u64le` refuses
anything above `2^63 - 1` with `ErrTooLarge` instead, and the test asserts that. The strictly
larger `0x7FFFFFFFFFFFFFFF` case is tested as a genuine round trip. This strengthens the acceptance
criterion rather than weakening it — an archive claiming 8+ exabytes is refused, not mis-parsed.

**A `RES` record field cannot be closed and then used in the same scope**, so the closed-handle
test closes through a callee (`SUB closeHandle(RES f AS fs::File)`), the case spec §15 says the
runtime flag catches. This confirms plan-139-A §1's requirement that closing the caller's file
makes later reads fail with `ErrResourceClosed` — measured, not assumed.

### 2026-09-15 — Prerequisites gate re-run: still NOT MET, plan not started

`/follow-plan 139` re-ran every Prerequisites command against a fresh release build
(worktree `.claude/worktrees/P-139`, branch `worktree-P-139`, forked from `main` at
`8957025e4`; `cargo build --release` → `Finished \`release\` profile [optimized] target(s)
in 1m 57s`). Four of six rows MET, two NOT MET. No phase of any letter was started —
the gate is the plan's only sanctioned stop, and it is closed.

| Row | Command run | Result |
|---|---|---|
| `fs::size(file AS fs::File) AS Integer` | `target/release/mfb man fs size` | **NOT MET** — ``error: unknown fs function `size` `` |
| `fs::readBytesAt(file, offset, count)` | `target/release/mfb man fs readBytesAt` | **NOT MET** — ``error: unknown fs function `readBytesAt` `` |
| `packages/zip`, `packages/tar` absent | `ls packages/zip packages/tar` | MET — `No such file or directory` ×2 |
| plan-136-B archived | `ls planning/completed/plan-136-B-package-parameter-defaults.md` | MET — present |
| release compiler current | `cargo build --release` | MET — `Finished` (above) |
| Python + `zip` + `bsdtar` | `python3 -c "import zipfile,tarfile"; which zip bsdtar` | MET — ok; `/usr/bin/zip`, `/usr/bin/bsdtar` |

Supporting evidence for the two blocked rows:

- The plan's own registry probe still finds nothing:
  `grep -h -o 'name: "[a-zA-Z]*"' src/codegen/builtins/fs/func_*.rs | grep -i -E 'size|seek|At"'`
  → exit 1, no matches. Also `grep -rn 'name: "size"' src/codegen/builtins/fs/` → no matches.
- `target/release/mfb man fs` lists exactly these handle reads: `readLine`, `readAll`,
  `readAllBytes`, `readBytes`, `eof`. None is positional and none reports a size.
- **`fs::readBytes` is not a substitute.** It is path-based, not handle-based, and is a
  whole-file read — `src/codegen/builtins/fs/func_read_bytes.rs` DESC: "opens the file named
  by `path` … reads its complete contents into a single `List OF Byte`, closes the file …
  The whole file is read in one call — there is no streaming and no partial result."
  Building the source layer on it is exactly the whole-file load §1 Non-goals forbids, so
  this is not a spelling difference the table can absorb.

**The prerequisite work is not merely undone — it is unplanned.** `grep -rln 'readBytesAt'
planning/` matches only this plan's own files, and `grep -rn -i 'readBytesAt|fs::size|pread|lseek'
planning/todo.md planning/*.md` finds no pending fs plan. Before plan-139 can be attempted, a
separate plan must land the two `fs` builtins (registry function each, per-target codegen for
`pread` / `lseek`+`read` / `ReadFile` with `OVERLAPPED`, man pages, spec), as §Prerequisites
already states.

No `- [ ]` box in any letter was ticked; no `packages/zip` or `packages/tar` file was created.

**Re-verified after main advanced.** While the gate was being measured, `main` moved from
`8957025e4` to `e0f72a1ae` (another session's codegen/resource-cleanup work, bugs 642-646).
`main` was merged into `worktree-P-139` and the release compiler rebuilt at the merged tip
(`cargo build --release` → `Finished \`release\` profile [optimized] target(s) in 1m 20s`).
Both blocked rows give the identical result at the new tip:

    $ target/release/mfb man fs size
    error: unknown fs function `size`
    $ target/release/mfb man fs readBytesAt
    error: unknown fs function `readBytesAt`

`ls src/codegen/builtins/fs/func_*.rs | wc -l` → 41 (unchanged), and the registry probe still
returns no matches. main's changes did not touch `fs`, so the gate verdict stands against
current main.

### 2026-09-19 — Prerequisites gate re-run at new main tip: still NOT MET, plan not started

`/follow-plan 139` re-ran every Prerequisites command in a fresh worktree
(`.claude/worktrees/P-139`, branch `worktree-P-139`, forked from `main` at `23c06f46b` —
main has advanced 4 commits since the 2026-09-15 run, which measured `e0f72a1ae`).
`cargo build --release` → `Finished \`release\` profile [optimized] target(s) in 1m 51s`.
Four of six rows MET, the same two NOT MET. No phase of any letter (A, B, C or D) was
started; no `packages/zip` or `packages/tar` file was created.

| Row | Command run | Result |
|---|---|---|
| `fs::size(file AS fs::File) AS Integer` | `target/release/mfb man fs size` | **NOT MET** — ``error: unknown fs function `size` `` |
| `fs::readBytesAt(file, offset, count)` | `target/release/mfb man fs readBytesAt` | **NOT MET** — ``error: unknown fs function `readBytesAt` `` |
| `packages/zip`, `packages/tar` absent | `ls packages/zip packages/tar` | MET — `No such file or directory` ×2 |
| plan-136-B archived | `ls planning/completed/plan-136-B-package-parameter-defaults.md` | MET — present |
| release compiler current | `cargo build --release` | MET — `Finished` (above) |
| Python + `zip` + `bsdtar` | `python3 -c "import zipfile,tarfile"; which zip bsdtar` | MET — ok; `/usr/bin/zip`, `/usr/bin/bsdtar` |

Evidence that main's four new commits did not move the gate:

- Registry probe unchanged: `grep -h -o 'name: "[a-zA-Z]*"' src/codegen/builtins/fs/func_*.rs
  | grep -i -E 'size|seek|At"'` → exit 1, no matches.
- The full sorted registry name list from that same grep contains no `size`, no `seek`, and no
  `*At` read. The handle reads are exactly `readLine`, `readAll`, `readAllBytes`, `eof`;
  `readBytes`/`readText` are the path-based whole-file forms (`mfb man fs`: "fs::readText and
  fs::readBytes read the entire file in one call"). Building the source layer on those is the
  whole-file load §1 Non-goals forbids, so no spelling substitution rescues the gate.

**The prerequisite work is still unplanned, not merely undone.** `grep -rln 'readBytesAt'
planning/` matches only this plan's own file. `grep -rln -E 'readBytesAt|fs::size|pread'
planning/*.md` additionally matched `plan-125-I`, `speed.md` and `todo.md`, but all three are
false positives on the word "spread" (verified by `grep -n`); none is a pending fs plan.
`ls planning/completed/ | grep -i -E 'fs-|positional|readbytesat|seek'` → nothing relevant.
A separate plan must first land the two `fs` builtins (registry function each, per-target
codegen for `pread` / `lseek`+`read` / `ReadFile` with `OVERLAPPED`, man pages, spec).

### 2026-09-19 — the two `fs` prerequisites absorbed into the plan as letter E

The Prerequisites section deferred the two `fs` builtins to "their own plan". Measured
2026-09-19, that plan does not exist and never did: `grep -rln 'readBytesAt' planning/`
matched only this file, and a broader `grep -rln -E 'readBytesAt|fs::size|pread' planning/*.md`
matched three more files that are all false positives on the word "spread" (confirmed with
`grep -n`). A precondition nobody owns is a precondition that never lands, so on the user's
explicit instruction the work was appended to this plan as **plan-139-E**
(`planning/plan-139-E-fs-size-and-positional-read.md`), with the edge `E -> A` added to the
letter table and this letter's `Depends on:` line. The alphabet is append-only; A-D keep their
letters and their order.

**A prediction in this section was wrong, and letter E is smaller because of it.** The
Prerequisites note claimed the two rows need "per-target codegen for `pread` / `lseek`+`read` /
`ReadFile` with `OVERLAPPED`". They do not. `CodegenPlatform` already declares `emit_seek_file`
and `emit_read_file` (`src/codegen/engine/types/types.rs:781,760`) and all three real target
backends already implement both (`src/target/macos_aarch64/code.rs:690,660`;
`src/target/linux_common/code.rs:1027,969`; `src/target/win_x86_64/code.rs:2262,2112`).
Three existing `fs` functions - `readAll`, `readAllBytes` and `eof` - already emit the exact
save/measure/restore seek triple `fs::size` needs (`gen_read_write.rs:509-547`, `:703-745`).
So letter E adds two registry descriptors and two codegen helpers composed from existing hooks,
and is scoped medium (1h-2h) rather than the large per-target effort this note predicted. The
note has been corrected in place.

## Summary

The engineering risk is untrusted offsets driving positional reads, contained by bounds-checking
against `sizeOf` before every `readAt` and fuzzed in letter D. The feature is blocked today on two
`fs` builtins; nothing in `src/` is touched by this plan.
