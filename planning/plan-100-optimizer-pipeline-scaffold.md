<!-- Feature plan. plan-100: install the -O dial (default -O1) and gate the existing Level-1 passes; reserve the two future seams. -->

# Optimizer pipeline scaffold — `-O` dial (default `-O1`) gating the existing Level-1 passes

Last updated: 2026-08-23
Effort: medium (half-day)

> **2026-08-23 revision — two changes, then a redesign.**
> 1. **Codegen was relocated.** The target-generic backend moved out of
>    `src/target/shared/code/**` into a tiered `src/codegen/**` (commits
>    `f32179ed4`, `eed149cd1`). Regalloc, the `builder_registers` seam site, and
>    every CLI `regalloc::` path now live under `src/codegen/engine/**`. All
>    References/Phase coordinates below are re-pinned to the new tree.
> 2. **A real MIR-opt layer already exists** at `src/codegen/compiler/opt/`
>    (`fma_fusion`, `peephole`, `selfmove_probe`), invoked from
>    `src/codegen/engine/function/function_lowering.rs`.
>
> **Redesign (this revision).** Rather than adding two *no-op* seams and leaving the
> existing passes ungated, the scaffold now **absorbs the existing passes into the
> gated pipeline** at their catalogued level, and the dial's **default is `-O1`**:
> - `fuse_scalar_fma`, `forward_stores_to_loads`, `remove_fp_shuttles` are all
>   **Level 1** rows in `planning/optimizations.md` (FMA instruction-combining;
>   machine store-to-load forwarding; machine copy-propagation / redundant-move
>   elimination). They move into `src/optimizer/opt2/` and are gated by
>   `level_enabled(1)`.
> - **`-O1` is the default (on).** At `-O1` the Level-1 passes run exactly as today,
>   so default/`-O1` codegen is **byte-identical to today's goldens**.
> - **`-O0` turns everything off.** It skips the Level-1 passes and therefore emits
>   **legitimately different, unoptimized** code — this is a *new, correct* path, not
>   a byte-identity target. Its proof is "builds + behaves identically," not
>   "byte-identical goldens" (see Phase 3).
> - `selfmove_probe` is a **read-only diagnostic** (env-gated `MFB_BUG387_SELFMOVE`),
>   not a transform — it is **not** gated and stays in `src/codegen/compiler/opt/`.
>
> **Module location (per request):** the scaffold lands in a new top-level
> `src/optimizer/` module — `src/optimizer/opt1/` (the NIR seam), `src/optimizer/opt2/`
> (the MIR/machine passes + the reserved MIR seam), and `src/optimizer/mod.rs` (the
> `OptLevel` global + `mod optimizer;` in `src/main.rs`).

Put the optimizer *pipeline* in place and wire the existing passes onto the dial —
without writing a single **new** optimization. Add an opt-level flag (`-O1` default,
`-O0` off) and gate the three already-shipped Level-1 passes behind it, plus two
reserved seams (Opt1 NIR, Opt2 MIR) that are **no-ops today** — homes for future
rows from the catalog in `planning/optimizations.md`.

Target pipeline (this plan wires the **bold** parts; new rows land later):

```
AST → HIR → IR → NIR → gated[Opt1(NIR)] → Plan1(storage/StorageType/symbols) → MIR
    → gated[ Plan2(CFG + SSA/def-use) → Opt2(MIR) → Out-of-SSA(MIR) ]
    → gated[ FMA-combine ] → regalloc → gated[ machine peepholes ] → machine code
```

- **Opt1(NIR)** — a `NirModule → NirModule` transform seam (`src/optimizer/opt1/`).
  No rows yet; identity today.
- **Opt2 bracket** — the reserved between-selection-and-regalloc MIR seam
  (`src/optimizer/opt2/`). No rows yet; identity today. Plan2 (CFG + SSA/def-use),
  Out-of-SSA, and Plan2's demand-driven analyses (SSA/mem2reg, alias, memory-SSA,
  range/trap, loop canonicalization, `no-trap` inference) are the *interior* of this
  bracket and are **not built in this plan** — they arrive with the first real Opt2
  pass that needs them (see Non-goals / Open Decisions).
- **Landed Level-1 passes (gated this plan).** `fuse_scalar_fma` (pre-regalloc,
  on the neutral stream) and the two post-regalloc machine peepholes
  (`forward_stores_to_loads`, `remove_fp_shuttles`) are absorbed into
  `src/optimizer/opt2/`, each self-guarded by `level_enabled(1)`. They run at
  their existing pipeline positions — the peepholes need physical registers, so
  they stay post-regalloc, not inside the reserved between-select seam.
- **Gating is per-row, not per-seam.** Each pass runs iff `row.level <= active_opt_level()`,
  so one `-ON` lights up rows across every seam (level ≠ stage). The dial is
  `-O0..-O5` (escalating shape distortion at *preserved* behavior); **Level 6** (`-O6`)
  is an orthogonal, explicit opt-in for semantic-relaxing passes (fast-math, the
  trap-order-affecting † rows) and is *never* implied by the dial, not even at
  `-O5`/"max". This scaffold has exactly three rows, all Level 1 — so `-O1`..`-O5`
  behave identically (today's codegen) and `-O0` alone is different.

References:

- `planning/optimizations.md` — the pass catalog + scale. The three landed rows:
  "Instruction selection / combining" (L1, FMA), "Peephole optimization" / the
  block-local "Store-to-load forwarding" embryo (L1, machine peephole),
  "Machine copy propagation / redundant-move elimination" (L1, `remove_fp_shuttles`).
- `--regalloc` flag plumbing, mirrored exactly for `-O` (all `regalloc::` paths are
  now `crate::codegen::engine::regalloc::` after the codegen move):
  - CLI parse: `src/cli/build/options.rs:8` (`parse_common_option`), `:41`
    (`parse_build_options`), `:119` (`parse_test_options`).
  - Options struct: `src/cli/build/mod.rs:91` (`BuildOptions`), field at `:113`.
  - Hand-off to backend global: `src/cli/build/mod.rs:179`
    (`crate::codegen::engine::regalloc::set_strategy(options.regalloc)`).
  - Global module pattern: `src/codegen/engine/regalloc/mod.rs:60` (`RegallocKind`),
    `:85` (`parse_kind`), `:96` (`SELECTED: OnceLock`), `:100` (`set_strategy`),
    `:107` (`active_kind`).
  - Parser parity tests: `src/cli/build/mod.rs:994-1002,1074-1076,1174-1177`.
  - Package-path defaults that also set regalloc: `src/cli/pkg.rs:127,420`.
- Opt1 seam site: `src/target/shared/lower.rs:8` (`lower_project`), wrap the
  `nir::lower_module(...)` result at `:21` (unchanged by the codegen move — `lower.rs`
  stayed under `target/shared`). Covers all four targets at once (consumed at
  `macos_aarch64/mod.rs:319` +5 sibling sites, `linux_aarch64/mod.rs:298`,
  `linux_x86_64/mod.rs:306`, `win_x86_64/mod.rs:414`).
- Reserved Opt2 MIR seam site: `src/codegen/engine/regalloc/builder_registers.rs:110`
  (`run_register_allocation`); between selection (`:151`,
  `self.instructions = backend.select(neutral)`) and regalloc (`:155`,
  `regalloc::allocate(...)`). No row occupies it yet.
- The passes to absorb + gate — all `&mut`-in-place, currently ungated, in
  `src/codegen/compiler/opt/`:
  - `fma_fusion.rs:70` `fuse_scalar_fma` — 1 call site,
    `src/codegen/engine/function/function_lowering.rs:1002` (pre-regalloc).
  - `peephole.rs:199` `forward_stores_to_loads` — 3 call sites,
    `function_lowering.rs:1066,1227,1524` (post-regalloc).
  - `peephole.rs:296` `remove_fp_shuttles` — 3 call sites,
    `function_lowering.rs:1070,1228,1525` (post-regalloc).
  - `selfmove_probe.rs` — read-only diagnostic, **not** absorbed, **not** gated.
- Golden harness: `scripts/test-accept.sh:385` (the primary `mfb build` invocation;
  also `:401` app, `:417` pkg-`.run`), `:109-112` (`MFB_TARGET` global-switch
  precedent), `scripts/sync-goldens.sh`.
- `.ai/testing-gates.md` (byte-identity, acceptance golden harness),
  `.ai/build-tooling.md` (rustfmt/clippy policy), `.ai/compiler.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| `--regalloc` is threaded via `BuildOptions` field + a `set_*`/`active_*` `OnceLock` global | `rg -n 'regalloc::set_strategy\|active_kind\|struct BuildOptions' src/cli src/codegen/engine/regalloc/mod.rs` | MET 2026-08-24 — 7 hits: `regalloc/mod.rs:107` `active_kind`, `cli/build/mod.rs:91` `struct BuildOptions` + `:179` `set_strategy`, `options.rs:49,122` defaults, `pkg.rs:127,420` |
| The Opt1 seam site returns the sole `NirModule` for every target | `rg -n 'lower_project' src/target` | MET 2026-08-24 — one definition (`shared/lower.rs:8`), 10 call sites across **five** targets (riscv64 too — see Corrections) |
| The three passes to gate are all Level 1 in the catalog | read `planning/optimizations.md` rows 103/138/156/200 | MET 2026-08-24 — "Peephole optimization" L1, "Instruction selection / combining" L1, "Machine copy propagation / redundant-move elimination" L1 ("Store-to-load forwarding" row is L3 = the *future* alias-based broadening; the landed block-local machine version rides the L1 peephole row) |
| Gating each pass at its function entry covers every call site | `rg -n 'fuse_scalar_fma\|forward_stores_to_loads\|remove_fp_shuttles' src/codegen/engine` | MET 2026-08-24 — exactly 1 + 3 + 3 production call sites in `function_lowering.rs` (1002 / 1066,1227,1524 / 1070,1228,1525), one guard each |
| `-O1` (default) reproduces today's exact codegen | Phase 3 default + `MFB_OPT=1` golden runs are clean diffs | UNMEASURED — Phase 3 gate |
| `-O0` builds and behaves identically (codegen may differ) | Phase 3 `MFB_OPT=0` run: builds pass + `.run` behavior matches | UNMEASURED — Phase 3 gate |

The whole value of this plan is that the **default (`-O1`) changes nothing** — the
Level-1 passes still run, so default codegen is byte-identical to today. `-O0` is the
only path that moves, and it moves *on purpose* (optimizations off).

## 1. Goal

- `mfb build` and `mfb test` accept `-O0`, `-O1`, `-O 0`/`-O 1`, `--optimize=0/1`
  (and the `-optimize` single-dash forms), mirroring `--regalloc` exactly.
  Unknown levels error the same way `--regalloc bogus` does.
- **Default is `-O1`.** Absent the flag, the Level-1 passes run and behavior +
  codegen are unchanged from today. `-O0` disables all dial passes.
- An `OptLevel` (numeric `0..=6`, a `u8` newtype, `Default = OptLevel(1)`) rides
  `BuildOptions` and is published to a process-wide `OnceLock`
  (`set_opt_level` / `active_opt_level`), same shape as `RegallocKind`. Levels `0..5`
  are the cumulative dial; `6` is the orthogonal semantic-relaxation opt-in (never
  reached by escalating the dial). The scaffold's parser accepts only `0` and `1`
  for now (Non-goals), but the type spans the full range so later passes slot in
  without a type change.
- The three existing Level-1 passes live in `src/optimizer/opt2/` and each begins
  with `if !crate::optimizer::level_enabled(1) { return; }`. At `-O0` they no-op; at
  `-O1`+ they run exactly as today.
- `optimize_nir(module, level)` exists as the **Opt1** seam (no rows yet), called in
  `lower_project` — identity today.
- The reserved **Opt2 MIR seam** (between selection and regalloc) exists as a no-op
  pass-through — home for future dataflow passes; no row occupies it yet.
- **Default/`-O1` machine code is byte-identical to today's goldens.** `-O0` machine
  code differs (dial passes off) but is correct: every fixture builds and its runtime
  behavior is unchanged.
- The harness can build/verify at a chosen opt level via a global switch (`MFB_OPT`),
  mirroring `MFB_TARGET`.

### Non-goals (explicit constraints)

- **No *new* optimization is implemented.** The only passes that run are the three
  already-shipping Level-1 ones, now dial-gated. Both reserved seams (Opt1 NIR, Opt2
  MIR) are identity. The rest of `optimizations.md` is future work, one pass at a time.
- **No SSA / CFG / analysis infrastructure is built.** Plan2 (CFG + SSA/def-use),
  Out-of-SSA, and Plan2's **demand-driven prerequisites** — SSA promotion (mem2reg),
  alias analysis, memory-SSA/memory-dependence, range/trap analysis, loop
  canonicalization, and function-attribute (`no-trap`) inference — are documented as
  the *interior* of the Opt2 bracket but are **not** implemented here. The reserved
  Opt2 seam is a single identity pass-through; these arrive with the first Opt2 pass
  that needs them. (See Open Decisions and §5.)
- **No `-O2`–`-O6` accepted yet.** The `OptLevel` type spans `0..=6` (dial `0–5` plus
  the `6` semantic-relaxation opt-in), but the parser accepts only `0` and `1` for
  now. Because all landed rows are Level 1, `-O1`..`-O5` would be identical anyway;
  higher levels are enabled by later plans as rows land. Level 6 additionally requires
  an explicit request and is never implied by "max".
- **No size objective (`-Os`/`-Oz`).** Size is a *second, orthogonal* axis (it
  re-weights pass profitability, not risk), composed with a numeric level — out of
  scope for the scaffold, a later flag. `OptLevel` stays a pure risk dial here.
- **`-O0` is not a byte-identity target.** Unlike the default, `-O0` legitimately
  differs from today's goldens (it turns optimizations off). Do **not** create a
  parallel `-O0` golden set and do **not** re-baseline the `-O1` goldens to `-O0`
  output. `-O0` is proven by "builds + behaves," not by byte-identity.
- **No new per-fixture flag file.** Opt-level goldens use a global `MFB_OPT`
  switch, matching the existing `MFB_TARGET` precedent; a per-fixture `build.args`
  seam is explicitly out of scope.
- **`selfmove_probe` is untouched.** It is a read-only diagnostic, not a dial pass;
  it keeps its env-gate and stays in `src/codegen/compiler/opt/`.

## 2. Phase 1 — the `-O`/`--optimize` flag + `OptLevel` global (default `-O1`)

Mirror `RegallocKind` end to end, except the default is `OptLevel(1)`, not 0.

- [x] New top-level module `src/optimizer/` — add `mod optimizer;` to
      `src/main.rs` (alphabetical, between `mod numeric;` and `mod os;`).
      `src/optimizer/mod.rs`: wires `pub(crate) mod opt1; pub(crate) mod opt2;` and
      holds the `OptLevel` global — `OptLevel(u8)` spanning `0..=6` with
      `Default = OptLevel(1)`, `parse_kind(&str)` (accepting only `0`/`1` for now —
      Non-goals), `available_levels()`, `static SELECTED: OnceLock<OptLevel>`,
      `set_opt_level`, `active_opt_level()` → defaults `OptLevel(1)`, plus a
      `level_enabled(row_level: u8) -> bool` helper (`row_level <= active_opt_level().0`)
      for the per-row seam filter. Copy the
      `src/codegen/engine/regalloc/mod.rs:60-109` shape; the one deliberate
      divergence from `RegallocKind` is the non-zero default.
- [x] `src/cli/build/options.rs`: add an `opt: &mut OptLevel` out-param to
      `parse_common_option` (or a sibling), handling `-O`/`--optimize`/`-optimize`
      in both space and `=` forms (lines 25-34 pattern). Default from
      `crate::optimizer::active_opt_level()` in `parse_build_options` (`:49` pattern)
      and `parse_test_options` (`:122` pattern); store into the struct at
      `:109`/`:148`.
- [x] `src/cli/build/mod.rs:91`: add `pub(crate) opt: crate::optimizer::OptLevel` to
      `BuildOptions`. `src/cli/pkg.rs:127,420`: default
      `opt: crate::optimizer::active_opt_level()`.
- [x] `src/cli/build/mod.rs:179`: add `crate::optimizer::set_opt_level(options.opt);`
      next to the `regalloc::set_strategy` call in `build_project`.
- [x] Parser parity unit tests mirroring `src/cli/build/mod.rs:994-1002,1074-1076,1174-1177`:
      `-O0` == `--optimize=0` == `-O 0`; `-O1` == `--optimize=1` == `-O 1`; **absent
      flag defaults to `OptLevel(1)`**; bogus level (e.g. `-O2`, `-Ox`) errors.
- Commit: `fc42026db` — `plan-100: add -O/--optimize opt-level flag (default -O1) mirroring --regalloc`

## 3. Phase 2 — absorb the Level-1 passes + reserve the two seams

- [x] **Relocate + gate the Level-1 passes.** Move `fma_fusion.rs` and `peephole.rs`
      from `src/codegen/compiler/opt/` into `src/optimizer/opt2/` (wire
      `src/optimizer/opt2/mod.rs`). Add to the top of each pass:
      `if !crate::optimizer::level_enabled(1) { return; }` — one guard per function
      (`fuse_scalar_fma`, `forward_stores_to_loads`, `remove_fp_shuttles`), covering
      all 7 call sites. Repoint the imports in
      `src/codegen/engine/function/function_lowering.rs` (currently
      `use crate::codegen::compiler::opt::{fma_fusion, peephole};`) to
      `crate::optimizer::opt2::{...}`; the 7 call sites are otherwise unchanged.
      Leave `selfmove_probe` in `src/codegen/compiler/opt/` (diagnostic, not gated).
- [x] **Opt1 seam.** Add `pub(crate) fn optimize_nir(module: NirModule, level: OptLevel) -> NirModule`
      in `src/optimizer/opt1/mod.rs`. Identity today (no rows), with a doc comment
      listing the Opt1 catalog rows (`optimizations.md`) as future contents. Call it
      in `src/target/shared/lower.rs:21`, wrapping the `nir::lower_module(...)?` result
      (`crate::optimizer::opt1::optimize_nir(module, crate::optimizer::active_opt_level())`).
- [x] **Reserved Opt2 MIR seam.** Add `pub(crate) fn optimize_mir(instructions: &mut Vec<CodeInstruction>, level: OptLevel)`
      in `src/optimizer/opt2/mod.rs` — in-place, matching the neighboring peephole
      signatures. Identity today (no rows). Call it in
      `src/codegen/engine/regalloc/builder_registers.rs` between selection (`:151`)
      and `regalloc::allocate` (`:155`), reading `crate::optimizer::active_opt_level()`.
      Doc comment marks this as the future home of Plan2(CFG + SSA/def-use +
      demand-driven mem2reg/alias/memory-SSA/range-trap/loop-canonicalization/
      `no-trap`-inference analyses) → Opt2 passes → Out-of-SSA; note the machine
      peepholes stay post-regalloc (they need physical registers) rather than moving
      into this seam.
- [x] Unit tests: (a) with `set_opt_level(OptLevel(0))`, each of the three passes is
      a no-op on a stream that at `-O1` it would rewrite (guard fires); (b) the two
      reserved seams (`optimize_nir` by value, `optimize_mir` in place) leave input
      structurally unchanged at every level (no accidental fire). These pin both the
      gate and the identity of the empty seams.
- Commit: `41ee2d909` — `plan-100: absorb FMA+peephole Level-1 passes onto the -O dial; reserve Opt1/Opt2 seams`

## 4. Phase 3 — harness opt-level switch + neutrality/correctness proof

- [x] `scripts/test-accept.sh`: add an `MFB_OPT` global switch mirroring
      `MFB_TARGET` (`:109-112`): when set, build an `opt_arg="-O$MFB_OPT"` there and
      append it to each `run_with_watchdog "$MFB_EXE" build` invocation (`:385`
      primary, `:401` app, `:417` pkg-`.run`). Default (unset) = no flag = the harness
      binary's own default = `-O1` = today's exact command, so the existing golden run
      is untouched.
- [x] **The gate — split by intent:**
      1. **default (no `MFB_OPT`)** — clean diff against current goldens. Proves the
         dial defaulting to `-O1` moved nothing.
      2. **`MFB_OPT=1`** — clean diff. Proves explicit `-O1` == default.
      3. **`MFB_OPT=0`** — a *correctness* run, **not** a byte-identity run. Codegen
         artifacts (`.ncodesum`/`.ir`) are **expected to drift** (dial passes off);
         do not re-baseline them. Require instead: every fixture **builds**, and every
         `.run`/behavior golden **matches** *except* for members of an
         **enumerated, individually-justified FP-contraction exception set** — see
         the Corrections entry "`-O0` behavior is not golden-identical". Two of the
         three gated passes (`forward_stores_to_loads`, `remove_fp_shuttles`) are
         strictly behavior-preserving, so **any** `-O0` divergence they could explain
         is a real bug. `fuse_scalar_fma` is not: FMA contraction rounds once instead
         of twice (plan-02 §6.2), so a fixture whose golden depends on that single
         rounding legitimately differs at `-O0`.
         The strengthened, checkable criterion — a divergent fixture passes only if
         **all four** hold:
         (i) it is listed in the exception set below, with its diff;
         (ii) its source contains a float `a*b (+|-) c` that `fuse_scalar_fma` fuses
              (confirmed by a fused op in the `-O1` `--ncode` that is absent at `-O0`);
         (iii) the `-O0` divergence is *only* the documented contraction difference —
              an `ErrFloatOverflow` where `-O1` is finite, or a last-ULP digit — never
              a wrong control-flow path, a crash, or an unrelated value; and
         (iv) it **matches its golden again** when rebuilt at `-O1`.
         Every other fixture must match byte-for-byte on behavior. A build failure at
         `-O0`, or a divergence failing any of (i)–(iv), is a real bug (an unguarded
         dependency on one of the passes); a pure codegen-artifact drift at `-O0` is
         expected and ignored.
      **Exception set — enumerated and justified.** Measured 2026-08-24,
      `MFB_OPT=0 bash scripts/test-accept.sh target/release/mfb /tmp/accept-o0`:
      8 mismatches over 1265 fixtures, of which **7 are `.ncode` codegen-artifact
      drift** (expected, ignored by this gate) and **exactly 1 is behavior**:

      | fixture | class | verdict |
      |---|---|---|
      | `rt-behavior/arithmetic/float-fma-fusion/build.log` | behavior | **exception, all four checks pass** |
      | `rt-behavior/collections/func_map_getor_hash_probe/….ncode` | codegen | expected drift |
      | `rt-behavior/collections/list-ops-codegen-rt/….ncode` | codegen | expected drift |
      | `rt-behavior/control-flow/control-flow-if/….ncode` | codegen | expected drift |
      | `syntax/app/macos-app-mode-io/….app.ncode` | codegen | expected drift |
      | `syntax/app/macos-app-mode-plumbing/….app.ncode` | codegen | expected drift |
      | `syntax/lexical/parser-hello-world/….ncode` | codegen | expected drift |
      | `syntax/match/control-flow-match/….ncode` | codegen | expected drift |

      The lone behavior exception against checks (i)–(iv):
      (i) enumerated above — it is the *only* behavior divergence in 1265 fixtures.
      (ii) its source is `LET r AS Float = a * 2.0 - a` with `a = 1.5e308`, and
           `grep -c 'fmadd\|fmsub\|fnmsub'` on the `--ncode` dump gives **5 at `-O1`,
           0 at `-O0`** — the fusion demonstrably is what changed.
      (iii) the diff is *only* the contraction difference — the four preceding
           printed values are byte-identical, and only the deliberately
           overflow-probing line moves:
           ```
            2.0000 / 4.0000 / 2.5000     (identical)
           -fused-finite-ok
           -[exit 0]
           +Error: 7-705-0015
           +Floating-point arithmetic overflowed to infinity.
           +[exit 255]
           ```
           No wrong control-flow path, no crash, no unrelated value.
      (iv) the same fixture matches its golden at `-O1` — gates 1 and 2 are both
           clean over it.

      Sanity-checked that the 7 codegen drifts are the dial and not garbage: in
      `control-flow-if` every hunk is `-{"op":"mov","dst":"x9","src":"x10"}` →
      `+{"op":"ldr_u64","dst":"x9","base":"sp",…}` — the reload
      `forward_stores_to_loads` folds at `-O1`, left as a real load at `-O0`.
      The rt-error companion `arithmetic-float-fma-observed-rt` does **not**
      diverge: it expects the trap at both levels.
      Runs 1–2: any diff is a gate bug, not a re-baseline — fix it, do not touch
      goldens (`AGENTS.md`).
- [x] Full `cargo test --no-fail-fast` green (parser parity + gate/identity tests). 62 test binaries, 0 failures (2026-08-24).
- [x] `rustup run 1.96.0 cargo fmt --all` + `cargo fmt --all --manifest-path repository/Cargo.toml`; no churn left.
- Commit: `plan-100: MFB_OPT harness switch; prove -O1 byte-identical, -O0 correct`

## 5. Follow-on (out of scope — one pass at a time, later plans)

Once the scaffold lands, each *new* optimization is its own small plan: it fills in
`optimize_nir`/`optimize_mir` (or adds a machine-peephole row) for a single catalog
row tagged with its scale level, self-guards by `level_enabled(row.level)`, gets its
**own** golden set at the level that first enables it (goldens may move *only* at that
level and up, never at the levels below), and adds a RED test proving the transform
fires. The first Opt2 pass that needs dataflow is also the plan that builds **Plan2** —
the persistent CFG + SSA/def-use (promoting the throwaway `build_cfg` in
`src/codegen/engine/regalloc/analysis.rs:523`), its **demand-driven analyses**
(mem2reg/SSA promotion, alias analysis, memory-SSA, range/trap analysis, loop
canonicalization, and function-attribute/`no-trap` inference — the prerequisites
listed in `optimizations.md`), and **Out-of-SSA** (phi elimination before regalloc).
**Level 6** (fast-math + the trap-order-relaxing † passes) is a later, orthogonal
opt-in with its own explicitly-requested golden set; it is never enabled by escalating
the numeric dial. Loop unrolling (an Opt1 row, level 5) is a natural first *Opt1* pass
because it needs no CFG/SSA — loops are still structured `NirOp::For`/`While` nodes at
the Opt1 seam; a behavior-preserving check-elision pass (broadening plan-39/plan-86) is
a natural first *Opt2* pass and the one that first justifies Plan2's range/trap analysis.

The always-on passes migrating in here are also the model for whether the *level-`—`
infrastructure* rows (base instruction selection, register allocation) ever want a
dial-gated *refinement* row — they do not move wholesale onto the dial; only their
optional refinements (coalescing, remat, cost-based combining) become rows.

## Corrections

- **Five targets consume `lower_project`, not four.** The References line lists
  "all four targets"; `rg -n 'lower_project' src/target` (2026-08-24) returns 10
  call sites across **five** backends — `macos_aarch64` (6), `linux_aarch64`,
  `linux_x86_64`, `win_x86_64`, **and `linux_riscv64/mod.rs:263`**. The Opt1 seam
  covers riscv64 too. No scope change: one wrap in `shared/lower.rs` still covers
  every target.
- **"Store-to-load forwarding" is an L3 row; the landed pass rides the L1 peephole
  row.** The Prerequisites row pointed at `optimizations.md` "rows 103/138/156/200"
  for three Level-1 entries. The standalone "Store-to-load forwarding" row is
  **Level 3** — it is the *future alias-analysis-based* broadening, and its own text
  says the shipping `forward_stores_to_loads` is "gated as a **Level-1** machine
  peephole under the 'Peephole optimization' row". So the three landed passes map to
  L1 rows "Peephole optimization", "Instruction selection / combining", and "Machine
  copy propagation / redundant-move elimination". Gating at level 1 is unchanged.
- **`level_enabled` and the `mod opt1;`/`mod opt2;` lines moved from the Phase-1
  commit to the Phase-2 commit.** Phase 1 lists them, but they have no consumer
  until Phase 2 lands the passes and seams; committing them a phase early means a
  commit that either does not compile (missing submodule files) or warns
  `dead_code`. Both land with their first consumer instead. No behavior difference —
  the two commits together are exactly what Phase 1 + Phase 2 specify.
- **`-O0` behavior is NOT golden-identical — the plan's "optimizations are
  behavior-preserving" premise is false for `fuse_scalar_fma`, and Phase 3's gate
  #3 was strengthened accordingly.** Measured 2026-08-24 on
  `tests/rt-behavior/arithmetic/float-fma-fusion` copied to `/tmp/o100`:

  ```
  $ target/release/mfb build     /tmp/o100 && /tmp/o100/build/....out
  10.0000 / 2.0000 / 4.0000 / 2.5000 / fused-finite-ok          exit 0
  $ target/release/mfb build -O0 /tmp/o100 && /tmp/o100/build/....out
  10.0000 / 2.0000 / 4.0000 / 2.5000
  Error: 7-705-0015 Floating-point arithmetic overflowed to infinity.   exit 255
  $ grep -c 'fmadd\|fmsub\|fnmsub' <--ncode dump>   # -O1: 5   -O0: 0
  ```

  This is the dial working, **not** a bug: the fixture's own comment (plan-02 §6.2)
  says `a * 2.0 - a` with `a = 1.5e308` overflows as a *separate* multiply and stays
  finite only because the single-rounded `fmsub` never materializes the product. FMA
  contraction is therefore a rounding change, so it is not behavior-preserving on
  overflow — the plan's Phase-3 wording ("optimizations are behavior-preserving, so
  runtime output is level-invariant") is simply wrong for this one pass. The other
  two gated passes *are* behavior-preserving (`forward_stores_to_loads` never removes
  or reorders; `remove_fp_shuttles` folds only provably-dead GPR shuttles).

  The criterion was **strengthened, not weakened**: `-O0` behavior divergence is now
  allowed only for an enumerated exception set, each member of which must exhibit a
  fused op present at `-O1` and absent at `-O0`, a diff that is *only* the documented
  contraction difference, and a clean match when rebuilt at `-O1` — four checks where
  the plan previously had one blanket assertion.

  Not a level-assignment error: `optimizations.md:157` catalogs FMA combining as
  Level 1 and does **not** mark it `†`, and moving it to Level 6 would take it out of
  `-O1` and so break this plan's core "default is byte-identical to today" goal. The
  observation that FP contraction is arguably a `†` semantic-relaxing row is left as
  a note for a future Level-6 plan; it changes nothing here.

- **Added task, not in the plan: fixed a real harness bug that made the Phase-3
  gate both flaky and wrong.** The first default-level gate run reported 2
  failures and `1193 test(s) ran`; a control build of `main` (`git worktree add
  --detach`, `cargo build --release`) reported the *same* 2 failures but
  `1208 test(s) ran` — the same suite, a different fixture count, so the suite
  was silently under-running. Root cause, in `scripts/test-accept.sh`: the
  driving loop is `while IFS= read -r project_json; do … done < <(find … )`, and
  the behavioral-test branch ran `test_out=$("$MFB_EXE" test …)` **bare** — the
  one subprocess in the file not going through `run_with_watchdog`, which exists
  precisely to give children `/dev/null` stdin (bug-320). So `mfb test` inherited
  the `find` pipe as stdin, and any fixture whose `TESTING` blocks read stdin ate
  the fixture list. That single defect produced *both* long-standing failures:
  * `expected a trap with code 77020003, but none occurred` — the io EOF cases
    saw pipe bytes instead of the EOF they assert; and
  * `could not read project name for fb/.claude/worktrees/P-100/tests/…` — the
    next fixture's path arrived truncated at a random prefix, with a
    nondeterministic number of fixtures swallowed.

  Fixed by routing that call through `run_with_watchdog`, and by adding
  `</dev/null` to the two other loop-body `mfb test` invocations (the `.testrun`
  capture and the `--coverage` run) that had the same inherited-stdin exposure.
  **Result: 1265 fixtures run and the suite is green** — the fix recovered 72
  fixtures that were never being executed and removed 2 permanent failures.
  Recorded here because the plan's gate is only as trustworthy as the harness
  running it; these two failures were previously written off as environmental.

- **Added task, not in the plan: document the flag.** `-O` is user-facing surface,
  and `AGENTS.md` requires the embedded spec to track every compiler change. Phase 1
  therefore also adds `-O`/`--optimize` to `BUILD_HELP`/`TEST_HELP`
  (`src/cli/help.rs`) and to the CLI reference (`src/docs/spec/tooling/07_cli-reference.md`:
  flag table, `mfb test` usage row, single-dash alias list, and the malformed-value
  diagnostics paragraph). A flag with no help line and no spec row is a flag users
  cannot find.

## Open Decisions

- **Default level (resolved 2026-08-23).** Default is **`-O1`** (Level-1 rows on),
  `-O0` is the all-off baseline. Rationale: today's shipping codegen already runs the
  three Level-1 passes, so `-O1`-as-default keeps the default byte-identical while
  giving `-O0` a real "no optimizations" meaning. The alternative (`-O0` default,
  matching gcc/clang) would make the *default* differ from today's goldens — a much
  larger, needless re-baseline — and is rejected.
- **Gating model (resolved).** Per-row `row.level <= active_opt_level()` at each pass's
  entry, *not* a binary bracket on/off — so one level lights up rows across every seam
  (level ≠ stage), and **Level 6** is an orthogonal opt-in never implied by the dial.
  The Plan2 prerequisites (mem2reg/SSA, alias, memory-SSA, range/trap, loop
  canonicalization, `no-trap` inference) are demand-driven infrastructure, not levels.
  Register allocation and base instruction selection are likewise infrastructure (run
  at every level; `optimizations.md` marks them `—`), gating only their refinements.
- **Where the gate lives (resolved).** Each pass self-guards at its function entry
  (one `level_enabled(1)` check per function), covering all call sites, rather than
  wrapping each of the 7 call sites or introducing a central dispatcher. Minimal
  churn, and the guard travels with the pass when it moves. A central catalog-driven
  dispatcher is a later refactor once there are many rows.
- **SSA infra timing.** The reserved Opt2 seam is a single identity pass-through;
  Plan2(CFG+SSA)/Out-of-SSA are deferred to the first real Opt2 pass. Build+destruct
  of SSA with zero consumers is pure risk against the default byte-identity gate.
- **Flag threading style.** Following `--regalloc`, the passes/seams read the
  process-wide `OnceLock` (`active_opt_level()`) rather than threading `OptLevel`
  through `lower_project`. Consistent with the existing backend, at the cost of a
  global.
- **Module location (resolved 2026-08-23).** The scaffold lives in a *new top-level*
  `src/optimizer/` (`opt1/`, `opt2/`, `mod.rs`), per request. The gated passes
  (`fma_fusion`, `peephole`) move here; `selfmove_probe` (a diagnostic) stays in
  `src/codegen/compiler/opt/`, which after the move hosts only that probe.
- **Peephole placement (resolved: stays post-regalloc).** `forward_stores_to_loads`
  and `remove_fp_shuttles` operate on physical registers, so they remain at their
  post-regalloc call sites and are *not* moved into the reserved between-select Opt2
  seam. `optimizations.md` classifies them as post-regalloc machine peepholes; the
  gate is orthogonal to where they run.
