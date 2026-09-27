### 1. Claims a universal implementation that the release binary does not use
UNIT:      man-page:collections/flatten
CLAIM:     "It does this by calling `collections::append(result, inner)` where `inner` is itself a `List OF T` — that is the list-concatenation overload of `append`, which accepts a second argument that is either the element type or the same list type as the first argument."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/collections/func_flatten.rs:flatten_fast_path` selects the native path for `Integer` and `String`; `CodeBuilder::lower_flatten_native` directly calls `lower_list_bulk_append_in_place`, not `collections::append`. The release-binary probe at `/tmp/plan-125-scratch/C-phase4/man-page-collections-flatten/src/main.mfb` built and printed `5`, `b`, `2 3`, `0`.
SUGGESTED: Each inner list contributes all of its elements to the result, rather than becoming a single nested element.