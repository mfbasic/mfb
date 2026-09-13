### 1. Broken route to the pattern-language reference
UNIT:      man-pkg:regex  
PAGE:      package-wide  
CATEGORY:  discoverability  
CLAIM:     “For the full pattern language, run `mfb man regex language`.”  
VERDICT:   wrong  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man regex language` printed `error: unknown regex function 'language'`; the claim originates in `src/codegen/builtins/regex/mod.rs:DESC`.  
SUGGESTED: Replace with: “For the complete pattern syntax and semantics, run `mfb spec stdlib regex`.” Or add and render the promised `regex language` man page.

### 2. Overview leaves `count` out of the `start`-argument guidance
UNIT:      man-pkg:regex  
PAGE:      package-wide  
CATEGORY:  consistency  
CLAIM:     “`find`, `findAll`, `findMatch` and `findAllMatches` take an optional `start` (default `0`)…”  
VERDICT:   misleading  
EVIDENCE:  `mfb man regex count` renders `regex::count(value AS String, pattern AS String, [start AS Integer]) AS Integer` and documents `start`; its registry descriptor is `src/codegen/builtins/regex/func_count.rs:register`.  
SUGGESTED: Include `count` in that list: “`count`, `find`, `findAll`, `findMatch`, and `findAllMatches` take an optional `start`…”