### 1. Compiler-internals leak in lambda restriction
UNIT:      man-page:collections/any
CLAIM:     “Note that a lambda passed here may not capture an outer `MUT` binding; the callback position proven non-escaping is `collections::forEach`, not `any`.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_any.rs:90-99` implements `any` as an ordinary loop and callback call. Probe `mut-capture.mfb` containing `collections::any([1, 2], LAMBDA(x AS Integer) -> x > total)` fails with `TYPE_LAMBDA_CAPTURE_UNSUPPORTED`: “Lambda captures mutable local `total`; mutable captures are invalid.” The “proven non-escaping” explanation is compiler terminology and teaches an implementation safety model, contrary to `.ai/man-content.md` §3–4.
SUGGESTED: A lambda passed to `any` cannot capture an outer `MUT` binding. Use `collections::forEach` when the scan needs to update one.