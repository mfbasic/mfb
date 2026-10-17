# breakpoints

Stop the program in a function or when memory changes, run it to the next stop, finish a function, and start over

## Description

A breakpoint stops the program when any thread reaches it, and a watchpoint
when a thread changes memory you name:

```
breakpoint <where> [<commands>]           bp …
breakpoint r <where>                      bp r …
breakpointif <test> at <where> [<commands>]
                                          bpi …
breakpointif r <test> at <where>          bpi r …
list breakpoint                           l breakpoint
watchpoint <where> [<size>] [<commands>]  wp …
watchpoint r <where>                      wp r …
list watchpoint                           l watchpoint
breakerror <code>... [<commands>]         be …
breakerror any [<commands>] | clear       be any … | be clear
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
  the way `bt` and a stop show one: `bp main+0x110`, `bp main+272` or
  `bp _mfb_fn_main+0x110`.

A name as you wrote it can belong to more than one function: two functions
of one name that take different types, every lambda (each is named `lambda`),
or your own `exit` beside the C library's. `mfb dbg` does not guess which you
mean:

```
DBG> bp show
error: 'show' is ambiguous: it names _mfb_fn_show_24Integer, _mfb_fn_show_24String; name one by its symbol
DBG> bp _mfb_fn_show_24Integer
Set a breakpoint at 0x000000010257884c in show.
```

`breakpoint r` takes the same forms and removes every breakpoint they name,
with a test or not. `list breakpoint` lists them.

A system function such as `_exit` has no address until the program has loaded
its libraries. Set before then, its breakpoint waits (`list breakpoint` shows
`-` for its address), and `run` places it once the libraries are loaded.

### Running

`run` lets every thread go until one reaches a breakpoint, an instruction
fails, or the program ends. Press Ctrl-C to pause it wherever it is. A program
`mfb dbg` started cannot read the keyboard while it runs: keystrokes go to
`mfb dbg`.

Each stop prints one line for each thread it stopped. Every line starts with
`--[` and a kind, so a script can pick them out with `grep '^--'`:

| Line | The program stopped because |
| --- | --- |
| `--[BP] T: 0 P: 0x… F: (isPrime) _mfb_fn_isPrime` | thread 0 reached a breakpoint |
| `--[WP] T: 0 P: 0x… F: (countPrimes) _mfb_fn_countPrimes+0x208 M: 0x…: 0x… (0) -> 0x… (1)` | thread 0 changed watched memory |
| `--[BE] T: 0 P: 0x… F: (main) _mfb_fn_main+0x110 E: errorCode::ErrOverflow (77050010)` | thread 0 raised an error you watch for |
| `--[XC] T: 1 P: 0x… F: … X: a bad memory access at 0x8` | an instruction failed |
| `--[CC] User Break` | you pressed Ctrl-C |
| `--[EX] Program exit (0)` | the program ended, with its exit status |

A program built with `--debug` writes a report as it ends
(`mfb man debug report`). `mfb dbg` keeps it off the screen, and the line says
how to read it:

```
--[EX] Program exit (0) - type "show report" to see the performance and memory report
```

`T:` is the thread, `P:` the address of the instruction, and `F:` the function
it is in: the name you wrote in parentheses, then the function's symbol, with
how far in (`-` when no function is known). The symbol tells apart functions
that share a written name, and it is a name `breakpoint` takes. A function
nobody wrote a name for shows its symbol alone: `F: _mfb_arena_alloc+0x4`. A program that ends
by a signal shows `--[EX] Program ended by signal (9)`, and one you attached
to shows `--[EX] Program exit`, since only its own parent learns its status.

Once the program has ended, only `help`, `list`, `list breakpoint`,
`list watchpoint`, `list reg` and `show type` on their own, `show report`,
`restart` and `quit` still work.

### Stopping only when a test holds

`breakpointif` (`bpi`) sets a breakpoint that stops a thread only when a test
holds for it. When the test fails, the thread runs on and nothing is printed:

```
bpi <what> eq <value> at <where>     <what> is <value>
bpi <what> ne <value> at <where>     <what> is not <value>
bpi <what> gt <value> at <where>     <what> is greater than <value>
bpi <what> lt <value> at <where>     <what> is less than <value>
bpi <what> in at <where>             <what> cannot be read
```

`<where>` is the breakpoint's place, as for `breakpoint`. `<what>` is worked
out for the thread that gets there, each time it does, the way `show mem` reads
a place (`mfb man debug inspecting`): a register is its value (`x0`), an address
or a function is the number it is, each with an offset, and any of these in `[`
and `]` is the 8 bytes stored there. `[sp+0x18]` is what is stored 0x18 above
`sp`, and `[[sp+0x10]+8]` what is stored 8 past the address stored at
`sp+0x10`. `<value>` is a whole number, decimal or hex after `0x`, perhaps
negative: `5`, `-1`, `0x10`. `gt` and `lt` compare the two as `Integer`s, so
`-1` is less than `0`. When `<what>` cannot be read, because a `[…]` in it
names memory the program does not have, no test holds but `in`, which holds
just then.

A function's first eight arguments arrive in `x0` to `x7`, so a test on one
stops only the calls you want. `countPrimes` gets the start of its range in
`x0`; with `bpi x0 eq 824084 at countPrimes` the first round's call runs on and
the second's stops:

```
DBG> bpi x0 eq 824084 at countPrimes
Set a breakpoint at 0x00000001048f0dc4 in countPrimes if x0 eq 0x00000000000c9314 (824084).
DBG> run
round 1: 37 primes
--[BP] T: 0 P: 0x00000001048f0dc4 F: (countPrimes) _mfb_fn_countPrimes
```

Breakpoints with different tests can share a place. A thread stops there when
any of them stops it, and each of those runs its own commands. `list breakpoint`
shows each one's test in an `If` column. `bpi r <what> <test> [<value>] at
<where>` removes just the one with that test; `bp r <where>` removes every one
there. Each arrival briefly stops the program while the debugger checks the
test, so a test on a function the program calls millions of times slows the run.

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
--[BP] T: 0 P: 0x0000000102e20a90 F: (isPrime) _mfb_fn_isPrime
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
--[BE] T: 0 P: 0x0000000102268908 F: (main) _mfb_fn_main+0x110 E: errorCode::ErrOverflow (77050010)
```

From there `show error` shows the error and where in your source it was
raised, `bt` how the thread got there, and `dis pc` the code about to decide
which `TRAP` handles it.
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

### Watching memory

`watchpoint` (`wp`) stops the program when a thread changes memory you name.
`<where>` is read the way `show mem` reads one: an address, a register, or a
function, each with an offset, such as `wp sp-0xf0`. It is worked out once,
when you set the watchpoint, so `wp x19` watches the address `x19` holds then,
not the register. `<size>` is how many bytes to watch, 8 when you leave it out
(the size of an `Integer`); it must be 1 to 8 bytes that do not cross an
address that is a multiple of 8. The processor has only a few watchpoints, 4 on
Apple silicon.

The program stops only when the value changes: a thread that writes the value
already there runs on. The line shows the value before and after. Here
`countPrimes` keeps its count of primes found 0xf0 bytes below where `sp` is
when the function starts, and the watchpoint sees it set to 0 and then counted
up:

```
DBG> bp countPrimes
Set a breakpoint at 0x0000000100b60d1c in countPrimes.
DBG> run
--[BP] T: 0 P: 0x0000000100b60d1c F: (countPrimes) _mfb_fn_countPrimes
DBG> wp sp-0xf0
Set a watchpoint at 0x000000016f29ce60 (8 bytes): 0xb4051d3a4bfdb18f (-5474937635778743921).
DBG> bp r countPrimes
Removed the breakpoint at 0x0000000100b60d1c in countPrimes.
DBG> run
--[WP] T: 0 P: 0x0000000100b60d40 F: (countPrimes) _mfb_fn_countPrimes+0x24 M: 0x000000016f29ce60: 0xb4051d3a4bfdb18f (-5474937635778743921) -> 0x0000000000000000 (0)
DBG> run
--[WP] T: 0 P: 0x0000000100b60f24 F: (countPrimes) _mfb_fn_countPrimes+0x208 M: 0x000000016f29ce60: 0x0000000000000000 (0) -> 0x0000000000000001 (1)
```

`P:` and `F:` are the instruction that made the change; the thread is stopped
just after it. `list watchpoint` lists the watchpoints with the value each
holds now, and `wp r <where>` removes one. Only a change a thread makes is
seen; data the system puts into the memory, such as a file's contents being
read into it, is not. A watched place on a thread's stack is used again by
later calls, so it can change for reasons that have nothing to do with the
value you meant to watch. `restart` removes every watchpoint, since the new
run's memory is laid out anew.

### Running commands at a stop

A breakpoint, a watchpoint or `breakerror` can carry commands. Write them after
the rest of the command, separated by `;`. At each stop the debugger prints the
stop's line and runs the commands. The program then stays stopped, unless the
last command is `run`: end the list with `run` to trace a run without
stopping it.

```
DBG> bp countPrimes list reg x0, x1; bt 3; run
Set a breakpoint at 0x0000000102750dc4 in countPrimes. Each stop runs 'list reg x0, x1; bt 3; run'.
DBG> run
--[BP] T: 0 P: 0x0000000102750dc4 F: (countPrimes) _mfb_fn_countPrimes
Register | Hex                | Decimal
x0       | 0x00000000000620a7 | (401575)
x1       | 0x00000000000001f4 | (500)
* thread #0, task = 'MFB Main'
  * frame #0: 0x0000000102750dc4 debug.out`countPrimes
    frame #1: 0x0000000102751284 debug.out`runRound + 344
    frame #2: 0x00000001027538f8 debug.out`runRounds + 5780
round 1: 37 primes
--[BP] T: 0 P: 0x0000000102750dc4 F: (countPrimes) _mfb_fn_countPrimes
Register | Hex                | Decimal
x0       | 0x00000000000c9314 | (824084)
x1       | 0x00000000000001f4 | (500)
* thread #0, task = 'MFB Main'
  * frame #0: 0x0000000102750dc4 debug.out`countPrimes
    frame #1: 0x0000000102751284 debug.out`runRound + 344
    frame #2: 0x00000001027538f8 debug.out`runRounds + 5780
round 2: 31 primes
total: 68 primes
--[EX] Program exit (0) - type "show report" to see the performance and memory report
```

The commands run with the stopped thread as the current one. They can be
`list`, `show`, `bt`, `dis`, or `be` alone: commands that look at the program
without running it. They can also be `j`, which moves the stopped thread
(`mfb man debug stepping`), `finish`, and, as the last one, `run`.
`be ErrOverflow show error; run` prints every overflow as it happens,
`wp sp-0xf0 8 bt 2; run` shows each change with the calls that made it, and
`bp countPrimes j lr x0=0 x1=7; run` makes every call of `countPrimes` return
7 at once.

`finish` among the commands runs the function to its return and prints what
it returned, so `finish; run` logs every call's result:

```
DBG> bp next_seed finish; run
Set a breakpoint at 0x00000001026bc978 in next_seed. Each stop runs 'finish; run'.
DBG> bp formatLine
Set a breakpoint at 0x00000001026bd398 in formatLine.
DBG> run
--[BP] T: 0 P: 0x00000001026bc978 F: (next_seed) _mfb_fn_next_5Fseed
Thread 0 returned from next_seed to 0x00000001026bec48 in runRounds+0x9e4: x0 = 0x0000000000000000 (0), x1 = 0x0000000041c67ea6 (1103527590).
--[BP] T: 0 P: 0x00000001026bc978 F: (next_seed) _mfb_fn_next_5Fseed
Thread 0 returned from next_seed to 0x00000001026bd160 in runRound+0x34: x0 = 0x0000000000000000 (0), x1 = 0x00000000167eb0e7 (377401575).
--[BP] T: 0 P: 0x00000001026bd398 F: (formatLine) _mfb_fn_formatLine
DBG> list breakpoint
Address            | Count | Symbol              | Written name | Commands
0x00000001026bc978 | 2     | _mfb_fn_next_5Fseed | next_seed    | finish; run
0x00000001026bd398 | 1     | _mfb_fn_formatLine  | formatLine   | -
```

Commands after a `finish` see the thread where it returned. When the function
being finished reaches another stop first, that stop's commands take over and
the rest of the first list is dropped.

When threads stop at the same time, the program runs on only if every one of
their stops has commands that end in `run`. A `finish` you type that meets a
stop whose commands end in `run` runs them and carries on toward the end of
its function.

`list breakpoint` and `list watchpoint` show each one's commands in a
`Commands` column, and in `Count` how many times each has stopped a thread
since the program started. A breakpoint whose `Count` is still 0 when the
program has ended was never reached; `restart` sets the counts back to 0.

### Starting over

`restart` starts the program again from the beginning, with the same
arguments and the environment `env` gives it, and keeps your breakpoints and
the errors you watch for; it removes your watchpoints. It works on a program
`mfb dbg` started, even after it has ended, and picks up a program you rebuilt
in the meantime.

### Changing the environment

`env` changes the environment variables the program gets the next time you
`restart` it. A program `mfb dbg` starts gets the same environment as `mfb dbg`
itself. The running program is never changed: what `os::getEnv` reads in it
stays as it was until the restart.

- `env s <name> <value>` (or `env set`) sets a variable. The value is the rest
  of the line after the name, spaces included, and may be empty.
- `env r <name>` (or `env remove`) removes one.
- `env clear` removes them all: the program starts with no environment at all.
- `env reset` goes back to the environment the session started the program
  with.
- `list env` shows the environment the next restart gives, sorted by name.
  When that is not the one the running program has, a line under the list
  says so.

A name holds no `=`, and names are case-sensitive: `env r path` does not
remove `PATH`. `env` works only on a program `mfb dbg` started, since one it
attached to cannot restart, and it still works once the program has ended,
ready for the `restart`. A stop cannot run `list env`.

## Examples

Stop in `isPrime`, and in the system's `exit` once every round is done:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp isPrime\nbp _exit\nrun\nbp r isPrime\nrun\nrun\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500
```

```
Launched examples/debug/build/debug.out as process 89957; paused before its first instruction.
DBG> bp isPrime
Set a breakpoint at 0x0000000100dd4aa4 in isPrime.
DBG> bp _exit
Set a pending breakpoint on _exit: it is placed when the program has loaded its libraries.
DBG> run
Placed the pending breakpoint on _exit at 0x00000001966eaf8c.
--[BP] T: 0 P: 0x0000000100dd4aa4 F: (isPrime) _mfb_fn_isPrime
DBG> bp r isPrime
Removed the breakpoint at 0x0000000100dd4aa4 in isPrime.
DBG> run
round 1: 37 primes
round 2: 31 primes
total: 68 primes
--[BP] T: 0 P: 0x00000001966eaf8c F: (exit) _exit
DBG> run
--[EX] Program exit (0) - type "show report" to see the performance and memory report
DBG> q
Process 89957 had already exited.
```

Stop in `countPrimes` only for the second round, where `x0`, its first argument,
is 824084:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bpi x0 eq 824084 at countPrimes\nrun\nlist breakpoint\nlist reg x0, x1\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500
```

```
Launched examples/debug/build/debug.out as process 82620; paused before its first instruction.
DBG> bpi x0 eq 824084 at countPrimes
Set a breakpoint at 0x00000001048f0dc4 in countPrimes if x0 eq 0x00000000000c9314 (824084).
DBG> run
round 1: 37 primes
--[BP] T: 0 P: 0x00000001048f0dc4 F: (countPrimes) _mfb_fn_countPrimes
DBG> list breakpoint
Address            | Count | Symbol              | Written name | If
0x00000001048f0dc4 | 1     | _mfb_fn_countPrimes | countPrimes  | x0 eq 0x00000000000c9314 (824084)
DBG> list reg x0, x1
Register | Hex                | Decimal
x0       | 0x00000000000c9314 | (824084)
x1       | 0x00000000000001f4 | (500)
DBG> q
Killed process 82620.
```

Read the line `formatLine` makes, then start the program over with the
breakpoint still set:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp formatLine\nrun\nfinish\nshow type string x1\nrestart\nlist breakpoint\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500
```

```
Launched examples/debug/build/debug.out as process 89963; paused before its first instruction.
DBG> bp formatLine
Set a breakpoint at 0x0000000100fb5374 in formatLine.
DBG> run
--[BP] T: 0 P: 0x0000000100fb5374 F: (formatLine) _mfb_fn_formatLine
DBG> finish
Thread 0 returned from formatLine to 0x0000000100fb7844 in runRounds+0x15cc: x0 = 0x0000000000000000 (0), x1 = 0x0000000101030440 (4311942208).
DBG> show type string x1
Length: 18 Value: "round 1: 37 primes"
DBG> restart
Restarted the program as process 89965; paused before its first instruction.
DBG> list breakpoint
Address            | Count | Symbol             | Written name
0x0000000104bd1374 | 0     | _mfb_fn_formatLine | formatLine
DBG> q
Killed process 89965.
```

Give the program `LOG_LEVEL` and take away `TERM` for its next start. Here
`mfb dbg` itself starts with only `HOME` and `TERM` (`env -i`, which needs the
full path to `mfb`), so the list stays short:

```sh macos
mfb build --debug -O0 examples/debug
printf 'env s LOG_LEVEL debug\nenv r TERM\nlist env\nrestart\nlist env\nq\n' | env -i HOME=/Users/me TERM=xterm-256color "$(command -v mfb)" dbg examples/debug/build/debug.out -- 2 500
```

```
Launched examples/debug/build/debug.out as process 49471; paused before its first instruction.
DBG> env s LOG_LEVEL debug
Set LOG_LEVEL=debug; the program gets it when it restarts.
DBG> env r TERM
Removed TERM; the program goes without it when it restarts.
DBG> list env
Name      | Value
HOME      | /Users/me
LOG_LEVEL | debug

Changed since the program started: it gets these when it restarts.
DBG> restart
Restarted the program as process 49493; paused before its first instruction.
DBG> list env
Name      | Value
HOME      | /Users/me
LOG_LEVEL | debug
DBG> q
Killed process 49493.
```

## See also

- `mfb man debug backtraces` — the calls that led to a stop
- `mfb man debug stepping` — go on one instruction at a time
- `mfb man debug values`
- `mfb man debug-external breakpoints` — breakpoints in lldb and gdb
