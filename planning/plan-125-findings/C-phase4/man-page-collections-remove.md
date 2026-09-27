### 1. Compiler-implementation detail in behavior description
UNIT:      man-page:collections/remove
CLAIM:     “When the compiler can prove the target is a same local being reassigned — the `set = collections::remove(set, x)` shape — is the cheap shape: it updates the set rather than building a second one.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/collections/func_remove.rs:DESC_REMOVE` renders this sentence; `src/codegen/collection/assign/builder_inplace_assign.rs:try_inplace_set_remove_assign` recognizes that assignment form and selects in-place lowering. This is compiler-source-dependent behavior, prohibited by `.ai/man-content.md` §3, and it also contains a grammatical error (“shape — is”).
SUGGESTED: Delete the sentence. The preceding sentence already gives the complete developer-visible contract: `remove` leaves the input value unchanged and returns the result.