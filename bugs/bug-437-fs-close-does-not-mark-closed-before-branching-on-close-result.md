<!-- Bug document. See .claude/skills/write-bug/template.md -->

# bug-437: `fs::close` stores the CLOSED flag *after* branching on the close result — a failed close leaves `CLOSED=0` and permits a double-close of a since-reused fd (regression of bug-63 item 3, all four targets)

Last updated: 2026-08-08
Effort: unknown (needs triage — real codegen regression vs. stale assertion)
Severity: MEDIUM
Class: Correctness (resource hygiene — double-close of a reused fd)

Status: Open (pre-existing; discovered while landing bug-436, NOT caused by it)
Regression Test: tests/rt_fs_error_path_hygiene.rs (already present — currently RED)

The codegen-inspection test `rt_fs_error_path_hygiene` asserts (item 3,
`assert_close_marked_before_branch`) that `_mfb_rt_fs_fs_close` stores the CLOSED
flag (`str_u64` at `FILE_OFFSET_CLOSED == 8`, base = the File pointer, not `sp`)
**between** the `close` syscall and the branch to `..._close_error`. On the
current tree the store sits *after* that branch (only on the success
fall-through), so a `close` that returns an error leaves `CLOSED=0` — exactly the
bug-63 item-3 defect that this test was written to guard. A subsequent scope-drop
or explicit close then double-closes a file descriptor that the OS may have
already handed to another open file.

This is a **pre-existing** failure: it reproduces identically on base commit
`03309dd8a` (verified via `git worktree add --detach`), so it predates bug-436 and
is unrelated to that fix (whose diff is `src/binary_repr/sections.rs` only). It
was surfaced because bug-436's finalization ran the full integration suite.

## Failing Reproduction

```sh
cargo build --release --bin mfb
cargo test --test rt_fs_error_path_hygiene --no-fail-fast
```

Observed — all four targets fail:

```text
test error_path_hygiene_macos_aarch64 ... FAILED
test error_path_hygiene_linux_aarch64 ... FAILED
test error_path_hygiene_linux_riscv64 ... FAILED
test error_path_hygiene_linux_x86_64 ... FAILED

macos-aarch64: fs::close does not store CLOSED before branching on the close
result (bug-63 item 3): a failed close would leave CLOSED=0 and permit a
double-close of a since-reused fd
```

## Root Cause (hypothesis — confirm before fixing)

The `fs::close` runtime helper (`_mfb_rt_fs_fs_close`) emits the CLOSED-flag store
on the success path only, after the `close`-result branch, rather than
unconditionally before the branch. Locate the emitter (search the fs runtime
codegen for `fs_close` / `FILE_OFFSET_CLOSED` / the `_close_error` label) and move
the `str_u64 [File+8] = 1` ahead of the branch. Because the assertion is
target-agnostic (`b.lt` on aarch64/x86, fused `rv.br` on riscv64), one fix in the
shared lowering should green all four.

**Triage first:** confirm this is a genuine codegen regression and not a test that
drifted from an intentional restructuring. `git log -S` the store/label and the
bug-63 fix; a fix must correct the codegen (the test protects a real double-free),
not re-baseline the assertion.

## Blast Radius

- The fs `close` runtime-helper emitter (shared codegen).
- `.ncode` of any fixture exercising `fs::close` — regenerate + inspect the delta
  (should be a single instruction reorder in `_mfb_rt_fs_fs_close`).

## Summary

`fs::close` marks the File CLOSED only on the success fall-through, after
branching on the close result, so a failed close leaves `CLOSED=0` and a later
drop double-closes a possibly-reused fd. The guard test `rt_fs_error_path_hygiene`
(bug-63 item 3) is RED on all four targets on the current tree. Pre-existing;
found during bug-436 finalization.
