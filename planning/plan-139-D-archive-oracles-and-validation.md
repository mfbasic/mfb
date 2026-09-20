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
Commit: PENDINGD3

### Phase 4 — final gate and archive

- [ ] `target/release/mfb test packages/zip` and `target/release/mfb test packages/tar` → pass.
- [ ] No `src/` change in letters A–D: `git diff --stat <plan-139-A first commit>^ HEAD -- src`
      → empty. (Corrected 2026-09-19: as written this said "across plan-139", which letter E
      falsifies — E adds two `fs` builtins under `src/codegen/builtins/fs/`. The property that
      matters is that the two *packages* need no compiler change, so it is measured from letter
      A's first commit, which lands after E. This is a correction, not a weakening: E carries its
      own artifact-gate and test-accept gate in its Phase 3.)
- [ ] Update `planning/todo.md` row 5 and "# Proposed API" section to point at plan-139 and note the
      API changes of plan-139-A §3.
- [ ] Move `planning/plan-139-*.md` to `planning/completed/`.

Acceptance: all gates green; plan archived.
  Check: the four commands above (est. 3 min).
Commit: —

## Validation Plan

- Tests: both packages' `TESTING` blocks; oracle `corpus`/`fuzz`/`roundtrip` modes.
- Coverage check: probe calls every read-side export; `roundtrip` calls every writer export.
- Runtime proof: Phase 3 RSS measurement.
- Doc sync: both `oracle/README.md`; `planning/todo.md`.
- Final gate (run ONCE): Phase 4.

## Open Decisions

- None beyond those carried from letters A–C.

## Corrections

### 2026-09-19 — Phase 4's "no `src/` change" criterion corrected for letter E

plan-139-E (the two `fs` builtins, appended 2026-09-19 — see plan-139-A Corrections) changes
`src/`, which made Phase 4's "No `src/` change across plan-139" check false as written. It is
corrected in place to cover letters A–D only, measured from letter A's first commit. The property
being protected — that `packages/zip` and `packages/tar` are pure MFBASIC and need no compiler
change — is unchanged and still checked.

## Summary

Independent-reader agreement plus a hard `sourcesAgree` invariant is what makes "same API, two
sources" true rather than asserted; the RSS measurement is the only direct proof of requirement 1.
