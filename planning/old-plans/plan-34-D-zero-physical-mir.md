# plan-34-D: Zero physical registers in the shared MIR — the FP_SCRATCH pool and the airtight guard

Last updated: 2026-07-10
Effort: large (3h–1d) — phases are individually small/medium and land independently
Depends on: plan-34-A (zero/lr/arena tokens), plan-34-B (role tokens, Phase 3b seam),
plan-34-C (SCRATCH pool, `%thread`, guard v1) — all complete. Interacts with bug-85
(the reverted plan-34-B Phase 4): this plan does NOT re-delete the x86 CFG inference;
every conversion realizes through the existing Phase-3b seam so the inference's input
is byte-identical.

> **STATUS: FULLY COMPLETE (2026-07-10).** Zero physical registers in every shared
> stream, every register class, all four targets — enforced by the no-allowlist source
> scan AND the always-on stream assertions, which caught three real leaks during the
> plan itself (platform emitter staging, linux_x86_64 native mmap staging, app-mode
> raw-input injectors). Commits: 6d10e17f, 7c134a7a, 84961f94, 4db8301a, 374fb6d2,
> 53109a44, 7ad0f5a6, d22e8f8a. Final validation: 2,483 unit tests green; artifact
> gate 969 goldens / 0 diffs; **full acceptance suite 861/861 passed**; full-exe
> baseline diff equivalent on 19/19 combos (pre-existing linux nondeterminism filed
> as bug-87); x86+riscv boxes run threads/math/datetime/live-TLS correctly.

plan-34-C eliminated every hand-picked physical **integer** register from shared
lowering and guarded the result. But the guard's net is narrower than its claim: it
scans only `src/target/shared/code/`, only quoted `"x9"`–`"x28"` literals, and only the
integer file. An audit (this session, 2026-07-10) found the physical registers that
remain in the shared MIR:

1. **The FP scratch bank `d0`–`d7`** — ~193 hand-picked literals across the float
   builders (`"d0"` ×115, `"d1"` ×46, `"d2"` ×19, `"d3"`–`"d7"` ×13). The FP analog of
   the bug-56 hazard class plan-34-C made unrepresentable for integers.
2. **`format!`-constructed names** — `link_thunk.rs` builds `format!("x{idx}")` (thunk
   `CodeParam.location`) and `format!("d{flt_idx}")` (C-call FP argument staging);
   invisible to a quoted-literal scan.
3. **abi helpers returning physical spellings into the stream** —
   `string_data_register()="x1"`, `string_length_register()="x2"`,
   `RETURN_REGISTER="x0"` (one instruction use: `datetime.rs` `localtime_r` NULL check).
4. **Runtime-helper spec locations** — `runtime/*_specs.rs` `location: "x0"`…`"x3"`
   flow into `CodeParam.location` and are emitted as prologue spill operands (MIR).
5. **The SIMD math-pool pin** — `RegisterModel::math_pool_base()` default `Some("x2")`
   (`shared/regmodel.rs:107`) flows into `builder_simd_float_math`'s stream on
   aarch64/riscv.

This plan converts every one of these to neutral tokens realized through the existing
plan-34-B Phase-3b seam (`abi::realize_abi_token`), teaches the register allocator's
occupancy analysis to see the new FP tokens (the one place correctness risk
concentrates), and then rebuilds the guard as (a) a comprehensive source scan over all
of `src/target/shared/` and (b) an always-on stream assertion at the
`run_register_allocation` seam. **Outcome: the shared-lowering-emitted MIR — every
function body entering register allocation, and every machine-floor stream (entry
stub, trampoline, formatter, link thunks) entering selection — names zero physical
registers, and both a source-level and a stream-level guard make regression
unrepresentable.**

References:

- `planning/old-plans/plan-34-A-zero-register-token.md`, `plan-34-B-role-named-registers.md`,
  `plan-34-C-vreg-remaining-scratch.md` — the token machinery this plan extends.
- `planning/bug-85-x86-entry-runtime-arg-staging-tokens.md` — why the x86 CFG
  inference stays and every realization must pass through the Phase-3b seam.
- `.ai/compiler.md` — runtime completion gate, register-lifetime rules.
- `mfb spec memory 06_native-calling-convention` + `src/docs/spec/**` (doc-sync
  obligation, plan-34-C precedent commit `0ba52fee`).

## 1. Goal

- After the final phase, both of these hold and are enforced by tests that run in
  `cargo test`:
  1. `rg`-style scan: no file under `src/target/shared/` **except `abi.rs`** contains,
     outside its `#[cfg(test)]` module, a quoted physical register name of any class
     (AArch64 `x0`–`x30`/`w*`/`d*`/`s*`/`v*`/`q*`, x86-64 GPR/`xmm*`, riscv64 ABI
     names) or a `format!("<reg-prefix>{…` construction of one.
  2. Stream assertion: every instruction stream shared lowering produces — function
     bodies at the `run_register_allocation` input, and the allocator-bypassing
     machine-floor streams (entry stub, thread trampoline, panic formatter, link
     thunks) — contains no field that `int_physical_index`/`fp_physical_index`
     recognizes as a physical register (`sp` excepted; it is the neutral
     stack-pointer spelling).
- All four targets produce **byte-identical executables** before/after (modulo none —
  no golden refresh is expected; unlike plan-34-C there is no allocatable-pool change).

### Non-goals (explicit constraints)

- **No behavior change, no byte change.** Every token realizes to exactly the physical
  spelling the code emits today, at the same pipeline point (the Phase-3b seam in
  selection, or earlier for allocator visibility). This is a renaming plan.
- **Do not re-delete the x86 CFG inference** (`remap_x86_abi`) or the `x31`→ZERO
  defensive arms in the backends — bug-85 territory; the backends' select code is the
  *realization layer* and may name physical registers freely.
- **Post-allocation MIR stays physical.** Coloring's output (`x8`–`x27`, `d0`–`d31`,
  per-target files) is the allocator's job; `abi::temporary_register`/
  `fp_temporary_register` (the BumpAndReset coloring oracle, `abi.rs:50/74`) produce
  allocator *output* and stay. The invariant covers the allocator's **input** and the
  machine-floor streams only.
- **`abi.rs` remains the one sanctioned home of physical spellings** in
  `src/target/shared/` (the realization tables: `realize_abi_token`, `SCRATCH`
  realizations, the coloring oracle, `is_callee_saved`). `arch/*` and per-target
  `target/{linux_*,macos_*}` code is out of scope entirely.
- **Do not touch the trampoline's structure** — its `%scratch` hand-liveness stands
  (plan-34-C follow-up notes; the vregged trampoline segfaults x86 cancellation).
- `RegisterModel::allocatable` pools are unchanged (no repeat of 34-C's x20 golden
  shift).

## 2. Current State

- **Token machinery.** `abi.rs` defines the neutral vocabulary: `ZERO`/`LR`/`ARENA`
  (plan-34-A), `ARG[8]`/`RET[4]`/`SYSARG[6]`/`SYSNR`/`SYSRET`/`CLOSURE_ENV`/
  `CURRENT_THREAD` (plan-34-B), `SCRATCH[19]` (plan-34-C).
  `abi::realize_abi_token` (`abi.rs:193`) is the Phase-3b seam: **all three backends
  apply it during selection** to translate each token to its AArch64 spelling before
  their per-ISA remap (aarch64 uses `xN` directly; riscv remaps `xN`→its file at
  `arch/riscv64/select.rs`; x86 runs `remap_x86_abi`'s CFG inference). A non-token
  passes through unchanged.
- **Pipeline order.** `function_lowering.rs:722` — `builder.run_register_allocation()`
  colors `%vN`/`%fN` at the **end of shared lowering**, before peephole/
  `finalize_frame`/selection. So anything shared lowering hand-names is visible to the
  allocator as a *string in the stream* at coloring time. The occupancy analysis
  (`regalloc/analysis.rs:392` `phys_busy_at`, consumed at `linear_scan.rs:133`
  `phys_busy_in`) parses stream fields with `int_physical_index`/`fp_physical_index`
  (`analysis.rs:156/190`) — physical names only; `%`-tokens return `None`.
  - This is safe today for every *int* token because their realizations (`x0`–`x8`,
    `x20`, `x28`) lie outside `INT_ALLOCATABLE` (`arch/aarch64/regmodel.rs:43`) — the
    allocator can never color onto them anyway.
  - It is **not** vacuously safe for FP: `FP_ALLOCATABLE`
    (`arch/aarch64/regmodel.rs:64`) *begins with* `d0`–`d7`. Today the builders'
    literal `"d0"` is seen by `fp_physical_index` and occupancy excludes it. A token
    the analysis cannot parse would silently lose that occupancy → the allocator
    colors a live `%fN` onto `d0` → clobber. **This is the plan's one real hazard.**
- **The FP literal sites.** `builder_numeric.rs`, `builder_math.rs`, `builder_pow.rs`,
  `builder_strings*.rs`, and neighbors hand-pick `d0`–`d7` as float scratch and as
  in-tree-kernel argument staging (e.g. `emit_float_pow("d0", "d1")`). The backends
  already treat `dN` as a neutral 1:1 bank: x86 `select.rs:409-421` maps `d/v/q N` →
  `xmmN`; riscv `select.rs:565` maps `dN` → `map_fp_register(n)`.
- **The int stragglers.** `abi.rs:3` `RETURN_REGISTER="x0"` (instruction use:
  `code/datetime.rs:116`; metadata use: many `runtime/*_specs.rs` `location:` fields);
  `abi.rs:247-253` `string_length_register()="x2"` / `string_data_register()="x1"`
  (callers: `term.rs`, `entry_and_arena.rs`, `fs_helpers_io.rs` — print/write argument
  staging); `link_thunk.rs:483/692` `format!("d{flt_idx}")`/`format!("x{idx}")`;
  `runtime/*_specs.rs` literal `location: "x0"`…`"x3"` (consumed at
  `code/mod.rs:1157+` into `CodeParam`, spilled by `function_lowering.rs:659` — these
  reach the MIR); `shared/regmodel.rs:106` `math_pool_base()` default `Some("x2")`
  (consumed via `math_pool_base_vreg` plumbing, `code/mod.rs:151-158`, and
  `builder_simd_float_math.rs:486` `emit_load_math_pool_base`).
- **The guard today.** `mir.rs:1653` `shared_lowering_names_no_physical_scratch_register`
  — quoted `"x9"`–`"x18"`/`"x20"`–`"x28"` literals, `src/target/shared/code/` only,
  skipping test modules. Passes. Its companions: `invariant_registers_are_neutral_tokens`
  (`mir.rs:1610`), the MIR round-trip tests.
- **Spec metadata that is NOT stream-visible** and stays physical inside `abi.rs`:
  `IO_PRINT_CLOBBERS` (`abi.rs:4`) — clobber lists are read for validation
  (`shared/validate.rs:212`) and docs, not emitted as MIR fields; the allocator's
  call-clobber model is the per-ISA static sets in `regalloc/analysis.rs:73+`.

## 3. Design Overview

Four independent conversions, then the guard. Every conversion is
"replace physical spelling S with token T where `realize(T) = S`", so selection input
is unchanged byte-for-byte; only the pre-realization stream (and `-mir` dumps) differ.

1. **`abi::FP_SCRATCH` pool** — `["%fscratch0"…"%fscratch7"]`, realized by
   `realize_abi_token` to `d0`…`d7`. The name deliberately avoids `%fs0` (collides
   with riscv's `fs0` ABI spelling in `riscv_fp_index`). Documented like `SCRATCH`:
   the low FP bank doubles as the AAPCS FP-argument registers, which is why C-call
   float staging (`link_thunk.rs:483`) uses the same pool.
2. **Allocator token-awareness** — extend `fp_physical_index` to map
   `"%fscratch{i}"` → `i`, exactly the index `"d{i}"` maps to today, so
   `phys_busy_at`/`Effect` def-use tracking and therefore coloring are bit-identical.
   (Int tokens stay unparsed: their realizations are outside the allocatable pool;
   adding them would only add dead occupancy bits. Do NOT add them — matching today's
   behavior is the byte-identity argument.)
3. **Int stragglers → existing role tokens.** No new int vocabulary is needed:
   - `string_data_register()`/`string_length_register()` return `ARG[1]`/`ARG[2]`
     (the sites stage the data/len arguments of print/write helpers; realization →
     `x1`/`x2`, inference input unchanged).
   - `datetime.rs:116` → `abi::RET[0]` (the comment already notes it is the return
     register).
   - `runtime/*_specs.rs` `location:` → `abi::ARG[i]`; delete `RETURN_REGISTER` once
     the last reference is gone.
   - `link_thunk.rs:692` → `abi::argument_register(idx)`; `:483` → `FP_SCRATCH[flt_idx]`.
   - `math_pool_base` → new token `abi::MATH_POOL = "%mathpool"`, realized → `x2`;
     the default impl in `shared/regmodel.rs` is removed and each arch declares it
     (aarch64/riscv `Some(abi::MATH_POOL)`, x86 `None` as today).
4. **The guard, rebuilt** (last, once offenders are zero):
   - *Source scan:* root widens to `src/target/shared/` minus `abi.rs`; forbidden set
     becomes every register class named in `int_physical_index`/`fp_physical_index`
     plus AArch64 `w`/`s`/`v`/`q` banks; a second scan rejects
     `format!("x{`/`format!("d{`/`format!("w{`/`format!("v{`/`format!("q{`/
     `format!("xmm` constructions. Test modules skipped as today; no allowlist.
   - *Stream assertion:* at the top of `run_register_allocation`, reject any field
     where `int_physical_index`/`fp_physical_index` is `Some` and the field is not
     `"sp"` — always-on (it is O(fields) over a pass that already walks every field),
     returning a lowering error naming the instruction. Machine-floor streams don't
     pass the allocator, so a unit test builds each (entry stub, trampoline,
     formatter, a link thunk) and applies the same predicate.

**Risk concentration:** piece 2. If any FP token site loses occupancy, the allocator
may color a live value onto a busy `dN` — a silent clobber the byte-gate would catch
only where allocation actually shifts. Mitigation: land pieces 1+2 together with a
regression test that reproduces the hazard (a `%fN` live across a `%fscratch0` def
must not color to `d0`), and byte-gate every phase.

**Rejected alternatives:**
- *Realize FP_SCRATCH before regalloc instead of teaching the analysis* — moves the
  Phase-3b seam for one token class only, splitting realization across two pipeline
  points; the analysis extension is one function arm and keeps the seam whole.
- *A separate `FP_ARG` bank for C-call float staging* — realizes to the same `dN`
  strings; two names for one realization invites drift. One pool, documented aliasing
  (the `SCRATCH` x20/x28 precedent).
- *Dedicated `%strdata`/`%strlen` tokens* — the sites are argument staging; `ARG[n]`
  already means that and needs no new realization arms.
- *Keeping spec `location:` physical as "metadata"* — rejected: `code/mod.rs:1157+`
  turns them into `CodeParam.location`, which `function_lowering.rs:659` emits as MIR
  spill operands. They are stream-visible and must be tokens.

## Compatibility / Format Impact

- **Executables:** byte-identical on all four targets (the acceptance bar for every
  phase). No golden refresh expected.
- **`-mir` / plan-JSON dumps:** textual change only — `dN`/`x1`/`x2`/`x0`/`x2`-pin
  spellings become `%fscratch*`/`%arg*`/`%ret0`/`%mathpool` (same as every prior
  plan-34 step; the byte-gate artifact is `.nobj`, not `-mir` — plan-34-A note).
- **Public surface:** none. `abi.rs` items are `pub(crate)`.

## Phases

### Phase 1 — FP_SCRATCH pool + allocator token-awareness (no callers yet)

Introduce the vocabulary and the occupancy mapping in one commit so the hazard window
never exists.

- [x] `abi.rs`: add `pub(crate) const FP_SCRATCH: [&str; 8]` (`"%fscratch0"`…`"%fscratch7"`)
      with a doc comment mirroring `SCRATCH`'s (what machine-floor/kernel scratch is,
      the AAPCS FP-argument aliasing note); extend `realize_abi_token` with the eight
      arms → `d0`…`d7`.
- [x] `regalloc/analysis.rs`: extend `fp_physical_index` to parse `"%fscratch{i}"` → `i`
      (document: same index the realized `d{i}` maps to, so occupancy is unchanged).
- [x] Tests (`abi.rs` tests + `regalloc/tests.rs`): realization of all eight tokens;
      `fp_physical_index("%fscratch0") == Some(0)`; the hazard regression — an interval
      test where a `%fN` live across a `%fscratch0` def/use must not be colored `d0`
      (build it with the existing linear-scan test harness).

Acceptance: new unit tests pass; `cargo test` green; byte-gate trivially identical
(no callers).
Commit: 6d10e17f
Note (implementation finding): x86's `dN`→`xmmN` arm runs in the main selection
loop, *before* the late Phase-3b seam — an FP token left for the seam would reach
the encoder unmapped. The x86 arm therefore realizes `d`-realizing tokens inline
(int tokens fall through untouched, preserving the inference's input — bug-85).

### Phase 2 — Convert the FP literal sites (~193) and the link-thunk `format!`s

Mechanical, file-by-file; non-test code only (test-module fixtures stay literal — the
guard skips them and they pin realization behavior).

- [x] Replace quoted `"d0"`–`"d7"` with `abi::FP_SCRATCH[0..8]` in every
      `src/target/shared/code/` builder above its test module (census list:
      `builder_numeric.rs`, `builder_math.rs`, `builder_pow.rs`,
      `builder_strings*.rs`, `builder_simd_*.rs`, `builder_collection_queries.rs`,
      `fs_helpers_io.rs`, `float_format.rs`, others per the Phase-2 census re-run).
- [x] `link_thunk.rs:483`: `format!("d{flt_idx}")` → `abi::FP_SCRATCH[flt_idx]`;
      `link_thunk.rs:692`: `format!("x{idx}")` → `abi::argument_register(idx)?`.
- [x] Re-run the census script; assert zero `dN`/`format!`-constructed names remain in
      non-test `shared/code`.

Acceptance: `scripts/artifact-gate.sh` byte-identical on all targets; full
`cargo test` green.
Commit: 7c134a7a (gate: 847 tests / 969 goldens / 0 diffs; host target — cross-ISA
byte checks land with Phase 6's box sweep)

### Phase 3 — Int stragglers to role tokens

- [x] `abi.rs`: `string_data_register()` → `ARG[1]`, `string_length_register()` →
      `ARG[2]` (keep the named helpers — they document the print-ABI roles).
- [x] `code/datetime.rs:116`: `abi::RETURN_REGISTER` → `abi::RET[0]`.
- [x] `runtime/*_specs.rs` (all ten files): `location:` literals and
      `abi::RETURN_REGISTER` references → `abi::ARG[i]` by position.
- [x] Delete `abi::RETURN_REGISTER` once reference-free (`IO_PRINT_CLOBBERS` stays —
      clobber metadata, `abi.rs` is sanctioned).
- [x] Verify the spill path: runtime-helper prologues (`function_lowering.rs:659`)
      now emit `%argN` operands; confirm selection realizes them at the seam
      (existing plan-34-B tests cover the arms).

Acceptance: byte-gate identical (this is bug-85 territory — additionally diff the
**final linked executables** of the acceptance corpus, not just `.nobj`, on aarch64 +
x86); `cargo test` green.
Commit: 84961f94 (gate 0 diffs after refreshing two .mir/.ncode goldens whose
param-metadata line changed `x0` → `%arg0`; instruction/nobj bytes identical.
The full-exe diff ran in Phase 6 — see its note on pre-existing nondeterminism.)

### Phase 4 — The math-pool pin becomes a token

- [x] `abi.rs`: add `pub(crate) const MATH_POOL: &str = "%mathpool"`;
      `realize_abi_token` arm → `"x2"`.
- [x] `shared/regmodel.rs`: delete the `math_pool_base` default body (make it a
      required method); `arch/aarch64/regmodel.rs` + `arch/riscv64/regmodel.rs`
      return `Some(abi::MATH_POOL)`; `arch/x86_64/regmodel.rs` keeps `None`.
- [x] Confirm `builder_simd_float_math` streams carry `%mathpool` pre-selection and
      the kernels' bytes are unchanged.

Acceptance: byte-gate identical on all targets; SIMD/transcendental unit tests green.
Commit: 4db8301a (simpler than planned: riscv64/x86-64 already overrode to `None`;
only the shared default carried the pin — flipped to `None`, aarch64 overrides with
the token.)

### Phase 4b — the vN NEON bank + the last int stragglers (unplanned, found by the Phase-5 pre-scan)

The Phase-2 census regex required a lane suffix, so plain `"v0"`–`"v7"` NEON
literals went uncounted: 729 sites across the three SIMD kernel files, plus five
int stragglers (`arena_base() == "s11"` ISA probes ×3, `== "rsp"` recognition ×2).

- [x] `abi::VEC_SCRATCH[0..8]` (`%vscratch0`…`7` → `v0`…`v7`, the 128-bit lane
      view of the FP_SCRATCH file); realize arms; `fp_physical_index` parses it
      at the same index (the views alias).
- [x] Convert all 729 `vN` literals (`builder_simd_float_math` 539,
      `builder_simd_math` 140, `builder_simd_fixed_math` 50).
- [x] ISA probes reference `arch::riscv64::regmodel::ARENA_BASE_REGISTER`; frame
      finalization references new `arch::x86_64::regmodel::STACK_POINTER`.

Acceptance: byte-gate identical; census of `src/target/shared/` non-test code = 0.
Commit: 374fb6d2

### Phase 5 — The guard, rebuilt (source scan + stream assertion)

Land last: it fails until Phases 2–4 are complete.

- [x] `mir.rs`: rewrite `shared_lowering_names_no_physical_scratch_register` →
      `shared_lowering_names_no_physical_register`: root `src/target/shared/` minus
      `abi.rs`; forbidden quoted literals = `x0..=x30`, `w0..=w30`, `d/s/v/q 0..=31`,
      the x86 GPR + `xmm0..=15` names, the riscv int + fp ABI names (reuse the
      `analysis.rs` tables — single source of truth); second scan for
      `format!("x{`-style constructions (all prefixes); test modules skipped; NO
      allowlist.
- [x] `run_register_allocation` (`code/mod.rs` or `regalloc/mod.rs` — wherever the
      entry sits): always-on input assertion — any field with
      `int_physical_index`/`fp_physical_index` = `Some` and ≠ `"sp"` is a lowering
      error naming the op and field. (Colored output is produced *after* this point,
      so the check is exact, and it is free — the analysis already walks every field.)
- [x] New test: build the machine-floor streams (entry stub via `entry_and_arena`,
      thread trampoline, panic formatter, one link thunk) and apply the same
      predicate to every field.
- [x] Update `invariant_registers_are_neutral_tokens` cross-references and the guard's
      doc comment (the §"guarded separately" claims now point at real code).

Acceptance: both guards pass; deliberately planting a `"d3"` in a builder or an
`"x9"` in `runtime/` fails the source scan; planting one in a lowering emit fails the
stream assertion with a named op.
Commit: 53109a44. Verified: the planted `"d3"` was caught at its exact line; the
stream assertions fired on TWO real leaks during bring-up and both were fixed:
(1) the per-platform `CodegenPlatform` emitters inject libc/syscall staging into
shared streams — all four `code.rs` files tokenized (`ARG`/`SYSARG`/`SCRATCH`/
`ZERO`, new `abi::SYSNR_DARWIN` → `x16` since the seam is ISA-wide and `%sysnr`
realizes Linux's `x8`); (2) `linux_x86_64/code.rs` staged mmap/getrandom/write
natively (rdi/rsi/rdx/r10/r8/r9/rax) → `SYSARG`/`%sysnr`. `int_physical_index`
parses the int tokens whose realizations are allocatable (`%scratch{i}`→9+i/10+i,
`%sysnr`→8, `%sysnr_darwin`→16) at exactly the replaced literal's index.
Boundary confirmed: standalone target-native streams (macOS app/TLS trampolines,
GTK app functions) never cross the assertions — realization-layer like `arch/`.

### Phase 6 — Validation sweep + doc sync (highest-risk verification last)

- [x] Full `cargo test` (all 2,555+ unit tests).
- [x] `scripts/artifact-gate.sh` — byte-identical, all targets.
- [x] Full-executable byte-diff of the acceptance corpus vs the pre-plan baseline
      commit on host (the bug-85 lesson: per-pkg `.nobj` alone missed entry/runtime
      divergence).
- [x] Runtime spot-checks on the boxes (plan-34-C precedent): host acceptance suite;
      x86 box (`ssh -p 2227`) and riscv box (`ssh -p 2229`) — hello-world, float/math
      acceptance (nbody or equivalent — FP_SCRATCH is the big surface),
      `thread-drop-cleanup`, one TLS connect.
- [x] Spec/doc sync: `src/docs/spec` memory section — document the FP_SCRATCH pool and
      the zero-physical invariant (update the register-model page plan-34-C touched in
      `0ba52fee`); `.ai/compiler.md` register-lifetime note if it names the old
      helpers.
- [x] Move this plan to `planning/old-plans/`.

Acceptance: everything above green; the executables are byte-identical to baseline;
memory updated.
Commit: 7ad0f5a6 (riscv v128 token slots + spec doc), d22e8f8a (app-mode injectors +
stale app goldens). Validation record:
- Full unit suite green (2,483 tests); artifact gate 969 goldens / 0 diffs.
- Full-exe byte-diff vs fa89792d: 19/19 target-combos equivalent. The linux
  math/datetime executables are NOT byte-deterministic at the baseline itself
  (4 hashes in 4 runs) — filed as `planning/bug-87-linux-exe-build-nondeterminism.md`;
  per-function ncode hashing across 3+3 runs shows every function body's hash set
  overlaps between the two compilers.
- Boxes: x86 (2227) and riscv (2229) — hello-world, control-flow, thread-drop-cleanup,
  math kernels, datetime, and a live TLS handshake all correct. The two build failures
  in the matrix (math_package/x86 imm32, signzero/riscv fmin_v) reproduce identically
  at the baseline — pre-existing.
- The stream guard caught a third real leak post-landing: -app builds (not in the
  console corpus) — macOS/GTK raw-input-mode injectors into the shared io read
  helpers; tokenized in d22e8f8a. The three macos-app-mode-* golden sets were stale
  since plan-34-C's x20 coloring shift and are refreshed to content verified equal
  to the baseline compiler's own output modulo dump text.
- Full acceptance suite: final clean run recorded before archiving.

## Validation Plan

- Tests: Phase-1 hazard regression (FP occupancy through tokens); realization arms for
  all new tokens; the two rebuilt guards (source + stream) with deliberate-plant
  negative cases; existing MIR round-trip suite stays green.
- Runtime proof: box matrix above — float-heavy programs are the proof the FP
  conversion is sound end-to-end (math acceptance + nbody on all three ISAs).
- Doc sync: spec memory register-model page; `.ai/compiler.md` if stale.
- Acceptance: `cargo test`, `scripts/artifact-gate.sh`, full-exe baseline diff,
  `tests/` acceptance suite on host + both boxes.

## Open Decisions

- **Token spelling `%fscratch*`** (recommended) vs `%fs*` — `%fs0`'s suffix collides
  with riscv's `fs0` ABI name in careless greps and minds; the long form is
  unambiguous. (§3.1)
- **Stream assertion severity** — always-on lowering error (recommended: it is free
  and makes the invariant hold in production builds) vs `debug_assert` + test-only.
  (§3.4)
- **`string_*_register()` helpers** — keep as named wrappers over `ARG[1]`/`ARG[2]`
  (recommended: they document the print ABI) vs inline the tokens at call sites.

## Summary

The engineering risk is a single point: FP occupancy through tokens (Phase 1) — get
that wrong and the allocator silently clobbers a live float on the preferred `d0`–`d7`
bank. Everything else is mechanical renaming behind the existing Phase-3b seam,
verified byte-identical per phase. Untouched: the backends' realization/select code
(including the x86 inference — bug-85), the allocatable pools, the trampoline's
hand-liveness, post-allocation physical output, and `abi.rs`'s role as the one home of
physical spellings in shared code.
