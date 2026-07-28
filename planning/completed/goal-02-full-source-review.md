# goal-02: Full compiler source review (fresh pass) — file-by-file bug hunt

Last updated: 2026-07-10
Status: COMPLETE (265 / 265 files reviewed — all 10 groups + 2 delegated sub-slices done, 2026-07-11). Filed bugs 88–147 (60 records). Next free bug number is 148.

## Objective

Read **every production source file in the compiler** (`src/**`), one file at a
time, and hunt for defects of any kind. This is a fresh, independent pass over
the whole tree — [goal-01](goal-01-compiler-source-review.md) reviewed the
263-file tree as of 2026-07-09; since then the tree has grown to 265 production
files (~190.5k LOC) and much codegen has been refactored (plan-31 `os`,
plan-34-B/C role-named registers & vreg scratch, rv64 backend, and more). Do not
assume a file is unchanged because goal-01 checked it — re-read it.

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

For **each item found**, create a `bug-NN-shortname.md` document in `planning/`
using the project's bug template (invoke the `write-bug` skill, or copy an
existing `bug-*.md` structure), then continue the review. The deliverable of this
goal is the review coverage (every file checked off below) **plus** one bug
document per real finding.

## Scope

265 production `.rs` files, ~190.5k LOC under `src/`. **Excluded** (not part of
this review):

- Per-module test code (16 files): `**/tests.rs`, `**/coverage_tests.rs`,
  `src/ast/testing.rs`, `src/builtins/testing.rs`, `src/testing.rs`,
  `src/testutil.rs`. Test code is out of scope unless a review of production code
  reveals a test is masking or failing to guard a real bug (note it in that bug
  doc).
- Generated tables: `src/unicode_runtime_tables.rs` (523 loc, machine-generated
  Unicode data).
- Everything outside `src/`: acceptance/function tests under `tests/`, build
  scripts, `bindings/`, `benchmark/`, docs, and MFBASIC-source packages compiled
  into the binary are not `.rs` compiler source and are out of scope here.

The full checklist is in [§ File census & progress](#file-census--progress)
below.

## Prior work — do NOT re-file known findings

- **[goal-01](goal-01-compiler-source-review.md)** — COMPLETE full-tree review
  (2026-07-09); filed bugs 09–84 (6 HIGH, 22 MED, 41 LOW). Before filing, check
  whether the same root cause already has a `bug-NN-*.md` (in `planning/` or
  `planning/old-plans/`).
- **`planning/audit-1-*.md`** — targeted audits: codegen-memory, frontend,
  fs-net-thread, linker-hardening, repository, plus `audit-1-summary.md`.
- **`planning/security-review-1.md`** — a prior security-focused pass.
- **`planning/bug-01`..`bug-84`** — existing bug docs (many fixed; `bug-79`,
  `bug-81`..`bug-84` and the low-cluster follow-ups may still be open). x86
  closures/scope-drop are known-broken at baseline (`bug-83`, `bug-84`).

If a file re-surfaces a *known-and-still-open* prior finding, reference that
finding's ID in the new record rather than duplicating the analysis. If it's a
*genuinely new* issue, file it fresh.

## What counts as a finding (and what doesn't)

- **Record a finding** for anything a maintainer would want fixed: wrong
  behavior, a safety/security hazard, a reachable crash, a leak, a register
  clobber, or dead/duplicated code of non-trivial size.
- **Batch trivial findings.** Many tiny same-class nits in one module can share
  one record scoped to that module — but keep distinct root causes in distinct
  records.
- **Do not file** style preferences, subjective naming, or speculative
  "could-refactor" items with no correctness/safety/clarity payoff.
- **Verify before filing.** Each finding must cite `file:line` (or
  `file:symbol`) and state the concrete failure scenario (inputs/state → wrong
  output/crash). If you cannot construct a plausible trigger, note it as
  defense-in-depth / latent and rank it LOW — do not inflate severity. Consult
  `.ai/compiler.md` (runtime completion gate, register lifetimes) before judging
  codegen findings.

## Workflow

This runs to completion — review every file, not a representative sample.

1. **Pick the next unchecked file** from the census (top to bottom; a whole
   directory group at a time keeps related invariants in context).
2. **Read the file** (and enough of its callers/callees to judge reachability).
   For built-ins / IR / codegen / runtime helpers / diagnostics, consult
   `.ai/compiler.md` first.
3. **Record findings** as `bug-NN-shortname.md`. **Next free number is 85.** Note
   the filed id(s) next to the file's checkbox.
4. **Check the box** (`- [ ]` → `- [x]`) and add a verdict: `clean`, or the
   finding ids filed (e.g. `bug-85, bug-86`).
5. **Update the counter** in the Status line at the top and the tallies in
   [§ Findings ledger](#findings-ledger).
6. Repeat until every box is checked.

Batch commits by directory group (e.g. "review src/builtins/** — file bug-85"),
using detailed itemized messages; never mix the review bookkeeping with unrelated
changes. Commit on the current branch — never create a branch.

Do **not** fix bugs as part of this goal (unless a fix is trivial-and-obvious and
the user has asked for fixes) — this goal's job is to *find and document*. Each
finding carries its own fix plan and is landed separately.

## Findings ledger

Update as findings are filed. (Severity per the finding's own effort/impact call.)

| Finding | File(s) | Class | Severity | Status |
|---------|---------|-------|----------|--------|
| bug-88 | os/macos/link/macho.rs | footgun (u32 narrowing, code-sig emitter) | LOW | filed |
| bug-89 | ast/parser.rs + ast/expr.rs | correctness (infinite recursion → stack-overflow abort, **reproduced**) | HIGH | filed |
| bug-90 | ast/items.rs | footgun (FREE block missing SYMBOL/ABI silently dropped → native leak) | MED | filed |
| bug-91 | numeric.rs | footgun (Fixed ≥39 frac digits rejected not rounded) | LOW | filed |
| bug-92 | ast/manifest.rs | correctness (real EACCES swallowed, silent build fail) | LOW | filed |
| bug-93 | coverage.rs, testing/desugar.rs, target.rs | correctness/dead/docs cluster (anchor collisions, inline-TRAP coverage gap, stale doc) | LOW | filed |
| bug-94 | builtins/datetime.rs | correctness (fixedOffset named-arg cross-overload → wrong zone) | MED | filed |
| bug-95 | code/io_helpers.rs | memory-safety (readLine working-buffer leak/call) | MED | filed |
| bug-96 | audit/collect/source.rs | correctness (audit omits tls/http/crypto → under-reports network use) | MED | filed |
| bug-97 | code/io_helpers.rs | correctness cluster (drain re-send on retry; continuation-byte EINTR) | LOW | filed |
| bug-98 | builtins/mod.rs, builtins/general.rs | correctness/footgun cluster (any-package qualified type; replace pre-len-check panic) | LOW | filed |
| bug-99 | ir/verify/mod.rs | memory-safety (Capture in non-closure body unbounded → OOB env read, crafted .mfp) | MED | filed |
| bug-100 | binary_repr/writer.rs | dead-code (unused return-type maps) | LOW | filed |
| bug-101 | code/fs_helpers_atomic.rs | memory-safety (readText fd leak on OOM) | MED | filed |
| bug-102 | fs_helpers_atomic/entry_and_arena/tls/runtime_helpers | footgun/dead cluster (temp O_CLOEXEC, _main reloc, TLS int sign-ext, dead store) | LOW | filed |
| bug-103 | monomorph/lower.rs + mod.rs | correctness (globals/builtin-args untyped → valid generics rejected, **reproduced**) | HIGH | filed |
| bug-104 | monomorph/lower.rs + helpers.rs | correctness/nondeterminism (substring+HashSet qualifier strip, **reproduced flapping**) | MED | filed |
| bug-105 | resolver/resolution.rs | correctness (grouped type names rejected, **reproduced**) | MED | filed |
| bug-106 | resolver/resolution.rs, syntaxcheck/types.rs | correctness (non-nesting-aware func-type reparse, **reproduced**) | MED | filed |
| bug-107 | monomorph/lower.rs | footgun (monomorph diagnostics attributed to first file, **reproduced**) | MED | filed |
| bug-108 | syntaxcheck/{inference,mod,types}.rs, monomorph/helpers.rs, resolver/resolution.rs | dead-code/footgun cluster (dead relocated rules; non-nesting `TO` splitters) | LOW | filed |
| bug-109 | net/io.rs + linux_x86_64/code.rs | correctness (net.write x86 raw-syscall stale errno → wrong error) | HIGH | filed |
| bug-110 | linux_gtk/bootstrap.rs | correctness (GTK exit-code formatter missing mask) | MED | filed |
| bug-111 | linux_gtk/app_io.rs | correctness (GTK term setters ignore inactive gate) | MED | filed |
| bug-112 | macos_aarch64/app/bootstrap.rs | memory-safety (autorelease pool never drained) | MED | filed |
| bug-113 | net/mod.rs + net/io.rs | correctness (bind-all-interfaces getaddrinfo(NULL,NULL) always fails) | MED | filed |
| bug-114 | app term_view.rs + linux_gtk/bootstrap.rs | footgun (keyDown pipe write can freeze UI) | LOW | filed |
| bug-115 | net/{io,mod,poll}.rs | correctness (net blocking calls lack EINTR retry) | LOW | filed |
| bug-116 | macos_aarch64/tls.rs | memory-safety (TLS configure-block leaks sec_protocol_options) | LOW | filed |
| bug-117 | linux_riscv64/code.rs, linux_aarch64/plan.rs, linux_gtk/term_draw.rs, app/bootstrap.rs | platform LOW cluster (dead GTK hooks, dead imports, grid race, busy-spin, docs) | LOW | filed |
| bug-118 | runtime/usage.rs | correctness (required_helpers skips MATCH guards → valid program rejected) | MED | filed |
| bug-119 | runtime/usage.rs, plan/symbols.rs, validate.rs | correctness (field named `result` → false Thread helper, **reproduced**) | MED | filed |
| bug-120 | runtime/strings_specs.rs, io_specs.rs | dead-code/footgun (dead strings specs; understated clobbers) | LOW | filed |
| bug-121 | arch/riscv64/v128.rs | correctness (missing FMin/FMax/Abs scalarize → reachable ICE) | HIGH | filed |
| bug-122 | arch/riscv64/v128.rs | correctness (global v128 slot region → thread race) | MED | filed |
| bug-123 | arch/x86_64/encode/emitter.rs | correctness (discarded carry-out writes r8, latent) | MED | filed |
| bug-124 | arch/aarch64/encode/* + regmodel.rs | correctness cluster (hidden x15-x17 clobber, d8-d15 v128 lane, branch range) | LOW | filed |
| bug-125 | arch/x86_64/encode/emitter.rs, regmodel.rs | correctness/docs cluster (carry-in destroy, pool doc drift, implicit-reg clobber) | LOW | filed |
| bug-126 | arch/riscv64/{v128,select}.rs | correctness cluster (rounding ties/round-trip, gp rhs re-read, shared-branch re-emit) | LOW | filed |
| bug-127 | regalloc/{analysis,linear_scan}.rs, aarch64+riscv encode/mod.rs | footgun cluster (dup-label gap, eviction panic, %scratch cross-ISA) | LOW | filed |
| bug-128 | builder_fixed_math.rs | correctness (Fixed atan2 overflow + i64::MIN negation) | MED | filed |
| bug-129 | builder_pow.rs | correctness (fdlibm pow subnormal-range garbage/0) | HIGH | filed |
| bug-130 | builder_simd_float_math.rs | correctness (NEON exp range boundaries) | MED | filed |
| bug-131 | builder_simd_float_math.rs | correctness (Float atan2(0,0) traps NaN) | MED | filed |
| bug-132 | builder_fs_paths.rs | correctness (pathNormalize root-adjacent pop) | MED | filed |
| bug-133 | builder_search.rs | correctness (find multibyte start-offset wrong index) | HIGH | filed |
| bug-134 | builder_simd_float_math.rs | correctness (log/log10 subnormal wrong) | MED | filed |
| bug-135 | builder_numeric.rs | correctness (Float^ operator linear loop → hang) | MED | filed |
| bug-136 | crypto_ec/openssl.rs | security cluster (key scratch not wiped, verify checks, SEC1 splice) | LOW | filed |
| bug-137 | builder_{fixed_math,math,numeric,pow}.rs, fma_fusion.rs | math LOW cluster (host-libm consts, XOR bump, error code, rand bias, pow -0.0, fma) | LOW | filed |
| bug-138 | builder_simd_float_math.rs, builder_vector_inline.rs, crypto.rs | dead-code/nit cluster (dead Pow kernel, stale comment, dead self-move) | LOW | filed |
| bug-139 | plan/*, nir/* | dead-code/docs cluster (dead fold, dead CallKind::Import, dedup drop, dump omissions, defaults, thunk collision) | LOW | filed |
| bug-140 | builder_value_semantics.rs | correctness (String MATCH pointer compare, **reproduced**) | HIGH | filed |
| bug-141 | builder_collection_layout.rs + codegen_primitives.rs + arena_transfer.rs | memory-safety (resource-union return truncate + double-close → SIGSEGV, **reproduced**) | HIGH | filed |
| bug-142 | builder_inplace_assign.rs | memory-safety (FOR EACH in-place append UAF, **reproduced**) | HIGH | filed |
| bug-143 | builder_inplace_assign.rs | correctness (self-append chain wrong value, **reproduced**) | HIGH | filed |
| bug-144 | builder_conversions.rs | correctness (base-10 toInt overflow escape, **reproduced**) | MED | filed |
| bug-145 | builder_collection_mutate.rs | memory-safety (map set value path leaks intermediates) | MED | filed |
| bug-146 | builder_arena_transfer.rs | memory-safety (thread-transfer walks capacity, trusts uninit flags) | MED | filed |
| bug-147 | builder_collection_*/value_semantics/values/codegen_primitives | collections LOW cluster (float-eq nesting, dead union arms, union-field order, unaligned payload, leaks, x86 wild free, size arith) | LOW | filed |
| (dup) bug-74/75 | builder_numeric.rs | correctness (Fixed/Int pow) — **root cause refined** to operand aliasing | HIGH/MED | already filed, note added |
| (dup) LNK-01/02/03 | os/linux/link/elf.rs | security (non-PIE, no GNU_STACK, no RELRO) | HIGH/MED | pre-existing, still open in audit-1-linker-hardening |
| (dup) LNK-06/07 | os/{linux,macos}/link/mod.rs | correctness/footgun (unchecked reloc math/writes) | LOW | pre-existing, still open |
| (dup) bug-33 | ir/binary.rs | correctness (Capture.index u32 narrowing) | LOW | already filed |

Tallies (new this goal, 60 records bug-88..147): HIGH 10 · MEDIUM 26 · LOW 24
(many LOW records are per-module clusters batching several dead-code/docs/latent
items). 10 HIGH bugs, 8 of them reproduced end-to-end against `target/debug/mfb`
(bug-89, 103, 133, 140, 141, 142, 143 + monomorph/resolver 104/105/106/107).
Dups of open prior findings (audit-1 LNK-*, bug-33/74/75/77/82/85, OS-02,
MEM-04/05) referenced in place, not re-counted.

## File census & progress

Reviewed top-to-bottom. Mark `- [x]` with a verdict when done. Grouped by
directory; LOC shown to help sequence the effort.

**`src/`**

- [x] `src/coverage.rs` (274 loc) — bug-93(1) anchor collisions
- [x] `src/doc.rs` (1099 loc) — clean
- [x] `src/escape.rs` (560 loc) — clean
- [x] `src/fmt.rs` (947 loc) — clean
- [x] `src/internal_name.rs` (149 loc) — clean
- [x] `src/lexer.rs` (1516 loc) — clean
- [x] `src/main.rs` (833 loc) — clean
- [x] `src/numeric.rs` (390 loc) — bug-91 Fixed frac-digit reject
- [x] `src/scope_privates.rs` (494 loc) — clean
- [x] `src/target.rs` (294 loc) — bug-93(3) stale app-mode doc
- [x] `src/unicode_backend.rs` (66 loc) — clean

**`src/arch/`**

- [x] `src/arch/mod.rs` (3 loc) — clean

**`src/arch/aarch64/`**

- [x] `src/arch/aarch64/backend.rs` (32 loc) — clean
- [x] `src/arch/aarch64/mod.rs` (10 loc) — clean
- [x] `src/arch/aarch64/ops.rs` (714 loc) — dup of bug-82 (misfiled CodeOp variants)
- [x] `src/arch/aarch64/regmodel.rs` (227 loc) — bug-124(2) d8–d15 v128 high-lane
- [x] `src/arch/aarch64/reloc.rs` (44 loc) — clean
- [x] `src/arch/aarch64/select.rs` (101 loc) — clean

**`src/arch/aarch64/encode/`**

- [x] `src/arch/aarch64/encode/data.rs` (59 loc) — clean
- [x] `src/arch/aarch64/encode/emitter.rs` (1175 loc) — bug-124(1) scratch clobber, bug-124(3) branch range
- [x] `src/arch/aarch64/encode/mod.rs` (163 loc) — bug-127(1) dup-label guard gap
- [x] `src/arch/aarch64/encode/operand.rs` (104 loc) — bug-124(1) scratch_excluding
- [x] `src/arch/aarch64/encode/sizing.rs` (146 loc) — bug-124(3) branch_imm range

**`src/arch/riscv64/`**

- [x] `src/arch/riscv64/backend.rs` (55 loc) — clean
- [x] `src/arch/riscv64/mod.rs` (21 loc) — clean
- [x] `src/arch/riscv64/regmodel.rs` (225 loc) — clean
- [x] `src/arch/riscv64/reloc.rs` (48 loc) — clean
- [x] `src/arch/riscv64/select.rs` (730 loc) — bug-126(2) gp rhs re-read, bug-126(3) shared-branch re-emit
- [x] `src/arch/riscv64/v128.rs` (666 loc) — bug-121 (HIGH, missing scalarize→ICE), bug-122 global slot race, bug-126(1) rounding

**`src/arch/riscv64/encode/`**

- [x] `src/arch/riscv64/encode/data.rs` (59 loc) — clean
- [x] `src/arch/riscv64/encode/emitter.rs` (697 loc) — clean
- [x] `src/arch/riscv64/encode/mod.rs` (134 loc) — bug-127(1) dup-label guard gap
- [x] `src/arch/riscv64/encode/operand.rs` (114 loc) — clean
- [x] `src/arch/riscv64/encode/sizing.rs` (190 loc) — clean

**`src/arch/x86_64/`**

- [x] `src/arch/x86_64/backend.rs` (57 loc) — clean
- [x] `src/arch/x86_64/mod.rs` (18 loc) — clean
- [x] `src/arch/x86_64/regmodel.rs` (255 loc) — bug-125(2) allocatable-pool doc drift
- [x] `src/arch/x86_64/reloc.rs` (46 loc) — clean
- [x] `src/arch/x86_64/select.rs` (669 loc) — clean (residual arg-staging = dup bug-85)

**`src/arch/x86_64/encode/`**

- [x] `src/arch/x86_64/encode/data.rs` (63 loc) — clean
- [x] `src/arch/x86_64/encode/emitter.rs` (1977 loc) — bug-123 carry-out r8, bug-125(1) carry-in destroy, bug-125(3) implicit-reg clobber
- [x] `src/arch/x86_64/encode/mod.rs` (148 loc) — clean (has the bug-15 dup-label guard)
- [x] `src/arch/x86_64/encode/operand.rs` (83 loc) — clean
- [x] `src/arch/x86_64/encode/sizing.rs` (12 loc) — clean

**`src/ast/`**

- [x] `src/ast/expr.rs` (745 loc) — bug-89 (Eof infinite recursion, with parser.rs)
- [x] `src/ast/items.rs` (1333 loc) — bug-90 FREE block silently dropped
- [x] `src/ast/lexical.rs` (127 loc) — clean
- [x] `src/ast/manifest.rs` (535 loc) — bug-92 EACCES swallowed
- [x] `src/ast/mod.rs` (35 loc) — clean
- [x] `src/ast/parser.rs` (288 loc) — bug-89 (HIGH, reproduced: stack-overflow abort)
- [x] `src/ast/serialize.rs` (1644 loc) — clean
- [x] `src/ast/stmt.rs` (723 loc) — clean
- [x] `src/ast/types.rs` (675 loc) — clean

**`src/audit/`**

- [x] `src/audit/json.rs` (552 loc) — clean
- [x] `src/audit/mod.rs` (298 loc) — clean
- [x] `src/audit/report.rs` (477 loc) — clean
- [x] `src/audit/text.rs` (402 loc) — clean

**`src/audit/collect/`**

- [x] `src/audit/collect/dependencies.rs` (220 loc) — clean
- [x] `src/audit/collect/findings.rs` (513 loc) — clean
- [x] `src/audit/collect/lockfile.rs` (163 loc) — clean
- [x] `src/audit/collect/mod.rs` (187 loc) — clean
- [x] `src/audit/collect/project.rs` (351 loc) — clean
- [x] `src/audit/collect/source.rs` (1038 loc) — bug-96 missing tls/http/crypto tables

**`src/binary_repr/`**

- [x] `src/binary_repr/builder.rs` (273 loc) — clean
- [x] `src/binary_repr/mod.rs` (572 loc) — clean
- [x] `src/binary_repr/reader.rs` (1512 loc) — clean (attacker-sized allocs guarded)
- [x] `src/binary_repr/sections.rs` (645 loc) — clean
- [x] `src/binary_repr/util.rs` (303 loc) — clean
- [x] `src/binary_repr/writer.rs` (1074 loc) — bug-100 dead return-type maps

**`src/builtins/`**

- [x] `src/builtins/bits.rs` (237 loc) — clean
- [x] `src/builtins/collections.rs` (533 loc) — clean
- [x] `src/builtins/crypto.rs` (814 loc) — clean
- [x] `src/builtins/csv.rs` (190 loc) — clean
- [x] `src/builtins/datetime.rs` (773 loc) — bug-94 fixedOffset named-arg cross-overload
- [x] `src/builtins/encoding.rs` (582 loc) — clean
- [x] `src/builtins/errorcode.rs` (118 loc) — clean
- [x] `src/builtins/fs.rs` (697 loc) — clean
- [x] `src/builtins/general.rs` (1466 loc) — bug-98(2) resolve_replace_list latent panic
- [x] `src/builtins/http.rs` (594 loc) — clean
- [x] `src/builtins/io.rs` (126 loc) — clean
- [x] `src/builtins/json.rs` (279 loc) — clean
- [x] `src/builtins/math.rs` (583 loc) — clean
- [x] `src/builtins/mod.rs` (823 loc) — bug-98(1) qualified type accepts any package pairing
- [x] `src/builtins/net.rs` (743 loc) — clean
- [x] `src/builtins/os.rs` (256 loc) — clean
- [x] `src/builtins/regex.rs` (304 loc) — clean
- [x] `src/builtins/resource.rs` (285 loc) — clean
- [x] `src/builtins/strings.rs` (517 loc) — clean
- [x] `src/builtins/term.rs` (326 loc) — clean
- [x] `src/builtins/thread.rs` (732 loc) — clean
- [x] `src/builtins/tls.rs` (424 loc) — clean
- [x] `src/builtins/vector.rs` (791 loc) — clean

**`src/cli/`**

- [x] `src/cli/build.rs` (1589 loc) — clean
- [x] `src/cli/doc.rs` (237 loc) — clean
- [x] `src/cli/fmt.rs` (275 loc) — clean
- [x] `src/cli/init.rs` (306 loc) — clean
- [x] `src/cli/man.rs` (439 loc) — clean
- [x] `src/cli/mod.rs` (242 loc) — clean (bug-27 install-path traversal verified fixed)
- [x] `src/cli/pkg.rs` (1870 loc) — clean (bug-30 verified fixed)
- [x] `src/cli/repo.rs` (369 loc) — clean
- [x] `src/cli/resolve.rs` (1046 loc) — clean (bug-30 compare_versions verified fixed)
- [x] `src/cli/spec.rs` (342 loc) — clean

**`src/docs/`**

- [x] `src/docs/mod.rs` (8 loc) — clean
- [x] `src/docs/render.rs` (957 loc) — clean

**`src/docs/man/`**

- [x] `src/docs/man/mod.rs` (317 loc) — clean

**`src/docs/spec/`**

- [x] `src/docs/spec/mod.rs` (139 loc) — clean

**`src/ir/`**

- [x] `src/ir/binary.rs` (1366 loc) — Capture.index u32 narrowing (dup of bug-33 part 2)
- [x] `src/ir/json.rs` (932 loc) — clean
- [x] `src/ir/link.rs` (84 loc) — clean
- [x] `src/ir/lower.rs` (3722 loc) — clean
- [x] `src/ir/mod.rs` (144 loc) — clean
- [x] `src/ir/op.rs` (129 loc) — clean
- [x] `src/ir/package.rs` (321 loc) — clean
- [x] `src/ir/types.rs` (85 loc) — clean
- [x] `src/ir/value.rs` (164 loc) — clean

**`src/ir/verify/`**

- [x] `src/ir/verify/mod.rs` (4287 loc) — bug-99 unbounded Capture in non-closure body (MED)

**`src/manifest/`**

- [x] `src/manifest/entry.rs` (280 loc) — clean
- [x] `src/manifest/mod.rs` (558 loc) — clean
- [x] `src/manifest/package.rs` (1521 loc) — clean (decoders hardened, PKG-03..07 verified)

**`src/monomorph/`**

- [x] `src/monomorph/helpers.rs` (884 loc) — bug-108(2) TO splitters
- [x] `src/monomorph/lower.rs` (2532 loc) — bug-103 (HIGH), bug-104 nondeterminism, bug-107 wrong-file diag
- [x] `src/monomorph/mod.rs` (86 loc) — bug-103 (FunctionContext has no globals table)

**`src/os/`**

- [x] `src/os/mod.rs` (2 loc) — clean

**`src/os/linux/`**

- [x] `src/os/linux/flavor.rs` (16 loc) — clean
- [x] `src/os/linux/mod.rs` (132 loc) — clean
- [x] `src/os/linux/object.rs` (1051 loc) — clean (bug-38 static-ELF alignment verified fixed)

**`src/os/linux/link/`**

- [x] `src/os/linux/link/elf.rs` (703 loc) — dups of open audit LNK-01 (non-PIE ET_EXEC), LNK-02 (no PT_GNU_STACK), LNK-03 Linux half (no RELRO); bug-39 DT_HASH fix verified holding
- [x] `src/os/linux/link/mod.rs` (538 loc) — dups of open audit LNK-06 (unchecked reloc truncation), LNK-07 (unchecked reloc slice writes)

**`src/os/macos/`**

- [x] `src/os/macos/mod.rs` (141 loc) — clean
- [x] `src/os/macos/object.rs` (1389 loc) — clean

**`src/os/macos/link/`**

- [x] `src/os/macos/link/commands.rs` (535 loc) — clean (LNK-03 macOS half verified fixed: SG_READ_ONLY on __DATA_CONST)
- [x] `src/os/macos/link/macho.rs` (295 loc) — bug-88 (code-signature u32 narrowing); LNK-06/07 dups
- [x] `src/os/macos/link/mod.rs` (515 loc) — dups of open audit LNK-06, LNK-07

**`src/resolver/`**

- [x] `src/resolver/mod.rs` (1040 loc) — clean
- [x] `src/resolver/packages.rs` (460 loc) — clean
- [x] `src/resolver/resolution.rs` (2160 loc) — bug-105 grouped types, bug-106 func-type reparse, bug-108(2) TO splitters

**`src/rules/`**

- [x] `src/rules/mod.rs` (181 loc) — clean
- [x] `src/rules/table.rs` (1227 loc) — clean (dup diagnostic codes 2-205-0001/0002 are documented-deliberate per bug-40)

**`src/syntaxcheck/`**

- [x] `src/syntaxcheck/builtins.rs` (2708 loc) — clean
- [x] `src/syntaxcheck/checking.rs` (1437 loc) — clean
- [x] `src/syntaxcheck/helpers.rs` (910 loc) — clean
- [x] `src/syntaxcheck/inference.rs` (2253 loc) — bug-108(1) dead rules, bug-108(2) TO splitters
- [x] `src/syntaxcheck/mod.rs` (2772 loc) — bug-108(1) dead union-expand rule
- [x] `src/syntaxcheck/resources.rs` (779 loc) — clean
- [x] `src/syntaxcheck/types.rs` (837 loc) — bug-106 func-type reparse, bug-108(1) require_comparable shell

**`src/target/linux_aarch64/`**

- [x] `src/target/linux_aarch64/code.rs` (761 loc) — clean
- [x] `src/target/linux_aarch64/mod.rs` (388 loc) — clean
- [x] `src/target/linux_aarch64/plan.rs` (367 loc) — bug-117(2) dead fsync/errno imports (bug-71 residual)

**`src/target/linux_gtk/`**

- [x] `src/target/linux_gtk/app_io.rs` (542 loc) — bug-111 term setters missing inactive gate
- [x] `src/target/linux_gtk/bootstrap.rs` (778 loc) — bug-110 exit-code mask, bug-114 keyDown pipe freeze
- [x] `src/target/linux_gtk/mod.rs` (796 loc) — clean
- [x] `src/target/linux_gtk/term_draw.rs` (653 loc) — bug-117(3) grid race + stale doc

**`src/target/linux_riscv64/`**

- [x] `src/target/linux_riscv64/code.rs` (748 loc) — bug-117(1) armed-but-dead GTK hooks
- [x] `src/target/linux_riscv64/mod.rs` (417 loc) — clean
- [x] `src/target/linux_riscv64/plan.rs` (402 loc) — clean

**`src/target/linux_x86_64/`**

- [x] `src/target/linux_x86_64/code.rs` (819 loc) — bug-109 (HIGH, net.write raw-syscall stale errno)
- [x] `src/target/linux_x86_64/mod.rs` (416 loc) — clean
- [x] `src/target/linux_x86_64/plan.rs` (398 loc) — bug-109 (dead libc write import aspect)

**`src/target/macos_aarch64/`**

- [x] `src/target/macos_aarch64/code.rs` (793 loc) — clean (Darwin constants/offsets verified)
- [x] `src/target/macos_aarch64/mod.rs` (381 loc) — clean
- [x] `src/target/macos_aarch64/plan.rs` (637 loc) — clean
- [x] `src/target/macos_aarch64/tls.rs` (221 loc) — bug-116 configure-block leak

**`src/target/macos_aarch64/app/`**

- [x] `src/target/macos_aarch64/app/app_io.rs` (1110 loc) — clean (stale clobber comment noted in bug-117(5))
- [x] `src/target/macos_aarch64/app/bootstrap.rs` (919 loc) — bug-112 autorelease-pool leak, bug-117(4) headless busy-spin
- [x] `src/target/macos_aarch64/app/mod.rs` (770 loc) — clean
- [x] `src/target/macos_aarch64/app/term_view.rs` (1285 loc) — bug-114 keyDown pipe freeze

**`src/target/package_mfp/`**

- [x] `src/target/package_mfp/mod.rs` (499 loc) — clean

**`src/target/shared/`**

- [x] `src/target/shared/abi.rs` (1153 loc) — clean
- [x] `src/target/shared/lower.rs` (19 loc) — clean
- [x] `src/target/shared/mod.rs` (14 loc) — clean
- [x] `src/target/shared/regmodel.rs` (99 loc) — bug-125(2) allocatable-pool doc drift
- [x] `src/target/shared/validate.rs` (1703 loc) — clean (used-helper scan; see bug-118/119 asymmetry on the usage.rs side)

**`src/target/shared/code/`**

- [x] `src/target/shared/code/builder_arena_transfer.rs` (851 loc) — bug-141 resource-union return, bug-146 transfer capacity walk
- [x] `src/target/shared/code/builder_bits.rs` (293 loc) — clean
- [x] `src/target/shared/code/builder_codegen_primitives.rs` (1978 loc) — bug-141 return-exit resource-union cleanup, bug-147(6) x86 owned-temp wild free
- [x] `src/target/shared/code/builder_collection_compare.rs` (469 loc) — bug-147(1) float-eq nesting, bug-147(2) dead union arms
- [x] `src/target/shared/code/builder_collection_layout.rs` (1816 loc) — bug-141 union size, bug-147(4) unaligned list payload
- [x] `src/target/shared/code/builder_collection_mutate.rs` (4227 loc) — bug-145 map-set leak, bug-147(5) leak batch, bug-147(7) unchecked size arith
- [x] `src/target/shared/code/builder_collection_queries.rs` (1392 loc) — bug-147(5) leak batch
- [x] `src/target/shared/code/builder_collection_query.rs` (625 loc) — clean
- [x] `src/target/shared/code/builder_control.rs` (1395 loc) — bug-147(5) per-iteration String leak
- [x] `src/target/shared/code/builder_conversions.rs` (1020 loc) — bug-144 base-10 toInt overflow
- [x] `src/target/shared/code/builder_emit_helpers.rs` (499 loc) — bug-147(5) thread-send leak
- [x] `src/target/shared/code/builder_fixed_math.rs` (941 loc) — bug-128 atan2, bug-137(1) host-libm consts, bug-137(3) pow neg-exp code
- [x] `src/target/shared/code/builder_fs_paths.rs` (655 loc) — bug-132 pathNormalize root pop
- [x] `src/target/shared/code/builder_inplace_assign.rs` (560 loc) — bug-142 (HIGH) FOR EACH append UAF, bug-143 (HIGH) self-append chain; regrow leak = dup bug-77
- [x] `src/target/shared/code/builder_math.rs` (1252 loc) — bug-137(4) rand modulo bias
- [x] `src/target/shared/code/builder_numeric.rs` (1807 loc) — bug-74/75 root-cause refinement (Fixed/Int pow aliasing), bug-135 Float^ hang, bug-137(2) XOR bump
- [x] `src/target/shared/code/builder_pow.rs` (791 loc) — bug-129 (HIGH) pow subnormal, bug-137(5) pow(-0.0)
- [x] `src/target/shared/code/builder_search.rs` (1106 loc) — bug-133 (HIGH) find multibyte start
- [x] `src/target/shared/code/builder_simd_fixed_math.rs` (331 loc) — clean
- [x] `src/target/shared/code/builder_simd_float_math.rs` (1400 loc) — bug-130 exp range, bug-131 atan2(0,0), bug-134 log subnormal, bug-138(1) dead Pow kernel
- [x] `src/target/shared/code/builder_simd_math.rs` (828 loc) — clean
- [x] `src/target/shared/code/builder_strings.rs` (1450 loc) — clean
- [x] `src/target/shared/code/builder_strings_builtins.rs` (2755 loc) — clean
- [x] `src/target/shared/code/builder_strings_package.rs` (450 loc) — clean
- [x] `src/target/shared/code/builder_value_semantics.rs` (810 loc) — bug-140 (HIGH) string MATCH pointer compare, bug-147(3) union-field HashMap order
- [x] `src/target/shared/code/builder_values.rs` (1733 loc) — bug-147(2) dead union constructor arm; bug-99 Capture lowering (verified)
- [x] `src/target/shared/code/builder_vector_inline.rs` (364 loc) — bug-138(2) stale comment
- [x] `src/target/shared/code/code_impl.rs` (329 loc) — clean
- [x] `src/target/shared/code/codegen_utils.rs` (698 loc) — clean (str_d callee-save context of bug-124(2))
- [x] `src/target/shared/code/crypto.rs` (235 loc) — bug-138(2) dead self-move
- [x] `src/target/shared/code/crypto_ec.rs` (278 loc) — bug-136 shared marshalling zeroization
- [x] `src/target/shared/code/data_objects.rs` (1265 loc) — clean
- [x] `src/target/shared/code/datetime.rs` (167 loc) — clean
- [x] `src/target/shared/code/entry_and_arena.rs` (2133 loc) — bug-102(2) hardcoded _main reloc (latent)
- [x] `src/target/shared/code/error_constants.rs` (461 loc) — clean
- [x] `src/target/shared/code/float_format.rs` (602 loc) — clean
- [x] `src/target/shared/code/fma_fusion.rs` (292 loc) — bug-137(6) fusion label-blind (latent)
- [x] `src/target/shared/code/fs_helpers.rs` (152 loc) — clean
- [x] `src/target/shared/code/fs_helpers_atomic.rs` (1662 loc) — bug-101 readText fd leak, bug-102(1) temp O_CLOEXEC
- [x] `src/target/shared/code/fs_helpers_io.rs` (2249 loc) — clean
- [x] `src/target/shared/code/fs_helpers_paths.rs` (1943 loc) — clean
- [x] `src/target/shared/code/function_lowering.rs` (935 loc) — clean
- [x] `src/target/shared/code/io_helpers.rs` (1941 loc) — bug-95 readLine buffer leak, bug-97 drain-retry + EINTR cluster
- [x] `src/target/shared/code/link_thunk.rs` (1066 loc) — clean
- [x] `src/target/shared/code/mir.rs` (1665 loc) — clean
- [x] `src/target/shared/code/mod.rs` (3190 loc) — clean
- [x] `src/target/shared/code/module_analysis.rs` (947 loc) — clean
- [x] `src/target/shared/code/os.rs` (1507 loc) — clean (env-lock balance verified)
- [x] `src/target/shared/code/peephole.rs` (449 loc) — clean
- [x] `src/target/shared/code/runtime_helpers.rs` (881 loc) — bug-102(4) dead arena-state store
- [x] `src/target/shared/code/runtime_helpers_thread.rs` (1304 loc) — clean
- [x] `src/target/shared/code/serialization_utils.rs` (17 loc) — clean
- [x] `src/target/shared/code/simd_kernel_coeffs.rs` (101 loc) — clean
- [x] `src/target/shared/code/term.rs` (952 loc) — clean (register lifetimes + termios slot layouts verified)
- [x] `src/target/shared/code/test_support.rs` (101 loc) — clean
- [x] `src/target/shared/code/type_utils.rs` (304 loc) — clean
- [x] `src/target/shared/code/types.rs` (567 loc) — clean
- [x] `src/target/shared/code/validation.rs` (556 loc) — clean

**`src/target/shared/code/crypto_ec/`**

- [x] `src/target/shared/code/crypto_ec/macos.rs` (1441 loc) — clean
- [x] `src/target/shared/code/crypto_ec/openssl.rs` (1712 loc) — bug-136 zeroization + verify checks + SEC1 splice

**`src/target/shared/code/net/`**

- [x] `src/target/shared/code/net/io.rs` (1661 loc) — bug-109 (HIGH) net.write errno, bug-113 bind-all, bug-115 EINTR; accept-timeout = dup audit OS-02
- [x] `src/target/shared/code/net/mod.rs` (792 loc) — bug-113 bind-all getaddrinfo, bug-115 connect-poll EINTR
- [x] `src/target/shared/code/net/poll.rs` (220 loc) — bug-115 poll EINTR

**`src/target/shared/code/private/`**

- [x] `src/target/shared/code/private/mod.rs` (1 loc) — clean
- [x] `src/target/shared/code/private/unicode.rs` (983 loc) — clean (audit-unicode hardening confirmed)

**`src/target/shared/code/regalloc/`**

- [x] `src/target/shared/code/regalloc/analysis.rs` (634 loc) — bug-127(3) %scratch token cross-ISA index
- [x] `src/target/shared/code/regalloc/linear_scan.rs` (372 loc) — bug-127(2) eviction panic invariant
- [x] `src/target/shared/code/regalloc/mod.rs` (310 loc) — clean

**`src/target/shared/code/tls/`**

- [x] `src/target/shared/code/tls/macos.rs` (3756 loc) — bug-102(3) int sign-extension (latent)
- [x] `src/target/shared/code/tls/mod.rs` (397 loc) — clean
- [x] `src/target/shared/code/tls/openssl.rs` (2305 loc) — bug-102(3) int sign-extension (latent)

**`src/target/shared/nir/`**

- [x] `src/target/shared/nir/json.rs` (895 loc) — bug-139(4) dumps omit LINK/provenance
- [x] `src/target/shared/nir/lower.rs` (528 loc) — bug-139(5) CallResult default-arg gap
- [x] `src/target/shared/nir/mod.rs` (346 loc) — bug-139(6) link-thunk symbol collision
- [x] `src/target/shared/nir/symbols.rs` (33 loc) — clean

**`src/target/shared/plan/`**

- [x] `src/target/shared/plan/function_builder.rs` (641 loc) — bug-139(1) dead fold, bug-139(3) dedup drops literals
- [x] `src/target/shared/plan/json.rs` (180 loc) — clean
- [x] `src/target/shared/plan/lower.rs` (208 loc) — bug-139(2) dead CallKind::Import
- [x] `src/target/shared/plan/mod.rs` (495 loc) — bug-139(4) dumps omit link_symbols
- [x] `src/target/shared/plan/symbols.rs` (754 loc) — bug-119 (third `result` heuristic site), bug-139(1) dead fold

**`src/target/shared/runtime/`**

- [x] `src/target/shared/runtime/catalog.rs` (169 loc) — clean (carries bug-120(1) dead strings entries)
- [x] `src/target/shared/runtime/crypto_specs.rs` (153 loc) — clean
- [x] `src/target/shared/runtime/datetime_specs.rs` (48 loc) — clean
- [x] `src/target/shared/runtime/fs_specs.rs` (495 loc) — clean
- [x] `src/target/shared/runtime/io_specs.rs` (212 loc) — bug-120(2) understated clobbers (representative)
- [x] `src/target/shared/runtime/mod.rs` (133 loc) — clean
- [x] `src/target/shared/runtime/net_specs.rs` (627 loc) — clean
- [x] `src/target/shared/runtime/os_specs.rs` (231 loc) — clean
- [x] `src/target/shared/runtime/strings_specs.rs` (189 loc) — bug-120(1) entirely dead
- [x] `src/target/shared/runtime/term_specs.rs` (216 loc) — clean
- [x] `src/target/shared/runtime/thread_specs.rs` (284 loc) — clean
- [x] `src/target/shared/runtime/usage.rs` (293 loc) — bug-118 MATCH-guard gap, bug-119 `result` false Thread helper

**`src/testing/`**

- [x] `src/testing/desugar.rs` (1109 loc) — bug-93(2) inline-TRAP coverage gap
