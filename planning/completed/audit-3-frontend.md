# audit-3 — Surface 2: language front end + IR / optimizer

Part of `planning/goal-08-platform-security-review.md`. Finding prefix `FE-`
(lexer/parser/resolver/monomorph/fmt/audit: FE-01..06; IR verifier + optimizer:
FE-50..51). Untrusted party: the author of an arbitrary `.mfb` source (or a
`.mfp` package whose pre-compiled IR is decoded) that the compiler is asked to
build, format, or audit.

**Verdict: 3 HIGH · 3 MEDIUM (front end) + 2 MEDIUM (IR/opt). No memory-safety
finding.** The compiler is DoS-exposed on hostile source in several places the
prior depth-cap work did not reach; none reaches codegen with invalid IR (the IR
verifier is a solid security boundary — the key negative result below).

## HIGH

### FE-01 — left-associative operator / postfix-member chain overflows the compiler stack → **bug-501**

Lead-reproduced: a 40 KB `1+1+1+…` chain aborts `mfb build` with "stack overflow,
aborting" (SIGABRT); 2 KB suffices in release. `MAX_EXPR_DEPTH` bounds
right-recursion but not the left-associative axis (`src/ast/expr.rs:166-186`,
`:393-403`; crash frame `src/ir/lower.rs:3312`). Reachable via build/fmt/audit.

### FE-02 — `mfb fmt` has no nesting cap and rewrites non-atomically → **bug-502**

Quadratic indent blowup (336 KB → 512 MB; 1.3 MB → 8.2 GB/17 GB RSS; `--indent
256` → 61 GB) written back over the user's file with a non-atomic `fs::write`
(`src/fmt.rs:122-124`, `src/cli/fmt.rs:117-119`) — memory exhaustion plus source
destruction on OOM mid-write.

### FE-03 — diagnostic-stream amplification → **bug-505**

Uncapped diagnostic count + full-line echo + per-diagnostic full-file re-read →
O(errors×filesize), 240 KB source → 10.4 GB stderr in 6 s
(`src/rules/mod.rs:63-83`).

## MEDIUM

- **FE-04** — diagnostics echo raw source bytes → ANSI/BEL/bidi terminal
  injection; `terminal_safe::safe` is not applied at `src/rules/mod.rs:70-71`.
  (Same site and fix family as bug-505/bug-489; fold the sanitizer in there.)
- **FE-05** — a `.mfb`-named symlink makes build/fmt/audit read an arbitrary host
  file and print its line 1 in a diagnostic (`src/ast/manifest.rs:498-518` +
  `src/rules/mod.rs:63-71`) — a small information-disclosure primitive.
- **FE-06** — `mfb audit`'s fallibility fixpoint is O(N²): 1 MB `.mfb` → 88 s
  (`src/audit/collect/source.rs:519-534`).
- **FE-50** — superlinear (≥O(n²)) per-branch scope cloning in `ir::verify` →
  compile-time DoS on the mandatory (no-`-O`) path: the "verify rules" span alone
  grew 228 ms → 969 ms → 7962 ms for N = 800/1600/3200 branches
  (`src/ir/verify/ops.rs:21-22`, `resources.rs:104-117`).
- **FE-51** — level-3 store-to-load / PRE MIR dataflow superlinear → `-O3`
  compile-time DoS: N=8000 branches = 57–131 s at `-O3` vs 9.3 s at `-O2`
  (`src/optimizer/opt2/stldfwd.rs:43`, `opt2/plans/memory.rs:143`). LOW normally,
  MEDIUM on a `-O3` build service.

## Key negative result — the IR verifier holds

The IR verifier is the security boundary between "IR the trusted front end
produced" and "IR bytes an attacker wrote in a `.mfp`", and it is well constructed
against the PKG-02 type-confusion class. Every codegen/lowering assumption traced
against the verifier's rules is backstopped: permissive `Unknown` and
unresolved-`includes` member access are caught by `validate_nir`; an attacker
`resource_owners` is re-derived by `TYPE_RESOURCE_REQUIRES_RES`; closure-capture
bounds and the double-close/moved-flag no-op hold; decoder arithmetic is checked
(Surface 1). **Both** `verify_package` and `verify_semantics` run on the package
path (they are the sole backstop for an unsigned/local package — cross-ref
PKG-01). No memory-safety finding on the type/shape/resource axes.

One latent, non-demonstrated concern recorded: `opt1` runs before `validate_nir`,
so several `unreachable!("… checked the shape")` sites assume NIR structure
validated only afterward — a crash-DoS at most, not memory unsafety.

## Re-verified fixed

bug-182 (monomorph polymorphic recursion), bug-183 (statement depth), bug-399
(fan-out budget), bug-191 (type depth), bug-11/bug-144 (numeric literals), bug-24
(audit text). bug-171-A / bug-220 are **half**-fixed — FE-01 and FE-02 are their
uncovered halves.

## Bug docs filed

bug-501 (FE-01), bug-502 (FE-02), bug-505 (FE-03). FE-04/05/06 and FE-50/51 are
recorded here (FE-04 folds into the bug-505/bug-489 sanitize-at-print-site fix).

## Coverage

Read: `src/lexer.rs`, `src/numeric.rs`, `src/ast/expr.rs`/`stmt.rs`,
`src/rules/mod.rs`, `src/fmt.rs`, `src/cli/fmt.rs`, `src/monomorph/` (recursion
caps), `src/ir/verify/**` (the boundary — enumerated its rules), `src/optimizer/`
opt2 memory rows. Repros run under `/tmp/fe/`.

Gaps: `src/audit/collect/project.rs`/`lockfile.rs` skimmed; `src/fmt.rs`'s
DOC/LINK helpers tested behaviourally not line-read; a raw `.mfp` byte-crafting
fuzz of `decode_binary_repr → verify_semantics → lower` is recommended to the lead
(the type-confusion reachability here is structural, not run against a hand-built
malicious package — cross-ref Surface 1's fuzz, which covered the decoder but not
adversarial post-verify IR).
