### 1. “Mutation” mislabels the helpers
UNIT:      man-page:collections/overview
CLAIM:     “The collections package provides package-qualified helpers for List, Map, and Set values: element access and mutation (get, set, append, prepend, insert, removeAt, removeKey) …”
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/collections/func_get.rs:INTO_GET` defines `get` as a read; `func_set.rs:DESC_SET` says `set` returns a new collection and does not change its argument. The overview’s following paragraph says the same. Probe `overview_probe` printed `10,99`, proving `collections::set(original, 0, 99)` left `original` unchanged.
SUGGESTED: “The collections package provides package-qualified helpers for List, Map, and Set values: element access and updates (`get`, `set`, `append`, `prepend`, `insert`, `removeAt`, `removeKey`) …”

### 2. Error conditions are absent
UNIT:      man-page:collections/overview
CLAIM:     “List or string index/range is outside valid bounds.”
VERDICT:   incomplete
EVIDENCE:  The rendered Errors table gives only the generic message. `func_get.rs:register` raises `ErrIndexOutOfRange` for an invalid list index; `func_mid.rs:DESC` raises it when `start`, `count`, or `start + count` is invalid; `func_insert.rs:register` and `func_remove_at.rs:register` also register it. The page does not tell the developer which operations or bounds produce this error.
SUGGESTED: “`ErrIndexOutOfRange` is raised when a list operation receives an invalid index or range; each function page states its exact accepted bounds.”

### 3. Invalid-argument conditions are absent
UNIT:      man-page:collections/overview
CLAIM:     “Argument value is not valid for the requested operation.”
VERDICT:   incomplete
EVIDENCE:  `func_chunks.rs:BODY` raises `ErrInvalidArgument` when `chunkSize < 1`; `func_window.rs:BODY` raises it when `size < 1 OR stride < 1`. The overview’s Errors table supplies neither condition nor the fact that zero and negative values raise rather than clamp.
SUGGESTED: “`ErrInvalidArgument` is raised when `chunks` receives `chunkSize < 1`, or when `window` receives `size < 1` or `stride < 1`; these values raise rather than clamp.”

### 4. Not-found conditions are absent
UNIT:      man-page:collections/overview
CLAIM:     “Requested item, key, file, or resource was not found.”
VERDICT:   incomplete
EVIDENCE:  `func_get.rs:register` registers `ErrNotFound` for an absent map key; `func_find.rs:register`, `func_find_index.rs:register`, and `func_find_last_index.rs:register` register it for searches without a match. The overview does not identify either condition.
SUGGESTED: “`ErrNotFound` is raised for an absent map key and when a `find`, `findIndex`, or `findLastIndex` search has no match.”

### 5. Overflow condition is absent
UNIT:      man-page:collections/overview
CLAIM:     “Arithmetic overflow or numeric conversion outside the destination range.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/collections/func_sum.rs:DESC_SUM` states that only `Integer` and `Fixed` sums raise `ErrOverflow` when the running total leaves the 64-bit destination range; Float sums use IEEE-754 behavior instead. The overview gives no package-specific condition.
SUGGESTED: “`ErrOverflow` is raised when an `Integer` or `Fixed` `sum` exceeds its destination range.”

### 6. No runnable package example
UNIT:      man-page:collections/overview
CLAIM:     “The rendered page has no Examples section.”
VERDICT:   incomplete
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man collections` rendered only Description, Functions, and Errors. The independently written probe at `/tmp/plan-125-scratch/C-phase4/man-page-collections-overview/probe-project/src/main.mfb` built and printed:
`10,99`
`3,1,2`
`2`
SUGGESTED: “Add an Examples section showing an unchanged input after `set`, stable `toList` insertion order, and an overloaded predicate such as `isPositive` in a typed higher-order call.”

### 7. Unobservable copy claim teaches the wrong mental model
UNIT:      man-page:collections/overview
CLAIM:     “List indexes are zero-based, and access reads without copying the collection.”
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §3 excludes implementation mechanics, and §4 permits `copy` only for developer-visible value semantics. Whether lookup copies the containing collection is not a developer-visible contract. `func_get.rs:lower_get` instead materializes the selected element; the probe verified observable lookup behavior (`10,99`), not the overview’s internal-copy assertion.
SUGGESTED: “List indexes are zero-based.”