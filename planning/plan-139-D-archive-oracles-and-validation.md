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

See plan-139-A § Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-139-C complete | every `- [ ]` in `planning/plan-139-C-*.md` ticked | NOT MET (re-verified 2026-09-15: `grep -c '^- \[ \]' planning/plan-139-C-*.md` → 10 unticked; the whole A→B→C→D chain is blocked at plan-139-A's `fs` prerequisite gate.) |

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

- [ ] `packages/zip/oracle/{README.md,diff.py,divergences.json,probe/}` and the same for `tar`;
      probe emits `sourcesAgree: false` if the two opens differ (a hard failure, never declarable).
- [ ] Corpus: fixture generators' output + `zip -r`/`bsdtar -cf` of `packages/jwt/src` + damaged
      variants (truncated at 5 offsets, EOCD comment length off by one, ZIP64 locator pointing past
      EOF, tar checksum flipped).
- [ ] Duplicate-name divergence (plan-139-A Open Decisions) recorded in `divergences.json` if kept.

Acceptance: corpus agrees.
  Check: `python3 packages/zip/oracle/diff.py corpus` → exit 0; same for tar (est. 2 min).
Commit: —

### Phase 2 — fuzz and round-trip

- [ ] `diff.py fuzz --count 2000 --seed 139` and `diff.py roundtrip` for both packages. Each finding:
      fix in the package + a `TCASE` reproducing it, in the same commit.

Acceptance: no undeclared disagreement, no crash, `sourcesAgree` always true.
  Check: `python3 packages/zip/oracle/diff.py fuzz --count 2000 --seed 139` → exit 0; same for tar;
  `roundtrip` → exit 0 (est. 8 min — fuzzing is the only check that reaches malformed-offset paths
  the hand corpus does not).
Commit: —

### Phase 3 — memory proof

- [ ] `/tmp` generator: a 2 GiB stored zip (one 2 GiB entry + one 10-byte entry `small.txt`) via
      Python `zipfile` with `ZIP_STORED`; the same content as a tar.
- [ ] `/tmp` probe: `RES f = fs::open(path, "read")`, `open(f)`, `find(a, "small.txt")`,
      `readText`, print it.
- [ ] Run `/usr/bin/time -l <probe> big.zip` and `… big.tar`; record max RSS in this plan.

Acceptance: file-backed open + one small read does not scale with archive size.
  Check: `/usr/bin/time -l` "maximum resident set size" < 67108864 for both (est. 3 min incl. writing
  4 GiB of test data; smaller files cannot distinguish a 64 MiB cap from a whole-file load).
Commit: —

### Phase 4 — final gate and archive

- [ ] `target/release/mfb test packages/zip` and `target/release/mfb test packages/tar` → pass.
- [ ] No `src/` change across plan-139: `git diff --stat <plan-139-A first commit>^ HEAD -- src` →
      empty.
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

## Summary

Independent-reader agreement plus a hard `sourcesAgree` invariant is what makes "same API, two
sources" true rather than asserted; the RSS measurement is the only direct proof of requirement 1.
