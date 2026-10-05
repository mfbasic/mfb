# breakpoints

Stop the program in a function, run it to the next stop, finish a function, and start over

## Description

A breakpoint stops the program when any thread reaches it:

```
breakpoint <where>        bp <where>
breakpoint r <where>      bp r <where>
list breakpoint           l breakpoint
breakerror <code>...      be <code>...
breakerror any | clear    be any | clear
```

`<where>` is an address (`0x` and hex digits, as `list func` and `list thread`
show them, perhaps with `+` or `-` an offset: `0x100a4d374+0x40`) or a
function name:

- a name as you wrote it: `isPrime`, `formatLine`. A function from a package
  is `package::name`, and a built-in function the runtime carries out for you
  is named the same way: `bp io::print` stops each time your program prints.
- a name exactly as `list func` shows it in its `Symbol` column, such as
  `_mfb_fn_isPrime` or `_exit`;
- `exit`, `write` and the other system functions by their C names;
- any of these with `+` and an offset, a place inside the function, written
  the way `bt` and a stop show one: `bp main+0x110` or `bp main+272`.

A name sets a breakpoint at every function it matches. `breakpoint r` takes the
same forms and removes what they name. `list breakpoint` lists them.

A system function such as `_exit` has no address until the program has loaded
its libraries. Set before then, its breakpoint waits (`list breakpoint` shows
`-` for its address), and `run` places it once the libraries are loaded.

### Running

`run` lets every thread go until one reaches a breakpoint, an instruction
fails, or the program ends. Press Ctrl-C to pause it wherever it is. A program
`mfb dbg` started cannot read the keyboard while it runs: keystrokes go to
`mfb dbg`.

Once the program has ended, only `help`, `list`, `list breakpoint`, `list reg`
and `show type` on their own, `restart` and `quit` still work.

### Finishing a function

`finish` lets the program run until the function the current thread is in
returns, then says where it went back to and what came back. MFBASIC returns
an error code in `x0` (0 when there was none) and the value in `x1`. A value
kept in memory, such as a `String`, comes back as where it is, so
`show type` can read it (`mfb man debug values`):

```
DBG> finish
Thread 0 returned from formatLine to 0x0000000100fb7844 in runRounds+0x15cc: x0 = 0x0000000000000000 (0), x1 = 0x0000000101030440 (4311942208).
DBG> show type string x1
Length: 18 Value: "round 1: 37 primes"
```

If a breakpoint is reached first, `finish` stops there instead and says so.
In a function that calls itself, or one that calls a function you have a
breakpoint in, that is the usual case:

```
DBG> finish
Thread 0 hit a breakpoint at 0x0000000100d9caa4 in isPrime.
The finish from countPrimes did not complete; type 'finish' to run it again from here.
```

A `finish` from there runs to the end of the function the thread is now in,
`isPrime`, not `countPrimes`.

### Stopping on an error

`breakerror` (`be`) stops the program when a thread raises an error you name,
at the moment it is raised: the thread is back in the function that raised it,
and no `TRAP` has run yet. Name the error by its code or by its `errorCode`
name:

```
DBG> be ErrOverflow
Stopping when a thread raises 77050010 (errorCode::ErrOverflow).
DBG> run
Thread 0 raised 77050010 (errorCode::ErrOverflow) at src/main.mfb:11:34; it is at 0x0000000102268908 in main+0x110 <mfb>, before a TRAP takes the error.
```

From there `show error` shows the error, `bt` how the thread got there, and
`dis pc` the code about to decide which `TRAP` handles it.
`be 77050004 ErrNotFound` watches for several codes at once, `be any` for every
error, `be clear` stops watching, and `be` alone says what is being watched
for, as `list breakpoint` does after its list. It needs a `--debug` build.

What `breakerror` does not catch, or does differently:

- **An error raised again.** `FAIL e`, which raises an `Error` you already
  hold, and the re-raise when a function's cleanup passes an error on, do not
  stop: they hand on an error that already exists rather than raising a new one.
  Watch for it where it was first raised.
- **An error when memory has run out.** When there is no memory left to record
  the error, it travels without its record and `breakerror` does not see it;
  `show error` reports nothing for it either.
- **An error from another thread.** An error a thread started with
  `thread::start` raises stops once in that thread, and again in the thread that
  receives it from `thread::waitFor`.
- **Two raises at once.** While the debugger runs a thread from the raise back
  to the function that raised it, an error another thread raises at the same
  time does not stop.
- **A breakpoint first.** If one of your breakpoints is reached on the way back
  to the raising function, that stop is reported, not the error.
- **Speed.** Every error the program raises, watched or not, briefly stops it
  while the debugger checks its code, so a program that raises many errors runs
  slower while `breakerror` is on.

### Starting over

`restart` starts the program again from the beginning, with the same
arguments, and keeps your breakpoints. It works on a program `mfb dbg`
started, even after it has ended, and picks up a program you rebuilt in the
meantime.

## Examples

Stop in `isPrime`, and in the system's `exit` once every round is done:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp isPrime\nbp _exit\nrun\nbp r isPrime\nrun\nrun\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500 2> report.txt
```

```
Launched examples/debug/build/debug.out as process 89957; paused before its first instruction.
DBG> bp isPrime
Set a breakpoint at 0x0000000100dd4aa4 in isPrime.
DBG> bp _exit
Set a pending breakpoint on _exit: it is placed when the program has loaded its libraries.
DBG> run
Placed the pending breakpoint on _exit at 0x00000001966eaf8c.
Thread 0 hit a breakpoint at 0x0000000100dd4aa4 in isPrime.
DBG> bp r isPrime
Removed the breakpoint at 0x0000000100dd4aa4 in isPrime.
DBG> run
round 1: 37 primes
round 2: 31 primes
total: 68 primes
Thread 0 hit a breakpoint at 0x00000001966eaf8c in exit.
DBG> run
The program exited with status 0.
DBG> q
Process 89957 had already exited.
```

Read the line `formatLine` makes, then start the program over with the
breakpoint still set:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp formatLine\nrun\nfinish\nshow type string x1\nrestart\nlist breakpoint\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500 2> report.txt
```

```
Launched examples/debug/build/debug.out as process 89963; paused before its first instruction.
DBG> bp formatLine
Set a breakpoint at 0x0000000100fb5374 in formatLine.
DBG> run
Thread 0 hit a breakpoint at 0x0000000100fb5374 in formatLine.
DBG> finish
Thread 0 returned from formatLine to 0x0000000100fb7844 in runRounds+0x15cc: x0 = 0x0000000000000000 (0), x1 = 0x0000000101030440 (4311942208).
DBG> show type string x1
Length: 18 Value: "round 1: 37 primes"
DBG> restart
Restarted the program as process 89965; paused before its first instruction.
DBG> list breakpoint
Address            Symbol             Written name
0x0000000104bd1374 _mfb_fn_formatLine formatLine
DBG> q
Killed process 89965.
```

## See also

- `mfb man debug backtraces` — the calls that led to a stop
- `mfb man debug stepping` — go on one instruction at a time
- `mfb man debug values`
- `mfb man debug-external breakpoints` — breakpoints in lldb and gdb
