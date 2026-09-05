# bug-501: a left-associative operator / postfix-member chain bypasses MAX_EXPR_DEPTH → compiler stack overflow (SIGABRT)

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (denial of service — compiler crash on hostile source)

Status: FIXED (56746e04b; see STATUS block at the end)

Regression Test: a `tests/syntax/` fixture with a long `+` chain asserting `MFB_EXPR_TOO_DEEP` (or equivalent), not a crash.

## Summary

The parser/lowerer's `MAX_EXPR_DEPTH` guard bounds right-recursive nesting but
not a *left*-associative operator chain (`1+1+1+…`) or a postfix-member chain,
which recurse on a different axis. A ~40 KB `.mfb` (2 KB in a release build)
overflows the native stack and aborts the compiler with SIGABRT — on `mfb build`,
`mfb fmt`, and `mfb audit` alike. Any of these is routinely run on untrusted
source (a PR, a downloaded package, an editor formatting on save).

## Mechanism

`src/ast/expr.rs:166-186` (binary-operator parse) and `:393-403` (postfix member)
build a left-leaning tree whose depth is not charged against `MAX_EXPR_DEPTH`; the
crash frame is the recursive lowering at `src/ir/lower.rs:3312`. bug-171-A and
bug-220 fixed the right-recursive and type-nesting halves; this left-associative
half was uncovered.

## Reproduction (lead-run, live)

`spikes/audit-3/FE-01/` — `python3 gen.py > /tmp/fe01/src/main.mfb; mfb build /tmp/fe01`:

```
thread 'main' has overflowed its stack
fatal runtime error: stack overflow, aborting
```

(40 076-byte source, a flat `1+1+…` chain.)

## Best fix

Charge left-associative operator chains and postfix-member chains against the same
depth budget the right-recursive path uses — either count chain length during the
iterative parse and emit `MFB_EXPR_TOO_DEEP` past the cap, or convert the deep
lowering recursion at `ir/lower.rs:3312` to an explicit worklist so depth is
heap-bounded. A clean diagnostic, never a crash.

## Non-goals

Do not lower the existing right-recursive cap; no language-surface change (a
legitimately deep expression already errors cleanly).

## Sub-issue B (found while fixing A): `|>` placeholder copies are exponential

`parse_pipeline` lowers `left |> right` by cloning `left` into EVERY `_` of
`right` (`substitute_placeholder`). A right-hand side with two placeholders
therefore doubles the tree per stage: measured on the pre-fix release binary,
`1 |> f(_, _)` × 12 stages = 6.5 s / 68 MB, × 14 = 19 s / 238 MB, × 16 = 100 s /
720 MB, × 20 did not finish in 10 minutes — from a 200-byte source. Distinct
mechanism from A (size, not depth: the tree stays shallow), same parser, same
hostile-source class.

Fix: charge `(placeholders − 1) × nodes(left)` against
`MAX_PIPELINE_COPIED_NODES` (4096) BEFORE cloning; a single `_` (the ordinary
form) is never charged. The doubling attack is refused at its 13th stage with a
located diagnostic.

## Prior art

bug-171-A, bug-220 (`bugs/completed/`) fixed adjacent depth halves; audit-2
FE-02/FE-03 (bug-182/183) are fixed (monomorph + statement depth). This
operator/member-chain axis is the remaining uncovered one (searched
`MAX_EXPR_DEPTH`, `expr depth`, `stack overflow`, `left-assoc`).

## STATUS: FIXED

Fixed in `56746e04b` (bug-501 (WIP): bound the BUILT expression tree depth in the
parser; charge |> placeholder copies), landed on `main` via the `worktree-B-501`
merge. Landing gates (after merging `main` at `90f6c1357`): the FE-01 spike exits 1
with the located diagnostic (pre-fix `main` binary: SIGABRT, exit 134);
`cargo test --no-fail-fast` green; `scripts/diag-set-diff.sh` 560 fixtures SAME;
`artifact-gate all` 1898 goldens, 0 diffs.

**Mechanism confirmed.** `MAX_EXPR_DEPTH` (`src/ast/expr.rs`, `enter_expr`) bounds
the parser's *recursion*, but the left-associative loops (`parse_or` …
`parse_multiplication`, `parse_member_access`) and `parse_pipeline` deepen the
*tree* by one per iteration without recursing. On the pre-fix release binary
(`mfb build` on `spikes/audit-3/FE-01`): 256 terms compiled, 300–2000 terms were
rejected cleanly by `ir::verify`'s 256-level backstop (`expression nesting
exceeds the 256 level limit`), 5 000+ overflowed the stack in the lowering
passes before verify ran (SIGABRT). A composite of 250 nested groups each
holding a 20-term chain (10.5 KB) also aborted — so charging chain LENGTH per
loop would not have fixed it; only the depth of the BUILT tree can see it.

**Fix (A).** `FileParser::expr_tree_depth` records the tree depth (0 = leaf,
`1 + max(children)`) of the expression just completed; every node-building site
in `expr.rs`/`stmt.rs` reports through `note_expr_tree_depth`, which emits
`MFB_PARSE_UNEXPECTED_TOKEN` / "Expression nesting is too deep." at the operator
that crosses `MAX_EXPR_DEPTH`, then latches `depth_exceeded` + `seek_to_end`
(the bug-183/191 recovery) so exactly one diagnostic renders. The convention
(root 0, reject > 256) is `ir::verify::check_value_depth`'s, so a 256-term chain
still compiles and nothing that verified before is rejected (pinned by
`tree_depth_guard_admits_a_chain_at_the_cap` and
`chain_at_the_cap_still_compiles`). For `|>`, `placeholder_shape` gives the
right-hand side's depth and its deepest `_`, so the spliced depth is exact.
The existing right-recursive cap is untouched.

**Fix (B).** See Sub-issue B above.

**Tests.** `src/ast/expr.rs` unit tests (`tree_depth_guard_*`,
`pipeline_placeholder_copy_budget`), `tests/cli_parse_expression_tree_depth.rs`
(real binary: 20 000-term chain, 250×20 composite, 20 000-member chain, 20-stage
`f(_, _)` pipeline → all `exit 1` with the located diagnostic; 256-term chain
still builds), and `tests/syntax/parser/parser_operator_chain_depth` (golden).
All RED on the pre-fix tree (SIGABRT / SIGTERM after 10 min / parse accepted),
GREEN after.

**Deviation from the doc.** `mfb fmt` is lexical and never parses, so it was
never reachable by this defect; `mfb build`/`mfb audit` were. No new rule code
was minted (rule codes race between sessions); the guard reuses the code the
existing expression-depth guard emits.
