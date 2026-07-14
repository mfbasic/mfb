# goal-01: Full compiler source review — file-by-file bug hunt

Last updated: 2026-07-09
Status: COMPLETE (263 / 263 files reviewed)

## Objective

Read **every production source file in the compiler** (`src/**`), one file at a
time, and hunt for defects of any kind:

- **Correctness bugs** — wrong results, wrong control flow, off-by-one, incorrect
  edge-case handling, missed error paths, platform-divergent behavior.
- **Memory-safety hazards** — unchecked size arithmetic (`a*b`, `a+b` before an
  allocation), OOB reads/writes, use-after-free / double-free, aliasing, register
  clobbers, missing frees / leaks.
- **Security issues** — trust-boundary gaps (untrusted `.mfp` decode, network/FS
  input), missing bounds/depth/rate limits, unsafe file permissions, TOCTOU,
  injection, weak crypto usage, information leaks.
- **Footguns** — APIs or invariants that are easy to misuse, silent-truncation or
  silent-wrong-value paths, non-obvious ordering/lifetime requirements, panics on
  attacker- or user-reachable input, `unwrap`/`expect`/`todo!`/`unimplemented!` on
  reachable paths, integer casts that narrow.
- **Dead code** — unreachable branches, unused helpers/fields/variants, stale
  feature flags, commented-out code, duplicated logic that should be unified.
- **Anything else worth fixing** — misleading names, incorrect comments/docs vs.
  behavior, TODO/FIXME/HACK markers that flag real gaps.

For **each item found**, create a `bug-NN-shortname.md` document in `planning/`
using the project bug template, then continue the review. The deliverable of this
goal is the review coverage (every file checked off below) **plus** one bug
document per real finding.

## Scope

263 production `.rs` files, ~184.5k LOC. **Excluded** (not part of this review):

- Per-module test files (`**/tests.rs`, `testutil.rs`, `testing.rs`) — 16 files.
  Test code is out of scope unless a review of production code reveals the test is
  masking or failing to guard a real bug (note it in that bug doc).
- Generated tables (`unicode_runtime_tables.rs`).

The full checklist is in [§ File census & progress](#file-census--progress) below.
Runtime helpers, encoders, the linker, and package decode are all in scope — they
are compiler source (they emit or process native code / binary formats).

## Prior work — do NOT re-file known findings

A previous security-focused pass exists; cross-check every candidate finding
against it before writing a new bug doc:

- `planning/audit-1-summary.md` (+ its sub-files `audit-1-*.md`) — CRITICAL→LOW
  findings PKG-/MEM-/FE-/OS-/LNK-/REPO-. Several are already FIXED (e.g. `.mfp`
  decode hardening PKG-01..07, per `planning/old-plans/audit-1-package-decode.md`).
- `planning/security-review-1.md` — earlier spec-only review.
- `planning/old-plans/bug-0*.md` and the memory index (`MEMORY.md`) — bug-01..08
  are landed fixes; do not re-report them.

If a file re-surfaces a *known-and-still-open* audit finding, reference that
finding's ID in the new bug doc rather than duplicating the analysis. If it's a
*genuinely new* issue, file it fresh.

## What counts as a finding (and what doesn't)

- **File a bug doc** for anything that is a real defect a maintainer would want
  fixed: wrong behavior, a safety/security hazard, a reachable panic, a leak, or
  dead/duplicated code of non-trivial size.
- **Batch trivial findings.** Many tiny same-class nits (a cluster of dead
  `pub(crate)` helpers in one module, a handful of stale comments) can share one
  bug doc scoped to that module rather than one doc each — but keep distinct
  root causes in distinct docs.
- **Do not file** style preferences, subjective naming, or speculative
  "could-refactor" items with no correctness/safety/clarity payoff.
- **Verify before filing.** Each finding must cite `file:line` (or `file:symbol`)
  and state the concrete failure scenario (inputs/state → wrong output/crash). If
  you cannot construct a plausible trigger, note it as defense-in-depth / latent
  and rank it LOW — do not inflate severity.

## Workflow

Per the AGENTS.md "finish the task" rule, this runs to completion — review every
file, not a representative sample.

1. **Pick the next unchecked file** from the census (top to bottom; a whole
   directory group at a time keeps related invariants in context).
2. **Read the file** (and enough of its callers/callees to judge reachability).
   For codegen/runtime files, consult `.ai/compiler.md` for the register-lifetime
   and validation conventions before judging a "clobber" or "leak".
3. **Record findings.** For each real defect, create `planning/bug-NN-*.md` from
   the template (`~/.claude/skills/write-bug/template.md` or the `write-bug`
   skill). Number sequentially from **bug-09** (bug-01..08 are taken). Note the
   bug id(s) next to the file's checkbox.
4. **Check the box** for that file (`- [ ]` → `- [x]`) and add a one-word verdict:
   `clean`, or the bug ids filed (e.g. `bug-11, bug-12`).
5. **Update the counter** in the Status line at the top and the tallies in
   [§ Findings ledger](#findings-ledger).
6. Repeat until every box is checked.

Batch commits by directory group (e.g. "review src/arch/** — file bug-09"), never
mixing the review bookkeeping with unrelated changes.

Do **not** fix bugs as part of this goal unless a fix is trivial-and-obvious and
the user has asked for fixes — this goal's job is to *find and document*. Each
`bug-NN` doc carries its own fix plan and is landed separately.

## Findings ledger

Update as bugs are filed. (Severity per the bug doc's own effort/impact call.)

| Bug | File(s) | Class | Severity | Status |
|-----|---------|-------|----------|--------|
| bug-09 | `arch/aarch64/encode/sizing.rs`, `emitter.rs` | correctness (size pre-pass vs emitter divergence) | LOW (latent; HIGH if triggered) | FIXED |
| bug-10 | `arch/aarch64/regmodel.rs` | doc (trait doc says spill slots are 8 bytes; impl returns 16) | LOW (latent footgun) | FIXED |
| bug-11 | `numeric.rs` | memory-safety (Fixed-literal large-exponent expansion → multi-GB string / i32 overflow) | MEDIUM | FIXED |
| bug-12 | `lexer.rs` | correctness (`\`-newline in string embeds newline + desyncs line counter) | LOW | FIXED |
| bug-13 | `escape.rs` | dead-code (`returned` set accumulated then discarded) | LOW | FIXED |
| bug-14 | `arch/riscv64/encode/emitter.rs` | correctness/mem-safety (out-of-range load materializes addr into `rd`; corrupts `base` when `rd==base`) | MEDIUM | FIXED |
| bug-15 | `arch/x86_64/select.rs` | correctness (duplicate `{target}__x86ford` skip label → NaN-only wrong control flow) | MEDIUM | FIXED |
| bug-16 | `arch/riscv64/encode/operand.rs`, `arch/x86_64/encode/emitter.rs` | correctness (64-bit-lane vector shift by 64: rv64 encode error, x86 silent 0 not sign-fill) | LOW | FIXED |
| bug-17 | `arch/x86_64/encode/emitter.rs` | mem-safety (f2i_nearest clobbers rax vs siblings that preserve it) | LOW (latent) | FIXED |
| bug-18 | `arch/x86_64/encode/data.rs` | footgun (`align()` div-by-zero on align 0) | LOW (latent) | FIXED |
| bug-19 | `ast/items.rs` | correctness (DOC EXAMPLE `dedent` non-char-boundary slice panic) | MEDIUM | FIXED |
| bug-20 | `binary_repr/reader.rs` | mem-safety (untrusted type id 0/9 underflows `serialize_type` → debug decoder panic) | LOW | FIXED |
| bug-21 | `binary_repr/reader.rs` | security (Type/Union/Enum ABI export sigHashes unverified at decode) | LOW (def-in-depth) | FIXED |
| bug-22 | `audit/collect/source.rs`, `findings.rs`, `mod.rs` | security/dead-code (audit under-reports net/process/env/clock/randomness/native capabilities) | MEDIUM | FIXED |
| bug-23 | `audit/collect/source.rs` | correctness (audit fallibility analysis collapses overloads by bare name) | MEDIUM | FIXED |
| bug-24 | `audit/text.rs` | security (untrusted package names printed raw → terminal ANSI/newline spoofing) | LOW | FIXED |
| bug-25 | `audit/collect/lockfile.rs`, `dependencies.rs` | footgun/security (lossy `lockfileVersion` cast + unbounded `.mfp` read for hashing) | LOW | FIXED |
| bug-26 | `builtins/general.rs` | correctness (`function_parts` mis-parses nested multi-arg FUNC type) | LOW (latent) | FIXED |
| bug-27 | `cli/pkg.rs`, `cli/resolve.rs`, `manifest/package.rs` | security (untrusted name/hash → path traversal + symlink-follow write BEFORE verification) | HIGH | FIXED |
| bug-28 | `builtins/net.rs` | correctness (`connectTcp` named-arg `timeoutMs` alias collides with `port`) | MEDIUM | FIXED |
| bug-29 | `cli/init.rs` | security (`write_new_file` check-then-write TOCTOU + symlink follow) | LOW | FIXED |
| bug-30 | `cli/pkg.rs`, `cli/resolve.rs` | dead-code/footgun (dead `pin` branch + lossy `compare_versions` parse) | LOW | FIXED |
| bug-31 | `ir/verify/mod.rs` | mem-safety (verify trusts computed-node self-reported types → type confusion on hostile IR) | HIGH | FIXED |
| bug-32 | `ir/verify/mod.rs` | mem-safety (ambiguous closure shape disables capture-index bounds check → OOB) | HIGH | FIXED |
| bug-33 | `ir/binary.rs` | correctness/footgun (`verify_package` skips For/DoUntil bodies + capture-index u32 truncation) | LOW | FIXED |
| bug-34 | `ir/lower.rs` | footgun (LINK CONST bit-63-set 64-bit value silently lowered to 0) | LOW | FIXED |
| bug-35 | `monomorph/helpers.rs` | correctness (type-string comma split not paren-depth-aware; same class as bug-26) | LOW | FIXED |
| bug-36 | `monomorph/lower.rs` | footgun (imported-overload binds untyped `[]` to first match, no ambiguity error) | LOW | FIXED |
| bug-37 | `manifest/package.rs` | footgun (`.mfp` binaryReprLength u64→usize truncation on 32-bit) | LOW (latent) | FIXED |
| bug-38 | `os/linux/link/elf.rs` | correctness (static aarch64/riscv ELF data unaligned vs page-aligned reloc `data_vmaddr`) | MEDIUM (latent) | FIXED |
| bug-39 | `os/linux/link/elf.rs`, `mod.rs` | correctness (DT_HASH chain off-by-one + riscv auipc hi20 silent truncation, LNK-06 class) | LOW (latent) | FIXED |
| bug-40 | `resolver/packages.rs`, `rules/table.rs` | correctness/dead-code (corrupt `.mfp` → `0-000-0000 UNKNOWN_RULE`; dead `IMPORT_MISSING_PACKAGE`) | MEDIUM | FIXED |
| bug-41 | `syntaxcheck/types.rs` | correctness/footgun (bare-name User unify + `Map TO` split + Byte-literal radix RECOVER) | LOW | FIXED |
| bug-42 | `target/shared/code/datetime.rs` | security/correctness (`localOffset` ignores `localtime_r` NULL → uninitialized-stack read, ASLR info-leak) | MEDIUM | FIXED |
| bug-43 | `syntaxcheck/resources.rs` | dead-code (two collection-ownership checks are empty no-op stubs whose docs claim enforcement) | LOW | FIXED |
| bug-44 | `target/{linux_x86_64,macos_aarch64,…}/code.rs`, `fs_helpers_atomic.rs` | correctness (bug-04 `int`-return width unported: `fsync` on x86/macOS, `close` on all 4 → atomic-write durability failures swallowed) | MEDIUM | FIXED |
| bug-45 | `target/shared/validate.rs` | correctness (`collect_bind_types` skips FOR EACH → resource-union bind in a FOR EACH fails build "unused runtime helper") | MEDIUM | FIXED |
| bug-46 | `target/macos_aarch64/app/term_view.rs` | correctness (line-echo Backspace falls through to raw path → injects DEL/BS into input pipe) | MEDIUM | FIXED |
| bug-47 | `target/shared/code/builder_collection_mutate.rs`, `builder_control.rs` | mem-safety (in-place prepend/set(list)/set(map) grow paths + StoreGlobal reassign leak the abandoned buffer; bug-01 class, new sites) | MEDIUM | FIXED |
| bug-48 | `target/shared/code/fs_helpers_paths.rs` | mem-safety (`listDirectory` sizes from one dir scan, fills from another with no bound → TOCTOU heap overflow on concurrent writer) | HIGH | FIXED |
| bug-49 | `target/shared/code/builder_conversions.rs` | correctness (`toInt(text,base)` signed compare on unsigned cutoff → power-of-2 negative overflow wraps to 0, no ErrOverflow) | MEDIUM | FIXED |
| bug-50 | `target/linux_gtk/bootstrap.rs` | mem-safety (GTK line input overruns fixed 1024-byte buffer into GtkDrawingArea*/term grid — no LINE_BUF_CAP bound) | MEDIUM | FIXED |
| bug-51 | `target/shared/code/fs_helpers_io.rs`, `io_helpers.rs` | correctness (single-`write()` output paths treat short positive count as complete → silent truncation; fs buffered + default io::print) | MEDIUM | FIXED |
| bug-52 | `target/shared/code/tls/macos.rs` | mem-safety/security (macOS `tls::readText` encoding-error exit skips release block → remote peer drives unbounded dispatch_data/content leak) | MEDIUM | FIXED |
| bug-53 | `target/macos_aarch64/app/{bootstrap,app_io}.rs` | mem-safety (every app-mode `io::print` leaks an owned NSAttributedString + NSString — no release anywhere) | MEDIUM | FIXED |
| bug-54 | `target/shared/code/regalloc/linear_scan.rs` | mem-safety (spill-reload "genuinely free" scratch can be a callee-saved reg not added to the frame save-set → clobbers caller's x20/d8) | MEDIUM (latent) | FIXED |
| bug-55 | `target/shared/code/tls/*`, `crypto_ec/*`, `net/io.rs` | security/mem-safety (TLS/crypto error paths leak SSL/EVP/CF/fd/addrinfo; min-proto + EVP_PKEY_assign returns unchecked; EC key scratch un-zeroed) | LOW | FIXED |
| bug-56 | `target/shared/code/link_thunk.rs` | mem-safety (`emit_link_expr` unbounded fixed-register scheme escalates into x19/arena_base for nested SUCCESS_ON/RESULT → program-wide corruption) | HIGH | FIXED |
| bug-57 | `target/shared/code/builder_control.rs` | correctness (WHILE condition folds a loop-mutated local's stale entry constant → wrong result / infinite loop) | MEDIUM | FIXED |
| bug-58 | `target/package_mfp/mod.rs` | security (`.mfp` output path built from unsanitized `metadata.name` → path traversal; bug-27 class, dev-sourced) | LOW | FIXED |
| bug-59 | `target/linux_gtk/{bootstrap,mod}.rs` | footgun/dead-code (pipe read-fd leaked after dup2 + fuzzy stdin EOF; dead `getenv` import) | LOW | FIXED |
| bug-60 | `target/shared/code/{builder_strings,builder_strings_builtins,runtime_helpers}.rs` | mem-safety (unchecked size-arith before alloc: replace/join/list-replace + thread-queue capacity; def-in-depth) | LOW | FIXED |
| bug-61 | `target/shared/code/builder_numeric.rs`, `builder_fixed_math.rs` | correctness/footgun (Fixed÷ of exact −2^31 wrongly traps ErrOverflow; integer/Fixed `^` linear loop hangs for \|base\|≤1 huge exponent) | LOW | FIXED |
| bug-62 | `target/shared/code/{fs_helpers_io,io_helpers}.rs` | correctness (EINTR = hard error no-retry; `write()`-0 drain spin; reconcile ignores failed rewind lseek) | LOW | FIXED |
| bug-63 | `target/shared/code/{fs_helpers_io,fs_helpers_atomic}.rs` | mem-safety (fd leak on record-alloc OOM; atomic-write temp not unlinked on failure; failed-close double-close) | LOW | FIXED |
| bug-64 | `target/shared/code/os.rs` | mem-safety (`os::getEnv`/`environ`/`userName` race concurrent `os::setEnv` → use-after-free; OS-08 class) | LOW | FIXED |
| bug-65 | `target/shared/code/builder_fs_paths.rs` | correctness/mem-safety (`pathBaseName("//")`→`""` not `/`; `pathNormalize("")` NUL write 1B past alloc, masked by arena rounding) | LOW | FIXED |
| bug-66 | `target/shared/code/link_thunk.rs` | correctness/footgun (LINK AND/OR bitwise on non-normalized operands; CInt32 const-pin skips param range check → silent truncation) | LOW | FIXED |
| bug-67 | `target/shared/code/module_analysis.rs` | correctness (`op_requires_empty_string_constant` skips FOR/DO UNTIL → uninit String in those loops fails build with dangling `_mfb_str_empty`) | HIGH | FIXED |
| bug-68 | `target/shared/code/{builder_simd_math,builder_simd_float_math}.rs` | correctness/dead-code (float array min/max/clamp odd-tail sign-of-zero divergence; NaN-only reduce latent Inf drop; stale commented code) | LOW | FIXED |
| bug-69 | `target/shared/code/{fma_fusion,types,peephole}.rs`, `regalloc/analysis.rs` | latent codegen (FMA product-redef, union-tag name-keying, peephole sp-overlap, `stream_is_riscv` field sniff — safe only by external invariant) | LOW | FIXED |
| bug-70 | `target/shared/{code,nir,plan,runtime}/*`, `entry_and_arena.rs` | footgun/dead-code (nir/json `unreachable!` + `-regalloc bump` `.expect` panics; plan-artifact call over-report; unicode dead helpers; stderr sign; exit>255) | LOW | FIXED |
| bug-71 | `target/{linux_x86_64,linux_riscv64,macos_aarch64}/{plan,code,mod}.rs` | dead-code/doc (stale/dead imports: x86 `_exit`/`getentropy`, `io.flush` fsync+errno all 3; riscv AAPCS64 comment + musl-flavor dumps) | LOW | FIXED |
| bug-72 | `target/shared/plan/*`, `os/{macos,linux}/object.rs` | correctness (every call became a relocation, so an `Indirect` call's "symbol" was a local's name → calling a function value never linked) | HIGH | FIXED |
| bug-73 | `target/shared/code/builder_collection_layout.rs` | contract (a `List OF FUNC(...)` type-checked then died in the backend; now supported, with reference semantics) | MEDIUM | FIXED |

**Status: every finding of this review (bug-09 .. bug-73) is fixed and committed.**
bug-09..19 in `879c0cb5..64080b6f`, bug-20..39 in `ba0d9d44..dfffc310`,
bug-40..73 in `1d8517c6..41578ef3`.

### Follow-up findings raised while fixing bug-40..73

Each was outside the fixing bug's blast radius and was deliberately left alone.

| bug | file(s) | class | sev | status |
| --- | --- | --- | --- | --- |
| bug-74 | `code/builder_numeric.rs` | correctness (`Fixed ^ Fixed` returns `0.00` for any `\|base\| >= 2` — `base` is clobbered across `emit_fixed_multiply`) | HIGH | OPEN |
| bug-75 | `code/builder_numeric.rs` | correctness (integer `^` with a negative base of magnitude >= 2 wrongly traps `ErrOverflow`) | MEDIUM | OPEN |
| bug-76 | `code/builder_math.rs` | correctness (`math::clamp(List OF Float, …)` spills a `d`-register bound with `store_u64` → broadcasts garbage for any non-literal bound) | MEDIUM | OPEN |
| bug-77 | `code/builder_inplace_assign.rs` | mem-safety (string self-append regrow leaks the old buffer; bug-01/47 class, entangled with bug-06's static-vs-arena carrier) | MEDIUM | OPEN |
| bug-78 | closure lowering | mem-safety (every function-value evaluation arena-allocs a never-freed 16-byte descriptor; a no-capture lambda should allocate nothing) | MEDIUM | OPEN |
| bug-79 | cluster | LOW cluster (macOS TLS SNI options leak; duplicate LINK thunk labels; `pathNormalize("a/..")`; x86 dead `write` import; `pick()(4)` unparseable) | LOW | OPEN |
| bug-80 | `code/validation.rs` + union tag assignment | correctness (a variant included at divergent positions in two unions is now *rejected* by bug-69; canonical global tags would let it compile, and would also fix the resource-union drop-dispatch nondeterminism) | MEDIUM | OPEN |

Tallies: CRITICAL 0 · HIGH 6 · MEDIUM 22 · LOW 41 · (dead-code components in bug-22, bug-30, bug-40, bug-43, bug-59, bug-68, bug-70, bug-71).
HIGH: bug-27, bug-31, bug-32 (earlier session) + bug-48, bug-56, bug-67 (this session).

New HIGH findings from the target/** review: **bug-48** (`listDirectory` TOCTOU heap
overflow), **bug-56** (LINK expr register escalation into arena_base), **bug-67** (uninit
String in FOR/DO UNTIL fails to compile). bug-45/49/57/67 and bug-42/61/65 were reproduced
end-to-end against `target/debug/mfb`. A systemic sub-pattern surfaced: **incomplete
loop-body traversal** — several independent NIR/AST walkers omit `FOR`/`DO UNTIL`/`FOR EACH`
arms (bug-45, bug-67); bug-67's blast radius calls for an audit of every such traversal.

## File census & progress

Reviewed top-to-bottom. Mark `- [x]` with a verdict when done. Grouped by
directory; LOC shown to help sequence the effort.

**`src/`**

- [x] `src/coverage.rs` (274 loc) — clean
- [x] `src/doc.rs` (1099 loc) — clean
- [x] `src/escape.rs` (567 loc) — bug-13
- [x] `src/fmt.rs` (947 loc) — clean
- [x] `src/internal_name.rs` (149 loc) — clean
- [x] `src/lexer.rs` (1473 loc) — bug-12
- [x] `src/main.rs` (833 loc) — clean
- [x] `src/numeric.rs` (303 loc) — bug-11
- [x] `src/scope_privates.rs` (494 loc) — clean
- [x] `src/target.rs` (294 loc) — clean
- [x] `src/unicode_backend.rs` (66 loc) — clean

**`src/arch/`**

- [x] `src/arch/mod.rs` (3 loc) — clean

**`src/arch/aarch64/`**

- [x] `src/arch/aarch64/abi.rs` (1051 loc) — clean
- [x] `src/arch/aarch64/backend.rs` (32 loc) — clean
- [x] `src/arch/aarch64/mod.rs` (7 loc) — clean
- [x] `src/arch/aarch64/ops.rs` (714 loc) — clean
- [x] `src/arch/aarch64/regmodel.rs` (278 loc) — bug-10
- [x] `src/arch/aarch64/reloc.rs` (44 loc) — clean
- [x] `src/arch/aarch64/select.rs` (94 loc) — clean

**`src/arch/aarch64/encode/`**

- [x] `src/arch/aarch64/encode/data.rs` (52 loc) — clean
- [x] `src/arch/aarch64/encode/emitter.rs` (1175 loc) — bug-09
- [x] `src/arch/aarch64/encode/mod.rs` (163 loc) — clean
- [x] `src/arch/aarch64/encode/operand.rs` (104 loc) — clean
- [x] `src/arch/aarch64/encode/sizing.rs` (139 loc) — bug-09

**`src/arch/riscv64/`**

- [x] `src/arch/riscv64/backend.rs` (55 loc) — clean
- [x] `src/arch/riscv64/mod.rs` (21 loc) — clean
- [x] `src/arch/riscv64/regmodel.rs` (215 loc) — clean
- [x] `src/arch/riscv64/reloc.rs` (48 loc) — clean
- [x] `src/arch/riscv64/select.rs` (714 loc) — clean
- [x] `src/arch/riscv64/v128.rs` (601 loc) — clean (bug-16 fix also touches here)

**`src/arch/riscv64/encode/`**

- [x] `src/arch/riscv64/encode/data.rs` (52 loc) — clean
- [x] `src/arch/riscv64/encode/emitter.rs` (685 loc) — bug-14
- [x] `src/arch/riscv64/encode/mod.rs` (134 loc) — clean
- [x] `src/arch/riscv64/encode/operand.rs` (114 loc) — bug-16
- [x] `src/arch/riscv64/encode/sizing.rs` (190 loc) — clean (no bug-09-class divergence)

**`src/arch/x86_64/`**

- [x] `src/arch/x86_64/backend.rs` (57 loc) — clean
- [x] `src/arch/x86_64/mod.rs` (18 loc) — clean
- [x] `src/arch/x86_64/regmodel.rs` (245 loc) — clean
- [x] `src/arch/x86_64/reloc.rs` (46 loc) — clean
- [x] `src/arch/x86_64/select.rs` (1011 loc) — bug-15

**`src/arch/x86_64/encode/`**

- [x] `src/arch/x86_64/encode/data.rs` (56 loc) — bug-18
- [x] `src/arch/x86_64/encode/emitter.rs` (1944 loc) — bug-16, bug-17
- [x] `src/arch/x86_64/encode/mod.rs` (142 loc) — clean
- [x] `src/arch/x86_64/encode/operand.rs` (82 loc) — clean
- [x] `src/arch/x86_64/encode/sizing.rs` (12 loc) — clean (no bug-09-class divergence)

**`src/ast/`**

- [x] `src/ast/expr.rs` (745 loc) — clean (FE-01 depth-guard known)
- [x] `src/ast/items.rs` (1322 loc) — bug-19
- [x] `src/ast/lexical.rs` (127 loc) — clean
- [x] `src/ast/manifest.rs` (535 loc) — clean
- [x] `src/ast/mod.rs` (35 loc) — clean
- [x] `src/ast/parser.rs` (288 loc) — clean
- [x] `src/ast/serialize.rs` (1644 loc) — clean
- [x] `src/ast/stmt.rs` (723 loc) — clean (FE-03 depth-guard known)
- [x] `src/ast/types.rs` (675 loc) — clean

**`src/audit/`**

- [x] `src/audit/json.rs` (552 loc) — clean
- [x] `src/audit/mod.rs` (298 loc) — clean
- [x] `src/audit/report.rs` (477 loc) — clean
- [x] `src/audit/text.rs` (334 loc) — bug-24

**`src/audit/collect/`**

- [x] `src/audit/collect/dependencies.rs` (223 loc) — bug-25
- [x] `src/audit/collect/findings.rs` (513 loc) — bug-22 (dead PERM arms)
- [x] `src/audit/collect/lockfile.rs` (127 loc) — bug-25
- [x] `src/audit/collect/mod.rs` (186 loc) — bug-22 (native_links empty)
- [x] `src/audit/collect/project.rs` (300 loc) — clean
- [x] `src/audit/collect/source.rs` (873 loc) — bug-22, bug-23

**`src/binary_repr/`**

- [x] `src/binary_repr/builder.rs` (273 loc) — clean
- [x] `src/binary_repr/mod.rs` (563 loc) — clean
- [x] `src/binary_repr/reader.rs` (1464 loc) — bug-20, bug-21 (PKG-01..07 hardening verified present)
- [x] `src/binary_repr/sections.rs` (645 loc) — clean
- [x] `src/binary_repr/util.rs` (292 loc) — clean
- [x] `src/binary_repr/writer.rs` (1074 loc) — clean

**`src/builtins/`**

- [x] `src/builtins/bits.rs` (237 loc) — clean
- [x] `src/builtins/collections.rs` (533 loc) — clean
- [x] `src/builtins/crypto.rs` (814 loc) — clean
- [x] `src/builtins/csv.rs` (190 loc) — clean
- [x] `src/builtins/datetime.rs` (773 loc) — clean
- [x] `src/builtins/encoding.rs` (582 loc) — clean
- [x] `src/builtins/errorcode.rs` (118 loc) — clean
- [x] `src/builtins/fs.rs` (697 loc) — clean
- [x] `src/builtins/general.rs` (1413 loc) — bug-26
- [x] `src/builtins/http.rs` (594 loc) — clean
- [x] `src/builtins/io.rs` (126 loc) — clean
- [x] `src/builtins/json.rs` (279 loc) — clean
- [x] `src/builtins/math.rs` (583 loc) — clean
- [x] `src/builtins/mod.rs` (649 loc) — clean
- [x] `src/builtins/net.rs` (721 loc) — bug-28
- [x] `src/builtins/os.rs` (256 loc) — clean
- [x] `src/builtins/regex.rs` (304 loc) — clean
- [x] `src/builtins/resource.rs` (285 loc) — clean
- [x] `src/builtins/strings.rs` (517 loc) — clean
- [x] `src/builtins/term.rs` (326 loc) — clean
- [x] `src/builtins/thread.rs` (732 loc) — clean
- [x] `src/builtins/tls.rs` (424 loc) — clean
- [x] `src/builtins/vector.rs` (791 loc) — clean

**`src/cli/`**

- [x] `src/cli/build.rs` (1589 loc) — clean (PKG-01 verify path present)
- [x] `src/cli/doc.rs` (237 loc) — clean
- [x] `src/cli/fmt.rs` (275 loc) — clean
- [x] `src/cli/init.rs` (269 loc) — bug-29
- [x] `src/cli/man.rs` (439 loc) — clean
- [x] `src/cli/mod.rs` (121 loc) — clean
- [x] `src/cli/pkg.rs` (1834 loc) — bug-27, bug-30
- [x] `src/cli/repo.rs` (369 loc) — clean
- [x] `src/cli/resolve.rs` (1013 loc) — bug-27, bug-30
- [x] `src/cli/spec.rs` (342 loc) — clean

**`src/docs/`**

- [x] `src/docs/mod.rs` (8 loc) — clean
- [x] `src/docs/render.rs` (957 loc) — clean (renders only trusted embedded Markdown)

**`src/docs/man/`**

- [x] `src/docs/man/mod.rs` (317 loc) — clean (panics only on malformed embedded data)

**`src/docs/spec/`**

- [x] `src/docs/spec/mod.rs` (139 loc) — clean

**`src/ir/`**

- [x] `src/ir/binary.rs` (1364 loc) — bug-33
- [x] `src/ir/json.rs` (932 loc) — clean
- [x] `src/ir/link.rs` (84 loc) — clean
- [x] `src/ir/lower.rs` (3666 loc) — bug-34
- [x] `src/ir/mod.rs` (144 loc) — clean
- [x] `src/ir/op.rs` (129 loc) — clean
- [x] `src/ir/package.rs` (321 loc) — clean
- [x] `src/ir/types.rs` (85 loc) — clean
- [x] `src/ir/value.rs` (161 loc) — clean

**`src/ir/verify/`**

- [x] `src/ir/verify/mod.rs` (4111 loc) — bug-31, bug-32 (HIGH verify-pass soundness holes)

**`src/manifest/`**

- [x] `src/manifest/entry.rs` (280 loc) — clean
- [x] `src/manifest/mod.rs` (558 loc) — clean
- [x] `src/manifest/package.rs` (1446 loc) — bug-37 (bug-27 name-traversal known)

**`src/monomorph/`**

- [x] `src/monomorph/helpers.rs` (835 loc) — bug-35
- [x] `src/monomorph/lower.rs` (2446 loc) — bug-36 (FE-02 recursion known)
- [x] `src/monomorph/mod.rs` (86 loc) — clean

**`src/os/`**

- [x] `src/os/mod.rs` (2 loc) — clean

**`src/os/linux/`**

- [x] `src/os/linux/flavor.rs` (16 loc) — clean
- [x] `src/os/linux/mod.rs` (132 loc) — clean
- [x] `src/os/linux/object.rs` (1046 loc) — clean (planOnly JSON emitter)

**`src/os/linux/link/`**

- [x] `src/os/linux/link/elf.rs` (672 loc) — bug-38, bug-39
- [x] `src/os/linux/link/mod.rs` (521 loc) — bug-39

**`src/os/macos/`**

- [x] `src/os/macos/mod.rs` (141 loc) — clean
- [x] `src/os/macos/object.rs` (1383 loc) — clean (planOnly JSON emitter)

**`src/os/macos/link/`**

- [x] `src/os/macos/link/commands.rs` (535 loc) — clean
- [x] `src/os/macos/link/macho.rs` (295 loc) — clean
- [x] `src/os/macos/link/mod.rs` (515 loc) — clean

**`src/resolver/`**

- [x] `src/resolver/mod.rs` (1040 loc) — clean
- [x] `src/resolver/packages.rs` (451 loc) — bug-40
- [x] `src/resolver/resolution.rs` (2160 loc) — clean

**`src/rules/`**

- [x] `src/rules/mod.rs` (125 loc) — clean
- [x] `src/rules/table.rs` (1227 loc) — bug-40 (dead `IMPORT_MISSING_PACKAGE`)

**`src/syntaxcheck/`**

- [x] `src/syntaxcheck/builtins.rs` (2591 loc) — clean (bug-28 connectTcp re-confirmed; datetime name-skip arity message cosmetic, not filed)
- [x] `src/syntaxcheck/checking.rs` (1437 loc) — clean (FE-05 known)
- [x] `src/syntaxcheck/helpers.rs` (910 loc) — clean
- [x] `src/syntaxcheck/inference.rs` (2266 loc) — clean (permissive infer-to-Unknown by design)
- [x] `src/syntaxcheck/mod.rs` (2774 loc) — clean
- [x] `src/syntaxcheck/resources.rs` (808 loc) — bug-43
- [x] `src/syntaxcheck/types.rs` (612 loc) — bug-41

**`src/target/linux_aarch64/`**

- [x] `src/target/linux_aarch64/code.rs` (766 loc) — bug-44 (close int-return width, all backends)
- [x] `src/target/linux_aarch64/mod.rs` (388 loc) — clean
- [x] `src/target/linux_aarch64/plan.rs` (359 loc) — clean

**`src/target/linux_gtk/`**

- [x] `src/target/linux_gtk/app_io.rs` (542 loc) — clean (io-write malloc + grid index bounded; scaffold gaps self-documented)
- [x] `src/target/linux_gtk/bootstrap.rs` (608 loc) — bug-50, bug-59
- [x] `src/target/linux_gtk/mod.rs` (760 loc) — bug-59 (dead getenv import)
- [x] `src/target/linux_gtk/term_draw.rs` (653 loc) — clean

**`src/target/linux_riscv64/`**

- [x] `src/target/linux_riscv64/code.rs` (751 loc) — bug-44 (close), bug-71 (AAPCS64 comment)
- [x] `src/target/linux_riscv64/mod.rs` (414 loc) — bug-71 (musl-flavor diagnostic dumps)
- [x] `src/target/linux_riscv64/plan.rs` (359 loc) — clean

**`src/target/linux_x86_64/`**

- [x] `src/target/linux_x86_64/code.rs` (818 loc) — bug-44 (fsync + close int-return width)
- [x] `src/target/linux_x86_64/mod.rs` (413 loc) — clean
- [x] `src/target/linux_x86_64/plan.rs` (323 loc) — bug-71 (dead `_exit`/`getentropy`/io.flush imports)

**`src/target/macos_aarch64/`**

- [x] `src/target/macos_aarch64/code.rs` (792 loc) — bug-44 (fsync + close int-return width)
- [x] `src/target/macos_aarch64/mod.rs` (381 loc) — clean
- [x] `src/target/macos_aarch64/plan.rs` (618 loc) — bug-71 (stale io.flush fsync+errno imports)
- [x] `src/target/macos_aarch64/tls.rs` (221 loc) — clean

**`src/target/macos_aarch64/app/`**

- [x] `src/target/macos_aarch64/app/app_io.rs` (1094 loc) — bug-53 (owned NSString leak)
- [x] `src/target/macos_aarch64/app/bootstrap.rs` (865 loc) — bug-53 (owned NSAttributedString leak), bug-70 (exit-code>255 formatter)
- [x] `src/target/macos_aarch64/app/mod.rs` (697 loc) — clean
- [x] `src/target/macos_aarch64/app/term_view.rs` (1226 loc) — bug-46 (backspace fallthrough)

**`src/target/package_mfp/`**

- [x] `src/target/package_mfp/mod.rs` (370 loc) — bug-58 (name path-traversal)

**`src/target/shared/`**

- [x] `src/target/shared/lower.rs` (19 loc) — clean
- [x] `src/target/shared/mod.rs` (6 loc) — clean
- [x] `src/target/shared/validate.rs` (1610 loc) — bug-45 (collect_bind_types skips FOR EACH)

**`src/target/shared/code/`**

- [x] `src/target/shared/code/builder_arena_transfer.rs` (851 loc) — clean (copy_record aliasing sites stash to slots correctly)
- [x] `src/target/shared/code/builder_bits.rs` (293 loc) — clean (shift counts guarded 0..=63)
- [x] `src/target/shared/code/builder_codegen_primitives.rs` (1926 loc) — bug-70 (temporary_vreg `.expect` under -regalloc bump)
- [x] `src/target/shared/code/builder_collection_compare.rs` (469 loc) — clean
- [x] `src/target/shared/code/builder_collection_layout.rs` (1788 loc) — clean (map bucket region sized correctly)
- [x] `src/target/shared/code/builder_collection_mutate.rs` (4149 loc) — bug-47 (prepend/set-list/set-map grow leaks)
- [x] `src/target/shared/code/builder_collection_queries.rs` (1394 loc) — bug-70 (closure-branch `.expect`)
- [x] `src/target/shared/code/builder_collection_query.rs` (625 loc) — clean
- [x] `src/target/shared/code/builder_control.rs` (1331 loc) — bug-47 (StoreGlobal leak), bug-57 (WHILE stale-constant fold)
- [x] `src/target/shared/code/builder_conversions.rs` (1012 loc) — bug-49 (toInt radix negative overflow)
- [x] `src/target/shared/code/builder_emit_helpers.rs` (499 loc) — clean
- [x] `src/target/shared/code/builder_fixed_math.rs` (918 loc) — bug-61 (pow linear-loop hang)
- [x] `src/target/shared/code/builder_fs_paths.rs` (643 loc) — bug-65 (pathBaseName all-slash, pathNormalize("") OOB)
- [x] `src/target/shared/code/builder_inplace_assign.rs` (560 loc) — clean
- [x] `src/target/shared/code/builder_math.rs` (1252 loc) — clean
- [x] `src/target/shared/code/builder_numeric.rs` (1719 loc) — bug-61 (Fixed÷ min traps, pow hang)
- [x] `src/target/shared/code/builder_pow.rs` (792 loc) — clean (faithful fdlibm port)
- [x] `src/target/shared/code/builder_search.rs` (1106 loc) — clean (search bounds guarded)
- [x] `src/target/shared/code/builder_simd_fixed_math.rs` (316 loc) — clean
- [x] `src/target/shared/code/builder_simd_float_math.rs` (1373 loc) — bug-68 (NaN-only reduce latent, stale comments)
- [x] `src/target/shared/code/builder_simd_math.rs` (815 loc) — bug-68 (float min/max/clamp tail sign-of-zero)
- [x] `src/target/shared/code/builder_strings.rs` (1432 loc) — bug-60 (replace/list-replace unchecked size-arith)
- [x] `src/target/shared/code/builder_strings_builtins.rs` (2747 loc) — bug-60 (join unchecked size-arith)
- [x] `src/target/shared/code/builder_strings_package.rs` (450 loc) — clean
- [x] `src/target/shared/code/builder_value_semantics.rs` (810 loc) — clean
- [x] `src/target/shared/code/builder_values.rs` (1733 loc) — clean
- [x] `src/target/shared/code/builder_vector_inline.rs` (364 loc) — clean
- [x] `src/target/shared/code/code_impl.rs` (329 loc) — clean
- [x] `src/target/shared/code/codegen_utils.rs` (698 loc) — clean (finalize_frame trusts the save-set; see bug-54 at its source)
- [x] `src/target/shared/code/crypto.rs` (235 loc) — clean
- [x] `src/target/shared/code/crypto_ec.rs` (278 loc) — clean
- [x] `src/target/shared/code/data_objects.rs` (1265 loc) — clean
- [x] `src/target/shared/code/datetime.rs` (122 loc) — bug-42 (localOffset unchecked localtime_r)
- [x] `src/target/shared/code/entry_and_arena.rs` (2126 loc) — bug-70 (stderr formatter sign from x19); arena size-arith fully guarded
- [x] `src/target/shared/code/error_constants.rs` (457 loc) — clean
- [x] `src/target/shared/code/float_format.rs` (602 loc) — clean (precision bounded by Byte type; digit counts fit buffers; limb arith invariants hold; alloc register-safe)
- [x] `src/target/shared/code/fma_fusion.rs` (265 loc) — bug-69 (product-redef guard gap, latent)
- [x] `src/target/shared/code/fs_helpers.rs` (150 loc) — clean (errno mapping)
- [x] `src/target/shared/code/fs_helpers_atomic.rs` (1525 loc) — bug-44, bug-51, bug-63
- [x] `src/target/shared/code/fs_helpers_io.rs` (1913 loc) — bug-51, bug-62, bug-63
- [x] `src/target/shared/code/fs_helpers_paths.rs` (1898 loc) — bug-48 (listDirectory TOCTOU overflow)
- [x] `src/target/shared/code/function_lowering.rs` (928 loc) — bug-70 (dead if/else in scan_loop_locals)
- [x] `src/target/shared/code/io_helpers.rs` (1781 loc) — bug-51, bug-62
- [x] `src/target/shared/code/link_thunk.rs` (985 loc) — bug-56 (register escalation into x19), bug-66 (AND/OR bitwise, CInt32 const truncation)
- [x] `src/target/shared/code/mir.rs` (1627 loc) — clean (fuse/expand exact inverses)
- [x] `src/target/shared/code/mod.rs` (3161 loc) — clean (dispatch routes to distinct helpers; unreachable arms guarded)
- [x] `src/target/shared/code/module_analysis.rs` (920 loc) — bug-67 (empty-string-constant skips FOR/DO UNTIL)
- [x] `src/target/shared/code/os.rs` (1324 loc) — bug-64 (env/pwd thread-unsafety)
- [x] `src/target/shared/code/peephole.rs` (371 loc) — bug-69 (store-to-load sp-overlap assumption, latent)
- [x] `src/target/shared/code/runtime_helpers.rs` (853 loc) — bug-60 (thread-queue capacity overflow)
- [x] `src/target/shared/code/runtime_helpers_thread.rs` (1304 loc) — clean (vreg-allocated, direction-queue routing correct)
- [x] `src/target/shared/code/serialization_utils.rs` (17 loc) — clean
- [x] `src/target/shared/code/simd_kernel_coeffs.rs` (101 loc) — clean (generated minimax coeffs)
- [x] `src/target/shared/code/term.rs` (952 loc) — clean
- [x] `src/target/shared/code/type_utils.rs` (294 loc) — clean
- [x] `src/target/shared/code/types.rs` (562 loc) — bug-69 (union-variant tag name-keying, latent)
- [x] `src/target/shared/code/validation.rs` (428 loc) — clean

**`src/target/shared/code/crypto_ec/`**

- [x] `src/target/shared/code/crypto_ec/macos.rs` (1283 loc) — bug-55 (SecKey/CFData error-path leaks)
- [x] `src/target/shared/code/crypto_ec/openssl.rs` (1374 loc) — bug-55 (EVP leaks on error, assign unchecked, key scratch un-zeroed)

**`src/target/shared/code/net/`**

- [x] `src/target/shared/code/net/io.rs` (1612 loc) — bug-55 (lookup addr_fail freeaddrinfo leak)
- [x] `src/target/shared/code/net/mod.rs` (789 loc) — clean (OS-06 connect fd leak known)
- [x] `src/target/shared/code/net/poll.rs` (217 loc) — clean

**`src/target/shared/code/private/`**

- [x] `src/target/shared/code/private/mod.rs` (1 loc) — clean
- [x] `src/target/shared/code/private/unicode.rs` (1067 loc) — bug-70 (dead seqindex helpers + stale spec); utf8 decode bounds-safe

**`src/target/shared/code/regalloc/`**

- [x] `src/target/shared/code/regalloc/analysis.rs` (644 loc) — bug-69 (stream_is_riscv field sniff, latent)
- [x] `src/target/shared/code/regalloc/linear_scan.rs` (325 loc) — bug-54 (callee-saved scratch not saved)
- [x] `src/target/shared/code/regalloc/mod.rs` (310 loc) — clean

**`src/target/shared/code/tls/`**

- [x] `src/target/shared/code/tls/macos.rs` (3139 loc) — bug-52 (readText encoding-error leak), bug-55 (connection leaks)
- [x] `src/target/shared/code/tls/mod.rs` (397 loc) — clean (shared-ctx borrow rule sound)
- [x] `src/target/shared/code/tls/openssl.rs` (2137 loc) — bug-55 (alloc_fail fd/SSL leaks, min-proto unchecked)

**`src/target/shared/nir/`**

- [x] `src/target/shared/nir/json.rs` (830 loc) — bug-70 (`unreachable!` on crafted record/resource kind)
- [x] `src/target/shared/nir/lower.rs` (528 loc) — clean
- [x] `src/target/shared/nir/mod.rs` (346 loc) — clean
- [x] `src/target/shared/nir/symbols.rs` (33 loc) — clean

**`src/target/shared/plan/`**

- [x] `src/target/shared/plan/function_builder.rs` (637 loc) — bug-70 (constant-clear over-reports plan calls)
- [x] `src/target/shared/plan/json.rs` (180 loc) — clean
- [x] `src/target/shared/plan/lower.rs` (208 loc) — clean
- [x] `src/target/shared/plan/mod.rs` (477 loc) — clean
- [x] `src/target/shared/plan/symbols.rs` (694 loc) — clean (collect_bind_type_names handles ForEach correctly)

**`src/target/shared/runtime/`**

- [x] `src/target/shared/runtime/catalog.rs` (169 loc) — clean (bidirectional spec parity verified)
- [x] `src/target/shared/runtime/crypto_specs.rs` (153 loc) — clean
- [x] `src/target/shared/runtime/datetime_specs.rs` (46 loc) — clean
- [x] `src/target/shared/runtime/fs_specs.rs` (495 loc) — clean
- [x] `src/target/shared/runtime/io_specs.rs` (192 loc) — bug-70 (IO_FLUSH_SPEC empty clobbers field)
- [x] `src/target/shared/runtime/mod.rs` (133 loc) — clean
- [x] `src/target/shared/runtime/net_specs.rs` (627 loc) — clean
- [x] `src/target/shared/runtime/os_specs.rs` (231 loc) — clean
- [x] `src/target/shared/runtime/strings_specs.rs` (189 loc) — clean
- [x] `src/target/shared/runtime/term_specs.rs` (216 loc) — clean
- [x] `src/target/shared/runtime/thread_specs.rs` (284 loc) — clean
- [x] `src/target/shared/runtime/usage.rs` (293 loc) — clean (required_helpers traverses ForEach)

**`src/testing/`**

- [x] `src/testing/desugar.rs` (1109 loc) — clean (TRAP/coverage desugar sound)
