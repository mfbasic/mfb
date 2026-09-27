### 1. Run-together error and type-requirement claims
UNIT:      man-page:collections/isSuperset
CLAIM:     "`isSuperset` raises no user-trappable error of its own.Both arguments must be the same `Set OF T`."
VERDICT:   misleading
EVIDENCE:  Rendered with `mfb man collections isSuperset`; it prints `own.Both` without separating the two claims. The descriptor source is `src/codegen/builtins/collections/func_is_superset.rs:DESC`; its `errors: vec![]` supports the first claim. A mismatched-set probe failed at compile time with `TYPE_CALL_ARGUMENT_MISMATCH`.
SUGGESTED: "`isSuperset` raises no user-trappable error of its own. Both arguments must be the same `Set OF T`."