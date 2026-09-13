### 1. Bare `TRAP` form is undiscoverable
UNIT:      man-topic:errors  
PAGE:      errors overview  
CATEGORY:  coverage  
CLAIM:     The unit never says that the error binding is optional in either inline or function-level `TRAP`.  
VERDICT:   missing  
EVIDENCE:  `src/docs/spec/language/08_error-model.md` §8.3 and §8.4 specify bare `TRAP`; probe `/tmp/plan-125-scratch/B-phase1/man-topic-errors/bare-trap` built and ran with `mfb build … && …/bare_trap.out`, printing `42` and exiting `0` for `TRAP` without `(e)`.  
SUGGESTED: Add: “The `(e)` name is optional. Write bare `TRAP` when the handler does not need to inspect the Error; `PROPAGATE` still re-propagates the caught error.”

### 2. Inline TRAP’s operator coverage and boundary are omitted
UNIT:      man-topic:errors  
PAGE:      errors overview  
CATEGORY:  coverage  
CLAIM:     The unit says an inline TRAP “traps exactly one expression,” but never tells developers that it catches raising operators within a qualifying expression, nor its short-circuit restriction.  
VERDICT:   missing  
EVIDENCE:  `src/docs/spec/language/08_error-model.md` §8.4 and §8.6 rule 11 define both behaviors. Probe `/tmp/plan-125-scratch/B-phase1/man-topic-errors/operator-trap` built and ran with `mfb build … && …/operator_trap.out`, printing `caught 77050002` then `42`: the inline TRAP caught `1 / 0` inside `toInt("1") + 1 / 0`.  
SUGGESTED: Add a short sharp-edge note: “An inline TRAP covers calls and raising operators within its expression. A fallible call or raising operator in a short-circuited AND/OR operand must be handled separately.”

### 3. Two advertised man-page cross-references fail
UNIT:      man-topic:errors  
PAGE:      errors overview  
CATEGORY:  discoverability  
CLAIM:     “See `mfb man general error`” and “`mfb man flow match`”.  
VERDICT:   wrong  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man general error` and `… mfb man flow match` both exit `2` with `error: mfb man: unknown package …`; `src/cli/man.rs:71-117` accepts a second positional only for registry packages, not guide topics or unqualified-global packages.  
SUGGESTED: Replace with resolvable overview links: `mfb man general` and `mfb man flow`; add a brief instruction to use the relevant page’s function/topic list to locate `error` or `match`.