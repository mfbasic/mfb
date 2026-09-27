### 1. Map key is described as a list index

UNIT:      man-page:collections/getOr  
CLAIM:     “The list index, zero-based. Unlike `collections::get`, out of range is not an error.”  
VERDICT:   wrong  
EVIDENCE:  Rendered map overload uses this description for `index AS K`; [`func_get_or.rs:111`](file:///tmp/plan-125-worktrees/C-phase4-4/src/codegen/builtins/collections/func_get_or.rs:111) passes that argument as `key_type` to `lower_map_get_or`. The rendered example’s `Map OF String TO Integer` lookup returns `0` for key `"Grace"` and `36` for `"Ada"`—keys have no zero-based or range semantics.  
SUGGESTED: For the map overload: “The key to look up. If it is absent, returns `default`.”

### 2. List-index bounds omit the required negative and empty-list behavior

UNIT:      man-page:collections/getOr  
CLAIM:     “The list index, zero-based. Unlike `collections::get`, out of range is not an error.”  
VERDICT:   incomplete  
EVIDENCE:  `lower_list_get_common` treats both `index < 0` and `index >= count` as the fallback path in [`gen_list.rs:82`](file:///tmp/plan-125-worktrees/C-phase4-4/src/codegen/builtins/collections/gen_list.rs:82). Probe `getOr([10], -1, 71)`, `getOr([10], 1, 72)`, and `getOr([], 0, 73)` printed `71`, `72`, and `73`.  
SUGGESTED: “The zero-based list index. A negative index or an index at or beyond the list length returns `default`; this includes index `0` on an empty list.”

### 3. “No trappable error” and “dead handler” are false for the call expression

UNIT:      man-page:collections/getOr  
CLAIM:     “It raises no trappable error at all, which is precisely the difference between the two: an inline `TRAP` on a `collections::getOr` call has a dead handler.”  
VERDICT:   wrong  
EVIDENCE:  Probe `collections::getOr([10], 0, 1 / 0) TRAP(e) RECOVER 77 END TRAP` printed `77`: `default` is evaluated before lookup, and its division-by-zero error reaches the inline handler.  
SUGGESTED: “The lookup itself returns `default` rather than raising when the index is out of range or the map key is absent. Expressions supplied as arguments can still raise.”

### 4. Return description teaches an ownership model indirectly

UNIT:      man-page:collections/getOr  
CLAIM:     “Both paths return a value that is yours to keep and independent of the collection.”  
VERDICT:   banned-vocabulary  
EVIDENCE:  This avoids the banned words but still frames values in terms of possession (“yours to keep”), contrary to `.ai/man-content.md` §4’s requirement to describe observable copy behavior. [`lower_get_or`](file:///tmp/plan-125-worktrees/C-phase4-4/src/codegen/builtins/collections/func_get_or.rs:190) materializes the selected element before returning it.  
SUGGESTED: “The result is a copy: changing it does not change the collection.”

### 5. Implementation-specific map-probe prose is out of scope

UNIT:      man-page:collections/getOr  
CLAIM:     “Map lookup for the common key types `String`, `Integer`, `Float`, `Fixed`, `Byte`, and `Boolean` is a direct lookup — the same one `collections::get` uses — with `default` substituted on the probe's not-found branch; other key types are found by scanning the map.”  
VERDICT:   out-of-scope  
EVIDENCE:  The distinction is selected by `CodeBuilder::map_key_probe_eligible` and emitted by `lower_map_get_or` in [`gen_map.rs:75`](file:///tmp/plan-125-worktrees/C-phase4-4/src/codegen/builtins/collections/gen_map.rs:75) and [`gen_map.rs:475`](file:///tmp/plan-125-worktrees/C-phase4-4/src/codegen/builtins/collections/gen_map.rs:475). “Probe’s not-found branch” describes lowering mechanics, not a developer-visible contract.  
SUGGESTED: “Map keys compare exactly; if no matching key exists, `getOr` returns `default`.”