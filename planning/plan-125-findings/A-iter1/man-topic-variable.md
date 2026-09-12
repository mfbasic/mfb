### 1. Thread transfer is an undisclosed exception to the handle model
UNIT:      man-topic:variable  
PAGE:      package-wide  
CATEGORY:  coverage  
CLAIM:     “A handle is not copied — a second name is an alias for the same open thing.” / “it can be handed to another thread — and the rules above hold in every one of those places”  
VERDICT:   misleading  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man thread transfer` says `thread::transfer` “takes the handle” and that after success “you cannot use it again”; `src/codegen/builtins/thread/func_transfer.rs` gives the same contract.  
SUGGESTED: Add an explicit exception: “A `RES` parameter is an alias, but `thread::transfer` hands the handle to the other thread; after a successful transfer the sending name cannot be used. See `mfb man thread transfer`.” Include that exact page in “Where to look next.”

### 2. “WITH is the only way” conflicts with resource-state record updates
UNIT:      man-topic:variable  
PAGE:      package-wide  
CATEGORY:  consistency  
CLAIM:     “WITH is the only way to update a record’s fields.”  
VERDICT:   misleading  
EVIDENCE:  `src/docs/spec/language/04_types.md` documents the exception `resource.state = value` and `resource.state.field = value`; `src/codegen/engine/control/builder_control.rs::emit_state_assign` implements those forms. The rendered `mfb man types --all` likewise states that a `RES` binding’s `STATE` payload is the exception.  
SUGGESTED: Qualify the statement: “For ordinary record values, `WITH` is the only way to update fields. A resource’s `STATE` payload has its own update form.”