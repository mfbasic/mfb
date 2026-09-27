### 1. Resource handles violate the claimed isolation

UNIT:      man-page:collections/chunks  
CLAIM:     “Each block is a new list holding its own copy of the corresponding elements, and bulk copy, so the elements are copied into new lists and nothing you do with a block affects `value`.”  
VERDICT:   misleading  
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase4/man-page-collections-chunks/examples/src/main.mfb` compiled with the specified release binary. It chunks `List OF RES fs::File`, closes the file obtained from a returned block, then reads through the original `f`; output was `Error: 7-703-0004 Resource handle is already closed.` The descriptor accepts `List OF T` without an element constraint at `src/codegen/builtins/collections/func_chunks.rs:411-425`.  
SUGGESTED: Each block is a new list. For ordinary values, it contains copies of the corresponding elements, so changing a returned block does not change `value`. If `T` is a `RES` handle, the returned block contains an alias of the same open resource; closing either handle closes it.