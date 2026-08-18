<!-- Feature plan. plan-100: put the opt-level pipeline + two no-op pass seams in place. -->

# Optimizer pipeline scaffold — `-O0`/`-O1` gate + two no-op pass seams

Last updated: 2026-08-16
Effort: medium (half-day)

Put the optimizer *pipeline* in place without writing a single optimization. Add
an opt-level flag (`-O0` default, `-O1`) and two **gated pass seams** that today
are no-ops (identity passes). The single behavioral outcome a correct
implementation produces: `mfb build` and `mfb test` accept `-O0`/`-O1`/`-O`
(and the `--optimize=` forms); at `-O0` (the default) the emitted machine code is
**byte-identical to today's**; at `-O1` the two seams run but, being no-ops today,
also emit byte-identical code — so both levels are provably neutral until real
passes land. Actual optimizations are added later, one pass at a time, from the
catalog in `planning/optimizations.md`.

Target pipeline (this plan builds the **bold** parts as seams only):

```
AST → IR → NIR → gated[Opt1(NIR)] → Plan1(storage/StorageType/symbols) → MIR
    → gated[ Plan2(CFG + SSA/def-use) → Opt2(MIR) → Out-of-SSA(MIR) ] → regalloc → machine code
```

- **Opt1(NIR)** — a `NirModule → NirModule` transform seam. No-op today.
- **Opt2 bracket** — a `Vec<CodeInstruction> → Vec<CodeInstruction>` transform seam
  sitting between instruction selection and register allocation. No-op today.
  Plan2 (CFG + SSA/def-use construction), Out-of-SSA (phi elimination), and Plan2's
  demand-driven analyses (SSA/mem2reg, alias, memory-SSA, range/trap, loop
  canonicalization, and function-attribute/`no-trap` inference) are the *interior*
  of this bracket; they are **not built in this plan** — they get built when the
  first real Opt2 pass needs them (see Non-goals / Open Decisions). Today the whole
  bracket is one identity function.
- **Gating is per-row, not per-seam.** Each seam runs its catalog rows filtered by
  `row.level <= active_opt_level()`, so one `-ON` lights up rows in *both* seams
  (level ≠ stage). The dial is `-O0..-O5` (escalating shape distortion at *preserved*
  behavior); **Level 6** (`-O6`) is an orthogonal, explicit opt-in for
  semantic-relaxing passes (fast-math, the trap-order-affecting † rows) and is
  *never* implied by the dial, not even at `-O5`/"max". This scaffold has zero rows
  yet, so both seams are identity at every level — but the seam contract already
  takes the level so rows drop in without reshaping it. (This supersedes an earlier
  binary `O0 => skip / O1 => run` bracket shape.)

References:

- `planning/optimizations.md` — the pass catalog mapped to Opt1/Opt2 stages (what
  gets added later; this plan adds none of them).
- `--regalloc` flag plumbing, mirrored exactly for `-O`:
  - CLI parse: `src/cli/build/options.rs:8` (`parse_common_option`), `:41`
    (`parse_build_options`), `:119` (`parse_test_options`).
  - Options struct: `src/cli/build/mod.rs:92` (`BuildOptions`), field at `:114`.
  - Hand-off to backend global: `src/cli/build/mod.rs:180`
    (`regalloc::set_strategy(options.regalloc)`).
  - Global module pattern: `src/target/shared/code/regalloc/mod.rs:62` (`RegallocKind`),
    `:86` (`parse_kind`), `:97` (`SELECTED: OnceLock`), `:101` (`set_strategy`),
    `:108` (`active_kind`).
  - Parser parity tests: `src/cli/build/mod.rs:993-1001,1075`.
  - Package-path defaults that also set regalloc: `src/cli/pkg.rs:127,420`.
- Opt1 seam site: `src/target/shared/lower.rs:8` (`lower_project`), wrap the
  `nir::lower_module(...)` result at `:21`. Covers all four targets at once
  (consumed at `macos_aarch64/mod.rs:322`, `linux_aarch64/mod.rs:293`,
  `linux_x86_64/mod.rs:306`, `win_x86_64/mod.rs:417`).
- Opt2 seam site: `src/target/shared/code/builder_registers.rs:104`
  (`run_register_allocation`); insert between selection (`:145`,
  `self.instructions = backend.select(neutral)`) and regalloc (`:149`,
  `regalloc::allocate(...)`).
- Golden harness: `scripts/test-accept.sh:385` (the `mfb build` invocation),
  `:109-112` (`MFB_TARGET` global-switch precedent), `scripts/sync-goldens.sh`.
- `.ai/testing-gates.md` (byte-identity, acceptance golden harness),
  `.ai/build-tooling.md` (rustfmt/clippy policy), `.ai/compiler.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| `--regalloc` is threaded via `BuildOptions` field + a `set_*`/`active_*` `OnceLock` global | `rg -n 'regalloc::set_strategy\|active_kind\|struct BuildOptions' src/cli src/target/shared/code/regalloc/mod.rs` | MET (see References) |
| The Opt1 seam site returns the sole `NirModule` for every target | `rg -n 'lower_project' src/target` | MET (single producer in `shared/lower.rs`) |
| The Opt2 seam site sees the finalized pre-regalloc stream for every function | read `builder_registers.rs:104-157` | MET (single `run_register_allocation`) |
| The default opt level maps to today's exact codegen | Phase 3 `-O0` golden run is a clean diff | UNMEASURED — Phase 3 gate |

Everything below assumes the seams are **identity** today. The whole value of this
plan is that turning the gate on changes nothing yet — so `-O0` and `-O1` must
both diff clean against current goldens before any real pass is written.

## 1. Goal

- `mfb build` and `mfb test` accept `-O0`, `-O1`, `-O 0`/`-O 1`, `--optimize=0/1`
  (and the `-optimize` single-dash forms), mirroring `--regalloc` exactly.
  Unknown levels error the same way `--regalloc bogus` does.
- Default is `-O0`. Absent the flag, behavior is unchanged.
- An `OptLevel` (numeric `0..=6`) rides `BuildOptions` and is published to a
  process-wide `OnceLock` (`set_opt_level` / `active_opt_level`), same shape as
  `RegallocKind`. Levels `0..5` are the cumulative dial; `6` is the orthogonal
  semantic-relaxation opt-in (never reached by escalating the dial). The scaffold's
  parser accepts only `0` and `1` for now (Non-goals), but the type spans the full
  range so later passes slot in without a type change.
- `optimize_nir(module, level)` exists as the **Opt1** seam and is called in
  `lower_project`. At `O0` it is skipped; at any level ≥ 1 it runs but, with no rows
  yet, returns the module unchanged.
- `optimize_mir(instructions, level)` exists as the **Opt2** seam and is called in
  `run_register_allocation` between selection and regalloc. At `O0` it is skipped;
  at any level ≥ 1 it runs but, with no rows yet, returns the stream unchanged.
- `-O0` machine code is byte-identical to today's goldens. `-O1` machine code is
  *also* byte-identical today (no-op passes), proving the gate itself is neutral.
- The harness can build/verify goldens at a chosen opt level via a global switch
  (`MFB_OPT`), mirroring `MFB_TARGET`.

### Non-goals (explicit constraints)

- **No optimization is implemented.** Both seams are identity functions. The
  catalog in `optimizations.md` is future work, added one pass at a time.
- **No SSA / CFG / analysis infrastructure is built.** Plan2 (CFG + SSA/def-use
  construction), Out-of-SSA (phi elimination), and Plan2's **demand-driven
  prerequisites** — SSA promotion (mem2reg), alias analysis, memory-SSA/memory-
  dependence, range/trap analysis, loop canonicalization, and function-attribute
  (`no-trap`) inference (all kept out of `optimizations.md`, which lists only real
  passes) — are documented as the *interior* of the Opt2 bracket but are **not**
  implemented here.
  Building and destructing SSA with zero consumers is pure cost and risk. The Opt2
  seam is a single pass-through over the existing `Vec<CodeInstruction>`; these
  arrive with the first Opt2 pass that needs them. (See Open Decisions and §5.)
- **No `-O2`–`-O6` accepted yet.** The `OptLevel` type spans `0..=6` (dial `0–5` plus
  the `6` semantic-relaxation opt-in), but the parser accepts only `0` and `1` for
  now — enough to prove the gate is neutral. Higher dial levels are enabled by later
  plans as rows land; Level 6 additionally requires an explicit request and is never
  implied by "max".
- **No size objective (`-Os`/`-Oz`).** Size is a *second, orthogonal* axis (it
  re-weights pass profitability, not risk), composed with a numeric level — out of
  scope for the scaffold, a later flag. `OptLevel` stays a pure risk dial here.
- **No change to the default codegen path.** `-O0` must be indistinguishable from
  today at the byte level; if any golden moves at `-O0`, that is a bug in this
  plan, not a re-baseline.
- **No new per-fixture flag file.** Opt-level goldens use a global `MFB_OPT`
  switch, matching the existing `MFB_TARGET` precedent; a per-fixture `build.args`
  seam is explicitly out of scope.

## 2. Phase 1 — the `-O`/`--optimize` flag + `OptLevel` global

Mirror `RegallocKind` end to end.

- [ ] New module `src/target/shared/opt/mod.rs` (or alongside regalloc):
      `OptLevel` as a numeric level `0..=6` (`#[default]` = 0), `parse_kind(&str)`
      (accepting only `0`/`1` for now — Non-goals), `available_levels()`, `static
      SELECTED: OnceLock<OptLevel>`, `set_opt_level`, `active_opt_level()` → defaults 0,
      plus a `level_enabled(row_level) -> bool` helper (`row_level <= active_opt_level()`)
      for the per-row seam filter. Copy the `regalloc/mod.rs:62-108` shape.
- [ ] `src/cli/build/options.rs`: add an `opt: &mut OptLevel` out-param to
      `parse_common_option` (or a sibling), handling `-O`/`--optimize`/`-optimize`
      in both space and `=` forms (lines 25-34 pattern). Default from
      `opt::active_opt_level()` in `parse_build_options` (`:49` pattern) and
      `parse_test_options` (`:122` pattern); store into the struct at `:109`/`:148`.
- [ ] `src/cli/build/mod.rs:114`: add `pub(crate) opt: OptLevel` to `BuildOptions`.
      `src/cli/pkg.rs:127,420`: default `opt: opt::active_opt_level()`.
- [ ] `src/cli/build/mod.rs:180`: add `opt::set_opt_level(options.opt);` next to the
      `regalloc::set_strategy` call in `build_project`.
- [ ] Parser parity unit tests mirroring `src/cli/build/mod.rs:993-1001,1075`:
      `-O1` == `--optimize=1` == `-O 1`; `-O0` default; bogus level errors.
- Commit: `plan-100: add -O/--optimize opt-level flag (O0 default) mirroring --regalloc`

## 3. Phase 2 — the two no-op pass seams

- [ ] **Opt1 seam.** Add `pub(crate) fn optimize_nir(module: NirModule, level: OptLevel) -> NirModule`
      (new file, e.g. `src/target/shared/opt/nir_passes.rs`). Today: `match level {
      O0 => module, O1 => module }` — identity, with a doc comment listing the
      Opt1 catalog rows (`optimizations.md`) as future contents. Call it in
      `src/target/shared/lower.rs:21`, wrapping the `nir::lower_module(...)?` result.
- [ ] **Opt2 seam.** Add `pub(super) fn optimize_mir(instructions: Vec<CodeInstruction>, level: OptLevel) -> Vec<CodeInstruction>`
      (e.g. `src/target/shared/code/opt_mir.rs`). Today: identity. Call it in
      `src/target/shared/code/builder_registers.rs` between selection (`:145`) and
      `regalloc::allocate` (`:149`), reading `opt::active_opt_level()` (the global,
      matching how `regalloc_kind` is read). Doc comment marks this as the future
      home of Plan2(CFG + SSA/def-use + demand-driven mem2reg/alias/memory-SSA/
      range-trap/loop-canonicalization/`no-trap`-inference analyses) → Opt2 passes
      → Out-of-SSA; note that when the first real pass lands, SSA
      construction/destruction and those analyses get built *inside* this bracket.
- [ ] Unit test: `optimize_nir`/`optimize_mir` at both levels return structurally
      equal output to their input (identity invariant), so a future pass that
      accidentally fires at `O0` is caught.
- Commit: `plan-100: add no-op Opt1(NIR) and Opt2(MIR) gated pass seams`

## 4. Phase 3 — harness opt-level switch + neutrality proof

- [ ] `scripts/test-accept.sh`: add an `MFB_OPT` global switch mirroring
      `MFB_TARGET` (`:109-112`): when set, append `-O$MFB_OPT` to the `mfb build`
      invocation at `:385`. Default (unset) = `-O0` = today's exact command, so the
      existing golden run is untouched.
- [ ] **Neutrality gate (the whole point):** run the acceptance suite three ways and
      require a clean diff against current goldens for all three:
      1. default (no `MFB_OPT`) — proves nothing moved.
      2. `MFB_OPT=0` — proves explicit `-O0` == default.
      3. `MFB_OPT=1` — proves the no-op `-O1` path emits byte-identical code.
      Any diff here is a bug in the seam/gate, not a re-baseline — fix it, do not
      touch goldens (`AGENTS.md`).
- [ ] Full `cargo test` green (parser parity + identity-invariant tests).
- [ ] `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.
- Commit: `plan-100: MFB_OPT harness switch; prove -O0/-O1 byte-identical to baseline`

## 5. Follow-on (out of scope — one pass at a time, later plans)

Once the scaffold lands, each real optimization is its own small plan: it fills in
`optimize_nir`/`optimize_mir` for a single catalog row (tagged with its scale
level), gates it by `row.level <= active_opt_level()`, gets its **own** golden set
at the level that first enables it (goldens may move *only* at that level and up,
never at `-O0`), and adds a RED test proving the transform fires. The first Opt2
pass that needs dataflow is also the plan that builds **Plan2** — the persistent
CFG + SSA/def-use (promoting the throwaway `build_cfg` in `regalloc/analysis.rs`),
its **demand-driven analyses** (mem2reg/SSA promotion, alias analysis, memory-SSA,
range/trap analysis, loop canonicalization, and function-attribute/`no-trap`
inference — the prerequisites listed in `optimizations.md`), and **Out-of-SSA**
(phi elimination before regalloc). **Level 6** (fast-math + the trap-order-relaxing
† passes) is a later, orthogonal opt-in with its own explicitly-requested golden
set; it is never enabled by escalating the numeric dial. Loop unrolling (an Opt1
row, level 5) is a natural first *Opt1* pass because it needs no CFG/SSA — loops are
still structured `NirOp::For`/`While` nodes at the Opt1 seam; a behavior-preserving
check-elision pass (broadening plan-39/plan-86) is a natural first *Opt2* pass and
the one that first justifies Plan2's range/trap analysis.

## Open Decisions

- **Gating model (resolved).** Per-row `row.level <= active_opt_level()` inside each
  seam, *not* a binary bracket on/off — so one level lights up rows across both Opt1
  and Opt2 (level ≠ stage), and **Level 6** is an orthogonal opt-in never implied by
  the dial. The scaffold's seams are level-aware no-ops; no row exists yet, so every
  level is identity. The Plan2 prerequisites (mem2reg/SSA, alias, memory-SSA,
  range/trap, loop canonicalization, `no-trap` function-attribute inference) are
  demand-driven infrastructure, not levels — they build when an enabled pass needs
  them. Register allocation and base instruction selection are likewise infrastructure
  (run at every level; `optimizations.md` marks them `—`), gating only their
  refinements. (Supersedes the original binary-bracket framing.)
- **SSA infra timing.** This plan makes the Opt2 bracket a single identity
  pass-through and defers Plan2(CFG+SSA)/Out-of-SSA to the first real Opt2 pass.
  Rationale: build+destruct of SSA with zero consumers is pure risk against the
  byte-identity gate. If instead we want the SSA *round-trip* stubbed now (build
  SSA → no passes → destruct), that is a larger, riskier Phase and must prove its
  own `-O1` byte-identity — flag it and we re-scope. Default taken: defer.
- **Flag threading style.** Following `--regalloc`, the seams read the process-wide
  `OnceLock` (`active_opt_level()`) rather than threading `OptLevel` through
  `lower_project` → `lower_module_for_platform`. Consistent with the existing
  backend, at the cost of a global. If explicit threading is preferred, the
  parameter chain exists — but that diverges from the established pattern.
