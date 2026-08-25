# optimizations

The `-O` optimization dial and the passes it enables

## Synopsis

```
mfb build -O <level> [path]     ' 0 off, 1 default, 2-3 enable more passes
mfb build -v ...                ' verbose: per-pass fire counts after codegen
mfb man optimizations
```

## Imports

`optimizations` is a documentation topic for the compiler's optimizer, not an
importable package. No `IMPORT` is needed; the dial is selected on the `mfb
build` / `mfb test` command line with `-O <level>` (also spelled `-O0`..`-O3`
or `--optimize <level>`).

## Description

The optimizer is organized as a numeric dial. Every level from `-O0` to the
maximum is **behavior-preserving by contract**: changing the level changes the
emitted code — never the program's observable results, and never *whether or
where* a runtime error such as `ErrOverflow` is raised. MFBASIC's integer
arithmetic is checked and its errors carry precise source locations, so a pass
may only rewrite or remove code when the replacement provably computes the same
values *and* raises the same errors. A transformation that cannot meet that bar
does not run at any dial level.

| Level | Flag | Character |
| --- | --- | --- |
| 0 | always on | Mandatory lowering the language requires; not gated by the dial. |
| 1 | `-O1` (default) | Transparent local rewrites — same operations, less waste. |
| 2 | `-O2` | Tidying — provably useless code is removed. |
| 3 | `-O3` | Restructuring — dead control structure is removed too. |

`-O0` turns every dial pass off (a correct but unoptimized build); `-O1` is the
default. Higher levels are cumulative: `-O3` enables everything below it.

With `-v`, `mfb build` prints one `<pass>: <count>` line per enabled pass after
codegen, reporting how many times it actually fired.

## Passes

Passes on the dial, in pipeline order. *Stage* says where the pass runs: `NIR`
(the structured native IR, before storage planning), `MIR` (the selected
machine-neutral stream, before register allocation), or `machine` (after
register allocation, on physical registers).

{{optimizer-catalog}}

Dead code (reachable but useless) is distinct from *unreachable* code (code
control flow can never arrive at); the two are separate future-and-present
passes because they need different proofs — removing dead code must show it
cannot trap, while unreachable code can never trap because it never runs.

## Always-on lowering (Level 0)

Some rewrites look like optimizations but are part of the language's meaning,
so they run at every level including `-O0`:

- **FMA contraction** — `a * b ± c` fuses to a single rounding, which is the
  specified Float behavior; gating it would make `-O0` produce different float
  results.
- **Most-negative-literal folding** — `-9223372036854775808` folds into a
  single literal; unfolded it would overflow at runtime.
- **Branch relaxation** — out-of-range conditional branches get veneers so
  large functions assemble at all.
- **Instruction selection and register allocation** — the backbone every build
  needs; only their optional refinements ride the dial.

## Errors

No errors. `-O` with an unknown level is rejected on the command line with the
list of available levels; the dial never changes which runtime errors a program
raises — that is the point.

## See also

- `mfb build --help` — the `-O` and `-v` flags
- `mfb man errors` — the runtime error model the optimizer must preserve
- `mfb spec language types` — checked arithmetic and Float observation
  boundaries (§4.1), the semantics the dial's contract is built on
