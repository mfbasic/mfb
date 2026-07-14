# goal-04: Full compiler source review (fresh pass) — file-by-file bug hunt

Last updated: 2026-07-13
Status: COMPLETE (267 / 267 files reviewed) — bugs 190–245 filed

## Objective

Read **every production source file in the compiler** (`src/**`), one file at a
time, and hunt for defects of any kind. This is a fresh, independent pass over
the whole tree. Prior passes:
[goal-01](old-plans/goal-01-compiler-source-review.md) reviewed the tree as of
2026-07-09 (263 files, bugs 09–71),
[goal-02](old-plans/goal-02-full-source-review.md) re-reviewed it as of
2026-07-10/11 (265 files, bugs 88–147), and
[goal-03](old-plans/goal-03-full-source-review.md) re-reviewed it as of
2026-07-12 (279 files, bugs 153–180, all since fixed). Since then the tree has
grown/changed to **267 production files (~202k LOC)** and further code has
landed — notably the scalar primitive front-end (plan-41) and its wiring
through binary_repr / type-table / constant-pool. **Do not assume a file is
unchanged because an earlier goal checked it — re-read it.**

Hunt for:

- **Correctness bugs** — wrong results, wrong control flow, off-by-one, incorrect
  edge-case handling, missed error paths, platform-divergent behavior (aarch64 /
  x86_64 / riscv64 / macOS / linux glibc+musl).
- **Memory-safety hazards** — unchecked size arithmetic (`a*b`, `a+b` before an
  allocation), OOB reads/writes, use-after-free / double-free, aliasing, register
  clobbers across helper calls, missing frees / leaks, wrong register lifetimes.
- **Security issues** — trust-boundary gaps (untrusted `.mfp`/manifest decode,
  network/FS input), missing bounds/depth/rate limits, unsafe file permissions,
  TOCTOU, path traversal, injection, weak crypto usage, information leaks.
- **Footguns** — APIs or invariants that are easy to misuse, silent-truncation or
  silent-wrong-value paths, non-obvious ordering/lifetime requirements, panics on
  attacker- or user-reachable input, `unwrap`/`expect`/`todo!`/`unimplemented!` on
  reachable paths, integer casts that narrow (`as u32`/`as usize`).
- **Dead code** — unreachable branches, unused helpers/fields/variants, stale
  feature flags, commented-out code, duplicated logic that should be unified.
- **Anything else worth fixing** — misleading names, incorrect comments/docs vs.
  behavior, TODO/FIXME/HACK markers that flag real gaps.

For **each item found**, create a `bug-NN-shortname.md` document in `bugs/` (see
[Finding recording](#finding-recording), below), then continue the review. The
deliverable of this goal is the review coverage (every file checked off below)
**plus** one bug document per real finding (batched by module where same-class).

Do **not** fix bugs as part of this goal — this goal's job is to *find and
document*. Each finding carries its own fix plan and is landed separately.

## Scope

**267 production files, ~202k LOC** across `src/**`. The full checklist is in
[§ File census & progress](#file-census--progress) below.

**Excluded** (not part of this review):

- **Unit/integration test code inside `src/`** — the 12 `#[cfg(test)]` modules:
  every `*/tests.rs` (`src/arch/{aarch64,riscv64,x86_64}/encode/tests.rs`,
  `src/ast/tests.rs`, `src/binary_repr/tests.rs`, `src/ir/tests.rs`,
  `src/ir/verify/tests.rs`, `src/os/{linux,macos}/link/tests.rs`,
  `src/target/shared/code/tests.rs`, `src/target/shared/code/regalloc/tests.rs`),
  plus `src/target/shared/code/test_support.rs` and
  `src/ir/coverage_tests.rs` (a `#[cfg(test)]` module despite its name).
  Test code is out of scope unless a production-code finding shows a test is
  masking or failing to guard a real bug — note that in the finding.
  *(Note: `src/ast/testing.rs`, `src/builtins/testing.rs`, and
  `src/testing/desugar.rs` are **in scope** — they implement the `TESTING`/
  `TGROUP`/`TCASE` language feature, not the compiler's own unit tests.)*
- **The root `tests/` tree** (acceptance/syntax/rt-error/rt-behavior fixtures and
  harness) — not `src/` production code.
- **The `repository/` crate** and other non-`src/` code (`bindings/`,
  `benchmark/`, `planning/`, `bugs/`, build scripts) — outside the compiler
  source tree this goal covers.
- **Generated files** — none were found under `src/` (no `@generated` /
  `DO NOT EDIT` markers, no build-script outputs checked in).
- **Non-code assets, docs, and config** — the embedded Markdown spec/man corpus
  under `src/docs/**` is data, not code; only the `*.rs` renderers/loaders there
  (`src/docs/mod.rs`, `src/docs/render.rs`, `src/docs/man/mod.rs`,
  `src/docs/spec/mod.rs`) are in scope.

## Prior work — do NOT re-file known findings

Cross-check every candidate finding against these before filing, so no
known-and-fixed issue is re-filed and no still-open issue is duplicated:

- [goal-01](old-plans/goal-01-compiler-source-review.md) — first full pass
  (bugs 09–71).
- [goal-02](old-plans/goal-02-full-source-review.md) — second full pass
  (bugs 88–147).
- [goal-03](old-plans/goal-03-full-source-review.md) — third full pass
  (bugs 153–180, all fixed).
- `bugs/completed-bugs/` — every fixed bug (through bug-181). Grep here first;
  if a candidate matches a fixed bug, confirm the fix is still present rather
  than re-filing.
- The `arena transient-churn quadratic` item
  ([allocator-20-coalesce-size-authority.md](allocator-20-coalesce-size-authority.md))
  is a known-open perf issue — reference it, do not duplicate it.

If a file re-surfaces a *known-and-still-open* prior finding, reference that
finding's ID in the new record rather than duplicating the analysis. If it's a
*genuinely new* issue, file it fresh.

## Finding recording

This repo records each finding as a `bugs/bug-NN-<shortname>.md` document.

- **Next free number: `bug-182`.** (Highest recorded is bug-181, in
  `bugs/completed-bugs/`.) Number findings sequentially from there.
- Use the **`write-bug` skill** to author each document — it triages small vs.
  large and applies the repo's bug template (symptom, `file:line`, trigger
  scenario, severity, suggested fix). New/open bug docs live directly in
  `bugs/`; they move to `bugs/completed-bugs/` once fixed (out of scope here).
- Each finding must cite `file:line` (or `file:symbol`) and state a concrete
  failure scenario (inputs/state → wrong output/crash). If you cannot construct
  a plausible trigger, mark it defense-in-depth / latent and rank it LOW — do
  not inflate severity.

## What counts as a finding (and what doesn't)

- **Record a finding** for anything that is a real defect a maintainer would
  want fixed: wrong behavior, a safety/security hazard, a reachable crash, a
  leak, or dead/duplicated code of non-trivial size.
- **Batch trivial findings.** Many tiny same-class nits in one module can share
  one record scoped to that module — but keep distinct root causes in distinct
  records.
- **Do not file** style preferences, subjective naming, or speculative
  "could-refactor" items with no correctness/safety/clarity payoff.
- **Verify before filing.** See the trigger/severity rule under
  [Finding recording](#finding-recording).

## Workflow

This runs to completion — review every file, not a representative sample.

1. **Pick the next unchecked file** from the census (top to bottom; a whole
   directory group at a time keeps related invariants in context).
2. **Read the file** (and enough of its callers/callees to judge reachability).
   For compiler / built-ins / IR / native-codegen / runtime-helper / diagnostics
   files, consult `.ai/compiler.md` (runtime completion gate, register
   lifetimes) before judging a class of finding. Load `mfb_spec` / `mfb_man` via
   `ToolSearch` when a finding depends on documented language/spec behavior.
3. **Record findings** per [Finding recording](#finding-recording) (next free
   number `bug-182`). Note the finding id(s) next to the file's checkbox.
4. **Check the box** for that file (`- [ ]` → `- [x]`) and add a one-word
   verdict: `clean`, or the finding ids filed (e.g. `bug-182, bug-183`).
5. **Update the counter** in the Status line at the top and the tallies in
   [§ Findings ledger](#findings-ledger).
6. Repeat until every box is checked.

Batch commits by directory group (e.g. "review src/binary_repr/** — file
bug-182"), with detailed itemized messages (imperative subject + `-` bullets),
never mixing the review bookkeeping with unrelated changes, and only touching
files changed this session.

## Findings ledger

Update as findings are filed. (Severity per the finding's own effort/impact
call.)

| Finding | File(s) | Class | Severity | Status |
|---------|---------|-------|----------|--------|
| bug-190 | `ir/verify/mod.rs` | memory-safety/security | HIGH | Open |
| bug-191 | `ast/expr.rs` | memory-safety | HIGH | Open |
| bug-192 | `arch/x86_64/encode/emitter.rs` | correctness | HIGH | Open |
| bug-206 | `shared/code/audio/alsa.rs` | memory-safety | HIGH | Open |
| bug-193 | `ast/testing.rs` | memory-safety | MEDIUM | Open |
| bug-194 | `ir/lower.rs` | memory-safety | MEDIUM | Open |
| bug-195 | `manifest/package.rs` (+audit/cli) | security | MEDIUM | Open |
| bug-196 | `monomorph/lower.rs` | correctness | MEDIUM | Open |
| bug-197 | `monomorph/lower.rs` | correctness | MEDIUM | Open |
| bug-198 | `resolver/resolution.rs` | correctness | MEDIUM | Open |
| bug-199 | `target/macos_aarch64/plan.rs` | correctness | MEDIUM | Open |
| bug-200 | `shared/code/builder_money.rs` | memory-safety | MEDIUM | Open |
| bug-201 | `shared/code/fs_helpers_atomic.rs` | memory-safety | MEDIUM | Open |
| bug-202 | `shared/code/tls/openssl.rs` | security | MEDIUM | Open |
| bug-203 | `linux_gtk/term_draw.rs` | correctness | MEDIUM | Open |
| bug-204 | `linux_gtk/mod.rs` | docs | MEDIUM | Open |
| bug-205 | `shared/code/runtime_helpers_thread.rs` | correctness | MEDIUM | Open |
| bug-207 | `shared/code/audio/alsa.rs` | correctness | MEDIUM | Open |
| bug-208 | `shared/code/io_helpers.rs` | memory-safety | MEDIUM | Open |
| bug-209 | `shared/plan/symbols.rs` | correctness | MEDIUM | Open |
| bug-210 | `audit/text.rs`, `cli/pkg.rs` | security | LOW | Open |
| bug-211 | `audit/collect/source.rs` | correctness | LOW | Open |
| bug-212 | `manifest/package.rs` | correctness | LOW | Open |
| bug-213 | `builtins/audio.rs`, `builtins/fs.rs` | footgun | LOW | Open |
| bug-214 | `docs/man/mod.rs` | correctness | LOW | Open |
| bug-215 | `binary_repr/reader.rs`, `writer.rs` | security | LOW | Open |
| bug-216 | `rules/table.rs` | dead-code | LOW | Open |
| bug-217 | `arch/aarch64/encode/emitter.rs` | dead-code | LOW | Open |
| bug-218 | `arch/riscv64/select.rs` | correctness | LOW | Open |
| bug-219 | `cli/resolve.rs` | correctness | LOW | Open |
| bug-220 | `cli/fmt.rs` | footgun | LOW | Open |
| bug-221 | `builtins/thread.rs` | footgun | LOW | Open |
| bug-222 | `builtins/strings.rs` | correctness | LOW | Open |
| bug-223 | `target/linux_riscv64/{mod,code}.rs` | footgun/docs | LOW | Open |
| bug-224 | `os/linux/link/elf.rs` | security | LOW | Open |
| bug-225 | `os/linux/link/{mod,elf}.rs` | memory-safety/dead-code | LOW | Open |
| bug-226 | `monomorph/{helpers,lower}.rs` | correctness | LOW | Open |
| bug-227 | `syntaxcheck/{checking,mod}.rs` | dead-code/docs | LOW | Open |
| bug-228 | `os/macos/link/commands.rs` | footgun | LOW | Open |
| bug-229 | `target/linux_x86_64/code.rs` | docs | LOW | Open |
| bug-230 | `shared/code/builder_money_math.rs` | correctness | LOW | Open |
| bug-231 | `syntaxcheck/resources.rs` | correctness | LOW | Open |
| bug-232 | `shared/code/builder_collection_queries.rs` | memory-safety/footgun | LOW | Open |
| bug-233 | `shared/code/builder_codegen_primitives.rs`, `builder_collection_layout.rs` | docs | LOW | Open |
| bug-234 | `shared/code/{builder_fs_paths,builder_collection_queries,builder_strings_package,fs_helpers_atomic}.rs` | dead-code | LOW | Open |
| bug-235 | `shared/code/builder_simd_float_math.rs` | footgun | LOW | Open |
| bug-236 | `shared/code/tls/{openssl,macos}.rs` | memory-safety | LOW | Open |
| bug-237 | `shared/code/crypto_ec/{openssl,macos}.rs` | memory-safety/docs | LOW | Open |
| bug-238 | `shared/code/link_thunk.rs` | correctness | LOW | Open |
| bug-239 | `shared/code/net/{poll,io}.rs` | footgun | LOW | Open |
| bug-240 | `linux_gtk/{bootstrap,app_io}.rs` | correctness/memory-safety/docs | LOW | Open |
| bug-241 | `macos_aarch64/app/{term_view,bootstrap,app_io}.rs` | correctness/memory-safety/dead-code | LOW | Open |
| bug-242 | `shared/code/regalloc/{analysis,linear_scan}.rs` | docs/footgun | LOW | Open |
| bug-243 | `shared/code/stdin_broadcast.rs` | footgun | LOW | Open |
| bug-244 | `shared/code/{fma_fusion,module_analysis}.rs`, `runtime/audio_specs.rs` | docs | LOW | Open |
| bug-245 | `arch/x86_64/select.rs` | footgun | LOW | Open |

Tallies: CRITICAL 0 · HIGH 4 · MEDIUM 16 · LOW 36 · **total 56** (bugs 190–245).

Two pre-existing findings were re-confirmed still-open and NOT re-filed:
`strings_specs.rs` dead runtime specs (bug-120.1) and untruthful `IO_PRINT_CLOBBERS`
(bug-120.2). The known-open arena transient-churn coalesce issue
(allocator-20-coalesce-size-authority.md) and bug-117 (GTK tearing race) were also
referenced, not duplicated.

## File census & progress

Reviewed top-to-bottom. Mark `- [x]` with a verdict when done. Grouped by
directory; LOC shown to help sequence the effort.

**`src/arch/`**

- [x] `src/arch/mod.rs` (6 loc) — clean
- [x] `src/arch/ops.rs` (714 loc) — clean

**`src/arch/aarch64/`**

- [x] `src/arch/aarch64/backend.rs` (32 loc) — clean
- [x] `src/arch/aarch64/mod.rs` (9 loc) — clean
- [x] `src/arch/aarch64/regmodel.rs` (272 loc) — clean
- [x] `src/arch/aarch64/reloc.rs` (44 loc) — clean
- [x] `src/arch/aarch64/select.rs` (101 loc) — clean

**`src/arch/aarch64/encode/`**

- [x] `src/arch/aarch64/encode/data.rs` (59 loc) — clean
- [x] `src/arch/aarch64/encode/emitter.rs` (1198 loc) — bug-217
- [x] `src/arch/aarch64/encode/mod.rs` (169 loc) — clean
- [x] `src/arch/aarch64/encode/operand.rs` (110 loc) — clean
- [x] `src/arch/aarch64/encode/sizing.rs` (146 loc) — clean

**`src/arch/riscv64/`**

- [x] `src/arch/riscv64/backend.rs` (55 loc) — clean
- [x] `src/arch/riscv64/mod.rs` (21 loc) — clean
- [x] `src/arch/riscv64/regmodel.rs` (248 loc) — clean
- [x] `src/arch/riscv64/reloc.rs` (48 loc) — clean
- [x] `src/arch/riscv64/select.rs` (1026 loc) — bug-218
- [x] `src/arch/riscv64/v128.rs` (1023 loc) — clean

**`src/arch/riscv64/encode/`**

- [x] `src/arch/riscv64/encode/data.rs` (59 loc) — clean
- [x] `src/arch/riscv64/encode/emitter.rs` (697 loc) — clean
- [x] `src/arch/riscv64/encode/mod.rs` (140 loc) — clean
- [x] `src/arch/riscv64/encode/operand.rs` (114 loc) — clean
- [x] `src/arch/riscv64/encode/sizing.rs` (190 loc) — clean

**`src/arch/x86_64/`**

- [x] `src/arch/x86_64/backend.rs` (57 loc) — clean
- [x] `src/arch/x86_64/mod.rs` (18 loc) — clean
- [x] `src/arch/x86_64/regmodel.rs` (275 loc) — clean
- [x] `src/arch/x86_64/reloc.rs` (46 loc) — clean
- [x] `src/arch/x86_64/select.rs` (1076 loc) — bug-245

**`src/arch/x86_64/encode/`**

- [x] `src/arch/x86_64/encode/data.rs` (63 loc) — clean
- [x] `src/arch/x86_64/encode/emitter.rs` (2195 loc) — bug-192
- [x] `src/arch/x86_64/encode/mod.rs` (148 loc) — clean
- [x] `src/arch/x86_64/encode/operand.rs` (83 loc) — clean
- [x] `src/arch/x86_64/encode/sizing.rs` (12 loc) — clean

**`src/ast/`**

- [x] `src/ast/expr.rs` (811 loc) — bug-191
- [x] `src/ast/items.rs` (1367 loc) — clean
- [x] `src/ast/lexical.rs` (127 loc) — clean
- [x] `src/ast/manifest.rs` (591 loc) — clean
- [x] `src/ast/mod.rs` (36 loc) — clean
- [x] `src/ast/parser.rs` (312 loc) — clean
- [x] `src/ast/serialize.rs` (1669 loc) — clean
- [x] `src/ast/stmt.rs` (738 loc) — clean
- [x] `src/ast/testing.rs` (141 loc) — bug-193
- [x] `src/ast/types.rs` (678 loc) — clean

**`src/audit/`**

- [x] `src/audit/json.rs` (552 loc) — clean
- [x] `src/audit/mod.rs` (298 loc) — clean
- [x] `src/audit/report.rs` (477 loc) — clean
- [x] `src/audit/text.rs` (402 loc) — bug-210

**`src/audit/collect/`**

- [x] `src/audit/collect/dependencies.rs` (220 loc) — bug-195
- [x] `src/audit/collect/findings.rs` (513 loc) — bug-195
- [x] `src/audit/collect/lockfile.rs` (163 loc) — clean
- [x] `src/audit/collect/mod.rs` (187 loc) — clean
- [x] `src/audit/collect/project.rs` (351 loc) — clean
- [x] `src/audit/collect/source.rs` (1112 loc) — bug-211

**`src/binary_repr/`**

- [x] `src/binary_repr/builder.rs` (273 loc) — clean
- [x] `src/binary_repr/mod.rs` (590 loc) — clean
- [x] `src/binary_repr/reader.rs` (1549 loc) — bug-215
- [x] `src/binary_repr/sections.rs` (665 loc) — clean
- [x] `src/binary_repr/util.rs` (303 loc) — clean
- [x] `src/binary_repr/writer.rs` (1076 loc) — bug-215

**`src/builtins/`**

- [x] `src/builtins/audio.rs` (687 loc) — bug-213
- [x] `src/builtins/bits.rs` (237 loc) — clean
- [x] `src/builtins/collections.rs` (533 loc) — clean
- [x] `src/builtins/crypto.rs` (814 loc) — clean
- [x] `src/builtins/csv.rs` (190 loc) — clean
- [x] `src/builtins/datetime.rs` (793 loc) — clean
- [x] `src/builtins/encoding.rs` (582 loc) — clean
- [x] `src/builtins/errorcode.rs` (118 loc) — clean
- [x] `src/builtins/fs.rs` (697 loc) — bug-213
- [x] `src/builtins/general.rs` (1502 loc) — clean
- [x] `src/builtins/http.rs` (594 loc) — clean
- [x] `src/builtins/io.rs` (126 loc) — clean
- [x] `src/builtins/json.rs` (279 loc) — clean
- [x] `src/builtins/math.rs` (600 loc) — clean
- [x] `src/builtins/mod.rs` (987 loc) — clean
- [x] `src/builtins/money.rs` (166 loc) — clean
- [x] `src/builtins/net.rs` (746 loc) — clean
- [x] `src/builtins/os.rs` (256 loc) — clean
- [x] `src/builtins/regex.rs` (304 loc) — clean
- [x] `src/builtins/resource.rs` (341 loc) — clean
- [x] `src/builtins/strings.rs` (517 loc) — bug-222
- [x] `src/builtins/term.rs` (331 loc) — clean
- [x] `src/builtins/testing.rs` (173 loc) — clean
- [x] `src/builtins/thread.rs` (770 loc) — bug-221
- [x] `src/builtins/tls.rs` (433 loc) — clean
- [x] `src/builtins/vector.rs` (791 loc) — clean

**`src/cli/`**

- [x] `src/cli/build.rs` (1793 loc) — clean
- [x] `src/cli/doc.rs` (237 loc) — clean
- [x] `src/cli/fmt.rs` (275 loc) — bug-220
- [x] `src/cli/init.rs` (324 loc) — clean
- [x] `src/cli/man.rs` (439 loc) — clean
- [x] `src/cli/mod.rs` (298 loc) — clean
- [x] `src/cli/pkg.rs` (1876 loc) — bug-210
- [x] `src/cli/repo.rs` (369 loc) — clean
- [x] `src/cli/resolve.rs` (1046 loc) — bug-219
- [x] `src/cli/spec.rs` (342 loc) — clean

**`src/docs/`**

- [x] `src/docs/mod.rs` (8 loc) — clean
- [x] `src/docs/render.rs` (957 loc) — clean

**`src/docs/man/`**

- [x] `src/docs/man/mod.rs` (319 loc) — bug-214

**`src/docs/spec/`**

- [x] `src/docs/spec/mod.rs` (139 loc) — clean

**`src/ir/`**

- [x] `src/ir/binary.rs` (1366 loc) — clean
- [x] `src/ir/json.rs` (932 loc) — clean
- [x] `src/ir/link.rs` (84 loc) — clean
- [x] `src/ir/lower.rs` (3845 loc) — bug-194
- [x] `src/ir/mod.rs` (144 loc) — clean
- [x] `src/ir/op.rs` (129 loc) — clean
- [x] `src/ir/package.rs` (321 loc) — clean
- [x] `src/ir/types.rs` (85 loc) — clean
- [x] `src/ir/value.rs` (164 loc) — clean

**`src/ir/verify/`**

- [x] `src/ir/verify/mod.rs` (4545 loc) — bug-190

**`src/manifest/`**

- [x] `src/manifest/entry.rs` (280 loc) — clean
- [x] `src/manifest/mod.rs` (702 loc) — clean
- [x] `src/manifest/package.rs` (1521 loc) — bug-195, bug-212

**`src/monomorph/`**

- [x] `src/monomorph/helpers.rs` (958 loc) — bug-226
- [x] `src/monomorph/lower.rs` (2660 loc) — bug-196, bug-197, bug-226
- [x] `src/monomorph/mod.rs` (101 loc) — clean

**`src/os/`**

- [x] `src/os/mod.rs` (2 loc) — clean

**`src/os/linux/`**

- [x] `src/os/linux/flavor.rs` (16 loc) — clean
- [x] `src/os/linux/mod.rs` (132 loc) — clean
- [x] `src/os/linux/object.rs` (1051 loc) — clean

**`src/os/linux/link/`**

- [x] `src/os/linux/link/elf.rs` (703 loc) — bug-224, bug-225
- [x] `src/os/linux/link/mod.rs` (569 loc) — bug-225

**`src/os/macos/`**

- [x] `src/os/macos/icon.rs` (203 loc) — clean
- [x] `src/os/macos/mod.rs` (147 loc) — clean
- [x] `src/os/macos/object.rs` (1410 loc) — clean

**`src/os/macos/link/`**

- [x] `src/os/macos/link/commands.rs` (552 loc) — bug-228
- [x] `src/os/macos/link/macho.rs` (295 loc) — clean
- [x] `src/os/macos/link/mod.rs` (555 loc) — clean

**`src/resolver/`**

- [x] `src/resolver/mod.rs` (1073 loc) — clean
- [x] `src/resolver/packages.rs` (460 loc) — clean
- [x] `src/resolver/resolution.rs` (2245 loc) — bug-198

**`src/rules/`**

- [x] `src/rules/mod.rs` (275 loc) — clean
- [x] `src/rules/table.rs` (1287 loc) — bug-216

**`src/syntaxcheck/`**

- [x] `src/syntaxcheck/builtins.rs` (3084 loc) — clean
- [x] `src/syntaxcheck/checking.rs` (1409 loc) — bug-227
- [x] `src/syntaxcheck/helpers.rs` (885 loc) — clean
- [x] `src/syntaxcheck/inference.rs` (2608 loc) — clean
- [x] `src/syntaxcheck/mod.rs` (2884 loc) — bug-227
- [x] `src/syntaxcheck/resources.rs` (791 loc) — bug-231
- [x] `src/syntaxcheck/types.rs` (982 loc) — clean

**`src/target/linux_aarch64/`**

- [x] `src/target/linux_aarch64/code.rs` (764 loc) — clean
- [x] `src/target/linux_aarch64/mod.rs` (413 loc) — clean
- [x] `src/target/linux_aarch64/plan.rs` (440 loc) — clean

**`src/target/linux_gtk/`**

- [x] `src/target/linux_gtk/app_io.rs` (625 loc) — bug-240
- [x] `src/target/linux_gtk/bootstrap.rs` (811 loc) — bug-240
- [x] `src/target/linux_gtk/mod.rs` (853 loc) — bug-204
- [x] `src/target/linux_gtk/term_draw.rs` (749 loc) — bug-203

**`src/target/linux_riscv64/`**

- [x] `src/target/linux_riscv64/code.rs` (753 loc) — bug-223
- [x] `src/target/linux_riscv64/mod.rs` (442 loc) — bug-223
- [x] `src/target/linux_riscv64/plan.rs` (470 loc) — clean

**`src/target/linux_x86_64/`**

- [x] `src/target/linux_x86_64/code.rs` (809 loc) — bug-229
- [x] `src/target/linux_x86_64/mod.rs` (441 loc) — clean
- [x] `src/target/linux_x86_64/plan.rs` (509 loc) — clean

**`src/target/macos_aarch64/`**

- [x] `src/target/macos_aarch64/code.rs` (796 loc) — clean
- [x] `src/target/macos_aarch64/mod.rs` (410 loc) — clean
- [x] `src/target/macos_aarch64/plan.rs` (826 loc) — bug-199
- [x] `src/target/macos_aarch64/tls.rs` (230 loc) — clean

**`src/target/macos_aarch64/app/`**

- [x] `src/target/macos_aarch64/app/app_io.rs` (1240 loc) — bug-241
- [x] `src/target/macos_aarch64/app/bootstrap.rs` (965 loc) — bug-241
- [x] `src/target/macos_aarch64/app/icon.rs` (9 loc) — clean
- [x] `src/target/macos_aarch64/app/mod.rs` (796 loc) — clean
- [x] `src/target/macos_aarch64/app/term_view.rs` (1504 loc) — bug-241

**`src/target/package_mfp/`**

- [x] `src/target/package_mfp/mod.rs` (499 loc) — clean

**`src/target/shared/`**

- [x] `src/target/shared/abi.rs` (1330 loc) — clean
- [x] `src/target/shared/lower.rs` (22 loc) — clean
- [x] `src/target/shared/mod.rs` (14 loc) — clean
- [x] `src/target/shared/regmodel.rs` (110 loc) — clean
- [x] `src/target/shared/validate.rs` (1710 loc) — clean

**`src/target/shared/code/`**

- [x] `src/target/shared/code/builder_arena_transfer.rs` (896 loc) — clean
- [x] `src/target/shared/code/builder_bits.rs` (293 loc) — clean
- [x] `src/target/shared/code/builder_codegen_primitives.rs` (2206 loc) — bug-233
- [x] `src/target/shared/code/builder_collection_compare.rs` (474 loc) — clean
- [x] `src/target/shared/code/builder_collection_layout.rs` (1872 loc) — bug-233
- [x] `src/target/shared/code/builder_collection_mutate.rs` (4426 loc) — clean
- [x] `src/target/shared/code/builder_collection_queries.rs` (1392 loc) — bug-232, bug-234
- [x] `src/target/shared/code/builder_collection_query.rs` (625 loc) — clean
- [x] `src/target/shared/code/builder_control.rs` (1485 loc) — clean
- [x] `src/target/shared/code/builder_conversions.rs` (1151 loc) — clean
- [x] `src/target/shared/code/builder_emit_helpers.rs` (525 loc) — clean
- [x] `src/target/shared/code/builder_fixed_math.rs` (1026 loc) — clean
- [x] `src/target/shared/code/builder_fs_paths.rs` (669 loc) — bug-234
- [x] `src/target/shared/code/builder_inplace_assign.rs` (612 loc) — clean
- [x] `src/target/shared/code/builder_math.rs` (1393 loc) — clean
- [x] `src/target/shared/code/builder_money.rs` (136 loc) — bug-200
- [x] `src/target/shared/code/builder_money_math.rs` (361 loc) — bug-230
- [x] `src/target/shared/code/builder_numeric.rs` (1881 loc) — clean
- [x] `src/target/shared/code/builder_pow.rs` (906 loc) — clean
- [x] `src/target/shared/code/builder_search.rs` (1120 loc) — clean
- [x] `src/target/shared/code/builder_simd_fixed_math.rs` (331 loc) — clean
- [x] `src/target/shared/code/builder_simd_float_math.rs` (1435 loc) — bug-235
- [x] `src/target/shared/code/builder_simd_math.rs` (831 loc) — clean
- [x] `src/target/shared/code/builder_strings.rs` (1686 loc) — clean
- [x] `src/target/shared/code/builder_strings_builtins.rs` (2780 loc) — clean
- [x] `src/target/shared/code/builder_strings_package.rs` (450 loc) — bug-234
- [x] `src/target/shared/code/builder_value_semantics.rs` (886 loc) — clean
- [x] `src/target/shared/code/builder_values.rs` (1758 loc) — clean
- [x] `src/target/shared/code/builder_vector_inline.rs` (363 loc) — clean
- [x] `src/target/shared/code/code_impl.rs` (329 loc) — clean
- [x] `src/target/shared/code/codegen_utils.rs` (752 loc) — clean
- [x] `src/target/shared/code/crypto.rs` (276 loc) — clean
- [x] `src/target/shared/code/crypto_ec.rs` (278 loc) — clean
- [x] `src/target/shared/code/data_objects.rs` (1300 loc) — clean
- [x] `src/target/shared/code/datetime.rs` (167 loc) — clean
- [x] `src/target/shared/code/entry_and_arena.rs` (2266 loc) — clean
- [x] `src/target/shared/code/error_constants.rs` (791 loc) — clean
- [x] `src/target/shared/code/float_format.rs` (602 loc) — clean
- [x] `src/target/shared/code/fma_fusion.rs` (303 loc) — bug-244
- [x] `src/target/shared/code/fs_helpers.rs` (153 loc) — clean
- [x] `src/target/shared/code/fs_helpers_atomic.rs` (1801 loc) — bug-201, bug-234
- [x] `src/target/shared/code/fs_helpers_io.rs` (2251 loc) — clean
- [x] `src/target/shared/code/fs_helpers_paths.rs` (1943 loc) — clean
- [x] `src/target/shared/code/function_lowering.rs` (940 loc) — clean
- [x] `src/target/shared/code/io_helpers.rs` (2252 loc) — bug-208
- [x] `src/target/shared/code/link_thunk.rs` (1076 loc) — bug-238
- [x] `src/target/shared/code/mir.rs` (1785 loc) — clean
- [x] `src/target/shared/code/mod.rs` (3386 loc) — clean
- [x] `src/target/shared/code/module_analysis.rs` (1086 loc) — bug-244
- [x] `src/target/shared/code/os.rs` (1507 loc) — clean
- [x] `src/target/shared/code/peephole.rs` (449 loc) — clean
- [x] `src/target/shared/code/runtime_helpers.rs` (981 loc) — clean
- [x] `src/target/shared/code/runtime_helpers_thread.rs` (1352 loc) — bug-205
- [x] `src/target/shared/code/serialization_utils.rs` (17 loc) — clean
- [x] `src/target/shared/code/simd_kernel_coeffs.rs` (101 loc) — clean
- [x] `src/target/shared/code/stdin_broadcast.rs` (749 loc) — bug-243
- [x] `src/target/shared/code/term.rs` (886 loc) — clean
- [x] `src/target/shared/code/term_grid.rs` (1031 loc) — clean
- [x] `src/target/shared/code/type_utils.rs` (365 loc) — clean
- [x] `src/target/shared/code/types.rs` (580 loc) — clean
- [x] `src/target/shared/code/validation.rs` (552 loc) — clean

**`src/target/shared/code/audio/`**

- [x] `src/target/shared/code/audio/alsa.rs` (1531 loc) — bug-206, bug-207
- [x] `src/target/shared/code/audio/macos.rs` (2289 loc) — clean
- [x] `src/target/shared/code/audio/mod.rs` (123 loc) — clean

**`src/target/shared/code/crypto_ec/`**

- [x] `src/target/shared/code/crypto_ec/macos.rs` (1441 loc) — bug-237
- [x] `src/target/shared/code/crypto_ec/openssl.rs` (1774 loc) — bug-237

**`src/target/shared/code/net/`**

- [x] `src/target/shared/code/net/io.rs` (1784 loc) — bug-239
- [x] `src/target/shared/code/net/mod.rs` (853 loc) — clean
- [x] `src/target/shared/code/net/poll.rs` (246 loc) — bug-239

**`src/target/shared/code/private/`**

- [x] `src/target/shared/code/private/mod.rs` (1 loc) — clean
- [x] `src/target/shared/code/private/unicode.rs` (983 loc) — clean

**`src/target/shared/code/regalloc/`**

- [x] `src/target/shared/code/regalloc/analysis.rs` (694 loc) — bug-242
- [x] `src/target/shared/code/regalloc/linear_scan.rs` (402 loc) — bug-242
- [x] `src/target/shared/code/regalloc/mod.rs` (384 loc) — clean

**`src/target/shared/code/tls/`**

- [x] `src/target/shared/code/tls/macos.rs` (3811 loc) — bug-236
- [x] `src/target/shared/code/tls/mod.rs` (416 loc) — clean
- [x] `src/target/shared/code/tls/openssl.rs` (2357 loc) — bug-202, bug-236

**`src/target/shared/nir/`**

- [x] `src/target/shared/nir/json.rs` (1051 loc) — clean
- [x] `src/target/shared/nir/lower.rs` (544 loc) — clean
- [x] `src/target/shared/nir/mod.rs` (377 loc) — clean
- [x] `src/target/shared/nir/symbols.rs` (78 loc) — clean

**`src/target/shared/plan/`**

- [x] `src/target/shared/plan/function_builder.rs` (656 loc) — clean
- [x] `src/target/shared/plan/json.rs` (181 loc) — clean
- [x] `src/target/shared/plan/lower.rs` (206 loc) — clean
- [x] `src/target/shared/plan/mod.rs` (515 loc) — clean
- [x] `src/target/shared/plan/symbols.rs` (810 loc) — bug-209

**`src/target/shared/runtime/`**

- [x] `src/target/shared/runtime/audio_specs.rs` (353 loc) — bug-244
- [x] `src/target/shared/runtime/catalog.rs` (176 loc) — clean
- [x] `src/target/shared/runtime/crypto_specs.rs` (153 loc) — clean
- [x] `src/target/shared/runtime/datetime_specs.rs` (48 loc) — clean
- [x] `src/target/shared/runtime/fs_specs.rs` (495 loc) — clean
- [x] `src/target/shared/runtime/io_specs.rs` (212 loc) — bug-120.2 (already filed, unfixed)
- [x] `src/target/shared/runtime/mod.rs` (142 loc) — clean
- [x] `src/target/shared/runtime/net_specs.rs` (627 loc) — clean
- [x] `src/target/shared/runtime/os_specs.rs` (231 loc) — clean
- [x] `src/target/shared/runtime/strings_specs.rs` (189 loc) — bug-120.1 (already filed, unfixed)
- [x] `src/target/shared/runtime/term_specs.rs` (227 loc) — clean
- [x] `src/target/shared/runtime/thread_specs.rs` (309 loc) — clean
- [x] `src/target/shared/runtime/usage.rs` (307 loc) — clean

**`src/testing/`**

- [x] `src/testing/desugar.rs` (1331 loc) — clean
