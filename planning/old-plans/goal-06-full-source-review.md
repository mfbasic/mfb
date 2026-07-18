# goal-06: Full platform source review (fresh pass) — file-by-file bug hunt

Last updated: 2026-07-18
Status: COMPLETE (307 / 307 files reviewed; bug-272..319 filed)

All 307 production files reviewed and checked off below. 48 bug documents filed
(bug-272 through bug-319). No CRITICAL findings; 8 HIGH-severity documents; the rest
MEDIUM individual findings and LOW clusters. Per the write-bug convention, no fixes
were landed as part of this goal — each finding carries its own test-first fix plan.

## Objective

Read **every production source file in the MFBASIC platform** — the compiler
(`src/**`), the MFBASIC-source standard library (`src/builtins/*.mfb`), the
build script (`build.rs`), and the package registry crate
(`repository/src/**`) — one file at a time, and hunt for defects of any kind.

This is a fresh, independent pass over the whole tree. Prior passes:
[goal-01](old-plans/goal-01-compiler-source-review.md) (2026-07-09, 263 files,
bugs 09–71), [goal-02](old-plans/goal-02-full-source-review.md) (2026-07-10/11,
265 files, bugs 88–147), [goal-03](old-plans/goal-03-full-source-review.md)
(2026-07-12, 279 files, bugs 153–180), and
[goal-04](old-plans/goal-04-full-source-review.md) (2026-07-13, 267 files,
bugs 190–245). Since goal-04 the tree has grown to **307 production files
(~253k LOC)** and substantial code has landed — notably the `Money` type
(plan-29), the resource STATE model and native stateful resources (plan-52 /
plan-53), the thread-plane STATE transfer (plan-54), native library locators
(plan-46), the `resources` manifest section and `os::resourcePath` (plan-55),
`fs::openWithin` (bug-259), and the repository blob/S3 backend work.
**Do not assume a file is unchanged because an earlier goal checked it —
re-read it.**

This pass is **wider than goal-01..04**: those covered `src/**` Rust only. This
one additionally covers the 12 hand-written `.mfb` standard-library packages,
`build.rs`, and the `repository/` crate (previously only reached by the
`audit-*` security passes, which looked for security issues only — not
correctness, leaks, or dead code).

Hunt for:

- **Correctness bugs** — wrong results, wrong control flow, off-by-one,
  incorrect edge-case handling, missed error paths, platform-divergent behavior
  (aarch64 / x86_64 / riscv64 / macOS / linux glibc+musl).
- **Memory-safety hazards** — unchecked size arithmetic (`a*b`, `a+b` before an
  allocation), OOB reads/writes, use-after-free / double-free, aliasing,
  register clobbers across helper calls, missing frees / leaks, wrong register
  lifetimes, arena block-offset vs. pointer confusion (records inline their
  `String` fields as block-relative offsets, never pointers).
- **Security issues** — trust-boundary gaps (untrusted `.mfp` / manifest decode,
  network/FS input, registry request handling), missing bounds/depth/rate
  limits, unsafe file permissions, TOCTOU, path traversal, injection, weak
  crypto usage, information leaks, authz gaps in `repository/`.
- **Footguns** — APIs or invariants that are easy to misuse, silent-truncation
  or silent-wrong-value paths, non-obvious ordering/lifetime requirements,
  panics on attacker- or user-reachable input, `unwrap`/`expect`/`todo!`/
  `unimplemented!` on reachable paths, integer casts that narrow (`as u32` /
  `as usize`).
- **Dead code** — unreachable branches, unused helpers/fields/variants, stale
  feature flags, commented-out code, duplicated logic that should be unified.
- **Anything else worth fixing** — misleading names, incorrect comments/docs vs.
  behavior, TODO/FIXME/HACK markers that flag real gaps, spec (`src/docs/spec/**`)
  or man-page text that contradicts the implementation.

For **each item found**, create a `bug-NN-shortname.md` document in `bugs/` (see
[Finding recording](#finding-recording) below), then continue the review. The
deliverable of this goal is the review coverage (every file checked off below)
**plus** one bug document per real finding (batched by module where same-class).

Do **not** fix bugs as part of this goal — this goal's job is to *find and
document*. Each finding carries its own fix plan and is landed separately.

## Scope

**307 production files, ~253k LOC**:

| Group | Files | LOC |
|-------|-------|-----|
| `src/**` Rust (compiler, codegen, runtime, linker, CLI) | 282 | ~229,600 |
| `src/builtins/*.mfb` (MFBASIC-source stdlib packages) | 12 | ~9,800 |
| `repository/src/**` (`mfb-repo` registry server + client) | 12 | ~13,100 |
| `build.rs` (root build script) | 1 | 365 |

The full checklist is in [§ File census & progress](#file-census--progress)
below.

**Excluded** (not part of this review), each with its reason:

- **Unit-test modules inside the crates** — every `*/tests.rs`
  (`src/arch/{aarch64,riscv64,x86_64}/encode/tests.rs`, `src/ast/tests.rs`,
  `src/binary_repr/tests.rs`, `src/ir/tests.rs`, `src/ir/verify/tests.rs`,
  `src/ir/coverage_tests.rs`, `src/os/{linux,macos}/link/tests.rs`,
  `src/target/shared/code/tests.rs`,
  `src/target/shared/code/regalloc/tests.rs`), plus `src/testutil.rs` and
  `src/target/shared/code/test_support.rs`. *Reason: test code, not shipped
  behavior.*
- **The root `tests/` tree and `repository/tests/`** — integration/acceptance
  tests and their fixtures. *Reason: test code.*
- **Generated files** — `src/builtins/regex_unicode.mfb` (generated by
  `scripts/gen_regex_unicode.py` from UCD 16.0.0) and
  `src/builtins/vector_package.mfb` (generated by
  `scripts/gen_vector_package.py`). *Reason: machine-generated; a defect here
  belongs to the generator script, not the output.*
- **`src/docs/**`** — the embedded spec and man pages. *Reason: documentation,
  not code. It is still **read as evidence**: a spec/man page that contradicts
  the implementation is a finding filed against the implementing source file.*
- **`scripts/`, `bindings/`, `benchmark/`, `examples/`** — tooling, sample
  bindings, and sample programs. *Reason: not shipped compiler/runtime
  behavior.*
- **Build outputs** (`target/`, `build/`) and everything else `git ls-files`
  excludes. *Reason: not source.*

Test code is out of scope **unless** a production-code finding shows a test is
masking or failing to guard a real bug — note that inside the finding.

## Prior work — do NOT re-file known findings

Cross-check every candidate finding against these before writing it up:

- **`planning/old-plans/goal-01..goal-05`** — the four prior full-source review
  passes plus the platform security review. Every file census there names the
  bugs filed against each file; grep the goal docs for the file path you are
  reviewing.
- **`planning/old-plans/audit-1-*.md`** (7 surface docs + summary) — first
  code-grounded security audit: package decode, codegen/memory, frontend,
  fs/net/thread, linker hardening, repository.
- **`planning/old-plans/audit-2-*.md`** (8 surface docs + summary, 2026-07-14)
  — second security audit: adds crypto/TLS and supply-chain surfaces. Result:
  0 CRITICAL, 0 new HIGH. Its remaining open findings were filed as
  **bug-259..271**.
- **`planning/old-plans/audit-unicode.md`** — Unicode-handling audit.
- **`bugs/completed-bugs/`** — 263 already-fixed bug documents, `bug-01`
  through `bug-271`. If you rediscover a symptom, check here first: it may be
  a regression (file it, and reference the original) or already fixed.
- **`bugs/bug-270-linker-mitigation-low-cluster.md`** — the only currently
  **open** bug document.
- **`bugs/skipped/`** — findings deliberately not fixed:
  `bug-189-supply-chain-bootstrap-downgrade.md`,
  `bug-218-riscv64-select-latent.md`, `bug-245-x86-float-branch-panic.md`.
  Do not re-file these; if new evidence changes the calculus, say so in a new
  document that references the skipped one.
- **`.ai/compiler.md`, `.ai/specifications.md`, `AGENTS.md`** — project
  invariants. A "bug" that contradicts a documented invariant is usually a
  misreading; check before filing.

If a file re-surfaces a *known-and-still-open* prior finding, reference that
finding's id in the new record rather than duplicating the analysis. If it is a
*genuinely new* issue, file it fresh.

## What counts as a finding (and what doesn't)

- **Record a finding** for anything that is a real defect a maintainer would
  want fixed: wrong behavior, a safety/security hazard, a reachable crash, a
  leak, or dead/duplicated code of non-trivial size.
- **Batch trivial findings.** Many tiny same-class nits in one module can share
  one bug document scoped to that module — but keep distinct root causes in
  distinct documents.
- **Do not file** style preferences, subjective naming, or speculative
  "could-refactor" items with no correctness/safety/clarity payoff.
- **Verify before filing.** Each finding must cite `file:line` (or
  `file:symbol`) and state the concrete failure scenario (inputs/state → wrong
  output/crash). Where a reproduction is cheap, run it against
  `target/debug/mfb` and paste the output. If you cannot construct a plausible
  trigger, note it as defense-in-depth / latent and rank it LOW — do not
  inflate severity.
- **Consult the spec, don't guess.** Use the `mfbasic` MCP tools (`mfb_spec`,
  `mfb_man`; load schemas with `ToolSearch` first) before declaring a language
  or built-in behavior wrong.

## Finding recording

Use the project's existing convention: one `bugs/bug-NN-<shortname>.md`
document per finding, authored from the **`write-bug` skill** and its bundled
template. **Next free number: `bug-272`.** Allocate numbers in the order
findings are filed, and never reuse a number from `bugs/completed-bugs/`.

Each document states: symptom, `file:line`, trigger scenario, severity, and the
suggested fix (test-first). Per the write-bug skill, a fix is *not* landed as
part of this goal.

## Workflow

This runs to completion — review every file, not a representative sample.

1. **Pick the next unchecked file** from the census (top to bottom; a whole
   directory group at a time keeps related invariants in context).
2. **Read the file** (and enough of its callers/callees to judge reachability).
   Before judging codegen / IR / runtime-helper findings, read `.ai/compiler.md`
   (register lifetimes, the runtime completion gate); before judging language
   or built-in semantics, check `mfb_spec` / `mfb_man`.
3. **File findings** as `bugs/bug-NN-<shortname>.md` per the convention above.
   Note the bug id(s) next to the file's checkbox.
4. **Check the box** for that file (`- [ ]` → `- [x]`) and add a one-word
   verdict: `clean`, or the bug ids filed (e.g. `bug-272, bug-273`).
5. **Update the counter** in the Status line at the top and the tallies in
   [§ Findings ledger](#findings-ledger).
6. Repeat until every box is checked.

Batch commits by directory group, with an itemized message per `AGENTS.md`
(imperative subject + `-` bullets), e.g.
`review(goal-06): src/ir/** — file bug-272, bug-273`. Never mix the review
bookkeeping with unrelated changes, and never create a branch — commit on the
current branch.

When the goal reaches COMPLETE, move this document to `planning/old-plans/`
(finished planning docs are moved, never deleted).

## Findings ledger

Update as findings are filed. (Severity per the finding's own effort/impact
call.)

| Bug | File(s) | Class | Severity | Status |
|-----|---------|-------|----------|--------|
| bug-272 | repository/src/client.rs, local.rs | Correctness (data loss) | HIGH | Open |
| bug-273 | repository/src/client.rs | Security (trust boundary) | MEDIUM | Open |
| bug-274 | repository/src/store.rs | Security (TOCTOU) | MEDIUM | Open |
| bug-275 | repository/src/server.rs, abi.rs | Security (DoS) | MEDIUM | Open |
| bug-276 | repository/src/{client,server,store,abi,local,main}.rs | mixed cluster | LOW | Open |
| bug-277 | src/binary_repr/reader.rs | Correctness (ABI hash) | MEDIUM | Open |
| bug-278 | src/audit/collect/source.rs | Correctness (under-report) | MEDIUM | Open |
| bug-279 | src/audit/collect/source.rs | Correctness | MEDIUM | Open |
| bug-280 | src/audit/collect/source.rs | Correctness | MEDIUM | Open |
| bug-281 | src/audit/collect/lockfile.rs, findings.rs | Correctness | MEDIUM | Open |
| bug-282 | src/binary_repr/** | Security/Dead-code cluster | LOW | Open |
| bug-283 | src/audit/** | Security/Docs cluster | LOW | Open |
| bug-284 | src/arch/{aarch64,riscv64,x86_64}/** | Footgun latent cluster | LOW | Open |
| bug-285 | src/scope_privates.rs | Correctness | MEDIUM | Open |
| bug-286 | src/ir/lower.rs | Correctness | MEDIUM | Open |
| bug-287 | src/testing.rs, src/testing/desugar.rs | Correctness | MEDIUM | Open |
| bug-288 | src/scope_privates.rs | Correctness/Footgun | MEDIUM | Open |
| bug-289 | src/ast/stmt.rs | Correctness (parser DoS) | HIGH | Open |
| bug-290 | src/escape.rs | Correctness (resource UAF) | HIGH | Open |
| bug-291 | src/escape.rs | Correctness (double-close) | HIGH | Open |
| bug-292 | src/ast/items.rs | Correctness (accepts-invalid) | MEDIUM | Open |
| bug-293 | src/fmt.rs | Correctness (semantics) | MEDIUM | Open |
| bug-294 | src/arch/x86_64/encode/emitter.rs | Correctness | MEDIUM | Open |
| bug-295 | src/arch/x86_64/encode/emitter.rs | Correctness (divergence) | MEDIUM | Open |
| bug-296 | src/arch/x86_64/select.rs | Correctness (ABI) | MEDIUM | Open |
| bug-297 | src/ir/verify/mod.rs | Memory-safety (untrusted .mfp) | MEDIUM | Open |
| bug-298 | src/cli/build.rs, manifest/mod.rs | Security (exfiltration) | MEDIUM | Open |
| bug-299 | src/fmt.rs, src/doc.rs | Correctness cluster | LOW | Open |
| bug-300 | cross-module | Docs/Dead-code cluster | LOW | Open |
| bug-301 | src/resolver, ir/verify, syntaxcheck | Correctness/Dead cluster | LOW | Open |
| bug-302 | src/builtins/json_package.mfb | Robustness (DoS crash) | HIGH | Open |
| bug-303 | src/builtins/http_package.mfb | Correctness | HIGH | Open |
| bug-304 | src/builtins/json_package.mfb | Correctness | MEDIUM | Open |
| bug-305 | src/builtins/crypto_package.mfb | Correctness (DoS hang) | MEDIUM | Open |
| bug-306 | src/builtins/*_package.mfb | Error-handling cluster | LOW | Open |
| bug-307 | src/target/shared/code/builder_collection_queries.rs | Memory-safety (leak) | MEDIUM | Open |
| bug-308 | src/target/shared/code/builder_simd_math.rs | Correctness | MEDIUM | Open |
| bug-309 | src/target/shared/code/fs_helpers_atomic.rs | Correctness (macOS broken) | HIGH | Open |
| bug-310 | src/target/shared/code/net/poll.rs | Correctness | MEDIUM | Open |
| bug-311 | src/target/shared/code/fs_helpers_io.rs | Correctness (data dup) | MEDIUM | Open |
| bug-312 | src/target/shared/code/builder_{strings,conversions,numeric}.rs | Correctness cluster | LOW | Open |
| bug-313 | src/target/shared/code/term_grid.rs | Memory-safety | MEDIUM | Open |
| bug-314 | src/target/shared/code/{io_helpers,net/io,term_grid,stdin_broadcast}.rs | Correctness cluster | LOW | Open |
| bug-315 | src/builtins/regex_package.mfb | Robustness (DoS crash+ReDoS) | HIGH | Open |
| bug-316 | src/builtins/regex_package.mfb | Correctness | MEDIUM | Open |
| bug-317 | src/target/shared/code/tls/**, crypto_ec/macos.rs | Memory-safety/Security (leak/DoS) | MEDIUM | Open |
| bug-318 | src/target/shared/code/builder_fs_paths.rs | Correctness (path/security) | MEDIUM | Open |
| bug-319 | src/target/shared/code/audio/alsa.rs | Memory-safety (leak) | MEDIUM | Open |

Tallies (bug-272..319 filed, 48 docs): CRITICAL 0 · HIGH 8 docs (272, 289, 290, 291,
302, 303, 309, 315) · MEDIUM 22 docs · LOW 10 clusters (~60 sub-items; bug-276, 282,
283, 284, 299, 300, 301, 306, 312, 314). Some docs bundle multiple sub-items; see each.
Additional latent/robustness items were appended to existing clusters (bug-284 C8;
bug-300 E9–E14) rather than filed separately.

## File census & progress

Reviewed top-to-bottom. Mark `- [x]` with a verdict when done. Grouped by
directory; LOC shown to help sequence the effort.

**`./`**

- [x] `build.rs` (365 loc) — clean

**`repository/src/`**

- [x] `repository/src/abi.rs` (509 loc) — bug-276 (R8)
- [x] `repository/src/blobstore.rs` (629 loc) — clean
- [x] `repository/src/client.rs` (1866 loc) — bug-272, bug-273, bug-276 (R1/R2/R3)
- [x] `repository/src/crypto.rs` (398 loc) — clean
- [x] `repository/src/lib.rs` (12 loc) — clean
- [x] `repository/src/local.rs` (478 loc) — bug-272, bug-276 (R9)
- [x] `repository/src/log.rs` (368 loc) — clean
- [x] `repository/src/main.rs` (584 loc) — bug-276 (R10)
- [x] `repository/src/package.rs` (838 loc) — clean
- [x] `repository/src/server.rs` (4059 loc) — bug-275, bug-276 (R4/R5)
- [x] `repository/src/store.rs` (3148 loc) — bug-274, bug-276 (R6/R7)
- [x] `repository/src/validation.rs` (172 loc) — clean

**`src/`**

- [x] `src/coverage.rs` (442 loc) — clean
- [x] `src/doc.rs` (1098 loc) — bug-299 (D3)
- [x] `src/escape.rs` (560 loc) — bug-290, bug-291
- [x] `src/fmt.rs` (959 loc) — bug-293, bug-299 (D1/D2)
- [x] `src/internal_name.rs` (149 loc) — clean
- [x] `src/lexer.rs` (1754 loc) — clean
- [x] `src/main.rs` (878 loc) — clean
- [x] `src/numeric.rs` (837 loc) — clean (root cause of bug-286 is ir/lower.rs)
- [x] `src/scope_privates.rs` (733 loc) — bug-285, bug-288
- [x] `src/target.rs` (369 loc) — bug-300 (E1)
- [x] `src/terminal_safe.rs` (95 loc) — clean
- [x] `src/testing.rs` (464 loc) — bug-287
- [x] `src/unicode_backend.rs` (66 loc) — clean
- [x] `src/unicode_runtime_tables.rs` (523 loc) — clean

**`src/arch/`**

- [x] `src/arch/mod.rs` (6 loc) — clean
- [x] `src/arch/ops.rs` (727 loc) — bug-300 (E2)

**`src/arch/aarch64/`**

- [x] `src/arch/aarch64/backend.rs` (36 loc) — clean
- [x] `src/arch/aarch64/mod.rs` (9 loc) — clean
- [x] `src/arch/aarch64/regmodel.rs` (272 loc) — clean
- [x] `src/arch/aarch64/reloc.rs` (44 loc) — clean
- [x] `src/arch/aarch64/select.rs` (101 loc) — clean

**`src/arch/aarch64/encode/`**

- [x] `src/arch/aarch64/encode/emitter.rs` (1226 loc) — bug-284 (C1)
- [x] `src/arch/aarch64/encode/mod.rs` (192 loc) — clean
- [x] `src/arch/aarch64/encode/operand.rs` (110 loc) — bug-284 (C1)
- [x] `src/arch/aarch64/encode/sizing.rs` (165 loc) — bug-284 (C2)

**`src/arch/riscv64/`**

- [x] `src/arch/riscv64/backend.rs` (55 loc) — clean
- [x] `src/arch/riscv64/mod.rs` (21 loc) — clean
- [x] `src/arch/riscv64/regmodel.rs` (255 loc) — clean
- [x] `src/arch/riscv64/reloc.rs` (48 loc) — clean
- [x] `src/arch/riscv64/select.rs` (1100 loc) — bug-284 (C3)
- [x] `src/arch/riscv64/v128.rs` (1164 loc) — bug-284 (C4/C5)

**`src/arch/riscv64/encode/`**

- [x] `src/arch/riscv64/encode/emitter.rs` (739 loc) — clean
- [x] `src/arch/riscv64/encode/mod.rs` (143 loc) — clean
- [x] `src/arch/riscv64/encode/operand.rs` (114 loc) — clean
- [x] `src/arch/riscv64/encode/sizing.rs` (197 loc) — clean

**`src/arch/x86_64/`**

- [x] `src/arch/x86_64/backend.rs` (57 loc) — clean
- [x] `src/arch/x86_64/mod.rs` (18 loc) — clean
- [x] `src/arch/x86_64/regmodel.rs` (275 loc) — bug-300 (E5)
- [x] `src/arch/x86_64/reloc.rs` (46 loc) — clean
- [x] `src/arch/x86_64/select.rs` (1085 loc) — bug-296, bug-284 (C6/C7)

**`src/arch/x86_64/encode/`**

- [x] `src/arch/x86_64/encode/emitter.rs` (2217 loc) — bug-294, bug-295, bug-284 (C6/C7)
- [x] `src/arch/x86_64/encode/mod.rs` (155 loc) — clean
- [x] `src/arch/x86_64/encode/operand.rs` (83 loc) — clean
- [x] `src/arch/x86_64/encode/sizing.rs` (12 loc) — clean

**`src/ast/`**

- [x] `src/ast/expr.rs` (873 loc) — clean
- [x] `src/ast/items.rs` (1621 loc) — bug-292
- [x] `src/ast/lexical.rs` (127 loc) — clean
- [x] `src/ast/manifest.rs` (591 loc) — clean
- [x] `src/ast/mod.rs` (36 loc) — clean
- [x] `src/ast/parser.rs` (349 loc) — clean
- [x] `src/ast/serialize.rs` (1725 loc) — bug-300 (E3)
- [x] `src/ast/stmt.rs` (786 loc) — bug-289
- [x] `src/ast/testing.rs` (154 loc) — clean
- [x] `src/ast/types.rs` (749 loc) — clean

**`src/audit/`**

- [x] `src/audit/json.rs` (552 loc) — bug-283 (A1)
- [x] `src/audit/mod.rs` (298 loc) — clean
- [x] `src/audit/report.rs` (477 loc) — clean
- [x] `src/audit/text.rs` (388 loc) — clean

**`src/audit/collect/`**

- [x] `src/audit/collect/dependencies.rs` (220 loc) — clean
- [x] `src/audit/collect/findings.rs` (513 loc) — bug-281 (shared)
- [x] `src/audit/collect/lockfile.rs` (163 loc) — bug-281
- [x] `src/audit/collect/mod.rs` (187 loc) — bug-283 (A3)
- [x] `src/audit/collect/project.rs` (355 loc) — clean
- [x] `src/audit/collect/source.rs` (1174 loc) — bug-278, bug-279, bug-280, bug-283 (A4)

**`src/binary_repr/`**

- [x] `src/binary_repr/builder.rs` (273 loc) — bug-282 (B2)
- [x] `src/binary_repr/mod.rs` (699 loc) — clean
- [x] `src/binary_repr/reader.rs` (1569 loc) — bug-277, bug-282 (B1/B3/B4)
- [x] `src/binary_repr/sections.rs` (860 loc) — bug-282 (B3)
- [x] `src/binary_repr/util.rs` (304 loc) — clean
- [x] `src/binary_repr/writer.rs` (1101 loc) — bug-282 (B4)

**`src/builtins/`**

- [x] `src/builtins/audio.rs` (757 loc) — clean
- [x] `src/builtins/audio_package.mfb` (582 loc) — clean
- [x] `src/builtins/bits.rs` (237 loc) — clean
- [x] `src/builtins/collections.rs` (533 loc) — clean
- [x] `src/builtins/collections_package.mfb` (353 loc) — clean (bug-306 S4 stale comment)
- [x] `src/builtins/crypto.rs` (814 loc) — clean
- [x] `src/builtins/crypto_package.mfb` (2262 loc) — bug-305
- [x] `src/builtins/csv.rs` (190 loc) — clean
- [x] `src/builtins/csv_package.mfb` (192 loc) — clean
- [x] `src/builtins/datetime.rs` (793 loc) — clean
- [x] `src/builtins/datetime_package.mfb` (991 loc) — bug-306 (F5)
- [x] `src/builtins/encoding.rs` (582 loc) — clean
- [x] `src/builtins/encoding_package.mfb` (1270 loc) — bug-306 (F6)
- [x] `src/builtins/errorcode.rs` (118 loc) — clean
- [x] `src/builtins/fs.rs` (712 loc) — clean
- [x] `src/builtins/general.rs` (1532 loc) — clean
- [x] `src/builtins/http.rs` (609 loc) — clean
- [x] `src/builtins/http_package.mfb` (1191 loc) — bug-303
- [x] `src/builtins/io.rs` (126 loc) — clean
- [x] `src/builtins/json.rs` (279 loc) — clean
- [x] `src/builtins/json_package.mfb` (773 loc) — bug-302, bug-304
- [x] `src/builtins/math.rs` (600 loc) — bug-300 (E6/E7)
- [x] `src/builtins/mod.rs` (1000 loc) — clean
- [x] `src/builtins/money.rs` (166 loc) — clean
- [x] `src/builtins/money_package.mfb` (19 loc) — clean
- [x] `src/builtins/net.rs` (746 loc) — clean
- [x] `src/builtins/net_package.mfb` (283 loc) — bug-306 (S3)
- [x] `src/builtins/os.rs` (280 loc) — clean
- [x] `src/builtins/regex.rs` (304 loc) — clean
- [x] `src/builtins/regex_package.mfb` (1811 loc) — bug-315, bug-316
- [x] `src/builtins/resource.rs` (361 loc) — clean
- [x] `src/builtins/strings.rs` (760 loc) — clean
- [x] `src/builtins/strings_package.mfb` (77 loc) — clean
- [x] `src/builtins/term.rs` (331 loc) — clean
- [x] `src/builtins/testing.rs` (175 loc) — clean
- [x] `src/builtins/thread.rs` (862 loc) — clean
- [x] `src/builtins/tls.rs` (433 loc) — clean
- [x] `src/builtins/vector.rs` (791 loc) — clean

**`src/cli/`**

- [x] `src/cli/build.rs` (2838 loc) — bug-298, bug-300 (E8)
- [x] `src/cli/doc.rs` (237 loc) — clean
- [x] `src/cli/fmt.rs` (286 loc) — clean
- [x] `src/cli/init.rs` (328 loc) — clean
- [x] `src/cli/man.rs` (447 loc) — clean
- [x] `src/cli/mod.rs` (339 loc) — clean
- [x] `src/cli/pkg.rs` (2093 loc) — clean
- [x] `src/cli/repo.rs` (394 loc) — clean
- [x] `src/cli/resolve.rs` (1063 loc) — clean
- [x] `src/cli/spec.rs` (342 loc) — clean
- [x] `src/cli/version.rs` (120 loc) — clean

**`src/docs/`**

- [x] `src/docs/mod.rs` (8 loc) — clean
- [x] `src/docs/render.rs` (957 loc) — clean

**`src/docs/man/`**

- [x] `src/docs/man/mod.rs` (322 loc) — clean

**`src/docs/spec/`**

- [x] `src/docs/spec/mod.rs` (139 loc) — clean

**`src/ir/`**

- [x] `src/ir/binary.rs` (1557 loc) — clean
- [x] `src/ir/json.rs` (932 loc) — clean
- [x] `src/ir/link.rs` (719 loc) — clean
- [x] `src/ir/lower.rs` (4036 loc) — bug-286 (unary-minus fold gap)
- [x] `src/ir/mod.rs` (177 loc) — clean
- [x] `src/ir/op.rs` (129 loc) — clean
- [x] `src/ir/package.rs` (365 loc) — clean
- [x] `src/ir/types.rs` (85 loc) — clean
- [x] `src/ir/value.rs` (164 loc) — clean

**`src/ir/verify/`**

- [x] `src/ir/verify/mod.rs` (5268 loc) — bug-297, bug-301 (G2)

**`src/manifest/`**

- [x] `src/manifest/entry.rs` (280 loc) — clean
- [x] `src/manifest/libraries.rs` (893 loc) — clean
- [x] `src/manifest/mod.rs` (1689 loc) — bug-298 (dst guard Unix-only)
- [x] `src/manifest/package.rs` (1562 loc) — clean

**`src/monomorph/`**

- [x] `src/monomorph/helpers.rs` (964 loc) — clean
- [x] `src/monomorph/lower.rs` (2826 loc) — clean
- [x] `src/monomorph/mod.rs` (108 loc) — clean

**`src/os/`**

- [x] `src/os/mod.rs` (40 loc) — clean
- [x] `src/os/note.rs` (121 loc) — clean

**`src/os/linux/`**

- [x] `src/os/linux/flavor.rs` (16 loc) — clean
- [x] `src/os/linux/mod.rs` (135 loc) — clean
- [x] `src/os/linux/object.rs` (1051 loc) — clean

**`src/os/linux/link/`**

- [x] `src/os/linux/link/elf.rs` (945 loc) — clean
- [x] `src/os/linux/link/mod.rs` (610 loc) — clean

**`src/os/macos/`**

- [x] `src/os/macos/icon.rs` (200 loc) — clean
- [x] `src/os/macos/mod.rs` (154 loc) — clean
- [x] `src/os/macos/object.rs` (1410 loc) — clean

**`src/os/macos/link/`**

- [x] `src/os/macos/link/commands.rs` (650 loc) — clean
- [x] `src/os/macos/link/macho.rs` (395 loc) — clean
- [x] `src/os/macos/link/mod.rs` (655 loc) — clean

**`src/resolver/`**

- [x] `src/resolver/mod.rs` (1087 loc) — clean
- [x] `src/resolver/packages.rs` (460 loc) — bug-301 (G1)
- [x] `src/resolver/resolution.rs` (2269 loc) — clean

**`src/rules/`**

- [x] `src/rules/mod.rs` (313 loc) — clean
- [x] `src/rules/table.rs` (1419 loc) — clean (all 233 rules match spec)

**`src/syntaxcheck/`**

- [x] `src/syntaxcheck/builtins.rs` (3090 loc) — clean
- [x] `src/syntaxcheck/checking.rs` (1405 loc) — clean
- [x] `src/syntaxcheck/helpers.rs` (885 loc) — clean
- [x] `src/syntaxcheck/inference.rs` (2641 loc) — clean
- [x] `src/syntaxcheck/mod.rs` (3332 loc) — bug-301 (G3/G4)
- [x] `src/syntaxcheck/resources.rs` (805 loc) — bug-301 (G4)
- [x] `src/syntaxcheck/types.rs` (1021 loc) — clean

**`src/target/linux_aarch64/`**

- [x] `src/target/linux_aarch64/code.rs` (774 loc) — clean
- [x] `src/target/linux_aarch64/mod.rs` (429 loc) — clean
- [x] `src/target/linux_aarch64/plan.rs` (458 loc) — clean

**`src/target/linux_gtk/`**

- [x] `src/target/linux_gtk/app_io.rs` (647 loc) — clean
- [x] `src/target/linux_gtk/bootstrap.rs` (843 loc) — clean
- [x] `src/target/linux_gtk/mod.rs` (874 loc) — clean
- [x] `src/target/linux_gtk/term_draw.rs` (817 loc) — clean

**`src/target/linux_riscv64/`**

- [x] `src/target/linux_riscv64/code.rs` (764 loc) — clean
- [x] `src/target/linux_riscv64/mod.rs` (458 loc) — clean
- [x] `src/target/linux_riscv64/plan.rs` (488 loc) — clean

**`src/target/linux_x86_64/`**

- [x] `src/target/linux_x86_64/code.rs` (821 loc) — bug-300 (E11)
- [x] `src/target/linux_x86_64/mod.rs` (457 loc) — clean
- [x] `src/target/linux_x86_64/plan.rs` (526 loc) — bug-300 (E10)

**`src/target/macos_aarch64/`**

- [x] `src/target/macos_aarch64/code.rs` (794 loc) — clean
- [x] `src/target/macos_aarch64/mod.rs` (437 loc) — clean
- [x] `src/target/macos_aarch64/plan.rs` (862 loc) — clean
- [x] `src/target/macos_aarch64/tls.rs` (230 loc) — clean

**`src/target/macos_aarch64/app/`**

- [x] `src/target/macos_aarch64/app/app_io.rs` (1087 loc) — clean
- [x] `src/target/macos_aarch64/app/bootstrap.rs` (978 loc) — clean
- [x] `src/target/macos_aarch64/app/icon.rs` (9 loc) — clean
- [x] `src/target/macos_aarch64/app/mod.rs` (796 loc) — clean
- [x] `src/target/macos_aarch64/app/term_view.rs` (1543 loc) — clean

**`src/target/package_mfp/`**

- [x] `src/target/package_mfp/mod.rs` (517 loc) — clean

**`src/target/shared/`**

- [x] `src/target/shared/abi.rs` (1384 loc) — clean
- [x] `src/target/shared/lower.rs` (22 loc) — clean
- [x] `src/target/shared/mod.rs` (14 loc) — clean
- [x] `src/target/shared/regmodel.rs` (110 loc) — clean
- [x] `src/target/shared/validate.rs` (1720 loc) — bug-300 (E12/E13)

**`src/target/shared/code/`**

- [x] `src/target/shared/code/builder_arena_transfer.rs` (1035 loc) — clean
- [x] `src/target/shared/code/builder_bits.rs` (311 loc) — clean
- [x] `src/target/shared/code/builder_codegen_primitives.rs` (2437 loc) — clean
- [x] `src/target/shared/code/builder_collection_compare.rs` (497 loc) — clean
- [x] `src/target/shared/code/builder_collection_layout.rs` (1960 loc) — clean
- [x] `src/target/shared/code/builder_collection_mutate.rs` (4471 loc) — clean
- [x] `src/target/shared/code/builder_collection_queries.rs` (2073 loc) — bug-307
- [x] `src/target/shared/code/builder_collection_query.rs` (674 loc) — clean
- [x] `src/target/shared/code/builder_control.rs` (1572 loc) — clean
- [x] `src/target/shared/code/builder_conversions.rs` (1499 loc) — bug-312 (K2)
- [x] `src/target/shared/code/builder_emit_helpers.rs` (525 loc) — clean
- [x] `src/target/shared/code/builder_fixed_math.rs` (1034 loc) — clean
- [x] `src/target/shared/code/builder_fs_paths.rs` (676 loc) — bug-318
- [x] `src/target/shared/code/builder_inplace_assign.rs` (624 loc) — clean
- [x] `src/target/shared/code/builder_math.rs` (1430 loc) — clean
- [x] `src/target/shared/code/builder_money.rs` (148 loc) — clean
- [x] `src/target/shared/code/builder_money_math.rs` (389 loc) — clean
- [x] `src/target/shared/code/builder_numeric.rs` (2025 loc) — bug-312 (K3)
- [x] `src/target/shared/code/builder_pow.rs` (927 loc) — clean
- [x] `src/target/shared/code/builder_search.rs` (1152 loc) — clean
- [x] `src/target/shared/code/builder_simd_fixed_math.rs` (387 loc) — clean
- [x] `src/target/shared/code/builder_simd_float_math.rs` (2372 loc) — clean
- [x] `src/target/shared/code/builder_simd_math.rs` (1002 loc) — bug-308
- [x] `src/target/shared/code/builder_strings.rs` (1796 loc) — bug-312 (K1)
- [x] `src/target/shared/code/builder_strings_builtins.rs` (2939 loc) — clean
- [x] `src/target/shared/code/builder_strings_package.rs` (448 loc) — clean
- [x] `src/target/shared/code/builder_value_semantics.rs` (890 loc) — clean
- [x] `src/target/shared/code/builder_values.rs` (1812 loc) — clean
- [x] `src/target/shared/code/builder_vector_inline.rs` (408 loc) — clean
- [x] `src/target/shared/code/code_impl.rs` (333 loc) — clean
- [x] `src/target/shared/code/codegen_utils.rs` (765 loc) — clean
- [x] `src/target/shared/code/crypto.rs` (276 loc) — clean (self-reviewed)
- [x] `src/target/shared/code/crypto_ec.rs` (278 loc) — clean
- [x] `src/target/shared/code/data_objects.rs` (1334 loc) — clean
- [x] `src/target/shared/code/datetime.rs` (167 loc) — clean
- [x] `src/target/shared/code/entry_and_arena.rs` (2379 loc) — clean
- [x] `src/target/shared/code/error_constants.rs` (841 loc) — clean
- [x] `src/target/shared/code/float_format.rs` (602 loc) — clean
- [x] `src/target/shared/code/fma_fusion.rs` (308 loc) — clean
- [x] `src/target/shared/code/fs_helpers.rs` (153 loc) — clean
- [x] `src/target/shared/code/fs_helpers_atomic.rs` (1855 loc) — bug-309
- [x] `src/target/shared/code/fs_helpers_io.rs` (2841 loc) — bug-311
- [x] `src/target/shared/code/fs_helpers_paths.rs` (1961 loc) — clean
- [x] `src/target/shared/code/function_lowering.rs` (944 loc) — clean
- [x] `src/target/shared/code/io_helpers.rs` (2290 loc) — bug-314 (H1)
- [x] `src/target/shared/code/link_locator.rs` (666 loc) — clean
- [x] `src/target/shared/code/link_thunk.rs` (2006 loc) — clean (x86 7-8 arg = bug-296)
- [x] `src/target/shared/code/mir.rs` (1797 loc) — clean
- [x] `src/target/shared/code/mod.rs` (3548 loc) — clean
- [x] `src/target/shared/code/module_analysis.rs` (1090 loc) — clean
- [x] `src/target/shared/code/os.rs` (2116 loc) — clean
- [x] `src/target/shared/code/peephole.rs` (449 loc) — bug-284 (C8)
- [x] `src/target/shared/code/runtime_helpers.rs` (1054 loc) — clean
- [x] `src/target/shared/code/runtime_helpers_thread.rs` (1457 loc) — clean
- [x] `src/target/shared/code/serialization_utils.rs` (17 loc) — clean
- [x] `src/target/shared/code/simd_kernel_coeffs.rs` (101 loc) — clean
- [x] `src/target/shared/code/stdin_broadcast.rs` (1126 loc) — bug-314 (H4)
- [x] `src/target/shared/code/term.rs` (890 loc) — clean
- [x] `src/target/shared/code/term_grid.rs` (1085 loc) — bug-313, bug-314 (H3)
- [x] `src/target/shared/code/type_utils.rs` (369 loc) — clean
- [x] `src/target/shared/code/types.rs` (745 loc) — clean
- [x] `src/target/shared/code/validation.rs` (554 loc) — bug-300 (E9)

**`src/target/shared/code/audio/`**

- [x] `src/target/shared/code/audio/alsa.rs` (2253 loc) — bug-319
- [x] `src/target/shared/code/audio/macos.rs` (2884 loc) — clean
- [x] `src/target/shared/code/audio/mod.rs` (123 loc) — clean

**`src/target/shared/code/crypto_ec/`**

- [x] `src/target/shared/code/crypto_ec/macos.rs` (1473 loc) — bug-317 (T4)
- [x] `src/target/shared/code/crypto_ec/openssl.rs` (1812 loc) — clean

**`src/target/shared/code/net/`**

- [x] `src/target/shared/code/net/io.rs` (1876 loc) — bug-314 (H2)
- [x] `src/target/shared/code/net/mod.rs` (869 loc) — clean
- [x] `src/target/shared/code/net/poll.rs` (255 loc) — bug-310

**`src/target/shared/code/private/`**

- [x] `src/target/shared/code/private/mod.rs` (1 loc) — clean
- [x] `src/target/shared/code/private/unicode.rs` (983 loc) — clean

**`src/target/shared/code/regalloc/`**

- [x] `src/target/shared/code/regalloc/analysis.rs` (710 loc) — clean (self-reviewed)
- [x] `src/target/shared/code/regalloc/linear_scan.rs` (402 loc) — clean (self-reviewed)
- [x] `src/target/shared/code/regalloc/mod.rs` (405 loc) — clean (self-reviewed)

**`src/target/shared/code/tls/`**

- [x] `src/target/shared/code/tls/macos.rs` (3960 loc) — bug-317 (T1/T3)
- [x] `src/target/shared/code/tls/mod.rs` (416 loc) — clean
- [x] `src/target/shared/code/tls/openssl.rs` (2457 loc) — bug-317 (T2)

**`src/target/shared/nir/`**

- [x] `src/target/shared/nir/json.rs` (1076 loc) — clean
- [x] `src/target/shared/nir/lower.rs` (554 loc) — clean
- [x] `src/target/shared/nir/mod.rs` (388 loc) — clean
- [x] `src/target/shared/nir/symbols.rs` (78 loc) — clean

**`src/target/shared/plan/`**

- [x] `src/target/shared/plan/function_builder.rs` (656 loc) — bug-300 (E14)
- [x] `src/target/shared/plan/json.rs` (182 loc) — clean
- [x] `src/target/shared/plan/lower.rs` (213 loc) — clean
- [x] `src/target/shared/plan/mod.rs` (522 loc) — clean
- [x] `src/target/shared/plan/symbols.rs` (841 loc) — clean

**`src/target/shared/runtime/`**

- [x] `src/target/shared/runtime/audio_specs.rs` (356 loc) — clean
- [x] `src/target/shared/runtime/catalog.rs` (178 loc) — clean
- [x] `src/target/shared/runtime/crypto_specs.rs` (153 loc) — clean
- [x] `src/target/shared/runtime/datetime_specs.rs` (48 loc) — clean
- [x] `src/target/shared/runtime/fs_specs.rs` (524 loc) — clean
- [x] `src/target/shared/runtime/io_specs.rs` (212 loc) — clean
- [x] `src/target/shared/runtime/mod.rs` (142 loc) — clean
- [x] `src/target/shared/runtime/net_specs.rs` (627 loc) — clean
- [x] `src/target/shared/runtime/os_specs.rs` (251 loc) — clean
- [x] `src/target/shared/runtime/strings_specs.rs` (189 loc) — clean (dead-code = existing bug-120, not re-filed)
- [x] `src/target/shared/runtime/term_specs.rs` (227 loc) — clean
- [x] `src/target/shared/runtime/thread_specs.rs` (309 loc) — clean
- [x] `src/target/shared/runtime/usage.rs` (308 loc) — clean

**`src/testing/`**

- [x] `src/testing/desugar.rs` (1326 loc) — bug-287 (collections/fs alias half; no new bug)
