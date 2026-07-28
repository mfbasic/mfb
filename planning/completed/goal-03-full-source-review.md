# goal-03: Full compiler source review (fresh pass) — file-by-file bug hunt

Last updated: 2026-07-12
Status: COMPLETE (279 / 279 files reviewed)

## Objective

Read **every production source file in the compiler** (`src/**`), one file at a
time, and hunt for defects of any kind. This is a fresh, independent pass over
the whole tree — [goal-01](goal-01-compiler-source-review.md) reviewed the tree
as of 2026-07-09 (263 files, bugs 09–71) and
[goal-02](goal-02-full-source-review.md) re-reviewed it as of 2026-07-10/11
(265 files, bugs 88–147). Since then the tree has grown to **279 production
files (~207k LOC)** and substantial code has landed: the built-in audio
synthesis package (plan-33), the shadow-grid terminal surface (plan-35), build
progress output (plan-36), bare-`TRAP` (plan-37), resource-closed defaults
(plan-38), and more. **Do not assume a file is unchanged because an earlier goal
checked it — re-read it.**

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

For **each item found**, a `bug-NN-shortname.md` document was created in
`bugs/`. The deliverable of this goal is the review coverage (every file
checked off below) **plus** one bug document per real finding (batched by module
where same-class).

## Scope

**279 production files, ~207k LOC** across `src/**`. Exclusions per the original
scope (unit-test `*/tests.rs`, `testutil.rs`, `test_support.rs`, the
`repository/` crate, the root `tests/` tree) were honored.

## Prior work — do NOT re-file known findings

Cross-checked against [goal-01](goal-01-compiler-source-review.md),
[goal-02](goal-02-full-source-review.md), bugs 148–152, and
[security-review-1.md](security-review-1.md). No known-and-fixed finding was
re-filed; the open `arena transient-churn quadratic` item
([allocator-20](allocator-20-coalesce-size-authority.md)) is referenced, not
duplicated (see bug-175H, which corrects the fictional large-bin-drain comment).

## Findings ledger

Next free bug number after this goal: **179**.

| Finding | File(s) | Class | Severity |
|---------|---------|-------|----------|
| [bug-153](../bugs/bug-153-mfp-type-graph-decode-stack-overflow.md) | binary_repr/reader.rs | Security (untrusted `.mfp` DoS) | MEDIUM |
| [bug-154](../bugs/bug-154-x86-add-carry-zero-token-rhs-adds-r8.md) | arch/x86_64 emitter (add_carry) | Correctness (x86 miscompile, PCG64 seed) | HIGH |
| [bug-155](../bugs/bug-155-toint-named-arg-param-table-misbind.md) | builtins/general.rs (toInt) | Correctness (named-arg bind) | MEDIUM |
| [bug-156](../bugs/bug-156-return-and-with-update-numeric-literal-not-coerced.md) | ir/lower.rs (RETURN, WITH) | Correctness (**runtime-confirmed**) | HIGH |
| [bug-157](../bugs/bug-157-macos-tls-write-byte-payload-count-not-capacity.md) | tls/macos.rs | Correctness (CAPACITY vs COUNT) | MEDIUM |
| [bug-158](../bugs/bug-158-riscv-v128-fmls-two-rounding.md) | arch/riscv64/v128.rs | Correctness (float fusion) | MEDIUM |
| [bug-159](../bugs/bug-159-fs-errno-mapping-fallthrough-misclassify.md) | fs_helpers.rs (+6 callers) | Correctness (wrong error code) | MEDIUM |
| [bug-160](../bugs/bug-160-net-sendto-byte-payload-count-not-capacity.md) | net/io.rs (sendTo) | Correctness (CAPACITY vs COUNT) | HIGH |
| [bug-161](../bugs/bug-161-nir-symbol-fragment-collision.md) | nir/symbols.rs | Correctness (symbol collision) | MEDIUM |
| [bug-162](../bugs/bug-162-ir-verify-builtin-return-type-not-reconciled.md) | ir/verify/mod.rs | Memory-safety/Security (`.mfp`) | MEDIUM |
| [bug-163](../bugs/bug-163-thread-write-helper-datasize-aliases-timespec.md) | runtime_helpers_thread.rs | Memory-safety (arena corruption) | HIGH |
| [bug-164](../bugs/bug-164-simd-exp-no-large-arg-guard.md) | builder_simd_float_math.rs (exp) | Correctness (large-arg) | MEDIUM |
| [bug-165](../bugs/bug-165-macos-term-clear-worker-thread-uaf.md) | macos app_io/term_view | Memory-safety (UAF race) | MEDIUM |
| [bug-166](../bugs/bug-166-fs-atomic-write-not-crash-durable.md) | fs_helpers_atomic.rs | Correctness (durability) | MEDIUM |
| [bug-167](../bugs/bug-167-alsa-audio-pollTimeout-and-devices-bugs.md) | audio/alsa.rs (+macos.rs) | Correctness/Memory-safety | MEDIUM |
| [bug-168](../bugs/bug-168-linker-relocation-range-checks-missing.md) | os/linux+macos link/** | Footgun (silent truncation) | LOW |
| [bug-169](../bugs/bug-169-type-name-scanners-non-ascii-char-boundary-panic.md) | resolution/monomorph/inference/types | Memory-safety (latent panic) | LOW |
| [bug-170](../bugs/bug-170-net-fs-libc-int-return-not-sign-extended.md) | net/** + fs open | Correctness (bug-04 class) | MEDIUM (low-conf) |
| [bug-171](../bugs/bug-171-ast-frontend-robustness-nits.md) | ast/** | Security/Correctness/Footgun | LOW (batch) |
| [bug-172](../bugs/bug-172-cli-robustness-nits.md) | cli/build.rs, cli/pkg.rs | Security/Correctness/Dead-code | LOW (batch) |
| [bug-173](../bugs/bug-173-builtins-syntaxcheck-typecheck-nits.md) | builtins/** + syntaxcheck/** | Correctness/Footgun/Dead-code | LOW (batch; A=soundness) |
| [bug-174](../bugs/bug-174-middle-end-nits.md) | ir/verify, ir/lower, resolver, monomorph | Correctness/Footgun | LOW (batch) |
| [bug-175](../bugs/bug-175-codegen-robustness-nits.md) | shared/code/** (10 sites) | Correctness/Memory-safety/Dead-code | LOW (batch) |
| [bug-176](../bugs/bug-176-target-frontend-nits.md) | abi/validate/plan/gtk/regalloc/desugar | Correctness/Footgun/Dead-code | LOW (batch) |
| [bug-177](../bugs/bug-177-net-tls-crypto-robustness-nits.md) | net/tls/crypto + fs EINTR | Correctness/Memory-safety/Security | LOW (batch) |
| [bug-178](../bugs/bug-178-arch-encoder-latent-nits.md) | arch aarch64/x86 encoders | Correctness/Footgun (latent) | LOW (batch) |

Tallies: **HIGH 4** (154, 156, 160, 163) · **MEDIUM 12** (153, 155, 157, 158,
159, 161, 162, 164, 165, 166, 167, 170) · **LOW 10 batch docs** (168, 169,
171–178) covering ~45 individual items, including ~14 dead-code / stale-comment
nits.

Highest-impact: **bug-156** (RETURN/WITH literal not coerced — runtime-confirmed
wrong `Fixed`/`Money` values), **bug-154** (x86 `add_carry` zero-token rhs adds
`r8` — corrupts PCG64 seed on x86), **bug-160** (`net.sendTo` sends wrong bytes
for any append-built list), **bug-163** (thread write-helper stack-slot alias →
wrong-size `arena_free`).

## File census & progress

All files reviewed. `clean` = no finding; otherwise the bug id(s) filed.

**`src/`**

- [x] `src/coverage.rs` — clean
- [x] `src/doc.rs` — bug-175 (dead no-op)
- [x] `src/escape.rs` — clean
- [x] `src/fmt.rs` — clean
- [x] `src/internal_name.rs` — clean
- [x] `src/lexer.rs` — clean
- [x] `src/main.rs` — clean
- [x] `src/numeric.rs` — clean
- [x] `src/scope_privates.rs` — clean
- [x] `src/target.rs` — clean
- [x] `src/testing.rs` — clean
- [x] `src/unicode_backend.rs` — clean
- [x] `src/unicode_runtime_tables.rs` — clean (build-time table gen over vendored data)

**`src/arch/`**

- [x] `src/arch/mod.rs` — clean
- [x] `src/arch/ops.rs` — clean

**`src/arch/aarch64/`**

- [x] `src/arch/aarch64/backend.rs` — clean
- [x] `src/arch/aarch64/mod.rs` — clean
- [x] `src/arch/aarch64/regmodel.rs` — clean
- [x] `src/arch/aarch64/reloc.rs` — clean
- [x] `src/arch/aarch64/select.rs` — clean

**`src/arch/aarch64/encode/`**

- [x] `src/arch/aarch64/encode/data.rs` — clean
- [x] `src/arch/aarch64/encode/emitter.rs` — bug-178 (mov sp→xzr, latent)
- [x] `src/arch/aarch64/encode/mod.rs` — clean
- [x] `src/arch/aarch64/encode/operand.rs` — bug-178 (x18/x29 unencodable, latent)
- [x] `src/arch/aarch64/encode/sizing.rs` — clean

**`src/arch/riscv64/`**

- [x] `src/arch/riscv64/backend.rs` — clean
- [x] `src/arch/riscv64/mod.rs` — clean
- [x] `src/arch/riscv64/regmodel.rs` — clean
- [x] `src/arch/riscv64/reloc.rs` — clean
- [x] `src/arch/riscv64/select.rs` — clean
- [x] `src/arch/riscv64/v128.rs` — bug-158 (FMlsV two-rounding)

**`src/arch/riscv64/encode/`**

- [x] `src/arch/riscv64/encode/data.rs` — clean
- [x] `src/arch/riscv64/encode/emitter.rs` — clean
- [x] `src/arch/riscv64/encode/mod.rs` — clean
- [x] `src/arch/riscv64/encode/operand.rs` — clean
- [x] `src/arch/riscv64/encode/sizing.rs` — clean

**`src/arch/x86_64/`**

- [x] `src/arch/x86_64/backend.rs` — clean
- [x] `src/arch/x86_64/mod.rs` — clean
- [x] `src/arch/x86_64/regmodel.rs` — clean
- [x] `src/arch/x86_64/reloc.rs` — clean
- [x] `src/arch/x86_64/select.rs` — clean

**`src/arch/x86_64/encode/`**

- [x] `src/arch/x86_64/encode/data.rs` — clean
- [x] `src/arch/x86_64/encode/emitter.rs` — bug-154 (HIGH add_carry), bug-178 (clz comment)
- [x] `src/arch/x86_64/encode/mod.rs` — clean
- [x] `src/arch/x86_64/encode/operand.rs` — clean
- [x] `src/arch/x86_64/encode/sizing.rs` — clean

**`src/ast/`**

- [x] `src/ast/expr.rs` — bug-171 (unbounded parse recursion)
- [x] `src/ast/items.rs` — bug-171 (DOC unterminated paren)
- [x] `src/ast/lexical.rs` — clean
- [x] `src/ast/manifest.rs` — bug-171 (glob backtracking, canonicalize-before-filter)
- [x] `src/ast/mod.rs` — clean
- [x] `src/ast/parser.rs` — bug-171 (peek/previous unchecked)
- [x] `src/ast/serialize.rs` — bug-171 (isolated dropped, Trapped asymmetry)
- [x] `src/ast/stmt.rs` — clean
- [x] `src/ast/testing.rs` — clean
- [x] `src/ast/types.rs` — clean

**`src/audit/`**

- [x] `src/audit/json.rs` — clean
- [x] `src/audit/mod.rs` — clean
- [x] `src/audit/report.rs` — clean
- [x] `src/audit/text.rs` — clean

**`src/audit/collect/`**

- [x] `src/audit/collect/dependencies.rs` — clean
- [x] `src/audit/collect/findings.rs` — clean
- [x] `src/audit/collect/lockfile.rs` — clean
- [x] `src/audit/collect/mod.rs` — clean
- [x] `src/audit/collect/project.rs` — clean
- [x] `src/audit/collect/source.rs` — clean

**`src/binary_repr/`**

- [x] `src/binary_repr/builder.rs` — clean
- [x] `src/binary_repr/mod.rs` — clean
- [x] `src/binary_repr/reader.rs` — bug-153 (type-graph decode stack overflow)
- [x] `src/binary_repr/sections.rs` — clean
- [x] `src/binary_repr/util.rs` — clean
- [x] `src/binary_repr/writer.rs` — clean

**`src/builtins/`**

- [x] `src/builtins/audio.rs` — clean
- [x] `src/builtins/bits.rs` — clean
- [x] `src/builtins/collections.rs` — clean
- [x] `src/builtins/crypto.rs` — clean
- [x] `src/builtins/csv.rs` — clean
- [x] `src/builtins/datetime.rs` — clean
- [x] `src/builtins/encoding.rs` — clean
- [x] `src/builtins/errorcode.rs` — clean
- [x] `src/builtins/fs.rs` — clean
- [x] `src/builtins/general.rs` — bug-155 (toInt param-name table)
- [x] `src/builtins/http.rs` — clean
- [x] `src/builtins/io.rs` — clean
- [x] `src/builtins/json.rs` — clean
- [x] `src/builtins/math.rs` — bug-173 (dup helpers, resolve_call arity)
- [x] `src/builtins/mod.rs` — clean
- [x] `src/builtins/money.rs` — clean
- [x] `src/builtins/net.rs` — bug-173 (argument_types overloaded)
- [x] `src/builtins/os.rs` — clean
- [x] `src/builtins/regex.rs` — clean
- [x] `src/builtins/resource.rs` — clean
- [x] `src/builtins/strings.rs` — clean
- [x] `src/builtins/term.rs` — clean
- [x] `src/builtins/testing.rs` — clean
- [x] `src/builtins/thread.rs` — bug-173 (internal names user-reachable)
- [x] `src/builtins/tls.rs` — bug-173 (internal names user-reachable)
- [x] `src/builtins/vector.rs` — clean

**`src/cli/`**

- [x] `src/cli/build.rs` — bug-172 (non-exclusive test temp dir)
- [x] `src/cli/doc.rs` — clean
- [x] `src/cli/fmt.rs` — clean
- [x] `src/cli/init.rs` — clean
- [x] `src/cli/man.rs` — clean
- [x] `src/cli/mod.rs` — clean
- [x] `src/cli/pkg.rs` — bug-172 (install/update arity msg, stale comment)
- [x] `src/cli/repo.rs` — clean
- [x] `src/cli/resolve.rs` — clean
- [x] `src/cli/spec.rs` — clean

**`src/docs/`**

- [x] `src/docs/mod.rs` — clean
- [x] `src/docs/render.rs` — clean

**`src/docs/man/`**

- [x] `src/docs/man/mod.rs` — clean

**`src/docs/spec/`**

- [x] `src/docs/spec/mod.rs` — clean

**`src/ir/`**

- [x] `src/ir/binary.rs` — clean (decoder well-hardened)
- [x] `src/ir/json.rs` — clean
- [x] `src/ir/link.rs` — clean
- [x] `src/ir/lower.rs` — bug-156 (HIGH RETURN/WITH coercion), bug-174 (For col-0)
- [x] `src/ir/mod.rs` — clean
- [x] `src/ir/op.rs` — clean
- [x] `src/ir/package.rs` — clean
- [x] `src/ir/types.rs` — clean
- [x] `src/ir/value.rs` — clean

**`src/ir/verify/`**

- [x] `src/ir/verify/mod.rs` — bug-162 (builtin return type), bug-174 (Money+Unknown, value recursion)

**`src/manifest/`**

- [x] `src/manifest/entry.rs` — clean
- [x] `src/manifest/mod.rs` — clean
- [x] `src/manifest/package.rs` — clean (untrusted-`.mfp` parser well-hardened)

**`src/monomorph/`**

- [x] `src/monomorph/helpers.rs` — bug-169 (type-name char-boundary)
- [x] `src/monomorph/lower.rs` — bug-174 (arg_types filter_map drop)
- [x] `src/monomorph/mod.rs` — clean

**`src/os/`**

- [x] `src/os/mod.rs` — clean

**`src/os/linux/`**

- [x] `src/os/linux/flavor.rs` — clean
- [x] `src/os/linux/mod.rs` — clean
- [x] `src/os/linux/object.rs` — clean (plan-only JSON emitter)

**`src/os/linux/link/`**

- [x] `src/os/linux/link/elf.rs` — bug-168 (adrp range check)
- [x] `src/os/linux/link/mod.rs` — bug-168 (branch_imm26 / rel32 range checks)

**`src/os/macos/`**

- [x] `src/os/macos/icon.rs` — clean
- [x] `src/os/macos/mod.rs` — clean
- [x] `src/os/macos/object.rs` — clean (plan-only JSON emitter)

**`src/os/macos/link/`**

- [x] `src/os/macos/link/commands.rs` — bug-168 (section-offset u32 truncation)
- [x] `src/os/macos/link/macho.rs` — clean
- [x] `src/os/macos/link/mod.rs` — bug-168 (branch_imm26 / adrp range checks)

**`src/resolver/`**

- [x] `src/resolver/mod.rs` — bug-174 (alias dup detection)
- [x] `src/resolver/packages.rs` — clean
- [x] `src/resolver/resolution.rs` — bug-169 (type-name char-boundary)

**`src/rules/`**

- [x] `src/rules/mod.rs` — clean
- [x] `src/rules/table.rs` — clean (2-205 code overlap is documented/intentional)

**`src/syntaxcheck/`**

- [x] `src/syntaxcheck/builtins.rs` — bug-173 (named-arg, term index, io dead branch)
- [x] `src/syntaxcheck/checking.rs` — bug-173 (dead empty-body ifs)
- [x] `src/syntaxcheck/helpers.rs` — clean
- [x] `src/syntaxcheck/inference.rs` — bug-169 (type-name char-boundary)
- [x] `src/syntaxcheck/mod.rs` — clean
- [x] `src/syntaxcheck/resources.rs` — bug-173 (resource-union sendable vacuous)
- [x] `src/syntaxcheck/types.rs` — bug-173 (function-param covariance), bug-169 (map-body char-boundary)

**`src/target/linux_aarch64/`**

- [x] `src/target/linux_aarch64/code.rs` — clean
- [x] `src/target/linux_aarch64/mod.rs` — clean
- [x] `src/target/linux_aarch64/plan.rs` — bug-176 (thread import arm)

**`src/target/linux_gtk/`**

- [x] `src/target/linux_gtk/app_io.rs` — clean
- [x] `src/target/linux_gtk/bootstrap.rs` — clean
- [x] `src/target/linux_gtk/mod.rs` — bug-176 (lib_for panic)
- [x] `src/target/linux_gtk/term_draw.rs` — clean

**`src/target/linux_riscv64/`**

- [x] `src/target/linux_riscv64/code.rs` — clean
- [x] `src/target/linux_riscv64/mod.rs` — clean
- [x] `src/target/linux_riscv64/plan.rs` — bug-176 (thread import arm)

**`src/target/linux_x86_64/`**

- [x] `src/target/linux_x86_64/code.rs` — clean
- [x] `src/target/linux_x86_64/mod.rs` — clean
- [x] `src/target/linux_x86_64/plan.rs` — bug-176 (thread import arm)

**`src/target/macos_aarch64/`**

- [x] `src/target/macos_aarch64/code.rs` — clean
- [x] `src/target/macos_aarch64/mod.rs` — clean
- [x] `src/target/macos_aarch64/plan.rs` — clean
- [x] `src/target/macos_aarch64/tls.rs` — clean

**`src/target/macos_aarch64/app/`**

- [x] `src/target/macos_aarch64/app/app_io.rs` — bug-165 (term::clear UAF, call site)
- [x] `src/target/macos_aarch64/app/bootstrap.rs` — clean
- [x] `src/target/macos_aarch64/app/icon.rs` — clean
- [x] `src/target/macos_aarch64/app/mod.rs` — bug-176 (stale dead_code attrs)
- [x] `src/target/macos_aarch64/app/term_view.rs` — bug-165 (term::clear helper)

**`src/target/package_mfp/`**

- [x] `src/target/package_mfp/mod.rs` — clean

**`src/target/shared/`**

- [x] `src/target/shared/abi.rs` — bug-176 (temporary_register pinned regs)
- [x] `src/target/shared/lower.rs` — clean
- [x] `src/target/shared/mod.rs` — clean
- [x] `src/target/shared/regmodel.rs` — clean
- [x] `src/target/shared/validate.rs` — bug-176 (uppercase Local passes)

**`src/target/shared/code/`**

- [x] `src/target/shared/code/builder_arena_transfer.rs` — clean
- [x] `src/target/shared/code/builder_bits.rs` — clean
- [x] `src/target/shared/code/builder_codegen_primitives.rs` — clean
- [x] `src/target/shared/code/builder_collection_compare.rs` — bug-175 (byte-compare advance, latent)
- [x] `src/target/shared/code/builder_collection_layout.rs` — clean
- [x] `src/target/shared/code/builder_collection_mutate.rs` — bug-175 (element alignment, latent)
- [x] `src/target/shared/code/builder_collection_queries.rs` — clean
- [x] `src/target/shared/code/builder_collection_query.rs` — clean
- [x] `src/target/shared/code/builder_control.rs` — clean
- [x] `src/target/shared/code/builder_conversions.rs` — clean
- [x] `src/target/shared/code/builder_emit_helpers.rs` — clean
- [x] `src/target/shared/code/builder_fixed_math.rs` — clean
- [x] `src/target/shared/code/builder_fs_paths.rs` — clean
- [x] `src/target/shared/code/builder_inplace_assign.rs` — clean
- [x] `src/target/shared/code/builder_math.rs` — clean
- [x] `src/target/shared/code/builder_money.rs` — bug-175 (round re-multiply overflow)
- [x] `src/target/shared/code/builder_money_math.rs` — clean
- [x] `src/target/shared/code/builder_numeric.rs` — clean
- [x] `src/target/shared/code/builder_pow.rs` — clean
- [x] `src/target/shared/code/builder_search.rs` — clean
- [x] `src/target/shared/code/builder_simd_fixed_math.rs` — clean
- [x] `src/target/shared/code/builder_simd_float_math.rs` — bug-164 (exp large-arg), bug-175 (stale comment)
- [x] `src/target/shared/code/builder_simd_math.rs` — bug-175 (duplicate const)
- [x] `src/target/shared/code/builder_strings.rs` — clean
- [x] `src/target/shared/code/builder_strings_builtins.rs` — bug-175 (split/case_map size)
- [x] `src/target/shared/code/builder_strings_package.rs` — clean
- [x] `src/target/shared/code/builder_value_semantics.rs` — clean
- [x] `src/target/shared/code/builder_values.rs` — bug-175 (dead Const-String, union size)
- [x] `src/target/shared/code/builder_vector_inline.rs` — clean
- [x] `src/target/shared/code/code_impl.rs` — clean
- [x] `src/target/shared/code/codegen_utils.rs` — clean
- [x] `src/target/shared/code/crypto.rs` — bug-177 (entropy zeroize, randomBytes overflow)
- [x] `src/target/shared/code/crypto_ec.rs` — clean
- [x] `src/target/shared/code/data_objects.rs` — clean
- [x] `src/target/shared/code/datetime.rs` — clean
- [x] `src/target/shared/code/entry_and_arena.rs` — bug-175 (large-bin comment, entry range-error)
- [x] `src/target/shared/code/error_constants.rs` — clean
- [x] `src/target/shared/code/float_format.rs` — clean
- [x] `src/target/shared/code/fma_fusion.rs` — clean
- [x] `src/target/shared/code/fs_helpers.rs` — bug-159 (errno fall-through)
- [x] `src/target/shared/code/fs_helpers_atomic.rs` — bug-159, bug-166 (durability), bug-170 (open), bug-177 (EINTR)
- [x] `src/target/shared/code/fs_helpers_io.rs` — bug-170 (open sign-extend)
- [x] `src/target/shared/code/fs_helpers_paths.rs` — bug-159 (errno fall-through)
- [x] `src/target/shared/code/function_lowering.rs` — clean
- [x] `src/target/shared/code/io_helpers.rs` — clean
- [x] `src/target/shared/code/link_thunk.rs` — clean
- [x] `src/target/shared/code/mir.rs` — clean
- [x] `src/target/shared/code/mod.rs` — clean
- [x] `src/target/shared/code/module_analysis.rs` — clean
- [x] `src/target/shared/code/os.rs` — clean
- [x] `src/target/shared/code/peephole.rs` — clean
- [x] `src/target/shared/code/runtime_helpers.rs` — clean
- [x] `src/target/shared/code/runtime_helpers_thread.rs` — bug-163 (HIGH DATA_SIZE alias)
- [x] `src/target/shared/code/serialization_utils.rs` — clean
- [x] `src/target/shared/code/simd_kernel_coeffs.rs` — clean
- [x] `src/target/shared/code/term.rs` — bug-175 (clear docstring)
- [x] `src/target/shared/code/term_grid.rs` — bug-175 (present-buffer headroom)
- [x] `src/target/shared/code/type_utils.rs` — bug-175 (function-type split)
- [x] `src/target/shared/code/types.rs` — clean
- [x] `src/target/shared/code/validation.rs` — clean

**`src/target/shared/code/audio/`**

- [x] `src/target/shared/code/audio/alsa.rs` — bug-167 (pollTimeout, devices SIGSEGV)
- [x] `src/target/shared/code/audio/macos.rs` — bug-167 (pollTimeout dispatch, leaks, dead labels)
- [x] `src/target/shared/code/audio/mod.rs` — clean

**`src/target/shared/code/crypto_ec/`**

- [x] `src/target/shared/code/crypto_ec/macos.rs` — clean
- [x] `src/target/shared/code/crypto_ec/openssl.rs` — bug-177 (keygen/encode return codes, SPKI length)

**`src/target/shared/code/net/`**

- [x] `src/target/shared/code/net/io.rs` — bug-160 (HIGH sendTo CAPACITY), bug-170, bug-177 (EINTR)
- [x] `src/target/shared/code/net/mod.rs` — bug-170 (sign-extend)
- [x] `src/target/shared/code/net/poll.rs` — bug-170 (sign-extend)

**`src/target/shared/code/private/`**

- [x] `src/target/shared/code/private/mod.rs` — clean
- [x] `src/target/shared/code/private/unicode.rs` — clean

**`src/target/shared/code/regalloc/`**

- [x] `src/target/shared/code/regalloc/analysis.rs` — clean
- [x] `src/target/shared/code/regalloc/linear_scan.rs` — clean
- [x] `src/target/shared/code/regalloc/mod.rs` — bug-176 (find_physical_operand scan)

**`src/target/shared/code/tls/`**

- [x] `src/target/shared/code/tls/macos.rs` — bug-157 (write CAPACITY vs COUNT)
- [x] `src/target/shared/code/tls/mod.rs` — clean
- [x] `src/target/shared/code/tls/openssl.rs` — bug-177 (load_fail fd leak, TLS-by-IP)

**`src/target/shared/nir/`**

- [x] `src/target/shared/nir/json.rs` — clean
- [x] `src/target/shared/nir/lower.rs` — clean
- [x] `src/target/shared/nir/mod.rs` — clean
- [x] `src/target/shared/nir/symbols.rs` — bug-161 (symbol_fragment collision)

**`src/target/shared/plan/`**

- [x] `src/target/shared/plan/function_builder.rs` — clean
- [x] `src/target/shared/plan/json.rs` — clean
- [x] `src/target/shared/plan/lower.rs` — clean
- [x] `src/target/shared/plan/mod.rs` — clean
- [x] `src/target/shared/plan/symbols.rs` — clean

**`src/target/shared/runtime/`**

- [x] `src/target/shared/runtime/audio_specs.rs` — clean
- [x] `src/target/shared/runtime/catalog.rs` — clean
- [x] `src/target/shared/runtime/crypto_specs.rs` — clean
- [x] `src/target/shared/runtime/datetime_specs.rs` — clean
- [x] `src/target/shared/runtime/fs_specs.rs` — clean
- [x] `src/target/shared/runtime/io_specs.rs` — clean
- [x] `src/target/shared/runtime/mod.rs` — clean
- [x] `src/target/shared/runtime/net_specs.rs` — clean
- [x] `src/target/shared/runtime/os_specs.rs` — clean
- [x] `src/target/shared/runtime/strings_specs.rs` — clean
- [x] `src/target/shared/runtime/term_specs.rs` — clean
- [x] `src/target/shared/runtime/thread_specs.rs` — clean
- [x] `src/target/shared/runtime/usage.rs` — clean

**`src/testing/`**

- [x] `src/testing/desugar.rs` — bug-176 (coverage dump aborts on first error)
