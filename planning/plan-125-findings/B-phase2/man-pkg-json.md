### 1. Overview hides pretty-printing
UNIT:      man-pkg:json  
PAGE:      package-wide  
CATEGORY:  overview-mismatch  
CLAIM:     “`json::stringify` renders a `json::Json` value back into compact JSON text” and “Serialization is compact”  
VERDICT:   misleading  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man json stringify` renders “Indented output” and documents both Integer and String `indent` overloads. The scratch probe built with `mfb build /tmp/plan-125-scratch/B-phase2/man-pkg-json/probe` printed both `{"a":1}` and a multi-line, two-space-indented form for `json::stringify(doc, 2)`. `src/codegen/builtins/json/func_stringify.rs:register` registers both overloads.  
SUGGESTED:  Say “`json::stringify` renders a `json::Json` value as compact JSON text by default, or as indented JSON when given an indent argument.” Change “Serialization is compact” to “By default, serialization is compact; see `json::stringify` for indented output.”

### 2. Overview initially describes paths as object-only
UNIT:      man-pkg:json  
PAGE:      package-wide  
CATEGORY:  consistency  
CLAIM:     “`json::get` and `json::getOr` walk a path of object keys to a nested member.”  
VERDICT:   inconsistent  
EVIDENCE:  The same rendered overview later says a path step is “an object key on a `json::JsonObj` and a zero-based decimal index on a `json::JsonArr`.” `src/codegen/builtins/json/func_get.rs:FUNC_BODY` and `func_get_or.rs:FUNC_BODY` each have a `JsonArr` arm that converts the path token with `__json_arrayIndex`; the scratch probe’s `json::getOr` with a malformed array index returned `"fallback"` rather than treating arrays as unsupported.  
SUGGESTED:  Say “`json::get` and `json::getOr` walk a path of object keys and array indexes to a nested member.”