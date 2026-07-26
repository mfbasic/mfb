# bug-389: `tests/byte-identity` covers a tiny fraction of the builtin surface — expand to every package function/overload, keeping the `.ncodesum` hash golden

Last updated: 2026-07-26
Effort: x-large (1d–3d) — a large authoring + multi-target golden-seeding effort; mechanically simple but broad and gated on box 2229 for the `linux-*` sums.
Severity: MEDIUM
Class: Test-coverage (a codegen regression in most builtins is currently invisible to the byte-identity gate)
Status: Open
Regression Test: `tests/byte-identity/**` (expanded) — the deliverable IS the tests.

`tests/byte-identity` exists to make `scripts/artifact-gate.sh` catch codegen
regressions in backends that would otherwise have no generated-code golden. Today
it has **6 package fixtures** (`audio`, `crypto`, `fs`, `net`, `os`, `tls`) — plus
`link-const-pins`, a bug-388 determinism harness that is not a package fixture and
is out of scope for this coverage work — each a single
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
golden is the **`.ncodesum` sha256 hash** the existing fixtures already use — one
per target, on all four targets — so a codegen change to *any* covered builtin
flips at least one fixture's hash and the gate goes red. Coverage that already
exists elsewhere (e.g. `rt-behavior/**` fixtures that own a `.ncode`/`.ncodesum`
golden) is counted and NOT duplicated.

**Golden type — DECISION (2026-07-26, user):** keep the compact `.ncodesum` hash,
do **not** switch to a full `.ncode` dump. A full per-package `.ncode` dump measures
**~6.8 MB / ~100k lines per target** (measured: `audio` fixture, `mfb build -ncode`,
release binary) because `IMPORT <pkg>` emits the *entire* package + its runtime into
the plan; across ~24 packages × 4 targets that is **~650 MB** of committed goldens —
infeasible, and exactly why `scripts/artifact-gate.sh:91-94` says these dumps "run to
tens of megabytes each and cannot be committed." The hash is a **change-sentinel**: it
tells you *something* in a package's codegen moved. It does not localize to a function
on its own; that is an accepted trade-off, resolved by the diagnosis workflow below.

**Diagnosis workflow when a fixture's hash goes red** (documented, manual — no
committed `.ncode`): from a clean tree, `git stash` the suspect change, run
`mfb build -q -ncode [-target <t>] <fixture>` to dump the baseline `.ncode`, save it,
`git stash pop`, rebuild the `.ncode`, and `diff` the two dumps — the diff localizes to
the exact symbol/instructions that moved. The hash tells you *that* something changed;
this recovers *what*, on demand, without carrying 650 MB in the repo.

**Two emission models — why "import + call every function" is load-bearing** (both
measured 2026-07-26, release binary, `-ncode`):
- **Whole-package-on-import** (e.g. `audio`, resource-heavy packages): a bare
  `IMPORT audio` with an empty `main()` already emits all 26 `audio_5F*` symbols —
  identical set to the fully-exercised fixture. For these, the *import alone* covers
  every out-of-line function; calling them adds nothing.
- **Per-function-on-call** (e.g. `math`, inlined/intrinsic/monomorphized builtins): a
  bare `IMPORT math` emits **0** `math_*` symbols; `math::sqrt(2.0)` is what makes
  `math_sqrt_valid_0` appear. For these, only a real *call site* forces emission, and a
  per-overload call is what covers a per-overload lowering.

Because a package can be either kind (and overloads can lower distinctly only when
called), the fixture must **both** `IMPORT` the package **and** exercise every
function + distinct overload. Import covers the whole-package emitters for free;
the call sites are what make the inlined/monomorphized surface visible. Calling a
function that was already emitted by import is harmless (belt-and-suspenders).

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
  deterministic. Seeding a `.ncodesum` hash golden while a fixture is still flaky
  just freezes one random seed's output, producing a permanently-flaky golden.
  Do not expand coverage until bug-388 has proven the relevant fixtures
  deterministic (gate at `diffs=0`).
- `scripts/artifact-gate.sh`, `scripts/test-accept.sh`, `scripts/sync-goldens.sh`
  — the harnesses. Note `sync-goldens.sh` only **refreshes existing** goldens; it
  never creates new ones, so each new fixture needs its goldens **seeded** first.
- Memory `fast-codegen-gate` — the gate is nearly blind to codegen (few native
  goldens). This bug fixes the *breadth* gap (cover every package/function) while
  keeping the compact `.ncodesum` hash. The hash is a change-sentinel (no built-in
  localization); localization is recovered on demand via the stash→regen→diff
  workflow above. Also: `tests/acceptance/**` has no `golden/` (mode switch) — add
  `byte-identity`/`rt-behavior` fixtures, never `golden/` to `acceptance`.
- Memory `linux-boxes-have-no-rust-toolchain` — the three `linux-*` `.ncodesum`
  goldens regenerate ONLY on box 2229 (release binary, `JOBS=10`); fixtures use
  repo-root-relative paths.
- Memory `bug-workflow-mechanics` — new fixtures need seeded goldens; `git mv`
  stages the index; acceptance is ~15 min.
- Design decision (2026-07-26, user, supersedes the prior session's note):
  **per-package project + `.ncodesum` hash golden**, exercising every function +
  overload. Chosen over "full `.ncode` golden" (~650 MB, infeasible — measured
  above) and over "one fixture per function" (fixture/golden explosion, hundreds of
  remote regens). The lost per-function localization is recovered on demand by the
  stash→regen→diff workflow, so it is not carried in the repo.

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
- Expected: every package, every function/overload, `.ncodesum` hash goldens,
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
  fixtures that already own a native golden.
- Each covered symbol's coverage is via a **`.ncodesum` hash** golden on all four
  targets (`macos-aarch64`, `linux-aarch64`, `linux-x86_64`, `linux-riscv64`).
  Each package fixture both `IMPORT`s the package and exercises every function +
  distinct overload (see the two emission models above), so both whole-package and
  per-call emitters are covered.
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
- **Do NOT duplicate existing coverage.** A symbol already pinned by a native
  golden under `rt-behavior/**` is covered; record it in the matrix, don't add a
  second fixture.
- **Do NOT add `golden/` to `tests/acceptance`** to gain coverage — that dir's
  absence of `golden/` is a harness mode switch and it declares no `entry`.

## Blast Radius

Not a code pattern — a coverage inventory, built in Phase 1:

- The 6 existing package fixtures — expanded in place (call every function +
  overload); golden type unchanged (`.ncodesum` hash).
- ~18 builtin packages with no byte-identity fixture — audited against
  `rt-behavior/**` native coverage first; a fixture added only where a real gap
  exists.
- Overloads: many builtins are overloaded (e.g. numeric `Integer` vs `Float`
  paths, `toString` variants). Each distinct overload that lowers to distinct
  codegen needs its own call site; enumerate from `mfb man <pkg>` / the package
  catalog, not from memory.
- Resource-producing builtins (`RES ... = pkg::open*`) need the full resource
  ceremony to type-check; author valid typed call sites per overload.

## Fix Design

Keep the **per-package project** model (6 package fixtures today, plus one per
uncovered package; `link-const-pins` is a determinism harness, not a package),
one `main.mfb` per package that both `IMPORT`s the package and exercises every
function + distinct overload of that package, golden = **`.ncodesum` hash per
target** (unchanged from today's fixtures). Rejected alternatives: full `.ncode`
dump (~650 MB across 24 packages × 4 targets — infeasible, measured above) and
one-fixture-per-function (hundreds of goldens × 3 remote-regen targets). The hash
is a change-sentinel; per-function localization is recovered on demand via the
stash→regen→diff workflow (documented above), not carried in the repo. Accepted
trade-off: a hash flip says "something in this package's codegen moved" without
naming the symbol until you run the diagnosis workflow — and a broad ABI/prologue
change flips many packages' hashes at once, the intended behavior of a
byte-identity test.

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
      native golden). Mark: covered-elsewhere / partially-covered / uncovered.
- [ ] Note each package's emission model (whole-package-on-import vs
      per-function-on-call — test with a bare `IMPORT`), so the matrix records
      which symbols need an explicit call site.
- [ ] Write the matrix into this doc; it defines exactly which fixtures to expand
      and which to add.

Acceptance: a complete package × function × overload matrix with a coverage
verdict per symbol; the fixture work-list falls out of it.

Commit: —

### Phase 2 — expand the 6 package fixtures to full function/overload coverage

Golden type is unchanged (`.ncodesum` hash). This phase only grows coverage.

- [ ] Grow each package's `main.mfb` to both `IMPORT` the package and call every
      function + distinct overload from the Phase 1 matrix, with valid typed call
      sites and resource ceremony where needed. Keep compile-only.
- [ ] Re-seed the four target `.ncodesum` goldens per fixture (macOS locally;
      `linux-*` on box 2229).
- [ ] `scripts/artifact-gate.sh` → `diffs=0`; confirm a deliberate scratch codegen
      tweak flips the relevant fixture's hash (proves the sentinel works), then
      revert the scratch tweak.

Acceptance: each of the 6 package fixtures covers 100% of its package's symbols per
the matrix; gate `diffs=0`; hash-flip demonstrated.
Commit: —

### Phase 3 — add fixtures for uncovered packages

- [ ] For each package the matrix marks uncovered (and not covered by an
      `rt-behavior/**` native golden), add a new byte-identity project +
      `main.mfb` that `IMPORT`s the package and exercises every function/overload;
      seed its four target `.ncodesum` goldens.
- [ ] `log`/record any package deliberately skipped (already covered elsewhere)
      with the fixture that covers it — no silent gaps.

Acceptance: the matrix shows no uncovered native-code symbol; every added fixture
has all four target `.ncodesum` goldens.
Commit: —

### Phase 4 — full validation

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
- Runtime proof: none — compile-only by design; the proof is a `.ncodesum` hash
  golden that flips on any codegen change to any covered symbol in the package
  (hash-flip demonstrated in Phase 2). Localizing which symbol moved is the manual
  stash→regen full `.ncode`→unstash→diff workflow, run on demand.
- Doc sync: update `fast-codegen-gate` memory; no `src/docs/spec` change (tests
  only).
- Full suite: `scripts/artifact-gate.sh` (`diffs=0`, repeated) + one
  `scripts/test-accept.sh`; `linux-*` goldens seeded/verified on box 2229.

## Open Decisions

- Whether pure-compute packages already well-covered by `rt-behavior/**` native
  fixtures (e.g. `collections`, `strings`) get a byte-identity fixture too, or are
  recorded as covered-elsewhere. **Recommended:** covered-elsewhere — don't
  duplicate; the matrix decides per symbol.
- Overload granularity — cover every overload that lowers to *distinct* codegen
  (recommended) vs. every overload unconditionally. The matrix should note when
  two overloads share a lowering so coverage isn't inflated. Note: for
  per-function-on-call packages (e.g. `math`) an uncalled overload emits no symbol,
  so a distinct overload is only covered by a distinct call site. **RESOLVED (golden
  type):** hash-only, no full `.ncode` in the repo (see Golden type decision above).

## Summary

The engineering risk is almost entirely in **breadth and golden-seeding
discipline**, not difficulty: the work is mechanical (author valid call sites,
seed `.ncodesum` hash goldens on four targets) but wide, and every `linux-*`
golden must be seeded on box 2229. The golden stays the compact `.ncodesum` hash
— a full `.ncode` dump is ~6.8 MB/target (~650 MB across the suite) and infeasible
to commit; the hash is a change-sentinel and per-symbol localization is recovered
on demand via stash→regen→diff. The one hard gate is ordering — bug-388 must land
first, or every golden seeded here inherits the nondeterminism and the whole suite
becomes flaky. Codegen itself is untouched; if this expansion uncovers a real
miscompile, that is a separate bug, filed, never folded into a test-coverage commit.
