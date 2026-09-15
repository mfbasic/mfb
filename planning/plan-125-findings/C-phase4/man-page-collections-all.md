### 1. Compiler-internal callback rationale on a developer page
UNIT:      man-page:collections/all
CLAIM:     “Note that a lambda passed here may not capture an outer `MUT` binding; the callback position proven non-escaping is `collections::forEach`, not `all`.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_all.rs:DESC` renders this sentence; `src/codegen/builtins/mod.rs:is_nonescaping_callback_arg` grants that implementation property only to `forEach`. A scratch program using `collections::all([1, 2], LAMBDA(x AS Integer) -> x > total)` with `MUT total` fails with `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`. “Proven non-escaping” explains compiler machinery rather than what a terminal user needs.
SUGGESTED: “A lambda passed to `all` cannot capture a `MUT` binding from its enclosing function.”

### 2. Run-together sentences obscure the side-effect contract
UNIT:      man-page:collections/all
CLAIM:     “Note that a lambda passed here may not capture an outer `MUT` binding; the callback position proven non-escaping is `collections::forEach`, not `all.It does not mutate `value` and has no other side effects beyond whatever `predicate` does.”
VERDICT:   misleading
EVIDENCE:  Rendered with `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man collections all`; the same missing space is in `src/codegen/builtins/collections/func_all.rs:DESC`. The implementation body at `func_all.rs:BODY` only reads `value`, calls `predicate`, and returns.
SUGGESTED: “`all` does not mutate `value` and has no side effects beyond calls to `predicate`.”