# plan-131-A: scripts/ cleanup — fix what is broken, delete what is dead

Last updated: 2026-09-12
Overall Effort: x-large (1d–3d) — five sub-plans, A → E
Effort: large (3h–1d)
Depends on: nothing

`scripts/` has 71 tracked top-level files (`git ls-files scripts | grep -c '^scripts/[^/]*$'`
→ 71, plus 5 files under `scripts/bench-probes/`). Agents have used it as a dumping ground: one-off
probes for finished plans, gates for finished bugs, generators that belong with their data, and
near-duplicate scripts. `scripts/README.md` does not mention 36 of them (the README-gap loop in
§2 → 36).

**plan-131 as a whole is done when:**
- `scripts/` holds only reusable helpers and gates: 40 top-level files (37 kept + 3 merged
  replacements, per the inventory in §3);
- every file there is indexed in `scripts/README.md`;
- a `cargo test` census fails if a file is added to `scripts/` without a README entry, or if the
  README names a file that does not exist.

This sub-plan (A) does the zero-design work first: it repairs the two scripts that are broken
today, deletes the 8 files that are dead outright, and fixes the stale references. Nothing is
renamed or moved here.

References:

- `scripts/README.md` — the current (incomplete) index.
- `tests/gate/gate_lock_covers_every_writer.rs` — `CLASSIFICATION`, `SCAN_FLOOR`. This test
  names scripts by filename and panics `classified script {name} is missing` when one is deleted.
- `.ai/testing-gates.md` — cites `bug387-gate.sh`, `exe-oracle.sh`, the regen scripts, and
  `ncode-determinism-alltargets.sh`.
- `AGENTS.md` "Never edit a test/golden to pass" — governs the `SCAN_FLOOR` edit in Phase 2.
- Memory: `never-run-treewide-scripts-unchecked`, `peer-sessions-share-main-checkout` — peers share
  this checkout; stage only files this plan touches.

## Prerequisites

This table is the gate for the whole plan-131 series. Sub-plans B–E point here.

| Must be true | Command | Status |
|---|---|---|
| No open peer work edits the scripts this plan deletes or moves | `git status --short scripts tools tests/gate` → empty | MET (2026-09-12 re-run at start of execution: empty in both the main checkout and `.claude/worktrees/P-131`) |
| Release binary builds at HEAD | `cargo build --release` → exit 0 | MET (2026-09-12 re-run in the worktree: `cargo build --release` exit 0, 1m 34s) |

Everything below assumes both rows hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run every
> command before starting, and again before deciding to stop. If you stop, report the status of
> *all* prerequisites.

## 1. Goal

- The ICMP-denied harness and the Vulkan canvas proof run again.
- 8 dead files are gone, and every live reference to them is removed.
- Every stale path reference found in the §2 audit points at a real file.

### Non-goals (explicit constraints)

- No compiler, runtime, or golden changes. Every edit is under `scripts/`, `tools/`, `.ai/`,
  `.github/workflows/` comments, `.gitattributes`, `.gitignore`, doc-comment text, or the census
  test in `tests/gate/`.
- No renames or moves. Those are sub-plans B and C.
- Completed docs (`planning/completed/**`, `bugs/completed/**`) are archive. Do not edit them, even
  where they cite a deleted path.
- Do not weaken the bug-470 lock census. `SCAN_FLOOR` is lowered only by the exact number of
  dump-emitting scripts deleted, re-measured by the command in Phase 2.

## 2. Current State

### Measured populations

| What | Count | Command |
|---|---|---|
| Tracked top-level files in `scripts/` | 71 | `git ls-files scripts \| grep -c '^scripts/[^/]*$'` |
| Total tracked lines in `scripts/` | 12069 | `git ls-files scripts \| xargs wc -l \| tail -1` |
| Scripts CI runs | 7 run (`check-generated`, `test-accept`, `artifact-gate`, `man-examples-gate`, `coverage`, `coverage-check`, `build-examples`) + 1 sourced (`coverage-common`) | `grep -n 'scripts/' .github/workflows/*.yml` |
| Files absent from `scripts/README.md` | 36 | `for f in $(git ls-files scripts \| grep '^scripts/[^/]*$'); do grep -qF "$(basename $f)" scripts/README.md \|\| echo $f; done \| wc -l` (the README itself is excluded by hand) |
| Scripts the census scan sees (non-comment dump flag) | 9 | loop over `scripts/*.sh`, skip `gate-lock.sh`/`artifact-kinds.sh`, `grep -v '^[[:space:]]*#' \| grep -qE -- '-ncode\|-nir\|-nplan\|-nobj\|-mir\|-ast\|\$DUMPS'` → artifact-gate, bench-lowering, bug387-gate, linux-artifact-baseline, ncode-determinism-alltargets, ncode-determinism, regen-ncodesum, regen-outside-ncode, test-accept |

### Verified properties (this sub-plan)

**Broken today:**

- **`scripts/check-icmp-permission.sh` does not compile its own program.** Extracting its inline
  `project.json` and `main.mfb` and running `target/release/mfb build` gives
  `main.mfb:7 error[2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER]: Identifier 'PingStatus' is not declared`
  at `CASE PingStatus.Ok` (run 2026-09-12). The bare imported enum has been refused since bug-480.
  `check-net-harness-selftest.sh` runs this harness, so it is red too.
- **`scripts/test-canvas-vulkan.sh` reads a file that moved.** Line 537 runs `sed` on
  `tests/rt_canvas_golden.rs`, but `ls` shows only `tests/canvas/rt_canvas_golden.rs`. The script
  is `set -euo pipefail`, so the groups stage dies. Its relative paths also assume cwd = repo root.
  The open `planning/plan-130-E-*.md` tells people to run this script.

**Dead:**

- **`bug387-gate.sh`** compares against a `/tmp/bug387` baseline. `ls -d /tmp/bug387` → no such
  file, and the script has no record mode. `bugs/completed/bug-387-*.md` exists (the bug is done).
  Live refs: the census row, `scripts/README.md`, `.ai/testing-gates.md`.
- **`typemodel-debug-sweep.sh`** built with a debug binary to trip a `debug_assertions`-only
  bijection assert. `assert_type_keys_are_bijective()` is now called unconditionally at
  `src/codegen/engine/validation/validation.rs` (`grep -n 'assert_type_keys_are_bijective()'`, both
  call sites read: no `cfg`). Every release build already runs it.
  `git grep -l typemodel-debug-sweep -- ':!planning/completed'` → no live refs.
- **`ncode-determinism.sh`** is the host-only copy of `ncode-determinism-alltargets.sh`; the
  all-targets version includes the host. Live refs: the census row and `scripts/README.md`.
- **`tls-secure-transport-version-probe.c`, `tls-wrap-adoption-probe-macos.c`,
  `tls-wrap-adoption-probe-macos-server.c`, `tls-wrap-adoption-probe-openssl.c`** are plan-110-D
  spikes (completed). Their build lines point at `/tmp/p110-probe/…`.
  `git grep -l -e tls-wrap-adoption -e tls-secure-transport -- ':!planning/completed'` → 0.
- **`snap-term.py`** has no caller. `git grep -n snap-term -- ':!planning/completed'` → only
  `.gitignore:56` and the file itself. (`snap-macos.py` is live: `test-macapp.sh` calls it.)

**Stale references (each confirmed by `ls` on the cited path → missing):**

- `tools/codepage-index/README.md` cites `scripts/audit-codepage-index.py`,
  `scripts/gen-codepage-tables.py` and `scripts/fetch-codepage-index.py`. The real names use
  underscores. It also names a test `codepage_tables_are_regenerable`; the real test is
  `codepage_tables_match_the_vendored_index_files`
  (`grep -n 'fn codepage_tables' src/codegen/builtins/encoding/mod.rs`).
- `scripts/fetch_codepage_index.py` docstring cites `scripts/gen-codepage-tables.py`.
- `tools/math-kernels/capture.sh` and `.gitattributes` cite `scripts/gen_regex_unicode.py`.
  The replacement is `gen_unicode_gencat_table.py` (its docstring says "Replaces
  `gen_regex_unicode.py`").
- `.gitattributes` lines 4–7 name `src/builtins/vector_package.mfb` and
  `src/builtins/unicode_gencat.mfb`. `ls src/builtins` → no such directory.
- `.github/workflows/coverage.yml:37` comment names `gen_regex_unicode.py`.
- `.github/workflows/coverage.yml:213` comment claims `check-icmp-permission.sh` covers the denied
  path, but no workflow step runs it.
- `scripts/net_blackhole_server.py` docstring names `check-net-connect-timeout.sh`; the current
  name is `check-tcp-connect-timeout.sh`.
- `scripts/linux-artifact-baseline.sh` header says there are zero Linux goldens and that
  artifact-gate uses `uname`. Both are false now that artifact-gate is multi-target.
- `.ai/testing-gates.md` says `regen-ncodesum.sh` sweeps only byte-identity fixtures. It sweeps all
  of `tests/` since plan-118-C.
- `scripts/test-accept-selftest.sh` section 5 defines `classify_argv` inside the selftest
  (`grep -n classify_argv scripts/*.sh` → only `test-accept-selftest.sh`). It tests no shipping
  code: the bug-455 pgrep guard it mirrored was replaced by gate-lock (bug-470).
- Plans still open cite scripts that never landed: `scripts/check-man-examples.py`,
  `doc-review-fanout.sh`, `man-manual.sh`, `spec-census.sh`, `update_man.sh`,
  `update_man_package.sh`, `fix_citations.py` (stale-path loop over
  `grep -rhoE 'scripts/[A-Za-z0-9_.-]+\.(sh|py|c|txt)'`). These are **not** fixed here: they are
  other plans' prose. The one in `.ai/` (`.ai/testing-gates.md` → `fix_citations.py`,
  `.ai/resources-packages.md` → `check-man-examples.py`) **is** fixed here, because `.ai/` is
  read as current instructions.

## 3. Design Overview — the whole plan-131 inventory

Every file in the target state is one of:

- **KEEP** — reusable, and scripts/ is where it belongs;
- **MERGE** — folded into another script;
- **MOVE** — belongs with its data under `tools/`, or next to its package;
- **DELETE**.

The evidence for each verdict is in each sub-plan's §2.

| Disposition | Sub-plan | Files |
|---|---|---|
| KEEP (37) | — | `README.md`, `artifact-gate.sh`, `artifact-kinds.sh`, `build-examples.sh`, `check-generated.sh`, `check-icmp-permission.sh` (fixed in A), `check-net-harness-selftest.sh`, `check-tcp-connect-timeout.sh`, `check-tls-loopback.sh`, `check-udp-echo.sh`, `coverage.sh`, `coverage-check.sh`, `coverage-common.sh`, `coverage-exceptions.txt`, `diag-set-diff.sh`, `gate-lock.sh`, `gen-test-tls-identity.sh`, `linux-runtime-proof.sh`, `man-census.sh`, `man-examples-gate.sh`, `man-examples-not-run.txt`, `man-examples-stdin.txt`, `man-run-examples.sh`, `ncode-determinism-alltargets.sh`, `net_blackhole_server.py`, `net_udp_echo_server.py`, `regen-spirv.sh`, `snap-macos.py`, `sync-goldens.sh`, `sync-package-mfp.sh`, `test-accept.sh`, `test-accept-selftest.sh`, `test-appimage.sh`, `test-canvas-vulkan.sh` (fixed in A), `test-macapp.sh`, `test-winapp.sh`, `test-winprocess.sh` |
| DELETE (8) | A | `bug387-gate.sh`, `typemodel-debug-sweep.sh`, `ncode-determinism.sh`, `snap-term.py`, `tls-secure-transport-version-probe.c`, `tls-wrap-adoption-probe-macos.c`, `tls-wrap-adoption-probe-macos-server.c`, `tls-wrap-adoption-probe-openssl.c` |
| MERGE (12 → 3 new) | B | `regen-ncodesum.sh` + `regen-outside-ncode.sh` + `regen-rt-goldens.sh` → **`regen-native-goldens.sh`**; `linux-artifact-baseline.sh` + `exe-oracle.sh` → **`artifact-baseline.sh`**; `coverage-src-{gaps,lines,shapes,dead-functions,delta}.py` → **`coverage-report.py`**; `coverage-bins.sh` → `coverage.sh --bins`; `check-tls-loopback-remote.sh` → `check-tls-loopback.sh --remote` |
| MOVE (14 + bench-probes/) | C | `gen_codepage_tables.py`, `fetch_codepage_index.py`, `audit_codepage_index.py` → `tools/codepage-index/`; `gen_unicode_gencat_table.py`, `gen_unicode_script_table.py`, `gen_regex_scripts.py` → `tools/unicode-tables/`; `gen_vector_package.py`, `check_vector_bodies.py` → `tools/vector-gen/` (see Open Decisions); `icmp-capability-probe.c`, `icmp-constants-probe.c` → `tools/net-probes/`; `rvv-qemu-runner.sh`, `rvv-ulp-two-profile.sh` → `tools/math-kernels/`; `bench-lowering.sh` + `bench-probes/` → `tools/bench-lowering/`; `yaml_oracle_diff.py` → `packages/yaml/oracle/pyyaml_diff.py` |
| DEDUP (no file count change) | D | Shared `scripts/lib/remote.sh` for the five `test-*` remote/app proofs |
| INDEX + GUARD | E | Rewrite `scripts/README.md`; add a census test; add an AGENTS.md rule |

Result: 37 KEEP + 3 new = **40** top-level files, down from 71. If sub-plan D lands, it adds 2
shared helpers (`remote-common.sh`, `rgba_compare.py`), for **42**. **No script leaves the tree
without its callers being updated in the same commit.**

**Rule for `scripts/` vs `tools/`** (written into AGENTS.md by sub-plan E):

- `scripts/` holds a reusable gate, harness or maintenance action you run against the compiler or
  the tree, plus the libraries and data files those scripts source.
- `tools/<name>/` holds offline tooling with its own inputs or data (generators with vendored
  indexes, probes, oracles, benchmarks), each with a README.
- A one-off probe for a single bug or plan lives in `/tmp` or in the bug doc. It never lands in
  `scripts/`.

**Correctness gate class.** A and C are provably neutral to the compiler: no `src/` code or
golden may change. Their gate is "`check-generated.sh` green, `artifact-gate.sh all` green, and
`git status` shows only the intended files".

B is behavior-preserving for scripts. Its gate is a differential run of old versus new script on
the same input before the old one is deleted.

D needs real remote-box runs.

**Where risk concentrates:**

- B's regen merge. A wrong merge silently rewrites goldens.
- C's generator moves. Each generator writes its own path into its output, so the output must be
  regenerated in the same commit.
- D, which cannot be verified locally.

**Rejected alternatives:**

- *Subdirectories per area (`scripts/net/`, `scripts/man/`, `scripts/coverage/`).* This churns
  paths cited by AGENTS.md, `.ai/`, CI and 12+ open plans (for the man scripts alone, per the
  audit) for navigation that the README index gives for free. See Open Decisions.
- *Merge the three man scripts into one tool.* They are layered, not duplicated: `man-run-examples`
  handles one package, `man-examples-gate` orchestrates it (`bash $root/scripts/man-run-examples.sh`),
  and `man-census` measures prose and never compiles. Merging buys nothing.
- *Merge `test-winprocess.sh` into `test-winapp.sh`.* Its header argues for a separate console
  proof, and two `src/` comments cite it as the proof of Windows spawn behavior.
- *Delete `check-tls-loopback.sh` legs 1–2 because `tests/net/rt_tls_*` covers them.* The
  selftest's TLS sabotage targets leg 1. That is out of cleanup scope.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as the work.
> `- [~]` = partial, with what remains. Moot tasks stay as `- [x] ~~text~~ — moot: <evidence>`.
> Fill `Commit:` when the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Repair the two broken scripts

Each is a self-contained fix to one script, and each restores a proof somebody is told to run.

- [x] `scripts/check-icmp-permission.sh`: change the four `CASE PingStatus.*` arms to
      `CASE net::PingStatus.*`. Re-run the extraction build from §2 → it must compile.
      (Proved by the harness run below, which builds that program: exit 0.)
- [x] Run `bash scripts/check-icmp-permission.sh target/release/mfb` on macOS. Expect `PASS` or a
      stated `SKIP`; record which, with its output line, in this plan.
      Result 2026-09-12: `baseline: ICMP permitted here, ping returned 'status:Ok'` then
      `PASS: net::ping raises (never a PingStatus) when the OS refuses an ICMP socket`, exit 0.
- [x] Run `bash scripts/check-net-harness-selftest.sh target/release/mfb`. It must pass all four
      as-shipped legs and fail every sabotaged leg for its injected reason. If a leg other than ICMP
      is red, that is a separate bug: file it via write-bug before continuing.
      Result 2026-09-12: 8/8 `ok` (tcp, udp, tls, icmp; each as shipped PASS and sabotaged FAIL),
      `PASS: every networking harness passes as shipped and fails on an injected wrong result`, exit 0.
- [x] `scripts/test-canvas-vulkan.sh`: set `ROOT` from the script's own location and `cd "$ROOT"`
      near the top. Change line 537/540 and the comments at 81, 229, 456, 512 to
      `tests/canvas/rt_canvas_golden.rs`. Fix `.ai/testing-gates.md` where it cites the old path.
      (`ROOT` was already set; the `cd` goes after `MFB_EXE` is made absolute, which needs the
      caller's cwd. Lines 81/229/456 name `rt_canvas_font.rs`/`rt_canvas_damage.rs`, not the golden
      file, so only 512/537/540 carried the old path. `.ai/canvas-threading.md` carried it too and
      is fixed.)
- [x] *(added)* `scripts/test-canvas-vulkan.sh`'s inline program used `canvas::Color`,
      `canvas::rgb`/`rgba` and `canvas::fontRef`, removed by plan-122-D (`0b3fc656f`) and plan-116-I
      (`8a9a9f294`). Ported to `color::Color`/`color::rgb`/`color::rgba` with `IMPORT color`, and the
      `RES` font is passed directly (`font := face`, as `tests/canvas/rt_canvas_metal.rs` does).
- [x] *(added)* `scripts/test-canvas-vulkan.sh`: a backticked `loadFont` in a comment inside the
      double-quoted remote command ran as a local command substitution
      (`line 603: loadFont: command not found`). Backticks removed.
- [~] *(added)* `scripts/test-macapp.sh`: the same rot. Five builds failed
      (`FAIL: build -app appdefault` … `wrongmode_io`) on the bare `Mode.Console` enum
      (`SYMBOL_UNKNOWN_IDENTIFIER`, reproduced by building the extracted program), and its two canvas
      programs used `canvas::rgb`. Qualify as `app::Mode.*`, port to `color::rgb` + `IMPORT color`.
      Non-GUI run → `macOS app mode runtime tests passed`, exit 0; the GUI legs (which hold the
      canvas programs) need an `MFB_MACAPP_GUI=1` run.
      The first GUI run found three more stale programs: bare `Size`/`DrawItem`/`Rectangle` in
      `canvasblit`/`canvasresize`, and bare `TermSize` in `tsize`. They are qualified as
      `canvas::`/`term::`. Each now builds with `mfb build -app` (exit 0; extracted from the heredocs).
      Re-run on the final script, 2026-09-12: non-GUI `macOS app mode runtime tests passed`, exit 0.
      Remaining: the `MFB_MACAPP_GUI=1` legs. That mode injects System Events keystrokes into the
      focused window of the user's live desktop, so it runs only with the user's explicit go-ahead
      (the one attempt typed into the user's windows and was killed). The `reconcile` leg's
      `got ''` in that attempt did not reproduce when the built bundle was run directly
      (`stdout=[RECONCILE_BEFORE]`).
- [~] Prove the canvas fix: run `bash scripts/test-canvas-vulkan.sh target/release/mfb --box 2228`
      from a directory other than the repo root (e.g. `/tmp`). The groups stage must get past the
      `sed` extraction. Record the stage results. Box access per `.ai/remote_systems.md`.
      Done on **2227** (musl, `--libc musl --icd auto`) from `/tmp`, 2026-09-12: 15 `ok`, 0 `FAIL`,
      exit 0, including `ok: groups: the software render reproduces tests/golden/canvas/groups.png
      exactly` and `ok: groups: the Vulkan render matches tests/golden/canvas/groups.png (ok worst=1
      differing=0.0521%)`. Remaining: the glibc run on 2228 (connection refused at plan start; the
      user is bringing it up).

Acceptance:
- The ICMP harness program builds.
- `check-net-harness-selftest.sh` exits 0.
- `test-canvas-vulkan.sh` run from `/tmp` reaches and completes the groups stage.

Commit: —

### Phase 2 — Delete the 8 dead files

Deletion only. The census test is edited in the same commit, so `cargo test` never goes red.

- [x] `git rm scripts/bug387-gate.sh scripts/typemodel-debug-sweep.sh scripts/ncode-determinism.sh scripts/snap-term.py scripts/tls-secure-transport-version-probe.c scripts/tls-wrap-adoption-probe-macos.c scripts/tls-wrap-adoption-probe-macos-server.c scripts/tls-wrap-adoption-probe-openssl.c`
- [x] `tests/gate/gate_lock_covers_every_writer.rs`: remove the `bug387-gate.sh` and
      `ncode-determinism.sh` rows from `CLASSIFICATION`, and update the module doc and the comment
      that names `bug387-gate.sh`.
- [x] Update `SCAN_FLOOR` and its comment. Justification under AGENTS.md's four questions:
      (1) bug-470 set it to the count of dump-emitting scripts;
      (2) it guards against the scan going blind;
      (3) nothing else reads it;
      (4) the population it counts shrinks by exactly the deleted scripts.
      Re-run the §2 scan loop → expect 7. Set the floor to the measured number, never lower.
      Measured 7 (artifact-gate, bench-lowering, linux-artifact-baseline,
      ncode-determinism-alltargets, regen-ncodesum, regen-outside-ncode, test-accept); floor = 7.
- [x] `.gitignore`: remove the `snap-term.py` venv/artifact block (lines 56–58). Keep the
      `snap-macos.py` block.
- [x] `scripts/README.md`: remove the `bug387-gate.sh` and `ncode-determinism.sh` entries. Make the
      `ncode-determinism-alltargets.sh` entry self-contained, since it currently says "Same
      determinism check as above".
- [x] `.ai/testing-gates.md`: remove the `bug387-gate.sh` paragraph. Keep the `exe-oracle.sh` text
      until B merges it. (It was two clauses inside the exe-oracle clobber note, not a paragraph;
      both now point at `exe-oracle.sh … compare`.)
- [x] Re-run `git grep -n -e bug387-gate -e typemodel-debug-sweep -e 'ncode-determinism\.sh' -e snap-term -e tls-wrap-adoption -e tls-secure-transport -- ':!planning/completed' ':!bugs/completed' ':!planning/plan-131-*'`
      → 0 hits. (`ncode-determinism\.sh` does not match the `-alltargets` name.) Measured: exit 1,
      0 hits, after re-wording two census comments that named the deleted scripts.

Acceptance:
- `cargo test --test gate_lock_covers_every_writer --test gate_mutual_exclusion --test gate_release_on_normal_exit --no-fail-fast` passes.
  Measured 2026-09-12: 1 + 5 + 3 passed, exit 0.
- The grep above finds 0 hits.

Commit: —

### Phase 3 — Fix every stale reference

Text-only corrections to docs and comments, each pointing at a file that exists.

- [x] `tools/codepage-index/README.md`: change the three hyphenated script names to their
      underscore names, and the test name to `codepage_tables_match_the_vendored_index_files`. Say
      that `check-generated.sh` is what proves the table is regenerable. (Sub-plan C rewrites these
      paths again when the scripts move; this phase makes them true today.)
- [x] `scripts/fetch_codepage_index.py` docstring: `gen-codepage-tables.py` → `gen_codepage_tables.py`.
- [x] `tools/math-kernels/capture.sh`, `.github/workflows/coverage.yml:37`: `gen_regex_unicode.py`
      → `gen_unicode_gencat_table.py`.
- [x] `.gitattributes`: replace the stale entries with the four current generated files
      (`src/codegen/string/unicode/unicode_gencat_ranges.txt`, `unicode_script_ranges.txt`,
      `unicode_script_names.mfb`, `src/codegen/builtins/encoding/helper_codepage_table.rs`).
      Keep whatever attribute the old lines set. Read the file first.
- [x] `.github/workflows/coverage.yml:213`: change the comment to say the ICMP-denied check is
      run by hand (`scripts/check-net-harness-selftest.sh`), not in CI. Whether to wire it into
      CI is an Open Decision in sub-plan E.
- [x] `scripts/net_blackhole_server.py` docstring: name `check-tcp-connect-timeout.sh`.
- [x] `scripts/linux-artifact-baseline.sh` header: remove the "zero Linux goldens / uses uname"
      claims, and state what it covers that `artifact-gate.sh` does not (linked `.out`, fixtures
      with no goldens).
- [x] `.ai/testing-gates.md`: correct the `regen-ncodesum.sh` scope claim (all of `tests/`). Remove
      or repoint the `fix_citations.py` citation. `git log --all --diff-filter=D --name-only -- scripts/fix_citations.py`
      shows whether it ever existed; if it did, cite the deleting commit.
      It existed; it was deleted in `4b693b6fa` (bug-344). The sentence now says so, naming
      `fix_citations.py` without a `scripts/` path.
- [x] `.ai/resources-packages.md`: repoint `scripts/check-man-examples.py` to
      `scripts/man-run-examples.sh`, or remove it (read the sentence first).
      Repointed to `man-examples-gate.sh` (which drives `man-run-examples.sh`). The same paragraph
      also claimed the tool wraps a block with no `main` in a synthetic `SUB main()`.
      `man-run-examples.sh` does no such wrapping (`grep -n -i 'SUB main\|wrap'` → nothing), so
      that claim is replaced with how blocks are really lifted.
- [x] `scripts/test-accept-selftest.sh`: delete section 5 (`classify_argv`) and its summary count.
      Run `bash scripts/test-accept-selftest.sh` → exit 0.
      There was no per-section count to update, only `$failures`. Result: `test-accept selftest:
      all checks passed`, exit 0.
- [x] `scripts/man-run-examples.sh`: add `--test` to the usage string.
- [x] Remove the untracked local `scripts/__pycache__/` (it is gitignored, so no commit).
      It existed only in the main checkout; removed there.

Acceptance:
- The §2 stale-path loop, restricted to `.ai/ tools/ scripts/ .github/ .gitattributes`, reports 0
  missing paths.
  Measured 2026-09-12 with `grep -rIhoE` (`-I`, because without it grep prints "Binary file
  matches" words as paths). The only hits are `scripts/.selftest-{icmp,tcp,tls,udp}.sh`: temp
  copies `check-net-harness-selftest.sh` creates and deletes at run time, not citations of a
  committed file. No real missing path.
- `sh scripts/check-generated.sh` → exit 0 (measured 2026-09-12).
- `test-accept-selftest.sh` → exit 0 (measured 2026-09-12).
- `sh scripts/check-generated.sh` exits 0.
- `test-accept-selftest.sh` exits 0.

Commit: —

## Validation Plan

- Tests: the three `tests/gate` census and lock tests. There is no new Rust test in A; the
  README census is sub-plan E.
- Runtime proof: the Phase 1 harness runs, with their output recorded above.
- Neutrality: `bash scripts/artifact-gate.sh target/release/mfb all` → 0 diffs (A touches no
  compiler input). `git diff --stat` shows only files named in this plan.
- Doc sync: `.ai/testing-gates.md`, `.ai/resources-packages.md`, `scripts/README.md`,
  `tools/codepage-index/README.md`. No `mfb man`/spec change.

## Open Decisions

(These cover the whole of plan-131.)

**Answered by the user, 2026-09-12 (at the start of execution):**
- Vector generator → **move to `tools/vector-gen/`**.
- Sub-plan D → **run it**: "do the work, test on macos for now, I'll bring up the server
  later". The remote-box legs run once boxes 2228/2230 are up. They were refused at the start
  (`ssh -o BatchMode=yes -o ConnectTimeout=8 -p 2228|2230|2232 test@127.0.0.1 true` → exit 255;
  2227 → exit 0).
- Network harnesses → **add a CI job** (sub-plan E; this reverses E's non-goal, see E Corrections).
- Layout → flat `scripts/` (the recommendation; the user was not asked to overturn it).

- **Vector generator (sub-plan C):** move `gen_vector_package.py` + `check_vector_bodies.py` to
  `tools/vector-gen/` (recommended) vs. delete both.
  - For moving: `check-generated.sh` gates them in CI today, and the generator keeps about 170
    overloads uniform.
  - For deleting: `src/builtins/vector_package.mfb` is gone, and the `BODY*` consts in
    `src/codegen/builtins/vector/` are the source of truth. Deleting both removes a second copy
    of every body that must be edited in lockstep.
- **Layout:** flat `scripts/` plus a README index (recommended) vs. per-area subdirectories. Flat
  avoids churning paths cited in AGENTS.md, `.ai/`, CI and open plans.
- **Shared remote lib (sub-plan D):** do it (recommended only if boxes 2228/2227/2230 are
  available for the before/after runs) vs. drop D. The gain is not measured yet; D's first task
  measures it.
- **Network harnesses in CI (sub-plan E):** keep them run-by-hand with honest comments
  (recommended; they need `unshare -Urn`/`sandbox-exec` and loopback TLS) vs. add a CI job.

## Corrections

- **2026-09-12, Phase 1: more scripts were broken than §2 listed.** `test-canvas-vulkan.sh` was
  broken in two further ways: a stale `canvas::` colour/font API, and a backtick command
  substitution in its remote command. `test-macapp.sh`, a KEEP file, failed 5 of its builds on the
  same bare-enum rule that broke the ICMP harness (bug-480). The §2 audit found breaks by checking
  paths, not by running each script. Both are fixed in Phase 1 as added tasks. Left unfixed, D's
  baseline step would have recorded the broken output as the baseline.
- **2026-09-12, Phase 1: the canvas proof ran on 2227, not 2228.** 2228 refused connections at plan
  start (`ssh … -p 2228 … true` → exit 255). 2227 is the musl half of the same test's evidence and
  was up; the glibc/2228 run stays `[~]`.
- **2026-09-12, Phase 1: the GUI legs of `test-macapp.sh` are gated on the user, not on this plan.**
  `MFB_MACAPP_GUI=1` types into the focused window of the live desktop. The macapp task stays `[~]`
  until the user approves a GUI run.
- **2026-09-12, Phase 3: the stale-path loop needs `-I` and an exclusion.** Without `-I` it reports
  `Binary`/`file`/`matches`. With it, the remaining hits are the selftest's runtime temp copies.

## Summary

A carries no compiler risk. Its real risk is the census test: it breaks on any delete unless it
is edited in the same commit, and its floor must be re-measured rather than guessed. Left
untouched here: renames, moves, merges, and the README index (B–E).
