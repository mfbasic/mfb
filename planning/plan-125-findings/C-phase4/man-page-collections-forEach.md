### 1. Lambdas are omitted as valid actions
UNIT:      man-page:collections/forEach
CLAIM:     "`action` must be a callable value — a reference to a declared `SUB` or `FUNC`."
VERDICT:   misleading
EVIDENCE:  `/tmp/plan-125-scratch/C-phase4/man-page-collections-forEach/src/main.mfb` compiled and ran with `collections::forEach([1, 2, 3], LAMBDA(n AS Integer) -> total = total + n)`; it printed `6`. The accepted type is registered in `src/codegen/builtins/collections/func_for_each.rs:register`.
SUGGESTED: `action` must be a callable value: a declared `SUB` or `FUNC`, or a `LAMBDA` with one parameter of element type `T` and success type `Nothing`.`

### 2. Compiler implementation detail in closure claim
UNIT:      man-page:collections/forEach
CLAIM:     "`action` is invoked through the shared direct-callable path, which restores a closure's captured environment around each call, so a callable value that carries an environment works as well as a plain named reference."
VERDICT:   out-of-scope
EVIDENCE:  This names an invocation mechanism rather than developer-visible behavior; it corresponds to `lower_for_each` calling `emit_direct_callable_branch` in `src/codegen/builtins/collections/func_for_each.rs:236`. The lambda probe above demonstrates the relevant user-visible fact without requiring this model.
SUGGESTED: `A LAMBDA may use an outer MUT binding during the call, as may a declared callable.`

### 3. Internal cleanup rationale
UNIT:      man-page:collections/forEach
CLAIM:     "Because forEach holds no accumulator, no cleanup runs on that path."
VERDICT:   out-of-scope
EVIDENCE:  This is an implementation-path claim, reflected in `src/codegen/builtins/collections/func_for_each.rs:239-245`; it does not affect what a developer writes or observes.
SUGGESTED: Delete this sentence.