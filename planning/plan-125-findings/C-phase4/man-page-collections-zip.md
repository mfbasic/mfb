### 1. Undocumented out-of-memory error
UNIT:      man-page:collections/zip
CLAIM:     "`zip` invokes no user callback and raises no error."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/collections/func_zip.rs:CodeBuilder::lower_list_zip_fixed` allocates the result and calls `raise_error_bare("ErrOutOfMemory")` on failure; `func_zip.rs:register` nevertheless declares `errors: vec![]`, leaving the rendered Errors table empty. The release-binary edge probe printed `z=(1,10);n=1`, `empty=0`, `after=(1,10);a=1;b=10`, and `mixed=2:t:f`, confirming normal truncation, empty-input, input-preservation, and independent-type behavior.
SUGGESTED: "`zip` invokes no user callback. It raises `ErrOutOfMemory` if it cannot create the result list."