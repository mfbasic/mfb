# debugger

Pause an MFBASIC program with mfb dbg, the MFBASIC debugger (macOS)

## Description

`mfb dbg` is the debugger that comes with the compiler. Give it a program to
start, or the process ID of one that is already running, and it pauses that
program and waits for commands at a `DBG>` prompt:

```
mfb dbg <binary> [-- <args>...]
mfb dbg <pid>
```

- `mfb dbg <binary>` starts the program and pauses it before it runs a
  single instruction. Everything after `--` is passed to the program as its
  arguments.
- `mfb dbg <pid>` pauses the running program with that process ID, on
  every thread.

The program must be an MFBASIC executable built for the computer you are on.
`mfb dbg` refuses anything else, and never touches the program:

| You give it | It says |
| --- | --- |
| a program that is not MFBASIC, such as `/bin/ls` | `'/bin/ls' is not an MFBASIC executable` |
| an MFBASIC program built with `--target` for another platform | `'…' is built for ELF x86-64; this debugger runs Mach-O aarch64 programs` |
| a process ID that is not running | `no process with PID 999999` |

When a file exists with a name made only of digits, `mfb dbg 1234` starts that
file. Name it `./1234` either way to be clear.

`mfb dbg` runs on macOS. On Linux and Windows it says `mfb dbg is not supported`
and stops; use gdb there (`mfb man debug breakpoints`).

The debugger is new: for now it pauses a program, lists its functions,
threads, registers and memory pools, steps one thread or all of them an
instruction at a time, runs it to breakpoints, shows its machine code, and
lets it go. Backtraces and the other tools in this topic still come from lldb
and gdb.

### Commands

| Command | Short | What it does |
| --- | --- | --- |
| `help` | `h` | lists the commands |
| `list` | `l` | shows what can be listed |
| `list func` | `l func` | lists the program's functions |
| `list thread` | `l thread` | lists the program's threads |
| `list reg` | `l reg` | shows how to list registers |
| `list reg [<id>] gpr` | `l reg …` | lists a thread's general-purpose registers |
| `list reg [<id>] simd` | `l reg …` | lists a thread's SIMD registers |
| `list reg [<id>] sys` | `l reg …` | lists a thread's system registers |
| `list reg [<id>] <register>, …` | `l reg …` | lists just the registers named |
| `list arena` | `l arena` | lists the program's memory pools |
| `show mem <where> [<size>]` | | shows the raw bytes of memory |
| `show arena [<id>]` | | shows one memory pool in detail |
| `show arena <id> <block> …` | | shows the raw bytes of part of a pool |
| `step [<count>] [<thread>]` | `s …` | runs one thread one instruction, or `<count>` |
| `step-all [<count>]` | `sa …` | runs every thread one instruction, or `<count>` |
| `breakpoint <where>` | `bp …` | sets a breakpoint (below) |
| `breakpoint r <where>` | `bp r …` | removes one |
| `list breakpoint` | `l breakpoint` | lists the breakpoints |
| `run` | `r` | runs the program until a breakpoint or its end |
| `finish` | `f` | runs until the current function returns |
| `restart` | `rs` | starts the program again |
| `disassemble` | `dis` | shows the machine code of the current function |
| `dis <count>` | | shows the next `<count>` instructions |
| `dis <function>` | | shows a function's machine code |
| `quit` | `q` | ends the session (below) |

How `quit` ends the session depends on how it began:

- A program `mfb dbg` started is stopped for good. It never ran, so it printed
  nothing and changed nothing.
- A program `mfb dbg` attached to carries on running from where it paused.

A command `mfb dbg` does not know is reported, and the prompt comes back.

Every listing starts with a line naming its columns, and each column starts
at the same place on every line.

### Listing functions

`list func` prints one function per line, under the header
`Type Address Symbol Written name`:

```
<type> <address> <symbol> <written name>
```

| Type | The function is |
| --- | --- |
| `MU` | one you wrote, in your program or a package it imports |
| `MR` | part of the MFBASIC runtime, which every program carries |
| `LN` | in a system library the program calls, such as `malloc` |

`<symbol>` is the function's name in the executable, the one lldb and gdb use
(`mfb man debug` explains how your names are spelled there). `<written name>`
is the name as you wrote it, so `_mfb_fn_next_5Fseed` shows as `next_seed`, and
a second version of a function (a name ending in `$own` and a number) shows
under the function's own name. A runtime function that does the work of a
built-in package function shows that function's name, so `_mfb_rt_io_io_print`
shows as `io::print`; the rest of the runtime shows `-`.

`<address>` is where the function is in the running program. A program `mfb
dbg` started is paused before it has loaded its system libraries, so every `LN`
function shows `-` there; attach to a running program to see where they are.
When a program calls a system function from more than one place in the runtime,
that function is listed once for each.

Your functions are named only in a program built with `--debug`. Start or attach
to any other program and `mfb dbg` says so:

```
note: this is not a debug build, so the program's function names are missing.
      Rebuild it with: mfb build --debug -O0 <project>
```

and `list func` shows only the `LN` functions.

### Listing threads

`list thread` prints one thread per line, under the header `ID Name PC Task`:

```
<id> <name> <pc> <task>
```

`<id>` is the number the other commands use for the thread. `<name>` is the
name the thread gave itself (system threads often have one); otherwise the
function it is in, such as `isPrime+0x278`; otherwise `thread-` and the
system's number for it. `<pc>` is the address of the next instruction it will
run.

| Task | The thread |
| --- | --- |
| `App` | runs the window of a program built with `--app` |
| `MFB Main` | runs your `main` function |
| `User Thread` | is one you started with `thread::start` |
| `Other` | belongs to the system or the MFBASIC runtime |

In a program built with `--app`, `main` runs on a thread of its own, not on
thread 0. Without `--debug`, `mfb dbg` can recognize that thread only while it
is running your code; while it waits (in `os::sleep`, say) it shows as `Other`.

### Listing registers

`list reg` on its own explains how to list registers. Name a group, or the
registers you want, and it prints them for thread 0, one per line under the
header `Register Hex Decimal`, in hex and then in decimal
(`0x000000000000002d (45)`):

| Group | The registers |
| --- | --- |
| `gpr` | the general-purpose registers: `x0` to `x28`, `fp` (`x29`) and `lr` (`x30`) |
| `simd` | the SIMD and floating-point registers: `v0` to `v31` |
| `sys` | the system registers: `sp`, `pc`, the condition flags `nzcv`, and `fpsr` and `fpcr` |

Put a thread's `<id>` first to see that thread's instead: `list reg 2 gpr`.
Name registers with spaces or commas, `list reg x8, x9, sp`, and mix them with
groups, `list reg sys x0`. A program paused before its first instruction has
almost all of them at zero; `pc` is where it will start.

### Listing memory pools

Each thread that runs MFBASIC code keeps the values it makes in a pool of its
own, called an arena. `list arena` prints one per line, under the header
`ID Thread Address Size (KB)`:

```
<id> <thread id> <address> <size in KB>
```

`<thread id>` is the thread that uses the pool (the `<id>` from `list thread`),
or `-` for a pool that only carries values from one thread to another.
`<size in KB>` is how much memory the pool holds. `list arena` works only on a
program built with `--debug`, and lists nothing until the program has started
running.

### Reading memory

`show mem` prints raw bytes, 64 unless you give a size. Name an address, or a
register with an offset: at `-O0` your variables live at offsets from `sp`, so
`show mem sp+0x10 24` shows three of them at once.

```
DBG> show mem sp+0x10 24
Memory at 0x000000016f325ea0 (sp+0x10 of thread 0):
0x000000016f325ea0 0a 00 00 00 00 00 00 00 2d 00 00 00 00 00 00 00 | . . . . . . . . - . . . . . . .
0x000000016f325eb0 0a 00 00 00 00 00 00 00                         | . . . . . . . .
```

Numbers are stored lowest byte first, so the 8 bytes `2d 00 … 00` are 45.

### Looking inside a memory pool

`show arena` shows the main thread's pool (`show arena 3` shows pool 3, the
`<id>` from `list arena`): every value MFBASIC keeps about the pool, in hex,
and then each block of memory the pool holds, oldest first. A pool starts with
one block and adds bigger ones as your program makes more values:

```
Arena Block (0):
  prevBlock         0x0000000000000000
  blockSize         0x0000000000001000
  usableCapacity    0x0000000000000fe0
  bumpOffset        0x0000000000000000
Arena Block (1):
  prevBlock         0x0000000105028000
  blockSize         0x0000000000001000
  usableCapacity    0x0000000000000fe0
  bumpOffset        0x0000000000000000
```

`blockSize` is the block's size in bytes, and `usableCapacity` how much of it
holds values. `show arena <id> <block>` prints the block's first 1 KB as raw
bytes, 16 to a line, with the printable ones spelled out at the end of the line;
add an offset and a size (decimal, or hex starting `0x`) to see another part:

```
DBG> show arena 0 0 0x100 40
Arena Block (0,0):
0x0000_0100 20 c1 cd 00 01 00 00 00 20 00 00 00 00 00 00 00 |   . . . . . . .   . . . . . . .
0x0000_0110 f7 82 17 23 e7 57 80 8e 5c 3c 1a ae c7 df e3 da | . . . # . W . . \ < . . . . . .
0x0000_0120 40 c1 cd 00 01 00 00 00                         | @ . . . . . . .
```

Like `list arena`, it works only on a program built with `--debug`.

### Stepping

`step` runs one instruction of thread 0, the main thread of a console program,
and pauses it again; `step 10` runs ten, and `step 10 2` runs ten of thread 2
(the `<id>` from `list thread`). Every other thread stays paused. It then says
where the thread is:

```
DBG> step 3
Stepped 3 instructions; thread 0 is at 0x0000000104fccc80 in isPrime+0x224.
```

`step 0` only says where the thread is, and a negative count is refused.

A thread waiting in the system, in `os::sleep` or `thread::waitFor` for
example, cannot run its next instruction until the wait ends. `step` notices
that the thread is waiting in the system, gives up after two seconds, and says
so:

```
Stopped after 0 instructions: thread 0 is waiting in a system call at 0x00000001967f53cc.
```

`step-all` runs every thread together, one instruction each per round, and
says where each one stopped. A thread that is waiting sits out the rest of the
rounds while the others carry on. Here the main thread waits for the worker
that `thread::start` began, and the worker runs three instructions:

```
DBG> sa 3
Thread 0: stopped after 0 instructions, waiting in a system call; at 0x00000001967f53cc.
Thread 1: stepped 3 instructions; at 0x000000010268cb8c in isPrime+0x130.
```

### Breakpoints

A breakpoint stops the program when any thread reaches it. `<where>` is an
address (`0x` and hex digits, as `list func` and `list thread` show them) or a
function name:

- a name exactly as `list func` shows it in its third column, such as
  `_mfb_fn_isPrime`, `_mfb_rt_io_io_print` or `_exit`;
- otherwise a name as written, as in the fourth column: `isPrime`, or `exit`
  for the system's `exit` (and for your own function `exit` if you wrote one).

A name sets a breakpoint at every function it matches, so `bp isPrime` also
stops in its second versions. `breakpoint r` takes the same forms and removes
what they name.

A system function such as `_exit` has no address until the program has loaded
its libraries. Set before then, its breakpoint waits (`list breakpoint`, under
the header `Address Symbol Written name`, shows `-` for its address), and `run` places it once the libraries are loaded.

`run` lets every thread go until one reaches a breakpoint, an instruction
fails, or the program ends. Press Ctrl-C to pause it wherever it is. A program
`mfb dbg` started cannot read the keyboard while it runs: keystrokes go to
`mfb dbg`.

```sh
mfb build --debug -O0 examples/debug
printf 'bp isPrime\nbp _exit\nrun\nbp r isPrime\nrun\nrun\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 500 2> report.txt
```

```
Launched examples/debug/build/debug.out as process 51166; paused before its first instruction.
DBG> bp isPrime
Set a breakpoint at 0x0000000100510a5c in isPrime.
DBG> bp _exit
Set a pending breakpoint on _exit: it is placed when the program has loaded its libraries.
DBG> run
Placed the pending breakpoint on _exit at 0x00000001966eaf8c.
Thread 0 hit a breakpoint at 0x0000000100510a5c in isPrime.
DBG> bp r isPrime
Removed the breakpoint at 0x0000000100510a5c in isPrime.
DBG> run
round 1: 37 primes
round 2: 31 primes
total: 68 primes
Thread 0 hit a breakpoint at 0x00000001966eaf8c in exit.
DBG> run
The program exited with status 0.
DBG> q
Process 51166 had already exited.
```

Once the program has ended, only `help`, `list`, `list breakpoint`, `list reg`
on its own, and `quit` still work.

### Finishing a function and starting over

`finish` lets the program run until the function the current thread is in
returns, then says where it went back to and what came back: MFBASIC returns
an error code in `x0` (0 when there was none) and the value in `x1`.

```
DBG> finish
Thread 0 returned from sumTo to 0x0000000100ad8854 in main+0x28: x0 = 0x0000000000000000 (0), x1 = 0x000000000000002d (45).
```

If a breakpoint is reached first, `finish` stops there instead.

`restart` starts the program again from the beginning, with the same
arguments, and keeps your breakpoints. It works on a program `mfb dbg`
started, even after it has ended, and picks up a program you rebuilt in the
meantime.

### Reading machine code

`dis` shows the machine code of the function the program is in, the way lldb
does; `->` marks the instruction the thread runs next. "The program" means the
thread `run` last stopped or `step` last ran (thread 0 at first). `dis 5` shows
just the next five instructions, and `dis isPrime` or `dis exit` shows a
function you name, the same way `breakpoint` reads a name.

```
DBG> dis 5
debug.out`isPrime:
->  0x10418ca5c <+0>:  sub    sp, sp, #0xf0
    0x10418ca60 <+4>:  str    x30, [sp]
    0x10418ca64 <+8>:  str    x0, [sp, #0x10]
    0x10418ca68 <+12>: ldr    x8, [sp, #0x10]
    0x10418ca6c <+16>: str    x8, [sp, #0x18]
```

The notes after `;` say where a branch goes (`<+68>` is 68 bytes into this
function), which function a call reaches, and the value of a number. Your
functions are shown in full only in a `--debug` build; otherwise, and for code
the debugger knows nothing about, it shows up to the first `ret` or 64
instructions. `mfb dbg` reads the instructions MFBASIC programs are made of; an
instruction it does not know, which happens mostly inside system libraries,
shows as `.word` and its number.

### Driving it from a script

`mfb dbg` reads its commands from standard input, so a script, or another
program, can send them through a pipe. Two things make that easy:

- Every prompt is printed as soon as the debugger is ready, so a program
  reading the output can wait for `DBG> ` before sending the next command.
- When the commands come from a pipe, each one is printed after the prompt, so
  the output reads the same as a session typed by hand.

When the input ends, the session ends just as if you had typed `quit`.

### Letting mfb dbg debug programs

macOS lets one program pause another only when the debugger is signed for it.
An `mfb` built from source is not, and `mfb dbg` stops with
`cannot attach to process …` and the command to fix it. From the root of the
MFBASIC repository, after every build of `mfb`:

```
sh scripts/sign-debugger.sh
```

Programs that `mfb build` makes need nothing extra.

## Examples

Start the example program paused, list the commands, and quit. Run it from the
root of the MFBASIC repository (`mfb man debug` describes `examples/debug`):

```sh
mfb build --debug -O0 examples/debug
printf 'help\nq\n' | mfb dbg examples/debug/build/debug.out -- 3000 20000
```

```
Launched examples/debug/build/debug.out as process 91023; paused before its first instruction.
DBG> help
Commands:
  help, h    Show this list of commands
  list, l    Show what can be listed
  quit, q    End the session: kill the program if the debugger launched it,
             or detach and let it run on if the debugger attached to it
DBG> q
Killed process 91023.
```

Attach to the running example and list the functions you wrote (`grep '^MU'`
keeps those lines) and where the program is (`pc`):

```sh
mfb build --debug -O0 examples/debug
examples/debug/build/debug.out 3000 20000 > rounds.txt 2> report.txt &
sleep 1
printf 'list func\nlist reg pc\nq\n' | mfb dbg $! | grep -E '^(MU|pc )'
kill $!
```

```
MU   0x0000000104694888 _mfb_fn_next_5Fseed                                 next_seed
MU   0x0000000104694a5c _mfb_fn_isPrime                                     isPrime
MU   0x0000000104694d38 _mfb_fn_countPrimes                                 countPrimes
MU   0x0000000104695078 _mfb_fn_runRound                                    runRound
MU   0x00000001046951ec _mfb_fn_roundWorker                                 roundWorker
MU   0x0000000104695254 _mfb_fn_formatLine                                  formatLine
MU   0x00000001046956bc _mfb_fn_runRounds                                   runRounds
MU   0x0000000104696040 _mfb_fn_main                                        main
pc       0x0000000104694c80 (4368977024)
```

The addresses change from run to run. Here `pc` is between `isPrime` and
`countPrimes`, so the program was inside `isPrime`.

Attach to the example while it runs, then let it go. `$!` is the process ID of
the program the shell just started in the background:

```sh
mfb build --debug -O0 examples/debug
examples/debug/build/debug.out 3000 20000 > rounds.txt 2> report.txt &
sleep 1
printf 'q\n' | mfb dbg $!
kill $!
```

```
Attached to process 91243 (/Users/you/mfb/examples/debug/build/debug.out); paused.
DBG> q
Detached from process 91243; it continues running.
```

## See also

- `mfb man debug`
- `mfb man debug breakpoints` — stop in a function with lldb or gdb
- `mfb spec tooling debugger` — the exact behavior of `mfb dbg`
