### 1. Float comparison does not match `=`
UNIT:      man-page:collections/hasKey
CLAIM:     "The key to test for. Compared with `=`."
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase4/man-page-collections-hasKey/src/main.mfb` printed `TRUE`, then `FALSE`, then `TRUE` for `positiveZero = negativeZero`, `hasKey(floatKeys, negativeZero)`, and `hasKey(floatKeys, positiveZero)`. `src/codegen/builtins/collections/gen_map.rs:CodeBuilder::emit_map_query_key` and `src/codegen/builtins/collections/gen_set.rs:CodeBuilder::emit_key_membership` compare Float key bytes.
SUGGESTED: The key to test for. Keys compare exactly; for example, a `Float` key `-0.0` does not match `0.0`.

### 2. Key parameter omits empty, zero, and negative cases
UNIT:      man-page:collections/hasKey
CLAIM:     "The key to test for. Compared with `=`."
VERDICT:   incomplete
EVIDENCE:  The same probe printed `TRUE` for stored keys `-1`, `0`, and `""`, and `FALSE` when querying `""` in an empty map. `src/codegen/builtins/collections/func_has_key.rs:register` accepts `key AS K` with no additional range restriction.
SUGGESTED: The key to test for, with the map's key type. Empty string keys and zero or negative numeric keys are valid.

### 3. Copy-detail sentence teaches an implementation model
UNIT:      man-page:collections/hasKey
CLAIM:     "The map is neither copied nor mutated, and the matching value is never copied out — only the key is compared."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_has_key.rs:lower_has_key` only produces a Boolean membership result; whether arguments or an unreturned map value are copied is not developer-observable. `.ai/man-content.md` §4 requires documentation to avoid teaching a memory-management model even when it avoids the mechanically banned terms.
SUGGESTED: The map is not modified.