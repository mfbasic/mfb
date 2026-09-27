### 1. Intro omits the Set overload
UNIT:      man-page:collections/contains
CLAIM:     "Test whether a list holds an item equal to a given value."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/collections/func_contains.rs:register` registers both `List OF T` and `Set OF T` overloads; the scratch probe built and printed `TRUE` then `FALSE` for a `Set OF Integer`.
SUGGESTED: Test whether a list or set holds an item equal to a given value.

### 2. Set parameter is called a list
UNIT:      man-page:collections/contains
CLAIM:     "The list to search. Not modified."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/collections/func_contains.rs:register` attaches this description to the `Set OF T` overload too; the scratch probe’s `Set OF Integer` calls built and ran.
SUGGESTED: The collection to search. Not modified.

### 3. Parameter says Float matching uses `=`
UNIT:      man-page:collections/contains
CLAIM:     "The element to look for. Compared with `=`, so it must be comparable with the element type."
VERDICT:   wrong
EVIDENCE:  Scratch probe `contains([0.0], -0.0)` printed `FALSE`, while `0.0 = -0.0` is stated as `TRUE`; `src/codegen/collection/compare/builder_collection_compare.rs:emit_comparable_values_match_branch_from_slots` compares `Float` bit patterns.
SUGGESTED: The element to look for. Its type must be comparable with the collection’s element type; Float matching uses exact bit patterns.

### 4. Resource and nested-collection matching cannot be called
UNIT:      man-page:collections/contains
CLAIM:     "A resource handle, or a nested collection that holds one, is compared by identity — whether it is the same handle — rather than by its contents."
VERDICT:   wrong
EVIDENCE:  Scratch probe `collections::contains(nested, [1, 2])`, with `nested AS List OF List OF Integer`, failed to build with `TYPE_REQUIRES_COMPARABLE: ... got List OF Integer`. `src/ir/verify/compat.rs:check_builtin_comparability` enforces comparable list elements; `src/docs/spec/language/12_collections.md` states that lists and resources are not comparable.
SUGGESTED: Remove this sentence. `contains` requires a comparable element type, so it cannot search lists of resources or nested collections.

### 5. Empty-set behavior is omitted
UNIT:      man-page:collections/contains
CLAIM:     "An empty list always yields `FALSE`."
VERDICT:   incomplete
EVIDENCE:  The page also supports `Set OF T`; the scratch probe built and printed `FALSE` for `collections::contains(Set OF Integer { }, 0)`.
SUGGESTED: An empty list or set always yields `FALSE`.

### 6. Overload-resolution implementation detail
UNIT:      man-page:collections/contains
CLAIM:     "Both forms take (collection, element) AS Boolean and answer the same membership question; the compiler picks the overload from the static type of the first argument."
VERDICT:   out-of-scope
EVIDENCE:  `.ai/man-content.md` §3 bans compiler-oriented explanations; `src/codegen/builtins/collections/func_contains.rs:register` supplies the two public overloads.
SUGGESTED: Both forms answer the same membership question: pass either a list or a set as the first argument.

### 7. “Dead handler” teaches a compiler model
UNIT:      man-page:collections/contains
CLAIM:     "`collections::contains` raises no trappable domain error, so an inline `TRAP` on a `contains` call has a dead handler."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_contains.rs:register` declares no errors, but `.ai/man-content.md` §3 excludes compiler mental-model terminology such as a “dead handler.”
SUGGESTED: `collections::contains` raises no trappable domain error.