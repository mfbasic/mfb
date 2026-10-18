# bug-1105: two wall-clock canvas tests fail under the full suite's load and pass alone

Last updated: 2026-10-18
Effort: small (<1h)
Severity: LOW
Class: Test (flaky under load)

Status: Open
Regression Test: the two tests themselves, under `scripts/test-full.sh`

Found by the bug-staging-24 coordinator: the full tier on main `7bb985ed8`
(`~/.cache/mfb-full-suite/runs/7bb985ed8eae.result`, exit 101, 355 binaries reached) failed
two tests; the full tier on the previous main `0dc51751f` passed both.

| test | assertion |
|---|---|
| `rt_canvas_autopaint::the_project_option_sets_how_long_a_flash_lasts` | `left: ["1", "3"]`, `right: ["1"]` (the paint held for `canvasAutoPaintMS` 10000 had moved on by the end of a 1500 ms sleep) |
| `rt_canvas_pointer::the_graphics_thread_reads_each_scripted_pointer_state` | `left: ["10,20,0,1", "0,0,0,0", "0,0,0,0"]`, `right: ["10,20,0,1", "30,40,1,1", "0,0,0,0"]` (one scripted pointer state was never read) |

Both decide on wall-clock time. The run was at load average about 55 (the full suite
beside a coordinator gate and agent builds); bs24-agent-a saw the autopaint test fail once
under a load of 30 to 60 and pass alone.

**Not reproduced alone:** each test run by name 5 times on `0dc51751f` and 5 times on
bug-staging-24 (which holds the canvas changes of `7bb985ed8`), load average 17 to 69:
10 of 10 passes on each (`/tmp/bs24-flake.sh`). So no change of that batch is shown to
cause it; what is left is the tests' timing margin under the suite's parallel binaries.

**The single correct behavior a fix produces:** both tests pass in every full-tier run
(they decide on the program's own frame/state sequence, not on how long a sleep lasted),
or they wait on an event rather than a duration.
