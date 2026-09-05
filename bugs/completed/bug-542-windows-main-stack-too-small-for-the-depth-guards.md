# bug-542: the front end's 256-deep guards do not fit Windows' 1 MiB main-thread stack

Last updated: 2026-09-04
Effort: small (<1h)
Severity: HIGH (Windows CI red; a legal program fails to compile on Windows)
Class: correctness / host portability

Status: Fixed (2026-09-04) — `fn main` (`src/main.rs`) spawns the compile on a
thread sized by `COMPILER_STACK_BYTES` (64 MiB) instead of running it on
whatever stack the host reserved for `main`, and exits 101 if that thread
panics so a panicking `mfb` is indistinguishable from before. Regression tests:
`tests/cli_parse_expression_tree_depth.rs::{deepest_admitted_expression_compiles_on_a_1mb_main_stack,
hostile_expression_is_diagnosed_on_a_1mb_main_stack}` drive the real binary
under `sh -c 'ulimit -s 1024 && exec …'` — the Windows main-thread size — so the
Windows-only fault is now reproducible on every Unix row.

`Test (windows-x86_64)` was the only red row of six
(https://github.com/mfbasic/mfb/actions/runs/33936179556):

```
---- nested_groups_of_short_chains_are_rejected_cleanly stdout ----
250 groups of 20-term chains: mfb must exit 1 with a diagnostic, not die by signal.
status: exit code: 0xc00000fd
stderr: thread 'main' (3056) has overflowed its stack
```

Trigger: `mfb build` of any expression that nests ~250 levels. The parser
recurses once per grouping level through the whole precedence chain, so ~250
levels is ~4 KB × 250 of native stack — fine in 8 MiB, over the edge in 1 MiB.

Root cause: NOT the bug-501 guard, which is doing its job. Every front-end depth
guard admits a tree 256 levels deep (`ast::expr::MAX_EXPR_DEPTH`, `ast::stmt`'s
block cap, `parse_type_name`'s type cap, all matched to
`ir::verify::check_value_depth`), and every pass after the parser walks that tree
recursively. The cap was calibrated against the **8 MiB** stack Linux and macOS
hand `main`; **Windows reserves 1 MiB**, and 256 levels do not fit in it.

This was never only a hostile-input problem. Measured on macOS with the pre-fix
release binary under `ulimit -s 1024` (the Windows main-thread size):

| input | pre-fix | post-fix |
| --- | --- | --- |
| 250 groups × 20-term chains (tree depth ~5 000 — must be REJECTED) | `exit=134`, stack overflow, no diagnostic | `exit=1`, `Expression nesting is too deep.` |
| 250 groups × 1 term (tree depth 250, under the cap — must COMPILE) | `exit=134`, stack overflow | `exit=0` |

So a **legal** program the language admits could not be compiled on Windows at
all; the red test was just the first shape to notice.

Fix: the guards define the language surface, so the stack is what moves. The
compiler runs on a thread whose stack it chooses (64 MiB — the size
`ast::expr::tests::on_big_stack` has always used, ~8x the headroom the two
passing platforms have). A thread stack is reserved address space, not committed
memory, on every supported host, and its size is independent of `RLIMIT_STACK`.

Rule for later: a recursion cap is only honest if the compiler HAS the stack that
cap costs on the SMALLEST host stack. Raising a depth cap, or adding a pass that
recurses per tree level, is budgeted against `COMPILER_STACK_BYTES` — never
against the host default. Written up in `.ai/arch-abi.md`, "The compiler's own
main-thread stack is 1 MiB on Windows, 8 MiB elsewhere".
