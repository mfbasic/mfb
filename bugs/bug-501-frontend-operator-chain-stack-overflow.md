# bug-501: a left-associative operator / postfix-member chain bypasses MAX_EXPR_DEPTH → compiler stack overflow (SIGABRT)

Last updated: 2026-09-03
Effort: small (<1h)
Severity: HIGH
Class: security (denial of service — compiler crash on hostile source)

Status: Open (found in audit-3, Surface 2 FE-01; reproduced live by the lead)

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

## Prior art

bug-171-A, bug-220 (`bugs/completed/`) fixed adjacent depth halves; audit-2
FE-02/FE-03 (bug-182/183) are fixed (monomorph + statement depth). This
operator/member-chain axis is the remaining uncovered one (searched
`MAX_EXPR_DEPTH`, `expr depth`, `stack overflow`, `left-assoc`).
