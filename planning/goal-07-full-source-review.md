# goal-07: Full platform source review (fresh pass) — file-by-file bug hunt

Last updated: 2026-07-28
Status: IN PROGRESS (128 / 402 files reviewed)

A fresh, independent pass over the entire shipped platform: the compiler
(`src/**` Rust), the MFBASIC-source standard library (`src/builtins/*.mfb`),
the root build script (`build.rs`), and the package-registry crate
(`repository/src/**`). Read **every production source file, one at a time**, and
hunt for defects of any kind. Per the `write-bug` convention, **no fixes are
landed as part of this goal** — each finding is captured as its own test-first
bug document and landed separately.

## Objective

Read every production source file listed in the [census](#file-census--progress)
and file a bug document for every real defect. Hunt for:

- **Correctness bugs** — wrong results, wrong control flow, off-by-one,
  incorrect edge-case handling, missed error paths, platform-divergent behavior
  (aarch64 / x86_64 / riscv64 / macOS / linux glibc+musl / Windows).
- **Memory-safety hazards** — unchecked size arithmetic (`a*b`, `a+b` before an
  allocation), OOB reads/writes, use-after-free / double-free, aliasing,
  register clobbers across helper calls (`_mfb_*` helpers destroy all x0–x17 —
  spill to stack), missing frees / leaks, wrong register lifetimes, arena
  block-offset vs. pointer confusion (records inline their `String` fields as
  block-relative offsets, never pointers), Win64 shadow-space clobbers.
- **Resource-safety hazards** — leaks, double-close, use-after-close of the
  RESOURCE plane (a RES is one pointer, not a borrow; close ≠ drop), imported-
  package resource decode gaps, thread-plane STATE transfer errors.
- **Security issues** — trust-boundary gaps (untrusted `.mfp` / manifest / lock-
  file decode, network/FS input, registry request handling), missing
  bounds/depth/rate limits, unsafe file permissions, TOCTOU, path traversal,
  injection (shell/terminal-escape), weak crypto usage, information leaks, authz
  gaps in `repository/`.
- **Footguns** — APIs or invariants easy to misuse, silent-truncation or
  silent-wrong-value paths, non-obvious ordering/lifetime requirements, panics
  on attacker- or user-reachable input, `unwrap`/`expect`/`todo!`/
  `unimplemented!` on reachable paths, narrowing integer casts (`as u32` /
  `as usize`).
- **Dead code** — unreachable branches, unused helpers/fields/variants, stale
  feature flags, commented-out code, duplicated logic that should be unified.
  No file-level `#![allow(dead_code)]`; flag any that exists.
- **Anything else worth fixing** — misleading names, incorrect comments/docs vs.
  behavior, TODO/FIXME/HACK markers that flag real gaps, spec
  (`src/docs/spec/**`) or man-page (`src/docs/man/**`) text that contradicts the
  implementation (filed against the implementing source file).

For **each item found**, create a `bugs/bug-NN-shortname.md` document (see
[Finding recording](#finding-recording)), note the id next to the file's
checkbox, then continue. The deliverable is the review coverage (every file
checked off) **plus** one bug document per real finding (batched by module when
same-class).

Do **not** fix bugs as part of this goal — find and document only.

## Scope

**402 production files, ~300,710 LOC**:

| Group | Files | LOC |
|-------|-------|-----|
| `src/**` Rust (compiler, codegen, runtime, linker, CLI) | 367 | ~262,518 |
| `src/builtins/*.mfb` (hand-written MFBASIC-source stdlib packages) | 19 | ~10,487 |
| `repository/src/**` (`mfb-repo` registry server + client) | 15 | ~27,340 |
| `build.rs` (root build script) | 1 | 365 |

The full checklist is in [§ File census & progress](#file-census--progress).

**Excluded** (not part of this review), each with its reason:

- **Unit-test modules inside the crates** — every file under a `*/tests/`
  directory, every `*/tests.rs` and `*_tests.rs`
  (e.g. `src/arch/{aarch64,riscv64,x86_64}/encode/tests.rs`, `src/ast/tests.rs`,
  the whole `src/binary_repr/tests/**` tree, `src/ir/tests.rs`,
  `src/ir/variant_corpus_tests.rs`, `src/ir/verify/tests.rs`,
  `src/os/{linux/appimage/squashfs,linux/link,macos/link}/tests.rs`,
  `src/target/shared/code/{tests.rs,regalloc/tests.rs,tls/macos/tests.rs}`),
  plus shared test fixtures `src/testutil.rs`, `src/ir/test_support.rs`, and
  `src/target/shared/code/test_support.rs`. *Reason: test code, not shipped
  behavior.* (Note: `src/testing.rs`, `src/testing/**`, `src/ast/testing.rs`,
  `src/builtins/testing.rs`, and `src/cli/build/test_mode.rs` are **in scope** —
  they are the compiler's implementation of the `mfb test` framework, not Rust
  unit tests.)
- **The root `tests/` tree and `repository/tests/`** — integration/acceptance
  tests and their fixtures. *Reason: test code.*
- **Generated `.mfb` files** — `src/builtins/unicode_gencat.mfb` and
  `src/builtins/vector_package.mfb` (emitted by `scripts/gen_regex_unicode.py`
  and `scripts/gen_vector_package.py`; both carry a `GENERATED FILE — do not
  edit` header). *Reason: machine-generated; a defect here belongs to the
  generator script, not the output.*
- **`src/docs/**`** — the embedded spec and man pages, including their two Rust
  shims (`src/docs/man/mod.rs`, `src/docs/spec/mod.rs`, which `include!` the
  build-generated tables). *Reason: documentation, not code. It is still **read
  as evidence**: a spec/man page that contradicts the implementation is a
  finding filed against the implementing source file.*
- **`scripts/`, `bindings/`, `benchmark/`, `examples/`, `tools/`** — tooling,
  sample bindings, generators, and sample programs. *Reason: not shipped
  compiler/runtime behavior.*
- **Build outputs** (`target/`, `tmp/`) and everything else `git ls-files`
  excludes. *Reason: not source.*

Test code is out of scope **unless** a production-code finding shows a test is
masking or failing to guard a real bug — note that inside the finding.

## Prior work — do NOT re-file known findings

Cross-check every candidate finding against these before writing it up:

- **`planning/completed/goal-01..goal-06`** — the five prior full-source review
  passes plus the platform security review. Each census names the bugs filed
  against each file; grep the goal doc for the path you are reviewing.
- **`bugs/completed/`** — every already-fixed bug document (`bug-01` through the
  latest completed). If you rediscover a symptom, check here first: it may be a
  regression (file it, and reference the original) or already fixed. A fixed bug
  may have no doc.
- **`bugs/` root** — currently open bug document(s), e.g.
  `bug-387-neutral-mir-stream-carries-aarch64-register-names.md`. Reference,
  don't duplicate.
- **`bugs/skipped/`** — findings deliberately not fixed: `bug-189`, `bug-218`,
  `bug-245`, `bug-270`. Do not re-file; if new evidence changes the calculus,
  write a new document that references the skipped one.
- **`bugs/repro/`** — standing reproduction fixtures for known issues.
- **`.ai/compiler.md`, `.ai/specifications.md`, `AGENTS.md`** — project
  invariants. A "bug" that contradicts a documented invariant is usually a
  misreading; check before filing.

If a file re-surfaces a *known-and-still-open* prior finding, reference that id
rather than duplicating the analysis. If it is *genuinely new*, file it fresh.

## What counts as a finding (and what doesn't)

- **Record a finding** for anything a maintainer would want fixed: wrong
  behavior, a safety/security/resource hazard, a reachable crash, a leak, or
  dead/duplicated code of non-trivial size.
- **Batch trivial findings.** Many tiny same-class nits in one module can share
  one bug document scoped to that module — but keep distinct root causes in
  distinct documents.
- **Do not file** style preferences, subjective naming, or speculative
  "could-refactor" items with no correctness/safety/clarity payoff.
- **Verify before filing.** Each finding must cite `file:line` (or
  `file:symbol`) and state the concrete failure scenario (inputs/state → wrong
  output/crash). Where a reproduction is cheap, run it against
  `target/debug/mfb` and paste the output. If you cannot construct a plausible
  trigger, mark it defense-in-depth / latent and rank it LOW — do not inflate
  severity. Per `AGENTS.md`: a claim is measured or it is a guess.
- **Consult the spec, don't guess.** Use the `mfbasic` MCP tools (`mfb_spec`,
  `mfb_man`; load schemas with `ToolSearch` first) before declaring a language
  or built-in behavior wrong.

## Finding recording

Use the project's existing convention: one `bugs/bug-NN-<shortname>.md` document
per finding, authored from the **`write-bug` skill** and its bundled template.
**Next free number: `bug-394`.** Allocate numbers in the order findings are
filed; never reuse a number from `bugs/completed/`.

Each document states: symptom, `file:line`, trigger scenario, severity, and the
suggested (test-first) fix. Per the write-bug skill, the fix is *not* landed as
part of this goal.

## Workflow

This runs to completion — review every file, not a representative sample.

1. **Pick the next unchecked file** from the census (top to bottom; a whole
   directory group at a time keeps related invariants in context).
2. **Read the file** (and enough of its callers/callees to judge reachability).
   Before judging codegen / IR / runtime-helper findings, read `.ai/compiler.md`
   (register lifetimes, the runtime completion gate); before judging language or
   built-in semantics, check `mfb_spec` / `mfb_man`.
3. **File findings** as `bugs/bug-NN-<shortname>.md` per the convention above.
4. **Check the box** (`- [ ]` → `- [x]`) and add a verdict: `clean`, or the bug
   ids filed (e.g. `bug-394, bug-395`).
5. **Update the counter** in the Status line and the [Findings ledger](#findings-ledger).
6. Repeat until every box is checked.

Batch commits by directory group, with an itemized message per `AGENTS.md`
(imperative subject + `-` bullets), e.g.
`review(goal-07): src/ir/** — file bug-394, bug-395`. Never mix review
bookkeeping with unrelated changes, stage only files you changed, and never
create a branch — commit on the current branch.

When the goal reaches COMPLETE, move this document to `planning/completed/`
(finished planning docs are moved, never deleted).

## Findings ledger

Update as findings are filed. (Severity per the finding's own effort/impact
call.)

| Bug | File(s) | Class | Severity | Status |
|-----|---------|-------|----------|--------|
| bug-394 | ast/doc_items, builtins/collections+net, x86_64/encode/emitter, riscv64/v128, cli/help, cli/resolve, cli/build/native_libs, rules/table, unicode/runtime_tables | Docs/text + 1 latent bounds (11-item batch) | LOW | Open |
| bug-395 | binary_repr/mod.rs:614, manifest/package.rs:377 | Security (path traversal / `.mfp` existence oracle via unvalidated `foreign_owner`) | MED | Open |
| bug-396 | ast/scope_privates.rs:340 | Correctness (latent) — `StateAssign.resource` not rewritten by private-rename | LOW | Open |
| bug-397 | x86_64/encode/emitter.rs:643 | Memory-safety (latent) — `cmp`/`cmp_imm` missing zero-token guard | LOW | Open |
| bug-398 | cli/resolve, manifest/mod, audit/lockfile + ~all `parse::<JsonValue>` sites | Security (DoS) — tinyjson unbounded recursion on untrusted JSON | MED | Open |
| bug-399 | monomorph/lower.rs:648,775 | Security (DoS) — no total-instantiation budget → exponential fan-out | HIGH | Open |
| bug-400 | monomorph/lower.rs:765 | Correctness (latent) — `instantiate_type` symbol collision | LOW | Open |
| bug-401 | ir/lower.rs:1312 | Correctness / compile-time blowup — inline-`TRAP` treeify exponential duplication | MED | Open |

## File census & progress

402 files, grouped by directory. Check each box as its file is reviewed.

### `src/**` — compiler, codegen, runtime, linker, CLI (367 files)

**`src/arch/`**

- [x] `src/arch/encode_operand.rs` (51 loc) — clean
- [x] `src/arch/encode_plan.rs` (177 loc) — clean
- [x] `src/arch/image.rs` (150 loc) — clean
- [x] `src/arch/mod.rs` (17 loc) — clean
- [x] `src/arch/ops.rs` (740 loc) — clean

**`src/arch/aarch64/`**

- [x] `src/arch/aarch64/backend.rs` (66 loc) — clean
- [x] `src/arch/aarch64/mod.rs` (9 loc) — clean
- [x] `src/arch/aarch64/regmodel.rs` (272 loc) — clean
- [x] `src/arch/aarch64/reloc.rs` (44 loc) — clean
- [x] `src/arch/aarch64/select.rs` (106 loc) — clean

**`src/arch/aarch64/encode/`**

- [x] `src/arch/aarch64/encode/emitter.rs` (1328 loc) — clean
- [x] `src/arch/aarch64/encode/mod.rs` (35 loc) — clean
- [x] `src/arch/aarch64/encode/operand.rs` (103 loc) — clean
- [x] `src/arch/aarch64/encode/sizing.rs` (103 loc) — clean

**`src/arch/riscv64/`**

- [x] `src/arch/riscv64/backend.rs` (55 loc) — clean
- [x] `src/arch/riscv64/mod.rs` (21 loc) — clean
- [x] `src/arch/riscv64/regmodel.rs` (254 loc) — clean
- [x] `src/arch/riscv64/reloc.rs` (48 loc) — clean
- [x] `src/arch/riscv64/select.rs` (1328 loc) — clean
- [x] `src/arch/riscv64/v128.rs` (2287 loc) — bug-394 (item 5)

**`src/arch/riscv64/encode/`**

- [x] `src/arch/riscv64/encode/emitter.rs` (915 loc) — clean
- [x] `src/arch/riscv64/encode/mod.rs` (48 loc) — clean
- [x] `src/arch/riscv64/encode/operand.rs` (95 loc) — clean
- [x] `src/arch/riscv64/encode/sizing.rs` (103 loc) — clean

**`src/arch/x86_64/`**

- [x] `src/arch/x86_64/backend.rs` (129 loc) — clean
- [x] `src/arch/x86_64/mod.rs` (18 loc) — clean
- [x] `src/arch/x86_64/regmodel.rs` (474 loc) — clean
- [x] `src/arch/x86_64/reloc.rs` (46 loc) — clean
- [x] `src/arch/x86_64/select.rs` (1322 loc) — clean

**`src/arch/x86_64/encode/`**

- [x] `src/arch/x86_64/encode/emitter.rs` (2365 loc) — bug-394 (item 4), bug-397
- [x] `src/arch/x86_64/encode/mod.rs` (62 loc) — clean
- [x] `src/arch/x86_64/encode/operand.rs` (51 loc) — clean
- [x] `src/arch/x86_64/encode/sizing.rs` (12 loc) — clean

**`src/ast/`**

- [x] `src/ast/build.rs` (241 loc) — clean
- [x] `src/ast/doc_items.rs` (355 loc) — bug-394 (item 1)
- [x] `src/ast/expr.rs` (1100 loc) — clean
- [x] `src/ast/items.rs` (612 loc) — clean
- [x] `src/ast/lexical.rs` (127 loc) — clean
- [x] `src/ast/link_items.rs` (846 loc) — clean
- [x] `src/ast/manifest.rs` (591 loc) — clean
- [x] `src/ast/mod.rs` (41 loc) — clean
- [x] `src/ast/overloads.rs` (50 loc) — clean
- [x] `src/ast/parser.rs` (349 loc) — clean
- [x] `src/ast/pipeline.rs` (265 loc) — clean
- [x] `src/ast/scope_privates.rs` (940 loc) — bug-396
- [x] `src/ast/serialize.rs` (1708 loc) — clean
- [x] `src/ast/stmt.rs` (797 loc) — clean
- [x] `src/ast/testing.rs` (154 loc) — clean
- [x] `src/ast/types.rs` (815 loc) — clean

**`src/audit/`**

- [x] `src/audit/json.rs` (639 loc) — clean
- [x] `src/audit/mod.rs` (298 loc) — clean
- [x] `src/audit/report.rs` (553 loc) — clean
- [x] `src/audit/text.rs` (442 loc) — clean

**`src/audit/collect/`**

- [x] `src/audit/collect/dependencies.rs` (220 loc) — clean
- [x] `src/audit/collect/findings.rs` (594 loc) — clean
- [x] `src/audit/collect/lockfile.rs` (192 loc) — bug-398
- [x] `src/audit/collect/mod.rs` (190 loc) — clean
- [x] `src/audit/collect/project.rs` (535 loc) — clean
- [x] `src/audit/collect/source.rs` (1388 loc) — clean

**`src/binary_repr/`**

- [x] `src/binary_repr/builder.rs` (288 loc) — clean
- [x] `src/binary_repr/mod.rs` (930 loc) — bug-395
- [x] `src/binary_repr/reader.rs` (1445 loc) — clean
- [x] `src/binary_repr/sections.rs` (1318 loc) — clean
- [x] `src/binary_repr/util.rs` (304 loc) — clean
- [x] `src/binary_repr/writer.rs` (1256 loc) — clean

**`src/builtins/`**

- [x] `src/builtins/app.rs` (178 loc) — clean
- [x] `src/builtins/audio.rs` (738 loc) — clean
- [x] `src/builtins/bits.rs` (237 loc) — clean
- [x] `src/builtins/collections.rs` (1355 loc) — bug-394 (item 2)
- [x] `src/builtins/crypto.rs` (814 loc) — clean
- [x] `src/builtins/csv.rs` (162 loc) — clean
- [x] `src/builtins/datetime.rs` (923 loc) — clean
- [x] `src/builtins/encoding.rs` (596 loc) — clean
- [x] `src/builtins/errorcode.rs` (118 loc) — clean
- [x] `src/builtins/fs.rs` (713 loc) — clean
- [x] `src/builtins/general.rs` (815 loc) — clean
- [x] `src/builtins/http.rs` (581 loc) — clean
- [x] `src/builtins/io.rs` (236 loc) — clean
- [x] `src/builtins/json.rs` (251 loc) — clean
- [x] `src/builtins/math.rs` (616 loc) — clean
- [x] `src/builtins/mod.rs` (1113 loc) — clean
- [x] `src/builtins/money.rs` (189 loc) — clean
- [x] `src/builtins/net.rs` (725 loc) — bug-394 (item 3)
- [x] `src/builtins/os.rs` (274 loc) — clean
- [x] `src/builtins/regex.rs` (298 loc) — clean
- [x] `src/builtins/resource.rs` (364 loc) — clean
- [x] `src/builtins/strings.rs` (875 loc) — clean
- [x] `src/builtins/term.rs` (477 loc) — clean
- [x] `src/builtins/testing.rs` (175 loc) — clean
- [x] `src/builtins/thread.rs` (840 loc) — clean
- [x] `src/builtins/tls.rs` (427 loc) — clean
- [x] `src/builtins/vector.rs` (770 loc) — clean

**`src/cli/`**

- [x] `src/cli/dispatch.rs` (241 loc) — clean
- [x] `src/cli/doc.rs` (237 loc) — clean
- [x] `src/cli/fmt.rs` (286 loc) — clean
- [x] `src/cli/help.rs` (224 loc) — bug-394 (item 6)
- [x] `src/cli/init.rs` (344 loc) — clean
- [x] `src/cli/man.rs` (478 loc) — clean
- [x] `src/cli/mod.rs` (512 loc) — clean
- [x] `src/cli/pkg.rs` (3296 loc) — clean
- [x] `src/cli/repo.rs` (616 loc) — clean
- [x] `src/cli/resolve.rs` (1741 loc) — bug-398, bug-394 (item 7)
- [x] `src/cli/spec.rs` (342 loc) — clean
- [x] `src/cli/version.rs` (120 loc) — clean

**`src/cli/build/`**

- [x] `src/cli/build/mod.rs` (3566 loc) — clean
- [x] `src/cli/build/native_libs.rs` (418 loc) — bug-394 (items 8, 9)
- [x] `src/cli/build/options.rs` (156 loc) — clean
- [x] `src/cli/build/packages.rs` (244 loc) — clean
- [x] `src/cli/build/resources.rs` (136 loc) — clean
- [x] `src/cli/build/signing.rs` (200 loc) — clean
- [x] `src/cli/build/test_mode.rs` (65 loc) — clean

**`src/doc/`**

- [x] `src/doc/html.rs` (734 loc) — clean
- [x] `src/doc/mod.rs` (402 loc) — clean

**`src/ir/`**

- [ ] `src/ir/binary.rs` (1701 loc)
- [ ] `src/ir/docs.rs` (210 loc)
- [ ] `src/ir/json.rs` (908 loc)
- [ ] `src/ir/link.rs` (1201 loc)
- [x] `src/ir/lower.rs` (3697 loc) — bug-401
- [ ] `src/ir/lower_link.rs` (387 loc)
- [ ] `src/ir/mod.rs` (99 loc)
- [ ] `src/ir/op.rs` (128 loc)
- [ ] `src/ir/package.rs` (303 loc)
- [ ] `src/ir/resource_escape.rs` (681 loc)
- [ ] `src/ir/types.rs` (174 loc)
- [ ] `src/ir/value.rs` (394 loc)

**`src/ir/verify/`**

- [ ] `src/ir/verify/calls.rs` (403 loc)
- [ ] `src/ir/verify/compat.rs` (719 loc)
- [ ] `src/ir/verify/link.rs` (736 loc)
- [ ] `src/ir/verify/matching.rs` (201 loc)
- [ ] `src/ir/verify/mod.rs` (1346 loc)
- [ ] `src/ir/verify/ops.rs` (766 loc)
- [ ] `src/ir/verify/resources.rs` (480 loc)
- [ ] `src/ir/verify/types.rs` (203 loc)
- [ ] `src/ir/verify/values.rs` (822 loc)

**`src/manifest/`**

- [x] `src/manifest/entry.rs` (280 loc) — clean
- [x] `src/manifest/json_edit.rs` (533 loc) — clean
- [x] `src/manifest/libraries.rs` (911 loc) — clean
- [x] `src/manifest/mod.rs` (2284 loc) — bug-398
- [x] `src/manifest/package.rs` (1609 loc) — bug-395 (2nd site)
- [x] `src/manifest/url.rs` (61 loc) — clean

**`src/monomorph/`**

- [x] `src/monomorph/helpers.rs` (977 loc) — clean
- [x] `src/monomorph/lower.rs` (2893 loc) — bug-399, bug-400
- [x] `src/monomorph/mod.rs` (108 loc) — clean

**`src/os/`**

- [ ] `src/os/link_encode.rs` (414 loc)
- [ ] `src/os/mod.rs` (60 loc)
- [ ] `src/os/note.rs` (121 loc)
- [ ] `src/os/object_plan.rs` (313 loc)

**`src/os/icon/`**

- [ ] `src/os/icon/default_png.rs` (11 loc)
- [ ] `src/os/icon/mod.rs` (156 loc)

**`src/os/linux/`**

- [ ] `src/os/linux/appdir.rs` (369 loc)
- [ ] `src/os/linux/flavor.rs` (49 loc)
- [ ] `src/os/linux/mod.rs` (318 loc)
- [ ] `src/os/linux/object.rs` (763 loc)

**`src/os/linux/appimage/`**

- [ ] `src/os/linux/appimage/mod.rs` (586 loc)

**`src/os/linux/appimage/squashfs/`**

- [ ] `src/os/linux/appimage/squashfs/mod.rs` (675 loc)

**`src/os/linux/link/`**

- [ ] `src/os/linux/link/elf.rs` (945 loc)
- [ ] `src/os/linux/link/mod.rs` (524 loc)

**`src/os/macos/`**

- [ ] `src/os/macos/icon.rs` (190 loc)
- [ ] `src/os/macos/mod.rs` (154 loc)
- [ ] `src/os/macos/object.rs` (1122 loc)

**`src/os/macos/link/`**

- [ ] `src/os/macos/link/commands.rs` (653 loc)
- [ ] `src/os/macos/link/macho.rs` (395 loc)
- [ ] `src/os/macos/link/mod.rs` (495 loc)

**`src/os/windows/`**

- [ ] `src/os/windows/mod.rs` (163 loc)
- [ ] `src/os/windows/object.rs` (922 loc)

**`src/os/windows/link/`**

- [ ] `src/os/windows/link/mod.rs` (902 loc)
- [ ] `src/os/windows/link/pe.rs` (487 loc)
- [ ] `src/os/windows/link/rsrc.rs` (400 loc)
- [ ] `src/os/windows/link/spike.rs` (407 loc)

**`src/resolver/`**

- [ ] `src/resolver/mod.rs` (1091 loc)
- [ ] `src/resolver/packages.rs` (474 loc)
- [ ] `src/resolver/resolution.rs` (2358 loc)

**`src/rules/`**

- [x] `src/rules/mod.rs` (317 loc) — clean
- [x] `src/rules/table.rs` (1476 loc) — bug-394 (item 10)

**`src/syntaxcheck/`**

- [ ] `src/syntaxcheck/builtins.rs` (2285 loc)
- [ ] `src/syntaxcheck/checking.rs` (1378 loc)
- [ ] `src/syntaxcheck/helpers.rs` (918 loc)
- [ ] `src/syntaxcheck/inference.rs` (2801 loc)
- [ ] `src/syntaxcheck/link.rs` (1933 loc)
- [ ] `src/syntaxcheck/mod.rs` (2848 loc)
- [ ] `src/syntaxcheck/resources.rs` (836 loc)
- [ ] `src/syntaxcheck/types.rs` (1028 loc)

**`src/target/linux_aarch64/`**

- [ ] `src/target/linux_aarch64/code.rs` (85 loc)
- [ ] `src/target/linux_aarch64/mod.rs` (297 loc)
- [ ] `src/target/linux_aarch64/plan.rs` (184 loc)

**`src/target/linux_common/`**

- [ ] `src/target/linux_common/code.rs` (1453 loc)
- [ ] `src/target/linux_common/mod.rs` (321 loc)
- [ ] `src/target/linux_common/plan.rs` (543 loc)

**`src/target/linux_gtk/`**

- [ ] `src/target/linux_gtk/app_io.rs` (623 loc)
- [ ] `src/target/linux_gtk/bootstrap.rs` (1001 loc)
- [ ] `src/target/linux_gtk/mod.rs` (1162 loc)
- [ ] `src/target/linux_gtk/term_draw.rs` (817 loc)

**`src/target/linux_riscv64/`**

- [ ] `src/target/linux_riscv64/code.rs` (100 loc)
- [ ] `src/target/linux_riscv64/mod.rs` (321 loc)
- [ ] `src/target/linux_riscv64/plan.rs` (175 loc)

**`src/target/linux_x86_64/`**

- [ ] `src/target/linux_x86_64/code.rs` (237 loc)
- [ ] `src/target/linux_x86_64/mod.rs` (310 loc)
- [ ] `src/target/linux_x86_64/plan.rs` (237 loc)

**`src/target/macos_aarch64/`**

- [ ] `src/target/macos_aarch64/code.rs` (881 loc)
- [ ] `src/target/macos_aarch64/mod.rs` (455 loc)
- [ ] `src/target/macos_aarch64/plan.rs` (875 loc)
- [ ] `src/target/macos_aarch64/tls.rs` (230 loc)

**`src/target/macos_aarch64/app/`**

- [ ] `src/target/macos_aarch64/app/app_io.rs` (1563 loc)
- [ ] `src/target/macos_aarch64/app/bootstrap.rs` (1349 loc)
- [ ] `src/target/macos_aarch64/app/mod.rs` (967 loc)
- [ ] `src/target/macos_aarch64/app/term_view.rs` (2823 loc)

**`src/target/package_mfp/`**

- [ ] `src/target/package_mfp/mod.rs` (575 loc)

**`src/target/shared/`**

- [ ] `src/target/shared/abi.rs` (1370 loc)
- [ ] `src/target/shared/lower.rs` (22 loc)
- [ ] `src/target/shared/mod.rs` (14 loc)
- [ ] `src/target/shared/regmodel.rs` (153 loc)

**`src/target/shared/code/`**

- [ ] `src/target/shared/code/app.rs` (130 loc)
- [ ] `src/target/shared/code/architecture_guards.rs` (141 loc)
- [ ] `src/target/shared/code/arena.rs` (1201 loc)
- [ ] `src/target/shared/code/builder_arena_transfer.rs` (1131 loc)
- [ ] `src/target/shared/code/builder_bits.rs` (313 loc)
- [ ] `src/target/shared/code/builder_collection_compare.rs` (552 loc)
- [ ] `src/target/shared/code/builder_collection_layout.rs` (2758 loc)
- [ ] `src/target/shared/code/builder_collection_queries.rs` (3504 loc)
- [ ] `src/target/shared/code/builder_collection_query.rs` (697 loc)
- [ ] `src/target/shared/code/builder_control.rs` (1653 loc)
- [ ] `src/target/shared/code/builder_conversions.rs` (1581 loc)
- [ ] `src/target/shared/code/builder_emit_helpers.rs` (439 loc)
- [ ] `src/target/shared/code/builder_error_emission.rs` (1036 loc)
- [ ] `src/target/shared/code/builder_exits.rs` (450 loc)
- [ ] `src/target/shared/code/builder_fixed_math.rs` (1054 loc)
- [ ] `src/target/shared/code/builder_fmod.rs` (194 loc)
- [ ] `src/target/shared/code/builder_fs_paths.rs` (698 loc)
- [ ] `src/target/shared/code/builder_inplace_assign.rs` (607 loc)
- [ ] `src/target/shared/code/builder_math.rs` (1329 loc)
- [ ] `src/target/shared/code/builder_money.rs` (148 loc)
- [ ] `src/target/shared/code/builder_money_math.rs` (445 loc)
- [ ] `src/target/shared/code/builder_numeric.rs` (1709 loc)
- [ ] `src/target/shared/code/builder_owned_cleanup.rs` (206 loc)
- [ ] `src/target/shared/code/builder_pow.rs` (916 loc)
- [ ] `src/target/shared/code/builder_registers.rs` (280 loc)
- [ ] `src/target/shared/code/builder_resource_cleanup.rs` (490 loc)
- [ ] `src/target/shared/code/builder_search.rs` (1210 loc)
- [ ] `src/target/shared/code/builder_simd_fixed_math.rs` (343 loc)
- [ ] `src/target/shared/code/builder_simd_float_math.rs` (2273 loc)
- [ ] `src/target/shared/code/builder_simd_math.rs` (1006 loc)
- [ ] `src/target/shared/code/builder_strings.rs` (2019 loc)
- [ ] `src/target/shared/code/builder_strings_builtins.rs` (2909 loc)
- [ ] `src/target/shared/code/builder_strings_package.rs` (442 loc)
- [ ] `src/target/shared/code/builder_thread_cleanup.rs` (224 loc)
- [ ] `src/target/shared/code/builder_value_semantics.rs` (926 loc)
- [ ] `src/target/shared/code/builder_values.rs` (1930 loc)
- [ ] `src/target/shared/code/builder_vector_inline.rs` (417 loc)
- [ ] `src/target/shared/code/code_impl.rs` (394 loc)
- [ ] `src/target/shared/code/codegen_utils.rs` (898 loc)
- [ ] `src/target/shared/code/collection_buffer.rs` (477 loc)
- [ ] `src/target/shared/code/collection_mutate.rs` (477 loc)
- [ ] `src/target/shared/code/crypto.rs` (218 loc)
- [ ] `src/target/shared/code/crypto_ec.rs` (127 loc)
- [ ] `src/target/shared/code/data_objects.rs` (1328 loc)
- [ ] `src/target/shared/code/datetime.rs` (357 loc)
- [ ] `src/target/shared/code/entry.rs` (1247 loc)
- [ ] `src/target/shared/code/error_constants.rs` (1005 loc)
- [ ] `src/target/shared/code/error_result.rs` (129 loc)
- [ ] `src/target/shared/code/float_format.rs` (596 loc)
- [ ] `src/target/shared/code/fma_fusion.rs` (308 loc)
- [ ] `src/target/shared/code/function_lowering.rs` (1002 loc)
- [ ] `src/target/shared/code/io_stdin.rs` (1285 loc)
- [ ] `src/target/shared/code/io_stdout.rs` (717 loc)
- [ ] `src/target/shared/code/io_terminal.rs` (260 loc)
- [ ] `src/target/shared/code/link_locator.rs` (665 loc)
- [ ] `src/target/shared/code/link_thunk.rs` (2838 loc)
- [ ] `src/target/shared/code/list_mutate.rs` (2510 loc)
- [ ] `src/target/shared/code/map_mutate.rs` (1571 loc)
- [ ] `src/target/shared/code/mir.rs` (1701 loc)
- [ ] `src/target/shared/code/mod.rs` (2905 loc)
- [ ] `src/target/shared/code/module_analysis.rs` (1069 loc)
- [ ] `src/target/shared/code/native_helpers.rs` (363 loc)
- [ ] `src/target/shared/code/peephole.rs` (532 loc)
- [ ] `src/target/shared/code/perf.rs` (963 loc)
- [ ] `src/target/shared/code/process_lifecycle.rs` (147 loc)
- [ ] `src/target/shared/code/rng_pcg64.rs` (204 loc)
- [ ] `src/target/shared/code/runtime_helpers.rs` (1398 loc)
- [ ] `src/target/shared/code/runtime_helpers_thread.rs` (1519 loc)
- [ ] `src/target/shared/code/simd_kernel_coeffs.rs` (109 loc)
- [ ] `src/target/shared/code/stdin_broadcast.rs` (1195 loc)
- [ ] `src/target/shared/code/term.rs` (1809 loc)
- [ ] `src/target/shared/code/term_grid.rs` (1205 loc)
- [ ] `src/target/shared/code/type_utils.rs` (443 loc)
- [ ] `src/target/shared/code/types.rs` (1284 loc)
- [ ] `src/target/shared/code/validation.rs` (659 loc)

**`src/target/shared/code/audio/`**

- [ ] `src/target/shared/code/audio/alsa.rs` (2339 loc)
- [ ] `src/target/shared/code/audio/common.rs` (68 loc)
- [ ] `src/target/shared/code/audio/macos.rs` (2910 loc)
- [ ] `src/target/shared/code/audio/mod.rs` (235 loc)
- [ ] `src/target/shared/code/audio/windows.rs` (255 loc)
- [ ] `src/target/shared/code/audio/windows_devices.rs` (327 loc)
- [ ] `src/target/shared/code/audio/windows_io.rs` (749 loc)
- [ ] `src/target/shared/code/audio/windows_open.rs` (527 loc)

**`src/target/shared/code/crypto_ec/`**

- [ ] `src/target/shared/code/crypto_ec/cng.rs` (424 loc)
- [ ] `src/target/shared/code/crypto_ec/cng_sign_verify.rs` (511 loc)
- [ ] `src/target/shared/code/crypto_ec/macos.rs` (1419 loc)
- [ ] `src/target/shared/code/crypto_ec/openssl.rs` (1678 loc)

**`src/target/shared/code/fs/`**

- [ ] `src/target/shared/code/fs/atomic.rs` (1571 loc)
- [ ] `src/target/shared/code/fs/io.rs` (2747 loc)
- [ ] `src/target/shared/code/fs/mod.rs` (231 loc)
- [ ] `src/target/shared/code/fs/paths.rs` (1767 loc)

**`src/target/shared/code/net/`**

- [ ] `src/target/shared/code/net/io.rs` (2093 loc)
- [ ] `src/target/shared/code/net/mod.rs` (997 loc)
- [ ] `src/target/shared/code/net/poll.rs` (265 loc)

**`src/target/shared/code/os/`**

- [ ] `src/target/shared/code/os/env.rs` (776 loc)
- [ ] `src/target/shared/code/os/introspect.rs` (537 loc)
- [ ] `src/target/shared/code/os/mod.rs` (367 loc)
- [ ] `src/target/shared/code/os/paths.rs` (516 loc)

**`src/target/shared/code/private/`**

- [ ] `src/target/shared/code/private/mod.rs` (1 loc)
- [ ] `src/target/shared/code/private/unicode.rs` (1071 loc)

**`src/target/shared/code/regalloc/`**

- [ ] `src/target/shared/code/regalloc/analysis.rs` (715 loc)
- [ ] `src/target/shared/code/regalloc/linear_scan.rs` (402 loc)
- [ ] `src/target/shared/code/regalloc/mod.rs` (399 loc)

**`src/target/shared/code/tls/`**

- [ ] `src/target/shared/code/tls/mod.rs` (432 loc)
- [ ] `src/target/shared/code/tls/openssl.rs` (2573 loc)
- [ ] `src/target/shared/code/tls/schannel.rs` (227 loc)
- [ ] `src/target/shared/code/tls/schannel_impl.rs` (491 loc)
- [ ] `src/target/shared/code/tls/schannel_io.rs` (339 loc)
- [ ] `src/target/shared/code/tls/schannel_read_close.rs` (466 loc)
- [ ] `src/target/shared/code/tls/schannel_server.rs` (959 loc)

**`src/target/shared/code/tls/macos/`**

- [ ] `src/target/shared/code/tls/macos/client.rs` (1490 loc)
- [ ] `src/target/shared/code/tls/macos/mod.rs` (589 loc)
- [ ] `src/target/shared/code/tls/macos/server.rs` (1788 loc)

**`src/target/shared/nir/`**

- [ ] `src/target/shared/nir/constfold.rs` (131 loc)
- [ ] `src/target/shared/nir/json.rs` (1096 loc)
- [ ] `src/target/shared/nir/lower.rs` (560 loc)
- [ ] `src/target/shared/nir/mod.rs` (407 loc)
- [ ] `src/target/shared/nir/symbols.rs` (78 loc)
- [ ] `src/target/shared/nir/visit.rs` (497 loc)

**`src/target/shared/plan/`**

- [ ] `src/target/shared/plan/function_builder.rs` (628 loc)
- [ ] `src/target/shared/plan/json.rs` (182 loc)
- [ ] `src/target/shared/plan/lower.rs` (224 loc)
- [ ] `src/target/shared/plan/mod.rs` (527 loc)
- [ ] `src/target/shared/plan/symbols.rs` (754 loc)

**`src/target/shared/runtime/`**

- [ ] `src/target/shared/runtime/app_specs.rs` (19 loc)
- [ ] `src/target/shared/runtime/audio_specs.rs` (112 loc)
- [ ] `src/target/shared/runtime/catalog.rs` (310 loc)
- [ ] `src/target/shared/runtime/crypto_specs.rs` (64 loc)
- [ ] `src/target/shared/runtime/datetime_specs.rs` (25 loc)
- [ ] `src/target/shared/runtime/fs_specs.rs` (223 loc)
- [ ] `src/target/shared/runtime/io_specs.rs` (99 loc)
- [ ] `src/target/shared/runtime/mod.rs` (157 loc)
- [ ] `src/target/shared/runtime/net_specs.rs` (137 loc)
- [ ] `src/target/shared/runtime/os_specs.rs` (105 loc)
- [ ] `src/target/shared/runtime/perf_specs.rs` (40 loc)
- [ ] `src/target/shared/runtime/term_specs.rs` (145 loc)
- [ ] `src/target/shared/runtime/thread_specs.rs` (114 loc)
- [ ] `src/target/shared/runtime/tls_specs.rs` (67 loc)
- [ ] `src/target/shared/runtime/usage.rs` (323 loc)

**`src/target/shared/validate/`**

- [ ] `src/target/shared/validate/body.rs` (887 loc)
- [ ] `src/target/shared/validate/capabilities.rs` (293 loc)
- [ ] `src/target/shared/validate/mod.rs` (437 loc)
- [ ] `src/target/shared/validate/names.rs` (126 loc)

**`src/target/win_x86_64/`**

- [ ] `src/target/win_x86_64/code.rs` (2916 loc)
- [ ] `src/target/win_x86_64/mod.rs` (394 loc)
- [ ] `src/target/win_x86_64/plan.rs` (564 loc)

**`src/target/win_x86_64/app/`**

- [ ] `src/target/win_x86_64/app/mod.rs` (1672 loc)

**`src/testing/`**

- [ ] `src/testing/coverage.rs` (420 loc)

**`src/testing/desugar/`**

- [ ] `src/testing/desugar/coverage.rs` (462 loc)
- [ ] `src/testing/desugar/driver.rs` (182 loc)
- [ ] `src/testing/desugar/expect.rs` (186 loc)
- [ ] `src/testing/desugar/mod.rs` (246 loc)
- [ ] `src/testing/desugar/placement.rs` (209 loc)

**`src/unicode/`**

- [x] `src/unicode/backend.rs` (66 loc) — clean
- [x] `src/unicode/mod.rs` (5 loc) — clean
- [x] `src/unicode/runtime_tables.rs` (573 loc) — bug-394 (item 11)

### `src/builtins/*.mfb` — hand-written stdlib packages (19 files)

**`src/builtins/`**

- [ ] `src/builtins/app_package.mfb` (27 loc)
- [ ] `src/builtins/audio_mml.mfb` (496 loc)
- [ ] `src/builtins/audio_render.mfb` (112 loc)
- [ ] `src/builtins/collections_package.mfb` (448 loc)
- [ ] `src/builtins/crypto_aead.mfb` (697 loc)
- [ ] `src/builtins/crypto_ecdsa.mfb` (120 loc)
- [ ] `src/builtins/crypto_ed25519.mfb` (567 loc)
- [ ] `src/builtins/crypto_hash.mfb` (765 loc)
- [ ] `src/builtins/crypto_util.mfb` (114 loc)
- [ ] `src/builtins/csv_package.mfb` (235 loc)
- [ ] `src/builtins/datetime_package.mfb` (1116 loc)
- [ ] `src/builtins/encoding_package.mfb` (1273 loc)
- [ ] `src/builtins/http_package.mfb` (1215 loc)
- [ ] `src/builtins/json_package.mfb` (820 loc)
- [ ] `src/builtins/money_package.mfb` (26 loc)
- [ ] `src/builtins/net_package.mfb` (338 loc)
- [ ] `src/builtins/regex_package.mfb` (1957 loc)
- [ ] `src/builtins/strings_package.mfb` (101 loc)
- [ ] `src/builtins/term_package.mfb` (60 loc)

### `repository/src/**` — mfb-repo registry server + client (15 files)

**`repository/src/`**

- [ ] `repository/src/abi.rs` (1063 loc)
- [ ] `repository/src/backfill.rs` (656 loc)
- [ ] `repository/src/blobstore.rs` (936 loc)
- [ ] `repository/src/client.rs` (4074 loc)
- [ ] `repository/src/crypto.rs` (398 loc)
- [ ] `repository/src/gc.rs` (1166 loc)
- [ ] `repository/src/lib.rs` (19 loc)
- [ ] `repository/src/local.rs` (926 loc)
- [ ] `repository/src/log.rs` (368 loc)
- [ ] `repository/src/main.rs` (1188 loc)
- [ ] `repository/src/package.rs` (840 loc)
- [ ] `repository/src/server.rs` (8831 loc)
- [ ] `repository/src/store.rs` (5621 loc)
- [ ] `repository/src/validation.rs` (172 loc)

**`repository/src/web/`**

- [ ] `repository/src/web/mod.rs` (1082 loc)

### root build script (1 file)

**`.` (repo root)**

- [ ] `build.rs` (365 loc)
