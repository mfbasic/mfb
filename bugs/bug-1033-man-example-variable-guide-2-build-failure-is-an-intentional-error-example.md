# bug-1033: the man example `variable::guide#2` does not build

Last updated: 2026-10-10
Effort: small (<1h)
Severity: LOW
Class: Docs / tooling

Status: Open
Regression Test: (proposed) `scripts/man-run-examples.sh variable --topic --run` reports `failed: 0`

The man example runner reports one failure for the `variable` guide.

**Correct behavior:** the runner reports `failed: 0`.

## Failing Reproduction

Measured on 2026-10-10 on main (3fa35d543, macOS aarch64):

```
MFB=/tmp/main-dbg-target/release/mfb scripts/man-run-examples.sh variable --topic --run
```

ends with `examples: 12   built: 11   ran: 11   not run: 0   failed: 1` and `failures: variable::guide#2(build)`.
The example is the first program in `src/docs/man/variable/package.md` under "A name declared inside a function cannot
reuse the name of a top-level `LET` or `MUT`":

```basic
IMPORT io

LET limit AS Integer = 5

FUNC main() AS Nothing
  LET limit AS Integer = 10
  io::print(toString(limit))
END FUNC
```

Diagnostic from the runner:

```
/tmp/man-examples/src/main.mfb:6 error[2-201-0022 SYMBOL_SHADOWS_TOP_LEVEL_BINDING]: local binding reuses the name of a visible top-level binding
               `limit` is already a top-level binding (declared at src/main.mfb:3); a local cannot reuse its name.
```

## What is known

The page itself says "This first program does not build; the compiler points at the inner `LET` and says `limit` is
already a top-level binding", so the diagnostic is the one the page promises. My judgement: this is an intentional
error example that the runner does not recognise, not a broken example. In `scripts/man-run-examples.sh` the only
allowance is for a program that RUNS, exits non-zero and prints output the page shows (the `documented` check around
the run step); I found no allowance for a build failure the page documents. The fix is probably in the runner (or a
marker for expected-build-failure examples), not the page; that is a guess.

Found by the bug-1013 agent while fixing bug-1013 (batch 19).
