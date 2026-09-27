### 1. Callable values exclude valid function parameters
UNIT:      man-page:collections/transform
CLAIM:     “f must be a callable value — a reference to a declared FUNC, or a LAMBDA.”
VERDICT:   misleading
EVIDENCE:  Probe `FUNC apply(f AS FUNC(Integer) AS Integer) AS List OF Integer / RETURN collections::transform([4], f)` compiled and printed `applied=8`; its callback is a function parameter, not a declared-function reference or lambda.
SUGGESTED: `f must be a callable value: a declared FUNC, a LAMBDA, or a compatible FUNC value passed to your function.`

### 2. Unobservable partial-output cleanup leaks implementation detail
UNIT:      man-page:collections/transform
CLAIM:     “The partially built output is discarded on that path before the error leaves.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_transform.rs:lower_transform` performs cleanup before forwarding callback failure, but no result is delivered to the MFBASIC developer; the preceding sentence already states that no result list is produced.
SUGGESTED: `Delete this sentence.`