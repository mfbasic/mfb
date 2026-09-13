### 1. Canvas close contradicts the universal handle contract
DIMENSION: guide-package
SCOPE:     canvas, tcp, variable
CLAIM:     `canvas::destroyImage` says, “Closing twice is the defined no-op,” while `tcp::close` says, “An already-closed handle is an error rather than a no-op … every built-in close gives,” and `variable` says a closed handle reports `ErrResourceClosed` when static checking cannot catch it.
VERDICT:   The universal `ErrResourceClosed` contract should win. The canvas implementation deliberately performs an unconditional closed-flag store, so the manual cannot be made consistent by prose alone; canvas behavior and its documentation need reconciliation.
EVIDENCE:  `mfb man canvas destroyImage` printed “Closing twice is the defined no-op”; `mfb man tcp close` printed “An already-closed handle is an error rather than a no-op”; `rg -n 'Double-close.*no-op|unconditional store' src/codegen/builtins/canvas/func_destroy_{image,font}.rs` printed comments and code establishing unconditional stores.
SUGGESTED: State one rule everywhere: “An explicit close of an already-closed handle raises `ErrResourceClosed`; automatic cleanup after an explicit close is a silent no-op.” Make `destroyImage` and `destroyFont` conform before publishing that text.

### 2. Map-order guide contradicts itself on whether insertion order is contractual
DIMENSION: guide-package
SCOPE:     types map, flow forEach
CLAIM:     `types map` says “Map iteration order is implementation-defined” but then says repeated traversal uses “the same insertion order.” `flow forEach` instead says map order is “implementation-defined but stable for a given unchanged map value.”
VERDICT:   The `flow forEach` form should win: it states the documented guarantee without accidentally promising insertion order. A probe currently printed insertion order, but one implementation result does not turn it into the language contract.
EVIDENCE:  `mfb man types map` and `mfb man flow forEach` printed the quoted sentences. The scratch probe built with `mfb build /tmp/plan-125-scratch/B-phase4/man-consistency-guide-package` and printed `b`, `a`, `c` for a map inserted in that order.
SUGGESTED: Replace “same insertion order” in `types map` with “the same order,” retaining “implementation-defined but stable for a given unchanged map value during one program run.”