# bug-389: `tests/byte-identity` covers a tiny fraction of the builtin surface — expand to every package function/overload, keeping the `.ncodesum` hash golden

Last updated: 2026-07-26
Effort: x-large (1d–3d) — a large authoring + multi-target golden-seeding effort; mechanically simple but broad and gated on box 2229 for the `linux-*` sums.
Severity: MEDIUM
Class: Test-coverage (a codegen regression in most builtins is currently invisible to the byte-identity gate)
Status: FIXED (c0d0f6b51)
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

## STATUS: FIXED (c0d0f6b51)

The byte-identity suite now covers **22 builtin packages** (~435 functions,
hundreds of distinct overloads), each `IMPORT`ing its package and exercising every
function/overload compile-only, pinned by a 4-target `.ncodesum` hash golden
(`macos-aarch64`, `linux-aarch64`, `linux-x86_64`, `linux-riscv64`). Fixtures:
`tests/byte-identity/{audio,bits,collections,crypto,csv,datetime,encoding,fs,
general,http,io,json,math,money,net,os,regex,strings,term,thread,tls,vector}`.

Verification: `scripts/artifact-gate.sh target/release/mfb` → `1087 tests, 1436
golden(s) checked, 0 diff(s)`, identical across 3 runs; hash-flip demonstrated
(a `bits::popCount` call flipped the bits `.ncodesum`, reverted); filtered
acceptance `acceptance tests passed (23 test(s) ran)`.

Deviations from the plan (both improvements):
1. **Box 2229 not needed** — all four target `.ncode` dumps regenerate on the
   macOS host via `mfb build -ncode -target <t>`, so every `.ncodesum` (including
   the three `linux-*`) is seeded locally. Supersedes the doc's box-2229 note.
2. **`app`, `errorCode`, `testing` recorded as covered-elsewhere** (Phase 1
   matrix): `app` is app-mode-only and cannot cross-compile (a 4-target fixture is
   impossible; covered by `syntax/app/*`); `errorCode` has no code symbols;
   `testing`'s assertion builtins are valid only inside a TCASE.

Codegen was NOT changed — this is a test-coverage-only bug (all changes under
`tests/byte-identity/`).

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

- [x] Confirm bug-388 has landed and the relevant fixtures are proven
      deterministic (`scripts/artifact-gate.sh` → `diffs=0`, stable across
      repeats). If not, STOP — do not seed goldens onto nondeterministic output.

Acceptance: bug-388 FIXED and gate `diffs=0`. **Met:** bug-388 is archived in
`bugs/completed-bugs/`; `scripts/artifact-gate.sh target/release/mfb` on a fresh
HEAD binary reports `1071 tests … 0 diff(s)` (stable). Determinism reseed of the
`audio` fixture reproduced the committed hashes on all four targets.
Commit: —

### Phase 1 — coverage matrix (no fixtures yet)

- [x] Enumerate every builtin package and, per package, every function and every
      distinct overload (source: `mfb man <pkg>` FUNCTIONS section and per-function
      `mfb man <pkg> <func>` Synopsis, on a fresh HEAD release binary).
- [x] Map each symbol to its current native-code golden coverage across the WHOLE
      tree. Native goldens outside byte-identity: `rt-behavior/collections/*`,
      `rt-behavior/crypto/crypto-ec-valid`, `rt-behavior/control-flow/*`,
      `syntax/app/macos-app-mode-*`, `syntax/{lexical,match}/*` — each pins only a
      handful of symbols, so no whole package was already fully covered.
- [x] Emission model: verified both exist (`IMPORT audio` with empty `main()`
      emits all `audio_5F*` symbols; `IMPORT math` alone emits zero — `math::sqrt`
      appears only on call). Each fixture therefore BOTH `IMPORT`s the package AND
      calls every function/overload, covering both models.
- [x] Matrix written below.

**Coverage matrix** (per-package; counts are functions / distinct overloads called,
from the fan-out reports; `types`/`language` man entries are doc topics, excluded):

| Package | fns / overloads | Fixture | Disposition |
|---|---|---|---|
| audio | 11 / 21 | `byte-identity/audio` | expanded (was 4 fns) |
| bits | 17 / 17 | `byte-identity/bits` | new |
| collections | 39 / all | `byte-identity/collections` | new |
| crypto | 31 / 43 | `byte-identity/crypto` | expanded (was ~1) |
| csv | 2 / 2 | `byte-identity/csv` | new |
| datetime | 46 / all | `byte-identity/datetime` | new |
| encoding | 34 / 36 | `byte-identity/encoding` | new |
| fs | 44 / all | `byte-identity/fs` | expanded (was ~5) |
| general | 18 / all | `byte-identity/general` | new (always-in-scope: len/toString/to*/typeName/error/is*) |
| http | 14 / 26 | `byte-identity/http` | new |
| io | 15 / 17 | `byte-identity/io` | new |
| json | 4 / all | `byte-identity/json` | new |
| math | 21 / 106 | `byte-identity/math` | new (Integer/Float/Fixed/Money + SIMD list overloads) |
| money | 3 / 3 | `byte-identity/money` | new |
| net | 22 / all | `byte-identity/net` | expanded (was ~6) |
| os | 15 / 15 | `byte-identity/os` | expanded (was ~5) |
| regex | 4 / 6 | `byte-identity/regex` | new (`language` topic excluded) |
| strings | 39 / 42 | `byte-identity/strings` | new |
| term | 17 / 17 | `byte-identity/term` | new |
| thread | 12 / 28 | `byte-identity/thread` | new (+ worker `.mfp`: worker-side handle overloads) |
| tls | 8 / 14 | `byte-identity/tls` | expanded (was 4) |
| vector | 19 / 159 | `byte-identity/vector` | new (Float/Fixed/Integer × 2D/3D/4D) |

**Covered-elsewhere / no byte-identity fixture (recorded, no silent gap):**
- `app` (2: getMode/setMode) — importable only in an `--app`/`"mode":"app"` build,
  and app mode **cannot cross-compile** (linux `-target … -ncode` fails; riscv64 app
  mode is unported per bug-117.1), so a 4-target `.ncodesum` fixture is impossible.
  Front-end codegen covered by `syntax/app/app_mode_surface_valid` (`.ast`/`.ir`);
  native app-mode codegen by `syntax/app/macos-app-mode-*` (`.app.ncode`, macOS).
- `errorCode` — named Integer constants only, **0 functions**; a constant reference
  lowers to an immediate, no out-of-line symbol to gate.
- `testing` — assertion builtins (`expectEqual`, …) are valid **only inside a TCASE
  body**; they are a compile error in a normal `FUNC`, so they cannot appear in an
  executable byte-identity fixture. Covered by the test-framework's own fixtures.

Acceptance: a complete package × function × overload matrix with a coverage
verdict per symbol; the fixture work-list falls out of it. **Met.**

Commit: —

### Phase 2 — expand the 6 package fixtures to full function/overload coverage

Golden type is unchanged (`.ncodesum` hash). This phase only grows coverage.

- [x] Grow each package's `main.mfb` to both `IMPORT` the package and call every
      function + distinct overload from the Phase 1 matrix, with valid typed call
      sites and resource ceremony where needed. Keep compile-only. (audio, crypto,
      fs, net, os, tls — see matrix.)
- [x] Re-seed the four target `.ncodesum` goldens per fixture. **Deviation from the
      plan:** box 2229 is NOT needed — all four targets' `.ncode` dumps regenerate
      on the macOS host via `mfb build -ncode -target <t>` (the gate already does
      this), so `macos-aarch64`, `linux-aarch64`, `linux-x86_64`, `linux-riscv64`
      sums are all seeded locally. Seeding mirrors the gate's build sequence (prime
      `-ast -ir`, then one `-ncode` per target with a native-artifact clean before
      each) to avoid an incremental-cache skip on rapid target-switching.
- [x] `scripts/artifact-gate.sh` → `diffs=0`; hash-flip demonstrated (see Phase 4).

Acceptance: each of the 6 package fixtures covers 100% of its package's symbols per
the matrix; gate `diffs=0`; hash-flip demonstrated. **Met.**
Commit: c0d0f6b51

### Phase 3 — add fixtures for uncovered packages

- [x] Added 15 new byte-identity fixtures (bits, collections, csv, datetime,
      encoding, general, http, io, json, math, money, regex, strings, term, thread,
      vector), each `IMPORT`ing its package and exercising every function/overload,
      with all four target `.ncodesum` goldens seeded (4/4 each — including thread's
      cross-target builds through its worker `.mfp`).
- [x] Skipped packages recorded in the Phase 1 matrix (`app`, `errorCode`,
      `testing`) with the reason and the fixture that covers them where one exists —
      no silent gaps.

Acceptance: the matrix shows no uncovered native-code symbol (except the app-mode
surface, which is inherently non-cross-compilable and covered by syntax/app); every
added fixture has all four target `.ncodesum` goldens. **Met.**
Commit: c0d0f6b51

### Phase 4 — full validation

- [x] `scripts/artifact-gate.sh target/release/mfb` → `diffs=0`, repeated 3×
      (identical: `1087 tests, 1206 build(s), 1436 golden(s) checked, 0 diff(s)`).
      Hash-flip demonstrated: adding one `bits::popCount(b)` call to the bits
      fixture flipped its macOS `.ncodesum` (`9f1078fd…` → `7a2f0a4a…`); reverted.
- [x] Acceptance: `scripts/test-accept.sh … 'byte-identity/*'` →
      `acceptance tests passed (23 test(s) ran)` — all fixtures compile-only (no
      `.run` golden, no `entry` execution). In a full-suite run all 23 byte-identity
      actuals matched their goldens (69/69 build.log/.ast/.ir) before the run wedged
      on the PRE-EXISTING, unrelated `rt-behavior/threads/thread-return-fixed`
      runtime-execution flake (a sandbox SIGKILL/hang on the executed thread binary,
      not touched by this bug — all changes are under `tests/byte-identity/`). No
      `src/` changed, so `cargo test` is unaffected.
- [x] `linux-*` goldens: **box 2229 NOT required** — all three `linux-*` `.ncode`
      dumps regenerate on the macOS host via `-target`, seeded locally (4/4 per
      fixture). This supersedes the doc's older box-2229 note.
- [x] Update memory `fast-codegen-gate` — the "gate is nearly blind to codegen"
      caveat is now materially reduced.

Acceptance: full suite green; gate `diffs=0` and stable; the coverage matrix is
100% for native-code symbols (except the inherently-non-cross-compilable app-mode
surface); memory updated. **Met.**
Commit: c0d0f6b51

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
