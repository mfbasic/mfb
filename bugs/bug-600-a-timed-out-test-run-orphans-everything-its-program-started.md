# bug-600: a timed-out test run orphans everything its program started

Last updated: 2026-09-12
Effort: small
Severity: MEDIUM — a hung RED run leaves processes spinning on the host indefinitely, silently starving every later suite and gate
Class: Test infrastructure

Status: Open
Regression Test: `tests/runtime/rt_process_spawn_ambient_fds.rs::a_timed_out_run_leaves_no_descendant_behind`

## How it was found

The user found two processes that had been running since Sun Sep 6 on the dev host:

| pid | started | CPU time | %CPU | command | parent |
|---|---|---|---|---|---|
| 53808 | Sep 6 15:29 | 7239 min | 67% | `…/mfb_bug543_sigpipe_46918_0/fdprobe543 spam` | 53807, `sh -c '…fdprobe543' spam \| head -c 8 …; echo pipeline-done` → reparented to launchd |
| 54963 | Sep 6 14:35 | 7275 min | 62% | `/tmp/spawnprobe/spam` | launchd |

(`ps -axo pid,ppid,etime,time,%cpu,lstart,args`.) Together they held about 1.3 cores
for five days.

The first orphan is exactly the pipeline `spawned_child_dies_on_a_closed_pipe`
in `tests/runtime/rt_process_spawn_ambient_fds.rs` makes its MFB program run. On a RED
build the child inherits an ignored SIGPIPE, so `fdprobe543 spam` never dies when `head`
exits: that is the bug-543 defect the test exists to catch. The test gave up after its
30 s timeout, as designed. But `common::run_bounded` killed only the MFB program, so the
`sh` and the probe were reparented to init and kept writing forever.

The second (`/tmp/spawnprobe/spam`) is referenced nowhere in `tests/`, `src/` or
`scripts/`. It came from an ad-hoc probe run and is not a tree defect.

## Root cause

Both bounded-run helpers stop a timed-out child with `Child::kill()`, which signals the
direct child only:
- `tests/common/mod.rs::run_bounded_command`, which `run_bounded` and
  `run_bounded_without_inherited_fds` share;
- the bug-543 test's own copy, `run_with_ambient_fds`.

Any descendant of a program under test outlives the test: a `process::spawn`, a
`process::shell` pipeline, or a thread's child. On a RED run that descendant is often the
very process that hangs.

## Fix

- `common::own_process_group` starts the child as a process-group leader
  (`CommandExt::process_group(0)`).
- `common::kill_process_tree` sends `killpg(pid, SIGKILL)` to that group before the
  usual `kill` and `wait`.
- Both helpers use the pair.

A descendant that deliberately leaves the group (`setsid`/`setpgid`) is still out of
reach. No test in the tree does that: the one other `process_group(0)` user,
`rt_process_spawn_env_replace_bogus_environ`, applies it to its own direct child.

## Not changed

The two live orphans on the host are not this session's processes and are left for the
owner to kill.
