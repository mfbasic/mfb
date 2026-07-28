# bug-171 — AST/front-end robustness nits (unbounded parse recursion, lossy serialize, unchecked token indexing, permissive DOC/manifest parsing)

Last updated: 2026-07-12
Severity: LOW (batch) — mostly latent / local-source robustness.
Class: Security / Correctness / Footgun (batched, same layer).
Status: FIXED (2026-07-13; goal: resolve bugs 170-180; full acceptance suite green)

## Findings

**A. Unbounded recursion depth in expression parsing → stack overflow.**
`src/ast/expr.rs:394-462` (`parse_primary` `(`/`[` arms; the
parse_or→…→parse_unary chain). Recursive descent with no depth counter: ~100k
nested `(` overflows the native stack (SIGSEGV) before a diagnostic. bug-89 only
closed a zero-progress loop, not depth. Input is local source → crashed compile,
not RCE. Fix: thread a depth counter and emit a "nesting too deep" diagnostic.

**B. `Function::to_json` drops the `isolated` flag (and `Param.line`).**
`src/ast/serialize.rs:644-708`. `ISOLATED FUNC f` and `FUNC f` serialize
byte-identically. No in-tree `.ast` reader, so lossy debug/golden dump, not a
round-trip break. Fix: emit an `"isolated"` field (guard goldens).

**C. `substitute_placeholder` omits the `Trapped` arm `contains_placeholder`
accepts.** `src/ast/serialize.rs:1492` vs `:1517-1616`. A `_` inside
`Expression::Trapped` would pass the pipeline "must contain `_`" check but not be
substituted. Latent (Trapped isn't produced inside a pipeline RHS today). Fix:
add a `Trapped` arm (or drop it from `contains_placeholder`) to keep the walkers
in lockstep.

**D. `peek`/`previous` index the token vector unchecked.**
`src/ast/parser.rs:264, 272`. `FileParser::new` with an empty or non-`Eof`-
terminated token vector panics (`self.tokens[self.current]` / `current - 1`
underflow). Safe only because the sole caller feeds `lexer::lex` output (always
`Eof`-terminated); no assertion enforces the invariant. Fix: assert `Eof`
terminator in `new`, or make the accessors saturating.

**E. `parse_header_signature` silently accepts an unterminated `(` in DOC
headers.** `src/ast/items.rs:1256-1300`. `FUNC query(Db, String` (no `)`) sets
`end = rest.len()` and treats the remainder as the param list with no diagnostic.
Fix: if `depth != 0` after the scan, report a malformed-signature diagnostic.

**F. Manifest glob super-linear backtracking / canonicalize-before-filter.**
`src/ast/manifest.rs:499-512` — `**` arm recurses
`match(remaining,path) || match(pattern,&path[1..])`, giving O(n^k) on patterns
with k `**` segments (local `project.json`, build-time slowdown). And `:429` —
`collect_mfb_files` `fs::canonicalize`s every entry *before* the extension/pattern
filter, so a broken symlink / EACCES entry anywhere under a source root aborts the
whole build (`MFB_SOURCE_READ_FAILED`). Fix: collapse/memoize `**`; filter by
extension/pattern before canonicalizing (or treat a canonicalize error on a
non-selected entry as skippable).
