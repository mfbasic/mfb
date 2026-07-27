# plan-68-A: Triage, exception-list repair, fresh line-level report

Last updated: 2026-07-27
Overall Effort (AI): large (3h–1d)   (whole plan-68 feature)
Effort (Human): medium (1h–2h)
Effort (AI): small (<1h)
Depends on: nothing
Produces:
- A regenerated `target/coverage/coverage.json` (current-HEAD line-level
  coverage) that B–H read to name their uncovered target regions.
- A repaired `scripts/coverage-exceptions.txt`: stale paths repointed, genuinely
  integration-only moved files re-excepted with fresh boundary justifications.
- The authoritative **worklist**: for each of the 55 failing files, the decision
  `except` | `backfill (letter X)` | `denominator-fix`, recorded in this doc's
  triage table. B–H trust these decisions and do not re-litigate them.

Part **A** of plan-68. Shared goal, prerequisites, dependency graph, measured
populations, design, and standing requirements live in the overview:
[plan-68-coverage-gate.md](plan-68-coverage-gate.md). Prerequisites are stated
there and gate this sub-plan too — re-run them before starting.

## Why this lands first

The overview's design section: the premise "all 55 files need unit tests" is
false, and the B–H split assumes A already removed the exceptions. A is the
cheapest experiment that falsifies the naive premise: it produces both the fresh
line data every backfill task needs and the definitive except-vs-backfill call
for every file. Nothing in B–H can be written followably until A's report and
worklist exist.

## Phases

### Phase A1 — Regenerate the coverage profile + report

The on-disk `target/coverage/coverage.json` is stale (overview §2 Verified
properties). A full instrumented run is the only source of current line-level
data.

- [x] Confirm prerequisites: `cargo llvm-cov --version` succeeds (0.8.7); `cargo test`
      → `0 failed` (`sh scripts/coverage.sh` exited 0, and it runs the full
      instrumented suite `--no-fail-fast` and exits with the suite status).
- [x] Run `sh scripts/coverage.sh`. Exited 0; (re)wrote
      `target/coverage/coverage.json`, `.../html/`, `lcov.info`, `cobertura.xml`.
- [x] Run `sh scripts/coverage-check.sh` and capture the current failing set.
      Result: 56 GATE FAILUREs = the overview's 55 + one new file
      `src/arch/x86_64/encode/emitter.rs` (94.66%, 1471/1554). Deltas recorded in
      Corrections; overview table + D scope updated to match.

Acceptance: `target/coverage/coverage.json` mtime is current;
`sh scripts/coverage-check.sh` reproduces a failing list; any difference from the
overview's 55-file table is written into Corrections with the measuring command.
Commit: — (no source change; this phase only produces artifacts + notes)

### Phase A2 — Repoint the stale exceptions

`scripts/coverage-exceptions.txt` names two paths whose code moved (overview §2).
Repair them so the exception follows the code, without excepting anything newly.

- [x] Replaced the `src/cli/build.rs` entry (file deleted by 19a95d448) with
      `src/cli/build/signing.rs`. VERIFIED against A1's fresh `lcov.info`: the 92
      uncovered lines are exactly `load_build_signing_info` (signing.rs:47–158),
      which calls `mfb_repository::client::request_attestation` (live HTTP
      registry) and reads the machine's ident-key files; the pure helpers
      (`signing_ident`/`apply_signing_metadata`/`executable_signing_metadata_json`)
      are all covered. The other `src/cli/build/*.rs` modules are majority-coverable
      → left for sub-plan B, NOT blanket-excepted.
- [x] Adjusted the `src/main.rs` entry (kept — file exists, 0/3, `fn main` is the
      process entry never invoked by a unit test) and added `src/cli/dispatch.rs`
      (the ex-`main.rs` router, verified integration-only overview §2). dispatch.rs
      excepted whole-file: `run()` reads `std::env::args` and its pure
      `is_help_flag`/USAGE arms cannot be isolated from that entry without
      terminating the test runner via `process::exit`.
- [x] Every added/changed entry names its concrete boundary (registry HTTP /
      subprocess linker / process::exit). Confirmed: `sh scripts/coverage-check.sh`
      now lists `src/cli/dispatch.rs` + `src/cli/build/signing.rs` under
      "Documented exceptions" (8 excepted), not GATE FAILURE.

Acceptance: after this phase, `sh scripts/coverage-check.sh` no longer reports
`src/cli/dispatch.rs` (and any newly-excepted `src/cli/build/*` orchestration
module) as a GATE FAILURE — they appear under "Documented exceptions" instead;
and no file with a coverable body was whole-file excepted (spot-check each new
entry against its source). `cargo test` still `0 failed` (exception list is not
compiled, so this is a no-op check confirming nothing else was touched).
Commit: — (touches only `scripts/coverage-exceptions.txt`)

### Phase A3 — Triage `src/os/windows/mod.rs` and finalize the worklist

`src/os/windows/mod.rs` is 0/38 and small. Decide its class, then freeze the
per-letter worklist B–H will consume.

- [x] Read `src/os/windows/mod.rs`. Classify: **backfill:C**. It is three thin
      wrappers — `validate_native_object_plan` (pure: `object::lower_plan(plan)?.
      validate()`), `write_native_object_plan` and `write_linked_executable` (each
      lowers/links in-memory then `fs::write`s into a caller-supplied dir). No
      subprocess/network; its sibling submodules (`object.rs`, `link/`) already
      carry `#[cfg(test)]` fixtures a `mod tests` here can reuse (build a
      `NativePlan`/`EncodedImage`, write into a `tempdir`, assert the artifact
      exists). Added as C phase C11.
- [x] Resolved the two overview Open Decisions with A1's data:
      `src/arch/aarch64/backend.rs` — now **6/9** (improved since the snapshot; 3
      uncov = the three trivial `Backend` method bodies) → **backfill:D** (D1, as
      the overview resolved). `src/json.rs` (13/14) — the lone uncovered line is
      the `.unwrap_or_else` `Err` arm of `stringify()` on a `JsonValue::String`,
      which is **infallible** for a `String` (only a non-finite `Number` fails).
      **Dead code → resolved by A, not excepted and not an H task:** replaced the
      dead fallback with `.expect("JsonValue::String is always stringifiable")`
      (executes every call → covered), per the Open Decision + AGENTS.md "no
      dead-code filler". json.rs drops off the gate; H4/H removes its json.rs task.
- [x] Filled the **Triage table** below. All 54 residual GATE-FAILURE files are
      `backfill:<letter>` (their overview letter, + emitter.rs→D, windows/mod.rs→C,
      json.rs resolved by A); the only `except`s are dispatch.rs + signing.rs (A2).

Acceptance: the Triage table is complete for all 55 files; every `except` row has
a boundary reason present in `coverage-exceptions.txt`; `sh scripts/coverage-check.sh`
now lists only files marked `backfill:<letter>` as GATE FAILUREs (the excepted
ones moved to the "Documented exceptions" section). The residual failing count
equals the number of `backfill` rows.
Commit: — (any exception additions from A3 go in `scripts/coverage-exceptions.txt`)

## Triage table (the worklist — filled during A3)

| File | uncov | Decision | Evidence / boundary |
|---|---|---|---|
| src/cli/dispatch.rs | 195 | **except** | ex-main.rs router; run() reads env::args, every arm spawns linker via build_project / live registry via run_pkg/repo / process::exit (verified, overview §2 + read). Now under "Documented exceptions". |
| src/cli/build/signing.rs | 92 | **except** (repoints build.rs) | VERIFIED against fresh lcov: all 92 uncov lines are `load_build_signing_info` (:47–158) → `request_attestation` live registry + machine ident-key reads; pure helpers covered. A2 repoints the stale `src/cli/build.rs` entry here. |
| src/os/windows/mod.rs | 38 | **backfill:C** (C11) | three thin wrappers; validate_native_object_plan pure, write fns fs::write into a tempdir; sibling object/link tests reusable. Coverable — not an exception. |
| src/arch/aarch64/backend.rs | 3 (now 6/9) | **backfill:D** (D1) | three trivial `Backend` method bodies; `select(&[])` safe (select.rs loops), is_aarch64/register_model trivial. |
| src/arch/x86_64/encode/emitter.rs | 83 (1471/1554) | **backfill:D** (D7, NEW) | NEW delta — not in overview 55; x86-64 instruction encoder, pure in-memory encode logic. Assigned to D (arch cohesion with D1). |
| src/json.rs | 1 | **resolved by A** (not except, not backfill) | lone uncov line = dead `Err` arm of `stringify()` on a String (infallible); replaced with `.expect(...)` → 14/14. No H task. |
| all other 52 GATE-FAILURE files | — | **backfill:<overview letter>** | default to their overview letter (B/C/D/E/F/G/H per §2 population table); no further A moves. |

## Exception / unreachable candidates surfaced by B–H (confirm against A1's fresh report)

The backfill sub-plans read their files and flagged these specific lines/arms as
NOT unit-coverable. A must confirm each against the fresh `coverage.json` and, if
truly unreachable-by-construction, add it as a **line-level** exception (or delete
it if it is dead code — `AGENTS.md`: no dead-code filler). Do NOT let a letter
chase these; they are A's to resolve so the file can still reach ≥95% on its
reachable lines.

| From | Site | Nature | A's action |
|---|---|---|---|
| B | `src/cli/build/signing.rs` (whole) | `load_build_signing_info` live-registry attestation | except at new path (repoints `build.rs`) |
| C | `src/os/linux/appimage/mod.rs` `seal` blob-length guard; `src/os/macos/icon.rs` icns `.map_err` arms | defensive, unreachable with valid input | line-level exception only if the file can't otherwise hit 95% (icon needs just 1 more line) |
| E | `src/ir/lower.rs:3108` `unreachable!("inline TRAP…")`; `src/ir/link.rs:585` `AbiDirection::Out => unreachable!`; `write_ir` `Err` arm | `unreachable!` guards + `std::fs` error arm | line-level exception; happy paths are E's backfill |
| F | `src/syntaxcheck/link.rs:844` `native_function_sig` `Type::Unknown` fallback; `Type::Result`/internal-only match arms in inference.rs/mod.rs | param type not source-spellable / exhaustiveness arms | confirm unreachable, then line-level exception (mirrors the existing `syntaxcheck/resources.rs` exception rationale) |
| G | `src/ast/scope_privates.rs:41-49` `PRIVATE_PATH_HASH_COLLISION` (`Some(prev) if prev != &file.path`) | fires only on a real `file_scope_hash` collision, not constructible | line-level exception. NOTE: use the exception FILE, not an inline `coverage:off` fence — cargo-llvm-cov 0.8.7 ignores inline markers (`coverage-exceptions.txt:7`) |
| H | `repository/src/main.rs::main()` socket-bind/live-HTTP tail (already fenced); `backfill.rs` `BlobFetch::Redirect` (S3-only); `src/docs/spec/mod.rs` `sort_by_key` fallback (package always in `PACKAGE_ORDER`); `src/manifest/json_edit.rs` 3 malformed-value arms | live I/O + unreachable defensive arms | except the `main.rs` tail with the socket/HTTP boundary named; line-level for the rest per the fresh report |
| H / Open Decision | `src/json.rs` (13/14, one line) | H judged the lone uncovered line an **infallible/dead** `stringify` fallback — contradicts the overview's "cover it" recommendation | READ it against the fresh report: if genuinely dead, **delete it** (→ 13/13 = 100%, no exception); if reachable, H covers it. Do NOT except a 14-line file for one line without proving unreachability |

## Validation Plan

- `sh scripts/coverage.sh` exits 0 and produces a current `coverage.json`.
- After A2+A3, `sh scripts/coverage-check.sh` shows the previously-excepted-then-
  moved files under "Documented exceptions", not GATE FAILURE.
- `cargo test` → `0 failed` (unchanged; confirms only the exception list moved).
- Each new exception line maps to a concrete boundary a reviewer can find in the
  named source file.

## Corrections

Measured with `sh scripts/coverage.sh` (exit 0) then `sh scripts/coverage-check.sh`
against the fresh `target/coverage/coverage.json` (2026-07-27):

1. **NEW file: `src/arch/x86_64/encode/emitter.rs` (94.66%, 1471/1554, 83 uncov)**
   is a GATE FAILURE but appears in **no** plan-68 doc (`grep -n emitter
   planning/plan-68-*.md` → nothing). It drifted below 95% since the user's
   snapshot. Assigned **backfill:D** as new phase **D7** (arch cohesion with D1's
   aarch64 backend); added to the overview §2 table and D's scope table.
2. **`src/arch/aarch64/backend.rs` improved 3/9 → 6/9** since the snapshot (3 uncov
   now, not 6). Still failing; still backfill:D (D1). Overview count updated.
3. **`src/os/linux/appimage/squashfs/mod.rs` drifted 327/351 → 321/351** (91.45%,
   30 uncov, was 93.16%/24). Still backfill:C (C4). Overview count updated.
4. The stale `src/cli/build.rs` exception line named a **deleted** file (matched
   nothing); replaced by `src/cli/build/signing.rs`. `src/main.rs` kept (still
   exists, still un-unit-testable process entry) with reworded boundary; the
   dispatch body it used to name now lives in the added `src/cli/dispatch.rs`
   exception.
5. `src/json.rs` Open Decision resolved: the uncovered line is dead (stringify
   infallible for String) → replaced with `.expect(...)` (commit lands with A).
   No H4 json.rs task.

Failing set after A2/A3: **54 backfill files** (56 GATE FAILUREs − dispatch.rs −
signing.rs excepted; json.rs resolved by A leaves it too once its `.expect` edit
is in the profile). Per-letter counts B/C/D/E/F/G/H unchanged except D gains
emitter.rs (D7) and C gains windows/mod.rs (C11).
