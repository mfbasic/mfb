## Optimizations

Here's the updated table with a **Stage** column mapped to your pipeline. Key reasoning: **Opt1 (NIR)** = high-level, language-aware, works without CFG/SSA (tree/linear rewrites, inlining, loop restructuring on structured IR). **Opt2 (MIR)** = anything needing CFG, SSA, def-use, or dataflow analysis. Some things live outside both gates (regalloc, machine-code emission).

The **Ok** column answers "would this work for *this* compiler, given the plan-100 pipeline?" — **Y** = applicable and hostable in the pipeline (even if it needs net-new infra like a machine model or a vectorizer); **N** = fundamentally inapplicable because MFB lacks the property it exploits. Only two rows are N (no undefined behavior; no null model). Read the **Feasibility notes** under the table — several Y rows carry a hard MFB-specific constraint (checked-overflow trapping) that limits *how* they may fire.

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
- **Levels 1–2 are your "always on" candidates** — even a debug build could run them with little downside.
- **Level 4 is the debuggability cliff** — stack traces stop being truthful; a natural default ceiling for development builds, with 4–5 reserved for release.
- **Level 6 is orthogonal and opt-in** — *never* implied by `-O5` or an "auto/max" setting; the user must name it (`-O6`). Cranking the dial for speed must never silently change results.
- **Prerequisites are not dial rows.** SSA construction (mem2reg), alias analysis, and range/trap analysis are Plan2 infrastructure, built *on demand* when an enabled pass needs them (LICM needs trap-analysis to hoist safely; alias-based passes need alias analysis; L6 rows need range analysis to minimize relaxation). They live in plan-100's Plan2, not in this table.


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
| Y   | Peephole optimization | 1 | Opt2 / post-regalloc | Local pattern rewrites; MIR peepholes in Opt2, machine peepholes after regalloc. |
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
| Y   | Loop interchange | 3 | Opt1 | Swap nested loop order; needs structured loop nests + dependence analysis. |
| Y   | Loop tiling / blocking | 3 | Opt1 | Block loops for cache; structured-loop transformation. |
| Y   | Loop unrolling | 5 | Opt1 or Opt2 | Replicate loop bodies; simple full-unroll in Opt1, runtime/partial unroll with trip-count analysis in Opt2. |
| Y   | Loop peeling | 3 | Opt2 | Split off first/last iterations; usually paired with Opt2 loop analyses. |
| Y   | Loop skewing | 3 | Opt1 | Shift iteration space; structured/polyhedral-level transform. |
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
| Y   | Store-to-load forwarding | 3 | Opt2 | Replace loads with prior stored values; needs alias analysis. (A block-local machine version already exists — `peephole.rs:198`.) |
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
| Y   | Register allocation | 2 | regalloc | Assign virtual registers to physical; your dedicated stage. (Exists — linear-scan.) |
| Y   | Register coalescing | 2 | regalloc | Eliminate copies via shared assignment; part of regalloc (interacts with out-of-SSA copies). (Planned — `allocator-20`.) |
| Y   | Rematerialization | 2 | regalloc | Recompute cheap values instead of spilling; regalloc component. |
| Y   | Stack slot coloring | 2 | regalloc | Reuse slots for non-overlapping lifetimes; regalloc/frame lowering. |
| Y   | Frame pointer omission | 2 | codegen | Free FP register; frame lowering. |
| Y   | Shrink wrapping | 2 | regalloc / codegen | Sink prologue/epilogue to needy paths; after regalloc knows clobbers. |
| Y   | Instruction selection / combining | 1 | codegen | Fuse MIR ops into machine instructions (e.g., FMA); MIR→machine lowering. (FMA fusion + adrp/add + cmp/branch fusion already exist.) |
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

**Key architectural takeaways for your pipeline:**

1. **Opt1's job**: interprocedural transforms (inlining, specialization, dead args), anything that **changes storage layout or signatures** (SROA, field reordering, escape analysis) — these *must* land before Plan1, and structured-loop transforms (interchange, tiling, fusion) which are much harder once you're on a CFG.
2. **Opt2's job**: everything requiring dataflow — GVN, SCCP, LICM, PRE, DSE, alias-based passes, vectorization, and late CFG cleanup (if-conversion, tail merging) just before out-of-SSA.
3. **Ordering within Opt2**: build SSA → SCCP → GVN/CSE → LICM/loop passes → alias-based memory passes → vectorize → late CFG cleanup → out-of-SSA. Rerun DCE + folding between major passes.
4. **Out-of-SSA copies**: register coalescing during regalloc is what cleans up the copies out-of-SSA inserts — treat them as one design problem.
5. **Watch for**: Opt1 inlining creating huge functions that blow up Opt2's SSA analyses — put an inline budget in early. (And beware the AArch64 >~1 MiB single-function branch-range limit — over-inlining can make a function uncompilable.)
6. **Trap-safety gate**: build the "can this op trap?" predicate + range analysis as a **Plan2 prerequisite** (plan-100), demand-driven. LICM stays on the numeric dial (L3) but may hoist only ops it proves trap-free; the trap-order-*relaxing* passes (reassociation, expression-tree balancing, loop strength reduction, loop deletion, speculative hoisting) plus fast-math live at **Level 6** — opt-in, never implied by the dial.
7. **Two-axis control**: the numeric dial (`-O0..-O5`) escalates *shape distortion at preserved behavior* and gates each row by `row.level <= active_opt_level()` in **both** seams; **Level 6** (`-O6`) is the orthogonal semantic-relaxation opt-in. Prerequisites (SSA/mem2reg, alias, range/trap analysis) are demand-driven Plan2 infrastructure, not levels.
