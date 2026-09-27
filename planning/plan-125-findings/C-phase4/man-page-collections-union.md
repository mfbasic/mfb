### 1. Implementation-detail explanation of errors
UNIT:      man-page:collections/union
CLAIM:     "While building the result it needs memory, but running out of memory is not a trappable domain error, and the `add` it is built on is classified infallible for exactly that reason."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_union.rs:DESC` names the internal `collections::add` implementation; `func_union.rs:BODY` shows that it is an implementation detail. The rendered page has no Errors table because `RegistryFunction.implementations[0].errors` is empty.
SUGGESTED: "`union` raises no user-trappable errors."

### 2. Two claims run together in rendered output
UNIT:      man-page:collections/union
CLAIM:     "While building the result it needs memory, but running out of memory is not a trappable domain error, and the `add` it is built on is classified infallible for exactly that reason.Both arguments must be the same `Set OF T`."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/collections/func_union.rs:DESC` contains `reason.Both` with no intervening whitespace; rendering `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man collections union` prints `reason.Both arguments`.
SUGGESTED: "`union` raises no user-trappable errors. Both arguments must be the same `Set OF T`."