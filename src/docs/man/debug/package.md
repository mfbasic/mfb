# debug

Debug an MFBASIC program with mfb dbg, the debugger built into mfb

## Synopsis

```
mfb build --debug -O0 [path]
mfb dbg <binary> [-- <args>...]
mfb dbg <pid>
mfb man debug [topic]
```

## Imports

`debug` is a documentation topic, not an importable package. No `IMPORT` is
needed: everything here happens at build time and in `mfb dbg`.

## Description

`mfb` builds your program, and it debugs it too. `mfb dbg` starts a program
paused, or pauses one that is already running, and gives you a `DBG>` prompt
where you can:

- stop it when it reaches a function, and run it on to the next stop;
- see the calls that led there, on one thread or all of them;
- step through it an instruction at a time, and read its machine code;
- read its registers and memory, and show the values in memory as what they
  are: a `String`, a `List`, a `Map`, a `Set`, a number, an open `File`;
- look inside the pools of memory each thread keeps its values in.

It needs nothing besides `mfb`. Everything it shows uses the names you wrote:
`isPrime`, `round 2`, `io::print`.

`mfb dbg` runs on macOS today. On Linux and Windows, lldb and gdb do the job;
`mfb man debug-external` shows how, and it covers profilers and crash-address
lookup on every platform too.

### Build for debugging

Build with `--debug -O0` before you debug:

```
mfb build --debug -O0 examples/debug
```

- `--debug` keeps the name of every function you wrote in the executable, so
  `mfb dbg` can show it and you can stop in it by name. It also makes the
  program print a report of its memory use when it exits
  (`mfb man debug report`).
- `-O0` keeps every function a call of its own. The default optimization level
  merges small functions into their callers, and a function that was merged
  away cannot be stopped in.

`mfb dbg` works on any MFBASIC program built for the computer you are on, but
without `--debug` it knows only the names of the system functions your
program calls, and the commands that read memory pools and values in them
(`list arena`, `show arena`, `show type`) need it.

### The example program

Every page in this topic debugs the same program, `examples/debug` in the
MFBASIC repository. It counts the primes in a run of ranges and prints one line
per round, then the total:

```
main -> runRounds -> runRound -> countPrimes -> isPrime
                              -> next_seed
                  -> formatLine
                  -> summarize
                  -> roundWorker -> runRound        (threads mode)
```

`runRounds` keeps what each round found in a `List OF Integer`, a
`Map OF String TO Integer` and a `Set OF Integer`, with lists of `Float`,
`Fixed` and `Money` figures, and writes each line to a log file it holds open.
It hands all of them to `summarize`, so a stop there sees one value of each
kind side by side.

It takes up to three optional arguments: `rounds` (default 5), `size` (default
20000), and `threads`. With `threads` set to `1`, each round runs on its own
worker thread (`roundWorker`). `2 500` runs in a moment; `3000 20000 1` runs
for about ten seconds, long enough to attach to it.

Every script in this topic runs from the root of the repository, builds the
example itself, and was run as written on macOS.

### A first session

`mfb dbg` reads its commands from the keyboard, or from a pipe, as here. This
stops in `summarize`, shows the calls that led there, two of the values it was
handed, and where `summarize` returns to:

```sh macos
mfb build --debug -O0 examples/debug
printf 'bp summarize\nrun\nbt\nshow type list x0 0..\nshow type map x1 get round 2\nfinish\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500 2> report.txt
```

```
Launched examples/debug/build/debug.out as process 89822; paused before its first instruction.
DBG> bp summarize
Set a breakpoint at 0x00000001005c9ee0 in summarize.
DBG> run
round 1: 37 primes
round 2: 31 primes
--[BP] T: 0 P: 0x00000001005c9ee0 F: summarize
DBG> bt
* thread #0, task = 'MFB Main'
  * frame #0: 0x00000001005c9ee0 debug.out`summarize
    frame #1: 0x00000001005cff28 debug.out`runRounds + 23728
    frame #2: 0x00000001005d1040 debug.out`main + 2912 <mfb>
    frame #3: 0x00000001005c8634 debug.out`main + 348 <native>
DBG> show type list x0 0..
Index Address            Bytes Value
0     0x0000000100644338 8     0x0000000000000025 (37)
1     0x0000000100644340 8     0x000000000000001f (31)
DBG> show type map x1 get round 2
Index: 1
Key: "round 2" at 0x0000000100644538 (7 bytes)
Value: 0x000000000000001f (31) at 0x0000000100644540 (8 bytes)
DBG> finish
Thread 0 returned from summarize to 0x00000001005cff28 in runRounds+0x5cb0: x0 = 0x0000000000000000 (0), x1 = 0x0000cd4e893e525a (225737193706074).
DBG> q
Killed process 89822.
```

The addresses change from run to run. `x0` and `x1` are the first two values
`summarize` was handed, `counts` and `byRound`: a function receives its first
eight in `x0` to `x7`, in order. `mfb man debug values` explains reading them.
The program's own output, and the debug report on stderr, mix in with the
session's.

### Commands

| Command | Short | What it does |
| --- | --- | --- |
| `help` | `h` | lists the commands |
| `breakpoint <where> [<commands>]` | `bp …` | stops the program when it reaches a function or address |
| `breakpoint r <where>` | `bp r …` | removes one |
| `watchpoint <where> [<size>] [<commands>]` | `wp …` | stops the program when a thread changes memory |
| `watchpoint r <where>` | `wp r …` | removes one |
| `breakerror <code>... [<commands>]` | `be …` | stops the program when a thread raises one of these errors; `be any` for every error |
| `breakerror clear` | `be clear` | stops watching for errors; `be` alone says what is watched for |
| `run` | `r` | runs the program until a breakpoint or its end (Ctrl-C pauses it) |
| `finish` | `f` | runs until the current function returns |
| `restart` | `rs` | starts the program again |
| `step [<count>] [<thread>]` | `s …` | runs one thread one instruction, or `<count>` |
| `step-all [<count>]` | `sa …` | runs every thread one instruction, or `<count>` |
| `disassemble` | `dis` | shows the machine code of the current function |
| `dis <count>` | | the next `<count>` instructions, or with `-` the ones before |
| `dis -<back> <count>` | | `<count>` instructions, from `<back>` before the current one |
| `dis <where> [<count>]`, `dis <function> [<count>]` | | from an address or register (`pc-0x20`), or a function's |
| `backtrace [<count>] [<thread>]` | `bt …` | shows the calls the current thread is in |
| `backtrace all` | `bt all` | shows the calls every thread is in |
| `list` | `l` | shows what can be listed |
| `list func [<filter>...]` | `l func …` | lists the program's functions, or those of a type or name |
| `list thread` | `l thread` | lists the program's threads |
| `list reg` | `l reg` | shows how to list registers: `gpr`, `simd`, `sys`, or by name |
| `list arena` | `l arena` | lists the program's memory pools |
| `list breakpoint` | `l breakpoint` | lists the breakpoints |
| `list watchpoint` | `l watchpoint` | lists the watchpoints |
| `show mem <where> [<size>]` | | shows the raw bytes of memory |
| `show error [<thread>]` | | shows the error a thread has raised and not yet handled |
| `show type <type> <where> …` | | shows a value in memory as a type |
| `show arena [<id>]` | | shows one memory pool in detail |
| `show arena <id> <block> …` | | shows the raw bytes of part of a pool |
| `quit` | `q` | ends the session |

## Topics

- `start` — start or attach to a program, end the session, and drive it from a script.
- `breakpoints` — stop in a function or when memory changes, run commands at a stop, finish a function, start over.
- `stepping` — run one instruction at a time, and read the machine code.
- `backtraces` — see the calls that led to where each thread is.
- `inspecting` — list the functions, threads and registers, and read memory.
- `values` — show a value in memory as the type it is.
- `pools` — the memory pools each thread keeps its values in.
- `report` — every line of the debug report a `--debug` build prints at exit.

## Errors

No errors. This topic describes a tool, not MFBASIC functions.

## See also

- `mfb man debug start`
- `mfb man debug breakpoints`
- `mfb man debug values`
- `mfb man debug report`
- `mfb man debug-external` — lldb, gdb, profilers and address lookup
- `mfb man optimizations`
- `mfb spec tooling debugger` — the exact behavior of `mfb dbg`
