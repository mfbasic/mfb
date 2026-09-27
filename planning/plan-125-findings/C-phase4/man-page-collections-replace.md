### 1. Implementation-detail performance claim

UNIT:      man-page:collections/replace  
CLAIM:     "`replace` does not change `value`. The list it names is unchanged; the modified list is the returned value, and a program observes the update only through what it does with that return value. Unlike `append`, `prepend`, and `set`, there is no cheap in-place shape for `replace`: every call copies the list."  
VERDICT:   out-of-scope  
EVIDENCE:  `.ai/man-content.md` §3 excludes implementation mechanics; `src/codegen/string/repr/builder_strings.rs:CodeBuilder::lower_list_replace` is the implementation-level basis for the “in-place shape” and copying assertion.  
SUGGESTED: "`replace` does not change `value`; it returns a new list containing the replacements."

### 2. Float matching sharp edge omitted

UNIT:      man-page:collections/replace  
CLAIM:     "Matching compares each element against `old` with the same equality `collections::contains` uses, so the element type must be comparable; `old` and `new` must both have exactly the element type `T`."  
VERDICT:   incomplete  
EVIDENCE:  Scratch probe `/tmp/plan-125-scratch/C-phase4/man-page-collections-replace/src/examples.mfb`, built with the requested binary, printed `TRUE` then `FALSE` after replacing `0.0` in `[-0.0, 0.0]`: `-0.0` remained while `0.0` was replaced. `src/codegen/collection/compare/builder_collection_compare.rs:CodeBuilder::emit_collection_payload_matches_value_branch` compares `Float` payload bits directly.  
SUGGESTED: "Matching uses the same rules as `collections::contains`; in particular, Float values match by bit pattern, so `-0.0` and `0.0` are distinct. `old` and `new` must both have element type `T`."