# plan-139-D: `zip` / `tar` — oracles, file-vs-memory differential, memory proof, final gate

Last updated: 2026-09-15
Effort: medium (1h–2h)
Depends on: plan-139-C (complete). Whole-feature prerequisites: plan-139-A § Prerequisites.

Checks both packages against independent readers and proves requirement 1 end-to-end. Outcome:
**over the corpus and a fuzzed set, each package agrees with Python `zipfile`/`tarfile` (or both
refuse, or the difference is declared); the `fs::File` and `List OF Byte` sources agree on every
input; reading one small entry from a 2 GiB archive through `fs::File` peaks below 64 MiB RSS.**

References:

- `packages/yaml/oracle/README.md`, `packages/jwt/oracle/` — probe layout (`probe/project.json` with
  `"packages": [{ "name": …, "source": "file:packages/<name>.mfp" }]`), `divergences.json`, exit 0
  iff all agree or declared.
- plan-139-A §4, plan-139-B §4, plan-139-C §3.

## Prerequisites

See plan-139-A § Prerequisites — its two `fs` rows are delivered by **plan-139-E**, which gates
the whole chain `E -> A -> B -> C -> D`. Plus:

| Must be true | Command | Status |
|---|---|---|
| plan-139-C complete | every `- [ ]` in `planning/plan-139-C-*.md` ticked | **MET** (2026-09-19: plan-139-C has 0 unticked boxes and is archived; `target/release/mfb test packages/zip` -> 76 pass, `... packages/tar` -> 44 pass) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run before you
> continue and before you stop; if you stop, report all prerequisites.

## 1. Goal

- Oracle agreement, source agreement, bounded memory — each a command with an exit status.

### Non-goals

- No API change. A disagreement found here is a bug in letter A–C code, fixed in the package, with
  a `TCASE` added — never a relaxed oracle comparison unless the spec permits both readings, in which
  case it goes in `divergences.json` with the spec citation.

## 2. Current State

After plan-139-C: both packages complete with unit tests and fixture generators
`packages/zip/oracle/fixtures.py`, `packages/tar/oracle/fixtures.py`.

## 3. Design Overview

Python is the oracle (stdlib only, no `node_modules`), because both `zipfile` and `tarfile` are
mature, independent implementations already on the box (Python 3.14.5, plan-139-A Prerequisites).
Per package: `oracle/probe/` — an MFB executable that takes a job file (list of archive paths),
opens each **twice** (`fs::open` → `open(RES file)`, and `fs::readBytes` → `open(bytes)`), and writes
one JSON line per archive: entry fields + SHA-256 of each entry's contents (`crypto::` hash), or
the error code. `oracle/diff.py` runs the probe, runs Python over the same paths, and compares.
Modes: `corpus` (fixtures + `zip`/`bsdtar`-made + hand-damaged files), `fuzz --count N` (mutate
bytes/offsets/lengths of corpus files with a fixed seed), `roundtrip` (package writes, Python reads).

Memory proof: `/usr/bin/time -l` reports "maximum resident set size" on macOS.

## Phases

> **NOTE — keep the checkboxes current as you go** (same rules as plan-139-A).

### Phase 1 — probes and corpus diff

- [x] `packages/zip/oracle/{README.md,diff.py,divergences.json,probe/}` and the same for `tar`.
      The probe describes each archive from BOTH sources and emits `sourcesAgree`; `diff.py`
      treats a false there as a hard failure no declaration can cover.
- [x] Corpus: the fixture archives, an archive made by `zip -r`/`bsdtar -cf` of
      `packages/jwt/src`, and damaged variants of every one — truncated at 5 offsets, plus (zip)
      an EOCD comment length that does not reach EOF, a central-directory offset past the end, an
      entry count larger than the directory holds, and a ZIP64 locator pointing past EOF; (tar) a
      flipped checksum digit, a non-octal size, a size claiming more data than the archive holds,
      and an unterminated name field. **70 zip archives and 60 tar archives, 0 disagreements.**
- [x] ~~Duplicate-name divergence recorded in `divergences.json`~~ — moot: no divergence arose.
      plan-139-A's Open Decision was whether `find` returns the first or the last of a repeated
      name; it returns the first, and Python's `zipfile.getinfo` returns the last. The oracle
      never sees it, because `diff.py` compares the full entry LIST in order rather than looking
      names up — both implementations list both entries, in the same order. The difference is
      real but confined to `find`, and it is documented on the package page instead.

Acceptance: corpus agrees. **Met.**
  Check: `python3 packages/zip/oracle/diff.py corpus` → `corpus: 70 archive(s), 0
  disagreement(s)`, exit 0; `… tar …` → `corpus: 60 archive(s), 0 disagreement(s)`, exit 0.
Commit: 1100e63d7

### Phase 2 — fuzz and round-trip

- [x] `diff.py fuzz --count 2000 --seed 139` and `diff.py roundtrip` for both packages. **The
      fuzzer found seven real bugs**, each fixed with a `TCASE`: six in zip (the two copies of an
      entry's name never compared; the directory's declared size never checked against its entry
      count; general-purpose flag bits 5/6/13 ignored; flags read only from the local header;
      overlapping entry data regions accepted; a NUL inside a name kept) and one in tar (a GNU
      long-name record is NUL-terminated, but NULs were being stripped from anywhere and the rest
      kept, so `a\0bbb` became the name `abbb`). Zip disagreements went 230 → 0 over the course of
      the fixes; tar 21 → 0.

Acceptance: no undeclared disagreement, no crash, `sourcesAgree` always true. **Met** —
`sourcesAgree` was true on every one of the 4000 fuzzed archives, and no run crashed.
  Check: `python3 packages/zip/oracle/diff.py fuzz --count 2000 --seed 139` → `fuzz: 2000
  archive(s), 0 disagreement(s)`, exit 0; same for tar; `roundtrip` → `6 archive(s), 0
  disagreement(s)` for each, exit 0.
Commit: 1100e63d7

### Phase 3 — memory proof

- [x] `/tmp/bigarch`: `big.zip` (2,147,483,994 bytes — one 2 GiB stored entry plus a 10-byte
      `small.txt`) and `big.tar` (2,147,491,840 bytes, the same content). Both written
      incrementally so the generator itself never holds 2 GiB.
- [x] `/tmp/bigprobe`: opens the archive through an `fs::File`, lists it, finds `small.txt`,
      reads it and prints it. Both packages in one probe, selected by argument.
- [x] Measured with `/usr/bin/time -l`, **plus a control on a tiny archive** — which is what
      turns the number into a proof, since a low RSS on one file says nothing on its own:

      | archive | size | maximum resident set size |
      |---|---|---|
      | `tiny.zip` | 228 B | 3,358,720 |
      | `big.zip` | 2,147,483,994 B | **3,440,640** |
      | `tiny.tar` | 10,240 B | 3,244,032 |
      | `big.tar` | 2,147,491,840 B | **3,244,032** |

      The zip grew by a factor of **9,419,666** and its resident set by 82 KB (+2.4%). The tar
      grew by a factor of **209,716** and its resident set by **zero bytes**. Both are ~3 MiB
      against a 64 MiB budget, and neither scales with the archive. Every run printed
      `entries=2`, `size=10`, `text=ten bytes!`.

Acceptance: file-backed open + one small read does not scale with archive size. **Met, and
demonstrated rather than merely satisfied**: the budget is 67,108,864 and both came in at ~3.3 MiB,
5% of it — and the tiny-archive control shows the figure is independent of archive size, which the
threshold alone would not establish.
  Check: `/usr/bin/time -l /tmp/bigprobe/build/bigprobe.out zip /tmp/bigarch/big.zip` → 3,440,640;
  `… tar /tmp/bigarch/big.tar` → 3,244,032. Both < 67,108,864.
Commit: ce50bab83

### Phase 4 — final gate and archive

- [x] `target/release/mfb test packages/zip` → `Tests: 83  Pass: 83  Fail: 0`;
      `target/release/mfb test packages/tar` → `Tests: 46  Pass: 46  Fail: 0`.
- [x] No `src/` change in letters A–D: the diff of `src` from letter A's first commit
      (`22a7747d9`) to HEAD is **empty**. The two packages are pure MFBASIC and needed no compiler
      change; everything `src/` gained in this plan was letter E's two `fs` builtins, which landed
      before letter A began. (Corrected 2026-09-19: as written this said "across plan-139", which letter E
      falsifies — E adds two `fs` builtins under `src/codegen/builtins/fs/`. The property that
      matters is that the two *packages* need no compiler change, so it is measured from letter
      A's first commit, which lands after E. This is a correction, not a weakening: E carries its
      own artifact-gate and test-accept gate in its Phase 3.)
- [x] `planning/todo.md` row 5 now reads **DONE — plan-139** with the measured outcome, and the
      "# Proposed API" section carries a note listing all seven ways the shipped API differs from
      the proposal (the second `open` overload, `tar` `maxBytes`, `tar::Entry.isDirectory`, shared
      `errorCode::` values, the package-local CP437 table, the extra `Entry` fields, and
      `tar::addSymlink` with its link-refusing `extractTo`).
- [x] All five letters moved to `planning/completed/`: A, B, C and E were archived as each
      completed, and D with this commit.

Acceptance: all gates green; plan archived. **Met**, and re-run on the merged tree after main
advanced (bug-630 changed codegen, so the earlier pass no longer applied):

| Gate | Result |
|---|---|
| `mfb test packages/zip` | `Tests: 83  Pass: 83  Fail: 0` |
| `mfb test packages/tar` | `Tests: 46  Pass: 46  Fail: 0` |
| `scripts/artifact-gate.sh target/release/mfb all` | 1466 tests, 1637 builds, 2058 goldens, **0 diffs** |
| `scripts/test-accept.sh target/release/mfb` | `acceptance tests passed (1490 test(s) ran)`, exit 0 |
| `cargo test --bin mfb` | `4270 passed; 0 failed; 1 ignored` |
| `diff.py corpus` (both) | 70 and 60 archives, 0 disagreements |
| `diff.py fuzz --count 2000 --seed 139` (both) | 2000 each, 0 disagreements |
| `src` diff from letter A's first commit | empty |

Commit: PENDINGD4

## Validation Plan

- Tests: both packages' `TESTING` blocks; oracle `corpus`/`fuzz`/`roundtrip` modes.
- Coverage check: probe calls every read-side export; `roundtrip` calls every writer export.
- Runtime proof: Phase 3 RSS measurement.
- Doc sync: both `oracle/README.md`; `planning/todo.md`.
- Final gate (run ONCE): Phase 4.

## Open Decisions

- None beyond those carried from letters A–C.

## Corrections

### 2026-09-19 — the fuzzer found seven real bugs, and what they had in common

§1 Non-goals says a disagreement here "is a bug in letter A–C code, fixed in the package, with a
`TCASE` added — never a relaxed oracle comparison". That is what happened seven times. Zip
disagreements went **230 → 0** and tar **21 → 0** over the course of the fixes, and every one has a
regression test:

| Package | Bug | Consequence |
|---|---|---|
| zip | the two copies of an entry's name were never compared | a tool listing the central directory and one walking local headers report different names for the same entry |
| zip | the directory's declared size was never checked against its entry count | a reader walking by size sees a different set of entries than one walking by count |
| zip | general-purpose flag bits 5, 6 and 13 were ignored | patched or strongly-encrypted data was inflated and returned as though it were the entry's contents |
| zip | flags were read only from the local header | the central directory's copy could declare encryption the local copy did not |
| zip | overlapping entry data regions were accepted | the shape of a zip bomb, and of a confusion attack |
| zip | a NUL inside an entry name was kept | Python truncates there; either way two tools disagree about the entry's name |
| tar | a GNU long-name record was NUL-*stripped* rather than NUL-*terminated* | `a\0bbb` became the single name `abbb` |

Six of the seven are the same underlying mistake: **a zip or tar records the same fact twice, and
the reader consulted only one copy.** The name, the flags, the entry count against the directory
size. None was reachable by a hand-written test, because writing one requires already suspecting
the gap; all seven came out of 4000 mutated archives compared against an implementation that was
not ours. That is the argument for this letter existing.

### 2026-09-19 — what a declared divergence is allowed to be

§1 Non-goals permits a declaration only "unless the spec permits both readings, in which case it
goes in `divergences.json` with the spec citation". Four were declared, all as *policies* covering
a class with one argument rather than as per-file suppressions:

- **we are stricter than Python** (both packages). We refuse self-inconsistent archives Python
  recovers from. APPNOTE.TXT and POSIX.1-2017 specify how a *well-formed* archive is laid out and
  do not require a reader to reconstruct a broken one, so both readings are permitted.
- **`version needed to extract` is advisory** (zip). Python refuses on the claimed field; we
  validate the features actually used.
- **name encoding is unspecified** (tar). A name that is not valid UTF-8 has no single right
  rendering; Python uses surrogate escapes, we use Latin-1. The bytes agree.
- **Python stops early** (tar). On some damaged archives it lists fewer entries than we do.

The last one is the only declaration in the dangerous direction, so it was **verified rather than
argued**: a third checksum walk written in the diff harness — belonging to neither implementation —
found 9 headers with valid checksums in archives where Python reported 3 members, and the 6 entries
we list are exactly the 6 the undamaged archive holds. Each policy is also one-directional in the
comparator: "we refused, Python accepted" is covered, "we accepted, Python refused" is not.

### 2026-09-19 — the probe had to be hardened before it could judge anything

The first fuzz run of the tar oracle died in `json.loads`: the probe emitted raw C1 control bytes
inside JSON strings, from names the lenient Latin-1 fallback had produced out of fuzzed bytes. Nine
lines in 2000 were unparseable.

That is a defect in the instrument, not in the package, and it is worth recording because an
instrument that fails on the inputs it exists to examine reports nothing. Both probes now escape
everything outside printable ASCII as `\uXXXX`, so their output is pure ASCII and survives any
byte sequence a fuzzer can produce.

### 2026-09-19 — Phase 4's "no `src/` change" criterion corrected for letter E

plan-139-E (the two `fs` builtins, appended 2026-09-19 — see plan-139-A Corrections) changes
`src/`, which made Phase 4's "No `src/` change across plan-139" check false as written. It is
corrected in place to cover letters A–D only, measured from letter A's first commit. The property
being protected — that `packages/zip` and `packages/tar` are pure MFBASIC and need no compiler
change — is unchanged and still checked.

## Summary

Independent-reader agreement plus a hard `sourcesAgree` invariant is what makes "same API, two
sources" true rather than asserted; the RSS measurement is the only direct proof of requirement 1.
