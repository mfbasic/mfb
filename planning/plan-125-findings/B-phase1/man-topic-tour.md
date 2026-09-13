### 1. First-program execution is not discoverable

UNIT:      man-topic:tour  
PAGE:      package-wide  
CATEGORY:  coverage  
CLAIM:     The tour shows a complete `SUB main()` program but never tells a terminal user how to run or build it, nor directs them to `mfb man tooling`.  
VERDICT:   missing  
EVIDENCE:  `sed -n '1,250p' src/docs/man/tour/package.md` shows the overview’s only “More” routes are `types`, `collections`, `flow`, `lambda`, `errors`, `variable`, `fs`, and `thread`; its final navigation lists the package index and compiler specifications, but not `tooling`. The rendered overview command `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man tour` produced the same routes.  
SUGGESTED: Add a short route immediately after “Hello, world,” such as: “Run it with `mfb run <file>`; for building, testing, and project commands, see `mfb man tooling`.”

### 2. Comparison pages inconsistently route readers away from the value model

UNIT:      man-topic:tour  
PAGE:      package-wide  
CATEGORY:  discoverability  
CLAIM:     The C, Java, and Go comparison pages direct readers to `mfb man variable`, while the TypeScript and Python pages—which teach the same value, cleanup, and message-passing model—do not.  
VERDICT:   inconsistent  
EVIDENCE:  `rg -n "mfb man variable|Where to go next" src/docs/man/tour` shows `01_c.md`, `02_java.md`, and `03_go.md` link `mfb man variable`; `04_typescript.md` and `05_python.md` do not. `.ai/man-content.md` §4.5 identifies `mfb man variable` as the one page explaining the value model end to end and requires other topics to link it rather than restate it.  
SUGGESTED: Add `mfb man variable` to the “Where to go next” lists for the TypeScript and Python pages, and make the comparison-page routing consistent.