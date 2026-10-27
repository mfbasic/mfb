### 1. `set` promises a key order the self-assign shape does not produce
UNIT:      man-pkg:collections
PAGE:      collections::set, collections::keys
CATEGORY:  consistency
CLAIM:     set: "When the key is present its value is overwritten and that entry moves to the end of the map's order" and, of the `c = collections::set(c, k, v)` shape, "The result is the same either way." keys: "Assigning an existing key with collections::set does change it: the returned map has that key last."
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/B-recheck/man-pkg-collections/p_order.mfb` (built with `mfb build -q proj`, run): `LET m2 = collections::set(m, "a", 9)` on `{a,b,c}` prints `copy-set keys b,c,a`; `m = collections::set(m, "a", 9)` on the same map prints `self-set keys a,b,c`. The two shapes the page says are equivalent give different `keys` order. `merge` with `preferB := TRUE` prints `merge keys a,b,z`, matching its own "does not move the key to the end". Source: `src/codegen/builtins/collections/func_set.rs:28`, `func_keys.rs:23`. The spec (`src/docs/spec/language/12_collections.md:106`, `10_control-flow.md:41`) makes map order implementation-defined, so this is a doc error, not a code bug.
SUGGESTED: On `set`, drop "and that entry moves to the end of the map's order" and say only that the value is overwritten; keep the "result is the same" sentence but scope it to contents. On `keys`, delete the "returned map has that key last" sentence and leave the implementation-defined hedge that already follows it. Both pages then agree with `merge`.

### 2. Two different MUT-capture rules stated across the predicate pages
UNIT:      man-pkg:collections
PAGE:      collections::all, collections::any (vs. findIndex, findLastIndex, partition, forEach)
CATEGORY:  consistency
CLAIM:     all/any: "A lambda passed to all cannot capture an outer MUT binding; use collections::forEach when the scan needs to update one." findIndex/findLastIndex/partition: "A lambda passed here cannot use a MUT binding from the surrounding function; see mfb man lambda." filter, transform, reduce, reduceRight, groupBy, mapValues, sortBy: nothing.
VERDICT:   inconsistent
EVIDENCE:  Probe `p_capture.mfb`: a module-level `MUT hits` read inside a LAMBDA passed to `collections::all` compiles and prints `TRUE`. Probe `p_capture2.mfb`: a local `MUT total` in a `filter` lambda fails with `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`. `mfb man lambda` states the rule precisely: local MUT is rejected, a top-level MUT is named directly. So all/any's "outer MUT binding" wording is wrong for top-level MUTs, the findIndex wording is right, and seven sibling HOF pages that take the same `FUNC(T) AS …` argument say nothing.
SUGGESTED: Use one sentence on every page that takes a function value: "A lambda passed here cannot use a MUT declared in the surrounding function; see mfb man lambda. collections::forEach is the one helper where it can." Or state it once in the overview's "Predicates and other function arguments" paragraph and link from the pages.

### 3. The overview says nothing is updated in place; ten pages say the self-assign shape is
UNIT:      man-pkg:collections
PAGE:      package-wide (overview; add, append, drop, insert, mid, removeAt, repeat, replace, set, take vs. prepend, remove, removeKey)
CATEGORY:  overview-mismatch
CLAIM:     Overview: "These helpers do not mutate their arguments. A function that changes a collection returns a new value and leaves the original unchanged." Pages: "Assigning straight back to the same MUT variable — list = collections::append(list, x) — is the cheap shape ... it grows the list instead of copying it" (and similar on nine others).
VERDICT:   inconsistent
EVIDENCE:  `grep -n "cheap shape" src/codegen/builtins/collections/func_*.rs` lists add, append, drop, insert, mid, removeAt, repeat, replace, set, take. The compiler's self-update rewrite (`src/codegen/collection/assign/builder_inplace_assign/mod.rs`, `resolve_self_update(... "add"|"append"|"insert"|"prepend"|"remove"|"removeAt"|"removeKey"|"repeat"|"set")`) also covers prepend, remove and removeKey, whose pages never mention it; `remove` is the sibling of `add`, which does. `mfb man types set` already states the idiom in one sentence. The overview, the only place a developer learns the package's contract, never introduces the idiom, so the first function page contradicts it.
SUGGESTED: Add one sentence to the overview after "leaves the original unchanged": "Writing the result back to the same MUT variable — xs = collections::append(xs, x) — lets the compiler update that variable in place; the result is identical, only the cost differs." Then either add the same sentence to prepend, remove and removeKey, or cut the per-page paragraphs down to a pointer to the overview so the eleven pages stop restating it in eleven wordings.

### 4. drop and take are absent from the overview's inventory
UNIT:      man-pkg:collections
PAGE:      overview
CATEGORY:  discoverability
CLAIM:     "element access and updates (get, set, append, prepend, insert, removeAt, removeKey, repeat), higher-order transforms (...), queries (...), reshaping (sort, sortBy, distinct, flatten, zip, chunks, window, partition, groupBy, merge), set conversion and algebra (...), and numeric folding (sum)."
VERDICT:   missing
EVIDENCE:  `mfb man collections | grep -n -w "drop\|take"` hits only the Functions table rows (lines 101, 167); neither word appears in the Description. `mid` and `replace` appear only in "The List-only overloads of find, mid, and replace live here", and `getOr` only inside the UNION paragraph. A developer asking "how do I slice a list" reads the grouped inventory and finds no slicing group.
SUGGESTED: Add a group to the inventory: "slicing (take, drop, mid)" and put getOr next to get in the access group and replace next to set in the updates group.

### 5. The overview never says there is no bracket indexing
UNIT:      man-pkg:collections
PAGE:      overview
CATEGORY:  coverage
CLAIM:     "List indexes are zero-based." (nothing else about how a list or map is read)
VERDICT:   missing
EVIDENCE:  Probe `p_index.mfb`: `xs[1] = 9` and `m["b"] = 2` fail with `SYMBOL_UNKNOWN_TYPE`. The spec says it in its first line (`src/docs/spec/language/12_collections.md:3`: "no indexing brackets, no key brackets"), and `mfb man types list` says it ("there is no indexing-bracket syntax. All list access is through free functions such as collections::get"). The collections overview, which is where a developer from any other BASIC lands after `xs(1)` or `xs[1]` fails, does not, and it links neither `mfb man types list`, `types map` nor `types set` (only `types pair` and `types partition`).
SUGGESTED: Extend the sentence: "List indexes are zero-based. There is no bracket indexing in the language: collections::get and collections::set are the only way to read or replace an element. See mfb man types list, types map and types set for the containers themselves, including how FOR EACH walks each one."
