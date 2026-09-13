### 1. `FOR EACH` omits supported `Set` iteration
UNIT:      man-topic:flow
PAGE:      package-wide
CATEGORY:  overview-mismatch
CLAIM:     The overview says “collection iteration over `List` and `Map` values,” while `forEach` says “`FOR EACH` iterates a `List OF T` or `Map OF K TO V` source; any other source type is a compile error.”
VERDICT:   wrong
EVIDENCE:  `src/ir/lower.rs:collection_iteration_type` accepts `ParameterType::SetOf`; `src/ir/verify/ops.rs` permits List, Set, or Map. I built and ran a scratch program containing `FOR EACH value IN Set OF Integer { 3, 1, 3, 2 }`; it printed `3`, `1`, `2`.
SUGGESTED: State on both pages that `FOR EACH` accepts `List OF T`, `Set OF T`, and `Map OF K TO V`; explain that a set loop binds `T` and visits each element once in insertion order.

### 2. Repeated pipeline placeholders re-evaluate the left side
UNIT:      man-topic:flow
PAGE:      pipeline
CATEGORY:  coverage
CLAIM:     “Only one substitution per `|>` is performed, so writing `_` more than once on a single right-hand side inlines the left expression at each site (evaluating it once, at the position of substitution).”
VERDICT:   misleading
EVIDENCE:  `src/ast/expr.rs:parse_pipeline` says every `_` receives its own copy of the left operand; `src/ast/pipeline.rs:substitute_placeholder` clones it for each placeholder. A scratch program `mark() |> pair(_, _)`, where `mark` increments a global counter, built and ran with output `3:2`: `mark()` ran twice.
SUGGESTED: Replace with: “Each `_` receives its own copy of the left-hand expression. If `_` appears more than once, that expression is evaluated once at each occurrence; bind it to a name first when it has effects or is expensive.”

### 3. Runtime-zero `STEP` behavior is undocumented
UNIT:      man-topic:flow
PAGE:      for
CATEGORY:  coverage
CLAIM:     The page says “A constant `STEP` of zero is a compile error,” but never tells a developer what happens when a nonconstant `STEP` evaluates to zero.
VERDICT:   missing
EVIDENCE:  `src/ir/verify/ops.rs` rejects only a resolvable zero literal. `src/codegen/engine/control/builder_control.rs:lower_numeric_for` admits zero through its nonnegative branch and increments by `step`, so a zero value never advances the counter. A scratch program with `LET delta AS Integer = 0` and `FOR i = 1 TO 2 STEP delta`, guarded by `EXIT FOR`, built and printed `1`.
SUGGESTED: Add: “Only a compile-time zero `STEP` is rejected. If a `STEP` expression evaluates to zero while the initial bound test succeeds, the loop counter never advances; ensure dynamic step values are nonzero.”