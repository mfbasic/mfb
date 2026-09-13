### 1. Constants cannot be discovered from the package
UNIT:      man-pkg:errorCode
PAGE:      package-wide
CATEGORY:  coverage
CLAIM:     The unit provides no way to find the available error-code names, their numeric values, or their meanings.
VERDICT:   missing
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man errorCode --all` rendered only the overview; `mfb man errorCode ErrPathNotFound` printed `unknown errorCode function`. `src/codegen/builtins/errorcode/mod.rs:register` registers 52 `Err*` constants, each with a value and message.
SUGGESTED: Add a Constants table to the overview, ordered by numeric code, with `errorCode::Name`, Integer value, and meaning; alternatively give each constant a reachable page and list it in the overview.

### 2. The only advertised navigation path is self-contradictory
UNIT:      man-pkg:errorCode
PAGE:      errorCode types
CATEGORY:  discoverability
CLAIM:     “The errorCode package has no public types. Run mfb man errorCode to list its functions.”
VERDICT:   inconsistent
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man errorCode types` printed that sentence, while `mfb man errorCode` says “It exports no functions and declares no types.” `src/cli/man.rs:82-102` supplies this generic fallback without checking whether the package has functions.
SUGGESTED: For a constants-only package, say: “The `errorCode` package exports named Integer constants only; see `mfb man errorCode` for the constants.”