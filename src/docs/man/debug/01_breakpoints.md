# breakpoints

Stop the program when it reaches a function

## Description

A breakpoint stops the program the moment it enters a function. While it is
stopped you can look at how it got there (`mfb man debug backtraces`), then
let it run on to the next stop.

Build with `--debug -O0` first (`mfb man debug`). A breakpoint is set by the
function's name in the executable. That is `_mfb_fn_` plus the name you wrote,
and a `_` in your name is spelled `_5F`. lldb on macOS drops the leading `_`.

| To | lldb (macOS) | gdb (Linux, Windows) |
| --- | --- | --- |
| stop in `isPrime` | `breakpoint set --name mfb_fn_isPrime` (short: `b mfb_fn_isPrime`) | `break _mfb_fn_isPrime` |
| stop in `next_seed` | `b mfb_fn_next_5Fseed` | `break _mfb_fn_next_5Fseed` |
| stop in every function matching a pattern | `breakpoint set --func-regex 'mfb_fn_run'` | `rbreak ^_mfb_fn_run` |
| list breakpoints | `breakpoint list` | `info breakpoints` |
| start the program | `run` | `run` |
| run on to the next stop | `continue` (short: `c`) | `continue` (short: `c`) |
| remove every breakpoint | `breakpoint delete` | `delete` |
| leave | `quit` | `quit` |

A pattern breakpoint is the easiest way to stop in a function without typing
its full name. It also stops in a second version of the function that some
callers use (a name ending in `$own` and a number), which a breakpoint on the
exact name would miss.

If a breakpoint never stops, the function was never called. Two common
reasons: the input never reached it, or the build used the default
optimization level, which merges small functions like `next_seed` into their
callers. Rebuild with `-O0`.

## A session, step by step

Start the debugger on the example, with arguments `1 1000` (one round of 1000
numbers):

```
lldb -- examples/debug/build/debug.out 1 1000          # macOS
gdb --args examples/debug/build/debug-glibc.out 1 1000 # Linux
gdb --args examples\debug\build\debug.exe 1 1000       # Windows
```

Set two breakpoints and start the program. lldb shown, gdb in the comments:

```
(lldb) b mfb_fn_next_5Fseed     # gdb: break _mfb_fn_next_5Fseed
Breakpoint 1: where = debug.out`mfb_fn_next_5Fseed, address = 0x0000000100000888
(lldb) b mfb_fn_isPrime         # gdb: break _mfb_fn_isPrime
Breakpoint 2: where = debug.out`mfb_fn_isPrime, address = 0x0000000100000a5c
(lldb) run
Process 95760 stopped
    frame #0: 0x0000000100000888 debug.out`mfb_fn_next_5Fseed
```

The first stop is `next_seed`. `runRounds` calls it to pick the round's seed.
`continue` reaches the second call, from `runRound`, and the next `continue`
reaches the first `isPrime`:

```
(lldb) continue
    frame #0: 0x0000000100000888 debug.out`mfb_fn_next_5Fseed
(lldb) continue
    frame #0: 0x0000000100000a5c debug.out`mfb_fn_isPrime
```

`isPrime` runs once per number, so it would stop a thousand more times. Remove
the breakpoints and let the program finish:

```
(lldb) breakpoint delete
(lldb) continue
round 1: 75 primes
total: 75 primes
Process 95760 exited with status = 0 (0x00000000)
```

The debug report follows on stderr (`mfb man debug report`).

## Examples

The same session as a script, for each platform. Each builds the example,
sets both breakpoints, stops three times, removes the breakpoints and runs to
the end.

macOS:

```sh macos
mfb build --debug -O0 examples/debug
lldb --batch \
  -o 'breakpoint set --name mfb_fn_next_5Fseed' \
  -o 'breakpoint set --name mfb_fn_isPrime' \
  -o 'run' \
  -o 'continue' \
  -o 'continue' \
  -o 'breakpoint delete --force' \
  -o 'continue' \
  -- examples/debug/build/debug.out 1 1000
```

`--force` deletes every breakpoint without asking for confirmation (lldb's
`help breakpoint delete`).

Linux:

```sh linux
mfb build --debug -O0 examples/debug
gdb -batch \
  -ex 'break _mfb_fn_next_5Fseed' \
  -ex 'break _mfb_fn_isPrime' \
  -ex 'run' \
  -ex 'continue' \
  -ex 'continue' \
  -ex 'delete' \
  -ex 'continue' \
  --args examples/debug/build/debug-glibc.out 1 1000
```

Windows (PowerShell):

```powershell windows
mfb build --debug -O0 examples\debug
gdb -batch `
  -ex 'break _mfb_fn_next_5Fseed' `
  -ex 'break _mfb_fn_isPrime' `
  -ex 'run' `
  -ex 'continue' `
  -ex 'continue' `
  -ex 'delete' `
  -ex 'continue' `
  --args examples\debug\build\debug.exe 1 1000
```

gdb prints each stop the same way on Linux and Windows. The Windows run, with
the debug report left out:

```
Breakpoint 1 at 0x14000208e
Breakpoint 2 at 0x140002387
[New Thread 15776.0x32c8]
[New Thread 15776.0x2f58]
[New Thread 15776.0x2b54]

Thread 1 hit Breakpoint 1, 0x00007ff6cc52208e in _mfb_fn_next_5Fseed ()

Thread 1 hit Breakpoint 1, 0x00007ff6cc52208e in _mfb_fn_next_5Fseed ()

Thread 1 hit Breakpoint 2, 0x00007ff6cc522387 in _mfb_fn_isPrime ()
round 1: 75 primes
total: 75 primes
[Thread 15776.0x2f58 exited with code 0]
[Thread 15776.0x2b54 exited with code 0]
[Thread 15776.0x32c8 exited with code 0]
[Inferior 1 (process 15776) exited normally]
```

The `[New Thread …]` lines are threads Windows starts in every process; your
program runs on thread 1. gdb may set a breakpoint a few bytes into the
function, past the code that sets up the call. It still stops on every call.

A pattern breakpoint, on Linux:

```sh linux
mfb build --debug -O0 examples/debug
gdb -batch -ex 'rbreak ^_mfb_fn_run' -ex 'info breakpoints' \
  --args examples/debug/build/debug-glibc.out
```

```
Num     Type           Disp Enb Address            What
1       breakpoint     keep y   0x0000000000001bc0 <_mfb_fn_runRound+16>
2       breakpoint     keep y   0x0000000000002214 <_mfb_fn_runRounds+32>
```

and the same on macOS:

```sh macos
mfb build --debug -O0 examples/debug
lldb --batch -o "breakpoint set --func-regex 'mfb_fn_run'" -o 'breakpoint list' \
  -- examples/debug/build/debug.out
```

and on Windows:

```powershell windows
mfb build --debug -O0 examples\debug
gdb -batch -ex 'rbreak ^_mfb_fn_run' -ex 'info breakpoints' --args examples\debug\build\debug.exe
```

## See also

- `mfb man debug backtraces`
- `mfb man debug`
