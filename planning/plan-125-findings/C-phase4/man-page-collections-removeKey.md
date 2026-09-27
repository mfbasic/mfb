### 1. Allocation failure is omitted and described incorrectly
UNIT:      man-page:collections/removeKey
CLAIM:     "`collections::removeKey` raises no trappable domain error — neither a missing key nor an empty map fails — so an inline `TRAP` on a `removeKey` call has a dead handler. Building the result map needs memory, and running out of it is not something a `TRAP` can catch."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/collections/gen_mutate.rs:CodeBuilder::lower_map_remove_key` calls `raise_error_bare("ErrOutOfMemory")` when creating the result map fails. `src/codegen/error/emission/builder_error_emission.rs:CodeBuilder::raise_error_bare` emits the normal error-return path; `src/docs/spec/language/14_memory-semantics.md` states that `ErrOutOfMemory` is an ordinary language-level error that auto-propagates like any other failure. The rendered Errors table is empty because `func_remove_key.rs:register` declares `errors: vec![]`.
SUGGESTED: `removeKey` does not raise an error for an empty map or an absent key. It may raise \`ErrOutOfMemory\` if it cannot create the result map; an inline \`TRAP\` can handle that error.

### 2. Description exposes lowering mechanics
UNIT:      man-page:collections/removeKey
CLAIM:     "It does not edit `value` in place: it counts the entries it will keep and how much room they need, builds a new map, and copies the retained entries into it."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/gen_mutate.rs:CodeBuilder::lower_map_remove_key` implements the two-pass count-and-copy algorithm. Those steps are not observable language behavior; `.ai/man-content.md` §3 excludes lowering mechanics from developer pages.
SUGGESTED: `It leaves \`value\` unchanged and returns a new map containing the retained entries.`

### 3. “Same map object” teaches an invalid mental model
UNIT:      man-page:collections/removeKey
CLAIM:     "Note that this is a new map rather than the same map object — a `removeKey` for an absent key still builds a copy, rather than returning the argument itself."
VERDICT:   out-of-scope
EVIDENCE:  MFBASIC’s language description explicitly says it has “no objects” (`src/docs/spec/language/spec.md`). The observable contract is already established by the page’s “original map is left untouched” claim and by the rendered examples, which compiled and ran: `1`, `FALSE`; `TRUE`; `1`.
SUGGESTED: `Even when \`key\` is absent, the call returns a new map with the same entries as \`value\`.`