# backtraces

See the chain of calls that led to where the program is

## Description

A backtrace lists the functions that are running right now, innermost first:
the function the program is in, the function that called it, and so on out to
the program's entry. It answers "how did the program get here?"

You get one in two ways:

- **At a breakpoint.** Stop the program in a function (`mfb man debug
  breakpoints`) and ask for the backtrace.
- **By attaching to a running program.** Point the debugger at a program that
  is already running, for example one that seems stuck, and ask what every
  thread is doing. The program stops while you look and carries on after you
  detach.

| To | lldb (macOS) | gdb (Linux, Windows) |
| --- | --- | --- |
| backtrace of the current thread | `bt` | `bt` |
| only the innermost 3 calls | `bt 3` | `bt 3` |
| backtrace of every thread | `thread backtrace all` | `thread apply all bt` |
| select call number 2 | `frame select 2` | `frame 2` |
| attach to a running program | `lldb -p <pid>` | `gdb -p <pid>` |
| let it go and leave | `detach`, then `quit` | `detach`, then `quit` |

Each line of a backtrace is a **frame**. Frame `#0` is where the program is,
and each frame after it is a caller. A `+ 568` after a name (lldb) is how far
into that function the call was made, in bytes.

Build with `--debug -O0` (`mfb man debug`). In a default build a function
merged into its caller has no frame of its own and is missing from the chain.

### Reading the end of the chain

The chain ends at `main` on macOS and `_main` on Linux, the program's entry.
gdb on Linux then prints
`Backtrace stopped: previous frame identical to this frame (corrupt stack?)`.
That only means there is nothing above the entry point: the stack is fine.

### Windows: only the innermost frames

On Windows gdb can name the function the program is in, and at a breakpoint
also the function that called it. Past that, the frames are `??` with random
addresses. Windows executables need unwind tables for a debugger to walk the
stack further, and MFBASIC does not write them yet. To see more of the chain
on Windows, set breakpoints in the outer functions too (`rbreak ^_mfb_fn_`)
and watch which one stops first.

## A session, step by step

Stop in `isPrime` and print the backtrace (macOS shown):

```
(lldb) b mfb_fn_isPrime
(lldb) run
(lldb) bt
* thread #1, queue = 'com.apple.main-thread', stop reason = breakpoint 1.1
  * frame #0: 0x0000000100000a5c debug.out`mfb_fn_isPrime
    frame #1: 0x0000000100000f70 debug.out`mfb_fn_countPrimes + 568
    frame #2: 0x00000001000011c0 debug.out`mfb_fn_runRound + 328
    frame #3: 0x0000000100001dd8 debug.out`mfb_fn_runRounds + 1820
    frame #4: 0x0000000100002b8c debug.out`mfb_fn_main + 2892
    frame #5: 0x00000001000005ec debug.out`main + 276
```

Read it bottom up: the program's entry called `main`, which called `runRounds`,
which called `runRound`, which called `countPrimes`, which called `isPrime`.
That is the call chain drawn on `mfb man debug`.

Now attach to a program that is already running. Start a long run in one
terminal:

```
examples/debug/build/debug.out 3000 20000
```

and attach from another (`pgrep` finds the process id):

```
lldb -p $(pgrep -f 'debug.out 3000')
(lldb) bt
(lldb) detach
(lldb) quit
```

The program stops while lldb is attached and carries on after `detach`. A
program that seems stuck in a loop shows the loop's function at frame `#0`
every time you attach.

## Examples

Each script builds the example, prints the backtrace at a breakpoint in
`isPrime`, then starts a long run in the background, attaches to it, prints
what its main thread is doing, and stops it.

macOS:

```sh macos
mfb build --debug -O0 examples/debug
lldb --batch \
  -o 'breakpoint set --name mfb_fn_isPrime' \
  -o 'run' \
  -o 'bt' \
  -o 'frame select 2' \
  -- examples/debug/build/debug.out 1 1000

examples/debug/build/debug.out 3000 20000 > /dev/null 2>&1 &
pid=$!
sleep 1
lldb --batch -p "$pid" -o 'thread backtrace all' -o 'detach'
kill "$pid"
```

Linux:

```sh linux
mfb build --debug -O0 examples/debug
gdb -batch \
  -ex 'break _mfb_fn_isPrime' \
  -ex 'run' \
  -ex 'bt' \
  -ex 'frame 2' \
  --args examples/debug/build/debug-glibc.out 1 1000

examples/debug/build/debug-glibc.out 3000 20000 > /dev/null 2>&1 &
pid=$!
sleep 1
gdb -batch -p "$pid" -ex 'thread apply all bt' -ex 'detach'
kill "$pid"
```

The attached half prints:

```
Thread 1 (process 185226 "debug-glibc.out"):
#0  0x0000aaaae64046d4 in _mfb_fn_isPrime ()
#1  0x0000aaaae6404aa8 in _mfb_fn_countPrimes ()
#2  0x0000aaaae6404cf8 in _mfb_fn_runRound ()
#3  0x0000aaaae6405910 in _mfb_fn_runRounds ()
#4  0x0000aaaae64066c4 in _mfb_fn_main ()
#5  0x0000aaaae6404124 in _main ()
[Inferior 1 (process 185226) detached]
```

Windows (PowerShell). `Start-Process -PassThru` starts the long run in the
background and returns its process, whose `Id` gdb attaches to:

```powershell windows
mfb build --debug -O0 examples\debug
gdb -batch `
  -ex 'break _mfb_fn_isPrime' `
  -ex 'run' `
  -ex 'bt' `
  -ex 'frame 1' `
  --args examples\debug\build\debug.exe 1 1000

$p = Start-Process -FilePath examples\debug\build\debug.exe -ArgumentList 3000, 20000 `
  -PassThru -WindowStyle Hidden -RedirectStandardOutput out.txt -RedirectStandardError err.txt
Start-Sleep -Seconds 1
gdb -batch -p $p.Id -ex 'thread apply 1 bt 1' -ex 'detach'
Stop-Process -Id $p.Id
```

At the breakpoint gdb names `isPrime` and its caller:

```
Thread 1 hit Breakpoint 1, 0x00007ff697e72387 in _mfb_fn_isPrime ()
#0  0x00007ff697e72387 in _mfb_fn_isPrime ()
#1  0x00007ff697e72c81 in _mfb_fn_countPrimes ()
#2  0x0000000000000136 in ?? ()
#3  0x0000000040000068 in ?? ()
...
```

Frames `#2` and beyond are noise: gdb is guessing.

The attached half asks for frame `#0` only (`bt 1`), the frame Windows reports
reliably. Attaching starts an extra thread in the program for the debugger, so
the script names thread 1, the one running your program.

## See also

- `mfb man debug breakpoints`
- `mfb man debug profiler`
- `mfb man debug`
