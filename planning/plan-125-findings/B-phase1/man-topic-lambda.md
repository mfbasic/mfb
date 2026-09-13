### 1. Local thread entries are wrongly excluded
UNIT:      man-topic:lambda
PAGE:      package-wide
CATEGORY:  consistency
CLAIM:     “only an exported top-level `FUNC` may be a thread entry point.”
VERDICT:   wrong
EVIDENCE:  `mfb man thread start` says an entry may be “one your own project declares, or an `EXPORT ISOLATED FUNC` of a package you imported.” A scratch program with a non-`EXPORT` `ISOLATED FUNC` built and ran via `mfb build /tmp/plan-125-scratch/B-phase1/man-topic-lambda/probes`; it printed `6`.
SUGGESTED: A lambda cannot be `ISOLATED`. A thread entry point is a top-level `ISOLATED FUNC`; one in your project need not be `EXPORT`, while one supplied by an imported package must be.

### 2. The ordinary capturing-closure workflow has no example
UNIT:      man-topic:lambda
PAGE:      package-wide
CATEGORY:  coverage
CLAIM:     The topic explains that an ordinary closure captures a copyable `LET` value and remains usable after its enclosing scope ends, but shows only a non-capturing lambda, the special `forEach` exception, and a rejected `MUT` capture.
VERDICT:   missing
EVIDENCE:  `src/docs/man/lambda/package.md` has no runnable ordinary-capture example; its only capture examples are lines 77–80 and 104–127. A scratch `makeAdder` program returning `LAMBDA(x AS Integer) -> x + base` built and ran with the release binary, printing `12`.
SUGGESTED: Add a short `makeAdder` example that returns a lambda capturing an immutable value, invokes it after `makeAdder` returns, and shows the result.

### 3. The copy-model authority is undiscoverable from this guide
UNIT:      man-topic:lambda
PAGE:      package-wide
CATEGORY:  discoverability
CLAIM:     The Capture rules section explains copies, snapshots, and why `MUT` capture differs, but See also omits `mfb man variable`.
VERDICT:   missing
EVIDENCE:  `src/docs/man/lambda/package.md:129-134` lists only collections pages and errors. `mfb man variable` explicitly says it is “the one place the whole model is written down”; `.ai/man-content.md` §4.5 requires pages needing more than one sentence about copies to link it.
SUGGESTED: Add `mfb man variable` to See also, ideally immediately after the first capture-rules explanation.