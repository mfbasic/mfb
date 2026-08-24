## Optimizations

Here's the updated table with a **Stage** column mapped to your pipeline. Key reasoning: **Opt1 (NIR)** = high-level, language-aware, works without CFG/SSA (tree/linear rewrites, inlining, loop restructuring on structured IR). **Opt2 (MIR)** = anything needing CFG, SSA, def-use, or dataflow analysis. Some things live outside both gates (regalloc, machine-code emission).

The **Ok** column answers "would this work for *this* compiler, given the plan-100 pipeline?" — **Y** = applicable and hostable in the pipeline (even if it needs net-new infra like a machine model or a vectorizer); **N** = fundamentally inapplicable because MFB lacks the property it exploits. The N rows are inapplicable for a concrete reason — a *missing feature*, not a weakness: no undefined behavior (UB-based), no null model (null-check), no emitted zero-extensions to remove (AArch64 subreg-zext), no hardening to omit (stack-protector), no reference counting (RC-op elimination), no tracing GC / write barriers (write-barrier elimination), and no async/coroutines (state-machine optimization). Read the **Feasibility notes** under the table — several Y rows carry a hard MFB-specific constraint (checked-overflow trapping) that limits *how* they may fire. A "—" in the Level column marks **infrastructure that runs at every level** (register allocation, instruction selection), not a dial pass.

## The Scale

Here's a 1–5 **risk/safety scale** for your compiler, where the number reflects how much an optimization can distort program behavior, introduce bugs, or destroy debuggability — not how expensive it is to run.

### Level 1 — Transparent
> *"The code is identical in every observable way, just less wasteful."*

- **Side effects:** None possible. The transformation is a pure identity rewrite — same values, same operations executed, same order of observable events.
- **Error risk:** Essentially zero; correctness is provable locally without any program-wide analysis.
- **Debuggability:** Fully preserved. Source-to-machine mapping intact, every variable inspectable, breakpoints land where expected.
- **Character:** Local, single-expression or single-instruction rewrites needing no analysis infrastructure.
- **Examples:** constant folding, algebraic simplification (`x+0` → `x`), division-by-constant lowering, jump table generation.

### Level 2 — Tidying
> *"Code is removed or merged, but everything that runs behaves identically."*

- **Side effects:** None on program output, but code/data *disappears* — unused stores, unreachable blocks, duplicate constants.
- **Error risk:** Very low; requires only simple, well-understood analysis (liveness, reachability within a function).
- **Debuggability:** Mildly degraded. Some variables show as "optimized out," some source lines have no corresponding instructions, but stepping still mostly makes sense.
- **Character:** Removal and deduplication based on straightforward local/intraprocedural facts.
- **Examples:** dead-code elimination, dead-store elimination, unreachable code elimination, basic block merging, copy propagation, constant/string merging.

### Level 3 — Restructuring
> *"Computations are moved, shared, and reordered — the result is the same, but the shape of execution is not."*

- **Side effects:** Execution order changes; expressions compute at different times/places than written. Observable behavior preserved *only if the analyses are correct*.
- **Error risk:** Moderate. Correctness depends on nontrivial infrastructure — SSA, dominance, alias analysis, loop analysis. Bugs here are subtle miscompiles, not crashes in the compiler.
- **Debuggability:** Significantly degraded. Stepping jumps around, values exist before their source line, variables merge or vanish. Stack traces still truthful.
- **Character:** Dataflow-driven code motion and redundancy elimination within a function.
- **Examples:** GVN/CSE, LICM (hoisting only trap-free/proven ops), SCCP, PRE, store-to-load forwarding, loop rotation/unswitching, if-conversion.

### Level 4 — Boundary-crossing
> *"Function and data boundaries the programmer wrote no longer exist."*

- **Side effects:** Call structure, stack frames, signatures, and data layout are rewritten. Program semantics preserved, but the runtime *structure* no longer matches the source.
- **Error risk:** High. Interprocedural analysis is where miscompiles hide — escape analysis mistakes, alias assumptions across calls, ABI subtleties. Bugs are hard to reproduce and localize.
- **Debuggability:** Severely degraded. Stack traces lie (inlined frames, eliminated tail calls), functions don't exist, struct layouts differ from declarations, breakpoints on a function may never fire.
- **Character:** Interprocedural and layout-changing transforms.
- **Examples:** inlining, tail-call optimization, function specialization/cloning, dead argument elimination, SROA, structure field reordering, heap-to-stack promotion, devirtualization, LTO.

### Level 5 — Aggressive (code-expanding / deepest analysis)
> *"Same observable results, but the code volume and analysis depth explode."*

- **Side effects:** Behavior is *preserved*, but code volume changes radically (unrolling, vectorization) and correctness now rides the deepest analyses (dependence / trip-count) where a bug silently corrupts data. Includes performance risk: can make code *slower*.
- **Error risk:** Very high — a dependence-analysis bug is a silent miscompile, not a crash.
- **Debuggability:** Effectively gone. One source line becomes 50 SIMD instructions; loops don't iterate the number of times written.
- **Character:** Massively code-expanding or deep-analysis transforms — but still **behavior-preserving**, so they stay on the numeric dial.
- **Examples:** aggressive unrolling, auto-vectorization, SLP, software pipelining, polyhedral transforms, auto-parallelization.

### Level 6 — Semantic-relaxing (opt-in, *off* the numeric dial)
> *"The compiler is allowed to change what the program observably does."*

- **Side effects:** Observable behavior can change — a different floating-point result, or a trap (`ErrOverflow` / `ErrFloatNaN` / `ErrIndexOutOfRange`) that fires at a different time, on a different path, or not at all. Under MFB's checked-overflow, precise-trap semantics these are *behavior* changes, not risk gradations.
- **Error risk:** By definition the transform is *permitted* to produce a program that behaves differently from the source. "Correct" here means "correct under the relaxed semantics the user explicitly accepted."
- **Debuggability:** N/A — the observable program itself differs.
- **Character:** Semantic relaxation. **Never implied by the numeric dial, not even at its maximum (`-O5`).** Requesting maximum *performance* must not silently opt you into different *results*; Level 6 is reached only by explicitly asking for it (`-O6`). This is why fast-math and the trap-order-affecting (†) passes were pulled off the 1–5 dial into their own tier rather than exposed as orthogonal `--fast-math` / `--relax-trap-order` flags.
- **Examples:** fast-math / FP reassociation, integer reassociation & expression-tree balancing (change which op overflows), loop strength reduction (the tail add can trap where the source never multiplied), loop deletion of a trap-capable loop, speculative hoisting of a trapping op. (UB-based optimization would live here too, but MFB has no UB to exploit — see the table.)

> **Note on the "safe form" of † passes.** Each † pass has a proof-gated variant that fires only when range/trap analysis proves trap-freedom — that variant *is* behavior-preserving and may run on the numeric dial (e.g. LICM at L3 hoisting only trap-free ops, or deleting a proven-trap-free loop at L2). Level 6 is specifically the *unproven / relaxing* form that fires anyway. Enabling Level 6 demand-drives the range/trap analysis (a Plan2 prerequisite — see plan-100) so the compiler minimizes how much it actually relaxes.
>
> **The dial-safe alternative: loop versioning.** For the loop-based † passes there is a third option that keeps precise semantics *and* stays on the numeric dial — emit two loop copies under a runtime "no-trap-possible in this range" guard and run the relaxed/vectorized form only in the proven copy (see the "Loop versioning" row). It is behavior-preserving by construction, so it is MFB's principal route to the Level-6 payoff without the Level-6 opt-in; the cost is code size + the guard, not correctness.

## Summary table

| Level | Name | Side effects | Miscompile risk | Debuggability | Analysis needed |
|---|---|---|---|---|---|
| 1 | Transparent | none | ~zero | perfect | none |
| 2 | Tidying | code disappears | very low | good | trivial (liveness) |
| 3 | Restructuring | execution reordered | moderate | poor | SSA, dominance, alias |
| 4 | Boundary-crossing | structure rewritten | high | very poor | interprocedural |
| 5 | Aggressive | code volume explodes (results same) | very high | none | deepest (dependence / trip-count) |
| 6 | Semantic-relaxing (opt-in) | **behavior may change** | n/a — user-blessed | n/a | trap/FP proof, to *minimize* relaxation |

Levels **1–5 are the numeric performance dial** — enabled cumulatively by `row.level <= active_opt_level()`. **Level 6 is orthogonal**: never implied by the dial (not even `-O5`/"max"), reached only by explicitly requesting it.

## Design notes for your compiler

- **Level ≠ stage; gate per row, not per seam.** Each seam runs `rows.filter(r => r.level <= active_opt_level() && enabled(r))`. A single `-ON` lights up rows in *both* the Opt1 and Opt2 seams — a seam is never simply "on/off." (This supersedes plan-100's original binary `O0 => skip bracket` / `O1 => run bracket` shape.)
- **Cumulative dial:** `-O3` enables levels 1–3. `-O5` is the maximum *behavior-preserving* level.
- **The default is `-O1`, not `-O0` (plan-100).** MFB's shipping codegen already runs two Level-1 passes (the post-regalloc machine peepholes), so plan-100 sets the dial's default to `-O1` — keeping the default byte-identical to today — and makes **`-O0` the explicit "all dial passes off"** baseline (a correct-but-unoptimized path, not a byte-identity target). Those two passes are the **first landed rows** and are gated by `level_enabled(1)`; see the annotated rows below. Both are strictly behavior-preserving, so **`-O0` produces identical program behavior** — only slower/larger code. FMA contraction was evaluated for the dial and deliberately left OFF it: it changes float results, so it is mandatory lowering, not an optimization (see the "Instruction selection / combining" row).
- **Levels 1–2 are your "always on" candidates** — even a debug build could run them with little downside. (Level 1 *is* the default-on set today.)
- **Level 4 is the debuggability cliff** — stack traces stop being truthful; a natural default ceiling for development builds, with 4–5 reserved for release.
- **Level 6 is orthogonal and opt-in** — *never* implied by `-O5` or an "auto/max" setting; the user must name it (`-O6`). Cranking the dial for speed must never silently change results.
- **Prerequisites are not dial rows.** SSA construction (mem2reg), alias analysis, range/trap analysis, memory-SSA, loop canonicalization, and function-attribute (`no-trap`) inference are Plan2 infrastructure, built *on demand* when an enabled pass needs them (LICM needs trap-analysis to hoist safely; alias-based passes need alias/memory-SSA; L6 rows need range analysis to minimize relaxation). They live in plan-100's Plan2, not in this table.
- **Size (`-Os`/`-Oz`) is a second, orthogonal objective — not a dial level.** It doesn't fit the 1–6 risk axis; it re-weights *profitability* (inlining/unrolling budgets shrink, outlining/error-path-dedup/shared-trap-stubs become more profitable, layout favors density). Model it like a target flag that shifts each pass's cost model, composed with a numeric level (e.g. `-Oz` + level-3 safety). Net-new — MFB has no `-O*`/size axis today.


| Ok  | Name | Level | Stage | Description |
| --- |---|---|---|---|
| Y   | Constant folding | 1 | Opt1 + Opt2 | Evaluate constant expressions at compile time; rerun in Opt2 as other passes expose constants. |
| Y   | Constant propagation | 2 | Opt2 | Replace variables known to hold constants; needs def-use/SSA to do properly. |
| Y   | Copy propagation | 2 | Opt2 | Replace uses of a variable with its source copy; trivial on SSA. |
| Y   | Dead-code elimination (DCE) | 2 | Opt1 + Opt2 | Remove unused code; simple tree-level DCE in Opt1, precise SSA-based DCE in Opt2. |
| Y   | Dead-store elimination | 2 | Opt2 | Remove stores overwritten before read; needs dataflow/alias info. |
| Y   | Unreachable code elimination | 2 | Opt1 + Opt2 | Prune statically-dead branches in Opt1; prune unreachable CFG blocks in Opt2. |
| Y   | Algebraic simplification | 1 | Opt1 | Apply identities (`x*1` → `x`); pure local rewrite, no analysis needed. |
| Y   | Strength reduction (non-loop) | 1 | Opt1 | Replace expensive ops with cheaper ones (`x*2` → `x<<1`); local rewrite. |
| Y   | Peephole optimization | 1 | Opt2 / post-regalloc | Local pattern rewrites; MIR peepholes in Opt2, machine peepholes after regalloc. **LANDED (plan-100): the post-regalloc machine peepholes `forward_stores_to_loads` + `remove_fp_shuttles` are gated Level-1 rows in `src/optimizer/opt2/` — on at `-O1`, off at `-O0`.** |
| Y   | Branch simplification / folding | 2 | Opt1 + Opt2 | Fold known conditions; structured version in Opt1, CFG version in Opt2. |
| Y   | Jump threading | 3 | Opt2 | Redirect jump-to-jump chains; requires CFG. |
| Y   | Basic block merging | 2 | Opt2 | Merge single-pred/single-succ blocks; requires CFG. |
| Y   | Tail-call optimization | 4 | Opt2 | Convert tail calls to jumps; needs call-position analysis on CFG, affects frame layout before regalloc. |
| Y   | Common subexpression elimination (CSE) | 3 | Opt2 | Reuse repeated computations; subsumed by GVN on SSA. |
| Y   | Local value numbering | 3 | Opt2 | Block-local redundancy elimination on MIR. |
| Y†  | Reassociation | 6 | Opt1 + Opt2 | Reorder associative ops; canonical form + ILP. **†off the dial: changes which int op overflows and changes FP results — Level 6 opt-in** |
| Y   | Global value numbering (GVN) | 3 | Opt2 | Whole-function redundancy elimination; requires SSA. |
| Y   | Sparse conditional constant propagation (SCCP) | 3 | Opt2 | Combined constant prop + unreachable pruning; classic SSA algorithm. |
| Y   | Loop-invariant code motion (LICM) | 3 | Opt2 | Hoist invariant code from loops; needs CFG loop analysis + SSA. (A simple structured-loop version can also live in Opt1.) **at L3 hoists only trap-free/proven ops; unconditional hoist of a trapping op is Level 6** |
| Y   | Induction variable simplification | 3 | Opt2 | Canonicalize loop counters; needs SSA loop analysis. |
| Y†  | Loop strength reduction | 6 | Opt2 | Replace induction-variable multiplies with adds; needs IV analysis. **†off the dial: the tail add can trap where the source never multiplied — Level 6 opt-in** |
| Y   | Loop unswitching | 3 | Opt2 | Hoist invariant conditionals out of loops; duplicates CFG regions. |
| Y   | Loop rotation | 3 | Opt2 | Convert to do-while form; CFG transformation. |
| Y†  | Loop deletion | 6 | Opt2 | Remove side-effect-free loops; needs SSA use analysis. **†off the dial: a loop that can trap is not side-effect-free — deleting it drops an observable raise; Level 6 opt-in (a proven-trap-free loop is safe to delete at L2)** |
| Y   | Loop fusion (jamming) | 3 | Opt1 | Merge adjacent same-bound loops; far easier on structured NIR loops than on CFG. |
| Y   | Loop fission (distribution) | 3 | Opt1 | Split loops for locality/vectorization; easiest on structured loops. |
| Y   | Loop interchange | 5 | Opt1 | Swap nested loop order; needs structured loop nests + dependence analysis. **Reclassified 3→5: relies on array dependence analysis (the deep-analysis-silent-corrupt risk that defines L5), even though results are preserved.** |
| Y   | Loop tiling / blocking | 5 | Opt1 | Block loops for cache; structured-loop transformation. **Reclassified 3→5: dependence-analysis-driven (see interchange).** |
| Y   | Loop unrolling | 5 | Opt1 or Opt2 | Replicate loop bodies; simple full-unroll in Opt1, runtime/partial unroll with trip-count analysis in Opt2. |
| Y   | Loop peeling | 3 | Opt2 | Split off first/last iterations; usually paired with Opt2 loop analyses. |
| Y   | Loop skewing | 5 | Opt1 | Shift iteration space; structured/polyhedral-level transform. **Reclassified 3→5: dependence-analysis-driven (see interchange).** |
| Y‡  | Software pipelining | 5 | Opt2 (late) | Overlap iterations; needs scheduling info, runs near end of Opt2. **‡net-new machine model** |
| Y   | Function inlining | 4 | Opt1 | Substitute callee bodies; do it on NIR before Plan1 so storage slots/symbols are computed once for the merged body. |
| Y   | Aggressive/heuristic inlining | 4 | Opt1 | Profile/cost-model-driven inlining; same stage, bigger budget. |
| Y   | Partial inlining | 4 | Opt1 | Inline hot early-exit portion; NIR-level function surgery. |
| Y   | Interprocedural constant propagation (IPCP) | 4 | Opt1 | Propagate constant args across functions; interprocedural work fits pre-Plan1. |
| Y   | Function specialization / cloning | 4 | Opt1 | Clone functions for known args; must precede Plan1 symbol/slot assignment. (Monomorphization already exists — reuse it.) |
| Y   | Argument promotion | 4 | Opt1 | Pass values instead of pointers; changes signatures → before Plan1. |
| Y   | Dead argument elimination | 4 | Opt1 | Remove unused params; changes signatures → before Plan1. |
| Y   | Return value propagation | 4 | Opt1 | Propagate known returns into callers; interprocedural, NIR-level. |
| Y   | Devirtualization | 4 | Opt1 | Resolve indirect calls to direct; enables Opt1 inlining. (Targets `FUNC` indirect calls.) |
| Y   | Escape analysis | 4 | Opt1 | Determine non-escaping objects; results feed Plan1 storage-slot decisions. (Resource-escape analysis already exists — `src/ir/resource_escape.rs`.) |
| Y   | Scalar replacement of aggregates (SROA) | 4 | Opt1 | Split structs into scalars; must run before Plan1 assigns StorageType/slots. |
| Y   | Store-to-load forwarding | 3 | Opt2 | Replace loads with prior stored values; needs alias analysis. (A block-local machine version already exists — `forward_stores_to_loads`, now `src/optimizer/opt2/peephole.rs`, gated as a **Level-1** machine peephole under the "Peephole optimization" row; the L3 entry here is the future alias-analysis-based broadening.) |
| Y   | Redundant load elimination | 3 | Opt2 | Remove loads of already-available values; SSA + alias analysis. |
| Y   | Memcpy/memset idiom recognition | 3 | Opt2 | Replace copy loops with bulk intrinsics; loop analysis on MIR. |
| Y   | If-conversion (predication) | 3 | Opt2 (late) | Convert branches to selects; CFG diamond pattern matching. (csel/cmov are emittable.) |
| Y   | Branch layout / prediction hints | 2 | Opt2 (post-SSA) / codegen | Order blocks for fall-through; late CFG layout, after out-of-SSA. |
| Y   | Code layout / block placement | 2 | codegen | Arrange hot code contiguously; machine-level. |
| Y   | Tail duplication | 3 | Opt2 | Duplicate small join blocks; CFG transform. |
| Y   | Jump table generation | 1 | Opt2 / codegen | Lower dense switches to tables; MIR lowering or backend. (Targets `MATCH`/union-tag dispatch.) |
| Y   | Switch lowering strategies | 1 | Opt2 / codegen | Binary search/bit tests for sparse switches; same as above. |
| Y‡  | Auto-vectorization (loop) | 5 | Opt2 | SIMD-ify loops; needs SSA, dependence, and trip-count analysis. **‡2-lane packed SIMD exists; wider = new encoders** |
| Y‡  | SLP vectorization | 5 | Opt2 | Pack straight-line scalar ops into SIMD; SSA-based. **‡2-lane packed SIMD exists; wider = new encoders** |
| Y‡  | Instruction scheduling | 3 | pre/post-regalloc | Reorder to hide latency; machine-level, around regalloc. **‡net-new machine/latency model** |
| Y   | Register allocation | — | regalloc | Assign virtual registers to physical; your dedicated stage. (Exists — linear-scan.) **Infrastructure, not a dial row: the allocator runs at every level; only its aggressiveness/refinements (coalescing, remat, live-range splitting, callee-save selection, spill-code opt) are level-gated.** |
| Y   | Register coalescing | 2 | regalloc | Eliminate copies via shared assignment; part of regalloc (interacts with out-of-SSA copies). (Planned — `allocator-20`.) |
| Y   | Rematerialization | 2 | regalloc | Recompute cheap values instead of spilling; regalloc component. |
| Y   | Stack slot coloring | 2 | regalloc | Reuse slots for non-overlapping lifetimes; regalloc/frame lowering. |
| Y   | Frame pointer omission | 2 | codegen | Free FP register; frame lowering. |
| Y   | Shrink wrapping | 2 | regalloc / codegen | Sink prologue/epilogue to needy paths; after regalloc knows clobbers. (Per-register callee-save placement is a finer-grained variant of the same pass.) |
| Y   | Instruction selection / combining | 1 | codegen | Fuse MIR ops into machine instructions (e.g., FMA); MIR→machine lowering. (FMA fusion + adrp/add + cmp/branch fusion already exist.) **Base selection is mandatory lowering (not level-gated); only the optional cost-based *combining* is the dial pass.** **LANDED (plan-100): `fuse_scalar_fma` is NOT a dial row — it stays mandatory, ungated, in `src/codegen/compiler/opt/fma_fusion.rs`, alongside adrp/add + cmp/branch fusion. Contraction rounds once instead of twice, so it changes *which float values exist*: with `a = 1.5e308`, unfused `a * 2.0` rounds to `+inf` and `LET r = a * 2.0 - a` traps `ErrFloatOverflow`, while the fused `fmsub` yields a finite `1.5e308`. Two fixtures pin that as a contract (`rt-behavior/arithmetic/float-fma-fusion` + `rt-error/arithmetic/arithmetic-float-fma-observed-rt`), so gating it would make `-O0` silently change results — exactly what this table reserves Level 6 for, and `-O0` is a *safety* request. The dial row for this entry remains the future cost-based combining.** |
| Y   | Addressing mode optimization | 1 | codegen | Fold address math into addressing modes; machine-specific. |
| Y   | Bit-tricks / idiom recognition | 1 | Opt2 | Recognize popcount/bswap/rotate patterns; SSA pattern matching. |
| Y   | Division-by-constant lowering | 1 | Opt2 / codegen | Div → multiply-shift; MIR lowering or instruction selection. |
| Y   | Narrowing / bit-width reduction | 2 | Opt2 | Shrink op widths when high bits unused; SSA demanded-bits analysis. |
| Y   | Sign/zero extension elimination | 2 | Opt2 | Remove redundant extensions; SSA analysis. |
| Y   | Select/cmov formation | 3 | Opt2 (late) | Turn diamonds into selects; pairs with if-conversion. |
| Y   | Dead global elimination | 2 | Opt1 | Remove unused globals/functions; module-level, before Plan1 symbols. |
| Y   | Merge duplicate constants/strings | 2 | Plan1 / codegen | Deduplicate constant data; data layout concern. |
| Y   | Identical code folding | 4 | codegen / link | Merge identical function bodies; late, machine-code level. |
| Y   | Outlining | 4 | Opt2 (late) / codegen | Extract repeated sequences into functions; post-SSA or machine level. |
| Y   | Link-time optimization (LTO) | 4 | Opt1 (whole-module) | Cross-unit optimization; run the Opt1 interprocedural passes over merged NIR. (Packages already merge into one NIR module — this is naturally whole-program.) |
| Y‡  | Profile-guided optimization (PGO) | 4 | all stages | Profile data feeding Opt1 inlining, Opt2 unswitching, codegen layout. **‡profile-consumption is net-new; `--coverage` counter injection seeds the instrumentation side** |
| Y   | Hot/cold splitting | 3 | Opt2 (late) / codegen | Move cold paths away from hot code; late CFG/layout. |
| Y†  | Speculative hoisting | 6 | Opt2 | Execute instructions early; SSA + CFG dominance. **†off the dial: speculating a trapping op (overflow/index/div) above its guard changes behavior — Level 6 opt-in** |
| Y   | Partial redundancy elimination (PRE) | 3 | Opt2 | Make partially redundant expressions fully redundant; SSA-based (often merged into GVN). |
| Y   | Load/store hoisting and sinking | 3 | Opt2 | Move memory ops across branches; CFG + alias analysis. |
| Y   | Bounds-check elimination | 3 | Opt2 | Remove provably-safe bounds checks; range analysis on SSA. (Real MFB checks raising `ErrIndexOutOfRange`; embryonic BCE already exists — `func_get.rs:152`, plan-86.) |
| N   | Null-check elimination | 3 | Opt2 | Remove redundant null checks; dominance-based. **MFB has no null model — see note.** |
| Y   | Correlated value propagation | 3 | Opt2 | Refine values using branch conditions; SSA + dominance. |
| Y   | Fast-math transformations | 6 | Opt1 + Opt2 | Non-strict FP rewrites; algebraic ones in Opt1, contraction in Opt2/codegen. **Off the dial: MFB Float is strict/finiteness-trapped by default — Level 6 opt-in.** |
| N   | UB-based optimization | 6 | Opt2 | Exploit language UB rules (no signed overflow, etc.) in SSA simplification. **N/A: MFB has no undefined behavior; would be Level 6 if it did — see note.** |
| Y   | Alignment optimization | 2 | Plan1 / codegen | Align data (Plan1 slots) and loops/functions (codegen). |
| Y‡  | Prefetch insertion | 2 | Opt2 (late) | Insert prefetches for predictable streams; loop + stride analysis. **‡net-new prefetch encoders** |
| Y   | Structure field reordering / packing | 4 | Opt1 (pre-Plan1) | Reorder fields for padding/locality; must precede Plan1 layout decisions. (Layout is not source-observable — no raw pointers — so this is always safe.) |
| Y   | AoS → SoA transformation | 4 | Opt1 (pre-Plan1) | Data layout transform; must precede storage planning. |
| Y   | Heap-to-stack promotion | 4 | Opt1 | Convert non-escaping heap allocs to stack; feeds Plan1 slot assignment (uses escape analysis). |
| Y   | Allocation sinking / elision | 4 | Opt1 + Opt2 | Eliminate allocations entirely; discovery in Opt1, final scalar promotion via Plan2/Opt2. |
| Y   | Tail merging (cross jumping) | 3 | Opt2 (post-SSA) / codegen | Merge identical branch tails; late CFG. |
| Y†  | Expression tree balancing | 6 | Opt2 | Balance dependency chains for ILP; SSA reassociation variant. **†off the dial: same trap/FP-order change as reassociation — Level 6 opt-in** |
| Y   | Polyhedral loop optimization | 5 | Opt1 | Advanced loop nest restructuring; needs structured loops, way easier pre-CFG. (Research-grade effort.) |
| Y   | Automatic parallelization | 5 | Opt1 | Thread-parallelize loops; structured-loop + dependence analysis. (Threading runtime exists; research-grade effort.) |
| Y   | Superoptimization | 5 | codegen | Exhaustive search for optimal machine sequences; final code level. (Applicable to any backend; research-grade.) |
| Y   | Code sinking | 3 | Opt2 | Move a computation down into the branch that uses it, so cold paths don't pay; dual of hoisting. **†sinking a trapping op that removes a trap on the skip path is Level 6; the non-trapping/proven form is L3** |
| Y   | Aggressive DCE (ADCE) | 3 | Opt2 | Control-dependence-based DCE (assume dead, prove live); removes whole dead control structures plain DCE misses. **†removing a trap-capable region is Level 6; proof-gated form is L3** |
| Y   | Dead error-handler / fallible-branch elimination | 3 | Opt2 | When a fallible call is *proven* trap-free (range/trap analysis), its error branch and `RECOVER`/TRAP handling is dead. MFB-specific companion to check-elision and the union/error-tag analogue. Behavior-preserving. |
| Y   | CFG simplification (simplifycfg) | 2 | Opt2 | Umbrella cleanup: block merge + branch fold + trivial-phi fold + two-entry-phi→select + empty-block removal + hoist common code from both arms. The named "rerun between passes" step. |
| Y   | Loop idiom recognition (broad) | 3 | Opt2 | Beyond memcpy/memset: popcount loops, strlen-style scans, arithmetic-series closed forms (`sum += i` → formula). **†closed-form replacement of a checked sum changes overflow behavior — Level 6 unless proven/versioned** |
| Y   | Loop versioning | 3 | Opt2 | Emit two loop copies guarded by a runtime check (no-alias / in-range / trip-count); run the relaxed form only in the proven copy. **Behavior-preserving by construction — the mechanism that keeps vectorization and the † passes on the numeric dial instead of Level 6. Highest strategic value of the additions.** |
| Y†  | Guard / check hoisting & combining | 6 | Opt2 | Merge N per-iteration bounds/overflow checks into one dominating range check. **Off the dial by default: the combined check traps earlier, at a synthetic location, skipping loop-body effects — differs from MFB's per-expression source-stamped trap (§4.1 / §8.5a). Versioned (keep the strict copy) it drops to L3.** |
| Y   | Constant hoisting | 2 | Opt2 (late) / codegen | Materialize expensive constants (big immediates, address bases) once and reuse — matters on AArch64 (a 64-bit constant is up to 4 `mov`s). |
| Y   | Global localization / constification | 2 | Opt1 | A global written once at init → constant; a global used by one function → local. Module-level; pairs with dead-global elimination. |
| Y   | Return-slot / NRVO copy elision | 3 | Opt1 | Construct a value-semantic aggregate directly in the caller's return slot, eliding the copy. **Partially exists — `plan_returned_move` for `RETURN <owned-local>` (plan-25-C, `builder_exits.rs:258`); broadening beyond the direct-`RETURN`-local case is the net-new part.** |
| Y   | Live-range splitting | 2 | regalloc | Split a long live range so only part spills. The single highest-value linear-scan improvement; pairs with coalescing (`allocator-20`). |
| Y   | Machine copy propagation / redundant-move elimination | 1 | post-regalloc | Post-regalloc cleanup of moves coalescing/out-of-SSA left behind. Distinct from MIR-level copy propagation (`remove_fp_shuttles` peephole already does a slice of this). **LANDED (plan-100): `remove_fp_shuttles` is a gated Level-1 post-regalloc pass in `src/optimizer/opt2/` — on at `-O1`, off at `-O0`.** |
| Y   | Shared out-of-line trap stubs | 2 | codegen | Collapse each check's inline miss path (per-site park / build-Error-block sequence, ~40+ instrs — `emit_error_register_return`) into one `bl` to a shared per-error-kind stub. **Net-new: plan-16 already outlined the Error-*Result* assembly (`_mfb_make_error_result`), but each site still inlines the preamble/park/routing — highest value-per-effort given a check after nearly every checked op.** |
| Y   | Literal interning + read-only placement | 2 | Plan1 / codegen | Dedup identical string/array literals and place immutable data in read-only pages. Companion to constant merging. |
| N   | Subregister-zeroing zext elimination (AArch64) | 2 | codegen | Exploit `w`-write-zeroes-upper-32 to drop explicit zero-extends. **N/A: the backend emits no `uxtw`/mask zero-extends — Integer math runs in 64-bit `x`-regs and the few narrow ops already rely on implicit zeroing. (Redundant *sign*-ext `sxtw` removal in FFI paths is the existing ext-elim row, still Y.)** |
| N   | Stack-protector / hardening omission | 1 | codegen | Skip canary/CFI hardening on cold paths. **N/A: MFB emits no stack protector / canary / CFI / PAC / BTI today — nothing to omit. (The FFI OUT-buffer canary in `link_thunk.rs` is a correctness guard, not omittable.)** |
| Y   | Overflow-check elimination | 3 | Opt2 | Prove a checked integer op cannot overflow and drop its trap branch. **Exists (plan-39: `integer_sub_elidable`/`integer_add_elidable`, `builder_numeric.rs:750`); broaden with a real range analysis — likely MFB's single highest-value pass.** |
| Y   | Redundant union-tag / error-tag check elimination | 3 | Opt2 | Remove a `MATCH` discriminant or fallible-result test dominated by an equivalent test. The direct MFB analogue of null-check elimination; preserve source-stamped traps. |
| Y   | Division / modulo-check elimination | 3 | Opt2 | Prove divisor ≠ 0 and (signed) exclude MIN ÷ -1; drop the corresponding error paths (`emit_integer_division_overflow_check`). |
| Y   | Float finiteness-check elimination & coalescing | 3 | Opt2 / codegen | Remove redundant observation-boundary finiteness checks or combine them — sanctioned by MFB's *explicitly imprecise* Float contract (§4.1). Distinct from fast-math. |
| Y   | Check fusion with existing comparisons | 3 | Opt2 | Reuse a source/program comparison to discharge a bounds/overflow/divisor/tag/finiteness check instead of emitting another compare. |
| Y   | Range-check widening / narrowing | 3 | Opt2 | Derive one safe-range fact from another (prove `i`,`i+1`,`i+2` from one dominating condition) without moving the trap. |
| Y   | Checked-operation fusion | 2 | Opt2 / codegen | Fuse a checked op + a compatible comparison/check into one flag-producing instruction, keeping the exact failing expression's source stamp. (Extends the existing `mir.rs` cmp/branch fusion.) |
| Y   | Trap-aware path-sensitive DCE | 3 | Opt2 | Model traps as explicit observable exits so cleanup is aggressive without treating a trap-capable op as dead/pure. |
| Y   | Error construction sinking | 3 | Opt2 | Keep `Error`/source-metadata/recovery-routing construction on the failing edge only; don't prepare it on the success path. |
| Y   | Error-path deduplication | 2 | Opt2 / codegen | Merge equivalent cold error blocks, keeping per-site source identity as a compact site-ID argument to a shared stub. (Pairs with Shared trap stubs.) |
| Y   | Recovery-region simplification | 3 | Opt2 | Coalesce nested `RECOVER`/TRAP regions, drop handlers that can't receive an error, route to the nearest live handler. |
| Y   | Source-location metadata compression | 2 | Plan1 / codegen | Dedup source-location records; pass compact site IDs to shared trap stubs — cuts the cost of precise traps *without* relaxing them. |
| Y   | Ownership-path optimization | 3 | Opt2 | Merge success/error cleanup paths, remove duplicate moves/drops, prove when an owned value need not be parked across a fallible op. |
| Y   | Destructor / drop optimization | 3 | Opt2 | Remove redundant drops, merge cleanup blocks, shorten ownership lifetimes — preserving deterministic destruction + trap order. (Machinery exists: `builder_owned_cleanup.rs`.) |
| Y   | Memory lifetime shortening | 3 | Opt2 | Move last-use cleanup earlier to cut peak memory + register pressure. |
| N   | Reference-count operation elimination | 3 | Opt1 + Opt2 | Cancel retain/release pairs, batch RC traffic. **N/A: MFB has no reference counting — compile-time ownership + arena (spec `memory-semantics`).** |
| N   | Write-barrier elimination / coalescing | 3 | Opt2 | Remove/merge GC write barriers. **N/A: MFB has no tracing GC and emits no write barriers.** |
| Y   | Interprocedural / global DCE (call-graph) | 4 | Opt1 | Call-graph reachability elimination of functions, methods, package initializers, unreachable SCCs. Distinct from the L2 dead-global row. |
| Y   | Global value specialization / global constant propagation | 4 | Opt1 | Propagate immutable/init-only globals across functions and specialize users. Broader than localization/constification. |
| Y   | Function merging (IR-level) | 4 | Opt1 | Merge semantically equivalent functions (incl. bodies differing only by constants or arg permutation) before codegen. More powerful than machine ICF. |
| Y   | Call-site splitting | 4 | Opt1 / Opt2 | Duplicate a join around a call so each copy gets stronger facts — enables specialization / const-prop / devirt. |
| Y   | Indirect-call promotion | 4 | Opt1 | Turn a profiled likely target (`FUNC`/closure call) into a guarded direct call + fallback. Complements devirt + PGO. |
| Y   | Recursive inlining / recursion peeling | 5 | Opt1 | Inline/peel a bounded recursive layer to expose constants + base cases. Strict growth controls. |
| Y   | Hot/cold function outlining | 4 | Opt1 / Opt2 | Extract exceptional/rare paths (esp. bulky MFB trap/error construction) into cold functions. Broader than block placement + generic outlining. |
| Y   | Fallibility specialization | 4 | Opt1 | Clone a function into proven-infallible + general variants; the infallible clone uses a simpler return convention and omits error-tag handling. (MFB-specific.) |
| Y   | No-trap call specialization | 4 | Opt1 | Given inferred arg ranges, specialize a fallible callee into a `no-trap` version — unlocks LICM/DSE/tail-calls across it. |
| Y   | Result / tag representation specialization | 4 | Opt1 | Specialize the internal fallible-result ABI when only one error kind is possible or payloads permit a cheaper representation. |
| Y   | Closure optimization | 4 | Opt1 | Elide non-escaping closure envs, drop unused captures, convert to direct calls, specialize by captured constants. **Partly exists: no-capture closures collapse to a static `env=0` descriptor; non-escaping capturing closures are drop-eliminated (`builder_control.rs:285`, plan-77).** |
| N   | Coroutine / state-machine optimization | 4 | Opt1 | Dead-state removal, state merging, frame scalar-replacement. **N/A: MFB has no async/generators/resumable functions — concurrency is OS-thread workers only.** |
| Y   | Object / aggregate copy propagation | 3 | Opt1 / Opt2 | Forward whole value-semantic aggregates + eliminate redundant copies not caught by scalar copy-prop or NRVO. |
| Y   | Allocation combining | 4 | Opt1 / Opt2 | Merge several short-lived allocations into one region/object where lifetimes permit. Complements allocation elision/sinking. |
| Y   | Dead allocation elimination | 4 | Opt2 | Remove an allocation whose result + initialization are dead, even when generic DCE doesn't model allocation effects. |
| Y   | Partial dead-store elimination | 3 | Opt2 | Remove stores dead on *some* paths, often by sinking/splitting them. |
| Y   | Store PRE / Load PRE | 3 | Opt2 | Memory PRE — distinct scope from scalar PRE (alias + trap constraints differ). |
| Y   | Store merging | 2 | Opt2 / codegen | Combine neighboring stores into wider stores / memset-like ops. |
| Y   | Load widening | 2 | Opt2 / codegen | One wider load for adjacent fields when legal + safe at object/page boundaries. |
| Y   | Read-only memory inference | 2 | Opt1 | Infer immutable objects/regions and exploit that fact across calls. |
| Y   | Alias-scope specialization via versioning | 3 | Opt2 | No-alias guarded fast path + strict fallback — a use of the Loop-versioning machinery. |
| Y   | Aggregate load/store combining | 2 | Opt2 / codegen | Combine adjacent scalar accesses into pair/vector accesses (AArch64 `ldp`/`stp`) when alignment + trap behavior permit. |
| Y   | Stack allocation merging / frame compaction | 2 | regalloc / codegen | Coalesce fixed stack objects (incl. non-spill locals) with non-overlapping lifetimes. Broader than spill-slot coloring. |
| Y   | Prologue / epilogue optimization | 2 | regalloc / codegen | Fold stack adjustment, paired saves/restores, return forms, leaf-function cases. Related to shrink-wrapping, distinct. |
| Y   | Callee-save selection | 2 | regalloc | Cost-based caller-saved-spill vs callee-saved-occupancy choice. Important for linear scan + hot loops. |
| Y   | Spill-code optimization | 2 | regalloc / post-regalloc | Fold reloads/spills into memory operands, kill redundant reloads, place spills optimally. |
| Y   | Register-pressure-aware code motion | 3 | Opt2 / scheduling | Constrain LICM/sinking/CSE/scheduling by estimated pressure so "optimized" MIR doesn't over-spill. |
| Y   | Machine branch relaxation | 1 | codegen | Select short/long branches, insert veneers/islands when range overflows. **Backend facility (correctness-required) + net-new: today AArch64 *hard-errors* on out-of-range branches (`sizing.rs:86`, the >1 MiB failure) — this pass removes that failure class.** |
| Y   | Constant-island / literal-pool placement | 1 | codegen | Place + dedup literals within architectural reach; coordinate with layout + branch relaxation. Backend facility. |
| Y   | Machine block reordering + fall-through inversion | 2 | codegen | Invert conditions + reorder successors to maximize fall-through (concrete form of the branch-layout-hints row). |
| Y   | Machine tail-call formation | 4 | post-regalloc / codegen | Catch ABI-valid tail calls exposed only after lowering/frame layout/copy resolution. Complements MIR-level TCO. |
| Y   | Load/store pair formation (ldp/stp) | 1 | codegen | Form AArch64 `ldp`/`stp` for adjacent accesses + saves/restores. **Net-new: backend emits only single `ldr`/`str` today.** |
| Y   | Pre/post-index addressing formation | 1 | codegen | Fold pointer updates into AArch64 memory ops when ordering is preserved. **Net-new: no writeback addressing today.** |
| Y   | Compare elimination / flag reuse | 1 | Opt2 / codegen | Reuse NZCV from arithmetic/prior compares; avoid repeated `cmp`/`test`. **Partly exists: `mir.rs` fuses cmp+branch, but general flag reuse is net-new.** |
| Y   | Boolean materialization elimination | 1 | Opt2 / codegen | Feed flags straight into branches/selects instead of producing + retesting a Boolean register. |
| Y   | Known-bits simplification | 2 | Opt2 | Track known-0/1 bits to simplify masks/shifts/compares/extensions/alignment checks. Broader than demanded-bits narrowing. |
| Y   | Bit-field extract/insert recognition | 1 | Opt2 / codegen | Recognize mask/shift patterns as `ubfx`/`sbfx`/`bfi`. |
| Y   | Carry / borrow chain formation | 1 | Opt2 / codegen | Form adc/sbc sequences for wide arithmetic or checked-op idioms. |
| Y   | Multi-instruction constant synthesis | 1 | codegen | Cost-model choice among `movz`/`movk`/`movn`, literal loads, logical immediates, shared bases. |
| Y   | Reciprocal / remainder lowering (exact) | 1 | Opt2 / codegen | Expand exact constant division + modulo jointly, sharing the quotient + checks. |
| Y   | Loop rerolling | 3 | Opt2 | Recognize unrolled repeated bodies and reconstruct a loop (shrinks size, re-enables vectorization). |
| Y†  | Loop predication | 5 | Opt2 | Convert per-iteration conditions/checks to predicates. **†strict trap semantics need proof or versioning — dial-safe form is versioned, else Level 6.** |
| Y   | Loop flattening / collapse | 3 | Opt1 | Collapse nested iteration spaces where ordering + overflow are preserved. **†checked index arithmetic needs proof.** |
| Y   | Loop reversal | 3 | Opt1 | Iterate in reverse to improve addressing / cut induction work. **†must preserve operation/trap order where observable.** |
| Y   | Loop-invariant branch elimination | 3 | Opt2 | Delete/fold an invariant branch without duplicating the whole loop (lighter than unswitching). |
| Y   | Loop exit-value simplification | 3 | Opt2 | Compute post-loop induction values directly; remove exit phis / final-iteration work. **†checked arithmetic needs proof.** |
| Y   | Loop-nest invariant code motion | 3 | Opt2 | Hoist to the shallowest safe loop level, not merely out of one loop. |
| Y   | Unroll-and-jam | 5 | Opt1 | Unroll an outer loop + fuse inner bodies for reuse/vectorization. Distinct from plain unroll + fusion. |
| Y   | Strip mining | 3 | Opt1 | Partition the iteration space explicitly; foundation for tiling + vector-width loops. |
| Y   | Vectorization epilogue optimization | 5 | Opt2 | Masked tails / scalar epilogues / epilogue vectorization / multiple vector widths. |
| Y†  | Reduction recognition + vectorization | 5 | Opt2 | Sums/products/min-max/logical reductions. **†checked integer reductions need proof/versioning or Level-6 relaxation.** |
| Y   | String concat / rope fusion | 3 | Opt1 / Opt2 | Fuse a chain `a & b & c` into one pre-sized allocation + writes instead of an intermediate per operator. **Real gap: general `&` chains allocate + discard intermediates (`lower_string_concat`, `builder_value_semantics.rs:475`); only the self-append idiom `s = s & …` is already fused (plan-02 §4.1). Highest-value string win.** |
| Y   | Small-string / small-array optimization (SSO) | 4 | Opt1 (pre-Plan1) | Inline storage for short strings/arrays to skip a heap/arena allocation; a representation decision before Plan1. **Real gap: general non-empty String/List/Array always arena-allocate; only empty-string, static literals, and the fixed `FloatN` vector carrier are inline today.** |
| Y   | Multi-value return in registers | 4 | Opt1 | Return a small record in value registers instead of boxing it and returning a pointer. **Real gap: records are materialized in the arena and returned as one pointer (`materialize_inline_value_in_arena`, `builder_exits.rs:335`); the 4-register form is the error-outcome ABI, not aggregate decomposition.** |
| Y   | Union layout narrowing | 4 | Opt1 | Drop the tag word from a union's runtime representation when it is provably single-variant / statically known. **Real gap: the tag is always stored at offset 0 (`store_u64(tag, block, 0)`); distinct from the tag-*check* elimination row (that removes tests, not the field).** |
| Y   | Error-payload per-call-site specialization | 4 | Opt1 / Opt2 | When a call site only checks *whether* it failed (not *which* error), skip constructing the `Error` payload and just set the fail flag. Call-site-specific — pairs with, but distinct from, per-function Result/tag specialization and per-edge error-construction sinking. |
| Y   | Codepoint `len()` caching | 3 | Opt2 | Cache the scanned UTF-8 codepoint count of a String across uses. **Byte length is already an O(1) header read (`strings.byteLen`), but `len(str)` scans codepoints every call (`builder_collection_layout.rs:1179`) — the scan result is the cacheable quantity.** |
| Y   | Multi-way select / CSEL-chain formation | 2 | codegen | Build 3+-way `csel`/`csinc` (AArch64) or `cmov` (x86) chains where a later select's false input is a prior select's result. Distinct from single-select formation. |
| Y   | x86 LEA-as-arithmetic | 1 | codegen | Use `lea` as a 3-operand add / shift-add for *arithmetic* (not just address computation). Distinct from the addressing-mode row. |
| Y   | Compare-with-zero branch (cbz/cbnz) formation | 1 | codegen | Fold `cmp x, #0` + `b.eq`/`b.ne` into AArch64 `cbz`/`cbnz`. Small peephole complementing the existing cmp/branch fusion. |
| Y   | SoA → AoS transformation | 4 | Opt1 (pre-Plan1) | The reverse of AoS→SoA — choose array-of-structs for pointer-chasing-light access patterns. Completes the bidirectional layout choice (pick a direction by access pattern). |

## Feasibility notes (grounded in the current compiler)

### The two N's

- **UB-based optimization — N.** MFB has *no undefined behavior* to exploit. Integer arithmetic is **checked and traps `ErrOverflow`** (never wraps) — `mfb spec language types` §4.1; codegen emits explicit overflow checks (`builder_numeric.rs:854/888/949`, `emit_overflow_if_flags_set` at `:1165`). There are no raw pointers (memory-safe by construction), and Float is finiteness-trapped (`ErrFloatNaN`/`ErrFloatOverflow`), not UB. The classic C levers (assume-no-signed-overflow, strict aliasing, assume-no-null-deref) simply don't exist. The realistic analogue is the *opposite* — **proving checks safe and eliding them** — which already exists (overflow-check elision, plan-39; BCE, plan-86). Pursue more provable elision, not UB assumptions.

- **Null-check elimination — N (as written).** MFB has no null/absent form for values: fields are mandatory owned values (§4.2), there is no built-in Option/Maybe (absence is an `error(...)`, §4.4), and `Nothing` is a unit marker (§4.6). So there are literally no null checks in emitted code to remove. **The valuable analogue is a distinct pass worth adding: redundant union-tag / error-tag (fallible-call) check elimination** — dominance-based removal of a `MATCH` discriminant test or a fallible-call success/error branch already proven on a dominating path. That analogue is a clean **Y** and belongs in Opt2; it's just not "null-check elimination."

### The cross-cutting constraint that colors every † row: checked-overflow trapping

Because integer `+ - * ^` and `MOD` **trap deterministically on overflow**, and index/division ops trap too, any transform that **reorders, removes, speculates, or re-associates** a trapping operation can change *whether/when a program raises* — which is observable behavior, not UB. Concretely, the † rows must gate on trap-safety:

- **Reassociation / expression-tree balancing / loop strength reduction:** `(a+b)+c` and `a+(b+c)` can differ in *which* addition overflows. Safe only on FP (which is order-defined but not trapping mid-expression), on values *proven* not to overflow (range analysis), or on operations already elided to unchecked. Do **not** freely reassociate checked integer arithmetic.
- **LICM / speculative hoisting:** hoisting a checked op above its guard can make it trap on a path (or iteration count) where the original never executed it. Hoist only trap-free ops, or ops proven in-range at the hoist point.
- **Loop deletion:** a loop whose body can trap (overflow, `ErrIndexOutOfRange`) is **not** side-effect-free — deleting it can remove an observable raise. Delete only after proving the body is trap-free.

This is why these passes were pulled onto **Level 6** (opt-in, off the numeric dial): their unproven form is a semantic relaxation. Their *proof-gated* safe form stays on the dial (LICM at L3 hoisting only trap-free ops; deleting a proven-trap-free loop at L2). Either way, the enabler is a **range/trap analysis** — now a **Plan2 prerequisite** (demand-driven, not a dial row — see plan-100), not an optional extra.

### The ‡ rows: applicable, but need net-new infrastructure

- **Auto-vectorization / SLP — ‡.** True packed SIMD **already exists and is emitted today**, but only 2-lane/128-bit (`FAddV`-family in `src/arch/ops.rs:144`, real NEON `.2d` at `aarch64/encode/emitter.rs:443`, SSE packed at `x86_64/encode/emitter.rs:974`, RISC-V scalarized fallback), driven only by `math::` array kernels (`builder_simd_math.rs`). A vectorizer would **reuse those encoders + `abi::vector_*` helpers** for f64×2 / i64×2 — no new encoders for the 2-lane case — but it's net-new *analysis* (dependence/trip-count + widening selection). Wider forms (4×f32, AVX ymm) *are* net-new encoders. Note: the `vector::` package is **not** packed SIMD — it keeps one scalar FP reg per lane (`builder_vector_inline.rs`).
- **Instruction scheduling / software pipelining — ‡.** No scheduler, latency model, or machine model exists today (searched `src/target`/`src/arch`: no `schedul|latency|pipelin|hazard`). Instructions emit in-order as lowered; the only post-lowering passes are FMA fusion + two peepholes. Feasible, but the machine model is entirely net-new.
- **PGO — ‡.** No profile-consuming optimizer today. But `mfb test --coverage` already injects per-statement block counters (`src/testing/desugar/coverage.rs`) and writes a `.covdata` sidecar — a working precedent for the *instrumentation* half. The *consumption* half (edge weights → inlining/layout/BCE) is net-new.
- **Prefetch insertion — ‡.** Needs prefetch instruction encoders (`prfm`/`prefetcht0`) that aren't present yet; addable.

### Free wins MFB's semantics hand you (not in the classic C playbook)

Memory-safety + checked arithmetic mean the highest-value early passes are **check-elision** ones: broaden the existing **overflow-check elision** (plan-39, `elide_overflow`/`integer_sub_elidable`) and **bounds-check elimination** (plan-86, `is_provable_index_access`) with a real **range/value-range analysis** (a Plan2 prerequisite — see plan-100). These remove real, measurable per-operation checks the compiler emits today — a bigger and safer payoff than chasing UB-style transforms that don't apply here. Note these are behavior-*preserving* (they only elide a check when they prove it can never fire), so they stay on the numeric dial — unlike the Level-6 relaxations.

### Plan2 infrastructure, not dial rows

Several items that look like passes are really the *fact base* other passes consume — like alias/range analysis, they belong in plan-100's Plan2 as demand-driven infrastructure, gated by "does an enabled pass need this," not by a level number:

- **Function attribute inference** (`pure` / `readonly` / `noreturn` / **`no-trap`**). Bottom-up per-function facts. The `no-trap` attribute is load-bearing: without it, every call is an optimization barrier under MFB's semantics (any callee might raise), so LICM/DSE/sinking *across calls* all depend on it. High priority — but as a prerequisite, not a numeric-dial row.
- **Trivial-phi / phi simplification.** SSA *maintenance* (remove trivial/duplicate phis), runs whenever SSA is live as part of Plan2 construction + out-of-SSA. Constantly re-runnable, never level-gated. Fold into Plan2, not a row. (The *user-visible* CFG cleanup — two-entry-phi→select, empty-block removal — is the separate "CFG simplification" dial row above.)
- **Memory SSA / memory-dependence analysis.** The shared representation that gives DSE, load elimination, store-to-load forwarding, memory PRE, and LICM their memory facts. Infrastructure, built alongside Plan2's alias analysis — not a dial row.
- **Loop canonicalization.** Preheaders, dedicated exits, latch normalization, reducible-loop normalization. Enabling infrastructure for every loop pass; runs whenever any loop pass is enabled, not level-gated.
- **Register allocation & base instruction selection** are also infrastructure (they run at every level — marked `—` in the Level column); only their *refinements/combining* are dial passes.

### The trap-precision decision (verified against the spec)

The one spec-level fork the review flags is real and only partly pre-decided (`mfb spec language types` §4.1, error-model §8.1/§8.5a):

- **Integer/Byte:** checked **per operation**, never wrap; `Error.source` is stamped at the *failing expression's* location and never rewritten. So the spec already pins expression-level precision — you cannot silently move *where* an integer trap claims to originate. That is what makes "Guard/check combining" a Level-6 (behavior-changing) pass by default, and what keeps the trap-order † rows †.
- **Float:** *explicitly imprecise* — finiteness is enforced "at observation boundaries rather than after each operation," transient non-finite intermediates are allowed to recover without trapping. So combining/reordering **Float** finiteness checks is already sanctioned by the spec (those rows are behavior-preserving, dial-eligible).
- **Not addressed by any named guarantee:** per-*iteration* precision and whether equivalent integer checks may be reordered/combined. This is the deliberate decision to pin before building the range/trap analysis. Two coherent answers: **(a)** keep precise integer traps and buy performance with **loop versioning** (recommended — preserves MFB's deterministic-error value proposition); or **(b)** weaken the integer contract to "same error for the same input, iteration/location unspecified," which drops the † from check-combining/early-trapping and several loop rows — at the cost of user-visible determinism. They are alternatives, not a stack.

### Reclassifications flagged by review (existing rows)

Level tweaks the review is right about — the loop ones are edited in the table; the rest are constraints on *when* a row keeps its level:

- **Loop interchange / tiling / skewing: 3 → 5** (edited). They rely on array dependence analysis, which is the "deepest-analysis, silent-corruption" risk that defines L5 — even though results are preserved. **Loop fusion / fission stay L3** (coarser legality check, no full dependence solver).
- **Register allocation & instruction selection → infrastructure** (marked `—`), not dial rows — they run at every level; only refinements/combining are gated.
- **Constant folding is L1 only for non-trapping expressions.** Folding a *trapping* constant expression (e.g. a constant overflow or divide-by-zero) must preserve the same runtime trap, source location, and execution condition — turning a conditionally-executed runtime trap into an unconditional/earlier one is a behavior change (L6, or emit a compile-time diagnostic only when the expression is unconditionally evaluated).
- **Division-by-constant lowering is L1 only for the arithmetic;** the checked path (zero divisor, signed MIN ÷ -1) must still be handled correctly — the row assumes the strength-reduced sequence preserves those traps.
- **Prefetch stays L2, deliberately:** on both arches the prefetch instruction is architecturally non-faulting (AArch64 `prfm`, x86 `prefetch*`), so it cannot miscompile — correctness risk is ~zero, which is an L2/L1 trait, not L3. It *is* code-additive; that's a code-size/perf concern, not a risk-level one.
- **Jump-table generation / switch lowering** are behavior-preserving codegen *policy* (not "same operations executed"); L1 reflects their ~zero miscompile risk, but read them as lowering-strategy choices rather than transparent rewrites.

### Proposed but moot / already done (recorded so they aren't re-proposed)

Verified against the source — these are *not* gaps:

- **Arena / thread-local state register promotion — already done.** The arena base *and* current-thread pointer are program-wide pinned, allocation-reserved registers on every ISA (AArch64 `x19`/`x20`, x86-64 `r15`/`rbx`, riscv64 `s11`/`s2`; `abi.rs:179`, `:227`). Nothing to promote.
- **String *byte*-length caching — moot.** Byte length is an O(1) header field at offset 0 (`strings.byteLen` = one load). (Only the *codepoint* `len()` scan is cacheable — that's a real row above.)
- **Load-and-zero-extend fusion — N/A.** The backend emits no explicit zero-extensions (Integer math runs in 64-bit `x`-regs; narrow ops rely on implicit `w`-write zeroing) — this is the AArch64 subreg-zext N row.
- **Write-combining buffer optimization — N/A.** MFB has no MMIO / write-combining memory model.
- **Lock-free mailbox pattern recognition — deferred.** Message-passing goes through runtime primitives; recognizing and rewriting synchronization is research-grade, not a near-term row.

**Key architectural takeaways for your pipeline:**

1. **Opt1's job**: interprocedural transforms (inlining, specialization, dead args), anything that **changes storage layout or signatures** (SROA, field reordering, escape analysis) — these *must* land before Plan1, and structured-loop transforms (interchange, tiling, fusion) which are much harder once you're on a CFG.
2. **Opt2's job**: everything requiring dataflow — GVN, SCCP, LICM, PRE, DSE, alias-based passes, vectorization, and late CFG cleanup (if-conversion, tail merging) just before out-of-SSA.
3. **Ordering within Opt2**: build SSA → SCCP → GVN/CSE → LICM/loop passes → alias-based memory passes → vectorize → late CFG cleanup → out-of-SSA. Rerun DCE + folding between major passes.
4. **Out-of-SSA copies**: register coalescing during regalloc is what cleans up the copies out-of-SSA inserts — treat them as one design problem.
5. **Watch for**: Opt1 inlining creating huge functions that blow up Opt2's SSA analyses — put an inline budget in early. (And beware the AArch64 >~1 MiB single-function branch-range limit — over-inlining can make a function uncompilable.)
6. **Trap-safety gate**: build the "can this op trap?" predicate + range analysis as a **Plan2 prerequisite** (plan-100), demand-driven. LICM stays on the numeric dial (L3) but may hoist only ops it proves trap-free; the trap-order-*relaxing* passes (reassociation, expression-tree balancing, loop strength reduction, loop deletion, speculative hoisting) plus fast-math live at **Level 6** — opt-in, never implied by the dial.
7. **Two-axis control**: the numeric dial (`-O0..-O5`) escalates *shape distortion at preserved behavior* and gates each row by `row.level <= active_opt_level()` in **both** seams; **Level 6** (`-O6`) is the orthogonal semantic-relaxation opt-in. Prerequisites (SSA/mem2reg, alias, range/trap analysis) are demand-driven Plan2 infrastructure, not levels.
