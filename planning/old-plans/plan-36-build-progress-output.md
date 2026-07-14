# Build Progress Output Plan

Last updated: 2026-07-13
Effort: medium (1h–2h)
Status: **DONE** (Phases 1–3). Phase 4 remains deferred (optional, needs
re-scoping). `Verbosity`/`Reporter` in `src/cli/build.rs`; `-q`/`--quiet` +
`-v`/`--verbose` (mutually exclusive); default summary `Building <name> (<kind>)
for <target>` and `-v` phase lines (`parse`/`resolve`/`verify`/`codegen+link`) on
stderr; artifact line unchanged on stdout. `mfb test`/`pkg publish`/`pkg
check-abi` build quietly. Harness passes `-q` for `build.log` captures (Decision
A) → zero golden churn. Validation: 935 acceptance green (no golden diff), 42
spec tests, artifact-gate 0/1093 diffs, new `tests/build_verbosity_output.rs`
(byte-identity across levels) + build.rs parser unit tests.

`mfb build` today is silent on success except for a single `Wrote executable to
<path>` line. This plan gives the build a **concise summary by default** (a
short context line naming what is being built and for which target, alongside
the preserved artifact line), a **`-q` / `--quiet`** flag that restores today's
minimal output, and a **`-v` / `--verbose`** flag that prints one timed line per
pipeline phase. The verbose path doubles as a lightweight build profiler.

The behavioral outcome a correct implementation produces: a plain `mfb build`
prints a two-ish line human summary and still ends with the exact
`Wrote executable to <path>` line; `mfb build -q` prints only what today's build
prints; `mfb build -v` additionally prints a `phase … Nms` line for each front
end stage — and none of these change the emitted artifact bytes.

References:

- `src/cli/build.rs` — `BuildOptions`, `parse_build_options`, `build_project`
  (the whole front-end pipeline and every `Wrote … to` line).
- `src/target.rs:180` `write_executable` / `:289` `write_package` — the codegen
  black box the front end calls once.
- `scripts/test-accept.sh:244,260,359` — the acceptance harness that captures
  `mfb build … 2>&1` into `build.log` and exact-compares it to the golden.
- Integration tests parsing the artifact line: `tests/native_io_runtime.rs:40`,
  `tests/native_float_pow_operator_runtime.rs:48`,
  `tests/native_numeric_pow_div_runtime.rs:57`,
  `tests/native_size_arith_overflow.rs:78`, `tests/native_loop_runtime.rs:46`,
  `tests/fs_error_path_hygiene.rs:282`, `tests/linux_app_mode.rs:103,134`.
- Memory: [[completed-plans-go-to-old-plans]], [[no-rebuild-during-acceptance]],
  [[fast-codegen-gate]].

## 1. Goal

- `mfb build` (no flags) prints, on success: a concise context line (project
  name, kind, target triple) **plus** the unchanged `Wrote executable to
  <path>` / `Wrote package to <path>` line. No timings, no color, no spinner.
- `mfb build -q` / `--quiet` reproduces today's stdout/stderr byte-for-byte
  (only the artifact line and any diagnostics).
- `mfb build -v` / `--verbose` prints one `phase <name> <N>ms` line per front-end
  stage (parse, resolve, verify, codegen+link) in addition to the normal
  summary.
- The emitted executable/package/dump bytes are identical across all three
  verbosity levels and identical to pre-plan output (verified by the artifact
  gate on every target).

### Non-goals (explicit constraints)

- **No change to artifact bytes** on any target (aarch64-macos, x86_64-linux,
  aarch64-linux, riscv64-linux). The verbosity level must never reach codegen.
- **The `Wrote executable to <path>` / `Wrote package to <path>` / `Wrote test
  executable to <path>` lines stay verbatim** and on stdout in every mode — many
  integration tests `strip_prefix` them.
- **No timing or other non-deterministic text in default or `-q` output.**
  `build.log` goldens are exact-compared; ms values would break every one on
  every run. Timings appear only under `-v`, which the acceptance harness never
  invokes.
- **No TTY-awareness, carriage-return status line, color, or spinner.** Plain
  lines only.
- **No `--json` / machine-readable event stream** (a possible future plan; out
  of scope here).
- No change to the dump flags (`-ir`, `-mir`, `-nobj`, …) or to diagnostic
  rendering (`rules::render_pending`).
- No change to backend trait signatures in this plan (see Phase 4 note).

## 2. Current State

`build_project` (`src/cli/build.rs:144`) runs the whole pipeline inline and, on
success, prints exactly one line per artifact:

- executable: `println!("Wrote executable to {}", …)` at `build.rs:422`
- test executable (cross target): `build.rs:417`
- package: `build.rs:458`
- dumps (`-ast`/`-ir`/…): their own `Wrote … to` lines at `build.rs:483..634`.

The front-end stages, in call order inside `build_project`:

1. `validate_project_manifest` / `verify_and_report_packages` (`:150`, `:157`)
2. `ast::parse_project` + `scope_privates` + `lower_testing_blocks` (`:226`–`:253`)
3. `resolver::resolve_project` + `monomorphize_project` +
   `resolve_project_with` (`:260`–`:265`)
4. `syntaxcheck` + `ir::lower_project_with_external_functions` +
   `ir::verify_source_diagnostics` (`:292`–`:304`)
5. `target::write_executable` / `write_package` (`:382`, `:452`) — a single
   opaque call that internally does lower_to_mir → register allocation →
   instruction select → emit → link. The CLI has no visibility inside it.

`BuildOptions` (`build.rs:24`) has no verbosity field; `parse_build_options`
(`build.rs:77`) handles `-target`, `--sign`, `-app`, `--unsigned`, `-regalloc`,
and dump flags, rejecting any other `-`-prefixed arg (`build.rs:126`).

Acceptance coupling: `scripts/test-accept.sh:244` runs `mfb build … 2>&1`, tees
it to `build.log` (`:260`), and `compare_file` exact-matches it against the
golden (`:359`). `scripts/sync-goldens.sh` regenerates goldens. Every rt-error /
rt-behavior test with a `golden/build.log` therefore encodes the current
success output.

Precedent to mirror: `verify_and_report_packages` (`build.rs:157`) already emits
a human per-package report to the same stream, so a summary here is consistent
with existing style.

## 3. Design Overview

Three independent pieces, layered:

1. **A `Verbosity` enum + a `Reporter`** (new, in the CLI layer). `Verbosity`
   is `Quiet | Normal | Verbose`. The `Reporter` owns the level and exposes a
   tiny API: `summary(&str)` (printed at Normal and Verbose), `phase(name,
   Duration)` (printed only at Verbose), and passes the artifact line through
   unchanged (always printed). All human progress goes through the reporter so
   there is exactly one place that knows the level.

2. **Flag plumbing.** `parse_build_options` learns `-q`/`--quiet` and
   `-v`/`--verbose` (mutually exclusive → error), storing a `Verbosity` on
   `BuildOptions` (default `Normal`). `-q` and `-v` are rejected today by the
   catch-all, so this is purely additive.

3. **Instrumentation in `build_project`.** Wrap the four front-end stage groups
   (parse, resolve, verify, codegen+link) with `Instant`-based timing, reported
   via `reporter.phase(...)`; emit the normal summary line before the pipeline
   runs; keep the artifact line as-is.

**Where the risk concentrates:** not in the code — it's a handful of
`println!`s — but in the **golden/test contract**. The two hazards are (a)
accidentally altering or moving the `Wrote … to` line (breaks ~7 integration
tests), and (b) letting any non-deterministic text (timings) into default/`-q`
output (breaks every `build.log` golden on every run). Phase 1 is a pure audit
that pins both down before any code changes; Phase 2 keeps timings strictly
inside `-v`, which the harness never exercises.

**Stream choice.** All human lines go to **stderr**, except the `Wrote … to`
artifact line which stays on **stdout** (tests parse stdout for it). Rationale:
stdout stays the machine-consumable artifact channel; progress is diagnostics.
Because `test-accept.sh` captures `2>&1`, this does not by itself spare the
goldens — see Open Decisions for how the harness handles the new summary line.

**Rejected alternatives:**

- *Put timings in the default summary.* Rejected: breaks exact-match goldens
  (non-deterministic ms) and forces a scrubber into the harness. Timings live
  only in `-v`.
- *Thread a reporter into the backend trait now* (per-phase codegen/link
  lines). Rejected for this plan: changes `NativeBackend::write_executable`
  across all four targets — a large surface for a cosmetic gain. Captured as an
  optional Phase 4 that, if pursued, warrants its own sizing.
- *TTY status line / color.* Rejected by explicit request ("nothing fancy").

## 4. Detailed Design

### 4.1 `Verbosity` and `Reporter`

New in `src/cli/build.rs` (or a small sibling `src/cli/reporter.rs`):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Verbosity {
    Quiet,
    #[default]
    Normal,
    Verbose,
}

pub(crate) struct Reporter { level: Verbosity }

impl Reporter {
    fn summary(&self, line: &str)              // Normal | Verbose  -> stderr
    fn phase(&self, name: &str, dt: Duration)  // Verbose only      -> stderr
}
```

`phase` formats as `phase <name> <ms>ms` with integer milliseconds (e.g.
`phase resolve 12ms`). Exact wording is finalized in Phase 1's output spec.

### 4.2 Flag parsing

In `parse_build_options` (`build.rs:87` loop): add arms for `-q`/`--quiet` and
`-v`/`--verbose` that set a local `verbosity`, erroring if both are given
(`"mfb build accepts at most one of -q / -v"`). Add `verbosity: Verbosity` to
`BuildOptions` and to the struct literal at `build.rs:132`. `mode`/`test` builds
inherit the same default.

### 4.3 Summary line content (Normal)

Composed from data already in hand before the pipeline: project `name`
(`build.rs:222`), `project_kind` (`build.rs:151`), and `target.name()`. Shape
(final text pinned in Phase 1), e.g.:

```
Building <name> (<kind>) for <target-triple>
```

Emitted once via `reporter.summary(...)` right after the target/kind are known
(after `build.rs:190`). Deterministic — safe if a golden ever captures it.

### 4.4 Phase timing (Verbose)

Bracket the four stage groups in `build_project` with `std::time::Instant`:

- `parse` — `parse_project` … `lower_testing_blocks` (`:226`–`:253`)
- `resolve` — `resolve_project` … `resolve_project_with` (`:260`–`:265`)
- `verify` — `syntaxcheck` … `verify_source_diagnostics` merge (`:292`–`:315`)
- `codegen+link` — the `write_executable` / `write_package` call (`:382` / `:452`)

Each group records elapsed and calls `reporter.phase(name, dt)`. Timing must be
cheap and side-effect free; guard nothing on the level except the print (always
compute so `-v` and default take identical code paths into codegen).

## Compatibility / Format Impact

- **stdout artifact lines** (`Wrote executable/package/test executable to …`):
  unchanged in all modes.
- **New default stderr lines:** one `Building …` summary line. This is the only
  externally observable change for a no-flag build.
- **`build.log` goldens:** captured with `2>&1`, so they gain the `Building …`
  line unless the harness suppresses it (Open Decision A). Regenerated
  mechanically via `sync-goldens.sh`.
- **Artifact bytes / dump files:** unchanged. Verified by the artifact gate.
- No wire-format, `.mfp`, ABI, or manifest change.

## Phases

### Phase 1 — Output-contract audit (no code) — DONE

**Finalized output spec:**

- **Summary line** (Normal & Verbose), on **stderr**, emitted once after the
  target/kind are known (after `build.rs:190`), for every build including dumps:
  `Building <name> (<kind>) for <target>` where `<name>` = validated project
  name, `<kind>` = `project_kind` (`executable` | `package`), `<target>` =
  `target.name()` (e.g. `linux-aarch64`).
- **Phase line** (Verbose only), on **stderr**: `phase <name> <N>ms` with integer
  milliseconds. Names in order: `parse`, `resolve`, `verify`, `codegen+link`.
- **Artifact line** (always), on **stdout**, verbatim/unchanged:
  `Wrote executable to <path>` / `Wrote package to <path>` / `Wrote test
  executable to <path>` / the dump `Wrote … to` lines.
- **`-q`/`--quiet`**: suppresses summary and phase → byte-identical to today.

**Blast radius (confirmed):**

- 9 integration `strip_prefix("Wrote executable to ")` sites — `native_io_runtime`,
  `macos_tls_write_capacity`, `linux_app_mode` (×2), `native_numeric_pow_div_runtime`,
  `native_float_pow_operator_runtime`, `native_size_arith_overflow`,
  `native_loop_runtime`, `fs_error_path_hygiene`. All parse **stdout** → unaffected
  (artifact line preserved).
- 432 `golden/build.log` files. `test-accept.sh` feeds them via four `mfb build`
  invocations (`:219`, `:223`, `:239`, `:244`), all captured `2>&1` into
  `build.log`, exact-compared.
- `BuildOptions` struct literals to update: `parse_build_options` (`:132`),
  `parse_test_options` (`:677`), `pkg.rs:129`, `pkg.rs:339`.

**Open Decision A — RESOLVED (recommended):** `test-accept.sh` passes `-q` on all
four `mfb build` invocations that feed `build.log`. Summary/phase are suppressed →
**zero golden churn**, goldens decoupled from summary wording. `sync-goldens.sh`
resync produces an empty diff.

**Open Decision B — RESOLVED (recommended):** summary/phase on **stderr**,
artifact line on **stdout**.

Pin the exact new wording and the full blast radius before touching code.

- [ ] Census every test/golden that asserts on build output: `strip_prefix`
      sites in `tests/*.rs` (the 7 named above — confirm the list is complete
      via `grep -rn "Wrote executable to\|Wrote package to" tests/`) and every
      `golden/build.log` under `tests/rt-error` and `tests/rt-behavior`.
- [ ] Confirm `test-accept.sh` capture semantics (`2>&1`, exact `compare_file`)
      and whether it invokes plain `mfb build` (it does; never `-v`).
- [ ] Write the final output spec into this plan: exact `Building …` wording,
      exact `phase … Nms` wording, and the target stream for each.
- [ ] Resolve Open Decision A (does the harness pass `-q`, or do goldens absorb
      the summary line).

Acceptance: this plan's §4.3/§4.4 wording is finalized and the golden blast
radius is an itemized list (file count for `build.log` churn known exactly).
Commit: —

### Phase 2 — Verbosity flags + Reporter + Normal default

The core deliverable: default concise summary, `-q` restores today's output.

- [ ] Add `Verbosity` enum + `Reporter` (`src/cli/build.rs` or
      `src/cli/reporter.rs`).
- [ ] Add `-q`/`--quiet` and `-v`/`--verbose` parsing (mutually exclusive) and
      the `verbosity` field to `BuildOptions` + its literal; extend the parser
      unit tests around `build.rs:1203`–`1258` (defaults to `Normal`; `-q -v`
      errors; each flag both spellings).
- [ ] Emit the `Building …` summary via `reporter.summary` after the
      target/kind are known; keep every `Wrote … to` line exactly as-is.
- [ ] Make `-q` suppress the summary (byte-identical to pre-plan success
      output).
- [ ] Regenerate affected `build.log` goldens with `scripts/sync-goldens.sh`
      and diff-review that **only** the summary line was added (per Open
      Decision A's outcome).
- [ ] Tests: extend `build.rs` unit tests (flag parsing + default); add/adjust
      an integration assertion that `-q` output equals the artifact line only
      and default output contains the summary.

Acceptance: `mfb build` prints the summary + artifact line; `mfb build -q`
prints only the artifact line (and diagnostics); the 7 integration tests that
parse `Wrote executable to` still pass; `scripts/test-accept.sh` is green with
regenerated goldens whose diff is limited to the summary line.
Commit: —

### Phase 3 — Verbose per-phase timing

Adds the `-v` profiler lines at front-end granularity.

- [ ] Bracket the four stage groups in `build_project` with `Instant` and call
      `reporter.phase(name, dt)` (§4.4).
- [ ] Confirm the timing code runs unconditionally (only the print is
      level-gated) so `-v` and default take an identical path into codegen.
- [ ] Tests: an integration test that `mfb build -v` on a fixture emits four
      `phase <name>` lines (match on the names/prefix, **not** the ms values)
      and still ends with the artifact line.

Acceptance: `mfb build -v <fixture>` prints `phase parse …`, `phase resolve …`,
`phase verify …`, `phase codegen+link …` (each with an integer-ms suffix) plus
the normal summary and artifact line; default and `-q` output are unchanged from
Phase 2; the acceptance suite (which never passes `-v`) is unaffected.
Commit: —

### Phase 4 — (Optional, deferred) backend sub-phase timing

Not part of the medium-effort core; listed so the boundary is explicit. Would
thread a reporter/callback into `NativeBackend::write_executable` so `-v` splits
`codegen+link` into lower_to_mir / regalloc / select / emit / link with
per-stage timings across all four targets. This changes a trait signature on
every backend and warrants its own sizing (likely a separate `plan-36-B` if
pursued). **Do not start without re-scoping.** Everything above ships and is
valuable without it.

Acceptance: n/a until scoped.
Commit: —

## Validation Plan

- Tests: `build.rs` unit tests for flag parsing (default `Normal`, both
  spellings of `-q`/`-v`, mutual-exclusion error); integration assertions for
  `-q` (artifact line only), default (summary present), and `-v` (four phase
  lines, matched by name not ms). The existing `strip_prefix("Wrote executable
  to ")` tests are the regression guard for the preserved artifact line.
- Runtime proof: `mfb build <fixture>`, `mfb build -q <fixture>`, and `mfb build
  -v <fixture>` on a real executable project — observe the three output shapes
  and confirm the produced binary runs identically in all three.
- Artifact identity: `scripts/artifact-gate.sh` (execution-free byte gate,
  [[fast-codegen-gate]]) must show byte-identical artifacts vs. pre-plan for all
  targets — proves verbosity never reached codegen.
- Doc sync: if `mfb build --help` / any CLI usage text or `src/docs/spec/tooling`
  enumerates build flags, add `-q`/`-v` there.
- Acceptance: full `scripts/test-accept.sh` green after golden resync
  ([[no-rebuild-during-acceptance]] — don't `cargo build` while it runs).

## Open Decisions

- **A. How goldens absorb the new default summary line.** (§4.3, Phase 1)
  - *Recommended:* have `test-accept.sh` invoke `mfb build -q` for the
    `build.log` capture, so goldens stay minimal and stable and are decoupled
    from cosmetic summary wording — a one-line harness change (`build.rs`
    invocation at `test-accept.sh:244`), no golden churn at all.
  - *Alternative:* keep the harness on default `mfb build` and regenerate every
    `build.log` to include the `Building …` line — larger one-time diff, and any
    future summary-wording tweak re-churns all goldens.
- **B. Summary/phase wording and stream.** (§4.1, §4.3) Recommended: summary and
  phase lines on **stderr**, artifact line stays on **stdout**; wording as in
  §4.3/§4.4. Alternative: everything on stdout (simpler, but couples the summary
  to any stdout-scraping tool).

## Summary

The engineering is small — an enum, a reporter, two flags, four `Instant`
brackets — but it sits on top of an exact-match golden contract and a
`Wrote … to` line that seven integration tests parse. The plan front-loads that
risk into a pure audit (Phase 1), keeps all non-deterministic timing strictly
inside `-v` (which the acceptance harness never runs), and preserves the
artifact line verbatim. Codegen, artifact bytes, backend traits, and diagnostic
rendering are all left untouched; the only externally observable change on a
default build is one deterministic `Building …` line.
