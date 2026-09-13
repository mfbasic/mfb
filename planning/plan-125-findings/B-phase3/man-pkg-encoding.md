### 1. Decoder error contracts are not consistently discoverable
UNIT:      man-pkg:encoding  
PAGE:      package-wide  
CATEGORY:  consistency  
CLAIM:     The overview says, “Decoders reject malformed input with `ErrInvalidFormat` (`77050003`).”  
VERDICT:   inconsistent  
EVIDENCE:  `mfb man encoding percentDecode` and `mfb man encoding formUrlDecode` render no **Errors** section, although their descriptions say malformed input raises an error. `rg -n 'errors: vec!' src/codegen/builtins/encoding/func_*.rs` shows both descriptors declare `errors: vec![]`; their bodies call the shared rejecting decoder. The probe built and run with `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb` printed `percent=77050003` and `form=77050003` for `percentDecode("%")` and `formUrlDecode("%")`, respectively. In contrast, `codepageDecode` renders an `ErrInvalidFormat` table and declares that error.  
SUGGESTED: Add `ErrInvalidFormat` to every decoder descriptor that can raise it, beginning with `percentDecode` and `formUrlDecode`, so each page’s derived Errors table gives the actionable catchable error promised by the package overview.