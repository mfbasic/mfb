### 1. Multi-scalar dialect inputs are described inconsistently and inaccurately
UNIT:      man-pkg:csv  
PAGE:      package-wide  
CATEGORY:  consistency  
CLAIM:     “Each must be a non-empty single character” (parse, parseStream, and stringify), versus parse’s “only its first Unicode scalar is used.”  
VERDICT:   inconsistent  
EVIDENCE:  `src/codegen/builtins/csv/helper_first_code.rs:BODY` returns only the first scalar for parsing; `helper_stringify_row.rs:BODY` appends the entire delimiter string. Probe built and ran with `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb build /tmp/plan-125-scratch/B-phase3/man-pkg-csv`: `csv::parse("a,b", ",;")` and `parseStream` each printed two fields, while `csv::stringify([["a","b"]], ",;")` printed `a,;b`. Thus multi-scalar inputs are accepted, but parsing and serialization do not use them alike.  
SUGGESTED:  “`delimiter` and `quote` must be non-empty. Parsing uses their first Unicode scalar; serialization uses the supplied text. Use exactly one Unicode scalar for each when producing CSV that `csv::parse` can read back with the same dialect.”