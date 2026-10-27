### 1. Top-level MUT bindings are read live, not captured, and the page says the opposite
UNIT:      man-topic:lambda
PAGE:      lambda (Capture rules / What is not possible)
CATEGORY:  coverage
CLAIM:     "Capturing a MUT binding is a compile error (TYPE_LAMBDA_CAPTURE_UNSUPPORTED) in any ordinary closure" and "Observing a captured value's later changes through the closure: the closure holds its own copy."
VERDICT:   misleading
EVIDENCE:  Probe /tmp/plan-125-scratch/B-recheck/man-topic-lambda/p16c_module_mut_live.mfb: a module-level `MUT counter`, `LET f = LAMBDA(x AS Integer) -> x + counter`, call, then `bump()` (a FUNC that adds 10 to counter), call again. Built with `mfb build -q` and run: prints `1` then `11`. No diagnostic. So a lambda may name a top-level MUT, it is not a "capture", and it sees every later change. Assigning one from an assignment-bodied lambda is rejected, but not with the code the page names: p16b_module_mut_assign.mfb (`LAMBDA(x AS Integer) -> counter = counter + x` on a top-level MUT) fails `TYPE_UNKNOWN_VALUE: Assignment target `counter` is not a local binding.` (`src/ir/shape/expressions.rs`, the `assign_target` check in the Lambda arm). The page's rules are all written for *local* bindings but never say so, and the spec (`src/docs/spec/language/06_functions.md` §Closures) is equally silent.
SUGGESTED: Add a bullet under Capture rules: "The capture rules apply to local bindings — a LET or MUT declared inside a function. A top-level LET or MUT is not captured at all: a lambda names it directly, like any function does, so it reads the current value every time it runs, including a top-level MUT's later changes. An assignment-bodied lambda may only assign a local binding; `counter = counter + x` on a top-level MUT is rejected (TYPE_UNKNOWN_VALUE)." Adjust the "What is not possible" bullet to "Observing a captured *local* value's later changes…".

### 2. A function value drops defaults and named arguments; the page only says lambdas can't declare defaults
UNIT:      man-topic:lambda
PAGE:      lambda (Function types)
CATEGORY:  coverage
CLAIM:     "parameter types are not inferred and parameters cannot declare default values" (nothing on calling through a FUNC-typed binding)
VERDICT:   missing
EVIDENCE:  p13_defaults_through_value.mfb: `FUNC add(a AS Integer, b AS Integer = 10)`; `add(1)` is fine, but `LET f AS FUNC(Integer, Integer) AS Integer = add` then `f(1)` fails `TYPE_CALL_ARITY_MISMATCH: Call to `f` has 1 argument(s), expected 2.` p13b_named_through_value.mfb: `f(y := 1, x := 5)` on a lambda-bound `f` fails `TYPE_CALL_ARGUMENT_MISMATCH: Call to function value `f` cannot use named arguments because the callable type does not preserve parameter names.` The spec states the first half (`06_functions.md:36-38`, "a call through a function value carries no defaults") and the compiler states the second in its own diagnostic; the page states neither, although "A binding of function type is called like any function" implies the opposite.
SUGGESTED: After "A binding of function type is called like any function:" add: "…with two differences: every argument is positional (a FUNC type keeps parameter types, not names, so `f(x := 1)` is rejected), and every argument is required — a named FUNC with default parameters loses its defaults when used as a value. That is also why a LAMBDA cannot declare a default."

### 3. A lambda stored in a record or list cannot be called in place
UNIT:      man-topic:lambda
PAGE:      lambda (Description)
CATEGORY:  coverage
CLAIM:     "Lambdas are ordinary values that can be bound to a LET, passed as an argument, returned from a function, and stored in records and collections"
VERDICT:   misleading
EVIDENCE:  p8_field_call.mfb: `io::print(toString(ops.f(10)))` where `ops.f` is a `FUNC(Integer) AS Integer` field → `MFB_PARSE_UNEXPECTED_TOKEN: Expected `)` after call arguments.` p6b_direct_call.mfb: `collections::get(fs, 0)()` → `MFB_PARSE_EXPECTED_EXPRESSION: Only identifiers can be called by the current parser.` Binding first works: p7_record_nested.mfb (`LET g = ops.f` / `g(10)`) prints `5`, and p6_for_capture-style `LET a = collections::get(fs, 0)` / `a()` works. Storing is true, but the natural next line a developer writes is a parse error, and nothing on the page or in the diagnostic says what to do instead.
SUGGESTED: Add one sentence after the storage claim: "Only a name can be called: to call a function held in a record field or a list, bind it to a LET first (`LET g = ops.f` then `g(10)`); `ops.f(10)` and `collections::get(fs, 0)()` are parse errors."

### 4. The page never points at FOR EACH, the ordinary way to accumulate into a MUT
UNIT:      man-topic:lambda
PAGE:      lambda (The non-escaping callback exception / See also)
CATEGORY:  discoverability
CLAIM:     "Use an assignment as the body … forEach(items, LAMBDA(x AS Integer) -> total = total + x)" presented as the way to sum into an outer MUT, with See also listing forEach/transform/reduce/variable/thread/errors.
VERDICT:   missing
EVIDENCE:  p21_foreach_plain.mfb: `FOR EACH x IN items` / `total = total + x` / `NEXT` prints `6` with no lambda and no exception to reason about. p6c_for_var.mfb shows the loop variable of both `FOR i = 1 TO 3` and `FOR EACH x IN xs` is capturable as a copy (prints 10 20 30 / 5 6). `mfb man lambda --all` contains no occurrence of "FOR EACH" (grep on the rendered output). A developer who arrives wanting to mutate from inside a loop is steered into the one carve-out instead of the plain statement; the standard asks for "Cross-references to the sibling a developer would otherwise reach for by mistake".
SUGGESTED: Add to the exception section: "If you are only accumulating into a MUT, a `FOR EACH x IN items` loop does the same thing with no capture rule involved (mfb man flow for); the forEach exception matters when the callback has to be a function value." Add `mfb man flow for` to See also.

### 5. A lambda cannot refer to itself
UNIT:      man-topic:lambda
PAGE:      lambda (What is not possible)
CATEGORY:  coverage
CLAIM:     The "What is not possible" list (three bullets, all about MUT and ThreadWorker captures).
VERDICT:   missing
EVIDENCE:  p14_self_ref.mfb: `LET f = LAMBDA(n AS Integer) -> f(n - 1)` → `SYMBOL_UNKNOWN_IDENTIFIER: Callable `f` is not a top-level function.` The binding being declared is not in scope inside its own body, so a recursive lambda is impossible and the diagnostic does not say why. Lower priority than 1–4, but it is the one remaining question a developer coming from another language asks that this unit never answers.
SUGGESTED: Add a bullet: "A recursive lambda: the LET being declared is not in scope inside its own body (`LET f = LAMBDA(n AS Integer) -> f(n - 1)` is SYMBOL_UNKNOWN_IDENTIFIER). Write a named FUNC for recursion and use it as a value."

Checked and found consistent, for the audit trail: the "only forEach" claim matches `is_nonescaping_callback_arg` in `src/codegen/builtins/mod.rs` and probes p2 (transform rejects a MUT capture) and p19 (a user-defined FUNC(T) AS Nothing parameter rejects it too); the rejected `makeCounter` example really fails with TYPE_LAMBDA_CAPTURE_UNSUPPORTED (p1, which also reports a second TYPE_RETURN_MISMATCH because the assignment body is Nothing, as the page's own text predicts); a MUT-capturing lambda bound to a LET and then passed to forEach is rejected (p4), matching "passed directly"; named functions as values, zero-parameter lambdas, returned closures and nested closures all behave as described (p3, p7, p11); SYMBOL_LAMBDA_PARAMETER_DEFAULT fires for a lambda default (p12). The sibling pages agree with this one: `mfb man collections forEach` states the same MUT exception, `mfb man variable` links here for it, `mfb man thread start` says a lambda is not an entry point, and every See also target renders.
