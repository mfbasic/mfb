# bug-389: `tests/byte-identity` covers a tiny fraction of the builtin surface — expand to every package function/overload with full `.ncode` goldens

Last updated: 2026-07-25
Effort: x-large (1d–3d) — a large authoring + multi-target golden-seeding effort; mechanically simple but broad and gated on box 2229 for the `linux-*` sums.
Severity: MEDIUM
Class: Test-coverage (a codegen regression in most builtins is currently invisible to the byte-identity gate)
Status: Open
Regression Test: `tests/byte-identity/**` (expanded) — the deliverable IS the tests.

`tests/byte-identity` exists to make `scripts/artifact-gate.sh` catch codegen
regressions in backends that would otherwise have no generated-code golden. Today
it has **6 fixtures** (`audio`, `crypto`, `fs`, `net`, `os`, `tls`), each a single
`main.mfb` that exercises only a **handful** of functions — e.g.
`tests/byte-identity/audio/src/main.mfb` touches just `openInput`, `read`,
`openOutput`, `write` (4 of the audio package's functions). Meanwhile
`src/docs/man/builtins/` lists **~24 builtin packages** (`audio`, `bits`,
`collections`, `crypto`, `csv`, `datetime`, `encoding`, `errorCode`, `fs`,
`general`, `http`, `io`, `json`, `math`, `money`, `net`, `os`, `regex`,
`strings`, `term`, `testing`, `thread`, `tls`, `vector`). So a codegen regression
in any uncovered package — or in any uncovered function/overload of the 6 covered
ones — passes the gate silently.

**The single correct behavior a fix produces:** every builtin package's every
function and every distinct overload is exercised by a byte-identity fixture whose
golden is a **full per-symbol `.ncode` dump** (not the opaque `.ncodesum` hash the
6 fixtures use today), on all four targets — so a codegen change to any builtin
produces a gate diff that localizes to the exact function. Coverage that already
exists elsewhere (e.g. `rt-behavior/**` fixtures that own a full `.ncode` golden)
is counted and NOT duplicated.

**Why the current state is dangerous:** the gate reports a healthy-looking
"N goldens, 0 diffs," but per memory `fast-codegen-gate` that number is dominated
by `.ast`/`.ir`/`.log` goldens; only a tiny set of fixtures carry a native-code
golden at all. A green gate today means "nothing *covered* changed," and almost
nothing is covered — a real backend miscompile in, say, `json` or `math` would
ship green.

<!-- When the fix fully lands:
       ## STATUS: FIXED (<commit hash>)
     then archive to bugs/completed-bugs/. -->

References:

- **bug-388** (`bugs/bug-388-flaky-codegen-cover-byte-identity.md`) —
  **PREREQUISITE.** Byte-identity goldens are only meaningful if codegen is
  deterministic. Seeding a full `.ncode` golden while a fixture is still flaky
  just freezes one random seed's output, producing a permanently-flaky golden.
  Do not expand coverage until bug-388 has proven the relevant fixtures
  deterministic (gate at `diffs=0`).
- `scripts/artifact-gate.sh`, `scripts/test-accept.sh`, `scripts/sync-goldens.sh`
  — the harnesses. Note `sync-goldens.sh` only **refreshes existing** goldens; it
  never creates new ones, so each new fixture needs its goldens **seeded** first.
- Memory `fast-codegen-gate` — the gate is nearly blind to codegen (few `.ncode`
  goldens); a full `.ncode` golden is a per-symbol dump, so one per-package
  fixture localizes a regression via the diff. `.ncodesum` is a hash (no
  localization). Also: `tests/acceptance/**` has no `golden/` (mode switch) — add
  `byte-identity`/`rt-behavior` fixtures, never `golden/` to `acceptance`.
- Memory `linux-boxes-have-no-rust-toolchain` — the three `linux-*` `.ncode`
  goldens regenerate ONLY on box 2229 (release binary, `JOBS=10`); fixtures use
  repo-root-relative paths.
- Memory `bug-workflow-mechanics` — new fixtures need seeded goldens; `git mv`
  stages the index; acceptance is ~15 min.
- Design decision (this session): **per-package project + full `.ncode` golden**,
  chosen over "one fixture per function" (fixture/golden explosion, hundreds of
  remote regens) and over "per-package + keep `.ncodesum`" (no localization).

## Failing Reproduction

Not a defect that "fails" — a coverage hole. The demonstration is the coverage
matrix, and its emptiness is the reproduction:

```
# Functions actually exercised by the current audio byte-identity fixture:
grep -oE 'audio::[a-zA-Z]+' tests/byte-identity/audio/src/main.mfb | sort -u
#   -> openInput, openOutput, read, write   (4)
# Functions the audio package actually exposes:
ls src/docs/man/builtins/audio/            # many more than 4
# Packages with a byte-identity fixture at all:
ls tests/byte-identity/                    # 6
# Builtin packages that exist:
ls src/docs/man/builtins/ | grep -v '\.rs' # ~24
```

- Observed: 6 packages, ~4 functions each, opaque `.ncodesum` goldens.
- Expected: every package, every function/overload, full `.ncode` goldens,
  minus what is already covered elsewhere.

## Root Cause

Not a code bug — a test-suite scope gap. The byte-identity suite was seeded
(plan-57) only for the backends that had *no* generated-code golden at the time
(the platform-specific / resource-heavy ones), with a minimal call set each, and
never grown to full API coverage. The `.ncodesum` hash golden was chosen for
compactness, which also means the existing fixtures give no per-function
localization even for the functions they do touch.

## Goal

- A coverage matrix (package × function × overload) exists and shows, for each
  builtin symbol, which fixture's golden covers it — with **no gaps** among
  symbols that emit native code, and **no duplication** of `rt-behavior/**`
  fixtures that already own a full `.ncode` golden.
- Each covered symbol's coverage is via a **full `.ncode` per-symbol dump** golden
  on all four targets (`macos-aarch64`, `linux-aarch64`, `linux-x86_64`,
  `linux-riscv64`).
- `scripts/artifact-gate.sh` on a clean tree stays `diffs=0` (post-bug-388), now
  over a vastly larger covered surface.

### Non-goals (must NOT change)

- **Codegen itself.** This bug adds tests only; it must not alter any builtin's
  emitted code. If expanding coverage surfaces a real miscompile or a new
  nondeterminism source, that is a *separate* bug (file it; do not fold a codegen
  fix into a test-coverage commit).
- **Do NOT seed goldens for a still-flaky fixture** (see bug-388 prerequisite) —
  a golden captured from nondeterministic output is worse than no golden.
- **Do NOT execute these fixtures.** They are compile-only by design (they open
  devices/sockets that do not exist in a test run); no `.run` golden, no `entry`
  execution. Keep them compile-only.
- **Do NOT duplicate existing coverage.** A symbol already pinned by a full
  `.ncode` golden under `rt-behavior/**` is covered; record it in the matrix,
  don't add a second fixture.
- **Do NOT add `golden/` to `tests/acceptance`** to gain coverage — that dir's
  absence of `golden/` is a harness mode switch and it declares no `entry`.

## Blast Radius

Not a code pattern — a coverage inventory, built in Phase 1:

- The 6 existing byte-identity fixtures — expanded in place, `.ncodesum` →
  `.ncode`.
- ~18 builtin packages with no byte-identity fixture — audited against
  `rt-behavior/**` `.ncode` coverage first; a fixture added only where a real gap
  exists.
- Overloads: many builtins are overloaded (e.g. numeric `Integer` vs `Float`
  paths, `toString` variants). Each distinct overload that lowers to distinct
  codegen needs its own call site; enumerate from `mfb man <pkg>` / the package
  catalog, not from memory.
- Resource-producing builtins (`RES ... = pkg::open*`) need the full resource
  ceremony to type-check; author valid typed call sites per overload.

## Fix Design

Keep the **per-package project** model (6 today, plus one per uncovered package),
one `main.mfb` per package that exercises every function + overload of that
package, golden = full `.ncode` dump per target. Rejected alternatives (this
session): one-fixture-per-function (hundreds of goldens × 3 remote-regen targets)
and per-package-with-`.ncodesum` (zero localization). The per-symbol `.ncode`
dump buys function-level localization from the diff while keeping the fixture
count at ~24. Accepted trade-off: `.ncode` goldens are larger and a broad
ABI/prologue change rewrites many symbols' dumps at once — that is the intended
behavior of a byte-identity test.

## Phases

### Phase 0 — prerequisite gate (bug-388)

- [ ] Confirm bug-388 has landed and the relevant fixtures are proven
      deterministic (`scripts/artifact-gate.sh` → `diffs=0`, stable across
      repeats). If not, STOP — do not seed goldens onto nondeterministic output.

Acceptance: bug-388 FIXED and gate `diffs=0`.
Commit: —

### Phase 1 — coverage matrix (no fixtures yet)

- [ ] Enumerate every builtin package and, per package, every function and every
      distinct overload (from `mfb man <pkg>` / the package catalog — cite the
      source, not memory).
- [ ] Map each symbol to its current native-code golden coverage across the WHOLE
      tree (`tests/byte-identity/**` and any `rt-behavior/**` fixture owning a
      `.ncode` golden). Mark: covered-elsewhere / partially-covered / uncovered.
- [ ] Write the matrix into this doc; it defines exactly which fixtures to expand
      and which to add.

Acceptance: a complete package × function × overload matrix with a coverage
verdict per symbol; the fixture work-list falls out of it.

Commit: —

### Phase 2 — switch the 6 existing fixtures to full `.ncode` goldens (no coverage change)

Isolate the golden-type change from the coverage change.

- [ ] For each of the 6, add a full `.ncode` golden and confirm the harness
      compares `.ncode` when present (per `fast-codegen-gate`), then remove the
      `.ncodesum` golden. No source change yet.
- [ ] Seed the four target goldens (macOS locally; `linux-*` on box 2229).
- [ ] `scripts/artifact-gate.sh` → `diffs=0`; confirm a deliberate scratch codegen
      tweak now shows up as a localized `.ncode` diff (proves the golden works).

Acceptance: 6 fixtures on `.ncode` goldens, gate green, localization demonstrated.
Commit: —

### Phase 3 — expand the 6 to full function/overload coverage

- [ ] Grow each package's `main.mfb` to call every function + overload from the
      Phase 1 matrix, with valid typed call sites and resource ceremony where
      needed. Keep compile-only.
- [ ] Re-seed the four target goldens per fixture.

Acceptance: each of the 6 fixtures covers 100% of its package's symbols per the
matrix; gate `diffs=0`.
Commit: —

### Phase 4 — add fixtures for uncovered packages

- [ ] For each package the matrix marks uncovered (and not covered by an
      `rt-behavior/**` `.ncode` golden), add a new byte-identity project +
      `main.mfb` exercising every function/overload; seed its four target goldens.
- [ ] `log`/record any package deliberately skipped (already covered elsewhere)
      with the fixture that covers it — no silent gaps.

Acceptance: the matrix shows no uncovered native-code symbol; every added fixture
has all four target goldens.
Commit: —

### Phase 5 — full validation

- [ ] `scripts/artifact-gate.sh target/debug/mfb` → `diffs=0`, repeated ≥3×
      (determinism holds across the enlarged surface).
- [ ] Run the full `scripts/test-accept.sh` once (byte-identity is compile-only,
      but confirm the suite is green and no fixture accidentally executes).
- [ ] Regenerate/verify all `linux-*` goldens on box 2229 (release, `JOBS=10`).
- [ ] Update memory `fast-codegen-gate` — the "gate is nearly blind to codegen"
      caveat is now materially reduced; record the new covered-symbol count.

Acceptance: full suite green; gate `diffs=0` and stable; the coverage matrix is
100% for native-code symbols; memory updated.
Commit: —

## Validation Plan

- Regression test(s): the expanded `tests/byte-identity/**` fixtures themselves.
- Runtime proof: none — compile-only by design; the proof is a per-symbol `.ncode`
  golden that flips on any codegen change to that symbol (demonstrated in Phase 2).
- Doc sync: update `fast-codegen-gate` memory; no `src/docs/spec` change (tests
  only).
- Full suite: `scripts/artifact-gate.sh` (`diffs=0`, repeated) + one
  `scripts/test-accept.sh`; `linux-*` goldens seeded/verified on box 2229.

## Open Decisions

- Whether pure-compute packages already well-covered by `rt-behavior/**` `.ncode`
  fixtures (e.g. `collections`, `strings`) get a byte-identity fixture too, or are
  recorded as covered-elsewhere. **Recommended:** covered-elsewhere — don't
  duplicate; the matrix decides per symbol.
- Overload granularity — cover every overload that lowers to *distinct* codegen
  (recommended) vs. every overload unconditionally. The matrix should note when
  two overloads share a lowering so coverage isn't inflated.

## Summary

The engineering risk is almost entirely in **breadth and golden-seeding
discipline**, not difficulty: the work is mechanical (author valid call sites,
seed `.ncode` goldens on four targets) but wide, and every `linux-*` golden must
be seeded on box 2229. The one hard gate is ordering — bug-388 must land first, or
every golden seeded here inherits the nondeterminism and the whole suite becomes
flaky. Codegen itself is untouched; if this expansion uncovers a real miscompile,
that is a separate bug, filed, never folded into a test-coverage commit.
