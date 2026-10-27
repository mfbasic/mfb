# bug-1131: a dropped `RES Thread` handle can still count against `maxThreads`, so a later `thread::start` fails under load

Last updated: 2026-10-26
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Runtime (threads, race)

Status: Open
Regression Test: — (the flaky test below becomes deterministic once fixed)

A program that starts a thread, drops the only handle to it, and starts another,
in a loop, can fail with `7-705-0029` "10 threads are already running; raise
options.maxThreads in project.json to allow more". The cap is only 10 live
handles, but at no point does the program hold more than one. The failure
needs machine load, so it shows as a flaky test.

**The correct behavior a fix produces:** once a `RES Thread` handle is closed
(dropped at end of scope, or replaced), it no longer counts toward
`options.maxThreads`, regardless of how quickly the OS thread itself exits.

## Failing Reproduction

`cargo test --workspace --no-fail-fast` on `main`-equivalent merge `38b689a5a`
(2026-10-26, full suite under load): one failure,

```
rt_scope_drop_leaks  loops_and_condition_temps::a_resource_temporary_in_a_container_is_closed
b695_thread_pool_100 failed:
Error:    7-705-0029
Message:  10 threads are already running; raise options.maxThreads in project.json to allow more
Location: src/main.mfb:15:38
```

The same test passed 3/3 run alone
(`cargo test --test rt_scope_drop_leaks a_resource_temporary_in_a_container_is_closed`).

Shape (`b695_thread_pool` in
`tests/runtime/rt_scope_drop_leaks/loops_and_condition_temps.rs`, N = 100):

```
WHILE i < N
  MUT pool AS List OF RES Thread OF Integer TO Integer = []
  pool = collections::append(pool, thread::start(work, 1))
  total = total + len(pool)
  i = i + 1
END WHILE
```

## Root Cause

Not yet traced (guess, from the symptom): the running-thread count is decremented
when the OS thread finishes or is reaped, not when the handle is closed, so a
closed-but-still-exiting thread holds a slot. Under load ten such threads overlap.
Phase 1: find where `thread::start` checks the cap and where the count is released.

## Goal

- Closing a handle releases its slot synchronously (or the cap counts open handles),
  so the loop above never fails at any load.
- No change to the error for a program that really holds more than `maxThreads` handles.

## Validation Plan

- Make the race deterministic: a test whose worker sleeps after returning its
  value (or a debug hook that delays thread exit), looped past `maxThreads`.
- Full `cargo test --workspace --no-fail-fast`.
- Spec: `mfb spec` thread topic if the slot-release rule is not already stated.
