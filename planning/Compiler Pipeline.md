All four deep-dives are back. Here's the consolidated review.

# MFB Compiler Pipeline — Architectural Review

## The pipeline at a glance

```
source ─▶ lexer ─▶ AST ─▶ scope_privates ─▶ TESTING-lowering ─▶ resolve ─▶ augment ─▶
         monomorph ─▶ resolve(again) ─▶ [syntaxcheck ∥ ir::verify] ─▶ IR lower ─▶
         NIR ─▶ MIR ─▶ per-arch select ─▶ regalloc ─▶ machine code ─▶ link
```

Driver: `build_project` in `src/cli/build/mod.rs:176-891` (the file is 3,581 lines but ~2,700 of that is `#[cfg(test)]`; the orchestration function itself is ~715 lines).

**Overall verdict.** This is a mature, unusually disciplined compiler that emits native code for four ISAs with no LLVM. The macro-architecture is genuinely good: a neutral virtual-register lowering shared across arches with thin per-arch backends, a well-staged verifier, a hash-recompute package boundary that fails safe, and remarkably little rot (3 real `unimplemented!` in non-test code, all the *same* documented riscv64-app blocker; ~7 TODO/FIXME in the 240k-line backend). Correctness discipline is high and everything is backed by ~1,240 golden fixtures plus 34k LOC of in-tree tests.

The debt is **not** stubs or dead code — it's **structural duplication and stringly-typed contracts** that the exhaustiveness checker can't police, plus several load-bearing invariants enforced only by `debug_assert!` or naming convention. These are the things that will burn future work.

---

## The dominant theme: stringly-typed data drives duplication everywhere

This single decision echoes through every stage and is the highest-leverage thing to fix.

- **Types are `String`, never a `Type` enum** (`src/ast/types.rs` — `return_type: Option<String>`, param types are strings). Consequently the same type grammar (`List OF`, `Map OF…TO…`, `FUNC(…) AS…`, `(T)` groups) is hand-parsed by recursive `strip_prefix` cascades in **five independent places**: `resolver/resolution.rs:1267,1386`, `monomorph/lower.rs:1534,1616`, `monomorph/helpers.rs:147-189`, `ast/expr.rs:595`. Adding one type constructor means editing all five in lockstep.

- **Type *inference* is re-implemented across three IR models**: the canonical AST engine `syntaxcheck/inference.rs` (114 KB), `ir::lower::expression_type` (`lower.rs:1930-2208`), `ir::verify::infer_type` (`verify/mod.rs:941`), plus **five NIR walks in codegen** (`static_nir_value_type`, `CodeBuilder::static_type_name` + `_for_fold`, `static_type_name_with_types` + `_for_fold_with_types`). These `_with_types`/`_for_fold` variants are near-clones differing only in the environment they consult — exactly the "sibling walk" family your own memory flags (bug-363→366). They *must* agree for the "verify accepts exactly what lowering emits" soundness rule to hold.

- **Numeric-promotion algebra is copied 7×**: base in `numeric.rs:378`, re-implemented in `ir/lower.rs:3632`, `codegen/engine/types/type_utils.rs:414`, `monomorph/helpers.rs:480`, `syntaxcheck/helpers.rs:272`, plus `promote_loop_numeric_type_name` twice.

- **Expression-tree walks: ~6 unshared exhaustive `match Expression`** traversals (scope_privates, resolver, monomorph lower, monomorph `expression_type`, pipeline placeholder subst, ast serialize) with no visitor abstraction.

Rust's exhaustiveness checking saves you on *adding a variant*, but not on *semantic drift* between these walks — and drift here is a silent wrong value, the worst class in this codebase.

---

## Findings by stage

### Front-end (lexer/ast/resolver/monomorph)
- **Resolution and augmentation each run twice.** `resolve_project` (`mod.rs:327`) internally augments+resolves, then the driver augments *again* (`:331`) and resolves *again* (`:337`). The first augmentation is discarded work; the two resolve passes differ only by a `validate_docs` bool threaded through three public fns (`resolver/mod.rs:68-99`) — DOC validation can only run pre-monomorph because monomorph renames overloaded decls. Correct, but the "which pass am I" knowledge leaks as a parameter.
- **Inverted dependency:** the front-end resolver hard-codes `BUILTIN_TYPES` pulled from `codegen::builtins::{fs,term,net,…}` (`resolver/mod.rs:14-44`) — the back-end leaking into name resolution.
- **Mixed mutation disciplines** on one AST: mutated in place twice (scope_privates, testing), read-only-validated twice (resolve), rebuilt-by-clone twice (augment, monomorph). The rebuild-by-clone loses source locations, forcing bug-107's `current_file`/`function_files` plumbing purely to keep diagnostics pointing at the right file (`monomorph/mod.rs:51-62`).

### Middle-end (IR + verify)
- **"Total lowering" converts *unsupported* into *unchecked*.** Lowering emits the `"Unknown"` type sentinel at **31 sites** (`lower.rs` `unwrap_or_else(|| "Unknown")`), and `ir::verify` deliberately *skips* any `"Unknown"` node rather than rejecting it (`verify/mod.rs:26-31`). On the **package/decoded-`.mfp` path verify is the *sole* type-confusion guard against a crafted package** — so anything lowering couldn't type is waved through. This is the structural cost of the total-lowering choice and the place to watch for soundness gaps.
- **`lower.rs` is a 3,832-line single pass** with `expect`/`unreachable` sites (`:652,1227,2422,3276`) that trust *parser* invariants, not syntaxcheck success — yet lowering runs even after syntaxcheck errors. A malformed-but-parseable AST could panic instead of diagnosing. Contrast the cleanly per-concern-staged `verify/` module — `lower.rs` is the natural candidate for the same decomposition.
- **Diagnostic "source order" is a misnomer** — it's stream concatenation (`syntaxcheck` batch, then `verify`, then `scope`, then export checks; `build/mod.rs:428-437`, `rules/mod.rs:20-26` explicitly *not* line-sorted). It's "correct" only because it matches pinned goldens and programs rarely trip both streams at once. Worse, **resolver and monomorph never joined the collect scheme** — they print immediately and short-circuit with `Err(())` (`mod.rs:327,332`), so a resolver error *suppresses* all syntaxcheck/verify diagnostics; the user fixes it, rebuilds, discovers a fresh wave.
- **Two split-defining lists guarded only by `debug_assert!`** (silent in release): `RELOCATED_TO_IR_VERIFY` (~75 codes, the sole syntaxcheck-vs-verify boundary, `verify/mod.rs:70-159`) and rule-name→identity resolution (`rules/mod.rs:125-145`). A stray relocated rule duplicates (or a deleted rule vanishes) silently in a release build.

### Back-end (codegen/target/arch)
- **Architecture is sound and low-duplication.** Neutral vreg lowering → MIR → per-arch `select`/`regmodel` → ISA-neutral linear-scan regalloc. Adding a builtin touches *one* neutral lowering, not four backends. The 118k-line `codegen/builtins` is mostly **declarative registry descriptors + man-page prose stored as Rust string consts**, not per-arch assembly — verbose but low-risk.
- **The neutral IR is AArch64-shaped, not truly neutral.** `MirOp` reuses the AArch64 `CodeOp` set 1:1 (`mir.rs:8-13`); AArch64 selection is identity, while x86-64/riscv64 selectors translate away and **hard-panic on unmapped ops** (26 `panic!`/`unreachable!` across select.rs). This forces riscv64 to emulate 128-bit vectors in `arch/riscv64/v128.rs` (2,534 LOC). A new neutral op omitted from a selector fails at *runtime*, not compile time — that should be a compile-time error.
- **The register-clobber invariant rests on a string prefix.** `call_clobber_mask` (`regalloc/analysis.rs:190`) sniffs `target.starts_with("_mfb_") && !"_mfb_fn_" && !"_mfb_ifn_"`. A helper named outside that convention silently gets a too-permissive mask → a live value clobbered → miscompile (exactly the class your `.ai/compiler.md` and `arena-alloc-clobbers-x14-x15` memory warn about). bug-350's regression test exists because this was previously mis-branched. Should be a registry-declared clobber attribute, not a naming convention.
- **Plan-101 `abi_function` migration is ~12% done** (24 `abi_function` + 18 `abi_inline` vs 136 `Native` + a hand-coded per-member legacy ladder at `builder_values.rs:741-766`). The codebase carries *both* seams simultaneously — the dominant back-end debt.
- Monolith files: `registry/mod.rs` (3,974 LOC, all concerns in one module), `mir.rs` (1,822), `riscv64/select.rs` (1,419).

### Driver + cross-cutting
- **Cross-package type flow is a self-documented hack.** To feed `verify` only external signatures for functions returning imported *resources*, the driver **round-trips type info structured→string→structured**: `signature.rsplit_once(" AS ")` (`mod.rs:400-405`) parses the return type back out of a `FUNC(…) AS T` string that was itself formatted from the package export. Any change to that format silently breaks the filter, and three helpers (`external_package_function_types`, `imported_type_defs`, `imported_resource_closers`) must stay in lockstep, some registering both bare and `<binding>.<Type>` spellings depending on pre/post-merge ordering.
- **`lower_augmented_project` is called 5× with near-identical external-map preambles** (`mod.rs:416,490,706,779,822`); the executable and package branches duplicate the `installed_package_files → …_from_files → lower` sequence almost verbatim. A single "assemble externals + lower" extraction would remove the biggest driver duplication.
- **The `.mfp` boundary fails *safe* but is *brittle*.** Correctness is enforced by *recomputing* every `sigHash` on decode (`reader.rs:1212-1266`) rather than trusting the wire — which is exactly why a committed `.mfp` goes stale on resource/type re-qualification (your memory note): it fails the build with "must be rebuilt" rather than miscompiling, but there's no migration path, only rebuild. It's also re-decoded 4-5× per build with no in-build cache.
- Path-traversal guards (bug-395, `package.rs:380`, `mod.rs:625`) are correct but were bolt-on discoveries; the "decoded field → filename" pattern isn't systematically audited.

---

## Recommendations, prioritized

1. **Introduce a parsed `Type` enum** and parse type strings once at the boundary. This is the single highest-leverage change — it collapses the 5 type-string parsers, is the precondition for de-duplicating the type-inference engines, and kills the driver's string round-trip. Biggest effort, biggest payoff.
2. **Consolidate the type-inference / numeric-promotion walks** behind shared functions (or at least a single source of truth per concern), so the AST/IR-lower/verify/NIR engines can't drift into silent wrong values.
3. **Harden the two silent-in-release invariants**: make the clobber model a registry-declared attribute instead of an `_mfb_` string sniff, and promote the `debug_assert!` guards on the rule split / rule-name resolution to real errors or build-time checks.
4. **Make missing per-arch selector arms a compile-time error** rather than a runtime panic; longer-term, decouple the neutral op set from AArch64's shape.
5. **Unify the diagnostic model** — route resolver/monomorph through the same collect-and-render path so all errors surface in one pass and one order, and stop short-circuiting before the batch renders.
6. **Finish (or explicitly schedule) the plan-101 `abi_function` migration** to retire the dual dispatch seam; extract the driver's repeated `lower_augmented_project` preamble; split `registry/mod.rs`.
7. **Audit the `"Unknown"`-skips-verify path** specifically on the package-decode side, since that's the sole guard against a hostile `.mfp`.

Want me to turn any of these into a written plan (`write-plan`) — the `Type`-enum refactor or the clobber-attribute hardening are the two best-scoped starting points — or dig deeper into a specific stage?