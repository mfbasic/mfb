### 1. MapEntry has no discoverable destination

UNIT:      man-topic:types  
PAGE:      package-wide  
CATEGORY:  discoverability  
CLAIM:     "`MapEntry OF K TO V` — the built-in record produced by `FOR EACH` over a map"  
VERDICT:   missing  
EVIDENCE:  `mfb man types --all` lists no MapEntry topic; `mfb man types mapentry` prints `error: unknown types topic page 'mapentry'`. `rg -n -F 'mfb man flow forEach' src/docs/man/types` prints nothing, although `mfb man flow forEach` renders the MapEntry loop-variable contract.  
SUGGESTED: Add `mfb man flow forEach` to the overview’s MapEntry bullet and/or See also list: “For map-loop syntax and `entry.key`/`entry.value`, see `mfb man flow forEach`.”

### 2. User-defined type declarations are announced but not covered

UNIT:      man-topic:types  
PAGE:      package-wide  
CATEGORY:  coverage  
CLAIM:     “User-defined `TYPE`, `UNION`, `ENUM`, and package-scope `RESOURCE … CLOSE BY` declarations … create additional program types”  
VERDICT:   missing  
EVIDENCE:  `mfb man types --all` renders nine child pages—comparisons, list, logical, map, numeric, pair, partition, set, and string—none covering declaration forms. `rg -n -F 'mfb man variable' src/docs/man/types` finds only resource/copy-model references, not a route from the declaration claim. `mfb man variable --all` does render a `TYPE Point` example under “Changing a record: WITH,” confirming it is the closest existing developer-facing destination.  
SUGGESTED: Add a concise “Defining types” overview subsection with minimal `TYPE`/`UNION`/`ENUM` examples and link `mfb man variable`; also state where developers should learn package-scope `RESOURCE … CLOSE BY` declarations.