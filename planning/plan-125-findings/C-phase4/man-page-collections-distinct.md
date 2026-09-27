### 1. Map alternative does not lose first-occurrence order at HEAD
UNIT:      man-page:collections/distinct  
CLAIM:     "For large inputs of a comparable key type, building a `Map` keyed by the element and reading `collections::keys` is asymptotically cheaper, at the cost of losing first-occurrence order."  
VERDICT:   misleading  
EVIDENCE:  Probe built a map by inserting `"b"`, `"a"`, duplicate `"b"`, then `"c"` and printed `collections::keys`; it printed `b`, `a`, `c`. `src/codegen/builtins/collections/func_keys.rs:DESC_KEYS` says maps retain insertion order at HEAD (while the language-level iteration order is implementation-defined).  
SUGGESTED: For large inputs, a `Map` keyed by each element can be asymptotically cheaper. Current maps return keys in insertion order, but map iteration order is implementation-defined, so do not rely on that order across versions.

### 2. Comparable-type list omits rejected collection types
UNIT:      man-page:collections/distinct  
CLAIM:     "Comparable types are `Integer`, `Float`, `Fixed`, `Money`, `Boolean`, `String`, `Byte`, `Scalar`, `Nothing`, the built-in `Error` and `ErrorLoc` record shapes, enum types, and records whose fields are all comparable. `List`, `Map`, `UNION` types, function values, threads, and resource handles are **not** comparable..."  
VERDICT:   incomplete  
EVIDENCE:  A probe calling `collections::distinct` on `List OF Set OF Integer` fails with `TYPE_REQUIRES_COMPARABLE`: “Call to `collections.contains` requires a comparable type, got `Set OF Integer`.” `src/ir/verify/values.rs:is_comparable_seen` also rejects `SetOf`, `ResultOf`, and `AttributedString`, none of which the page names.  
SUGGESTED: Add `Set`, `Result`, and `AttributedString` to the non-comparable types, or make the lists explicitly non-exhaustive and point readers to the compile-time diagnostic for other types.