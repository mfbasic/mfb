### 1. List bounds omit the actual valid range

UNIT:      man-page:collections/get  
CLAIM:     “The list index, zero-based. Out of range raises — use `collections::getOr` to supply a fallback instead.”  
VERDICT:   incomplete  
EVIDENCE:  Scratch probe `src/main.mfb` printed `list-negative=ERR 77050001`, `list-length=ERR 77050001`, and `list-empty=ERR 77050001`; `src/codegen/builtins/collections/gen_list.rs:lower_list_get_common` checks `0 <= index < count`.  
SUGGESTED: “The zero-based list index. It must be at least 0 and less than `len(value)`; a negative index, an index equal to `len(value)`, or any index into an empty list raises `ErrIndexOutOfRange`. Use `collections::getOr` to supply a fallback instead.”

### 2. Compiler-facing “raw-supported” leaks into the page

UNIT:      man-page:collections/get  
CLAIM:     “It reports a missing element as a trappable domain error rather than substituting anything, and it is raw-supported, so an inline `TRAP` on a `collections::get` call catches the real runtime error.”  
VERDICT:   out-of-scope  
EVIDENCE:  `raw-supported` appears as the compiler predicate `inline_builtin_raw_supported` in `src/codegen/builtins/mod.rs:320`; it is not a MFBASIC developer concept. The probe caught `ErrIndexOutOfRange` and `ErrNotFound` with inline `TRAP` blocks.  
SUGGESTED: “It reports a missing element as a trappable error rather than substituting a value, so an inline `TRAP` can handle the error.”

### 3. Map lookup implementation strategy is not a developer contract

UNIT:      man-page:collections/get  
CLAIM:     “Map lookup for the common key types `String`, `Integer`, `Float`, `Fixed`, `Byte`, and `Boolean` is a direct lookup; other key types are found by scanning the map.”  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/builtins/collections/gen_map.rs:map_key_probe_eligible` selects this path, while `lower_map_get` contains both the probe and scan implementations. This is lowering strategy rather than terminal-user behavior.  
SUGGESTED: Delete this sentence and the following implementation-performance paragraph.

### 4. “Yours to keep” teaches an ownership model

UNIT:      man-page:collections/get  
CLAIM:     “The value returned is yours to keep and independent of the collection — you get a copy, so nothing you do with it afterwards can disturb the collection it came from.”  
VERDICT:   misleading  
EVIDENCE:  The scratch probe retrieved a nested list, changed the returned list with `collections::set`, and printed `copy-original=1` and `copy-changed=9`; the independence claim is true. But “yours to keep” teaches an ownership model without using a banned term, contrary to `.ai/man-content.md` §4.  
SUGGESTED: “The returned item or value is a copy. Changing that copy does not change the collection.”