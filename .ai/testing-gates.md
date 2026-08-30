# Testing, goldens & gate mechanics

Reference for the MFB compiler's test/golden/gate harness — the codegen artifact-gate, byte-identity goldens, the acceptance/test-accept harness, and the concurrency and staleness hazards around them.

## The artifact-gate (`scripts/artifact-gate.sh`)

`scripts/artifact-gate.sh` is the codegen gate: it runs only execution-free `mfb build` dumps (`-ast -ir -br -nir -nplan -nobj -ncode -mir`; front-end + codegen, NO link, NO execution), diffs deterministic artifact goldens, and CANNOT be killed by a runtime test. Use it — not `test-accept.sh` — for any change that only affects codegen / IR / lowering. It needs an explicit `<mfb-exe> <builtin|all>` selector: a bare invocation prints usage and runs NOTHING.

### When to run it
- Per individual piece / phase: use FAST targeted checks (seconds), NOT the full gate. The scoped gate `artifact-gate.sh <exe> <builtin>` (e.g. `collections`, ~1 min), plus `test-accept.sh` on the touched fixtures and `cargo test --bin mfb`. To prove blast radius is contained, rebuild ONE representative *unaffected* fixture (`byte-identity/strings`) and confirm its `.ncode` sha still matches its golden.
- The full `artifact-gate.sh <exe> all` (~15–20 min) runs ONCE per major section/grouping boundary and at finalization — the authoritative tree-wide proof before merge. Do NOT run it per sub-phase. When waiting on it, kick it off in the background and do other work / give an ETA — don't chain 600s foreground until-loops that time out and re-block.

### Coverage & the `.ncodesum` goldens
- Byte-identity fixtures `tests/byte-identity/<pkg>/golden/*.ncodesum` (×5 targets: `macos-aarch64`, `linux-x86_64`, `linux-aarch64`, `linux-riscv64`, `windows-x86_64`) cover **22 builtin packages** (audio, bits, collections, crypto, csv, datetime, encoding, fs, general, http, io, json, math, money, net, os, regex, strings, term, thread, tls, vector) + `rt-behavior/crypto/crypto-ec-valid`. The gate `shasum -a 256`s the built `.ncode` and compares — so a codegen change to any covered builtin flips at least one hash and goes red. All five targets regenerate on the macOS host (`-ncode` is execution-free; no box 2229 needed for sums).
- **Per-target fixture counts (not uniform):** each of the four Unix targets carries all 24 fixtures; `windows-x86_64` carries **21** — `os` (`os.resourcePath`), `process` (`process.shell`), and `link-const-pins` (native lib `c`, no Windows locator) use runtime surface the Windows backend does not support, so no `windows-x86_64.ncodesum` exists for them (they are execution-non-buildable there, not just skipped). Adding a `<pkg>.<target>.ncodesum` under `golden/` is what makes the gate build+sum that target for that fixture — the target list per fixture is derived from the golden filenames (`artifact-gate.sh`), so a missing golden silently drops that target. `thread` gained its `windows-x86_64` golden once the `thread.*` Windows import set declared the stdin-broadcast heap/console symbols (it drives the same stdin log as `io.input`).
- `.ncodesum` is an **artifact-gate-ONLY** signal: `test-accept.sh` does NOT read it (it only adds `-ncode` when a `.ncode` golden is present). The `.ncodesum` hash is a change-SENTINEL — it says *something* moved, not *what*; localize with stash → `mfb build -ncode` → diff.
- Gotcha: `mfb build -ncode` writes `$pkg.ncode` (no target infix); the golden is `$pkg.<target>.ncode`. The gate maps infix-less actual → infixed golden.
- Green means "nothing *covered* changed" — NOT 100%. NOT covered (record before trusting green for a site): `app` (app-mode only, non-cross-compilable — covered by `syntax/app/*`), `testing` (TCASE-only), `errorCode` (constants, no code symbols), any compiler-internal path not reached by a builtin call, and `tests/acceptance/**` (no `golden/` dir → the harness runs `mfb test` and compares nothing; do NOT add a golden there — presence of `golden/` is a mode switch and that project declares no `entry`). A real false negative: `strings::graphemeAt` lived only in `tests/acceptance/`; 0 diffs across 1217 goldens while a real `mul` operand-order change existed. Before trusting the gate for a site, confirm it's reachable from a test that owns a `golden/`; where it isn't, add a `tests/rt-behavior/**` fixture, or build a scratch project + `mfb build -ncode` and diff against the pre-change compiler (`git show <commit>^:<path>` — and CHECK THE BUILD SUCCEEDED; a failed build silently reuses the old binary and fakes a match).

### Broadly-emitted changes break goldens BEYOND byte-identity/
The `byte-identity/<pkg>` fixtures are a per-package SMOKE test, not the whole golden surface. `syntax/**` and `rt-behavior/**` also carry `.ir` host dumps and `.app.ncode`/`.ncodesum` native goldens the gate checks. A change to a broadly-emitted path breaks those too, and a per-phase check that only regenerates `byte-identity/` MISSES them — the full gate is the only authoritative finder. `sync-goldens.sh` refreshes `.ir`/`.ast`/`build.log` for byte-identity + rt-behavior but SKIPS `syntax/**` and does NOT refresh target-infixed `.ncodesum` or `.app.ncode` — regen those by hand (`mfb build -q -ncode [-target T] [--app]` then `cp`/`shasum` into `golden/`). Rule: after any shared-codegen change, run the FULL gate once before merge and regenerate every fixture it flags, tree-wide.

### The gate is BLIND to diagnostic prose (run test-accept for those)
The gate checks CODEGEN; it cannot see the error message of an *invalid* program. A metadata/diagnostics migration (e.g. collapsing `expected_arguments`, which dropped the `[optional]` bracket: `strings.find` → `"String, String, Integer"` instead of `"String, String[, Integer]"`) is INVISIBLE to the gate — only `test-accept.sh` catches it. Also, deleting dead wrappers can break CROSS-MODULE tests invisibly to `cargo build --bin mfb` warnings; `cargo build --bin mfb --tests` is the real check.

### Concurrency & macOS hazards
- Do NOT run two artifact-gates at once (yours + another session/worktree): they saturate cores and one gets KILLED mid-run. Exit 144 / 0-byte output = infrastructure kill, NOT a codegen regression — re-run alone, don't treat it as a diff (a real diff prints `artifact-gate: N tests … M diff(s)` + DIFF lines). Check `pgrep -f artifact-gate` (and `ps -o command=`) first. A killed run leaves stray untracked dump files (`tests/byte-identity/.../*.ncode`) — clean before re-running.
- Never run `cargo` (even `cargo check`) while the gate/acceptance is using `target/debug/mfb`: macOS invalidates the in-place-modified Mach-O's signature, so every subsequent harness exec is SIGKILLed and the suite silently stops producing actuals (looks like a pile of "missing actual"). If the binary's mtime changed mid-run (`stat -f "%m" target/debug/mfb`), discard that run's actuals.

### A/B two artifact baselines (Linux)
To show a codegen fix changed only what it should: `scripts/linux-artifact-baseline.sh <old-mfb> capture /tmp/a.manifest` with the OLD compiler *before* rebuilding, again with the new, then `diff` the files yourself (`verify` mode only prints `head -80` and deletes its temp manifest). 11,154 hashes across all three Linux targets, fully local — replaces hours of emulated-hardware runtime proof (e.g. exactly 19 of 1014 fixtures, all reaching `fs::tempDirectory`). Note the `JOBS=10` + release-binary caveat for the Linux boxes (only one box has a Rust toolchain; cross-compile and ship).

### Scope: execution-free BY DESIGN
`artifact-gate.sh` and `test-accept.sh` both source `scripts/artifact-kinds.sh` (one table of dump kinds: host `ast ir hex`, native `nir nplan nobj ncode mir`, app `nir nplan ncode`; `hex`→`-br`, else `-<kind>`). Add a new codegen-dump kind = edit ONE file. The 11 kinds NOT in the gate (`mfp`, `info`, `audit`, `testrun`, `covmap.json`, `covdata`, `covfail`) each need a link / `pkg info` / `audit` / `test` RUN step, so they're deliberately out of scope — do NOT "bring the gate to 19 kinds"; its `checked` count is correct. Baseline is literal `diffs=0`.

## Linker-stage changes are invisible to the artifact-gate

The `.ncodesum` artifact-gate golden is generated PRE-link. A `write_executable` section/header change cannot shift it, because the gate stops at the codegen dump stages before any linking happens, and there is no full linked-executable golden anywhere in the tree. To verify a linker-stage change, dump a real *built* executable (section/header layout of an actually-linked exe) and inspect that — do NOT rely on the artifact-gate, which will stay green regardless.

## Byte-identity: targets it gates & what churns the goldens

Byte-identity = the committed `.ncode` / `.ncodesum` codegen goldens (checked by `scripts/artifact-gate.sh`). This is about WHICH targets it gates and WHAT legitimately churns the goldens.

### Which targets byte-identity gates
- The byte-identity gate that MATTERS most is **AArch64 + RISC-V** (`linux-aarch64` / `linux-riscv64`). SysV-x86 (boxes 2227/2228) codegen is proven by rt-behavior / execution, not bytes.
- **`windows-x86_64` now DOES carry byte-identity `.ncodesum` goldens** (21 of the 24 fixtures — see the per-target counts above). They are a per-target DETERMINISM/CHANGE sentinel: a Windows codegen change flips the hash and reds the gate, so regenerate the `windows-x86_64.ncodesum` like any other target (macOS host, `-ncode` is execution-free). This UPDATES the older "Win64 byte-identity is a non-goal / proven by execution not bytes" stance — Windows codegen IS change-gated by bytes now.
- The non-goal that REMAINS: do not *shape a design* "to keep Windows byte-identical across a change" or treat a Windows golden churn as a defect to avoid — a churn just means "regenerate the golden," never "revert the change." The sentinel catches DRIFT, it is not a stability constraint.
- Byte-identity is about BYTES, not CORRECTNESS. Windows must still be EXECUTION-verified on box 2230, or a real regression hides: deleting the x86 fixpoint once made Win64 produce ZERO output (broken arena), undetected because Win64 checks were skipped as "non-goal". Gate for ANY ABI/codegen change: `cargo test` + AArch64/RISC-V byte-identity + SysV-x86 (2227/2228) AND Win64 (2230) EXECUTION.

### Win64 codegen invariant
The `win_x86_64` HAND-WRITTEN emitters read Win32 call results (GetStdHandle/WriteFile/VirtualAlloc/CreateFileW/GetFileAttributesW/…). Those land in the C-return register (`rax`) — read them via the EXPLICIT `c_return(0)`, never the generic `return_register()` (which is the aligned MFB result `rcx` on Win64, once the CFG fixpoint is gone). Win64 MFB args AND results are aligned to the call bank (rcx,rdx,r8,r9); a variable-shifted result can land on rcx, so the x86 encoder's `var_shift` shifts in a scratch when `dst==rcx`.

### What churns the goldens #1: the embedded utf8proc table
Changing ANY byte of the embedded `_mfb_unicode_properties` table (e.g. packing new bits into `flags`) shifts the `.ncode`/`.ncodesum` of **every fixture that embeds the table** — far more than the `strings::` unicode callers, because the table rides in ~any binary doing string work (`toString`, error messages). Measured: **11** fixtures across all targets — `byte-identity/`{strings, regex, crypto, csv, datetime, encoding, http, json, net, term} + `rt-behavior/crypto/crypto-ec-valid` (+ app-mode `macos-app-mode-term`). `os`/`audio`/`tls`/`math`/… do NOT embed it. Confirm membership: `mfb build -q -ncode <fixtureDir>` then `grep -oE '_mfb_unicode_[a-z0-9_]+' <pkg>.ncode | sort -u` (symbol names appear in the `.ncode` TEXT dump but NOT `.nobj`, which references data by offset — scan `.ncode`). Structural changes count too: removing a whole data object (e.g. dropping `_mfb_unicode_sequences`) or per-table feature-gated emission (a table emits iff a function relocates against its symbol) shifts the same 11. All 5 targets shift equally (table bytes are target-independent); only `nobj`/`ncode` stages embed it (`.ast`/`.ir`/`.nir`/`.nplan` do NOT).

### What churns the goldens #2: label renames (no machine-byte change)
The `-ncode` output is a **textual JSON instruction dump** that contains label names verbatim (`{"op":"label","name":"..."}`, `{"op":"b.eq","target":"..._zero_skip"}`). So renaming a label — e.g. collapsing an inline loop onto a shared emitter whose label suffixes differ — changes the `.ncode` dump and its `.ncodesum` **even though the final encoded machine bytes are identical** (labels resolve to offsets; same instruction count → same offsets). Consequence: a "generic emitter" merge that only differs in label spelling is NOT free — it churns a committed golden. If the merge serves no structural goal, keep the loop inline (e.g. `crypto.rs` randomBytes zero-loop is kept inline). The `.ncodesum` hash catching label renames is CORRECT, not a false positive; to prove machine-byte identity independent of labels you'd diff `.nobj`/final bytes, but treating any `.ncode` change as golden churn is the right conservative stance.

### Regenerating the goldens
- The committed `.ncode`/`.ncodesum` are **RELEASE**-generated. Debug and release `mfb` from the SAME source emit identical `.ncode` (verified), but always run `scripts/artifact-gate.sh target/release/mfb all` for a true 0-diff.
- **Two regen scripts, and `regen-ncodesum.sh` alone is NOT enough** (plan-99). It sweeps only `tests/byte-identity/*/golden/*.ncodesum`; the `.ncode`/`.ncodesum` goldens that live elsewhere — `rt-behavior/crypto/crypto-ec-valid` (4 targets) and `syntax/app/macos-app-mode-{io,plumbing,term}` (incl. `--app` targets) — stay stale and keep `artifact-gate all` red after a "full" regen. Run `scripts/regen-outside-ncode.sh target/release/mfb` for those (same contract, decodes the `.app` target suffix), then re-run the gate to 0.
- **Classify before you regenerate.** Run `artifact-gate.sh <exe> all` from a detached main-tip worktree FIRST (`git worktree add --detach /tmp/<x> main`, release build). If that baseline is `0 diff(s)`, every diff on your branch is yours and regenerating is correct; a baseline diff is a pre-existing stale golden and must be classified separately. A one-instruction change in the program entry (e.g. the `entry_error_code_write` staging) churns EVERY fixture on EVERY target — 125 goldens in plan-99 — because the entry stub is in every binary; that is expected, not a red flag.
- `artifact-gate.sh` has **no accept/write mode**. Regenerate a `.ncodesum` by building the target's dump and writing its sha256: for each target token `t` in `golden/<pkg>.<t>.ncodesum`, `mfb build -q -ncode [-target <t>] [--app if t ends .app] <fixtureDir>` writes `<fixtureDir>/<pkg>.ncode`; then `shasum -a 256 … | cut -d' ' -f1 > golden/<pkg>.<t>.ncodesum`. Host target = `macos-aarch64` (no `-target`). Raw `.ncode` goldens (small backends) are `cp`'d instead of summed.

## Acceptance golden harness mechanics (`scripts/test-accept.sh` / `sync-goldens.sh`)

**`MFB_OPT` — the opt-level global switch (plan-100).** `MFB_OPT=<n>` appends
`-O<n>` to every `mfb build` the loop runs, mirroring `MFB_TARGET`. Unset (the
default) appends nothing, so the harness runs the exact command it always has and
the binary applies its own default of `-O1`. It is deliberately NOT echoed into
the `$ mfb build …` label the way `target_label` is: `build.log` is itself an
exact-compared golden, so echoing `-O1` would drift every fixture's build.log
under `MFB_OPT=1` and break the very "explicit `-O1` == default" gate that run
exists to prove. `MFB_OPT=0` is a **correctness** run, not a byte-identity run —
`.ncode` artifacts are expected to drift (the dial passes are off) and must NOT be
re-baselined; what must hold is that every fixture builds and behavior matches,
with **no exceptions** — both dial rows are behavior-preserving. If a `-O0` run
ever shows a *behavior* mismatch, that is a real bug, not an expected difference.
(FMA contraction looks like a counter-example and is not: it changes float results
— `a*b-c` staying finite vs. trapping `ErrFloatOverflow` — which is exactly why
`fuse_scalar_fma` is **not** on the dial. It is mandatory lowering in
`src/codegen/compiler/opt/`, pinned by `rt-behavior/arithmetic/float-fma-fusion`
plus `rt-error/arithmetic/arithmetic-float-fma-observed-rt`.)

**The fixture count is a signal.** The summary line is `acceptance tests passed
(N test(s) ran)`. If `N` moves between two runs of the same tree, the harness is
losing fixtures, whatever the pass/fail says — that is how a stdin bug that had
been silently skipping 72 fixtures was found. Every subprocess in the loop body
must get `/dev/null` stdin (via `run_with_watchdog`, or an explicit `</dev/null`),
because the driving loop reads its fixture list from a `find` pipe on fd 0 and a
fixture that reads stdin will eat it.

Generating acceptance goldens for a NEW `tests/**` fixture:

- **`scripts/diag-set-diff.sh <mfb> [-v] [globs]` is the diagnostic-relocation gate** (plan-107): for every golden `build.log`/`test.log`/`*.testrun` carrying a diagnostic it re-runs the echoed `mfb build …`/`mfb test …` command (no `-q` for `test`) and classifies the fixture SAME / REORDER (same multiset, different order — the expected churn when a rule moves between `ir::shape` and `ir::verify`; regenerate those goldens and prove them pure line moves) / SETDIFF (a rule went missing, doubled, or changed wording — a bug). It records `[exit N]` and unlocated `error:` lines. Blind spot: a fixture whose golden is CLEAN is never run, so a relocation that introduces a false error on a compiling program (e.g. a package/resource typing change) shows up only in `artifact-gate all` (MISSING `.ast`/`.ir`) or `test-accept` — run the gate before landing such a change, not at the letter's end.
- **`scripts/sync-goldens.sh` honors its name filters:** `sync-goldens.sh <exe> <name-glob>...` forwards the globs to test-accept.sh, so a single-fixture sync is ~4s (was the full ~15-min cycle). Args are arg-shape agnostic — basename, full `tests/`-relative path, or glob all work; no glob still syncs everything. It refreshes only files that already exist in `golden/` and never creates new ones, and never touches `.run`. For a brand-new fixture create the placeholder golden files first (`ast`, `ir`, `build.log`, optional `.run`), then run the filtered sync.
- A golden-bearing fixture needs `golden/{ast,ir,build.log}` + optionally `<pkg>.run`. Create `project.json` + `src/main.mfb` plus empty placeholder golden files so the harness knows what to produce.
- **The `<pkg>.run` golden is a TRIGGER FLAG, not diff-compared.** Its mere existence makes the harness build+run the exe; the run's stdout is captured into **`build.log`** (which IS compared). Keep `.run` accurate for docs but a content mismatch there never fails the suite.
- Actuals land in `$ACTUAL_DIR/<test-rel-path>/{<pkg>.ast,<pkg>.ir,build.log}`. No `.run` actual is emitted.
- **What makes the harness run the PLAIN `mfb build`** (vs. the `-ast -ir` dump pass): a `golden/<pkg>.mfp` OR `.info` OR `.run` exists. This matters for a fixture whose diagnostic only fires at package-emit time — the `-ast -ir` pass never reaches that code, so with no such golden the fixture silently proves nothing. Use `.mfp` when the build SUCCEEDS (it also pins emitted bytes); use `.run` when it FAILS (no `.mfp` is produced).
- **An unexpected actual is a FAILURE**, not a no-op: `compare_optional_output` fails with "unexpected actual" when an actual exists with no golden. A new fixture needs placeholders for EVERY artifact the harness produces.
- On macOS aarch64 host only aarch64 native goldens are checked — x86/riscv `.ncode` shifts don't appear.
- **test-accept.sh does NOT understand `.<ext>sum` goldens; artifact-gate.sh does.** The harness only checks the raw `.<ext>` golden (e.g. `.app.ncode`) when deciding which `-<flag>` to request and what to diff. So a fixture whose codegen dump is committed as a checksum (`.app.ncodesum`, because the raw dump is tens of MB) gets NO `-ncode` request from test-accept — the ncodesum is verified only by artifact-gate. Consequence: a sum-only fixture's `build.log` golden must NOT contain an `-ncode` command / "Wrote native code plan" line, or the harness (which no longer emits it) reds on the build.log diff. This bit `syntax/app/macos-app-mode-term` — its build.log was left referencing `-app … -ncode` after the `.app.ncode`→`.app.ncodesum` migration, while 116 console byte-identity siblings correctly show only `-ast -ir`. Fix = regen the build.log (drop the ncode line); ncodesum stays owned by artifact-gate.
- **Per-fixture skip hook: `test-gate.sh`.** A fixture dir may ship an executable `test-gate.sh`; test-accept runs it from the fixture dir before the fixture, exit 0 = run, non-zero = skip with the gate's stdout as the logged reason (`[skip] <name>: <reason>`, counted separately in the summary, never a failure). For environment-dependent fixtures (e.g. the live-network `rt-behavior/tls/tls-connect-google-rt`) so an offline box skips loudly instead of reding. A filter matching only skipped fixtures is NOT "no tests matched".

### Registry/attestation fixtures need a hermetic MFB_HOME
`mfb build` verifies an imported package's attestation against the key pinned at `$MFB_HOME/<sha256(repo-url)>/server.pub` (`local_paths_for_repo` in `src/cli/mod.rs`), defaulting to `$HOME/.mfb`. The harness never isolated this, so a fixture's output depended on the **machine**, not the code. Now `test-accept.sh` exports a per-run `mktemp -d` as `MFB_HOME`.

How it surfaced: changing `DEFAULT_REPO_URL` to mfb-repo.fly.dev changed which `~/.mfb/<hash>/` dir is consulted, so `pkg-01-tampered-signature` reported "invalid attestation signature" instead of "no pinned registry key". **The trap: the obvious fix was to sync the golden, and it was WRONG.** The committed golden was correct; syncing would have baked one laptop's key store into the tree. The decisive experiment is one command: run the fixture twice, once with `MFB_HOME=$(mktemp -d)`. Do that before believing any package/registry golden diff.

Note also that a `repository/`-only change CAN turn the main acceptance suite red — run it too, don't trust "repository suite 164 pass" alone. When waiting on a run, poll the output FILE in the foreground (every `pgrep`-based waiter self-matches). Never run cargo during a run.

## Perf goldens break execution acceptance

Running `scripts/test-accept.sh <exe> <dir>` for a FULL execution pass is **inherently noisy** at baseline and cannot reach 0 mismatches — do not treat a clean full test-accept as the acceptance gate.

**Why:** a debug-gated `_mfb_rt_perf_*` table is printed at program exit (macOS). Many executing fixtures' `build.log`/`.ncode` goldens were seeded with a *debug* mfb, so they carry perf symbols (`.ncode`) and a perf table with **run-varying nanosecond timings** in `build.log` (`program 1 37000` one run, `29000` the next). A **release** mfb strips the perf table entirely; a **debug** mfb reproduces it with different numbers — neither profile diffs clean. Verified identical on pure `main` (`git worktree add --detach`), so it is a pre-existing baseline, not any one plan's regression. `tests/common/mfb_exe()` resolves **release** precisely to keep the cargo-test acceptance deterministic (no perf).

**The real gates (use these):**
- Full `cargo test` (behavior + IR + citation gates). Green = the executable proof.
- `scripts/artifact-gate.sh target/debug/mfb` — codegen byte-identity, execution-free (~10min). Run with **debug** so its `.ncode`/`.ncodesum` goldens (which carry debug perf symbols) match.
- For a NEW rt-behavior fixture: seed its goldens with the **release** mfb (`sync-goldens.sh target/release/mfb <glob>`) so the program output is deterministic (no perf table), and verify with `test-accept.sh target/release/mfb`. A new fixture needs its `golden/` dir pre-created with empty placeholder files (build.log + `<name>.ast`/`.ir`/`.run`) — sync-goldens only *refreshes* existing golden files, never creates them.

**Known-stale (macOS host):** `{audio,http,json,net,regex,strings}_codegen_cover_rt.macos-aarch64.ncodesum` byte-identity goldens differ from a locally-rebuilt macOS mfb (regen'd on another host/profile). Pre-existing on main; not something a feature branch introduced.

## `mfb_exe()` reuses a stale release binary

`tests/common/mod.rs::mfb_exe()` resolves the **release** binary and builds it only inside `BUILD_RELEASE_MFB.call_once`, which early-returns when `target/release/mfb` already exists (`mod.rs:610` region). So once that binary exists it is **never rebuilt** for the life of the target dir — `cargo test` (which builds debug) will happily run any subprocess/CLI integration test (`cli_json_depth_limit.rs`, etc.) against a stale release binary.

Two phantom reds this produced, both pure staleness (not regressions):
- Built release pre-merge, then `git merge main` pulled a sibling's JSON-depth-limit fix + its new test; the stale binary lacked the guard and aborted (SIGABRT/stack overflow) while the fresh debug binary reported the bounded error correctly. Looked like the change broke an unrelated test.
- Full `cargo test` on main returned `CARGO_EXIT=101` with the new CLI depth tests panicking "killed by signal" — `target/release/mfb` was days old (pre-fix). Unit tests passed; only the release-subprocess tests failed.

**How to apply:** before trusting a full-suite result that includes any `mfb_exe()` subprocess test, `cargo build --release --bin mfb` first (or check `ls -la target/release/mfb` mtime vs your last edit). A signal/behavior RED from a subprocess test on an otherwise-green tree is a stale-release smell → `rm target/release/mfb` and re-run. Debug-path repro (`target/debug/mfb`) stays correct because you rebuild it explicitly. Real CI with a fresh target dir never hits this; confirm at HEAD via a detached worktree.

## Compiler tests live in the bin target

`mfb` has **no `src/lib.rs`** — it is a binary crate (`src/main.rs` with `mod arch;`). So all the compiler/codegen/native-encoder unit tests (e.g. `arch::x86_64::encode::tests::*`, `#[cfg(test)] mod tests;`) compile into the **bin** test target.

**How to apply:** to run/filter a single compiler test fast, use `cargo test --bin mfb <filter>`. `cargo test --lib` runs the `mfb_repository` sub-crate's lib (~313 tests) and will report `0 tests / N filtered out` for any compiler test name — misleading, looks like the test vanished. Plain `cargo test` runs everything (bin + repository lib + `tests/rt_native_*` integration bins) and is the full-suite gate.

## A split can break ONLY the test build

A pure file split can leave the **release build + artifact-gate + acceptance all green while the TEST build is broken**. This happens when a `#[cfg(test)] mod tests` stays in file A but exercises private helpers that the split moved to sibling file B: `cargo build --release` never compiles test modules, and the `.mfb` acceptance suite runs the compiled compiler (not the Rust unit tests), so neither catches it. This shipped once when `package.rs`'s tests called `percent_decode_path`/`insert_package_dependency`/… after they moved to `url.rs`/`json_edit.rs`.

**Why it slipped:** the verify saw "acceptance tests passed" and *inferred* the `citations_resolve` cargo-test passed, but that test's build had a compile error. Inferring a cargo-test result from the acceptance result is invalid — they compile different things.

**How to apply:** for every split, after acceptance, **confirm an actual `test result: ok` line** from a `cargo test -p mfb --bins <module>::` run that compiles the split area's test modules — do not infer it. If a moved private fn is used by a test that stays behind, either move the tests with the fn or widen the fn to `pub(super)` and import it into the test module.

## Splits must sweep man AND spec citations

When splitting/moving a file, OR removing/renaming a stdlib symbol, sweep `[[path:Symbol]]` provenance citations in **both** `src/docs/spec/` AND `src/docs/man/`, then run **both** `spec_citations_resolve` and `man_citations_resolve` (`cargo test -p mfb --bins citations_resolve`).

**Why:** the two guard tests differ in strictness and it bites you.
- `spec_citations_resolve` (src/docs/spec/mod.rs) is **file-level only** — it passes as long as the cited file exists, even if the symbol moved out of it.
- `man_citations_resolve` (src/docs/man/mod.rs) is **symbol-level** — it fails if the symbol isn't actually defined in the cited file, and FAILS the whole `cargo test` (not just acceptance).

So a split that only sweeps spec/ leaves man/ citations broken, and the file-level spec test won't warn you. The tooling `scripts/fix_citations.py` is **broken** (its `SPEC_DIR` resolves to `src/spec`, but the spec lives at `src/docs/spec`), so it finds zero citations — do the repoint by hand.

**How to apply:** after a move, `grep -rn "\[\[.*<oldfile>" src/docs/spec src/docs/man`, map each symbol to its new file (grep the actual definition — a symbol can land in a different file than the doc's suggested name, e.g. crypto `ed25519Sign` ended up in `crypto_ecdsa.mfb`), repoint, rebuild (docs are embedded), run both tests. Citations are stripped at render time so repointing them changes no golden.

**Not just file moves — REMOVALS/RENAMES too.** Deleting or renaming a *private* stdlib helper (`__crypto_*`, `__http_*`, `__csv_*`, …) breaks any man/spec page that cites it by symbol, and `man_citations_resolve` FAILS the whole `cargo test`. E.g. removing `__crypto_bytePrefix`, `__crypto_pbkdf2Block256/512`, `__csv_crChar`, `__http_crlf` — each was cited from `src/docs/man/builtins/crypto/{generateP*,pbkdf2Sha*}.md` and `src/docs/spec/stdlib/`; the acceptance suite stayed green, only `cargo test` caught it. So after ANY stdlib `.mfb` dedup/inline, `grep -rn "__<pkg>_" src/docs/` for every symbol you removed and repoint to the surviving one, then run the citation tests.

## Concurrent test-accept clobbers actuals

On this SHARED machine other agents run their own `test-accept.sh` concurrently (seen: an `agent-*` worktree doing Windows cross-compile + ssh to port 2230). If a foreign run and yours both write the **same** `target/accept-actual` dir, they clobber each other's per-test output mid-check, yielding phantom `missing actual rt-behavior/.../x.ast` / `.ir` mismatches on fixtures **unrelated to your change** (e.g. `money_inexact_float_warn` "failed" this way while only `collections::partition` was touched).

**Why:** `test-accept.sh` runs each test, writes `target/accept-actual/<rel>/*`, diffs, then `remove_output_dir`s it. A second concurrent run racing the same dir deletes files the first is about to diff.

**How to apply:** before trusting an acceptance failure, (1) check the failing fixture actually exercises your change (`grep` it); (2) re-run JUST that fixture into a **private** actual dir: `scripts/test-accept.sh <exe> /tmp/accept-check <glob>` — a pass there proves it was a concurrent-clobber false failure, not a regression. Also: `pgrep -fl test-accept` to see if a foreign run is live, and wait on YOUR specific PID (`until ! kill -0 <pid>`) not `grep -q test-accept` (which matches foreign runs and never fires).

## The `.run` golden is an empty marker

When hand-validating a fixture's runtime output, do NOT diff against `golden/<pkg>.run` — that file is a **zero-byte marker** whose mere presence tells `test-accept.sh` to build, run, and capture. The **expected program stdout is in `golden/build.log`**, between the bare `$ .../build/<pkg>.out` run line and its following `[exit N]`. Extract it with:

    awk '/^\$ .*\/build\/.*\.out$/{c=1;next} c&&/^\[exit /{c=0} c{print}' golden/build.log

Diffing riscv64 output against the empty `.run` once produced ~11 wrongly-reported "failures" that were actually correct (identical to build.log). Also: riscv64 runtime is NOT exercised by the acceptance suite (rv64 fixtures carry only compile-only `.ncodesum` goldens), so to validate rv64 behavior you must build `-target linux-riscv64`, ship to 2229, run, and diff against the build.log run output yourself — and beware host-path/thread-timing fixtures that differ for environmental reasons (confirm with an arena-vs-HEAD output diff, not vs golden).

## exe-oracle concurrent clobber

`scripts/exe-oracle.sh <exe> <target> record` builds every fixture into the fixture's own `tests/<fixture>/build` dir, which is **NOT namespaced by target**. So running two `record` (or the `bug387-gate` compare, or any sweep) for DIFFERENT targets at the same time makes them build the same fixture into the same `build/` dir simultaneously — they clobber each other's `.out`, and `shasum` then hits "No such file", silently DROPPING entries from the baseline (e.g. 1315 lines instead of 1320, missing 5 glibc variants).

**Why:** a corrupt baseline makes the very next `bug387-gate.sh full` show a false DIFF (missing/wrong hashes) that looks like a real byte-identity regression but isn't.

**How to apply:** run every corpus sweep/record/compare **SERIALLY**, one target at a time (a driver script looping `for t in …; do exe-oracle … record …; done`). Also: do NOT `nohup … &` inside a `run_in_background` Bash call — the double-background orphans the real work to PPID 1 and the harness reports premature "completed" for the launcher shell.

## Acceptance pre-existing reds baseline

A plain `scripts/test-accept.sh` on the macOS host reports **4 pre-existing mismatches unrelated to any given change** — reproduced identically on a clean detached base checkout:

- `rt-behavior/native/libsnd-load-sound-rt` + `rt-behavior/native/libsnd-playback-rt` — `error: PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: ResultValue is annotated \`SoundFile\` but its Result carries \`SoundFile STATE SoundInfo\`` at build.
- `rt-behavior/native/native-link-inline-trap-rt` — same class, `Db` / `Db STATE DbInfo`.
- `rt-behavior/tls/tls-connect-google-rt` — needs a live network peer (google:443).

The three `native` ones are a real, deterministic native-resource STATE binary-representation red on main (a different subsystem from most work); the tls one is environmental. **Before treating an acceptance mismatch as your regression, prove it:** `git worktree add --detach <path> <base>`, build, and `scripts/test-accept.sh <exe> <dir> "<fixture-glob>"` (test-accept takes name-glob filters) — if it fails on the clean base too, it's baseline, not you.

Also: a plain host `test-accept` only builds the HOST target (macos-aarch64), so it never checks any `*.windows-x86_64.ncodesum` — those are artifact-gate's job; `ncode-determinism-alltargets.sh`'s TARGETS list also excludes windows.

## Known-red baseline: durable lessons

Two lasting lessons survive from the citation-test baseline history:

- **A file/symbol move MUST sweep its `[[path:symbol]]` citations in BOTH** `src/docs/man/` (symbol-level, strict — fails `cargo test`) and `src/docs/spec/` (file-level). A move that commits the rename but skips the sweep turns the citation tests red on its own branch AND surfaces only when merged. When a citation test is red, run it, read the `[[path:symbol]]` list, and check whether each symbol simply MOVED (repoint by hand; `fix_citations.py` is broken). (See the "Splits must sweep man AND spec citations" section — the mechanics live there.)
- **`std::ptr::eq` on a `const`-promoted `&[...]` is unstable.** The promoted allocation is duplicated across call sites (inlining), so the same logical element resolves to two different addresses. `supported_helper_specs()` returned such a promoted array; a diagnostic showed the slice was pointer-stable across two *direct* calls yet `spec_for_call()` (a separate fn) returned a different address for the same spec. Fix = a single named `static`. If you ever compare catalogued data by pointer identity, it MUST live in a named `static`, never a promoted `&[...]`.
