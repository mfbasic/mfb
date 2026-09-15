### 1. Float errors are omitted and contradicted

UNIT:      man-page:collections/add  
CLAIM:     "`add` is **infallible**: nothing it does raises a trappable domain error, so an inline `TRAP` written on an `add` call has a dead handler."  
VERDICT:   wrong  
EVIDENCE:  `src/codegen/builtins/collections/func_add.rs:lower_add` calls `builder.observe_float_vr(&item)`, but its descriptor declares `errors: vec![]`, so the rendered page has no Errors table. Scratch probe `/tmp/plan-125-scratch/C-phase4/man-page-collections-add/src/main.mfb`, built with the requested release binary, calls `collections::add` with `big * big` as a `Float` item; its function-level handler printed `caught=77050015`. The direct unhandled probe with `(big * big) / (big * big)` printed `Error: 7-705-0013 Floating-point operation produced a NaN result.`  
SUGGESTED: A `Float` item produced by arithmetic must be finite: a NaN raises `ErrFloatNaN`, and infinity raises `ErrFloatOverflow`. Register both errors so the Errors table renders them.

### 2. Compiler-strategy performance promise

UNIT:      man-page:collections/add  
CLAIM:     "Assigning straight back to the same variable — `set = collections::add(set, x)` — is the cheap shape: it updates the set rather than building a second one."  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/collection/assign/builder_inplace_assign.rs:try_inplace_set_add_assign` implements this only for a qualifying `MUT` local and excludes cases such as a live `FOR EACH`; otherwise `src/codegen/builtins/collections/func_add.rs:lower_add` copies the set. This is compiler optimization detail, barred by `.ai/man-content.md` §3.  
SUGGESTED: “You can assign the returned set back to the same variable: `set = collections::add(set, x)`.”