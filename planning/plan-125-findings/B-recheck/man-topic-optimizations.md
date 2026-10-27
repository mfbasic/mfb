### 1. Nothing on the page says what `-O` does to debugging
UNIT:      man-topic:optimizations
PAGE:      optimizations (Description / See also)
CATEGORY:  coverage
CLAIM:     The page never mentions `mfb dbg`, `--debug`, breakpoints or `-O0` as the debugging level; its See also lists only `mfb build --help`, `mfb test --help`, `mfb man errors` and `mfb spec language types`.
VERDICT:   missing
EVIDENCE:  `sed -n 42,62p src/docs/man/debug/package.md` prints "Build with `--debug -O0` before you debug … `-O0` keeps every function a call of its own. The default optimization level merges small functions into their callers, and a function that was merged away cannot be stopped in." `mfb dbg --help | grep -n -i -E "optim|-O"` line 92: "Function names and arenas come from a debug build (mfb build --debug -O0)". `grep -n -i -E "\-O[0-3]|optimi" src/docs/man/optimizations/package.md` has no hit for debug/dbg. The probe at `/tmp/plan-125-scratch/B-recheck/man-topic-optimizations/p1` built with `mfb build -v .` prints "Leaf function inlining: 1" at the default level, confirming the default does merge a helper away.
SUGGESTED: Add to the level table's `-O0` row or a short paragraph after it: "Use `-O0` when you will step through the program in `mfb dbg`: at `-O1` and above, a small helper may be merged into its caller (Leaf function inlining), so it can no longer be stopped in by name, and locals may live in registers rather than their own stack slots." Add `mfb man debug` to See also.

### 2. The Stage column is in compiler vocabulary the page never defines
UNIT:      man-topic:optimizations
PAGE:      optimizations (Passes table)
CATEGORY:  discoverability
CLAIM:     "*Stage* names the part of the build each pass works on." The column values are `NIR`, `MIR`, `NIR + MIR`, `Plan1`, `regalloc`, `codegen`, `machine`, none of which is explained anywhere on the page.
VERDICT:   misleading
EVIDENCE:  `mfb man optimizations --all` Passes table (Stage column); `grep -n 'stage: "' src/optimizer/catalog.rs | sort -u` yields exactly those seven labels. `.ai/man-content.md` §3 bans "NIR, IR, .ncode, regalloc, or any codegen mechanics" from a page. The stage names the developer actually sees from `-v` are different: `mfb build -O0 -v .` prints "phase parse", "phase resolve", "phase verify", "codegen: lowering module", "codegen: planning + regalloc", "codegen: emitting native code", "phase codegen+link" — no "NIR", "MIR" or "Plan1" appears there, so the developer cannot map the column onto anything they can observe.
SUGGESTED: Either add a four-line legend under the sentence that introduces *Stage* in developer terms ("NIR — your program's statements, before storage is planned; MIR — the chosen instruction stream, before registers are assigned; Plan1 — data layout; regalloc — register assignment; machine — the final instructions; codegen — the String lowering step"), or rename the labels in `catalog.rs:Row.stage` to plain words ("statements", "instructions", "data layout", "registers", "final code"). The legend should also say that the `-v` "phase …" lines group all of them under `codegen+link`.

### 3. Loop rotation row uses a loop form the language does not have
UNIT:      man-topic:optimizations
PAGE:      optimizations (Passes table, "Loop rotation")
CATEGORY:  consistency
CLAIM:     "Converts a head-tested WHILE into the guarded bottom-tested form (IF c THEN DO .. UNTIL NOT c)"
VERDICT:   inconsistent
EVIDENCE:  `mfb man flow --all | grep -n -i UNTIL` lines 68-70 and 80: the post-test form is `DO … LOOP UNTIL condition`; there is no `DO .. UNTIL`. Source: `src/optimizer/catalog.rs` row "Loop rotation".
SUGGESTED: "(IF c THEN DO … LOOP UNTIL NOT c)".

### 4. Level table's one-word characters do not describe what the levels contain
UNIT:      man-topic:optimizations
PAGE:      optimizations (Description, level table)
CATEGORY:  overview-mismatch
CLAIM:     "2 | `-O2` | Tidying across a whole function — values tracked through every path, and provably useless code removed." and "3 | `-O3` | Restructuring — dead control structure is removed too."
VERDICT:   misleading
EVIDENCE:  Rendered Passes table: Level 2 includes module-wide and non-function rows — "Dead global elimination", "Global constant propagation", "Read-only memory promotion" (NIR, private globals), "Alignment" (Plan1, data layout), and four regalloc rows. Level 3 is dominated by rows that are neither "restructuring" nor "dead control structure": "Overflow-check elimination", "Division guard elimination", "Bounds-check elimination", "Range-check propagation", "Redundant test elimination", "String concat fusion" (codegen), "Codepoint len() CSE", plus seven loop transforms; `mfb build -O3 -v .` on the probe prints 65 rows against 17 at `-O1`. A developer reading "Restructuring" would not guess `-O3` is where checked-arithmetic guards get removed, which is exactly the thing the Description's behavior-preservation paragraph makes them worry about.
SUGGESTED: "-O2: Whole-function and whole-module tidying — values tracked through every path, private globals folded or dropped, data laid out without padding, smarter register and stack-slot reuse." "-O3: Loop restructuring (hoisting, unswitching, fusion, rotation), removing redundant computations and stack traffic, and dropping overflow, division and index checks only where the operands' proven ranges make the check impossible to fail; dead control structure is removed too."

### 5. "future-and-present passes" leaks the planning catalog's status into the page
UNIT:      man-topic:optimizations
PAGE:      optimizations (Passes, paragraph after the table)
CATEGORY:  consistency
CLAIM:     "the two are separate future-and-present passes because they need different proofs"
VERDICT:   inconsistent
EVIDENCE:  Both rows are shipped and on the dial at the same level: the rendered table lists "Dead-code elimination (DCE, NIR)" at Level 1 and "Unreachable code elimination" at Level 1, and `mfb build -v .` on the probe prints "Dead-code elimination (DCE, NIR): 0" and "Unreachable code elimination: 11". The page's own Imports/Description say the catalog describes only what ships (`src/optimizer/catalog.rs` header: "this module describes only what ships"), so nothing on the page is "future".
SUGGESTED: "the two are separate passes because they need different proofs".
