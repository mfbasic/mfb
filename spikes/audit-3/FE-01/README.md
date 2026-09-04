# FE-01 spike — compiler stack overflow on a left-associative operator chain

audit-3 FE-01 (`planning/audit-3-frontend.md`), bug-501. `MAX_EXPR_DEPTH` bounds
right-recursion but not a left-associative operator chain (or a postfix-member
chain), so a flat `1+1+1+…` overflows the native stack in the parser/lowerer.

```
python3 gen.py > /tmp/fe01/src/main.mfb   # ~40 KB
mfb build /tmp/fe01
```

## Observed (defect present)

```
thread 'main' has overflowed its stack
fatal runtime error: stack overflow, aborting        # SIGABRT
```

Reachable by `mfb build`, `mfb fmt`, and `mfb audit` on a hostile source file
(a PR, a downloaded package, an editor-on-save).

## Expected

A clean `MFB_EXPR_TOO_DEEP`-style diagnostic, as the right-recursive form already
produces.
